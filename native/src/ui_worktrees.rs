use crate::automation::{lock_automation, DomainAction, DomainOperationState, DomainRequest};
use crate::herdr::{RequestOrigin, WorktreeRemoveError, WorktreeRemoveResult};
use crate::i18n::{text, Message};
use crate::session_view::{WorktreeRemoveTarget, WorktreeRemoveTargetError};
use crate::worktree_confirmation::WorktreeConfirmations;
use serde_json::{json, Value};
use std::collections::VecDeque;

use super::Ui;

const MAX_AWAITING_OBSERVATIONS: usize = 32;

struct AwaitingObservation {
    operation_id: String,
    target: WorktreeRemoveTarget,
}

pub(super) struct WorktreeAutomation {
    confirmations: WorktreeConfirmations,
    awaiting_observation: VecDeque<AwaitingObservation>,
}

impl WorktreeAutomation {
    pub(super) fn new() -> Self {
        Self {
            confirmations: WorktreeConfirmations::new(),
            awaiting_observation: VecDeque::new(),
        }
    }
}

fn target_summary(target: &WorktreeRemoveTarget, instance_id: &str) -> Value {
    let worktree = &target.worktree;
    json!({
        "key": {
            "instance_id": instance_id,
            "source_id": target.key.source_id,
            "generation": target.key.generation,
            "terminal_id": target.key.terminal_id,
        },
        "workspace_id": target.workspace_id,
        "pane_id": target.pane_id,
        "worktree": {
            "repo_key": worktree.repo_key,
            "repo_root": worktree.repo_root,
            "checkout_path": worktree.checkout_path,
            // WorkspaceWorktreeInfo does not report a branch or a main flag.
            "branch": Value::Null,
            "is_main": false,
            "is_linked": worktree.is_linked_worktree,
        },
    })
}

fn removal_result(target: &WorktreeRemoveTarget, instance_id: &str, acknowledged: bool) -> Value {
    json!({
        "target": target_summary(target, instance_id),
        "acknowledged": acknowledged,
        "final_observed": false,
    })
}

// A new generation is valid evidence only after a coherent full snapshot of
// the same registered source; AppState enforces that provenance. Never infer
// that a pane has disappeared merely because its agent session has ended.
fn observation_settles(source_id: u64, generation: u64, observation: &Value) -> bool {
    observation.get("source_id").and_then(Value::as_u64) == Some(source_id)
        && observation
            .get("generation")
            .and_then(Value::as_u64)
            .is_some_and(|current| current >= generation)
        && observation.get("pane_absent").and_then(Value::as_bool) == Some(true)
        && observation.get("session_absent").and_then(Value::as_bool) == Some(true)
}

fn target_error(error: WorktreeRemoveTargetError) -> (&'static str, &'static str) {
    match error {
        WorktreeRemoveTargetError::ReadOnly => {
            ("worktree_read_only", "worktree source is not local")
        }
        WorktreeRemoveTargetError::Offline => ("worktree_offline", "worktree source is not live"),
        WorktreeRemoveTargetError::Stale => ("stale_worktree", "worktree target changed"),
        WorktreeRemoveTargetError::NotLinkedWorktree => (
            "not_linked_worktree",
            "target is not a distinct linked checkout",
        ),
    }
}

