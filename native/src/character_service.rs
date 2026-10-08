use crate::assets::CharacterMetadata;
use crate::assets::ManagedPack;
use crate::character_renderer;
use crate::character_store::{PackStore, PackTransaction};
use crate::character_types::{
    CharacterRef, PackAction, PackListing, PackOperation, PackRequest, RendererToken,
};
use crate::official_characters::OfficialCharacters;
use crate::ui;
use objc2::MainThreadMarker;
use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};

const MAX_PENDING_OPERATIONS: usize = 32;
const MAX_RETAINED_OPERATIONS: usize = 128;
const MAX_OPERATION_ID_BYTES: usize = 128;
const MAX_DIAGNOSTIC_BYTES: usize = 1024;
const MAX_PENDING_METADATA: usize = 16;
const MAX_CACHED_METADATA: usize = 32;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadPhase {
    Resolving,
    Downloading,
    Validating,
    Installing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub operation_id: String,
    pub revision: u64,
    pub received: u64,
    pub total: u64,
    pub phase: DownloadPhase,
}

const ACCEPTED: &str = "accepted";
const PREPARING: &str = "preparing";
const COMPLETED: &str = "completed";
const FAILED: &str = "failed";
const CANCELED: &str = "canceled";
const APPLYING: &str = "applying";
const COMMITTED_PENDING_APPLY: &str = "committed_pending_apply";
const DURABILITY_UNKNOWN: &str = "durability_unknown";
const EXISTING_OPERATION_NOT_EXECUTED: &str =
    "existing operation; this submission was not executed; use status";

#[derive(Debug)]
struct OperationEntry {
    fingerprint: Vec<u8>,
    operation: PackOperation,
    cancel: Option<Arc<AtomicBool>>,
    is_official: bool,
}

#[derive(Debug, Default)]
struct ServiceState {
    operations: HashMap<String, OperationEntry>,
    order: VecDeque<String>,
    active: Option<CharacterRef>,
    override_active: bool,
    runtime_error: Option<String>,
    renderer_error: Option<String>,
    active_operation: Option<String>,
    /// Once a transaction starts committing, its native token is no longer
    /// cancelable, even before the committed result is published.
    commit_started: bool,
    progress: HashMap<String, DownloadProgress>,
    ui_ready: bool,
}

#[derive(Debug)]
struct QueuedRequest {
    request: PackRequest,
    cancel: Arc<AtomicBool>,
}

/// Presentation data from the requested character's validated manifest.
/// `None` metadata means the legacy manifest has no authored dialogue.
#[derive(Clone, Debug)]
pub(crate) struct DialogueMetadata {
    pub metadata: Option<CharacterMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataKey {
    reference: CharacterRef,
    generation: u64,
}

#[derive(Clone, Copy)]
enum MetadataRequest {
    Automatic,
    Explicit,
}

fn listing_has_dialogue_reference(
    listing: Option<&PackListing>,
    reference: &CharacterRef,
    generation: u64,
) -> bool {
    listing.is_some_and(|listing| {
        listing.generation == generation
            && (reference.is_builtin()
                || listing.packs.iter().any(|record| {
                    record.id == reference.id && record.revisions.contains(&reference.revision)
                }))
    })
}

#[derive(Default)]
struct MetadataState {
    pending: VecDeque<MetadataKey>,
    loading: Option<MetadataKey>,
    results: VecDeque<(MetadataKey, Result<DialogueMetadata, String>)>,
}

/// The serial coordinator for managed character-pack operations.
///
/// Filesystem work is deliberately kept on the worker.  Submission only
/// validates and records an in-memory request before waking that worker, so a
/// control client cannot hold up the daemon on staging or native AppKit work.
pub struct PackService {
    config_dir: PathBuf,
    builtin_assets: PathBuf,
    /// A raw legacy `--assets` path, when the daemon was explicitly launched
    /// with one.  It is intentionally immutable for the daemon lifetime.
    pub override_assets: Option<PathBuf>,
    official: Arc<OfficialCharacters>,
    state: Mutex<ServiceState>,
    cache: Mutex<Option<PackListing>>,
    /// Serializes store reads against the worker's begin/commit/finish
    /// transaction.  UI/menu callers use `try_lock` and receive a bounded
    /// busy response instead of waiting behind native preflight.
    io_gate: Mutex<()>,
    /// Fixture-owned interleaving after the transaction and service gate drop.
    #[cfg(test)]
    after_transaction_scope: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Fixture-owned panic point while the committed transaction still owns its flock.
    #[cfg(test)]
    after_committed_publication: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    queue: Mutex<VecDeque<QueuedRequest>>,
    metadata: Mutex<MetadataState>,
    wake: Condvar,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// Set by the shutdown owner after the JoinHandle has been joined; this
    /// lets concurrent shutdown callers wait for the completed join.
    worker_done: (Mutex<bool>, Condvar),
    started: AtomicBool,
    stopping: AtomicBool,
}

impl PackService {
    pub fn new(
        config_dir: PathBuf,
        builtin_assets: PathBuf,
        override_assets: Option<PathBuf>,
    ) -> Arc<Self> {
        let builtin_id = builtin_manifest_id(&builtin_assets);
        Arc::new(Self {
            config_dir,
            builtin_assets,
            override_assets,
            official: OfficialCharacters::new(builtin_id),
            state: Mutex::new(ServiceState::default()),
            cache: Mutex::new(None),
            io_gate: Mutex::new(()),
            #[cfg(test)]
            after_transaction_scope: Mutex::new(None),
            #[cfg(test)]
            after_committed_publication: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            metadata: Mutex::new(MetadataState::default()),
            wake: Condvar::new(),
            worker: Mutex::new(None),
            worker_done: (Mutex::new(true), Condvar::new()),
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        })
    }

