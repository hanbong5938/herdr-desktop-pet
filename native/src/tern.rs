use crate::herdr::{PromptError, PromptReceipt};
use crate::session::{AgentRecord, AgentStatus, SessionMetadata, MAX_AGENT_RECORDS};
use crate::session_view::SessionKey;
use crate::sources::SourceRoute;
use crate::state::SourceCounts;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};
pub(crate) const MAX_WINDOWS: usize = 64;
// Request JSON only; the optional line terminator is not part of this budget.
pub(crate) const MAX_EXCHANGE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 512 * 1024;
pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(crate) const FRESHNESS: Duration = Duration::from_secs(5);
const MAX_FIELD_BYTES: usize = 4096;
static NEXT_COMMAND: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TernExchangeRequest {
    pub version: u64,
    pub kind: String,
    pub window: String,
    pub epoch: u64,
    pub sequence: u64,
    pub instance_id: Option<String>,
    pub agents: Vec<TernAgent>,
    pub results: Vec<TernResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TernAgent {
    pub pane: String,
    pub title: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub session_label: Option<String>,
    pub tab_id: Option<String>,
    pub tab_label: Option<String>,
    pub state: TernAgentState,
    pub delivery_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TernAgentState {
    Idle,
    Working,
    WaitingInput,
    Exited,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TernResult {
    pub operation_id: String,
    pub pane: String,
    pub status: TernResultStatus,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TernResultStatus {
    Accepted,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TernCommand {
    pub operation_id: String,
    pub pane: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TernExchangeErrorCode {
    BindingRequired,
    InstanceChanged,
    SourceRetired,
    ReceiptRejected,
    Rejected,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TernExchangeReply {
    pub ok: bool,
    pub instance_id: String,
    pub generation: u64,
    pub command: Option<TernCommand>,
    pub error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_error_code")]
    pub error_code: Option<TernExchangeErrorCode>,
}

impl TernExchangeReply {
    pub(crate) fn error(
        instance_id: String,
        code: TernExchangeErrorCode,
        mut error: String,
    ) -> Self {
        if error.len() > MAX_FIELD_BYTES {
            let mut end = MAX_FIELD_BYTES;
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            error.truncate(end);
        }
        Self {
            ok: false,
            instance_id,
            generation: 0,
            command: None,
            error: Some(error),
            error_code: Some(code),
        }
    }
}

fn deserialize_error_code<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TernExchangeErrorCode>, D::Error> {
    TernExchangeErrorCode::deserialize(deserializer).map(Some)
}

pub(crate) fn serialize_response(reply: &TernExchangeReply) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(reply)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(serde_json::Error::io(std::io::Error::other(
            "Tern response size admission invariant failed",
        )));
    }
    Ok(bytes)
}

#[derive(Serialize)]
struct CommandRef<'a> {
    operation_id: &'a str,
    pane: &'a str,
    text: &'a str,
}

#[derive(Serialize)]
struct ReplyRef<'a> {
    ok: bool,
    instance_id: &'a str,
    generation: u64,
    command: Option<CommandRef<'a>>,
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<TernExchangeErrorCode>,
}

impl Serialize for TernExchangeReply {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ReplyRef {
            ok: self.ok,
            instance_id: &self.instance_id,
            generation: self.generation,
            command: self.command.as_ref().map(|command| CommandRef {
                operation_id: &command.operation_id,
                pane: &command.pane,
                text: &command.text,
            }),
            error: self.error.as_deref(),
            error_code: self.error_code,
        }
        .serialize(serializer)
    }
}

#[derive(Default)]
struct ResponseCounter {
    bytes: usize,
    too_large: bool,
}

impl std::io::Write for ResponseCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > (MAX_RESPONSE_BYTES - 1).saturating_sub(self.bytes) {
            self.too_large = true;
            return Err(std::io::Error::other("Tern response is too large"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Count the same borrowed envelope used by the real reply serializer, including
/// the helper's trailing LF, without allocating a second copy of the text.
pub(crate) fn check_command_response(
    instance_id: &str,
    generation: u64,
    operation_id: &str,
    pane: &str,
    text: &str,
) -> Result<(), PromptError> {
    let reply = ReplyRef {
        ok: true,
        instance_id,
        generation,
        command: Some(CommandRef {
            operation_id,
            pane,
            text,
        }),
        error: None,
        error_code: None,
    };
    let mut counter = ResponseCounter::default();
    let result = serde_json::to_writer(&mut counter, &reply);
    if counter.too_large {
        Err(PromptError::TooLarge)
    } else {
        result.map_err(|error| PromptError::Other(error.to_string()))
    }
}

fn bounded(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_FIELD_BYTES && !value.contains('\0')
}
fn optional_bounded(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| bounded(value))
}
fn optional_sized(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_none_or(|value| value.len() <= MAX_FIELD_BYTES && !value.contains('\0'))
}

impl TernExchangeRequest {
    /// Validate the entire inventory before touching any source or receipt.
    pub(crate) fn normalize(&self) -> Result<(Vec<AgentRecord>, SourceCounts), String> {
        if self.version != 1
            || self.kind != "tern"
            || !bounded(&self.window)
            || self.epoch == 0
            || self.epoch > MAX_SAFE_INTEGER
            || self.sequence == 0
            || self.sequence > MAX_SAFE_INTEGER
            || !optional_bounded(&self.instance_id)
            || self.agents.len() > MAX_AGENT_RECORDS
            || self.results.len() > 1
        {
            return Err("invalid Tern exchange fields".into());
        }
        let mut panes = HashSet::with_capacity(self.agents.len());
        let mut records = Vec::with_capacity(self.agents.len());
        let mut counts = SourceCounts::default();
        for agent in &self.agents {
            if !bounded(&agent.pane)
                || agent.title.len() > MAX_FIELD_BYTES
                || agent.cwd.len() > MAX_FIELD_BYTES
                || !optional_sized(&agent.session_id)
                || !optional_sized(&agent.session_label)
                || !optional_sized(&agent.tab_id)
                || !optional_sized(&agent.tab_label)
                || !optional_sized(&agent.delivery_error)
                || !panes.insert(agent.pane.as_str())
            {
                return Err("invalid or duplicate Tern agent pane".into());
            }
            // Pane IDs are window-scoped; session IDs are metadata, never identity.
            let terminal_id = agent.pane.clone();
            let status = match agent.state {
                TernAgentState::Idle => AgentStatus::Idle,
                TernAgentState::Working => AgentStatus::Working,
                TernAgentState::WaitingInput => AgentStatus::Blocked,
                // No completion signal or outcome is declared by the installed API.
                TernAgentState::Exited => AgentStatus::Unknown,
            };
            counts.sessions += 1;
            match status {
                AgentStatus::Working => counts.working += 1,
                AgentStatus::Blocked => counts.blocked += 1,
                AgentStatus::Unknown => counts.unknown += 1,
                AgentStatus::Idle | AgentStatus::Done => {}
            }
            records.push(AgentRecord {
                terminal_id,
                pane_id: agent.pane.clone(),
                status,
                metadata: SessionMetadata {
                    title: Some(agent.title.clone()),
                    agent: Some("Tern".into()),
                    workspace_id: agent.session_id.clone(),
                    workspace_label: agent.session_label.clone(),
                    tab_id: agent.tab_id.clone(),
                    tab_label: agent.tab_label.clone(),
                    cwd: Some(agent.cwd.clone()),
                    ..SessionMetadata::default()
                },
                outcome_authoritative: false,
                outcome: None,
            });
        }
        for result in &self.results {
            if !bounded(&result.operation_id)
                || !bounded(&result.pane)
                || !optional_sized(&result.error)
                || matches!(result.status, TernResultStatus::Accepted) && result.error.is_some()
            {
                return Err("invalid Tern command result".into());
            }
        }
        Ok((records, counts))
    }
}

pub(crate) fn source_id(window: &str) -> String {
    let mut source = String::with_capacity(5 + window.len() * 2);
    source.push_str("tern:");
    for byte in window.bytes() {
        use std::fmt::Write;
        let _ = write!(source, "{byte:02x}");
    }
    source
}

#[derive(Debug)]
pub(crate) struct TernRegistry {
    windows: HashMap<String, Window>,
}

#[derive(Debug)]
pub(crate) struct Window {
    pub(crate) epoch: u64,
    pub(crate) sequence: u64,
    pub(crate) generation: u64,
    pub(crate) source: String,
    pub(crate) last_seen: Instant,
    pub(crate) offline: bool,
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    key: SessionKey,
    command: TernCommand,
    sender: SyncSender<Result<PromptReceipt, PromptError>>,
    offered: bool,
}

impl Default for TernRegistry {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }
}

impl TernRegistry {
    pub(crate) fn get(&self, window: &str) -> Option<&Window> {
        self.windows.get(window)
    }
    pub(crate) fn get_mut(&mut self, window: &str) -> Option<&mut Window> {
        self.windows.get_mut(window)
    }
    pub(crate) fn len(&self) -> usize {
        self.windows.len()
    }
    pub(crate) fn insert(&mut self, window: String, entry: Window) {
        self.windows.insert(window, entry);
    }
    pub(crate) fn expired_registrations(
        &self,
        now: Instant,
    ) -> impl Iterator<Item = (&str, &Window)> {
        self.windows.iter().filter_map(move |(window, entry)| {
            (entry.offline
                && entry.pending.is_none()
                && now.saturating_duration_since(entry.last_seen) >= FRESHNESS)
                .then_some((window.as_str(), entry))
        })
    }
    pub(crate) fn source_generation(&self, source: &str) -> Option<u64> {
        self.windows
            .values()
            .find_map(|entry| (entry.source == source).then_some(entry.generation))
    }
    pub(crate) fn remove_source(&mut self, source: &str, generation: u64) {
        let window = self.windows.iter().find_map(|(window, entry)| {
            (entry.source == source && entry.generation == generation).then(|| window.clone())
        });
        if let Some(window) = window {
            self.reject_pending(&window, PromptError::StaleTarget);
            self.windows.remove(&window);
        }
    }
    pub(crate) fn route(&self, source: &str, generation: u64) -> Option<SourceRoute> {
        self.windows.iter().find_map(|(window, entry)| {
            (entry.source == source
                && entry.generation == generation
                && !entry.offline
                && entry.last_seen.elapsed() < FRESHNESS)
                .then(|| SourceRoute::TernWindow {
                    window: window.clone(),
                    epoch: entry.epoch,
                })
        })
    }
    pub(crate) fn expire(&mut self, now: Instant) -> Vec<(String, u64)> {
        let mut offline = Vec::new();
        for entry in self.windows.values_mut() {
            if now.saturating_duration_since(entry.last_seen) >= FRESHNESS {
                let was_offline = entry.offline;
                entry.offline = true;
                if let Some(pending) = entry.pending.take() {
                    let _ = pending.sender.send(Err(if pending.offered {
                        PromptError::UnknownDelivery
                    } else {
                        PromptError::Offline
                    }));
                }
                if !was_offline {
                    offline.push((entry.source.clone(), entry.generation));
                }
            }
        }
        offline
    }
    pub(crate) fn queue(
        &mut self,
        window: &str,
        epoch: u64,
        key: SessionKey,
        pane: &str,
        text: &str,
        instance_id: &str,
    ) -> Result<(String, Receiver<Result<PromptReceipt, PromptError>>), PromptError> {
        let entry = self.windows.get_mut(window).ok_or(PromptError::Offline)?;
        if entry.epoch != epoch
            || entry.generation != key.generation
            || entry.offline
            || entry.last_seen.elapsed() >= FRESHNESS
        {
            return Err(PromptError::StaleTarget);
        }
        if entry.pending.is_some() {
            return Err(PromptError::Busy);
        }
        let operation_id = format!(
            "tern-prompt-{}-{}",
            std::process::id(),
            NEXT_COMMAND
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| PromptError::Other("Tern command IDs exhausted".into()))?
        );
        check_command_response(instance_id, entry.generation, &operation_id, pane, text)?;
        let (tx, rx) = mpsc::sync_channel(1);
        entry.pending = Some(Pending {
            key,
            command: TernCommand {
                operation_id: operation_id.clone(),
                pane: pane.to_owned(),
                text: text.to_owned(),
            },
            sender: tx,
            offered: false,
        });
        Ok((operation_id, rx))
    }
    pub(crate) fn cancel(&mut self, window: &str, operation_id: &str) {
        if let Some(entry) = self.windows.get_mut(window) {
            if entry
                .pending
                .as_ref()
                .is_some_and(|pending| pending.command.operation_id == operation_id)
            {
                if let Some(pending) = entry.pending.take() {
                    let _ = pending.sender.send(Err(if pending.offered {
                        PromptError::UnknownDelivery
                    } else {
                        PromptError::Offline
                    }));
                }
            }
        }
    }
    pub(crate) fn validate_result(&self, request: &TernExchangeRequest) -> Result<(), String> {
        let pending = self
            .windows
            .get(&request.window)
            .and_then(|window| window.pending.as_ref());
        for result in &request.results {
            if !pending.is_some_and(|pending| {
                pending.offered
                    && pending.command.operation_id == result.operation_id
                    && pending.command.pane == result.pane
            }) {
                return Err("stale or unrecognized Tern command result".into());
            }
        }
        Ok(())
    }
    pub(crate) fn settle(&mut self, window: &str, result: Option<&TernResult>) {
        let Some(entry) = self.windows.get_mut(window) else {
            return;
        };
        if let Some(pending) = entry.pending.as_ref() {
            if pending.offered {
                let pending = entry.pending.take().expect("offered command");
                let receipt = match result {
                    Some(TernResult {
                        status: TernResultStatus::Accepted,
                        ..
                    }) => Ok(PromptReceipt::Accepted),
                    Some(TernResult {
                        status: TernResultStatus::Failed,
                        error,
                        ..
                    }) => Err(PromptError::Other(
                        error.clone().unwrap_or_else(|| "Tern rejected ask".into()),
                    )),
                    None => Err(PromptError::UnknownDelivery),
                };
                let _ = pending.sender.send(receipt);
            }
        }
    }
    pub(crate) fn pending_target(&self, window: &str) -> Option<(SessionKey, String)> {
        let pending = self.windows.get(window)?.pending.as_ref()?;
        (!pending.offered).then(|| (pending.key.clone(), pending.command.pane.clone()))
    }
    pub(crate) fn offer(&mut self, window: &str, instance_id: &str) -> Option<TernCommand> {
        let entry = self.windows.get(window)?;
        let pending = entry.pending.as_ref()?;
        if pending.offered {
            return None;
        }
        let admission = if entry.offline || entry.last_seen.elapsed() >= FRESHNESS {
            Err(PromptError::StaleTarget)
        } else {
            check_command_response(
                instance_id,
                entry.generation,
                &pending.command.operation_id,
                &pending.command.pane,
                &pending.command.text,
            )
        };
        if let Err(error) = admission {
            self.reject_pending(window, error);
            return None;
        }
        let pending = self.windows.get_mut(window)?.pending.as_mut()?;
        pending.offered = true;
        Some(TernCommand {
            operation_id: pending.command.operation_id.clone(),
            pane: pending.command.pane.clone(),
            text: std::mem::take(&mut pending.command.text),
        })
    }
    pub(crate) fn reject_pending(&mut self, window: &str, error: PromptError) {
        if let Some(pending) = self
            .windows
            .get_mut(window)
            .and_then(|entry| entry.pending.take())
        {
            let _ = pending.sender.send(Err(if pending.offered {
                PromptError::UnknownDelivery
            } else {
                error
            }));
        }
    }
}

impl Window {
    pub(crate) fn new(window: &str, epoch: u64, generation: u64, now: Instant) -> Self {
        Self {
            epoch,
            sequence: 0,
            generation,
            source: source_id(window),
            last_seen: now,
            offline: false,
            pending: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: String) -> TernExchangeReply {
        TernExchangeReply {
            ok: true,
            instance_id: "daemon-a".into(),
            generation: MAX_SAFE_INTEGER,
            command: Some(TernCommand {
                operation_id: "boundary-operation".into(),
                pane: "pane-한".into(),
                text,
            }),
            error: None,
            error_code: None,
        }
    }

    #[test]
    fn real_reply_serializer_admits_exact_utf8_and_escaped_boundaries() {
        let escaped = "\"\\\n\t한🪶";
        let empty = serialize_response(&reply(String::new())).unwrap().len();
        let escaped_len = serde_json::to_string(escaped).unwrap().len() - 2;
        let text = format!(
            "{escaped}{}",
            "a".repeat(MAX_RESPONSE_BYTES - empty - escaped_len)
        );
        let exact = reply(text.clone());
        let bytes = serialize_response(&exact).unwrap();
        assert_eq!(bytes.len(), MAX_RESPONSE_BYTES);
        assert_eq!(bytes.last(), Some(&b'\n'));
        let decoded: TernExchangeReply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.command.unwrap().text, text);
        assert_eq!(
            check_command_response(
                "daemon-a",
                MAX_SAFE_INTEGER,
                "boundary-operation",
                "pane-한",
                &text,
            ),
            Ok(())
        );
        let mut above = text.clone();
        above.push('a');
        assert_eq!(
            check_command_response(
                "daemon-a",
                MAX_SAFE_INTEGER,
                "boundary-operation",
                "pane-한",
                &above,
            ),
            Err(PromptError::TooLarge)
        );
        assert!(serialize_response(&reply(above)).is_err());
        let mut below = text;
        below.pop();
        assert_eq!(
            serialize_response(&reply(below.clone())).unwrap().len(),
            MAX_RESPONSE_BYTES - 1
        );
        assert_eq!(
            check_command_response(
                "daemon-a",
                MAX_SAFE_INTEGER,
                "boundary-operation",
                "pane-한",
                &below,
            ),
            Ok(())
        );
    }

    #[test]
    fn preoffer_size_invariant_fails_definitely_before_offered_state() {
        let mut registry = TernRegistry::default();
        let mut window = Window::new("window-a", 1, MAX_SAFE_INTEGER, Instant::now());
        let (sender, receipt) = mpsc::sync_channel(1);
        window.pending = Some(Pending {
            key: SessionKey {
                source_id: 0,
                generation: MAX_SAFE_INTEGER,
                terminal_id: "pane-한".into(),
            },
            command: reply("a".repeat(MAX_RESPONSE_BYTES)).command.unwrap(),
            sender,
            offered: false,
        });
        registry.insert("window-a".into(), window);
        assert!(registry.offer("window-a", "daemon-a").is_none());
        assert_eq!(receipt.try_recv(), Ok(Err(PromptError::TooLarge)));
        assert!(registry.get("window-a").unwrap().pending.is_none());
        let key = SessionKey {
            source_id: 0,
            generation: MAX_SAFE_INTEGER,
            terminal_id: "pane-한".into(),
        };
        let (_, accepted) = registry
            .queue("window-a", 1, key, "pane-한", "next valid ask", "daemon-a")
            .unwrap();
        let command = registry.offer("window-a", "daemon-a").unwrap();
        assert_eq!(command.text, "next valid ask");
        assert!(registry.offer("window-a", "daemon-a").is_none());
        registry.settle("window-a", None);
        assert_eq!(accepted.try_recv(), Ok(Err(PromptError::UnknownDelivery)));
    }

    #[test]
    fn freshness_boundary_finalizes_pending_before_selecting_registration() {
        let mut registry = TernRegistry::default();
        let now = Instant::now();
        registry.insert("window-a".into(), Window::new("window-a", 1, 1, now));
        let key = SessionKey {
            source_id: 0,
            generation: 1,
            terminal_id: "pane".into(),
        };
        let (_, receipt) = registry
            .queue("window-a", 1, key.clone(), "pane", "ask", "daemon-a")
            .unwrap();
        registry.get_mut("window-a").unwrap().last_seen = now - FRESHNESS;
        assert!(registry.route(&source_id("window-a"), 1).is_none());
        assert!(matches!(
            registry.queue("window-a", 1, key, "pane", "stale", "daemon-a",),
            Err(PromptError::StaleTarget)
        ));
        assert_eq!(registry.expired_registrations(now).count(), 0);
        assert_eq!(registry.expire(now), vec![(source_id("window-a"), 1)]);
        assert_eq!(receipt.try_recv(), Ok(Err(PromptError::Offline)));
        assert_eq!(registry.expired_registrations(now).count(), 1);
    }

    #[test]
    fn canonical_error_codes_are_bounded_and_success_keeps_old_shape() {
        let failure = TernExchangeReply::error(
            "daemon-a".into(),
            TernExchangeErrorCode::ReceiptRejected,
            "한".repeat(4096),
        );
        assert!(failure.error.as_ref().unwrap().len() <= MAX_FIELD_BYTES);
        let failure: serde_json::Value =
            serde_json::from_slice(&serialize_response(&failure).unwrap()).unwrap();
        assert_eq!(failure["error_code"], "receipt_rejected");
        assert_eq!(failure["generation"], 0);
        assert!(failure["command"].is_null());
        let mut success = reply("ask".into());
        success.command = None;
        let bytes = serialize_response(&success).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("error_code").is_none());
        assert!(serde_json::from_slice::<TernExchangeReply>(&bytes).is_ok());
        value["error_code"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<TernExchangeReply>(value).is_err());
    }
}
