//! App-side, read-only update discovery and explicit helper handoff.
//! AppKit never waits for a manager, hash, source probe, or helper stdout.
use crate::i18n::{text, Message, UiLocale};
use crate::update_card::UpdateCardModel;
use herdr_update_coordinator::protocol::{
    latest_operation, lock_operation, private_updates, start_allowed, ExecutionFence,
    OperationPhase, OperationRecord, UPDATER_PROTOCOL,
};
use herdr_update_coordinator::{
    check, read_operation, read_plan, recheck_plan, stage_helper, write_plan, CheckSnapshot,
    CheckStatus, InstallOrigin, UpdateContext, UpdatePlan, UpdatePolicy,
};
use serde::Deserialize;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL: u64 = 24 * 60 * 60;

pub(super) enum Event {
    Checked {
        request_id: u64,
        policy_generation: u64,
        result: Result<CheckSnapshot, String>,
        settled: Option<SettledOperationProof>,
    },
    LaunchWaiting {
        operation_id: String,
    },
    Launched {
        operation_id: String,
        result: Result<(), LaunchFailure>,
    },
    Journal {
        operation_id: String,
        result: Result<Option<OperationRecord>, String>,
        prepared_recovery: Option<PreparedRecoveryProof>,
    },
    LatestJournal(Result<Option<OperationRecord>, String>),
}

#[derive(Deserialize)]
struct OwnershipAck {
    protocol: u32,
    operation_id: String,
    visible: bool,
    prepared: bool,
    #[serde(default)]
    error: Option<String>,
}

// Only the staging worker can prove whether Command::spawn was never reached
// (or failed). Once spawn succeeds, even a missing reply cannot release ownership.
pub(super) enum LaunchFailure {
    NoSpawn(String),
    Spawned(String),
}

// Only a background admission check can construct this proof. It identifies the exact
// journal observation that a fresh check may retire; durable evidence is never removed.
pub(super) struct SettledOperationProof(OperationRecord);
// Constructed only by the journal worker while holding the finished helper's
// operation lease. Unlike fresh-check retirement, this must match the original
// prepared instance and its immutable plan, not a replacement check's context.
pub(super) struct PreparedRecoveryProof {
    record: OperationRecord,
    plan: UpdatePlan,
}

pub(super) struct AppUpdateService {
    context: Option<UpdateContext>,
    snapshot: Option<CheckSnapshot>,
    check_busy: bool,
    launching: Option<String>,
    launch_plan: Option<UpdatePlan>,
    launch_waiting: bool,
    prepared_plan: Option<UpdatePlan>,
    cancelled_preparation: Option<String>,
    operation: Option<OperationRecord>,
    previous_outcome: Option<OperationRecord>,
    observed_operation_id: Option<String>,
    retired_operation_id: Option<String>,
    next_journal_poll: Instant,
    journal_in_flight: Option<String>,
    problem: Option<String>,
    journal_error: Option<String>,
    journal_error_operation_id: Option<String>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    last_checked: Option<u64>,
    auto_check: bool,
    policy: UpdatePolicy,
    policy_generation: u64,
    candidate_generation: u64,
    next_request_id: u64,
    pending_request: Option<u64>,
    launch_generation: Option<(u64, u64)>,
    next_auto_due: Option<u64>,
    scheduled_due_before: Option<Option<u64>>,
    auto_failure_paused: bool,
    revision: u64,
}

impl AppUpdateService {
    pub(super) fn new(
        context: Option<UpdateContext>,
        auto_check: bool,
        policy: UpdatePolicy,
        last_checked: Option<u64>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        if let Some(context) = context.as_ref() {
            let sender = sender.clone();
            let state_dir = context.state_dir.clone();
            std::thread::spawn(move || {
                let _ = sender.send(Event::LatestJournal(latest_operation(&state_dir)));
            });
        }
        let mut service = Self {
            context,
            snapshot: None,
            check_busy: false,
            launching: None,
            launch_plan: None,
            launch_waiting: false,
            prepared_plan: None,
            cancelled_preparation: None,
            previous_outcome: None,
            operation: None,
            observed_operation_id: None,
            retired_operation_id: None,
            next_journal_poll: Instant::now(),
            journal_in_flight: None,
            problem: None,
            journal_error: None,
            journal_error_operation_id: None,
            sender,
            receiver,
            last_checked,
            auto_check,
            policy,
            policy_generation: 0,
            candidate_generation: 0,
            next_request_id: 0,
            pending_request: None,
            launch_generation: None,
            next_auto_due: None,
            scheduled_due_before: None,
            auto_failure_paused: false,
            revision: 0,
        };
        service.check_due();
        service
    }
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    pub(super) fn policy(&self) -> UpdatePolicy {
        self.policy
    }

    pub(super) fn settings_admitted(&self) -> bool {
        !self.check_busy
            && self.launching.is_none()
            && self.launch_plan.is_none()
            && self.prepared_plan.is_none()
            && self.observed_operation_id.is_none()
            && self.journal_error.is_none()
            && self
                .operation
                .as_ref()
                .is_none_or(|record| terminal_phase(record.phase))
    }

    pub(super) fn change_policy(&mut self, policy: UpdatePolicy, checked_at: Option<u64>) {
        if !self.settings_admitted() || self.policy == policy {
            return;
        }
        self.policy = policy;
        self.policy_generation = self.policy_generation.wrapping_add(1);
        self.pending_request = None;
        self.check_busy = false;
        self.snapshot = None;
        self.problem = None;
        self.last_checked = checked_at;
        self.next_auto_due = None;
        self.scheduled_due_before = None;
        self.auto_failure_paused = false;
        self.revision = self.revision.wrapping_add(1);
        if self.auto_check {
            if let Some(time) = now() {
                self.start_auto_check(time);
            }
        }
    }

    pub(super) fn accepts_checked(&self, request_id: u64, policy_generation: u64) -> bool {
        self.pending_request == Some(request_id) && self.policy_generation == policy_generation
    }
    pub(super) fn consent_generation(&self) -> (u64, u64) {
        (self.policy_generation, self.candidate_generation)
    }