fn removal_error(error: &WorktreeRemoveError) -> (DomainOperationState, &'static str, String) {
    let (state, code, detail) = match error {
        WorktreeRemoveError::Busy => (
            DomainOperationState::Failed,
            "worktree_busy",
            "another worktree removal is in flight",
        ),
        WorktreeRemoveError::Offline => (
            DomainOperationState::Failed,
            "worktree_offline",
            "worktree source is offline",
        ),
        WorktreeRemoveError::ReadOnly => (
            DomainOperationState::Failed,
            "worktree_read_only",
            "worktree source is not local",
        ),
        WorktreeRemoveError::StaleTarget => (
            DomainOperationState::Failed,
            "stale_worktree",
            "worktree target changed",
        ),
        WorktreeRemoveError::NotLinkedWorktree => (
            DomainOperationState::Failed,
            "not_linked_worktree",
            "target is not a distinct linked checkout",
        ),
        WorktreeRemoveError::Unsupported => (
            DomainOperationState::Failed,
            "worktree_unsupported",
            "worktree removal is unsupported",
        ),
        WorktreeRemoveError::UnknownDelivery => (
            DomainOperationState::UnknownDelivery,
            "unknown_delivery",
            "the removal may have been delivered; do not retry without inspecting the worktree",
        ),
        WorktreeRemoveError::Rejected { message, .. } => (
            DomainOperationState::Failed,
            "worktree_rejected",
            message.as_str(),
        ),
        WorktreeRemoveError::Other(detail) => (
            DomainOperationState::Failed,
            "worktree_failed",
            detail.as_str(),
        ),
    };
    (state, code, detail.to_owned())
}

impl Ui {
    fn finish_worktree_request(
        &self,
        id: &str,
        state: DomainOperationState,
        committed: bool,
        result: Option<Value>,
        error_code: Option<&str>,
        error: Option<String>,
    ) -> bool {
        lock_automation(&self.automation)
            .finish_domain_operation(
                id,
                state,
                committed,
                false,
                result,
                error_code.map(str::to_owned),
                error,
            )
            .is_ok()
    }

    fn worktree_cli_feedback(&mut self, message: Message) {
        self.cli_preference_feedback = Some(text(self.locale, message).to_owned());
        self.bubble_content_dirty = true;
    }

