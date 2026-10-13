//! Herdr's socket, saved-machine, lifecycle and outcome authority for Desktop Pet.
//! Construction is inert; composition explicitly selects and starts this integration.
mod endpoint;
mod extensions;
mod i18n;
mod operations;
mod outcome;
mod protocol;
mod remote;
mod settings;
mod socket;

use desktop_pet_contracts::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_ENDPOINTS: usize = 64;
const MAX_OPERATIONS: usize = 128;
const DISCONNECTION_GRACE: Duration = Duration::from_secs(30);
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
enum Route { Local(PathBuf), Remote(String) }
struct Binding {
    route: Route,
    generation: u64,
    connected: bool,
    disabled: bool,
    offline_since: Option<Instant>,
}
enum Completion {
    Prompt(Sender<Result<PromptReceipt, PromptError>>),
    Worktree(Sender<Result<(), WorktreeRemoveError>>),
}
struct PendingOperation {
    source: String,
    generation: u64,
    epoch: u64,
    instance: String,
    offered: bool,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
}
impl PendingOperation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        match &self.completion {
            Completion::Prompt(tx) => { let _ = tx.send(Err(if self.offered { PromptError::UnknownDelivery } else { PromptError::StaleTarget })); }
            Completion::Worktree(tx) => { let _ = tx.send(Err(if self.offered { WorktreeRemoveError::UnknownDelivery } else { WorktreeRemoveError::StaleTarget })); }
        }
    }
}
struct Registry {
    enabled: bool,
    epoch: u64,
    configured_socket: Option<PathBuf>,
    initial_endpoints: Vec<PathBuf>,
    endpoints_loaded: bool,
    endpoint_revision: u64,
    bindings: HashMap<String, Binding>,
    operations: HashMap<u64, PendingOperation>,
    preferences: settings::ObservationPreferences,
    catalog: settings::SourceCatalog,
    exit_with_herdr: bool,
    policy_selection: ObservationSelection,
    deadline: Option<Instant>,
    connected: usize,
    finalized: bool,
    failure: Option<String>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            enabled: false, epoch: 0, configured_socket: None, initial_endpoints: Vec::new(),
            endpoints_loaded: false, endpoint_revision: 0, bindings: HashMap::new(), operations: HashMap::new(),
            preferences: settings::ObservationPreferences::default(), catalog: settings::SourceCatalog::default(),
            exit_with_herdr: true, policy_selection: ObservationSelection::Auto, deadline: Some(Instant::now() + DISCONNECTION_GRACE), connected: 0,
            finalized: false, failure: None,
        }
    }
}
struct Shared {
    runtime: OnceLock<Arc<dyn RuntimePort>>,
    store: Mutex<Option<ProfileStore>>,
    registry: Mutex<Registry>,
    // Serializes successful policy persistence with the final exit decision;
    // never acquired inside a domain callback or used for host I/O.
    policy_writes: Mutex<()>,
    stop: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
impl Shared {
    /// This is the sole domain/registry lock ordering boundary. No caller enters
    /// with a registry guard, and no callback performs host I/O, waits or joins.
    fn transaction(&self, operation: &mut dyn FnMut(&mut dyn DomainAccess, &mut Registry)) -> Result<(), String> {
        let runtime = self.runtime.get().ok_or("Herdr integration is not started")?;
        let mut error = None;
        let result = runtime.with_domain(&mut |domain| {
            match self.registry.lock() {
                Ok(mut registry) => operation(domain, &mut registry),
                Err(_) => error = Some("Herdr registry is poisoned".to_owned()),
            }
        });
        if let Err(error) = result {
            self.fail(error.clone());
            return Err(error);
        }
        if let Some(error) = error { self.fail(error.clone()); return Err(error); }
        Ok(())
    }
    fn fail(&self, error: String) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.failure = Some(error);
            for binding in registry.bindings.values_mut() {
                binding.connected = false;
                binding.offline_since.get_or_insert(Instant::now());
            }
        }
        self.wake();
    }
    fn wake(&self) { if let Some(runtime) = self.runtime.get() { runtime.wake_ui(); } }
    fn epoch(&self) -> Option<u64> {
        if self.stop.load(Ordering::Acquire) { return None; }
        self.registry.lock().ok().filter(|registry| registry.enabled && !registry.finalized).map(|registry| registry.epoch)
    }
    fn current(&self, epoch: u64) -> bool { self.epoch() == Some(epoch) }
    fn with_epoch(&self, epoch: u64, operation: &mut dyn FnMut(&mut dyn DomainAccess, &mut Registry)) -> Result<(), String> {
        let mut current = false;
        self.transaction(&mut |domain, registry| {
            if registry.enabled && registry.epoch == epoch && !registry.finalized && !self.stop.load(Ordering::Acquire) {
                current = true;
                operation(domain, registry);
            }
        })?;
        if current { Ok(()) } else { Err("Herdr observation epoch retired".to_owned()) }
    }
    fn capacity_transaction(&self, epoch: u64, source: &str, generation: u64, operation: &mut dyn FnMut(&mut dyn DomainAccess, &mut Registry)) -> Result<(), String> {
        let runtime = self.runtime.get().ok_or("Herdr integration is not started")?;
        let mut error = None;
        let mut source_failure = false;
        runtime.with_domain(&mut |domain| {
            match self.registry.lock() {
                Ok(registry) if registry.enabled && registry.epoch == epoch && !registry.finalized && !self.stop.load(Ordering::Acquire) => {}
                Ok(_) => { error = Some("Herdr observation epoch retired".to_owned()); return; }
                Err(_) => { source_failure = true; error = Some("Herdr registry is poisoned".to_owned()); return; }
            }
            // Cross-provider reclamation must run before our own registry lock.
            domain.ensure_source_capacity(source, generation, Instant::now());
            match self.registry.lock() {
                Ok(mut registry) if registry.enabled && registry.epoch == epoch && !registry.finalized && !self.stop.load(Ordering::Acquire) => operation(domain, &mut registry),
                Ok(_) => error = Some("Herdr observation epoch retired".to_owned()),
                Err(_) => { source_failure = true; error = Some("Herdr registry is poisoned".to_owned()); }
            }
        }).map_err(|error| { self.fail(error.clone()); error })?;
        if let Some(error) = error { if source_failure { self.fail(error.clone()); } return Err(error); }
        Ok(())
    }
    fn admission_transaction(&self, epoch: u64, source: &str, minimum_generation: u64, operation: &mut dyn FnMut(&mut dyn DomainAccess, &mut Registry, u64)) -> Result<(), String> {
        let runtime = self.runtime.get().ok_or("Herdr integration is not started")?;
        let mut error = None;
        let mut source_failure = false;
        runtime.with_domain(&mut |domain| {
            match self.registry.lock() {
                Ok(registry) if registry.enabled && registry.epoch == epoch && !registry.finalized && !self.stop.load(Ordering::Acquire) => {}
                Ok(_) => { error = Some("Herdr observation epoch retired".to_owned()); return; }
                Err(_) => { source_failure = true; error = Some("Herdr registry is poisoned".to_owned()); return; }
            }
            let previous = domain.source_generation(source).unwrap_or(0).max(domain.retired_source_generation(source).unwrap_or(0));
            let Some(generation) = previous.checked_add(1).map(|next| next.max(minimum_generation)) else {
                error = Some("Herdr source generation exhausted".to_owned()); return;
            };
            domain.ensure_source_capacity(source, generation, Instant::now());
            match self.registry.lock() {
                Ok(mut registry) if registry.enabled && registry.epoch == epoch && !registry.finalized && !self.stop.load(Ordering::Acquire) => operation(domain, &mut registry, generation),
                Ok(_) => error = Some("Herdr observation epoch retired".to_owned()),
                Err(_) => { source_failure = true; error = Some("Herdr registry is poisoned".to_owned()); }
            }
        }).map_err(|error| { self.fail(error.clone()); error })?;
        if let Some(error) = error { if source_failure { self.fail(error.clone()); } return Err(error); }
        Ok(())
    }
    fn begin(&self, domain: &mut dyn DomainAccess, registry: &mut Registry, descriptor: SourceDescriptor, generation: u64, route: Route) -> bool {
        let source = descriptor.id.clone();
        let admitted = domain.begin_source(descriptor, generation);
        if admitted {
            let observed = match &route {
                Route::Local(_) => registry.preferences.local,
                Route::Remote(id) => registry.preferences.remote && registry.preferences.machines.contains(id),
            };
            domain.set_source_observed(&source, generation, observed);
            registry.operations.retain(|_, operation| {
                if operation.source == source && operation.generation != generation { operation.cancel(); false } else { true }
            });
            registry.bindings.insert(source, Binding { route, generation, connected: false, disabled: false, offline_since: Some(Instant::now()) });
        }
        admitted
    }
    fn spawn(&self, name: &str, worker: impl FnOnce() + Send + 'static) -> Result<(), String> {
        let handle = thread::Builder::new().name(name.to_owned()).spawn(worker).map_err(|error| format!("cannot start {name}: {error}"))?;
        match self.workers.lock() {
            Ok(mut workers) => { workers.push(handle); Ok(()) }
            Err(_) => {
                self.stop.store(true, Ordering::Release);
                let _ = handle.join();
                Err("Herdr workers lock is poisoned".to_owned())
            }
        }
    }
    fn observe_connections(registry: &mut Registry, now: Instant) {
        let connected = registry.bindings.values().filter(|binding| binding.connected).count();
        if connected > 0 { registry.deadline = None; }
        else if registry.connected > 0 && registry.exit_with_herdr { registry.deadline = Some(now + DISCONNECTION_GRACE); }
        registry.connected = connected;
    }
}