    pub(super) fn consent_matches(&self, plan: &UpdatePlan, generation: (u64, u64)) -> bool {
        self.consent_generation() == generation && self.plan() == Some(plan)
    }
    pub(super) fn download_url(&self) -> Option<&str> {
        if self.check_busy
            || self.launching.is_some()
            || self.prepared_plan.is_some()
            || self.problem.is_some()
            || self.journal_error.is_some()
            || self.observed_operation_id.is_some()
        {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        if snapshot.policy != self.policy
            || !matches!(
                snapshot.status,
                CheckStatus::Local | CheckStatus::Unsupported
            )
        {
            return None;
        }
        let release = snapshot.release.as_ref()?;
        release.validate().ok()?;
        Some(&release.archive_url)
    }

    pub(super) fn checked_event(
        &mut self,
        request_id: u64,
        policy_generation: u64,
        result: Result<CheckSnapshot, String>,
        settled: Option<SettledOperationProof>,
    ) -> Option<u64> {
        if !self.accepts_checked(request_id, policy_generation) {
            return None;
        }
        self.pending_request = None;
        self.scheduled_due_before = None;
        self.checked(result, settled)
    }

    pub(super) fn checked_at_for_save(
        &self,
        request_id: u64,
        policy_generation: u64,
        result: &Result<CheckSnapshot, String>,
    ) -> Option<u64> {
        if !self.accepts_checked(request_id, policy_generation) {
            return None;
        }
        match result {
            Ok(snapshot)
                if snapshot.policy == self.policy && snapshot.status != CheckStatus::Failed =>
            {
                snapshot.checked_at.or_else(now)
            }
            _ => None,
        }
    }

    pub(super) fn mark_check_save_failure(&mut self, error: String) {
        if let Some(previous_due) = self.scheduled_due_before.take() {
            self.next_auto_due = previous_due;
        }
        self.auto_failure_paused = true;
        self.pending_request = None;
        self.check_busy = false;
        self.problem = Some(format!(
            "Update was checked, but the check time could not be saved: {error}"
        ));
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn set_auto_check(&mut self, enabled: bool) {
        if !self.settings_admitted() || self.auto_check == enabled {
            return;
        }
        self.auto_check = enabled;
        self.revision = self.revision.wrapping_add(1);
        if enabled {
            self.policy_generation = self.policy_generation.wrapping_add(1);
            self.pending_request = None;
            self.snapshot = None;
            self.problem = None;
            self.next_auto_due = None;
            self.scheduled_due_before = None;
            self.auto_failure_paused = false;
            if let Some(time) = now() {
                self.start_auto_check(time);
            }
        }
    }

    pub(super) fn check_due(&mut self) {
        let Some(time) = now() else { return };
        if self.auto_check
            && !self.auto_failure_paused
            && self.next_auto_due.is_none_or(|due| time >= due)
            && self
                .last_checked
                .is_none_or(|last| time.saturating_sub(last) >= CHECK_INTERVAL)
        {
            self.start_auto_check(time);
        }
    }

    fn start_auto_check(&mut self, time: u64) {
        let previous_due = self.next_auto_due;
        let previous_request = self.pending_request;
        self.check_now();
        if self.pending_request != previous_request {
            self.scheduled_due_before = Some(previous_due);
            self.next_auto_due = Some(time.saturating_add(CHECK_INTERVAL));
        }
    }

    pub(super) fn check_now(&mut self) {
        if self.check_busy
            || self.launching.is_some()
            || (self.launch_plan.is_some()
                && !self
                    .operation
                    .as_ref()
                    .is_some_and(|record| terminal_phase(record.phase)))
            || (self.prepared_plan.is_some()
                && !self.operation.as_ref().is_some_and(|record| {
                    terminal_phase(record.phase)
                        && self.cancelled_preparation.as_deref()
                            == Some(record.operation_id.as_str())
                }))
            || self
                .operation
                .as_ref()
                .is_some_and(|record| !terminal_phase(record.phase))
            || (self.observed_operation_id.is_some()
                && !self
                    .operation
                    .as_ref()
                    .is_some_and(|record| terminal_phase(record.phase)))
            || self.journal_error.is_some()
        {
            return;
        }
        let Some(context) = self.context.clone() else {
            let problem = "Running app update context is unavailable";
            if self.problem.as_deref() != Some(problem) {
                self.problem = Some(problem.into());
                self.revision = self.revision.wrapping_add(1);
            }
            return;
        };
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        let policy_generation = self.policy_generation;
        let policy = self.policy;
        self.pending_request = Some(request_id);
        self.auto_failure_paused = false;
        self.check_busy = true;
        self.problem = None;
        self.revision = self.revision.wrapping_add(1);
        let sender = self.sender.clone();
        let observed_operation_id = self.observed_operation_id.clone();
        std::thread::spawn(move || {
            let result = check(&context, policy);
            let prior_operation = observed_operation_id.or_else(|| {
                latest_operation(&context.state_dir)
                    .ok()
                    .flatten()
                    .map(|record| record.operation_id)
            });
            let settled = result.as_ref().ok().and_then(|snapshot| {
                prior_operation.as_deref().and_then(|operation_id| {
                    settlement_proof(&context, snapshot, operation_id).ok()
                })
            });
            let _ = sender.send(Event::Checked {
                request_id,
                policy_generation,
                result,
                settled,
            });
        });
    }

    pub(super) fn next_event(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }

    pub(super) fn checked(
        &mut self,
        result: Result<CheckSnapshot, String>,
        settled: Option<SettledOperationProof>,
    ) -> Option<u64> {
        if result
            .as_ref()
            .is_ok_and(|snapshot| snapshot.policy != self.policy)
        {
            self.check_busy = false;
            self.revision = self.revision.wrapping_add(1);
            return None;
        }
        self.candidate_generation = self.candidate_generation.wrapping_add(1);
        let mut changed = self.check_busy
            || match &result {
                Ok(next) => self.snapshot.as_ref().is_none_or(|old| {
                    old.status != next.status
                        || old.origin != next.origin
                        || old.policy != next.policy
                        || old.release != next.release
                        || old.installed_version != next.installed_version
                        || old.detail != next.detail
                        || old.plan != next.plan
                }),
                Err(_) => false,
            };
        self.check_busy = false;
        let checked_at = match result {
            Ok(snapshot) if snapshot.status != CheckStatus::Failed => {
                let checked_at = snapshot.checked_at.or_else(now);
                changed |= self.last_checked != checked_at || self.problem.is_some();
                self.last_checked = checked_at;
                self.next_auto_due = checked_at.map(|time| time.saturating_add(CHECK_INTERVAL));
                self.problem = None;
                if let Some(SettledOperationProof(record)) = settled {
                    let matching_observation = self.observed_operation_id.as_deref()
                        == Some(record.operation_id.as_str())
                        && self.operation.as_ref() == Some(&record);
                    let not_yet_observed =
                        self.observed_operation_id.is_none() && self.operation.is_none();
                    if self.launching.is_none()
                        && (self.launch_plan.is_none()
                            || (self.prepared_plan.is_none()
                                && matching_observation
                                && terminal_phase(record.phase)))
                        && (self.prepared_plan.is_none()
                            || self.cancelled_preparation.as_deref()
                                == Some(record.operation_id.as_str()))
                        && self.journal_error.is_none()
                        && (matching_observation || not_yet_observed)
                    {
                        // Keep the proven outcome for presentation even after the runtime
                        // observation retires; never delete or alter the durable journal.
                        self.previous_outcome = Some(record.clone());
                        changed = true;
                        self.observed_operation_id = None;
                        self.operation = None;
                        self.launch_plan = None;
                        self.prepared_plan = None;
                        self.cancelled_preparation = None;
                        self.retired_operation_id = Some(record.operation_id);
                    }
                }
                self.snapshot = Some(snapshot);
                checked_at
            }
            Ok(snapshot) => {
                changed |= self.problem.as_deref() != Some(snapshot.detail.as_str());
                self.problem = Some(snapshot.detail.clone());
                self.snapshot = Some(snapshot);
                None
            }
            Err(error) => {
                changed |= self.problem.as_deref() != Some(error.as_str());
                self.problem = Some(error);
                None
            }
        };
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        checked_at
    }
    pub(super) fn plan(&self) -> Option<&UpdatePlan> {
        if self.check_busy
            || self.launching.is_some()
            || self.launch_plan.is_some()
            || self.prepared_plan.is_some()
            || self.observed_operation_id.is_some()
            || self.operation.is_some()
            || self.problem.is_some()
            || self.journal_error.is_some()
        {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        if snapshot.policy != self.policy {
            return None;
        }
        snapshot.plan.as_ref().filter(|plan| {
            plan.target.as_ref().is_none_or(|target| {
                target.policy == self.policy && snapshot.release.as_ref() == Some(&target.release)
            })
        })
    }
    pub(super) fn preparing_plan(&self, operation_id: &str) -> Option<&UpdatePlan> {
        self.launch_plan.as_ref().filter(|plan| {
            plan.operation_id == operation_id
                && self.launching.as_deref() == Some(operation_id)
                && self.observed_operation_id.as_deref() == Some(operation_id)
                && !self
                    .operation
                    .as_ref()
                    .is_some_and(|record| terminal_phase(record.phase))
                && self.launch_generation == Some(self.consent_generation())
                && plan
                    .target
                    .as_ref()
                    .is_none_or(|target| target.policy == self.policy)
        })
    }

    pub(super) fn launch_waiting(&mut self, operation_id: &str) {
        if self.launching.as_deref() == Some(operation_id)
            && !self
                .operation
                .as_ref()
                .is_some_and(|record| terminal_phase(record.phase))
            && !self.launch_waiting
        {
            self.launch_waiting = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub(super) fn observing(&self, operation_id: &str) -> bool {
        self.observed_operation_id.as_deref() == Some(operation_id)
    }

    pub(super) fn preparation_cancelled(&mut self, operation_id: &str) {
        if self
            .launch_plan
            .as_ref()
            .is_some_and(|plan| plan.operation_id == operation_id)
            || self
                .prepared_plan
                .as_ref()
                .is_some_and(|plan| plan.operation_id == operation_id)
        {
            // Native state explicitly cancelled this request. Retain the
            // exact plan until independent, locked settlement proof.
            self.cancelled_preparation = Some(operation_id.to_owned());
        }
    }

    pub(super) fn recovery_reconciled(&mut self, operation_id: &str) {
        if self
            .prepared_plan
            .as_ref()
            .is_some_and(|plan| plan.operation_id == operation_id)
        {
            self.prepared_plan = None;
            self.cancelled_preparation = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub(super) fn launch(
        &mut self,
        plan: UpdatePlan,
        generation: (u64, u64),
    ) -> Result<(), String> {
        if !self.consent_matches(&plan, generation) {
            return Err("Update state changed; check again before applying".into());
        }
        self.launching = Some(plan.operation_id.clone());
        self.observed_operation_id = Some(plan.operation_id.clone());
        self.prepared_plan = None;
        self.cancelled_preparation = None;
        self.launch_waiting = false;
        self.launch_plan = Some(plan.clone());
        self.problem = None;
        self.launch_generation = Some(generation);
        self.revision = self.revision.wrapping_add(1);
        let sender = self.sender.clone();
        let notice = self.sender.clone();
        std::thread::spawn(move || {
            let result = stage_and_launch(&plan, || {
                let _ = notice.send(Event::LaunchWaiting {
                    operation_id: plan.operation_id.clone(),
                });
            });
            let _ = sender.send(Event::Launched {
                operation_id: plan.operation_id,
                result,
            });
        });
        Ok(())
    }

    pub(super) fn launch_result(
        &mut self,
        operation_id: &str,
        result: Result<(), LaunchFailure>,
    ) -> bool {
        if self.launching.as_deref() != Some(operation_id) {
            return false;
        }
        self.launch_waiting = false;
        if let Err(error) = result {
            self.launching = None;
            self.launch_generation = None;
            match error {
                LaunchFailure::NoSpawn(detail) => {
                    self.problem = Some(detail);
                    // Staging/spawn failure never gave this helper ownership.
                    // Keep any independently observed journal evidence.
                    if self.operation.is_none()
                        && (self.journal_error.is_none()
                            || self.journal_error_operation_id.as_deref() == Some(operation_id))
                    {
                        self.launch_plan = None;
                        self.observed_operation_id = None;
                        self.journal_error = None;
                        self.journal_error_operation_id = None;
                    }
                }
                LaunchFailure::Spawned(detail) => {
                    self.problem = Some(detail);
                    // The helper may have accepted native Prepare even if
                    // stdout failed before its final ownership reply.
                    self.prepared_plan = self.launch_plan.take();
                }
            }
            self.revision = self.revision.wrapping_add(1);
            return false;
        }
        // Accepted native Prepare precedes the final ACK. Preserve its exact
        // immutable plan for independently proved recovery, and consume this
        // launch so a duplicate final reply cannot be accepted.
        self.prepared_plan = self.launch_plan.take();
        self.launching = None;
        self.launch_generation = None;
        self.revision = self.revision.wrapping_add(1);
        self.next_journal_poll = Instant::now();
        true
    }
    pub(super) fn abort_launch(&mut self, operation_id: &str, detail: String) {
        if self.observed_operation_id.as_deref() != Some(operation_id)
            || (self.launching.as_deref() != Some(operation_id)
                && !self
                    .prepared_plan
                    .as_ref()
                    .is_some_and(|plan| plan.operation_id == operation_id))
        {
            return;
        }
        self.launching = None;
        self.launch_generation = None;
        self.launch_waiting = false;
        self.problem = Some(detail);
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn poll_operation(&mut self) {
        if Instant::now() < self.next_journal_poll {
            return;
        }
        let Some(operation_id) = self.observed_operation_id.clone() else {
            return;
        };
        self.next_journal_poll = Instant::now() + Duration::from_secs(5);
        self.reload_journal(operation_id);
    }
    pub(super) fn reload_journal(&mut self, operation_id: String) {
        if self.observed_operation_id.as_deref() != Some(operation_id.as_str())
            || self.journal_in_flight.is_some()
        {
            return;
        }
        let Some(context) = self.context.as_ref() else {
            return;
        };
        self.journal_in_flight = Some(operation_id.clone());
        let context = context.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = read_operation(&context.state_dir, &operation_id);
            let prepared_recovery = result
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .and_then(|record| prepared_recovery_proof(&context, &operation_id, record).ok());
            let _ = sender.send(Event::Journal {
                operation_id,
                result,
                prepared_recovery,
            });
        });
    }

    pub(super) fn journal(
        &mut self,
        operation_id: &str,
        result: Result<Option<OperationRecord>, String>,
        prepared_recovery: Option<PreparedRecoveryProof>,
    ) -> Option<UpdateContext> {
        if self.journal_in_flight.as_deref() == Some(operation_id) {
            self.journal_in_flight = None;
        }
        if self.observed_operation_id.as_deref() != Some(operation_id) {
            return None;
        }
        if self.operation.as_ref().is_some_and(|record| {
            record.operation_id == operation_id
                && terminal_phase(record.phase)
                && matches!(&result, Ok(Some(next))
                    if next.operation_id == operation_id
                        && next.phase != record.phase
                        && next.phase != OperationPhase::UserStopped)
        }) {
            return None;
        }
        let old_record = self.operation.as_ref();
        let old_error = self.journal_error.as_ref();
        let mut changed = match &result {
            Ok(Some(record)) if record.operation_id == operation_id => {
                old_record != Some(record) || old_error.is_some()
            }
            Ok(_) => {
                old_error.map(String::as_str)
                    != Some("Coordinator operation journal is not available; outcome is unknown")
            }
            Err(error) => {
                old_error.map(String::as_str)
                    != Some(
                        format!("Coordinator operation journal cannot be read: {error}").as_str(),
                    )
            }
        };
        let terminal = matches!(&result, Ok(Some(record))
        if record.operation_id == operation_id
            && matches!(
                record.phase,
                OperationPhase::Completed
                    | OperationPhase::Failed
                    | OperationPhase::Unknown
                    | OperationPhase::UserStopped
            ));
        match result {
            Ok(Some(record)) if record.operation_id == operation_id => {
                self.journal_error = None;
                self.journal_error_operation_id = None;
                self.operation = Some(record);
            }
            Ok(_) => {
                self.journal_error = Some(
                    "Coordinator operation journal is not available; outcome is unknown".into(),
                );
                self.journal_error_operation_id = Some(operation_id.into());
            }
            Err(error) => {
                self.journal_error = Some(format!(
                    "Coordinator operation journal cannot be read: {error}"
                ));
                self.journal_error_operation_id = Some(operation_id.into());
            }
        }
        if terminal {
            self.launch_waiting = false;
        }
        if terminal {
            if let Some(record) = self.operation.as_ref() {
                if self.previous_outcome.as_ref() != Some(record) {
                    self.previous_outcome = Some(record.clone());
                    changed = true;
                }
            }
        }
        if terminal && self.launching.as_deref() == Some(operation_id) {
            self.launching = None;
            self.launch_generation = None;
            // A terminal journal can precede the final ACK. Retain its exact
            // plan for locked recovery without treating a late ACK as success.
            if let Some(plan) = self.launch_plan.take() {
                self.prepared_plan = Some(plan);
            }
            changed = true;
        }
        let reconciled = prepared_recovery.and_then(|proof| {
            let context = self.context.as_ref()?;
            if self.prepared_plan.as_ref() == Some(&proof.plan)
                && proof.plan.context == *context
                && self.operation.as_ref() == Some(&proof.record)
                && self.journal_error.is_none()
            {
                Some(context.clone())
            } else {
                None
            }
        });
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        reconciled
    }
    pub(super) fn latest_journal(&mut self, result: Result<Option<OperationRecord>, String>) {
        // Startup discovery must not replace a locally owned launch/preparation
        // or an active operation. An unowned terminal startup observation can
        // be superseded by a subsequently observed active operation.
        if self.observed_operation_id.is_some()
            && !(self.launching.is_none()
                && self.launch_plan.is_none()
                && self.prepared_plan.is_none()
                && self
                    .operation
                    .as_ref()
                    .is_some_and(|record| terminal_phase(record.phase))
                && matches!(&result, Ok(Some(record))
                    if !terminal_phase(record.phase)
                        && self.observed_operation_id.as_deref()
                            != Some(record.operation_id.as_str())))
        {
            if let Err(error) = result {
                if self.operation.is_none() && self.journal_error.is_none() {
                    self.journal_error =
                        Some(format!("Update journal cannot be verified: {error}"));
                    self.journal_error_operation_id = None;
                    self.revision = self.revision.wrapping_add(1);
                }
            }
            return;
        }
        match result {
            Ok(Some(record))
                if self.retired_operation_id.as_deref() != Some(record.operation_id.as_str()) =>
            {
                self.observed_operation_id = Some(record.operation_id.clone());
                if terminal_phase(record.phase) {
                    self.previous_outcome = Some(record.clone());
                }
                self.operation = Some(record);
                self.journal_error = None;
                self.journal_error_operation_id = None;
                self.revision = self.revision.wrapping_add(1);
            }
            Err(error) => {
                self.journal_error = Some(format!("Update journal cannot be verified: {error}"));
                self.journal_error_operation_id = None;
                self.revision = self.revision.wrapping_add(1);
            }
            Ok(_) => {}
        }
    }

    pub(super) fn set_problem(&mut self, problem: String) {
        if self.problem.as_ref() != Some(&problem) {
            self.revision = self.revision.wrapping_add(1);
        }
        self.problem = Some(problem);
    }

    pub(super) fn model(&self, locale: UiLocale) -> UpdateCardModel {
        let origin = self
            .snapshot
            .as_ref()
            .map(|snapshot| &snapshot.origin)
            .or_else(|| self.context.as_ref()?.running_origin.as_deref());
        let active = self
            .operation
            .as_ref()
            .filter(|record| !terminal_phase(record.phase));
        let outcome = self
            .operation
            .as_ref()
            .filter(|record| terminal_phase(record.phase))
            .or(self.previous_outcome.as_ref());
        let history = outcome.map_or_else(String::new, |record| {
            format!(
                "{}: {}",
                text(locale, Message::AppUpdatePreviousOperation),
                text(locale, outcome_message(record))
            )
        });
        let (status, status_message, detail_message) = if self.journal_error.is_some() {
            (
                CheckStatus::Failed,
                Message::AppUpdateOutcomeUnknown,
                Message::AppUpdateJournalHelp,
            )
        } else if self.problem.is_some() && !self.check_busy {
            (
                CheckStatus::Failed,
                Message::AppUpdateFailed,
                Message::AppUpdateFailedHelp,
            )
        } else if let Some(record) = active {
            (
                CheckStatus::Checking,
                phase_message(record.phase),
                if record.phase == OperationPhase::Starting {
                    Message::AppUpdateStartingHelp
                } else {
                    Message::AppUpdateActiveHelp
                },
            )
        } else if self.launching.is_some() {
            (
                CheckStatus::Checking,
                Message::AppUpdateLaunchPending,
                Message::AppUpdateActiveHelp,
            )
        } else if self.check_busy {
            (
                CheckStatus::Checking,
                Message::AppUpdateChecking,
                Message::AppUpdateCheckingHelp,
            )
        } else if self.operation.as_ref().is_some_and(|record| {
            matches!(
                record.phase,
                OperationPhase::Unknown | OperationPhase::Failed | OperationPhase::UserStopped
            )
        }) {
            let record = self.operation.as_ref().expect("terminal operation");
            (
                CheckStatus::Failed,
                outcome_message(record),
                match record.phase {
                    OperationPhase::Unknown => Message::AppUpdateUnknownHelp,
                    OperationPhase::UserStopped => Message::AppUpdateStoppedHelp,
                    _ => Message::AppUpdateBlockedHelp,
                },
            )
        } else {
            let status = self
                .snapshot
                .as_ref()
                .map_or(CheckStatus::NotChecked, |snapshot| snapshot.status);
            let detail = if self.observed_operation_id.is_some() || self.prepared_plan.is_some() {
                Message::AppUpdateBlockedHelp
            } else {
                check_detail_message(status)
            };
            (status, check_status_message(status), detail)
        };
        let mut diagnostics = String::new();
        if let Some(context) = self.context.as_ref() {
            let _ = writeln!(
                diagnostics,
                "Running executable: {}",
                context.running.path.display()
            );
            let _ = writeln!(diagnostics, "Running SHA-256: {}", context.running.sha256);
        }
        if let Some(origin) = origin {
            let _ = writeln!(
                diagnostics,
                "Running source: {}",
                source_description(origin)
            );
        }
        if let Some(snapshot) = self.snapshot.as_ref() {
            let _ = writeln!(
                diagnostics,
                "Check: {:?} · {}",
                snapshot.status, snapshot.detail
            );
            if let Some(version) = snapshot.installed_version.as_ref() {
                let _ = writeln!(diagnostics, "Installed package: {version}");
            }
            if let Some(release) = snapshot.release.as_ref() {
                let _ = writeln!(
                    diagnostics,
                    "Official release: {} · {}",
                    release.version, release.tag
                );
            }
            if let Some(plan) = snapshot.plan.as_ref() {
                let _ = writeln!(diagnostics, "Candidate operation: {}", plan.operation_id);
                if let Some(candidate) = plan.candidate.as_ref() {
                    let _ = writeln!(
                        diagnostics,
                        "Candidate: {} · SHA-256 {}",
                        candidate.path.display(),
                        candidate.sha256
                    );
                }
            }
        }
        if let Some(record) = self.operation.as_ref() {
            append_operation_diagnostics(&mut diagnostics, "Observed operation", record);
        }
        if let Some(record) = self.previous_outcome.as_ref() {
            if self.operation.as_ref() != Some(record) {
                append_operation_diagnostics(&mut diagnostics, "Previous operation", record);
            }
        }
        if self.launch_waiting && active.is_none() && self.launching.is_some() {
            let _ = writeln!(
                diagnostics,
                "{}: {}",
                text(locale, Message::AppUpdateLaunchPending),
                text(locale, Message::AppUpdateActiveHelp)
            );
        }
        if let Some(error) = self.problem.as_ref() {
            let _ = writeln!(diagnostics, "Check or launch error: {error}");
        }
        if let Some(error) = self.journal_error.as_ref() {
            let _ = writeln!(diagnostics, "Journal error: {error}");
        }
        UpdateCardModel {
            auto_check: self.auto_check,
            receive_beta_updates: self.policy == UpdatePolicy::StableAndBeta,
            busy: self.check_busy
                || self.launching.is_some()
                || self.launch_plan.is_some()
                || self.prepared_plan.is_some()
                || active.is_some()
                || self.journal_error.is_some(),
            settings_locked: !self.settings_admitted(),
            status,
            status_text: text(locale, status_message).into(),
            running_version: self
                .context
                .as_ref()
                .map_or_else(String::new, |context| context.version.clone()),
            source: match origin {
                Some(InstallOrigin::Homebrew { formula, .. }) => {
                    format!(
                        "{} · {formula}",
                        text(locale, Message::AppUpdateSourceHomebrew)
                    )
                }
                _ => text(
                    locale,
                    origin.map_or(Message::AppUpdateSourceUnknown, source_message),
                )
                .into(),
            },
            detail: text(locale, detail_message).into(),
            history,
            diagnostics,
            last_checked: self.last_checked,
            action: self.plan().map(|plan| plan.action.clone()),
            download_url: self.download_url().map(str::to_owned),
            release: self
                .snapshot
                .as_ref()
                .filter(|snapshot| {
                    snapshot.policy == self.policy && !self.check_busy && self.problem.is_none()
                })
                .and_then(|snapshot| snapshot.release.clone()),
        }
    }
}

// Journal worker only. The helper must have exited, recovered NoSpawn with no
// applied/installed result, and released *both* reservation scopes. A mere
// Failed journal or a different original instance is never a thaw signal.
fn prepared_recovery_proof(
    context: &UpdateContext,
    operation_id: &str,
    observed: &OperationRecord,
) -> Result<PreparedRecoveryProof, String> {
    if observed.version != UPDATER_PROTOCOL
        || observed.phase != OperationPhase::Failed
        || observed.execution_fence != ExecutionFence::NoSpawn
        || observed.installed
        || observed.applied
    {
        return Err("prepared update has no reconciled NoSpawn failure".into());
    }
    let _lease = lock_operation(&context.state_dir, operation_id)?;
    let record = read_operation(&context.state_dir, operation_id)?
        .ok_or("prepared operation journal is missing")?;
    if &record != observed {
        return Err("prepared operation journal changed during reconciliation".into());
    }
    let plan = read_plan(&context.state_dir, operation_id)?;
    if plan.context != *context
        || plan.running != context.running
        || matches!(plan.origin, InstallOrigin::Unknown { .. })
    {
        return Err("prepared operation belongs to a different running instance or source".into());
    }
    if latest_operation(&context.state_dir)?.as_ref() != Some(&record) {
        return Err("prepared operation is no longer the latest journal".into());
    }
    start_allowed(&plan.origin, &context.state_dir, None, None)?;
    Ok(PreparedRecoveryProof { record, plan })
}

// Runs only on the check worker. A journal's terminal phase by itself is not
// reconciliation: verify its immutable plan, nonblocking helper lease and both
// installation/profile admission markers before retiring the runtime observation.
fn settlement_proof(
    context: &UpdateContext,
    snapshot: &CheckSnapshot,
    operation_id: &str,
) -> Result<SettledOperationProof, String> {
    if let Some(fresh_plan) = snapshot.plan.as_ref() {
        if fresh_plan.context != *context
            || fresh_plan.running != context.running
            || fresh_plan.origin != snapshot.origin
            || fresh_plan.operation_id == operation_id
        {
            return Err("fresh check plan is not for the running profile/source".into());
        }
    } else if context.running_origin.as_deref() != Some(&snapshot.origin) {
        // An up-to-date installation has no next update plan. Its checked source
        // must still match the startup capture before retiring prior ownership.
        return Err("fresh check source is not the captured running installation".into());
    }
    let _lease = lock_operation(&context.state_dir, operation_id)?;
    let record = read_operation(&context.state_dir, operation_id)?
        .ok_or("previous operation journal is missing")?;
    if !settled_terminal(&record) {
        return Err("previous operation is not a reconciled terminal outcome".into());
    }
    let prior_plan = read_plan(&context.state_dir, operation_id)?;
    if prior_plan.context.config_dir != context.config_dir {
        return Err("previous operation belongs to a different configuration profile".into());
    }
    let latest = latest_operation(&context.state_dir)?
        .ok_or("previous operation is no longer the latest journal")?;
    if latest != record {
        return Err("previous operation journal changed during admission".into());
    }
    start_allowed(&prior_plan.origin, &context.state_dir, None, None)?;
    if snapshot.origin != prior_plan.origin {
        start_allowed(&snapshot.origin, &context.state_dir, None, None)?;
    }
    Ok(SettledOperationProof(record))
}

fn settled_terminal(record: &OperationRecord) -> bool {
    record.version == UPDATER_PROTOCOL
        && record.execution_fence != ExecutionFence::ManagerIntent
        && match record.phase {
            OperationPhase::Completed => record.installed && record.applied,
            OperationPhase::Failed | OperationPhase::UserStopped => !record.applied,
            _ => false,
        }
}

fn terminal_phase(phase: OperationPhase) -> bool {
    matches!(
        phase,
        OperationPhase::Completed
            | OperationPhase::Failed
            | OperationPhase::Unknown
            | OperationPhase::UserStopped
    )
}

fn phase_message(phase: OperationPhase) -> Message {
    match phase {
        OperationPhase::Preparing => Message::AppUpdatePreparing,
        OperationPhase::Stopping => Message::AppUpdateStopping,
        OperationPhase::Installing => Message::AppUpdateInstalling,
        OperationPhase::Validating => Message::AppUpdateValidating,
        OperationPhase::Starting => Message::AppUpdateStarting,
        OperationPhase::Completed => Message::AppUpdateApplied,
        OperationPhase::Failed => Message::AppUpdateFailed,
        OperationPhase::Unknown => Message::AppUpdateOutcomeUnknown,
        OperationPhase::UserStopped => Message::AppUpdateUserStopped,
    }
}

fn outcome_message(record: &OperationRecord) -> Message {
    match record.phase {
        OperationPhase::Completed if record.installed && record.applied => {
            Message::AppUpdateApplied
        }
        OperationPhase::Failed => Message::AppUpdateFailed,
        OperationPhase::UserStopped => Message::AppUpdateUserStopped,
        _ => Message::AppUpdateOutcomeUnknown,
    }
}

fn source_message(origin: &InstallOrigin) -> Message {
    match origin {
        InstallOrigin::Herdr { .. } => Message::AppUpdateSourceHerdr,
        InstallOrigin::Local { .. } => Message::AppUpdateSourceLocal,
        InstallOrigin::Homebrew { .. } => Message::AppUpdateSourceHomebrew,
        InstallOrigin::Manual { .. } => Message::AppUpdateSourceManual,
        InstallOrigin::Unknown { .. } => Message::AppUpdateSourceUnknown,
    }
}

fn check_status_message(status: CheckStatus) -> Message {
    match status {
        CheckStatus::NotChecked => Message::AppUpdateNotChecked,
        CheckStatus::Checking => Message::AppUpdateChecking,
        CheckStatus::Current => Message::AppUpdateCurrent,
        CheckStatus::Applied => Message::AppUpdateApplied,
        CheckStatus::Available => Message::AppUpdateAvailable,
        CheckStatus::Installed => Message::AppUpdateInstalled,
        CheckStatus::Local => Message::AppUpdateLocal,
        CheckStatus::Unsupported => Message::AppUpdateUnsupported,
        CheckStatus::Failed => Message::AppUpdateFailed,
    }
}

fn check_detail_message(status: CheckStatus) -> Message {
    match status {
        CheckStatus::NotChecked => Message::AppUpdateNotCheckedHelp,
        CheckStatus::Checking => Message::AppUpdateCheckingHelp,
        CheckStatus::Current | CheckStatus::Applied => Message::AppUpdateCurrentHelp,
        CheckStatus::Available => Message::AppUpdateAvailableHelp,
        CheckStatus::Installed => Message::AppUpdateInstalledHelp,
        CheckStatus::Local => Message::AppUpdateLocalHelp,
        CheckStatus::Unsupported => Message::AppUpdateUnsupportedHelp,
        CheckStatus::Failed => Message::AppUpdateFailedHelp,
    }
}

fn append_operation_diagnostics(diagnostics: &mut String, label: &str, record: &OperationRecord) {
    let _ = writeln!(
        diagnostics,
        "{label}: {} · phase {:?} · installed {} · applied {} · fence {:?}",
        record.operation_id, record.phase, record.installed, record.applied, record.execution_fence
    );
    let _ = writeln!(diagnostics, "Operation detail: {}", record.detail);
    if let Some(candidate) = record.candidate.as_ref() {
        let _ = writeln!(
            diagnostics,
            "Candidate: {} · SHA-256 {}",
            candidate.path.display(),
            candidate.sha256
        );
    }
}

pub(super) fn source_description(origin: &InstallOrigin) -> String {
    match origin {
        InstallOrigin::Herdr {
            host,
            host_config_dir,
            checkout_root,
            plugin_root,
            source,
            requested_ref,
            resolved_commit,
            enabled,
        } => format!(
            "Herdr · host {} · host config {} · checkout {} · plugin {} · {source} · ref {} · commit {} · {}",
            host.display(),
            host_config_dir.display(),
            checkout_root.display(),
            plugin_root.display(),
            requested_ref.as_deref().unwrap_or("default"),
            resolved_commit.as_deref().unwrap_or("not observed"),
            if *enabled { "enabled" } else { "disabled" }
        ),
        InstallOrigin::Local { root } => format!("Local checkout · {}", root.display()),
        InstallOrigin::Homebrew {
            brew,
            formula,
            cellar_formula_root,
            prefix,
            locator,
        } => format!(
            "Homebrew · {formula} · brew {} · prefix {} · formula root {} · app {}",
            brew.display(),
            prefix.display(),
            cellar_formula_root.display(),
            locator.display()
        ),
        InstallOrigin::Manual { locator } => format!("Manual · {}", locator.display()),
        InstallOrigin::Unknown { reason } => format!("Unknown · {reason}"),
    }
}

fn now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|time| time.as_secs())
}

fn stage_and_launch(plan: &UpdatePlan, on_waiting: impl FnOnce()) -> Result<(), LaunchFailure> {
    // The plan remains bound to the startup image and captured install policy.
    recheck_plan(plan).map_err(LaunchFailure::NoSpawn)?;
    let helper = stage_helper(plan).map_err(LaunchFailure::NoSpawn)?;
    if let Err(error) = write_plan(plan) {
        let _ = std::fs::remove_file(&helper);
        return Err(LaunchFailure::NoSpawn(error));
    }
    let cwd = private_updates(&plan.context.state_dir).map_err(LaunchFailure::NoSpawn)?;
    let mut command = Command::new(helper);
    command
        .arg("run")
        .arg("--state-dir")
        .arg(&plan.context.state_dir)
        .arg("--operation-id")
        .arg(&plan.operation_id)
        .current_dir(cwd)
        .env_clear()
        .envs(&plan.context.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(host_config_dir) = plan.context.host_plugin_config_dir.as_ref() {
        command.env(
            "HERDR_DESKTOP_PET_CAPTURED_HOST_PLUGIN_CONFIG_DIR",
            host_config_dir,
        );
    }
    // The helper owns its own AppKit loop and journal after this process exits.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|error| {
        LaunchFailure::NoSpawn(format!("Cannot launch update coordinator: {error}"))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LaunchFailure::Spawned("Coordinator ownership channel missing".into()))?;
    // The helper owns its journal and UI; after spawn, never kill it on timeout.
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(read_ownership_ack(stdout));
    });
    await_ownership_ack(
        receiver,
        &plan.operation_id,
        Duration::from_secs(30),
        on_waiting,
    )
}

fn read_ownership_ack(stdout: impl Read) -> Result<OwnershipAck, String> {
    let mut line = String::new();
    let length = BufReader::new(stdout.take(4096))
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if length == 0 || length >= 4096 {
        return Err("Coordinator ownership reply missing or oversized".into());
    }
    if !line.ends_with('\n') {
        return Err("Coordinator ownership reply ended before its final line".into());
    }
    serde_json::from_str(&line).map_err(|error| error.to_string())
}

fn await_ownership_ack(
    receiver: Receiver<Result<OwnershipAck, String>>,
    operation_id: &str,
    notice_after: Duration,
    on_waiting: impl FnOnce(),
) -> Result<(), LaunchFailure> {
    let reply = match receiver.recv_timeout(notice_after) {
        Ok(reply) => reply,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            on_waiting();
            receiver.recv().map_err(|_| {
                LaunchFailure::Spawned("Coordinator ownership channel disconnected".into())
            })?
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(LaunchFailure::Spawned(
                "Coordinator ownership channel disconnected".into(),
            ));
        }
    };
    let ack = reply.map_err(LaunchFailure::Spawned)?;
    if ack.protocol != UPDATER_PROTOCOL
        || ack.operation_id != operation_id
        || !ack.visible
        || !ack.prepared
        || ack.error.is_some()
    {
        return Err(LaunchFailure::Spawned(ack.error.unwrap_or_else(|| {
            "Coordinator did not confirm visible ownership and preparation".into()
        })));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_update_coordinator::{
        ExecutableIdentity, ReleaseCandidate, ReleaseChannel, UpdateAction,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn local_check() -> CheckSnapshot {
        let root = PathBuf::from("/isolated/local-source");
        let running = ExecutableIdentity {
            path: root.join("app"),
            sha256: "current image".into(),
        };
        let context = UpdateContext {
            executable: running.path.clone(),
            version: "0.2.0".into(),
            instance_id: "isolated".into(),
            config_dir: PathBuf::from("/isolated/config"),
            host_plugin_config_dir: None,
            state_dir: PathBuf::from("/isolated/state"),
            herdr_socket: PathBuf::from("/isolated/socket"),
            running: running.clone(),
            running_origin: None,
            assets_override: None,
            environment: BTreeMap::new(),
            locale: "en".into(),
        };
        let origin = InstallOrigin::Local { root };
        CheckSnapshot {
            status: CheckStatus::Local,
            policy: UpdatePolicy::StableOnly,
            release: None,
            origin: origin.clone(),
            installed_version: Some("0.2.0".into()),
            observed_revision: None,
            detail: "rebuild on-disk sources".into(),
            plan: Some(UpdatePlan {
                version: UPDATER_PROTOCOL,
                operation_id: "e".repeat(32),
                context,
                origin,
                action: UpdateAction::LocalRebuild,
                running,
                candidate: None,
                target: None,
            }),
            checked_at: Some(42),
        }
    }

    fn failed_record(id: char) -> OperationRecord {
        OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: id.to_string().repeat(32),
            phase: OperationPhase::Failed,
            execution_fence: ExecutionFence::NoSpawn,
            detail: "no manager ran; recovery released both reservations".into(),
            installed: false,
            applied: false,
            candidate: None,
        }
    }

    #[test]
    fn exact_recovered_prepared_failure_preserves_failed_card_and_thaws_only_that_operation() {
        let record = failed_record('a');
        let mut plan = local_check().plan.unwrap();
        plan.operation_id = record.operation_id.clone();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.context = Some(plan.context.clone());
        service.launching = Some(record.operation_id.clone());
        service.launch_plan = Some(plan.clone());
        service.observed_operation_id = Some(record.operation_id.clone());
        service.launch_generation = Some(service.consent_generation());
        assert_eq!(
            service.journal(
                &record.operation_id,
                Ok(Some(record.clone())),
                Some(PreparedRecoveryProof {
                    record: record.clone(),
                    plan: plan.clone(),
                }),
            ),
            Some(plan.context.clone())
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        service.recovery_reconciled(&record.operation_id);
        assert!(service.prepared_plan.is_none());
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Failed);
        assert!(service.plan().is_none());
        assert!(service.launching.is_none());

        let mut newer = plan.clone();
        newer.operation_id = "b".repeat(32);
        service.launch_plan = Some(newer.clone());
        service.prepared_plan = Some(newer.clone());
        service.observed_operation_id = Some(newer.operation_id.clone());
        assert_eq!(
            service.journal(
                &record.operation_id,
                Ok(Some(record.clone())),
                Some(PreparedRecoveryProof {
                    record: record.clone(),
                    plan,
                }),
            ),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&newer));
        let mut blocked = record;
        blocked.operation_id = newer.operation_id.clone();
        blocked.phase = OperationPhase::Unknown;
        assert_eq!(
            service.journal(&newer.operation_id, Ok(Some(blocked)), None),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&newer));
    }

