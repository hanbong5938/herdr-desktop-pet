//! Bounded automation mailboxes. Only the UI thread reduces requests or publishes native evidence.
use crate::bubble::BubblePlacement;
use crate::character_types::validate_pack_id;
use crate::dialogue::DialogueTarget;
use crate::dialogue_automation::{DialogueBaseline, DialogueIdentity, DialogueSelection};
use crate::preferences::PreferencePatch;
use crate::session_view::SessionKey;
use crate::state::{normalize_scale, Scene};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

const MAX_QUEUED: usize = 64;
const MAX_OPERATIONS: usize = 256;
const MAX_COMPLETED: usize = 192;
const MAX_ID_BYTES: usize = 128;
const MAX_ERROR_BYTES: usize = 512;
const MAX_DOMAIN_PROMPT_BYTES: usize = 512 * 1024;
const MAX_DOMAIN_REQUEST_BYTES: usize = MAX_DOMAIN_PROMPT_BYTES + 4096;
const MAX_DOMAIN_OTHER_REQUEST_BYTES: usize = 16 * 1024;
// Keep the complete bounded settings snapshot, including catalog metadata,
// queryable after the UI has already committed a successful save.
const MAX_DOMAIN_RESULT_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_ID_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionIdentity {
    pub instance_id: String,
    pub source_id: u64,
    pub generation: u64,
    pub terminal_id: String,
}

impl From<&SessionIdentity> for SessionKey {
    fn from(identity: &SessionIdentity) -> Self {
        Self {
            source_id: identity.source_id,
            generation: identity.generation,
            terminal_id: identity.terminal_id.clone(),
        }
    }
}

