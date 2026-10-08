use crate::agent_outcome::{AgentOutcome, OutcomeReport};
use crate::herdr_protocol::{AgentRecord, AgentStatus, SessionMetadata, WorkspaceWorktreeInfo};
use crate::i18n::{session_local_source, text, Message, UiLocale};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

const MAX_SOURCES: usize = 64;
const MAX_RECORDS_PER_SOURCE: usize = crate::herdr_protocol::MAX_AGENT_RECORDS;
const MAX_ROWS: usize = 128;
/// Maximum number of materialized rows in a single complete-session page.
pub(crate) const MAX_PAGE_ROWS: usize = MAX_ROWS;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionKey {
    pub(crate) source_id: u64,
    pub(crate) generation: u64,
    pub(crate) terminal_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Availability {
    Live,
    Offline,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DisplayStatus {
    NoSessions,
    Idle,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Completed,
    Unknown,
    Offline,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionStatusSummary {
    pub(crate) status: DisplayStatus,
    pub(crate) count: usize,
    pub(crate) total: usize,
}

impl Default for SessionStatusSummary {
    fn default() -> Self {
        Self {
            status: DisplayStatus::NoSessions,
            count: 0,
            total: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionFilter {
    #[default]
    All,
    Idle,
    Working,
    Waiting,
    Completed,
    Unknown,
    Offline,
}

impl SessionFilter {
    pub(crate) const ALL: [Self; 7] = [
        Self::All,
        Self::Idle,
        Self::Working,
        Self::Waiting,
        Self::Completed,
        Self::Unknown,
        Self::Offline,
    ];
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SessionSort {
    #[default]
    Stable,
    TitleAsc,
    SourceAsc,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionListOptions<'a> {
    pub(crate) filter: SessionFilter,
    pub(crate) query: &'a str,
    pub(crate) sort: SessionSort,
    pub(crate) running_first: bool,
    pub(crate) locale: UiLocale,
}

impl Default for SessionListOptions<'_> {
    fn default() -> Self {
        Self {
            filter: SessionFilter::All,
            query: "",
            sort: SessionSort::Stable,
            running_first: false,
            locale: UiLocale::En,
        }
    }
}

impl<'a> SessionListOptions<'a> {
    #[cfg(test)]
    pub(crate) fn with_filter(filter: SessionFilter) -> Self {
        Self {
            filter,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionView {
    pub(crate) key: SessionKey,
    pub(crate) source_label: String,
    pub(crate) is_local: bool,
    pub(crate) pane_id: String,
    pub(crate) status: AgentStatus,
    pub(crate) outcome: Option<AgentOutcome>,
    pub(crate) metadata: SessionMetadata,
    pub(crate) availability: Availability,
}

impl SessionView {
    pub(crate) fn display_status(&self) -> DisplayStatus {
        display_status(self.availability, self.status, self.outcome)
    }
}

fn display_status(
    availability: Availability,
    status: AgentStatus,
    outcome: Option<AgentOutcome>,
) -> DisplayStatus {
    if availability == Availability::Offline {
        return DisplayStatus::Offline;
    }
    match outcome {
        Some(AgentOutcome::Succeeded) => DisplayStatus::Succeeded,
        Some(AgentOutcome::Failed) => DisplayStatus::Failed,
        Some(AgentOutcome::Cancelled) => DisplayStatus::Cancelled,
        _ => match status {
            AgentStatus::Idle => DisplayStatus::Idle,
            AgentStatus::Working => DisplayStatus::Running,
            AgentStatus::Blocked => DisplayStatus::Waiting,
            AgentStatus::Done => DisplayStatus::Completed,
            AgentStatus::Unknown => DisplayStatus::Unknown,
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionSnapshot {
    pub(crate) revision: u64,
    pub(crate) rows: Vec<SessionView>,
    pub(crate) displays: Vec<CardDisplay>,
    pub(crate) status_summary: SessionStatusSummary,
    pub(crate) total: usize,
    pub(crate) matched: usize,
    pub(crate) omitted: usize,
    pub(crate) selected: Option<SessionKey>,
}

/// Exclusive position in visible source-id/terminal-id order. The caller must
/// also supply the revision returned by the previous page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCursor {
    pub(crate) source_id: u64,
    pub(crate) terminal_id: String,
}

/// A cursor belongs to exactly one daemon, store revision, and filter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionPageCursor {
    pub(crate) instance_id: String,
    pub(crate) revision: u64,
    pub(crate) filter: SessionFilter,
    pub(crate) position: SessionCursor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionPageRequest {
    pub(crate) instance_id: String,
    pub(crate) filter: SessionFilter,
    pub(crate) cursor: Option<SessionPageCursor>,
    pub(crate) limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionPage {
    pub(crate) revision: u64,
    pub(crate) rows: Vec<SessionView>,
    pub(crate) status_summary: SessionStatusSummary,
    pub(crate) total: usize,
    pub(crate) matched: usize,
    pub(crate) next_cursor: Option<SessionCursor>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPageError {
    StaleRevision,
    InvalidLimit,
}

struct CollectedRows {
    rows: Vec<SessionView>,
    displays: Vec<CardDisplay>,
    status_summary: SessionStatusSummary,
    total: usize,
    matched: usize,
    has_more: bool,
}

#[derive(Debug)]
struct StoredRecord {
    pane_id: String,
    status: AgentStatus,
    metadata: SessionMetadata,
    outcome: Option<AgentOutcome>,
    accepted_report: Option<OutcomeReport>,
    current_report: Option<OutcomeReport>,
}

#[derive(Debug)]
struct SourceState {
    source: String,
    source_id: u64,
    generation: u64,
    label: String,
    visible: bool,
    availability: Availability,
    records: BTreeMap<String, StoredRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptTarget {
    pub(crate) source: String,
    pub(crate) pane_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptTargetError {
    ReadOnly,
    Offline,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeRemoveTarget {
    pub(crate) key: SessionKey,
    pub(crate) source: String,
    pub(crate) pane_id: String,
    pub(crate) workspace_id: String,
    pub(crate) worktree: Arc<WorkspaceWorktreeInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeRemoveTargetError {
    ReadOnly,
    Offline,
    Stale,
    NotLinkedWorktree,
}

#[derive(Debug, Default)]
pub(crate) struct SessionStore {
    sources: Vec<SourceState>,
    next_source_id: u64,
    revision: u64,
}

impl SessionStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Begin a source generation. A newer generation keeps cached rows, but
    /// makes them offline until a coherent snapshot replaces them.
    ///
    /// The return value reports whether this was an accepted source operation,
    /// rather than whether visible rows changed. A duplicate same-generation
    /// begin is accepted and leaves the revision unchanged.
    pub(crate) fn begin_source(&mut self, source: &str, generation: u64) -> bool {
        if generation == 0 {
            return false;
        }
        if let Some(index) = self.source_index(source) {
            let existing = &mut self.sources[index];
            if generation < existing.generation {
                return false;
            }
            if generation == existing.generation {
                return true;
            }

            // Even already-offline rows receive a new key generation, which
            // invalidates a selected old-generation key.
            let was_live = existing.availability == Availability::Live;
            existing.generation = generation;
            existing.availability = Availability::Offline;
            for record in existing.records.values_mut() {
                record.outcome = None;
                record.accepted_report = None;
                record.current_report = None;
            }
            if was_live || !existing.records.is_empty() {
                self.bump_revision();
            }
            return true;
        }

        if self.sources.len() >= MAX_SOURCES {
            return false;
        }
        let Some(next_source_id) = self.next_source_id.checked_add(1) else {
            return false;
        };
        let source_id = self.next_source_id;
        self.next_source_id = next_source_id;
        // Source IDs are intentionally opaque and are never copied into a
        // card from the source path itself.
        self.sources.push(SourceState {
            source: source.to_owned(),
            source_id,
            generation,
            label: "Local".to_owned(),
            visible: true,
            availability: Availability::Offline,
            records: BTreeMap::new(),
        });
        self.bump_revision();
        true
    }

    /// Replace one current-generation source with a coherent bounded
    /// snapshot. Input records are borrowed and unchanged records are kept in
    /// place; only inserted or changed strings are copied.
    pub(crate) fn replace_source<'a, I>(
        &mut self,
        source: &str,
        generation: u64,
        records: I,
    ) -> bool
    where
        I: IntoIterator<Item = &'a AgentRecord>,
    {
        let Some(index) = self.source_index(source) else {
            return false;
        };
        if self.sources[index].generation != generation {
            return false;
        }

        // Protocol snapshots are already bounded, but keep this API bounded
        // for callers that construct an iterator directly. Holding only
        // references avoids cloning a complete incoming record set.
        let input: Vec<&AgentRecord> = records.into_iter().take(MAX_RECORDS_PER_SOURCE).collect();
        let mut accepted = HashSet::<&str>::with_capacity(input.len());
        for record in &input {
            if accepted.len() == MAX_RECORDS_PER_SOURCE {
                break;
            }
            accepted.insert(record.terminal_id.as_str());
        }

        let source_state = &mut self.sources[index];
        let had_rows = !source_state.records.is_empty();
        let was_live = source_state.availability == Availability::Live;
        let previous_len = source_state.records.len();
        let mut changed = false;

        for record in input {
            if !accepted.contains(record.terminal_id.as_str()) {
                continue;
            }
            if let Some(existing) = source_state.records.get_mut(&record.terminal_id) {
                let report = record
                    .outcome
                    .as_ref()
                    .filter(|_| record.outcome_authoritative);
                // Keep an accepted result only while the current authoritative
                // report is the very observation that the watcher accepted.
                // Raw status can stay Idle/Done across a session replacement,
                // missing report, or contradictory same-turn outcome.
                let accepted_report_changed = existing
                    .accepted_report
                    .as_ref()
                    .is_some_and(|accepted| report != Some(accepted));
                if existing.pane_id != record.pane_id {
                    existing.pane_id.clone_from(&record.pane_id);
                    existing.outcome = None;
                    existing.accepted_report = None;
                    changed = true;
                }
                if existing.status != record.status {
                    existing.status = record.status;
                    if matches!(record.status, AgentStatus::Working | AgentStatus::Blocked) {
                        existing.outcome = None;
                        existing.accepted_report = None;
                    }
                    changed = true;
                }
                if accepted_report_changed {
                    existing.accepted_report = None;
                    if existing.outcome.take().is_some() {
                        changed = true;
                    }
                }
                if existing.current_report.as_ref() != report {
                    existing.current_report = report.cloned();
                }
                if existing.metadata != record.metadata {
                    existing.metadata.clone_from(&record.metadata);
                    changed = true;
                }
            } else {
                source_state.records.insert(
                    record.terminal_id.clone(),
                    StoredRecord {
                        pane_id: record.pane_id.clone(),
                        status: record.status,
                        metadata: record.metadata.clone(),
                        outcome: None,
                        accepted_report: None,
                        current_report: record
                            .outcome
                            .as_ref()
                            .filter(|_| record.outcome_authoritative)
                            .cloned(),
                    },
                );
                changed = true;
            }
        }

        source_state
            .records
            .retain(|terminal_id, _| accepted.contains(terminal_id.as_str()));
        if source_state.records.len() != previous_len {
            changed = true;
        }
        let after_rows = !source_state.records.is_empty();
        if !was_live {
            source_state.availability = Availability::Live;
            changed = true;
        }
        if had_rows && !after_rows {
            changed = true;
        }
        if changed {
            self.bump_revision();
        }
        true
    }

    /// Apply a status event only to an existing current-generation terminal
    /// and its pane. Offline rows retain the event as last-known status but
    /// remain offline until a coherent snapshot arrives.
    pub(crate) fn update_status(
        &mut self,
        source: &str,
        generation: u64,
        terminal_id: &str,
        pane_id: &str,
        status: AgentStatus,
    ) -> bool {
        let Some(index) = self.source_index(source) else {
            return false;
        };
        let source_state = &mut self.sources[index];
        if source_state.generation != generation {
            return false;
        }
        let Some(record) = source_state.records.get_mut(terminal_id) else {
            return false;
        };
        if record.pane_id != pane_id {
            return false;
        }
        let mut changed = false;
        if matches!(status, AgentStatus::Working | AgentStatus::Blocked)
            && record.outcome.take().is_some()
        {
            record.accepted_report = None;
            changed = true;
        }
        if record.status != status {
            record.status = status;
            changed = true;
        }
        if changed {
            self.bump_revision();
        }
        true
    }

    /// Revoke an accepted result when the watcher loses unique ownership of
    /// this terminal/pane. Keep the raw report only as an observation baseline,
    /// never as an accepted card result.
    pub(crate) fn invalidate_outcome(
        &mut self,
        source: &str,
        generation: u64,
        terminal_id: &str,
        pane_id: &str,
    ) {
        let Some(source_state) = self
            .sources
            .iter_mut()
            .find(|entry| entry.source == source && entry.generation == generation)
        else {
            return;
        };
        let Some(record) = source_state.records.get_mut(terminal_id) else {
            return;
        };
        if record.pane_id == pane_id {
            record.accepted_report = None;
            if record.outcome.take().is_some() {
                self.bump_revision();
            }
        }
    }

    /// Mirror an already accepted watcher observation, never raw metadata.
    pub(crate) fn accept_outcome(
        &mut self,
        source: &str,
        generation: u64,
        terminal_id: &str,
        pane_id: &str,
        outcome: AgentOutcome,
        at_unix_ms: u64,
    ) -> bool {
        let Some(source_state) = self.sources.iter_mut().find(|entry| {
            entry.source == source
                && entry.generation == generation
                && entry.availability == Availability::Live
        }) else {
            return false;
        };
        let Some(record) = source_state.records.get_mut(terminal_id) else {
            return false;
        };
        let Some(report) = record.current_report.as_ref() else {
            return false;
        };
        if record.pane_id != pane_id
            || report.outcome != outcome
            || report.at_unix_ms != at_unix_ms
            || !matches!(
                outcome,
                AgentOutcome::Succeeded | AgentOutcome::Failed | AgentOutcome::Cancelled
            )
        {
            return false;
        }
        if record.outcome != Some(outcome) || record.accepted_report.as_ref() != Some(report) {
            record.outcome = Some(outcome);
            record.accepted_report = Some(report.clone());
            self.bump_revision();
        }
        true
    }

    /// Mark a current-generation source offline while retaining its last
    /// coherent rows and statuses.
    pub(crate) fn mark_offline(&mut self, source: &str, generation: u64) -> bool {
        let Some(index) = self.source_index(source) else {
            return false;
        };
        let source_state = &mut self.sources[index];
        if source_state.generation != generation {
            return false;
        }
        if source_state.availability == Availability::Live {
            // Empty included sources also change the aggregate indicator.
            source_state.availability = Availability::Offline;
            for record in source_state.records.values_mut() {
                record.outcome = None;
                record.accepted_report = None;
                record.current_report = None;
            }
            self.bump_revision();
        }
        true
    }

    /// Remove a current-generation source and all of its retained rows.
    pub(crate) fn remove_source(&mut self, source: &str, generation: u64) -> bool {
        let Some(index) = self.source_index(source) else {
            return false;
        };
        if self.sources[index].generation != generation {
            return false;
        }
        // Removing an empty source changes the no-sessions aggregate too.
        self.sources.remove(index);
        self.bump_revision();
        true
    }

    /// Resolve from retained source records, not the filtered and capped UI rows.
    pub(crate) fn prompt_target(
        &self,
        key: &SessionKey,
    ) -> Result<PromptTarget, PromptTargetError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.source_id == key.source_id)
            .ok_or(PromptTargetError::Stale)?;
        if source.generation != key.generation || !source.visible {
            return Err(PromptTargetError::Stale);
        }
        let record = source
            .records
            .get(&key.terminal_id)
            .ok_or(PromptTargetError::Stale)?;
        if crate::sources::remote_machine_id(&source.source).is_some() {
            return Err(PromptTargetError::ReadOnly);
        }
        if source.availability != Availability::Live {
            return Err(PromptTargetError::Offline);
        }
        if source.records.iter().any(|(terminal, other)| {
            terminal != &key.terminal_id && other.pane_id == record.pane_id
        }) {
            return Err(PromptTargetError::Stale);
        }
        Ok(PromptTarget {
            source: source.source.clone(),
            pane_id: record.pane_id.clone(),
        })
    }

    /// Read an identity from the retained, uncapped full source rows. A new
    /// generation is acceptable only for the same registered source ID/path;
    /// offline rows are never negative evidence.
    pub(crate) fn worktree_session_absent(
        &self,
        source_path: &str,
        source_id: u64,
        generation: u64,
        terminal_id: &str,
    ) -> Option<bool> {
        let source = self
            .sources
            .iter()
            .find(|entry| entry.source_id == source_id)?;
        if source.source != source_path
            || source.generation != generation
            || source.availability != Availability::Live
        {
            return None;
        }
        Some(!source.records.contains_key(terminal_id))
    }

    /// Resolve the clicked card's retained identity, never UI selection.
    pub(crate) fn worktree_remove_target(
        &self,
        key: &SessionKey,
    ) -> Result<WorktreeRemoveTarget, WorktreeRemoveTargetError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.source_id == key.source_id)
            .ok_or(WorktreeRemoveTargetError::Stale)?;
        if !source.visible || source.generation != key.generation {
            return Err(WorktreeRemoveTargetError::Stale);
        }
        let record = source
            .records
            .get(&key.terminal_id)
            .ok_or(WorktreeRemoveTargetError::Stale)?;
        if crate::sources::remote_machine_id(&source.source).is_some() {
            return Err(WorktreeRemoveTargetError::ReadOnly);
        }
        if source.availability != Availability::Live {
            return Err(WorktreeRemoveTargetError::Offline);
        }
        if source.records.iter().any(|(terminal, other)| {
            terminal != &key.terminal_id && other.pane_id == record.pane_id
        }) {
            return Err(WorktreeRemoveTargetError::Stale);
        }
        let (Some(workspace_id), Some(worktree)) = (
            record.metadata.workspace_id.as_ref(),
            record.metadata.worktree.as_ref(),
        ) else {
            return Err(WorktreeRemoveTargetError::NotLinkedWorktree);
        };
        if !worktree.is_linked_worktree || worktree.checkout_path == worktree.repo_root {
            return Err(WorktreeRemoveTargetError::NotLinkedWorktree);
        }
        // A pane can contain at most one terminal, but one workspace legitimately
        // contains multiple panes. Divergent descriptors cannot grant authority.
        if source.records.values().any(|other| {
            other.metadata.workspace_id.as_ref() == Some(workspace_id)
                && other.metadata.worktree.as_ref() != Some(worktree)
        }) {
            return Err(WorktreeRemoveTargetError::Stale);
        }
        Ok(WorktreeRemoveTarget {
            key: key.clone(),
            source: source.source.clone(),
            pane_id: record.pane_id.clone(),
            workspace_id: workspace_id.clone(),
            worktree: Arc::clone(worktree),
        })
    }

    /// Visibility changes invalidate cached snapshots even when rows are unchanged.
    pub(crate) fn set_visibility(&mut self, source: &str, visible: bool) {
        if let Some(entry) = self.sources.iter_mut().find(|entry| entry.source == source) {
            if entry.visible != visible {
                entry.visible = visible;
                self.bump_revision();
            }
        }
    }

    pub(crate) fn set_label(&mut self, source: &str, label: &str) {
        if let Some(entry) = self.sources.iter_mut().find(|entry| entry.source == source) {
            if entry.label != label {
                entry.label.clear();
                entry.label.push_str(label);
                if entry.visible {
                    self.bump_revision();
                }
            }
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.bump_revision();
    }
    /// Resolve a displayed key directly from retained rows, independently of
    /// snapshot filtering/capping and prompt eligibility.
    pub(crate) fn view_for_key(&self, key: &SessionKey) -> Option<SessionView> {
        let source = self.sources.iter().find(|source| {
            source.visible
                && source.source_id == key.source_id
                && source.generation == key.generation
        })?;
        let record = source.records.get(&key.terminal_id)?;
        Some(Self::view_from_record(source, &key.terminal_id, record))
    }
    /// Present a retained target against the complete visible universe, even
    /// when it is outside the current filter or the bounded list.
    pub(crate) fn display_for_key(
        &self,
        locale: UiLocale,
        key: &SessionKey,
    ) -> Option<CardDisplay> {
        if !self.contains_key(key) {
            return None;
        }
        let entries = self.presentation_rows();
        let index = entries.iter().position(|entry| {
            entry.key.source_id == key.source_id && entry.key.terminal_id == key.terminal_id
        })?;
        Some(presentation_displays(locale, &entries).swap_remove(index))
    }
    pub(crate) fn displays_for_keys(
        &self,
        locale: UiLocale,
        keys: &[&SessionKey],
    ) -> Vec<Option<CardDisplay>> {
        if keys.is_empty() {
            return Vec::new();
        }
        let entries = self.presentation_rows();
        let mut indices = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            indices.insert((entry.key.source_id, entry.key.terminal_id), index);
        }
        let displays = presentation_displays(locale, &entries);
        keys.iter()
            .map(|key| {
                if !self.contains_key(key) {
                    return None;
                }
                indices
                    .get(&(key.source_id, key.terminal_id.as_str()))
                    .map(|&index| displays[index].clone())
            })
            .collect()
    }

    fn presentation_rows(&self) -> Vec<PresentationRow<'_>> {
        self.sources
            .iter()
            .filter(|source| source.visible)
            .flat_map(|source| {
                let is_local = crate::sources::remote_machine_id(&source.source).is_none();
                source
                    .records
                    .iter()
                    .map(move |(terminal_id, record)| PresentationRow {
                        key: PresentationKey {
                            source_id: source.source_id,
                            terminal_id,
                        },
                        source_label: &source.label,
                        is_local,
                        metadata: &record.metadata,
                    })
            })
            .collect()
    }

    pub(crate) fn snapshot(
        &self,
        options: &SessionListOptions<'_>,
        selected: Option<&SessionKey>,
    ) -> SessionSnapshot {
        let collected = self.collect_rows(options.filter, None, MAX_ROWS, Some(options));
        let selected = selected.and_then(|key| self.contains_key(key).then(|| key.clone()));
        let omitted = collected.matched.saturating_sub(collected.rows.len());
        SessionSnapshot {
            revision: self.revision,
            rows: collected.rows,
            displays: collected.displays,
            status_summary: collected.status_summary,
            total: collected.total,
            matched: collected.matched,
            omitted,
            selected,
        }
    }

    /// Return an exclusive page in stable source-id/terminal-id order, without
    /// the GUI's search, sort, running priority, or 128-row truncation. Totals
    /// and the status summary describe *all* visible rows, not just this page.
    pub(crate) fn page(
        &self,
        filter: SessionFilter,
        expected_revision: u64,
        after: Option<&SessionCursor>,
        limit: usize,
    ) -> Result<SessionPage, SessionPageError> {
        if expected_revision != self.revision {
            return Err(SessionPageError::StaleRevision);
        }
        if limit == 0 || limit > MAX_PAGE_ROWS {
            return Err(SessionPageError::InvalidLimit);
        }
        let collected = self.collect_rows(filter, after, limit, None);
        let next_cursor = if collected.has_more {
            collected.rows.last().map(|row| SessionCursor {
                source_id: row.key.source_id,
                terminal_id: row.key.terminal_id.clone(),
            })
        } else {
            None
        };
        Ok(SessionPage {
            revision: self.revision,
            rows: collected.rows,
            status_summary: collected.status_summary,
            total: collected.total,
            matched: collected.matched,
            next_cursor,
        })
    }

    fn collect_rows(
        &self,
        filter: SessionFilter,
        after: Option<&SessionCursor>,
        limit: usize,
        options: Option<&SessionListOptions<'_>>,
    ) -> CollectedRows {
        let mut total = 0usize;
        let mut matched = 0usize;
        let mut rows = Vec::with_capacity(if options.is_some() { 0 } else { limit });
        let mut has_more = false;
        let mut candidates = Vec::with_capacity(if options.is_some() { limit } else { 0 });
        let query = options
            .map(|options| options.query.trim().to_lowercase())
            .unwrap_or_default();
        // Names are computed against the entire visible universe before filtering;
        // pagination does not need to materialize or sort any card displays.
        let presentation = options.map(|options| {
            let all = self.presentation_rows();
            let displays = presentation_displays(options.locale, &all);
            (all, displays)
        });
        let mut global_index = 0usize;
        let mut buckets = [0usize; 10];
        let mut included_sources = 0usize;
        let mut disconnected = false;
        // Source and terminal iteration share the order of `presentation_rows`.
        // A snapshot retains only the best 128 matching record references.
        for source in &self.sources {
            if !source.visible {
                continue;
            }
            included_sources += 1;
            disconnected |= source.availability == Availability::Offline;
            total = total.saturating_add(source.records.len());
            for (terminal_id, record) in &source.records {
                let index = global_index;
                global_index += 1;
                let availability = source.availability;
                let bucket = display_status(availability, record.status, record.outcome);
                let bucket_index = match bucket {
                    DisplayStatus::NoSessions => 0,
                    DisplayStatus::Idle => 1,
                    DisplayStatus::Running => 2,
                    DisplayStatus::Waiting => 3,
                    DisplayStatus::Succeeded => 4,
                    DisplayStatus::Failed => 5,
                    DisplayStatus::Cancelled => 6,
                    DisplayStatus::Completed => 7,
                    DisplayStatus::Unknown => 8,
                    DisplayStatus::Offline => 9,
                };
                buckets[bucket_index] += 1;
                if !filter_matches(filter, availability, record.status) {
                    continue;
                }
                if let Some((options, (_, displays))) = options.zip(presentation.as_ref()) {
                    if !query.is_empty()
                        && !search_matches(&query, options.locale, source, record, &displays[index])
                    {
                        continue;
                    }
                }
                matched = matched.saturating_add(1);
                if let Some((options, (all, displays))) = options.zip(presentation.as_ref()) {
                    let running = bucket == DisplayStatus::Running;
                    let title = &displays[index].title;
                    let unnamed = displays[index].unnamed;
                    let sort_key = match options.sort {
                        SessionSort::Stable => String::new(),
                        SessionSort::TitleAsc => title.to_lowercase(),
                        SessionSort::SourceAsc => {
                            card_source_label(options.locale, &all[index]).to_lowercase()
                        }
                    };
                    let candidate = Candidate {
                        source,
                        terminal_id,
                        record,
                        index,
                        running,
                        unnamed,
                        sort_key,
                    };
                    let position = candidates
                        .binary_search_by(|existing| {
                            compare_candidates(existing, &candidate, options)
                        })
                        .unwrap_or_else(|position| position);
                    if position < limit {
                        candidates.insert(position, candidate);
                        if candidates.len() > limit {
                            candidates.pop();
                        }
                    }
                } else {
                    if after.is_some_and(|cursor| {
                        source.source_id < cursor.source_id
                            || (source.source_id == cursor.source_id
                                && terminal_id.as_str() <= cursor.terminal_id.as_str())
                    }) {
                        continue;
                    }
                    if rows.len() < limit {
                        rows.push(Self::view_from_record(source, terminal_id, record));
                    } else {
                        has_more = true;
                    }
                }
            }
        }

        let status = if included_sources == 0 {
            DisplayStatus::NoSessions
        } else if buckets[3] > 0 {
            DisplayStatus::Waiting
        } else if buckets[2] > 0 {
            DisplayStatus::Running
        } else if buckets[9] > 0 || disconnected {
            DisplayStatus::Offline
        } else if buckets[8] > 0 {
            DisplayStatus::Unknown
        } else if buckets[5] > 0 {
            DisplayStatus::Failed
        } else if buckets[6] > 0 {
            DisplayStatus::Cancelled
        } else if total > 0 && buckets[4] == total {
            DisplayStatus::Succeeded
        } else if buckets[7] > 0 || buckets[4] > 0 {
            DisplayStatus::Completed
        } else {
            DisplayStatus::Idle
        };
        let count = match status {
            DisplayStatus::NoSessions => 0,
            DisplayStatus::Idle => buckets[1],
            DisplayStatus::Running => buckets[2],
            DisplayStatus::Waiting => buckets[3],
            DisplayStatus::Succeeded => buckets[4],
            DisplayStatus::Failed => buckets[5],
            DisplayStatus::Cancelled => buckets[6],
            DisplayStatus::Completed => buckets[7] + buckets[4],
            DisplayStatus::Unknown => buckets[8],
            DisplayStatus::Offline => buckets[9],
        };
        let status_summary = SessionStatusSummary {
            status,
            count,
            total,
        };
        let mut chosen_displays = Vec::with_capacity(candidates.len());
        if let Some((_, displays)) = presentation {
            rows.reserve(candidates.len());
            for candidate in candidates {
                rows.push(Self::view_from_record(
                    candidate.source,
                    candidate.terminal_id,
                    candidate.record,
                ));
                chosen_displays.push(displays[candidate.index].clone());
            }
        }
        CollectedRows {
            rows,
            displays: chosen_displays,
            status_summary,
            total,
            matched,
            has_more,
        }
    }

    fn view_from_record(
        source: &SourceState,
        terminal_id: &str,
        record: &StoredRecord,
    ) -> SessionView {
        SessionView {
            key: SessionKey {
                source_id: source.source_id,
                generation: source.generation,
                terminal_id: terminal_id.to_owned(),
            },
            source_label: source.label.clone(),
            is_local: crate::sources::remote_machine_id(&source.source).is_none(),
            pane_id: record.pane_id.clone(),
            status: record.status,
            outcome: record.outcome,
            metadata: record.metadata.clone(),
            availability: source.availability,
        }
    }

    fn source_index(&self, source: &str) -> Option<usize> {
        self.sources.iter().position(|entry| entry.source == source)
    }

    fn contains_key(&self, key: &SessionKey) -> bool {
        self.sources.iter().any(|source| {
            source.visible
                && source.source_id == key.source_id
                && source.generation == key.generation
                && source.records.contains_key(&key.terminal_id)
        })
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

fn filter_matches(filter: SessionFilter, availability: Availability, status: AgentStatus) -> bool {
    match filter {
        SessionFilter::All => true,
        SessionFilter::Offline => availability == Availability::Offline,
        // Cached offline statuses are deliberately excluded from every live
        // status filter. Offline has its own explicit filter.
        _ if availability == Availability::Offline => false,
        SessionFilter::Idle => status == AgentStatus::Idle,
        SessionFilter::Working => status == AgentStatus::Working,
        SessionFilter::Waiting => status == AgentStatus::Blocked,
        SessionFilter::Completed => status == AgentStatus::Done,
        SessionFilter::Unknown => status == AgentStatus::Unknown,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CardDisplay {
    pub(crate) title: String,
    pub(crate) context: String,
    unnamed: bool,
    directory_fallback: bool,
    directory_context: bool,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct PresentationKey<'a> {
    source_id: u64,
    terminal_id: &'a str,
}

struct PresentationRow<'a> {
    key: PresentationKey<'a>,
    source_label: &'a str,
    is_local: bool,
    metadata: &'a SessionMetadata,
}

struct Candidate<'a> {
    source: &'a SourceState,
    terminal_id: &'a str,
    record: &'a StoredRecord,
    index: usize,
    running: bool,
    unnamed: bool,
    sort_key: String,
}

fn compare_candidates(
    left: &Candidate<'_>,
    right: &Candidate<'_>,
    options: &SessionListOptions<'_>,
) -> Ordering {
    let priority = options
        .running_first
        .then(|| right.running.cmp(&left.running))
        .unwrap_or(Ordering::Equal);
    let sorted = match options.sort {
        SessionSort::Stable => Ordering::Equal,
        SessionSort::TitleAsc => left
            .unnamed
            .cmp(&right.unnamed)
            .then_with(|| left.sort_key.cmp(&right.sort_key)),
        SessionSort::SourceAsc => left.sort_key.cmp(&right.sort_key),
    };
    priority
        .then(sorted)
        .then_with(|| left.source.source_id.cmp(&right.source.source_id))
        .then_with(|| left.terminal_id.cmp(right.terminal_id))
}

fn search_matches(
    query: &str,
    locale: UiLocale,
    source: &SourceState,
    record: &StoredRecord,
    display: &CardDisplay,
) -> bool {
    let contains = |value: &str| value.to_lowercase().contains(query);
    [
        record.metadata.title.as_deref(),
        record.metadata.tab_label.as_deref(),
        record.metadata.workspace_label.as_deref(),
        record.metadata.agent.as_deref(),
        record.metadata.cwd.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|field| {
        contains(field) || display_value(Some(field)).is_some_and(|shown| contains(&shown))
    }) || contains(&display.title)
        || (record
            .metadata
            .tab_label
            .as_deref()
            .is_none_or(|tab| tab.trim().is_empty())
            && contains(text(locale, Message::UnnamedTab)))
        || (record.metadata.tab_label.as_deref().is_some_and(|tab| {
            tab.trim()
                .chars()
                .all(|character| character.is_ascii_digit())
                && contains(&format!("{} {}", text(locale, Message::Tab), tab.trim()))
        }))
        || (crate::sources::remote_machine_id(&source.source).is_none()
            && contains(session_local_source(locale)))
        || (crate::sources::remote_machine_id(&source.source).is_some()
            && display_value(Some(&source.label)).is_some_and(|label| contains(&label)))
}

pub(crate) fn display_value(value: Option<&str>) -> Option<String> {
    let value = value?;
    let mut result = String::with_capacity(value.len());
    let mut space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            space = !result.is_empty();
        } else if !unsafe_identifier_char(character) {
            if space {
                result.push(' ');
                space = false;
            }
            result.push(character);
        }
    }
    (!result.is_empty()).then_some(result)
}

fn meaningful_title(value: Option<&str>) -> Option<String> {
    let title = display_value(value)?;
    if [
        "omp",
        "claude",
        "claude code",
        "codex",
        "openai codex",
        "terminal",
        "shell",
        "bash",
        "zsh",
        "fish",
        "sh",
        "nu",
        "pwsh",
        "powershell",
    ]
    .iter()
    .any(|generic| title.eq_ignore_ascii_case(generic))
    {
        return None;
    }
    Some(title)
}

fn tab_location(locale: UiLocale, view: &PresentationRow<'_>) -> String {
    match display_value(view.metadata.tab_label.as_deref()) {
        Some(label) if label.chars().all(|c| c.is_ascii_digit()) => {
            format!("{} {label}", text(locale, Message::Tab))
        }
        Some(label) => label,
        None => text(locale, Message::UnnamedTab).to_owned(),
    }
}

fn card_source_label<'a>(locale: UiLocale, view: &'a PresentationRow<'_>) -> Cow<'a, str> {
    if view.is_local {
        Cow::Borrowed(session_local_source(locale))
    } else {
        display_value(Some(&view.source_label))
            .map(Cow::Owned)
            .unwrap_or_else(|| Cow::Owned(format!("#{}", view.key.source_id)))
    }
}

#[cfg(test)]
pub(crate) fn card_displays(locale: UiLocale, rows: &[SessionView]) -> Vec<CardDisplay> {
    let entries: Vec<_> = rows
        .iter()
        .map(|view| PresentationRow {
            key: PresentationKey {
                source_id: view.key.source_id,
                terminal_id: &view.key.terminal_id,
            },
            source_label: &view.source_label,
            is_local: view.is_local,
            metadata: &view.metadata,
        })
        .collect();
    presentation_displays(locale, &entries)
}

fn presentation_displays(locale: UiLocale, rows: &[PresentationRow<'_>]) -> Vec<CardDisplay> {
    let cwd_paths: Vec<_> = rows
        .iter()
        .map(|view| display_value(view.metadata.cwd.as_deref()))
        .collect();
    let mut displays: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, view)| {
            let workspace = display_value(view.metadata.workspace_label.as_deref());
            let agent = display_value(view.metadata.agent.as_deref());
            let cwd = cwd_paths[index].as_deref();
            let directory = cwd
                .and_then(|path| path.rsplit('/').find(|part| !part.is_empty()))
                .map(str::to_owned);
            let tab = tab_location(locale, view);
            let title = meaningful_title(view.metadata.title.as_deref()).or_else(|| {
                meaningful_title(view.metadata.tab_label.as_deref())
                    .filter(|label| !label.chars().all(|c| c.is_ascii_digit()))
            });
            let directory_context = workspace.is_none() && directory.is_some();
            let directory_fallback = title.is_none() && directory_context;
            let unnamed = title.is_none() && workspace.is_none() && directory.is_none();
            let title = title.unwrap_or_else(|| {
                workspace
                    .clone()
                    .or(directory.clone())
                    .map(|base| format!("{base} · {tab}"))
                    .unwrap_or_else(|| text(locale, Message::UnnamedSession).to_owned())
            });
            let context = [workspace.or(directory), Some(tab), agent]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            CardDisplay {
                title,
                context,
                unnamed,
                directory_fallback,
                directory_context,
            }
        })
        .collect();

    // Compare original basenames before modifying either display.
    let original_groups: Vec<Vec<usize>> = {
        let mut groups: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        for (index, display) in displays.iter().enumerate() {
            if display.directory_context {
                groups
                    .entry((&display.title, &display.context))
                    .or_default()
                    .push(index);
            }
        }
        groups
            .into_values()
            .filter(|group| group.len() > 1)
            .collect()
    };

    // Refine only colliding directory groups. A short, exhausted path stays in
    // its previous collision group until the longer paths have been partitioned.
    #[derive(Clone, Copy)]
    struct SuffixState<'a> {
        index: usize,
        remaining: &'a str,
        depth: usize,
    }

    for group in &original_groups {
        let mut frontier = vec![group
            .iter()
            .map(|&index| SuffixState {
                index,
                remaining: cwd_paths[index]
                    .as_deref()
                    .expect("directory context has cwd"),
                depth: 0,
            })
            .collect::<Vec<_>>()];
        let mut chosen = Vec::with_capacity(group.len());
        while let Some(states) = frontier.pop() {
            let mut buckets: HashMap<&str, Vec<SuffixState<'_>>> = HashMap::new();
            for mut state in states {
                let component = loop {
                    match state.remaining.rsplit_once('/') {
                        Some((prefix, "")) => state.remaining = prefix,
                        Some((prefix, part)) => {
                            state.remaining = prefix;
                            break Some(part);
                        }
                        None if state.remaining.is_empty() => break None,
                        None => {
                            let part = state.remaining;
                            state.remaining = "";
                            break Some(part);
                        }
                    }
                };
                if let Some(component) = component {
                    state.depth += 1;
                    buckets.entry(component).or_default().push(state);
                } else {
                    chosen.push(state);
                }
            }
            for (_, mut bucket) in buckets {
                if bucket.len() == 1 {
                    chosen.push(bucket.pop().expect("singleton suffix bucket"));
                } else {
                    frontier.push(bucket);
                }
            }
        }
        for state in chosen {
            debug_assert!(state.depth > 0);
            let index = state.index;
            let path = cwd_paths[index]
                .as_deref()
                .expect("directory context has cwd");
            let mut directory = String::new();
            for part in path[state.remaining.len()..]
                .split('/')
                .filter(|part| !part.is_empty())
            {
                if !directory.is_empty() {
                    directory.push('/');
                }
                directory.push_str(part);
            }
            if displays[index].directory_fallback {
                displays[index].title =
                    format!("{directory} · {}", tab_location(locale, &rows[index]));
            }
            let basename = path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .expect("directory context has basename");
            if let Some(rest) = displays[index].context.strip_prefix(basename) {
                displays[index].context = format!("{directory}{rest}");
            }
        }
    }
    let mut collision_groups: Vec<Vec<usize>> = {
        let mut groups: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        for (index, display) in displays.iter().enumerate() {
            groups
                .entry((&display.title, &display.context))
                .or_default()
                .push(index);
        }
        groups.into_values().collect()
    };
    // Keys, not current filter/order, define compact ranks.
    for group in &mut collision_groups {
        if group.len() == 1 {
            let index = group[0];
            let label = card_source_label(locale, &rows[index]);
            if displays[index].title == text(locale, Message::UnnamedSession) {
                let source = rows[index].key.source_id;
                displays[index].context =
                    format!("(#{source}) · {label} · {}", displays[index].context);
            } else {
                displays[index].context = format!("{label} · {}", displays[index].context);
            }
            continue;
        }
        group.sort_unstable_by_key(|&index| rows[index].key);
        let mut source_counts: HashMap<u64, usize> = HashMap::new();
        for &index in group.iter() {
            *source_counts.entry(rows[index].key.source_id).or_default() += 1;
        }
        let mut ranks: HashMap<u64, usize> = HashMap::new();
        for &index in group.iter() {
            let source = rows[index].key.source_id;
            let rank = ranks.entry(source).or_default();
            *rank += 1;
            let label = card_source_label(locale, &rows[index]);
            let discriminator = if source_counts[&source] > 1 {
                format!("#{source} · {rank}")
            } else {
                format!("#{source}")
            };
            displays[index].context =
                format!("({discriminator}) · {label} · {}", displays[index].context);
        }
    }
    displays
}
fn unsafe_identifier_char(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn record(terminal_id: &str, pane_id: &str, status: AgentStatus) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal_id.to_owned(),
            pane_id: pane_id.to_owned(),
            status,
            metadata: SessionMetadata::default(),
            outcome_authoritative: false,
            outcome: None,
        }
    }

    fn reported(terminal: &str, turn: &str, outcome: AgentOutcome) -> AgentRecord {
        let mut row = record(terminal, terminal, AgentStatus::Done);
        row.outcome_authoritative = true;
        row.outcome = Some(OutcomeReport {
            session: "session-one".into(),
            turn: turn.into(),
            outcome,
            at_unix_ms: 123,
        });
        row
    }

    fn path_record(terminal_id: &str, cwd: &str, status: AgentStatus) -> AgentRecord {
        let mut row = record(terminal_id, terminal_id, status);
        row.metadata.cwd = Some(cwd.into());
        row.metadata.tab_label = Some("2".into());
        row
    }

    #[test]
    fn directory_names_partition_short_normalized_and_unrelated_paths() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut rows = vec![
            path_record("short", "/src", AgentStatus::Idle),
            path_record("alpha", "/alpha/src", AgentStatus::Idle),
            path_record("beta", "/beta/src", AgentStatus::Idle),
            path_record("deep", "/alpha/deep/src", AgentStatus::Idle),
            path_record("n1", "/same//src//", AgentStatus::Idle),
            path_record("n2", "///same/src/", AgentStatus::Idle),
            path_record("root", "////", AgentStatus::Idle),
        ];
        let mut titled = path_record("titled", "/alpha/src", AgentStatus::Idle);
        titled.metadata.title = Some("Build".into());
        rows.push(titled);
        let mut workspace = path_record("workspace", "/alpha/src", AgentStatus::Idle);
        workspace.metadata.workspace_label = Some("Project".into());
        rows.push(workspace);
        assert!(store.replace_source("socket", 1, rows.iter()));
        let snapshot = store.snapshot(&SessionListOptions::default(), None);
        let display = |id: &str| {
            let index = snapshot
                .rows
                .iter()
                .position(|row| row.key.terminal_id == id)
                .unwrap();
            &snapshot.displays[index]
        };
        for (id, expected) in [
            ("short", "src · Tab 2"),
            ("alpha", "alpha/src · Tab 2"),
            ("beta", "beta/src · Tab 2"),
            ("deep", "deep/src · Tab 2"),
            ("n1", "same/src · Tab 2"),
            ("n2", "same/src · Tab 2"),
        ] {
            assert_eq!(display(id).title, expected, "{id}");
            assert!(display(id).context.contains(expected), "{id}");
        }
        assert!(display("n1").context.starts_with("(#0 · 1) · "));
        assert!(display("n2").context.starts_with("(#0 · 2) · "));
        assert_eq!(
            display("root").title,
            text(UiLocale::En, Message::UnnamedSession)
        );
        assert!(!display("root").context.contains("src"));
        assert_eq!(display("titled").title, "Build");
        assert!(display("titled").context.contains(" · src · Tab 2"));
        assert_eq!(display("workspace").title, "Project · Tab 2");
        assert!(display("workspace").context.contains(" · Project · Tab 2"));
    }

    #[test]
    fn directory_names_refine_long_shared_suffix_without_depth_limit() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let shared = "nested/".repeat(160);
        let west = format!("/west/{shared}src");
        let east = format!("/east/{shared}src");
        let other = path_record("other", "/other/src", AgentStatus::Idle);
        let rows = [
            path_record("west", &west, AgentStatus::Idle),
            path_record("east", &east, AgentStatus::Idle),
            other,
        ];
        assert!(store.replace_source("socket", 1, rows.iter()));
        let snapshot = store.snapshot(&SessionListOptions::default(), None);
        for (id, expected) in [
            ("west", west.trim_start_matches('/').to_owned()),
            ("east", east.trim_start_matches('/').to_owned()),
            ("other", "other/src".to_owned()),
        ] {
            let index = snapshot
                .rows
                .iter()
                .position(|row| row.key.terminal_id == id)
                .unwrap();
            assert_eq!(
                snapshot.displays[index].title,
                format!("{expected} · Tab 2")
            );
            assert!(snapshot.displays[index].context.contains(&expected));
        }
    }

    #[test]
    fn identical_suffixes_keep_source_ranks_after_reordering() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("first", 1));
        assert!(store.begin_source("second", 1));
        let first = [
            path_record("z", "/one//src/", AgentStatus::Idle),
            path_record("a", "/one/src", AgentStatus::Idle),
        ];
        let second = [
            path_record("b", "/one/src", AgentStatus::Idle),
            path_record("c", "/two/src", AgentStatus::Idle),
        ];
        assert!(store.replace_source("first", 1, first.iter()));
        assert!(store.replace_source("second", 1, second.iter()));
        let expected = [
            ("a", "one/src · Tab 2", "(#0 · 1) · "),
            ("z", "one/src · Tab 2", "(#0 · 2) · "),
            ("b", "one/src · Tab 2", "(#1) · "),
            ("c", "two/src · Tab 2", "This Mac · "),
        ];
        for reverse in [false, true] {
            if reverse {
                assert!(store.replace_source("first", 1, first.iter().rev()));
                assert!(store.replace_source("second", 1, second.iter().rev()));
            }
            let snapshot = store.snapshot(&SessionListOptions::default(), None);
            for (id, title, prefix) in expected {
                let index = snapshot
                    .rows
                    .iter()
                    .position(|row| row.key.terminal_id == id)
                    .unwrap();
                assert_eq!(snapshot.displays[index].title, title, "{id}");
                assert!(snapshot.displays[index].context.starts_with(prefix), "{id}");
            }
        }
    }

    #[test]
    fn names_use_full_visible_universe_before_query_filter_sort_cap_and_lookup() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("first", 1));
        assert!(store.begin_source("second", 1));
        let mut rows: Vec<_> = (0..129)
            .map(|index| {
                path_record(
                    &format!("a-{index:03}"),
                    &format!("/filler-{index:03}/src"),
                    AgentStatus::Idle,
                )
            })
            .collect();
        rows.push(path_record(
            "z-target",
            "/north/needle/src",
            AgentStatus::Working,
        ));
        let rival = path_record("rival", "/south/needle/src", AgentStatus::Idle);
        assert!(store.replace_source("first", 1, rows.iter()));
        assert!(store.replace_source("second", 1, [&rival]));
        let key = SessionKey {
            source_id: 0,
            generation: 1,
            terminal_id: "z-target".into(),
        };
        let original = store.display_for_key(UiLocale::En, &key).unwrap();
        assert_eq!(original.title, "north/needle/src · Tab 2");
        let full = store.snapshot(&SessionListOptions::default(), Some(&key));
        assert_eq!((full.total, full.matched, full.omitted), (131, 131, 3));
        assert!(full.rows.iter().all(|row| row.key != key));
        assert_eq!(full.selected, Some(key.clone()));
        for options in [
            SessionListOptions {
                query: "north",
                ..SessionListOptions::default()
            },
            SessionListOptions::with_filter(SessionFilter::Working),
            SessionListOptions {
                sort: SessionSort::TitleAsc,
                query: "north",
                ..SessionListOptions::default()
            },
            SessionListOptions {
                sort: SessionSort::SourceAsc,
                running_first: true,
                filter: SessionFilter::Working,
                ..SessionListOptions::default()
            },
        ] {
            let snapshot = store.snapshot(&options, Some(&key));
            assert_eq!(snapshot.matched, 1);
            assert_eq!(snapshot.rows[0].key, key);
            assert_eq!(snapshot.displays[0], original);
        }
        store.set_visibility("second", false);
        let narrowed = store.display_for_key(UiLocale::En, &key).unwrap();
        assert_eq!(narrowed.title, "needle/src · Tab 2");
        assert_eq!(
            store
                .snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Working),
                    None
                )
                .displays[0],
            narrowed
        );
        store.set_visibility("second", true);
        assert_eq!(store.display_for_key(UiLocale::En, &key), Some(original));
    }

    #[test]
    fn search_intersects_status_before_cap_and_uses_localized_visible_fields() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket-private-id", 1));
        let mut rows: Vec<_> = (0..130)
            .map(|index| {
                let mut row = record(&format!("hidden-{index:03}"), "pane", AgentStatus::Idle);
                row.metadata.title = Some("Irrelevant".into());
                row
            })
            .collect();
        let mut target = record("opaque-needlestem", "pane-target", AgentStatus::Working);
        target.metadata.title = Some("🧪 한국어 LATIN".into());
        target.metadata.cwd = Some("/workspace/src".into());
        target.metadata.tab_label = Some("2".into());
        rows.push(target);
        assert!(store.replace_source("socket-private-id", 1, rows.iter()));
        let key = store
            .snapshot(
                &SessionListOptions::with_filter(SessionFilter::Working),
                None,
            )
            .rows[0]
            .key
            .clone();
        for query in [" 한국어 ", "🧪", "latin", "/WORKSPACE/SRC", "Tab 2"] {
            let options = SessionListOptions {
                query,
                filter: SessionFilter::Working,
                ..SessionListOptions::default()
            };
            let snapshot = store.snapshot(&options, Some(&key));
            assert_eq!(
                (snapshot.total, snapshot.matched, snapshot.omitted),
                (131, 1, 0)
            );
            assert_eq!(snapshot.rows[0].key, key);
            assert_eq!(snapshot.selected, Some(key.clone()));
        }
        let across_cap = store.snapshot(
            &SessionListOptions {
                query: "latin",
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(across_cap.rows[0].key, key);
        assert_eq!((across_cap.matched, across_cap.omitted), (1, 0));
        let whitespace = store.snapshot(
            &SessionListOptions {
                query: "   ",
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!((whitespace.matched, whitespace.omitted), (131, 3));
        for query in ["needlestem", "socket-private-id", "pane-target"] {
            let snapshot = store.snapshot(
                &SessionListOptions {
                    query,
                    ..SessionListOptions::default()
                },
                None,
            );
            assert_eq!(snapshot.matched, 0, "{query}");
        }
        let snapshot = store.snapshot(
            &SessionListOptions {
                query: "🧪",
                filter: SessionFilter::Idle,
                ..SessionListOptions::default()
            },
            Some(&key),
        );
        assert_eq!(snapshot.matched, 0);
        assert_eq!(snapshot.selected, Some(key));
        assert_eq!(snapshot.status_summary.total, 131);
        let unnamed = record("blank", "pane", AgentStatus::Idle);
        assert!(store.replace_source("socket-private-id", 1, [&unnamed]));
        let korean = store.snapshot(
            &SessionListOptions {
                query: text(UiLocale::Ko, Message::UnnamedSession),
                locale: UiLocale::Ko,
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(korean.matched, 1);
        let local = store.snapshot(
            &SessionListOptions {
                query: session_local_source(UiLocale::Ko),
                locale: UiLocale::Ko,
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(local.matched, 1);
    }

    #[test]
    fn running_priority_is_global_bounded_and_requires_effective_live_running() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("first", 1));
        assert!(store.begin_source("second", 1));
        let first: Vec<_> = (0..140)
            .map(|index| record(&format!("idle-{index:03}"), "pane", AgentStatus::Idle))
            .collect();
        let second: Vec<_> = (0..140)
            .map(|index| record(&format!("running-{index:03}"), "pane", AgentStatus::Working))
            .collect();
        assert!(store.replace_source("first", 1, first.iter()));
        assert!(store.replace_source("second", 1, second.iter()));
        let pinned = SessionListOptions {
            running_first: true,
            ..SessionListOptions::default()
        };
        let snapshot = store.snapshot(&pinned, None);
        assert_eq!(
            (snapshot.total, snapshot.matched, snapshot.omitted),
            (280, 280, 152)
        );
        assert_eq!(snapshot.rows.len(), 128);
        assert!(snapshot
            .rows
            .iter()
            .all(|row| row.display_status() == DisplayStatus::Running));
        assert_eq!(snapshot.rows[127].key.terminal_id, "running-127");
        let default = store.snapshot(&SessionListOptions::default(), None);
        assert!(default
            .rows
            .iter()
            .all(|row| row.status == AgentStatus::Idle));
        let selected = snapshot.rows[127].key.clone();
        assert!(store.mark_offline("second", 1));
        let offline = store.snapshot(&pinned, Some(&selected));
        assert_eq!(offline.rows[0].key.source_id, default.rows[0].key.source_id);
        assert_eq!(offline.selected, Some(selected));
        assert_eq!(offline.status_summary.status, DisplayStatus::Offline);
        let mut completed = reported("idle-000", "turn", AgentOutcome::Succeeded);
        completed.status = AgentStatus::Working;
        completed.pane_id = "pane".into();
        let mut revised = first.clone();
        revised[0] = completed;
        assert!(store.replace_source("first", 1, revised.iter()));
        assert!(store.accept_outcome("first", 1, "idle-000", "pane", AgentOutcome::Succeeded, 123));
        let effective = store.snapshot(&pinned, None);
        assert_eq!(effective.rows[0].display_status(), DisplayStatus::Succeeded);
        assert!(effective
            .rows
            .iter()
            .all(|row| row.display_status() != DisplayStatus::Running));
    }

    #[test]
    fn sorting_and_duplicate_discriminators_hold_outside_default_cap() {
        let mut store = SessionStore::new();
        let first = crate::sources::remote_source("east");
        let second = crate::sources::remote_source("west");
        assert!(store.begin_source(&first, 1));
        assert!(store.begin_source(&second, 1));
        store.set_label(&first, "Zulu");
        store.set_label(&second, "Alpha");
        let mut early: Vec<_> = (0..130)
            .map(|index| {
                let mut row = record(&format!("a-{index:03}"), "pane", AgentStatus::Idle);
                row.metadata.title = Some("Shared".into());
                row.metadata.tab_label = Some("2".into());
                row.metadata.workspace_label = Some("Idea".into());
                row
            })
            .collect();
        early[129].metadata.cwd = Some("/secret/only".into());
        let mut later = record("z-later", "pane", AgentStatus::Idle);
        later.metadata.title = Some("Aardvark".into());
        assert!(store.replace_source(&first, 1, early.iter()));
        assert!(store.replace_source(&second, 1, [&later]));
        let title = SessionListOptions {
            sort: SessionSort::TitleAsc,
            ..SessionListOptions::default()
        };
        let snapshot = store.snapshot(&title, None);
        assert_eq!(snapshot.rows[0].key.terminal_id, "z-later");
        assert_eq!((snapshot.matched, snapshot.omitted), (131, 3));
        let page = store
            .page(SessionFilter::All, snapshot.revision, None, 1)
            .unwrap();
        assert_eq!(page.rows[0].key.terminal_id, "a-000");
        assert_eq!(page.rows[0].metadata.title.as_deref(), Some("Shared"));
        assert_eq!(page.matched, snapshot.matched);
        assert_eq!(page.status_summary, snapshot.status_summary);
        let source = store.snapshot(
            &SessionListOptions {
                sort: SessionSort::SourceAsc,
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(source.rows[0].key.terminal_id, "z-later");
        let selected = SessionKey {
            source_id: 0,
            generation: 1,
            terminal_id: "a-129".into(),
        };
        let display = store.display_for_key(UiLocale::En, &selected).unwrap();
        let displayed = store.snapshot(
            &SessionListOptions {
                query: "Shared",
                ..title
            },
            Some(&selected),
        );
        assert!(displayed.rows.iter().all(|row| row.key != selected));
        assert!(display.context.starts_with("(#0 · 130) · "));
        let narrow = store.snapshot(
            &SessionListOptions {
                query: "SECRET",
                ..title
            },
            Some(&selected),
        );
        assert_eq!(narrow.matched, 1);
        assert_eq!(narrow.rows[0].key, selected);
        assert_eq!(narrow.displays[0], display);
        assert_eq!(displayed.selected, Some(selected));
        assert_eq!(displayed.matched, 130);
        assert_eq!(store.revision(), snapshot.revision);
        let named_source = store.snapshot(
            &SessionListOptions {
                query: "aLpHa",
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(named_source.matched, 1);
        assert_eq!(named_source.rows[0].key.terminal_id, "z-later");
        store.set_label(&second, "Beta");
        assert_eq!(
            store
                .snapshot(
                    &SessionListOptions {
                        query: "Alpha",
                        ..SessionListOptions::default()
                    },
                    None
                )
                .matched,
            0
        );
        assert_eq!(
            store
                .snapshot(
                    &SessionListOptions {
                        query: "beta",
                        ..SessionListOptions::default()
                    },
                    None
                )
                .matched,
            1
        );
    }

    #[test]
    fn title_sort_prefers_named_tab_then_workspace_and_leaves_unnamed_last() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut explicit = record("a", "pane-a", AgentStatus::Idle);
        explicit.metadata.title = Some("Zebra".into());
        let mut named_tab = record("b", "pane-b", AgentStatus::Idle);
        named_tab.metadata.title = Some("zsh".into());
        named_tab.metadata.tab_label = Some("Alpha".into());
        let mut workspace = record("c", "pane-c", AgentStatus::Idle);
        workspace.metadata.tab_label = Some("2".into());
        workspace.metadata.workspace_label = Some("Beta".into());
        let unnamed = record("d", "pane-d", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, [&explicit, &named_tab, &workspace, &unnamed]));
        let snapshot = store.snapshot(
            &SessionListOptions {
                sort: SessionSort::TitleAsc,
                ..SessionListOptions::default()
            },
            None,
        );
        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| row.key.terminal_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c", "a", "d"]
        );
        assert_eq!(snapshot.displays[1].title, "Beta · Tab 2");
        assert_eq!(
            snapshot.displays[3].title,
            text(UiLocale::En, Message::UnnamedSession)
        );
    }

    #[test]
    fn terminal_results_require_acceptance_and_new_activity_invalidates_them() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        for (outcome, display) in [
            (AgentOutcome::Succeeded, DisplayStatus::Succeeded),
            (AgentOutcome::Failed, DisplayStatus::Failed),
            (AgentOutcome::Cancelled, DisplayStatus::Cancelled),
        ] {
            let row = reported("terminal", "1", outcome);
            assert!(store.replace_source("socket", 1, [&row]));
            assert_eq!(
                store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
                DisplayStatus::Completed
            );
            let revision = store.revision();
            assert!(store.accept_outcome("socket", 1, "terminal", "terminal", outcome, 123));
            assert!(store.revision() > revision);
            assert_eq!(
                store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
                display
            );
            assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Working));
            assert_eq!(
                store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
                DisplayStatus::Running
            );
        }
        let row = reported("terminal", "2", AgentOutcome::Succeeded);
        assert!(store.replace_source("socket", 1, [&row]));
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let mut running = reported("terminal", "3", AgentOutcome::Running);
        running.status = AgentStatus::Working;
        assert!(store.replace_source("socket", 1, [&running]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Running
        );
        assert!(store.replace_source("socket", 1, [&row]));
        assert_ne!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        // A new running report clears an accepted result even if raw Working
        // never changed; repeating the older terminal report cannot restore it.
        let mut lagging = reported("terminal", "4", AgentOutcome::Succeeded);
        lagging.status = AgentStatus::Working;
        assert!(store.replace_source("socket", 1, [&lagging]));
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        let mut new_work = reported("terminal", "5", AgentOutcome::Running);
        new_work.status = AgentStatus::Working;
        assert!(store.replace_source("socket", 1, [&new_work]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Running
        );
        assert!(store.replace_source("socket", 1, [&lagging]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Running
        );
    }

    #[test]
    fn source_and_report_identity_invalidate_success_without_revival() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = reported("terminal", "1", AgentOutcome::Succeeded);
        assert!(store.replace_source("socket", 1, [&row]));
        assert!(!store.accept_outcome(
            "socket",
            1,
            "terminal",
            "wrong-pane",
            AgentOutcome::Succeeded,
            123
        ));
        assert!(!store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            124
        ));
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let mut next = reported("terminal", "2", AgentOutcome::Running);
        next.status = AgentStatus::Working;
        assert!(store.replace_source("socket", 1, [&next]));
        assert!(store.replace_source("socket", 1, [&row]));
        assert_ne!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        assert!(store.mark_offline("socket", 1));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Offline
        );
        assert!(store.begin_source("socket", 2));
        assert!(store.replace_source("socket", 2, [&row]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Completed
        );
        assert!(!store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        assert!(store.accept_outcome(
            "socket",
            2,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let mut changed = row.clone();
        changed.outcome.as_mut().unwrap().session = "session-two".into();
        assert!(store.replace_source("socket", 2, [&changed]));
        assert_ne!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        changed.pane_id = "replacement".into();
        assert!(store.replace_source("socket", 2, [&changed]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].outcome,
            None
        );
    }

    #[test]
    fn missing_authoritative_report_invalidates_success_without_raw_status_change() {
        for status in [AgentStatus::Idle, AgentStatus::Done] {
            for authoritative in [false, true] {
                let mut store = SessionStore::new();
                assert!(store.begin_source("socket", 1));
                let mut row = reported("terminal", "1", AgentOutcome::Succeeded);
                row.status = status;
                assert!(store.replace_source("socket", 1, [&row]));
                assert!(store.accept_outcome(
                    "socket",
                    1,
                    "terminal",
                    "terminal",
                    AgentOutcome::Succeeded,
                    123
                ));
                assert_eq!(
                    store
                        .snapshot(&SessionListOptions::default(), None)
                        .status_summary
                        .status,
                    DisplayStatus::Succeeded
                );

                row.outcome_authoritative = authoritative;
                row.outcome = None;
                let revision = store.revision();
                assert!(store.replace_source("socket", 1, [&row]));
                assert_eq!(store.revision(), revision + 1);
                let snapshot =
                    store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
                assert_eq!(snapshot.rows[0].outcome, None);
                assert_eq!(snapshot.rows[0].status, status);
                assert_eq!(
                    snapshot.status_summary.status,
                    if status == AgentStatus::Idle {
                        DisplayStatus::Idle
                    } else {
                        DisplayStatus::Completed
                    }
                );
                assert!(!store.accept_outcome(
                    "socket",
                    1,
                    "terminal",
                    "terminal",
                    AgentOutcome::Succeeded,
                    123
                ));
                assert!(store.replace_source("socket", 1, [&row]));
                assert_eq!(store.revision(), revision + 1);
            }
        }
    }

    #[test]
    fn contradictory_same_turn_report_invalidates_success() {
        for contradict_outcome in [false, true] {
            let mut store = SessionStore::new();
            assert!(store.begin_source("socket", 1));
            let mut row = reported("terminal", "1", AgentOutcome::Succeeded);
            assert!(store.replace_source("socket", 1, [&row]));
            assert!(store.accept_outcome(
                "socket",
                1,
                "terminal",
                "terminal",
                AgentOutcome::Succeeded,
                123
            ));
            if contradict_outcome {
                row.outcome.as_mut().unwrap().outcome = AgentOutcome::Failed;
            } else {
                row.outcome.as_mut().unwrap().at_unix_ms = 124;
            }
            assert!(store.replace_source("socket", 1, [&row]));
            let snapshot =
                store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
            assert_eq!(snapshot.rows[0].outcome, None);
            assert_eq!(snapshot.status_summary.status, DisplayStatus::Completed);
            assert!(!store.accept_outcome(
                "socket",
                1,
                "terminal",
                "terminal",
                AgentOutcome::Succeeded,
                123
            ));
        }
    }

    #[test]
    fn repeated_accepted_report_preserves_outcome_and_revision() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = reported("terminal", "1", AgentOutcome::Succeeded);
        assert!(store.replace_source("socket", 1, [&row]));
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let revision = store.revision();
        assert!(store.replace_source("socket", 1, [&row]));
        assert_eq!(store.revision(), revision);
        let snapshot = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(snapshot.rows[0].outcome, Some(AgentOutcome::Succeeded));
        assert_eq!(snapshot.status_summary.status, DisplayStatus::Succeeded);
    }

    #[test]
    fn full_source_summary_ignores_filter_and_row_cap_but_requires_all_live_success() {
        let mut store = SessionStore::new();
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::NoSessions
        );
        assert!(store.begin_source("socket", 1));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::Offline
        );
        let records: Vec<_> = (0..150)
            .map(|index| {
                reported(
                    &format!("terminal-{index:03}"),
                    "1",
                    AgentOutcome::Succeeded,
                )
            })
            .collect();
        assert!(store.replace_source("socket", 1, records.iter()));
        for row in &records {
            assert!(store.accept_outcome(
                "socket",
                1,
                &row.terminal_id,
                &row.pane_id,
                AgentOutcome::Succeeded,
                123
            ));
        }
        let filtered = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Working),
            None,
        );
        assert!(filtered.rows.is_empty());
        assert_eq!(
            filtered.status_summary,
            SessionStatusSummary {
                status: DisplayStatus::Succeeded,
                count: 150,
                total: 150
            }
        );
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .rows
                .len(),
            MAX_ROWS
        );
        assert!(store.begin_source("other", 1));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary,
            SessionStatusSummary {
                status: DisplayStatus::Offline,
                count: 0,
                total: 150
            }
        );
        assert!(store.replace_source("other", 1, std::iter::empty::<&AgentRecord>()));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::Succeeded
        );
        let idle = record("idle", "idle", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, records.iter().chain([&idle])));
        assert_ne!(
            store
                .snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Working),
                    None
                )
                .status_summary
                .status,
            DisplayStatus::Succeeded
        );
        assert!(store.mark_offline("other", 1));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::Offline
        );
    }

    #[test]
    fn same_terminal_ids_are_isolated_by_opaque_source_identity() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("/private/one.sock", 1));
        assert!(store.begin_source("/private/two.sock", 1));
        let first = record("terminal", "pane-one", AgentStatus::Working);
        let second = record("terminal", "pane-two", AgentStatus::Done);
        assert!(store.replace_source("/private/one.sock", 1, [&first]));
        assert!(store.replace_source("/private/two.sock", 1, [&second]));

        let snapshot = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(snapshot.rows.len(), 2);
        assert_ne!(
            snapshot.rows[0].key.source_id,
            snapshot.rows[1].key.source_id
        );
        assert_eq!(snapshot.rows[0].key.terminal_id, "terminal");
        assert_eq!(snapshot.rows[0].pane_id, "pane-one");
        assert_eq!(snapshot.rows[1].pane_id, "pane-two");
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("/private/one.sock"));
        assert!(!debug.contains("/private/two.sock"));
    }

    #[test]
    fn stale_generation_callbacks_are_rejected_and_new_rows_start_offline() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let old = record("terminal", "pane", AgentStatus::Working);
        assert!(store.replace_source("socket", 1, [&old]));
        assert!(store.begin_source("socket", 2));
        let after_begin =
            store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(after_begin.rows[0].availability, Availability::Offline);
        assert_eq!(after_begin.rows[0].key.generation, 2);
        assert!(!store.replace_source("socket", 1, [&old]));
        assert!(!store.update_status("socket", 1, "terminal", "pane", AgentStatus::Done));
        assert!(!store.mark_offline("socket", 1));
        assert!(!store.remove_source("socket", 1));
        assert!(store.replace_source("socket", 2, std::iter::empty::<&AgentRecord>(),));
        assert!(store
            .snapshot(&SessionListOptions::default(), None)
            .rows
            .is_empty());
    }

    #[test]
    fn offline_reconnect_empty_and_removal_preserve_then_clear_rows() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let cached = record("terminal", "pane", AgentStatus::Done);
        assert!(store.replace_source("socket", 1, [&cached]));
        assert!(store.mark_offline("socket", 1));
        let offline = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(offline.rows[0].status, AgentStatus::Done);
        assert_eq!(
            store
                .snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Completed),
                    None
                )
                .matched,
            0
        );
        assert_eq!(
            store
                .snapshot(
                    &SessionListOptions::with_filter(SessionFilter::Offline),
                    None
                )
                .matched,
            1
        );

        assert!(store.begin_source("socket", 2));
        let reconnecting =
            store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(reconnecting.rows[0].availability, Availability::Offline);
        assert!(store.replace_source("socket", 2, std::iter::empty::<&AgentRecord>(),));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).total,
            0
        );
        assert!(store.replace_source("socket", 2, [&cached]));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].availability,
            Availability::Live
        );
        assert!(store.remove_source("socket", 2));
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).total,
            0
        );
    }

    #[test]
    fn filtering_counts_before_cap_and_reports_omitted_rows() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let records: Vec<AgentRecord> = (0..200)
            .map(|index| {
                let status = if index < 150 {
                    AgentStatus::Idle
                } else {
                    AgentStatus::Working
                };
                record(&format!("terminal-{index:03}"), "pane", status)
            })
            .collect();
        assert!(store.replace_source("socket", 1, records.iter()));

        let all = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(all.total, 200);
        assert_eq!(all.matched, 200);
        assert_eq!(all.rows.len(), 128);
        assert_eq!(all.omitted, 72);
        let working = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Working),
            None,
        );
        assert_eq!(working.matched, 50);
        assert_eq!(working.rows.len(), 50);
        assert_eq!(working.omitted, 0);
    }

    #[test]
    fn complete_pages_reach_every_row_beyond_gui_cap_in_source_order() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("first", 1));
        assert!(store.begin_source("second", 1));
        assert!(store.begin_source("hidden", 1));
        let first: Vec<_> = (0..129)
            .map(|index| record(&format!("terminal-{index:03}"), "first", AgentStatus::Idle))
            .collect();
        let second = [
            record("a", "second", AgentStatus::Idle),
            record("b", "second", AgentStatus::Idle),
        ];
        let hidden = record("invisible", "hidden", AgentStatus::Idle);
        assert!(store.replace_source("first", 1, first.iter()));
        assert!(store.replace_source("second", 1, second.iter()));
        assert!(store.replace_source("hidden", 1, [&hidden]));
        store.set_visibility("hidden", false);
        let gui = store.snapshot(&SessionListOptions::default(), None);
        assert_eq!(gui.rows.len(), 128);
        assert_eq!(gui.omitted, 3);

        let mut cursor = None;
        let mut all_rows = Vec::new();
        loop {
            let page = store
                .page(SessionFilter::All, gui.revision, cursor.as_ref(), 17)
                .unwrap();
            assert_eq!(page.total, 131);
            assert_eq!(page.matched, 131);
            assert_eq!(page.status_summary, gui.status_summary);
            assert!(page.rows.len() <= 17);
            all_rows.extend(page.rows);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(all_rows.len(), 131);
        assert_eq!(&all_rows[..128], gui.rows.as_slice());
        assert_eq!(all_rows[128].key.terminal_id, "terminal-128");
        assert_eq!(all_rows[129].key.terminal_id, "a");
        assert!(all_rows[128].key.source_id < all_rows[129].key.source_id);
        assert_eq!(all_rows[130].key.terminal_id, "b");

        let boundary = store
            .page(SessionFilter::All, gui.revision, None, MAX_PAGE_ROWS)
            .unwrap();
        let last_first_source = store
            .page(
                SessionFilter::All,
                gui.revision,
                boundary.next_cursor.as_ref(),
                1,
            )
            .unwrap();
        assert_eq!(last_first_source.rows, all_rows[128..129]);
        let next = store
            .page(
                SessionFilter::All,
                gui.revision,
                last_first_source.next_cursor.as_ref(),
                2,
            )
            .unwrap();
        assert_eq!(next.rows, all_rows[129..]);
        assert_eq!(next.next_cursor, None);
    }

    #[test]
    fn complete_pages_preserve_filter_and_unfiltered_summary() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("live", 1));
        assert!(store.begin_source("offline", 1));
        let live: Vec<_> = (0..130)
            .map(|index| {
                record(
                    &format!("terminal-{index:03}"),
                    "live",
                    if index % 2 == 0 {
                        AgentStatus::Working
                    } else {
                        AgentStatus::Idle
                    },
                )
            })
            .collect();
        let cached = record("cached", "offline", AgentStatus::Working);
        assert!(store.replace_source("live", 1, live.iter()));
        assert!(store.replace_source("offline", 1, [&cached]));
        assert!(store.mark_offline("offline", 1));
        let revision = store.revision();
        let gui = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Working),
            None,
        );
        let first = store
            .page(SessionFilter::Working, revision, None, 32)
            .unwrap();
        let second = store
            .page(
                SessionFilter::Working,
                revision,
                first.next_cursor.as_ref(),
                33,
            )
            .unwrap();
        assert_eq!(first.total, 131);
        assert_eq!(first.matched, 65);
        assert_eq!(first.status_summary, gui.status_summary);
        assert_eq!(first.status_summary.status, DisplayStatus::Running);
        assert_eq!(first.status_summary.count, 65);
        assert_eq!(second.status_summary, first.status_summary);
        assert_eq!(second.matched, first.matched);
        assert_eq!(second.next_cursor, None);
        let mut rows = first.rows;
        rows.extend(second.rows);
        assert_eq!(rows, gui.rows);
        assert!(rows.iter().all(|row| row.status == AgentStatus::Working));
        let offline = store
            .page(SessionFilter::Offline, revision, None, 1)
            .unwrap();
        assert_eq!(offline.matched, 1);
        assert_eq!(offline.rows[0].key.terminal_id, "cached");
        assert_eq!(offline.next_cursor, None);
    }

    #[test]
    fn complete_pages_reject_stale_revision_and_invalid_limits() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = record("one", "pane", AgentStatus::Idle);
        let other = record("two", "other-pane", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, [&row, &other]));
        let revision = store.revision();
        assert_eq!(
            store.page(SessionFilter::All, revision, None, 0),
            Err(SessionPageError::InvalidLimit)
        );
        assert_eq!(
            store.page(SessionFilter::All, revision, None, MAX_PAGE_ROWS + 1),
            Err(SessionPageError::InvalidLimit)
        );
        let first = store.page(SessionFilter::All, revision, None, 1).unwrap();
        assert!(first.next_cursor.is_some());
        assert!(store.update_status("socket", 1, "two", "other-pane", AgentStatus::Working));
        assert_eq!(
            store.page(SessionFilter::All, revision, first.next_cursor.as_ref(), 1),
            Err(SessionPageError::StaleRevision)
        );
    }

    #[test]
    fn displayed_key_observes_filtered_row_past_all_cap_through_outcome_and_invalidation() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut records: Vec<_> = (0..128)
            .map(|index| {
                record(
                    &format!("idle-{index:03}"),
                    "shared-pane",
                    AgentStatus::Idle,
                )
            })
            .collect();
        records.push(record("working", "working-pane", AgentStatus::Working));
        assert!(store.replace_source("socket", 1, records.iter()));
        let filtered = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Working),
            None,
        );
        let key = filtered.rows[0].key.clone();
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .rows
                .len(),
            128
        );
        assert!(!store
            .snapshot(&SessionListOptions::default(), None)
            .rows
            .iter()
            .any(|row| row.key == key));
        assert_eq!(store.view_for_key(&key), Some(filtered.rows[0].clone()));
        assert_eq!(
            store.view_for_key(&key).unwrap().display_status(),
            DisplayStatus::Running
        );
        assert!(store.update_status("socket", 1, "working", "working-pane", AgentStatus::Done));
        assert!(store
            .snapshot(
                &SessionListOptions::with_filter(SessionFilter::Working),
                None
            )
            .rows
            .is_empty());
        let completed = store.view_for_key(&key).unwrap();
        assert_eq!(completed.status, AgentStatus::Done);
        assert_eq!(completed.display_status(), DisplayStatus::Completed);

        let mut result = reported("working", "turn-1", AgentOutcome::Succeeded);
        result.pane_id = "working-pane".into();
        records[128] = result;
        assert!(store.replace_source("socket", 1, records.iter()));
        assert!(store.accept_outcome(
            "socket",
            1,
            "working",
            "working-pane",
            AgentOutcome::Succeeded,
            123
        ));
        let accepted = store.view_for_key(&key).unwrap();
        assert_eq!(accepted.outcome, Some(AgentOutcome::Succeeded));
        assert_eq!(accepted.display_status(), DisplayStatus::Succeeded);
        assert_eq!(store.prompt_target(&key).unwrap().pane_id, "working-pane");

        assert!(store.mark_offline("socket", 1));
        let offline = store.view_for_key(&key).unwrap();
        assert_eq!(offline.availability, Availability::Offline);
        assert_eq!(offline.display_status(), DisplayStatus::Offline);
        assert_eq!(store.prompt_target(&key), Err(PromptTargetError::Offline));
        store.set_visibility("socket", false);
        assert_eq!(store.view_for_key(&key), None);
        store.set_visibility("socket", true);
        assert_eq!(
            store.view_for_key(&key).unwrap().display_status(),
            DisplayStatus::Offline
        );
        assert!(store.begin_source("socket", 2));
        assert_eq!(store.view_for_key(&key), None);
        let new_key = SessionKey {
            generation: 2,
            ..key
        };
        assert_eq!(
            store.view_for_key(&new_key).unwrap().display_status(),
            DisplayStatus::Offline
        );
    }

    #[test]
    fn display_lookup_keeps_remote_and_ambiguous_panes_without_prompt_permission() {
        let mut store = SessionStore::new();
        let remote = crate::sources::remote_source("east");
        assert!(store.begin_source(&remote, 1));
        let remote_row = record("remote", "remote-pane", AgentStatus::Working);
        assert!(store.replace_source(&remote, 1, [&remote_row]));
        let remote_key = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert_eq!(
            store.view_for_key(&remote_key).unwrap().status,
            AgentStatus::Working
        );
        assert_eq!(
            store.prompt_target(&remote_key),
            Err(PromptTargetError::ReadOnly)
        );

        assert!(store.begin_source("socket", 1));
        let first = record("first", "shared-pane", AgentStatus::Idle);
        let second = record("second", "shared-pane", AgentStatus::Done);
        assert!(store.replace_source("socket", 1, [&first, &second]));
        let local = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        let ambiguous = local
            .rows
            .iter()
            .find(|row| row.key.terminal_id == "second")
            .unwrap();
        assert_eq!(store.view_for_key(&ambiguous.key), Some(ambiguous.clone()));
        assert_eq!(
            store.prompt_target(&ambiguous.key),
            Err(PromptTargetError::Stale)
        );
        assert!(store.remove_source("socket", 1));
        assert_eq!(store.view_for_key(&ambiguous.key), None);
    }

    #[test]
    fn worktree_target_keeps_card_identity_and_rejects_ambiguous_or_stale_rows() {
        let info = Arc::new(WorkspaceWorktreeInfo {
            repo_key: "repo".into(),
            repo_name: "Project".into(),
            repo_root: "/project".into(),
            checkout_path: "/linked".into(),
            is_linked_worktree: true,
            pane_count: 2,
            tab_count: 2,
        });
        let mut store = SessionStore::new();
        assert!(store.begin_source("/local.sock", 1));
        let mut first = record("first", "pane-one", AgentStatus::Working);
        first.metadata.workspace_id = Some("w".into());
        first.metadata.worktree = Some(Arc::clone(&info));
        let mut second = record("second", "pane-two", AgentStatus::Working);
        second.metadata = first.metadata.clone();
        assert!(store.replace_source("/local.sock", 1, [&first, &second]));
        let rows = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        let key = rows[1].key.clone();
        let target = store.worktree_remove_target(&key).unwrap();
        assert_eq!(target.key, key);
        assert_eq!(target.pane_id, "pane-two");
        assert_eq!(target.workspace_id, "w");
        assert_eq!(target.worktree.as_ref(), info.as_ref());

        let mut missing = second.clone();
        missing.metadata.worktree = None;
        assert!(store.replace_source("/local.sock", 1, [&first, &missing]));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::NotLinkedWorktree)
        );

        let mut main = second.clone();
        main.metadata.worktree = Some(Arc::new(WorkspaceWorktreeInfo {
            is_linked_worktree: false,
            ..(*info).clone()
        }));
        assert!(store.replace_source("/local.sock", 1, [&first, &main]));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::NotLinkedWorktree)
        );
        let mut falsely_linked_root = second.clone();
        falsely_linked_root.metadata.worktree = Some(Arc::new(WorkspaceWorktreeInfo {
            checkout_path: info.repo_root.clone(),
            ..(*info).clone()
        }));
        assert!(store.replace_source("/local.sock", 1, [&first, &falsely_linked_root]));
        assert!(store.view_for_key(&key).is_some());
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::NotLinkedWorktree)
        );
        assert!(store.replace_source("/local.sock", 1, [&first, &second]));
        assert!(store.worktree_remove_target(&key).is_ok());
        let mut changed = second.clone();
        changed.metadata.worktree = Some(Arc::new(WorkspaceWorktreeInfo {
            tab_count: 3,
            ..(*info).clone()
        }));
        assert!(store.replace_source("/local.sock", 1, [&first, &changed]));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Stale)
        );
        changed.pane_id = "pane-one".into();
        assert!(store.replace_source("/local.sock", 1, [&first, &changed]));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Stale)
        );
        assert!(store.mark_offline("/local.sock", 1));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Offline)
        );
        assert!(store.begin_source("/local.sock", 2));
        assert_eq!(
            store.worktree_remove_target(&key),
            Err(WorktreeRemoveTargetError::Stale)
        );
        let remote = crate::sources::remote_source("east");
        assert!(store.begin_source(&remote, 1));
        assert!(store.replace_source(&remote, 1, [&first]));
        let remote_key = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows
            .into_iter()
            .find(|row| row.key.terminal_id == "first" && row.key.source_id != key.source_id)
            .unwrap()
            .key;
        assert_eq!(
            store.worktree_remove_target(&remote_key),
            Err(WorktreeRemoveTargetError::ReadOnly)
        );
    }

    #[test]
    fn selection_survives_status_changes_but_not_generation_changes() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let initial = record("terminal", "pane", AgentStatus::Working);
        assert!(store.replace_source("socket", 1, [&initial]));
        let key = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert!(store.update_status("socket", 1, "terminal", "pane", AgentStatus::Done));
        let changed = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Working),
            Some(&key),
        );
        assert_eq!(changed.selected, Some(key.clone()));
        assert!(changed.rows.is_empty());
        assert!(store.begin_source("socket", 2));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), Some(&key))
                .selected,
            None
        );
    }

    #[test]
    fn rows_are_sorted_by_source_ordinal_then_terminal_id() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("first", 1));
        assert!(store.begin_source("second", 1));
        let first_b = record("b", "pane-b", AgentStatus::Idle);
        let first_a = record("a", "pane-a", AgentStatus::Idle);
        let second_a = record("a", "pane-c", AgentStatus::Idle);
        assert!(store.replace_source("first", 1, [&first_b, &first_a]));
        assert!(store.replace_source("second", 1, [&second_a]));
        let rows = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert_eq!(rows[0].pane_id, "pane-a");
        assert_eq!(rows[1].pane_id, "pane-b");
        assert_eq!(rows[2].pane_id, "pane-c");
        assert!(rows[0].key.source_id < rows[2].key.source_id);
    }

    #[test]
    fn duplicate_publication_does_not_advance_revision() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = record("terminal", "pane", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, [&row]));
        let revision = store.revision();
        assert!(store.replace_source("socket", 1, [&row]));
        assert_eq!(store.revision(), revision);
        assert!(store.update_status("socket", 1, "terminal", "pane", AgentStatus::Idle));
        assert_eq!(store.revision(), revision);
        assert!(store.mark_offline("socket", 1));
        let offline_revision = store.revision();
        assert!(store.mark_offline("socket", 1));
        assert_eq!(store.revision(), offline_revision);
    }

    #[test]
    fn title_change_repaints_without_changing_key_order_or_selection() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut first = record("a", "pane-a", AgentStatus::Working);
        first.metadata.title = Some("Initial task".to_owned());
        let second = record("b", "pane-b", AgentStatus::Working);
        assert!(store.replace_source("socket", 1, [&second, &first]));
        let initial = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        let selected = initial.rows[0].key.clone();
        let keys: Vec<_> = initial.rows.iter().map(|row| row.key.clone()).collect();

        let mut renamed = first.clone();
        renamed.metadata.title = Some("Renamed task".to_owned());
        assert!(store.replace_source("socket", 1, [&renamed, &second]));
        let changed = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::All),
            Some(&selected),
        );
        assert_eq!(changed.revision, initial.revision + 1);
        assert_eq!(changed.selected, Some(selected));
        assert_eq!(
            changed
                .rows
                .iter()
                .map(|row| row.key.clone())
                .collect::<Vec<_>>(),
            keys
        );
        assert_eq!(
            changed.rows[0].metadata.title.as_deref(),
            Some("Renamed task")
        );
        assert_eq!(changed.rows[0].status, AgentStatus::Working);

        let same_metadata = renamed.clone();
        assert!(store.replace_source("socket", 1, [&second, &same_metadata]));
        assert_eq!(store.revision(), changed.revision);
    }

    #[test]
    fn offline_title_survives_and_old_generation_cannot_replace_it() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut cached = record("terminal", "pane", AgentStatus::Working);
        cached.metadata.title = Some("Cached task".to_owned());
        assert!(store.replace_source("socket", 1, [&cached]));
        assert!(store.mark_offline("socket", 1));
        let offline = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Offline),
            None,
        );
        assert_eq!(
            offline.rows[0].metadata.title.as_deref(),
            Some("Cached task")
        );

        assert!(store.begin_source("socket", 2));
        let reconnecting = store.snapshot(
            &SessionListOptions::with_filter(SessionFilter::Offline),
            None,
        );
        assert_eq!(
            reconnecting.rows[0].metadata.title.as_deref(),
            Some("Cached task")
        );
        assert_eq!(reconnecting.rows[0].key.generation, 2);
        let mut stale = cached.clone();
        stale.metadata.title = Some("Stale task".to_owned());
        assert!(!store.replace_source("socket", 1, [&stale]));
        assert_eq!(store.revision(), reconnecting.revision);

        let mut current = cached.clone();
        current.metadata.title = Some("Current task".to_owned());
        assert!(store.replace_source("socket", 2, [&current]));
        let live = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(live.rows[0].metadata.title.as_deref(), Some("Current task"));
        assert!(!store.replace_source("socket", 1, [&stale]));
        assert_eq!(store.snapshot(&SessionListOptions::default(), None), live);
    }

    #[test]
    fn generation_change_invalidates_selection_even_when_already_offline() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = record("terminal", "pane", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, [&row]));
        let key = store
            .snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows[0]
            .key
            .clone();
        assert!(store.mark_offline("socket", 1));
        let revision = store.revision();
        assert!(store.begin_source("socket", 2));
        assert!(store.revision() > revision);
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), Some(&key))
                .selected,
            None
        );
    }

    #[test]
    fn accepted_terminal_result_survives_idle_in_either_arrival_order() {
        for (outcome, display) in [
            (AgentOutcome::Succeeded, DisplayStatus::Succeeded),
            (AgentOutcome::Failed, DisplayStatus::Failed),
            (AgentOutcome::Cancelled, DisplayStatus::Cancelled),
        ] {
            for event_first in [true, false] {
                let mut store = SessionStore::new();
                assert!(store.begin_source("socket", 1));
                let row = reported("terminal", "1", outcome);
                assert!(store.replace_source("socket", 1, [&row]));
                let before_accept = store.revision();
                assert!(store.accept_outcome("socket", 1, "terminal", "terminal", outcome, 123));
                assert_eq!(store.revision(), before_accept + 1);

                let mut idle = row.clone();
                idle.status = AgentStatus::Idle;
                if event_first {
                    assert!(store.update_status(
                        "socket",
                        1,
                        "terminal",
                        "terminal",
                        AgentStatus::Idle
                    ));
                    let after_event = store.revision();
                    assert_eq!(after_event, before_accept + 2);
                    assert!(store.replace_source("socket", 1, [&idle]));
                    assert_eq!(store.revision(), after_event);
                } else {
                    assert!(store.replace_source("socket", 1, [&idle]));
                    let after_snapshot = store.revision();
                    assert_eq!(after_snapshot, before_accept + 2);
                    assert!(store.update_status(
                        "socket",
                        1,
                        "terminal",
                        "terminal",
                        AgentStatus::Idle
                    ));
                    assert_eq!(store.revision(), after_snapshot);
                }
                let revision = store.revision();
                assert!(store.replace_source("socket", 1, [&idle]));
                assert!(store.update_status(
                    "socket",
                    1,
                    "terminal",
                    "terminal",
                    AgentStatus::Idle
                ));
                assert!(store.accept_outcome("socket", 1, "terminal", "terminal", outcome, 123));
                assert_eq!(store.revision(), revision);
                let snapshot =
                    store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
                assert_eq!(snapshot.rows[0].status, AgentStatus::Idle);
                assert_eq!(snapshot.rows[0].display_status(), display);
                assert_eq!(snapshot.status_summary.status, display);
            }
        }
    }

    #[test]
    fn idle_without_acceptance_is_neutral_and_activity_event_revokes_same_status() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let mut row = reported("terminal", "1", AgentOutcome::Succeeded);
        row.status = AgentStatus::Idle;
        assert!(store.replace_source("socket", 1, [&row]));
        let snapshot = store.snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(snapshot.rows[0].outcome, None);
        assert_eq!(snapshot.status_summary.status, DisplayStatus::Idle);
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Working));
        // A lagging authoritative terminal report may coexist with a Working
        // status; a new activity event must revoke it even if status is unchanged.
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let revision = store.revision();
        assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Working));
        assert_eq!(store.revision(), revision + 1);
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Running
        );
        let revision = store.revision();
        assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Working));
        assert_eq!(store.revision(), revision);
        assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Blocked));
        assert!(store.accept_outcome(
            "socket",
            1,
            "terminal",
            "terminal",
            AgentOutcome::Succeeded,
            123
        ));
        let revision = store.revision();
        assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Blocked));
        assert_eq!(store.revision(), revision + 1);
        assert_eq!(
            store.snapshot(&SessionListOptions::default(), None).rows[0].display_status(),
            DisplayStatus::Waiting
        );
    }

    #[test]
    fn new_source_requires_coherent_empty_snapshot_before_idle() {
        let mut store = SessionStore::new();
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::NoSessions
        );
        assert!(store.begin_source("socket", 1));
        let before_snapshot = store.revision();
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary,
            SessionStatusSummary {
                status: DisplayStatus::Offline,
                count: 0,
                total: 0
            }
        );
        assert!(store.begin_source("socket", 1));
        assert!(!store.replace_source("socket", 2, std::iter::empty::<&AgentRecord>()));
        assert!(!store.mark_offline("socket", 2));
        assert_eq!(store.revision(), before_snapshot);
        assert!(store.replace_source("socket", 1, std::iter::empty::<&AgentRecord>()));
        assert_eq!(store.revision(), before_snapshot + 1);
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::Idle
        );
        assert!(store.replace_source("socket", 1, std::iter::empty::<&AgentRecord>()));
        assert_eq!(store.revision(), before_snapshot + 1);
        assert!(store.mark_offline("socket", 1));
        assert_eq!(
            store
                .snapshot(&SessionListOptions::default(), None)
                .status_summary
                .status,
            DisplayStatus::Offline
        );
        let disconnected_revision = store.revision();
        assert!(store.mark_offline("socket", 1));
        assert_eq!(store.revision(), disconnected_revision);
    }
}
