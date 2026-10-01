use crate::character_renderer;
use crate::character_store::PackStore;
use crate::character_types::{
    CharacterRef, PackAction, PackListing, PackOperation, PackRequest, RendererToken,
};
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
}

#[derive(Debug, Default)]
struct ServiceState {
    operations: HashMap<String, OperationEntry>,
    order: VecDeque<String>,
    active: Option<CharacterRef>,
    override_active: bool,
    runtime_error: Option<String>,
    renderer_error: Option<String>,
    active_cancel: Option<Arc<AtomicBool>>,
    ui_ready: bool,
}

#[derive(Debug)]
struct QueuedRequest {
    request: PackRequest,
    cancel: Arc<AtomicBool>,
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
    state: Mutex<ServiceState>,
    cache: Mutex<Option<PackListing>>,
    /// Serializes store reads against the worker's begin/commit/finish
    /// transaction.  UI/menu callers use `try_lock` and receive a bounded
    /// busy response instead of waiting behind native preflight.
    io_gate: Mutex<()>,
    queue: Mutex<VecDeque<QueuedRequest>>,
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
        Arc::new(Self {
            config_dir,
            builtin_assets,
            override_assets,
            state: Mutex::new(ServiceState::default()),
            cache: Mutex::new(None),
            io_gate: Mutex::new(()),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            worker: Mutex::new(None),
            worker_done: (Mutex::new(true), Condvar::new()),
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        })
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
        Ok(self.cache_listing(listing))
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
    /// Look up a retained operation without touching the registry.  Expired
    /// entries intentionally return `None`; UI refresh must not fall through
    /// to the filesystem-backed `status` method.
    pub fn cached_status(&self, operation_id: &str) -> Option<PackOperation> {
        lock_unpoisoned(&self.state)
            .operations
            .get(operation_id)
            .map(|entry| bound_operation(entry.operation.clone()))
    }

