use crate::agent_outcome::OutcomeReport;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};
use std::sync::Arc;

pub(crate) const MAX_FRAME_BYTES: usize = 512 * 1024;
pub(crate) const MAX_AGENT_RECORDS: usize = 16_384;
const MAX_METADATA_BYTES: usize = 4096;
const DEFAULT_EVENT_TIMEOUT: Duration = Duration::from_millis(100);
const EVENT_FRAME_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) enum ProtocolError {
    Io(io::Error),
    Json(serde_json::Error),
    FrameTooLarge,
    EmptyResponse,
    Timeout,
    InvalidResponse(String),
    Remote { code: String, message: String },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::FrameTooLarge => write!(f, "Herdr API frame exceeds the size limit"),
            Self::EmptyResponse => write!(f, "Herdr API returned an empty response"),
            Self::Timeout => write!(f, "Herdr API request timed out"),
            Self::InvalidResponse(message) => write!(f, "invalid Herdr API response: {message}"),
            Self::Remote { code, message } => write!(f, "Herdr API error {code}: {message}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            Self::Timeout
        } else {
            Self::Io(error)
        }
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug)]
pub(crate) enum Response {
    Success { result: Value },
    Error { code: String, message: String },
}

impl Response {
    pub(crate) fn result(self) -> Result<Value, ProtocolError> {
        match self {
            Self::Success { result } => Ok(result),
            Self::Error { code, message } => Err(ProtocolError::Remote { code, message }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    fn parse(value: Option<&Value>) -> Result<Self, ProtocolError> {
        match value.and_then(Value::as_str) {
            Some("idle") => Ok(Self::Idle),
            Some("working") => Ok(Self::Working),
            Some("blocked") => Ok(Self::Blocked),
            Some("done") => Ok(Self::Done),
            Some("unknown") => Ok(Self::Unknown),
            Some(other) => Err(ProtocolError::InvalidResponse(format!(
                "unsupported agent_status {other:?}"
            ))),
            None => Err(ProtocolError::InvalidResponse(
                "agent_status must be a string".to_owned(),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceWorktreeInfo {
    pub(crate) repo_key: String,
    pub(crate) repo_name: String,
    pub(crate) repo_root: String,
    pub(crate) checkout_path: String,
    pub(crate) is_linked_worktree: bool,
    pub(crate) pane_count: usize,
    pub(crate) tab_count: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionMetadata {
    pub(crate) title: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) worktree: Option<Arc<WorkspaceWorktreeInfo>>,
    pub(crate) workspace_label: Option<String>,
    pub(crate) tab_id: Option<String>,
    pub(crate) tab_label: Option<String>,
    pub(crate) cwd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentRecord {
    pub terminal_id: String,
    pub pane_id: String,
    pub status: AgentStatus,
    pub metadata: SessionMetadata,
    pub outcome_authoritative: bool,
    pub outcome: Option<OutcomeReport>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub panes: Vec<String>,
    pub agents: Vec<AgentRecord>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SnapshotCounts {
    pub sessions: usize,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub unknown: usize,
}

impl Snapshot {
    pub(crate) fn pane_ids(&self) -> Vec<String> {
        let mut panes = self.panes.clone();
        panes.sort_unstable();
        panes.dedup();
        panes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PluginAvailability {
    Enabled,
    Disabled,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionEvent {
    AgentStatusChanged {
        pane_id: String,
        status: AgentStatus,
    },
    Other,
}
pub(crate) struct SubscriptionReader {
    stream: UnixStream,
    partial: Vec<u8>,
    frame_deadline: Option<Instant>,
}

impl SubscriptionReader {
    pub(crate) fn read_event(&mut self) -> Result<SubscriptionEventRead, ProtocolError> {
        if let Some(deadline) = self.frame_deadline {
            if !self.partial.is_empty() && deadline <= Instant::now() {
                self.frame_deadline = None;
                return Err(ProtocolError::Timeout);
            }
        }
        let started = Instant::now();
        let poll_deadline = started + DEFAULT_EVENT_TIMEOUT;
        let deadline = self
            .frame_deadline
            .map_or(poll_deadline, |frame| frame.min(poll_deadline));
        match read_frame(&mut self.stream, &mut self.partial, deadline) {
            Ok(None) => {
                self.frame_deadline = None;
                Ok(SubscriptionEventRead::Closed)
            }
            Ok(Some(line)) => {
                self.frame_deadline = None;
                Ok(SubscriptionEventRead::Event(parse_subscription_line(
                    &line,
                )?))
            }
            Err(ProtocolError::Timeout) => {
                if self.partial.is_empty() {
                    self.frame_deadline = None;
                } else if self.frame_deadline.is_none() {
                    self.frame_deadline = Some(started + EVENT_FRAME_TIMEOUT);
                }
                Ok(SubscriptionEventRead::Timeout)
            }
            Err(error) => {
                self.frame_deadline = None;
                Err(error)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionEventRead {
    Event(SubscriptionEvent),
    Timeout,
    Closed,
}

pub(crate) fn request(
    endpoint: &Path,
    id: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Response, ProtocolError> {
    request_with_delivery(endpoint, id, method, params, timeout).map_err(|(error, _)| error)
}

/// The flag reports whether a write was attempted. Once that happens a failed
/// response cannot establish that the server did not receive the submission.
pub(crate) fn request_with_delivery(
    endpoint: &Path,
    id: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Response, (ProtocolError, bool)> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        (
            ProtocolError::InvalidResponse("request deadline overflow".to_owned()),
            false,
        )
    })?;
    let mut stream = crate::socket::connect(endpoint, deadline)
        .map_err(|error| (ProtocolError::from(error), false))?;
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params,
    });
    let mut write_attempted = false;
    write_json_line(&mut stream, &request, deadline, &mut write_attempted)
        .map_err(|error| (error, write_attempted))?;
    let mut partial = Vec::with_capacity(1024);
    let line = read_frame(&mut stream, &mut partial, deadline)
        .map_err(|error| (error, true))?
        .ok_or((ProtocolError::EmptyResponse, true))?;
    parse_response_line(&line, id).map_err(|error| (error, true))
}

pub(crate) fn subscribe(
    endpoint: &Path,
    id: &str,
    pane_ids: &[String],
    timeout: Duration,
) -> Result<SubscriptionReader, ProtocolError> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        ProtocolError::InvalidResponse("subscription deadline overflow".to_owned())
    })?;
    let mut stream = crate::socket::connect(endpoint, deadline).map_err(ProtocolError::from)?;
    let subscriptions: Vec<Value> = pane_ids
        .iter()
        .map(|pane_id| {
            serde_json::json!({
                "type": "pane.agent_status_changed",
                "pane_id": pane_id,
            })
        })
        .collect();
    let request = serde_json::json!({
        "id": id,
        "method": "events.subscribe",
        "params": { "subscriptions": subscriptions },
    });
    write_json_line(&mut stream, &request, deadline, &mut false)?;
    let mut partial = Vec::with_capacity(1024);
    let line =
        read_frame(&mut stream, &mut partial, deadline)?.ok_or(ProtocolError::EmptyResponse)?;
    let response = parse_response_line(&line, id)?;
    let result = response.result()?;
    let kind = result
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::InvalidResponse("subscription ack has no type".to_owned()))?;
    if kind != "subscription_started" {
        return Err(ProtocolError::InvalidResponse(format!(
            "expected subscription_started, got {kind:?}"
        )));
    }
    Ok(SubscriptionReader {
        stream,
        partial,
        frame_deadline: None,
    })
}

pub(crate) fn parse_response_line(
    line: &str,
    expected_id: &str,
) -> Result<Response, ProtocolError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let value: Value = serde_json::from_str(line.trim())?;
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::InvalidResponse("response must be a JSON object".to_owned())
    })?;
    let id = string_field(object, "id")?.to_owned();
    if id != expected_id {
        return Err(ProtocolError::InvalidResponse(format!(
            "response id {id:?} does not match request {expected_id:?}"
        )));
    }
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => {
            if !result.is_object() {
                return Err(ProtocolError::InvalidResponse(
                    "response result must be an object".to_owned(),
                ));
            }
            Ok(Response::Success {
                result: result.clone(),
            })
        }
        (None, Some(error)) => {
            let error = error.as_object().ok_or_else(|| {
                ProtocolError::InvalidResponse("response error must be an object".to_owned())
            })?;
            let code = string_field(error, "code")?.to_owned();
            let message = string_field(error, "message")?.to_owned();
            Ok(Response::Error { code, message })
        }
        (Some(_), Some(_)) => Err(ProtocolError::InvalidResponse(
            "response has both result and error".to_owned(),
        )),
        (None, None) => Err(ProtocolError::InvalidResponse(
            "response has neither result nor error".to_owned(),
        )),
    }
}

pub(crate) fn parse_snapshot(response: Response) -> Result<Snapshot, ProtocolError> {
    let result = response.result()?;
    if result.get("type").and_then(Value::as_str) != Some("session_snapshot") {
        return Err(ProtocolError::InvalidResponse(
            "expected session_snapshot result".to_owned(),
        ));
    }
    let snapshot = result
        .get("snapshot")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ProtocolError::InvalidResponse("session_snapshot has no snapshot".to_owned())
        })?;
    let panes = snapshot
        .get("panes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::InvalidResponse("session snapshot has no panes".to_owned())
        })?;
    if panes.len() > MAX_AGENT_RECORDS {
        return Err(ProtocolError::InvalidResponse("too many panes".to_owned()));
    }
    let mut pane_ids = Vec::with_capacity(panes.len());
    for value in panes {
        let pane = value.as_object().ok_or_else(|| {
            ProtocolError::InvalidResponse("session pane must be an object".to_owned())
        })?;
        let pane_id = string_field(pane, "pane_id")?.to_owned();
        if pane_id.is_empty() {
            return Err(ProtocolError::InvalidResponse(
                "session pane IDs must not be empty".to_owned(),
            ));
        }
        pane_ids.push(pane_id);
    }
    let agents = snapshot
        .get("agents")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::InvalidResponse("session snapshot has no agents".to_owned())
        })?;
    if agents.len() > MAX_AGENT_RECORDS {
        return Err(ProtocolError::InvalidResponse("too many agents".to_owned()));
    }
    // Optional collections cannot invalidate an otherwise usable snapshot.
    let workspaces = metadata_labels(snapshot, "workspaces", "workspace_id");
    let worktree_info = workspace_worktrees(snapshot);
    let tabs = metadata_labels(snapshot, "tabs", "tab_id");
    let mut records = Vec::with_capacity(agents.len());
    for value in agents {
        let agent = value.as_object().ok_or_else(|| {
            ProtocolError::InvalidResponse("session agent must be an object".to_owned())
        })?;
        let terminal_id = string_field(agent, "terminal_id")?.to_owned();
        let pane_id = string_field(agent, "pane_id")?.to_owned();
        if terminal_id.is_empty() || pane_id.is_empty() {
            return Err(ProtocolError::InvalidResponse(
                "session agent IDs must not be empty".to_owned(),
            ));
        }
        let status = AgentStatus::parse(agent.get("agent_status"))?;
        let (outcome_authoritative, outcome) = parse_agent_outcome(agent);
        let workspace_id = safe_text(agent, "workspace_id");
        let tab_id = optional_text(agent, "tab_id");
        let omp = agent.get("agent").and_then(Value::as_str) == Some("omp");
        let title = agent
            .get("terminal_title_stripped")
            .and_then(Value::as_str)
            .and_then(|value| normalize_title(value, omp))
            .or_else(|| {
                agent
                    .get("terminal_title")
                    .and_then(Value::as_str)
                    .and_then(|value| normalize_title(value, omp))
            });
        let metadata = SessionMetadata {
            title,
            agent: optional_text(agent, "agent").map(str::to_owned),
            workspace_label: workspace_id
                .and_then(|id| workspaces.get(id).map(|label| (*label).to_owned())),
            workspace_id: workspace_id.map(str::to_owned),
            worktree: workspace_id.and_then(|id| worktree_info.get(id).cloned()),
            tab_label: tab_id.and_then(|id| tabs.get(id).map(|label| (*label).to_owned())),
            tab_id: tab_id.map(str::to_owned),
            cwd: ["foreground_cwd", "cwd"]
                .into_iter()
                .filter_map(|field| optional_text(agent, field))
                .find(|path| path.starts_with('/') && !path.chars().any(is_unsafe_format))
                .map(str::to_owned),
        };
        records.push(AgentRecord {
            terminal_id,
            pane_id,
            status,
            outcome_authoritative,
            metadata,
            outcome,
        });
    }
    Ok(Snapshot {
        panes: pane_ids,
        agents: records,
    })
}

// Worktree permission is optional metadata, independent of snapshot validity.
// Any repeated workspace identity, even with an invalid second record, fails closed.
fn workspace_worktrees(snapshot: &Map<String, Value>) -> HashMap<&str, Arc<WorkspaceWorktreeInfo>> {
    let Some(workspaces) = snapshot.get("workspaces").and_then(Value::as_array) else {
        return HashMap::new();
    };
    if workspaces.len() > MAX_AGENT_RECORDS {
        return HashMap::new();
    }
    let mut result = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for entry in workspaces {
        let Some(workspace) = entry.as_object() else {
            continue;
        };
        let Some(id) = safe_text(workspace, "workspace_id") else {
            continue;
        };
        if !seen.insert(id) {
            result.remove(id);
            continue;
        }
        let Some(worktree) = workspace.get("worktree").and_then(Value::as_object) else {
            continue;
        };
        let Some((repo_key, repo_name, repo_root, checkout_path)) = (|| {
            Some((
                safe_text(worktree, "repo_key")?,
                safe_text(worktree, "repo_name")?,
                safe_absolute_path(worktree, "repo_root")?,
                safe_absolute_path(worktree, "checkout_path")?,
            ))
        })() else {
            continue;
        };
        let (Some(is_linked_worktree), Some(pane_count), Some(tab_count)) = (
            worktree.get("is_linked_worktree").and_then(Value::as_bool),
            bounded_positive_count(workspace, "pane_count"),
            bounded_positive_count(workspace, "tab_count"),
        ) else {
            continue;
        };
        result.insert(
            id,
            Arc::new(WorkspaceWorktreeInfo {
                repo_key: repo_key.to_owned(),
                repo_name: repo_name.to_owned(),
                repo_root: repo_root.to_owned(),
                checkout_path: checkout_path.to_owned(),
                is_linked_worktree,
                pane_count,
                tab_count,
            }),
        );
    }
    result
}

fn safe_text<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    optional_text(object, field).filter(|text| !text.chars().any(is_unsafe_format))
}

