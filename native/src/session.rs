use std::time::Instant;

/// A single, per-session transition from active work to a known Done state.
///
/// Observations are emitted by the Herdr watcher only after a continuous
/// Working or Blocked state has been seen for the same terminal and pane.  The
/// source and generation identify the connection that observed the transition;
/// callers must not treat an observation from another generation as current.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionObservation {
    pub source: String,
    pub generation: u64,
    pub terminal_id: String,
    pub pane_id: String,
    pub observed_at: Instant,
}

/// A terminal result reported by the active agent's authoritative lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutcomeObservation {
    pub source: String,
    pub generation: u64,
    pub terminal_id: String,
    pub pane_id: String,
    pub outcome: crate::agent_outcome::AgentOutcome,
    pub at_unix_ms: u64,
    pub observed_at: Instant,
}
