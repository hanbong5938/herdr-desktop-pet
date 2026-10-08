//! In-memory selection intent. Browsing never touches the pack service or renderer.

use crate::character_types::{CharacterRef, PackAction, PackListing, PackOperation, PackRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Choice {
    Head,
    Revision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub reference: CharacterRef,
    pub choice: Choice,
    pub base_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    NoListing,
    Unavailable,
    Busy,
    NoCandidate,
    Stale,
    AlreadyApplied,
    InvalidOperationId,
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoListing => "character listing is unavailable",
            Self::Unavailable => "character revision is no longer available; select again",
            Self::Busy => "character selection operation is still in progress",
            Self::NoCandidate => "select a character before applying",
            Self::Stale => "character listing changed; select again",
            Self::AlreadyApplied => "character is already selected and active",
            Self::InvalidOperationId => "character operation id must not be empty",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    AwaitingSubmission,
    MissingStatus,
    Accepted,
    Preparing,
    Applying,
    Completed,
    Failed,
    Canceled,
    CommittedPendingApply,
    DurabilityUnknown,
    Unknown,
}

impl OperationStatus {
    pub fn from_operation(operation: &PackOperation) -> Self {
        match operation.state.as_str() {
            "accepted" => Self::Accepted,
            "preparing" => Self::Preparing,
            "applying" => Self::Applying,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            "committed_pending_apply" => Self::CommittedPendingApply,
            "durability_unknown" => Self::DurabilityUnknown,
            _ => Self::Unknown,
        }
    }

    /// Executed outcomes are terminal; unknown durability is not safe to retry.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::CommittedPendingApply
        )
    }

    /// States the service never changes once reached; only these are evicted from its
    /// bounded retention cache (`character_service::retain_capacity`).
    fn is_final(self) -> bool {
        self.is_terminal() || self == Self::DurabilityUnknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackedOperation {
    id: String,
    observed: Option<PackOperation>,
    submitted: bool,
    current: bool,
    owned_apply: bool,
    official_id: Option<String>,
    rejected: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CharacterSelection {
    listing: Option<PackListing>,
    candidate: Option<Candidate>,
    stale: bool,
    operation: Option<TrackedOperation>,
}

impl CharacterSelection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn candidate(&self) -> Option<&Candidate> {
        self.candidate.as_ref()
    }

    pub fn stale(&self) -> bool {
        self.stale
    }

    pub fn generation(&self) -> Option<u64> {
        self.listing.as_ref().map(|listing| listing.generation)
    }

    pub fn operation_id(&self) -> Option<&str> {
        self.operation
            .as_ref()
            .map(|operation| operation.id.as_str())
    }
    pub fn official_operation_id(&self) -> Option<&str> {
        self.operation
            .as_ref()
            .filter(|operation| operation.official_id.is_some())
            .map(|operation| operation.id.as_str())
    }
    pub fn official_pack_id(&self) -> Option<&str> {
        self.operation
            .as_ref()
            .and_then(|operation| operation.official_id.as_deref())
    }

    pub fn operation(&self) -> Option<&PackOperation> {
        self.operation
            .as_ref()
            .filter(|op| op.current)
            .and_then(|op| op.observed.as_ref())
    }

    pub fn last_observation(&self) -> Option<&PackOperation> {
        self.operation.as_ref().and_then(|op| op.observed.as_ref())
    }

    pub fn operation_error(&self) -> Option<&str> {
        self.operation
            .as_ref()
            .and_then(|op| op.rejected.as_deref())
    }

    /// A prior nonbusy attempt remains in diagnostics, not a freshly staged candidate's footer.
    pub fn operation_visible_for_candidate(&self) -> bool {
        self.candidate.is_none()
            || self.operation.as_ref().is_some_and(|op| op.owned_apply)
            || self.is_busy()
    }

    pub fn operation_status(&self) -> Option<OperationStatus> {
        self.operation.as_ref().and_then(|op| {
            if op.rejected.is_some() {
                return None;
            }
            Some(if !op.current && op.submitted {
                OperationStatus::MissingStatus
            } else if let Some(observed) = &op.observed {
                OperationStatus::from_operation(observed)
            } else if op.submitted {
                OperationStatus::MissingStatus
            } else {
                OperationStatus::AwaitingSubmission
            })
        })
    }

    pub fn is_busy(&self) -> bool {
        let Some(tracked) = &self.operation else {
            return false;
        };
        if tracked.rejected.is_some() {
            return false;
        }
        match self.operation_status() {
            Some(OperationStatus::Completed) if tracked.owned_apply => {
                self.candidate.is_some() || tracked.official_id.is_some()
            }
            Some(OperationStatus::CommittedPendingApply) => {
                tracked.observed.as_ref().is_none_or(|op| {
                    !op.generation.is_some_and(|generation| {
                        self.generation()
                            .is_some_and(|current| current >= generation)
                    })
                })
            }
            Some(status) => !status.is_terminal(),
            None => false,
        }
    }

    pub fn can_apply(&self) -> bool {
        let (Some(candidate), Some(listing)) = (&self.candidate, &self.listing) else {
            return false;
        };
        !self.is_busy()
            && !self.stale
            && (listing.selected != candidate.reference
                || listing.active.as_ref() != Some(&candidate.reference)
                || listing.override_active)
    }

    pub fn stage_head(&mut self, id: &str) -> Result<(), SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        let listing = self.listing.as_ref().ok_or(SelectionError::NoListing)?;
        let reference = if id == "default" {
            CharacterRef::builtin()
        } else {
            // Head identity is fixed at staging, never inferred from a later listing.
            let pack = listing
                .packs
                .iter()
                .find(|pack| pack.id == id)
                .ok_or(SelectionError::Unavailable)?;
            CharacterRef {
                id: id.to_owned(),
                revision: pack.head,
            }
        };
        let generation = listing.generation;
        self.stage(reference, Choice::Head, generation)
    }

    pub fn stage_revision(&mut self, reference: CharacterRef) -> Result<(), SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        let generation = self
            .listing
            .as_ref()
            .ok_or(SelectionError::NoListing)?
            .generation;
        self.stage(reference, Choice::Revision, generation)
    }

    fn stage(
        &mut self,
        reference: CharacterRef,
        choice: Choice,
        generation: u64,
    ) -> Result<(), SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        if !self
            .listing
            .as_ref()
            .is_some_and(|listing| Self::listed(listing, &reference, choice))
        {
            return Err(SelectionError::Unavailable);
        }
        self.candidate = Some(Candidate {
            reference,
            choice,
            base_generation: generation,
        });
        self.stale = false;
        // A fresh intent cannot inherit a previous Apply's completion or its footer result.
        if let Some(operation) = self.operation.as_mut() {
            operation.owned_apply = false;
            operation.official_id = None;
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        self.candidate = None;
        self.stale = false;
        if let Some(operation) = self.operation.as_mut() {
            operation.owned_apply = false;
        }
        // Canceling the candidate does not erase the latest attempt from diagnostics.
        Ok(())
    }

    /// Reserve this operation before submitting the returned request to the service.
    /// On a definitive submission rejection call `submission_failed` to release the reservation.
    pub fn request_apply(&mut self, operation_id: String) -> Result<PackRequest, SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        if operation_id.is_empty() {
            return Err(SelectionError::InvalidOperationId);
        }
        let candidate = self.candidate.as_ref().ok_or(SelectionError::NoCandidate)?;
        if self.stale
            || self
                .listing
                .as_ref()
                .is_none_or(|listing| listing.generation != candidate.base_generation)
        {
            return Err(SelectionError::Stale);
        }
        if !self.can_apply() {
            return Err(SelectionError::AlreadyApplied);
        }
        let action = match candidate.choice {
            Choice::Head => PackAction::Select {
                id: candidate.reference.id.clone(),
            },
            Choice::Revision => PackAction::Restore {
                id: candidate.reference.id.clone(),
                revision: candidate.reference.revision,
            },
        };
        let request = PackRequest {
            operation_id: operation_id.clone(),
            expected_generation: Some(candidate.base_generation),
            action,
        };
        self.operation = Some(TrackedOperation {
            id: operation_id,
            observed: None,
            submitted: false,
            current: true,
            owned_apply: true,
            rejected: None,
            official_id: None,
        });
        Ok(request)
    }

    /// Record one current status for the latest UI attempt; never substitute a previous observation.
    pub fn record_operation(&mut self, operation: &PackOperation) -> bool {
        let Some(tracked) = self.operation.as_mut() else {
            return false;
        };
        if tracked.id != operation.operation_id || tracked.rejected.is_some() {
            return false;
        }
        tracked.submitted = true;
        tracked.current = true;
        if tracked.observed.as_ref() != Some(operation) {
            tracked.observed = Some(operation.clone());
        }
        if tracked.owned_apply
            && OperationStatus::from_operation(operation) == OperationStatus::CommittedPendingApply
        {
            self.stale = true;
        }
        self.resolve_completed();
        true
    }

    /// An official install is one owned Apply intent, not a candidate staged from
    /// the previously installed head. Keep an existing candidate until native
    /// activation of the new exact revision has actually been observed.
    pub fn reserve_official(
        &mut self,
        operation_id: String,
        official_id: String,
    ) -> Result<(), SelectionError> {
        if self.is_busy() {
            return Err(SelectionError::Busy);
        }
        if operation_id.is_empty() {
            return Err(SelectionError::InvalidOperationId);
        }
        if self.listing.is_none() {
            return Err(SelectionError::NoListing);
        }
        self.operation = Some(TrackedOperation {
            id: operation_id,
            observed: None,
            submitted: false,
            current: true,
            owned_apply: true,
            official_id: Some(official_id),
            rejected: None,
        });
        Ok(())
    }

    pub fn reserve_other(&mut self, operation_id: String) {
        self.operation = Some(TrackedOperation {
            id: operation_id,
            observed: None,
            submitted: false,
            current: true,
            owned_apply: false,
            official_id: None,
            rejected: None,
        });
    }

    /// A definitive rejection is not an acknowledged PackOperation.
    pub fn submission_failed(&mut self, operation_id: &str, error: String) -> bool {
        if let Some(tracked) = self
            .operation
            .as_mut()
            .filter(|op| op.id == operation_id && !op.submitted)
        {
            tracked.rejected = Some(error);
            tracked.current = false;
            true
        } else {
            false
        }
    }

    /// A miss after a final observation is retention eviction, not new information: keep it current.
    /// Otherwise a miss leaves the ID and last observation intact, but current status is uncertain.
    pub fn reconcile_operation(&mut self, operation: Option<&PackOperation>) {
        if let Some(operation) = operation {
            self.record_operation(operation);
        } else if let Some(tracked) = self.operation.as_mut() {
            let evicted = tracked
                .observed
                .as_ref()
                .is_some_and(|op| OperationStatus::from_operation(op).is_final());
            if tracked.rejected.is_none() && !evicted {
                tracked.submitted = true;
                tracked.current = false;
            }
        }
    }

    /// The menu may be hidden/reopened without changing this model.
    pub fn reconcile(&mut self, listing: &PackListing) {
        if self.listing.as_ref() != Some(listing) {
            self.listing = Some(listing.clone());
        }
        self.resolve_completed();
        if let Some(candidate) = &self.candidate {
            if listing.generation != candidate.base_generation
                || !Self::listed(listing, &candidate.reference, candidate.choice)
            {
                self.stale = true;
            }
        }
    }

    fn resolve_completed(&mut self) {
        let (Some(listing), Some(operation)) = (&self.listing, &self.operation) else {
            return;
        };
        let Some(observed) = operation.observed.as_ref() else {
            return;
        };
        if !operation.owned_apply
            || !operation.current
            || OperationStatus::from_operation(observed) != OperationStatus::Completed
            || !observed.ui_applied
            || !observed.committed
            || !observed
                .generation
                .is_some_and(|generation| listing.generation >= generation)
        {
            return;
        }
        if let Some(id) = operation.official_id.as_ref() {
            if observed.generation == Some(listing.generation) {
                // At this generation the installed head identifies this import's
                // revision; the matching live selection confirms native apply.
                if listing.selected.id != *id
                    || !listing
                        .packs
                        .iter()
                        .any(|pack| pack.id == *id && pack.head == listing.selected.revision)
                    || listing.active.as_ref() != Some(&listing.selected)
                    || listing.override_active
                {
                    return;
                }
            }
            // A later generation supersedes the completed intent. PackOperation
            // does not carry its exact installed revision, so a newer listing's
            // head/selection cannot prove it is still the imported revision.
            // Release admission without rewriting that newer listing.
        } else if listing.active.as_ref() != Some(&listing.selected)
            || listing.override_active
            || !self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.reference == listing.selected)
        {
            return;
        }
        self.candidate = None;
        self.stale = false;
        if let Some(operation) = self.operation.as_mut() {
            operation.official_id = None;
        }
    }

    fn listed(listing: &PackListing, reference: &CharacterRef, choice: Choice) -> bool {
        if reference.is_builtin() {
            return choice == Choice::Head;
        }
        listing.packs.iter().any(|pack| {
            pack.id == reference.id
                && match choice {
                    Choice::Head => pack.head == reference.revision,
                    Choice::Revision => pack.revisions.contains(&reference.revision),
                }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character_types::PackRecord;

    fn reference(id: &str, revision: u64) -> CharacterRef {
        CharacterRef {
            id: id.into(),
            revision,
        }
    }

    fn listing() -> PackListing {
        PackListing {
            generation: 7,
            selected: CharacterRef::builtin(),
            active: Some(CharacterRef::builtin()),
            override_active: false,
            packs: vec![PackRecord {
                id: "cat".into(),
                name: "Cat".into(),
                head: 3,
                revisions: vec![1, 2, 3],
            }],
            error: None,
        }
    }

    fn status(id: &str, state: &str, generation: Option<u64>, ui_applied: bool) -> PackOperation {
        PackOperation {
            operation_id: id.into(),
            state: state.into(),
            committed: generation.is_some(),
            ui_applied,
            generation,
            error: None,
        }
    }

    fn installed_official_listing() -> PackListing {
        let mut current = listing();
        current.generation = 8;
        current.selected = reference("fox", 4);
        current.active = Some(reference("fox", 4));
        current.packs.push(PackRecord {
            id: "fox".into(),
            name: "Fox".into(),
            head: 4,
            revisions: vec![4],
        });
        current
    }

    #[test]
    fn staging_and_cancel_only_change_candidate() {
        let original = listing();
        let mut selection = CharacterSelection::new();
        selection.reconcile(&original);
        selection.stage_head("cat").unwrap();
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        assert_eq!(selection.listing.as_ref(), Some(&original));
        selection.stage_revision(reference("cat", 1)).unwrap();
        assert_eq!(selection.candidate().unwrap().choice, Choice::Revision);
        selection.cancel().unwrap();
        assert!(selection.candidate().is_none());
        assert_eq!(selection.listing.as_ref(), Some(&original));
    }

    #[test]
    fn stale_generation_or_revision_requires_fresh_stage() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        current.generation += 1;
        current.packs[0].head = 4;
        current.packs[0].revisions.push(4);
        selection.reconcile(&current);
        assert!(selection.stale());
        assert_eq!(
            selection.request_apply("old".into()).unwrap_err(),
            SelectionError::Stale
        );
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        selection.stage_head("cat").unwrap();
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 4)
        );
        let request = selection.request_apply("new".into()).unwrap();
        assert_eq!(request.expected_generation, Some(8));
        assert!(matches!(request.action, PackAction::Select { id } if id == "cat"));
        assert_eq!(selection.cancel(), Err(SelectionError::Busy));
    }

    #[test]
    fn explicit_revision_identity_survives_head_changes_until_reselection() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_revision(reference("cat", 1)).unwrap();
        let request = selection.request_apply("restore".into()).unwrap();
        assert!(matches!(request.action, PackAction::Restore { id, revision: 1 } if id == "cat"));
        selection.record_operation(&status("restore", "failed", None, false));
        current.generation += 1;
        current.packs[0].revisions.retain(|revision| *revision != 1);
        selection.reconcile(&current);
        assert!(selection.stale());
        assert_eq!(
            selection.stage_revision(reference("cat", 1)),
            Err(SelectionError::Unavailable)
        );
    }

    #[test]
    fn deleted_pack_does_not_retarget_candidate() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        current.generation += 1;
        current.packs.clear();
        selection.reconcile(&current);
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        assert!(selection.stale());
        assert_eq!(
            selection.request_apply("deleted".into()).unwrap_err(),
            SelectionError::Stale
        );
        assert_eq!(
            selection.stage_head("cat"),
            Err(SelectionError::Unavailable)
        );
    }

    #[test]
    fn own_operation_blocks_duplicates_until_its_real_outcome() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        selection.request_apply("ours".into()).unwrap();
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::AwaitingSubmission)
        );
        assert_eq!(
            selection.request_apply("other".into()).unwrap_err(),
            SelectionError::Busy
        );
        assert!(!selection.record_operation(&status("other", "completed", Some(8), true)));
        selection.reconcile_operation(None);
        assert_eq!(selection.cancel(), Err(SelectionError::Busy));
        selection.record_operation(&status("ours", "accepted", None, false));
        assert!(!selection.submission_failed("ours", "late rejection".into()));
        selection.reconcile_operation(None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::MissingStatus)
        );
        assert_eq!(selection.operation_id(), Some("ours"));
        assert_eq!(selection.last_observation().unwrap().state, "accepted");
        selection.record_operation(&status("ours", "committed_pending_apply", Some(8), false));
        assert_eq!(
            selection.request_apply("retry".into()).unwrap_err(),
            SelectionError::Busy
        );
        selection.record_operation(&status("ours", "durability_unknown", Some(8), false));
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
    }

    #[test]
    fn pending_apply_recovery_requires_fresh_explicit_selection() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection.request_apply("first".into()).unwrap();
        selection.record_operation(&status("first", "committed_pending_apply", Some(8), false));
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::CommittedPendingApply)
        );
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy)); // Await committed listing.
        current.generation = 8;
        current.selected = reference("cat", 3);
        selection.reconcile(&current); // Live still builtin.
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        assert_eq!(
            selection.request_apply("blind-retry".into()).unwrap_err(),
            SelectionError::Stale
        );
        selection.stage_head("cat").unwrap();
        assert!(selection.can_apply());
        let request = selection.request_apply("recovery".into()).unwrap();
        assert_eq!(request.expected_generation, Some(8));
        assert!(matches!(request.action, PackAction::Select { id } if id == "cat"));
    }

    #[test]
    fn submission_rejection_and_failed_operation_keep_intent() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        selection.request_apply("rejected".into()).unwrap();
        assert!(!selection.submission_failed("other", "other error".into()));
        assert!(selection.submission_failed("rejected", "rejected by service".into()));
        assert_eq!(selection.operation_id(), Some("rejected"));
        assert_eq!(selection.operation_error(), Some("rejected by service"));
        assert_eq!(selection.operation_status(), None);
        assert!(selection.can_apply());
        selection.request_apply("failed".into()).unwrap();
        selection.record_operation(&status("failed", "failed", None, false));
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        assert!(selection.can_apply());
    }

    #[test]
    fn only_confirmed_success_clears_candidate() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection.request_apply("ours".into()).unwrap();
        selection.record_operation(&status("ours", "completed", Some(8), true));
        assert!(selection.candidate().is_some());
        assert!(selection.operation_visible_for_candidate());
        assert_eq!(
            selection.request_apply("premature".into()).unwrap_err(),
            SelectionError::Busy
        );
        current.generation = 8;
        current.selected = reference("cat", 3);
        selection.reconcile(&current);
        assert!(selection.candidate().is_some()); // Persisted, not live.
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        assert!(selection.candidate().is_none());
        assert_eq!(selection.operation_id(), Some("ours"));
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::Completed)
        );
    }

    #[test]
    fn generic_failure_then_owned_success_keeps_latest_after_candidate_clear() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.reserve_other("import".into());
        selection.record_operation(&status("import", "failed", None, false));
        selection.stage_head("cat").unwrap();
        assert_eq!(selection.operation_id(), Some("import"));
        assert_eq!(selection.operation_status(), Some(OperationStatus::Failed));
        assert!(!selection.operation_visible_for_candidate());
        selection.request_apply("apply".into()).unwrap();
        selection.record_operation(&status("apply", "completed", Some(8), true));
        current.generation = 8;
        current.selected = reference("cat", 3);
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        assert!(selection.candidate().is_none());
        assert_eq!(selection.operation_id(), Some("apply"));
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::Completed)
        );
    }

    #[test]
    fn completed_apply_cannot_resolve_fresh_revision_intent() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection.request_apply("first".into()).unwrap();
        selection.record_operation(&status("first", "completed", Some(8), true));
        current.generation = 8;
        current.selected = reference("cat", 3);
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        assert!(selection.candidate().is_none());

        selection.stage_revision(reference("cat", 1)).unwrap();
        assert_eq!(selection.operation_id(), Some("first"));
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::Completed)
        );
        assert!(!selection.is_busy());
        assert!(!selection.operation_visible_for_candidate());
        current.generation = 9;
        current.selected = reference("cat", 1);
        current.active = Some(reference("cat", 1));
        selection.record_operation(&status("first", "completed", Some(8), true));
        selection.reconcile(&current);
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 1)
        );
        assert!(selection.stale());
        assert_eq!(selection.operation_id(), Some("first"));
    }

    #[test]
    fn cancellation_keeps_latest_and_new_attempt_replaces_it() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        selection.request_apply("rejected".into()).unwrap();
        selection.submission_failed("rejected", "not accepted".into());
        selection.cancel().unwrap();
        assert_eq!(selection.operation_id(), Some("rejected"));
        assert_eq!(selection.operation_error(), Some("not accepted"));

        selection.stage_revision(reference("cat", 1)).unwrap();
        assert!(!selection.operation_visible_for_candidate());
        selection.reserve_other("import".into());
        assert_eq!(selection.operation_id(), Some("import"));
        assert_eq!(selection.operation_error(), None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::AwaitingSubmission)
        );
        assert!(selection.operation_visible_for_candidate());
        selection.record_operation(&status("import", "failed", None, false));
        assert!(!selection.operation_visible_for_candidate());
        selection.request_apply("restore".into()).unwrap();
        assert_eq!(selection.operation_id(), Some("restore"));
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::AwaitingSubmission)
        );
        assert!(selection.operation_visible_for_candidate());
    }

    #[test]
    fn generic_terminal_result_survives_cache_eviction() {
        for (state, expected) in [
            ("completed", OperationStatus::Completed),
            ("failed", OperationStatus::Failed),
            ("canceled", OperationStatus::Canceled),
        ] {
            let mut selection = CharacterSelection::new();
            selection.reconcile(&listing());
            selection.reserve_other("update".into());
            selection.record_operation(&status("update", state, Some(8), false));
            assert!(!selection.is_busy(), "{state}");
            selection.reconcile_operation(None);
            assert_eq!(selection.operation_id(), Some("update"), "{state}");
            assert_eq!(selection.operation_status(), Some(expected), "{state}");
            assert_eq!(selection.operation().unwrap().state, state);
            assert!(!selection.is_busy(), "{state}");
            assert!(selection.stage_head("cat").is_ok(), "{state}");
        }
    }

    #[test]
    fn owned_completed_resolves_after_cache_eviction() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection.request_apply("ours".into()).unwrap();
        selection.record_operation(&status("ours", "completed", Some(8), true));
        selection.reconcile_operation(None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::Completed)
        );
        assert!(selection.is_busy()); // Candidate awaits the committed listing.
        current.generation = 8;
        current.selected = reference("cat", 3);
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        assert!(selection.candidate().is_none());
        assert!(!selection.is_busy());
    }

    #[test]
    fn pending_apply_after_cache_eviction_waits_for_committed_listing() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection.request_apply("first".into()).unwrap();
        selection.record_operation(&status("first", "committed_pending_apply", Some(8), false));
        selection.reconcile_operation(None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::CommittedPendingApply)
        );
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
        current.generation = 8;
        current.selected = reference("cat", 3);
        selection.reconcile(&current);
        assert!(!selection.is_busy());
        assert!(selection.stale());
        assert_eq!(
            selection.request_apply("blind-retry".into()).unwrap_err(),
            SelectionError::Stale
        );
        selection.stage_head("cat").unwrap();
        assert!(selection.can_apply());
    }

    #[test]
    fn unknown_outcome_and_missing_status_keep_owned_id_and_prevent_retry() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_revision(reference("cat", 1)).unwrap();
        let request = selection.request_apply("restore".into()).unwrap();
        assert!(matches!(
            request.action,
            PackAction::Restore { revision: 1, .. }
        ));
        selection.record_operation(&status("restore", "unexpected_state", None, false));
        assert_eq!(selection.operation_status(), Some(OperationStatus::Unknown));
        assert!(selection.operation_visible_for_candidate());
        assert_eq!(selection.cancel(), Err(SelectionError::Busy));
        selection.reconcile_operation(None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::MissingStatus)
        );
        assert_eq!(selection.operation_id(), Some("restore"));
        assert_eq!(
            selection.last_observation().unwrap().state,
            "unexpected_state"
        );
        assert!(selection.operation_visible_for_candidate());
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
    }

    #[test]
    fn equality_disables_apply_unless_override_or_live_mismatch() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("default").unwrap();
        assert!(!selection.can_apply());
        assert_eq!(
            selection.request_apply("same".into()).unwrap_err(),
            SelectionError::AlreadyApplied
        );
        current.override_active = true;
        selection.reconcile(&current);
        selection.stage_head("default").unwrap();
        assert!(selection.can_apply());
        selection.cancel().unwrap();
        current.override_active = false;
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        selection.stage_head("default").unwrap();
        assert!(selection.can_apply());
    }
    #[test]
    fn official_completion_requires_exact_installed_head_and_native_active() {
        let mut selection = CharacterSelection::new();
        let mut current = listing();
        selection.reconcile(&current);
        selection.stage_head("cat").unwrap();
        selection
            .reserve_official("download".into(), "fox".into())
            .unwrap();
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
        selection.record_operation(&status("download", "completed", Some(8), false));
        assert!(selection.is_busy());
        selection.record_operation(&status("download", "completed", Some(8), true));
        assert!(selection.is_busy());
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        current.generation = 8;
        current.packs.push(PackRecord {
            id: "fox".into(),
            name: "Fox".into(),
            head: 4,
            revisions: vec![4],
        });
        current.selected = reference("fox", 4);
        selection.reconcile(&current);
        assert!(selection.is_busy()); // Disk selection is not native activation.
        current.active = Some(reference("fox", 4));
        current.override_active = true;
        selection.reconcile(&current);
        assert!(selection.is_busy());
        current.packs[1].head = 5;
        current.packs[1].revisions.push(5);
        current.override_active = false;
        selection.reconcile(&current);
        assert!(selection.is_busy()); // Matching ID is not enough; revision must be the head.
        current.packs[1].head = 4;
        selection.reconcile(&current);
        assert!(selection.candidate().is_none());
        assert!(!selection.is_busy());
        assert!(selection.official_operation_id().is_none());
    }

    #[test]
    fn official_completion_after_unrelated_remove_releases_admission() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        selection
            .reserve_official("download".into(), "fox".into())
            .unwrap();
        selection.record_operation(&status("download", "completed", Some(8), true));

        let mut current = installed_official_listing();
        current.generation = 9;
        current.packs.retain(|pack| pack.id != "cat");
        selection.reconcile(&current);

        assert!(!selection.is_busy());
        assert!(selection.candidate().is_none());
        assert_eq!(selection.official_operation_id(), None);
        assert_eq!(
            selection.operation_status(),
            Some(OperationStatus::Completed)
        );
        assert_eq!(selection.last_observation().unwrap().generation, Some(8));
        assert_eq!(selection.generation(), Some(9));
        selection.stage_head("default").unwrap();
        let request = selection.request_apply("next".into()).unwrap();
        assert_eq!(request.expected_generation, Some(9));
        assert!(matches!(request.action, PackAction::Select { id } if id == "default"));
    }

    #[test]
    fn superseded_official_completion_preserves_new_selection_head_removal_and_override() {
        let mut selected_other = installed_official_listing();
        selected_other.generation = 9;
        selected_other.selected = reference("cat", 3);
        selected_other.active = Some(reference("cat", 3));

        let mut changed_head = installed_official_listing();
        changed_head.generation = 9;
        changed_head.packs[1].head = 5;
        changed_head.packs[1].revisions.push(5);
        changed_head.selected = reference("fox", 5);
        changed_head.active = Some(reference("fox", 5));

        let mut removed = installed_official_listing();
        removed.generation = 9;
        removed.packs.pop();
        removed.selected = CharacterRef::builtin();
        removed.active = Some(CharacterRef::builtin());

        let mut overridden = installed_official_listing();
        overridden.generation = 9;
        overridden.override_active = true;

        for (current, next_id, next_revision) in [
            (selected_other, "fox", 4),
            (changed_head, "fox", 5),
            (removed, "cat", 3),
            (overridden, "cat", 3),
        ] {
            let mut selection = CharacterSelection::new();
            selection.reconcile(&listing());
            selection.stage_revision(reference("cat", 1)).unwrap();
            selection
                .reserve_official("download".into(), "fox".into())
                .unwrap();
            selection.record_operation(&status("download", "completed", Some(8), true));
            selection.reconcile(&current);

            assert!(!selection.is_busy(), "{current:?}");
            assert!(selection.candidate().is_none(), "{current:?}");
            assert_eq!(selection.generation(), Some(9));
            assert_eq!(selection.listing.as_ref(), Some(&current));
            assert_eq!(selection.operation_id(), Some("download"));
            assert_eq!(
                selection.operation_status(),
                Some(OperationStatus::Completed)
            );
            selection.stage_head(next_id).unwrap();
            assert_eq!(
                selection.candidate().unwrap().reference,
                reference(next_id, next_revision)
            );
            selection.reconcile(&current);
            assert_eq!(
                selection.candidate().unwrap().reference,
                reference(next_id, next_revision)
            );
        }
    }

    #[test]
    fn newer_listing_before_official_terminal_result_releases_admission() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        selection
            .reserve_official("download".into(), "fox".into())
            .unwrap();

        let mut current = installed_official_listing();
        current.generation = 9;
        current.selected = reference("cat", 3);
        current.active = Some(reference("cat", 3));
        selection.reconcile(&current);
        assert!(selection.is_busy());
        assert_eq!(selection.stage_head("fox"), Err(SelectionError::Busy));

        let mut completed = status("download", "completed", Some(8), true);
        completed.error = Some("post-apply cleanup warning".into());
        selection.record_operation(&completed);
        assert!(!selection.is_busy());
        assert!(selection.candidate().is_none());
        assert_eq!(selection.official_operation_id(), None);
        assert_eq!(selection.last_observation(), Some(&completed));
        assert_eq!(selection.listing.as_ref(), Some(&current));
        selection.stage_head("fox").unwrap();
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("fox", 4)
        );
    }

    #[test]
    fn failed_official_operation_keeps_old_candidate_and_unknown_blocks_retry() {
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing());
        selection.stage_revision(reference("cat", 1)).unwrap();
        selection
            .reserve_official("first".into(), "fox".into())
            .unwrap();
        selection.record_operation(&status("first", "committed_pending_apply", Some(8), false));
        assert!(selection.is_busy()); // Committed registry not observed yet.
        let mut committed = listing();
        committed.generation = 8;
        committed.selected = reference("fox", 4);
        committed.packs.push(PackRecord {
            id: "fox".into(),
            name: "Fox".into(),
            head: 4,
            revisions: vec![4],
        });
        selection.reconcile(&committed);
        assert!(!selection.is_busy());
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 1)
        );
        selection
            .reserve_official("retry".into(), "fox".into())
            .unwrap();
        selection.record_operation(&status("retry", "durability_unknown", Some(9), false));
        assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
        let mut failed = CharacterSelection::new();
        failed.reconcile(&listing());
        failed.stage_revision(reference("cat", 1)).unwrap();
        failed
            .reserve_official("failed-download".into(), "fox".into())
            .unwrap();
        failed.record_operation(&status("failed-download", "failed", None, false));
        assert_eq!(failed.candidate().unwrap().reference, reference("cat", 1));
        assert!(!failed.is_busy());
    }

    #[test]
    fn official_submission_rejection_releases_admission_without_losing_selection() {
        let mut selection = CharacterSelection::new();
        assert_eq!(
            selection.reserve_official("download".into(), "fox".into()),
            Err(SelectionError::NoListing)
        );
        selection.reconcile(&listing());
        selection.stage_head("cat").unwrap();
        assert_eq!(
            selection.reserve_official("".into(), "fox".into()),
            Err(SelectionError::InvalidOperationId)
        );
        selection
            .reserve_official("download".into(), "fox".into())
            .unwrap();
        assert!(selection.is_busy());
        assert_eq!(selection.official_operation_id(), Some("download"));
        assert!(selection.submission_failed("download", "worker unavailable".into()));
        assert!(!selection.is_busy());
        assert_eq!(
            selection.candidate().unwrap().reference,
            reference("cat", 3)
        );
        assert_eq!(selection.operation_error(), Some("worker unavailable"));
        selection.stage_head("cat").unwrap();
        assert!(selection.official_operation_id().is_none());
    }
    #[test]
    fn official_completion_requires_both_listing_and_native_ack_in_either_order() {
        for operation_first in [true, false] {
            let mut selection = CharacterSelection::new();
            selection.reconcile(&listing());
            selection.stage_revision(reference("cat", 1)).unwrap();
            selection
                .reserve_official("download".into(), "fox".into())
                .unwrap();
            let completed = status("download", "completed", Some(8), true);
            let committed = installed_official_listing();
            if operation_first {
                selection.record_operation(&completed);
                assert!(selection.is_busy());
                assert_eq!(selection.generation(), Some(7));
                selection.reconcile(&committed);
            } else {
                selection.reconcile(&committed);
                assert!(selection.is_busy());
                selection.record_operation(&completed);
            }
            assert!(!selection.is_busy());
            assert!(selection.candidate().is_none());
            assert_eq!(selection.listing.as_ref(), Some(&committed));
            assert_eq!(selection.official_operation_id(), None);
        }
    }

    #[test]
    fn pending_official_listing_does_not_infer_native_apply_and_unknown_blocks_retry() {
        for operation_first in [true, false] {
            let mut selection = CharacterSelection::new();
            selection.reconcile(&listing());
            selection.stage_head("cat").unwrap();
            selection
                .reserve_official("download".into(), "fox".into())
                .unwrap();
            let mut committed = installed_official_listing();
            committed.active = Some(CharacterRef::builtin());
            let pending = status("download", "committed_pending_apply", Some(8), false);
            if operation_first {
                selection.record_operation(&pending);
                assert!(selection.is_busy());
                selection.reconcile(&committed);
            } else {
                selection.reconcile(&committed);
                assert!(selection.is_busy());
                selection.record_operation(&pending);
            }
            assert!(!selection.is_busy());
            assert_eq!(selection.official_operation_id(), Some("download"));
            assert!(selection.stale());
            assert_eq!(
                selection.request_apply("blind".into()),
                Err(SelectionError::Stale)
            );
            selection.stage_head("fox").unwrap();
            assert!(selection.can_apply());
            selection
                .reserve_official("uncertain".into(), "fox".into())
                .unwrap();
            selection.record_operation(&status("uncertain", "durability_unknown", Some(9), false));
            assert!(selection.is_busy());
            assert_eq!(selection.stage_head("cat"), Err(SelectionError::Busy));
        }
    }
}
