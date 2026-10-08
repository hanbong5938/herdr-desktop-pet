use super::*;
use crate::automation::{DomainAction, DomainOperationState, DomainRequest};
use crate::dialogue::validate_text;
use crate::dialogue_automation::{
    metadata_token, read_entry, target_overrides_token, DialogueBaseline, DialogueIdentity,
    DialogueSelection,
};
use serde_json::{json, Value};

const MAX_PENDING_DIALOGUES: usize = 16;
const MAX_DIALOGUE_RESULT_BYTES: usize = 60 * 1024;

pub(super) struct DialogueCommit {
    pub(super) result: Value,
    pub(super) active: bool,
    pub(super) native_applied: bool,
}

pub(super) struct DialogueMutationError {
    pub(super) code: &'static str,
    pub(super) detail: String,
}

impl DialogueMutationError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

struct PendingMetadata {
    operation_id: String,
    action: DomainAction,
    explicit_read_pending: bool,
}

struct PendingNativeDialogue {
    operation_id: String,
    identity: DialogueIdentity,
    renderer_token: RendererToken,
    overrides_token: String,
    result: Value,
}

impl PendingNativeDialogue {
    fn matches_renderer_and_overrides(
        &self,
        renderer: &RendererToken,
        overrides: &crate::dialogue::DialogueOverrides,
    ) -> bool {
        renderer == &self.renderer_token
            && target_overrides_token(overrides, &self.identity.target) == self.overrides_token
    }
}

pub(super) struct DialogueAutomation {
    metadata: Vec<PendingMetadata>,
    native: Vec<PendingNativeDialogue>,
}

impl DialogueAutomation {
    pub(super) fn new() -> Self {
        Self {
            metadata: Vec::new(),
            native: Vec::new(),
        }
    }

    pub(super) fn has_pending_update_work(&self) -> bool {
        !self.metadata.is_empty() || !self.native.is_empty()
    }
}

#[derive(Default)]
struct ResultByteCounter(usize);

impl std::io::Write for ResultByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MAX_DIALOGUE_RESULT_BYTES {
            return Err(std::io::Error::other(
                "dialogue result exceeds operation limit",
            ));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn result_fits(result: &Value) -> bool {
    serde_json::to_writer(ResultByteCounter::default(), result).is_ok()
}

impl Ui {
    fn dialogue_choice(
        &self,
        identity: &DialogueIdentity,
    ) -> Result<&DialogueChoice, DialogueMutationError> {
        let choice = self.dialogue_choices.iter().find(|choice| {
            choice.target == identity.target
                && choice.reference == identity.reference
                && choice.generation == identity.generation
        });
        match choice {
            Some(choice) if self.dialogue_choice_valid(choice) => Ok(choice),
            _ => Err(DialogueMutationError::new(
                "stale_target",
                "dialogue target, generation or character reference is no longer available",
            )),
        }
    }

    fn dialogue_metadata(
        &self,
        identity: &DialogueIdentity,
        explicit_read: bool,
    ) -> Result<Option<Option<CharacterMetadata>>, DialogueMutationError> {
        let choice = self.dialogue_choice(identity)?;
        if self.active_dialogue_matches(&identity.target)
            && identity.reference.as_ref() == Some(&self.active.token().reference)
        {
            return Ok(Some(self.active.metadata().cloned()));
        }
        if identity.reference.is_none() {
            // The only unreferenced choice is the already selected external asset.
            if self.external_dialogue_target.as_ref() == Some(&identity.target)
                && self.active_dialogue_matches(&identity.target)
            {
                return Ok(Some(self.external_dialogue_metadata.clone()));
            }
            return Err(DialogueMutationError::new(
                "stale_target",
                "external dialogue is no longer active",
            ));
        }
        let reference = choice.reference.as_ref().expect("checked reference above");
        if explicit_read {
            self.packs
                .retry_dialogue_metadata(reference.clone(), choice.generation)
                .map_err(|error| {
                    DialogueMutationError::new(
                        if error.starts_with("Busy:") {
                            "busy"
                        } else {
                            "metadata_unavailable"
                        },
                        error,
                    )
                })?;
        }
        if let Some(cached) = self
            .packs
            .cached_dialogue_metadata(reference, choice.generation)
        {
            return cached
                .map(|metadata| Some(metadata.metadata))
                .map_err(|error| DialogueMutationError::new("metadata_unavailable", error));
        }
        self.packs
            .request_dialogue_metadata(reference.clone(), choice.generation)
            .map_err(|error| {
                DialogueMutationError::new(
                    if error.starts_with("Busy:") {
                        "busy"
                    } else {
                        "metadata_unavailable"
                    },
                    error,
                )
            })?;
        Ok(None)
    }

