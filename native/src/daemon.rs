use crate::automation::{new_automation, PresentationTarget};
use crate::character_service::PackService;
use crate::control::ControlServer;
use crate::herdr::Watchers;
use crate::lifecycle::{self, Paths};
use crate::preferences::Preferences;
use crate::remote::RemoteWatchers;
use crate::socket;
use crate::state::AppState;
use crate::ui;
use herdr_update_coordinator::protocol::{register_active, start_allowed, unregister_active};
use herdr_update_coordinator::{detect_origin, executable_identity, InstallOrigin, UpdateContext};
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub paths: Paths,
    pub assets: PathBuf,
    pub assets_override: bool,
    pub herdr_socket: PathBuf,
    pub startup_token: Option<String>,
    pub automatic_start: bool,
    pub host_plugin_config_dir: Option<PathBuf>,
}

impl DaemonConfig {
    pub fn new(
        paths: Paths,
        assets: PathBuf,
        herdr_socket: PathBuf,
        startup_token: Option<String>,
        assets_override: bool,
        automatic_start: bool,
        host_plugin_config_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            paths,
            assets,
            assets_override,
            herdr_socket,
            startup_token,
            automatic_start,
            host_plugin_config_dir,
        }
    }
}

struct PackWorkerGuard(Arc<PackService>);

impl Drop for PackWorkerGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

struct ActiveUpdateGuard {
    origin: InstallOrigin,
    state_dir: PathBuf,
    instance_id: String,
}

impl Drop for ActiveUpdateGuard {
    fn drop(&mut self) {
        if let Err(error) = unregister_active(
            &self.origin,
            &self.state_dir,
            std::process::id(),
            &self.instance_id,
        ) {
            eprintln!("desktop-pet: cannot unregister active installation: {error}");
        }
    }
}

fn update_context(
    config: &DaemonConfig,
    instance_id: String,
    executable: PathBuf,
    prefs: &Preferences,
) -> Result<UpdateContext, String> {
    let environment = [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "HERDR_BIN_PATH",
        "HERDR_PLUGIN_ID",
        "HERDR_ENV",
        "HERDR_PLUGIN_CONFIG_DIR",
        "HERDR_PLUGIN_STATE_DIR",
        "HERDR_SOCKET_PATH",
    ]
    .into_iter()
    .filter_map(|key| env::var(key).ok().map(|value| (key.to_owned(), value)))
    .collect::<BTreeMap<_, _>>();
    let locale = ui::current_ui_locale(prefs.language()).tag();
    let mut context = UpdateContext {
        running: executable_identity(&executable)?,
        executable,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        instance_id,
        config_dir: config.paths.config_dir.clone(),
        host_plugin_config_dir: config.host_plugin_config_dir.clone(),
        state_dir: config.paths.state_dir.clone(),
        herdr_socket: config.herdr_socket.clone(),
        running_origin: None,
        assets_override: config.assets_override.then(|| config.assets.clone()),
        environment,
        locale: locale.to_owned(),
    };
    context.running_origin =
        Some(Box::new(detect_origin(&context).unwrap_or_else(|error| {
            InstallOrigin::Unknown { reason: error }
        })));
    Ok(context)
}