fn safe_absolute_path<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    safe_text(object, field).filter(|path| Path::new(path).is_absolute())
}

fn bounded_positive_count(object: &Map<String, Value>, field: &str) -> Option<usize> {
    object
        .get(field)?
        .as_u64()
        .filter(|count| (1..=MAX_AGENT_RECORDS as u64).contains(count))
        .map(|count| count as usize)
}

fn optional_text<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= MAX_METADATA_BYTES)
}

fn metadata_labels<'a>(
    snapshot: &'a Map<String, Value>,
    collection: &str,
    id_field: &str,
) -> HashMap<&'a str, &'a str> {
    snapshot
        .get(collection)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_AGENT_RECORDS)
        .filter_map(|entry| {
            let object = entry.as_object()?;
            Some((
                optional_text(object, id_field)?,
                optional_text(object, "label")?,
            ))
        })
        .collect()
}

fn is_unsafe_format(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061c}'
                | '\u{200b}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
        )
}

fn normalize_title(value: &str, omp: bool) -> Option<String> {
    if value.len() > MAX_METADATA_BYTES {
        return None;
    }
    let value = value.trim();
    let value = if omp {
        if let Some(rest) = value.strip_prefix('π') {
            if rest.starts_with(char::is_whitespace)
                || rest.starts_with(|c: char| ('\u{2800}'..='\u{28ff}').contains(&c))
                || rest.is_empty()
            {
                let rest = rest.trim_start();
                rest.strip_prefix(|c: char| ('\u{2800}'..='\u{28ff}').contains(&c))
                    .unwrap_or(rest)
                    .trim_start()
            } else {
                value
            }
        } else {
            value
        }
    } else {
        value
    };
    let mut normalized = String::with_capacity(value.len());
    for word in value.split(|c: char| c.is_whitespace() || is_unsafe_format(c)) {
        if !word.is_empty() {
            if !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str(word);
        }
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_agent_outcome(agent: &Map<String, Value>) -> (bool, Option<OutcomeReport>) {
    // OMP's status is not an outcome, even before companion metadata arrives.
    if agent.get("agent").and_then(Value::as_str) != Some("omp") {
        return (false, None);
    }
    let Some(tokens) = agent.get("tokens") else {
        return (true, None);
    };
    if tokens.get("pet_outcome_authority").and_then(Value::as_str) != Some("omp:v1") {
        return (true, None);
    }
    let report = OutcomeReport::parse_tokens(tokens).filter(|report| {
        let Some(session) = agent.get("agent_session").and_then(Value::as_object) else {
            return false;
        };
        if session.get("agent").and_then(Value::as_str) != Some("omp") {
            return false;
        }
        let Some(kind @ ("id" | "path")) = session.get("kind").and_then(Value::as_str) else {
            return false;
        };
        let Some(value) = session.get("value").and_then(Value::as_str) else {
            return false;
        };
        if value.is_empty() || value.len() > 4096 {
            return false;
        }
        let mut hash = Sha256::new();
        hash.update(kind.as_bytes());
        hash.update(b":");
        hash.update(value.as_bytes());
        report.session == format!("{:x}", hash.finalize())
    });
    (true, report)
}

pub(crate) fn parse_plugin_availability(
    response: Response,
) -> Result<PluginAvailability, ProtocolError> {
    let result = response.result()?;
    if result.get("type").and_then(Value::as_str) != Some("plugin_list") {
        return Err(ProtocolError::InvalidResponse(
            "expected plugin_list result".to_owned(),
        ));
    }
    let plugins = result
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::InvalidResponse("plugin_list has no plugins".to_owned()))?;
    let mut found_present = false;
    let mut found_enabled = false;
    for value in plugins {
        let plugin = value.as_object().ok_or_else(|| {
            ProtocolError::InvalidResponse("plugin entry must be an object".to_owned())
        })?;
        let plugin_id = string_field(plugin, "plugin_id")?;
        let enabled = plugin
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                ProtocolError::InvalidResponse("plugin enabled must be a boolean".to_owned())
            })?;
        if plugin_id == "desktop-pet" {
            found_present = true;
            if enabled {
                found_enabled = true;
            }
        }
    }
    Ok(if found_enabled {
        PluginAvailability::Enabled
    } else if found_present {
        PluginAvailability::Disabled
    } else {
        PluginAvailability::Missing
    })
}

