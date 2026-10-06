#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(not(target_os = "macos"))]
compile_error!("Herdr Desktop Pet requires a macOS target");

mod agent_outcome;
mod alpha;
mod animation;
mod assets;
mod behavior;
mod bubble;
mod bundle;
mod character_menu;
mod character_renderer;
mod character_selection;
mod character_service;
mod character_store;
mod character_types;
mod control;
mod daemon;
mod dialogue;
mod dialogue_editor;
mod display_geometry;
mod herdr;
mod herdr_protocol;
mod i18n;
mod interaction;
mod lifecycle;
mod lifecycle_settings_ui;
mod menu_panel;
mod pack_authoring;
mod pose;
mod preferences;
mod remote;
mod rig_renderer;
mod session;
mod session_cards;
mod session_view;
mod socket;
mod sources;
mod state;
mod status_indicator;
#[cfg(test)]
mod transport_tests;
mod ui;
use character_service::execute_offline;
use character_store::PackStore;
use character_types::{validate_pack_id, PackAction, PackOperation, PackRequest};
use control::{
    send_command, send_pack_list, send_pack_request, send_pack_status, ControlResponse,
    PackReplyEnvelope,
};
use daemon::DaemonConfig;
use lifecycle::{default_herdr_socket, LifecycleLock, LifecycleSetting, Paths};
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions, Permissions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(
    character_renderer::MAX_PREPARATION_TIMEOUT.as_secs()
        + 2 * character_renderer::SURFACE_PREPARATION_TIMEOUT.as_secs(),
);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const LOG_MODE: u32 = 0o600;
const MAX_SOURCE_PATH_BYTES: usize = 4096;
const USAGE: &str = concat!("Usage: herdr-desktop-pet [COMMAND] [OPTIONS]\n\nCommands:\n    ensure              Start automatically if auto_start is on; otherwise skip\n    start               Start the daemon without changing lifecycle settings\n    stop                Stop the current daemon or pending startup\n    restart             Stop, then manually start without changing settings\n    settings            Open lifecycle settings without starting the pet\n    settings get        Show lifecycle settings as JSON\n    settings set auto_start on|off\n    settings set exit_with_herdr on|off\n    status              Show daemon, registration, data, and executable status\n    show                Show character; reattach visible bubble\n    hide                Hide character; leave enabled bubble standalone\n    toggle              Toggle character visibility, independently of bubble\n    passthrough         Toggle full-window click-through for both windows\n    alpha_passthrough   Toggle alpha-mask click-through for character only\n    show_bubble         Show bubble, including when character is hidden\n    hide_bubble         Hide bubble without changing character visibility\n    bubble_above        Place bubble above pet when attached\n    bubble_below        Place bubble below pet when attached\n    bubble_left         Place bubble to the left of pet when attached\n    bubble_right        Place bubble to the right of pet when attached\n    bubble_auto         Place bubble automatically when attached\n    reset               Reset character and standalone bubble positions; keep visibility\n    bigger              Increase renderer scale\n    smaller             Decrease renderer scale\n    pack list            List managed character packs\n    pack import         Import a character pack directory or archive (--path PATH)\n    pack validate       Validate and fully prepare a character source (--path PATH)\n    pack preview        Render a deterministic source-sized PNG (--path PATH --output PATH [--phase idle|running|waiting|unknown] [--time-ms N] [--reaction head_tap|body_tap|pet|completion_observed] [--reaction-age-ms N] [--hit-overlay])\n    pack export ID      Export a managed revision or builtin ([--revision N] --output PATH)\n    pack select ID      Select the newest revision of a pack\n    pack update ID      Add a revision from a directory (--path DIR)\n    pack restore ID     Restore a historic revision (--revision N)\n    pack remove ID      Remove a pack (active selection falls back to builtin)\n    pack status OPID    Show a pack operation\n\nOptions:\n    --socket PATH       Herdr Unix socket path\n    --assets PATH       Renderer assets directory (legacy commands only)\n    --config-dir PATH   Lifecycle and preference directory\n    --state-dir PATH    Runtime state directory\n    -h, --help          Show this help\n    -V, --version       Show version\n", "\nIndependent-model preview:\n    --pose NAME         waiting, writing, failed, cancelled, disconnected, bored,\n                        happy, head-tap, torso-tap, head-pet (v5 rig packs)\n");

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CommandKind {
    Ensure,
    Start,
    Stop,
    Restart,
    Status,
    Settings,
    Show,
    Hide,
    Toggle,
    Passthrough,
    AlphaPassthrough,
    ShowBubble,
    HideBubble,
    BubbleAbove,
    BubbleBelow,
    BubbleLeft,
    BubbleRight,
    BubbleAuto,
    Reset,
    Bigger,
    Smaller,
    Daemon,
    Pack,
    SettingsGet,
    SettingsSet,
}