impl From<SessionIdentity> for SessionKey {
    fn from(identity: SessionIdentity) -> Self {
        Self {
            source_id: identity.source_id,
            generation: identity.generation,
            terminal_id: identity.terminal_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DomainAction {
    PreferencesGet {},
    PreferencesSet {
        patch: PreferencePatch,
        expected_revision: Option<u64>,
    },
    SessionPrompt {
        key: SessionIdentity,
        text: String,
    },
    DialogueList {},
    DialogueRead {
        selection: DialogueSelection,
    },
    DialogueSet {
        selection: DialogueSelection,
        baseline: DialogueBaseline,
        text: String,
    },
    DialogueResetEntry {
        selection: DialogueSelection,
        baseline: DialogueBaseline,
    },
    DialogueResetCharacter {
        identity: DialogueIdentity,
        metadata_token: String,
        target_overrides_token: String,
    },
    WorktreeInspect {
        key: SessionIdentity,
    },
    WorktreeRemove {
        token: String,
    },
}

impl DomainAction {
    fn kind(&self) -> &'static str {
        match self {
            Self::PreferencesGet {} => "preferences_get",
            Self::PreferencesSet { .. } => "preferences_set",
            Self::SessionPrompt { .. } => "session_prompt",
            Self::DialogueList {} => "dialogue_list",
            Self::DialogueRead { .. } => "dialogue_read",
            Self::DialogueSet { .. } => "dialogue_set",
            Self::DialogueResetEntry { .. } => "dialogue_reset_entry",
            Self::DialogueResetCharacter { .. } => "dialogue_reset_character",
            Self::WorktreeInspect { .. } => "worktree_inspect",
            Self::WorktreeRemove { .. } => "worktree_remove",
        }
    }
}

fn valid_digest(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_session_identity(key: &SessionIdentity, instance_id: &str) -> Result<(), String> {
    if key.instance_id != instance_id {
        return Err("session belongs to another daemon instance".into());
    }
    if key.terminal_id.is_empty() || key.terminal_id.len() > MAX_TERMINAL_ID_BYTES {
        return Err("invalid terminal ID".into());
    }
    Ok(())
}

fn validate_dialogue_identity(identity: &DialogueIdentity) -> Result<(), String> {
    match &identity.target {
        DialogueTarget::Character(id) => {
            if id != "default" {
                validate_pack_id(id)?;
            }
            let Some(reference) = identity.reference.as_ref() else {
                return Err("dialogue character reference does not match target".into());
            };
            if reference.id != *id || (reference.revision == 0) != (id == "default") {
                return Err("dialogue character reference does not match target".into());
            }
        }
        DialogueTarget::ExternalAssets(path) => {
            if path.is_empty()
                || path.len() > MAX_TERMINAL_ID_BYTES
                || !Path::new(path).is_absolute()
                || path.chars().any(char::is_control)
                || identity.reference.is_some()
            {
                return Err("invalid external dialogue identity".into());
            }
        }
    }
    Ok(())
}

fn validate_dialogue_baseline(baseline: &DialogueBaseline) -> Result<(), String> {
    if !valid_digest(&baseline.metadata_token) || !valid_digest(&baseline.target_overrides_token) {
        return Err("invalid dialogue baseline token".into());
    }
    if baseline
        .override_entry
        .as_ref()
        .is_some_and(|entry| entry.len() > 2048 || entry.trim().is_empty())
    {
        return Err("invalid dialogue baseline entry".into());
    }
    Ok(())
}

fn validate_domain_action(action: &DomainAction, instance_id: &str) -> Result<(), String> {
    match action {
        DomainAction::SessionPrompt { key, text } => {
            validate_session_identity(key, instance_id)?;
            if text.is_empty() {
                return Err("prompt text is empty".into());
            }
            if text.len() > MAX_DOMAIN_PROMPT_BYTES {
                return Err("prompt text exceeds 512 KiB limit".into());
            }
        }
        DomainAction::DialogueRead { selection } => {
            validate_dialogue_identity(&selection.identity)?;
        }
        DomainAction::DialogueSet {
            selection,
            baseline,
            text,
        } => {
            validate_dialogue_identity(&selection.identity)?;
            validate_dialogue_baseline(baseline)?;
            if text.len() > 2048 {
                return Err("invalid dialogue text".into());
            }
        }
        DomainAction::DialogueResetEntry {
            selection,
            baseline,
        } => {
            validate_dialogue_identity(&selection.identity)?;
            validate_dialogue_baseline(baseline)?;
        }
        DomainAction::DialogueResetCharacter {
            identity,
            metadata_token,
            target_overrides_token,
        } => {
            validate_dialogue_identity(identity)?;
            if !valid_digest(metadata_token) || !valid_digest(target_overrides_token) {
                return Err("invalid dialogue baseline token".into());
            }
        }
        DomainAction::WorktreeInspect { key } => validate_session_identity(key, instance_id)?,
        DomainAction::WorktreeRemove { token } if !valid_digest(token) => {
            return Err("invalid worktree confirmation token".into());
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainRequest {
    pub instance_id: String,
    pub operation_id: String,
    pub action: DomainAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DomainOperationState {
    Accepted,
    Pending,
    Applied,
    AgentPrompted,
    Failed,
    UnknownDelivery,
    Superseded,
    Shutdown,
}

impl DomainOperationState {
    pub(crate) fn terminal(self) -> bool {
        !matches!(self, Self::Accepted | Self::Pending)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainOperation {
    pub instance_id: String,
    pub operation_id: String,
    pub kind: String,
    pub state: DomainOperationState,
    pub committed: bool,
    pub native_applied: bool,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

/// `serde_json::to_writer` streams into the hasher rather than allocating a
/// second copy of a prompt; the byte counter bounds even escaped JSON payloads.
struct RequestHasher {
    hash: Sha256,
    bytes: usize,
    limit: usize,
}

impl Write for RequestHasher {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|length| *length <= self.limit)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "domain payload exceeds limit")
            })?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn domain_fingerprint(request: &DomainRequest) -> Result<[u8; 32], String> {
    let mut writer = RequestHasher {
        hash: Sha256::new(),
        bytes: 0,
        limit: if matches!(&request.action, DomainAction::SessionPrompt { .. }) {
            MAX_DOMAIN_REQUEST_BYTES
        } else {
            MAX_DOMAIN_OTHER_REQUEST_BYTES
        },
    };
    serde_json::to_writer(&mut writer, request)
        .map_err(|_| "domain request exceeds limit or cannot be encoded".to_owned())?;
    Ok(writer.hash.finalize().into())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_passthrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble_visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble_placement: Option<BubblePlacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

impl PresentationPatch {
    /// Accumulate only fields explicitly named by accepted requests.
    pub(crate) fn merge_requested(&mut self, newer: &Self) {
        if let Some(value) = newer.visible {
            self.visible = Some(value);
        }
        if let Some(value) = newer.passthrough {
            self.passthrough = Some(value);
        }
        if let Some(value) = newer.alpha_passthrough {
            self.alpha_passthrough = Some(value);
        }
        if let Some(value) = newer.bubble_visible {
            self.bubble_visible = Some(value);
        }
        if let Some(value) = newer.bubble_placement {
            self.bubble_placement = Some(value);
        }
        if let Some(value) = newer.scale {
            self.scale = Some(value);
        }
    }

    /// Persist requested fields using the final, normalized scene, not raw
    /// request values or the previous on-disk state.
    pub(crate) fn requested_scene_values(&self, scene: &Scene) -> Self {
        Self {
            visible: self.visible.map(|_| scene.visible),
            passthrough: self.passthrough.map(|_| scene.passthrough),
            alpha_passthrough: self.alpha_passthrough.map(|_| scene.alpha_passthrough),
            bubble_visible: self.bubble_visible.map(|_| scene.bubble_visible),
            bubble_placement: self.bubble_placement.map(|_| scene.bubble_placement),
            scale: self.scale.map(|_| scene.scale),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PresentationAction {
    Set { patch: PresentationPatch },
    ResetPosition,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationRequest {
    pub instance_id: String,
    pub operation_id: String,
    pub expected_revision: Option<u64>,
    pub action: PresentationAction,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PresentationTarget {
    pub visible: bool,
    pub passthrough: bool,
    pub alpha_passthrough: bool,
    pub bubble_visible: bool,
    pub bubble_placement: BubblePlacement,
    pub scale: f64,
    pub reset_position_revision: u64,
}

impl PresentationTarget {
    pub(crate) fn from_scene(scene: &Scene) -> Self {
        Self {
            visible: scene.visible,
            passthrough: scene.passthrough,
            alpha_passthrough: scene.alpha_passthrough,
            bubble_visible: scene.bubble_visible,
            bubble_placement: scene.bubble_placement,
            scale: scene.scale,
            reset_position_revision: scene.reset_position_revision,
        }
    }

    /// Normalization belongs to AppState; use this to calculate an intended target,
    /// then let the UI reducer apply the same action before calling start_request.
    pub(crate) fn applying(self, action: &PresentationAction) -> Result<Self, String> {
        let mut target = self;
        match action {
            PresentationAction::Set { patch } => {
                if let Some(value) = patch.visible {
                    target.visible = value;
                }
                if let Some(value) = patch.passthrough {
                    target.passthrough = value;
                }
                if let Some(value) = patch.alpha_passthrough {
                    target.alpha_passthrough = value;
                }
                if let Some(value) = patch.bubble_visible {
                    target.bubble_visible = value;
                }
                if let Some(value) = patch.bubble_placement {
                    target.bubble_placement = value;
                }
                if let Some(value) = patch.scale {
                    target.scale = normalize_scale(value).ok_or("scale must be finite")?;
                }
            }
            PresentationAction::ResetPosition => {
                target.reset_position_revision = target.reset_position_revision.saturating_add(1);
            }
        }
        Ok(target)
    }
}

/// A partial save can preserve older on-disk fields after a different operation
/// failed. Credit only fields this request actually chose; never infer whole-
/// target persistence from an unrelated successful write.
fn persisted_request_fields(
    action: &PresentationAction,
    target: PresentationTarget,
    persisted: PresentationTarget,
) -> bool {
    match action {
        PresentationAction::ResetPosition => {
            target.reset_position_revision == persisted.reset_position_revision
        }
        PresentationAction::Set { patch } => {
            let touched = patch.visible.is_some()
                || patch.passthrough.is_some()
                || patch.alpha_passthrough.is_some()
                || patch.bubble_visible.is_some()
                || patch.bubble_placement.is_some()
                || patch.scale.is_some();
            if !touched {
                return target == persisted;
            }
            patch
                .visible
                .is_none_or(|_| target.visible == persisted.visible)
                && patch
                    .passthrough
                    .is_none_or(|_| target.passthrough == persisted.passthrough)
                && patch
                    .alpha_passthrough
                    .is_none_or(|_| target.alpha_passthrough == persisted.alpha_passthrough)
                && patch
                    .bubble_visible
                    .is_none_or(|_| target.bubble_visible == persisted.bubble_visible)
                && patch
                    .bubble_placement
                    .is_none_or(|_| target.bubble_placement == persisted.bubble_placement)
                && patch.scale.is_none_or(|_| target.scale == persisted.scale)
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct WindowFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingReason {
    NativeApply,
    Persistence,
    ImeComposition,
    Tracking,
    Gesture,
}

impl PendingReason {
    const COUNT: usize = 5;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PresentationSnapshot {
    pub instance_id: String,
    pub revision: u64,
    pub desired: PresentationTarget,
    /// None until the main thread has observed both native windows.
    pub effective: Option<PresentationTarget>,
    pub persisted: Option<PresentationTarget>,
    pub pet_window_visible: Option<bool>,
    pub bubble_window_visible: Option<bool>,
    pub pet_window_frame: Option<WindowFrame>,
    pub bubble_window_frame: Option<WindowFrame>,
    pub pending_reasons: Vec<PendingReason>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Accepted,
    Pending,
    Applied,
    Superseded,
    PersistFailed,
    Rejected,
    Shutdown,
    Unknown,
}

impl OperationState {
    fn terminal(self) -> bool {
        !matches!(self, Self::Accepted | Self::Pending)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PresentationOperation {
    pub instance_id: String,
    pub operation_id: String,
    pub state: OperationState,
    pub revision: Option<u64>,
    pub target: Option<PresentationTarget>,
    pub native_applied: bool,
    pub persisted: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct PresentationCheckpoint {
    pub revision: u64,
    pub effective: Option<PresentationTarget>,
    /// True only when the UI has actually finished native application, not merely
    /// when AppState or a window target changed. A deferred IME/drag is false.
    pub native_applied: bool,
    /// The exact target successfully written by Preferences::save, if any.
    pub persisted: Option<PresentationTarget>,
    /// Operation ID, exact attempted target, and error; unrelated saves are not
    /// attributable to this operation even at the same revision.
    pub save_error: Option<(String, PresentationTarget, String)>,
    pub pet_window_visible: Option<bool>,
    pub bubble_window_visible: Option<bool>,
    pub pet_window_frame: Option<WindowFrame>,
    pub bubble_window_frame: Option<WindowFrame>,
    pub pending_reasons: Vec<PendingReason>,
}

#[derive(Debug)]
struct Entry {
    request: PresentationRequest,
    operation: PresentationOperation,
}

#[derive(Debug)]
struct DomainEntry {
    fingerprint: [u8; 32],
    request: Option<DomainRequest>,
    operation: DomainOperation,
}

pub(crate) type SharedAutomation = Arc<Mutex<AutomationState>>;

#[derive(Debug)]
pub(crate) struct AutomationState {
    snapshot: PresentationSnapshot,
    queued: VecDeque<String>,
    entries: HashMap<String, Entry>,
    completed: VecDeque<String>,
    domain_queued: VecDeque<String>,
    domain_entries: HashMap<String, DomainEntry>,
    domain_completed: VecDeque<String>,
    native_evidence: Option<(u64, PresentationTarget)>,
    persisted_evidence: Option<(u64, PresentationTarget)>,
    stopped: bool,
}

pub(crate) fn lock_automation(handle: &SharedAutomation) -> MutexGuard<'_, AutomationState> {
    handle
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// OS entropy is required; startup must fail rather than share an invented identity.
pub(crate) fn new_automation(
    initial: PresentationTarget,
    persisted: Option<PresentationTarget>,
) -> io::Result<SharedAutomation> {
    let mut bytes = [0u8; 16];
    // getentropy is an OS-backed, fallible API on macOS; 16 bytes are below its limit.
    if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut instance_id = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        instance_id.push(char::from(HEX[usize::from(byte >> 4)]));
        instance_id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(Arc::new(Mutex::new(AutomationState::with_instance(
        instance_id,
        initial,
        persisted,
    ))))
}

#[cfg(test)]
pub(crate) fn test_automation(initial: PresentationTarget) -> SharedAutomation {
    Arc::new(Mutex::new(AutomationState::with_instance(
        "test".to_owned(),
        initial,
        None,
    )))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn bounded_error(error: String) -> String {
    if error.len() <= MAX_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_ERROR_BYTES;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error[..end].to_owned()
}

impl AutomationState {
    fn with_instance(
        instance_id: String,
        initial: PresentationTarget,
        persisted: Option<PresentationTarget>,
    ) -> Self {
        let persisted_evidence = persisted.map(|target| (0, target));
        Self {
            snapshot: PresentationSnapshot {
                instance_id,
                revision: 0,
                desired: initial,
                effective: None,
                persisted,
                pet_window_visible: None,
                bubble_window_visible: None,
                pet_window_frame: None,
                bubble_window_frame: None,
                pending_reasons: Vec::new(),
            },
            queued: VecDeque::new(),
            entries: HashMap::new(),
            completed: VecDeque::new(),
            domain_queued: VecDeque::new(),
            domain_entries: HashMap::new(),
            domain_completed: VecDeque::new(),
            native_evidence: None,
            persisted_evidence,
            stopped: false,
        }
    }

    pub(crate) fn instance(&self) -> &str {
        &self.snapshot.instance_id
    }
    pub(crate) fn revision(&self) -> u64 {
        self.snapshot.revision
    }
    pub(crate) fn desired(&self) -> PresentationTarget {
        self.snapshot.desired
    }
    pub(crate) fn snapshot(&self) -> PresentationSnapshot {
        self.snapshot.clone()
    }

    /// A different daemon instance must never accept a stale client's mutation.
    /// Identical IDs are idempotent only while retained, and only for the entire request.
    pub(crate) fn submit(
        &mut self,
        request: PresentationRequest,
    ) -> Result<PresentationOperation, String> {
        if request.instance_id != self.instance() {
            return Err("daemon instance changed".into());
        }
        if !valid_id(&request.operation_id) {
            return Err("invalid operation ID".into());
        }
        if self.domain_entries.contains_key(&request.operation_id) {
            return Err("operation ID used for a different request".into());
        }
        if let Some(entry) = self.entries.get(&request.operation_id) {
            return if entry.request == request {
                Ok(entry.operation.clone())
            } else {
                Err("operation ID used for a different request".into())
            };
        }
        if self.stopped {
            return Err("automation is shut down".into());
        }
        if request
            .expected_revision
            .is_some_and(|revision| revision != self.snapshot.revision)
        {
            return Err("presentation revision changed".into());
        }
        // Reject bad input before reserving an ID or consuming a mailbox slot.
        self.snapshot.desired.applying(&request.action)?;
        if self.queued.len() >= MAX_QUEUED {
            return Err("presentation inbox is busy".into());
        }
        if self.entries.len() >= MAX_OPERATIONS {
            if let Some(id) = self.completed.pop_front() {
                self.entries.remove(&id);
            }
        }
        if self.entries.len() >= MAX_OPERATIONS {
            return Err("presentation operation history is busy".into());
        }
        let operation = PresentationOperation {
            instance_id: request.instance_id.clone(),
            operation_id: request.operation_id.clone(),
            state: OperationState::Accepted,
            revision: None,
            target: None,
            native_applied: false,
            persisted: false,
            error: None,
        };
        self.queued.push_back(request.operation_id.clone());
        self.entries.insert(
            request.operation_id.clone(),
            Entry {
                request,
                operation: operation.clone(),
            },
        );
        Ok(operation)
    }

    /// UI thread drains without holding the mailbox mutex while touching native state.
    pub(crate) fn drain_queued(&mut self) -> Vec<PresentationRequest> {
        self.queued
            .drain(..)
            .filter_map(|id| self.entries.get(&id).map(|entry| entry.request.clone()))
            .collect()
    }

    /// Call after reducing the action into AppState, passing the resulting full scene target.
    /// Recheck expected_revision immediately before changing AppState; reject_request if stale.
    pub(crate) fn start_request(
        &mut self,
        id: &str,
        target: PresentationTarget,
    ) -> Result<PresentationOperation, String> {
        let entry = self.entries.get(id).ok_or("unknown operation")?;
        if entry.operation.state != OperationState::Accepted {
            return Err("operation is not accepted".into());
        }
        if self.stopped {
            return Err("automation is shut down".into());
        }
        if entry
            .request
            .expected_revision
            .is_some_and(|revision| revision != self.snapshot.revision)
        {
            return Err("presentation revision changed".into());
        }
        if self.snapshot.desired.applying(&entry.request.action)? != target {
            return Err("reduced presentation target does not match request".into());
        }
        self.change_desired(target);
        let revision = self.snapshot.revision;
        let entry = self.entries.get_mut(id).expect("entry checked above");
        entry.operation.state = OperationState::Pending;
        entry.operation.revision = Some(revision);
        entry.operation.target = Some(target);
        self.finish_operation(id)
    }

    pub(crate) fn reject_request(
        &mut self,
        id: &str,
        error: String,
    ) -> Result<PresentationOperation, String> {
        let entry = self.entries.get_mut(id).ok_or("unknown operation")?;
        if entry.operation.state != OperationState::Accepted {
            return Err("operation is not accepted".into());
        }
        entry.operation.state = OperationState::Rejected;
        entry.operation.error = Some(bounded_error(error));
        self.queued.retain(|queued_id| queued_id != id);
        let result = self.entries[id].operation.clone();
        self.record_completed(id);
        Ok(result)
    }

    /// For ordinary UI changes, supply the whole new scene target; conservative
    /// full-target comparison supersedes incompatible outstanding operations.
    pub(crate) fn ui_change(&mut self, target: PresentationTarget) {
        if !self.stopped {
            self.change_desired(target);
        }
    }

    fn change_desired(&mut self, target: PresentationTarget) {
        if target == self.snapshot.desired {
            return;
        }
        self.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .expect("presentation revision exhausted");
        self.snapshot.desired = target;
        self.native_evidence = None;
        self.persisted_evidence = self
            .snapshot
            .persisted
            .map(|persisted| (self.snapshot.revision, persisted));
        let superseded: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.operation.state == OperationState::Pending
                    && entry.operation.target != Some(target)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in superseded {
            self.entries.get_mut(&id).unwrap().operation.state = OperationState::Superseded;
            self.record_completed(&id);
        }
    }

    /// UI publishes only observed window values. Revision and full target MUST
    /// match for per-operation evidence; high-watermark alone is insufficient.
    pub(crate) fn publish_checkpoint(&mut self, checkpoint: PresentationCheckpoint) {
        if self.stopped {
            return;
        }
        if let Some(persisted) = checkpoint.persisted {
            self.snapshot.persisted = Some(persisted);
            self.persisted_evidence = Some((self.snapshot.revision, persisted));
        }
        if checkpoint.revision != self.snapshot.revision {
            return;
        }
        self.snapshot.effective = checkpoint.effective;
        self.snapshot.pet_window_visible = checkpoint.pet_window_visible;
        self.snapshot.bubble_window_visible = checkpoint.bubble_window_visible;
        self.snapshot.pet_window_frame = checkpoint.pet_window_frame;
        self.snapshot.bubble_window_frame = checkpoint.bubble_window_frame;
        self.snapshot.pending_reasons = checkpoint.pending_reasons;
        self.snapshot.pending_reasons.truncate(PendingReason::COUNT);
        let target = self.snapshot.desired;
        if checkpoint.effective != Some(target) {
            self.native_evidence = None;
        }
        if checkpoint.native_applied && checkpoint.effective == Some(target) {
            self.native_evidence = Some((checkpoint.revision, target));
        }
        let matching: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.operation.state == OperationState::Pending
                    && entry.operation.revision == Some(checkpoint.revision)
                    && entry.operation.target == Some(target)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let failed_batch = checkpoint
            .save_error
            .as_ref()
            .is_some_and(|(failed_id, _, _)| matching.iter().any(|id| id == failed_id));
        for id in matching {
            if let Some((failed_id, _attempted, error)) = &checkpoint.save_error {
                // A single failed write can serve several accepted, coalesced
                // requests. Do not leave the unsaved siblings pending forever.
                if failed_batch && (failed_id == &id || !self.entries[&id].operation.persisted) {
                    let entry = self.entries.get_mut(&id).unwrap();
                    entry.operation.native_applied =
                        self.native_evidence == Some((checkpoint.revision, target));
                    entry.operation.state = OperationState::PersistFailed;
                    entry.operation.error = Some(bounded_error(error.clone()));
                    self.record_completed(&id);
                    continue;
                }
            }
            let _ = self.finish_operation(&id);
        }
    }

    /// Reconciles a pending operation only with exact native and save evidence.
    pub(crate) fn finish_operation(&mut self, id: &str) -> Result<PresentationOperation, String> {
        let entry = self.entries.get_mut(id).ok_or("unknown operation")?;
        if entry.operation.state == OperationState::Pending {
            let key = entry.operation.revision.zip(entry.operation.target);
            entry.operation.native_applied = key == self.native_evidence;
            entry.operation.persisted = match (
                entry.operation.revision,
                entry.operation.target,
                self.persisted_evidence,
            ) {
                (Some(revision), Some(target), Some((saved_revision, saved)))
                    if revision == saved_revision =>
                {
                    persisted_request_fields(&entry.request.action, target, saved)
                }
                _ => false,
            };
            if entry.operation.native_applied && entry.operation.persisted {
                entry.operation.state = OperationState::Applied;
                self.record_completed(id);
            }
        }
        Ok(self.entries[id].operation.clone())
    }

    pub(crate) fn status(
        &self,
        instance_id: &str,
        id: &str,
    ) -> Result<PresentationOperation, String> {
        if instance_id != self.instance() {
            return Err("daemon instance changed".into());
        }
        if !valid_id(id) {
            return Err("invalid operation ID".into());
        }
        Ok(self
            .entries
            .get(id)
            .map(|entry| entry.operation.clone())
            .unwrap_or_else(|| PresentationOperation {
                instance_id: self.snapshot.instance_id.clone(),
                operation_id: id.to_owned(),
                state: OperationState::Unknown,
                revision: None,
                target: None,
                native_applied: false,
                persisted: false,
                error: None,
            }))
    }

    /// A domain operation owns its request until the UI takes it. Only a
    /// digest and the bounded outcome remain in the ledger after draining.
    pub(crate) fn submit_domain(
        &mut self,
        request: DomainRequest,
    ) -> Result<DomainOperation, String> {
        if request.instance_id != self.instance() {
            return Err("daemon instance changed".into());
        }
        if !valid_id(&request.operation_id) {
            return Err("invalid operation ID".into());
        }
        validate_domain_action(&request.action, &request.instance_id)?;
        let fingerprint = domain_fingerprint(&request)?;
        if self.entries.contains_key(&request.operation_id) {
            return Err("operation ID used for a different request".into());
        }
        if let Some(entry) = self.domain_entries.get(&request.operation_id) {
            return if entry.fingerprint == fingerprint {
                Ok(entry.operation.clone())
            } else {
                Err("operation ID used for a different request".into())
            };
        }
        if self.stopped {
            return Err("automation is shut down".into());
        }
        if self.domain_queued.len() >= MAX_QUEUED {
            return Err("domain inbox is busy".into());
        }
        if self.domain_entries.len() >= MAX_OPERATIONS {
            if let Some(id) = self.domain_completed.pop_front() {
                self.domain_entries.remove(&id);
            }
        }
        if self.domain_entries.len() >= MAX_OPERATIONS {
            return Err("domain operation history is busy".into());
        }
        let operation = DomainOperation {
            instance_id: request.instance_id.clone(),
            operation_id: request.operation_id.clone(),
            kind: request.action.kind().to_owned(),
            state: DomainOperationState::Accepted,
            committed: false,
            native_applied: false,
            result: None,
            error_code: None,
            error: None,
        };
        self.domain_queued.push_back(request.operation_id.clone());
        self.domain_entries.insert(
            request.operation_id.clone(),
            DomainEntry {
                fingerprint,
                request: Some(request),
                operation: operation.clone(),
            },
        );
        Ok(operation)
    }

    pub(crate) fn domain_status(
        &self,
        instance_id: &str,
        operation_id: &str,
    ) -> Result<DomainOperation, String> {
        if instance_id != self.instance() {
            return Err("daemon instance changed".into());
        }
        if !valid_id(operation_id) {
            return Err("invalid operation ID".into());
        }
        self.domain_entries
            .get(operation_id)
            .map(|entry| entry.operation.clone())
            .ok_or_else(|| "unknown domain operation".into())
    }

    pub(crate) fn drain_domain_requests(&mut self) -> Vec<DomainRequest> {
        self.domain_queued
            .drain(..)
            .filter_map(|id| self.domain_entries.get_mut(&id)?.request.take())
            .collect()
    }

    pub(crate) fn start_domain_request(
        &mut self,
        operation_id: &str,
    ) -> Result<DomainOperation, String> {
        if self.stopped {
            return Err("automation is shut down".into());
        }
        let entry = self
            .domain_entries
            .get_mut(operation_id)
            .ok_or("unknown domain operation")?;
        if entry.operation.state != DomainOperationState::Accepted || entry.request.is_some() {
            return Err("domain operation has not been drained or is already started".into());
        }
        entry.operation.state = DomainOperationState::Pending;
        Ok(entry.operation.clone())
    }

    pub(crate) fn finish_domain_operation(
        &mut self,
        operation_id: &str,
        state: DomainOperationState,
        committed: bool,
        native_applied: bool,
        result: Option<Value>,
        error_code: Option<String>,
        error: Option<String>,
    ) -> Result<DomainOperation, String> {
        let entry = self
            .domain_entries
            .get(operation_id)
            .ok_or("unknown domain operation")?;
        if entry.operation.state != DomainOperationState::Pending {
            return Err("domain operation is not pending".into());
        }
        if state == DomainOperationState::Accepted {
            return Err("invalid domain operation state".into());
        }
        let kind = entry.operation.kind.as_str();
        if entry.operation.committed && !committed {
            return Err("committed domain operation cannot lose its save or ACK".into());
        }
        if matches!(
            kind,
            "preferences_get" | "dialogue_list" | "dialogue_read" | "worktree_inspect"
        ) && (committed || native_applied)
        {
            return Err("read-only operation cannot commit or apply native state".into());
        }
        if native_applied && !committed {
            return Err("native application requires a committed operation".into());
        }
        if native_applied
            && !matches!(
                kind,
                "preferences_set"
                    | "dialogue_set"
                    | "dialogue_reset_entry"
                    | "dialogue_reset_character"
            )
        {
            return Err("only native preference or dialogue changes can be applied".into());
        }
        if state == DomainOperationState::AgentPrompted
            && (kind != "session_prompt" || !committed || native_applied)
        {
            return Err("only acknowledged prompts can be agent_prompted".into());
        }
        if state == DomainOperationState::UnknownDelivery && committed {
            return Err("acknowledged delivery cannot become unknown".into());
        }
        if state == DomainOperationState::Applied {
            let invalid = match kind {
                "preferences_get" | "dialogue_list" | "dialogue_read" | "worktree_inspect" => {
                    committed || native_applied
                }
                "preferences_set" => !committed || !native_applied,
                "dialogue_set" | "dialogue_reset_entry" | "dialogue_reset_character" => !committed,
                "worktree_remove" => !committed || native_applied,
                _ => true,
            };
            if invalid {
                return Err("applied requires completed operation evidence".into());
            }
        }
        if result.is_some() {
            let mut writer = RequestHasher {
                hash: Sha256::new(),
                bytes: 0,
                limit: MAX_DOMAIN_RESULT_BYTES,
            };
            serde_json::to_writer(&mut writer, &result)
                .map_err(|_| "domain result exceeds limit or cannot be encoded".to_owned())?;
        }
        if error_code.as_deref().is_some_and(|code| !valid_id(code)) {
            return Err("invalid domain error code".into());
        }
        let entry = self
            .domain_entries
            .get_mut(operation_id)
            .expect("entry checked above");
        entry.operation.state = state;
        entry.operation.committed = committed;
        entry.operation.native_applied = native_applied;
        entry.operation.result = result;
        entry.operation.error_code = error_code;
        entry.operation.error = error.map(bounded_error);
        let operation = entry.operation.clone();
        if state.terminal() {
            self.domain_completed.push_back(operation_id.to_owned());
            while self.domain_completed.len() > MAX_COMPLETED {
                if let Some(id) = self.domain_completed.pop_front() {
                    self.domain_entries.remove(&id);
                }
            }
        }
        Ok(operation)
    }

    pub(crate) fn shutdown(&mut self) {
        self.stopped = true;
        self.domain_queued.clear();
        let outstanding_domain: Vec<_> = self
            .domain_entries
            .iter()
            .filter(|(_, entry)| !entry.operation.state.terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in outstanding_domain {
            let entry = self
                .domain_entries
                .get_mut(&id)
                .expect("entry checked above");
            entry.operation.state = if entry.operation.state == DomainOperationState::Pending
                && !entry.operation.committed
                && !matches!(
                    entry.operation.kind.as_str(),
                    "preferences_get" | "dialogue_list" | "dialogue_read" | "worktree_inspect"
                ) {
                DomainOperationState::UnknownDelivery
            } else {
                DomainOperationState::Shutdown
            };
            if entry.operation.state == DomainOperationState::Shutdown
                && entry.operation.kind == "worktree_remove"
                && entry.operation.committed
            {
                entry.operation.error_code = Some("observation_interrupted".into());
                entry.operation.error = Some(
                    "worktree removal was acknowledged, but watcher observation was interrupted"
                        .into(),
                );
            }
            entry.request = None;
            self.domain_completed.push_back(id);
        }
        self.queued.clear();
        let outstanding: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.operation.state.terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in outstanding {
            self.entries.get_mut(&id).unwrap().operation.state = OperationState::Shutdown;
            self.record_completed(&id);
        }
    }

    fn record_completed(&mut self, id: &str) {
        self.completed.push_back(id.to_owned());
        self.trim_completed();
    }

    fn trim_completed(&mut self) {
        while self.completed.len() > MAX_COMPLETED {
            let Some(id) = self.completed.pop_front() else {
                break;
            };
            self.entries.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PresentationTarget {
        PresentationTarget {
            visible: true,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            bubble_placement: BubblePlacement::Above,
            scale: 0.65,
            reset_position_revision: 0,
        }
    }
    fn request(id: &str, action: PresentationAction) -> PresentationRequest {
        PresentationRequest {
            instance_id: "test".into(),
            operation_id: id.into(),
            expected_revision: None,
            action,
        }
    }
    fn set_visible(value: bool) -> PresentationAction {
        PresentationAction::Set {
            patch: PresentationPatch {
                visible: Some(value),
                ..Default::default()
            },
        }
    }
    fn checkpoint(
        revision: u64,
        observed: PresentationTarget,
        saved: Option<PresentationTarget>,
    ) -> PresentationCheckpoint {
        PresentationCheckpoint {
            revision,
            effective: Some(observed),
            native_applied: true,
            persisted: saved,
            save_error: None,
            pet_window_visible: Some(observed.visible),
            bubble_window_visible: Some(observed.bubble_visible),
            pet_window_frame: None,
            bubble_window_frame: None,
            pending_reasons: vec![],
        }
    }

    #[test]
    fn hide_then_show_never_applies_hide_from_show_checkpoint() {
        let mut state = AutomationState::with_instance("test".into(), target(), Some(target()));
        state.submit(request("a", set_visible(false))).unwrap();
        let hidden = target().applying(&set_visible(false)).unwrap();
        state.start_request("a", hidden).unwrap();
        state.submit(request("b", set_visible(true))).unwrap();
        state.start_request("b", target()).unwrap();
        let revision = state.snapshot().revision;
        state.publish_checkpoint(checkpoint(revision, target(), Some(target())));
        assert_eq!(
            state.status("test", "a").unwrap().state,
            OperationState::Superseded
        );
        assert_eq!(
            state.status("test", "b").unwrap().state,
            OperationState::Applied
        );
    }

    #[test]
    fn mismatched_instance_and_conflicting_ids_cannot_mutate() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        let mut wrong = request("a", set_visible(false));
        wrong.instance_id = "previous".into();
        assert!(state.submit(wrong).is_err());
        let accepted = state.submit(request("a", set_visible(false))).unwrap();
        assert_eq!(
            state
                .submit(request("a", set_visible(false)))
                .unwrap()
                .state,
            accepted.state
        );
        assert!(state.submit(request("a", set_visible(true))).is_err());
        assert!(state.status("previous", "a").is_err());
        assert_eq!(state.snapshot().revision, 0);
    }

    #[test]
    fn pending_records_are_never_evicted_and_full_inbox_is_busy() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        for index in 0..MAX_QUEUED {
            state
                .submit(request(&format!("op{index}"), set_visible(false)))
                .unwrap();
        }
        assert!(state
            .submit(request("overflow", set_visible(false)))
            .is_err());
        let drained = state.drain_queued();
        assert_eq!(drained.len(), MAX_QUEUED);
        let hidden = target().applying(&set_visible(false)).unwrap();
        for item in drained {
            state.start_request(&item.operation_id, hidden).unwrap();
        }
        for index in 0..MAX_COMPLETED {
            state
                .submit(request(&format!("rejected{index}"), set_visible(true)))
                .unwrap();
            state.drain_queued();
            state
                .reject_request(&format!("rejected{index}"), "stale".into())
                .unwrap();
        }
        assert_eq!(
            state.status("test", "op0").unwrap().state,
            OperationState::Pending
        );
        state
            .submit(request("another", set_visible(false)))
            .unwrap();
        assert_eq!(
            state.status("test", "op0").unwrap().state,
            OperationState::Pending
        );
    }

    #[test]
    fn entirely_pending_history_refuses_admission() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        let hidden = target().applying(&set_visible(false)).unwrap();
        for batch in 0..(MAX_OPERATIONS / MAX_QUEUED) {
            for index in 0..MAX_QUEUED {
                state
                    .submit(request(
                        &format!("pending{batch}_{index}"),
                        set_visible(false),
                    ))
                    .unwrap();
            }
            for item in state.drain_queued() {
                state.start_request(&item.operation_id, hidden).unwrap();
            }
        }
        assert!(state.submit(request("busy", set_visible(false))).is_err());
        assert_eq!(
            state.status("test", "pending0_0").unwrap().state,
            OperationState::Pending
        );
    }

    #[test]
    fn unrelated_save_error_and_stale_native_revision_provide_no_evidence() {
        let mut state = AutomationState::with_instance("test".into(), target(), Some(target()));
        let hidden = target().applying(&set_visible(false)).unwrap();
        state.submit(request("hide", set_visible(false))).unwrap();
        state.start_request("hide", hidden).unwrap();
        let mut unrelated = checkpoint(1, hidden, None);
        unrelated.save_error = Some(("other".into(), target(), "other save failed".into()));
        state.publish_checkpoint(unrelated);
        assert_eq!(
            state.status("test", "hide").unwrap().state,
            OperationState::Pending
        );
        state.publish_checkpoint(checkpoint(0, hidden, Some(hidden)));
        assert!(!state.status("test", "hide").unwrap().persisted);
        assert_eq!(
            state.status("test", "hide").unwrap().state,
            OperationState::Pending
        );
    }

    #[test]
    fn strict_patch_rejects_unknown_fields_and_invalid_placement() {
        assert!(
            serde_json::from_str::<PresentationPatch>(r#"{"visible":true,"visble":false}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<PresentationPatch>(r#"{"bubble_placement":"center"}"#).is_err()
        );
        assert!(serde_json::from_str::<PresentationPatch>(r#"{"scale":"0.8"}"#).is_err());
    }

    #[test]
    fn save_failure_does_not_claim_persistence_or_erase_native_evidence() {
        let mut state = AutomationState::with_instance("test".into(), target(), Some(target()));
        state.submit(request("hide", set_visible(false))).unwrap();
        let hidden = target().applying(&set_visible(false)).unwrap();
        state.start_request("hide", hidden).unwrap();
        let mut observed = checkpoint(1, hidden, None);
        observed.save_error = Some(("hide".into(), hidden, "preferences cannot be saved".into()));
        state.publish_checkpoint(observed);
        let op = state.status("test", "hide").unwrap();
        assert_eq!(op.state, OperationState::PersistFailed);
        assert!(op.native_applied);
        assert!(!op.persisted);
        assert_eq!(state.snapshot().persisted, Some(target()));
    }

    #[test]
    fn partial_candidate_persists_only_the_new_requests_fields() {
        let mut state = AutomationState::with_instance("test".into(), target(), Some(target()));
        let hidden = target().applying(&set_visible(false)).unwrap();
        state.submit(request("hide", set_visible(false))).unwrap();
        state.start_request("hide", hidden).unwrap();
        let mut failed = checkpoint(1, hidden, None);
        failed.save_error = Some(("hide".into(), hidden, "blocked preferences path".into()));
        state.publish_checkpoint(failed);

        let bubble = PresentationAction::Set {
            patch: PresentationPatch {
                bubble_visible: Some(false),
                ..Default::default()
            },
        };
        let desired = hidden.applying(&bubble).unwrap();
        state.submit(request("bubble", bubble)).unwrap();
        state.start_request("bubble", desired).unwrap();
        let persisted = target()
            .applying(&PresentationAction::Set {
                patch: PresentationPatch {
                    bubble_visible: Some(false),
                    ..Default::default()
                },
            })
            .unwrap();
        state.publish_checkpoint(checkpoint(2, desired, Some(persisted)));
        assert_eq!(
            state.status("test", "hide").unwrap().state,
            OperationState::PersistFailed
        );
        let operation = state.status("test", "bubble").unwrap();
        assert_eq!(operation.state, OperationState::Applied);
        assert!(operation.native_applied && operation.persisted);
        assert_eq!(state.snapshot().persisted, Some(persisted));
        assert_ne!(state.snapshot().persisted, Some(desired));
    }

    #[test]
    fn failed_coalesced_save_finishes_only_unsaved_siblings() {
        let mut initial_persisted = target();
        initial_persisted.bubble_visible = false;
        let mut state =
            AutomationState::with_instance("test".into(), target(), Some(initial_persisted));
        let set_bubble = PresentationAction::Set {
            patch: PresentationPatch {
                bubble_visible: Some(true),
                ..Default::default()
            },
        };
        state.submit(request("visible", set_visible(true))).unwrap();
        state.start_request("visible", target()).unwrap();
        assert!(state.status("test", "visible").unwrap().persisted);
        state.submit(request("bubble", set_bubble)).unwrap();
        state.start_request("bubble", target()).unwrap();
        assert!(!state.status("test", "bubble").unwrap().persisted);
        let mut deferred = checkpoint(0, target(), None);
        deferred.native_applied = false;
        deferred.save_error = Some(("bubble".into(), target(), "save failed".into()));
        state.publish_checkpoint(deferred);
        assert_eq!(
            state.status("test", "visible").unwrap().state,
            OperationState::Pending
        );
        assert_eq!(
            state.status("test", "bubble").unwrap().state,
            OperationState::PersistFailed
        );

        let mut other = AutomationState::with_instance("test".into(), target(), None);
        other.submit(request("a", set_visible(true))).unwrap();
        other.start_request("a", target()).unwrap();
        other.submit(request("b", set_visible(true))).unwrap();
        other.start_request("b", target()).unwrap();
        let mut failed = checkpoint(0, target(), None);
        failed.save_error = Some(("b".into(), target(), "save failed".into()));
        other.publish_checkpoint(failed);
        assert_eq!(
            other.status("test", "a").unwrap().state,
            OperationState::PersistFailed
        );
        assert_eq!(
            other.status("test", "b").unwrap().state,
            OperationState::PersistFailed
        );
    }

    #[test]
    fn noop_requires_real_checkpoint_not_just_target_or_revision() {
        let mut state = AutomationState::with_instance("test".into(), target(), Some(target()));
        state.submit(request("noop", set_visible(true))).unwrap();
        assert_eq!(
            state.start_request("noop", target()).unwrap().state,
            OperationState::Pending
        );
        let mut deferred = checkpoint(0, target(), Some(target()));
        deferred.native_applied = false;
        state.publish_checkpoint(deferred);
        assert_eq!(
            state.status("test", "noop").unwrap().state,
            OperationState::Pending
        );
        state.publish_checkpoint(checkpoint(0, target(), None));
        assert_eq!(
            state.status("test", "noop").unwrap().state,
            OperationState::Applied
        );
    }
    fn domain_request(id: &str, action: DomainAction) -> DomainRequest {
        DomainRequest {
            instance_id: "test".into(),
            operation_id: id.into(),
            action,
        }
    }
    fn dialogue_selection() -> DialogueSelection {
        DialogueSelection {
            identity: DialogueIdentity {
                target: DialogueTarget::Character("default".into()),
                reference: Some(crate::character_types::CharacterRef::builtin()),
                generation: 1,
            },
            locale: crate::dialogue_automation::DialogueLanguage::Ko,
            slot: crate::dialogue::DialogueSlot::Idle,
        }
    }

    fn dialogue_baseline() -> DialogueBaseline {
        DialogueBaseline {
            metadata_token: "a".repeat(64),
            override_entry: None,
            target_overrides_token: "b".repeat(64),
        }
    }

    fn prompt(id: &str, text: String) -> DomainRequest {
        domain_request(
            id,
            DomainAction::SessionPrompt {
                key: SessionIdentity {
                    instance_id: "test".into(),
                    source_id: 0,
                    generation: 2,
                    terminal_id: "terminal:one".into(),
                },
                text,
            },
        )
    }

    #[test]
    fn domain_drain_moves_prompt_body_and_replay_retains_only_fingerprint() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        let text = "prompt\n".repeat(12_000);
        let address = text.as_ptr();
        let accepted = state.submit_domain(prompt("prompt-1", text)).unwrap();
        assert_eq!(accepted.state, DomainOperationState::Accepted);
        let mut requests = state.drain_domain_requests();
        assert_eq!(requests.len(), 1);
        let drained = requests.pop().unwrap();
        let DomainAction::SessionPrompt { text, .. } = &drained.action else {
            panic!("expected prompt");
        };
        assert_eq!(text.as_ptr(), address);
        assert!(state.domain_entries["prompt-1"].request.is_none());
        assert_eq!(
            state.submit_domain(drained).unwrap().state,
            DomainOperationState::Accepted
        );
        assert_eq!(
            state.start_domain_request("prompt-1").unwrap().state,
            DomainOperationState::Pending
        );
        let ack = state
            .finish_domain_operation(
                "prompt-1",
                DomainOperationState::AgentPrompted,
                true,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(ack.state.terminal());
        assert!(state.drain_domain_requests().is_empty());
        assert!(state
            .submit_domain(prompt("prompt-1", "different".into()))
            .is_err());
    }

    #[test]
    fn domain_rejects_other_instance_invalid_ids_and_cross_mailbox_collisions() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        let mut foreign = domain_request("foreign", DomainAction::PreferencesGet {});
        foreign.instance_id = "other".into();
        assert!(state.submit_domain(foreign).is_err());
        let mut wrong_key = prompt("wrong-key", "hi".into());
        if let DomainAction::SessionPrompt { key, .. } = &mut wrong_key.action {
            key.instance_id = "other".into();
        }
        assert!(state.submit_domain(wrong_key).is_err());
        assert!(state.submit_domain(prompt("bad id", "hi".into())).is_err());
        assert!(state
            .submit_domain(prompt("too-large", "x".repeat(MAX_DOMAIN_REQUEST_BYTES)))
            .is_err());
        assert!(state
            .submit_domain(domain_request(
                "settings-large",
                DomainAction::PreferencesSet {
                    patch: PreferencePatch {
                        observation_machines: Some(
                            vec!["x".repeat(MAX_DOMAIN_OTHER_REQUEST_BYTES)]
                        ),
                        ..Default::default()
                    },
                    expected_revision: None,
                },
            ))
            .is_err());
        state
            .submit(request("presentation-id", set_visible(false)))
            .unwrap();
        assert!(state
            .submit_domain(domain_request(
                "presentation-id",
                DomainAction::PreferencesGet {}
            ))
            .is_err());
        state
            .submit_domain(domain_request("domain-id", DomainAction::PreferencesGet {}))
            .unwrap();
        assert!(state
            .submit(request("domain-id", set_visible(false)))
            .is_err());
        assert!(state.domain_status("other", "domain-id").is_err());
        assert!(state.domain_status("test", "bad id").is_err());
        assert!(state.domain_status("test", "missing").is_err());
    }

    #[test]
    fn domain_settings_and_reads_have_independent_evidence() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        state
            .submit_domain(domain_request("get", DomainAction::PreferencesGet {}))
            .unwrap();
        state
            .submit_domain(domain_request(
                "set",
                DomainAction::PreferencesSet {
                    patch: PreferencePatch {
                        show_status_indicators: Some(false),
                        ..Default::default()
                    },
                    expected_revision: Some(9),
                },
            ))
            .unwrap();
        assert_eq!(state.drain_domain_requests().len(), 2);
        state.start_domain_request("get").unwrap();
        state.start_domain_request("set").unwrap();
        assert!(state
            .finish_domain_operation(
                "set",
                DomainOperationState::Applied,
                true,
                false,
                None,
                None,
                None,
            )
            .is_err());
        let pending = state
            .finish_domain_operation(
                "set",
                DomainOperationState::Pending,
                true,
                false,
                Some(serde_json::json!({"revision": 10, "pending_reasons": ["ime"]})),
                None,
                None,
            )
            .unwrap();
        assert!(pending.committed);
        assert!(!pending.state.terminal());
        assert_eq!(state.revision(), 0);
        assert_eq!(
            state
                .finish_domain_operation(
                    "get",
                    DomainOperationState::Applied,
                    false,
                    false,
                    Some(serde_json::json!({"revision": 10})),
                    None,
                    None,
                )
                .unwrap()
                .state,
            DomainOperationState::Applied
        );
        assert_eq!(
            state
                .finish_domain_operation(
                    "set",
                    DomainOperationState::Applied,
                    true,
                    true,
                    Some(serde_json::json!({"revision": 10})),
                    None,
                    None,
                )
                .unwrap()
                .state,
            DomainOperationState::Applied
        );
        assert!(state
            .finish_domain_operation(
                "set",
                DomainOperationState::Failed,
                false,
                false,
                None,
                None,
                None
            )
            .is_err());
    }

    #[test]
    fn domain_capacity_and_result_size_are_explicitly_bounded() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        for index in 0..MAX_QUEUED {
            state
                .submit_domain(domain_request(
                    &format!("q-{index}"),
                    DomainAction::PreferencesGet {},
                ))
                .unwrap();
        }
        assert!(state
            .submit_domain(domain_request("overflow", DomainAction::PreferencesGet {}))
            .unwrap_err()
            .contains("inbox"));
        assert_eq!(state.drain_domain_requests().len(), MAX_QUEUED);
        for index in 0..MAX_OPERATIONS - MAX_QUEUED {
            let id = format!("h-{index}");
            state
                .submit_domain(domain_request(&id, DomainAction::PreferencesGet {}))
                .unwrap();
            state.drain_domain_requests();
        }
        assert!(state
            .submit_domain(domain_request(
                "history-overflow",
                DomainAction::PreferencesGet {}
            ))
            .unwrap_err()
            .contains("history"));
        state.start_domain_request("q-0").unwrap();
        assert!(state
            .finish_domain_operation(
                "q-0",
                DomainOperationState::Applied,
                false,
                false,
                Some(Value::String("x".repeat(MAX_DOMAIN_RESULT_BYTES))),
                None,
                None,
            )
            .is_err());
        assert_eq!(
            state.domain_status("test", "q-0").unwrap().state,
            DomainOperationState::Pending
        );
        state
            .finish_domain_operation(
                "q-0",
                DomainOperationState::Failed,
                false,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        state
            .submit_domain(domain_request(
                "history-available",
                DomainAction::PreferencesGet {},
            ))
            .unwrap();
    }

    #[test]
    fn shutdown_preserves_terminal_results_and_never_acknowledges_uncertain_prompt() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        state
            .submit_domain(prompt("pending", "hello".into()))
            .unwrap();
        state
            .submit_domain(prompt("unstarted", "hello".into()))
            .unwrap();
        state
            .submit_domain(prompt("finished", "hello".into()))
            .unwrap();
        state.drain_domain_requests();
        state.start_domain_request("pending").unwrap();
        state.start_domain_request("finished").unwrap();
        state
            .finish_domain_operation(
                "finished",
                DomainOperationState::AgentPrompted,
                true,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        state.shutdown();
        assert_eq!(
            state.domain_status("test", "pending").unwrap().state,
            DomainOperationState::UnknownDelivery
        );
        assert_eq!(
            state.domain_status("test", "unstarted").unwrap().state,
            DomainOperationState::Shutdown
        );
        assert_eq!(
            state.domain_status("test", "finished").unwrap().state,
            DomainOperationState::AgentPrompted
        );
        assert!(state
            .finish_domain_operation(
                "pending",
                DomainOperationState::AgentPrompted,
                true,
                false,
                None,
                None,
                None,
            )
            .is_err());
        assert!(state
            .submit_domain(prompt("later", "hello".into()))
            .is_err());
        assert!(state.drain_domain_requests().is_empty());
    }

    #[test]
    fn shutdown_keeps_all_bounded_inflight_outcomes_queryable() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        for index in 0..MAX_OPERATIONS {
            state
                .submit_domain(domain_request(
                    &format!("queued-{index}"),
                    DomainAction::PreferencesGet {},
                ))
                .unwrap();
            if (index + 1) % MAX_QUEUED == 0 {
                state.drain_domain_requests();
            }
        }
        state.shutdown();
        for index in 0..MAX_OPERATIONS {
            assert_eq!(
                state
                    .domain_status("test", &format!("queued-{index}"))
                    .unwrap()
                    .state,
                DomainOperationState::Shutdown
            );
        }
    }

    #[test]
    fn dialogue_admission_preserves_utf8_boundary_and_rejects_malformed_identity_and_baseline() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        let text = format!(" \n{}  \t", "가".repeat(681));
        assert_eq!(text.len(), 2048);
        let request = domain_request(
            "dialogue",
            DomainAction::DialogueSet {
                selection: dialogue_selection(),
                baseline: dialogue_baseline(),
                text: text.clone(),
            },
        );
        assert_eq!(
            state.submit_domain(request.clone()).unwrap().kind,
            "dialogue_set"
        );
        let drained = state.drain_domain_requests();
        let DomainAction::DialogueSet { text: stored, .. } = &drained[0].action else {
            panic!("expected dialogue set");
        };
        assert_eq!(stored, &text);
        assert_eq!(
            state.submit_domain(request.clone()).unwrap().state,
            DomainOperationState::Accepted
        );
        let mut collision = request;
        let DomainAction::DialogueSet { text, .. } = &mut collision.action else {
            unreachable!()
        };
        text.push('x');
        assert!(state.submit_domain(collision).is_err());
        let mut wrong_reference = dialogue_selection();
        wrong_reference.identity.reference.as_mut().unwrap().id = "other".into();
        assert!(state
            .submit_domain(domain_request(
                "wrong-reference",
                DomainAction::DialogueRead {
                    selection: wrong_reference
                }
            ))
            .is_err());
        let mut malformed = dialogue_baseline();
        malformed.target_overrides_token = "A".repeat(64);
        assert!(state
            .submit_domain(domain_request(
                "bad-baseline",
                DomainAction::DialogueResetEntry {
                    selection: dialogue_selection(),
                    baseline: malformed,
                },
            ))
            .is_err());
        let mut blank_entry = dialogue_baseline();
        blank_entry.override_entry = Some("  \n  ".into());
        assert!(state
            .submit_domain(domain_request(
                "blank-entry",
                DomainAction::DialogueResetEntry {
                    selection: dialogue_selection(),
                    baseline: blank_entry,
                },
            ))
            .is_err());
        let mut invalid_key = SessionIdentity {
            instance_id: "other".into(),
            source_id: 1,
            generation: 2,
            terminal_id: "term".into(),
        };
        assert!(state
            .submit_domain(domain_request(
                "wrong-instance",
                DomainAction::WorktreeInspect {
                    key: invalid_key.clone()
                },
            ))
            .is_err());
        invalid_key.instance_id = "test".into();
        invalid_key.terminal_id.clear();
        assert!(state
            .submit_domain(domain_request(
                "empty-terminal",
                DomainAction::WorktreeInspect { key: invalid_key },
            ))
            .is_err());
        assert!(state
            .submit_domain(domain_request(
                "bad-token",
                DomainAction::WorktreeRemove {
                    token: "not-a-token".into()
                },
            ))
            .is_err());
        assert!(state
            .submit_domain(domain_request(
                "too-long",
                DomainAction::DialogueSet {
                    selection: dialogue_selection(),
                    baseline: dialogue_baseline(),
                    text: "한".repeat(683),
                },
            ))
            .is_err());
    }

    #[test]
    fn saved_dialogue_and_acknowledged_removal_survive_interrupted_observation() {
        let mut state = AutomationState::with_instance("test".into(), target(), None);
        for (id, action) in [
            (
                "inactive-dialogue",
                DomainAction::DialogueResetEntry {
                    selection: dialogue_selection(),
                    baseline: dialogue_baseline(),
                },
            ),
            (
                "saved-dialogue",
                DomainAction::DialogueResetCharacter {
                    identity: dialogue_selection().identity,
                    metadata_token: "a".repeat(64),
                    target_overrides_token: "b".repeat(64),
                },
            ),
            (
                "unacknowledged",
                DomainAction::WorktreeRemove {
                    token: "c".repeat(64),
                },
            ),
            (
                "acknowledged",
                DomainAction::WorktreeRemove {
                    token: "d".repeat(64),
                },
            ),
            ("reader", DomainAction::DialogueList {}),
        ] {
            state.submit_domain(domain_request(id, action)).unwrap();
        }
        state.drain_domain_requests();
        for id in [
            "inactive-dialogue",
            "saved-dialogue",
            "unacknowledged",
            "acknowledged",
            "reader",
        ] {
            state.start_domain_request(id).unwrap();
        }
        assert!(state
            .finish_domain_operation(
                "inactive-dialogue",
                DomainOperationState::Applied,
                false,
                false,
                None,
                None,
                None
            )
            .is_err());
        let inactive = state
            .finish_domain_operation(
                "inactive-dialogue",
                DomainOperationState::Applied,
                true,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(inactive.committed && !inactive.native_applied);
        assert!(state
            .finish_domain_operation(
                "reader",
                DomainOperationState::Applied,
                true,
                false,
                None,
                None,
                None
            )
            .is_err());
        state
            .finish_domain_operation(
                "reader",
                DomainOperationState::Applied,
                false,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        state
            .finish_domain_operation(
                "saved-dialogue",
                DomainOperationState::Pending,
                true,
                false,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(state
            .finish_domain_operation(
                "saved-dialogue",
                DomainOperationState::Pending,
                false,
                false,
                None,
                None,
                None
            )
            .is_err());
        assert!(state
            .finish_domain_operation(
                "acknowledged",
                DomainOperationState::Applied,
                true,
                true,
                None,
                None,
                None
            )
            .is_err());
        let ack = serde_json::json!({"acknowledged": true, "final_observed": false});
        state
            .finish_domain_operation(
                "acknowledged",
                DomainOperationState::Pending,
                true,
                false,
                Some(ack.clone()),
                None,
                None,
            )
            .unwrap();
        state.shutdown();
        let saved = state.domain_status("test", "saved-dialogue").unwrap();
        assert_eq!(saved.state, DomainOperationState::Shutdown);
        assert!(saved.committed);
        assert_eq!(
            state.domain_status("test", "unacknowledged").unwrap().state,
            DomainOperationState::UnknownDelivery
        );
        let acknowledged = state.domain_status("test", "acknowledged").unwrap();
        assert_eq!(acknowledged.state, DomainOperationState::Shutdown);
        assert!(acknowledged.committed);
        assert_eq!(acknowledged.result, Some(ack));
        assert_eq!(
            acknowledged.error_code.as_deref(),
            Some("observation_interrupted")
        );
    }

    #[test]
    fn domain_wire_rejects_unknown_fields_and_wrong_scalar_types() {
        assert!(serde_json::from_str::<SessionIdentity>(
            r#"{"instance_id":"test","source_id":0,"generation":1,"terminal_id":"t","extra":1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<DomainRequest>(
            r#"{"instance_id":"test","operation_id":"a","action":{"action":"session_prompt","key":{"instance_id":"test","source_id":"0","generation":1,"terminal_id":"t"},"text":"hi"}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<DomainRequest>(
            r#"{"instance_id":"test","operation_id":"a","action":{"action":"preferences_get","extra":true}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<DomainRequest>(
            r#"{"instance_id":"test","operation_id":"a","action":{"action":"dialogue_list","extra":true}}"#
        )
        .is_err());
        let mut invalid = serde_json::json!({
            "instance_id": "test",
            "operation_id": "a",
            "action": {
                "action": "dialogue_set",
                "selection": dialogue_selection(),
                "baseline": dialogue_baseline(),
                "text": "hello",
            }
        });
        invalid["action"]["baseline"]["unexpected"] = Value::Bool(true);
        assert!(serde_json::from_value::<DomainRequest>(invalid.clone()).is_err());
        invalid["action"]["baseline"] = serde_json::to_value(dialogue_baseline()).unwrap();
        invalid["action"]["selection"]["identity"]["unexpected"] = Value::Bool(true);
        assert!(serde_json::from_value::<DomainRequest>(invalid).is_err());
        assert!(serde_json::from_str::<DomainRequest>(
            r#"{"instance_id":"test","operation_id":"a","action":{"action":"worktree_inspect","key":{"instance_id":"test","source_id":1,"generation":2,"terminal_id":"t","unexpected":true}}}"#
        )
        .is_err());
    }
}