pub(crate) fn parse_subscription_line(line: &str) -> Result<SubscriptionEvent, ProtocolError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let value: Value = serde_json::from_str(line.trim())?;
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::InvalidResponse("subscription envelope must be an object".to_owned())
    })?;
    let event = string_field(object, "event")?;
    let data = object
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ProtocolError::InvalidResponse("subscription event has no data".to_owned())
        })?;
    if event != "pane.agent_status_changed" {
        return Ok(SubscriptionEvent::Other);
    }
    let pane_id = string_field(data, "pane_id")?.to_owned();
    if pane_id.is_empty() {
        return Err(ProtocolError::InvalidResponse(
            "subscription pane IDs must not be empty".to_owned(),
        ));
    }
    let _workspace_id = string_field(data, "workspace_id")?;
    let status = AgentStatus::parse(data.get("agent_status"))?;
    Ok(SubscriptionEvent::AgentStatusChanged { pane_id, status })
}

fn write_json_line(
    stream: &mut UnixStream,
    value: &Value,
    deadline: Instant,
    write_attempted: &mut bool,
) -> Result<(), ProtocolError> {
    let mut encoded = serde_json::to_vec(value)?;
    if encoded.len().saturating_add(1) > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    encoded.push(b'\n');
    let mut offset = 0usize;
    while offset < encoded.len() {
        let remaining = crate::socket::remaining(deadline).map_err(ProtocolError::from)?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(ProtocolError::from)?;
        *write_attempted = true;
        let written = stream.write(&encoded[offset..])?;
        if written == 0 {
            return Err(ProtocolError::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                "Herdr API request write made no progress",
            )));
        }
        offset = offset.saturating_add(written);
    }
    Ok(())
}

