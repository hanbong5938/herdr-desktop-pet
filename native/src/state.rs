use crate::automation::{
    PresentationAction, PresentationPatch, PresentationTarget, SessionIdentity, SharedAutomation,
};
use crate::bubble::BubblePlacement;
use crate::herdr_protocol::{AgentRecord, AgentStatus};
use crate::lifecycle::LifecycleSettings;
use crate::session::{CompletionObservation, OutcomeObservation};
use crate::session_view::{
    CardDisplay, PromptTarget, PromptTargetError, SessionKey, SessionListOptions,
    SessionPageCursor, SessionPageError, SessionPageRequest, SessionSnapshot, SessionStore,
    SessionView, WorktreeRemoveTarget, WorktreeRemoveTargetError,
};
use crate::sources::{remote_machine_id, remote_source, ObservationPreferences, SourceCatalog};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_COMPLETIONS: usize = 64;
const COMPLETION_MAX_AGE: Duration = Duration::from_secs(30);
/// Only export stored, displayable metadata. In particular, raw source paths,
/// full working directories, and worktree root/checkout paths are not session wire data.
fn automation_session_row(instance_id: &str, view: &SessionView) -> Value {
    use crate::agent_outcome::AgentOutcome;
    let status = match view.status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Unknown => "unknown",
    };
    let outcome = view.outcome.map(|outcome| match outcome {
        AgentOutcome::Running => "running",
        AgentOutcome::Succeeded => "succeeded",
        AgentOutcome::Failed => "failed",
        AgentOutcome::Cancelled => "cancelled",
    });
    json!({
        "key": {
            "instance_id": instance_id,
            "source_id": view.key.source_id,
            "generation": view.key.generation,
            "terminal_id": view.key.terminal_id,
        },
        "source_label": view.source_label,
        "is_local": view.is_local,
        "pane_id": view.pane_id,
        "availability": view.availability,
        "status": status,
        "outcome": outcome,
        "display_status": view.display_status(),
        "metadata": {
            "title": view.metadata.title,
            "agent": view.metadata.agent,
            "workspace_id": view.metadata.workspace_id,
            "workspace_label": view.metadata.workspace_label,
            "tab_id": view.metadata.tab_id,
            "tab_label": view.metadata.tab_label,
        },
    })
}

fn completion_is_fresh(observed_at: Instant, now: Instant) -> bool {
    now.checked_duration_since(observed_at)
        .is_none_or(|age| age <= COMPLETION_MAX_AGE)
}
fn completion_matches_source(
    observation: &CompletionObservation,
    source: &SourceState,
    now: Instant,
) -> bool {
    source.connected
        && source.generation == observation.generation
        && completion_is_fresh(observation.observed_at, now)
}

fn purge_completions_for_source(completions: &mut VecDeque<CompletionObservation>, source: &str) {
    completions.retain(|observation| observation.source != source);
}

pub(crate) const MAX_COUNT: u32 = 1_000_000;
pub(crate) const DEFAULT_SCALE: f64 = 0.65;
pub(crate) const MIN_SCALE: f64 = 0.35;
pub(crate) const MAX_SCALE: f64 = 1.25;

pub(crate) fn normalize_scale(scale: f64) -> Option<f64> {
    scale.is_finite().then(|| scale.clamp(MIN_SCALE, MAX_SCALE))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Running,
    Waiting,
    Unknown,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub phase: Phase,
    pub sessions: usize,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub unknown: usize,
    pub connected_sources: usize,
    pub disconnected_sources: usize,
    pub visible: bool,
    pub passthrough: bool,
    pub alpha_passthrough: bool,
    pub bubble_visible: bool,
    pub bubble_placement: BubblePlacement,
    pub scale: f64,
    pub reset_position_revision: u64,
    pub shutdown: bool,
}