    fn current_dialogue_metadata(
        &self,
        identity: &DialogueIdentity,
    ) -> Result<Option<CharacterMetadata>, DialogueMutationError> {
        self.dialogue_metadata(identity, false)?.ok_or_else(|| {
            DialogueMutationError::new("metadata_unavailable", "dialogue metadata is still loading")
        })
    }

    fn dialogue_native_ready(&self, target: &DialogueTarget) -> bool {
        self.active_dialogue_matches(target)
            && self.did_present
            && !self.last_scene.shutdown
            && !self.bubble_content_dirty
            && !self.pending_bubble_content
            && self.pending_bubble_scene.is_none()
            && self.shared.lock().is_ok_and(|state| {
                let current = state.scene();
                !current.shutdown
                    && presentation_matches_scene(&current, &self.last_scene)
                    && !status_fields_changed(&current, &self.last_scene)
            })
    }

    fn apply_committed_dialogue(&mut self, identity: &DialogueIdentity) -> (bool, bool) {
        let active = self.active_dialogue_matches(&identity.target);
        self.editor_content_dirty = true;
        if active {
            self.rebuild_effective_dialogue();
            let scene = self.last_scene.clone();
            self.refresh_bubble_content(&scene);
        }
        (
            active,
            active && self.dialogue_native_ready(&identity.target),
        )
    }

    pub(super) fn commit_dialogue_mutation(
        &mut self,
        selection: &DialogueSelection,
        baseline: &DialogueBaseline,
        value: Option<String>,
    ) -> Result<DialogueCommit, DialogueMutationError> {
        let metadata = self.current_dialogue_metadata(&selection.identity)?;
        self.commit_dialogue_mutation_with_metadata(selection, baseline, value, metadata)
    }

    fn commit_dialogue_mutation_with_metadata(
        &mut self,
        selection: &DialogueSelection,
        baseline: &DialogueBaseline,
        value: Option<String>,
        metadata: Option<CharacterMetadata>,
    ) -> Result<DialogueCommit, DialogueMutationError> {
        if metadata_token(&selection.identity, metadata.as_ref()) != baseline.metadata_token {
            return Err(DialogueMutationError::new(
                "metadata_conflict",
                "dialogue source metadata changed",
            ));
        }
        let current = self.prefs.dialogue_overrides().entry(
            &selection.identity.target,
            selection.locale.tag(),
            selection.slot,
        );
        if current != baseline.override_entry.as_deref() {
            return Err(DialogueMutationError::new(
                "entry_conflict",
                "dialogue entry changed since it was read",
            ));
        }
        if let Some(value) = value.as_ref() {
            validate_text(value)
                .map_err(|detail| DialogueMutationError::new("invalid_dialogue", detail))?;
        }
        self.dialogue_choice(&selection.identity)?;
        self.prefs
            .save_dialogue_entry(
                &selection.identity.target,
                selection.locale.tag(),
                selection.slot,
                value,
                &self.lifecycle_paths.config_dir,
            )
            .map_err(|detail| DialogueMutationError::new("persist_failed", detail))?;
        let (active, native_applied) = self.apply_committed_dialogue(&selection.identity);
        let mut result = read_entry(
            selection,
            metadata.as_ref(),
            self.prefs.dialogue_overrides(),
        );
        result["active"] = json!(active);
        Ok(DialogueCommit {
            result,
            active,
            native_applied,
        })
    }

    pub(super) fn commit_dialogue_character_reset(
        &mut self,
        identity: &DialogueIdentity,
        metadata_token_expected: &str,
        target_overrides_token_expected: &str,
    ) -> Result<DialogueCommit, DialogueMutationError> {
        let metadata = self.current_dialogue_metadata(identity)?;
        self.commit_dialogue_character_reset_with_metadata(
            identity,
            metadata_token_expected,
            target_overrides_token_expected,
            metadata,
        )
    }

