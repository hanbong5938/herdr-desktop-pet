//! Human-facing automation command parsing and operation-aware control client.
use crate::automation::{
    DomainAction, DomainOperation, DomainOperationState, DomainRequest, SessionIdentity,
};
use crate::automation::{OperationState, PresentationOperation, PresentationPatch};
use crate::bubble::BubblePlacement;
use crate::control::{
    self, AutomationRequestEnvelope, PresentationReplyEnvelope, PresentationRequestEnvelope,
    SessionsRequestEnvelope,
};
use crate::dialogue::DialogueSlot;
use crate::dialogue_automation::{
    DialogueBaseline, DialogueIdentity, DialogueLanguage, DialogueSelection,
};
use crate::i18n::LanguagePreference;
use crate::preferences::{
    BubbleAppearance, BubbleColor, BubbleTheme, MenuBarMode, PreferencePatch,
};
use crate::session_view::{SessionFilter, SessionPageCursor, SessionPageRequest};
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) enum PresentationCommand {
    Get,
    Set {
        patch: PresentationPatch,
        expected_revision: Option<u64>,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    Reset {
        expected_revision: Option<u64>,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    Status {
        instance_id: String,
        operation_id: String,
    },
}

const DEFAULT_WAIT: Duration = Duration::from_secs(15);
const MAX_WAIT_SECONDS: f64 = 86_400.0;
const POLL_INTERVAL: Duration = Duration::from_millis(75);

fn valid_id(value: &str, name: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!(
            "{name} must contain 1–128 ASCII letters, digits, '-' or '_'"
        ));
    }
    Ok(value.to_owned())
}

fn on_off(value: &str, name: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{name} must be on or off")),
    }
}

fn option_value<'a>(
    tokens: &'a [String],
    index: &mut usize,
    inline: Option<&'a str>,
    name: &str,
) -> Result<&'a str, String> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(format!("{name} needs a value"));
        }
        return Ok(value);
    }
    *index += 1;
    let value = tokens
        .get(*index)
        .ok_or_else(|| format!("{name} needs a value"))?;
    if value.starts_with("--") {
        return Err(format!("{name} needs a value"));
    }
    Ok(value)
}

fn unique<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("duplicate {name}"));
    }
    Ok(())
}

pub(crate) fn parse(tokens: &[String]) -> Result<PresentationCommand, String> {
    let operation = tokens
        .first()
        .ok_or("presentation requires get, set, reset or status")?;
    if !matches!(operation.as_str(), "get" | "set" | "reset" | "status") {
        return Err(format!("unknown presentation operation {operation:?}"));
    }
    let mut patch = PresentationPatch::default();
    let mut touched = false;
    let mut expected_revision = None;
    let mut operation_id = None;
    let mut instance_id = None;
    let mut wait = None;
    let mut no_wait = false;
    let mut positional = Vec::new();
    let mut index = 1;
    while index < tokens.len() {
        let token = &tokens[index];
        if !token.starts_with("--") {
            positional.push(token.as_str());
            index += 1;
            continue;
        }
        let (name, inline) = match token.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (token.as_str(), None),
        };
        if name == "--no-wait" {
            if inline.is_some() {
                return Err("--no-wait does not take a value".into());
            }
            if no_wait {
                return Err("duplicate --no-wait".into());
            }
            no_wait = true;
            index += 1;
            continue;
        }
        let value = option_value(tokens, &mut index, inline, name)?;
        match name {
            "--visible" => {
                unique(&mut patch.visible, on_off(value, name)?, name)?;
                touched = true;
            }
            "--passthrough" => {
                unique(&mut patch.passthrough, on_off(value, name)?, name)?;
                touched = true;
            }
            "--alpha-passthrough" => {
                unique(&mut patch.alpha_passthrough, on_off(value, name)?, name)?;
                touched = true;
            }
            "--bubble-visible" => {
                unique(&mut patch.bubble_visible, on_off(value, name)?, name)?;
                touched = true;
            }
            "--bubble-placement" => {
                let placement = match value {
                    "above" => BubblePlacement::Above,
                    "below" => BubblePlacement::Below,
                    "left" => BubblePlacement::Left,
                    "right" => BubblePlacement::Right,
                    "auto" => BubblePlacement::Auto,
                    _ => {
                        return Err(
                            "--bubble-placement must be above, below, left, right or auto".into(),
                        )
                    }
                };
                unique(&mut patch.bubble_placement, placement, name)?;
                touched = true;
            }
            "--scale" => {
                let scale = value
                    .parse::<f64>()
                    .map_err(|_| "--scale must be a finite number")?;
                if !scale.is_finite() {
                    return Err("--scale must be a finite number".into());
                }
                unique(&mut patch.scale, scale, name)?;
                touched = true;
            }
            "--expected-revision" => {
                let revision = decimal_u64(value, name)?;
                unique(&mut expected_revision, revision, name)?;
            }
            "--operation-id" => unique(&mut operation_id, valid_id(value, name)?, name)?,
            "--instance" => unique(&mut instance_id, valid_id(value, name)?, name)?,
            "--wait" => {
                let seconds = value
                    .parse::<f64>()
                    .map_err(|_| "--wait must be finite nonnegative seconds")?;
                if !seconds.is_finite() || seconds < 0.0 || seconds > MAX_WAIT_SECONDS {
                    return Err(format!(
                        "--wait must be finite seconds between 0 and {MAX_WAIT_SECONDS}"
                    ));
                }
                unique(&mut wait, Duration::from_secs_f64(seconds), name)?;
            }
            _ => return Err(format!("unknown presentation option {name:?}")),
        }
        index += 1;
    }
    if wait.is_some() && no_wait {
        return Err("--wait and --no-wait cannot be combined".into());
    }
    match operation.as_str() {
        "get" if !touched && expected_revision.is_none() && operation_id.is_none() && instance_id.is_none() && wait.is_none() && !no_wait && positional.is_empty() => Ok(PresentationCommand::Get),
        "set" if touched && instance_id.is_none() && positional.is_empty() => Ok(PresentationCommand::Set {
            patch, expected_revision, operation_id, wait: if no_wait { None } else { Some(wait.unwrap_or(DEFAULT_WAIT)) },
        }),
        "reset" if !touched && instance_id.is_none() && positional.is_empty() => Ok(PresentationCommand::Reset {
            expected_revision, operation_id, wait: if no_wait { None } else { Some(wait.unwrap_or(DEFAULT_WAIT)) },
        }),
        "status" if !touched && expected_revision.is_none() && operation_id.is_none() && wait.is_none() && !no_wait && positional.len() == 1 => Ok(PresentationCommand::Status {
            operation_id: valid_id(positional[0], "operation ID")?,
            instance_id: instance_id.ok_or("presentation status requires --instance ID")?,
        }),
        _ => Err(format!("invalid presentation {operation} arguments (get has no fields; set requires a field; reset takes no fields; status requires OPID --instance ID)")),
    }
}

fn decimal_u64(value: &str, name: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{name} must be an unsigned decimal integer"));
    }
    value.parse().map_err(|_| format!("{name} is out of range"))
}

fn request(
    command: &str,
    instance_id: Option<String>,
    operation_id: Option<String>,
    expected_revision: Option<u64>,
    patch: Option<PresentationPatch>,
) -> PresentationRequestEnvelope {
    PresentationRequestEnvelope {
        version: 1,
        kind: "presentation".to_owned(),
        command: command.to_owned(),
        instance_id,
        operation_id,
        expected_revision,
        patch,
    }
}

fn send(
    socket: &Path,
    request: PresentationRequestEnvelope,
    timeout: Duration,
) -> Result<PresentationReplyEnvelope, String> {
    control::send_presentation_request(socket, request, timeout)
}

fn print_operation(
    operation: &PresentationOperation,
    ok: bool,
    deadline_exceeded: bool,
) -> Result<(), String> {
    super::print_json(&serde_json::json!({
        "ok": ok,
        "instance_id": operation.instance_id,
        "operation": operation,
        "deadline_exceeded": deadline_exceeded,
    }))
}

fn result(operation: &PresentationOperation) -> Result<(), String> {
    if operation.state == OperationState::Applied {
        Ok(())
    } else {
        Err(operation.error.clone().unwrap_or_else(|| {
            format!(
                "presentation operation {} is {:?}",
                operation.operation_id, operation.state
            )
        }))
    }
}

fn operation_reply(
    reply: PresentationReplyEnvelope,
    instance: &str,
    operation_id: &str,
) -> Result<PresentationOperation, String> {
    if let Some(operation) = reply.operation {
        if operation.instance_id != instance
            || operation.operation_id != operation_id
            || reply.instance_id.as_deref() != Some(instance)
        {
            return Err("presentation reply has a different operation identity".into());
        }
        if !reply.ok {
            print_operation(&operation, false, false)?;
            return Err(operation
                .error
                .unwrap_or_else(|| "presentation request failed".into()));
        }
        return Ok(operation);
    }
    let error = reply
        .error
        .unwrap_or_else(|| "presentation operation was not accepted".into());
    super::print_json(&serde_json::json!({
        "ok": false, "instance_id": instance, "operation_id": operation_id,
        "state": "rejected", "error": error,
    }))?;
    Err(error)
}