    pub fn official_catalog(&self) -> Arc<OfficialCharacters> {
        Arc::clone(&self.official)
    }
    /// A portrait may not compete with a mutation for the store's flock.
    /// Contention is pending admission, not an image failure.
    pub fn load_preview(
        &self,
        reference: &CharacterRef,
        generation: u64,
    ) -> Result<Option<crate::assets::ValidatedCharacter>, String> {
        let _io = match self.io_gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let guard = poisoned.into_inner();
                self.io_gate.clear_poison();
                guard
            }
        };
        PackStore::new(self.config_dir.clone(), Some(self.builtin_assets.clone()))
            .load_preview(reference, generation)
            .map_err(bound_diagnostic)
    }

    pub fn cached_download_progress(&self, operation_id: &str) -> Option<DownloadProgress> {
        lock_unpoisoned(&self.state)
            .progress
            .get(operation_id)
            .cloned()
    }

    pub fn cancel_download(&self, operation_id: &str) -> bool {
        let state = lock_unpoisoned(&self.state);
        let Some(entry) = state.operations.get(operation_id) else {
            return false;
        };
        if !entry.is_official
            || entry.operation.committed
            || is_terminal(&entry.operation.state)
            || (state.commit_started && state.active_operation.as_deref() == Some(operation_id))
        {
            return false;
        }
        let Some(cancel) = &entry.cancel else {
            return false;
        };
        cancel.store(true, Ordering::Release);
        drop(state);
        self.wake.notify_one();
        ui::wake();
        true
    }

    /// Start exactly one serial worker.
    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("pack service has already shut down".to_owned());
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let initial_listing = {
            let _io = lock_unpoisoned(&self.io_gate);
            let store = PackStore::new(self.config_dir.clone(), None);
            store.list().map_err(bound_diagnostic)
        };
        let initial_listing = match initial_listing {
            Ok(listing) => listing,
            Err(error) => {
                self.started.store(false, Ordering::Release);
                return Err(error);
            }
        };
        *lock_unpoisoned(&self.cache) = Some(bound_listing(initial_listing));

        {
            let mut done = lock_unpoisoned(&self.worker_done.0);
            *done = false;
        }
        let weak = Arc::downgrade(self);
        let mut slot = lock_unpoisoned(&self.worker);
        if self.stopping.load(Ordering::Acquire) {
            self.started.store(false, Ordering::Release);
            let mut done = lock_unpoisoned(&self.worker_done.0);
            *done = true;
            self.worker_done.1.notify_all();
            return Err("pack service has already shut down".to_owned());
        }
        let handle = match thread::Builder::new()
            .name("desktop-pet-pack-worker".to_owned())
            .spawn(move || worker_loop(weak))
        {
            Ok(handle) => handle,
            Err(error) => {
                self.started.store(false, Ordering::Release);
                let mut done = lock_unpoisoned(&self.worker_done.0);
                *done = true;
                self.worker_done.1.notify_all();
                return Err(format!("cannot start character-pack worker: {error}"));
            }
        };
        *slot = Some(handle);
        Ok(())
    }

    /// Read the current registry and overlay the daemon's runtime active state.
    /// IPC callers get a bounded busy response while the serial transaction is
    /// holding the store gate; the UI uses `cached_list` instead.
    pub fn list(&self) -> Result<PackListing, String> {
        let _io = self.try_lock_io_gate()?;
        let store = PackStore::new(self.config_dir.clone(), None);
        let listing = store.list().map_err(bound_diagnostic)?;
        self.cache_listing(listing);
        Ok(self.cached_list())
    }

    /// Return a memory-only snapshot for AppKit menus and refresh callbacks.
    pub fn cached_list(&self) -> PackListing {
        let cached = lock_unpoisoned(&self.cache).clone();
        let mut listing = cached.unwrap_or_else(|| PackListing {
            generation: 0,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: Vec::new(),
            error: Some("pack registry is not ready".to_owned()),
        });
        if let Some(error) = &lock_unpoisoned(&self.state).renderer_error {
            listing.error = Some(error.clone());
        }
        bound_listing(listing)
    }
    /// Check identity against the current in-memory listing without filesystem
    /// work. Call before writing an override, as well as while displaying it.
    pub(crate) fn dialogue_reference_valid(
        &self,
        reference: &CharacterRef,
        generation: u64,
    ) -> bool {
        listing_has_dialogue_reference(lock_unpoisoned(&self.cache).as_ref(), reference, generation)
    }

    /// Automatic requests retain negative results. Only a new explicit read
    /// or user event can retry a failed exact reference/generation.
    pub(crate) fn request_dialogue_metadata(
        &self,
        reference: CharacterRef,
        generation: u64,
    ) -> Result<(), String> {
        self.admit_dialogue_metadata(reference, generation, MetadataRequest::Automatic)
    }

    pub(crate) fn retry_dialogue_metadata(
        &self,
        reference: CharacterRef,
        generation: u64,
    ) -> Result<(), String> {
        self.admit_dialogue_metadata(reference, generation, MetadataRequest::Explicit)
    }

    fn admit_dialogue_metadata(
        &self,
        reference: CharacterRef,
        generation: u64,
        request: MetadataRequest,
    ) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) || !self.started.load(Ordering::Acquire) {
            return Err("pack service is not running".to_owned());
        }
        let key = MetadataKey {
            reference,
            generation,
        };
        {
            // The worker tests both queues under this mutex before sleeping.
            // Keep listing validity and metadata admission atomic with cache
            // invalidation (queue -> cache -> metadata).
            let _queue = lock_unpoisoned(&self.queue);
            let cache = lock_unpoisoned(&self.cache);
            if !listing_has_dialogue_reference(cache.as_ref(), &key.reference, generation) {
                return Err("character reference is stale or unavailable".to_owned());
            }
            let mut state = lock_unpoisoned(&self.metadata);
            if state.pending.contains(&key) || state.loading.as_ref() == Some(&key) {
                return Ok(());
            }
            let cached = state
                .results
                .iter()
                .position(|(candidate, _)| candidate == &key);
            if let Some(index) = cached {
                if state.results[index].1.is_ok() || matches!(request, MetadataRequest::Automatic) {
                    return Ok(());
                }
            }
            if state.pending.len() >= MAX_PENDING_METADATA {
                return Err("Busy: character metadata queue is full".to_owned());
            }
            if let Some(index) = cached {
                state.results.remove(index);
            }
            state.pending.push_back(key);
        }
        self.wake.notify_one();
        Ok(())
    }

    /// None means pending or not requested; Some(Err) means this exact
    /// identity failed. Never return data from a different revision/target.
    pub(crate) fn cached_dialogue_metadata(
        &self,
        reference: &CharacterRef,
        generation: u64,
    ) -> Option<Result<DialogueMetadata, String>> {
        let cache = lock_unpoisoned(&self.cache);
        if !listing_has_dialogue_reference(cache.as_ref(), reference, generation) {
            return Some(Err("character reference is stale or unavailable".to_owned()));
        }
        let state = lock_unpoisoned(&self.metadata);
        state.results.iter().find_map(|(cached, result)| {
            (cached.generation == generation && &cached.reference == reference)
                .then(|| result.clone())
        })
    }

    fn publish_dialogue_metadata(
        &self,
        key: MetadataKey,
        result: Result<DialogueMetadata, String>,
    ) {
        // Cache and metadata must stay locked together through publication;
        // invalidating the generation must not race a completed worker read.
        let cache = lock_unpoisoned(&self.cache);
        let mut metadata = lock_unpoisoned(&self.metadata);
        let was_loading = metadata.loading.as_ref() == Some(&key);
        if was_loading {
            metadata.loading = None;
        }
        let published = was_loading
            && listing_has_dialogue_reference(cache.as_ref(), &key.reference, key.generation)
            && !self.stopping.load(Ordering::Acquire);
        if published {
            metadata.results.push_back((key, result));
            if metadata.results.len() > MAX_CACHED_METADATA {
                metadata.results.pop_front();
            }
        }
        drop(metadata);
        drop(cache);
        if published {
            ui::wake();
        }
    }

    /// Look up a retained operation without touching the registry.  Expired
    /// entries intentionally return `None`; UI refresh must not fall through
    /// to the filesystem-backed `status` method.
    pub fn cached_status(&self, operation_id: &str) -> Option<PackOperation> {
        lock_unpoisoned(&self.state)
            .operations
            .get(operation_id)
            .map(|entry| bound_operation(entry.operation.clone()))
    }

    fn cache_listing(&self, mut listing: PackListing) {
        listing = bound_listing(listing);
        let state = lock_unpoisoned(&self.state);
        if state.active.is_some() || state.override_active {
            listing.active = state.active.clone();
        }
        listing.override_active = state.override_active;
        if let Some(error) = state.runtime_error.clone() {
            listing.error = Some(bound_diagnostic(error));
        }
        {
            let mut cache = lock_unpoisoned(&self.cache);
            if cache.as_ref().map(|old| old.generation) != Some(listing.generation) {
                let mut metadata = lock_unpoisoned(&self.metadata);
                metadata.pending.clear();
                metadata.loading = None;
                metadata.results.clear();
            }
            *cache = Some(listing);
        }
    }

    /// Return startup candidates in precedence order.  A raw explicit asset
    /// override is a legacy candidate and suppresses managed candidates.
    pub fn startup_candidates(&self) -> Result<Vec<(CharacterRef, PathBuf, bool)>, String> {
        if let Some(path) = self.override_assets.clone() {
            return Ok(vec![(CharacterRef::builtin(), path, true)]);
        }
        let _io = self.try_lock_io_gate()?;
        let store = PackStore::new(self.config_dir.clone(), Some(self.builtin_assets.clone()));
        Ok(store
            .startup_candidates()
            .map_err(bound_diagnostic)?
            .into_iter()
            .map(|(reference, path)| (reference, path, false))
            .collect())
    }

    pub fn load_startup_managed(
        &self,
        reference: &CharacterRef,
    ) -> Result<crate::assets::ValidatedCharacter, String> {
        let _io = self.try_lock_io_gate()?;
        PackStore::new(self.config_dir.clone(), Some(self.builtin_assets.clone()))
            .load_revision(reference)
            .map_err(bound_diagnostic)
    }

    /// Whether a pack mutation is pending or executing (not metadata lookup).
    /// This is an observation only; UI submissions use atomic admission below.
    pub fn ui_mutation_busy(&self) -> bool {
        let state = lock_unpoisoned(&self.state);
        let queue = lock_unpoisoned(&self.queue);
        mutation_busy(&state, &queue)
    }

    /// Mutations include accepted queue entries, native preflight and commit,
    /// even when the worker has temporarily removed an item from its queue.
    /// Requested dialogue metadata can also still publish into the editor.
    pub fn has_pending_update_work(&self) -> bool {
        let state = lock_unpoisoned(&self.state);
        let queue = lock_unpoisoned(&self.queue);
        let mutating = mutation_busy(&state, &queue)
            || state.commit_started
            || state.operations.values().any(|entry| {
                matches!(
                    entry.operation.state.as_str(),
                    COMMITTED_PENDING_APPLY | DURABILITY_UNKNOWN
                )
            });
        let metadata = lock_unpoisoned(&self.metadata);
        mutating || !metadata.pending.is_empty() || metadata.loading.is_some()
    }

    /// Enqueue a UI request only when no other pack mutation owns admission.
    pub fn submit_ui_if_idle(&self, request: PackRequest) -> Result<PackOperation, String> {
        self.submit_inner(request, true)
    }

    /// Enqueue one request without touching the filesystem.
    pub fn submit(&self, request: PackRequest) -> Result<PackOperation, String> {
        self.submit_inner(request, false)
    }

    fn submit_inner(&self, request: PackRequest, ui_only: bool) -> Result<PackOperation, String> {
        if matches!(request.action, PackAction::ImportAndSelect { .. }) {
            crate::control::validate_pack_request(&request)?;
        }
        validate_operation_id(&request.operation_id)?;
        let fingerprint = serde_json::to_vec(&request)
            .map_err(|error| format!("cannot fingerprint pack operation: {error}"))?;

        if self.stopping.load(Ordering::Acquire) {
            return Err("pack service is shutting down".to_owned());
        }
        if !self.started.load(Ordering::Acquire) {
            return Err("pack service is not running".to_owned());
        }
        let mut state = lock_unpoisoned(&self.state);
        if !state.ui_ready {
            return Err("pack service is waiting for native UI readiness".to_owned());
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err("pack service is shutting down".to_owned());
        }
        if let Some(existing) = state.operations.get(&request.operation_id) {
            if existing.fingerprint.is_empty() {
                return Err(format!(
                    "operation id {} already exists; {EXISTING_OPERATION_NOT_EXECUTED}",
                    request.operation_id
                ));
            }
            if existing.fingerprint != fingerprint {
                return Err(format!(
                    "operation id {} was already used for a different request",
                    request.operation_id
                ));
            }
            return Ok(bound_operation(existing.operation.clone()));
        }
        let mut queue = lock_unpoisoned(&self.queue);
        if ui_only && mutation_busy(&state, &queue) {
            return Err("another pack operation is in progress".to_owned());
        }
        if queue.len() >= MAX_PENDING_OPERATIONS {
            return Err("too many queued pack operations".to_owned());
        }
        retain_capacity(&mut state);
        if state.operations.len() >= MAX_RETAINED_OPERATIONS {
            return Err("too many retained pack operation results".to_owned());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let operation = PackOperation {
            operation_id: request.operation_id.clone(),
            state: ACCEPTED.to_owned(),
            committed: false,
            ui_applied: false,
            generation: None,
            error: None,
        };
        state.order.push_back(request.operation_id.clone());
        state.operations.insert(
            request.operation_id.clone(),
            OperationEntry {
                fingerprint,
                operation: operation.clone(),
                cancel: Some(Arc::clone(&cancel)),
                is_official: matches!(request.action, PackAction::ImportAndSelect { .. }),
            },
        );
        queue.push_back(QueuedRequest { request, cancel });
        drop(state);
        drop(queue);
        self.wake.notify_one();
        Ok(operation)
    }

    /// Obtain a retained result, or recover the durable last-operation record
    /// after a daemon restart.
    pub fn status(&self, operation_id: &str) -> Result<PackOperation, String> {
        validate_operation_id(operation_id)?;
        {
            let state = lock_unpoisoned(&self.state);
            if let Some(entry) = state.operations.get(operation_id) {
                return Ok(bound_operation(entry.operation.clone()));
            }
        }
        let _io = self.try_lock_io_gate()?;
        let store = PackStore::new(self.config_dir.clone(), None);
        if let Some(operation) = store
            .operation_status(operation_id)
            .map_err(bound_diagnostic)?
        {
            let operation = bound_operation(operation);
            let mut state = lock_unpoisoned(&self.state);
            retain_capacity(&mut state);
            if state.operations.len() < MAX_RETAINED_OPERATIONS {
                state.order.push_back(operation_id.to_owned());
                state.operations.insert(
                    operation_id.to_owned(),
                    OperationEntry {
                        // Unknown recovered fingerprints never match a new submission.
                        fingerprint: Vec::new(),
                        operation: operation.clone(),
                        cancel: None,
                        is_official: false,
                    },
                );
            }
            return Ok(operation);
        }
        Err(format!("unknown pack operation {operation_id}"))
    }

    pub fn set_active(
        &self,
        reference: CharacterRef,
        override_active: bool,
        error: Option<String>,
    ) {
        let error = error.map(bound_diagnostic);
        {
            let mut state = lock_unpoisoned(&self.state);
            state.active = Some(reference.clone());
            state.override_active = override_active;
            state.runtime_error = error.clone();
            state.renderer_error = None;
            state.ui_ready = true;
            if let Some(listing) = lock_unpoisoned(&self.cache).as_mut() {
                listing.active = Some(reference);
                listing.override_active = override_active;
                listing.error = error;
            }
        }
        ui::wake();
    }

    /// Only a matching native Applied acknowledgement reaches this method.
    /// Publish its runtime identity and operation evidence as one observation.
    fn acknowledge_applied(&self, operation_id: &str, reference: CharacterRef) {
        {
            let mut state = lock_unpoisoned(&self.state);
            state.active = Some(reference.clone());
            state.override_active = false;
            state.runtime_error = None;
            state.renderer_error = None;
            state.ui_ready = true;
            if let Some(entry) = state.operations.get_mut(operation_id) {
                entry.operation.ui_applied = true;
            }
            if let Some(listing) = lock_unpoisoned(&self.cache).as_mut() {
                listing.active = Some(reference);
                listing.override_active = false;
                listing.error = None;
            }
        }
        ui::wake();
    }

    pub fn set_renderer_error(&self, error: Option<String>) {
        let error = error.map(bound_diagnostic);
        let mut state = lock_unpoisoned(&self.state);
        if state.renderer_error == error {
            return;
        }
        state.renderer_error = error;
        drop(state);
        ui::wake();
    }

    /// Cancel pending work and stop the sole worker.  Shutdown owns the
    /// worker join, so no mutation can outlive daemon lifecycle ownership.
    pub fn shutdown(&self) {
        {
            let mut state = lock_unpoisoned(&self.state);
            let first = !self.stopping.swap(true, Ordering::AcqRel);
            if first {
                for (id, entry) in &state.operations {
                    if !is_terminal(&entry.operation.state)
                        && !(state.commit_started && state.active_operation.as_deref() == Some(id))
                    {
                        if let Some(cancel) = &entry.cancel {
                            cancel.store(true, Ordering::Release);
                        }
                    }
                }
                let mut queue = lock_unpoisoned(&self.queue);
                for item in queue.drain(..) {
                    if let Some(entry) = state.operations.get_mut(&item.request.operation_id) {
                        entry.operation.state = CANCELED.to_owned();
                        entry.operation.error =
                            Some("pack service shut down before execution".to_owned());
                        entry.cancel = None;
                    }
                }
            }
        };

        // Wake both the worker's condition variable and any cancellation-aware
        // AppKit bridge before waiting.  Neither wake requires a service lock.
        self.wake.notify_all();
        ui::wake();

        let handle = {
            let mut slot = lock_unpoisoned(&self.worker);
            slot.take()
        };
        if let Some(handle) = handle {
            // The daemon is the sole shutdown owner; joining here cannot be a
            // self-join.  No worker/main-thread mutex is held while waiting.
            let _ = handle.join();
            self.official.shutdown();
            let mut done = lock_unpoisoned(&self.worker_done.0);
            *done = true;
            self.worker_done.1.notify_all();
            return;
        }

        // A concurrent shutdown caller may already own the JoinHandle.  Wait
        // for that caller to finish the join rather than returning early.
        let mut done = lock_unpoisoned(&self.worker_done.0);
        while !*done {
            done = match self.worker_done.1.wait(done) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        drop(done);
        self.official.shutdown();
    }
}

enum WorkerItem {
    Operation(QueuedRequest),
    Metadata(MetadataKey),
}

fn worker_loop(weak: Weak<PackService>) {
    loop {
        let Some(service) = weak.upgrade() else {
            return;
        };
        let item = {
            let mut queue = lock_unpoisoned(&service.queue);
            loop {
                if service.stopping.load(Ordering::Acquire) {
                    break None;
                }
                if let Some(item) = queue.pop_front() {
                    break Some(WorkerItem::Operation(item));
                }
                let pending = {
                    let mut metadata = lock_unpoisoned(&service.metadata);
                    let pending = metadata.pending.pop_front();
                    metadata.loading = pending.clone();
                    pending
                };
                if let Some(key) = pending {
                    break Some(WorkerItem::Metadata(key));
                }
                queue = match service.wake.wait(queue) {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
            }
        };
        let Some(item) = item else {
            break;
        };
        let item = match item {
            WorkerItem::Metadata(key) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let _io = lock_unpoisoned(&service.io_gate);
                    PackStore::new(
                        service.config_dir.clone(),
                        Some(service.builtin_assets.clone()),
                    )
                    .dialogue_metadata(&key.reference, key.generation)
                    .map(|(_, metadata)| DialogueMetadata { metadata })
                    .map_err(bound_diagnostic)
                }))
                .unwrap_or_else(|_| Err("character metadata lookup aborted".to_string()));
                service.publish_dialogue_metadata(key, result);
                continue;
            }
            WorkerItem::Operation(item) => item,
        };
        let operation_id = item.request.operation_id.clone();
        if service.stopping.load(Ordering::Acquire) {
            item.cancel.store(true, Ordering::Release);
            service.update_operation(&operation_id, |operation| {
                operation.state = CANCELED.to_owned();
                operation.error = Some("pack service shut down before execution".to_owned());
            });
            service.retire_operation(&operation_id);
            continue;
        }
        if catch_unwind(AssertUnwindSafe(|| service.execute(item))).is_err() {
            let _io = lock_unpoisoned(&service.io_gate);
            let recovered =
                PackStore::new(service.config_dir.clone(), None).listing_and_status(&operation_id);
            service.recover_aborted_operation(
                &operation_id,
                recovered,
                None,
                false,
                "pack worker aborted; registry commit outcome unknown",
            );
            drop(_io);
            service.retire_operation(&operation_id);
        }
    }
}
impl PackService {
    fn execute(&self, item: QueuedRequest) {
        let operation_id = item.request.operation_id.clone();
        let managed_active = {
            let mut state = lock_unpoisoned(&self.state);
            state.active_operation = Some(operation_id.clone());
            let active = if state.override_active {
                None
            } else {
                state.active.clone()
            };
            if let Some(entry) = state.operations.get_mut(&operation_id) {
                entry.operation.state = PREPARING.to_owned();
            }
            active
        };
        ui::wake();
        if self.stopping.load(Ordering::Acquire) {
            item.cancel.store(true, Ordering::Release);
        }
        if item.cancel.load(Ordering::Acquire) {
            self.update_operation(&operation_id, |operation| {
                operation.state = CANCELED.to_owned();
                operation.error = Some(
                    if self.stopping.load(Ordering::Acquire) {
                        "pack service shut down before execution"
                    } else {
                        "pack operation canceled before execution"
                    }
                    .to_owned(),
                );
            });
            self.retire_operation(&operation_id);
            return;
        }

        let result = (|| {
            let verified = if let PackAction::ImportAndSelect { official } = &item.request.action {
                // A durable identity is authoritative before touching the network.
                let _io = lock_unpoisoned(&self.io_gate);
                let existing =
                    PackStore::new(self.config_dir.clone(), None).listing_and_status(&operation_id);
                let (listing, durable) = match existing {
                    Ok(pair) => pair,
                    Err(error) => {
                        self.mark_durable_lookup_unknown(&operation_id, error);
                        return Ok(());
                    }
                };
                if let Some(durable) = durable {
                    self.reconcile_durable_operation(listing, durable);
                    return Ok(());
                }
                drop(_io);
                let expected = item
                    .request
                    .expected_generation
                    .ok_or_else(|| "official import requires expected_generation".to_owned())?;
                if listing.generation != expected {
                    return Err(format!(
                        "character store generation mismatch (expected {expected}, current {})",
                        listing.generation
                    ));
                }
                if listing.packs.iter().any(|pack| pack.id == official.id) {
                    return Err(format!("character pack {} already exists", official.id));
                }
                if item.cancel.load(Ordering::Acquire) {
                    return Err("official download canceled before start".to_owned());
                }
                self.publish_download_progress(&operation_id, DownloadPhase::Resolving, 0, 0);
                let managed = self.official.download_verified(
                    official,
                    &item.cancel,
                    |received, total| {
                        self.publish_download_progress(
                            &operation_id,
                            DownloadPhase::Downloading,
                            received,
                            total,
                        );
                    },
                )?;
                if item.cancel.load(Ordering::Acquire) {
                    return Err("official download canceled before commit".to_owned());
                }
                self.publish_download_progress(&operation_id, DownloadPhase::Validating, 0, 0);
                Some(managed)
            } else {
                None
            };
            let _io = lock_unpoisoned(&self.io_gate);
            if verified.is_some() {
                self.publish_download_progress(&operation_id, DownloadPhase::Installing, 0, 0);
            }
            self.execute_transaction(&item, managed_active.as_ref(), verified)
        })();
        #[cfg(test)]
        let at_boundary = lock_unpoisoned(&self.after_transaction_scope).take();
        #[cfg(test)]
        if let Some(at_boundary) = at_boundary {
            at_boundary();
        }
        // The transaction has already published its known commit under its flock.
        if let Err(error) = result {
            self.update_operation(&operation_id, |operation| {
                if !operation.committed {
                    operation.state = if item.cancel.load(Ordering::Acquire) {
                        CANCELED.to_owned()
                    } else {
                        FAILED.to_owned()
                    };
                    operation.error = Some(error);
                } else if operation.state != DURABILITY_UNKNOWN {
                    // The normal post-commit path handles finish errors
                    // itself.  Keep an unexpected error nonterminal rather
                    // than publishing pending before finalization.
                    operation.error = Some(append_error(operation.error.take(), error));
                }
            });
        }
        self.retire_operation(&operation_id);
    }