    #[test]
    fn failed_phase_alone_or_a_different_prepared_image_cannot_thaw() {
        let record = failed_record('c');
        let mut plan = local_check().plan.unwrap();
        plan.operation_id = record.operation_id.clone();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.context = Some(plan.context.clone());
        service.observed_operation_id = Some(record.operation_id.clone());
        service.prepared_plan = Some(plan.clone());
        assert_eq!(
            service.journal(&record.operation_id, Ok(Some(record.clone())), None),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        let mut other_image = plan.clone();
        other_image.context.running.sha256 = "different original".into();
        assert_eq!(
            service.journal(
                &record.operation_id,
                Ok(Some(record.clone())),
                Some(PreparedRecoveryProof {
                    record: record.clone(),
                    plan: other_image,
                }),
            ),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        let mut intent = record;
        intent.execution_fence = ExecutionFence::ManagerIntent;
        let operation_id = intent.operation_id.clone();
        assert_eq!(service.journal(&operation_id, Ok(Some(intent)), None), None);
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
    }

    #[test]
    fn fresh_check_retires_only_the_proven_reconciled_failure_and_exposes_rebuild() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let prior = failed_record('a');
        service.latest_journal(Ok(Some(prior.clone())));
        assert!(service.plan().is_none());
        assert_eq!(
            service.checked(
                Ok(local_check()),
                Some(SettledOperationProof(prior.clone()))
            ),
            Some(42)
        );
        assert_eq!(
            service.plan().map(|plan| &plan.action),
            Some(&UpdateAction::LocalRebuild)
        );
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Local);
        let history = service.model(UiLocale::En).history;
        assert!(!service.model(UiLocale::En).busy);
        assert_eq!(
            service.model(UiLocale::En).action,
            Some(UpdateAction::LocalRebuild)
        );
        service.latest_journal(Ok(Some(prior.clone())));
        let prior_id = prior.operation_id.clone();
        service.journal(&prior_id, Ok(Some(prior)), None);
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Local);
        assert_eq!(service.model(UiLocale::En).history, history);
        assert!(service.plan().is_some());
    }

    #[test]
    fn checked_settlement_keeps_late_startup_history_from_blocking_a_new_plan() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let prior = failed_record('a');
        service.checked(
            Ok(local_check()),
            Some(SettledOperationProof(prior.clone())),
        );
        service.latest_journal(Ok(Some(prior)));
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Local);
        assert_eq!(
            service.plan().map(|plan| &plan.action),
            Some(&UpdateAction::LocalRebuild)
        );
    }

    #[test]
    fn late_unproven_startup_history_still_blocks_replacement() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.checked(Ok(local_check()), None);
        let mut prior = failed_record('a');
        prior.phase = OperationPhase::Unknown;
        service.latest_journal(Ok(Some(prior)));
        assert!(service.plan().is_none());
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Failed);
    }

    #[test]
    fn reconciled_user_stop_can_offer_a_new_explicit_plan() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let mut prior = failed_record('f');
        prior.phase = OperationPhase::UserStopped;
        assert!(settled_terminal(&prior));
        service.latest_journal(Ok(Some(prior.clone())));
        assert!(service.plan().is_none());
        service.checked(Ok(local_check()), Some(SettledOperationProof(prior)));
        assert_eq!(
            service.plan().map(|plan| &plan.action),
            Some(&UpdateAction::LocalRebuild)
        );
    }

    #[test]
    fn clearance_rejects_busy_lease_legacy_journal_and_retained_profile_marker() {
        use herdr_update_coordinator::protocol::{
            write_operation, write_private_bytes, write_private_new,
        };
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let state_dir = temporary.path().join("state");
        fs::create_dir(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let updates = private_updates(&state_dir).unwrap();
        let mut snapshot = local_check();
        let old = failed_record('f');
        let mut old_plan = snapshot.plan.as_ref().unwrap().clone();
        old_plan.operation_id = old.operation_id.clone();
        old_plan.context.state_dir = state_dir.clone();
        snapshot.plan.as_mut().unwrap().context.state_dir = state_dir.clone();
        write_private_new(
            &updates.join(format!("{}.plan.json", old.operation_id)),
            &serde_json::to_vec(&old_plan).unwrap(),
            0o600,
        )
        .unwrap();
        write_operation(&state_dir, &old).unwrap();

        let lease = lock_operation(&state_dir, &old.operation_id).unwrap();
        assert!(settlement_proof(&old_plan.context, &snapshot, &old.operation_id).is_err());
        assert!(prepared_recovery_proof(&old_plan.context, &old.operation_id, &old).is_err());
        drop(lease);

        let mut wrong_original = old_plan.context.clone();
        wrong_original.instance_id = "other-original".into();
        assert!(prepared_recovery_proof(&wrong_original, &old.operation_id, &old).is_err());
        for (phase, fence) in [
            (OperationPhase::Unknown, ExecutionFence::NoSpawn),
            (OperationPhase::Failed, ExecutionFence::ManagerIntent),
        ] {
            let mut unsafe_outcome = old.clone();
            unsafe_outcome.phase = phase;
            unsafe_outcome.execution_fence = fence;
            assert!(
                prepared_recovery_proof(&old_plan.context, &old.operation_id, &unsafe_outcome)
                    .is_err()
            );
        }
        let mut legacy = old.clone();
        legacy.version = 1;
        write_private_bytes(
            &updates.join(format!("{}.result.json", old.operation_id)),
            &serde_json::to_vec(&legacy).unwrap(),
            0o600,
        )
        .unwrap();
        assert!(settlement_proof(&old_plan.context, &snapshot, &old.operation_id).is_err());
        assert!(prepared_recovery_proof(&old_plan.context, &old.operation_id, &old).is_err());

        write_private_bytes(
            &updates.join(format!("{}.result.json", old.operation_id)),
            &serde_json::to_vec(&old).unwrap(),
            0o600,
        )
        .unwrap();
        write_private_bytes(
            &updates.join("profile.reservation.json"),
            br#"{"version":1}"#,
            0o600,
        )
        .unwrap();
        assert!(settlement_proof(&old_plan.context, &snapshot, &old.operation_id).is_err());
        assert!(prepared_recovery_proof(&old_plan.context, &old.operation_id, &old).is_err());
    }

    #[test]
    fn stopped_original_restoration_history_requires_independent_settlement_before_beta_admission()
    {
        use herdr_update_coordinator::protocol::{
            reserve_install, write_operation, write_private_new,
        };
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let state_dir = temporary.path().join("state");
        fs::create_dir(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let root = temporary.path().join("source");
        fs::create_dir(&root).unwrap();
        let mut snapshot = local_check();
        snapshot.origin = InstallOrigin::Local { root: root.clone() };
        let next_plan = snapshot.plan.as_mut().unwrap();
        next_plan.origin = snapshot.origin.clone();
        next_plan.running.path = root.join("app");
        next_plan.context.running = next_plan.running.clone();
        next_plan.context.executable = next_plan.running.path.clone();
        next_plan.context.running_origin = Some(Box::new(snapshot.origin.clone()));
        next_plan.context.state_dir = state_dir.clone();
        let context = next_plan.context.clone();
        let mut stopped = failed_record('f');
        stopped.phase = OperationPhase::UserStopped;
        stopped.execution_fence = ExecutionFence::ManagerExited;
        stopped.candidate = Some(ExecutableIdentity {
            path: next_plan.running.path.clone(),
            sha256: "historical candidate image I".into(),
        });
        let mut prior_plan = next_plan.clone();
        prior_plan.operation_id = stopped.operation_id.clone();
        let updates = private_updates(&state_dir).unwrap();
        let plan_path = updates.join(format!("{}.plan.json", stopped.operation_id));
        write_private_new(&plan_path, &serde_json::to_vec(&prior_plan).unwrap(), 0o600).unwrap();
        let result_path = write_operation(&state_dir, &stopped).unwrap();
        let plan_bytes = fs::read(&plan_path).unwrap();
        let result_bytes = fs::read(&result_path).unwrap();

        let reservation =
            reserve_install(&prior_plan.origin, None, &state_dir, &stopped.operation_id).unwrap();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.latest_journal(Ok(Some(stopped.clone())));
        assert!(!service.settings_admitted());
        assert!(settlement_proof(&context, &snapshot, &stopped.operation_id).is_err());
        service.checked(Ok(snapshot.clone()), None);
        service.change_policy(UpdatePolicy::StableAndBeta, None);
        assert_eq!(service.policy(), UpdatePolicy::StableOnly);
        assert!(!service.settings_admitted());

        reservation.release().unwrap();
        let operation_lease = lock_operation(&state_dir, &stopped.operation_id).unwrap();
        assert!(settlement_proof(&context, &snapshot, &stopped.operation_id).is_err());
        drop(operation_lease);
        let mut mismatched = snapshot.clone();
        mismatched.origin = InstallOrigin::Unknown {
            reason: "original source did not match fresh discovery".into(),
        };
        assert!(settlement_proof(&context, &mismatched, &stopped.operation_id).is_err());
        let proof = settlement_proof(&context, &snapshot, &stopped.operation_id).unwrap();
        service.checked(Ok(snapshot.clone()), Some(proof));
        assert!(service.settings_admitted());
        assert_eq!(
            service.retired_operation_id.as_deref(),
            Some(stopped.operation_id.as_str())
        );
        assert_eq!(service.previous_outcome.as_ref(), Some(&stopped));
        assert_eq!(
            service.plan().map(|plan| &plan.action),
            Some(&UpdateAction::LocalRebuild)
        );
        service.change_policy(UpdatePolicy::StableAndBeta, None);
        assert_eq!(service.policy(), UpdatePolicy::StableAndBeta);
        service.latest_journal(Ok(Some(stopped.clone())));
        assert!(service.settings_admitted());
        assert_eq!(fs::read(&plan_path).unwrap(), plan_bytes);
        assert_eq!(fs::read(&result_path).unwrap(), result_bytes);

        let old_proof = settlement_proof(&context, &snapshot, &stopped.operation_id).unwrap();
        let mut newer = failed_record('a');
        newer.phase = OperationPhase::Installing;
        newer.execution_fence = ExecutionFence::ManagerIntent;
        let mut newer_service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        newer_service.latest_journal(Ok(Some(stopped.clone())));
        newer_service.latest_journal(Ok(Some(newer.clone())));
        newer_service.checked(Ok(snapshot), Some(old_proof));
        assert_eq!(newer_service.operation.as_ref(), Some(&newer));
        assert!(!newer_service.settings_admitted());
        assert!(newer_service.retired_operation_id.is_none());
    }

    #[test]
    fn completed_current_check_without_candidate_unlocks_release_policy() {
        use herdr_update_coordinator::protocol::{write_operation, write_private_new};
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let state_dir = temporary.path().join("state");
        fs::create_dir(&state_dir).unwrap();
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let root = temporary.path().join("source");
        fs::create_dir(&root).unwrap();
        let mut snapshot = local_check();
        snapshot.origin = InstallOrigin::Local { root };
        snapshot.status = CheckStatus::Current;
        snapshot.policy = UpdatePolicy::StableAndBeta;
        let mut prior_plan = snapshot.plan.take().unwrap();
        prior_plan.context.state_dir = state_dir.clone();
        prior_plan.context.running_origin = Some(Box::new(snapshot.origin.clone()));
        prior_plan.origin = snapshot.origin.clone();
        let mut completed = failed_record('f');
        completed.phase = OperationPhase::Completed;
        completed.execution_fence = ExecutionFence::ManagerExited;
        completed.installed = true;
        completed.applied = true;
        completed.candidate = Some(prior_plan.running.clone());
        prior_plan.operation_id = completed.operation_id.clone();
        let updates = private_updates(&state_dir).unwrap();
        let plan_path = updates.join(format!("{}.plan.json", completed.operation_id));
        write_private_new(&plan_path, &serde_json::to_vec(&prior_plan).unwrap(), 0o600).unwrap();
        write_operation(&state_dir, &completed).unwrap();
        let result_path = updates.join(format!("{}.result.json", completed.operation_id));
        let result_bytes = fs::read(&result_path).unwrap();
        let plan_bytes = fs::read(&plan_path).unwrap();
        let context = &prior_plan.context;

        let lease = lock_operation(&state_dir, &completed.operation_id).unwrap();
        assert!(settlement_proof(context, &snapshot, &completed.operation_id).is_err());
        drop(lease);
        let mut mismatched = snapshot.clone();
        mismatched.origin = InstallOrigin::Unknown {
            reason: "running source is not the checked source".into(),
        };
        assert!(settlement_proof(context, &mismatched, &completed.operation_id).is_err());
        let proof = settlement_proof(context, &snapshot, &completed.operation_id).unwrap();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableAndBeta, None);
        service.latest_journal(Ok(Some(completed.clone())));
        assert!(!service.settings_admitted());
        service.checked(Ok(snapshot), Some(proof));
        assert!(service.settings_admitted());
        assert!(service.plan().is_none());
        service.change_policy(UpdatePolicy::StableOnly, None);
        assert_eq!(service.policy(), UpdatePolicy::StableOnly);
        assert_eq!(service.previous_outcome.as_ref(), Some(&completed));
        assert_eq!(fs::read(&result_path).unwrap(), result_bytes);
        assert_eq!(fs::read(&plan_path).unwrap(), plan_bytes);
    }

    #[test]
    fn unknown_active_and_unreleased_outcomes_remain_blocked_after_check() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let mut unknown = failed_record('a');
        unknown.phase = OperationPhase::Unknown;
        assert!(!settled_terminal(&unknown));
        service.latest_journal(Ok(Some(unknown)));
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());

        let mut active = failed_record('b');
        active.phase = OperationPhase::Installing;
        active.execution_fence = ExecutionFence::ManagerIntent;
        assert!(!settled_terminal(&active));
        service.latest_journal(Ok(Some(active)));
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());

        let failed_with_retained_marker = failed_record('c');
        service.latest_journal(Ok(Some(failed_with_retained_marker.clone())));
        // A failed proof (retained profile/global marker, busy lease or legacy bytes)
        // does not supply a settlement token, even though the journal is terminal.
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());
        let mut intent = failed_with_retained_marker;
        intent.execution_fence = ExecutionFence::ManagerIntent;
        assert!(!settled_terminal(&intent));
    }

    #[test]
    fn late_settlement_cannot_retire_a_newer_operation() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let old = failed_record('a');
        service.latest_journal(Ok(Some(old.clone())));
        let mut current = failed_record('b');
        current.phase = OperationPhase::Installing;
        current.execution_fence = ExecutionFence::ManagerIntent;
        service.latest_journal(Ok(Some(current.clone())));
        assert_eq!(service.previous_outcome.as_ref(), Some(&old));
        service.latest_journal(Ok(Some(old.clone())));
        assert_eq!(service.operation.as_ref(), Some(&current));
        service.checked(Ok(local_check()), Some(SettledOperationProof(old)));
        assert_eq!(service.operation.as_ref(), Some(&current));
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(current.operation_id.as_str())
        );
        assert!(service.retired_operation_id.is_none());
        assert!(service.plan().is_none());
    }

    #[test]
    fn failed_check_does_not_advance_successful_check_time() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, Some(11));
        assert_eq!(
            service.checked(Err("untranslated /isolated/check-failure".into()), None),
            None
        );
        let ko = service.model(UiLocale::Ko);
        assert_eq!(ko.last_checked, Some(11));
        assert_eq!(ko.status, CheckStatus::Failed);
        assert!(!ko.detail.contains("untranslated"));
        assert!(!ko.status_text.contains("/isolated/"));
        assert!(ko
            .diagnostics
            .contains("untranslated /isolated/check-failure"));
    }

    #[test]
    fn unresolved_active_journal_outweighs_a_successful_read_only_check() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, Some(11));
        service.latest_journal(Err("reserved operation has no valid journal".into()));
        service.checked(Ok(local_check()), None);
        let model = service.model(UiLocale::En);
        assert_eq!(model.status, CheckStatus::Failed);
        assert!(model
            .diagnostics
            .contains("reserved operation has no valid journal"));
        assert!(!model.detail.contains("reserved operation"));
        assert!(service.plan().is_none());
    }

    #[test]
    fn installation_during_restart_is_not_reported_as_applied() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.latest_journal(Ok(Some(OperationRecord {
            version: UPDATER_PROTOCOL,
            execution_fence: ExecutionFence::ManagerExited,
            operation_id: "a".repeat(32),
            phase: OperationPhase::Starting,
            detail: "new instance not yet verified".into(),
            installed: true,
            applied: false,
            candidate: None,
        })));
        let model = service.model(UiLocale::En);
        assert_eq!(model.status, CheckStatus::Checking);
        assert!(model.busy);
        assert!(model.action.is_none());
        assert!(model.history.is_empty());
        assert!(!model.detail.contains("new instance not yet verified"));
        assert!(model.diagnostics.contains("new instance not yet verified"));
    }

    #[test]
    fn completed_applied_is_a_historical_result_not_latest_or_restart_pending() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let operation_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let record = OperationRecord {
            operation_id: operation_id.into(),
            version: UPDATER_PROTOCOL,
            execution_fence: ExecutionFence::ManagerExited,
            phase: OperationPhase::Completed,
            detail: "replacement confirmed".into(),
            installed: true,
            applied: true,
            candidate: None,
        };
        service.latest_journal(Ok(Some(record.clone())));
        let revision = service.revision();
        let model = service.model(UiLocale::En);
        assert_eq!(model.status, CheckStatus::NotChecked);
        assert!(!model.busy);
        assert!(model.action.is_none());
        service.journal(operation_id, Ok(Some(record)), None);
        assert_eq!(service.revision(), revision);
    }

    #[test]
    fn completed_history_survives_check_retirement_and_never_replaces_current_discovery() {
        let snapshot = local_check();
        let context = snapshot.plan.as_ref().unwrap().context.clone();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.context = Some(context);
        let mut completed = failed_record('b');
        completed.phase = OperationPhase::Completed;
        completed.execution_fence = ExecutionFence::ManagerExited;
        completed.installed = true;
        completed.applied = true;
        completed.detail = "private /isolated/operation/result".into();
        completed.candidate = Some(ExecutableIdentity {
            path: PathBuf::from("/isolated/secret/new-app"),
            sha256: "private candidate sha".into(),
        });
        service.latest_journal(Ok(Some(completed.clone())));
        service.check_busy = true;
        let checking = service.model(UiLocale::Ko);
        assert!(checking.busy);
        assert_eq!(checking.status, CheckStatus::Checking);
        assert!(checking.action.is_none());
        let history = checking.history;

        service.checked(Ok(snapshot), Some(SettledOperationProof(completed.clone())));
        let discovered = service.model(UiLocale::Ko);
        assert!(!discovered.busy);
        assert_eq!(discovered.status, CheckStatus::Local);
        assert_eq!(discovered.action, Some(UpdateAction::LocalRebuild));
        assert_eq!(discovered.history, history);
        assert_eq!(
            discovered.running_version,
            service.context.as_ref().unwrap().version
        );
        for summary in [
            &discovered.status_text,
            &discovered.source,
            &discovered.detail,
            &discovered.history,
        ] {
            assert!(!summary.contains("/isolated/"));
            assert!(!summary.contains("private"));
        }
        assert!(discovered
            .diagnostics
            .contains("/isolated/local-source/app"));
        assert!(discovered.diagnostics.contains("/isolated/secret/new-app"));
        assert!(discovered.diagnostics.contains("private candidate sha"));
        assert!(discovered
            .diagnostics
            .contains("private /isolated/operation/result"));
        service.latest_journal(Ok(Some(completed)));
        assert!(service.plan().is_some());
        assert_eq!(service.model(UiLocale::Ko).history, history);

        let mut current = local_check();
        current.status = CheckStatus::Current;
        current.plan = None;
        service.checked(Ok(current), None);
        let current = service.model(UiLocale::Ko);
        assert_eq!(current.status, CheckStatus::Current);
        assert!(!current.busy);
        assert!(current.action.is_none());
        assert_eq!(current.history, history);
    }

    #[test]
    fn homebrew_source_shows_formula_without_exposing_install_paths() {
        let mut snapshot = local_check();
        let origin = InstallOrigin::Homebrew {
            brew: PathBuf::from("/isolated/private/bin/brew"),
            formula: "example/tap/herdr-pet".into(),
            cellar_formula_root: PathBuf::from("/isolated/private/Cellar/herdr-pet"),
            prefix: PathBuf::from("/isolated/private/prefix"),
            locator: PathBuf::from("/isolated/private/app"),
        };
        snapshot.origin = origin.clone();
        snapshot.plan = None;
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.checked(Ok(snapshot), None);
        for locale in [UiLocale::En, UiLocale::Ko] {
            let model = service.model(locale);
            assert_eq!(model.source, "Homebrew · example/tap/herdr-pet");
            assert!(!model.source.contains("/isolated/"));
            assert!(model.diagnostics.contains("/isolated/private/bin/brew"));
            assert!(model.diagnostics.contains("/isolated/private/app"));
        }
        assert!(source_description(&origin).contains("/isolated/private/bin/brew"));
        assert!(source_description(&origin).contains("/isolated/private/app"));
    }

    #[test]
    fn newer_completed_operation_replaces_retained_history() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let prior = failed_record('a');
        service.latest_journal(Ok(Some(prior.clone())));
        service.checked(Ok(local_check()), Some(SettledOperationProof(prior)));
        let prior_history = service.model(UiLocale::En).history;

        let mut newer = failed_record('b');
        newer.phase = OperationPhase::Starting;
        newer.execution_fence = ExecutionFence::ManagerIntent;
        service.latest_journal(Ok(Some(newer.clone())));
        let active = service.model(UiLocale::En);
        assert_eq!(active.status, CheckStatus::Checking);
        assert!(active.busy);
        assert!(active.action.is_none());
        assert_eq!(active.history, prior_history);

        newer.phase = OperationPhase::Completed;
        newer.execution_fence = ExecutionFence::ManagerExited;
        newer.installed = true;
        newer.applied = true;
        service.journal(&newer.operation_id, Ok(Some(newer.clone())), None);
        let completed = service.model(UiLocale::En);
        assert_eq!(completed.status, CheckStatus::Local);
        assert!(!completed.busy);
        assert!(completed.action.is_none());
        assert_ne!(completed.history, prior_history);

        service.checked(Ok(local_check()), Some(SettledOperationProof(newer)));
        let retired = service.model(UiLocale::En);
        assert_eq!(retired.status, CheckStatus::Local);
        assert_eq!(retired.action, Some(UpdateAction::LocalRebuild));
        assert_eq!(retired.history, completed.history);
    }

    #[test]
    fn unresolved_outcomes_remain_blocked_without_admission() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let mut record = failed_record('a');
        record.phase = OperationPhase::Unknown;
        record.detail = "raw /isolated/uncertain".into();
        service.latest_journal(Ok(Some(record.clone())));
        service.checked(Ok(local_check()), None);
        let unknown = service.model(UiLocale::Ko);
        assert_eq!(unknown.status, CheckStatus::Failed);
        assert!(!unknown.busy);
        assert!(unknown.action.is_none());
        assert!(unknown.diagnostics.contains("raw /isolated/uncertain"));
        assert!(!unknown.detail.contains("/isolated/"));

        record.phase = OperationPhase::UserStopped;
        service.journal(&record.operation_id, Ok(Some(record.clone())), None);
        service.checked(Ok(local_check()), None);
        let stopped = service.model(UiLocale::Ko);
        assert_eq!(stopped.status, CheckStatus::Failed);
        assert!(stopped.action.is_none());
        assert!(!stopped.detail.contains("/isolated/"));

        record.phase = OperationPhase::Failed;
        service.journal(&record.operation_id, Ok(Some(record.clone())), None);
        assert_eq!(
            service.operation.as_ref().unwrap().phase,
            OperationPhase::UserStopped
        );
    }

    #[test]
    fn journal_transitions_advance_revision_without_duplicate_paints() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        let operation_id = "cccccccccccccccccccccccccccccccc";
        let mut record = OperationRecord {
            operation_id: operation_id.into(),
            version: UPDATER_PROTOCOL,
            execution_fence: ExecutionFence::ManagerIntent,
            phase: OperationPhase::Installing,
            detail: "working".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        service.latest_journal(Ok(Some(record.clone())));
        let revision = service.revision();
        service.journal(operation_id, Ok(Some(record.clone())), None);
        assert_eq!(service.revision(), revision);
        record.phase = OperationPhase::Failed;
        record.detail = "apply failed".into();
        service.journal(operation_id, Ok(Some(record)), None);
        assert_ne!(service.revision(), revision);
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Failed);
    }
    #[test]
    fn ownership_ack_keeps_live_operation_busy_and_paints_pre_stop_journal() {
        let (mut service, plan) = pending_local_launch();
        let operation_id = plan.operation_id.as_str();
        assert!(service.launch_result(operation_id, Ok(())));
        assert!(service.model(UiLocale::En).busy);
        let mut record = OperationRecord {
            version: UPDATER_PROTOCOL,
            execution_fence: ExecutionFence::NoSpawn,
            operation_id: operation_id.into(),
            phase: OperationPhase::Preparing,
            detail: "awaiting original Stop".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        service.journal(operation_id, Ok(Some(record.clone())), None);
        let card = service.model(UiLocale::En);
        assert!(card.busy);
        assert_eq!(card.status, CheckStatus::Checking);
        assert!(card.diagnostics.contains("awaiting original Stop"));
        assert!(!card.detail.contains("awaiting original Stop"));
        record.phase = OperationPhase::Failed;
        record.detail = "Stop reply failed".into();
        service.journal(operation_id, Ok(Some(record)), None);
        let failed = service.model(UiLocale::En);
        assert!(failed.busy); // The accepted preparation still owns its exact plan.
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(!service.settings_admitted());
        assert_eq!(failed.status, CheckStatus::Failed);
        assert!(failed.diagnostics.contains("Stop reply failed"));
        assert!(!failed.detail.contains("Stop reply failed"));
    }
    fn pending_local_launch() -> (AppUpdateService, UpdatePlan) {
        let snapshot = local_check();
        let plan = snapshot.plan.as_ref().unwrap().clone();
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, None);
        service.context = Some(plan.context.clone());
        service.checked(Ok(snapshot), None);
        assert!(service.plan().is_some());
        service.launching = Some(plan.operation_id.clone());
        service.observed_operation_id = Some(plan.operation_id.clone());
        service.launch_plan = Some(plan.clone());
        service.launch_generation = Some(service.consent_generation());
        (service, plan)
    }

    #[test]
    fn failed_before_spawn_retires_only_its_missing_journal_and_fresh_check_admits_apply() {
        let (mut service, plan) = pending_local_launch();
        service.journal(&plan.operation_id, Ok(None), None);
        assert!(service.plan().is_none());
        assert!(!service.launch_result(
            &plan.operation_id,
            Err(LaunchFailure::NoSpawn("staging failed".into())),
        ));
        assert!(service.plan().is_none()); // Error remains visible until a fresh check.
        service.checked(Ok(local_check()), None);
        assert_eq!(service.plan(), Some(&plan));
        assert_eq!(
            service.model(UiLocale::En).action,
            Some(UpdateAction::LocalRebuild)
        );
        service.journal(&plan.operation_id, Err("late poll".into()), None);
        assert_eq!(service.plan(), Some(&plan));
    }

    #[test]
    fn failed_after_spawn_keeps_uncertain_ownership_blocked_after_fresh_check() {
        let (mut service, plan) = pending_local_launch();
        service.journal(&plan.operation_id, Ok(None), None);
        assert!(!service.launch_result(
            &plan.operation_id,
            Err(LaunchFailure::Spawned("ownership reply lost".into())),
        ));
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.launch_plan.is_none());
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(plan.operation_id.as_str())
        );
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Failed);
    }

    #[test]
    fn spawned_stdout_failure_retains_accepted_prepare_for_later_locked_recovery() {
        let (mut service, plan) = pending_local_launch();
        service.launch_waiting(&plan.operation_id);
        assert_eq!(service.preparing_plan(&plan.operation_id), Some(&plan));
        assert!(!service.launch_result(
            &plan.operation_id,
            Err(LaunchFailure::Spawned("ownership stdout closed".into())),
        ));
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.launch_plan.is_none());
        let finished = failed_record('e');
        service.journal(&plan.operation_id, Ok(Some(finished.clone())), None);
        service.checked(
            Ok(local_check()),
            Some(SettledOperationProof(finished.clone())),
        );
        assert!(!service.settings_admitted());
        let mut wrong = plan.clone();
        wrong.running.sha256 = "another original".into();
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(finished.clone())),
                Some(PreparedRecoveryProof {
                    record: finished.clone(),
                    plan: wrong,
                }),
            ),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(finished.clone())),
                Some(PreparedRecoveryProof {
                    record: finished.clone(),
                    plan: plan.clone(),
                }),
            ),
            Some(plan.context.clone())
        );
        service.recovery_reconciled(&plan.operation_id);
        service.checked(Ok(local_check()), Some(SettledOperationProof(finished)));
        assert!(service.settings_admitted());
        assert_eq!(service.plan(), Some(&plan));
    }

    #[test]
    fn spawned_without_any_journal_still_cannot_apply_after_successful_check() {
        let (mut service, plan) = pending_local_launch();
        assert!(!service.launch_result(
            &plan.operation_id,
            Err(LaunchFailure::Spawned("no ownership confirmation".into())),
        ));
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());
        assert_eq!(service.model(UiLocale::En).action, None);
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.launch_plan.is_none());
    }

    #[test]
    fn no_spawn_does_not_hide_an_independent_unverified_journal() {
        let (mut service, plan) = pending_local_launch();
        service.latest_journal(Err("other operation cannot be verified".into()));
        assert!(!service.launch_result(
            &plan.operation_id,
            Err(LaunchFailure::NoSpawn("staging failed".into())),
        ));
        service.checked(Ok(local_check()), None);
        assert!(service.plan().is_none());
        let model = service.model(UiLocale::En);
        assert_eq!(model.status, CheckStatus::Failed);
        assert!(model
            .diagnostics
            .contains("other operation cannot be verified"));
        assert!(!model.detail.contains("other operation cannot be verified"));
    }

    #[test]
    fn ack_then_unknown_then_exact_recovery_keeps_original_plan_until_verified() {
        let (mut service, plan) = pending_local_launch();
        assert!(service.launch_result(&plan.operation_id, Ok(())));
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        let mut unknown = failed_record('e');
        unknown.phase = OperationPhase::Unknown;
        assert_eq!(unknown.operation_id, plan.operation_id);
        assert_eq!(
            service.journal(&plan.operation_id, Ok(Some(unknown.clone())), None),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.plan().is_none());

        let recovered = unknown;
        let mut wrong_plan = plan.clone();
        wrong_plan.context.instance_id = "different original".into();
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(recovered.clone())),
                Some(PreparedRecoveryProof {
                    record: recovered.clone(),
                    plan: wrong_plan,
                }),
            ),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.plan().is_none());
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(recovered.clone())),
                Some(PreparedRecoveryProof {
                    record: recovered,
                    plan: plan.clone(),
                }),
            ),
            Some(plan.context.clone())
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        service.recovery_reconciled(&plan.operation_id);
        assert!(service.prepared_plan.is_none());
    }
    #[test]
    fn stale_checked_result_cannot_replace_current_request_or_retire_an_operation() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, Some(19));
        let active = failed_record('a');
        service.latest_journal(Ok(Some(active.clone())));
        service.pending_request = Some(12);
        service.check_busy = true;
        let revision = service.revision();
        assert_eq!(
            service.checked_event(
                11,
                0,
                Ok(local_check()),
                Some(SettledOperationProof(active.clone()))
            ),
            None
        );
        assert_eq!(service.pending_request, Some(12));
        assert!(service.check_busy);
        assert_eq!(service.last_checked, Some(19));
        assert!(service.snapshot.is_none());
        assert_eq!(service.operation.as_ref(), Some(&active));
        assert!(service.retired_operation_id.is_none());
        assert_eq!(service.revision(), revision);
        assert_eq!(
            service.checked_event(12, 0, Ok(local_check()), None),
            Some(42)
        );
        assert!(!service.check_busy);
    }

    #[test]
    fn direct_setting_admission_blocks_active_checks_preparation_and_unsettled_reservations() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, Some(19));
        let mut pending = failed_record('a');
        pending.phase = OperationPhase::Installing;
        for block in 0..4 {
            match block {
                0 => service.check_busy = true,
                1 => service.launching = Some(pending.operation_id.clone()),
                2 => service.prepared_plan = local_check().plan,
                _ => service.latest_journal(Ok(Some(pending.clone()))),
            }
            let before = service.policy();
            service.change_policy(UpdatePolicy::StableAndBeta, Some(29));
            service.set_auto_check(true);
            assert_eq!(service.policy(), before);
            assert!(!service.auto_check);
            service.check_busy = false;
            service.launching = None;
            service.prepared_plan = None;
            service.operation = None;
            service.observed_operation_id = None;
        }
    }

    #[test]
    fn confirmation_rejects_candidate_or_policy_changed_after_snapshot() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableOnly, Some(19));
        let original = local_check();
        let plan = original.plan.clone().unwrap();
        service.checked(Ok(original), None);
        let generation = service.consent_generation();
        assert!(service.consent_matches(&plan, generation));
        let mut replacement = local_check();
        let asset = "HerdrDesktopPet-v0.3.3-macos-arm64.tar.gz";
        let base = "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/v0.3.3/";
        replacement.release = Some(ReleaseCandidate {
            version: "0.3.3".into(),
            channel: ReleaseChannel::Stable,
            tag: "v0.3.3".into(),
            release_id: 42,
            asset_name: asset.into(),
            archive_url: format!("{base}{asset}"),
            archive_size: 1234,
            sha256: "a".repeat(64),
            checksum_url: format!("{base}{asset}.sha256"),
        });
        service.checked(Ok(replacement.clone()), None);
        assert_eq!(service.plan(), Some(&plan));
        assert!(!service.consent_matches(&plan, generation));
        let refreshed_generation = service.consent_generation();
        replacement.plan.as_mut().unwrap().operation_id = "d".repeat(32);
        service.checked(Ok(replacement), None);
        assert!(!service.consent_matches(&plan, refreshed_generation));
        service.change_policy(UpdatePolicy::StableAndBeta, Some(23));
        assert!(service.plan().is_none());
        assert!(!service.consent_matches(&plan, generation));
        assert_eq!(service.last_checked, Some(23));
    }
    #[test]
    fn delayed_final_ack_notices_once_then_valid_reply_preserves_ownership() {
        let operation_id = "e".repeat(32);
        let valid = OwnershipAck {
            protocol: UPDATER_PROTOCOL,
            operation_id: operation_id.clone(),
            visible: true,
            prepared: true,
            error: None,
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut notices = 0;
        assert!(
            await_ownership_ack(receiver, &operation_id, Duration::ZERO, || {
                notices += 1;
                sender.send(Ok(valid)).unwrap();
            })
            .is_ok()
        );
        assert_eq!(notices, 1);

        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Ok(OwnershipAck {
                protocol: UPDATER_PROTOCOL,
                operation_id: operation_id.clone(),
                visible: true,
                prepared: true,
                error: None,
            }))
            .unwrap();
        assert!(
            await_ownership_ack(receiver, &operation_id, Duration::ZERO, || {
                notices += 1;
            })
            .is_ok()
        );
        assert_eq!(notices, 1);
    }

    #[test]
    fn negative_or_broken_final_ack_is_spawned_failure_even_after_waiting() {
        let operation_id = "e".repeat(32);
        for (protocol, id, visible, prepared, error) in [
            (UPDATER_PROTOCOL + 1, operation_id.clone(), true, true, None),
            (UPDATER_PROTOCOL, "f".repeat(32), true, true, None),
            (UPDATER_PROTOCOL, operation_id.clone(), false, true, None),
            (UPDATER_PROTOCOL, operation_id.clone(), true, false, None),
            (
                UPDATER_PROTOCOL,
                operation_id.clone(),
                true,
                true,
                Some("rejected".into()),
            ),
        ] {
            let (sender, receiver) = mpsc::sync_channel(1);
            assert!(matches!(
                await_ownership_ack(receiver, &operation_id, Duration::ZERO, || {
                    sender
                        .send(Ok(OwnershipAck {
                            protocol,
                            operation_id: id,
                            visible,
                            prepared,
                            error,
                        }))
                        .unwrap();
                }),
                Err(LaunchFailure::Spawned(_))
            ));
        }
        for bytes in [
            Vec::new(),
            b"{\"protocol\":3".to_vec(),
            b"{invalid}\n".to_vec(),
            vec![b'a'; 4096],
        ] {
            assert!(read_ownership_ack(bytes.as_slice()).is_err());
        }
        let (sender, receiver) = mpsc::sync_channel::<Result<OwnershipAck, String>>(1);
        drop(sender);
        assert!(matches!(
            await_ownership_ack(receiver, &operation_id, Duration::ZERO, || {}),
            Err(LaunchFailure::Spawned(_))
        ));
    }

    #[test]
    fn waiting_still_accepts_prepare_and_transfers_only_on_current_final_ack() {
        let (mut service, plan) = pending_local_launch();
        service.launch_waiting(&plan.operation_id);
        assert!(service.launch_waiting);
        assert_eq!(service.preparing_plan(&plan.operation_id), Some(&plan));
        assert!(service.prepared_plan.is_none());
        assert!(!service.launch_result("f0000000000000000000000000000000", Ok(())));
        assert_eq!(service.launch_plan.as_ref(), Some(&plan));
        assert!(service.launch_result(&plan.operation_id, Ok(())));
        assert!(service.launch_plan.is_none());
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.launching.is_none());
        assert!(!service.launch_waiting);
        assert!(!service.launch_result(&plan.operation_id, Ok(())));
        assert!(service.preparing_plan(&plan.operation_id).is_none());
        assert!(!service.settings_admitted());
    }

    #[test]
    fn waiting_retains_exact_consent_and_terminal_disallows_prepare_or_late_ack() {
        let (mut service, plan) = pending_local_launch();
        let generation = service.launch_generation;
        service.launch_waiting("f0000000000000000000000000000000");
        assert!(!service.launch_waiting);
        service.launch_waiting(&plan.operation_id);
        assert!(service.launch_waiting);
        assert_eq!(service.launch_plan.as_ref(), Some(&plan));
        assert!(service.prepared_plan.is_none());
        assert_eq!(service.launch_generation, generation);
        assert_eq!(service.preparing_plan(&plan.operation_id), Some(&plan));
        assert!(!service.settings_admitted());
        let waiting = service.model(UiLocale::En);
        assert_eq!(waiting.status, CheckStatus::Checking);
        assert!(waiting.diagnostics.contains(&waiting.status_text));
        service.latest_journal(Ok(Some(failed_record('b'))));
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(plan.operation_id.as_str())
        );
        assert_eq!(service.launch_plan.as_ref(), Some(&plan));

        let mut terminal = failed_record('e');
        terminal.phase = OperationPhase::UserStopped;
        service.journal(&plan.operation_id, Ok(Some(terminal.clone())), None);
        assert!(service.launch_plan.is_none());
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.preparing_plan(&plan.operation_id).is_none());
        assert!(!service.launch_waiting);
        assert!(!service.launch_result(&plan.operation_id, Ok(())));
        service.launch_waiting(&plan.operation_id);
        assert!(!service.launch_waiting);
        let mut old = terminal.clone();
        old.phase = OperationPhase::Preparing;
        service.journal(&plan.operation_id, Ok(Some(old)), None);
        assert_eq!(service.operation.as_ref(), Some(&terminal));
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(!service.settings_admitted());
        service.checked(
            Ok(local_check()),
            Some(SettledOperationProof(terminal.clone())),
        );
        assert!(!service.settings_admitted());
        service.preparation_cancelled(&plan.operation_id);
        service.checked(Ok(local_check()), Some(SettledOperationProof(terminal)));
        assert!(service.settings_admitted());
        assert_eq!(service.plan(), Some(&plan));
    }

    #[test]
    fn terminal_before_ack_waits_for_a_later_exact_recovery_proof() {
        let (mut service, plan) = pending_local_launch();
        service.launch_waiting(&plan.operation_id);
        let finished = failed_record('e');
        assert_eq!(
            service.journal(&plan.operation_id, Ok(Some(finished.clone())), None),
            None
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(!service.launch_result(&plan.operation_id, Ok(())));
        assert!(service.preparing_plan(&plan.operation_id).is_none());
        service.checked(
            Ok(local_check()),
            Some(SettledOperationProof(finished.clone())),
        );
        assert!(!service.settings_admitted());
        assert!(service.plan().is_none());
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(finished.clone())),
                Some(PreparedRecoveryProof {
                    record: finished.clone(),
                    plan: plan.clone(),
                }),
            ),
            Some(plan.context.clone())
        );
        assert!(!service.settings_admitted());
        service.recovery_reconciled(&plan.operation_id);
        service.checked(Ok(local_check()), Some(SettledOperationProof(finished)));
        assert!(service.settings_admitted());
        assert_eq!(service.plan(), Some(&plan));
    }

    #[test]
    fn final_ack_promotes_once_and_late_proof_holds_settings_until_reconciled_current_check() {
        let (mut service, plan) = pending_local_launch();
        assert_eq!(service.preparing_plan(&plan.operation_id), Some(&plan));
        assert!(service.prepared_plan.is_none());
        assert_eq!(
            service.launch_generation,
            Some(service.consent_generation())
        );
        assert!(service.launch_result(&plan.operation_id, Ok(())));
        assert!(service.launch_plan.is_none());
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(service.launching.is_none());
        assert!(!service.launch_result(&plan.operation_id, Ok(())));
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        // A late startup discovery of another ID cannot unpin the owned plan.
        let mut other = failed_record('b');
        other.phase = OperationPhase::Installing;
        service.latest_journal(Ok(Some(other)));
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(plan.operation_id.as_str())
        );
        let finished = failed_record('e');
        service.journal(&plan.operation_id, Ok(Some(finished.clone())), None);
        service.checked(
            Ok(local_check()),
            Some(SettledOperationProof(finished.clone())),
        );
        assert!(!service.settings_admitted());
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(plan.operation_id.as_str())
        );
        assert_eq!(
            service.journal(
                &plan.operation_id,
                Ok(Some(finished.clone())),
                Some(PreparedRecoveryProof {
                    record: finished.clone(),
                    plan: plan.clone(),
                }),
            ),
            Some(plan.context.clone())
        );
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        service.recovery_reconciled(&plan.operation_id);
        service.checked(Ok(local_check()), Some(SettledOperationProof(finished)));
        assert!(service.settings_admitted());
        assert_eq!(service.plan(), Some(&plan));
    }

    #[test]
    fn expired_final_ack_reports_failure_without_releasing_the_owned_plan() {
        let (mut service, plan) = pending_local_launch();
        assert!(service.launch_result(&plan.operation_id, Ok(())));
        service.abort_launch("f0000000000000000000000000000000", "wrong operation".into());
        assert!(service.problem.is_none());
        service.abort_launch(&plan.operation_id, "native preparation expired".into());
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert_eq!(
            service.observed_operation_id.as_deref(),
            Some(plan.operation_id.as_str())
        );
        assert_eq!(
            service.problem.as_deref(),
            Some("native preparation expired")
        );
        assert!(!service.settings_admitted());
        assert!(service.plan().is_none());
    }

    #[test]
    fn native_cancellation_retains_plan_until_exact_independent_settlement() {
        let (mut service, plan) = pending_local_launch();
        service.preparation_cancelled("f0000000000000000000000000000000");
        assert!(service.cancelled_preparation.is_none());
        service.preparation_cancelled(&plan.operation_id);
        assert_eq!(service.launch_plan.as_ref(), Some(&plan));
        assert_eq!(
            service.cancelled_preparation.as_deref(),
            Some(plan.operation_id.as_str())
        );
        assert!(service.launch_result(&plan.operation_id, Ok(())));
        let finished = failed_record('e');
        service.journal(&plan.operation_id, Ok(Some(finished.clone())), None);
        service.checked(Ok(local_check()), None);
        assert_eq!(service.prepared_plan.as_ref(), Some(&plan));
        assert!(!service.settings_admitted());
        let mut current = local_check();
        current.status = CheckStatus::Current;
        current.plan = None;
        service.checked(Ok(current), Some(SettledOperationProof(finished)));
        assert!(service.prepared_plan.is_none());
        assert!(service.settings_admitted());
        assert_eq!(service.model(UiLocale::En).status, CheckStatus::Current);
    }

    #[test]
    fn changed_consent_and_inflight_poll_cannot_reauthorize_preparing() {
        let (mut service, plan) = pending_local_launch();
        service.journal_in_flight = Some(plan.operation_id.clone());
        service.next_journal_poll = Instant::now();
        service.poll_operation();
        assert_eq!(
            service.journal_in_flight.as_deref(),
            Some(plan.operation_id.as_str())
        );
        service.candidate_generation += 1;
        assert!(service.preparing_plan(&plan.operation_id).is_none());
        assert_eq!(service.launch_plan.as_ref(), Some(&plan));
        assert!(!service.settings_admitted());
    }

    #[test]
    fn failed_timestamp_commit_preserves_original_schedule_and_does_not_retire_journal() {
        let mut service = AppUpdateService::new(None, false, UpdatePolicy::StableAndBeta, Some(19));
        let journal = failed_record('c');
        service.latest_journal(Ok(Some(journal.clone())));
        service.next_auto_due = Some(500);
        service.scheduled_due_before = Some(Some(90));
        service.pending_request = Some(7);
        service.check_busy = true;
        service.mark_check_save_failure("disk unavailable".into());
        assert_eq!(service.last_checked, Some(19));
        assert_eq!(service.next_auto_due, Some(90));
        assert!(service.auto_failure_paused);
        assert_eq!(service.operation.as_ref(), Some(&journal));
        assert!(service.retired_operation_id.is_none());
        assert!(service.snapshot.is_none());
        assert!(service.pending_request.is_none());
        assert!(!service.check_busy);
        assert!(service.plan().is_none());
    }
}
