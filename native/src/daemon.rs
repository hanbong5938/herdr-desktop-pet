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
}

impl DaemonConfig {
    pub fn new(
        paths: Paths,
        assets: PathBuf,
        herdr_socket: PathBuf,
        startup_token: Option<String>,
        assets_override: bool,
        automatic_start: bool,
    ) -> Self {
        Self {
            paths,
            assets,
            assets_override,
            herdr_socket,
            startup_token,
            automatic_start,
        }
    }
}

struct PackWorkerGuard(Arc<PackService>);

impl Drop for PackWorkerGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// Run the sole native daemon instance in the foreground of its detached child.
///
/// The lifecycle lock belongs to the daemon for its entire lifetime.  Control
/// clients never mutate AppState while doing disk I/O, and all worker shutdown
/// joins happen after AppKit has returned.
pub fn run(config: DaemonConfig) -> Result<(), String> {
    let _lock = crate::acquire_lock_until(&config.paths, Instant::now() + crate::STARTUP_TIMEOUT)?;
    let settings = lifecycle::read_settings(&config.paths.config_dir)?;
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
    let prefs = Preferences::load_for_daemon()?;
    initial_state.apply_observation_preferences(prefs.observation().clone());
    initial_state.set_preferences(&prefs);
    let automation =
        new_automation(PresentationTarget::from_scene(&initial_state.scene()), None)
            .map_err(|error| format!("cannot initialize presentation automation: {error}"))?;
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

    let executable = crate::bundle::executable()
        .map_err(|error| format!("cannot resolve desktop-pet executable path: {error}"))?;
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

    let ui_result = ui::run(
        Arc::clone(&shared),
        &config.assets,
        Arc::clone(&packs),
        prefs,
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