impl CommandKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "ensure" => Self::Ensure,
            "start" => Self::Start,
            "stop" => Self::Stop,
            "restart" => Self::Restart,
            "status" => Self::Status,
            "settings" => Self::Settings,
            "show" => Self::Show,
            "hide" => Self::Hide,
            "toggle" => Self::Toggle,
            "passthrough" => Self::Passthrough,
            "alpha_passthrough" => Self::AlphaPassthrough,
            "show_bubble" => Self::ShowBubble,
            "hide_bubble" => Self::HideBubble,
            "bubble_above" => Self::BubbleAbove,
            "bubble_below" => Self::BubbleBelow,
            "bubble_left" => Self::BubbleLeft,
            "bubble_right" => Self::BubbleRight,
            "bubble_auto" => Self::BubbleAuto,
            "reset" => Self::Reset,
            "bigger" => Self::Bigger,
            "smaller" => Self::Smaller,
            "daemon" => Self::Daemon,
            "pack" => Self::Pack,
            _ => return None,
        })
    }

    fn needs_assets(self) -> bool {
        self == Self::Daemon
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Ensure => "ensure",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Status => "status",
            Self::Show => "show",
            Self::Settings => "settings",
            Self::Hide => "hide",
            Self::Toggle => "toggle",
            Self::Passthrough => "passthrough",
            Self::AlphaPassthrough => "alpha_passthrough",
            Self::ShowBubble => "show_bubble",
            Self::HideBubble => "hide_bubble",
            Self::BubbleAbove => "bubble_above",
            Self::BubbleBelow => "bubble_below",
            Self::BubbleLeft => "bubble_left",
            Self::BubbleRight => "bubble_right",
            Self::BubbleAuto => "bubble_auto",
            Self::Reset => "reset",
            Self::Bigger => "bigger",
            Self::Smaller => "smaller",
            Self::Daemon => "daemon",
            Self::Pack => "pack",
            Self::SettingsGet => "settings-get",
            Self::SettingsSet => "settings-set",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum PackCommand {
    List,
    Import {
        path: PathBuf,
    },
    Validate {
        path: PathBuf,
    },
    Preview {
        path: PathBuf,
        output: PathBuf,
        phase: pack_authoring::PreviewPhase,
        time_ms: u64,
        reaction: Option<pack_authoring::PreviewReaction>,
        reaction_age_ms: Option<u64>,
        pose: Option<pose::PoseKind>,
        hit_overlay: bool,
    },
    Export {
        id: String,
        revision: Option<u64>,
        output: PathBuf,
    },
    Select {
        id: String,
    },
    Update {
        id: String,
        path: PathBuf,
    },
    Restore {
        id: String,
        revision: u64,
    },
    Remove {
        id: String,
    },
    Status {
        operation_id: String,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SettingsCommand {
    Get,
    Set(LifecycleSetting, bool),
}

#[derive(Debug, Default)]
struct Cli {
    command: Option<CommandKind>,
    pack: Option<PackCommand>,
    settings: Option<SettingsCommand>,
    socket: Option<PathBuf>,
    assets: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    version: bool,
    help: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("herdr-desktop-pet: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = parse_cli(decode_args(env::args_os().skip(1))?)?;
    if cli.help {
        print!("{USAGE}");
        return Ok(());
    }
    if cli.version {
        println!("herdr-desktop-pet {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let command = cli.command.unwrap_or(CommandKind::Ensure);
    if matches!(
        command,
        CommandKind::Pack
            | CommandKind::Settings
            | CommandKind::SettingsGet
            | CommandKind::SettingsSet
    ) && cli.assets.is_some()
    {
        return Err("--assets is not valid with this command".to_owned());
    }
    let paths = Paths::resolve(cli.config_dir.as_deref(), cli.state_dir.as_deref())?;
    paths.export_environment();
    let herdr_socket = cli.socket.unwrap_or_else(default_herdr_socket);
    if herdr_socket.is_relative() || herdr_socket.as_os_str().is_empty() {
        return Err("Herdr socket path must be a non-empty absolute path".to_owned());
    }
    let assets_override = cli.assets.is_some();
    let assets = if command.needs_assets() {
        Some(resolve_assets(cli.assets.as_deref())?)
    } else {
        cli.assets
    };
    if command == CommandKind::Daemon {
        let automatic_start = match env::var("HERDR_DESKTOP_PET_AUTOMATIC_START") {
            Ok(value) if value == "1" => true,
            Ok(value) if value == "0" => false,
            Err(env::VarError::NotPresent) => false,
            _ => return Err("invalid HERDR_DESKTOP_PET_AUTOMATIC_START value".to_owned()),
        };
        return daemon::run(DaemonConfig::new(
            paths,
            assets.ok_or_else(|| "daemon assets are unavailable".to_owned())?,
            herdr_socket,
            env::var("HERDR_DESKTOP_PET_STARTUP_TOKEN").ok(),
            assets_override,
            automatic_start,
        ));
    }
    execute_command(
        command,
        paths,
        herdr_socket,
        assets,
        assets_override,
        cli.pack,
        cli.settings,
    )
}

fn execute_command(
    command: CommandKind,
    paths: Paths,
    herdr_socket: PathBuf,
    assets: Option<PathBuf>,
    assets_override: bool,
    pack: Option<PackCommand>,
    settings: Option<SettingsCommand>,
) -> Result<(), String> {
    match command {
        CommandKind::Ensure => ensure(paths, herdr_socket, assets, assets_override, true),
        CommandKind::Start => ensure(paths, herdr_socket, assets, assets_override, false),
        CommandKind::Stop => stop(paths),
        CommandKind::Restart => restart(paths, herdr_socket, assets, assets_override),
        CommandKind::Status => status(paths, herdr_socket),
        CommandKind::Settings => lifecycle_settings_ui::run(paths),
        CommandKind::SettingsGet | CommandKind::SettingsSet => {
            let settings = match settings.ok_or_else(|| "settings operation missing".to_owned())? {
                SettingsCommand::Get => control::get_lifecycle_settings(&paths)?,
                SettingsCommand::Set(key, value) => {
                    control::set_lifecycle_setting(&paths, key, value)?
                }
            };
            print_json(&serde_json::json!({"ok": true, "settings": settings}))
        }
        CommandKind::Show
        | CommandKind::Hide
        | CommandKind::Toggle
        | CommandKind::Passthrough
        | CommandKind::AlphaPassthrough
        | CommandKind::ShowBubble
        | CommandKind::HideBubble
        | CommandKind::BubbleAbove
        | CommandKind::BubbleBelow
        | CommandKind::BubbleLeft
        | CommandKind::BubbleRight
        | CommandKind::BubbleAuto
        | CommandKind::Reset
        | CommandKind::Bigger
        | CommandKind::Smaller => control_action(command, paths, herdr_socket),
        CommandKind::Pack => execute_pack(
            pack.ok_or_else(|| "pack requires an operation".to_owned())?,
            paths,
        ),
        CommandKind::Daemon => unreachable!(),
    }
}

fn execute_pack(command: PackCommand, paths: Paths) -> Result<(), String> {
    match command {
        PackCommand::List => execute_pack_list(&paths),
        PackCommand::Status { operation_id } => execute_pack_status(&paths, &operation_id),
        PackCommand::Import { path } => execute_pack_mutation(&paths, PackAction::Import { path }),
        PackCommand::Validate { path } => pack_authoring::validate(&path),
        PackCommand::Preview {
            path,
            output,
            phase,
            time_ms,
            reaction,
            reaction_age_ms,
            pose,
            hit_overlay,
        } => pack_authoring::preview(
            &path,
            &output,
            phase,
            time_ms,
            reaction,
            reaction_age_ms,
            pose,
            hit_overlay,
        ),
        PackCommand::Export {
            id,
            revision,
            output,
        } => {
            let builtin_assets = if id == "default" {
                Some(resolve_assets(None)?)
            } else {
                None
            };
            pack_authoring::export(
                &paths.config_dir,
                builtin_assets.as_deref(),
                &id,
                revision,
                &output,
            )
        }
        PackCommand::Select { id } => execute_pack_mutation(&paths, PackAction::Select { id }),
        PackCommand::Update { id, path } => {
            execute_pack_mutation(&paths, PackAction::Update { id, path })
        }
        PackCommand::Restore { id, revision } => {
            execute_pack_mutation(&paths, PackAction::Restore { id, revision })
        }
        PackCommand::Remove { id } => execute_pack_mutation(&paths, PackAction::Remove { id }),
    }
}

fn execute_pack_list(paths: &Paths) -> Result<(), String> {
    let socket = &paths.control_socket;
    if control::control_socket_is_live(socket, CONTROL_TIMEOUT) {
        let reply = send_pack_list(socket, CONTROL_TIMEOUT)?;
        if !reply.ok {
            return Err(reply
                .error
                .unwrap_or_else(|| "pack listing failed".to_owned()));
        }
        let listing = reply
            .listing
            .ok_or_else(|| "pack listing is unavailable".to_owned())?;
        return print_json(&listing);
    }
    let lock = acquire_lock_until(paths, Instant::now() + CONTROL_TIMEOUT)?;
    if control::control_socket_is_live(socket, Duration::from_millis(150)) {
        drop(lock);
        let reply = send_pack_list(socket, CONTROL_TIMEOUT)?;
        if !reply.ok {
            return Err(reply
                .error
                .unwrap_or_else(|| "pack listing failed".to_owned()));
        }
        let listing = reply
            .listing
            .ok_or_else(|| "pack listing is unavailable".to_owned())?;
        return print_json(&listing);
    }
    let store = PackStore::new(paths.config_dir.clone(), None);
    let listing = store.list()?;
    drop(lock);
    print_json(&listing)
}
fn execute_pack_status(paths: &Paths, operation_id: &str) -> Result<(), String> {
    let socket = &paths.control_socket;
    if control::control_socket_is_live(socket, CONTROL_TIMEOUT) {
        let reply = send_pack_status(socket, operation_id, CONTROL_TIMEOUT)?;
        return print_pack_operation_reply(reply);
    }
    let lock = acquire_lock_until(paths, Instant::now() + CONTROL_TIMEOUT)?;
    if control::control_socket_is_live(socket, Duration::from_millis(150)) {
        drop(lock);
        let reply = send_pack_status(socket, operation_id, CONTROL_TIMEOUT)?;
        return print_pack_operation_reply(reply);
    }
    let store = PackStore::new(paths.config_dir.clone(), None);
    let operation = store.operation_status(operation_id)?;
    drop(lock);
    match operation {
        Some(operation) => print_pack_operation(&operation),
        None => {
            let operation = PackOperation {
                operation_id: operation_id.to_owned(),
                state: "unknown".to_owned(),
                committed: false,
                ui_applied: false,
                generation: None,
                error: Some("operation identity is not retained".to_owned()),
            };
            print_pack_operation(&operation)
        }
    }
}

fn execute_pack_mutation(paths: &Paths, action: PackAction) -> Result<(), String> {
    let request = PackRequest {
        operation_id: new_operation_id(),
        expected_generation: None,
        action,
    };
    let socket = &paths.control_socket;
    if control::control_socket_is_live(socket, CONTROL_TIMEOUT) {
        return submit_live_pack(socket, request);
    }
    let lock = acquire_lock_until(paths, Instant::now() + CONTROL_TIMEOUT)?;
    if control::control_socket_is_live(socket, Duration::from_millis(150)) {
        drop(lock);
        return submit_live_pack(socket, request);
    }
    let builtin_assets = resolve_assets(None)?;
    let operation = execute_offline(paths.config_dir.clone(), builtin_assets, request)?;
    drop(lock);
    print_json(&operation)?;
    operation_result(&operation)
}

fn submit_live_pack(socket: &Path, request: PackRequest) -> Result<(), String> {
    let operation_id = request.operation_id.clone();
    let reply = match send_pack_request(socket, request, CONTROL_TIMEOUT) {
        Ok(reply) => reply,
        Err(error) => {
            return Err(format!(
                "pack request outcome is unknown after control disconnect: {error}"
            ))
        }
    };
    let operation = reply.operation.ok_or_else(|| {
        reply
            .error
            .clone()
            .unwrap_or_else(|| "pack request was not accepted".to_owned())
    })?;
    print_json(&operation)?;
    if !reply.ok {
        return Err(operation
            .error
            .unwrap_or_else(|| "pack request failed".to_owned()));
    }
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut last_state = operation.state.clone();
    if is_pack_terminal(&operation.state) {
        return operation_result(&operation);
    }
    while Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
        let reply = match send_pack_status(socket, &operation_id, CONTROL_TIMEOUT) {
            Ok(reply) => reply,
            Err(error) => {
                return Err(format!(
                "pack operation {operation_id} status is unknown after control disconnect: {error}"
            ))
            }
        };
        let operation = reply.operation.ok_or_else(|| {
            reply
                .error
                .clone()
                .unwrap_or_else(|| "pack operation status is unavailable".to_owned())
        })?;
        if operation.state != last_state {
            print_json(&operation)?;
            last_state = operation.state.clone();
        }
        if is_pack_terminal(&operation.state) {
            return operation_result(&operation);
        }
    }
    Err(format!(
        "pack operation {operation_id} remains pending; status is unknown"
    ))
}

fn is_pack_terminal(state: &str) -> bool {
    matches!(
        state,
        "completed"
            | "failed"
            | "canceled"
            | "durability_unknown"
            | "committed_pending_apply"
            | "unknown"
    )
}

fn operation_result(operation: &PackOperation) -> Result<(), String> {
    if operation.state == "completed" {
        Ok(())
    } else {
        Err(operation
            .error
            .clone()
            .unwrap_or_else(|| format!("pack operation ended in {}", operation.state)))
    }
}

fn pack_status_result(operation: &PackOperation) -> Result<(), String> {
    if matches!(
        operation.state.as_str(),
        "accepted" | "preparing" | "applying"
    ) {
        Ok(())
    } else {
        operation_result(operation)
    }
}

fn print_pack_operation(operation: &PackOperation) -> Result<(), String> {
    print_json(operation)?;
    pack_status_result(operation)
}

fn print_pack_operation_reply(reply: PackReplyEnvelope) -> Result<(), String> {
    if let Some(operation) = reply.operation {
        print_pack_operation(&operation)
    } else {
        Err(reply
            .error
            .unwrap_or_else(|| "pack operation is unavailable".to_owned()))
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let text = serde_json::to_string(value)
        .map_err(|error| format!("cannot encode pack response: {error}"))?;
    println!("{text}");
    Ok(())
}

fn new_operation_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{:x}-{counter:x}", std::process::id())
}

fn ensure(
    paths: Paths,
    herdr_socket: PathBuf,
    assets: Option<PathBuf>,
    assets_override: bool,
    automatic_start: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut last_error = None;
    while Instant::now() < deadline {
        if control::control_socket_is_live(&paths.control_socket, Duration::from_millis(150)) {
            match wait_for_ready(&paths, &herdr_socket, deadline) {
                Ok(response) => return print_response(&response),
                Err(ReadyFailure::Exiting(error)) => {
                    last_error = Some(error);
                    wait_for_finalization(&paths, deadline)?;
                    continue;
                }
                Err(ReadyFailure::TimedOut(error)) => {
                    last_error = Some(error);
                    if control::control_socket_is_live(
                        &paths.control_socket,
                        Duration::from_millis(150),
                    ) {
                        match send_command(&paths.control_socket, "status", None, CONTROL_TIMEOUT) {
                            Ok(response) if !response.shutdown => {
                                return Err(last_error
                                    .take()
                                    .unwrap_or_else(|| "desktop-pet is not ready".to_owned()));
                            }
                            Err(error)
                                if control::control_socket_is_live(
                                    &paths.control_socket,
                                    Duration::from_millis(100),
                                ) =>
                            {
                                return Err(error)
                            }
                            _ => {}
                        }
                    }
                    wait_for_finalization(&paths, deadline)?;
                    continue;
                }
            }
        }
        let Some(lock) = LifecycleLock::try_acquire(&paths.state_dir)? else {
            thread::sleep(POLL_INTERVAL);
            continue;
        };
        if control::control_socket_is_live(&paths.control_socket, Duration::from_millis(150)) {
            drop(lock);
            continue;
        }
        let settings = lifecycle::read_settings(&paths.config_dir)?;
        if automatic_start && !settings.auto_start {
            if lifecycle::startup_pending(&paths.config_dir)? {
                drop(lock);
                thread::sleep(POLL_INTERVAL);
                continue;
            }
            drop(lock);
            return print_json(&serde_json::json!({
                "type": "ensure", "ok": true, "running": false, "skipped": true,
                "auto_start": settings.auto_start, "exit_with_herdr": settings.exit_with_herdr
            }));
        }
        let assets = resolve_assets(assets.as_deref())?;
        let lease = match lifecycle::begin_startup(&paths.config_dir)? {
            Some(lease) => lease,
            None => {
                drop(lock);
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };
        let child_pid = match spawn_daemon(
            &paths,
            &herdr_socket,
            Some(&assets),
            assets_override,
            &lease.token,
            automatic_start,
        ) {
            Ok(pid) => pid,
            Err(error) => {
                let _ = lifecycle::abandon_startup(&paths.config_dir, &lease.token);
                return Err(error);
            }
        };
        lifecycle::set_startup_child(&paths.config_dir, &lease.token, child_pid)?;
        drop(lock);
        match wait_for_ready(&paths, &herdr_socket, deadline) {
            Ok(response) => return print_response(&response),
            Err(failure) => {
                if let ReadyFailure::TimedOut(error) = &failure {
                    if control::control_socket_is_live(
                        &paths.control_socket,
                        Duration::from_millis(150),
                    ) {
                        match send_command(&paths.control_socket, "status", None, CONTROL_TIMEOUT) {
                            Ok(response) if !response.shutdown => return Err(error.clone()),
                            Err(error)
                                if control::control_socket_is_live(
                                    &paths.control_socket,
                                    Duration::from_millis(100),
                                ) =>
                            {
                                return Err(error)
                            }
                            _ => {}
                        }
                    }
                }
                clear_startup_reservation(&paths, &lease.token, deadline)?;
                wait_for_finalization(&paths, deadline)?;
                // One ensure never spawns a second daemon: its child either
                // exited before ready (a respawn would repeat that) or used up
                // the whole deadline.
                return Err(failure.into_message());
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "timed out waiting for desktop-pet readiness".to_owned()))
}

fn acquire_lock_until(paths: &Paths, deadline: Instant) -> Result<LifecycleLock, String> {
    loop {
        match LifecycleLock::try_acquire(&paths.state_dir)? {
            Some(lock) => return Ok(lock),
            None if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            None => return Err("timed out waiting for desktop-pet lifecycle lock".to_owned()),
        }
    }
}

fn clear_startup_reservation(paths: &Paths, token: &str, deadline: Instant) -> Result<(), String> {
    let lock = acquire_lock_until(paths, deadline)?;
    lifecycle::abandon_startup(&paths.config_dir, token)?;
    drop(lock);
    Ok(())
}

fn wait_for_finalization(paths: &Paths, deadline: Instant) -> Result<(), String> {
    loop {
        if control::control_socket_is_live(&paths.control_socket, Duration::from_millis(100)) {
            if Instant::now() >= deadline {
                return Err("timed out waiting for desktop-pet finalization".to_owned());
            }
            thread::sleep(POLL_INTERVAL);
            continue;
        }
        if let Some(lock) = LifecycleLock::try_acquire(&paths.state_dir)? {
            let pending = lifecycle::startup_pending(&paths.config_dir)?;
            if !pending {
                lifecycle::cancel_startup(&paths.config_dir)?;
                drop(lock);
                return Ok(());
            }
            drop(lock);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for desktop-pet finalization".to_owned());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn restart(
    paths: Paths,
    herdr_socket: PathBuf,
    assets: Option<PathBuf>,
    assets_override: bool,
) -> Result<(), String> {
    stop(paths.clone())?;
    ensure(paths, herdr_socket, assets, assets_override, false)
}

fn stop(paths: Paths) -> Result<(), String> {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err("timed out stopping desktop-pet".to_owned());
        }
        if control::control_socket_is_live(&paths.control_socket, Duration::from_millis(150)) {
            match send_command(&paths.control_socket, "stop", None, CONTROL_TIMEOUT) {
                Ok(response) if response.ok || response.shutdown => {
                    wait_for_finalization(&paths, deadline)?;
                    return Ok(());
                }
                Ok(response) => {
                    return Err(response
                        .error
                        .unwrap_or_else(|| "cannot stop daemon".to_owned()))
                }
                Err(error)
                    if control::control_socket_is_live(
                        &paths.control_socket,
                        Duration::from_millis(100),
                    ) =>
                {
                    return Err(error)
                }
                Err(_) => continue,
            }
        }
        let Some(lock) = LifecycleLock::try_acquire(&paths.state_dir)? else {
            thread::sleep(POLL_INTERVAL);
            continue;
        };
        if control::control_socket_is_live(&paths.control_socket, Duration::from_millis(150)) {
            drop(lock);
            continue;
        }
        lifecycle::cancel_startup(&paths.config_dir)?;
        drop(lock);
        wait_for_finalization(&paths, deadline)?;
        return Ok(());
    }
}

fn status(paths: Paths, herdr_socket: PathBuf) -> Result<(), String> {
    let response = if control::control_socket_is_live(&paths.control_socket, CONTROL_TIMEOUT) {
        send_command(&paths.control_socket, "status", None, CONTROL_TIMEOUT)?
    } else {
        let settings = lifecycle::read_settings(&paths.config_dir)?;
        let executable = bundle::executable()
            .map_err(|error| format!("cannot resolve executable path: {error}"))?;
        ControlResponse {
            r#type: "status".to_owned(),
            version: 1,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            ok: true,
            command: "status".to_owned(),
            error: None,
            ready: false,
            running: false,
            phase: "unknown".to_owned(),
            ui_ready: false,
            control_ready: false,
            registration_accepted: false,
            data_connected: false,
            connected_sources: 0,
            disconnected_sources: 0,
            sessions: 0,
            working: 0,
            blocked: 0,
            done: 0,
            unknown: 0,
            visible: false,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            bubble_placement: bubble::BubblePlacement::Above,
            scale: state::DEFAULT_SCALE,
            shutdown: false,
            auto_start: settings.auto_start,
            exit_with_herdr: settings.exit_with_herdr,
            pid: 0,
            executable_path: executable.display().to_string(),
            config_dir: paths.config_dir.display().to_string(),
            state_dir: paths.state_dir.display().to_string(),
            herdr_socket: herdr_socket.display().to_string(),
        }
    };
    print_response(&response)
}

fn control_action(command: CommandKind, paths: Paths, herdr_socket: PathBuf) -> Result<(), String> {
    let registration = send_command(
        &paths.control_socket,
        "register",
        Some(&herdr_socket),
        CONTROL_TIMEOUT,
    )?;
    if !registration.ok {
        return Err(registration
            .error
            .unwrap_or_else(|| "Herdr endpoint registration failed".to_owned()));
    }
    let response = send_command(
        &paths.control_socket,
        command.as_str(),
        Some(&herdr_socket),
        CONTROL_TIMEOUT,
    )?;
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "control action failed".to_owned()));
    }
    print_response(&response)
}

/// Why `wait_for_ready` gave up.
#[derive(Debug)]
enum ReadyFailure {
    /// The daemon reported shutdown, or its socket vanished after it had
    /// answered: waiting longer cannot make it ready.
    Exiting(String),
    /// `deadline` passed without a ready response.
    TimedOut(String),
}

impl ReadyFailure {
    fn into_message(self) -> String {
        match self {
            Self::Exiting(message) | Self::TimedOut(message) => message,
        }
    }
}

fn wait_for_ready(
    paths: &Paths,
    herdr_socket: &Path,
    deadline: Instant,
) -> Result<ControlResponse, ReadyFailure> {
    let mut last_error = None;
    let mut registered = false;
    let mut answered = false;
    while Instant::now() < deadline {
        let (command, endpoint) = if registered {
            ("ready", None)
        } else {
            ("register", Some(herdr_socket))
        };
        match send_command(
            &paths.control_socket,
            command,
            endpoint,
            CONTROL_TIMEOUT.min(Duration::from_millis(500)),
        ) {
            Ok(response) if response.shutdown => {
                return Err(ReadyFailure::Exiting(
                    response
                        .error
                        .unwrap_or_else(|| "desktop pet is shutting down".to_owned()),
                ));
            }
            Ok(response) if !registered => {
                answered = true;
                if response.ok && response.registration_accepted {
                    registered = true;
                } else {
                    last_error = response
                        .error
                        .or_else(|| Some("Herdr endpoint registration is pending".to_owned()));
                }
            }
            Ok(response) if response.ok && response.ready && response.control_ready => {
                return Ok(response);
            }
            Ok(response) => {
                answered = true;
                last_error = response.error;
            }
            Err(error) => {
                // Connection errors before any answer are expected while a
                // fresh daemon is still binding its socket.
                if answered
                    && fs::symlink_metadata(&paths.control_socket)
                        .is_err_and(|missing| missing.kind() == io::ErrorKind::NotFound)
                {
                    return Err(ReadyFailure::Exiting(error));
                }
                last_error = Some(error);
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(ReadyFailure::TimedOut(last_error.unwrap_or_else(|| {
        "timed out waiting for desktop-pet readiness".to_owned()
    })))
}

fn spawn_daemon(
    paths: &Paths,
    herdr_socket: &Path,
    assets: Option<&Path>,
    assets_override: bool,
    startup_token: &str,
    automatic_start: bool,
) -> Result<u32, String> {
    let executable = bundle::executable()
        .map_err(|error| format!("cannot resolve desktop-pet executable path: {error}"))?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(LOG_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&paths.log_file)
        .map_err(|error| {
            format!(
                "cannot open daemon log {}: {error}",
                paths.log_file.display()
            )
        })?;
    let log_stderr = log
        .try_clone()
        .map_err(|error| format!("cannot duplicate daemon log: {error}"))?;
    fs::set_permissions(&paths.log_file, Permissions::from_mode(LOG_MODE)).map_err(|error| {
        format!(
            "cannot secure daemon log {}: {error}",
            paths.log_file.display()
        )
    })?;
    let mut command = Command::new(&executable);
    command
        .arg("daemon")
        .arg("--socket")
        .arg(herdr_socket)
        .arg("--config-dir")
        .arg(&paths.config_dir)
        .arg("--state-dir")
        .arg(&paths.state_dir)
        .env("HERDR_PLUGIN_CONFIG_DIR", &paths.config_dir)
        .env("HERDR_PLUGIN_STATE_DIR", &paths.state_dir)
        .env("HERDR_SOCKET_PATH", herdr_socket)
        .env("HERDR_DESKTOP_PET_STARTUP_TOKEN", startup_token)
        .env(
            "HERDR_DESKTOP_PET_AUTOMATIC_START",
            if automatic_start { "1" } else { "0" },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_stderr));
    if assets_override {
        command
            .arg("--assets")
            .arg(assets.ok_or_else(|| "explicit renderer assets are unavailable".to_owned())?);
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map(|child| child.id())
        .map_err(|error| format!("cannot start detached desktop-pet daemon: {error}"))
}

fn print_response(response: &ControlResponse) -> Result<(), String> {
    let text = serde_json::to_string(response)
        .map_err(|error| format!("cannot encode status response: {error}"))?;
    println!("{text}");
    Ok(())
}
fn decode_args<I>(arguments: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = OsString>,
{
    arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "command-line arguments must be valid UTF-8".to_owned())
        })
        .collect()
}

fn parse_cli<I>(arguments: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli::default();
    let mut arguments = arguments.into_iter();
    let mut pack_tokens = Vec::new();
    let mut settings_tokens = Vec::new();
    while let Some(argument) = arguments.next() {
        if parse_global_option(&argument, &mut arguments, &mut cli)? {
            continue;
        }
        if cli.command == Some(CommandKind::Pack) {
            pack_tokens.push(argument);
            continue;
        }
        if cli.command == Some(CommandKind::Settings) {
            settings_tokens.push(argument);
            continue;
        }
        let Some(command) = CommandKind::parse(&argument) else {
            return Err(format!("unknown command {argument:?}\n\n{USAGE}"));
        };
        if cli.command.replace(command).is_some() {
            return Err("only one command may be specified".to_owned());
        }
    }
    if cli.command == Some(CommandKind::Pack) {
        if pack_tokens.is_empty() {
            if cli.help || cli.version {
                return Ok(cli);
            }
            return Err(format!("pack requires an operation\n\n{USAGE}"));
        }
        cli.pack = Some(parse_pack_tokens(&pack_tokens)?);
    }
    if cli.command == Some(CommandKind::Settings) && !settings_tokens.is_empty() {
        let operation = parse_settings_tokens(&settings_tokens)?;
        cli.command = Some(match operation {
            SettingsCommand::Get => CommandKind::SettingsGet,
            SettingsCommand::Set(_, _) => CommandKind::SettingsSet,
        });
        cli.settings = Some(operation);
    }
    Ok(cli)
}

fn parse_settings_tokens(tokens: &[String]) -> Result<SettingsCommand, String> {
    match tokens {
        [operation] if operation == "get" => Ok(SettingsCommand::Get),
        [operation, key, value] if operation == "set" => {
            let key = key.parse::<LifecycleSetting>()?;
            let value = match value.as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("settings value must be on or off".to_owned()),
            };
            Ok(SettingsCommand::Set(key, value))
        }
        _ => Err(
            "expected settings get or settings set auto_start|exit_with_herdr on|off".to_owned(),
        ),
    }
}

fn parse_global_option<I>(argument: &str, arguments: &mut I, cli: &mut Cli) -> Result<bool, String>
where
    I: Iterator<Item = String>,
{
    match argument {
        "--help" | "-h" => {
            if cli.help {
                return Err("duplicate --help".to_owned());
            }
            cli.help = true;
            Ok(true)
        }
        "--version" | "-V" => {
            if cli.version {
                return Err("duplicate --version".to_owned());
            }
            cli.version = true;
            Ok(true)
        }
        "--socket" => {
            set_unique_path(
                &mut cli.socket,
                next_path(arguments, "--socket")?,
                "--socket",
            )?;
            Ok(true)
        }
        "--assets" => {
            set_unique_path(
                &mut cli.assets,
                next_path(arguments, "--assets")?,
                "--assets",
            )?;
            Ok(true)
        }
        "--config-dir" => {
            set_unique_path(
                &mut cli.config_dir,
                next_path(arguments, "--config-dir")?,
                "--config-dir",
            )?;
            Ok(true)
        }
        "--state-dir" => {
            set_unique_path(
                &mut cli.state_dir,
                next_path(arguments, "--state-dir")?,
                "--state-dir",
            )?;
            Ok(true)
        }
        value if value.starts_with("--socket=") => {
            set_unique_path(
                &mut cli.socket,
                nonempty_path(value.trim_start_matches("--socket="), "--socket")?,
                "--socket",
            )?;
            Ok(true)
        }
        value if value.starts_with("--assets=") => {
            set_unique_path(
                &mut cli.assets,
                nonempty_path(value.trim_start_matches("--assets="), "--assets")?,
                "--assets",
            )?;
            Ok(true)
        }
        value if value.starts_with("--config-dir=") => {
            set_unique_path(
                &mut cli.config_dir,
                nonempty_path(value.trim_start_matches("--config-dir="), "--config-dir")?,
                "--config-dir",
            )?;
            Ok(true)
        }
        value if value.starts_with("--state-dir=") => {
            set_unique_path(
                &mut cli.state_dir,
                nonempty_path(value.trim_start_matches("--state-dir="), "--state-dir")?,
                "--state-dir",
            )?;
            Ok(true)
        }
        "--path" | "--output" | "--revision" | "--phase" | "--time-ms" | "--reaction"
        | "--reaction-age-ms" | "--pose" | "--hit-overlay" => Ok(false),
        value
            if value.starts_with("--path=")
                || value.starts_with("--output=")
                || value.starts_with("--revision=")
                || value.starts_with("--phase=")
                || value.starts_with("--time-ms=")
                || value.starts_with("--reaction=")
                || value.starts_with("--pose=")
                || value.starts_with("--reaction-age-ms=") =>
        {
            Ok(false)
        }
        value if value.starts_with('-') => Err(format!("unknown option {value:?}\n\n{USAGE}")),
        _ => Ok(false),
    }
}

fn set_unique_path(
    destination: &mut Option<PathBuf>,
    value: PathBuf,
    option: &str,
) -> Result<(), String> {
    if destination.replace(value).is_some() {
        return Err(format!("duplicate {option}"));
    }
    Ok(())
}

fn parse_pack_tokens(tokens: &[String]) -> Result<PackCommand, String> {
    let operation = tokens
        .first()
        .ok_or_else(|| format!("pack requires an operation\n\n{USAGE}"))?;
    let mut index = 1usize;
    let mut path = None;
    let mut output = None;
    let mut revision = None;
    let mut phase = None;
    let mut time_ms = None;
    let mut reaction = None;
    let mut reaction_age_ms = None;
    let mut pose = None;
    let mut hit_overlay = false;
    let mut positional = Vec::new();
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "--path" => {
                if path.is_some() {
                    return Err("duplicate --path".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--path requires a directory".to_owned())?;
                path = Some(absolute_input_path(value)?);
            }
            value if value.starts_with("--path=") => {
                if path.is_some() {
                    return Err("duplicate --path".to_owned());
                }
                path = Some(absolute_input_path(value.trim_start_matches("--path="))?);
            }
            "--output" => {
                if output.is_some() {
                    return Err("duplicate --output".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--output requires a path".to_owned())?;
                output = Some(absolute_output_path(value)?);
            }
            value if value.starts_with("--output=") => {
                if output.is_some() {
                    return Err("duplicate --output".to_owned());
                }
                output = Some(absolute_output_path(value.trim_start_matches("--output="))?);
            }
            "--revision" => {
                if revision.is_some() {
                    return Err("duplicate --revision".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--revision requires a number".to_owned())?;
                revision = Some(parse_revision(value)?);
            }
            value if value.starts_with("--revision=") => {
                if revision.is_some() {
                    return Err("duplicate --revision".to_owned());
                }
                revision = Some(parse_revision(value.trim_start_matches("--revision="))?);
            }
            "--phase" => {
                if phase.is_some() {
                    return Err("duplicate --phase".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--phase requires a value".to_owned())?;
                phase = Some(pack_authoring::parse_phase(value)?);
            }
            value if value.starts_with("--phase=") => {
                if phase.is_some() {
                    return Err("duplicate --phase".to_owned());
                }
                phase = Some(pack_authoring::parse_phase(
                    value.trim_start_matches("--phase="),
                )?);
            }
            "--time-ms" => {
                if time_ms.is_some() {
                    return Err("duplicate --time-ms".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--time-ms requires a value".to_owned())?;
                time_ms = Some(pack_authoring::parse_millis(value, "--time-ms")?);
            }
            value if value.starts_with("--time-ms=") => {
                if time_ms.is_some() {
                    return Err("duplicate --time-ms".to_owned());
                }
                time_ms = Some(pack_authoring::parse_millis(
                    value.trim_start_matches("--time-ms="),
                    "--time-ms",
                )?);
            }
            "--reaction" => {
                if reaction.is_some() {
                    return Err("duplicate --reaction".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--reaction requires a value".to_owned())?;
                reaction = Some(pack_authoring::parse_reaction(value)?);
            }
            value if value.starts_with("--reaction=") => {
                if reaction.is_some() {
                    return Err("duplicate --reaction".to_owned());
                }
                reaction = Some(pack_authoring::parse_reaction(
                    value.trim_start_matches("--reaction="),
                )?);
            }
            "--reaction-age-ms" => {
                if reaction_age_ms.is_some() {
                    return Err("duplicate --reaction-age-ms".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--reaction-age-ms requires a value".to_owned())?;
                reaction_age_ms = Some(pack_authoring::parse_millis(value, "--reaction-age-ms")?);
            }
            value if value.starts_with("--reaction-age-ms=") => {
                if reaction_age_ms.is_some() {
                    return Err("duplicate --reaction-age-ms".to_owned());
                }
                reaction_age_ms = Some(pack_authoring::parse_millis(
                    value.trim_start_matches("--reaction-age-ms="),
                    "--reaction-age-ms",
                )?);
            }
            "--pose" => {
                if pose.is_some() {
                    return Err("duplicate --pose".to_owned());
                }
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "--pose requires a value".to_owned())?;
                pose = Some(
                    pose::PoseKind::parse(value)
                        .ok_or_else(|| format!("unknown pose {value:?}"))?,
                );
            }
            value if value.starts_with("--pose=") => {
                if pose.is_some() {
                    return Err("duplicate --pose".to_owned());
                }
                let value = value.trim_start_matches("--pose=");
                pose = Some(
                    pose::PoseKind::parse(value)
                        .ok_or_else(|| format!("unknown pose {value:?}"))?,
                );
            }
            "--hit-overlay" => {
                if hit_overlay {
                    return Err("duplicate --hit-overlay".to_owned());
                }
                hit_overlay = true;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown pack option {value:?}\n\n{USAGE}"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }
    let has_preview_options = phase.is_some()
        || time_ms.is_some()
        || reaction.is_some()
        || reaction_age_ms.is_some()
        || pose.is_some()
        || hit_overlay;
    if operation.as_str() == "preview" && reaction.is_none() && reaction_age_ms.is_some() {
        return Err("--reaction-age-ms requires --reaction".to_owned());
    }
    match operation.as_str() {
        "list"
            if positional.is_empty()
                && path.is_none()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::List)
        }
        "import"
            if positional.is_empty()
                && path.is_some()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Import {
                path: path.expect("checked above"),
            })
        }
        "validate"
            if positional.is_empty()
                && path.is_some()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Validate {
                path: path.expect("checked above"),
            })
        }
        "preview"
            if positional.is_empty()
                && path.is_some()
                && output.is_some()
                && revision.is_none() =>
        {
            Ok(PackCommand::Preview {
                path: path.expect("checked above"),
                output: output.expect("checked above"),
                phase: phase.unwrap_or(pack_authoring::PreviewPhase::Idle),
                time_ms: time_ms.unwrap_or(0),
                reaction,
                reaction_age_ms,
                pose,
                hit_overlay,
            })
        }
        "export"
            if positional.len() == 1
                && path.is_none()
                && output.is_some()
                && !has_preview_options =>
        {
            Ok(PackCommand::Export {
                id: select_id(&positional[0])?,
                revision,
                output: output.expect("checked above"),
            })
        }
        "select"
            if positional.len() == 1
                && path.is_none()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Select {
                id: select_id(&positional[0])?,
            })
        }
        "update"
            if positional.len() == 1
                && path.is_some()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Update {
                id: nonempty_id(&positional[0])?,
                path: path.expect("checked above"),
            })
        }
        "restore"
            if positional.len() == 1
                && path.is_none()
                && output.is_none()
                && revision.is_some()
                && !has_preview_options =>
        {
            Ok(PackCommand::Restore {
                id: nonempty_id(&positional[0])?,
                revision: revision.expect("checked above"),
            })
        }
        "remove"
            if positional.len() == 1
                && path.is_none()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Remove {
                id: nonempty_id(&positional[0])?,
            })
        }
        "status"
            if positional.len() == 1
                && path.is_none()
                && output.is_none()
                && revision.is_none()
                && !has_preview_options =>
        {
            Ok(PackCommand::Status {
                operation_id: nonempty_operation_id(&positional[0])?,
            })
        }
        _ => Err(format!("invalid pack operation arguments\n\n{USAGE}")),
    }
}
fn parse_revision(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid revision {value:?}"));
    }
    value
        .parse::<u64>()
        .map_err(|_| format!("revision {value:?} is out of range"))
}

fn nonempty_id(value: &str) -> Result<String, String> {
    validate_pack_id(value)?;
    Ok(value.to_owned())
}

fn select_id(value: &str) -> Result<String, String> {
    if value == "default" {
        Ok(value.to_owned())
    } else {
        nonempty_id(value)
    }
}

fn nonempty_operation_id(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 128
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("pack operation ID is invalid".to_owned());
    }
    Ok(value.to_owned())
}
fn validate_source_path(path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err("pack source path must not contain NUL bytes".to_owned());
    }
    if bytes.len() > MAX_SOURCE_PATH_BYTES {
        return Err(format!(
            "pack source path must be at most {MAX_SOURCE_PATH_BYTES} bytes"
        ));
    }
    Ok(())
}

fn absolute_input_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.starts_with('-') {
        return Err("--path requires a directory".to_owned());
    }
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| {
                format!("cannot resolve --path relative to current directory: {error}")
            })?
    };
    validate_source_path(&path)?;
    Ok(path)
}