    pub(super) fn handle_cli_worktree(&mut self, request: DomainRequest) {
        let DomainRequest {
            instance_id,
            operation_id,
            action,
        } = request;
        let current_instance = lock_automation(&self.automation).instance() == instance_id;
        if !current_instance {
            self.finish_worktree_request(
                &operation_id,
                DomainOperationState::Failed,
                false,
                None,
                Some("daemon_instance_changed"),
                Some("daemon instance changed".into()),
            );
            self.worktree_cli_feedback(Message::CliWorktreeConflict);
            return;
        }
        match action {
            DomainAction::WorktreeInspect { key } => {
                if key.instance_id != instance_id {
                    self.finish_worktree_request(
                        &operation_id,
                        DomainOperationState::Failed,
                        false,
                        None,
                        Some("daemon_instance_changed"),
                        Some("worktree key belongs to another daemon instance".into()),
                    );
                    self.worktree_cli_feedback(Message::CliWorktreeConflict);
                    return;
                }
                let target = self
                    .shared
                    .lock()
                    .map_err(|_| WorktreeRemoveTargetError::Offline)
                    .and_then(|state| state.worktree_remove_target(&(&key).into()));
                match target {
                    Ok(target) => match self
                        .worktree_automation
                        .confirmations
                        .issue(&instance_id, target)
                    {
                        Ok(confirmation) => {
                            let value = serde_json::to_value(confirmation)
                                .expect("worktree confirmation is serializable");
                            self.finish_worktree_request(
                                &operation_id,
                                DomainOperationState::Applied,
                                false,
                                Some(value),
                                None,
                                None,
                            );
                        }
                        Err(error) => {
                            let code = if error == "too many outstanding worktree confirmations" {
                                "confirmation_busy"
                            } else {
                                "confirmation_unavailable"
                            };
                            self.finish_worktree_request(
                                &operation_id,
                                DomainOperationState::Failed,
                                false,
                                None,
                                Some(code),
                                Some(error),
                            );
                            self.worktree_cli_feedback(Message::CliWorktreeConflict);
                        }
                    },
                    Err(error) => {
                        let (code, detail) = target_error(error);
                        self.finish_worktree_request(
                            &operation_id,
                            DomainOperationState::Failed,
                            false,
                            None,
                            Some(code),
                            Some(detail.into()),
                        );
                        self.worktree_cli_feedback(Message::CliWorktreeConflict);
                    }
                }
            }
            DomainAction::WorktreeRemove { token } => {
                // Consumption precedes all admission checks, even a GUI modal or
                // a Busy sender. A rejected attempt cannot reuse its authority.
                let target = match self
                    .worktree_automation
                    .confirmations
                    .consume(&token, &instance_id)
                {
                    Ok(target) => target,
                    Err(error) => {
                        self.finish_worktree_request(
                            &operation_id,
                            DomainOperationState::Failed,
                            false,
                            None,
                            Some("invalid_confirmation"),
                            Some(error),
                        );
                        self.worktree_cli_feedback(Message::CliWorktreeConflict);
                        return;
                    }
                };
                let current = self
                    .shared
                    .lock()
                    .map_err(|_| WorktreeRemoveTargetError::Offline)
                    .and_then(|state| state.worktree_remove_target(&target.key));
                match &current {
                    Ok(live) if live == &target => {}
                    Ok(_) => {
                        self.finish_worktree_request(
                            &operation_id,
                            DomainOperationState::Failed,
                            false,
                            None,
                            Some("stale_worktree"),
                            Some("frozen worktree target changed".into()),
                        );
                        self.worktree_cli_feedback(Message::CliWorktreeConflict);
                        return;
                    }
                    Err(error) => {
                        let (code, detail) = target_error(*error);
                        self.finish_worktree_request(
                            &operation_id,
                            DomainOperationState::Failed,
                            false,
                            None,
                            Some(code),
                            Some(detail.into()),
                        );
                        self.worktree_cli_feedback(Message::CliWorktreeConflict);
                        return;
                    }
                }
                if self.worktree_confirming
                    || self.worktree_automation.awaiting_observation.len()
                        >= MAX_AWAITING_OBSERVATIONS
                {
                    self.finish_worktree_request(
                        &operation_id,
                        DomainOperationState::Failed,
                        false,
                        None,
                        Some("worktree_busy"),
                        Some("worktree removal or observation capacity is busy".into()),
                    );
                    self.worktree_cli_feedback(Message::CliWorktreeConflict);
                    return;
                }
                let summary = removal_result(&target, &instance_id, false);
                match self.worktree_sender.submit(
                    target,
                    RequestOrigin::Cli {
                        operation_id: operation_id.clone(),
                    },
                ) {
                    Ok(()) => {
                        self.finish_worktree_request(
                            &operation_id,
                            DomainOperationState::Pending,
                            false,
                            Some(summary),
                            None,
                            None,
                        );
                        self.worktree_cli_feedback(Message::CliWorktreePending);
                    }
                    Err(error) => {
                        let (state, code, detail) = removal_error(&error);
                        self.finish_worktree_request(
                            &operation_id,
                            state,
                            false,
                            Some(summary),
                            Some(code),
                            Some(detail),
                        );
                        self.worktree_cli_feedback(Message::CliWorktreeConflict);
                    }
                }
            }
            _ => {
                self.finish_worktree_request(
                    &operation_id,
                    DomainOperationState::Failed,
                    false,
                    None,
                    Some("invalid_action"),
                    Some("expected a worktree action".into()),
                );
                self.worktree_cli_feedback(Message::CliWorktreeConflict);
            }
        }
    }