    fn commit_dialogue_character_reset_with_metadata(
        &mut self,
        identity: &DialogueIdentity,
        metadata_token_expected: &str,
        target_overrides_token_expected: &str,
        metadata: Option<CharacterMetadata>,
    ) -> Result<DialogueCommit, DialogueMutationError> {
        if metadata_token(identity, metadata.as_ref()) != metadata_token_expected {
            return Err(DialogueMutationError::new(
                "metadata_conflict",
                "dialogue source metadata changed",
            ));
        }
        if target_overrides_token(self.prefs.dialogue_overrides(), &identity.target)
            != target_overrides_token_expected
        {
            return Err(DialogueMutationError::new(
                "target_conflict",
                "character dialogue overrides changed since they were read",
            ));
        }
        self.dialogue_choice(identity)?;
        self.prefs
            .reset_character_dialogue(&identity.target, &self.lifecycle_paths.config_dir)
            .map_err(|detail| DialogueMutationError::new("persist_failed", detail))?;
        let (active, native_applied) = self.apply_committed_dialogue(identity);
        let result = json!({
            "identity": identity,
            "metadata_token": metadata_token(identity, metadata.as_ref()),
            "target_overrides_token": target_overrides_token(self.prefs.dialogue_overrides(), &identity.target),
            "overrides": self.prefs.dialogue_overrides().locales(&identity.target),
            "active": active,
        });
        Ok(DialogueCommit {
            result,
            active,
            native_applied,
        })
    }

    fn finish_cli_dialogue(
        &self,
        id: &str,
        mut state: DomainOperationState,
        committed: bool,
        native_applied: bool,
        mut result: Option<Value>,
        mut error: Option<DialogueMutationError>,
    ) {
        if result.as_ref().is_some_and(|result| !result_fits(result)) {
            result = if committed {
                Some(json!({"result_omitted_reason":"response_limit"}))
            } else {
                None
            };
            if !committed {
                state = DomainOperationState::Failed;
                error = Some(DialogueMutationError::new(
                    "result_too_large",
                    "dialogue result exceeds operation limit",
                ));
            }
        }
        let mut ledger = lock_automation(&self.automation);
        if let Err(failure) = ledger.finish_domain_operation(
            id,
            state,
            committed,
            native_applied,
            result,
            error.as_ref().map(|error| error.code.to_owned()),
            error.map(|error| error.detail),
        ) {
            eprintln!("desktop-pet: dialogue operation result failed: {failure}");
        }
    }

    fn dialogue_catalog(&self) -> Value {
        let generation = self.packs.cached_list().generation;
        let slots: Vec<_> = DialogueSlot::ALL.iter().map(|slot| slot.key()).collect();
        json!({ "targets": self.dialogue_choices.iter().filter(|choice| {
            choice.generation == generation
                && match &choice.reference {
                    Some(reference) => self.packs.dialogue_reference_valid(reference, generation),
                    None => self.external_dialogue_target.as_ref() == Some(&choice.target),
                }
        }).map(|choice| {
            json!({"identity": DialogueIdentity::from(choice), "name": choice.name, "locales": ["ko", "en"], "slots": slots})
        }).collect::<Vec<_>>() })
    }

    pub(super) fn handle_cli_dialogue(&mut self, request: DomainRequest) {
        let id = request.operation_id;
        match request.action {
            DomainAction::DialogueList {} => self.finish_cli_dialogue(
                &id,
                DomainOperationState::Applied,
                false,
                false,
                Some(self.dialogue_catalog()),
                None,
            ),
            action @ (DomainAction::DialogueRead { .. }
            | DomainAction::DialogueSet { .. }
            | DomainAction::DialogueResetEntry { .. }
            | DomainAction::DialogueResetCharacter { .. }) => {
                if self.dialogue_automation.metadata.len() + self.dialogue_automation.native.len()
                    >= MAX_PENDING_DIALOGUES
                {
                    if !matches!(&action, DomainAction::DialogueRead { .. }) {
                        self.set_cli_dialogue_feedback(Message::CliDialogueFailed, None);
                    }
                    self.finish_cli_dialogue(
                        &id,
                        DomainOperationState::Failed,
                        false,
                        false,
                        None,
                        Some(DialogueMutationError::new(
                            "busy",
                            "too many pending dialogue lookups",
                        )),
                    );
                    return;
                }
                let explicit_read_pending = matches!(&action, DomainAction::DialogueRead { .. });
                self.dialogue_automation.metadata.push(PendingMetadata {
                    operation_id: id,
                    action,
                    explicit_read_pending,
                });
                self.poll_cli_dialogues();
            }
            _ => unreachable!("dialogue handler received another domain action"),
        }
    }

