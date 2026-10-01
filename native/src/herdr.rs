use crate::agent_outcome::OutcomeTracker;
use crate::herdr_protocol::{
    self, AgentRecord, AgentStatus, PluginAvailability, ProtocolError, SnapshotCounts,
    SubscriptionEvent, SubscriptionEventRead, SubscriptionReader,
};
use crate::lifecycle::LifecycleSettings;
use crate::session::OutcomeObservation;
use crate::session_view::SessionKey;
use crate::state::{AppState, SessionStatusUpdate, SourceCounts};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_ENDPOINTS: usize = 64;
const COMMAND_QUEUE: usize = 128;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(15);
const MAX_PANE_CONVERGENCE: usize = 3;
const WORKER_TICK: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(650);
const DISCONNECTION_GRACE: Duration = Duration::from_secs(30);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(3);
static PROMPT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PromptError {
    Empty,
    TooLarge,
    Busy,
    Offline,
    ReadOnly,
    StaleTarget,
    Blocked,
    NotReady,
    Unsupported,
    UnknownDelivery,
    Other(String),
}

#[derive(Debug)]
pub(crate) struct PromptResult {
    pub(crate) key: SessionKey,
    pub(crate) text: String,
    pub(crate) result: Result<(), PromptError>,
}

/// Exactly one request per sender. The dedicated one-shot worker never shares
/// the serial status watcher or holds the application mutex during socket I/O.
pub(crate) struct PromptSender {
    shared: Arc<Mutex<AppState>>,
    receiver: Option<Receiver<PromptResult>>,
}

impl PromptSender {
    pub(crate) fn new(shared: Arc<Mutex<AppState>>) -> Self {
        Self {
            shared,
            receiver: None,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.receiver.is_some()
    }

    pub(crate) fn submit(&mut self, key: SessionKey, text: String) -> Result<(), PromptError> {
        if self.is_pending() {
            return Err(PromptError::Busy);
        }
        if text.trim().is_empty() {
            return Err(PromptError::Empty);
        }
        if text.len() > herdr_protocol::MAX_FRAME_BYTES {
            return Err(PromptError::TooLarge);
        }
        let target = {
            let state = self.shared.lock().map_err(|_| PromptError::Offline)?;
            state.prompt_target(&key)?
        };
        let frame = serde_json::json!({
            "id": "desktop-pet-prompt-4294967295-18446744073709551615",
            "method": "agent.prompt",
            "params": { "target": target.pane_id, "text": text },
        });
        if serde_json::to_vec(&frame)
            .map_err(|error| PromptError::Other(error.to_string()))?
            .len()
            .saturating_add(1)
            > herdr_protocol::MAX_FRAME_BYTES
        {
            return Err(PromptError::TooLarge);
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let shared = Arc::clone(&self.shared);
        let worker_text = text;
        thread::Builder::new()
            .name("herdr-desktop-pet-prompt".to_owned())
            .spawn(move || {
                let result = send_prompt(&shared, &key, &worker_text);
                let _ = tx.send(PromptResult {
                    key,
                    text: worker_text,
                    result,
                });
                wake_ui();
            })
            .map_err(|error| PromptError::Other(error.to_string()))?;
        self.receiver = Some(rx);
        Ok(())
    }

    pub(crate) fn try_result(&mut self) -> Option<PromptResult> {
        let receiver = self.receiver.as_ref()?;
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                None
            }
        }
    }
}

fn prompt_request_id() -> String {
    format!(
        "desktop-pet-prompt-{}-{}",
        std::process::id(),
        PROMPT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn send_prompt(
    shared: &Arc<Mutex<AppState>>,
    key: &SessionKey,
    text: &str,
) -> Result<(), PromptError> {
    let target = shared
        .lock()
        .map_err(|_| PromptError::Offline)?
        .prompt_target(key)?;
    let path = Path::new(&target.source);
    // Watchers register canonical local endpoint identities. Never reinterpret
    // a synthetic or aliased source as a different socket; filesystem
    // resolution happens after releasing the application mutex.
    if crate::socket::canonical_endpoint(path).map_err(|_| PromptError::StaleTarget)? != path {
        return Err(PromptError::StaleTarget);
    }
    let snapshot = herdr_protocol::request(
        path,
        &prompt_request_id(),
        "session.snapshot",
        serde_json::json!({}),
        REQUEST_TIMEOUT,
    )
    .and_then(herdr_protocol::parse_snapshot)
    .map_err(|error| match error {
        ProtocolError::Io(_) | ProtocolError::Timeout | ProtocolError::EmptyResponse => {
            PromptError::Offline
        }
        other => PromptError::Other(other.to_string()),
    })?;
    // Require a unique live pane/terminal pair on this server, not merely an
    // old watcher row. The final foreground/readiness guard belongs to Herdr.
    if snapshot
        .panes
        .iter()
        .filter(|pane| *pane == &target.pane_id)
        .count()
        != 1
        || snapshot
            .agents
            .iter()
            .filter(|agent| agent.terminal_id == key.terminal_id || agent.pane_id == target.pane_id)
            .count()
            != 1
        || !snapshot
            .agents
            .iter()
            .any(|agent| agent.terminal_id == key.terminal_id && agent.pane_id == target.pane_id)
    {
        return Err(PromptError::StaleTarget);
    }
    let current = shared
        .lock()
        .map_err(|_| PromptError::Offline)?
        .prompt_target(key)?;
    if current != target {
        return Err(PromptError::StaleTarget);
    }
    let response = herdr_protocol::request_with_delivery(
        path,
        &prompt_request_id(),
        "agent.prompt",
        serde_json::json!({ "target": target.pane_id, "text": text }),
        PROMPT_TIMEOUT,
    )
    .map_err(|(error, attempted)| {
        if attempted {
            PromptError::UnknownDelivery
        } else {
            match error {
                ProtocolError::FrameTooLarge => PromptError::TooLarge,
                ProtocolError::Io(_) | ProtocolError::Timeout => PromptError::Offline,
                other => PromptError::Other(other.to_string()),
            }
        }
    })?;
    match response {
        herdr_protocol::Response::Error { code, message } => Err(match code.as_str() {
            "agent_blocked" => PromptError::Blocked,
            "agent_not_ready" | "agent_not_idle" => PromptError::NotReady,
            "unsupported_method" | "method_not_found" | "unknown_method" => {
                PromptError::Unsupported
            }
            "agent_not_running" | "agent_not_found" | "pane_not_found" => PromptError::StaleTarget,
            _ => PromptError::Other(format!("{code}: {message}")),
        }),
        herdr_protocol::Response::Success { result } => {
            let agent = result.get("agent");
            if result.get("type").and_then(serde_json::Value::as_str) != Some("agent_prompted")
                || agent
                    .and_then(|agent| agent.get("terminal_id"))
                    .and_then(serde_json::Value::as_str)
                    != Some(key.terminal_id.as_str())
                || agent
                    .and_then(|agent| agent.get("pane_id"))
                    .and_then(serde_json::Value::as_str)
                    != Some(target.pane_id.as_str())
            {
                return Err(PromptError::UnknownDelivery);
            }
            Ok(())
        }
    }
}

#[derive(Debug)]
enum Command {
    Register(PathBuf),
    Stop,
}
type RegistrationState = Arc<Mutex<HashMap<PathBuf, bool>>>;

struct ExitPolicy {
    settings: LifecycleSettings,
    connected: usize,
    remote_connected: bool,
    deadline: Option<Instant>,
    finalized: bool,
}

impl ExitPolicy {
    fn new(settings: LifecycleSettings, now: Instant) -> Self {
        Self {
            settings,
            connected: 0,
            remote_connected: false,
            deadline: settings
                .exit_with_herdr
                .then_some(now + DISCONNECTION_GRACE),
            finalized: false,
        }
    }

    fn observe_connection(&mut self, was_connected: bool, is_connected: bool, now: Instant) {
        if was_connected == is_connected {
            return;
        }
        if is_connected {
            self.connected += 1;
            self.deadline = None;
        } else {
            self.connected -= 1;
            if self.connected == 0 && !self.remote_connected && self.settings.exit_with_herdr {
                self.deadline = Some(now + DISCONNECTION_GRACE);
            }
        }
    }

    fn observe_remote_connection(&mut self, connected: bool, now: Instant) {
        if self.remote_connected == connected {
            return;
        }
        self.remote_connected = connected;
        if connected {
            self.deadline = None;
        } else if self.connected == 0 && self.settings.exit_with_herdr {
            self.deadline = Some(now + DISCONNECTION_GRACE);
        }
    }

    fn change_settings(&mut self, settings: LifecycleSettings, now: Instant) {
        if settings.exit_with_herdr != self.settings.exit_with_herdr {
            self.deadline =
                if settings.exit_with_herdr && self.connected == 0 && !self.remote_connected {
                    Some(now + DISCONNECTION_GRACE)
                } else {
                    None
                };
        }
        self.settings = settings;
    }

    fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
}

type SharedPolicy = Arc<Mutex<ExitPolicy>>;

/// Owns the bounded set of Herdr endpoint watchers.  Registration only queues
/// work; it does not wait for a server response or claim that a snapshot is
/// fresh.
pub struct Watchers {
    command_tx: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    registry: RegistrationState,
    shared: Arc<Mutex<AppState>>,
    policy: SharedPolicy,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl Watchers {
    pub fn new(shared: Arc<Mutex<AppState>>) -> Self {
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let settings = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .lifecycle_settings();
        let policy = Arc::new(Mutex::new(ExitPolicy::new(settings, Instant::now())));
        let worker_shared = Arc::clone(&shared);
        let worker_policy = Arc::clone(&policy);
        let worker_stop = Arc::clone(&stop);
        let worker_registry = Arc::clone(&registry);
        let join = thread::Builder::new()
            .name("herdr-desktop-pet-watchers".to_owned())
            .spawn(move || {
                watcher_loop(
                    worker_shared,
                    command_rx,
                    worker_stop,
                    worker_registry,
                    worker_policy,
                )
            })
            .expect("failed to start Herdr watcher thread");
        Self {
            command_tx,
            stop,
            registry,
            policy,
            shared,
            join: Mutex::new(Some(join)),
        }
    }

    /// Canonicalize and queue an endpoint without blocking on remote I/O.
    pub fn register(&self, endpoint: PathBuf) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("Herdr watchers are shut down".to_owned());
        }
        let policy = self
            .policy
            .lock()
            .map_err(|_| "Herdr watcher policy is poisoned".to_owned())?;
        if self.stop.load(Ordering::Acquire)
            || policy.finalized
            || self
                .shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .scene()
                .shutdown
        {
            return Err("Herdr watchers are shutting down".to_owned());
        }
        let endpoint = crate::socket::canonical_endpoint(&endpoint)?;
        let (inserted, reactivated) = {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| "Herdr watcher registry is poisoned".to_owned())?;
            match registry.get_mut(&endpoint) {
                Some(disabled) if !*disabled => return Ok(()),
                Some(disabled) => {
                    *disabled = false;
                    (false, true)
                }
                None => {
                    if registry.len() >= MAX_ENDPOINTS {
                        return Err("Herdr watcher endpoint limit reached".to_owned());
                    }
                    registry.insert(endpoint.clone(), false);
                    (true, false)
                }
            }
        };
        match self
            .command_tx
            .try_send(Command::Register(endpoint.clone()))
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.rollback_registration(&endpoint, inserted, reactivated);
                Err("Herdr watcher queue is full".to_owned())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.rollback_registration(&endpoint, inserted, reactivated);
                Err("Herdr watcher thread stopped".to_owned())
            }
        }
    }

    /// Serialize successful persistent updates with the worker's shutdown claim.
    pub(crate) fn update_lifecycle_settings<F>(
        &self,
        persist: F,
    ) -> Result<LifecycleSettings, String>
    where
        F: FnOnce() -> Result<LifecycleSettings, String>,
    {
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| "Herdr watcher policy is poisoned".to_owned())?;
        if policy.finalized
            || self
                .shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .scene()
                .shutdown
        {
            return Err("desktop-pet is shutting down".to_owned());
        }
        let settings = persist()?;
        policy.change_settings(settings, Instant::now());
        // The worker's policy and cached AppState are updated while holding the
        // same lock that guards the final autoexit decision.
        let mut state = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        state.set_lifecycle_settings(settings);
        Ok(settings)
    }

    fn rollback_registration(&self, endpoint: &Path, inserted: bool, reactivated: bool) {
        if let Ok(mut registry) = self.registry.lock() {
            if inserted {
                registry.remove(endpoint);
            } else if reactivated {
                registry.insert(endpoint.to_path_buf(), true);
            }
        }
    }

    /// Stop the worker and join it.  All socket reads have finite deadlines;
    /// therefore this join is bounded by a small number of request deadlines.
    pub fn shutdown(&mut self) {
        if self.stop.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.command_tx.try_send(Command::Stop);
        let Ok(mut slot) = self.join.lock() else {
            return;
        };
        if let Some(join) = slot.take() {
            let _ = join.join();
        }
    }
}