    /// CLI results never enter GUI's modal feedback, selection, or composer paths.
    pub(super) fn route_cli_worktree_result(
        &mut self,
        result: WorktreeRemoveResult,
    ) -> Option<WorktreeRemoveResult> {
        let RequestOrigin::Cli { operation_id } = &result.origin else {
            return Some(result);
        };
        let operation_id = operation_id.clone();
        let instance_id = lock_automation(&self.automation).instance().to_owned();
        let summary = removal_result(&result.target, &instance_id, result.result.is_ok());
        match result.result {
            Ok(()) => {
                // Keep the operation queryable as an acknowledged but unobserved
                // removal. An upstream ACK never certifies disk or GUI state.
                if self.finish_worktree_request(
                    &operation_id,
                    DomainOperationState::Pending,
                    true,
                    Some(summary),
                    None,
                    None,
                ) {
                    self.worktree_automation
                        .awaiting_observation
                        .push_back(AwaitingObservation {
                            operation_id,
                            target: result.target,
                        });
                    self.worktree_cli_feedback(Message::CliWorktreeAcknowledged);
                }
            }
            Err(error) => {
                let (state, code, detail) = removal_error(&error);
                if self.finish_worktree_request(
                    &operation_id,
                    state,
                    false,
                    Some(summary),
                    Some(code),
                    Some(detail),
                ) {
                    self.worktree_cli_feedback(if state == DomainOperationState::UnknownDelivery {
                        Message::CliWorktreeUncertain
                    } else {
                        Message::CliWorktreeConflict
                    });
                }
            }
        }
        None
    }

    pub(super) fn poll_cli_worktrees(&mut self) {
        let instance_id = lock_automation(&self.automation).instance().to_owned();
        let mut index = 0;
        while index < self.worktree_automation.awaiting_observation.len() {
            let pending = &self.worktree_automation.awaiting_observation[index];
            let still_pending = lock_automation(&self.automation)
                .domain_status(&instance_id, &pending.operation_id)
                .is_ok_and(|status| {
                    status.state == DomainOperationState::Pending && status.committed
                });
            if !still_pending {
                self.worktree_automation.awaiting_observation.remove(index);
                continue;
            }
            let observation = self
                .shared
                .lock()
                .ok()
                .and_then(|state| state.automation_worktree_observation(&pending.target));
            if let Some(observation) = observation.filter(|value| {
                observation_settles(
                    pending.target.key.source_id,
                    pending.target.key.generation,
                    value,
                )
            }) {
                let result = json!({
                    "target": target_summary(&pending.target, &instance_id),
                    "acknowledged": true,
                    "final_observed": true,
                    "observation": observation,
                });
                let id = pending.operation_id.clone();
                self.worktree_automation.awaiting_observation.remove(index);
                if self.finish_worktree_request(
                    &id,
                    DomainOperationState::Applied,
                    true,
                    Some(result),
                    None,
                    None,
                ) {
                    self.worktree_cli_feedback(Message::CliWorktreeObserved);
                }
            } else {
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_evidence_requires_both_independent_absences_from_coherent_identity() {
        for (source, generation, pane, session, settled) in [
            (4, 7, true, true, true),
            (4, 8, true, true, true),
            (4, 7, false, true, false),
            (4, 7, true, false, false),
            (5, 8, true, true, false),
            (4, 6, true, true, false),
        ] {
            let observation = json!({
                "source_id": source,
                "generation": generation,
                "pane_absent": pane,
                "session_absent": session,
            });
            assert_eq!(observation_settles(4, 7, &observation), settled);
        }
        assert!(!observation_settles(
            4,
            7,
            &json!({ "source_id": 4, "generation": 7, "pane_absent": true })
        ));
    }

    #[test]
    fn ambiguous_transport_is_not_a_rejection_or_acknowledgement() {
        let (state, code, _) = removal_error(&WorktreeRemoveError::UnknownDelivery);
        assert_eq!(state, DomainOperationState::UnknownDelivery);
        assert_eq!(code, "unknown_delivery");
        let (state, code, _) = removal_error(&WorktreeRemoveError::Rejected {
            code: "dirty_worktree".into(),
            message: "checkout has unsaved changes".into(),
        });
        assert_eq!(state, DomainOperationState::Failed);
        assert_eq!(code, "worktree_rejected");
    }
}