fn uncertain(instance: &str, operation_id: &str, error: String) -> Result<(), String> {
    let message = format!("presentation outcome unknown; query presentation status {operation_id} --instance {instance}: {error}");
    super::print_json(&serde_json::json!({
        "ok": false, "instance_id": instance, "operation_id": operation_id,
        "state": "unknown", "error": message,
    }))?;
    Err(message)
}

pub(crate) fn execute(command: PresentationCommand, socket: &Path) -> Result<(), String> {
    match command {
        PresentationCommand::Get => {
            let reply = send(
                socket,
                request("get", None, None, None, None),
                super::CONTROL_TIMEOUT,
            )?;
            if !reply.ok {
                return Err(reply
                    .error
                    .unwrap_or_else(|| "presentation get failed".into()));
            }
            let snapshot = reply.snapshot.ok_or("presentation snapshot unavailable")?;
            if reply.instance_id.as_deref() != Some(snapshot.instance_id.as_str()) {
                return Err("presentation snapshot has a different instance identity".into());
            }
            super::print_json(
                &serde_json::json!({ "ok": true, "instance_id": snapshot.instance_id, "snapshot": snapshot }),
            )
        }
        PresentationCommand::Status {
            instance_id,
            operation_id,
        } => {
            let reply = send(
                socket,
                request(
                    "status",
                    Some(instance_id.clone()),
                    Some(operation_id.clone()),
                    None,
                    None,
                ),
                super::CONTROL_TIMEOUT,
            )?;
            let operation = operation_reply(reply, &instance_id, &operation_id)?;
            print_operation(
                &operation,
                operation.state == OperationState::Applied,
                false,
            )?;
            result(&operation)
        }
        PresentationCommand::Set {
            patch,
            expected_revision,
            operation_id,
            wait,
        } => mutate(
            socket,
            "set",
            Some(patch),
            expected_revision,
            operation_id,
            wait,
        ),
        PresentationCommand::Reset {
            expected_revision,
            operation_id,
            wait,
        } => mutate(socket, "reset", None, expected_revision, operation_id, wait),
    }
}

fn mutate(
    socket: &Path,
    command: &str,
    patch: Option<PresentationPatch>,
    expected_revision: Option<u64>,
    operation_id: Option<String>,
    wait: Option<Duration>,
) -> Result<(), String> {
    // Identity is created before the first transmission so even a disconnected ACK can
    // be investigated without sending the mutation a second time.
    let operation_id = operation_id.unwrap_or_else(super::new_operation_id);
    let get = send(
        socket,
        request("get", None, None, None, None),
        super::CONTROL_TIMEOUT,
    )?;
    if !get.ok {
        return Err(get
            .error
            .unwrap_or_else(|| "presentation get failed".into()));
    }
    let snapshot = get.snapshot.ok_or("presentation snapshot unavailable")?;
    let instance = snapshot.instance_id;
    if get.instance_id.as_deref() != Some(instance.as_str()) {
        return Err("presentation snapshot has a different instance identity".into());
    }
    let submission = request(
        command,
        Some(instance.clone()),
        Some(operation_id.clone()),
        expected_revision,
        patch,
    );
    // The control transport has its own two-second deadline. Waiting for
    // application begins only after the submission reply is received.
    let reply = match send(socket, submission, super::CONTROL_TIMEOUT) {
        Ok(reply) => reply,
        Err(error) => return uncertain(&instance, &operation_id, error),
    };
    let deadline = wait.map(|duration| Instant::now() + duration);
    let mut operation = match operation_reply(reply, &instance, &operation_id) {
        Err(error) if error == "presentation reply has a different operation identity" => {
            return uncertain(&instance, &operation_id, error)
        }
        other => other?,
    };
    if wait.is_none()
        && matches!(
            operation.state,
            OperationState::Accepted | OperationState::Pending
        )
    {
        print_operation(&operation, true, false)?;
        return Ok(()); // ACK means accepted, not applied.
    }
    if !matches!(
        operation.state,
        OperationState::Accepted | OperationState::Pending
    ) {
        print_operation(
            &operation,
            operation.state == OperationState::Applied,
            false,
        )?;
        return result(&operation);
    }
    let deadline = deadline.expect("wait checked above");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            print_operation(&operation, false, true)?;
            return Err(format!("presentation operation {operation_id} remains pending; query presentation status {operation_id} --instance {instance}"));
        }
        thread::sleep(POLL_INTERVAL.min(remaining));
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            continue;
        }
        let reply = match send(
            socket,
            request(
                "status",
                Some(instance.clone()),
                Some(operation_id.clone()),
                None,
                None,
            ),
            super::CONTROL_TIMEOUT.min(remaining),
        ) {
            Ok(reply) => reply,
            Err(error) => return uncertain(&instance, &operation_id, error),
        };
        operation = match operation_reply(reply, &instance, &operation_id) {
            Err(error) if error == "presentation reply has a different operation identity" => {
                return uncertain(&instance, &operation_id, error)
            }
            other => other?,
        };
        if !matches!(
            operation.state,
            OperationState::Accepted | OperationState::Pending
        ) {
            print_operation(
                &operation,
                operation.state == OperationState::Applied,
                false,
            )?;
            return result(&operation);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DomainFamily {
    Preferences,
    Sessions,
    Dialogue,
    Worktree,
}

impl DomainFamily {
    fn name(self) -> &'static str {
        match self {
            Self::Preferences => "preferences",
            Self::Sessions => "sessions",
            Self::Dialogue => "dialogue",
            Self::Worktree => "worktree",
        }
    }

    fn contains(self, kind: &str) -> bool {
        match self {
            Self::Preferences => matches!(kind, "preferences_get" | "preferences_set"),
            Self::Sessions => kind == "session_prompt",
            Self::Dialogue => matches!(
                kind,
                "dialogue_list"
                    | "dialogue_read"
                    | "dialogue_set"
                    | "dialogue_reset_entry"
                    | "dialogue_reset_character"
            ),
            Self::Worktree => matches!(kind, "worktree_inspect" | "worktree_remove"),
        }
    }
}

