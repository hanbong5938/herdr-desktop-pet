use crate::agent_outcome::OutcomeReport;
use std::sync::Arc;
use std::time::Instant;

pub(crate) const MAX_AGENT_RECORDS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceWorktreeInfo {
    pub(crate) repo_key: String,
    pub(crate) repo_name: String,
    pub(crate) repo_root: String,
    pub(crate) checkout_path: String,
    pub(crate) is_linked_worktree: bool,
    pub(crate) pane_count: usize,
    pub(crate) tab_count: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionMetadata {
    pub(crate) title: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) worktree: Option<Arc<WorkspaceWorktreeInfo>>,
    pub(crate) workspace_label: Option<String>,
    pub(crate) tab_id: Option<String>,
    pub(crate) tab_label: Option<String>,
    pub(crate) cwd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentRecord {
    pub terminal_id: String,
    pub pane_id: String,
    pub status: AgentStatus,
    pub metadata: SessionMetadata,
    pub outcome_authoritative: bool,
    pub outcome: Option<OutcomeReport>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub panes: Vec<String>,
    pub agents: Vec<AgentRecord>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SnapshotCounts {
    pub sessions: usize,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub unknown: usize,
}

impl Snapshot {
    pub(crate) fn pane_ids(&self) -> Vec<String> {
        let mut panes = self.panes.clone();
        panes.sort_unstable();
        panes.dedup();
        panes
    }
}

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