    fn process_dialogue_action(&mut self, entry: PendingMetadata) -> bool {
        let id = entry.operation_id;
        let mutation = !matches!(&entry.action, DomainAction::DialogueRead { .. });
        let identity = match &entry.action {
            DomainAction::DialogueRead { selection }
            | DomainAction::DialogueSet { selection, .. }
            | DomainAction::DialogueResetEntry { selection, .. } => &selection.identity,
            DomainAction::DialogueResetCharacter { identity, .. } => identity,
            _ => unreachable!("queued non-dialogue action"),
        };
        let metadata = match self.dialogue_metadata(identity, entry.explicit_read_pending) {
            Ok(None) => {
                self.dialogue_automation.metadata.push(PendingMetadata {
                    operation_id: id,
                    action: entry.action,
                    explicit_read_pending: false,
                });
                return false;
            }
            Err(error) => {
                if mutation {
                    let message = if error.code == "stale_target" {
                        Message::CliDialogueConflict
                    } else {
                        Message::CliDialogueFailed
                    };
                    self.set_cli_dialogue_feedback(message, Some(&error.detail));
                }
                self.finish_cli_dialogue(
                    &id,
                    DomainOperationState::Failed,
                    false,
                    false,
                    None,
                    Some(error),
                );
                return true;
            }
            Ok(Some(metadata)) => metadata,
        };
        match entry.action {
            DomainAction::DialogueRead { selection } => {
                let mut result = read_entry(
                    &selection,
                    metadata.as_ref(),
                    self.prefs.dialogue_overrides(),
                );
                result["active"] = json!(self.active_dialogue_matches(&selection.identity.target));
                self.finish_cli_dialogue(
                    &id,
                    DomainOperationState::Applied,
                    false,
                    false,
                    Some(result),
                    None,
                );
            }
            DomainAction::DialogueSet {
                selection,
                baseline,
                text,
            } => {
                let commit = self.commit_dialogue_mutation_with_metadata(
                    &selection,
                    &baseline,
                    Some(text),
                    metadata,
                );
                self.finish_dialogue_commit(&id, &selection.identity, commit);
            }
            DomainAction::DialogueResetEntry {
                selection,
                baseline,
            } => {
                let commit = self
                    .commit_dialogue_mutation_with_metadata(&selection, &baseline, None, metadata);
                self.finish_dialogue_commit(&id, &selection.identity, commit);
            }
            DomainAction::DialogueResetCharacter {
                identity,
                metadata_token,
                target_overrides_token,
            } => {
                let commit = self.commit_dialogue_character_reset_with_metadata(
                    &identity,
                    &metadata_token,
                    &target_overrides_token,
                    metadata,
                );
                self.finish_dialogue_commit(&id, &identity, commit);
            }
            _ => unreachable!("queued non-dialogue action"),
        }
        true
    }

    fn set_cli_dialogue_feedback(&mut self, message: Message, detail: Option<&str>) {
        let title = text(self.locale, message);
        self.cli_preference_feedback = Some(match detail {
            Some(detail) => format!("{title}: {detail}"),
            None => title.to_owned(),
        });
        self.bubble_content_dirty = true;
    }

    fn finish_dialogue_commit(
        &mut self,
        id: &str,
        identity: &DialogueIdentity,
        commit: Result<DialogueCommit, DialogueMutationError>,
    ) {
        match commit {
            Err(error) => {
                let message = if matches!(
                    error.code,
                    "metadata_conflict" | "entry_conflict" | "target_conflict" | "stale_target"
                ) {
                    Message::CliDialogueConflict
                } else {
                    Message::CliDialogueFailed
                };
                self.set_cli_dialogue_feedback(message, Some(&error.detail));
                self.finish_cli_dialogue(
                    id,
                    DomainOperationState::Failed,
                    false,
                    false,
                    None,
                    Some(error),
                );
            }
            Ok(commit) if !commit.active || commit.native_applied => {
                self.set_cli_dialogue_feedback(Message::CliDialogueSaved, None);
                self.finish_cli_dialogue(
                    id,
                    DomainOperationState::Applied,
                    true,
                    commit.native_applied,
                    Some(commit.result),
                    None,
                );
            }
            Ok(commit) => {
                let overrides_token =
                    target_overrides_token(self.prefs.dialogue_overrides(), &identity.target);
                // The ledger must learn about the saved candidate before waiting for native
                // evidence: shutdown cannot turn a committed write into uncertain delivery.
                let result = if result_fits(&commit.result) {
                    commit.result
                } else {
                    json!({"active": true, "result_omitted_reason": "response_limit"})
                };
                self.finish_cli_dialogue(
                    id,
                    DomainOperationState::Pending,
                    true,
                    false,
                    Some(result.clone()),
                    None,
                );
                self.dialogue_automation.native.push(PendingNativeDialogue {
                    operation_id: id.to_owned(),
                    identity: identity.clone(),
                    renderer_token: self.active.token().clone(),
                    overrides_token,
                    result,
                });
                self.set_cli_dialogue_feedback(Message::CliDialoguePending, None);
                self.queue_language_apply();
            }
        }
    }