impl Drop for Watchers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) struct Endpoint {
    path: PathBuf,
    source: String,
    registry: RegistrationState,
    generation: u64,
    next_generation: u64,
    agents: HashMap<String, AgentRecord>,
    outcomes: HashMap<String, Vec<TrackedOutcome>>,
    duplicate_agents: Vec<AgentRecord>,
    pending_prime: HashMap<String, HashSet<String>>,
    ambiguous_terminal_ids: HashSet<String>,
    outcome_snapshot_unix_ms: Option<u64>,
    last_sessions: usize,
    pane_ids: Vec<String>,
    stream: Option<SubscriptionReader>,
    next_attempt: Instant,
    next_reconcile: Instant,
    backoff: Duration,
    disabled: bool,
    observation_epoch: Option<u64>,
}

struct TrackedOutcome {
    pane_id: String,
    session: String,
    tracker: OutcomeTracker,
}

#[derive(Debug, Eq, PartialEq)]
enum EventRecordUpdate<'a> {
    Applied {
        terminal_id: &'a str,
        completion: bool,
        counts: SourceCounts,
    },
    Ambiguous,
}
struct EventPublication<'a> {
    source: &'a str,
    generation: u64,
    terminal_id: &'a str,
    pane_id: &'a str,
    status: AgentStatus,
    counts: SourceCounts,
    completion: bool,
    observed_at: Instant,
}

fn publish_event(shared: &Arc<Mutex<AppState>>, publication: EventPublication<'_>) {
    if let Ok(mut state) = shared.lock() {
        let _ = state.update_session_status(SessionStatusUpdate {
            source: publication.source,
            generation: publication.generation,
            terminal_id: publication.terminal_id,
            pane_id: publication.pane_id,
            status: publication.status,
            counts: publication.counts,
            completion: publication.completion,
            observed_at: publication.observed_at,
        });
    }
    wake_ui();
}
fn source_counts(counts: SnapshotCounts) -> SourceCounts {
    SourceCounts {
        sessions: counts.sessions,
        working: counts.working,
        blocked: counts.blocked,
        done: counts.done,
        unknown: counts.unknown,
    }
}

fn adjust_source_counts(
    mut counts: SourceCounts,
    previous: AgentStatus,
    next: AgentStatus,
) -> SourceCounts {
    match previous {
        AgentStatus::Working => counts.working = counts.working.saturating_sub(1),
        AgentStatus::Blocked => counts.blocked = counts.blocked.saturating_sub(1),
        AgentStatus::Done => counts.done = counts.done.saturating_sub(1),
        AgentStatus::Unknown => counts.unknown = counts.unknown.saturating_sub(1),
        AgentStatus::Idle => {}
    }
    match next {
        AgentStatus::Working => counts.working = counts.working.saturating_add(1),
        AgentStatus::Blocked => counts.blocked = counts.blocked.saturating_add(1),
        AgentStatus::Done => counts.done = counts.done.saturating_add(1),
        AgentStatus::Unknown => counts.unknown = counts.unknown.saturating_add(1),
        AgentStatus::Idle => {}
    }
    counts
}

fn apply_event_records<'a>(
    agents: &'a mut HashMap<String, AgentRecord>,
    ambiguous_terminal_ids: &HashSet<String>,
    pane_id: &str,
    status: AgentStatus,
    counts: SourceCounts,
) -> EventRecordUpdate<'a> {
    let mut match_count = 0usize;
    let mut ambiguous_terminal = false;
    for agent in agents.values() {
        if agent.pane_id == pane_id {
            match_count = match_count.saturating_add(1);
            ambiguous_terminal |= ambiguous_terminal_ids.contains(agent.terminal_id.as_str());
        }
    }
    if match_count != 1 || ambiguous_terminal {
        return EventRecordUpdate::Ambiguous;
    }
    for agent in agents.values_mut() {
        if agent.pane_id == pane_id {
            let previous = agent.status;
            let completion = !agent.outcome_authoritative
                && matches!(agent.status, AgentStatus::Working | AgentStatus::Blocked)
                && status == AgentStatus::Done;
            agent.status = status;
            return EventRecordUpdate::Applied {
                terminal_id: agent.terminal_id.as_str(),
                completion,
                counts: adjust_source_counts(counts, previous, status),
            };
        }
    }
    EventRecordUpdate::Ambiguous
}

impl Endpoint {
    fn with_registry(path: PathBuf, registry: RegistrationState) -> Self {
        let source = path.to_string_lossy().into_owned();
        Self {
            path,
            source,
            registry,
            generation: 0,
            next_generation: 0,
            ambiguous_terminal_ids: HashSet::new(),
            agents: HashMap::new(),
            duplicate_agents: Vec::new(),
            pending_prime: HashMap::new(),
            outcomes: HashMap::new(),
            outcome_snapshot_unix_ms: None,
            last_sessions: 0,
            pane_ids: Vec::new(),
            stream: None,
            next_attempt: Instant::now(),
            next_reconcile: Instant::now(),
            backoff: INITIAL_BACKOFF,
            disabled: false,
            observation_epoch: None,
        }
    }

    pub(crate) fn remote(source: String, generation: u64, epoch: u64) -> Self {
        let mut endpoint =
            Self::with_registry(PathBuf::new(), Arc::new(Mutex::new(HashMap::new())));
        endpoint.source = source;
        endpoint.generation = generation;
        endpoint.next_generation = generation;
        endpoint.observation_epoch = Some(epoch);
        endpoint
    }

    pub(crate) fn remote_snapshot(
        &mut self,
        shared: &Arc<Mutex<AppState>>,
        snapshot: herdr_protocol::Snapshot,
        observed_at: Instant,
        baseline: bool,
    ) {
        let completions = if baseline {
            self.replace_snapshot(snapshot);
            Vec::new()
        } else {
            self.reconcile_snapshot(snapshot)
        };
        self.publish_snapshot(shared, &completions, observed_at);
    }

    pub(crate) fn remote_offline(&mut self, shared: &Arc<Mutex<AppState>>) {
        self.mark_retry(shared, Instant::now());
    }

