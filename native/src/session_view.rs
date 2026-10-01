use crate::agent_outcome::{AgentOutcome, OutcomeReport};
use crate::herdr_protocol::{AgentRecord, AgentStatus, SessionMetadata};
use std::collections::{BTreeMap, HashSet};

const MAX_SOURCES: usize = 64;
const MAX_RECORDS_PER_SOURCE: usize = crate::herdr_protocol::MAX_AGENT_RECORDS;
const MAX_ROWS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct SessionKey {
    pub(crate) source_id: u64,
    pub(crate) generation: u64,
    pub(crate) terminal_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Availability {
    Live,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
    pub(crate) status_summary: SessionStatusSummary,
    pub(crate) total: usize,
    pub(crate) matched: usize,
    pub(crate) omitted: usize,
    pub(crate) selected: Option<SessionKey>,
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

    pub(crate) fn snapshot(
        &self,
        filter: SessionFilter,
        selected: Option<&SessionKey>,
    ) -> SessionSnapshot {
        let mut total = 0usize;
        let mut matched = 0usize;
        let mut rows = Vec::with_capacity(MAX_ROWS);

        let mut buckets = [0usize; 10];
        let mut included_sources = 0usize;
        let mut disconnected = false;
        // Sources are kept in monotonically assigned source-id order and each
        // record map is ordered by terminal ID. This streams deterministic rows
        // without collecting or sorting all retained records.
        for source in &self.sources {
            if !source.visible {
                continue;
            }
            included_sources += 1;
            disconnected |= source.availability == Availability::Offline;
            total = total.saturating_add(source.records.len());
            for (terminal_id, record) in &source.records {
                let availability = source.availability;
                let bucket = display_status(availability, record.status, record.outcome);
                let index = match bucket {
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
                buckets[index] += 1;
                if !filter_matches(filter, availability, record.status) {
                    continue;
                }
                matched = matched.saturating_add(1);
                if rows.len() < MAX_ROWS {
                    rows.push(SessionView {
                        key: SessionKey {
                            source_id: source.source_id,
                            generation: source.generation,
                            terminal_id: terminal_id.clone(),
                        },
                        source_label: source.label.clone(),
                        is_local: crate::sources::remote_machine_id(&source.source).is_none(),
                        pane_id: record.pane_id.clone(),
                        status: record.status,
                        outcome: record.outcome,
                        metadata: record.metadata.clone(),
                        availability,
                    });
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
        let selected = selected.and_then(|key| self.contains_key(key).then(|| key.clone()));
        let omitted = matched.saturating_sub(rows.len());
        SessionSnapshot {
            revision: self.revision,
            rows,
            status_summary,
            total,
            matched,
            omitted,
            selected,
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
                store.snapshot(SessionFilter::All, None).rows[0].display_status(),
                DisplayStatus::Completed
            );
            let revision = store.revision();
            assert!(store.accept_outcome("socket", 1, "terminal", "terminal", outcome, 123));
            assert!(store.revision() > revision);
            assert_eq!(
                store.snapshot(SessionFilter::All, None).rows[0].display_status(),
                display
            );
            assert!(store.update_status("socket", 1, "terminal", "terminal", AgentStatus::Working));
            assert_eq!(
                store.snapshot(SessionFilter::All, None).rows[0].display_status(),
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Running
        );
        assert!(store.replace_source("socket", 1, [&row]));
        assert_ne!(
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        let mut new_work = reported("terminal", "5", AgentOutcome::Running);
        new_work.status = AgentStatus::Working;
        assert!(store.replace_source("socket", 1, [&new_work]));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Running
        );
        assert!(store.replace_source("socket", 1, [&lagging]));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        assert!(store.mark_offline("socket", 1));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Offline
        );
        assert!(store.begin_source("socket", 2));
        assert!(store.replace_source("socket", 2, [&row]));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Succeeded
        );
        changed.pane_id = "replacement".into();
        assert!(store.replace_source("socket", 2, [&changed]));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].outcome,
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
                        .snapshot(SessionFilter::All, None)
                        .status_summary
                        .status,
                    DisplayStatus::Succeeded
                );

                row.outcome_authoritative = authoritative;
                row.outcome = None;
                let revision = store.revision();
                assert!(store.replace_source("socket", 1, [&row]));
                assert_eq!(store.revision(), revision + 1);
                let snapshot = store.snapshot(SessionFilter::All, None);
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
            let snapshot = store.snapshot(SessionFilter::All, None);
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
        let snapshot = store.snapshot(SessionFilter::All, None);
        assert_eq!(snapshot.rows[0].outcome, Some(AgentOutcome::Succeeded));
        assert_eq!(snapshot.status_summary.status, DisplayStatus::Succeeded);
    }

    #[test]
    fn full_source_summary_ignores_filter_and_row_cap_but_requires_all_live_success() {
        let mut store = SessionStore::new();
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
                .status_summary
                .status,
            DisplayStatus::NoSessions
        );
        assert!(store.begin_source("socket", 1));
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
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
        let filtered = store.snapshot(SessionFilter::Working, None);
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
            store.snapshot(SessionFilter::All, None).rows.len(),
            MAX_ROWS
        );
        assert!(store.begin_source("other", 1));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).status_summary,
            SessionStatusSummary {
                status: DisplayStatus::Offline,
                count: 0,
                total: 150
            }
        );
        assert!(store.replace_source("other", 1, std::iter::empty::<&AgentRecord>()));
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
                .status_summary
                .status,
            DisplayStatus::Succeeded
        );
        let idle = record("idle", "idle", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, records.iter().chain([&idle])));
        assert_ne!(
            store
                .snapshot(SessionFilter::Working, None)
                .status_summary
                .status,
            DisplayStatus::Succeeded
        );
        assert!(store.mark_offline("other", 1));
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
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

        let snapshot = store.snapshot(SessionFilter::All, None);
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
        let after_begin = store.snapshot(SessionFilter::All, None);
        assert_eq!(after_begin.rows[0].availability, Availability::Offline);
        assert_eq!(after_begin.rows[0].key.generation, 2);
        assert!(!store.replace_source("socket", 1, [&old]));
        assert!(!store.update_status("socket", 1, "terminal", "pane", AgentStatus::Done));
        assert!(!store.mark_offline("socket", 1));
        assert!(!store.remove_source("socket", 1));
        assert!(store.replace_source("socket", 2, std::iter::empty::<&AgentRecord>(),));
        assert!(store.snapshot(SessionFilter::All, None).rows.is_empty());
    }

    #[test]
    fn offline_reconnect_empty_and_removal_preserve_then_clear_rows() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let cached = record("terminal", "pane", AgentStatus::Done);
        assert!(store.replace_source("socket", 1, [&cached]));
        assert!(store.mark_offline("socket", 1));
        let offline = store.snapshot(SessionFilter::All, None);
        assert_eq!(offline.rows[0].status, AgentStatus::Done);
        assert_eq!(store.snapshot(SessionFilter::Completed, None).matched, 0);
        assert_eq!(store.snapshot(SessionFilter::Offline, None).matched, 1);

        assert!(store.begin_source("socket", 2));
        let reconnecting = store.snapshot(SessionFilter::All, None);
        assert_eq!(reconnecting.rows[0].availability, Availability::Offline);
        assert!(store.replace_source("socket", 2, std::iter::empty::<&AgentRecord>(),));
        assert_eq!(store.snapshot(SessionFilter::All, None).total, 0);
        assert!(store.replace_source("socket", 2, [&cached]));
        assert_eq!(
            store.snapshot(SessionFilter::All, None).rows[0].availability,
            Availability::Live
        );
        assert!(store.remove_source("socket", 2));
        assert_eq!(store.snapshot(SessionFilter::All, None).total, 0);
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

        let all = store.snapshot(SessionFilter::All, None);
        assert_eq!(all.total, 200);
        assert_eq!(all.matched, 200);
        assert_eq!(all.rows.len(), 128);
        assert_eq!(all.omitted, 72);
        let working = store.snapshot(SessionFilter::Working, None);
        assert_eq!(working.matched, 50);
        assert_eq!(working.rows.len(), 50);
        assert_eq!(working.omitted, 0);
    }

    #[test]
    fn selection_survives_status_changes_but_not_generation_changes() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let initial = record("terminal", "pane", AgentStatus::Working);
        assert!(store.replace_source("socket", 1, [&initial]));
        let key = store.snapshot(SessionFilter::All, None).rows[0].key.clone();
        assert!(store.update_status("socket", 1, "terminal", "pane", AgentStatus::Done));
        let changed = store.snapshot(SessionFilter::Working, Some(&key));
        assert_eq!(changed.selected, Some(key.clone()));
        assert!(changed.rows.is_empty());
        assert!(store.begin_source("socket", 2));
        assert_eq!(
            store.snapshot(SessionFilter::All, Some(&key)).selected,
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
        let rows = store.snapshot(SessionFilter::All, None).rows;
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
        let initial = store.snapshot(SessionFilter::All, None);
        let selected = initial.rows[0].key.clone();
        let keys: Vec<_> = initial.rows.iter().map(|row| row.key.clone()).collect();

        let mut renamed = first.clone();
        renamed.metadata.title = Some("Renamed task".to_owned());
        assert!(store.replace_source("socket", 1, [&renamed, &second]));
        let changed = store.snapshot(SessionFilter::All, Some(&selected));
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
        let offline = store.snapshot(SessionFilter::Offline, None);
        assert_eq!(
            offline.rows[0].metadata.title.as_deref(),
            Some("Cached task")
        );

        assert!(store.begin_source("socket", 2));
        let reconnecting = store.snapshot(SessionFilter::Offline, None);
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
        let live = store.snapshot(SessionFilter::All, None);
        assert_eq!(live.rows[0].metadata.title.as_deref(), Some("Current task"));
        assert!(!store.replace_source("socket", 1, [&stale]));
        assert_eq!(store.snapshot(SessionFilter::All, None), live);
    }

    #[test]
    fn generation_change_invalidates_selection_even_when_already_offline() {
        let mut store = SessionStore::new();
        assert!(store.begin_source("socket", 1));
        let row = record("terminal", "pane", AgentStatus::Idle);
        assert!(store.replace_source("socket", 1, [&row]));
        let key = store.snapshot(SessionFilter::All, None).rows[0].key.clone();
        assert!(store.mark_offline("socket", 1));
        let revision = store.revision();
        assert!(store.begin_source("socket", 2));
        assert!(store.revision() > revision);
        assert_eq!(
            store.snapshot(SessionFilter::All, Some(&key)).selected,
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
                let snapshot = store.snapshot(SessionFilter::All, None);
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
        let snapshot = store.snapshot(SessionFilter::All, None);
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
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
            store.snapshot(SessionFilter::All, None).rows[0].display_status(),
            DisplayStatus::Waiting
        );
    }

    #[test]
    fn new_source_requires_coherent_empty_snapshot_before_idle() {
        let mut store = SessionStore::new();
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
                .status_summary
                .status,
            DisplayStatus::NoSessions
        );
        assert!(store.begin_source("socket", 1));
        let before_snapshot = store.revision();
        assert_eq!(
            store.snapshot(SessionFilter::All, None).status_summary,
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
                .snapshot(SessionFilter::All, None)
                .status_summary
                .status,
            DisplayStatus::Idle
        );
        assert!(store.replace_source("socket", 1, std::iter::empty::<&AgentRecord>()));
        assert_eq!(store.revision(), before_snapshot + 1);
        assert!(store.mark_offline("socket", 1));
        assert_eq!(
            store
                .snapshot(SessionFilter::All, None)
                .status_summary
                .status,
            DisplayStatus::Offline
        );
        let disconnected_revision = store.revision();
        assert!(store.mark_offline("socket", 1));
        assert_eq!(store.revision(), disconnected_revision);
    }
}