    fn execute_transaction(
        &self,
        item: &QueuedRequest,
        managed_active: Option<&CharacterRef>,
        verified: Option<ManagedPack>,
    ) -> Result<(), String> {
        let operation_id = &item.request.operation_id;
        let store = PackStore::new(self.config_dir.clone(), Some(self.builtin_assets.clone()));
        let existing = match store.listing_and_status(operation_id) {
            Ok(pair) => pair,
            Err(error) => {
                self.mark_durable_lookup_unknown(operation_id, error);
                return Ok(());
            }
        };
        if let (listing, Some(durable)) = existing {
            self.reconcile_durable_operation(listing, durable);
            return Ok(());
        }
        let begin_result = match verified {
            Some(verified) => {
                store.begin_with_verified_import(&item.request, managed_active, verified)
            }
            None => store.begin(&item.request, managed_active),
        };
        let mut transaction = match begin_result {
            Ok(transaction) => transaction,
            Err(error) => match store.listing_and_status(operation_id) {
                Ok((listing, Some(durable))) => {
                    self.reconcile_durable_operation(listing, durable);
                    return Ok(());
                }
                Ok((_, None)) => return Err(error),
                Err(status_error) => {
                    self.mark_durable_lookup_unknown(
                        operation_id,
                        format!("begin failed: {error}; status lookup failed: {status_error}"),
                    );
                    return Ok(());
                }
            },
        };
        let mut prepared: Option<RendererToken> = None;
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.execute_transaction_body(item, managed_active, &mut transaction, &mut prepared)
        }));
        match result {
            Ok(result) => result,
            Err(_) => {
                let recovered = transaction.listing_and_status();
                let known = transaction.committed_listing();
                self.recover_aborted_operation(
                    operation_id,
                    recovered,
                    known,
                    transaction.durability_unknown(),
                    "pack transaction aborted; registry commit outcome unknown",
                );
                if !self
                    .cached_status(operation_id)
                    .is_some_and(|operation| operation.ui_applied)
                {
                    if let Some(token) = prepared {
                        ui::discard_character(token);
                    }
                }
                Ok(())
            }
        }
    }

    fn execute_transaction_body(
        &self,
        item: &QueuedRequest,
        managed_active: Option<&CharacterRef>,
        transaction: &mut PackTransaction,
        prepared: &mut Option<RendererToken>,
    ) -> Result<(), String> {
        let operation_id = &item.request.operation_id;
        let selected = transaction.selected.clone();
        let candidate_ref = transaction.candidate_ref.clone();
        let changes_selection = transaction.changes_selection;

        if let Some(candidate) = transaction.candidate.take() {
            if item.cancel.load(Ordering::Acquire) {
                let error = "pack operation canceled before native preflight".to_owned();
                let _ = transaction.mark_failed(error.clone());
                return Err(error);
            }
            let token = RendererToken::new(
                operation_id.clone(),
                candidate_ref
                    .clone()
                    .ok_or_else(|| "native candidate has no revision identity".to_owned())?,
                candidate.content_digest().to_owned(),
            )?;
            *prepared = Some(token.clone());
            let ready = ui::prepare_character(token.clone(), candidate, item.cancel.clone())
                .and_then(|actual| {
                    if actual == token {
                        Ok(())
                    } else {
                        Err("native Ready token mismatch".to_owned())
                    }
                });
            if let Err(error) = ready {
                ui::discard_character(prepared.take().expect("native token was staged"));
                let error = format!("native preflight failed: {error}");
                let _ = transaction.mark_failed(error.clone());
                return Err(error);
            }
        }
        if !self.begin_commit(item) {
            if let Some(token) = prepared.take() {
                ui::discard_character(token);
            }
            let error = "pack operation canceled before commit".to_owned();
            let _ = transaction.mark_failed(error.clone());
            return Err(error);
        }

        let commit = match transaction.commit() {
            Ok(commit) => commit,
            Err(error) => {
                if let Some(token) = prepared.take() {
                    ui::discard_character(token);
                }
                // commit() only fails before registry.json is replaced, so
                // this is an ordinary failure, not an unknown outcome.
                return Err(format!("registry commit failed: {error}"));
            }
        };
        let commit_error = commit.error.clone();
        let durability_unknown = commit.durability_unknown || commit_error.is_some();
        // The transaction still owns its flock. Publish the observed registry
        // snapshot before a committed/applying wake can expose its generation.
        if let Some(listing) = transaction.committed_listing() {
            self.cache_listing(listing);
        }
        self.update_operation(operation_id, |operation| {
            operation.committed = true;
            operation.generation = Some(commit.generation);
            // The in-memory result remains nonterminal until native apply and
            // transaction finalization have both returned.  The durable
            // registry still contains its ordinary committed-pending record.
            operation.state = APPLYING.to_owned();
            operation.error = commit_error.clone();
        });
        #[cfg(test)]
        let after_publish = lock_unpoisoned(&self.after_committed_publication).take();
        #[cfg(test)]
        if let Some(after_publish) = after_publish {
            after_publish();
        }

        let (override_active, explicit_override_clear) = {
            let state = lock_unpoisoned(&self.state);
            (
                state.override_active,
                matches!(
                    &item.request.action,
                    PackAction::Select { .. }
                        | PackAction::Restore { .. }
                        | PackAction::ImportAndSelect { .. }
                ),
            )
        };
        let removing_active = matches!(
            &item.request.action,
            PackAction::Remove { id }
                if managed_active.is_some_and(|active| active.id.as_str() == id.as_str())
        );
        let apply = candidate_ref.is_some()
            && (changes_selection || explicit_override_clear || removing_active)
            && (!override_active || explicit_override_clear);
        let mut ui_applied = false;
        let mut apply_error = None;
        if apply {
            let applied = match prepared.as_ref() {
                Some(token) if token.reference == selected => {
                    ui::apply_character(token.clone(), item.cancel.clone()).and_then(|actual| {
                        if actual == *token {
                            Ok(())
                        } else {
                            Err("native Applied token mismatch".to_owned())
                        }
                    })
                }
                _ => Err("native candidate does not match committed selection".to_owned()),
            };
            if let Err(error) = applied {
                if let Some(token) = prepared.take() {
                    ui::discard_character(token);
                }
                apply_error = Some(format!("native apply failed after commit: {error}"));
            } else {
                ui_applied = true;
                self.acknowledge_applied(operation_id, selected.clone());
                prepared.take();
            }
        } else if let Some(token) = prepared.take() {
            ui::discard_character(token);
        }
        if let Some(error) = apply_error {
            self.update_operation(operation_id, |operation| {
                operation.error = Some(append_error(operation.error.take(), error));
            });
        }
        let allow_gc = !durability_unknown && (!apply || ui_applied);
        transaction.set_ui_applied(ui_applied);
        let finish_warning = match transaction.finish(allow_gc) {
            Ok(warning) => warning,
            Err(error) => {
                self.update_operation(operation_id, |operation| {
                    operation.error = Some(append_error(operation.error.take(), error));
                    operation.state = DURABILITY_UNKNOWN.to_owned();
                });
                return Ok(());
            }
        };
        self.update_operation(operation_id, |operation| {
            operation.state = if durability_unknown {
                DURABILITY_UNKNOWN.to_owned()
            } else if allow_gc {
                COMPLETED.to_owned()
            } else {
                COMMITTED_PENDING_APPLY.to_owned()
            };
            if let Some(warning) = finish_warning.clone() {
                operation.error = Some(append_error(operation.error.take(), warning));
            }
        });
        Ok(())
    }

    fn publish_download_progress(
        &self,
        operation_id: &str,
        phase: DownloadPhase,
        received: u64,
        total: u64,
    ) {
        let mut state = lock_unpoisoned(&self.state);
        let next = state
            .progress
            .entry(operation_id.to_owned())
            .or_insert_with(|| DownloadProgress {
                operation_id: operation_id.to_owned(),
                revision: 0,
                received: 0,
                total: 0,
                phase,
            });
        if next.phase == phase
            && next.total == total
            && received < total
            && received.saturating_sub(next.received) < 64 * 1024
        {
            return;
        }
        next.phase = phase;
        next.received = received;
        next.total = total;
        next.revision = next.revision.saturating_add(1);
        drop(state);
        ui::wake();
    }

    fn try_lock_io_gate(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        match self.io_gate.try_lock() {
            Ok(guard) => Ok(guard),
            Err(std::sync::TryLockError::WouldBlock) => {
                Err("pack store is busy preparing a character".to_owned())
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let guard = poisoned.into_inner();
                self.io_gate.clear_poison();
                Ok(guard)
            }
        }
    }

    /// Pair the last cancellation check with the transition to noncancelable
    /// commit under the mutex used by cancel_download and shutdown. No store
    /// or AppKit work runs while this mutex is held.
    fn begin_commit(&self, item: &QueuedRequest) -> bool {
        let mut state = lock_unpoisoned(&self.state);
        if self.stopping.load(Ordering::Acquire) || item.cancel.load(Ordering::Acquire) {
            return false;
        }
        state.commit_started = true;
        true
    }

    fn update_operation<F>(&self, operation_id: &str, update: F)
    where
        F: FnOnce(&mut PackOperation),
    {
        {
            let mut state = lock_unpoisoned(&self.state);
            if let Some(entry) = state.operations.get_mut(operation_id) {
                update(&mut entry.operation);
                entry.operation.error = entry.operation.error.take().map(bound_diagnostic);
            }
        }
        ui::wake();
    }

    /// Final publication precedes release of the retained cancellation handle
    /// and mutation admission. No memory guard survives the final UI wake.
    fn retire_operation(&self, operation_id: &str) {
        let mut state = lock_unpoisoned(&self.state);
        if let Some(entry) = state.operations.get_mut(operation_id) {
            entry.cancel = None;
        }
        state.progress.remove(operation_id);
        if state.active_operation.as_deref() == Some(operation_id) {
            state.active_operation = None;
            state.commit_started = false;
        }
        drop(state);
        ui::wake();
    }

    /// Verified reads are paired under one flock. An unconfirmed or mismatched
    /// read never erases previously observed commit or native ACK evidence.
    fn recover_aborted_operation(
        &self,
        operation_id: &str,
        recovered: Result<(PackListing, Option<PackOperation>), String>,
        known: Option<PackListing>,
        durability_unknown: bool,
        diagnostic: &str,
    ) {
        let (listing, durable, uncertain, read_error) = match recovered {
            Ok((listing, durable)) => {
                let matches_known = known.as_ref().is_none_or(|known| {
                    known.generation == listing.generation
                        && known.selected == listing.selected
                        && known.packs == listing.packs
                });
                if matches_known {
                    (Some(listing), durable, durability_unknown, None)
                } else {
                    (known, None, true, None)
                }
            }
            Err(error) => (known, None, true, Some(error)),
        };
        if let Some(listing) = listing {
            self.cache_listing(listing);
        }
        self.update_operation(operation_id, |operation| {
            if let Some(durable) = durable {
                if durable.committed {
                    operation.committed = true;
                    operation.generation = durable.generation.or(operation.generation);
                    operation.ui_applied |= durable.ui_applied;
                    if !uncertain {
                        operation.state = durable.state;
                        operation.error = durable.error;
                        return;
                    }
                }
            }
            operation.state = DURABILITY_UNKNOWN.to_owned();
            let detail = match read_error {
                Some(error) => format!("{diagnostic}; status lookup failed: {error}"),
                None => diagnostic.to_owned(),
            };
            operation.error = Some(append_error(operation.error.take(), detail));
        });
    }

    fn reconcile_durable_operation(&self, listing: PackListing, durable: PackOperation) {
        self.cache_listing(listing);
        let operation_id = durable.operation_id.clone();
        self.update_operation(&operation_id, |operation| {
            *operation = durable;
            operation.error = Some(append_error(
                operation.error.take(),
                EXISTING_OPERATION_NOT_EXECUTED.to_owned(),
            ));
        });
    }
    fn mark_durable_lookup_unknown(&self, operation_id: &str, detail: String) {
        self.update_operation(operation_id, |operation| {
            operation.state = DURABILITY_UNKNOWN.to_owned();
            operation.error = Some(append_error(
                operation.error.take(),
                format!(
                    "{EXISTING_OPERATION_NOT_EXECUTED}; prior status could not be verified: {detail}"
                ),
            ));
        });
    }
}