    fn epoch_matches(&self, state: &AppState) -> bool {
        self.observation_epoch
            .is_none_or(|epoch| state.observation_epoch() == epoch)
    }

    fn set_registration_disabled(&self, disabled: bool) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.insert(self.path.clone(), disabled);
        }
    }
    fn begin_generation(&mut self, shared: &Arc<Mutex<AppState>>) {
        self.next_generation = self.next_generation.saturating_add(1).max(1);
        self.generation = self.next_generation;
        if !self.agents.is_empty() {
            self.last_sessions = self.agents.len();
        }
        self.stream = None;
        self.agents.clear();
        self.duplicate_agents.clear();
        self.pending_prime.clear();
        self.outcomes.clear();
        self.outcome_snapshot_unix_ms = None;
        self.ambiguous_terminal_ids.clear();
        self.pane_ids.clear();
        if let Ok(mut state) = shared.lock() {
            let _ = state.begin_source(self.source.clone(), self.generation);
        }
        wake_ui();
    }

    fn counts(&self) -> SnapshotCounts {
        let mut counts = SnapshotCounts {
            sessions: self.agents.len(),
            ..SnapshotCounts::default()
        };
        for agent in self.agents.values() {
            match agent.status {
                AgentStatus::Idle => {}
                AgentStatus::Working => counts.working = counts.working.saturating_add(1),
                AgentStatus::Blocked => counts.blocked = counts.blocked.saturating_add(1),
                AgentStatus::Done => counts.done = counts.done.saturating_add(1),
                AgentStatus::Unknown => counts.unknown = counts.unknown.saturating_add(1),
            }
        }
        counts
    }
    fn publish(&self, shared: &Arc<Mutex<AppState>>, connected: bool) {
        let counts = self.counts();
        if let Ok(mut state) = shared.lock() {
            if !self.epoch_matches(&state) {
                return;
            }
            let _ = state.update_source(
                &self.source,
                self.generation,
                connected,
                SourceCounts {
                    sessions: counts.sessions,
                    working: counts.working,
                    blocked: counts.blocked,
                    done: counts.done,
                    unknown: counts.unknown,
                },
            );
        }
        wake_ui();
    }

    fn publish_snapshot(
        &mut self,
        shared: &Arc<Mutex<AppState>>,
        completions: &[(String, String)],
        observed_at: Instant,
    ) {
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.publish_snapshot_at(shared, completions, observed_at, now_unix_ms);
    }

    fn publish_snapshot_at(
        &mut self,
        shared: &Arc<Mutex<AppState>>,
        completions: &[(String, String)],
        observed_at: Instant,
        now_unix_ms: u64,
    ) {
        let counts = self.counts();
        if let Ok(mut state) = shared.lock() {
            if !self.epoch_matches(&state)
                || !state.publish_source_snapshot(
                    &self.source,
                    self.generation,
                    self.agents.values(),
                    SourceCounts {
                        sessions: counts.sessions,
                        working: counts.working,
                        blocked: counts.blocked,
                        done: counts.done,
                        unknown: counts.unknown,
                    },
                    completions,
                    observed_at,
                )
            {
                return;
            }
            let mut pane_counts = HashMap::<&str, usize>::new();
            for agent in self.agents.values().chain(self.duplicate_agents.iter()) {
                *pane_counts.entry(agent.pane_id.as_str()).or_default() += 1;
            }
            // Index only multi-candidate terminals; ordinary unique reports
            // continue through their single short tracker vector.
            let mut candidate_slots = HashMap::<
                &str,
                (
                    HashMap<(&str, &str), Option<usize>>,
                    HashSet<&str>,
                    HashSet<&str>,
                ),
            >::new();
            for agent in self.agents.values().chain(self.duplicate_agents.iter()) {
                if !self
                    .ambiguous_terminal_ids
                    .contains(agent.terminal_id.as_str())
                {
                    continue;
                }
                let index = candidate_slots
                    .entry(agent.terminal_id.as_str())
                    .or_default();
                index.1.insert(agent.pane_id.as_str());
                if let Some(report) = &agent.outcome {
                    index
                        .0
                        .insert((agent.pane_id.as_str(), report.session.as_str()), None);
                }
            }
            for (terminal, index) in &mut candidate_slots {
                if let Some(slots) = self.outcomes.get(*terminal) {
                    // The temporary lookup borrows existing trackers only in
                    // this scope. The long-lived index keeps incoming-record
                    // keys, so the tracker vector may be extended below.
                    let old_slots: HashMap<(&str, &str), usize> = slots
                        .iter()
                        .enumerate()
                        .map(|(position, tracked)| {
                            (
                                (tracked.pane_id.as_str(), tracked.session.as_str()),
                                position,
                            )
                        })
                        .collect();
                    let old_panes: HashSet<&str> = slots
                        .iter()
                        .map(|tracked| tracked.pane_id.as_str())
                        .collect();
                    for (identity, position) in &mut index.0 {
                        *position = old_slots.get(identity).copied();
                    }
                    for pane in &index.1 {
                        if old_panes.contains(*pane) {
                            index.2.insert(*pane);
                        }
                    }
                }
            }
            for agent in self.agents.values().chain(self.duplicate_agents.iter()) {
                let ambiguous = self.ambiguous_terminal_ids.contains(&agent.terminal_id)
                    || pane_counts.get(agent.pane_id.as_str()) != Some(&1);
                if ambiguous || !agent.outcome_authoritative || agent.outcome.is_none() {
                    state.invalidate_session_outcome(
                        &self.source,
                        self.generation,
                        &agent.terminal_id,
                        &agent.pane_id,
                    );
                }
                if !agent.outcome_authoritative {
                    continue;
                }
                let Some(report) = &agent.outcome else {
                    continue;
                };
                if !self.outcomes.contains_key(agent.terminal_id.as_str()) {
                    self.outcomes.insert(agent.terminal_id.clone(), Vec::new());
                }
                let slots = self.outcomes.get_mut(agent.terminal_id.as_str()).unwrap();
                let slot = if let Some(index) = candidate_slots.get(agent.terminal_id.as_str()) {
                    index
                        .0
                        .get(&(agent.pane_id.as_str(), report.session.as_str()))
                        .copied()
                        .flatten()
                } else {
                    slots.iter().position(|tracked| {
                        tracked.pane_id == agent.pane_id && tracked.session == report.session
                    })
                };
                let tracked = if let Some(slot) = slot {
                    &mut slots[slot]
                } else {
                    let rebound = candidate_slots.get(agent.terminal_id.as_str()).map_or_else(
                        || slots.iter().any(|tracked| tracked.pane_id == agent.pane_id),
                        |index| index.2.contains(agent.pane_id.as_str()),
                    ) || self
                        .pending_prime
                        .get(agent.terminal_id.as_str())
                        .is_some_and(|panes| panes.contains(agent.pane_id.as_str()));
                    slots.push(TrackedOutcome {
                        pane_id: agent.pane_id.clone(),
                        session: report.session.clone(),
                        tracker: OutcomeTracker::new(if rebound {
                            None
                        } else {
                            self.outcome_snapshot_unix_ms
                        }),
                    });
                    if let Some(index) = candidate_slots.get_mut(agent.terminal_id.as_str()) {
                        let position = slots.len() - 1;
                        index.0.insert(
                            (agent.pane_id.as_str(), report.session.as_str()),
                            Some(position),
                        );
                        index.2.insert(agent.pane_id.as_str());
                    }
                    slots.last_mut().expect("inserted outcome tracker")
                };
                if ambiguous {
                    tracked.tracker.observe_ambiguous(report);
                    continue;
                }
                if let Some(panes) = self.pending_prime.get_mut(agent.terminal_id.as_str()) {
                    panes.remove(agent.pane_id.as_str());
                }
                if let Some(outcome) = tracked.tracker.observe(report, now_unix_ms) {
                    state.push_outcome(OutcomeObservation {
                        source: self.source.clone(),
                        generation: self.generation,
                        terminal_id: agent.terminal_id.clone(),
                        pane_id: agent.pane_id.clone(),
                        outcome,
                        at_unix_ms: report.at_unix_ms,
                        observed_at,
                    });
                }
            }
            // The observation started before the snapshot request. Reports
            // published while that request was in flight must remain new.
            self.outcome_snapshot_unix_ms = Some(
                now_unix_ms
                    .saturating_sub(observed_at.elapsed().as_millis().min(u64::MAX as u128) as u64),
            );
        }
        wake_ui();
    }

    fn replace_snapshot(&mut self, snapshot: herdr_protocol::Snapshot) {
        let mut terminal_counts = HashMap::<&str, usize>::new();
        for record in &snapshot.agents {
            *terminal_counts
                .entry(record.terminal_id.as_str())
                .or_default() += 1;
        }
        self.ambiguous_terminal_ids.clear();
        for (terminal_id, count) in terminal_counts {
            if count > 1 {
                self.ambiguous_terminal_ids.insert(terminal_id.to_owned());
            }
        }
        self.pane_ids = snapshot.pane_ids();
        // Classify against the complete incoming snapshot while the previous
        // winners are still available; rebuilding below can then reuse the map.
        for record in &snapshot.agents {
            let was_rebound = self
                .agents
                .get(record.terminal_id.as_str())
                .is_some_and(|old| {
                    old.pane_id != record.pane_id
                        || old
                            .outcome
                            .as_ref()
                            .zip(record.outcome.as_ref())
                            .is_some_and(|(before, after)| before.session != after.session)
                })
                || (self
                    .ambiguous_terminal_ids
                    .contains(record.terminal_id.as_str())
                    && record.outcome.is_none());
            if was_rebound {
                self.pending_prime
                    .entry(record.terminal_id.clone())
                    .or_default()
                    .insert(record.pane_id.clone());
            }
        }
        self.agents.clear();
        self.duplicate_agents.clear();
        self.agents.reserve(snapshot.agents.len());
        for record in snapshot.agents {
            let terminal_id = record.terminal_id.clone();
            if let Some(displaced) = self.agents.insert(terminal_id, record) {
                self.duplicate_agents.push(displaced);
            }
        }
        self.last_sessions = self.agents.len();
        let mut identities = HashSet::<(&str, &str)>::new();
        let mut reports = HashSet::<(&str, &str, &str)>::new();
        let mut absent_reports = HashSet::<(&str, &str)>::new();
        let mut reported_pairs = HashSet::<(&str, &str)>::new();
        for agent in self.agents.values().chain(self.duplicate_agents.iter()) {
            let pair = (agent.terminal_id.as_str(), agent.pane_id.as_str());
            identities.insert(pair);
            if let Some(report) = &agent.outcome {
                reported_pairs.insert(pair);
                reports.insert((pair.0, pair.1, report.session.as_str()));
            } else {
                absent_reports.insert(pair);
            }
        }
        self.pending_prime.retain(|terminal, panes| {
            panes.retain(|pane| identities.contains(&(terminal.as_str(), pane.as_str())));
            !panes.is_empty()
        });
        self.outcomes.retain(|terminal, slots| {
            slots.retain(|tracked| {
                let pair = (terminal.as_str(), tracked.pane_id.as_str());
                (absent_reports.contains(&pair) && !reported_pairs.contains(&pair))
                    || reports.contains(&(pair.0, pair.1, tracked.session.as_str()))
            });
            !slots.is_empty()
        });
    }
    /// Compare a coherent same-stream snapshot with the previous observation.
    /// Only an unchanged, uniquely identified terminal/pane pair can produce a
    /// completion.  All other snapshot changes become the new baseline.
    fn reconcile_snapshot(&mut self, snapshot: herdr_protocol::Snapshot) -> Vec<(String, String)> {
        let pane_ids = snapshot.pane_ids();
        let completions = if pane_ids != self.pane_ids {
            Vec::new()
        } else {
            let mut old_pane_counts = HashMap::<&str, usize>::new();
            for agent in self.agents.values() {
                let count = old_pane_counts.entry(agent.pane_id.as_str()).or_default();
                *count = count.saturating_add(1);
            }
            let mut new_pane_counts = HashMap::<&str, usize>::new();
            let mut new_terminal_counts = HashMap::<&str, usize>::new();
            for agent in &snapshot.agents {
                let pane_count = new_pane_counts.entry(agent.pane_id.as_str()).or_default();
                *pane_count = pane_count.saturating_add(1);
                let terminal_count = new_terminal_counts
                    .entry(agent.terminal_id.as_str())
                    .or_default();
                *terminal_count = terminal_count.saturating_add(1);
            }
            let mut completions = Vec::new();
            for agent in &snapshot.agents {
                let Some(previous) = self.agents.get(&agent.terminal_id) else {
                    continue;
                };
                let same_identity = previous.pane_id == agent.pane_id;
                let unique_pane = old_pane_counts.get(previous.pane_id.as_str()).copied()
                    == Some(1)
                    && new_pane_counts.get(agent.pane_id.as_str()).copied() == Some(1);
                let unique_terminal =
                    new_terminal_counts.get(agent.terminal_id.as_str()).copied() == Some(1);
                if same_identity
                    && !previous.outcome_authoritative
                    && !agent.outcome_authoritative
                    && unique_pane
                    && unique_terminal
                    && !self
                        .ambiguous_terminal_ids
                        .contains(agent.terminal_id.as_str())
                    && matches!(previous.status, AgentStatus::Working | AgentStatus::Blocked)
                    && agent.status == AgentStatus::Done
                {
                    completions.push((agent.terminal_id.clone(), agent.pane_id.clone()));
                }
            }
            completions
        };
        self.replace_snapshot(snapshot);
        completions
    }

    fn mark_retry(&mut self, shared: &Arc<Mutex<AppState>>, now: Instant) {
        self.stream = None;
        if self.agents.is_empty() && self.last_sessions > 0 {
            let counts = SourceCounts {
                sessions: self.last_sessions,
                unknown: self.last_sessions,
                ..SourceCounts::default()
            };
            if let Ok(mut state) = shared.lock() {
                if !self.epoch_matches(&state) {
                    return;
                }
                let _ = state.update_source(&self.source, self.generation, false, counts);
            }
            wake_ui();
        } else {
            self.publish(shared, false);
        }
        self.next_attempt = now + self.backoff;
        self.backoff = self
            .backoff
            .checked_mul(2)
            .unwrap_or(MAX_BACKOFF)
            .min(MAX_BACKOFF);
    }

    fn mark_connected(&mut self, shared: &Arc<Mutex<AppState>>, now: Instant) {
        self.mark_connected_with_completions(shared, now, Vec::new());
    }

    fn mark_connected_with_completions(
        &mut self,
        shared: &Arc<Mutex<AppState>>,
        now: Instant,
        completions: Vec<(String, String)>,
    ) {
        self.backoff = INITIAL_BACKOFF;
        self.next_reconcile = now + RECONCILE_INTERVAL;
        self.publish_snapshot(shared, &completions, now);
    }
}