fn read_frame(
    stream: &mut UnixStream,
    partial: &mut Vec<u8>,
    deadline: Instant,
) -> Result<Option<String>, ProtocolError> {
    loop {
        if let Some(position) = partial.iter().position(|byte| *byte == b'\n') {
            let frame: Vec<u8> = partial.drain(..=position).collect();
            if frame.len() > MAX_FRAME_BYTES {
                return Err(ProtocolError::FrameTooLarge);
            }
            let line = String::from_utf8(frame)
                .map_err(|_| ProtocolError::InvalidResponse("frame is not UTF-8".to_owned()))?;
            if line.trim().is_empty() {
                return Err(ProtocolError::EmptyResponse);
            }
            return Ok(Some(line));
        }
        if partial.len() >= MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        let available = MAX_FRAME_BYTES.saturating_sub(partial.len()).min(8192);
        let mut chunk = [0u8; 8192];
        let read = crate::socket::read_with_deadline(stream, &mut chunk[..available], deadline)?;
        if read == 0 {
            if partial.is_empty() {
                return Ok(None);
            }
            return Err(ProtocolError::InvalidResponse(
                "truncated Herdr API frame".to_owned(),
            ));
        }
        partial.extend_from_slice(&chunk[..read]);
    }
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, ProtocolError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::InvalidResponse(format!("{field} must be a string")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_tokens_must_match_the_current_omp_session() {
        let identity = "/private/session/current.jsonl";
        let hash = format!(
            "{:x}",
            Sha256::digest(format!("path:{identity}").as_bytes())
        );
        let mut agent = serde_json::json!({
            "agent": "omp",
            "agent_session": { "agent": "omp", "kind": "path", "value": identity },
            "tokens": {
                "pet_outcome_authority": "omp:v1",
                "pet_outcome_session": hash,
                "pet_outcome_turn": "100",
                "pet_outcome": "failed",
                "pet_outcome_at": "1790120000000"
            }
        });
        let (authoritative, report) = parse_agent_outcome(agent.as_object().unwrap());
        assert!(authoritative);
        assert_eq!(
            report.map(|report| report.outcome),
            Some(crate::agent_outcome::AgentOutcome::Failed)
        );
        agent["agent_session"]["value"] = Value::String("/private/session/next.jsonl".to_owned());
        let (authoritative, report) = parse_agent_outcome(agent.as_object().unwrap());
        assert!(authoritative);
        assert!(report.is_none());
        agent["tokens"] = serde_json::json!({});
        assert_eq!(
            parse_agent_outcome(agent.as_object().unwrap()),
            (true, None)
        );
        agent["agent"] = Value::String("another-agent".to_owned());
        assert_eq!(
            parse_agent_outcome(agent.as_object().unwrap()),
            (false, None)
        );
    }

    #[test]
    fn parses_real_success_response_and_rejects_wrong_id() {
        let response = parse_response_line(
            r#"{"id":"snapshot-1","result":{"type":"session_snapshot","snapshot":{"panes":[{"pane_id":"p1"},{"pane_id":"p2"}],"agents":[]}}}"#,
            "snapshot-1",
        )
        .unwrap();
        let snapshot = parse_snapshot(response).unwrap();
        assert!(snapshot.agents.is_empty());
        assert_eq!(snapshot.pane_ids(), vec!["p1".to_owned(), "p2".to_owned()]);
        assert!(
            parse_response_line(r#"{"id":"other","result":{"type":"pong"}}"#, "snapshot-1")
                .is_err()
        );
    }

    #[test]
    fn optional_session_metadata_joins_exact_ids_and_normalizes_omp_titles() {
        let mut agent = serde_json::json!({
            "terminal_id": "t", "pane_id": "p", "agent_status": "working",
            "agent": "omp", "terminal_title_stripped": "π ⠹ Split\u{202e} README \n by Language",
            "terminal_title": "fallback", "workspace_id": "w2", "tab_id": "tab2",
            "foreground_cwd": "/repo/full/path", "cwd": "/repo"
        });
        let response = |agent: Value| Response::Success {
            result: serde_json::json!({
                "type": "session_snapshot",
                "snapshot": {
                    "panes": [{"pane_id": "p"}], "agents": [agent],
                    "workspaces": [{"workspace_id": "w", "label": "Wrong"}, {"workspace_id": "w2", "label": "Idea"}],
                    "tabs": [{"tab_id": "tab", "label": "Wrong"}, {"tab_id": "tab2", "label": "2"}]
                }
            }),
        };
        let first = parse_snapshot(response(agent.clone()))
            .unwrap()
            .agents
            .remove(0);
        assert_eq!(
            first.metadata.title.as_deref(),
            Some("Split README by Language")
        );
        assert_eq!(first.metadata.workspace_label.as_deref(), Some("Idea"));
        assert_eq!(first.metadata.tab_label.as_deref(), Some("2"));
        assert_eq!(first.metadata.cwd.as_deref(), Some("/repo/full/path"));
        assert!(first.outcome_authoritative);
        agent["terminal_title_stripped"] = Value::String("π ⠇ Split README by Language".to_owned());
        let second = parse_snapshot(response(agent)).unwrap().agents.remove(0);
        assert_eq!(first, second);
        assert_eq!(
            normalize_title("✨ Ship 🚀", true).as_deref(),
            Some("✨ Ship 🚀")
        );
        assert_eq!(
            normalize_title("πxel editor", true).as_deref(),
            Some("πxel editor")
        );
    }

    #[test]
    fn old_snapshots_and_malformed_optional_fields_keep_core_records() {
        let snapshot = |agent: Value, workspaces: Value| Response::Success {
            result: serde_json::json!({
                "type": "session_snapshot",
                "snapshot": {"panes": [{"pane_id": "p"}], "agents": [agent], "workspaces": workspaces, "tabs": false}
            }),
        };
        let old = serde_json::json!({"terminal_id": "t", "pane_id": "p", "agent_status": "done"});
        let record = parse_snapshot(snapshot(old.clone(), Value::Null))
            .unwrap()
            .agents
            .remove(0);
        assert_eq!(record.metadata, SessionMetadata::default());
        let mut malformed = old;
        malformed["agent"] = serde_json::json!(17);
        malformed["terminal_title_stripped"] = serde_json::json!([]);
        malformed["terminal_title"] = serde_json::json!("✨ Useful");
        malformed["workspace_id"] = serde_json::json!("w");
        malformed["foreground_cwd"] = serde_json::json!(false);
        malformed["cwd"] = serde_json::json!("/fallback");
        let record = parse_snapshot(snapshot(
            malformed,
            serde_json::json!([{"workspace_id":"w","label":false}]),
        ))
        .unwrap()
        .agents
        .remove(0);
        assert_eq!(record.status, AgentStatus::Done);
        assert_eq!(record.metadata.title.as_deref(), Some("✨ Useful"));
        assert_eq!(record.metadata.workspace_label, None);
        assert_eq!(record.metadata.cwd.as_deref(), Some("/fallback"));
        assert_eq!(
            normalize_title(&"x".repeat(MAX_METADATA_BYTES + 1), false),
            None
        );
    }

    #[test]
    fn worktree_metadata_requires_unique_complete_safe_workspace_records() {
        let agent = serde_json::json!({
            "terminal_id": "t", "pane_id": "p", "agent_status": "working",
            "workspace_id": "w"
        });
        let good = serde_json::json!({
            "workspace_id": "w", "pane_count": 2, "tab_count": 3,
            "worktree": {
                "repo_key": "repo", "repo_name": "Project", "repo_root": "/repo",
                "checkout_path": "/repo-linked", "is_linked_worktree": true
            }
        });
        let parse = |workspaces: Value| {
            parse_snapshot(Response::Success {
                result: serde_json::json!({
                    "type": "session_snapshot",
                    "snapshot": {"panes": [{"pane_id": "p"}], "agents": [agent], "workspaces": workspaces}
                }),
            })
            .unwrap()
            .agents
            .remove(0)
        };
        let valid = parse(serde_json::json!([good.clone()]));
        let info = valid.metadata.worktree.as_ref().unwrap();
        assert_eq!((info.pane_count, info.tab_count), (2, 3));
        assert_eq!(info.checkout_path, "/repo-linked");
        assert!(info.is_linked_worktree);
        let mut unrelated = good.clone();
        unrelated["workspace_id"] = serde_json::json!("other");
        assert!(parse(serde_json::json!([unrelated]))
            .metadata
            .worktree
            .is_none());
        let mut main = good.clone();
        main["worktree"]["is_linked_worktree"] = serde_json::json!(false);
        assert!(
            !parse(serde_json::json!([main]))
                .metadata
                .worktree
                .unwrap()
                .is_linked_worktree
        );
        let duplicate = parse(serde_json::json!([good.clone(), {"workspace_id": "w"}]));
        assert!(duplicate.metadata.worktree.is_none());
        assert_eq!(duplicate.status, AgentStatus::Working);
        let duplicate_first = parse(serde_json::json!([{"workspace_id": "w"}, good.clone()]));
        assert!(duplicate_first.metadata.worktree.is_none());
        for bad in [
            Value::Null,
            serde_json::json!([{"workspace_id": "w", "worktree": null}]),
            serde_json::json!([{"workspace_id": "w", "pane_count": 0, "tab_count": 1, "worktree": good["worktree"]}]),
            serde_json::json!([{"workspace_id": "w", "pane_count": 1, "tab_count": "2", "worktree": good["worktree"]}]),
            serde_json::json!([{"workspace_id": "w", "pane_count": 1, "tab_count": 1,
                "worktree": {"repo_key": "repo", "repo_name": "Project", "repo_root": "/repo",
                    "checkout_path": "relative", "is_linked_worktree": true}}]),
            serde_json::json!([{"workspace_id": "w", "pane_count": 1, "tab_count": 1,
                "worktree": {"repo_key": "repo", "repo_name": "Project", "repo_root": "/repo",
                    "checkout_path": "/unsafe\npath", "is_linked_worktree": true}}]),
        ] {
            let record = parse(bad);
            assert!(record.metadata.worktree.is_none());
            assert_eq!(record.metadata.workspace_id.as_deref(), Some("w"));
        }
        let oversized = Value::Array(vec![good; MAX_AGENT_RECORDS + 1]);
        assert!(parse(oversized).metadata.worktree.is_none());
    }

    #[test]
    fn parses_subscription_ack_event_envelope() {
        let event = parse_subscription_line(
            r#"{"event":"pane.agent_status_changed","data":{"pane_id":"p1","workspace_id":"w","agent_status":"blocked"}}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            SubscriptionEvent::AgentStatusChanged {
                pane_id: "p1".to_owned(),
                status: AgentStatus::Blocked,
            }
        );
    }

    #[test]
    fn plugin_availability_distinguishes_missing_disabled_and_enabled() {
        let cases = [
            (serde_json::json!([]), PluginAvailability::Missing),
            (
                serde_json::json!([{"plugin_id": "other", "enabled": true}]),
                PluginAvailability::Missing,
            ),
            (
                serde_json::json!([{"plugin_id": "desktop-pet", "enabled": false}]),
                PluginAvailability::Disabled,
            ),
            (
                serde_json::json!([{"plugin_id": "desktop-pet", "enabled": true}]),
                PluginAvailability::Enabled,
            ),
            (
                serde_json::json!([
                    {"plugin_id": "desktop-pet", "enabled": false},
                    {"plugin_id": "desktop-pet", "enabled": true}
                ]),
                PluginAvailability::Enabled,
            ),
        ];
        for (plugins, expected) in cases {
            let response = Response::Success {
                result: serde_json::json!({"type": "plugin_list", "plugins": plugins}),
            };
            assert_eq!(parse_plugin_availability(response).unwrap(), expected);
        }
    }

    #[test]
    fn malformed_plugin_lists_and_api_errors_do_not_report_availability() {
        let invalid_results = [
            serde_json::json!({"type": "session_snapshot", "plugins": []}),
            serde_json::json!({"type": "plugin_list"}),
            serde_json::json!({"type": "plugin_list", "plugins": [false]}),
            serde_json::json!({"type": "plugin_list", "plugins": [{"enabled": true}]}),
            serde_json::json!({"type": "plugin_list", "plugins": [
                {"plugin_id": "desktop-pet", "enabled": "true"}
            ]}),
            serde_json::json!({"type": "plugin_list", "plugins": [
                {"plugin_id": "desktop-pet", "enabled": true},
                {"plugin_id": "other", "enabled": null}
            ]}),
        ];
        for result in invalid_results {
            assert!(matches!(
                parse_plugin_availability(Response::Success { result }),
                Err(ProtocolError::InvalidResponse(_))
            ));
        }
        assert!(matches!(
            parse_plugin_availability(Response::Error {
                code: "unavailable".to_owned(),
                message: "retry later".to_owned(),
            }),
            Err(ProtocolError::Remote { .. })
        ));
    }

    #[test]
    fn fragmented_subscription_frame_survives_read_timeout() {
        use std::io::Write;
        let (stream, mut writer) = UnixStream::pair().unwrap();
        let mut reader = SubscriptionReader {
            stream,
            partial: Vec::new(),
            frame_deadline: None,
        };
        writer.write_all(br#"{"event":"pane.agent_status_changed","data":{"workspace_id":"w1","pane_id":"p1","agent_status":"working"}"#).unwrap();
        assert!(matches!(
            reader.read_event().unwrap(),
            SubscriptionEventRead::Timeout
        ));
        writer.write_all(b"}\n").unwrap();
        assert_eq!(
            reader.read_event().unwrap(),
            SubscriptionEventRead::Event(SubscriptionEvent::AgentStatusChanged {
                pane_id: "p1".to_owned(),
                status: AgentStatus::Working,
            })
        );
    }
}