/// Execute a mutation synchronously on the process main thread.  It performs
/// the exact same store transaction and native renderer preflight as the live
/// worker, but never asks Ui to apply a native object; callers receive
/// `ui_applied = false` and can report that fact honestly.
pub fn execute_offline(
    config_dir: PathBuf,
    builtin_assets: PathBuf,
    request: PackRequest,
) -> Result<PackOperation, String> {
    if matches!(request.action, PackAction::ImportAndSelect { .. }) {
        return Err("official download and apply requires the live pack service".to_owned());
    }
    validate_operation_id(&request.operation_id)?;
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "offline pack mutations must run on the AppKit main thread".to_owned())?;
    let store = PackStore::new(config_dir, Some(builtin_assets));
    let listing = store.list().map_err(bound_diagnostic)?;
    let mut transaction = match store.begin(&request, listing.active.as_ref()) {
        Ok(transaction) => transaction,
        Err(error) => {
            return Ok(failed_operation(&request.operation_id, error));
        }
    };
    if let Some(candidate) = transaction.candidate.take() {
        let token = RendererToken::new(
            request.operation_id.clone(),
            transaction
                .candidate_ref
                .clone()
                .ok_or_else(|| "native candidate has no revision identity".to_owned())?,
            candidate.content_digest().to_owned(),
        )?;
        if let Err(error) = character_renderer::prepare(candidate, token, mtm) {
            let error = format!("native preflight failed: {error}");
            let _ = transaction.mark_failed(error.clone());
            return Ok(failed_operation(&request.operation_id, error));
        }
    }
    let commit = match transaction.commit() {
        Ok(commit) => commit,
        Err(error) => {
            return Ok(failed_operation(
                &request.operation_id,
                format!("registry commit failed: {error}"),
            ));
        }
    };
    let mut operation = PackOperation {
        operation_id: request.operation_id,
        state: if commit.durability_unknown || commit.error.is_some() {
            DURABILITY_UNKNOWN.to_owned()
        } else {
            COMPLETED.to_owned()
        },
        committed: true,
        ui_applied: false,
        generation: Some(commit.generation),
        error: commit.error,
    };
    transaction.set_ui_applied(false);
    match transaction.finish(true) {
        Ok(Some(warning)) => {
            operation.error = Some(append_error(operation.error.take(), warning));
        }
        Ok(None) => {}
        Err(error) => {
            operation.state = DURABILITY_UNKNOWN.to_owned();
            operation.error = Some(append_error(operation.error.take(), error));
        }
    }
    Ok(bound_operation(operation))
}
/// The bundled manifest ID can differ from the registry's reserved "default".
fn builtin_manifest_id(root: &std::path::Path) -> String {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(root.join("manifest.json")) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if file.take(64 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 64 * 1024 {
        return String::new();
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|manifest| {
            manifest
                .get("id")
                .and_then(|id| id.as_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

fn bound_diagnostic(mut diagnostic: String) -> String {
    if diagnostic.len() <= MAX_DIAGNOSTIC_BYTES {
        return diagnostic;
    }
    const ELLIPSIS: &str = "…";
    let mut keep = MAX_DIAGNOSTIC_BYTES - ELLIPSIS.len();
    while keep > 0 && !diagnostic.is_char_boundary(keep) {
        keep -= 1;
    }
    diagnostic.truncate(keep);
    diagnostic.push_str(ELLIPSIS);
    diagnostic
}

fn bound_operation(mut operation: PackOperation) -> PackOperation {
    operation.error = operation.error.take().map(bound_diagnostic);
    operation
}

fn bound_listing(mut listing: PackListing) -> PackListing {
    listing.error = listing.error.take().map(bound_diagnostic);
    listing
}

fn failed_operation(operation_id: &str, error: String) -> PackOperation {
    PackOperation {
        operation_id: operation_id.to_owned(),
        state: FAILED.to_owned(),
        committed: false,
        ui_applied: false,
        generation: None,
        error: Some(bound_diagnostic(error)),
    }
}

fn retain_capacity(state: &mut ServiceState) {
    let scan_limit = state.order.len();
    for _ in 0..scan_limit {
        if state.operations.len() < MAX_RETAINED_OPERATIONS {
            break;
        }
        let Some(id) = state.order.pop_front() else {
            break;
        };
        let removable = state
            .operations
            .get(&id)
            .map(|entry| {
                is_terminal(&entry.operation.state)
                    && entry.cancel.is_none()
                    && state.active_operation.as_deref() != Some(id.as_str())
            })
            .unwrap_or(true);
        if removable {
            state.operations.remove(&id);
        } else {
            state.order.push_back(id);
        }
    }
}

fn is_terminal(state: &str) -> bool {
    matches!(
        state,
        COMPLETED | FAILED | CANCELED | COMMITTED_PENDING_APPLY | DURABILITY_UNKNOWN
    )
}

fn validate_operation_id(operation_id: &str) -> Result<(), String> {
    if operation_id.is_empty() {
        return Err("pack operation id must not be empty".to_owned());
    }
    if operation_id.len() > MAX_OPERATION_ID_BYTES {
        return Err("pack operation id is too long".to_owned());
    }
    if operation_id.chars().any(|character| character.is_control()) {
        return Err("pack operation id contains a control character".to_owned());
    }
    Ok(())
}

fn append_error(previous: Option<String>, next: String) -> String {
    let combined = match previous {
        Some(previous) if !previous.is_empty() => format!("{previous}; {next}"),
        _ => next,
    };
    bound_diagnostic(combined)
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn mutation_busy(state: &ServiceState, queue: &VecDeque<QueuedRequest>) -> bool {
    state.active_operation.is_some()
        || !queue.is_empty()
        || state
            .operations
            .values()
            .any(|entry| !is_terminal(&entry.operation.state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_service() -> Arc<PackService> {
        let service = PackService::new(PathBuf::new(), PathBuf::new(), None);
        service.started.store(true, Ordering::Release);
        lock_unpoisoned(&service.state).ui_ready = true;
        service
    }

    fn request(id: &str) -> PackRequest {
        PackRequest {
            operation_id: id.to_owned(),
            expected_generation: None,
            action: PackAction::Select {
                id: "character".to_owned(),
            },
        }
    }

    #[test]
    fn update_blocker_keeps_dequeued_pack_commit_and_metadata_visible() {
        let root = tempfile::tempdir().unwrap();
        let service =
            PackService::new(root.path().to_path_buf(), root.path().join("builtin"), None);
        service.started.store(true, Ordering::Release);
        lock_unpoisoned(&service.state).ui_ready = true;
        assert!(!service.has_pending_update_work());

        service.submit(request("pack-update-blocker")).unwrap();
        assert!(service.has_pending_update_work());
        let _dequeued = lock_unpoisoned(&service.queue).pop_front().unwrap();
        assert!(service.has_pending_update_work());
        {
            let mut state = lock_unpoisoned(&service.state);
            let operation = &mut state
                .operations
                .get_mut("pack-update-blocker")
                .unwrap()
                .operation;
            operation.state = APPLYING.into();
            operation.committed = true;
            state.commit_started = true;
        }
        assert!(service.has_pending_update_work());
        {
            let mut state = lock_unpoisoned(&service.state);
            state
                .operations
                .get_mut("pack-update-blocker")
                .unwrap()
                .operation
                .state = COMPLETED.into();
            state.commit_started = false;
        }
        assert!(!service.has_pending_update_work());
        lock_unpoisoned(&service.state)
            .operations
            .get_mut("pack-update-blocker")
            .unwrap()
            .operation
            .state = DURABILITY_UNKNOWN.into();
        assert!(service.has_pending_update_work());
        lock_unpoisoned(&service.state)
            .operations
            .get_mut("pack-update-blocker")
            .unwrap()
            .operation
            .state = COMPLETED.into();
        lock_unpoisoned(&service.metadata).loading = Some(MetadataKey {
            reference: CharacterRef::builtin(),
            generation: 1,
        });
        assert!(service.has_pending_update_work());
        lock_unpoisoned(&service.metadata).loading = None;
        assert!(!service.has_pending_update_work());
    }

    #[test]
    fn official_import_is_rejected_offline_before_store_or_native_access() {
        let request = PackRequest {
            operation_id: "offline-official".to_owned(),
            expected_generation: Some(0),
            action: PackAction::ImportAndSelect {
                official: crate::character_types::OfficialPackIdentity {
                    id: "official-cat".to_owned(),
                    version: "0.0.2".to_owned(),
                    release_tag: "v0.0.2".to_owned(),
                    sha256: "a".repeat(64),
                },
            },
        };
        assert!(execute_offline(PathBuf::new(), PathBuf::new(), request)
            .unwrap_err()
            .contains("live pack service"));
    }

    #[test]
    fn canceled_queued_official_import_never_changes_registry() {
        let root = tempfile::tempdir().unwrap();
        let service =
            PackService::new(root.path().to_path_buf(), root.path().join("builtin"), None);
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        let operation_id = "cancel-official-before-commit";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::ImportAndSelect {
                    official: crate::character_types::OfficialPackIdentity {
                        id: "official-cat".to_owned(),
                        version: "0.0.2".to_owned(),
                        release_tag: "v0.0.2".to_owned(),
                        sha256: "a".repeat(64),
                    },
                },
            })
            .unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        assert!(lock_unpoisoned(&service.state).active_operation.is_none());
        assert!(service.cancel_download(operation_id));
        assert!(item.cancel.load(Ordering::Acquire));
        service.execute(item);
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(status.state, CANCELED);
        assert!(!status.committed);
        assert!(!status.ui_applied);
        assert!(service.cached_download_progress(operation_id).is_none());
        let after = service.list().unwrap();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.selected, before.selected);
        assert_eq!(after.packs, before.packs);
        assert_eq!(after.active, before.active);
        assert_eq!(after.active, Some(CharacterRef::builtin()));
        assert!(PackStore::new(root.path().to_path_buf(), None)
            .operation_status(operation_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn shutdown_cancels_dequeued_official_handle_before_active_registration() {
        let root = tempfile::tempdir().unwrap();
        let (store, _reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        let operation_id = "shutdown-dequeued-official";
        let request = PackRequest {
            operation_id: operation_id.to_owned(),
            expected_generation: Some(before.generation),
            action: PackAction::ImportAndSelect {
                official: crate::character_types::OfficialPackIdentity {
                    id: "official-cat".to_owned(),
                    version: "0.0.2".to_owned(),
                    release_tag: "v0.0.2".to_owned(),
                    sha256: "a".repeat(64),
                },
            },
        };
        service.submit(request.clone()).unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        assert!(Arc::ptr_eq(
            lock_unpoisoned(&service.state)
                .operations
                .get(operation_id)
                .unwrap()
                .cancel
                .as_ref()
                .unwrap(),
            &item.cancel
        ));
        assert_eq!(service.submit(request).unwrap().state, ACCEPTED);
        assert!(lock_unpoisoned(&service.queue).is_empty());
        service.shutdown();
        assert!(item.cancel.load(Ordering::Acquire));
        service.execute(item);
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(status.state, CANCELED);
        assert!(!status.committed);
        assert!(!service.ui_mutation_busy());
        let after = service.cached_list();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.packs, before.packs);
        assert_eq!(after.selected, before.selected);
        assert_eq!(after.active, before.active);
        assert!(store.operation_status(operation_id).unwrap().is_none());
    }

    #[test]
    fn cancellation_boundary_keeps_native_token_live_once_commit_starts() {
        let official_request = |operation_id: &str| PackRequest {
            operation_id: operation_id.to_owned(),
            expected_generation: Some(0),
            action: PackAction::ImportAndSelect {
                official: crate::character_types::OfficialPackIdentity {
                    id: "official-cat".to_owned(),
                    version: "0.0.2".to_owned(),
                    release_tag: "v0.0.2".to_owned(),
                    sha256: "a".repeat(64),
                },
            },
        };
        let service = ready_service();
        service.submit(official_request("before-commit")).unwrap();
        let before = lock_unpoisoned(&service.queue).pop_front().unwrap();
        {
            let mut state = lock_unpoisoned(&service.state);
            state.active_operation = Some(before.request.operation_id.clone());
            state
                .operations
                .get_mut("before-commit")
                .unwrap()
                .operation
                .state = PREPARING.to_owned();
        }
        assert_eq!(
            service.cached_status("before-commit").unwrap().state,
            PREPARING
        );
        assert!(service.cancel_download("before-commit"));
        assert!(!service.begin_commit(&before));
        assert!(before.cancel.load(Ordering::Acquire));
        service.update_operation("before-commit", |operation| {
            operation.state = CANCELED.to_owned();
        });
        service.retire_operation("before-commit");

        service.submit(official_request("commit-started")).unwrap();
        let committing = lock_unpoisoned(&service.queue).pop_front().unwrap();
        {
            let mut state = lock_unpoisoned(&service.state);
            state.active_operation = Some(committing.request.operation_id.clone());
        }
        assert!(service.begin_commit(&committing));
        // The durable committed flag has not yet been published. A late
        // cancellation must nevertheless leave the native apply token live.
        assert!(!service.cached_status("commit-started").unwrap().committed);
        let cancel_service = Arc::clone(&service);
        assert!(
            !thread::spawn(move || cancel_service.cancel_download("commit-started"))
                .join()
                .unwrap()
        );
        assert!(!committing.cancel.load(Ordering::Acquire));

        // Shutdown must not cancel that token or wait for a store read gate:
        // the committing worker may own the gate for its entire transaction.
        let _gate = lock_unpoisoned(&service.io_gate);
        service.shutdown();
        assert!(!committing.cancel.load(Ordering::Acquire));
        service.update_operation("commit-started", |operation| {
            operation.state = FAILED.to_owned();
            operation.error = Some("cutoff fixture did not execute a store transaction".to_owned());
        });
        service.retire_operation("commit-started");
        assert!(!lock_unpoisoned(&service.state).commit_started);
    }

    #[test]
    fn ui_admission_rejects_accepted_cli_work_but_cli_can_queue_after_ui() {
        let service = ready_service();
        service.submit(request("cli-first")).unwrap();
        assert!(service.ui_mutation_busy());
        assert!(service.submit_ui_if_idle(request("ui-rejected")).is_err());
        assert!(service.cached_status("ui-rejected").is_none());
        assert_eq!(lock_unpoisoned(&service.queue).len(), 1);

        let service = ready_service();
        service.submit_ui_if_idle(request("ui-first")).unwrap();
        assert!(service.submit_ui_if_idle(request("ui-second")).is_err());
        assert!(service.cached_status("ui-second").is_none());
        assert_eq!(
            service.submit(request("cli-second")).unwrap().state,
            ACCEPTED
        );
        assert_eq!(lock_unpoisoned(&service.queue).len(), 2);
    }

    #[test]
    fn ui_admission_covers_worker_pop_and_terminal_publication() {
        let service = ready_service();
        service.submit(request("active")).unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        assert!(service.ui_mutation_busy());
        assert!(service.submit_ui_if_idle(request("just-popped")).is_err());
        service.update_operation("active", |operation| {
            operation.state = PREPARING.to_owned();
        });
        assert!(service.ui_mutation_busy());
        assert!(service.submit_ui_if_idle(request("while-active")).is_err());
        lock_unpoisoned(&service.state).active_operation = Some(item.request.operation_id);
        service.update_operation("active", |operation| {
            operation.state = COMPLETED.to_owned();
        });
        assert!(service.ui_mutation_busy());
        assert!(service
            .submit_ui_if_idle(request("while-finalizing"))
            .is_err());
        service.retire_operation("active");
        assert!(!service.ui_mutation_busy());
        assert_eq!(
            service
                .submit_ui_if_idle(request("after-finalization"))
                .unwrap()
                .state,
            ACCEPTED
        );
    }

    #[test]
    fn final_ui_wake_observes_released_admission_after_canceled_execute() {
        let service = ready_service();
        service.submit(request("canceled")).unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        item.cancel.store(true, Ordering::Release);
        let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let service = Arc::clone(&service);
            let observed = std::rc::Rc::clone(&observed);
            ui::WAKE_OBSERVER.with(|observer| {
                *observer.borrow_mut() = Some(Box::new(move || {
                    observed.borrow_mut().push(service.ui_mutation_busy());
                }));
            });
        }
        service.execute(item);
        ui::WAKE_OBSERVER.with(|observer| observer.borrow_mut().take());

        assert_eq!(service.cached_status("canceled").unwrap().state, CANCELED);
        assert_eq!(observed.borrow().last(), Some(&false));
        assert!(!service.ui_mutation_busy());
    }

    #[test]
    fn failed_dialogue_metadata_is_retained_without_automatic_retry() {
        let service = ready_service();
        *lock_unpoisoned(&service.cache) = Some(PackListing {
            generation: 7,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: Vec::new(),
            error: None,
        });
        let key = MetadataKey {
            reference: CharacterRef::builtin(),
            generation: 7,
        };
        service
            .request_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert_eq!(
            lock_unpoisoned(&service.metadata).pending.pop_front(),
            Some(key.clone())
        );
        lock_unpoisoned(&service.metadata).loading = Some(key.clone());
        service.publish_dialogue_metadata(key.clone(), Err("invalid manifest".to_owned()));
        for _ in 0..3 {
            service
                .request_dialogue_metadata(key.reference.clone(), 7)
                .unwrap();
            assert!(lock_unpoisoned(&service.metadata).pending.is_empty());
            let result = service.cached_dialogue_metadata(&key.reference, 7).unwrap();
            assert!(matches!(&result, Err(error) if error == "invalid manifest"));
        }

        // A new read/user event retries once, without making frame refreshes
        // or another simultaneous explicit event enqueue a second lookup.
        service
            .retry_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert!(service
            .cached_dialogue_metadata(&key.reference, 7)
            .is_none());
        service
            .retry_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        service
            .request_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert_eq!(lock_unpoisoned(&service.metadata).pending.len(), 1);
        assert_eq!(
            lock_unpoisoned(&service.metadata).pending.pop_front(),
            Some(key.clone())
        );
        lock_unpoisoned(&service.metadata).loading = Some(key.clone());
        service
            .retry_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert!(lock_unpoisoned(&service.metadata).pending.is_empty());
        service.publish_dialogue_metadata(key.clone(), Err("still Busy".to_owned()));
        service
            .request_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert!(matches!(
            service.cached_dialogue_metadata(&key.reference, 7),
            Some(Err(error)) if error == "still Busy"
        ));
        assert!(lock_unpoisoned(&service.metadata).pending.is_empty());

        service
            .retry_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert!(service
            .cached_dialogue_metadata(&key.reference, 7)
            .is_none());
        assert_eq!(
            lock_unpoisoned(&service.metadata).pending.pop_front(),
            Some(key.clone())
        );
        lock_unpoisoned(&service.metadata).loading = Some(key.clone());
        service.publish_dialogue_metadata(key.clone(), Ok(DialogueMetadata { metadata: None }));
        service
            .retry_dialogue_metadata(key.reference.clone(), 7)
            .unwrap();
        assert!(lock_unpoisoned(&service.metadata).pending.is_empty());
        assert!(service
            .cached_dialogue_metadata(&key.reference, 7)
            .unwrap()
            .is_ok());
    }

    #[test]
    fn explicit_retry_preserves_cached_failure_when_fifo_is_full() {
        use crate::character_types::PackRecord;

        let service = ready_service();
        *lock_unpoisoned(&service.cache) = Some(PackListing {
            generation: 7,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: vec![PackRecord {
                id: "forest".to_owned(),
                name: "Forest".to_owned(),
                head: 16,
                revisions: (1..=16).collect(),
            }],
            error: None,
        });
        let failed = CharacterRef::builtin();
        service
            .request_dialogue_metadata(failed.clone(), 7)
            .unwrap();
        let key = lock_unpoisoned(&service.metadata)
            .pending
            .pop_front()
            .unwrap();
        lock_unpoisoned(&service.metadata).loading = Some(key.clone());
        service.publish_dialogue_metadata(key, Err("transient Busy".to_owned()));
        for revision in 1..=MAX_PENDING_METADATA as u64 {
            service
                .request_dialogue_metadata(
                    CharacterRef {
                        id: "forest".to_owned(),
                        revision,
                    },
                    7,
                )
                .unwrap();
        }
        assert!(service
            .retry_dialogue_metadata(failed.clone(), 7)
            .unwrap_err()
            .starts_with("Busy:"));
        assert!(matches!(
            service.cached_dialogue_metadata(&failed, 7),
            Some(Err(error)) if error == "transient Busy"
        ));
        {
            let mut metadata = lock_unpoisoned(&service.metadata);
            assert_eq!(metadata.pending.len(), MAX_PENDING_METADATA);
            assert_eq!(metadata.pending.pop_front().unwrap().reference.revision, 1);
        }
        service.retry_dialogue_metadata(failed.clone(), 7).unwrap();
        let metadata = lock_unpoisoned(&service.metadata);
        assert_eq!(metadata.pending.len(), MAX_PENDING_METADATA);
        assert_eq!(metadata.pending.front().unwrap().reference.revision, 2);
        assert_eq!(metadata.pending.back().unwrap().reference, failed);
        assert!(metadata.results.is_empty());
    }

    #[test]
    fn metadata_fifo_retains_each_revision_and_drops_stale_generations() {
        use crate::character_types::PackRecord;

        let service = ready_service();
        let listing = |generation| PackListing {
            generation,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: vec![PackRecord {
                id: "forest".to_owned(),
                name: "Forest".to_owned(),
                head: 32,
                revisions: (1..=32).collect(),
            }],
            error: None,
        };
        *lock_unpoisoned(&service.cache) = Some(listing(7));
        let gui = CharacterRef::builtin();
        service.request_dialogue_metadata(gui.clone(), 7).unwrap();
        for revision in 1..MAX_PENDING_METADATA as u64 {
            service
                .request_dialogue_metadata(
                    CharacterRef {
                        id: "forest".to_owned(),
                        revision,
                    },
                    7,
                )
                .unwrap();
        }
        assert!(service
            .request_dialogue_metadata(
                CharacterRef {
                    id: "forest".to_owned(),
                    revision: 16,
                },
                7
            )
            .unwrap_err()
            .starts_with("Busy:"));
        assert_eq!(
            lock_unpoisoned(&service.metadata)
                .pending
                .front()
                .unwrap()
                .reference,
            gui
        );
        let first = lock_unpoisoned(&service.metadata)
            .pending
            .pop_front()
            .unwrap();
        lock_unpoisoned(&service.metadata).loading = Some(first.clone());
        service.publish_dialogue_metadata(
            first,
            Ok(DialogueMetadata {
                metadata: Some(CharacterMetadata { dialogue: None }),
            }),
        );
        for revision in 1..MAX_PENDING_METADATA as u64 {
            let key = lock_unpoisoned(&service.metadata)
                .pending
                .pop_front()
                .unwrap();
            assert_eq!(key.reference.revision, revision);
            lock_unpoisoned(&service.metadata).loading = Some(key.clone());
            service.publish_dialogue_metadata(
                key,
                Ok(DialogueMetadata {
                    metadata: Some(CharacterMetadata {
                        dialogue: Some(std::collections::BTreeMap::from([(
                            "en".to_owned(),
                            crate::assets::DialogueLocale {
                                phases: Some(std::collections::BTreeMap::from([(
                                    "idle".to_owned(),
                                    format!("revision-{revision}"),
                                )])),
                                reactions: None,
                            },
                        )])),
                    }),
                }),
            );
        }
        assert!(service
            .cached_dialogue_metadata(&gui, 7)
            .unwrap()
            .unwrap()
            .metadata
            .is_some());
        for revision in 1..MAX_PENDING_METADATA as u64 {
            let reference = CharacterRef {
                id: "forest".to_owned(),
                revision,
            };
            let metadata = service
                .cached_dialogue_metadata(&reference, 7)
                .unwrap()
                .unwrap()
                .metadata
                .unwrap();
            assert_eq!(
                metadata.dialogue_text("idle", None, "en"),
                Some(format!("revision-{revision}").as_str())
            );
        }
        let stale = MetadataKey {
            reference: gui.clone(),
            generation: 7,
        };
        lock_unpoisoned(&service.metadata).loading = Some(stale.clone());
        service.cache_listing(listing(8));
        service.publish_dialogue_metadata(stale, Ok(DialogueMetadata { metadata: None }));
        assert!(service.cached_dialogue_metadata(&gui, 7).unwrap().is_err());
        assert!(service.cached_dialogue_metadata(&gui, 8).is_none());
        service.request_dialogue_metadata(gui, 8).unwrap();
        assert_eq!(
            lock_unpoisoned(&service.metadata)
                .pending
                .front()
                .unwrap()
                .generation,
            8
        );
    }

    #[test]
    fn invalidated_lookup_cannot_publish_when_generation_returns() {
        let service = ready_service();
        let listing = |generation| PackListing {
            generation,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: Vec::new(),
            error: None,
        };
        service.cache_listing(listing(7));
        let reference = CharacterRef::builtin();
        service
            .request_dialogue_metadata(reference.clone(), 7)
            .unwrap();
        let stale = lock_unpoisoned(&service.metadata)
            .pending
            .pop_front()
            .unwrap();
        lock_unpoisoned(&service.metadata).loading = Some(stale.clone());
        service.cache_listing(listing(8));
        assert!(service
            .retry_dialogue_metadata(reference.clone(), 7)
            .is_err());
        service.cache_listing(listing(7));
        service
            .retry_dialogue_metadata(reference.clone(), 7)
            .unwrap();
        service.publish_dialogue_metadata(stale, Err("stale worker failure".to_owned()));
        assert!(service.cached_dialogue_metadata(&reference, 7).is_none());
        let fresh = lock_unpoisoned(&service.metadata)
            .pending
            .pop_front()
            .unwrap();
        lock_unpoisoned(&service.metadata).loading = Some(fresh.clone());
        service.publish_dialogue_metadata(fresh, Ok(DialogueMetadata { metadata: None }));
        assert!(service
            .cached_dialogue_metadata(&reference, 7)
            .unwrap()
            .is_ok());
    }

    #[test]
    fn ui_admission_preserves_exact_id_idempotency_and_collision() {
        let service = ready_service();
        let first = request("same");
        assert_eq!(
            service.submit_ui_if_idle(first.clone()).unwrap().state,
            ACCEPTED
        );
        assert_eq!(
            service.submit_ui_if_idle(first.clone()).unwrap().state,
            ACCEPTED
        );
        let mut changed = first;
        changed.expected_generation = Some(1);
        assert!(service.submit_ui_if_idle(changed.clone()).is_err());
        assert!(service.submit(changed.clone()).is_err());
        {
            let queue = lock_unpoisoned(&service.queue);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue.front().unwrap().request.operation_id, "same");
            assert_eq!(queue.front().unwrap().request.expected_generation, None);
        }
        assert_eq!(service.cached_status("same").unwrap().state, ACCEPTED);
        lock_unpoisoned(&service.queue).pop_front();
        service.update_operation("same", |operation| {
            operation.state = COMPLETED.to_owned();
        });
        service.retire_operation("same");
        assert!(service.submit_ui_if_idle(changed).is_err());
        assert!(lock_unpoisoned(&service.queue).is_empty());
        assert_eq!(service.cached_status("same").unwrap().state, COMPLETED);
    }

    #[test]
    fn concurrent_ui_admission_allows_exactly_one_new_request() {
        let service = ready_service();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|id| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    service.submit_ui_if_idle(request(id)).is_ok()
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|accepted| *accepted)
                .count(),
            1
        );
        assert_eq!(lock_unpoisoned(&service.queue).len(), 1);
    }

    #[test]
    fn terminal_cli_uncertainty_does_not_block_unrelated_ui_admission() {
        let service = ready_service();
        service.submit(request("old-cli")).unwrap();
        lock_unpoisoned(&service.queue).pop_front();
        service.update_operation("old-cli", |operation| {
            operation.state = DURABILITY_UNKNOWN.to_owned();
        });
        service.retire_operation("old-cli");
        assert_eq!(
            service.submit_ui_if_idle(request("new-ui")).unwrap().state,
            ACCEPTED
        );
    }

    #[test]
    fn operation_ids_reject_controls_and_empty_values() {
        assert!(validate_operation_id("").is_err());
        assert!(validate_operation_id("ok\nno").is_err());
        assert!(validate_operation_id("ok").is_ok());
    }

    #[test]
    fn diagnostics_are_utf8_safe_and_bounded() {
        let bounded = bound_diagnostic("é".repeat(MAX_DIAGNOSTIC_BYTES));
        assert!(bounded.len() <= MAX_DIAGNOSTIC_BYTES);
        assert!(bounded.ends_with('…'));
        assert!(bounded.is_char_boundary(bounded.len()));

        let operation = bound_operation(PackOperation {
            operation_id: "op".to_owned(),
            state: FAILED.to_owned(),
            committed: false,
            ui_applied: false,
            generation: None,
            error: Some("x".repeat(MAX_DIAGNOSTIC_BYTES + 1)),
        });
        assert!(operation
            .error
            .as_ref()
            .is_some_and(|error| error.len() <= MAX_DIAGNOSTIC_BYTES));
    }

    #[test]
    fn concurrent_shutdown_callers_wait_for_the_worker_join() {
        let service = PackService::new(PathBuf::new(), PathBuf::new(), None);
        {
            let mut done = lock_unpoisoned(&service.worker_done.0);
            *done = false;
        }
        let weak = Arc::downgrade(&service);
        let worker = thread::spawn(move || worker_loop(weak));
        *lock_unpoisoned(&service.worker) = Some(worker);

        let first_service = Arc::clone(&service);
        let second_service = Arc::clone(&service);
        let first = thread::spawn(move || first_service.shutdown());
        let second = thread::spawn(move || second_service.shutdown());
        first.join().expect("first shutdown caller panicked");
        second.join().expect("second shutdown caller panicked");

        assert!(service.stopping.load(Ordering::Acquire));
        assert!(*lock_unpoisoned(&service.worker_done.0));
        assert!(lock_unpoisoned(&service.worker).is_none());
    }

    #[test]
    fn retained_state_evicts_only_retired_terminal_results() {
        let mut state = ServiceState::default();
        for index in 0..MAX_RETAINED_OPERATIONS {
            let id = format!("{index}");
            state.order.push_back(id.clone());
            state.operations.insert(
                id.clone(),
                OperationEntry {
                    fingerprint: Vec::new(),
                    operation: PackOperation {
                        operation_id: id,
                        state: if index == 0 {
                            PREPARING.to_owned()
                        } else {
                            COMPLETED.to_owned()
                        },
                        committed: index != 0,
                        ui_applied: false,
                        generation: None,
                        error: None,
                    },
                    cancel: (index <= 1).then(|| Arc::new(AtomicBool::new(false))),
                    is_official: false,
                },
            );
        }
        state.active_operation = Some("2".to_owned());
        retain_capacity(&mut state);
        assert!(state.operations.contains_key("0"));
        assert!(state.operations.contains_key("1"));
        assert!(state.operations.contains_key("2"));
        assert_eq!(state.operations.len(), MAX_RETAINED_OPERATIONS - 1);
    }
    #[test]
    fn portrait_gate_contention_is_pending_without_touching_durable_store() {
        let root = tempfile::tempdir().unwrap();
        let service =
            PackService::new(root.path().to_path_buf(), root.path().join("builtin"), None);
        let initial = service.list().unwrap();
        let gate = lock_unpoisoned(&service.io_gate);
        assert!(service
            .load_preview(&CharacterRef::builtin(), initial.generation)
            .unwrap()
            .is_none());
        assert_eq!(service.cached_list().generation, initial.generation);
        drop(gate);
        // The same request now reaches the generation-pinned store lookup;
        // contention was not converted into an image or durability failure.
        assert!(service
            .load_preview(&CharacterRef::builtin(), initial.generation + 1)
            .unwrap_err()
            .contains("generation mismatch"));
        assert_eq!(service.list().unwrap().generation, initial.generation);
    }

    fn write_private_fixture(path: &std::path::Path, bytes: &[u8]) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn installed_unselected_png(root: &std::path::Path) -> (PackStore, CharacterRef, String) {
        let source = root.join("source");
        std::fs::create_dir(&source).unwrap();
        let manifest = serde_json::json!({
            "version": 3,
            "format": "herdr.character",
            "id": "fixture-cat",
            "name": "Fixture Cat",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["frame.png"]},
                "running": {"fps": 8, "frames": ["frame.png"]},
                "waiting": {"fps": 8, "frames": ["frame.png"]},
                "unknown": {"fps": 8, "frames": ["frame.png"]}
            }
        });
        write_private_fixture(
            &source.join("manifest.json"),
            &serde_json::to_vec(&manifest).unwrap(),
        );
        let mut frame = Vec::new();
        {
            let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut frame), 384, 512);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![255; 384 * 512]).unwrap();
        }
        write_private_fixture(&source.join("frame.png"), &frame);
        let original_digest = ManagedPack::load(&source)
            .unwrap()
            .assets
            .content_digest()
            .to_owned();
        let store = PackStore::new(root.join("config"), Some(source.clone()));
        let mut import = store
            .begin(
                &PackRequest {
                    operation_id: "seed-unselected-png".to_owned(),
                    expected_generation: Some(0),
                    action: PackAction::Import { path: source },
                },
                None,
            )
            .unwrap();
        let reference = import.candidate_ref.clone().unwrap();
        import.commit().unwrap();
        import.finish(true).unwrap();
        drop(import);
        assert_eq!(store.list().unwrap().selected, CharacterRef::builtin());
        (store, reference, original_digest)
    }

    struct RegistryFlockChild {
        child: std::process::Child,
        _stdout: std::io::BufReader<std::process::ChildStdout>,
    }

    impl RegistryFlockChild {
        fn release(&mut self) {
            use std::io::Write;
            self.child.stdin.take().unwrap().write_all(b"x").unwrap();
            assert!(self.child.wait().unwrap().success());
        }
    }

    impl Drop for RegistryFlockChild {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    fn hold_external_registry_flock(config: &std::path::Path) -> RegistryFlockChild {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let lock = config.join("characters").join(".registry.lock");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "character_service::tests::registry_flock_child_fixture",
            ])
            .env("HERDR_TEST_REGISTRY_FLOCK", lock)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut child = RegistryFlockChild {
            child,
            _stdout: std::io::BufReader::new(stdout),
        };
        loop {
            let mut line = String::new();
            assert_ne!(
                child._stdout.read_line(&mut line).unwrap(),
                0,
                "lock child exited before locking"
            );
            if line.contains("REGISTRY_FLOCK_HELD") {
                break;
            }
        }
        child
    }

    #[test]
    #[ignore = "child-only external registry flock fixture"]
    fn registry_flock_child_fixture() {
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        let path = std::env::var("HERDR_TEST_REGISTRY_FLOCK")
            .expect("registry flock child requires HERDR_TEST_REGISTRY_FLOCK");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) }, 0);
        std::io::stdout()
            .write_all(b"REGISTRY_FLOCK_HELD\n")
            .unwrap();
        std::io::stdin().read_exact(&mut [0]).unwrap();
    }

    #[test]
    fn external_flock_defers_preview_but_preserves_real_assets() {
        let root = tempfile::tempdir().unwrap();
        let (store, reference, digest) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        let generation = service.list().unwrap().generation;
        let mut child = hold_external_registry_flock(&root.path().join("config"));
        assert!(service.try_lock_io_gate().is_ok(), "service gate is free");
        assert!(service
            .load_preview(&reference, generation)
            .expect("flock contention is pending, not an image error")
            .is_none());
        assert!(service.list().unwrap_err().contains("Busy:"));
        let archive = root.path().join("export.herdrchar");
        assert!(store
            .export(&reference, &archive)
            .unwrap_err()
            .contains("Busy:"));
        assert!(!archive.exists());
        child.release();
        let assets = service
            .load_preview(&reference, generation)
            .unwrap()
            .expect("the same reference loads once the flock is free");
        assert!(matches!(&assets, crate::assets::ValidatedCharacter::Png(_)));
        assert_eq!(assets.content_digest(), digest);
        store.export(&reference, &archive).unwrap();
        assert!(archive.is_file());
    }

    fn remove_publishes_while_contended(external_flock: bool) {
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::Duration;
        let root = tempfile::tempdir().unwrap();
        let (store, reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        assert_eq!(before.packs.len(), 1);
        let operation_id = "remove-unselected-while-reader-owns-lock";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove {
                    id: reference.id.clone(),
                },
            })
            .unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        let (boundary_tx, boundary_rx) = mpsc::sync_channel(0);
        let (resume_tx, resume_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        *lock_unpoisoned(&service.after_transaction_scope) = Some(Box::new(move || {
            boundary_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        }));
        let executing = Arc::clone(&service);
        let worker = thread::spawn(move || {
            executing.execute(item);
            done_tx.send(()).unwrap();
        });
        boundary_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("real Remove reached transaction scope boundary");
        let gate = if external_flock {
            None
        } else {
            Some(lock_unpoisoned(&service.io_gate))
        };
        let mut child =
            external_flock.then(|| hold_external_registry_flock(&root.path().join("config")));
        if external_flock {
            assert!(
                service.try_lock_io_gate().is_ok(),
                "external flock leaves service gate free"
            );
        }
        resume_tx.send(()).unwrap();
        let done = done_rx.recv_timeout(Duration::from_secs(20));
        if done == Err(RecvTimeoutError::Timeout) {
            drop(gate);
            if let Some(child) = child.as_mut() {
                child.release();
            }
            worker.join().unwrap();
            panic!("Remove did not finalize while the independent reader held its lock");
        }
        done.unwrap();
        worker.join().unwrap();
        let cached = service.cached_list();
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(cached.generation, before.generation + 1);
        assert!(cached.packs.is_empty());
        assert_eq!(cached.selected, before.selected);
        assert_eq!(cached.active, before.active);
        assert_eq!(status.state, COMPLETED);
        assert!(status.committed);
        assert_eq!(status.generation, Some(cached.generation));
        assert!(!service.ui_mutation_busy());
        drop(gate);
        if let Some(child) = child.as_mut() {
            child.release();
        }
        let durable = store.list().unwrap();
        assert_eq!(durable.generation, cached.generation);
        assert_eq!(durable.packs, cached.packs);
        assert_eq!(
            store.operation_status(operation_id).unwrap().unwrap().state,
            COMPLETED
        );
    }

    #[test]
    fn remove_publishes_committed_cache_before_reader_reacquires_service_gate() {
        remove_publishes_while_contended(false);
    }

    #[test]
    fn remove_publishes_committed_cache_before_external_flock_reacquisition() {
        remove_publishes_while_contended(true);
    }

    #[test]
    fn existing_durable_id_publishes_verified_listing_without_reexecuting_mutation() {
        let root = tempfile::tempdir().unwrap();
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        let (store, reference, _) = installed_unselected_png(root.path());
        let durable = store.list().unwrap();
        assert_eq!(durable.generation, before.generation + 1);
        assert_eq!(service.cached_list().generation, before.generation);
        let operation_id = "seed-unselected-png";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove { id: reference.id },
            })
            .unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        service.execute(item);
        let cached = service.cached_list();
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(cached.generation, durable.generation);
        assert_eq!(cached.packs, durable.packs);
        assert_eq!(cached.active, before.active);
        assert_eq!(status.state, COMPLETED);
        assert!(status
            .error
            .unwrap()
            .contains(EXISTING_OPERATION_NOT_EXECUTED));
        assert!(!service.ui_mutation_busy());
        assert_eq!(store.list().unwrap().generation, durable.generation);
        assert_eq!(
            store.operation_status(operation_id).unwrap().unwrap().state,
            COMPLETED
        );
    }

    #[test]
    fn committed_wakes_observe_current_listing_before_final_admission_release() {
        let root = tempfile::tempdir().unwrap();
        let (store, reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        lock_unpoisoned(&service.state).ui_ready = true;
        let before = service.list().unwrap();
        let operation_id = "ordered-remove";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove { id: reference.id },
            })
            .unwrap();
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let service = Arc::clone(&service);
            let observed = std::rc::Rc::clone(&observed);
            let store = store.clone();
            ui::WAKE_OBSERVER.with(|observer| {
                *observer.borrow_mut() = Some(Box::new(move || {
                    let listing = service.cached_list();
                    let status = service.cached_status(operation_id).unwrap();
                    let busy = service.ui_mutation_busy();
                    let flock_busy = store
                        .list()
                        .err()
                        .is_some_and(|error| error.starts_with("Busy:"));
                    observed
                        .borrow_mut()
                        .push((listing.generation, status, busy, flock_busy));
                }));
            });
        }
        service.execute(item);
        ui::WAKE_OBSERVER.with(|observer| observer.borrow_mut().take());
        let observed = observed.borrow();
        assert!(observed
            .iter()
            .any(|(_, status, _, _)| status.state == APPLYING));
        assert!(observed
            .iter()
            .any(|(_, status, _, _)| status.state == COMPLETED));
        for (generation, status, busy, flock_busy) in observed.iter() {
            if status.committed {
                assert_eq!(*generation, before.generation + 1);
                if *busy {
                    assert!(*flock_busy, "the transaction must still own its flock");
                }
            }
        }
        let (generation, status, busy, flock_busy) = observed.last().unwrap();
        assert_eq!(*generation, before.generation + 1);
        assert_eq!(status.state, COMPLETED);
        assert!(!busy);
        assert!(!flock_busy);
    }

    #[test]
    fn postcommit_panic_recovers_under_pinned_flock_without_fabricating_completion() {
        let root = tempfile::tempdir().unwrap();
        let (store, reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        lock_unpoisoned(&service.state).ui_ready = true;
        let before = service.list().unwrap();
        let operation_id = "panic-after-commit";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove { id: reference.id },
            })
            .unwrap();
        *lock_unpoisoned(&service.after_committed_publication) =
            Some(Box::new(|| panic!("fixture postcommit unwind")));
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let service = Arc::clone(&service);
            let store = store.clone();
            let observed = std::rc::Rc::clone(&observed);
            ui::WAKE_OBSERVER.with(|observer| {
                *observer.borrow_mut() = Some(Box::new(move || {
                    let status = service.cached_status(operation_id).unwrap();
                    observed.borrow_mut().push((
                        status,
                        service.cached_list().generation,
                        service.ui_mutation_busy(),
                        store
                            .list()
                            .err()
                            .is_some_and(|error| error.starts_with("Busy:")),
                    ));
                }));
            });
        }
        service.execute(item);
        ui::WAKE_OBSERVER.with(|observer| observer.borrow_mut().take());
        let observed = observed.borrow();
        assert!(observed.iter().any(|(status, _, busy, flock_busy)| {
            status.state == COMMITTED_PENDING_APPLY && *busy && *flock_busy
        }));
        for (status, generation, _, _) in observed.iter() {
            if status.committed {
                assert_eq!(*generation, before.generation + 1);
                assert!(!status.ui_applied);
                assert_ne!(status.state, COMPLETED);
            }
        }
        let (status, generation, busy, flock_busy) = observed.last().unwrap();
        assert_eq!(*generation, before.generation + 1);
        assert_eq!(status.state, COMMITTED_PENDING_APPLY);
        assert_eq!(status.generation, Some(*generation));
        assert!(!busy);
        assert!(!flock_busy);
        assert_eq!(
            store.operation_status(operation_id).unwrap().unwrap().state,
            COMMITTED_PENDING_APPLY
        );
    }

    #[test]
    fn postcommit_panic_with_unreadable_registry_preserves_known_commit_evidence() {
        let root = tempfile::tempdir().unwrap();
        let (_store, reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        let operation_id = "unreadable-after-commit";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove { id: reference.id },
            })
            .unwrap();
        let registry = root.path().join("config").join("characters");
        *lock_unpoisoned(&service.after_committed_publication) = Some(Box::new(move || {
            for slot in ["registry.json", "registry.previous.json"] {
                let path = registry.join(slot);
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            panic!("fixture same-lock recovery read failure");
        }));
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        service.execute(item);
        let listing = service.cached_list();
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(listing.generation, before.generation + 1);
        assert!(listing.packs.is_empty());
        assert_eq!(listing.active, before.active);
        assert_eq!(status.state, DURABILITY_UNKNOWN);
        assert!(status.committed);
        assert!(!status.ui_applied);
        assert_eq!(status.generation, Some(listing.generation));
        assert!(status.error.unwrap().contains("status lookup failed"));
        assert!(!service.ui_mutation_busy());
    }

    #[test]
    fn failed_completion_bookkeeping_keeps_known_commit_and_reports_unknown_durability() {
        let root = tempfile::tempdir().unwrap();
        let (store, reference, _) = installed_unselected_png(root.path());
        let service = PackService::new(
            root.path().join("config"),
            root.path().join("builtin"),
            None,
        );
        service.started.store(true, Ordering::Release);
        {
            let mut state = lock_unpoisoned(&service.state);
            state.ui_ready = true;
            state.active = Some(CharacterRef::builtin());
        }
        let before = service.list().unwrap();
        let operation_id = "remove-bookkeeping-failure";
        service
            .submit(PackRequest {
                operation_id: operation_id.to_owned(),
                expected_generation: Some(before.generation),
                action: PackAction::Remove { id: reference.id },
            })
            .unwrap();
        let previous = root
            .path()
            .join("config")
            .join("characters")
            .join("registry.previous.json");
        *lock_unpoisoned(&service.after_committed_publication) = Some(Box::new(move || {
            std::fs::remove_file(&previous).unwrap();
            std::fs::create_dir(&previous).unwrap();
        }));
        let item = lock_unpoisoned(&service.queue).pop_front().unwrap();
        service.execute(item);
        let cached = service.cached_list();
        let status = service.cached_status(operation_id).unwrap();
        assert_eq!(cached.generation, before.generation + 1);
        assert!(cached.packs.is_empty());
        assert_eq!(cached.selected, before.selected);
        assert_eq!(cached.active, before.active);
        assert_eq!(status.state, DURABILITY_UNKNOWN);
        assert!(status.committed);
        assert!(!status.ui_applied);
        assert_eq!(status.generation, Some(cached.generation));
        assert!(status.error.unwrap().contains("registry-previous"));
        assert!(!service.ui_mutation_busy());
        assert_eq!(store.list().unwrap().generation, cached.generation);
        assert_eq!(
            store.operation_status(operation_id).unwrap().unwrap().state,
            COMMITTED_PENDING_APPLY
        );
    }

    #[test]
    fn poisoned_io_gate_is_recovered_for_nonblocking_reads() {
        let service = PackService::new(PathBuf::new(), PathBuf::new(), None);
        let poisoned = catch_unwind(AssertUnwindSafe({
            let service = Arc::clone(&service);
            move || {
                let _guard = match service.io_gate.lock() {
                    Ok(guard) => guard,
                    Err(_) => panic!("gate unexpectedly poisoned before test"),
                };
                panic!("poison the pack store gate");
            }
        }));
        assert!(poisoned.is_err());
        assert!(service.try_lock_io_gate().is_ok());
    }

    #[test]
    fn applying_state_is_not_terminal_or_evictable() {
        assert!(!is_terminal(APPLYING));
        assert!(is_terminal(COMMITTED_PENDING_APPLY));
        let mut state = ServiceState::default();
        for index in 0..MAX_RETAINED_OPERATIONS {
            let id = format!("{index}");
            state.order.push_back(id.clone());
            state.operations.insert(
                id.clone(),
                OperationEntry {
                    fingerprint: Vec::new(),
                    operation: PackOperation {
                        operation_id: id,
                        state: if index == 0 {
                            APPLYING.to_owned()
                        } else {
                            COMPLETED.to_owned()
                        },
                        committed: index != 0,
                        ui_applied: false,
                        generation: None,
                        error: None,
                    },
                    cancel: (index == 0).then(|| Arc::new(AtomicBool::new(false))),
                    is_official: false,
                },
            );
        }
        retain_capacity(&mut state);
        assert!(state.operations.contains_key("0"));
    }
}