/// Register this object as Integration and register `extensions()` alongside it.
/// The common selected ProfileStore MUST be injected before starting workers.
pub struct HerdrIntegration { shared: Arc<Shared>, started: AtomicBool }
impl HerdrIntegration {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { shared: Arc::new(Shared { runtime: OnceLock::new(), store: Mutex::new(None), registry: Mutex::new(Registry::default()), policy_writes: Mutex::new(()), stop: AtomicBool::new(false), workers: Mutex::new(Vec::new()) }), started: AtomicBool::new(false) })
    }
    pub fn command_reads_input(&self, command: &str, arguments: &[std::ffi::OsString]) -> Result<bool, String> {
        if command != "herdr-observation" || arguments.len() > 1 { return Err("usage: herdr-observation [get|set]".to_owned()); }
        match arguments.first().map(|argument| argument.to_str()) {
            None | Some(Some("get")) => Ok(false),
            Some(Some("set")) => Ok(true),
            _ => Err("usage: herdr-observation [get|set]".to_owned()),
        }
    }
    pub fn configure_profile(&self, store: ProfileStore) -> Result<(), String> {
        let (object, exists) = store.read_object_with_presence()?;
        let exit = desktop_pet_contracts::profile_store::setting_bool(store.config_dir(), &object, "exit_with_herdr")?.unwrap_or(!exists);
        let preferences = settings::read_preferences(&store)?;
        let mut registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        registry.exit_with_herdr = exit;
        registry.preferences = preferences;
        registry.deadline = exit.then_some(Instant::now() + DISCONNECTION_GRACE);
        drop(registry);
        *self.shared.store.lock().map_err(|_| "Herdr profile is poisoned")? = Some(store);
        Ok(())
    }
    pub fn extensions(self: &Arc<Self>) -> IntegrationExtensions {
        IntegrationExtensions { control: vec![Arc::new(extensions::Registration(Arc::clone(self))), Arc::new(extensions::Observation(Arc::clone(self)))], cli: vec![Arc::clone(self) as Arc<dyn CliExtension>], lifecycle: vec![Arc::clone(self) as Arc<dyn LifecycleExtension>] }
    }
    pub fn registration_frame(&self, selection: &ObservationSelection) -> Result<Option<Vec<u8>>, String> {
        if !selection.includes_provider("herdr") { return Ok(None); }
        let configured = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?.configured_socket.clone();
        let endpoint = socket::canonical_endpoint(&configured.unwrap_or_else(settings::default_herdr_socket))?;
        serde_json::to_vec(&serde_json::json!({"kind":"register","endpoint":endpoint})).map(Some).map_err(|error| error.to_string())
    }
    fn store(&self) -> Result<ProfileStore, String> {
        self.shared.store.lock().map_err(|_| "Herdr profile is poisoned")?.clone().ok_or_else(|| "Herdr profile is not configured".to_owned())
    }
    /// Generic composition projection used only by the explicit legacy codec.
    pub fn legacy_status_fields(&self) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        let socket = registry.configured_socket.clone().unwrap_or_else(settings::default_herdr_socket);
        let mut fields = serde_json::Map::new();
        fields.insert("herdr_socket".to_owned(), serde_json::to_value(socket).map_err(|error| error.to_string())?);
        fields.insert("exit_with_herdr".to_owned(), serde_json::Value::Bool(registry.exit_with_herdr));
        Ok(fields)
    }
    pub fn legacy_endpoint_registered(&self, endpoint: &std::path::Path) -> Result<bool, String> {
        // Legacy callers already pass the canonical endpoint captured at launch.
        // Readiness never mutates or touches the host filesystem during Prepare.
        let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        Ok(registry.initial_endpoints.iter().any(|known| known == endpoint))
    }
    pub fn register(&self, endpoint: PathBuf) -> Result<(), String> {
        if self.shared.epoch().is_none() { return Err("Herdr observation is disabled".to_owned()); }
        let endpoint = socket::canonical_endpoint(&endpoint)?;
        // Persistence is outside the domain lock. Runtime registration is a
        // separate bounded transaction; a late retired registration cannot send.
        settings::add_endpoint(&self.store()?, &endpoint)?;
        let source = endpoint.to_string_lossy().into_owned();
        let mut admitted = Err("Herdr observation is disabled".to_owned());
        self.shared.transaction(&mut |domain, registry| {
            admitted = (|| {
                domain.mutation_allowed()?;
                if !registry.enabled || registry.finalized { return Err("Herdr observation is disabled".to_owned()); }
                if !registry.initial_endpoints.contains(&endpoint) {
                    if registry.initial_endpoints.len() >= MAX_ENDPOINTS { return Err("Herdr endpoint limit reached".to_owned()); }
                    registry.initial_endpoints.push(endpoint.clone());
                    registry.endpoint_revision += 1;
                }
                if let Some(binding) = registry.bindings.get_mut(&source) { binding.disabled = false; }
                Ok(())
            })();
        })?;
        admitted?;
        self.shared.wake();
        Ok(())
    }
    fn retire_registry(&self, domain: &mut dyn DomainAccess, registry: &mut Registry) {
        registry.epoch = registry.epoch.saturating_add(1);
        registry.enabled = false;
        for (source, binding) in registry.bindings.drain() { domain.remove_source(&source, binding.generation); }
        for (_, operation) in registry.operations.drain() { operation.cancel(); }
        registry.connected = 0;
        registry.deadline = registry.exit_with_herdr.then_some(Instant::now() + DISCONNECTION_GRACE);
        registry.endpoints_loaded = false;
    }
}
impl Integration for HerdrIntegration {
    fn id(&self) -> &'static str { "herdr" }
    fn start(&self, runtime: Arc<dyn RuntimePort>) -> Result<(), String> {
        self.store()?;
        if self.started.swap(true, Ordering::AcqRel) { return Err("Herdr integration already started".to_owned()); }
        self.shared.runtime.set(runtime).map_err(|_| "Herdr runtime already configured")?;
        let shared = Arc::clone(&self.shared);
        self.shared.spawn("desktop-pet-herdr-watchers", move || endpoint::watcher_loop(shared))?;
        let shared = Arc::clone(&self.shared);
        if let Err(error) = self.shared.spawn("desktop-pet-herdr-remote", move || remote::run_coordinator(shared)) { self.shutdown(); return Err(error); }
        Ok(())
    }
    fn tick(&self, now: Instant) {
        if let Ok(mut registry) = self.shared.registry.lock() { Shared::observe_connections(&mut registry, now); }
        let finished = if let Ok(mut workers) = self.shared.workers.lock() {
            let mut finished = Vec::new();
            let mut index = 0;
            while index < workers.len() { if workers[index].is_finished() { finished.push(workers.swap_remove(index)); } else { index += 1; } }
            finished
        } else { Vec::new() };
        for worker in finished { if worker.join().is_err() { self.shared.fail("Herdr worker panicked".to_owned()); } }
    }
    fn set_enabled(&self, enabled: bool, domain: &mut dyn DomainAccess) {
        match self.shared.registry.lock() {
            Ok(mut registry) => {
                if registry.enabled != enabled {
                    self.retire_registry(domain, &mut registry);
                    registry.enabled = enabled;
                    registry.finalized = false;
                }
            }
            Err(_) => { self.shared.stop.store(true, Ordering::Release); }
        }
    }
    fn retire(&self, domain: &mut dyn DomainAccess) {
        if let Ok(mut registry) = self.shared.registry.lock() { self.retire_registry(domain, &mut registry); }
        else { self.shared.stop.store(true, Ordering::Release); }
    }
    fn reclaim_expired_sources(&self, now: Instant, domain: &mut dyn DomainAccess) -> usize {
        let Ok(mut registry) = self.shared.registry.lock() else { return 0; };
        let mut removed = 0;
        let Registry { bindings, operations, .. } = &mut *registry;
        bindings.retain(|source, binding| {
            if !binding.connected && binding.offline_since.is_some_and(|at| now.saturating_duration_since(at) >= DISCONNECTION_GRACE)
                && !operations.values().any(|operation| operation.source == *source)
                && domain.remove_offline_source(source, binding.generation) {
                removed += 1; false
            } else { true }
        });
        removed
    }
    fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        // Composition retires under its domain transaction, returns from the UI,
        // then calls shutdown. All host operations have finite deadlines.
        let workers = match self.shared.workers.lock() {
            Ok(mut workers) => std::mem::take(&mut *workers),
            Err(_) => { self.shared.fail("Herdr workers lock is poisoned".to_owned()); return; }
        };
        for worker in workers { if worker.join().is_err() { self.shared.fail("Herdr worker panicked".to_owned()); } }
    }
    fn submit_prompt(&self, key: SessionKey, text: String) -> Result<Receiver<Result<PromptReceipt, PromptError>>, PromptError> {
        if text.trim().is_empty() { return Err(PromptError::Empty); }
        let text_bytes = operations::serialized_len(&text).map_err(|error| PromptError::Other(error.to_string()))?;
        let (tx, rx) = mpsc::channel();
        let operation = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
        let mut admission = Err(PromptError::Offline);
        self.shared.transaction(&mut |domain, registry| {
            admission = (|| {
                domain.mutation_allowed().map_err(PromptError::Other)?;
                let (target, provider) = domain.prompt_route(&key)?;
                if provider != "herdr" { return Err(PromptError::Unsupported); }
                let binding = registry.bindings.get(&target.source).ok_or(PromptError::Offline)?;
                let path = match &binding.route { Route::Local(path) => path.clone(), Route::Remote(_) => return Err(PromptError::ReadOnly) };
                if !registry.enabled || registry.finalized || !binding.connected || binding.disabled { return Err(PromptError::Offline); }
                if binding.generation != key.generation { return Err(PromptError::StaleTarget); }
                if !operations::prompt_frame_fits(&target.pane_id, text_bytes).map_err(|error| PromptError::Other(error.to_string()))? { return Err(PromptError::TooLarge); }
                if registry.operations.len() >= MAX_OPERATIONS { return Err(PromptError::Busy); }
                let cancelled = Arc::new(AtomicBool::new(false));
                registry.operations.insert(operation, PendingOperation { source: target.source.clone(), generation: key.generation, epoch: registry.epoch, instance: domain.instance_id().to_owned(), offered: false, cancelled: Arc::clone(&cancelled), completion: Completion::Prompt(tx.clone()) });
                Ok((target, path, cancelled))
            })();
        }).map_err(PromptError::Other)?;
        let (target, path, cancelled) = admission?;
        let shared = Arc::clone(&self.shared);
        if let Err(error) = self.shared.spawn("desktop-pet-herdr-prompt", move || {
            let result = operations::send_prompt(&shared, operation, &key, &target, &path, &text, &cancelled);
            if let Ok(mut registry) = shared.registry.lock() {
                if let Some(pending) = registry.operations.remove(&operation) { if let Completion::Prompt(tx) = pending.completion { let _ = tx.send(result); } }
            }
            shared.wake();
        }) { if let Ok(mut registry) = self.shared.registry.lock() { registry.operations.remove(&operation); } return Err(PromptError::Other(error)); }
        Ok(rx)
    }
    fn submit_worktree_remove(&self, target: WorktreeRemoveTarget) -> Result<Receiver<Result<(), WorktreeRemoveError>>, WorktreeRemoveError> {
        let (tx, rx) = mpsc::channel();
        let operation = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
        let mut admission = Err(WorktreeRemoveError::Offline);
        self.shared.transaction(&mut |domain, registry| {
            admission = (|| {
                domain.mutation_allowed().map_err(WorktreeRemoveError::Other)?;
                let descriptor = domain.source_descriptor(&target.source).ok_or(WorktreeRemoveError::Offline)?;
                if descriptor.provider != "herdr" || !descriptor.capabilities.remove_worktree || descriptor.locality != SourceLocality::Local { return Err(WorktreeRemoveError::ReadOnly); }
                if domain.worktree_remove_target(&target.key)? != target { return Err(WorktreeRemoveError::StaleTarget); }
                let binding = registry.bindings.get(&target.source).ok_or(WorktreeRemoveError::Offline)?;
                let path = match &binding.route { Route::Local(path) => path.clone(), Route::Remote(_) => return Err(WorktreeRemoveError::ReadOnly) };
                if !registry.enabled || registry.finalized || !binding.connected || binding.disabled { return Err(WorktreeRemoveError::Offline); }
                if binding.generation != target.key.generation { return Err(WorktreeRemoveError::StaleTarget); }
                if registry.operations.len() >= MAX_OPERATIONS { return Err(WorktreeRemoveError::Busy); }
                let cancelled = Arc::new(AtomicBool::new(false));
                registry.operations.insert(operation, PendingOperation { source: target.source.clone(), generation: target.key.generation, epoch: registry.epoch, instance: domain.instance_id().to_owned(), offered: false, cancelled: Arc::clone(&cancelled), completion: Completion::Worktree(tx.clone()) });
                Ok((path, cancelled))
            })();
        }).map_err(WorktreeRemoveError::Other)?;
        let (path, cancelled) = admission?;
        let shared = Arc::clone(&self.shared);
        if let Err(error) = self.shared.spawn("desktop-pet-herdr-worktree", move || {
            let result = operations::send_worktree_remove(&shared, operation, &target, &path, &cancelled);
            if let Ok(mut registry) = shared.registry.lock() {
                if let Some(pending) = registry.operations.remove(&operation) { if let Completion::Worktree(tx) = pending.completion { let _ = tx.send(result); } }
            }
            shared.wake();
        }) { if let Ok(mut registry) = self.shared.registry.lock() { registry.operations.remove(&operation); } return Err(WorktreeRemoveError::Other(error)); }
        Ok(rx)
    }
}

#[cfg(test)]
mod harness;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod settings_tests;

impl Drop for HerdrIntegration {
    fn drop(&mut self) { self.shared.stop.store(true, Ordering::Release); }
}
