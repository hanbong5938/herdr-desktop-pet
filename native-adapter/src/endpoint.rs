use crate::{Route, Shared};
use crate::outcome::OutcomeTracker;
use crate::protocol::{self, PluginAvailability, ProtocolError, SubscriptionEvent, SubscriptionEventRead, SubscriptionReader};
use desktop_pet_contracts::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(15);
const MAX_PANE_CONVERGENCE: usize = 3;
const WORKER_TICK: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(650);
pub(crate) struct Endpoint {
    path: PathBuf,
    source: String,
    registry: Arc<Shared>,
    generation: u64,
    next_generation: u64,
    // Retry only a denied begin on ordinary coherent snapshots, never an
    // already-admitted or superseded generation.
    source_admission_pending: bool,
    agents: HashMap<String, AgentRecord>,
    outcomes: HashMap<String, Vec<TrackedOutcome>>,
    duplicate_agents: Vec<AgentRecord>,
    pending_prime: HashMap<String, HashSet<String>>,
    ambiguous_terminal_ids: HashSet<String>,
    outcome_snapshot_unix_ms: Option<u64>,
    last_sessions: usize,
    pane_ids: Arc<[String]>,
    stream: Option<SubscriptionReader>,
    next_attempt: Instant,
    next_reconcile: Instant,
    backoff: Duration,
    disabled: bool,
    seen_plugin_present: bool,
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
    observation_epoch: Option<u64>,
    terminal_id: &'a str,
    pane_id: &'a str,
    status: AgentStatus,
    counts: SourceCounts,
    completion: bool,
    observed_at: Instant,
}