fn absolute_output_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.starts_with('-') {
        return Err("--output requires a non-empty path".to_owned());
    }
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| {
                format!("cannot resolve --output relative to current directory: {error}")
            })?
    };
    validate_source_path(&path).map_err(|error| error.replace("source path", "output path"))?;
    Ok(path)
}

fn next_path<I>(arguments: &mut I, option: &str) -> Result<PathBuf, String>
where
    I: Iterator<Item = String>,
{
    let value = arguments
        .next()
        .ok_or_else(|| format!("{option} requires a path\n\n{USAGE}"))?;
    nonempty_path(&value, option)
}

fn nonempty_path(value: &str, option: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.starts_with('-') {
        Err(format!("{option} requires a non-empty path\n\n{USAGE}"))
    } else {
        Ok(PathBuf::from(value))
    }
}

fn resolve_assets(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return validate_assets_path(path);
    }
    let candidate = match bundle::contents_dir() {
        Some(contents) => contents.join("Resources/default"),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/rubelia-default"),
    };
    validate_assets_path(&candidate)
}

fn validate_assets_path(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("assets path {} is unavailable: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("assets path {} is not a directory", path.display()));
    }
    let manifest = path.join("manifest.json");
    let manifest_metadata = fs::symlink_metadata(&manifest).map_err(|error| {
        format!(
            "assets manifest {} is unavailable: {error}",
            manifest.display()
        )
    })?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(format!(
            "assets manifest {} is not a regular file",
            manifest.display()
        ));
    }
    fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve assets path {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_cli_rejects_ambiguous_or_invalid_mutations() {
        let args = |values: &[&str]| parse_cli(values.iter().map(|value| (*value).to_owned()));
        for tokens in [
            vec!["settings", "set", "auto_start", "true"],
            vec!["settings", "set", "auto_start"],
            vec!["settings", "set", "exit_with_herdr", "off", "extra"],
            vec!["settings", "set", "enabled", "off"],
            vec!["settings", "get", "auto_start"],
            vec!["settings", "set", "exit_with_herdr", "OFF"],
        ] {
            assert!(args(&tokens).is_err(), "{tokens:?}");
        }
        let parsed = args(&["settings", "set", "exit_with_herdr", "off"]).unwrap();
        assert_eq!(
            parsed.settings,
            Some(SettingsCommand::Set(LifecycleSetting::ExitWithHerdr, false))
        );
    }

    #[test]
    fn cli_accepts_every_command_and_explicit_paths() {
        let cli = parse_cli([
            "restart".to_owned(),
            "--socket=/tmp/herdr.sock".to_owned(),
            "--assets".to_owned(),
            "/tmp/assets".to_owned(),
            "--config-dir".to_owned(),
            "/tmp/config".to_owned(),
            "--state-dir".to_owned(),
            "/tmp/state".to_owned(),
        ])
        .expect("cli");
        assert_eq!(cli.command, Some(CommandKind::Restart));
        assert_eq!(cli.socket, Some(PathBuf::from("/tmp/herdr.sock")));
    }

    #[test]
    fn assets_never_use_legacy_omp_paths() {
        let source = resolve_assets(Some(Path::new("/tmp/not-a-real-assets-path")));
        assert!(source.is_err());
    }

    #[test]
    fn pack_cli_is_grouped_strict_and_normalizes_relative_paths() {
        let cli = parse_cli([
            "pack".to_owned(),
            "import".to_owned(),
            "--path".to_owned(),
            "packs/example".to_owned(),
        ])
        .expect("pack import");
        assert_eq!(cli.command, Some(CommandKind::Pack));
        match cli.pack {
            Some(PackCommand::Import { path }) => assert!(path.is_absolute()),
            other => panic!("unexpected pack command: {other:?}"),
        }
        let cli = parse_cli(["pack".to_owned(), "select".to_owned(), "default".to_owned()])
            .expect("builtin select");
        assert!(matches!(
            cli.pack,
            Some(PackCommand::Select { id }) if id == "default"
        ));
        assert!(parse_cli(["pack".to_owned(), "select".to_owned(), "@png".to_owned()]).is_err());
        assert!(
            parse_cli(["pack".to_owned(), "remove".to_owned(), "default".to_owned(),]).is_err()
        );
        assert!(parse_cli([
            "pack".to_owned(),
            "select".to_owned(),
            "my-pack".to_owned(),
            "my-other-pack".to_owned(),
        ])
        .is_err());
        assert!(parse_cli([
            "pack".to_owned(),
            "import".to_owned(),
            "--path".to_owned(),
            "one".to_owned(),
            "--path".to_owned(),
            "two".to_owned(),
        ])
        .is_err());
    }

    #[test]
    fn pack_status_exit_boundaries_distinguish_progress_and_failure() {
        let operation = |state: &str| PackOperation {
            operation_id: "operation".to_owned(),
            state: state.to_owned(),
            committed: state != "accepted",
            ui_applied: false,
            generation: None,
            error: Some("operation failed".to_owned()),
        };
        for state in ["accepted", "preparing", "applying"] {
            assert!(pack_status_result(&operation(state)).is_ok());
        }
        for state in [
            "failed",
            "canceled",
            "unknown",
            "durability_unknown",
            "committed_pending_apply",
        ] {
            assert!(pack_status_result(&operation(state)).is_err());
        }
        assert!(pack_status_result(&operation("completed")).is_ok());
    }

    #[test]
    fn pack_status_accepts_opaque_operation_ids_but_rejects_controls() {
        let cli = parse_cli([
            "pack".to_owned(),
            "status".to_owned(),
            "operation.123".to_owned(),
        ])
        .expect("pack status");
        assert!(matches!(
            cli.pack,
            Some(PackCommand::Status { operation_id }) if operation_id == "operation.123"
        ));
        assert!(parse_cli([
            "pack".to_owned(),
            "status".to_owned(),
            "operation\n123".to_owned(),
        ])
        .is_err());
    }
    #[test]
    fn non_utf8_arguments_are_rejected_without_panicking() {
        use std::os::unix::ffi::OsStringExt;

        let error = decode_args([OsString::from_vec(vec![b'p', 0xff])])
            .expect_err("non-UTF-8 argv must be rejected");
        assert!(error.contains("UTF-8"));
    }

    #[test]
    fn pack_source_paths_reject_nul_and_oversize_values() {
        assert!(absolute_input_path("/tmp/pack\0source").is_err());
        assert!(absolute_input_path(&format!("/{}", "x".repeat(4096))).is_err());
    }

    fn control_reply(ok: bool, registration_accepted: bool, shutdown: bool) -> ControlResponse {
        ControlResponse {
            r#type: "status".to_owned(),
            version: 1,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            ok,
            command: "register".to_owned(),
            error: shutdown.then(|| "desktop pet is stopping".to_owned()),
            ready: false,
            running: true,
            phase: "starting".to_owned(),
            ui_ready: false,
            control_ready: true,
            registration_accepted,
            data_connected: false,
            connected_sources: 0,
            disconnected_sources: 0,
            sessions: 0,
            working: 0,
            blocked: 0,
            done: 0,
            unknown: 0,
            visible: false,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            bubble_placement: bubble::BubblePlacement::Above,
            scale: state::DEFAULT_SCALE,
            shutdown,
            auto_start: true,
            exit_with_herdr: true,
            pid: std::process::id(),
            executable_path: String::new(),
            config_dir: String::new(),
            state_dir: String::new(),
            herdr_socket: String::new(),
        }
    }

    /// Binds a private control socket that answers exactly one request with
    /// `reply`; `vanish` unlinks the socket before answering, as an exiting
    /// daemon does.
    fn serve_one_reply(reply: ControlResponse, vanish: bool) -> (Paths, thread::JoinHandle<()>) {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = PathBuf::from("/tmp").join(format!(
            "herdr-ready-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, Permissions::from_mode(0o700)).unwrap();
        let control_socket = root.join("control.sock");
        let listener = UnixListener::bind(&control_socket).unwrap();
        fs::set_permissions(&control_socket, Permissions::from_mode(0o600)).unwrap();
        let paths = Paths {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            control_socket: control_socket.clone(),
            log_file: root.join("daemon.log"),
        };
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            if vanish {
                fs::remove_file(&control_socket).unwrap();
            }
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).unwrap();
        });
        (paths, server)
    }

    #[test]
    fn readiness_wait_stops_once_the_daemon_reports_shutdown() {
        let (paths, server) = serve_one_reply(control_reply(false, false, true), false);
        let start = Instant::now();
        let result = wait_for_ready(
            &paths,
            Path::new("/tmp/herdr.sock"),
            Instant::now() + STARTUP_TIMEOUT,
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(paths.control_socket.parent().unwrap());
        assert!(
            matches!(result, Err(ReadyFailure::Exiting(_))),
            "{result:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn readiness_wait_stops_once_an_answering_daemon_unlinks_its_socket() {
        let (paths, server) = serve_one_reply(control_reply(true, true, false), true);
        let start = Instant::now();
        let result = wait_for_ready(
            &paths,
            Path::new("/tmp/herdr.sock"),
            Instant::now() + STARTUP_TIMEOUT,
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(paths.control_socket.parent().unwrap());
        assert!(
            matches!(result, Err(ReadyFailure::Exiting(_))),
            "{result:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
