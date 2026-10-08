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
    origin: RequestOrigin,
    target: WorktreeRemoveTarget,
    baseline_revision: u64,
    /// None until the worker returns an ACK or an ambiguous transport result.
    observed_delivery: Option<bool>,
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
    pub(super) fn has_pending_update_work(&self) -> bool {
        self.confirmations.has_pending() || !self.awaiting_observation.is_empty()
    }

    fn can_dispatch_remove(&self) -> bool {
        self.awaiting_observation.len() < MAX_AWAITING_OBSERVATIONS
    }
    fn can_dispatch_target(&self, target: &WorktreeRemoveTarget) -> bool {
        self.can_dispatch_remove()
            && !self
                .awaiting_observation
                .iter()
                .any(|pending| pending.target == *target)
    }

    fn retain_dispatch(
        &mut self,
        target: WorktreeRemoveTarget,
        origin: RequestOrigin,
        baseline_revision: u64,
    ) {
        // Admission and dispatch are on the UI thread; no nested event loop
        // intervenes between the capacity check, sender submission and this insert.
        assert!(self.can_dispatch_target(&target));
        self.awaiting_observation.push_back(AwaitingObservation {
            origin,
            target,
            baseline_revision,
            observed_delivery: None,
        });
    }

    fn record_result(&mut self, result: &WorktreeRemoveResult) {
        let Some(index) = self
            .awaiting_observation
            .iter()
            .position(|pending| pending.origin == result.origin && pending.target == result.target)
        else {
            return;
        };
        match &result.result {
            Ok(()) => self.awaiting_observation[index].observed_delivery = Some(true),
            Err(WorktreeRemoveError::UnknownDelivery) => {
                self.awaiting_observation[index].observed_delivery = Some(false);
            }
            Err(_) => {
                self.awaiting_observation.remove(index);
            }
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

// Require both independent absences from the same authoritative source in a
// later coherent publication, not merely a newer status or generation.
fn observation_settles(
    target: &WorktreeRemoveTarget,
    baseline_revision: u64,
    observation: &Value,
) -> bool {
    observation.get("source").and_then(Value::as_str) == Some(target.source.as_str())
        && observation.get("source_id").and_then(Value::as_u64) == Some(target.key.source_id)
        && observation
            .get("generation")
            .and_then(Value::as_u64)
            .is_some_and(|current| current >= target.key.generation)
        && observation
            .get("snapshot_revision")
            .and_then(Value::as_u64)
            .is_some_and(|revision| revision > baseline_revision)
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
    pub(super) fn gui_worktree_check_capacity(
        &self,
        target: &WorktreeRemoveTarget,
    ) -> Result<(), WorktreeRemoveError> {
        if !self.worktree_automation.can_dispatch_target(target) {
            return Err(WorktreeRemoveError::Busy);
        }
        Ok(())
    }

    pub(super) fn retain_gui_worktree_dispatch(
        &mut self,
        target: WorktreeRemoveTarget,
        baseline_revision: u64,
    ) {
        self.worktree_automation
            .retain_dispatch(target, RequestOrigin::Gui, baseline_revision);
    }

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
                    || !self.worktree_automation.can_dispatch_target(&target)
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
                let origin = RequestOrigin::Cli {
                    operation_id: operation_id.clone(),
                };
                match self.worktree_sender.submit(target.clone(), origin.clone()) {
                    Ok(baseline_revision) => {
                        self.worktree_automation
                            .retain_dispatch(target, origin, baseline_revision);
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
        // A worker completing clears sender.pending; retain ACK or ambiguous
        // delivery before the UI can overwrite its presentation feedback.
        self.worktree_automation.record_result(&result);
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

    /// Poll both GUI and CLI retained dispatches. A sender result and the
    /// painted feedback are not authority for mutation safety.
    pub(super) fn poll_cli_worktrees(&mut self) {
        let instance_id = lock_automation(&self.automation).instance().to_owned();
        let mut index = 0;
        while index < self.worktree_automation.awaiting_observation.len() {
            let pending = &self.worktree_automation.awaiting_observation[index];
            let Some(acknowledged) = pending.observed_delivery else {
                index += 1;
                continue;
            };
            let origin = pending.origin.clone();
            if let RequestOrigin::Cli { operation_id } = &origin {
                let expected = if acknowledged {
                    DomainOperationState::Pending
                } else {
                    DomainOperationState::UnknownDelivery
                };
                if !lock_automation(&self.automation)
                    .domain_status(&instance_id, operation_id)
                    .is_ok_and(|status| {
                        status.state == expected && status.committed == acknowledged
                    })
                {
                    index += 1;
                    continue;
                }
            }
            let observation = self
                .shared
                .lock()
                .ok()
                .and_then(|state| state.automation_worktree_observation(&pending.target));
            let Some(observation) = observation.filter(|value| {
                observation_settles(&pending.target, pending.baseline_revision, value)
            }) else {
                index += 1;
                continue;
            };
            let settled = match &origin {
                RequestOrigin::Gui => true,
                RequestOrigin::Cli { operation_id } => {
                    let result = json!({
                        "target": target_summary(&pending.target, &instance_id),
                        "acknowledged": acknowledged,
                        // Pane/session absence is not disk deletion evidence.
                        "final_observed": true,
                        "observation": observation,
                    });
                    self.finish_worktree_request(
                        operation_id,
                        if acknowledged {
                            DomainOperationState::Applied
                        } else {
                            // Source absence supersedes the request, but does
                            // not prove the unknown delivery or disk deletion.
                            DomainOperationState::Superseded
                        },
                        acknowledged,
                        Some(result),
                        (!acknowledged).then_some("target_absent"),
                        (!acknowledged).then(|| {
                            "later coherent pane/session absence; delivery and disk deletion unconfirmed".into()
                        }),
                    )
                }
            };
            if settled {
                let clear_gui_feedback = matches!(&origin, RequestOrigin::Gui)
                    && self
                        .worktree_feedback
                        .as_ref()
                        .is_some_and(|feedback| match feedback {
                            super::WorktreeFeedback::Finished(target, Ok(()))
                            | super::WorktreeFeedback::Finished(
                                target,
                                Err(WorktreeRemoveError::UnknownDelivery),
                            ) => {
                                target
                                    == &self.worktree_automation.awaiting_observation[index].target
                            }
                            _ => false,
                        });
                self.worktree_automation.awaiting_observation.remove(index);
                if clear_gui_feedback {
                    self.worktree_feedback = None;
                    self.bubble_content_dirty = true;
                    super::wake();
                }
                if matches!(&origin, RequestOrigin::Cli { .. }) {
                    self.worktree_cli_feedback(if acknowledged {
                        Message::CliWorktreeObserved
                    } else {
                        Message::CliWorktreeUncertain
                    });
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
    use crate::herdr_protocol::WorkspaceWorktreeInfo;
    use crate::session_view::SessionKey;
    use std::sync::Arc;

    fn target() -> WorktreeRemoveTarget {
        WorktreeRemoveTarget {
            key: SessionKey {
                source_id: 4,
                generation: 7,
                terminal_id: "terminal".into(),
            },
            source: "/tmp/source.sock".into(),
            pane_id: "pane".into(),
            workspace_id: "workspace".into(),
            worktree: Arc::new(WorkspaceWorktreeInfo {
                repo_key: "repo".into(),
                repo_name: "Repository".into(),
                repo_root: "/repo".into(),
                checkout_path: "/repo/linked".into(),
                is_linked_worktree: true,
                pane_count: 1,
                tab_count: 1,
            }),
        }
    }

    #[test]
    fn final_evidence_requires_later_coherent_both_absences_and_exact_source_path() {
        let target = target();
        let baseline = 12;
        for (source, path, generation, revision, pane, session, settled) in [
            (4, "/tmp/source.sock", 7, 13, true, true, true),
            (4, "/tmp/source.sock", 8, 14, true, true, true),
            (4, "/tmp/source.sock", 7, 12, true, true, false),
            (4, "/tmp/source.sock", 8, 11, true, true, false),
            (4, "/tmp/source.sock", 7, 13, false, true, false),
            (4, "/tmp/source.sock", 7, 13, true, false, false),
            (5, "/tmp/source.sock", 8, 13, true, true, false),
            (4, "/tmp/other.sock", 8, 13, true, true, false),
            (4, "/tmp/source.sock", 6, 13, true, true, false),
        ] {
            let observation = json!({
                "source": path,
                "source_id": source,
                "generation": generation,
                "snapshot_revision": revision,
                "pane_absent": pane,
                "session_absent": session,
            });
            assert_eq!(
                observation_settles(&target, baseline, &observation),
                settled
            );
        }
        assert!(!observation_settles(
            &target,
            baseline,
            &json!({ "source": target.source, "source_id": 4, "generation": 7,
                "snapshot_revision": 13, "pane_absent": true })
        ));
    }

    #[test]
    fn gui_and_cli_uncertainty_share_capacity_after_sender_clears_pending() {
        let mut owner = WorktreeAutomation::new();
        let mut rejected_target = None;
        for index in 0..MAX_AWAITING_OBSERVATIONS {
            let mut frozen = target();
            frozen.workspace_id = format!("workspace-{index}");
            let origin = if index % 2 == 0 {
                RequestOrigin::Gui
            } else {
                RequestOrigin::Cli {
                    operation_id: format!("remove-{index}"),
                }
            };
            assert!(owner.can_dispatch_target(&frozen));
            owner.retain_dispatch(frozen.clone(), origin.clone(), 10);
            assert!(!owner.can_dispatch_target(&frozen));
            if index + 1 == MAX_AWAITING_OBSERVATIONS {
                rejected_target = Some((frozen, origin));
                continue;
            }
            let result = WorktreeRemoveResult {
                target: frozen,
                origin,
                result: if index % 3 == 0 {
                    Ok(())
                } else {
                    Err(WorktreeRemoveError::UnknownDelivery)
                },
            };
            owner.record_result(&result);
            assert!(owner.has_pending_update_work());
        }
        assert!(!owner.can_dispatch_target(&target()));
        let (frozen, origin) = rejected_target.unwrap();
        owner.record_result(&WorktreeRemoveResult {
            target: frozen,
            origin,
            result: Err(WorktreeRemoveError::Rejected {
                code: "dirty_worktree".into(),
                message: "not removed".into(),
            }),
        });
        assert!(owner.can_dispatch_target(&target()));
        assert!(owner.has_pending_update_work());
        assert_eq!(
            owner.awaiting_observation.len(),
            MAX_AWAITING_OBSERVATIONS - 1
        );
        assert!(owner
            .awaiting_observation
            .iter()
            .any(|pending| pending.origin == RequestOrigin::Gui
                && pending.observed_delivery == Some(false)));
        assert!(owner.awaiting_observation.iter().any(|pending| matches!(
            pending.origin,
            RequestOrigin::Cli { .. }
        ) && pending.observed_delivery
            == Some(true)));
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
