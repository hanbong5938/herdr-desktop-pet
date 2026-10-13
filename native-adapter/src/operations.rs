use crate::{Route, Shared};
use crate::protocol::{self, ProtocolError};
use desktop_pet_contracts::*;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(650);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(3);
const WORKTREE_REMOVE_TIMEOUT: Duration = Duration::from_secs(30);
static REQUEST_ID: AtomicU64 = AtomicU64::new(0);
fn request_id(kind: &str) -> String {
    format!("desktop-pet-{kind}-{}-{}", std::process::id(), REQUEST_ID.fetch_add(1, Ordering::Relaxed))
}
pub(crate) fn serialized_len(value: &impl serde::Serialize) -> Result<usize, serde_json::Error> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 = self.0.saturating_add(bytes.len()); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}
pub(crate) fn prompt_frame_fits(pane: &str, text_bytes: usize) -> Result<bool, serde_json::Error> {
    // Longest possible actual request ID, without copying the submitted text.
    let overhead = br#"{"id":"desktop-pet-prompt-4294967295-18446744073709551615","method":"agent.prompt","params":{"target":"#.len()
        + br#","text":"#.len() + b"}}\n".len();
    Ok(overhead.saturating_add(serialized_len(&pane)?).saturating_add(text_bytes) <= protocol::MAX_FRAME_BYTES)
}
#[derive(serde::Serialize)]
struct PromptParams<'a> { target: &'a str, text: &'a str }
#[derive(serde::Serialize)]
struct RemoveParams<'a> { workspace_id: &'a str, force: bool }
pub(crate) fn send_prompt(shared: &Arc<Shared>, operation: u64, key: &SessionKey, target: &PromptTarget, path: &Path, text: &str, cancelled: &AtomicBool) -> Result<PromptReceipt, PromptError> {
    if cancelled.load(Ordering::Acquire) { return Err(PromptError::StaleTarget); }
    // Watchers register canonical local endpoint identities. Never reinterpret
    // a synthetic or aliased source as a different socket; filesystem
    // resolution happens after releasing the application mutex.
    if crate::socket::canonical_endpoint(path).map_err(|_| PromptError::StaleTarget)? != path {
        return Err(PromptError::StaleTarget);
    }
    let snapshot = protocol::request(
        path,
        &request_id("prompt"),
        "session.snapshot",
        serde_json::json!({}),
        REQUEST_TIMEOUT,
    )
    .and_then(protocol::parse_snapshot)
    .map_err(|error| match error {
        ProtocolError::Io(_) | ProtocolError::Timeout | ProtocolError::EmptyResponse => {
            PromptError::Offline
        }
        other => PromptError::Other(other.to_string()),
    })?;
    // Require a unique live pane/terminal pair on this server, not merely an
    // old watcher row. The final foreground/readiness guard belongs to Herdr.
    if snapshot
        .panes
        .iter()
        .filter(|pane| *pane == &target.pane_id)
        .count()
        != 1
        || snapshot
            .agents
            .iter()
            .filter(|agent| agent.terminal_id == key.terminal_id || agent.pane_id == target.pane_id)
            .count()
            != 1
        || !snapshot
            .agents
            .iter()
            .any(|agent| agent.terminal_id == key.terminal_id && agent.pane_id == target.pane_id)
    {
        return Err(PromptError::StaleTarget);
    }
    let mut admitted = Err(PromptError::StaleTarget);
    shared.transaction(&mut |domain, registry| {
        admitted = (|| {
            domain.mutation_allowed().map_err(PromptError::Other)?;
            let (current, provider) = domain.prompt_route(key)?;
            if current != *target || provider != "herdr" { return Err(PromptError::StaleTarget); }
            let binding = registry.bindings.get(&target.source).ok_or(PromptError::Offline)?;
            if binding.generation != key.generation || !binding.connected || binding.disabled || !matches!(&binding.route, Route::Local(current) if current.as_path() == path) { return Err(PromptError::StaleTarget); }
            let pending = registry.operations.get_mut(&operation).ok_or(PromptError::StaleTarget)?;
            if !registry.enabled || registry.finalized || registry.epoch != pending.epoch || domain.instance_id() != pending.instance || pending.generation != key.generation || cancelled.load(Ordering::Acquire) { return Err(PromptError::StaleTarget); }
            pending.offered = true;
            Ok(())
        })();
    }).map_err(PromptError::Other)?;
    admitted?;
    if cancelled.load(Ordering::Acquire) { return Err(PromptError::UnknownDelivery); }
    let response = protocol::request_with_delivery(
        path,
        &request_id("prompt"),
        "agent.prompt",
        PromptParams { target: &target.pane_id, text },
        PROMPT_TIMEOUT,
    )
    .map_err(|(error, attempted)| {
        if attempted {
            PromptError::UnknownDelivery
        } else {
            match error {
                ProtocolError::FrameTooLarge => PromptError::TooLarge,
                ProtocolError::Io(_) | ProtocolError::Timeout => PromptError::Offline,
                other => PromptError::Other(other.to_string()),
            }
        }
    })?;
    match response {
        protocol::Response::Error { code, message } => Err(match code.as_str() {
            "agent_blocked" => PromptError::Blocked,
            "agent_not_ready" | "agent_not_idle" => PromptError::NotReady,
            "unsupported_method" | "method_not_found" | "unknown_method" => {
                PromptError::Unsupported
            }
            "agent_not_running" | "agent_not_found" | "pane_not_found" => PromptError::StaleTarget,
            _ => PromptError::Other(format!("{code}: {message}")),
        }),
        protocol::Response::Success { result } => {
            let agent = result.get("agent");
            if result.get("type").and_then(serde_json::Value::as_str) != Some("agent_prompted")
                || agent
                    .and_then(|agent| agent.get("terminal_id"))
                    .and_then(serde_json::Value::as_str)
                    != Some(key.terminal_id.as_str())
                || agent
                    .and_then(|agent| agent.get("pane_id"))
                    .and_then(serde_json::Value::as_str)
                    != Some(target.pane_id.as_str())
            {
                return Err(PromptError::UnknownDelivery);
            }
            Ok(PromptReceipt::Acknowledged)
        }
    }
}
pub(crate) fn send_worktree_remove(shared: &Arc<Shared>, operation: u64, target: &WorktreeRemoveTarget, path: &Path, cancelled: &AtomicBool) -> Result<(), WorktreeRemoveError> {
    if cancelled.load(Ordering::Acquire) { return Err(WorktreeRemoveError::StaleTarget); }
    if crate::socket::canonical_endpoint(path).map_err(|_| WorktreeRemoveError::StaleTarget)?
        != path
    {
        return Err(WorktreeRemoveError::StaleTarget);
    }
    let snapshot = protocol::request(
        path,
        &request_id("worktree-remove"),
        "session.snapshot",
        serde_json::json!({}),
        REQUEST_TIMEOUT,
    )
    .and_then(protocol::parse_snapshot)
    .map_err(|error| match error {
        ProtocolError::Io(_) | ProtocolError::Timeout | ProtocolError::EmptyResponse => {
            WorktreeRemoveError::Offline
        }
        _ => WorktreeRemoveError::StaleTarget,
    })?;
    if snapshot
        .panes
        .iter()
        .filter(|pane| *pane == &target.pane_id)
        .count()
        != 1
        || snapshot
            .agents
            .iter()
            .filter(|agent| {
                agent.terminal_id == target.key.terminal_id || agent.pane_id == target.pane_id
            })
            .count()
            != 1
        || !snapshot.agents.iter().any(|agent| {
            agent.terminal_id == target.key.terminal_id
                && agent.pane_id == target.pane_id
                && agent.metadata.workspace_id.as_deref() == Some(target.workspace_id.as_str())
                && agent.metadata.worktree.as_deref() == Some(target.worktree.as_ref())
        })
    {
        return Err(WorktreeRemoveError::StaleTarget);
    }
    let mut admitted = Err(WorktreeRemoveError::StaleTarget);
    shared.transaction(&mut |domain, registry| {
        admitted = (|| {
            domain.mutation_allowed().map_err(WorktreeRemoveError::Other)?;
            if domain.worktree_remove_target(&target.key)? != *target { return Err(WorktreeRemoveError::StaleTarget); }
            let binding = registry.bindings.get(&target.source).ok_or(WorktreeRemoveError::Offline)?;
            if binding.generation != target.key.generation || !binding.connected || binding.disabled || !matches!(&binding.route, Route::Local(current) if current.as_path() == path) { return Err(WorktreeRemoveError::StaleTarget); }
            let pending = registry.operations.get_mut(&operation).ok_or(WorktreeRemoveError::StaleTarget)?;
            if !registry.enabled || registry.finalized || registry.epoch != pending.epoch || domain.instance_id() != pending.instance || pending.generation != target.key.generation || cancelled.load(Ordering::Acquire) { return Err(WorktreeRemoveError::StaleTarget); }
            pending.offered = true;
            Ok(())
        })();
    }).map_err(WorktreeRemoveError::Other)?;
    admitted?;
    if cancelled.load(Ordering::Acquire) { return Err(WorktreeRemoveError::UnknownDelivery); }
    // Snapshot equality does not close the race before worktree.remove. Keep
    // force:false so Herdr performs its own dirty-worktree safety check; even
    // a subsequent watcher disappearance certifies neither disk nor pixels.
    let response = protocol::request_with_delivery(
        path,
        &request_id("worktree-remove"),
        "worktree.remove",
        RemoveParams { workspace_id: &target.workspace_id, force: false },
        WORKTREE_REMOVE_TIMEOUT,
    )
    .map_err(|(error, attempted)| {
        if attempted {
            WorktreeRemoveError::UnknownDelivery
        } else {
            match error {
                ProtocolError::Io(_) | ProtocolError::Timeout => WorktreeRemoveError::Offline,
                other => WorktreeRemoveError::Other(other.to_string()),
            }
        }
    })?;
    match response {
        protocol::Response::Error { code, message } => Err(
            if matches!(
                code.as_str(),
                "unsupported_method" | "method_not_found" | "unknown_method"
            ) {
                WorktreeRemoveError::Unsupported
            } else {
                WorktreeRemoveError::Rejected { code, message }
            },
        ),
        protocol::Response::Success { result } => {
            if result.get("type").and_then(serde_json::Value::as_str) != Some("worktree_removed")
                || result
                    .get("workspace_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(target.workspace_id.as_str())
                || result.get("path").and_then(serde_json::Value::as_str)
                    != Some(target.worktree.checkout_path.as_str())
                || result.get("forced").and_then(serde_json::Value::as_bool) != Some(false)
            {
                return Err(WorktreeRemoveError::UnknownDelivery);
            }
            Ok(())
        }
    }
}
