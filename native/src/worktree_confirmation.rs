use crate::automation::SessionIdentity;
use crate::session_view::WorktreeRemoveTarget;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::time::{Duration, Instant};

const CONFIRMATION_TTL: Duration = Duration::from_secs(60);
const MAX_CONFIRMATIONS: usize = 32;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WorktreeConfirmation {
    pub(crate) token: String,
    pub(crate) instance_id: String,
    pub(crate) expires_in_seconds: u64,
    pub(crate) key: SessionIdentity,
    pub(crate) workspace_id: String,
    pub(crate) pane_id: String,
    pub(crate) worktree: FrozenWorktreeSummary,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FrozenWorktreeSummary {
    repo_key: String,
    repo_root: String,
    checkout_path: String,
    // Herdr's validated WorkspaceWorktreeInfo has no branch field. Do not
    // invent one from a path or claim a known branch when none was observed.
    branch: Option<String>,
    is_main: bool,
    is_linked: bool,
}

struct ConfirmationEntry {
    instance_id: String,
    target: WorktreeRemoveTarget,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct WorktreeConfirmations {
    entries: HashMap<String, ConfirmationEntry>,
}

impl WorktreeConfirmations {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn has_pending(&self) -> bool {
        let now = Instant::now();
        self.entries.values().any(|entry| now < entry.expires_at)
    }

    pub(crate) fn issue(
        &mut self,
        instance_id: &str,
        target: WorktreeRemoveTarget,
    ) -> Result<WorktreeConfirmation, String> {
        self.issue_at(instance_id, target, Instant::now())
    }

    fn issue_at(
        &mut self,
        instance_id: &str,
        target: WorktreeRemoveTarget,
        now: Instant,
    ) -> Result<WorktreeConfirmation, String> {
        if !target.worktree.is_linked_worktree
            || target.worktree.checkout_path == target.worktree.repo_root
        {
            return Err("worktree confirmation requires a linked checkout target".into());
        }
        self.entries.retain(|_, entry| now < entry.expires_at);
        if self.entries.len() == MAX_CONFIRMATIONS {
            return Err("too many outstanding worktree confirmations".into());
        }
        let expires_at = now
            .checked_add(CONFIRMATION_TTL)
            .ok_or("worktree confirmation deadline overflow")?;
        let token = loop {
            let token = random_token()?;
            if !self.entries.contains_key(&token) {
                break token;
            }
        };
        let info = &target.worktree;
        let confirmation = WorktreeConfirmation {
            token: token.clone(),
            instance_id: instance_id.to_owned(),
            expires_in_seconds: CONFIRMATION_TTL.as_secs(),
            key: SessionIdentity {
                instance_id: instance_id.to_owned(),
                source_id: target.key.source_id,
                generation: target.key.generation,
                terminal_id: target.key.terminal_id.clone(),
            },
            workspace_id: target.workspace_id.clone(),
            pane_id: target.pane_id.clone(),
            worktree: FrozenWorktreeSummary {
                repo_key: info.repo_key.clone(),
                repo_root: info.repo_root.clone(),
                checkout_path: info.checkout_path.clone(),
                branch: None,
                is_main: false,
                is_linked: info.is_linked_worktree,
            },
        };
        self.entries.insert(
            token,
            ConfirmationEntry {
                instance_id: instance_id.to_owned(),
                target,
                expires_at,
            },
        );
        Ok(confirmation)
    }

    pub(crate) fn consume(
        &mut self,
        token: &str,
        instance_id: &str,
    ) -> Result<WorktreeRemoveTarget, String> {
        self.consume_at(token, instance_id, Instant::now())
    }

    fn consume_at(
        &mut self,
        token: &str,
        instance_id: &str,
        now: Instant,
    ) -> Result<WorktreeRemoveTarget, String> {
        self.entries.retain(|_, entry| now < entry.expires_at);
        let Some(entry) = self.entries.get(token) else {
            return Err("unknown or expired worktree confirmation".into());
        };
        if entry.instance_id != instance_id {
            return Err("worktree confirmation belongs to another instance".into());
        }
        Ok(self
            .entries
            .remove(token)
            .expect("matching confirmation exists")
            .target)
    }
}

fn random_token() -> Result<String, String> {
    let mut random = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .map_err(|error| format!("cannot obtain OS random worktree confirmation: {error}"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = [0u8; 64];
    for (pair, byte) in encoded.chunks_exact_mut(2).zip(random) {
        pair[0] = HEX[usize::from(byte >> 4)];
        pair[1] = HEX[usize::from(byte & 15)];
    }
    Ok(String::from_utf8(encoded.to_vec()).expect("hex digits are ASCII"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr_protocol::WorkspaceWorktreeInfo;
    use crate::session_view::SessionKey;
    use serde_json::json;
    use std::sync::Arc;

    fn target() -> WorktreeRemoveTarget {
        WorktreeRemoveTarget {
            key: SessionKey {
                source_id: 4,
                generation: 7,
                terminal_id: "term".into(),
            },
            source: "/private/socket".into(),
            pane_id: "pane".into(),
            workspace_id: "workspace".into(),
            worktree: Arc::new(WorkspaceWorktreeInfo {
                repo_key: "repo".into(),
                repo_name: "Display".into(),
                repo_root: "/repo".into(),
                checkout_path: "/repo/linked".into(),
                is_linked_worktree: true,
                pane_count: 2,
                tab_count: 1,
            }),
        }
    }

    #[test]
    fn freezes_summary_and_consumes_exact_target_only_once_for_own_instance() {
        let now = Instant::now();
        let mut confirmations = WorktreeConfirmations::new();
        let original = target();
        let frozen = confirmations
            .issue_at("daemon-a", original.clone(), now)
            .unwrap();
        let token = frozen.token.clone();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            serde_json::to_value(frozen).unwrap(),
            json!({
                "token": token.clone(), "instance_id": "daemon-a", "expires_in_seconds": 60,
                "key": {"instance_id": "daemon-a", "source_id": 4, "generation": 7,
                        "terminal_id": "term"},
                "workspace_id": "workspace", "pane_id": "pane",
                "worktree": {"repo_key": "repo", "repo_root": "/repo",
                             "checkout_path": "/repo/linked", "branch": null,
                             "is_main": false, "is_linked": true}
            })
        );
        assert!(confirmations.consume_at(&token, "daemon-b", now).is_err());
        assert_eq!(
            confirmations.consume_at(&token, "daemon-a", now).unwrap(),
            original
        );
        assert!(confirmations.consume_at(&token, "daemon-a", now).is_err());
    }

    #[test]
    fn expiry_is_exclusive_and_expired_slots_are_reclaimed() {
        let now = Instant::now();
        let mut confirmations = WorktreeConfirmations::new();
        let early = confirmations.issue_at("a", target(), now).unwrap();
        assert!(confirmations
            .consume_at(&early.token, "a", now + CONFIRMATION_TTL)
            .is_err());
        let fresh = confirmations
            .issue_at("a", target(), now + CONFIRMATION_TTL)
            .unwrap();
        assert_eq!(
            confirmations
                .consume_at(&fresh.token, "a", now + CONFIRMATION_TTL)
                .unwrap(),
            target()
        );
    }

    #[test]
    fn only_linked_non_root_checkout_targets_can_receive_tokens() {
        let now = Instant::now();
        let mut confirmations = WorktreeConfirmations::new();
        let mut nonlinked = target();
        let mut metadata = (*nonlinked.worktree).clone();
        metadata.is_linked_worktree = false;
        nonlinked.worktree = Arc::new(metadata);
        assert!(confirmations.issue_at("a", nonlinked, now).is_err());
        let mut root = target();
        let mut metadata = (*root.worktree).clone();
        metadata.checkout_path = metadata.repo_root.clone();
        root.worktree = Arc::new(metadata);
        assert!(confirmations.issue_at("a", root, now).is_err());
        assert!(confirmations.entries.is_empty());
    }

    #[test]
    fn caps_live_entries_without_evicting_valid_tokens() {
        let now = Instant::now();
        let mut confirmations = WorktreeConfirmations::new();
        let tokens: Vec<_> = (0..MAX_CONFIRMATIONS)
            .map(|_| confirmations.issue_at("a", target(), now).unwrap().token)
            .collect();
        assert_eq!(
            tokens
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            32
        );
        assert!(confirmations.issue_at("a", target(), now).is_err());
        assert_eq!(
            confirmations.consume_at(&tokens[0], "a", now).unwrap(),
            target()
        );
        assert!(confirmations.issue_at("a", target(), now).is_ok());
    }
}