/// Run the sole native daemon instance in the foreground of its detached child.
///
/// The lifecycle lock belongs to the daemon for its entire lifetime.  Control
/// clients never mutate AppState while doing disk I/O, and all worker shutdown
/// joins happen after AppKit has returned.
pub fn run(config: DaemonConfig) -> Result<(), String> {
    let _lock = crate::acquire_lock_until(&config.paths, Instant::now() + crate::STARTUP_TIMEOUT)?;
    let settings = lifecycle::read_settings(&config.paths.config_dir)?;
    let updater_generation = env::var("HERDR_DESKTOP_PET_UPDATER_GENERATION")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| "invalid updater generation".to_owned())
        })
        .transpose()?;
    let updater_token = env::var("HERDR_DESKTOP_PET_UPDATER_TOKEN").ok();
    if updater_token.is_some() != updater_generation.is_some() {
        return Err("updater startup requires both token and generation".into());
    }
    if let Some(token) = config.startup_token.as_deref() {
        lifecycle::claim_startup(&config.paths.config_dir, token)?;
    }
    if config.automatic_start && !settings.auto_start {
        if let Some(token) = config.startup_token.as_deref() {
            let _ = lifecycle::abandon_startup(&config.paths.config_dir, token);
        }
        return Err("desktop-pet automatic start is disabled".to_owned());
    }
    lifecycle::write_settings(&config.paths.config_dir, settings)?;

    let builtin_assets = if config.assets_override {
        crate::resolve_assets(None).unwrap_or_else(|_| config.assets.clone())
    } else {
        config.assets.clone()
    };
    let override_assets = config.assets_override.then(|| config.assets.clone());
    let packs = PackService::new(
        config.paths.config_dir.clone(),
        builtin_assets,
        override_assets,
    );
    // Every early return joins the worker before releasing the lifecycle lock.
    let _pack_worker = PackWorkerGuard(Arc::clone(&packs));
    packs.start()?;

    let mut initial_state = AppState::new();
    initial_state.set_lifecycle_settings(settings);
    let prefs = Preferences::load_for_daemon(&config.paths.config_dir)?;
    initial_state.apply_observation_preferences(prefs.observation().clone());
    initial_state.set_preferences(&prefs);
    let automation =
        new_automation(PresentationTarget::from_scene(&initial_state.scene()), None)
            .map_err(|error| format!("cannot initialize presentation automation: {error}"))?;
    let instance_id = crate::automation::lock_automation(&automation)
        .instance()
        .to_owned();
    let executable = crate::bundle::executable()
        .map_err(|error| format!("cannot resolve desktop-pet executable path: {error}"))?;
    let context = update_context(&config, instance_id.clone(), executable, &prefs)?;
    let origin = context
        .running_origin
        .as_deref()
        .expect("origin captured")
        .clone();
    start_allowed(
        &origin,
        &config.paths.state_dir,
        updater_generation,
        updater_token.as_deref(),
    )?;
    let _active = if matches!(origin, InstallOrigin::Unknown { .. }) {
        None
    } else {
        register_active(
            &origin,
            &config.paths.state_dir,
            std::process::id(),
            &instance_id,
            updater_generation,
            updater_token.as_deref(),
        )?;
        Some(ActiveUpdateGuard {
            origin: origin.clone(),
            state_dir: config.paths.state_dir.clone(),
            instance_id,
        })
    };
    let executable = context.running.path.clone();
    initial_state.set_update_context(context);
    initial_state.set_automation(automation.clone());
    let shared = Arc::new(Mutex::new(initial_state));
    let watchers = Arc::new(Mutex::new(Watchers::new(Arc::clone(&shared))));
    let mut remote_watchers = RemoteWatchers::new(Arc::clone(&shared));
    let launch_endpoint = socket::canonical_endpoint(&config.herdr_socket)?;
    let mut endpoints = vec![launch_endpoint.clone()];
    for endpoint in lifecycle::read_endpoints(&config.paths.config_dir)? {
        if endpoint != launch_endpoint && !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    let mut accepted_endpoints = Vec::new();
    for endpoint in &endpoints {
        let result = {
            let watcher = watchers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            watcher.register(endpoint.clone())
        };
        match result {
            Ok(()) => accepted_endpoints.push(endpoint.clone()),
            Err(error) => {
                eprintln!(
                    "desktop-pet: Herdr registration pending for {}: {error}",
                    endpoint.display()
                );
            }
        }
    }

    let mut control = match ControlServer::bind(
        Arc::clone(&shared),
        Arc::clone(&watchers),
        config.paths.clone(),
        Some(launch_endpoint),
        executable,
        Arc::clone(&packs),
    ) {
        Ok(control) => control,
        Err(error) => {
            packs.shutdown();
            return Err(error);
        }
    };
    for endpoint in accepted_endpoints {
        if let Err(error) = control.record_registered_endpoint(endpoint.clone()) {
            eprintln!(
                "desktop-pet: cannot persist Herdr endpoint {}: {error}",
                endpoint.display()
            );
        }
    }
    start_allowed(
        &origin,
        &config.paths.state_dir,
        updater_generation,
        updater_token.as_deref(),
    )?;
    if let Err(error) = control.start() {
        packs.shutdown();
        return Err(error);
    }
    if let Some(token) = config.startup_token.as_deref() {
        if let Err(error) = lifecycle::finish_startup(&config.paths.config_dir, token) {
            control.shutdown();
            packs.shutdown();
            return Err(error);
        }
    }
    if updater_generation.is_some() {
        if let Err(error) = start_allowed(
            &origin,
            &config.paths.state_dir,
            updater_generation,
            updater_token.as_deref(),
        ) {
            control.shutdown();
            packs.shutdown();
            return Err(error);
        }
    }

    let ui_result = ui::run(
        Arc::clone(&shared),
        &config.assets,
        Arc::clone(&packs),
        prefs,
        config.paths.clone(),
    );
    crate::automation::lock_automation(&automation).shutdown();
    if ui_result.is_err() {
        let mut state = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.request_shutdown();
        drop(state);
        ui::wake();
    }

    // AppKit has stopped before watcher/control threads are joined.  This
    // ordering prevents a watcher event from touching a destroyed UI.
    {
        let mut watcher = watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        watcher.shutdown();
    }
    remote_watchers.shutdown();
    control.shutdown();
    packs.shutdown();

    ui_result
}
