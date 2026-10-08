use crate::automation::{lock_automation, DomainAction, DomainOperationState, DomainRequest};
use crate::herdr::{PromptError, PromptResult, RequestOrigin};
use crate::i18n::{cli_prompt_feedback, CliPromptFeedback};

use super::Ui;

impl Ui {
    pub(super) fn handle_cli_session_prompt(&mut self, request: DomainRequest) {
        let DomainRequest {
            instance_id,
            operation_id,
            action,
        } = request;
        let DomainAction::SessionPrompt { key, text } = action else {
            let _ = lock_automation(&self.automation).finish_domain_operation(
                &operation_id,
                DomainOperationState::Failed,
                false,
                false,
                None,
                Some("invalid_action".into()),
                Some("expected a session prompt".into()),
            );
            self.set_cli_prompt_feedback(CliPromptFeedback::Failed);
            return;
        };

        // The daemon identity is part of the target, not merely the control envelope.
        // A stale identity must not be silently rebound to a live session key.
        let current_instance = {
            let ledger = lock_automation(&self.automation);
            instance_id == ledger.instance() && key.instance_id == ledger.instance()
        };
        if !current_instance {
            let _ = lock_automation(&self.automation).finish_domain_operation(
                &operation_id,
                DomainOperationState::Failed,
                false,
                false,
                None,
                Some("daemon_instance_changed".into()),
                Some("session belongs to another daemon instance".into()),
            );
            self.set_cli_prompt_feedback(CliPromptFeedback::Failed);
            return;
        }

        // Move the original UTF-8 body into the same single-flight sender used by
        // the GUI. The sender checks the fresh local/live target and frame size.
        if let Err(error) = self.prompt_sender.submit(
            key.into(),
            text,
            RequestOrigin::Cli {
                operation_id: operation_id.clone(),
            },
        ) {
            let (state, code, message) = cli_prompt_failure(&error);
            let _ = lock_automation(&self.automation).finish_domain_operation(
                &operation_id,
                state,
                false,
                false,
                None,
                Some(code.into()),
                Some(message),
            );
            self.set_cli_prompt_feedback(CliPromptFeedback::Failed);
        } else {
            self.set_cli_prompt_feedback(CliPromptFeedback::Sending);
        }
    }

    /// Remove CLI completions before the GUI's composer-clear and draft paths.
    pub(super) fn route_cli_prompt_result(&mut self, result: PromptResult) -> Option<PromptResult> {
        let RequestOrigin::Cli { operation_id } = &result.submission.origin else {
            return Some(result);
        };
        let mut ledger = lock_automation(&self.automation);
        let (state, committed, value, error_code, error) = match &result.result {
            Ok(()) => (
                DomainOperationState::AgentPrompted,
                true,
                Some(serde_json::json!({
                    "key": {
                        "instance_id": ledger.instance(),
                        "source_id": result.submission.key.source_id,
                        "generation": result.submission.key.generation,
                        "terminal_id": &result.submission.key.terminal_id,
                    },
                    "delivery": "acknowledged",
                })),
                None,
                None,
            ),
            Err(error) => {
                let (state, code, message) = cli_prompt_failure(error);
                (state, false, None, Some(code.into()), Some(message))
            }
        };
        // A shutdown may already have resolved this pending operation to
        // UnknownDelivery; never turn that uncertainty into a later ACK.
        let completed = ledger.finish_domain_operation(
            operation_id,
            state,
            committed,
            false,
            value,
            error_code,
            error,
        );
        let feedback = if completed.is_ok() {
            Some(match state {
                DomainOperationState::AgentPrompted => CliPromptFeedback::Acknowledged,
                DomainOperationState::UnknownDelivery => CliPromptFeedback::UnknownDelivery,
                _ => CliPromptFeedback::Failed,
            })
        } else {
            // The ledger may have resolved a pending send during shutdown.
            let instance = ledger.instance().to_owned();
            ledger
                .domain_status(&instance, operation_id)
                .ok()
                .filter(|operation| operation.state == DomainOperationState::UnknownDelivery)
                .map(|_| CliPromptFeedback::UnknownDelivery)
        };
        drop(ledger);
        if let Some(feedback) = feedback {
            self.set_cli_prompt_feedback(feedback);
        }
        None
    }

    fn set_cli_prompt_feedback(&mut self, feedback: CliPromptFeedback) {
        self.cli_preference_feedback = Some(cli_prompt_feedback(self.locale, feedback).into());
        self.bubble_content_dirty = true;
    }
}

fn cli_prompt_failure(error: &PromptError) -> (DomainOperationState, &'static str, String) {
    let (state, code, message) = match error {
        PromptError::Empty => (
            DomainOperationState::Failed,
            "empty_prompt",
            "prompt text is empty",
        ),
        PromptError::TooLarge => (
            DomainOperationState::Failed,
            "prompt_too_large",
            "prompt exceeds the worker frame limit",
        ),
        PromptError::Busy => (
            DomainOperationState::Failed,
            "prompt_busy",
            "another prompt is in flight",
        ),
        PromptError::Offline => (
            DomainOperationState::Failed,
            "session_offline",
            "session is offline",
        ),
        PromptError::ReadOnly => (
            DomainOperationState::Failed,
            "session_read_only",
            "session is read-only",
        ),
        PromptError::StaleTarget => (
            DomainOperationState::Failed,
            "stale_session",
            "session target changed",
        ),
        PromptError::Blocked => (
            DomainOperationState::Failed,
            "session_blocked",
            "session is blocked",
        ),
        PromptError::NotReady => (
            DomainOperationState::Failed,
            "session_not_ready",
            "session is not ready",
        ),
        PromptError::Unsupported => (
            DomainOperationState::Failed,
            "prompt_unsupported",
            "session does not support prompts",
        ),
        PromptError::UnknownDelivery => (
            DomainOperationState::UnknownDelivery,
            "unknown_delivery",
            "prompt delivery is unknown; do not resend without checking the agent",
        ),
        PromptError::Other(detail) => {
            return (
                DomainOperationState::Failed,
                "prompt_failed",
                detail.clone(),
            );
        }
    };
    (state, code, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_delivery_is_not_reported_as_a_rejected_prompt() {
        let (state, code, message) = cli_prompt_failure(&PromptError::UnknownDelivery);
        assert_eq!(state, DomainOperationState::UnknownDelivery);
        assert_eq!(code, "unknown_delivery");
        assert!(message.contains("do not resend"));
        assert_eq!(
            cli_prompt_failure(&PromptError::Busy).0,
            DomainOperationState::Failed
        );
        assert_ne!(
            cli_prompt_failure(&PromptError::StaleTarget).1,
            cli_prompt_failure(&PromptError::ReadOnly).1
        );
    }
}