    fn cache_listing(&self, mut listing: PackListing) -> PackListing {
        listing = bound_listing(listing);
        let state = lock_unpoisoned(&self.state);
        if state.active.is_some() || state.override_active {
            listing.active = state.active.clone();
        }
        listing.override_active = state.override_active;
        if let Some(error) = state.runtime_error.clone() {
            listing.error = Some(bound_diagnostic(error));
        }
        *lock_unpoisoned(&self.cache) = Some(listing.clone());
        if let Some(error) = &state.renderer_error {
            listing.error = Some(error.clone());
        }
        listing
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

    /// Enqueue one request without touching the filesystem.
    pub fn submit(&self, request: PackRequest) -> Result<PackOperation, String> {
        validate_operation_id(&request.operation_id)?;
        let fingerprint = serde_json::to_vec(&request)
            .map_err(|error| format!("cannot fingerprint pack operation: {error}"))?;

        if self.stopping.load(Ordering::Acquire) {
            return Err("pack service is shutting down".to_owned());
        }
        if !self.started.load(Ordering::Acquire) {
            return Err("pack service is not running".to_owned());
        }
        if !lock_unpoisoned(&self.state).ui_ready {
            return Err("pack service is waiting for native UI readiness".to_owned());
        }

        {
            let state = lock_unpoisoned(&self.state);
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
        let mut state = lock_unpoisoned(&self.state);
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
        if queue.len() >= MAX_PENDING_OPERATIONS {
            return Err("too many queued pack operations".to_owned());
        }
        retain_capacity(&mut state);
        if state.operations.len() >= MAX_RETAINED_OPERATIONS {
            return Err("too many retained pack operation results".to_owned());
        }
        state.order.push_back(request.operation_id.clone());
        state.operations.insert(
            request.operation_id.clone(),
            OperationEntry {
                fingerprint: fingerprint.clone(),
                operation: operation.clone(),
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
                        // A durable identity is never re-used for a new
                        // request.  Its unknown fingerprint blocks submit
                        // retries from assuming a payload match.
                        fingerprint: Vec::new(),
                        operation: operation.clone(),
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
        if !self.stopping.swap(true, Ordering::AcqRel) {
            let active_cancel = {
                let state = lock_unpoisoned(&self.state);
                state.active_cancel.clone()
            };
            if let Some(cancel) = active_cancel {
                cancel.store(true, Ordering::Release);
            }
            let mut queue = lock_unpoisoned(&self.queue);
            let canceled: Vec<_> = queue.drain(..).collect();
            drop(queue);
            for item in canceled {
                self.update_operation(&item.request.operation_id, |operation| {
                    operation.state = CANCELED.to_owned();
                    operation.error = Some("pack service shut down before execution".to_owned());
                });
            }
        }

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
    }
}

fn worker_loop(weak: Weak<PackService>) {
    loop {
        let Some(service) = weak.upgrade() else {
            return;
        };
        let item = {
            let mut queue = lock_unpoisoned(&service.queue);
            loop {
                if let Some(item) = queue.pop_front() {
                    break Some(item);
                }
                if service.stopping.load(Ordering::Acquire) {
                    break None;
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
        if service.stopping.load(Ordering::Acquire) {
            item.cancel.store(true, Ordering::Release);
            service.update_operation(&item.request.operation_id, |operation| {
                operation.state = CANCELED.to_owned();
                operation.error = Some("pack service shut down before execution".to_owned());
            });
            continue;
        }
        let operation_id = item.request.operation_id.clone();
        if catch_unwind(AssertUnwindSafe(|| service.execute(item))).is_err() {
            {
                let mut state = lock_unpoisoned(&service.state);
                state.active_cancel = None;
            }
            let durable = service.durable_operation_status(&operation_id);
            service.refresh_cache();
            service.update_operation(&operation_id, |operation| {
                if let Some(durable) = durable {
                    // A durable record is authoritative after a panic; in
                    // particular, never turn a confirmed commit into a
                    // transient in-memory rollback.
                    if durable.committed {
                        *operation = durable;
                        return;
                    }
                    operation.committed = false;
                    operation.generation = durable.generation;
                    operation.ui_applied = durable.ui_applied;
                } else {
                    operation.committed = false;
                    operation.generation = None;
                    operation.ui_applied = false;
                }
                operation.state = DURABILITY_UNKNOWN.to_owned();
                operation.error =
                    Some("pack worker aborted; registry commit outcome unknown".to_owned());
            });
        }
    }
}
impl PackService {
    fn execute(&self, item: QueuedRequest) {
        let operation_id = item.request.operation_id.clone();
        self.update_operation(&operation_id, |operation| {
            operation.state = PREPARING.to_owned();
        });
        if self.stopping.load(Ordering::Acquire) {
            item.cancel.store(true, Ordering::Release);
        }

        let managed_active = {
            let state = lock_unpoisoned(&self.state);
            if state.override_active {
                None
            } else {
                state.active.clone()
            }
        };
        {
            let mut state = lock_unpoisoned(&self.state);
            state.active_cancel = Some(item.cancel.clone());
        }
        if self.stopping.load(Ordering::Acquire) {
            item.cancel.store(true, Ordering::Release);
        }
        if item.cancel.load(Ordering::Acquire) {
            let mut state = lock_unpoisoned(&self.state);
            state.active_cancel = None;
            drop(state);
            self.update_operation(&operation_id, |operation| {
                operation.state = CANCELED.to_owned();
                operation.error = Some("pack service shut down before execution".to_owned());
            });
            return;
        }

        let result = {
            let _io = lock_unpoisoned(&self.io_gate);
            self.execute_transaction(&item, managed_active.as_ref())
        };
        {
            let mut state = lock_unpoisoned(&self.state);
            state.active_cancel = None;
        }
        self.refresh_cache();
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
    }

    fn execute_transaction(
        &self,
        item: &QueuedRequest,
        managed_active: Option<&CharacterRef>,
    ) -> Result<(), String> {
        let operation_id = &item.request.operation_id;
        let store = PackStore::new(self.config_dir.clone(), Some(self.builtin_assets.clone()));
        let durable = match store.operation_status(operation_id) {
            Ok(durable) => durable,
            Err(error) => {
                self.mark_durable_lookup_unknown(operation_id, error);
                return Ok(());
            }
        };
        if let Some(durable) = durable {
            self.reconcile_durable_operation(durable);
            return Ok(());
        }
        let mut transaction = match store.begin(&item.request, managed_active) {
            Ok(transaction) => transaction,
            Err(error) => match store.operation_status(operation_id) {
                Ok(Some(durable)) => {
                    self.reconcile_durable_operation(durable);
                    return Ok(());
                }
                Ok(None) => return Err(error),
                Err(status_error) => {
                    self.mark_durable_lookup_unknown(
                        operation_id,
                        format!("begin failed: {error}; status lookup failed: {status_error}"),
                    );
                    return Ok(());
                }
            },
        };
        let selected = transaction.selected.clone();
        let candidate_ref = transaction.candidate_ref.clone();
        let changes_selection = transaction.changes_selection;
        let mut prepared: Option<RendererToken> = None;

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
            prepared = Some(token.clone());
            let ready = ui::prepare_character(token.clone(), candidate, item.cancel.clone())
                .and_then(|actual| {
                    if actual == token {
                        Ok(())
                    } else {
                        Err("native Ready token mismatch".to_owned())
                    }
                });
            if let Err(error) = ready {
                ui::discard_character(token);
                let error = format!("native preflight failed: {error}");
                let _ = transaction.mark_failed(error.clone());
                return Err(error);
            }
        }
        if item.cancel.load(Ordering::Acquire) {
            if let Some(token) = &prepared {
                ui::discard_character(token.clone());
            }
            let error = "pack operation canceled before commit".to_owned();
            let _ = transaction.mark_failed(error.clone());
            return Err(error);
        }

        let commit = match transaction.commit() {
            Ok(commit) => commit,
            Err(error) => {
                if let Some(token) = &prepared {
                    ui::discard_character(token.clone());
                }
                // commit() only fails before registry.json is replaced, so
                // this is an ordinary failure, not an unknown outcome.
                return Err(format!("registry commit failed: {error}"));
            }
        };
        let commit_error = commit.error.clone();
        let durability_unknown = commit.durability_unknown || commit_error.is_some();
        self.update_operation(operation_id, |operation| {
            operation.committed = true;
            operation.generation = Some(commit.generation);
            // The in-memory result remains nonterminal until native apply and
            // transaction finalization have both returned.  The durable
            // registry still contains its ordinary committed-pending record.
            operation.state = APPLYING.to_owned();
            operation.error = commit_error.clone();
        });

        let (override_active, explicit_override_clear) = {
            let state = lock_unpoisoned(&self.state);
            (
                state.override_active,
                matches!(
                    &item.request.action,
                    PackAction::Select { .. } | PackAction::Restore { .. }
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
                if let Some(token) = &prepared {
                    ui::discard_character(token.clone());
                }
                apply_error = Some(format!("native apply failed after commit: {error}"));
            } else {
                ui_applied = true;
                self.set_active(selected.clone(), false, None);
            }
        } else if let Some(token) = &prepared {
            ui::discard_character(token.clone());
        }
        self.update_operation(operation_id, |operation| {
            operation.ui_applied = ui_applied;
            if let Some(error) = apply_error.clone() {
                operation.error = Some(append_error(operation.error.take(), error));
            }
        });
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

    fn refresh_cache(&self) {
        let Ok(_io) = self.try_lock_io_gate() else {
            return;
        };
        let store = PackStore::new(self.config_dir.clone(), None);
        let Ok(listing) = store.list() else {
            return;
        };
        let _ = self.cache_listing(listing);
        ui::wake();
    }
    fn durable_operation_status(&self, operation_id: &str) -> Option<PackOperation> {
        let _io = self.try_lock_io_gate().ok()?;
        let store = PackStore::new(self.config_dir.clone(), None);
        store
            .operation_status(operation_id)
            .ok()
            .flatten()
            .map(bound_operation)
    }
    fn reconcile_durable_operation(&self, durable: PackOperation) {
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
            .map(|entry| is_terminal(&entry.operation.state))
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn retained_state_evicts_only_terminal_results() {
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
                },
            );
        }
        retain_capacity(&mut state);
        assert!(state.operations.contains_key("0"));
        assert_eq!(state.operations.len(), MAX_RETAINED_OPERATIONS - 1);
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
                },
            );
        }
        retain_capacity(&mut state);
        assert!(state.operations.contains_key("0"));
    }
}