    pub(super) fn has_pending_native_dialogue(&self) -> bool {
        !self.dialogue_automation.native.is_empty()
    }

    pub(super) fn poll_cli_dialogues(&mut self) {
        let metadata = std::mem::take(&mut self.dialogue_automation.metadata);
        for entry in metadata {
            self.process_dialogue_action(entry);
        }
        self.settle_pending_native_dialogues();
    }

    pub(super) fn settle_pending_native_dialogues(&mut self) {
        let native = std::mem::take(&mut self.dialogue_automation.native);
        for entry in native {
            let still_current = self.dialogue_choice(&entry.identity).is_ok()
                && self.active_dialogue_matches(&entry.identity.target)
                && entry.matches_renderer_and_overrides(
                    self.active.token(),
                    self.prefs.dialogue_overrides(),
                );
            if !still_current {
                self.set_cli_dialogue_feedback(Message::CliDialogueConflict, None);
                self.finish_cli_dialogue(
                    &entry.operation_id,
                    DomainOperationState::Superseded,
                    true,
                    false,
                    Some(entry.result),
                    None,
                );
            } else if self.dialogue_native_ready(&entry.identity.target) {
                self.set_cli_dialogue_feedback(Message::CliDialogueSaved, None);
                self.finish_cli_dialogue(
                    &entry.operation_id,
                    DomainOperationState::Applied,
                    true,
                    true,
                    Some(entry.result),
                    None,
                );
            } else {
                self.dialogue_automation.native.push(entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogue::DialogueOverrides;

    #[test]
    fn pending_native_proof_follows_renderer_epoch_and_shared_target_changes() {
        let target = DialogueTarget::Character("shared".to_owned());
        let authored = DialogueIdentity {
            target: target.clone(),
            reference: Some(CharacterRef {
                id: "shared".to_owned(),
                revision: 1,
            }),
            generation: 12,
        };
        let active = RendererToken::new(
            "active-two".to_owned(),
            CharacterRef {
                id: "shared".to_owned(),
                revision: 2,
            },
            "a".repeat(64),
        )
        .unwrap();
        let mut overrides = DialogueOverrides::default();
        overrides
            .set_entry(
                &target,
                "ko",
                DialogueSlot::Idle,
                Some("shared edit".to_owned()),
            )
            .unwrap();
        let pending = PendingNativeDialogue {
            operation_id: "save-one".to_owned(),
            identity: authored,
            renderer_token: active.clone(),
            overrides_token: target_overrides_token(&overrides, &target),
            result: json!({}),
        };
        // Authored @1 and the active @2 are intentionally distinct.
        assert!(pending.matches_renderer_and_overrides(&active, &overrides));
        overrides
            .set_entry(
                &DialogueTarget::Character("other".to_owned()),
                "ko",
                DialogueSlot::Idle,
                Some("unrelated edit".to_owned()),
            )
            .unwrap();
        assert!(pending.matches_renderer_and_overrides(&active, &overrides));

        let replaced = RendererToken::new(
            "active-three".to_owned(),
            active.reference.clone(),
            active.content_digest.clone(),
        )
        .unwrap();
        assert!(!pending.matches_renderer_and_overrides(&replaced, &overrides));
        overrides
            .set_entry(
                &target,
                "ko",
                DialogueSlot::Idle,
                Some("newer edit".to_owned()),
            )
            .unwrap();
        assert!(!pending.matches_renderer_and_overrides(&active, &overrides));
    }
}