fn publish_event(shared: &Arc<Shared>, publication: EventPublication<'_>) {
    let _ = shared.with_epoch(publication.observation_epoch.unwrap_or(0), &mut |domain, _| {
        domain.update_session_status(SessionStatusUpdate {
            source: publication.source, generation: publication.generation,
            terminal_id: publication.terminal_id, pane_id: publication.pane_id,
            status: publication.status, counts: publication.counts,
            completion: publication.completion, observed_at: publication.observed_at,
        });
    });
    shared.wake();
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
    fn with_registry(path: PathBuf, registry: Arc<Shared>) -> Self {
        let source = path.to_string_lossy().into_owned();
        Self {
            path,
            source,
            registry,
            generation: 0,
            next_generation: 0,
            source_admission_pending: false,
            ambiguous_terminal_ids: HashSet::new(),
            agents: HashMap::new(),
            duplicate_agents: Vec::new(),
            pending_prime: HashMap::new(),
            outcomes: HashMap::new(),
            outcome_snapshot_unix_ms: None,
            last_sessions: 0,
            pane_ids: Arc::from([]),
            stream: None,
            next_attempt: Instant::now(),
            next_reconcile: Instant::now(),
            backoff: INITIAL_BACKOFF,
            disabled: false,
            seen_plugin_present: false,
            observation_epoch: None,
        }
    }
    pub(crate) fn remote(shared: Arc<Shared>, source: String, generation: u64, epoch: u64) -> Self {
        let mut endpoint = Self::with_registry(PathBuf::new(), shared);
        endpoint.source = source;
        endpoint.generation = generation;
        endpoint.next_generation = generation;
        endpoint.observation_epoch = Some(epoch);
        endpoint.source_admission_pending = true;
        endpoint
    }
    pub(crate) fn remote_snapshot(
        &mut self,
        shared: &Arc<Shared>,
        snapshot: Snapshot,
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

    pub(crate) fn remote_offline(&mut self, shared: &Arc<Shared>) {
        self.mark_retry(shared, Instant::now());
    }
    fn reconcile_observation(&mut self, shared: &Arc<Shared>, now: Instant) -> bool {
        let Some(epoch) = shared.epoch() else { self.stream = None; return false; };
        if self.observation_epoch != Some(epoch) {
            self.observation_epoch = Some(epoch);
            self.generation = 0;
            self.source_admission_pending = false;
            self.stream = None;
            self.agents.clear(); self.duplicate_agents.clear(); self.pending_prime.clear();
            self.outcomes.clear(); self.outcome_snapshot_unix_ms = None;
            self.ambiguous_terminal_ids.clear(); self.last_sessions = 0; self.pane_ids = Arc::from([]);
            self.next_attempt = now; self.next_reconcile = now; self.backoff = INITIAL_BACKOFF;
            self.disabled = false; self.seen_plugin_present = false;
            self.set_registration_disabled(false);
        }
        true
    }
    fn check_observation(&self, shared: &Arc<Shared>) -> Result<(), ProtocolError> {
        if self.observation_epoch.is_some_and(|epoch| shared.current(epoch)) { Ok(()) }
        else { Err(ProtocolError::InvalidResponse("Herdr observation epoch retired".to_owned())) }
    }
    fn set_registration_disabled(&self, disabled: bool) {
        if let Ok(mut registry) = self.registry.registry.lock() {
            if let Some(binding) = registry.bindings.get_mut(&self.source) { binding.disabled = disabled; }
        }
    }
    fn descriptor(&self) -> SourceDescriptor {
        let remote = self.path.as_os_str().is_empty();
        SourceDescriptor {
            id: self.source.clone(), provider: "herdr".to_owned(), label: self.source.clone(),
            locality: if remote { SourceLocality::Remote } else { SourceLocality::Local },
            capabilities: SourceCapabilities { prompt: !remote, remove_worktree: !remote },
        }
    }
    fn route(&self) -> Route {
        if self.path.as_os_str().is_empty() { Route::Remote(self.source.clone()) } else { Route::Local(self.path.clone()) }
    }
    fn begin_generation(&mut self, shared: &Arc<Shared>) -> bool {
        let epoch = self.observation_epoch.unwrap_or(0);
        let mut started = false;
        let source = self.source.clone();
        let Some(minimum_generation) = self.next_generation.checked_add(1) else { return false; };
        let result = shared.admission_transaction(epoch, &source, minimum_generation, &mut |domain, registry, generation| {
            self.next_generation = generation; self.generation = generation;
            self.source_admission_pending = !shared.begin(domain, registry, self.descriptor(), generation, self.route());
            started = true;
        });
        if result.is_err() || !started { return false; }
        if !self.agents.is_empty() { self.last_sessions = self.agents.len(); }
        self.stream = None; self.agents.clear(); self.duplicate_agents.clear(); self.pending_prime.clear();
        self.outcomes.clear(); self.outcome_snapshot_unix_ms = None; self.ambiguous_terminal_ids.clear(); self.pane_ids = Arc::from([]);
        shared.wake(); true
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
    fn publish(&self, shared: &Arc<Shared>, connected: bool) {
        let _ = shared.with_epoch(self.observation_epoch.unwrap_or(0), &mut |domain, registry| {
            if !connected { domain.mark_source_offline(&self.source, self.generation); }
            if let Some(binding) = registry.bindings.get_mut(&self.source).filter(|binding| binding.generation == self.generation) {
                if !connected { binding.connected = false; binding.offline_since.get_or_insert(Instant::now()); }
            }
            Shared::observe_connections(registry, Instant::now());
        });
        shared.wake();
    }
    fn publish_snapshot(
        &mut self,
        shared: &Arc<Shared>,
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
        shared: &Arc<Shared>,
        completions: &[(String, String)],
        observed_at: Instant,
        now_unix_ms: u64,
    ) {
        let counts = self.counts();
        let source = self.source.clone();
        let _ = shared.capacity_transaction(self.observation_epoch.unwrap_or(0), &source, self.generation, &mut |state, registry| {
            let baseline = self.source_admission_pending;
            if baseline {
                match state.source_generation(&self.source) {
                    Some(generation) if generation == self.generation => {}
                    Some(_) => {
                        // A different registered generation owns this source.
                        // Never let a later removal revive this stale endpoint.
                        self.source_admission_pending = false;
                        return;
                    }
                    None => {
                        if !shared.begin(state, registry, self.descriptor(), self.generation, self.route()) {
                            return;
                        }
                    }
                }
                self.source_admission_pending = false;
                // Observations made while admission was denied are a baseline,
                // not completion transitions or fresh outcome reports.
                self.outcomes.clear();
                self.pending_prime.clear();
                self.outcome_snapshot_unix_ms = None;
            }
            if !state.publish_source_snapshot(
                &self.source,
                self.generation,
                &mut self.agents.values(),
                SourceCounts {
                    sessions: counts.sessions,
                    working: counts.working,
                    blocked: counts.blocked,
                    done: counts.done,
                    unknown: counts.unknown,
                },
                Arc::clone(&self.pane_ids),
                if baseline { &[] } else { completions },
                observed_at,
            ) {
                return;
            }
            if let Some(binding) = registry.bindings.get_mut(&self.source).filter(|binding| binding.generation == self.generation) {
                binding.connected = true; binding.offline_since = None;
            }
            Shared::observe_connections(registry, observed_at);
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
        });
        shared.wake();
    }
    fn replace_snapshot(&mut self, snapshot: Snapshot) {
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
        let panes = snapshot.pane_ids();
        if panes.as_slice() != self.pane_ids.as_ref() {
            self.pane_ids = Arc::from(panes);
        }
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
    fn reconcile_snapshot(&mut self, snapshot: Snapshot) -> Vec<(String, String)> {
        let pane_ids = snapshot.pane_ids();
        let completions = if pane_ids.as_slice() != self.pane_ids.as_ref() {
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
    fn mark_retry(&mut self, shared: &Arc<Shared>, now: Instant) {
        self.stream = None; self.publish(shared, false);
        self.next_attempt = now + self.backoff;
        self.backoff = self.backoff.checked_mul(2).unwrap_or(MAX_BACKOFF).min(MAX_BACKOFF);
    }
    fn mark_connected(&mut self, shared: &Arc<Shared>, now: Instant) {
        self.mark_connected_with_completions(shared, now, Vec::new());
    }

    fn mark_connected_with_completions(
        &mut self,
        shared: &Arc<Shared>,
        now: Instant,
        completions: Vec<(String, String)>,
    ) {
        self.backoff = INITIAL_BACKOFF;
        self.next_reconcile = now + RECONCILE_INTERVAL;
        self.publish_snapshot(shared, &completions, now);
    }
}

pub(crate) fn watcher_loop(shared: Arc<Shared>) {
    let mut endpoints = HashMap::<PathBuf, Endpoint>::new();
    let mut request_counter = 0;
    let mut epoch = None;
    let mut endpoint_revision = None;
    let mut next_load = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        if let Some(current) = shared.epoch() {
            if epoch != Some(current) { endpoints.clear(); epoch = Some(current); endpoint_revision = None; next_load = Instant::now(); }
            let load = shared.registry.lock().ok().and_then(|registry| (!registry.endpoints_loaded).then(|| registry.configured_socket.clone()));
            if let Some(configured) = load.filter(|_| Instant::now() >= next_load) {
                let result = (|| {
                    let store = shared.store.lock().map_err(|_| "Herdr profile is poisoned")?.clone().ok_or("Herdr profile is not configured")?;
                    let mut paths = crate::settings::read_endpoints(&store)?;
                    let path = crate::socket::canonical_endpoint(&configured.unwrap_or_else(crate::settings::default_herdr_socket))?;
                    if !paths.contains(&path) { paths.push(path); }
                    Ok::<_, String>(paths)
                })();
                match result {
                    Ok(paths) => { let _ = shared.with_epoch(current, &mut |_, registry| {
                        for path in &paths {
                            if !registry.initial_endpoints.contains(path) && registry.initial_endpoints.len() < crate::MAX_ENDPOINTS {
                                registry.initial_endpoints.push(path.clone()); registry.endpoint_revision += 1;
                            }
                        }
                        registry.endpoints_loaded = true;
                    }); }
                    Err(error) => { shared.fail(error); next_load = Instant::now() + RECONCILE_INTERVAL; }
                }
            }
            let paths = shared.registry.lock().ok().and_then(|registry| {
                if endpoint_revision != Some(registry.endpoint_revision) { endpoint_revision = Some(registry.endpoint_revision); Some(registry.initial_endpoints.clone()) } else { None }
            }).unwrap_or_default();
            for path in paths { if endpoints.len() < crate::MAX_ENDPOINTS && !endpoints.contains_key(&path) { endpoints.insert(path.clone(), Endpoint::with_registry(path, Arc::clone(&shared))); } }
            for endpoint in endpoints.values_mut() {
                if !shared.current(current) { break; }
                if endpoint.disabled && shared.registry.lock().is_ok_and(|registry| registry.bindings.get(&endpoint.source).is_some_and(|binding| !binding.disabled)) { endpoint.disabled = false; endpoint.next_attempt = Instant::now(); }
                process_endpoint(endpoint, &shared, &mut request_counter, Instant::now());
            }
        } else { endpoints.clear(); epoch = None; }
        thread::sleep(WORKER_TICK);
    }
}
fn process_endpoint(
    endpoint: &mut Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    now: Instant,
) {
    if !endpoint.reconcile_observation(shared, now) || endpoint.disabled {
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
                            observation_epoch: endpoint.observation_epoch,
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
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    now: Instant,
) {
    if !endpoint.begin_generation(shared) {
        return;
    }
    let result = (|| -> Result<Option<(SubscriptionReader, Snapshot)>, ProtocolError> {
        let ping = request(
            endpoint,
            shared,
            request_counter,
            "ping",
            serde_json::json!({}),
        )?
        .result()?;
        if ping.get("type").and_then(serde_json::Value::as_str) != Some("pong") {
            return Err(ProtocolError::InvalidResponse(
                "expected pong result".to_owned(),
            ));
        }
        let first = herdr_snapshot(endpoint, shared, request_counter)?;
        endpoint.replace_snapshot(first.clone());
        let availability = herdr_plugin(endpoint, shared, request_counter)?;
        if !plugin_allows_observation(endpoint, availability) {
            return Ok(None);
        }
        converge_subscription(endpoint, shared, request_counter, first).map(Some)
    })();
    if endpoint.check_observation(shared).is_err() {
        return;
    }

    match result {
        Ok(Some((stream, snapshot))) => {
            endpoint.replace_snapshot(snapshot);
            endpoint.stream = Some(stream);
            endpoint.mark_connected(shared, now);
        }
        Ok(None) => detach_endpoint(endpoint, shared),
        Err(_) => endpoint.mark_retry(shared, now),
    }
}

fn reconcile_endpoint(
    endpoint: &mut Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    now: Instant,
) {
    match herdr_plugin(endpoint, shared, request_counter) {
        Ok(availability) => {
            if endpoint.check_observation(shared).is_err() {
                return;
            }
            if !plugin_allows_observation(endpoint, availability) {
                detach_endpoint(endpoint, shared);
                return;
            }
        }
        Err(_) => {
            endpoint.mark_retry(shared, now);
            return;
        }
    }
    let Ok(snapshot) = herdr_snapshot(endpoint, shared, request_counter) else {
        endpoint.mark_retry(shared, now);
        return;
    };
    if endpoint.check_observation(shared).is_err() {
        return;
    }
    let completions;
    if snapshot.pane_ids().as_slice() != endpoint.pane_ids.as_ref() {
        completions = Vec::new();
        endpoint.replace_snapshot(snapshot.clone());
        match converge_subscription(endpoint, shared, request_counter, snapshot) {
            Ok((stream, snapshot)) => {
                if endpoint.check_observation(shared).is_err() {
                    return;
                }
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
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    mut snapshot: Snapshot,
) -> Result<(SubscriptionReader, Snapshot), ProtocolError> {
    for _ in 0..=MAX_PANE_CONVERGENCE {
        let target_panes = snapshot.pane_ids();
        let stream = subscribe(endpoint, shared, request_counter, &target_panes)?;
        let fresh = herdr_snapshot(endpoint, shared, request_counter)?;
        if fresh.pane_ids() == target_panes {
            return Ok((stream, fresh));
        }
        snapshot = fresh;
    }
    Err(ProtocolError::InvalidResponse(
        "pane set did not converge after subscription rebuilds".to_owned(),
    ))
}
fn plugin_allows_observation(endpoint: &mut Endpoint, availability: PluginAvailability) -> bool {
    match availability {
        PluginAvailability::Enabled => {
            endpoint.seen_plugin_present = true;
            true
        }
        PluginAvailability::Disabled => {
            endpoint.seen_plugin_present = true;
            false
        }
        PluginAvailability::Missing => !endpoint.seen_plugin_present,
    }
}
fn detach_endpoint(endpoint: &mut Endpoint, shared: &Arc<Shared>) {
    let _ = shared.with_epoch(endpoint.observation_epoch.unwrap_or(0), &mut |domain, registry| {
        endpoint.stream = None; endpoint.disabled = true;
        domain.remove_source(&endpoint.source, endpoint.generation);
        registry.operations.retain(|_, operation| {
            if operation.source == endpoint.source { operation.cancel(); false } else { true }
        });
        if let Some(binding) = registry.bindings.get_mut(&endpoint.source) { binding.disabled = true; binding.connected = false; binding.offline_since.get_or_insert(Instant::now()); }
        Shared::observe_connections(registry, Instant::now());
    });
    shared.wake();
}
fn request(
    endpoint: &Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    method: &str,
    params: serde_json::Value,
) -> Result<protocol::Response, ProtocolError> {
    endpoint.check_observation(shared)?;
    *request_counter = request_counter.wrapping_add(1);
    let id = format!("desktop-pet-{}", *request_counter);
    protocol::request(&endpoint.path, &id, method, params, REQUEST_TIMEOUT)
}

fn herdr_snapshot(
    endpoint: &Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
) -> Result<Snapshot, ProtocolError> {
    let response = request(
        endpoint,
        shared,
        request_counter,
        "session.snapshot",
        serde_json::json!({}),
    )?;
    protocol::parse_snapshot(response)
}

fn herdr_plugin(
    endpoint: &Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
) -> Result<PluginAvailability, ProtocolError> {
    let response = request(
        endpoint,
        shared,
        request_counter,
        "plugin.list",
        serde_json::json!({ "plugin_id": "desktop-pet" }),
    )?;
    protocol::parse_plugin_availability(response)
}

fn subscribe(
    endpoint: &Endpoint,
    shared: &Arc<Shared>,
    request_counter: &mut u64,
    pane_ids: &[String],
) -> Result<SubscriptionReader, ProtocolError> {
    endpoint.check_observation(shared)?;
    *request_counter = request_counter.wrapping_add(1);
    let id = format!("desktop-pet-{}", *request_counter);
    protocol::subscribe(&endpoint.path, &id, pane_ids, REQUEST_TIMEOUT)
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