#[derive(Debug)]
pub(crate) enum DomainCommand {
    PreferencesGet,
    PreferencesSet {
        patch: PreferencePatch,
        revision: Option<u64>,
        operation_id: Option<String>,
        wait: Option<Duration>,
        preserve_custom: bool,
        color_flags: [bool; 5],
    },
    Status {
        family: DomainFamily,
        instance: String,
        operation_id: String,
    },
    SessionsList {
        instance: String,
        filter: SessionFilter,
        limit: usize,
    },
    SessionsShow(SessionIdentity),
    SessionsPrompt {
        identity: SessionIdentity,
        text: PromptText,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    DialogueList,
    DialogueRead(DialogueSelection),
    DialogueSet {
        selection: DialogueSelection,
        baseline: Option<DialogueBaseline>,
        text: PromptText,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    DialogueResetEntry {
        selection: DialogueSelection,
        baseline: Option<DialogueBaseline>,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    DialogueResetCharacter {
        identity: DialogueIdentity,
        baseline: Option<DialogueBaseline>,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
    WorktreeInspect(SessionIdentity),
    WorktreeRemove {
        token: String,
        operation_id: Option<String>,
        wait: Option<Duration>,
    },
}

#[derive(Debug)]
pub(crate) enum PromptText {
    Literal(String),
    File(PathBuf),
    Stdin,
}

fn wait_seconds(value: &str) -> Result<Duration, String> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| "--wait requires positive finite seconds".to_owned())?;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_WAIT_SECONDS {
        return Err("--wait requires positive finite seconds (at most 86400)".into());
    }
    Ok(Duration::from_secs_f64(seconds))
}
pub(crate) fn parse_wait(value: &str) -> Result<Duration, String> {
    wait_seconds(value)
}

pub(crate) fn parse_domain(preferences: bool, tokens: &[String]) -> Result<DomainCommand, String> {
    let operation = tokens.first().ok_or("operation is required")?.as_str();
    let mut fields = std::collections::HashMap::<&str, Vec<String>>::new();
    let mut index = 1;
    while index < tokens.len() {
        let (name, inline) = tokens[index]
            .split_once('=')
            .map_or((tokens[index].as_str(), None), |(a, b)| (a, Some(b)));
        let known = if preferences {
            matches!(
                name,
                "--language"
                    | "--theme"
                    | "--surface"
                    | "--text"
                    | "--muted"
                    | "--border"
                    | "--accent"
                    | "--status-indicators"
                    | "--menu-bar"
                    | "--observation-local"
                    | "--observation-remote"
                    | "--machine"
                    | "--clear-machines"
                    | "--expected-revision"
                    | "--operation-id"
                    | "--instance"
                    | "--wait"
                    | "--no-wait"
            )
        } else {
            matches!(
                name,
                "--instance"
                    | "--source"
                    | "--generation"
                    | "--terminal"
                    | "--filter"
                    | "--limit"
                    | "--text"
                    | "--file"
                    | "--stdin"
                    | "--operation-id"
                    | "--wait"
                    | "--no-wait"
            )
        };
        if !known {
            return Err(format!("unknown option {:?}", tokens[index]));
        }
        let value = if name == "--text" && !preferences {
            if let Some(value) = inline {
                value.to_owned()
            } else {
                index += 1;
                tokens.get(index).ok_or("--text requires a value")?.clone()
            }
        } else if matches!(name, "--stdin" | "--no-wait" | "--clear-machines") {
            if inline.is_some() {
                return Err(format!("{name} does not take a value"));
            }
            String::new()
        } else {
            option_value(tokens, &mut index, inline, name)?.to_owned()
        };
        if name != "--machine" && fields.contains_key(name) {
            return Err(format!("duplicate {name}"));
        }
        fields.entry(name).or_default().push(value);
        index += 1;
    }
    let take = |name: &str| fields.get(name).and_then(|v| v.first()).map(String::as_str);
    let number = |name| take(name).map(|v| decimal_u64(v, name)).transpose();
    let wait = if take("--no-wait").is_some() {
        if take("--wait").is_some() {
            return Err("--wait conflicts with --no-wait".into());
        }
        None
    } else {
        Some(
            take("--wait")
                .map(wait_seconds)
                .transpose()?
                .unwrap_or(DEFAULT_WAIT),
        )
    };
    let id = take("--operation-id")
        .map(|v| valid_id(v, "--operation-id"))
        .transpose()?;
    let instance = || {
        take("--instance")
            .ok_or_else(|| "--instance is required".to_owned())
            .and_then(|v| valid_id(v, "--instance"))
    };
    let identity = || -> Result<SessionIdentity, String> {
        Ok(SessionIdentity {
            instance_id: instance()?,
            source_id: number("--source")?.ok_or("--source is required")?,
            generation: number("--generation")?.ok_or("--generation is required")?,
            terminal_id: take("--terminal")
                .filter(|v| !v.is_empty())
                .ok_or("--terminal is required")?
                .to_owned(),
        })
    };
    if preferences {
        match operation {
            "get" if fields.is_empty() => Ok(DomainCommand::PreferencesGet),
            "status" if fields.len() == 2 && take("--instance").is_some() && id.is_some() => {
                Ok(DomainCommand::Status {
                    family: DomainFamily::Preferences,
                    instance: instance()?,
                    operation_id: id.unwrap(),
                })
            }
            "set" => {
                if take("--instance").is_some() {
                    return Err("--instance is not valid for preferences set".into());
                }
                let mut patch = PreferencePatch::default();
                if let Some(v) = take("--language") {
                    patch.language = Some(match v {
                        "system" => LanguagePreference::System,
                        "ko" => LanguagePreference::Ko,
                        "en" => LanguagePreference::En,
                        _ => return Err("unknown language (expected system|ko|en)".into()),
                    });
                }
                let colors = ["--surface", "--text", "--muted", "--border", "--accent"];
                let appearance =
                    take("--theme").is_some() || colors.iter().any(|key| take(key).is_some());
                if appearance {
                    // A partial palette is merged with a revision-bound saved snapshot before submission.
                    let theme = take("--theme")
                        .map(|v| {
                            serde_json::from_value(serde_json::json!(v))
                                .map_err(|_| "unknown theme")
                        })
                        .transpose()?
                        .unwrap_or(BubbleTheme::Custom);
                    let color = |name| take(name).map(BubbleColor::parse_hex).transpose();
                    let mut appearance_value = BubbleAppearance::default();
                    appearance_value.theme = theme;
                    if let Some(value) = color("--surface")? {
                        appearance_value.custom.surface = value;
                    }
                    if let Some(value) = color("--text")? {
                        appearance_value.custom.text = value;
                    }
                    if let Some(value) = color("--muted")? {
                        appearance_value.custom.muted = value;
                    }
                    if let Some(value) = color("--border")? {
                        appearance_value.custom.border = value;
                    }
                    if let Some(value) = color("--accent")? {
                        appearance_value.custom.accent = value;
                    }
                    patch.bubble_appearance = Some(appearance_value);
                }
                patch.show_status_indicators = take("--status-indicators")
                    .map(|v| on_off(v, "--status-indicators"))
                    .transpose()?;
                patch.menu_bar_mode = take("--menu-bar")
                    .map(|v| {
                        serde_json::from_value::<MenuBarMode>(serde_json::json!(v))
                            .map_err(|_| "unknown menu-bar mode")
                    })
                    .transpose()?;
                patch.observation_local = take("--observation-local")
                    .map(|v| on_off(v, "--observation-local"))
                    .transpose()?;
                patch.observation_remote = take("--observation-remote")
                    .map(|v| on_off(v, "--observation-remote"))
                    .transpose()?;
                if take("--clear-machines").is_some() && fields.contains_key("--machine") {
                    return Err("--clear-machines conflicts with --machine".into());
                }
                if take("--clear-machines").is_some() {
                    patch.observation_machines = Some(Vec::new());
                }
                if let Some(machines) = fields.get("--machine") {
                    patch.observation_machines = Some(machines.clone());
                }
                if !appearance
                    && patch.language.is_none()
                    && patch.show_status_indicators.is_none()
                    && patch.menu_bar_mode.is_none()
                    && patch.observation_local.is_none()
                    && patch.observation_remote.is_none()
                    && patch.observation_machines.is_none()
                {
                    return Err("preferences set requires a setting".into());
                }
                let color_flags = colors.map(|key| take(key).is_some());
                Ok(DomainCommand::PreferencesSet {
                    patch,
                    revision: number("--expected-revision")?,
                    operation_id: id,
                    wait,
                    preserve_custom: appearance && !color_flags.iter().all(|selected| *selected),
                    color_flags,
                })
            }
            _ => Err("invalid preferences operation or options".into()),
        }
    } else {
        match operation {
            "list"
                if fields
                    .keys()
                    .all(|k| matches!(*k, "--instance" | "--filter" | "--limit")) =>
            {
                let filter = take("--filter")
                    .map(|v| {
                        serde_json::from_value(serde_json::json!(v))
                            .map_err(|_| "unknown session filter")
                    })
                    .transpose()?
                    .unwrap_or_default();
                let limit = number("--limit")?.unwrap_or(32);
                if !(1..=128).contains(&limit) {
                    return Err("--limit must be 1..128".into());
                }
                Ok(DomainCommand::SessionsList {
                    instance: instance()?,
                    filter,
                    limit: limit as usize,
                })
            }
            "show"
                if fields.keys().all(|k| {
                    matches!(
                        *k,
                        "--instance" | "--source" | "--generation" | "--terminal"
                    )
                }) =>
            {
                Ok(DomainCommand::SessionsShow(identity()?))
            }
            "prompt"
                if fields.keys().all(|k| {
                    matches!(
                        *k,
                        "--instance"
                            | "--source"
                            | "--generation"
                            | "--terminal"
                            | "--text"
                            | "--file"
                            | "--stdin"
                            | "--operation-id"
                            | "--wait"
                            | "--no-wait"
                    )
                }) =>
            {
                let sources = [
                    take("--text").is_some(),
                    take("--file").is_some(),
                    take("--stdin").is_some(),
                ];
                if sources.iter().filter(|selected| **selected).count() != 1 {
                    return Err("choose exactly one of --text, --file, --stdin".into());
                }
                let text = if let Some(value) = take("--text") {
                    PromptText::Literal(value.to_owned())
                } else if let Some(value) = take("--file") {
                    PromptText::File(PathBuf::from(value))
                } else {
                    PromptText::Stdin
                };
                Ok(DomainCommand::SessionsPrompt {
                    identity: identity()?,
                    text,
                    operation_id: id,
                    wait,
                })
            }
            "status" if fields.len() == 2 && take("--instance").is_some() && id.is_some() => {
                Ok(DomainCommand::Status {
                    family: DomainFamily::Sessions,
                    instance: instance()?,
                    operation_id: id.unwrap(),
                })
            }
            _ => Err("invalid sessions operation or options".into()),
        }
    }
}

pub(crate) fn parse_family(family: &str, tokens: &[String]) -> Result<DomainCommand, String> {
    match family {
        "preferences" => return parse_domain(true, tokens),
        "sessions" => return parse_domain(false, tokens),
        "dialogue" | "worktree" => {}
        _ => return Err("unknown automation command".into()),
    }
    let operation = tokens.first().ok_or("operation is required")?.as_str();
    let mut fields = std::collections::HashMap::<&str, &str>::new();
    let mut index = 1;
    while index < tokens.len() {
        let (name, inline) = tokens[index]
            .split_once('=')
            .map_or((tokens[index].as_str(), None), |(a, b)| (a, Some(b)));
        let known = if family == "dialogue" {
            matches!(
                name,
                "--target"
                    | "--locale"
                    | "--slot"
                    | "--baseline"
                    | "--text"
                    | "--file"
                    | "--stdin"
                    | "--instance"
                    | "--operation-id"
                    | "--wait"
                    | "--no-wait"
            )
        } else {
            matches!(
                name,
                "--instance"
                    | "--source"
                    | "--generation"
                    | "--terminal"
                    | "--token"
                    | "--operation-id"
                    | "--wait"
                    | "--no-wait"
            )
        };
        if !known {
            return Err(format!("unknown option {:?}", tokens[index]));
        }
        let value = if matches!(name, "--stdin" | "--no-wait") {
            if inline.is_some() {
                return Err(format!("{name} does not take a value"));
            }
            ""
        } else if name == "--text" {
            if let Some(value) = inline {
                value
            } else {
                index += 1;
                tokens.get(index).ok_or("--text requires a value")?.as_str()
            }
        } else {
            option_value(tokens, &mut index, inline, name)?
        };
        if fields.insert(name, value).is_some() {
            return Err(format!("duplicate {name}"));
        }
        index += 1;
    }
    let take = |name: &str| fields.get(name).copied();
    let only = |allowed: &[&str]| fields.keys().all(|key| allowed.contains(key));
    let id = take("--operation-id")
        .map(|v| valid_id(v, "--operation-id"))
        .transpose()?;
    let instance = || {
        take("--instance")
            .ok_or_else(|| "--instance is required".to_owned())
            .and_then(|value| valid_id(value, "--instance"))
    };
    let status = |family| {
        if fields.len() != 2 || take("--instance").is_none() || id.is_none() {
            return Err("status requires --instance ID --operation-id ID".into());
        }
        Ok(DomainCommand::Status {
            family,
            instance: instance()?,
            operation_id: id.clone().unwrap(),
        })
    };
    if operation == "status" {
        return status(if family == "dialogue" {
            DomainFamily::Dialogue
        } else {
            DomainFamily::Worktree
        });
    }
    let wait = || -> Result<Option<Duration>, String> {
        if take("--no-wait").is_some() && take("--wait").is_some() {
            return Err("--wait conflicts with --no-wait".into());
        }
        if take("--no-wait").is_some() {
            Ok(None)
        } else {
            Ok(Some(
                take("--wait")
                    .map(wait_seconds)
                    .transpose()?
                    .unwrap_or(DEFAULT_WAIT),
            ))
        }
    };
    if family == "worktree" {
        return match operation {
            "inspect" if only(&["--instance", "--source", "--generation", "--terminal"]) => {
                Ok(DomainCommand::WorktreeInspect(SessionIdentity {
                    instance_id: instance()?,
                    source_id: take("--source")
                        .ok_or_else(|| "--source is required".to_owned())
                        .and_then(|v| decimal_u64(v, "--source"))?,
                    generation: take("--generation")
                        .ok_or_else(|| "--generation is required".to_owned())
                        .and_then(|v| decimal_u64(v, "--generation"))?,
                    terminal_id: take("--terminal")
                        .filter(|v| !v.is_empty())
                        .ok_or("--terminal is required")?
                        .into(),
                }))
            }
            "remove" if only(&["--token", "--operation-id", "--wait", "--no-wait"]) => {
                let token = take("--token")
                    .filter(|v| !v.is_empty() && v.len() <= 4096)
                    .ok_or("--token is required")?
                    .to_owned();
                Ok(DomainCommand::WorktreeRemove {
                    token,
                    operation_id: id,
                    wait: wait()?,
                })
            }
            _ => Err("invalid worktree operation or options".into()),
        };
    }
    if operation == "list" && fields.is_empty() {
        return Ok(DomainCommand::DialogueList);
    }
    let identity = || -> Result<DialogueIdentity, String> {
        serde_json::from_str(take("--target").ok_or("--target JSON identity is required")?)
            .map_err(|error| format!("invalid --target JSON identity: {error}"))
    };
    let selection = || -> Result<DialogueSelection, String> {
        let locale = match take("--locale") {
            Some("ko") => DialogueLanguage::Ko,
            Some("en") => DialogueLanguage::En,
            _ => return Err("--locale must be ko or en".into()),
        };
        let slot: DialogueSlot = serde_json::from_value(serde_json::json!(
            take("--slot").ok_or("--slot is required")?
        ))
        .map_err(|_| "unknown dialogue slot")?;
        Ok(DialogueSelection {
            identity: identity()?,
            locale,
            slot,
        })
    };
    let baseline = || {
        take("--baseline")
            .map(|v| {
                serde_json::from_str::<DialogueBaseline>(v)
                    .map_err(|error| format!("invalid --baseline JSON: {error}"))
            })
            .transpose()
    };
    let source = || -> Result<PromptText, String> {
        if [take("--text"), take("--file"), take("--stdin")]
            .iter()
            .filter(|v| v.is_some())
            .count()
            != 1
        {
            return Err("choose exactly one of --text, --file, --stdin".into());
        }
        Ok(if let Some(value) = take("--text") {
            PromptText::Literal(value.into())
        } else if let Some(value) = take("--file") {
            PromptText::File(value.into())
        } else {
            PromptText::Stdin
        })
    };
    match operation {
        "get" if only(&["--target", "--locale", "--slot"]) => {
            Ok(DomainCommand::DialogueRead(selection()?))
        }
        "set"
            if only(&[
                "--target",
                "--locale",
                "--slot",
                "--baseline",
                "--text",
                "--file",
                "--stdin",
                "--operation-id",
                "--wait",
                "--no-wait",
            ]) =>
        {
            Ok(DomainCommand::DialogueSet {
                selection: selection()?,
                baseline: baseline()?,
                text: source()?,
                operation_id: id,
                wait: wait()?,
            })
        }
        "reset-entry"
            if only(&[
                "--target",
                "--locale",
                "--slot",
                "--baseline",
                "--operation-id",
                "--wait",
                "--no-wait",
            ]) =>
        {
            Ok(DomainCommand::DialogueResetEntry {
                selection: selection()?,
                baseline: baseline()?,
                operation_id: id,
                wait: wait()?,
            })
        }
        "reset-character"
            if only(&[
                "--target",
                "--baseline",
                "--operation-id",
                "--wait",
                "--no-wait",
            ]) =>
        {
            Ok(DomainCommand::DialogueResetCharacter {
                identity: identity()?,
                baseline: baseline()?,
                operation_id: id,
                wait: wait()?,
            })
        }
        _ => Err("invalid dialogue operation or options".into()),
    }
}

fn automation(
    socket: &Path,
    command: &str,
    request: Option<DomainRequest>,
    instance_id: Option<String>,
    operation_id: Option<String>,
) -> Result<crate::control::AutomationReplyEnvelope, String> {
    let kind = if matches!(
        request.as_ref().map(|r| &r.action),
        Some(DomainAction::SessionPrompt { .. })
    ) {
        "prompt"
    } else {
        "automation"
    };
    control::send_automation_request(
        socket,
        &AutomationRequestEnvelope {
            version: 1,
            kind: kind.into(),
            command: command.into(),
            request,
            instance_id,
            operation_id,
        },
    )
}

fn automation_status_until(
    socket: &Path,
    instance: &str,
    id: &str,
    deadline: Instant,
) -> Result<crate::control::AutomationReplyEnvelope, control::ClientDeadlineError> {
    control::send_automation_request_until(
        socket,
        &AutomationRequestEnvelope {
            version: 1,
            kind: "automation".into(),
            command: "status".into(),
            request: None,
            instance_id: Some(instance.to_owned()),
            operation_id: Some(id.to_owned()),
        },
        deadline,
    )
}

fn poll_deadline(overall: Instant) -> Instant {
    overall.min(Instant::now() + super::CONTROL_TIMEOUT)
}

fn domain_wait_expired(
    family: DomainFamily,
    instance: &str,
    id: &str,
    operation: &DomainOperation,
) -> Result<(), String> {
    super::print_json(
        &serde_json::json!({"ok":false,"deadline_exceeded":true,"operation":operation}),
    )?;
    Err(format!(
        "operation {id} remains pending; query {} status --instance {instance} --operation-id {id}",
        family.name()
    ))
}

fn domain_reply(
    reply: crate::control::AutomationReplyEnvelope,
    instance: &str,
    id: &str,
) -> Result<DomainOperation, String> {
    if !reply.ok && reply.operation.is_none() {
        return Err(reply
            .error
            .unwrap_or_else(|| "automation request rejected".into()));
    }
    if reply.instance_id.as_deref() != Some(instance) {
        return Err("daemon instance changed; operation outcome uncertain".into());
    }
    let operation = reply.operation.ok_or_else(|| {
        format!(
            "operation outcome uncertain: {}",
            reply
                .error
                .unwrap_or_else(|| "operation unavailable".into())
        )
    })?;
    if operation.instance_id != instance || operation.operation_id != id {
        return Err("operation identity changed; outcome uncertain".into());
    }
    Ok(operation)
}

fn action_family(action: &DomainAction) -> (DomainFamily, &'static str) {
    match action {
        DomainAction::PreferencesGet {} => (DomainFamily::Preferences, "preferences_get"),
        DomainAction::PreferencesSet { .. } => (DomainFamily::Preferences, "preferences_set"),
        DomainAction::SessionPrompt { .. } => (DomainFamily::Sessions, "session_prompt"),
        DomainAction::DialogueList {} => (DomainFamily::Dialogue, "dialogue_list"),
        DomainAction::DialogueRead { .. } => (DomainFamily::Dialogue, "dialogue_read"),
        DomainAction::DialogueSet { .. } => (DomainFamily::Dialogue, "dialogue_set"),
        DomainAction::DialogueResetEntry { .. } => (DomainFamily::Dialogue, "dialogue_reset_entry"),
        DomainAction::DialogueResetCharacter { .. } => {
            (DomainFamily::Dialogue, "dialogue_reset_character")
        }
        DomainAction::WorktreeInspect { .. } => (DomainFamily::Worktree, "worktree_inspect"),
        DomainAction::WorktreeRemove { .. } => (DomainFamily::Worktree, "worktree_remove"),
    }
}

fn ensure_kind(
    operation: DomainOperation,
    family: DomainFamily,
    kind: &str,
) -> Result<DomainOperation, String> {
    if operation.kind != kind || !family.contains(&operation.kind) {
        return Err("operation kind changed; outcome uncertain".into());
    }
    Ok(operation)
}

fn domain_uncertain(
    family: DomainFamily,
    instance: &str,
    id: &str,
    error: String,
) -> Result<(), String> {
    let message = format!("operation outcome unknown; query {} status --instance {instance} --operation-id {id}; an expired/restarted status cannot prove the mutation never executed: {error}", family.name());
    super::print_json(&serde_json::json!({
        "ok":false,"instance_id":instance,"operation_id":id,"state":"unknown","error":message,
    }))?;
    Err(message)
}

fn domain_mutation(
    socket: &Path,
    instance: String,
    action: DomainAction,
    operation_id: Option<String>,
    wait: Option<Duration>,
) -> Result<(), String> {
    let (family, expected_kind) = action_family(&action);
    let id = operation_id.unwrap_or_else(super::new_operation_id);
    let reply = automation(
        socket,
        "request",
        Some(DomainRequest {
            instance_id: instance.clone(),
            operation_id: id.clone(),
            action,
        }),
        None,
        None,
    );
    let mut operation = match reply {
        Ok(reply) => match domain_reply(reply, &instance, &id)
            .and_then(|operation| ensure_kind(operation, family, expected_kind))
        {
            Ok(operation) => operation,
            Err(error) if error.contains("outcome uncertain") => {
                return domain_uncertain(family, &instance, &id, error)
            }
            Err(error) => return Err(error),
        },
        Err(error) => return domain_uncertain(family, &instance, &id, error),
    };
    let deadline = wait.map(|duration| Instant::now() + duration);
    loop {
        if operation.state.terminal() || deadline.is_none() {
            super::print_json(
                &serde_json::json!({"ok": !operation.state.terminal() || matches!(operation.state,DomainOperationState::Applied|DomainOperationState::AgentPrompted), "operation":operation}),
            )?;
            return if matches!(
                operation.state,
                DomainOperationState::Failed
                    | DomainOperationState::UnknownDelivery
                    | DomainOperationState::Superseded
                    | DomainOperationState::Shutdown
            ) {
                Err(operation
                    .error
                    .unwrap_or_else(|| format!("operation {} ended in {:?}", id, operation.state)))
            } else {
                Ok(())
            };
        }
        let overall = deadline.expect("wait checked above");
        let remaining = overall.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return domain_wait_expired(family, &instance, &id, &operation);
        }
        thread::sleep(POLL_INTERVAL.min(remaining));
        if Instant::now() >= overall {
            return domain_wait_expired(family, &instance, &id, &operation);
        }
        let reply = match automation_status_until(socket, &instance, &id, poll_deadline(overall)) {
            Ok(reply) => reply,
            Err(control::ClientDeadlineError::Elapsed) if Instant::now() >= overall => {
                return domain_wait_expired(family, &instance, &id, &operation);
            }
            Err(error) => return domain_uncertain(family, &instance, &id, error.to_string()),
        };
        if Instant::now() >= overall {
            return domain_wait_expired(family, &instance, &id, &operation);
        }
        operation = match domain_reply(reply, &instance, &id)
            .and_then(|operation| ensure_kind(operation, family, expected_kind))
        {
            Ok(operation) => operation,
            Err(error) => return domain_uncertain(family, &instance, &id, error),
        };
    }
}

fn read_prompt(text: PromptText) -> Result<String, String> {
    read_text(text, 512 * 1024 - 4096, "prompt")
}

fn read_text(text: PromptText, max_text: usize, label: &str) -> Result<String, String> {
    let bytes = match text {
        PromptText::Literal(value) => value.into_bytes(),
        PromptText::File(path) => {
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("cannot read {label} file: {error}"))?;
            let mut bytes = Vec::new();
            file.take((max_text + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("cannot read {label} file: {error}"))?;
            bytes
        }
        PromptText::Stdin => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take((max_text + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            bytes
        }
    };
    if bytes.len() > max_text {
        return Err(format!("{label} exceeds {max_text} UTF-8 bytes"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{label} must be UTF-8"))
}

fn daemon_instance(socket: &Path) -> Result<String, String> {
    control::send_presentation_request(
        socket,
        request("get", None, None, None, None),
        super::CONTROL_TIMEOUT,
    )?
    .instance_id
    .ok_or_else(|| "daemon instance unavailable".into())
}

fn domain_read(
    socket: &Path,
    instance: &str,
    action: DomainAction,
) -> Result<serde_json::Value, String> {
    domain_read_with_wait(socket, instance, action, DEFAULT_WAIT)
}

fn domain_read_with_wait(
    socket: &Path,
    instance: &str,
    action: DomainAction,
    wait: Duration,
) -> Result<serde_json::Value, String> {
    let (family, expected_kind) = action_family(&action);
    let id = super::new_operation_id();
    let reply = automation(
        socket,
        "request",
        Some(DomainRequest {
            instance_id: instance.to_owned(),
            operation_id: id.clone(),
            action,
        }),
        None,
        None,
    )?;
    let mut operation = ensure_kind(domain_reply(reply, instance, &id)?, family, expected_kind)?;
    let deadline = Instant::now() + wait;
    while !operation.state.terminal() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("{} read remains pending (last state {:?}); query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()));
        }
        thread::sleep(POLL_INTERVAL.min(remaining));
        if Instant::now() >= deadline {
            return Err(format!("{} read remains pending (last state {:?}); query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()));
        }
        let reply = match automation_status_until(socket, instance, &id, poll_deadline(deadline)) {
            Ok(reply) => reply,
            Err(control::ClientDeadlineError::Elapsed) if Instant::now() >= deadline => {
                return Err(format!("{} read remains pending (last state {:?}); query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()));
            }
            Err(error) => {
                return Err(format!("{} read status uncertain (last state {:?}): {error}; query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()));
            }
        };
        if Instant::now() >= deadline {
            return Err(format!("{} read remains pending (last state {:?}); query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()));
        }
        operation = domain_reply(reply, instance, &id)
            .and_then(|latest| ensure_kind(latest, family, expected_kind))
            .map_err(|error| format!("{} read status uncertain (last state {:?}): {error}; query {} status --instance {instance} --operation-id {id}", family.name(), operation.state, family.name()))?;
    }
    if operation.state != DomainOperationState::Applied {
        return Err(operation
            .error
            .unwrap_or_else(|| format!("{} read ended in {:?}", family.name(), operation.state)));
    }
    operation
        .result
        .ok_or_else(|| format!("{} read returned no result", family.name()))
}

fn dialogue_baseline(
    socket: &Path,
    instance: &str,
    selection: DialogueSelection,
) -> Result<DialogueBaseline, String> {
    let value = domain_read(socket, instance, DomainAction::DialogueRead { selection })?;
    serde_json::from_value(
        value
            .get("baseline")
            .cloned()
            .ok_or("dialogue read baseline unavailable")?,
    )
    .map_err(|_| "dialogue read returned invalid baseline".into())
}

pub(crate) fn execute_domain(command: DomainCommand, socket: &Path) -> Result<(), String> {
    match command {
        DomainCommand::PreferencesGet => {
            let reply = control::send_presentation_request(
                socket,
                request("get", None, None, None, None),
                super::CONTROL_TIMEOUT,
            )?;
            let instance = reply.instance_id.ok_or("daemon instance unavailable")?;
            domain_mutation(
                socket,
                instance,
                DomainAction::PreferencesGet {},
                None,
                Some(DEFAULT_WAIT),
            )
        }
        DomainCommand::PreferencesSet {
            mut patch,
            mut revision,
            operation_id,
            wait,
            preserve_custom,
            color_flags,
        } => {
            let reply = control::send_presentation_request(
                socket,
                request("get", None, None, None, None),
                super::CONTROL_TIMEOUT,
            )?;
            let instance = reply.instance_id.ok_or("daemon instance unavailable")?;
            if preserve_custom {
                let id = super::new_operation_id();
                let request = DomainRequest {
                    instance_id: instance.clone(),
                    operation_id: id.clone(),
                    action: DomainAction::PreferencesGet {},
                };
                let reply = automation(socket, "request", Some(request), None, None)?;
                let mut operation = domain_reply(reply, &instance, &id)?;
                let deadline = Instant::now() + DEFAULT_WAIT;
                while !operation.state.terminal() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(format!("preferences snapshot timed out before mutation (last state {:?}); query preferences status --instance {instance} --operation-id {id}", operation.state));
                    }
                    thread::sleep(POLL_INTERVAL.min(remaining));
                    if Instant::now() >= deadline {
                        return Err(format!("preferences snapshot timed out before mutation (last state {:?}); query preferences status --instance {instance} --operation-id {id}", operation.state));
                    }
                    let reply =
                        automation_status_until(socket, &instance, &id, poll_deadline(deadline));
                    if Instant::now() >= deadline {
                        return Err(format!("preferences snapshot timed out before mutation (last state {:?}); query preferences status --instance {instance} --operation-id {id}", operation.state));
                    }
                    operation = domain_reply(
                        reply.map_err(|error| format!("preferences snapshot status uncertain before mutation: {error}; query preferences status --instance {instance} --operation-id {id}"))?,
                        &instance,
                        &id,
                    )?;
                }
                if Instant::now() >= deadline {
                    return Err(format!("preferences snapshot timed out before mutation (last state {:?}); query preferences status --instance {instance} --operation-id {id}", operation.state));
                }
                if operation.state != DomainOperationState::Applied {
                    return Err(operation
                        .error
                        .unwrap_or_else(|| "preferences snapshot unavailable".into()));
                }
                let observed_revision = operation
                    .result
                    .as_ref()
                    .and_then(|value| value.get("revision"))
                    .and_then(|value| value.as_u64())
                    .ok_or("saved preference revision unavailable")?;
                if revision.is_some_and(|expected| expected != observed_revision) {
                    return Err("preference revision changed before mutation".into());
                }
                revision = Some(observed_revision);
                let custom = operation
                    .result
                    .as_ref()
                    .and_then(|value| value.get("desired"))
                    .and_then(|desired| desired.get("bubble_appearance"))
                    .and_then(|appearance| appearance.get("custom"))
                    .ok_or("saved custom palette unavailable")?;
                let mut saved: crate::preferences::BubblePalette =
                    serde_json::from_value(custom.clone())
                        .map_err(|_| "saved custom palette invalid")?;
                let selected = patch
                    .bubble_appearance
                    .as_ref()
                    .expect("appearance selected")
                    .custom;
                if color_flags[0] {
                    saved.surface = selected.surface;
                }
                if color_flags[1] {
                    saved.text = selected.text;
                }
                if color_flags[2] {
                    saved.muted = selected.muted;
                }
                if color_flags[3] {
                    saved.border = selected.border;
                }
                if color_flags[4] {
                    saved.accent = selected.accent;
                }
                patch
                    .bubble_appearance
                    .as_mut()
                    .expect("appearance selected")
                    .custom = saved;
                if Instant::now() >= deadline {
                    return Err(format!("preferences snapshot timed out before mutation (last state {:?}); query preferences status --instance {instance} --operation-id {id}", operation.state));
                }
            }
            domain_mutation(
                socket,
                instance,
                DomainAction::PreferencesSet {
                    patch,
                    expected_revision: revision,
                },
                operation_id,
                wait,
            )
        }
        DomainCommand::Status {
            family,
            instance,
            operation_id,
        } => {
            let reply = automation(
                socket,
                "status",
                None,
                Some(instance.clone()),
                Some(operation_id.clone()),
            ).map_err(|error| format!("{error}; status may expire or restart without proving the original mutation never executed"))?;
            let operation = domain_reply(reply, &instance, &operation_id)
                .map_err(|error| format!("{error}; status may expire or restart without proving the original mutation never executed"))?;
            if !family.contains(&operation.kind) {
                return Err(format!(
                    "{} status cannot show {} operations",
                    family.name(),
                    operation.kind
                ));
            }
            super::print_json(
                &serde_json::json!({"ok":matches!(operation.state,DomainOperationState::Accepted|DomainOperationState::Pending|DomainOperationState::Applied|DomainOperationState::AgentPrompted),"operation":operation}),
            )?;
            if matches!(
                operation.state,
                DomainOperationState::Failed
                    | DomainOperationState::UnknownDelivery
                    | DomainOperationState::Superseded
                    | DomainOperationState::Shutdown
            ) {
                Err(operation.error.unwrap_or_else(|| {
                    format!("operation {} ended in {:?}", operation_id, operation.state)
                }))
            } else {
                Ok(())
            }
        }
        DomainCommand::SessionsShow(identity) => {
            let reply = control::send_sessions_request(
                socket,
                &SessionsRequestEnvelope {
                    version: 1,
                    kind: "sessions".into(),
                    command: "detail".into(),
                    page: None,
                    identity: Some(identity),
                },
            )?;
            if !reply.ok {
                return Err(reply
                    .error
                    .unwrap_or_else(|| "session detail failed".into()));
            }
            super::print_json(&reply.result.ok_or("session detail unavailable")?)
        }
        DomainCommand::SessionsList {
            instance,
            filter,
            limit,
        } => {
            let mut cursor: Option<SessionPageCursor> = None;
            let mut rows = Vec::new();
            let mut revision = None;
            let mut matched = None;
            loop {
                let reply = control::send_sessions_request(
                    socket,
                    &SessionsRequestEnvelope {
                        version: 1,
                        kind: "sessions".into(),
                        command: "list".into(),
                        page: Some(SessionPageRequest {
                            instance_id: instance.clone(),
                            filter,
                            cursor: cursor.clone(),
                            limit,
                        }),
                        identity: None,
                    },
                )?;
                if !reply.ok {
                    return Err(reply.error.unwrap_or_else(|| "session page failed".into()));
                }
                let mut value = reply.result.ok_or("session page unavailable")?;
                if value.get("instance_id").and_then(|v| v.as_str()) != Some(instance.as_str())
                    || value.get("filter") != Some(&serde_json::json!(filter))
                {
                    return Err("session page identity or filter changed".into());
                }
                let page_revision = value
                    .get("revision")
                    .and_then(|v| v.as_u64())
                    .ok_or("session revision unavailable")?;
                if revision.is_some_and(|previous| previous != page_revision) {
                    return Err("session revision changed during pagination".into());
                }
                let page_matched = value
                    .get("matched")
                    .and_then(|v| v.as_u64())
                    .ok_or("session matched count unavailable")?;
                if matched.is_some_and(|previous| previous != page_matched) {
                    return Err("session matched count changed during pagination".into());
                }
                matched = Some(page_matched);
                let previous_cursor = cursor.take();
                revision = Some(page_revision);
                rows.extend(
                    value
                        .get_mut("rows")
                        .and_then(|v| v.as_array_mut())
                        .ok_or("session rows unavailable")?
                        .drain(..),
                );
                cursor = serde_json::from_value(
                    value
                        .get("next_cursor")
                        .cloned()
                        .ok_or("session cursor unavailable")?,
                )
                .map_err(|_| "invalid session cursor")?;
                if let Some(next) = &cursor {
                    if next.instance_id != instance
                        || next.revision != page_revision
                        || next.filter != filter
                    {
                        return Err("session cursor changed identity, revision or filter".into());
                    }
                    if previous_cursor.as_ref() == Some(next) {
                        return Err("session cursor did not advance".into());
                    }
                    if rows.len() as u64 >= page_matched {
                        return Err("session cursor continued beyond matched count".into());
                    }
                } else {
                    if value.get("matched").and_then(|v| v.as_u64()) != Some(rows.len() as u64) {
                        return Err("session page count changed".into());
                    }
                    value["rows"] = serde_json::Value::Array(rows);
                    return super::print_json(&value);
                }
            }
        }
        DomainCommand::SessionsPrompt {
            identity,
            text,
            operation_id,
            wait,
        } => {
            let instance = identity.instance_id.clone();
            domain_mutation(
                socket,
                instance,
                DomainAction::SessionPrompt {
                    key: identity,
                    text: read_prompt(text)?,
                },
                operation_id,
                wait,
            )
        }
        DomainCommand::DialogueList => {
            let instance = daemon_instance(socket)?;
            super::print_json(&domain_read(
                socket,
                &instance,
                DomainAction::DialogueList {},
            )?)
        }
        DomainCommand::DialogueRead(selection) => {
            let instance = daemon_instance(socket)?;
            super::print_json(&domain_read(
                socket,
                &instance,
                DomainAction::DialogueRead { selection },
            )?)
        }
        DomainCommand::DialogueSet {
            selection,
            baseline,
            text,
            operation_id,
            wait,
        } => {
            let text = read_text(text, 2048, "dialogue text")?;
            let instance = daemon_instance(socket)?;
            let baseline = match baseline {
                Some(baseline) => baseline,
                None => dialogue_baseline(socket, &instance, selection.clone())?,
            };
            domain_mutation(
                socket,
                instance,
                DomainAction::DialogueSet {
                    selection,
                    baseline,
                    text,
                },
                operation_id,
                wait,
            )
        }
        DomainCommand::DialogueResetEntry {
            selection,
            baseline,
            operation_id,
            wait,
        } => {
            let instance = daemon_instance(socket)?;
            let baseline = match baseline {
                Some(baseline) => baseline,
                None => dialogue_baseline(socket, &instance, selection.clone())?,
            };
            domain_mutation(
                socket,
                instance,
                DomainAction::DialogueResetEntry {
                    selection,
                    baseline,
                },
                operation_id,
                wait,
            )
        }
        DomainCommand::DialogueResetCharacter {
            identity,
            baseline,
            operation_id,
            wait,
        } => {
            let instance = daemon_instance(socket)?;
            let baseline = match baseline {
                Some(baseline) => baseline,
                None => dialogue_baseline(
                    socket,
                    &instance,
                    DialogueSelection {
                        identity: identity.clone(),
                        locale: DialogueLanguage::Ko,
                        slot: DialogueSlot::Idle,
                    },
                )?,
            };
            domain_mutation(
                socket,
                instance,
                DomainAction::DialogueResetCharacter {
                    identity,
                    metadata_token: baseline.metadata_token,
                    target_overrides_token: baseline.target_overrides_token,
                },
                operation_id,
                wait,
            )
        }
        DomainCommand::WorktreeInspect(key) => {
            let instance = key.instance_id.clone();
            super::print_json(&domain_read(
                socket,
                &instance,
                DomainAction::WorktreeInspect { key },
            )?)
        }
        DomainCommand::WorktreeRemove {
            token,
            operation_id,
            wait,
        } => {
            let instance = daemon_instance(socket)?;
            domain_mutation(
                socket,
                instance,
                DomainAction::WorktreeRemove { token },
                operation_id,
                wait,
            )
        }
    }
}
#[cfg(test)]
mod domain_tests {
    use super::*;

    fn parse(preferences: bool, values: &[&str]) -> Result<DomainCommand, String> {
        parse_domain(
            preferences,
            &values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn preferences_merge_partial_palette_and_reject_conflicting_controls() {
        assert!(matches!(
            parse(true, &["set", "--theme", "custom", "--surface", "#abcdef"]).unwrap(),
            DomainCommand::PreferencesSet {
                preserve_custom: true,
                color_flags: [true, false, false, false, false],
                ..
            }
        ));
        assert!(parse(
            true,
            &["set", "--language", "en", "--wait", "1", "--no-wait"]
        )
        .is_err());
        assert!(parse(true, &["get", "--language", "en"]).is_err());
        assert!(parse(
            true,
            &[
                "set",
                "--observation-local",
                "on",
                "--observation-remote",
                "off",
                "--machine",
                "node_a",
                "--machine",
                "node_b",
                "--expected-revision",
                "4"
            ]
        )
        .is_ok());
        assert!(
            matches!(parse(true,&["set","--clear-machines"]).unwrap(),DomainCommand::PreferencesSet { patch:PreferencePatch { observation_machines:Some(machines),.. },.. } if machines.is_empty())
        );
        assert!(parse(true, &["set", "--clear-machines", "--machine", "node_a"]).is_err());
    }

    #[test]
    fn cli_language_tokens_are_exact_in_split_and_inline_forms() {
        for (token, expected) in [
            ("system", LanguagePreference::System),
            ("ko", LanguagePreference::Ko),
            ("en", LanguagePreference::En),
        ] {
            for args in [
                vec!["set".to_owned(), "--language".to_owned(), token.to_owned()],
                vec!["set".to_owned(), format!("--language={token}")],
            ] {
                let command = parse_domain(true, &args).expect("supported CLI language");
                assert!(matches!(
                    command,
                    DomainCommand::PreferencesSet { patch: PreferencePatch { language: Some(language), .. }, .. }
                        if language == expected
                ));
            }
        }
        for token in ["eng", "EN", "", " en", "en ", "ko\n"] {
            for args in [
                vec!["set".to_owned(), "--language".to_owned(), token.to_owned()],
                vec!["set".to_owned(), format!("--language={token}")],
            ] {
                assert!(parse_domain(true, &args).is_err(), "{args:?}");
            }
        }
    }

    #[test]
    fn domain_wait_ignores_terminal_reply_after_deadline_without_resending_mutation() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        let root = std::env::temp_dir().join(format!("hdw{}", super::super::new_operation_id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = root.join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for state in [DomainOperationState::Pending, DomainOperationState::Applied] {
                listener.set_nonblocking(true).unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < until =>
                        {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("expected {state:?} request: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                let request: AutomationRequestEnvelope = serde_json::from_str(&request).unwrap();
                seen.push(request.command.clone());
                if state == DomainOperationState::Applied {
                    std::thread::sleep(Duration::from_millis(260));
                }
                let reply = crate::control::AutomationReplyEnvelope {
                    version: 1,
                    kind: "automation".into(),
                    ok: true,
                    error: None,
                    instance_id: Some("daemon_1".into()),
                    operation: Some(DomainOperation {
                        instance_id: "daemon_1".into(),
                        operation_id: "wait_1".into(),
                        kind: "preferences_get".into(),
                        state,
                        committed: state == DomainOperationState::Applied,
                        native_applied: false,
                        result: None,
                        error_code: None,
                        error: None,
                    }),
                };
                let _ = serde_json::to_writer(&mut stream, &reply);
                let _ = stream.write_all(b"\n");
            }
            seen
        });
        let result = domain_mutation(
            &socket,
            "daemon_1".into(),
            DomainAction::PreferencesGet {},
            Some("wait_1".into()),
            Some(Duration::from_millis(170)),
        );
        assert!(result.unwrap_err().contains("remains pending"));
        assert_eq!(server.join().unwrap(), vec!["request", "status"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dialogue_list_read_rejects_late_terminal_status_without_repeating_request() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        let root = std::env::temp_dir().join(format!("hdr{}", super::super::new_operation_id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = root.join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let peer = std::thread::spawn(move || {
            let mut seen = Vec::new();
            let mut operation_id: Option<String> = None;
            for state in [DomainOperationState::Pending, DomainOperationState::Applied] {
                listener.set_nonblocking(true).unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < until =>
                        {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("expected {state:?} read request: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                let request: AutomationRequestEnvelope = serde_json::from_str(&request).unwrap();
                let id = if let Some(submitted) = &operation_id {
                    assert_eq!(request.operation_id.as_deref(), Some(submitted.as_str()));
                    submitted.clone()
                } else {
                    let id = request.request.as_ref().unwrap().operation_id.clone();
                    operation_id = Some(id.clone());
                    id
                };
                seen.push(request.command);
                std::thread::sleep(if state == DomainOperationState::Pending {
                    Duration::from_millis(100)
                } else {
                    Duration::from_millis(260)
                });
                let reply = crate::control::AutomationReplyEnvelope {
                    version: 1,
                    kind: "automation".into(),
                    ok: true,
                    error: None,
                    instance_id: Some("daemon_read".into()),
                    operation: Some(DomainOperation {
                        instance_id: "daemon_read".into(),
                        operation_id: id,
                        kind: "dialogue_list".into(),
                        state,
                        committed: false,
                        native_applied: false,
                        result: Some(serde_json::json!({"targets":[]})),
                        error_code: None,
                        error: None,
                    }),
                };
                let _ = serde_json::to_writer(&mut stream, &reply);
                let _ = stream.write_all(b"\n");
            }
            seen
        });
        let result = domain_read_with_wait(
            &socket,
            "daemon_read",
            DomainAction::DialogueList {},
            Duration::from_millis(180),
        );
        let error = result.unwrap_err();
        assert!(
            error.contains("read remains pending") && error.contains("Pending"),
            "{error}"
        );
        assert_eq!(peer.join().unwrap(), vec!["request", "status"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_identity_and_prompt_text_sources_are_strict_and_raw() {
        let prefix = [
            "prompt",
            "--instance",
            "daemon_1",
            "--source",
            "12",
            "--generation",
            "3",
            "--terminal",
            "term_1",
        ];
        let mut args = prefix.to_vec();
        args.extend(["--text", " \n--literal\r\n "]);
        let command = parse(false, &args).expect("raw prompt");
        assert!(
            matches!(command,DomainCommand::SessionsPrompt { text:PromptText::Literal(value), identity:SessionIdentity { source_id:12,generation:3,.. },.. } if value==" \n--literal\r\n ")
        );
        let mut conflicting = prefix.to_vec();
        conflicting.extend(["--stdin", "--file", "/tmp/prompt"]);
        assert!(parse(false, &conflicting).is_err());
        assert!(parse(false, &["list", "--instance", "daemon_1", "--limit", "129"]).is_err());
        assert!(parse(
            false,
            &[
                "show",
                "--instance",
                "daemon_1",
                "--source",
                "12",
                "--generation",
                "3"
            ]
        )
        .is_err());
    }

    #[test]
    fn finite_wait_rejects_zero_nan_and_infinity() {
        for input in ["0", "NaN", "inf", "-1", "86401"] {
            assert!(parse_wait(input).is_err());
        }
        assert!(parse_wait("0.001").is_ok());
    }
    #[test]
    fn prompt_payload_keeps_large_utf8_and_whitespace_without_normalizing() {
        let text = format!("  \n{}終端\r\n  ", "多行\n".repeat(25_000));
        assert_eq!(
            read_prompt(PromptText::Literal(text.clone())).unwrap(),
            text
        );
        assert!(read_prompt(PromptText::Literal("x".repeat(512 * 1024))).is_err());
    }
    #[test]
    fn dialogue_identity_slots_baseline_and_text_sources() {
        let target = r#"{"target":{"kind":"character","id":"default"},"reference":{"id":"default","revision":0},"generation":3}"#;
        let baseline = serde_json::json!({
            "metadata_token": "a".repeat(64),
            "override_entry": " before ",
            "target_overrides_token": "b".repeat(64),
        })
        .to_string();
        for slot in [
            "idle",
            "running",
            "waiting",
            "unknown",
            "head_tap",
            "body_tap",
            "pet",
            "completion_observed",
        ] {
            let args =
                ["get", "--target", target, "--locale", "ko", "--slot", slot].map(str::to_owned);
            assert!(
                matches!(
                    parse_family("dialogue", &args),
                    Ok(DomainCommand::DialogueRead(_))
                ),
                "{slot}"
            );
        }
        let args = [
            "set",
            "--target",
            target,
            "--locale",
            "en",
            "--slot",
            "head_tap",
            "--baseline",
            &baseline,
            "--text",
            " \n안녕\r\n ",
            "--no-wait",
        ]
        .map(str::to_owned);
        assert!(matches!(parse_family("dialogue", &args),
            Ok(DomainCommand::DialogueSet {
                text:PromptText::Literal(value), baseline:Some(DialogueBaseline { override_entry:Some(entry),.. }),
                wait:None,..
            }) if value==" \n안녕\r\n " && entry==" before "));
        let reset = [
            "reset-character",
            "--target",
            target,
            "--baseline",
            &baseline,
        ]
        .map(str::to_owned);
        assert!(matches!(
            parse_family("dialogue", &reset),
            Ok(DomainCommand::DialogueResetCharacter {
                baseline: Some(_),
                ..
            })
        ));
        for args in [
            vec![
                "set",
                "--target",
                target,
                "--locale",
                "ko",
                "--slot",
                "idle",
                "--stdin",
                "--file",
                "/tmp/text",
            ],
            vec![
                "get", "--target", target, "--locale", "system", "--slot", "idle",
            ],
            vec![
                "get",
                "--target",
                target,
                "--locale",
                "en",
                "--slot",
                "completion",
            ],
            vec![
                "reset-entry",
                "--target",
                target,
                "--locale",
                "en",
                "--slot",
                "idle",
                "--baseline",
                "{}",
            ],
            vec!["reset-character", "--target", target, "--slot", "idle"],
        ] {
            assert!(parse_family(
                "dialogue",
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err());
        }
    }

    #[test]
    fn dialogue_text_limit_counts_utf8_bytes_and_preserves_whitespace() {
        let text = format!(" \n{}   ", "가".repeat(681));
        assert_eq!(text.len(), 2048);
        assert_eq!(
            read_text(PromptText::Literal(text.clone()), 2048, "dialogue text").unwrap(),
            text
        );
        assert!(read_text(
            PromptText::Literal(format!("{text}!")),
            2048,
            "dialogue text"
        )
        .is_err());
        assert!(read_text(PromptText::Literal("가".repeat(683)), 2048, "dialogue text").is_err());
        let path = std::env::temp_dir().join(format!(
            "herdr-dialogue-cli-{}",
            super::super::new_operation_id()
        ));
        std::fs::write(&path, text.as_bytes()).unwrap();
        let actual = read_text(PromptText::File(path.clone()), 2048, "dialogue text");
        std::fs::write(&path, format!("{text}!").as_bytes()).unwrap();
        let overflow = read_text(PromptText::File(path.clone()), 2048, "dialogue text");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let invalid_utf8 = read_text(PromptText::File(path.clone()), 2048, "dialogue text");
        std::fs::remove_file(path).unwrap();
        assert_eq!(actual.unwrap(), text);
        assert!(overflow.is_err());
        assert!(invalid_utf8.is_err());
    }

    #[test]
    fn worktree_requires_explicit_coherent_key_and_token_only_remove() {
        let inspect = [
            "inspect",
            "--instance",
            "daemon_1",
            "--source",
            "12",
            "--generation",
            "3",
            "--terminal",
            "pane_1",
        ]
        .map(str::to_owned);
        assert!(matches!(
            parse_family("worktree", &inspect),
            Ok(DomainCommand::WorktreeInspect(SessionIdentity {
                source_id: 12,
                generation: 3,
                ..
            }))
        ));
        let remove = ["remove", "--token", "opaque_one_use_token", "--no-wait"].map(str::to_owned);
        assert!(matches!(
            parse_family("worktree", &remove),
            Ok(DomainCommand::WorktreeRemove { wait: None, .. })
        ));
        for args in [
            vec![
                "inspect",
                "--instance",
                "daemon_1",
                "--source",
                "12",
                "--terminal",
                "pane_1",
            ],
            vec!["remove", "--token", "opaque", "--force"],
            vec!["remove", "--token", "opaque", "--instance", "daemon_1"],
            vec!["remove", "--token", "opaque", "--path", "/tmp/repo"],
            vec!["remove", "--token", "opaque", "--wait", "0"],
        ] {
            assert!(parse_family(
                "worktree",
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err());
        }
        assert!(matches!(
            parse_family(
                "sessions",
                &["status", "--instance", "daemon_1", "--operation-id", "op_1"].map(str::to_owned)
            ),
            Ok(DomainCommand::Status {
                family: DomainFamily::Sessions,
                ..
            })
        ));
        for (family, expected) in [
            ("preferences", DomainFamily::Preferences),
            ("dialogue", DomainFamily::Dialogue),
            ("worktree", DomainFamily::Worktree),
        ] {
            let status =
                ["status", "--instance", "daemon_1", "--operation-id", "op_1"].map(str::to_owned);
            assert!(matches!(parse_family(family, &status),
                Ok(DomainCommand::Status { family, .. }) if family == expected));
        }
        assert!(!DomainFamily::Preferences.contains("session_prompt"));
        assert!(!DomainFamily::Sessions.contains("worktree_remove"));
        assert!(!DomainFamily::Dialogue.contains("preferences_set"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| (*token).to_owned()).collect()
    }

    #[test]
    fn absolute_set_fields_and_default_wait() {
        let command = parse(&args(&[
            "set",
            "--visible",
            "off",
            "--passthrough=on",
            "--bubble-placement",
            "auto",
            "--scale",
            "1.25",
            "--expected-revision",
            "7",
            "--operation-id",
            "custom_1",
        ]))
        .unwrap();
        let PresentationCommand::Set {
            patch,
            expected_revision,
            operation_id,
            wait,
        } = command
        else {
            panic!("expected set");
        };
        assert_eq!(patch.visible, Some(false));
        assert_eq!(patch.passthrough, Some(true));
        assert_eq!(patch.bubble_placement, Some(BubblePlacement::Auto));
        assert_eq!(patch.scale, Some(1.25));
        assert_eq!(expected_revision, Some(7));
        assert_eq!(operation_id.as_deref(), Some("custom_1"));
        assert_eq!(wait, Some(DEFAULT_WAIT));
    }

    #[test]
    fn status_requires_instance_and_forbids_mutation_fields() {
        assert!(parse(&args(&["status", "op1"])).is_err());
        assert!(parse(&args(&["status", "op1", "--instance", "i", "--wait", "1"])).is_err());
        assert!(matches!(
            parse(&args(&["status", "op1", "--instance=i"])).unwrap(),
            PresentationCommand::Status { .. }
        ));
        assert!(parse(&args(&["get", "--instance", "i"])).is_err());
        assert!(parse(&args(&["reset", "--visible", "on"])).is_err());
        assert!(parse(&args(&["set"])).is_err());
    }

    #[test]
    fn rejects_duplicates_conflicts_and_invalid_values() {
        assert!(parse(&args(&["set", "--visible", "on", "--visible", "off"])).is_err());
        assert!(parse(&args(&["reset", "--wait", "1", "--no-wait"])).is_err());
        assert!(parse(&args(&["set", "--scale", "NaN"])).is_err());
        assert!(parse(&args(&["set", "--visible", "yes"])).is_err());
        assert!(parse(&args(&["set", "--visible", "on", "--operation-id", "a b"])).is_err());
        assert!(parse(&args(&[
            "set",
            "--visible",
            "on",
            "--expected-revision",
            "-1"
        ]))
        .is_err());
    }
}