fn watcher_loop(
    shared: Arc<Mutex<AppState>>,
    command_rx: Receiver<Command>,
    stop: Arc<AtomicBool>,
    registry: RegistrationState,
    policy: SharedPolicy,
) {
    let mut endpoints = HashMap::<PathBuf, Endpoint>::new();
    let mut request_counter = 0u64;
    while !stop.load(Ordering::Acquire) {
        drain_commands(
            &shared,
            &command_rx,
            &mut endpoints,
            &stop,
            &registry,
            &policy,
        );
        if stop.load(Ordering::Acquire) || claim_shutdown(&shared, &policy, Instant::now(), false) {
            break;
        }
        for endpoint in endpoints.values_mut() {
            if stop.load(Ordering::Acquire)
                || claim_shutdown(&shared, &policy, Instant::now(), false)
            {
                break;
            }
            let was_connected = endpoint.stream.is_some();
            process_endpoint(endpoint, &shared, &mut request_counter, Instant::now());
            let mut guard = policy.lock().unwrap_or_else(|p| p.into_inner());
            guard.observe_connection(was_connected, endpoint.stream.is_some(), Instant::now());
            drop(guard);
            if stop.load(Ordering::Acquire)
                || claim_shutdown(&shared, &policy, Instant::now(), false)
            {
                break;
            }
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        if claim_shutdown(
            &shared,
            &policy,
            Instant::now(),
            all_endpoints_disabled(&endpoints),
        ) {
            break;
        }
        match command_rx.recv_timeout(WORKER_TICK) {
            Ok(command) => {
                handle_command(command, &shared, &mut endpoints, &stop, &registry, &policy)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn claim_shutdown(
    shared: &Arc<Mutex<AppState>>,
    policy: &SharedPolicy,
    now: Instant,
    all_disabled: bool,
) -> bool {
    let mut policy = policy.lock().unwrap_or_else(|p| p.into_inner());
    if policy.finalized {
        return true;
    }
    if !all_disabled && policy.connected == 0 {
        let remote_connected = shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .has_healthy_observed_remote();
        policy.observe_remote_connection(remote_connected, now);
    }
    if !all_disabled && !policy.expired(now) {
        return false;
    }
    policy.finalized = true;
    let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
    state.request_shutdown();
    drop(state);
    drop(policy);
    wake_ui();
    true
}

fn drain_commands(
    shared: &Arc<Mutex<AppState>>,
    command_rx: &Receiver<Command>,
    endpoints: &mut HashMap<PathBuf, Endpoint>,
    stop: &Arc<AtomicBool>,
    registry: &RegistrationState,
    policy: &SharedPolicy,
) {
    // A busy producer must not indefinitely postpone the deadline check.
    for _ in 0..COMMAND_QUEUE {
        let Ok(command) = command_rx.try_recv() else {
            break;
        };
        handle_command(command, shared, endpoints, stop, registry, policy);
    }
}

fn handle_command(
    command: Command,
    shared: &Arc<Mutex<AppState>>,
    endpoints: &mut HashMap<PathBuf, Endpoint>,
    stop: &Arc<AtomicBool>,
    registry: &RegistrationState,
    policy: &SharedPolicy,
) {
    match command {
        Command::Stop => stop.store(true, Ordering::Release),
        Command::Register(path) => {
            if policy.lock().unwrap_or_else(|p| p.into_inner()).finalized {
                return;
            }
            if endpoints.len() >= MAX_ENDPOINTS && !endpoints.contains_key(&path) {
                return;
            }
            if let Some(endpoint) = endpoints.get_mut(&path) {
                if endpoint.disabled {
                    endpoint.disabled = false;
                    endpoint.next_attempt = Instant::now();
                    endpoint.backoff = INITIAL_BACKOFF;
                    endpoint.begin_generation(shared);
                }
                return;
            }
            let mut endpoint = Endpoint::with_registry(path.clone(), Arc::clone(registry));
            endpoint.begin_generation(shared);
            endpoints.insert(path, endpoint);
        }
    }
}

fn all_endpoints_disabled(endpoints: &HashMap<PathBuf, Endpoint>) -> bool {
    !endpoints.is_empty() && endpoints.values().all(|endpoint| endpoint.disabled)
}

fn process_endpoint(
    endpoint: &mut Endpoint,
    shared: &Arc<Mutex<AppState>>,
    request_counter: &mut u64,
    now: Instant,
) {
    if endpoint.disabled {
        return;
    }
    if endpoint.stream.is_none() {
        if now >= endpoint.next_attempt {
            connect_endpoint(endpoint, shared, request_counter, now);
        }
        return;
    }

    let read = endpoint.stream.as_mut().map(SubscriptionReader::read_event);
    match read {
        Some(Ok(SubscriptionEventRead::Event(SubscriptionEvent::AgentStatusChanged {
            pane_id,
            status,
        }))) => {
            // Outcome metadata can follow status on a separate IPC connection.
            endpoint.next_reconcile = endpoint.next_reconcile.min(now + WORKER_TICK);
            let counts = source_counts(endpoint.counts());
            let source = endpoint.source.as_str();
            let generation = endpoint.generation;
            match apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                &pane_id,
                status,
                counts,
            ) {
                EventRecordUpdate::Applied {
                    terminal_id,
                    completion,
                    counts,
                } => {
                    publish_event(
                        shared,
                        EventPublication {
                            source,
                            generation,
                            terminal_id,
                            pane_id: &pane_id,
                            status,
                            counts,
                            completion,
                            observed_at: now,
                        },
                    );
                }
                EventRecordUpdate::Ambiguous => {
                    endpoint.next_reconcile = now;
                    endpoint.publish(shared, true);
                }
            }
        }
        Some(Ok(SubscriptionEventRead::Event(SubscriptionEvent::Other)))
        | Some(Ok(SubscriptionEventRead::Timeout)) => {}
        Some(Ok(SubscriptionEventRead::Closed)) | Some(Err(_)) | None => {
            endpoint.mark_retry(shared, now);
            return;
        }
    }

    if now >= endpoint.next_reconcile {
        reconcile_endpoint(endpoint, shared, request_counter, now);
    }
}

fn connect_endpoint(
    endpoint: &mut Endpoint,
    shared: &Arc<Mutex<AppState>>,
    request_counter: &mut u64,
    now: Instant,
) {
    endpoint.begin_generation(shared);
    let generation = endpoint.generation;
    let result = (|| -> Result<(SubscriptionReader, herdr_protocol::Snapshot), ProtocolError> {
        let ping = request(endpoint, request_counter, "ping", serde_json::json!({}))?.result()?;
        if ping.get("type").and_then(serde_json::Value::as_str) != Some("pong") {
            return Err(ProtocolError::InvalidResponse(
                "expected pong result".to_owned(),
            ));
        }
        let first = herdr_snapshot(endpoint, request_counter)?;
        endpoint.replace_snapshot(first.clone());
        match herdr_plugin(endpoint, request_counter)? {
            PluginAvailability::Enabled => {}
            PluginAvailability::Disabled => {
                return Err(ProtocolError::InvalidResponse("plugin disabled".to_owned()))
            }
        }
        converge_subscription(endpoint, request_counter, first)
    })();

    match result {
        Ok((stream, snapshot)) => {
            endpoint.replace_snapshot(snapshot);
            endpoint.stream = Some(stream);
            endpoint.mark_connected(shared, now);
        }
        Err(ProtocolError::InvalidResponse(message)) if message == "plugin disabled" => {
            endpoint.stream = None;
            endpoint.disabled = true;
            endpoint.set_registration_disabled(true);
            if let Ok(mut state) = shared.lock() {
                let _ = state.remove_source(&endpoint.source, generation);
            }
            wake_ui();
        }
        Err(_) => endpoint.mark_retry(shared, now),
    }
}

fn reconcile_endpoint(
    endpoint: &mut Endpoint,
    shared: &Arc<Mutex<AppState>>,
    request_counter: &mut u64,
    now: Instant,
) {
    let plugin = herdr_plugin(endpoint, request_counter);
    match plugin {
        Ok(PluginAvailability::Disabled) => {
            detach_endpoint(endpoint, shared);
            return;
        }
        Ok(PluginAvailability::Enabled) => {}
        Err(_) => {
            endpoint.mark_retry(shared, now);
            return;
        }
    }
    let Ok(snapshot) = herdr_snapshot(endpoint, request_counter) else {
        endpoint.mark_retry(shared, now);
        return;
    };
    let completions;
    if snapshot.pane_ids() != endpoint.pane_ids {
        completions = Vec::new();
        endpoint.replace_snapshot(snapshot.clone());
        match converge_subscription(endpoint, request_counter, snapshot) {
            Ok((stream, snapshot)) => {
                endpoint.stream = Some(stream);
                endpoint.replace_snapshot(snapshot);
            }
            Err(_) => endpoint.mark_retry(shared, now),
        }
    } else {
        completions = endpoint.reconcile_snapshot(snapshot);
    }
    if endpoint.stream.is_some() {
        endpoint.mark_connected_with_completions(shared, now, completions);
    }
}

fn converge_subscription(
    endpoint: &Endpoint,
    request_counter: &mut u64,
    mut snapshot: herdr_protocol::Snapshot,
) -> Result<(SubscriptionReader, herdr_protocol::Snapshot), ProtocolError> {
    for _ in 0..=MAX_PANE_CONVERGENCE {
        let target_panes = snapshot.pane_ids();
        let stream = subscribe(endpoint, request_counter, &target_panes)?;
        let fresh = herdr_snapshot(endpoint, request_counter)?;
        if fresh.pane_ids() == target_panes {
            return Ok((stream, fresh));
        }
        snapshot = fresh;
    }
    Err(ProtocolError::InvalidResponse(
        "pane set did not converge after subscription rebuilds".to_owned(),
    ))
}
fn detach_endpoint(endpoint: &mut Endpoint, shared: &Arc<Mutex<AppState>>) {
    endpoint.stream = None;
    endpoint.disabled = true;
    endpoint.set_registration_disabled(true);
    if let Ok(mut state) = shared.lock() {
        let _ = state.remove_source(&endpoint.source, endpoint.generation);
    }
    wake_ui();
}
fn request(
    endpoint: &Endpoint,
    request_counter: &mut u64,
    method: &str,
    params: serde_json::Value,
) -> Result<herdr_protocol::Response, ProtocolError> {
    *request_counter = request_counter.wrapping_add(1);
    let id = format!("desktop-pet-{}", *request_counter);
    herdr_protocol::request(&endpoint.path, &id, method, params, REQUEST_TIMEOUT)
}

fn herdr_snapshot(
    endpoint: &Endpoint,
    request_counter: &mut u64,
) -> Result<herdr_protocol::Snapshot, ProtocolError> {
    let response = request(
        endpoint,
        request_counter,
        "session.snapshot",
        serde_json::json!({}),
    )?;
    herdr_protocol::parse_snapshot(response)
}

fn herdr_plugin(
    endpoint: &Endpoint,
    request_counter: &mut u64,
) -> Result<PluginAvailability, ProtocolError> {
    let response = request(
        endpoint,
        request_counter,
        "plugin.list",
        serde_json::json!({ "plugin_id": "desktop-pet" }),
    )?;
    herdr_protocol::parse_plugin_availability(response)
}

fn subscribe(
    endpoint: &Endpoint,
    request_counter: &mut u64,
    pane_ids: &[String],
) -> Result<SubscriptionReader, ProtocolError> {
    *request_counter = request_counter.wrapping_add(1);
    let id = format!("desktop-pet-{}", *request_counter);
    herdr_protocol::subscribe(&endpoint.path, &id, pane_ids, REQUEST_TIMEOUT)
}

fn wake_ui() {
    crate::ui::wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(exit_with_herdr: bool) -> LifecycleSettings {
        LifecycleSettings {
            auto_start: false,
            exit_with_herdr,
        }
    }

    #[test]
    fn disconnect_grace_requires_last_healthy_connection_and_expires_at_boundary() {
        let start = Instant::now();
        let grace = DISCONNECTION_GRACE;
        let mut policy = ExitPolicy::new(settings(true), start);
        assert!(!policy.expired(start + grace - Duration::from_nanos(1)));
        assert!(policy.expired(start + grace));
        policy.observe_connection(false, true, start + Duration::from_secs(1));
        assert!(!policy.expired(start + grace));
        policy.observe_connection(false, true, start + Duration::from_secs(2));
        policy.observe_connection(true, false, start + Duration::from_secs(3));
        assert!(!policy.expired(start + grace * 3));
        policy.observe_connection(true, false, start + Duration::from_secs(4));
        assert!(!policy.expired(start + Duration::from_secs(4) + grace - Duration::from_nanos(1)));
        assert!(policy.expired(start + Duration::from_secs(4) + grace));
        // Repeated failed attempts do not constitute a connection transition.
        policy.observe_connection(false, false, start + Duration::from_secs(25));
        assert!(policy.expired(start + Duration::from_secs(4) + grace));
    }

    #[test]
    fn changing_exit_policy_resets_only_on_exit_toggle() {
        let start = Instant::now();
        let mut policy = ExitPolicy::new(settings(false), start);
        assert!(!policy.expired(start + DISCONNECTION_GRACE * 5));
        policy.change_settings(settings(true), start + Duration::from_secs(7));
        let deadline = start + Duration::from_secs(7) + DISCONNECTION_GRACE;
        policy.change_settings(
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            },
            start + Duration::from_secs(20),
        );
        assert_eq!(policy.deadline, Some(deadline));
        policy.change_settings(settings(false), deadline - Duration::from_nanos(1));
        assert!(!policy.expired(deadline));
        policy.change_settings(settings(true), deadline + Duration::from_secs(1));
        assert_eq!(
            policy.deadline,
            Some(deadline + Duration::from_secs(1) + DISCONNECTION_GRACE)
        );
        policy.observe_connection(false, true, deadline + Duration::from_secs(2));
        assert_eq!(policy.deadline, None);
    }

    #[test]
    fn selected_idle_remote_prevents_autoexit_until_its_own_grace_expires() {
        use crate::sources::{MachineInfo, MachineStatus, ObservationPreferences, SourceCatalog};

        let start = Instant::now();
        let mut state = AppState::new();
        let source = crate::sources::remote_source("east");
        state.set_observation_catalog(SourceCatalog {
            machines: vec![MachineInfo {
                id: "east".to_owned(),
                label: "East".to_owned(),
                remote_session: "session".to_owned(),
                enabled: true,
                status: MachineStatus::Online,
                error: None,
            }],
            error: None,
        });
        state.apply_observation_preferences(ObservationPreferences {
            local: false,
            remote: true,
            machines: vec!["east".to_owned()],
        });
        assert!(state.begin_source(source.clone(), 1));
        let empty: [AgentRecord; 0] = [];
        assert!(state.publish_source_snapshot(
            &source,
            1,
            empty.iter(),
            SourceCounts::default(),
            &[],
            start,
        ));
        let shared = Arc::new(Mutex::new(state));
        let policy = Arc::new(Mutex::new(ExitPolicy::new(settings(false), start)));
        assert!(!claim_shutdown(&shared, &policy, start, false));
        policy
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .change_settings(settings(true), start);
        assert!(!claim_shutdown(
            &shared,
            &policy,
            start + DISCONNECTION_GRACE,
            false,
        ));
        assert_eq!(
            policy.lock().unwrap_or_else(|p| p.into_inner()).deadline,
            None
        );

        assert!(shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .update_source(&source, 1, false, SourceCounts::default()));
        let lost = start + DISCONNECTION_GRACE + Duration::from_secs(1);
        assert!(!claim_shutdown(&shared, &policy, lost, false));
        let expires = lost + DISCONNECTION_GRACE;
        policy
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .change_settings(
                LifecycleSettings {
                    auto_start: true,
                    exit_with_herdr: true,
                },
                lost + Duration::from_secs(10),
            );
        assert!(!claim_shutdown(
            &shared,
            &policy,
            expires - Duration::from_nanos(1),
            false,
        ));
        assert!(claim_shutdown(&shared, &policy, expires, false));
        assert!(
            shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .scene()
                .shutdown
        );
    }

    #[test]
    fn rejected_persistence_preserves_cached_policy_and_state() {
        let shared = Arc::new(Mutex::new(AppState::new()));
        shared
            .lock()
            .unwrap()
            .set_lifecycle_settings(settings(false));
        let mut watchers = Watchers::new(Arc::clone(&shared));
        assert_eq!(
            watchers.update_lifecycle_settings(|| Err("disk full".to_owned())),
            Err("disk full".to_owned())
        );
        assert_eq!(shared.lock().unwrap().lifecycle_settings(), settings(false));
        assert_eq!(watchers.policy.lock().unwrap().settings, settings(false));
        watchers.shutdown();
    }

    fn record(terminal_id: &str, pane_id: &str, status: AgentStatus) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal_id.to_owned(),
            pane_id: pane_id.to_owned(),
            status,
            metadata: herdr_protocol::SessionMetadata::default(),
            outcome_authoritative: false,
            outcome: None,
        }
    }

    fn snapshot(panes: &[&str], agents: Vec<AgentRecord>) -> herdr_protocol::Snapshot {
        herdr_protocol::Snapshot {
            panes: panes.iter().map(|pane| (*pane).to_owned()).collect(),
            agents,
        }
    }

    fn selected_remote() -> (Arc<Mutex<AppState>>, String, u64) {
        use crate::sources::{MachineInfo, MachineStatus, ObservationPreferences, SourceCatalog};

        let mut state = AppState::new();
        let source = crate::sources::remote_source("machine-one");
        state.set_observation_catalog(SourceCatalog {
            machines: vec![MachineInfo {
                id: "machine-one".to_owned(),
                label: "Workstation".to_owned(),
                remote_session: "remote".to_owned(),
                enabled: true,
                status: MachineStatus::Online,
                error: None,
            }],
            error: None,
        });
        state.apply_observation_preferences(ObservationPreferences {
            local: true,
            remote: true,
            machines: vec!["machine-one".to_owned()],
        });
        let epoch = state.observation_epoch();
        (Arc::new(Mutex::new(state)), source, epoch)
    }

    #[test]
    fn remote_first_and_reconnected_snapshots_are_baselines_but_new_transitions_complete() {
        use crate::session_view::{Availability, SessionFilter};

        let (shared, source, epoch) = selected_remote();
        assert!(shared.lock().unwrap().begin_source(source.clone(), 1));
        let mut endpoint = Endpoint::remote(source.clone(), 1, epoch);
        let done = || {
            snapshot(
                &["pane"],
                vec![record("terminal", "pane", AgentStatus::Done)],
            )
        };
        let working = || {
            snapshot(
                &["pane"],
                vec![record("terminal", "pane", AgentStatus::Working)],
            )
        };
        endpoint.remote_snapshot(&shared, done(), Instant::now(), true);
        assert!(shared.lock().unwrap().take_completions().is_empty());
        endpoint.remote_snapshot(&shared, working(), Instant::now(), false);
        endpoint.remote_snapshot(&shared, done(), Instant::now(), false);
        {
            let mut state = shared.lock().unwrap();
            let completions = state.take_completions();
            assert_eq!(completions.len(), 1);
            assert_eq!(completions[0].source, source);
            assert_eq!(completions[0].terminal_id, "terminal");
            assert_eq!(completions[0].pane_id, "pane");
        }
        endpoint.remote_snapshot(&shared, done(), Instant::now(), false);
        assert!(shared.lock().unwrap().take_completions().is_empty());

        endpoint.remote_offline(&shared);
        {
            let mut state = shared.lock().unwrap();
            assert_eq!(state.scene().unknown, 1);
            let rows = state.session_snapshot(SessionFilter::All, None).rows;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].availability, Availability::Offline);
            assert!(state.take_completions().is_empty());
        }
        assert!(shared.lock().unwrap().begin_source(source.clone(), 2));
        let mut reconnected = Endpoint::remote(source.clone(), 2, epoch);
        reconnected.remote_snapshot(&shared, done(), Instant::now(), true);
        assert!(shared.lock().unwrap().take_completions().is_empty());
        reconnected.remote_snapshot(&shared, working(), Instant::now(), false);
        reconnected.remote_snapshot(&shared, done(), Instant::now(), false);
        let mut state = shared.lock().unwrap();
        assert_eq!(state.take_completions().len(), 1);
        assert_eq!(state.scene().done, 1);
        assert_eq!(
            state.session_snapshot(SessionFilter::All, None).rows[0].availability,
            Availability::Live
        );
    }

    #[test]
    fn old_remote_worker_cannot_publish_after_reinclude_even_with_newer_generation() {
        use crate::session_view::SessionFilter;
        use crate::sources::ObservationPreferences;

        let (shared, source, epoch) = selected_remote();
        assert!(shared.lock().unwrap().begin_source(source.clone(), 10));
        let mut old = Endpoint::remote(source.clone(), 10, epoch);
        let working = || {
            snapshot(
                &["pane"],
                vec![record("terminal", "pane", AgentStatus::Working)],
            )
        };
        let done = || {
            snapshot(
                &["pane"],
                vec![record("terminal", "pane", AgentStatus::Done)],
            )
        };
        old.remote_snapshot(&shared, working(), Instant::now(), true);
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: false,
                machines: vec!["machine-one".to_owned()],
            });
        assert_eq!(shared.lock().unwrap().scene().sessions, 0);
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: true,
                machines: vec!["machine-one".to_owned()],
            });
        let new_epoch = shared.lock().unwrap().observation_epoch();
        assert!(new_epoch > epoch);
        // A worker whose request began before exclusion must not initialize
        // a replacement source or overwrite a newly accepted observation.
        assert!(!shared.lock().unwrap().begin_source(source.clone(), 10));
        assert!(shared.lock().unwrap().begin_source(source.clone(), 11));
        let mut current = Endpoint::remote(source.clone(), 11, new_epoch);
        current.remote_snapshot(&shared, working(), Instant::now(), true);
        old.remote_snapshot(&shared, done(), Instant::now(), false);
        old.remote_offline(&shared);
        {
            let mut state = shared.lock().unwrap();
            assert_eq!(state.scene().working, 1);
            assert_eq!(state.scene().done, 0);
            let rows = state.session_snapshot(SessionFilter::All, None).rows;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].status, AgentStatus::Working);
            assert_eq!(rows[0].key.generation, 11);
            assert!(state.take_completions().is_empty());
            assert!(state.take_outcomes().is_empty());
        }
        current.remote_snapshot(&shared, done(), Instant::now(), false);
        let mut state = shared.lock().unwrap();
        assert_eq!(state.take_completions().len(), 1);
        assert_eq!(state.scene().done, 1);
    }

    #[test]
    fn remote_cached_outcomes_do_not_replay_after_reconnect_or_epoch_change() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        use crate::sources::ObservationPreferences;

        let (shared, source, epoch) = selected_remote();
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let report = |turn: u64, outcome| {
            snapshot(
                &["pane"],
                vec![AgentRecord {
                    outcome_authoritative: true,
                    outcome: Some(OutcomeReport {
                        session: "a".repeat(64),
                        turn: turn.to_string(),
                        outcome,
                        at_unix_ms: at,
                    }),
                    ..record("terminal", "pane", AgentStatus::Done)
                }],
            )
        };
        assert!(shared.lock().unwrap().begin_source(source.clone(), 20));
        let mut old = Endpoint::remote(source.clone(), 20, epoch);
        old.remote_snapshot(
            &shared,
            report(1, AgentOutcome::Succeeded),
            Instant::now(),
            true,
        );
        assert!(shared.lock().unwrap().take_outcomes().is_empty());
        old.remote_offline(&shared);
        assert!(shared.lock().unwrap().begin_source(source.clone(), 21));
        let mut reconnected = Endpoint::remote(source.clone(), 21, epoch);
        reconnected.remote_snapshot(
            &shared,
            report(1, AgentOutcome::Succeeded),
            Instant::now(),
            true,
        );
        assert!(shared.lock().unwrap().take_outcomes().is_empty());
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: false,
                machines: vec!["machine-one".to_owned()],
            });
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: true,
                machines: vec!["machine-one".to_owned()],
            });
        let new_epoch = shared.lock().unwrap().observation_epoch();
        assert!(shared.lock().unwrap().begin_source(source.clone(), 22));
        let mut current = Endpoint::remote(source.clone(), 22, new_epoch);
        current.remote_snapshot(
            &shared,
            report(1, AgentOutcome::Succeeded),
            Instant::now(),
            true,
        );
        reconnected.remote_snapshot(
            &shared,
            report(2, AgentOutcome::Failed),
            Instant::now(),
            false,
        );
        assert!(shared.lock().unwrap().take_outcomes().is_empty());
        current.remote_snapshot(
            &shared,
            report(2, AgentOutcome::Running),
            Instant::now(),
            false,
        );
        current.remote_snapshot(
            &shared,
            report(2, AgentOutcome::Failed),
            Instant::now(),
            false,
        );
        let mut state = shared.lock().unwrap();
        let outcomes = state.take_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].outcome, AgentOutcome::Failed);
        assert_eq!(outcomes[0].generation, 22);
        assert!(state.take_completions().is_empty());
    }
    #[test]
    fn authoritative_outcomes_do_not_infer_success_or_replay_across_generations() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        let shared = Arc::new(Mutex::new(AppState::new()));
        let take_outcomes = || match shared.lock() {
            Ok(mut state) => state.take_outcomes(),
            Err(_) => panic!("watcher state was poisoned"),
        };
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/outcome-regression.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let agent = |session: char, turn: u64, outcome| AgentRecord {
            outcome_authoritative: true,
            outcome: Some(OutcomeReport {
                session: session.to_string().repeat(64),
                turn: turn.to_string(),
                outcome,
                at_unix_ms: at,
            }),
            ..record("terminal", "pane", AgentStatus::Working)
        };
        endpoint.begin_generation(&shared);
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![agent('a', 100, AgentOutcome::Running)],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        let counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(matches!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane",
                AgentStatus::Done,
                counts
            ),
            EventRecordUpdate::Applied {
                completion: false,
                ..
            }
        ));
        let completions = endpoint.reconcile_snapshot(snapshot(
            &["pane"],
            vec![agent('a', 100, AgentOutcome::Failed)],
        ));
        assert!(completions.is_empty());
        endpoint.publish_snapshot(&shared, &completions, Instant::now());
        let outcomes = take_outcomes();
        assert_eq!(
            outcomes
                .iter()
                .map(|event| event.outcome)
                .collect::<Vec<_>>(),
            vec![AgentOutcome::Failed]
        );
        match shared.lock() {
            Ok(mut state) => assert!(state.take_completions().is_empty()),
            Err(_) => panic!("watcher state was poisoned"),
        }
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        assert!(take_outcomes().is_empty());
        endpoint.begin_generation(&shared);
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![agent('a', 100, AgentOutcome::Failed)],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        assert!(take_outcomes().is_empty());
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![agent('b', 1, AgentOutcome::Running)],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![agent('b', 1, AgentOutcome::Cancelled)],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        assert_eq!(
            take_outcomes()
                .iter()
                .map(|event| event.outcome)
                .collect::<Vec<_>>(),
            vec![AgentOutcome::Cancelled]
        );
    }

    #[test]
    fn short_turn_outcome_survives_coalesced_running_metadata() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        let shared = Arc::new(Mutex::new(AppState::new()));
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/short-outcome-regression.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.begin_generation(&shared);
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![AgentRecord {
                outcome_authoritative: true,
                ..record("terminal", "pane", AgentStatus::Unknown)
            }],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        // Both producer transitions occur between two snapshots. No running
        // report survives, but the authoritative terminal report is new.
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 1;
        endpoint.replace_snapshot(snapshot(
            &["pane"],
            vec![AgentRecord {
                outcome_authoritative: true,
                outcome: Some(OutcomeReport {
                    session: "e".repeat(64),
                    turn: "1".to_owned(),
                    outcome: AgentOutcome::Cancelled,
                    at_unix_ms: at,
                }),
                ..record("terminal", "pane", AgentStatus::Done)
            }],
        ));
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        let outcomes = shared.lock().unwrap().take_outcomes();
        assert_eq!(
            outcomes
                .iter()
                .map(|event| event.outcome)
                .collect::<Vec<_>>(),
            vec![AgentOutcome::Cancelled]
        );
        endpoint.publish_snapshot(&shared, &[], Instant::now());
        assert!(shared.lock().unwrap().take_outcomes().is_empty());
        assert!(shared.lock().unwrap().take_completions().is_empty());
    }

    #[test]
    fn ambiguous_identity_revokes_card_result_without_replaying_cached_report() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        use crate::session_view::{DisplayStatus, SessionFilter};

        for duplicate_terminal in [false, true] {
            let shared = Arc::new(Mutex::new(AppState::new()));
            let mut endpoint = Endpoint::with_registry(
                PathBuf::from("/tmp/ambiguous-outcome-regression.sock"),
                Arc::new(Mutex::new(HashMap::new())),
            );
            endpoint.begin_generation(&shared);
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let agent = |terminal: &str, turn: u64, outcome| AgentRecord {
                outcome_authoritative: true,
                outcome: Some(OutcomeReport {
                    session: "a".repeat(64),
                    turn: turn.to_string(),
                    outcome,
                    at_unix_ms: at,
                }),
                ..record(terminal, "pane", AgentStatus::Done)
            };
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent("first", 1, AgentOutcome::Running)],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent("first", 1, AgentOutcome::Succeeded)],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            let accepted_revision = {
                let mut state = shared.lock().unwrap();
                let view = state.session_snapshot(SessionFilter::All, None);
                assert_eq!(view.rows[0].display_status(), DisplayStatus::Succeeded);
                assert_eq!(view.status_summary.status, DisplayStatus::Succeeded);
                assert_eq!(state.take_outcomes().len(), 1);
                view.revision
            };
            let second = if duplicate_terminal {
                "first"
            } else {
                "second"
            };
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![
                    agent("first", 1, AgentOutcome::Succeeded),
                    agent(second, 1, AgentOutcome::Succeeded),
                ],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            let revoked_revision = {
                let mut state = shared.lock().unwrap();
                let view = state.session_snapshot(SessionFilter::Completed, None);
                assert!(view.revision > accepted_revision);
                assert_eq!(view.rows.len(), if duplicate_terminal { 1 } else { 2 });
                assert!(view.rows.iter().all(|row| row.outcome.is_none()));
                assert_eq!(view.status_summary.status, DisplayStatus::Completed);
                assert_eq!(view.status_summary.count, view.total);
                assert!(state.take_outcomes().is_empty());
                view.revision
            };
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            assert_eq!(shared.lock().unwrap().session_revision(), revoked_revision);

            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent("first", 1, AgentOutcome::Succeeded)],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            {
                let mut state = shared.lock().unwrap();
                let view = state.session_snapshot(SessionFilter::Working, None);
                assert!(view.rows.is_empty());
                assert_eq!(view.status_summary.status, DisplayStatus::Completed);
                assert_eq!(view.status_summary.total, 1);
                assert!(state.take_outcomes().is_empty());
            }
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent("first", 2, AgentOutcome::Running)],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent("first", 2, AgentOutcome::Succeeded)],
            ));
            endpoint.publish_snapshot(&shared, &[], Instant::now());
            let mut state = shared.lock().unwrap();
            let view = state.session_snapshot(SessionFilter::All, None);
            assert_eq!(view.rows[0].display_status(), DisplayStatus::Succeeded);
            assert_eq!(view.status_summary.status, DisplayStatus::Succeeded);
            assert_eq!(state.take_outcomes().len(), 1);
        }
    }

    #[test]
    fn candidate_order_does_not_replay_or_publish_ambiguous_terminal_reports() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        use crate::session_view::{DisplayStatus, SessionFilter};

        for duplicate_terminal in [false, true] {
            for reversed in [false, true] {
                let shared = Arc::new(Mutex::new(AppState::new()));
                let mut endpoint = Endpoint::with_registry(
                    PathBuf::from("/tmp/candidate-order.sock"),
                    Arc::new(Mutex::new(HashMap::new())),
                );
                endpoint.begin_generation(&shared);
                let agent =
                    |terminal: &str, pane: &str, session: char, turn: u64, outcome| AgentRecord {
                        outcome_authoritative: true,
                        outcome: Some(OutcomeReport {
                            session: session.to_string().repeat(64),
                            turn: turn.to_string(),
                            outcome,
                            at_unix_ms: 14_000,
                        }),
                        ..record(terminal, pane, AgentStatus::Done)
                    };
                let publish = |endpoint: &mut Endpoint| {
                    endpoint.publish_snapshot_at(&shared, &[], Instant::now(), 10_000);
                };
                let primary = |turn, outcome| agent("first", "pane", 'a', turn, outcome);
                endpoint
                    .replace_snapshot(snapshot(&["pane"], vec![primary(1, AgentOutcome::Running)]));
                publish(&mut endpoint);
                endpoint.replace_snapshot(snapshot(
                    &["pane"],
                    vec![primary(1, AgentOutcome::Succeeded)],
                ));
                publish(&mut endpoint);
                let accepted = {
                    let mut state = shared.lock().unwrap();
                    let view = state.session_snapshot(SessionFilter::All, None);
                    assert_eq!(view.status_summary.status, DisplayStatus::Succeeded);
                    assert_eq!(state.take_outcomes().len(), 1);
                    view.revision
                };
                let other = if duplicate_terminal {
                    agent("first", "other", 'b', 2, AgentOutcome::Failed)
                } else {
                    agent("second", "pane", 'b', 2, AgentOutcome::Failed)
                };
                let mut candidates = vec![primary(1, AgentOutcome::Succeeded), other];
                if reversed {
                    candidates.reverse();
                }
                endpoint.replace_snapshot(snapshot(&["pane", "other"], candidates));
                publish(&mut endpoint);
                let revoked = {
                    let mut state = shared.lock().unwrap();
                    let view = state.session_snapshot(SessionFilter::All, None);
                    assert!(view.revision > accepted);
                    assert!(view.rows.iter().all(|row| row.outcome.is_none()));
                    assert_eq!(view.status_summary.status, DisplayStatus::Completed);
                    assert!(state.take_outcomes().is_empty());
                    view.revision
                };
                endpoint.replace_snapshot(snapshot(
                    &["pane"],
                    vec![primary(1, AgentOutcome::Succeeded)],
                ));
                publish(&mut endpoint);
                {
                    let mut state = shared.lock().unwrap();
                    let view = state.session_snapshot(SessionFilter::All, None);
                    assert_eq!(view.status_summary.status, DisplayStatus::Completed);
                    assert!(view.rows.iter().all(|row| row.outcome.is_none()));
                    assert!(state.take_outcomes().is_empty());
                    assert!(view.revision >= revoked);
                }
                let stable = shared.lock().unwrap().session_revision();
                publish(&mut endpoint);
                assert_eq!(shared.lock().unwrap().session_revision(), stable);
                endpoint
                    .replace_snapshot(snapshot(&["pane"], vec![primary(2, AgentOutcome::Running)]));
                publish(&mut endpoint);
                endpoint.replace_snapshot(snapshot(
                    &["pane"],
                    vec![primary(2, AgentOutcome::Succeeded)],
                ));
                publish(&mut endpoint);
                let mut state = shared.lock().unwrap();
                assert_eq!(state.take_outcomes().len(), 1);
                assert_eq!(
                    state
                        .session_snapshot(SessionFilter::All, None)
                        .status_summary
                        .status,
                    DisplayStatus::Succeeded
                );
            }
        }
    }

    #[test]
    fn ambiguous_first_reports_and_temporary_authority_loss_keep_consumption() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        use crate::session_view::{DisplayStatus, SessionFilter};

        for first_is_running in [false, true] {
            let shared = Arc::new(Mutex::new(AppState::new()));
            let mut endpoint = Endpoint::with_registry(
                PathBuf::from("/tmp/first-ambiguous.sock"),
                Arc::new(Mutex::new(HashMap::new())),
            );
            endpoint.begin_generation(&shared);
            let publish = |endpoint: &mut Endpoint| {
                endpoint.publish_snapshot_at(&shared, &[], Instant::now(), 10_000);
            };
            let agent = |session: char, turn: u64, outcome| AgentRecord {
                outcome_authoritative: true,
                outcome: Some(OutcomeReport {
                    session: session.to_string().repeat(64),
                    turn: turn.to_string(),
                    outcome,
                    at_unix_ms: 14_000,
                }),
                ..record("terminal", "pane", AgentStatus::Done)
            };
            endpoint.replace_snapshot(snapshot(&["pane"], vec![]));
            publish(&mut endpoint);
            let first = if first_is_running {
                AgentOutcome::Running
            } else {
                AgentOutcome::Succeeded
            };
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent('a', 1, first), agent('b', 1, AgentOutcome::Failed)],
            ));
            publish(&mut endpoint);
            assert!(shared.lock().unwrap().take_outcomes().is_empty());
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent('a', 1, AgentOutcome::Succeeded)],
            ));
            publish(&mut endpoint);
            {
                let mut state = shared.lock().unwrap();
                let view = state.session_snapshot(SessionFilter::All, None);
                assert_eq!(
                    view.status_summary.status,
                    if first_is_running {
                        DisplayStatus::Succeeded
                    } else {
                        DisplayStatus::Completed
                    }
                );
                assert_eq!(state.take_outcomes().len(), usize::from(first_is_running));
            }
            // A temporarily missing report or authority revokes the mirror,
            // but cannot reset the already consumed turn.
            let mut absent = agent('a', 1, AgentOutcome::Succeeded);
            absent.outcome = None;
            endpoint.replace_snapshot(snapshot(&["pane"], vec![absent.clone()]));
            publish(&mut endpoint);
            absent.outcome_authoritative = false;
            endpoint.replace_snapshot(snapshot(&["pane"], vec![absent]));
            publish(&mut endpoint);
            endpoint.replace_snapshot(snapshot(
                &["pane"],
                vec![agent('a', 1, AgentOutcome::Succeeded)],
            ));
            publish(&mut endpoint);
            let mut state = shared.lock().unwrap();
            assert!(state.take_outcomes().is_empty());
            let view = state.session_snapshot(SessionFilter::All, None);
            assert!(view.rows.iter().all(|row| row.outcome.is_none()));
            assert_eq!(view.status_summary.status, DisplayStatus::Completed);
        }
    }

    #[test]
    fn canonical_endpoint_deduplicates_lexical_aliases() {
        let first =
            crate::socket::canonical_endpoint(Path::new("/tmp/herdr/../herdr.sock")).unwrap();
        let second = crate::socket::canonical_endpoint(Path::new("/tmp/herdr.sock")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn startup_and_reconnect_snapshots_are_baselines() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Done)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Done)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn only_active_to_done_transition_is_observed_once() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: true,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: false,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Unknown)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: false,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
    }

    #[test]
    fn ambiguous_pane_events_do_not_update_or_complete() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![
                record("terminal-a", "pane-a", AgentStatus::Working),
                record("terminal-b", "pane-a", AgentStatus::Blocked),
            ],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Ambiguous
        );
        let counts = endpoint.counts();
        assert_eq!(counts.working, 1);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.done, 0);
    }

    #[test]
    fn duplicate_terminal_baseline_is_disarmed_until_unique_baseline() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![
                record("terminal-a", "pane-a", AgentStatus::Working),
                record("terminal-a", "pane-a", AgentStatus::Working),
            ],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Ambiguous
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: true,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
    }
    #[test]
    fn snapshot_remap_and_new_or_unknown_sessions_do_not_complete() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a", "pane-b"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a", "pane-b"],
                vec![record("terminal-a", "pane-b", AgentStatus::Done)],
            ))
            .is_empty());
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a", "pane-b"],
                vec![record("terminal-b", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn coherent_snapshot_transition_completes_once() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Blocked)],
        ));
        assert_eq!(
            endpoint.reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            )),
            vec![("terminal-a".to_owned(), "pane-a".to_owned())]
        );
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn metadata_changes_never_complete_or_fabricate_omp_outcomes() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        let mut original = record("terminal", "pane", AgentStatus::Working);
        original.outcome_authoritative = true;
        original.metadata.title = Some("Split README".to_owned());
        endpoint.replace_snapshot(snapshot(&["pane"], vec![original.clone()]));
        let mut changed = original.clone();
        changed.metadata.title = Some("Different title".to_owned());
        changed.metadata.workspace_label = Some("Idea".to_owned());
        assert!(endpoint
            .reconcile_snapshot(snapshot(&["pane"], vec![changed.clone()]))
            .is_empty());
        assert_eq!(endpoint.agents["terminal"].metadata, changed.metadata);
        changed.status = AgentStatus::Done;
        assert!(endpoint
            .reconcile_snapshot(snapshot(&["pane"], vec![changed]))
            .is_empty());
        assert!(endpoint.agents["terminal"].outcome.is_none());
    }

    #[test]
    fn endpoint_counts_do_not_use_provider_specific_fields() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            Arc::new(Mutex::new(HashMap::new())),
        );
        endpoint.agents.insert(
            "terminal-a".to_owned(),
            AgentRecord {
                terminal_id: "terminal-a".to_owned(),
                pane_id: "pane-a".to_owned(),
                status: AgentStatus::Working,
                metadata: herdr_protocol::SessionMetadata::default(),
                outcome_authoritative: false,
                outcome: None,
            },
        );
        endpoint.agents.insert(
            "terminal-b".to_owned(),
            AgentRecord {
                terminal_id: "terminal-b".to_owned(),
                pane_id: "pane-b".to_owned(),
                status: AgentStatus::Done,
                metadata: herdr_protocol::SessionMetadata::default(),
                outcome_authoritative: false,
                outcome: None,
            },
        );
        let counts = endpoint.counts();
        assert_eq!(counts.sessions, 2);
        assert_eq!(counts.working, 1);
        assert_eq!(counts.done, 1);
    }
}