impl Scene {
    pub(crate) fn needs_recovery_entry(&self) -> bool {
        !self.shutdown && (self.passthrough || (!self.visible && !self.bubble_visible))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceCounts {
    pub sessions: usize,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub unknown: usize,
}

impl SourceCounts {
    pub(crate) fn disconnected(self) -> Self {
        let sessions = self.sessions.min(MAX_COUNT as usize);
        Self {
            sessions,
            working: 0,
            blocked: 0,
            done: 0,
            unknown: sessions,
        }
    }

    fn bounded(self) -> Self {
        let max = MAX_COUNT as usize;
        Self {
            sessions: self.sessions.min(max),
            working: self.working.min(max),
            blocked: self.blocked.min(max),
            done: self.done.min(max),
            unknown: self.unknown.min(max),
        }
    }
}
pub(crate) struct SessionStatusUpdate<'a> {
    pub(crate) source: &'a str,
    pub(crate) generation: u64,
    pub(crate) terminal_id: &'a str,
    pub(crate) pane_id: &'a str,
    pub(crate) status: AgentStatus,
    pub(crate) counts: SourceCounts,
    pub(crate) completion: bool,
    pub(crate) observed_at: Instant,
}

#[derive(Clone, Debug)]
struct SourceState {
    generation: u64,
    connected: bool,
    coherent: bool,
    counts: SourceCounts,
    // Populated only by a complete, accepted snapshot for this generation.
    pane_ids: Option<Arc<[String]>>,
}

#[derive(Debug)]
pub struct AppState {
    visible: bool,
    passthrough: bool,
    alpha_passthrough: bool,
    bubble_visible: bool,
    bubble_placement: BubblePlacement,
    scale: f64,
    reset_position_revision: u64,
    automation: Option<SharedAutomation>,
    shutdown: bool,
    lifecycle_settings: LifecycleSettings,
    ui_ready: bool,
    sources: HashMap<String, SourceState>,
    observation: ObservationPreferences,
    observation_catalog: SourceCatalog,
    observation_epoch: u64,
    retired_remote_generations: HashMap<String, u64>,
    session_store: SessionStore,
    completions: VecDeque<CompletionObservation>,
    outcomes: VecDeque<OutcomeObservation>,
    phase: Phase,
    sessions: usize,
    working: usize,
    blocked: usize,
    done: usize,
    unknown: usize,
    connected_sources: usize,
    disconnected_sources: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            visible: true,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            bubble_placement: BubblePlacement::default(),
            scale: DEFAULT_SCALE,
            reset_position_revision: 0,
            automation: None,
            shutdown: false,
            lifecycle_settings: LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            },
            ui_ready: false,
            sources: HashMap::new(),
            observation: ObservationPreferences::default(),
            observation_catalog: SourceCatalog::default(),
            observation_epoch: 0,
            retired_remote_generations: HashMap::new(),
            session_store: SessionStore::new(),
            completions: VecDeque::new(),
            outcomes: VecDeque::new(),
            phase: Phase::Unknown,
            sessions: 0,
            working: 0,
            blocked: 0,
            done: 0,
            unknown: 0,
            connected_sources: 0,
            disconnected_sources: 0,
        }
    }

    /// Return the last watcher aggregate without allocating or walking source
    /// records.  Watcher updates maintain these counters off the UI hot path.
    pub fn scene(&self) -> Scene {
        Scene {
            phase: self.phase,
            sessions: self.sessions,
            working: self.working,
            blocked: self.blocked,
            done: self.done,
            unknown: self.unknown,
            connected_sources: self.connected_sources,
            disconnected_sources: self.disconnected_sources,
            visible: self.visible,
            passthrough: self.passthrough,
            alpha_passthrough: self.alpha_passthrough,
            bubble_visible: self.bubble_visible,
            bubble_placement: self.bubble_placement,
            scale: self.scale,
            reset_position_revision: self.reset_position_revision,
            shutdown: self.shutdown,
        }
    }
    pub(crate) fn observation_preferences(&self) -> &ObservationPreferences {
        &self.observation
    }

    pub(crate) fn observation_epoch(&self) -> u64 {
        self.observation_epoch
    }

    pub(crate) fn observation_catalog(&self) -> &SourceCatalog {
        &self.observation_catalog
    }

    fn includes_source(&self, source: &str) -> bool {
        if !self.observation.includes(source) {
            return false;
        }
        remote_machine_id(source).is_none_or(|id| {
            self.observation_catalog
                .machines
                .iter()
                .any(|machine| machine.id == id && machine.enabled)
        })
    }

    fn reconcile_observation(&mut self) {
        let excluded_remotes: Vec<(String, u64)> = self
            .sources
            .iter()
            .filter(|(source, _)| {
                remote_machine_id(source).is_some() && !self.includes_source(source)
            })
            .map(|(source, state)| (source.clone(), state.generation))
            .collect();
        for (source, generation) in excluded_remotes {
            let _ = self.remove_source(&source, generation);
        }
        for source in self.sources.keys() {
            let included = self.includes_source(source);
            self.session_store.set_visibility(source, included);
        }
        self.completions.retain(|observation| {
            self.observation.includes(&observation.source)
                && remote_machine_id(&observation.source).is_none_or(|id| {
                    self.observation_catalog
                        .machines
                        .iter()
                        .any(|machine| machine.id == id && machine.enabled)
                })
        });
        self.outcomes.retain(|observation| {
            self.observation.includes(&observation.source)
                && remote_machine_id(&observation.source).is_none_or(|id| {
                    self.observation_catalog
                        .machines
                        .iter()
                        .any(|machine| machine.id == id && machine.enabled)
                })
        });
        self.recompute_aggregate();
    }

    pub(crate) fn apply_observation_preferences(
        &mut self,
        mut preferences: ObservationPreferences,
    ) {
        preferences.sanitize();
        if self.observation == preferences {
            return;
        }
        self.observation = preferences;
        self.observation_epoch = self.observation_epoch.saturating_add(1);
        self.reconcile_observation();
        self.session_store.invalidate();
    }

    pub(crate) fn set_observation_catalog(&mut self, catalog: SourceCatalog) {
        if self.observation_catalog == catalog {
            return;
        }
        self.observation_catalog = catalog;
        self.reconcile_observation();
        for machine in &self.observation_catalog.machines {
            self.session_store
                .set_label(&remote_source(&machine.id), &machine.label);
        }
    }

    /// A selected remote source is healthy after a coherent snapshot, even if
    /// it has no agents. Local source visibility does not govern daemon exit.
    pub(crate) fn has_healthy_observed_remote(&self) -> bool {
        self.observation.remote
            && self.sources.iter().any(|(source, state)| {
                remote_machine_id(source).is_some()
                    && state.connected
                    && state.coherent
                    && self.includes_source(source)
            })
    }

    pub(crate) fn session_revision(&self) -> u64 {
        self.session_store.revision()
    }

    pub(crate) fn session_snapshot(
        &self,
        options: &SessionListOptions<'_>,
        selected: Option<&SessionKey>,
    ) -> SessionSnapshot {
        self.session_store.snapshot(options, selected)
    }
    pub(crate) fn session_display_for_key(
        &self,
        locale: crate::i18n::UiLocale,
        key: &SessionKey,
    ) -> Option<CardDisplay> {
        self.session_store.display_for_key(locale, key)
    }
    pub(crate) fn session_displays_for_keys(
        &self,
        locale: crate::i18n::UiLocale,
        keys: &[&SessionKey],
    ) -> Vec<Option<CardDisplay>> {
        self.session_store.displays_for_keys(locale, keys)
    }
    pub(crate) fn session_view_for_key(&self, key: &SessionKey) -> Option<SessionView> {
        self.session_store.view_for_key(key)
    }

    /// Paginate retained rows without the GUI's snapshot cap or selection state.
    pub(crate) fn automation_session_page(
        &self,
        request: &SessionPageRequest,
    ) -> Result<Value, String> {
        let automation = self.automation.as_ref().ok_or("automation unavailable")?;
        let instance = crate::automation::lock_automation(automation)
            .instance()
            .to_owned();
        if request.instance_id != instance {
            return Err("daemon instance changed".into());
        }
        let revision = self.session_store.revision();
        let after = match request.cursor.as_ref() {
            Some(cursor) => {
                if cursor.instance_id != instance {
                    return Err("cursor daemon instance changed".into());
                }
                if cursor.revision != revision {
                    return Err("session revision changed".into());
                }
                if cursor.filter != request.filter {
                    return Err("cursor filter changed".into());
                }
                Some(&cursor.position)
            }
            None => None,
        };
        let page = self
            .session_store
            .page(request.filter, revision, after, request.limit)
            .map_err(|error| match error {
                SessionPageError::StaleRevision => "session revision changed",
                SessionPageError::InvalidLimit => "session page limit must be between 1 and 128",
            })?;
        let next_cursor = page.next_cursor.map(|position| SessionPageCursor {
            instance_id: instance.clone(),
            revision: page.revision,
            filter: request.filter,
            position,
        });
        let rows: Vec<_> = page
            .rows
            .iter()
            .map(|view| automation_session_row(&instance, view))
            .collect();
        Ok(json!({
            "instance_id": instance,
            "revision": page.revision,
            "filter": request.filter,
            "rows": rows,
            "total": page.total,
            "matched": page.matched,
            "status_summary": page.status_summary,
            "next_cursor": next_cursor,
        }))
    }

    /// Resolve an exact identity, including a cached offline row, without
    /// consulting or mutating GUI selection.
    pub(crate) fn automation_session_detail(
        &self,
        identity: &SessionIdentity,
    ) -> Result<Value, String> {
        let automation = self.automation.as_ref().ok_or("automation unavailable")?;
        let instance = crate::automation::lock_automation(automation)
            .instance()
            .to_owned();
        if identity.instance_id != instance {
            return Err("daemon instance changed".into());
        }
        let key = SessionKey {
            source_id: identity.source_id,
            generation: identity.generation,
            terminal_id: identity.terminal_id.clone(),
        };
        let view = self
            .session_store
            .view_for_key(&key)
            .ok_or("stale session key")?;
        Ok(automation_session_row(&instance, &view))
    }

    pub(crate) fn prompt_target(
        &self,
        key: &SessionKey,
    ) -> Result<PromptTarget, crate::herdr::PromptError> {
        use crate::herdr::PromptError;
        if self.shutdown {
            return Err(PromptError::Offline);
        }
        let target = self
            .session_store
            .prompt_target(key)
            .map_err(|error| match error {
                PromptTargetError::ReadOnly => PromptError::ReadOnly,
                PromptTargetError::Offline => PromptError::Offline,
                PromptTargetError::Stale => PromptError::StaleTarget,
            })?;
        if !self.includes_source(&target.source) {
            return Err(PromptError::StaleTarget);
        }
        if !std::path::Path::new(&target.source).is_absolute() {
            return Err(PromptError::StaleTarget);
        }
        if !self.sources.get(&target.source).is_some_and(|source| {
            source.generation == key.generation && source.connected && source.coherent
        }) {
            return Err(PromptError::Offline);
        }
        Ok(target)
    }

    pub(crate) fn worktree_remove_target(
        &self,
        key: &SessionKey,
    ) -> Result<WorktreeRemoveTarget, WorktreeRemoveTargetError> {
        if self.shutdown {
            return Err(WorktreeRemoveTargetError::Offline);
        }
        let target = self.session_store.worktree_remove_target(key)?;
        if !self.includes_source(&target.source) {
            return Err(WorktreeRemoveTargetError::Stale);
        }
        if !std::path::Path::new(&target.source).is_absolute() {
            return Err(WorktreeRemoveTargetError::Stale);
        }
        if !self.sources.get(&target.source).is_some_and(|source| {
            source.generation == key.generation && source.connected && source.coherent
        }) {
            return Err(WorktreeRemoveTargetError::Offline);
        }
        Ok(target)
    }
    /// The watcher supplies complete pane IDs independently of agent rows:
    /// an exited agent can leave a shell pane behind. A reconnect can prove
    /// absence only if the registered source ID/path is unchanged and its new
    /// generation has provided another coherent full snapshot.
    pub(crate) fn automation_worktree_observation(
        &self,
        target: &WorktreeRemoveTarget,
    ) -> Option<Value> {
        if self.shutdown
            || remote_machine_id(&target.source).is_some()
            || !std::path::Path::new(&target.source).is_absolute()
            || !self.includes_source(&target.source)
        {
            return None;
        }
        let source = self.sources.get(&target.source)?;
        if !source.connected || !source.coherent || source.generation < target.key.generation {
            return None;
        }
        let panes = source.pane_ids.as_ref()?;
        let session_absent = self.session_store.worktree_session_absent(
            &target.source,
            target.key.source_id,
            source.generation,
            &target.key.terminal_id,
        )?;
        Some(json!({
            "source_id": target.key.source_id,
            "generation": source.generation,
            "pane_absent": !panes.iter().any(|id| id == &target.pane_id),
            "session_absent": session_absent,
        }))
    }

    pub(crate) fn prompt_available(
        &self,
        key: &SessionKey,
    ) -> Result<(), crate::herdr::PromptError> {
        self.prompt_target(key).map(|_| ())
    }

    /// Consume observations that still belong to a connected current source.
    /// The empty path does not inspect source records or allocate.
    pub fn take_completions(&mut self) -> Vec<CompletionObservation> {
        if self.completions.is_empty() {
            return Vec::new();
        }
        let now = Instant::now();
        let mut observations = Vec::with_capacity(self.completions.len());
        while let Some(observation) = self.completions.pop_front() {
            if !self.includes_source(&observation.source) {
                continue;
            }
            let Some(source) = self.sources.get(&observation.source) else {
                continue;
            };
            if completion_matches_source(&observation, source, now) {
                observations.push(observation);
            }
        }
        observations
    }

    /// Queue one watcher observation after checking its source generation.
    /// Queue ownership remains in AppState so source disconnects can discard
    /// pending UI reactions atomically with their status transition.
    pub(crate) fn push_completion(&mut self, observation: CompletionObservation) {
        if !self.includes_source(&observation.source) {
            return;
        }
        let Some(source) = self.sources.get(&observation.source) else {
            return;
        };
        if !completion_matches_source(&observation, source, Instant::now()) {
            return;
        }
        if self.completions.len() >= MAX_COMPLETIONS {
            let _ = self.completions.pop_front();
        }
        self.completions.push_back(observation);
    }

    pub(crate) fn take_outcomes(&mut self) -> Vec<OutcomeObservation> {
        if self.outcomes.is_empty() {
            return Vec::new();
        }
        let now = Instant::now();
        let mut observations = Vec::with_capacity(self.outcomes.len());
        while let Some(observation) = self.outcomes.pop_front() {
            if !self.includes_source(&observation.source) {
                continue;
            }
            if self.sources.get(&observation.source).is_some_and(|source| {
                source.connected
                    && source.generation == observation.generation
                    && completion_is_fresh(observation.observed_at, now)
            }) {
                observations.push(observation);
            }
        }
        observations
    }

    pub(crate) fn push_outcome(&mut self, observation: OutcomeObservation) {
        if !self.includes_source(&observation.source) {
            return;
        }
        if !self.sources.get(&observation.source).is_some_and(|source| {
            source.connected
                && source.coherent
                && source.generation == observation.generation
                && completion_is_fresh(observation.observed_at, Instant::now())
        }) {
            return;
        }
        let _ = self.session_store.accept_outcome(
            &observation.source,
            observation.generation,
            &observation.terminal_id,
            &observation.pane_id,
            observation.outcome,
            observation.at_unix_ms,
        );
        if self.outcomes.len() >= MAX_COMPLETIONS {
            let _ = self.outcomes.pop_front();
        }
        self.outcomes.push_back(observation);
    }

    pub(crate) fn automation(&self) -> Option<SharedAutomation> {
        self.automation.clone()
    }

    pub(crate) fn set_automation(&mut self, automation: SharedAutomation) {
        self.automation = Some(automation);
    }

    pub(crate) fn apply_presentation(
        &mut self,
        action: &PresentationAction,
    ) -> Result<PresentationTarget, String> {
        let target = PresentationTarget::from_scene(&self.scene()).applying(action)?;
        self.visible = target.visible;
        self.passthrough = target.passthrough;
        self.alpha_passthrough = target.alpha_passthrough;
        self.bubble_visible = target.bubble_visible;
        self.bubble_placement = target.bubble_placement;
        self.scale = target.scale;
        self.reset_position_revision = target.reset_position_revision;
        Ok(target)
    }

    fn presentation_changed(&self) {
        if let Some(automation) = &self.automation {
            crate::automation::lock_automation(automation)
                .ui_change(PresentationTarget::from_scene(&self.scene()));
        }
    }

    /// Restore an interactive character without changing the bubble or other settings.
    pub(crate) fn recover_interaction(&mut self) {
        self.apply_presentation(&PresentationAction::Set {
            patch: PresentationPatch {
                visible: Some(true),
                passthrough: Some(false),
                ..PresentationPatch::default()
            },
        })
        .expect("recovery target is valid");
        self.presentation_changed();
    }

    pub fn apply_control(&mut self, action: &str) -> Result<(), String> {
        let mut patch = PresentationPatch::default();
        let action = match action {
            "show" => {
                patch.visible = Some(true);
                PresentationAction::Set { patch }
            }
            "hide" => {
                patch.visible = Some(false);
                PresentationAction::Set { patch }
            }
            "toggle" => {
                patch.visible = Some(!self.visible);
                PresentationAction::Set { patch }
            }
            "toggle_passthrough" | "passthrough" => {
                patch.passthrough = Some(!self.passthrough);
                PresentationAction::Set { patch }
            }
            "alpha_passthrough" | "toggle_alpha_passthrough" => {
                patch.alpha_passthrough = Some(!self.alpha_passthrough);
                PresentationAction::Set { patch }
            }
            "show_bubble" => {
                patch.bubble_visible = Some(true);
                PresentationAction::Set { patch }
            }
            "hide_bubble" => {
                patch.bubble_visible = Some(false);
                PresentationAction::Set { patch }
            }
            "bubble_above" => {
                patch.bubble_placement = Some(BubblePlacement::Above);
                PresentationAction::Set { patch }
            }
            "bubble_below" => {
                patch.bubble_placement = Some(BubblePlacement::Below);
                PresentationAction::Set { patch }
            }
            "bubble_left" => {
                patch.bubble_placement = Some(BubblePlacement::Left);
                PresentationAction::Set { patch }
            }
            "bubble_right" => {
                patch.bubble_placement = Some(BubblePlacement::Right);
                PresentationAction::Set { patch }
            }
            "bubble_auto" => {
                patch.bubble_placement = Some(BubblePlacement::Auto);
                PresentationAction::Set { patch }
            }
            "scale_up" => {
                patch.scale = Some(self.scale + 0.05);
                PresentationAction::Set { patch }
            }
            "scale_down" => {
                patch.scale = Some(self.scale - 0.05);
                PresentationAction::Set { patch }
            }
            "reset_position" => PresentationAction::ResetPosition,
            "status" => PresentationAction::Set { patch },
            _ => return Err(format!("unsupported control action: {action}")),
        };
        self.apply_presentation(&action)?;
        self.presentation_changed();
        Ok(())
    }

    pub fn set_scale(&mut self, scale: f64) -> bool {
        let Some(scale) = normalize_scale(scale) else {
            return false;
        };
        if self.scale == scale {
            return false;
        }
        self.apply_presentation(&PresentationAction::Set {
            patch: PresentationPatch {
                scale: Some(scale),
                ..PresentationPatch::default()
            },
        })
        .expect("normalized scale is valid");
        self.presentation_changed();
        true
    }

    pub fn set_preferences(&mut self, prefs: &crate::preferences::Preferences) {
        self.scale = normalize_scale(prefs.scale()).unwrap_or(self.scale);
        self.visible = prefs.visible();
        self.passthrough = prefs.passthrough();
        self.alpha_passthrough = prefs.alpha_passthrough();
        self.bubble_visible = prefs.bubble_visible();
        self.bubble_placement = prefs.bubble_placement();
    }

    pub fn set_ui_ready(&mut self) {
        self.ui_ready = true;
    }

    pub fn is_ui_ready(&self) -> bool {
        self.ui_ready
    }

    pub(crate) fn lifecycle_settings(&self) -> LifecycleSettings {
        self.lifecycle_settings
    }

    pub(crate) fn set_lifecycle_settings(&mut self, settings: LifecycleSettings) {
        self.lifecycle_settings = settings;
    }

    /// Request an orderly daemon shutdown without changing persisted settings.
    pub fn request_shutdown(&mut self) {
        self.shutdown = true;
    }

    /// Begin a new connection generation for a canonical endpoint.  Any
    /// previous live counts become unknown until the new generation provides a
    /// coherent snapshot.
    pub(crate) fn begin_source(&mut self, source: String, generation: u64) -> bool {
        if remote_machine_id(&source).is_some() && !self.includes_source(&source)
            || remote_machine_id(&source).is_some_and(|_| {
                self.retired_remote_generations
                    .get(&source)
                    .is_some_and(|retired| generation <= *retired)
            })
        {
            return false;
        }
        if generation == 0 {
            return false;
        }
        if let Some(existing) = self.sources.get(&source) {
            if generation <= existing.generation {
                return false;
            }
        }
        if !self.session_store.begin_source(&source, generation) {
            return false;
        }
        self.session_store
            .set_visibility(&source, self.includes_source(&source));
        if let Some(id) = remote_machine_id(&source) {
            if let Some(machine) = self
                .observation_catalog
                .machines
                .iter()
                .find(|machine| machine.id == id)
            {
                self.session_store.set_label(&source, &machine.label);
            }
        }
        purge_completions_for_source(&mut self.completions, &source);
        self.outcomes
            .retain(|observation| observation.source != source);
        let counts = self
            .sources
            .get(&source)
            .map(|existing| existing.counts.disconnected())
            .unwrap_or_default();
        self.sources.insert(
            source,
            SourceState {
                generation,
                connected: false,
                coherent: false,
                counts,
                pane_ids: None,
            },
        );
        self.recompute_aggregate();
        true
    }

    /// Publish one endpoint's complete, already-normalized aggregate.  A
    /// callback from an older connection generation is ignored.  A connected
    /// source's records are published only through `publish_source_snapshot`,
    /// after the watcher has a coherent snapshot.
    pub(crate) fn update_source(
        &mut self,
        source: &str,
        generation: u64,
        connected: bool,
        counts: SourceCounts,
    ) -> bool {
        if remote_machine_id(source).is_some() && !self.includes_source(source) {
            return false;
        }
        {
            let Some(existing) = self.sources.get_mut(source) else {
                return false;
            };
            if existing.generation != generation {
                return false;
            }
            existing.connected = connected;
            existing.coherent = if connected { existing.coherent } else { false };
            if !connected {
                existing.pane_ids = None;
            }
            existing.counts = if connected {
                counts.bounded()
            } else {
                counts.disconnected()
            };
        }
        if !connected {
            let _ = self.session_store.mark_offline(source, generation);
            purge_completions_for_source(&mut self.completions, source);
            self.outcomes
                .retain(|observation| observation.source != source);
        }
        self.recompute_aggregate();
        true
    }

    /// Revoke only the current source generation's matching card result;
    /// existing outcome reactions remain owned by the watcher queue.
    pub(crate) fn invalidate_session_outcome(
        &mut self,
        source: &str,
        generation: u64,
        terminal_id: &str,
        pane_id: &str,
    ) {
        self.session_store
            .invalidate_outcome(source, generation, terminal_id, pane_id);
    }

    /// Publish a coherent snapshot, its complete pane list and any completions
    /// observed while converging it. All facts change in one AppState mutation.
    pub(crate) fn publish_source_snapshot<'a, I>(
        &mut self,
        source: &str,
        generation: u64,
        records: I,
        counts: SourceCounts,
        pane_ids: Arc<[String]>,
        completions: &[(String, String)],
        observed_at: Instant,
    ) -> bool
    where
        I: IntoIterator<Item = &'a AgentRecord>,
    {
        if remote_machine_id(source).is_some() && !self.includes_source(source) {
            return false;
        }
        let Some(existing) = self.sources.get(source) else {
            return false;
        };
        if existing.generation != generation {
            return false;
        }
        if !self
            .session_store
            .replace_source(source, generation, records)
        {
            return false;
        }
        if let Some(existing) = self.sources.get_mut(source) {
            existing.connected = true;
            existing.coherent = true;
            if existing
                .pane_ids
                .as_ref()
                .is_none_or(|before| !Arc::ptr_eq(before, &pane_ids))
            {
                existing.pane_ids = Some(pane_ids);
            }
            existing.counts = counts.bounded();
        }
        self.recompute_aggregate();
        for (terminal_id, pane_id) in completions {
            self.push_completion(CompletionObservation {
                source: source.to_owned(),
                generation,
                terminal_id: terminal_id.clone(),
                pane_id: pane_id.clone(),
                observed_at,
            });
        }
        true
    }

    /// Apply one unique live status transition without rebuilding the source
    /// record set.  The watcher supplies aggregate counts maintained from its
    /// authoritative records and a completion marker for active-to-Done.
    pub(crate) fn update_session_status(&mut self, update: SessionStatusUpdate<'_>) -> bool {
        let SessionStatusUpdate {
            source,
            generation,
            terminal_id,
            pane_id,
            status,
            counts,
            completion,
            observed_at,
        } = update;
        if remote_machine_id(source).is_some() && !self.includes_source(source) {
            return false;
        }
        let Some(existing) = self.sources.get(source) else {
            return false;
        };
        if existing.generation != generation || !existing.connected || !existing.coherent {
            return false;
        }
        if !self
            .session_store
            .update_status(source, generation, terminal_id, pane_id, status)
        {
            return false;
        }
        if let Some(existing) = self.sources.get_mut(source) {
            existing.counts = counts.bounded();
        }
        self.recompute_aggregate();
        if completion {
            self.push_completion(CompletionObservation {
                source: source.to_owned(),
                generation,
                terminal_id: terminal_id.to_owned(),
                pane_id: pane_id.to_owned(),
                observed_at,
            });
        }
        true
    }

    /// Remove a confirmed disabled/unlinked endpoint.  Stale callbacks from a
    /// prior generation cannot detach a newly registered endpoint.
    pub(crate) fn remove_source(&mut self, source: &str, generation: u64) -> bool {
        let Some(existing) = self.sources.get(source) else {
            return false;
        };
        if existing.generation != generation {
            return false;
        }
        if !self.session_store.remove_source(source, generation) {
            return false;
        }
        purge_completions_for_source(&mut self.completions, source);
        self.outcomes
            .retain(|observation| observation.source != source);
        if remote_machine_id(source).is_some() {
            self.retired_remote_generations
                .entry(source.to_owned())
                .and_modify(|retired| *retired = (*retired).max(generation))
                .or_insert(generation);
        }
        self.sources.remove(source);
        self.recompute_aggregate();
        true
    }

    fn recompute_aggregate(&mut self) {
        let mut aggregate = SourceCounts::default();
        let mut connected_sources = 0usize;
        let mut disconnected_sources = 0usize;
        for (source_id, source) in &self.sources {
            if !self.includes_source(source_id) {
                continue;
            }
            if source.connected {
                connected_sources = connected_sources.saturating_add(1);
            } else {
                disconnected_sources = disconnected_sources.saturating_add(1);
            }
            aggregate.sessions = aggregate
                .sessions
                .saturating_add(source.counts.sessions)
                .min(MAX_COUNT as usize);
            aggregate.working = aggregate
                .working
                .saturating_add(source.counts.working)
                .min(MAX_COUNT as usize);
            aggregate.blocked = aggregate
                .blocked
                .saturating_add(source.counts.blocked)
                .min(MAX_COUNT as usize);
            aggregate.done = aggregate
                .done
                .saturating_add(source.counts.done)
                .min(MAX_COUNT as usize);
            aggregate.unknown = aggregate
                .unknown
                .saturating_add(source.counts.unknown)
                .min(MAX_COUNT as usize);
        }
        self.sessions = aggregate.sessions;
        self.working = aggregate.working;
        self.blocked = aggregate.blocked;
        self.done = aggregate.done;
        self.unknown = aggregate.unknown;
        self.connected_sources = connected_sources;
        self.disconnected_sources = disconnected_sources;
        self.phase = if connected_sources == 0 {
            Phase::Unknown
        } else if aggregate.blocked > 0 {
            Phase::Waiting
        } else if aggregate.working > 0 {
            Phase::Running
        } else if aggregate.unknown > 0 {
            Phase::Unknown
        } else {
            Phase::Idle
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_view::SessionFilter;
    use std::time::{Duration, Instant};

    fn connect(state: &mut AppState, source: &str, generation: u64) {
        assert!(state.begin_source(source.to_owned(), generation));
    }

    fn observation(source: &str, generation: u64, observed_at: Instant) -> CompletionObservation {
        CompletionObservation {
            source: source.to_owned(),
            generation,
            terminal_id: "terminal-a".to_owned(),
            pane_id: "pane-a".to_owned(),
            observed_at,
        }
    }
    fn record(terminal_id: &str, pane_id: &str, status: AgentStatus) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal_id.to_owned(),
            pane_id: pane_id.to_owned(),
            status,
            metadata: crate::herdr_protocol::SessionMetadata::default(),
            outcome_authoritative: false,
            outcome: None,
        }
    }

    fn policy(local: bool, remote: bool, ids: &[&str]) -> ObservationPreferences {
        ObservationPreferences {
            local,
            remote,
            machines: ids.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    fn catalog(profiles: &[(&str, &str, bool)]) -> SourceCatalog {
        SourceCatalog {
            initialized: true,
            machines: profiles
                .iter()
                .map(|(id, label, enabled)| crate::sources::MachineInfo {
                    id: (*id).to_owned(),
                    label: (*label).to_owned(),
                    remote_session: "session".to_owned(),
                    enabled: *enabled,
                    status: crate::sources::MachineStatus::Online,
                    error: None,
                })
                .collect(),
            error: None,
        }
    }

    fn publish(state: &mut AppState, source: &str, generation: u64, status: AgentStatus) {
        let records = [record("same-terminal", "same-pane", status)];
        let mut counts = SourceCounts {
            sessions: 1,
            ..SourceCounts::default()
        };
        match status {
            AgentStatus::Working => counts.working = 1,
            AgentStatus::Blocked => counts.blocked = 1,
            AgentStatus::Done => counts.done = 1,
            _ => counts.unknown = 1,
        }
        assert!(state.publish_source_snapshot(
            source,
            generation,
            records.iter(),
            counts,
            Arc::from(vec!["same-pane".to_owned()]),
            &[],
            Instant::now()
        ));
    }

    fn session_request(filter: SessionFilter, limit: usize) -> SessionPageRequest {
        SessionPageRequest {
            instance_id: "test".into(),
            filter,
            cursor: None,
            limit,
        }
    }

    #[test]
    fn session_wire_rejects_unknown_keys_and_invalid_filter_cursor_shapes() {
        assert!(serde_json::from_str::<SessionPageRequest>(
            r#"{"instance_id":"test","filter":"all","cursor":null,"limit":32,"unexpected":true}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SessionPageRequest>(
            r#"{"instance_id":"test","filter":"invented","cursor":null,"limit":32}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SessionPageCursor>(
            r#"{"instance_id":"test","revision":1,"filter":"all","position":{"source_id":0,"terminal_id":"t","extra":1}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SessionKey>(
            r#"{"source_id":0,"generation":1,"terminal_id":"t","extra":1}"#
        )
        .is_err());
    }

    #[test]
    fn automation_pages_traverse_beyond_gui_cap_and_bind_cursor_to_instance_filter_revision() {
        let mut state = AppState::new();
        let automation =
            crate::automation::test_automation(PresentationTarget::from_scene(&state.scene()));
        state.set_automation(automation);
        connect(&mut state, "local.sock", 1);
        let records: Vec<_> = (0..140)
            .map(|index| {
                record(
                    &format!("terminal-{index:03}"),
                    &format!("pane-{index:03}"),
                    AgentStatus::Working,
                )
            })
            .collect();
        assert!(state.publish_source_snapshot(
            "local.sock",
            1,
            &records,
            SourceCounts {
                sessions: 140,
                working: 140,
                ..SourceCounts::default()
            },
            Arc::from(
                records
                    .iter()
                    .map(|record| record.pane_id.clone())
                    .collect::<Vec<_>>()
            ),
            &[],
            Instant::now(),
        ));
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                .rows
                .len(),
            128
        );
        let mut request = session_request(SessionFilter::All, 32);
        let mut ids = Vec::new();
        let first = state.automation_session_page(&request).unwrap();
        assert_eq!(first["matched"], 140);
        let cursor: SessionPageCursor =
            serde_json::from_value(first["next_cursor"].clone()).unwrap();
        let mut altered = request.clone();
        altered.cursor = Some(cursor.clone());
        altered.instance_id = "old-daemon".into();
        assert!(state.automation_session_page(&altered).is_err());
        altered.instance_id = "test".into();
        altered.filter = SessionFilter::Waiting;
        assert!(state.automation_session_page(&altered).is_err());
        altered.filter = SessionFilter::All;
        altered.cursor.as_mut().unwrap().instance_id = "old-daemon".into();
        assert!(state.automation_session_page(&altered).is_err());
        loop {
            let page = state.automation_session_page(&request).unwrap();
            ids.extend(page["rows"].as_array().unwrap().iter().map(|row| {
                assert_eq!(row["key"]["instance_id"], "test");
                row["key"]["terminal_id"].as_str().unwrap().to_owned()
            }));
            if page["next_cursor"].is_null() {
                break;
            }
            request.cursor = Some(serde_json::from_value(page["next_cursor"].clone()).unwrap());
        }
        assert_eq!(ids.len(), 140);
        assert_eq!(ids.first().unwrap(), "terminal-000");
        assert_eq!(ids.last().unwrap(), "terminal-139");
        let mut invalid = session_request(SessionFilter::All, 0);
        assert!(state.automation_session_page(&invalid).is_err());
        invalid.limit = 129;
        assert!(state.automation_session_page(&invalid).is_err());
        assert!(state.update_source(
            "local.sock",
            1,
            false,
            SourceCounts {
                sessions: 140,
                working: 140,
                ..SourceCounts::default()
            },
        ));
        assert!(state.automation_session_page(&request).is_err());
    }

    #[test]
    fn automation_detail_retains_offline_metadata_and_rejects_hidden_or_reconnected_key() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        let mut state = AppState::new();
        let automation =
            crate::automation::test_automation(PresentationTarget::from_scene(&state.scene()));
        state.set_automation(automation);
        connect(&mut state, "local.sock", 1);
        let mut row = record("same-terminal", "same-pane", AgentStatus::Done);
        row.metadata.title = Some("Real title".into());
        row.metadata.cwd = Some("/private/secret".into());
        row.outcome_authoritative = true;
        row.outcome = Some(OutcomeReport {
            session: "current".into(),
            turn: "1".into(),
            outcome: AgentOutcome::Succeeded,
            at_unix_ms: 1,
        });
        assert!(state.publish_source_snapshot(
            "local.sock",
            1,
            [&row],
            SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
            Arc::from(vec!["same-pane".to_owned()]),
            &[],
            Instant::now(),
        ));
        let key = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        let identity = SessionIdentity {
            instance_id: "test".into(),
            source_id: key.source_id,
            generation: key.generation,
            terminal_id: key.terminal_id.clone(),
        };
        let detail = state.automation_session_detail(&identity).unwrap();
        assert_eq!(detail["status"], "done");
        assert_eq!(detail["metadata"]["title"], "Real title");
        assert!(detail["metadata"].get("cwd").is_none());
        assert_eq!(detail["outcome"], Value::Null);
        assert_eq!(
            state
                .session_snapshot(
                    &SessionListOptions::with_filter(SessionFilter::All),
                    Some(&key)
                )
                .selected,
            Some(key.clone())
        );
        state.push_outcome(outcome("local.sock", 1));
        let completed = state.automation_session_detail(&identity).unwrap();
        assert_eq!(completed["outcome"], "succeeded");
        assert_eq!(completed["display_status"], "succeeded");
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                .rows[0]
                .outcome,
            Some(AgentOutcome::Succeeded)
        );
        assert_eq!(
            state
                .session_snapshot(
                    &SessionListOptions::with_filter(SessionFilter::All),
                    Some(&key)
                )
                .selected,
            Some(key.clone())
        );
        assert!(state.update_source(
            "local.sock",
            1,
            false,
            SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
        ));
        let offline = state.automation_session_detail(&identity).unwrap();
        assert_eq!(offline["availability"], "offline");
        assert_eq!(offline["status"], "done");
        assert_eq!(offline["metadata"]["title"], "Real title");
        assert!(offline["metadata"].get("cwd").is_none());
        assert_eq!(offline["outcome"], Value::Null);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                .rows[0]
                .outcome,
            None
        );
        assert_eq!(offline["display_status"], "offline");
        state.apply_observation_preferences(policy(false, false, &[]));
        assert!(state.automation_session_detail(&identity).is_err());
        state.apply_observation_preferences(policy(true, false, &[]));
        assert!(state.automation_session_detail(&identity).is_ok());
        connect(&mut state, "local.sock", 2);
        assert!(state.automation_session_detail(&identity).is_err());
        let mut wrong = identity;
        wrong.instance_id = "old-daemon".into();
        assert!(state.automation_session_detail(&wrong).is_err());
    }

    fn outcome(source: &str, generation: u64) -> OutcomeObservation {
        OutcomeObservation {
            source: source.to_owned(),
            generation,
            terminal_id: "same-terminal".to_owned(),
            pane_id: "same-pane".to_owned(),
            outcome: crate::agent_outcome::AgentOutcome::Succeeded,
            at_unix_ms: 1,
            observed_at: Instant::now(),
        }
    }

    #[test]
    fn display_bridge_observes_cached_offline_row_independently_of_prompt_readiness() {
        let mut state = AppState::new();
        connect(&mut state, "local.sock", 1);
        publish(&mut state, "local.sock", 1, AgentStatus::Working);
        let key = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert_eq!(
            state.session_view_for_key(&key),
            Some(
                state
                    .session_snapshot(&SessionListOptions::default(), None)
                    .rows[0]
                    .clone()
            )
        );
        assert!(state.update_source(
            "local.sock",
            1,
            false,
            SourceCounts {
                sessions: 1,
                working: 1,
                ..SourceCounts::default()
            }
        ));
        assert_eq!(
            state.session_view_for_key(&key).unwrap().display_status(),
            crate::session_view::DisplayStatus::Offline
        );
        assert!(state.prompt_available(&key).is_err());
        assert!(state.begin_source("local.sock".to_owned(), 2));
        assert_eq!(state.session_view_for_key(&key), None);
    }

    #[test]
    fn worktree_target_requires_current_included_coherent_absolute_local_source() {
        let mut state = AppState::new();
        let source = "/private/local.sock";
        let mut row = record("t", "p", AgentStatus::Working);
        row.metadata.workspace_id = Some("w".into());
        row.metadata.worktree = Some(std::sync::Arc::new(
            crate::herdr_protocol::WorkspaceWorktreeInfo {
                repo_key: "repo".into(),
                repo_name: "Project".into(),
                repo_root: "/repo".into(),
                checkout_path: "/checkout".into(),
                is_linked_worktree: true,
                pane_count: 1,
                tab_count: 1,
            },
        ));
        connect(&mut state, source, 1);
        let counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            source,
            1,
            [&row],
            counts,
            Arc::from(vec!["p".to_owned()]),
            &[],
            Instant::now()
        ));
        let key = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert_eq!(
            state.worktree_remove_target(&key).unwrap().workspace_id,
            "w"
        );
        state.apply_observation_preferences(policy(false, false, &[]));
        assert_eq!(
            state.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Stale)
        );
        state.apply_observation_preferences(policy(true, false, &[]));
        assert!(state.update_source(source, 1, false, counts));
        assert_eq!(
            state.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Offline)
        );
        connect(&mut state, source, 2);
        assert_eq!(
            state.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Stale)
        );
        assert!(state.publish_source_snapshot(
            source,
            2,
            [&row],
            counts,
            Arc::from(vec!["p".to_owned()]),
            &[],
            Instant::now()
        ));
        let new_key = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        state.request_shutdown();
        assert_eq!(
            state.worktree_remove_target(&new_key),
            Err(WorktreeRemoveTargetError::Offline)
        );

        let mut relative = AppState::new();
        connect(&mut relative, "local.sock", 1);
        assert!(relative.publish_source_snapshot(
            "local.sock",
            1,
            [&row],
            counts,
            Arc::from(vec!["p".to_owned()]),
            &[],
            Instant::now()
        ));
        let relative_key = relative
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert_eq!(
            relative.worktree_remove_target(&relative_key),
            Err(WorktreeRemoveTargetError::Stale)
        );
    }

    #[test]
    fn worktree_observation_requires_live_full_panes_and_same_registered_source() {
        let source = "/private/worktree.sock";
        let mut state = AppState::new();
        let mut row = record("target-terminal", "target-pane", AgentStatus::Working);
        row.metadata.workspace_id = Some("workspace".into());
        row.metadata.worktree = Some(Arc::new(crate::herdr_protocol::WorkspaceWorktreeInfo {
            repo_key: "repo".into(),
            repo_name: "Repo".into(),
            repo_root: "/repo".into(),
            checkout_path: "/repo/linked".into(),
            is_linked_worktree: true,
            pane_count: 1,
            tab_count: 1,
        }));
        connect(&mut state, source, 1);
        let live = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            source,
            1,
            [&row],
            live,
            Arc::from(vec!["target-pane".to_owned()]),
            &[],
            Instant::now(),
        ));
        let key = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        let target = state.worktree_remove_target(&key).unwrap();
        assert_eq!(
            state.automation_worktree_observation(&target),
            Some(json!({"source_id":key.source_id,"generation":1,
                        "pane_absent":false,"session_absent":false}))
        );
        let empty: [AgentRecord; 0] = [];
        assert!(state.publish_source_snapshot(
            source,
            1,
            empty.iter(),
            SourceCounts::default(),
            Arc::from(vec!["target-pane".to_owned()]),
            &[],
            Instant::now(),
        ));
        // The agent exited, but the containing shell pane remains.
        assert_eq!(
            state.automation_worktree_observation(&target),
            Some(json!({"source_id":key.source_id,"generation":1,
                        "pane_absent":false,"session_absent":true}))
        );
        assert!(state.publish_source_snapshot(
            source,
            1,
            [&row],
            live,
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        assert_eq!(
            state.automation_worktree_observation(&target),
            Some(json!({"source_id":key.source_id,"generation":1,
                        "pane_absent":true,"session_absent":false}))
        );
        assert!(state.update_source(source, 1, false, live));
        assert_eq!(state.automation_worktree_observation(&target), None);
        connect(&mut state, source, 2);
        assert_eq!(state.automation_worktree_observation(&target), None);
        assert!(!state.publish_source_snapshot(
            source,
            1,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        assert_eq!(state.automation_worktree_observation(&target), None);
        assert!(state.publish_source_snapshot(
            source,
            2,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        assert_eq!(
            state.automation_worktree_observation(&target),
            Some(json!({"source_id":key.source_id,"generation":2,
                        "pane_absent":true,"session_absent":true}))
        );
        assert!(state.remove_source(source, 2));
        connect(&mut state, source, 3);
        assert!(state.publish_source_snapshot(
            source,
            3,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        assert_eq!(state.automation_worktree_observation(&target), None);
    }

    #[test]
    fn remote_health_requires_selected_coherent_source_even_with_no_sessions() {
        let mut state = AppState::new();
        let source = remote_source("east");
        state.set_observation_catalog(catalog(&[("east", "East", true)]));
        state.apply_observation_preferences(policy(false, true, &["east"]));
        connect(&mut state, &source, 1);
        assert!(!state.has_healthy_observed_remote());
        let empty: [AgentRecord; 0] = [];
        assert!(state.publish_source_snapshot(
            &source,
            1,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        assert!(state.has_healthy_observed_remote());
        assert_eq!(state.scene().phase, Phase::Idle);
        state.apply_observation_preferences(policy(false, false, &["east"]));
        assert!(!state.has_healthy_observed_remote());
        state.apply_observation_preferences(policy(false, true, &["east"]));
        assert!(!state.has_healthy_observed_remote());
        connect(&mut state, &source, 2);
        assert!(!state.has_healthy_observed_remote());
        assert!(state.publish_source_snapshot(
            &source,
            2,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        state.set_observation_catalog(catalog(&[("east", "East", false)]));
        assert!(!state.has_healthy_observed_remote());
    }

    #[test]
    fn accepted_outcome_mirrors_only_eligible_current_source_without_consuming_reaction() {
        use crate::agent_outcome::{AgentOutcome, OutcomeReport};
        use crate::session_view::DisplayStatus;
        let mut state = AppState::new();
        connect(&mut state, "local.sock", 1);
        let mut row = record("same-terminal", "same-pane", AgentStatus::Working);
        row.outcome_authoritative = true;
        row.outcome = Some(OutcomeReport {
            session: "current".into(),
            turn: "1".into(),
            outcome: AgentOutcome::Succeeded,
            at_unix_ms: 1,
        });
        let counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            "local.sock",
            1,
            [&row],
            counts,
            Arc::from(vec!["same-pane".to_owned()]),
            &[],
            Instant::now()
        ));
        let revision = state.session_revision();
        state.push_outcome(outcome("local.sock", 1));
        assert!(state.session_revision() > revision);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .rows[0]
                .display_status(),
            DisplayStatus::Succeeded
        );
        assert_eq!(state.take_outcomes().len(), 1);
        let revision = state.session_revision();
        let mut stale = outcome("local.sock", 1);
        stale.pane_id = "wrong".into();
        state.push_outcome(stale);
        assert_eq!(state.session_revision(), revision);
        let active = SessionStatusUpdate {
            source: "local.sock",
            generation: 1,
            terminal_id: "same-terminal",
            pane_id: "same-pane",
            status: AgentStatus::Working,
            counts,
            completion: false,
            observed_at: Instant::now(),
        };
        assert!(state.update_session_status(active));
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .rows[0]
                .display_status(),
            DisplayStatus::Running
        );
        state.update_source("local.sock", 1, false, counts);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .rows[0]
                .display_status(),
            DisplayStatus::Offline
        );
        state.push_outcome(outcome("local.sock", 1));
        assert!(state.take_outcomes().is_empty());
    }

    #[test]
    fn policy_filters_phase_rows_selection_and_pending_effects_without_discarding_local() {
        let mut state = AppState::new();
        let remote = remote_source("east");
        state.set_observation_catalog(catalog(&[("east", "East", true)]));
        assert!(!state.begin_source(remote.clone(), 1));
        connect(&mut state, "local.sock", 1);
        publish(&mut state, "local.sock", 1, AgentStatus::Working);
        state.apply_observation_preferences(policy(true, true, &["east"]));
        connect(&mut state, &remote, 1);
        publish(&mut state, &remote, 1, AgentStatus::Blocked);
        let both =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(both.total, 2);
        assert_eq!(state.scene().phase, Phase::Waiting);
        assert_ne!(both.rows[0].key, both.rows[1].key);
        let local_key = both.rows[0].key.clone();
        state.push_completion(observation(&remote, 1, Instant::now()));
        state.push_outcome(outcome(&remote, 1));
        let revision = state.session_revision();
        state.apply_observation_preferences(policy(true, false, &["east"]));
        assert!(state.session_revision() > revision);
        assert_eq!(state.scene().phase, Phase::Running);
        let local = state.session_snapshot(
            &SessionListOptions::with_filter(SessionFilter::Waiting),
            Some(&local_key),
        );
        assert_eq!((local.total, local.matched, local.omitted), (1, 0, 0));
        assert_eq!(local.selected, Some(local_key.clone()));
        assert!(state.take_completions().is_empty());
        state.push_completion(observation(&remote, 1, Instant::now()));
        state.push_outcome(outcome(&remote, 1));
        assert!(state.take_completions().is_empty());
        assert!(state.take_outcomes().is_empty());
        state.apply_observation_preferences(policy(false, false, &["east"]));
        assert_eq!(state.scene().phase, Phase::Unknown);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), Some(&local_key))
                .selected,
            None
        );
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .total,
            0
        );
        publish(&mut state, "local.sock", 1, AgentStatus::Done);
        state.apply_observation_preferences(policy(true, false, &["east"]));
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .rows[0]
                .status,
            AgentStatus::Done
        );
    }

    #[test]
    fn catalog_disable_delete_rename_and_reinclude_reject_old_generations() {
        let mut state = AppState::new();
        let east = remote_source("east");
        let west = remote_source("west");
        state.set_observation_catalog(catalog(&[("east", "East", true), ("west", "West", true)]));
        state.apply_observation_preferences(policy(false, true, &["east", "west"]));
        connect(&mut state, &east, 1);
        connect(&mut state, &west, 1);
        publish(&mut state, &east, 1, AgentStatus::Working);
        publish(&mut state, &west, 1, AgentStatus::Blocked);
        let rows = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert_eq!(
            (rows[0].source_label.as_str(), rows[1].source_label.as_str()),
            ("East", "West")
        );
        assert_ne!(rows[0].key, rows[1].key);
        let previous = state.session_revision();
        state.set_observation_catalog(catalog(&[
            ("east", "Renamed", true),
            ("west", "West", true),
        ]));
        assert!(state.session_revision() > previous);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .rows[0]
                .source_label,
            "Renamed"
        );
        state.set_observation_catalog(catalog(&[("east", "Renamed", false)]));
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .total,
            0
        );
        assert_eq!(state.scene().phase, Phase::Unknown);
        assert!(!state.begin_source(east.clone(), 1));
        let empty: [AgentRecord; 0] = [];
        assert!(!state.publish_source_snapshot(
            &west,
            1,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now()
        ));
        state.set_observation_catalog(catalog(&[("east", "Renamed", true)]));
        assert!(!state.begin_source(east.clone(), 1));
        state.push_completion(observation(&east, 1, Instant::now()));
        state.push_outcome(outcome(&east, 1));
        assert!(state.take_completions().is_empty());
        assert!(state.take_outcomes().is_empty());
        connect(&mut state, &east, 2);
        publish(&mut state, &east, 2, AgentStatus::Done);
        assert!(!state.update_source(&east, 1, true, SourceCounts::default()));
        assert!(!state.update_session_status(SessionStatusUpdate {
            source: &east,
            generation: 1,
            terminal_id: "same-terminal",
            pane_id: "same-pane",
            status: AgentStatus::Blocked,
            counts: SourceCounts::default(),
            completion: true,
            observed_at: Instant::now(),
        }));
        let revision = state.session_revision();
        publish(&mut state, &east, 2, AgentStatus::Done);
        assert_eq!(state.session_revision(), revision);
        assert_eq!(
            state
                .session_snapshot(&SessionListOptions::default(), None)
                .total,
            1
        );
        assert!(state.take_completions().is_empty());
    }

    #[test]
    fn session_rows_remain_offline_until_coherent_replacement_or_empty_snapshot() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        let initial_records = [record("terminal-a", "pane-a", AgentStatus::Working)];
        let initial_counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            "one.sock",
            1,
            initial_records.iter(),
            initial_counts,
            Arc::from(vec!["pane-a".to_owned()]),
            &[],
            Instant::now(),
        ));
        let live =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(live.rows.len(), 1);
        assert_eq!(
            live.rows[0].availability,
            crate::session_view::Availability::Live
        );
        assert_eq!(live.rows[0].pane_id, "pane-a");

        assert!(state.update_source("one.sock", 1, false, initial_counts));
        let offline =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(offline.rows.len(), 1);
        assert_eq!(
            offline.rows[0].availability,
            crate::session_view::Availability::Offline
        );
        assert_eq!(offline.rows[0].status, AgentStatus::Working);

        assert!(state.begin_source("one.sock".to_owned(), 2));
        let restarted =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(restarted.rows.len(), 1);
        assert_eq!(restarted.rows[0].key.generation, 2);
        assert_eq!(
            restarted.rows[0].availability,
            crate::session_view::Availability::Offline
        );
        let in_progress = [record("terminal-a", "pane-a", AgentStatus::Done)];
        assert!(!state.update_session_status(SessionStatusUpdate {
            source: "one.sock",
            generation: 2,
            terminal_id: "terminal-a",
            pane_id: "pane-a",
            status: AgentStatus::Done,
            counts: SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
            completion: true,
            observed_at: Instant::now(),
        }));
        let still_offline =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(still_offline.rows[0].status, AgentStatus::Working);
        assert_eq!(
            still_offline.rows[0].availability,
            crate::session_view::Availability::Offline
        );

        assert!(state.publish_source_snapshot(
            "one.sock",
            2,
            in_progress.iter(),
            SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
            Arc::from(vec!["pane-a".to_owned()]),
            &[],
            Instant::now(),
        ));
        let replaced =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(replaced.rows[0].status, AgentStatus::Done);
        assert_eq!(
            replaced.rows[0].availability,
            crate::session_view::Availability::Live
        );

        let before_rejected =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        let before_revision = state.session_revision();
        let before_scene = state.scene();
        assert!(!state.update_session_status(SessionStatusUpdate {
            source: "one.sock",
            generation: 2,
            terminal_id: "terminal-a",
            pane_id: "wrong-pane",
            status: AgentStatus::Working,
            counts: SourceCounts {
                sessions: 1,
                working: 1,
                ..SourceCounts::default()
            },
            completion: true,
            observed_at: Instant::now(),
        }));
        assert_eq!(state.session_revision(), before_revision);
        assert_eq!(
            state.session_snapshot(&SessionListOptions::default(), None),
            before_rejected
        );
        let after_scene = state.scene();
        assert_eq!(after_scene.sessions, before_scene.sessions);
        assert_eq!(after_scene.working, before_scene.working);
        assert_eq!(after_scene.done, before_scene.done);
        assert!(state.take_completions().is_empty());

        assert!(state.begin_source("one.sock".to_owned(), 3));
        let empty: [AgentRecord; 0] = [];
        assert!(state.publish_source_snapshot(
            "one.sock",
            3,
            empty.iter(),
            SourceCounts::default(),
            Arc::from([]),
            &[],
            Instant::now(),
        ));
        let cleared =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert!(cleared.rows.is_empty());
        assert_eq!(cleared.total, 0);
        assert_eq!(cleared.matched, 0);
    }
    #[test]
    fn completions_are_consumed_once_and_purged_on_reconnect_or_disconnect() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        assert!(state.update_source("one.sock", 1, true, SourceCounts::default()));
        state.push_completion(observation("one.sock", 1, Instant::now()));
        assert_eq!(state.take_completions().len(), 1);
        state.push_completion(observation("one.sock", 1, Instant::now()));
        assert!(state.begin_source("one.sock".to_owned(), 2));
        assert!(state.take_completions().is_empty());
        assert!(state.update_source("one.sock", 2, true, SourceCounts::default()));
        state.push_completion(observation("one.sock", 2, Instant::now()));
        assert!(state.update_source("one.sock", 2, false, SourceCounts::default()));
        assert!(state.take_completions().is_empty());
    }

    #[test]
    fn stale_completion_observations_are_discarded() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        assert!(state.update_source("one.sock", 1, true, SourceCounts::default()));
        state.push_completion(observation(
            "one.sock",
            1,
            Instant::now() - Duration::from_secs(31),
        ));
        assert!(state.take_completions().is_empty());
    }

    #[test]
    fn initial_scene_has_no_ready_connection() {
        let scene = AppState::new().scene();
        assert_eq!(scene.phase, Phase::Unknown);
        assert_eq!(scene.sessions, 0);
        assert_eq!(scene.connected_sources, 0);
        assert_eq!(scene.disconnected_sources, 0);
    }

    #[test]
    fn connected_zero_agents_is_idle_but_disconnect_is_unknown() {
        let mut state = AppState::new();
        connect(&mut state, "herdr.sock", 1);
        assert!(state.update_source("herdr.sock", 1, true, SourceCounts::default()));
        assert_eq!(state.scene().phase, Phase::Idle);
        assert!(state.update_source(
            "herdr.sock",
            1,
            true,
            SourceCounts {
                sessions: 1,
                working: 1,
                ..SourceCounts::default()
            },
        ));
        assert_eq!(state.scene().phase, Phase::Running);
        assert!(state.update_source(
            "herdr.sock",
            1,
            false,
            SourceCounts {
                sessions: 1,
                working: 1,
                ..SourceCounts::default()
            },
        ));
        let scene = state.scene();
        assert_eq!(scene.phase, Phase::Unknown);
        assert_eq!(scene.sessions, 1);
        assert_eq!(scene.unknown, 1);
        assert_eq!(scene.connected_sources, 0);
        assert_eq!(scene.disconnected_sources, 1);
    }

    #[test]
    fn stale_generation_cannot_commit_or_remove_new_source() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        assert!(state.update_source(
            "one.sock",
            1,
            true,
            SourceCounts {
                sessions: 1,
                working: 1,
                ..SourceCounts::default()
            },
        ));
        assert!(state.begin_source("one.sock".to_owned(), 2));
        assert!(!state.update_source(
            "one.sock",
            1,
            true,
            SourceCounts {
                sessions: 1,
                blocked: 1,
                ..SourceCounts::default()
            },
        ));
        assert!(!state.remove_source("one.sock", 1));
        assert!(state.update_source(
            "one.sock",
            2,
            true,
            SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
        ));
        assert_eq!(state.scene().done, 1);
        assert_eq!(state.scene().working, 0);
    }

    #[test]
    fn repeated_generations_keep_last_known_sessions_until_new_snapshot() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        assert!(state.update_source(
            "one.sock",
            1,
            true,
            SourceCounts {
                sessions: 2,
                working: 2,
                ..SourceCounts::default()
            },
        ));
        assert!(state.begin_source("one.sock".to_owned(), 2));
        assert!(state.update_source(
            "one.sock",
            2,
            false,
            SourceCounts {
                sessions: 2,
                unknown: 2,
                ..SourceCounts::default()
            },
        ));
        assert!(state.begin_source("one.sock".to_owned(), 3));
        let scene = state.scene();
        assert_eq!(scene.sessions, 2);
        assert_eq!(scene.unknown, 2);
        assert_eq!(scene.phase, Phase::Unknown);
        assert!(state.update_source("one.sock", 3, true, SourceCounts::default(),));
        assert_eq!(state.scene().sessions, 0);
        assert_eq!(state.scene().phase, Phase::Idle);
    }

    #[test]
    fn independent_sources_with_equal_terminal_ids_are_counted_independently() {
        let mut state = AppState::new();
        connect(&mut state, "one.sock", 1);
        connect(&mut state, "two.sock", 1);
        let counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.update_source("one.sock", 1, true, counts));
        assert!(state.update_source("two.sock", 1, true, counts));
        let scene = state.scene();
        assert_eq!(scene.sessions, 2);
        assert_eq!(scene.working, 2);
        assert_eq!(scene.connected_sources, 2);
    }

    #[test]
    fn remote_snapshot_preserves_title_provenance_and_source_on_status_and_rename() {
        let mut state = AppState::new();
        let east = remote_source("east");
        let west = remote_source("west");
        state.set_observation_catalog(catalog(&[("east", "East", true), ("west", "West", true)]));
        state.apply_observation_preferences(policy(false, true, &["east", "west"]));
        connect(&mut state, &east, 1);
        connect(&mut state, &west, 1);
        let mut east_record = record("shared", "pane-east", AgentStatus::Working);
        east_record.metadata.title = Some("Build east".into());
        east_record.metadata.cwd = Some("/projects/east".into());
        east_record.metadata.tab_label = Some("2".into());
        let mut west_record = record("shared", "pane-west", AgentStatus::Blocked);
        west_record.metadata.title = Some("Build west".into());
        let counts = SourceCounts {
            sessions: 1,
            working: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            &east,
            1,
            [&east_record],
            counts,
            Arc::from(vec!["pane-east".to_owned()]),
            &[],
            Instant::now()
        ));
        let waiting = SourceCounts {
            sessions: 1,
            blocked: 1,
            ..SourceCounts::default()
        };
        assert!(state.publish_source_snapshot(
            &west,
            1,
            [&west_record],
            waiting,
            Arc::from(vec!["pane-west".to_owned()]),
            &[],
            Instant::now()
        ));
        let rows = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert_ne!(rows[0].key, rows[1].key);
        assert_eq!(rows[0].metadata, east_record.metadata);
        assert_eq!(rows[1].metadata.title.as_deref(), Some("Build west"));
        assert_eq!(rows[0].source_label, "East");
        assert_eq!(rows[1].source_label, "West");

        assert!(state.update_session_status(SessionStatusUpdate {
            source: &east,
            generation: 1,
            terminal_id: "shared",
            pane_id: "pane-east",
            status: AgentStatus::Done,
            counts: SourceCounts {
                sessions: 1,
                done: 1,
                ..SourceCounts::default()
            },
            completion: false,
            observed_at: Instant::now(),
        }));
        state.set_observation_catalog(catalog(&[
            ("east", "Renamed", true),
            ("west", "West", true),
        ]));
        let rows = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert_eq!(rows[0].metadata, east_record.metadata);
        assert_eq!(rows[0].source_label, "Renamed");
        assert_eq!(rows[0].status, AgentStatus::Done);
        assert_eq!(rows[1].metadata, west_record.metadata);
        assert_eq!(
            state
                .session_snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Completed),
                    None
                )
                .matched,
            1
        );
        assert_eq!(
            state
                .session_snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Waiting),
                    None
                )
                .matched,
            1
        );
    }

    #[test]
    fn legacy_and_recovery_revisions_precede_automation_reduction() {
        use crate::automation::{
            lock_automation, test_automation, PresentationAction, PresentationPatch,
            PresentationRequest, PresentationTarget,
        };

        let mut state = AppState::new();
        let automation = test_automation(PresentationTarget::from_scene(&state.scene()));
        state.set_automation(automation.clone());
        let stale = PresentationRequest {
            instance_id: "test".into(),
            operation_id: "stale".into(),
            expected_revision: Some(0),
            action: PresentationAction::Set {
                patch: PresentationPatch {
                    bubble_visible: Some(false),
                    ..PresentationPatch::default()
                },
            },
        };
        lock_automation(&automation).submit(stale.clone()).unwrap();
        state.apply_control("hide").unwrap();
        assert_eq!(lock_automation(&automation).snapshot().revision, 1);
        let stale_target = PresentationTarget::from_scene(&state.scene())
            .applying(&stale.action)
            .unwrap();
        assert!(lock_automation(&automation)
            .start_request("stale", stale_target)
            .is_err());
        assert!(state.scene().bubble_visible);

        let current = PresentationRequest {
            operation_id: "current".into(),
            expected_revision: Some(1),
            ..stale
        };
        lock_automation(&automation)
            .submit(current.clone())
            .unwrap();
        let current_target = PresentationTarget::from_scene(&state.scene())
            .applying(&current.action)
            .unwrap();
        lock_automation(&automation)
            .start_request("current", current_target)
            .unwrap();
        state.apply_presentation(&current.action).unwrap();
        assert_eq!(lock_automation(&automation).snapshot().revision, 2);
        state.apply_control("hide").unwrap();
        assert_eq!(lock_automation(&automation).snapshot().revision, 2);
        state.recover_interaction();
        assert_eq!(lock_automation(&automation).snapshot().revision, 3);
        assert!(state.scene().visible);
    }

    #[test]
    fn controls_preserve_scale_and_stop_semantics() {
        let mut state = AppState::new();
        assert!(state.apply_control("toggle").is_ok());
        assert!(!state.scene().visible);
        assert!(state.apply_control("toggle_passthrough").is_ok());
        assert!(state.scene().passthrough);
        assert!(state.set_scale(0.873125));
        assert!(!state.set_scale(f64::NAN));
        assert_eq!(state.scene().scale, 0.873125);
        state.request_shutdown();
        assert!(state.scene().shutdown);
    }

    #[test]
    fn full_and_alpha_passthrough_remain_independent() {
        let mut state = AppState::new();
        assert!(state.apply_control("alpha_passthrough").is_ok());
        assert!(state.scene().alpha_passthrough);
        assert!(!state.scene().passthrough);

        assert!(state.apply_control("toggle_passthrough").is_ok());
        assert!(state.scene().passthrough);
        assert!(state.scene().alpha_passthrough);

        assert!(state.apply_control("toggle_alpha_passthrough").is_ok());
        assert!(!state.scene().alpha_passthrough);
        assert!(state.scene().passthrough);

        assert!(state.apply_control("toggle_passthrough").is_ok());
        assert!(!state.scene().passthrough);
        assert!(!state.scene().alpha_passthrough);
    }

    #[test]
    fn bubble_controls_update_visibility_and_placement_without_touching_passthrough() {
        let mut state = AppState::new();
        assert!(state.apply_control("hide_bubble").is_ok());
        assert!(!state.scene().bubble_visible);
        assert!(state.apply_control("bubble_left").is_ok());
        assert_eq!(state.scene().bubble_placement, BubblePlacement::Left);
        assert!(state.apply_control("show_bubble").is_ok());
        assert!(state.scene().bubble_visible);
        assert!(state.apply_control("bubble_auto").is_ok());
        assert_eq!(state.scene().bubble_placement, BubblePlacement::Auto);
        assert!(!state.scene().passthrough);
        assert!(!state.scene().alpha_passthrough);
    }

    #[test]
    fn recovery_entry_requires_missing_interactive_surfaces_or_full_passthrough() {
        for (visible, bubble_visible, passthrough, shutdown, expected) in [
            (true, true, false, false, false),
            (true, false, false, false, false),
            (false, true, false, false, false),
            (false, false, false, false, true),
            (true, true, true, false, true),
            (true, false, true, false, true),
            (false, true, true, false, true),
            (false, false, true, false, true),
            (true, true, false, true, false),
            (false, false, false, true, false),
            (false, true, true, true, false),
        ] {
            for alpha_passthrough in [false, true] {
                let mut state = AppState::new();
                state.visible = visible;
                state.bubble_visible = bubble_visible;
                state.passthrough = passthrough;
                state.alpha_passthrough = alpha_passthrough;
                state.shutdown = shutdown;
                assert_eq!(
                    state.scene().needs_recovery_entry(),
                    expected,
                    "visible={visible}, bubble_visible={bubble_visible}, passthrough={passthrough}, shutdown={shutdown}, alpha_passthrough={alpha_passthrough}"
                );
            }
        }
    }

    #[test]
    fn recovery_restores_character_interaction_without_changing_other_settings() {
        for bubble_visible in [false, true] {
            let mut state = AppState::new();
            state.visible = false;
            state.passthrough = true;
            state.alpha_passthrough = true;
            state.bubble_visible = bubble_visible;
            state.bubble_placement = BubblePlacement::Left;
            state.scale = 0.873125;
            state.reset_position_revision = 7;
            let lifecycle = LifecycleSettings {
                auto_start: false,
                exit_with_herdr: false,
            };
            state.set_lifecycle_settings(lifecycle);

            state.recover_interaction();

            let scene = state.scene();
            assert!(scene.visible);
            assert!(!scene.passthrough);
            assert_eq!(scene.bubble_visible, bubble_visible);
            assert!(scene.alpha_passthrough);
            assert_eq!(scene.bubble_placement, BubblePlacement::Left);
            assert_eq!(scene.scale, 0.873125);
            assert_eq!(scene.reset_position_revision, 7);
            assert_eq!(state.lifecycle_settings(), lifecycle);
        }
    }

    #[test]
    fn show_control_does_not_disable_full_passthrough() {
        let mut state = AppState::new();
        state.visible = false;
        state.passthrough = true;
        state.bubble_visible = false;

        assert!(state.apply_control("show").is_ok());

        let scene = state.scene();
        assert!(scene.visible);
        assert!(scene.passthrough);
        assert!(!scene.bubble_visible);
        assert!(scene.needs_recovery_entry());
    }
}
