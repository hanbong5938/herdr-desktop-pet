use crate::bubble::BubblePlacement;
use crate::character_service::PackService;
use crate::character_types::{PackAction, PackListing, PackOperation, PackRequest};
use crate::herdr::Watchers;
use crate::lifecycle::{self, LifecycleLock, LifecycleSetting, LifecycleSettings, Paths};
use crate::socket;
use crate::state::AppState;
use crate::ui;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs::{self, Permissions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u64 = 1;
const MAX_FRAME_BYTES: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 32;
const PACK_LIST_PAGE_SIZE: usize = 8;
const MAX_PACK_LIST_RECORDS: usize = 32;
const MAX_PACK_LIST_ROUND_TRIPS: usize = MAX_PACK_LIST_RECORDS / PACK_LIST_PAGE_SIZE + 1;
const MAX_SOURCE_PATH_BYTES: usize = 4096;

#[derive(Debug, Deserialize)]
struct FrameKind {
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRequest {
    #[serde(default)]
    version: Option<u64>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleRequest {
    version: u64,
    kind: String,
    operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<LifecycleSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleReply {
    version: u64,
    kind: String,
    ok: bool,
    settings: Option<LifecycleSettings>,
    error: Option<String>,
}

fn validate_lifecycle_request(request: &LifecycleRequest) -> Result<(), String> {
    if request.version != PROTOCOL_VERSION || request.kind != "lifecycle" {
        return Err("invalid lifecycle protocol".to_owned());
    }
    match request.operation.as_str() {
        "get" if request.key.is_none() && request.value.is_none() => Ok(()),
        "set" if request.key.is_some() && request.value.is_some() => Ok(()),
        _ => Err("invalid lifecycle operation or key/value".to_owned()),
    }
}

fn lifecycle_reply(result: Result<LifecycleSettings, String>) -> LifecycleReply {
    match result {
        Ok(settings) => LifecycleReply {
            version: PROTOCOL_VERSION,
            kind: "lifecycle".to_owned(),
            ok: true,
            settings: Some(settings),
            error: None,
        },
        Err(error) => LifecycleReply {
            version: PROTOCOL_VERSION,
            kind: "lifecycle".to_owned(),
            ok: false,
            settings: None,
            error: Some(error),
        },
    }
}

struct UniqueJsonKeys;

impl<'de> DeserializeSeed<'de> for UniqueJsonKeys {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = access.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            access.next_value_seed(UniqueJsonKeys)?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while access.next_element_seed(UniqueJsonKeys)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }
}

pub const PACK_PROTOCOL_VERSION: u64 = 1;

/// Bounded pack-only IPC envelope.  The envelope intentionally contains only
/// validated metadata and paths; manifests and image bytes never cross the
/// control socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRequestEnvelope {
    pub version: u64,
    pub kind: String,
    pub command: String,
    #[serde(default)]
    pub request: Option<PackRequest>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackReplyEnvelope {
    pub version: u64,
    pub kind: String,
    pub command: String,
    pub ok: bool,
    #[serde(default)]
    pub listing: Option<PackListing>,
    #[serde(default)]
    pub operation: Option<PackOperation>,
    #[serde(default)]
    pub error: Option<String>,
}

const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
const SOCKET_MODE: u32 = 0o600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    pub r#type: String,
    pub version: u64,
    pub app_version: String,
    pub ok: bool,
    pub command: String,
    pub error: Option<String>,
    pub ready: bool,
    pub running: bool,
    pub phase: String,
    pub ui_ready: bool,
    pub control_ready: bool,
    pub registration_accepted: bool,
    pub data_connected: bool,
    pub connected_sources: usize,
    pub disconnected_sources: usize,
    pub sessions: usize,
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub unknown: usize,
    pub visible: bool,
    pub passthrough: bool,
    #[serde(default)]
    pub alpha_passthrough: bool,
    #[serde(default = "default_bubble_visible")]
    pub bubble_visible: bool,
    #[serde(default)]
    pub bubble_placement: BubblePlacement,
    pub scale: f64,
    pub shutdown: bool,
    pub auto_start: bool,
    pub exit_with_herdr: bool,
    pub pid: u32,
    pub executable_path: String,
    pub config_dir: String,
    pub state_dir: String,
    pub herdr_socket: String,
}

fn default_bubble_visible() -> bool {
    true
}

impl ControlResponse {
    fn status(command: impl Into<String>, inner: &ControlInner) -> Self {
        let command = command.into();
        let state = lock_unpoisoned(&inner.shared);
        let ui_ready = state.is_ui_ready();
        let scene = state.scene();
        let settings = state.lifecycle_settings();
        drop(state);
        let endpoint = lock_unpoisoned(&inner.registered);
        let registration_accepted = inner
            .endpoint
            .as_ref()
            .map(|path| endpoint.contains(path))
            .unwrap_or(true);
        Self {
            r#type: if command == "ready" {
                "ready"
            } else {
                "status"
            }
            .to_owned(),
            version: PROTOCOL_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            ok: true,
            command,
            error: None,
            ready: ui_ready && !scene.shutdown,
            running: true,
            phase: scene.phase.as_str().to_owned(),
            ui_ready,
            control_ready: true,
            registration_accepted,
            data_connected: scene.connected_sources > 0,
            connected_sources: scene.connected_sources,
            disconnected_sources: scene.disconnected_sources,
            sessions: scene.sessions,
            working: scene.working,
            blocked: scene.blocked,
            done: scene.done,
            unknown: scene.unknown,
            visible: scene.visible,
            passthrough: scene.passthrough,
            alpha_passthrough: scene.alpha_passthrough,
            bubble_visible: scene.bubble_visible,
            bubble_placement: scene.bubble_placement,
            scale: scene.scale,
            shutdown: scene.shutdown,
            auto_start: settings.auto_start,
            exit_with_herdr: settings.exit_with_herdr,
            pid: inner.pid,
            executable_path: inner.executable_path.display().to_string(),
            config_dir: inner.paths.config_dir.display().to_string(),
            state_dir: inner.paths.state_dir.display().to_string(),
            herdr_socket: inner
                .endpoint
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }

    fn error(command: impl Into<String>, inner: &ControlInner, message: impl Into<String>) -> Self {
        let mut response = Self::status(command, inner);
        response.r#type = "error".to_owned();
        response.ok = false;
        response.error = Some(message.into());
        response
    }
}

/// Serialize a local-control response as one bounded newline-delimited frame.
fn response_json(response: &ControlResponse) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control response exceeds frame limit",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

struct ControlInner {
    stopping: AtomicBool,
    shared: Arc<Mutex<AppState>>,
    watchers: Arc<Mutex<Watchers>>,
    packs: Arc<PackService>,
    paths: Paths,
    endpoint: Option<PathBuf>,
    registered: Mutex<HashSet<PathBuf>>,
    lifecycle_writes: Mutex<()>,
    executable_path: PathBuf,
    pid: u32,
    clients: Mutex<Vec<JoinHandle<()>>>,
    active_clients: Mutex<usize>,
}

pub struct ControlServer {
    inner: Arc<ControlInner>,
    listener: Option<UnixListener>,
    accept_thread: Option<JoinHandle<()>>,
    started: bool,
    stopped: bool,
}

impl ControlServer {
    pub fn bind(
        shared: Arc<Mutex<AppState>>,
        watchers: Arc<Mutex<Watchers>>,
        paths: Paths,
        endpoint: Option<PathBuf>,
        executable_path: PathBuf,
        packs: Arc<PackService>,
    ) -> Result<Self, String> {
        lifecycle::validate_directory(&paths.state_dir, true)?;
        let listener = bind_private_socket(&paths.control_socket)?;
        let inner = Arc::new(ControlInner {
            stopping: AtomicBool::new(false),
            shared,
            watchers,
            packs,
            paths,
            endpoint: endpoint
                .map(|path| socket::canonical_endpoint(&path))
                .transpose()?,
            registered: Mutex::new(HashSet::new()),
            lifecycle_writes: Mutex::new(()),
            executable_path,
            pid: std::process::id(),
            clients: Mutex::new(Vec::new()),
            active_clients: Mutex::new(0),
        });
        Ok(Self {
            inner,
            listener: Some(listener),
            accept_thread: None,
            started: false,
            stopped: false,
        })
    }

    pub fn record_registered_endpoint(&self, endpoint: PathBuf) -> Result<(), String> {
        let canonical = socket::canonical_endpoint(&endpoint)?;
        {
            let _write_guard = lock_unpoisoned(&self.inner.lifecycle_writes);
            lifecycle::add_endpoint(&self.inner.paths.config_dir, &canonical)?;
        }
        lock_unpoisoned(&self.inner.registered).insert(canonical);
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let listener = self
            .listener
            .take()
            .ok_or_else(|| "local control listener is unavailable".to_owned())?;
        let inner = Arc::clone(&self.inner);
        self.accept_thread = Some(
            thread::Builder::new()
                .name("desktop-pet-control".to_owned())
                .spawn(move || accept_loop(listener, inner))
                .map_err(|error| format!("cannot start local control listener: {error}"))?,
        );
        self.started = true;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.inner.stopping.store(true, Ordering::Release);
        wake_listener(&self.inner.paths.control_socket);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        let mut clients = lock_unpoisoned(&self.inner.clients);
        let handles = std::mem::take(&mut *clients);
        drop(clients);
        for handle in handles {
            let _ = handle.join();
        }
        let _ = remove_owned_socket(&self.inner.paths.control_socket);
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(listener: UnixListener, inner: Arc<ControlInner>) {
    loop {
        if inner.stopping.load(Ordering::Acquire) {
            return;
        }
        reap_finished_clients(&inner);
        let (stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) if inner.stopping.load(Ordering::Acquire) => return,
            Err(_) => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        if inner.stopping.load(Ordering::Acquire) || !verify_peer_uid(&stream) {
            continue;
        }
        // Darwin inherits the listener's nonblocking flag on accepted sockets.
        // Client I/O uses blocking syscalls bounded by the absolute deadline.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        if !acquire_client_slot(&inner) {
            continue;
        }
        let client_inner = Arc::clone(&inner);
        let handle = thread::Builder::new()
            .name("desktop-pet-control-client".to_owned())
            .spawn(move || {
                handle_client(stream, client_inner.clone());
                release_client_slot(&client_inner);
            });
        match handle {
            Ok(handle) => lock_unpoisoned(&inner.clients).push(handle),
            Err(_) => release_client_slot(&inner),
        }
    }
}

fn reap_finished_clients(inner: &ControlInner) {
    let mut finished = Vec::new();
    {
        let mut clients = lock_unpoisoned(&inner.clients);
        let mut index = 0;
        while index < clients.len() {
            if clients[index].is_finished() {
                finished.push(clients.swap_remove(index));
            } else {
                index += 1;
            }
        }
    }
    for handle in finished {
        let _ = handle.join();
    }
}

fn acquire_client_slot(inner: &ControlInner) -> bool {
    let mut active = lock_unpoisoned(&inner.active_clients);
    if *active >= MAX_CONNECTIONS {
        return false;
    }
    *active += 1;
    true
}

fn release_client_slot(inner: &ControlInner) {
    let mut active = lock_unpoisoned(&inner.active_clients);
    *active = active.saturating_sub(1);
}

fn read_frame(
    stream: &mut UnixStream,
    output: &mut [u8; MAX_FRAME_BYTES],
    deadline: Instant,
) -> io::Result<usize> {
    let mut chunk = [0u8; 1024];
    let mut used = 0usize;
    loop {
        let read = socket::read_with_deadline(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "control peer closed before frame terminator",
            ));
        }
        let newline = chunk[..read].iter().position(|byte| *byte == b'\n');
        let payload_len = newline.unwrap_or(read);
        if used + payload_len > output.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control frame exceeds limit",
            ));
        }
        output[used..used + payload_len].copy_from_slice(&chunk[..payload_len]);
        used += payload_len;
        if newline.is_some() {
            if used > 0 && output[used - 1] == b'\r' {
                used -= 1;
            }
            return Ok(used);
        }
    }
}

fn reject_duplicate_json_keys(bytes: &[u8]) -> Result<(), String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    UniqueJsonKeys
        .deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let mut written = 0usize;
    while written < bytes.len() {
        stream.set_write_timeout(Some(socket::remaining(deadline)?))?;
        let count = stream.write(&bytes[written..])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "control peer closed while writing",
            ));
        }
        written += count;
    }
    Ok(())
}

fn handle_client(mut stream: UnixStream, inner: Arc<ControlInner>) {
    let deadline = Instant::now() + CLIENT_TIMEOUT;
    let mut frame = [0u8; MAX_FRAME_BYTES];
    let length = match read_frame(&mut stream, &mut frame, deadline) {
        Ok(length) => length,
        Err(_) => return,
    };
    if reject_duplicate_json_keys(&frame[..length]).is_err() {
        let response = ControlResponse::error("unknown", &inner, "invalid control request");
        let _ = write_response(&mut stream, &response, deadline);
        return;
    }
    let frame_kind: FrameKind = match serde_json::from_slice(&frame[..length]) {
        Ok(frame_kind) => frame_kind,
        Err(_) => {
            let response = ControlResponse::error("unknown", &inner, "invalid control request");
            let _ = write_response(&mut stream, &response, deadline);
            return;
        }
    };
    if frame_kind.kind.as_deref() == Some("pack") {
        let envelope: PackRequestEnvelope = match serde_json::from_slice(&frame[..length]) {
            Ok(request) => request,
            Err(error) => {
                let response = pack_error("", format!("invalid pack request: {error}"));
                let _ = write_pack_response(&mut stream, &response, deadline);
                return;
            }
        };
        if envelope.version != PACK_PROTOCOL_VERSION {
            let response = pack_error(
                &envelope.command,
                "unsupported pack protocol version".to_owned(),
            );
            let _ = write_pack_response(&mut stream, &response, deadline);
            return;
        }
        if envelope.kind != "pack" || envelope.command.len() > 64 {
            let response = pack_error(&envelope.command, "pack request is invalid".to_owned());
            let _ = write_pack_response(&mut stream, &response, deadline);
            return;
        }
        let response = dispatch_pack(&inner, envelope);
        let _ = write_pack_response(&mut stream, &response, deadline);
        return;
    }
    if frame_kind.kind.as_deref() == Some("lifecycle") {
        let result = serde_json::from_slice::<LifecycleRequest>(&frame[..length])
            .map_err(|error| format!("invalid lifecycle request: {error}"))
            .and_then(|request| {
                validate_lifecycle_request(&request)?;
                if inner.stopping.load(Ordering::Acquire)
                    || lock_unpoisoned(&inner.shared).scene().shutdown
                {
                    return Err("desktop pet is stopping".to_owned());
                }
                if request.operation == "get" {
                    return Ok(lock_unpoisoned(&inner.shared).lifecycle_settings());
                }
                let _guard = lock_unpoisoned(&inner.lifecycle_writes);
                let key = request
                    .key
                    .ok_or_else(|| "missing lifecycle key".to_owned())?;
                let value = request
                    .value
                    .ok_or_else(|| "missing lifecycle value".to_owned())?;
                lock_unpoisoned(&inner.watchers).update_lifecycle_settings(|| {
                    lifecycle::update_setting(&inner.paths.config_dir, key, value)
                })
            });
        let mut bytes = match serde_json::to_vec(&lifecycle_reply(result)) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        bytes.push(b'\n');
        if bytes.len() <= MAX_FRAME_BYTES {
            let _ = write_frame(&mut stream, &bytes, deadline);
        }
        return;
    }
    let request: LegacyRequest = match serde_json::from_slice(&frame[..length]) {
        Ok(request) => request,
        Err(_) => {
            let response = ControlResponse::error("unknown", &inner, "invalid control request");
            let _ = write_response(&mut stream, &response, deadline);
            return;
        }
    };
    if request.version != Some(PROTOCOL_VERSION) {
        let response =
            ControlResponse::error("unknown", &inner, "unsupported control protocol version");
        let _ = write_response(&mut stream, &response, deadline);
        return;
    }
    let command = match request.command.as_deref() {
        Some(command) if command.len() <= 64 => command,
        _ => {
            let response = ControlResponse::error("unknown", &inner, "control command is invalid");
            let _ = write_response(&mut stream, &response, deadline);
            return;
        }
    };
    let endpoint = request.endpoint.map(PathBuf::from);
    let response = dispatch(&inner, command, endpoint);
    let _ = write_response(&mut stream, &response, deadline);
}

fn pack_error(command: &str, error: String) -> PackReplyEnvelope {
    PackReplyEnvelope {
        version: PACK_PROTOCOL_VERSION,
        kind: "pack".to_owned(),
        command: command.to_owned(),
        ok: false,
        listing: None,
        operation: None,
        error: Some(error),
    }
}

fn validate_source_path(path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err("pack source path must not contain NUL bytes".to_owned());
    }
    if bytes.is_empty() || !path.is_absolute() {
        return Err("pack source path must be a non-empty absolute path".to_owned());
    }
    if bytes.len() > MAX_SOURCE_PATH_BYTES {
        return Err(format!(
            "pack source path must be at most {MAX_SOURCE_PATH_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_pack_request(request: &PackRequest) -> Result<(), String> {
    match &request.action {
        PackAction::Import { path } | PackAction::Update { path, .. } => validate_source_path(path),
        PackAction::Select { .. } | PackAction::Restore { .. } | PackAction::Remove { .. } => {
            Ok(())
        }
    }
}

fn dispatch_pack(inner: &ControlInner, request: PackRequestEnvelope) -> PackReplyEnvelope {
    if inner.stopping.load(Ordering::Acquire) {
        return pack_error(&request.command, "desktop pet is stopping".to_owned());
    }
    match request.command.as_str() {
        "list"
            if request.request.is_none()
                && request.operation_id.is_none()
                && (request.offset.is_some() || request.limit.is_some()) =>
        {
            let offset = request.offset.unwrap_or(0);
            let limit = request.limit.unwrap_or(1);
            if limit == 0 || limit > PACK_LIST_PAGE_SIZE {
                return pack_error("list", "pack list page size is invalid".to_owned());
            }
            match inner.packs.list() {
                Ok(mut listing) => {
                    if offset > listing.packs.len() {
                        return pack_error("list", "pack list offset is out of range".to_owned());
                    }
                    listing.packs = listing.packs.into_iter().skip(offset).take(limit).collect();
                    PackReplyEnvelope {
                        version: PACK_PROTOCOL_VERSION,
                        kind: "pack".to_owned(),
                        command: "list".to_owned(),
                        ok: true,
                        listing: Some(listing),
                        operation: None,
                        error: None,
                    }
                }
                Err(error) => pack_error("list", error),
            }
        }
        "list" if request.request.is_none() && request.operation_id.is_none() => {
            pack_error("list", "pack list pagination is required".to_owned())
        }
        "submit"
            if request.request.is_some()
                && request.operation_id.is_none()
                && request.offset.is_none()
                && request.limit.is_none() =>
        {
            let request = request.request.expect("checked above");
            if let Err(error) = validate_pack_request(&request) {
                return pack_error("submit", error);
            }
            match inner.packs.submit(request) {
                Ok(operation) => PackReplyEnvelope {
                    version: PACK_PROTOCOL_VERSION,
                    kind: "pack".to_owned(),
                    command: "submit".to_owned(),
                    ok: true,
                    listing: None,
                    operation: Some(operation),
                    error: None,
                },
                Err(error) => pack_error("submit", error),
            }
        }
        "status"
            if request.request.is_none()
                && request.operation_id.is_some()
                && request.offset.is_none()
                && request.limit.is_none() =>
        {
            let operation_id = request.operation_id.expect("checked above");
            match inner.packs.status(&operation_id) {
                Ok(operation) => PackReplyEnvelope {
                    version: PACK_PROTOCOL_VERSION,
                    kind: "pack".to_owned(),
                    command: "status".to_owned(),
                    ok: true,
                    listing: None,
                    operation: Some(operation),
                    error: None,
                },
                Err(error) => pack_error("status", error),
            }
        }
        _ => pack_error(&request.command, "invalid pack request fields".to_owned()),
    }
}

fn pack_response_json(response: &PackReplyEnvelope) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pack response exceeds frame limit",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

fn dispatch(inner: &ControlInner, command: &str, endpoint: Option<PathBuf>) -> ControlResponse {
    if inner.stopping.load(Ordering::Acquire) && command != "status" {
        return ControlResponse::error(command, inner, "desktop pet is stopping");
    }
    match command {
        "register" => {
            let Some(endpoint) = endpoint.or_else(|| inner.endpoint.clone()) else {
                return ControlResponse::error(command, inner, "Herdr socket path is unavailable");
            };
            match register_endpoint(inner, endpoint.clone()) {
                Ok(()) => {
                    let mut response = ControlResponse::status(command, inner);
                    if let Ok(endpoint) = socket::canonical_endpoint(&endpoint) {
                        response.registration_accepted =
                            lock_unpoisoned(&inner.registered).contains(&endpoint);
                    }
                    response
                }
                Err(error) => ControlResponse::error(command, inner, error),
            }
        }
        "ensure" => {
            let Some(target) = endpoint.or_else(|| inner.endpoint.clone()) else {
                return ControlResponse::error(command, inner, "Herdr socket path is unavailable");
            };
            if let Err(error) = register_endpoint(inner, target.clone()) {
                return ControlResponse::error(command, inner, error);
            }
            let mut response = ControlResponse::status(command, inner);
            if let Ok(target) = socket::canonical_endpoint(&target) {
                response.registration_accepted =
                    lock_unpoisoned(&inner.registered).contains(&target);
            }
            if response.shutdown && !response.ui_ready {
                response.ok = false;
                response.error = Some("desktop pet stopped before UI became ready".to_owned());
            }
            response
        }
        "ready" => {
            let mut response = ControlResponse::status(command, inner);
            if let Some(target) = endpoint {
                if let Ok(target) = socket::canonical_endpoint(&target) {
                    response.registration_accepted =
                        lock_unpoisoned(&inner.registered).contains(&target);
                }
            }
            if response.shutdown && !response.ui_ready {
                response.ok = false;
                response.error = Some("desktop pet stopped before UI became ready".to_owned());
            }
            response
        }
        "status" | "ping" => ControlResponse::status(command, inner),
        "stop" => {
            {
                let mut state = lock_unpoisoned(&inner.shared);
                state.request_shutdown();
            }
            ui::wake();
            ControlResponse::status(command, inner)
        }
        "shutdown" => {
            {
                let mut state = lock_unpoisoned(&inner.shared);
                state.request_shutdown();
            }
            ui::wake();
            ControlResponse::status(command, inner)
        }
        "show"
        | "hide"
        | "toggle"
        | "passthrough"
        | "toggle_passthrough"
        | "alpha_passthrough"
        | "toggle_alpha_passthrough"
        | "show_bubble"
        | "hide_bubble"
        | "bubble_above"
        | "bubble_below"
        | "bubble_left"
        | "bubble_right"
        | "bubble_auto"
        | "reset"
        | "reset_position"
        | "bigger"
        | "smaller"
        | "scale_up"
        | "scale_down" => {
            let action = match command {
                "passthrough" => "toggle_passthrough",
                "reset" => "reset_position",
                "bigger" => "scale_up",
                "smaller" => "scale_down",
                other => other,
            };
            let result = {
                let mut state = lock_unpoisoned(&inner.shared);
                state.apply_control(action)
            };
            match result {
                Ok(()) => {
                    ui::wake();
                    ControlResponse::status(command, inner)
                }
                Err(error) => ControlResponse::error(command, inner, error),
            }
        }
        _ => ControlResponse::error(
            command,
            inner,
            format!("unsupported control command: {command}"),
        ),
    }
}

fn send_lifecycle_request(
    path: &Path,
    request: LifecycleRequest,
) -> Result<LifecycleSettings, String> {
    let deadline = Instant::now() + CLIENT_TIMEOUT;
    let mut stream = connect_private_socket(path, deadline)?;
    let mut bytes = serde_json::to_vec(&request)
        .map_err(|error| format!("cannot encode lifecycle request: {error}"))?;
    bytes.push(b'\n');
    write_frame(&mut stream, &bytes, deadline)
        .map_err(|error| format!("cannot send lifecycle request: {error}"))?;
    let mut frame = [0; MAX_FRAME_BYTES];
    let length = read_frame(&mut stream, &mut frame, deadline)
        .map_err(|error| format!("cannot read lifecycle response: {error}"))?;
    reject_duplicate_json_keys(&frame[..length])?;
    let reply: LifecycleReply = serde_json::from_slice(&frame[..length])
        .map_err(|error| format!("invalid lifecycle response: {error}"))?;
    if reply.kind != "lifecycle" || reply.version != PROTOCOL_VERSION {
        return Err("invalid lifecycle response protocol".to_owned());
    }
    match (reply.ok, reply.settings, reply.error) {
        (true, Some(settings), None) => Ok(settings),
        (false, None, Some(error)) => Err(error),
        _ => Err("invalid lifecycle response fields".to_owned()),
    }
}

fn lifecycle_request(
    operation: &str,
    key: Option<LifecycleSetting>,
    value: Option<bool>,
) -> LifecycleRequest {
    LifecycleRequest {
        version: PROTOCOL_VERSION,
        kind: "lifecycle".to_owned(),
        operation: operation.to_owned(),
        key,
        value,
    }
}

fn access_lifecycle_settings(
    paths: &Paths,
    change: Option<(LifecycleSetting, bool)>,
) -> Result<LifecycleSettings, String> {
    let deadline = Instant::now() + crate::STARTUP_TIMEOUT;
    loop {
        if control_socket_is_live(&paths.control_socket, Duration::from_millis(150)) {
            let request = match change {
                Some((key, value)) => lifecycle_request("set", Some(key), Some(value)),
                None => lifecycle_request("get", None, None),
            };
            match send_lifecycle_request(&paths.control_socket, request) {
                Ok(settings) => return Ok(settings),
                Err(error)
                    if error == "desktop pet is stopping"
                        || error == "desktop-pet is shutting down" => {}
                Err(_)
                    if !control_socket_is_live(
                        &paths.control_socket,
                        Duration::from_millis(100),
                    ) => {}
                Err(error) => return Err(error),
            }
        } else if let Some(lock) = LifecycleLock::try_acquire(&paths.state_dir)? {
            if !control_socket_is_live(&paths.control_socket, Duration::from_millis(150))
                && !lifecycle::startup_pending(&paths.config_dir)?
            {
                let settings = match change {
                    Some((key, value)) => lifecycle::update_setting(&paths.config_dir, key, value)?,
                    None => lifecycle::read_settings(&paths.config_dir)?,
                };
                drop(lock);
                return Ok(settings);
            }
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for desktop-pet lifecycle finalization".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn get_lifecycle_settings(paths: &Paths) -> Result<LifecycleSettings, String> {
    access_lifecycle_settings(paths, None)
}

pub(crate) fn set_lifecycle_setting(
    paths: &Paths,
    key: LifecycleSetting,
    value: bool,
) -> Result<LifecycleSettings, String> {
    access_lifecycle_settings(paths, Some((key, value)))
}

fn register_endpoint(inner: &ControlInner, endpoint: PathBuf) -> Result<(), String> {
    let canonical_key = socket::canonical_endpoint(&endpoint)?;
    let was_registered = lock_unpoisoned(&inner.registered).contains(&canonical_key);
    // Explicit register/ensure calls must always reach Watchers: a source can
    // have been confirmed disabled since the last successful registration.
    {
        let watchers = lock_unpoisoned(&inner.watchers);
        watchers.register(canonical_key.clone())?;
    }
    if !was_registered {
        let _write_guard = lock_unpoisoned(&inner.lifecycle_writes);
        lifecycle::add_endpoint(&inner.paths.config_dir, &canonical_key)?;
        lock_unpoisoned(&inner.registered).insert(canonical_key);
    }
    Ok(())
}

pub fn send_command(
    socket_path: &Path,
    command: &str,
    endpoint: Option<&Path>,
    timeout: Duration,
) -> Result<ControlResponse, String> {
    if socket_path.as_os_str().is_empty() {
        return Err("control socket path is empty".to_owned());
    }
    let deadline = Instant::now() + timeout;
    let mut stream = connect_private_socket(socket_path, deadline)?;
    let mut request = Map::new();
    request.insert("version".to_owned(), Value::from(PROTOCOL_VERSION));
    request.insert("command".to_owned(), Value::String(command.to_owned()));
    if let Some(endpoint) = endpoint {
        request.insert(
            "endpoint".to_owned(),
            Value::String(endpoint.display().to_string()),
        );
    }
    let mut bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err("control request exceeds frame limit".to_owned());
    }
    bytes.push(b'\n');
    write_frame(&mut stream, &bytes, deadline)
        .map_err(|error| format!("cannot send control request: {error}"))?;
    let mut response = [0u8; MAX_FRAME_BYTES];

    let length = read_frame(&mut stream, &mut response, deadline)
        .map_err(|error| format!("cannot read control response: {error}"))?;
    serde_json::from_slice(&response[..length])
        .map_err(|error| format!("invalid control response: {error}"))
}

fn send_pack_envelope(
    socket_path: &Path,
    envelope: PackRequestEnvelope,
    timeout: Duration,
) -> Result<PackReplyEnvelope, String> {
    if socket_path.as_os_str().is_empty() {
        return Err("control socket path is empty".to_owned());
    }
    if envelope.kind != "pack" {
        return Err("pack request kind is invalid".to_owned());
    }
    if let Some(request) = envelope.request.as_ref() {
        validate_pack_request(request)?;
    }
    let deadline = Instant::now() + timeout;
    let mut stream = connect_private_socket(socket_path, deadline)?;
    let mut bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err("pack request exceeds frame limit".to_owned());
    }
    bytes.push(b'\n');
    write_frame(&mut stream, &bytes, deadline)
        .map_err(|error| format!("cannot send pack request: {error}"))?;
    let mut response = [0u8; MAX_FRAME_BYTES];
    let length = read_frame(&mut stream, &mut response, deadline)
        .map_err(|error| format!("cannot read pack response: {error}"))?;
    let response: PackReplyEnvelope = serde_json::from_slice(&response[..length])
        .map_err(|error| format!("invalid pack response: {error}"))?;
    if response.version != PACK_PROTOCOL_VERSION || response.kind != "pack" {
        return Err("invalid pack response envelope".to_owned());
    }
    Ok(response)
}

fn collect_pack_list_pages<F>(mut request_page: F) -> Result<PackReplyEnvelope, String>
where
    F: FnMut(usize, usize) -> Result<PackReplyEnvelope, String>,
{
    let mut combined: Option<PackReplyEnvelope> = None;
    let mut seen_ids = HashSet::new();
    let mut offset = 0usize;

    for _ in 0..MAX_PACK_LIST_ROUND_TRIPS {
        let reply = request_page(offset, PACK_LIST_PAGE_SIZE)?;
        if !reply.ok {
            return Ok(reply);
        }
        let mut page = reply
            .listing
            .ok_or_else(|| "pack listing is unavailable".to_owned())?;
        if page.packs.len() > PACK_LIST_PAGE_SIZE {
            return Err("pack list page exceeds the page size".to_owned());
        }
        if offset.saturating_add(page.packs.len()) > MAX_PACK_LIST_RECORDS {
            return Err("pack listing exceeds the maximum record count".to_owned());
        }
        for record in &page.packs {
            if !seen_ids.insert(record.id.clone()) {
                return Err("pack listing contains duplicate records".to_owned());
            }
        }

        let page_len = page.packs.len();
        if let Some(current) = combined.as_mut() {
            let current_listing = current
                .listing
                .as_mut()
                .ok_or_else(|| "pack listing is unavailable".to_owned())?;
            if current_listing.generation != page.generation
                || current_listing.selected != page.selected
                || current_listing.active != page.active
                || current_listing.override_active != page.override_active
            {
                return Err("pack listing changed while it was being paged".to_owned());
            }
            current_listing.packs.append(&mut page.packs);
        } else {
            combined = Some(PackReplyEnvelope {
                version: PACK_PROTOCOL_VERSION,
                kind: "pack".to_owned(),
                command: "list".to_owned(),
                ok: true,
                listing: Some(page),
                operation: None,
                error: None,
            });
        }
        offset += page_len;
        if page_len < PACK_LIST_PAGE_SIZE {
            return combined.ok_or_else(|| "pack listing is unavailable".to_owned());
        }
    }
    Err("pack listing exceeded the pagination round-trip limit".to_owned())
}

pub fn send_pack_list(socket_path: &Path, timeout: Duration) -> Result<PackReplyEnvelope, String> {
    collect_pack_list_pages(|offset, limit| {
        send_pack_envelope(
            socket_path,
            PackRequestEnvelope {
                version: PACK_PROTOCOL_VERSION,
                kind: "pack".to_owned(),
                command: "list".to_owned(),
                request: None,
                operation_id: None,
                offset: Some(offset),
                limit: Some(limit),
            },
            timeout,
        )
    })
}

pub fn send_pack_request(
    socket_path: &Path,
    request: PackRequest,
    timeout: Duration,
) -> Result<PackReplyEnvelope, String> {
    send_pack_envelope(
        socket_path,
        PackRequestEnvelope {
            version: PACK_PROTOCOL_VERSION,
            kind: "pack".to_owned(),
            command: "submit".to_owned(),
            request: Some(request),
            operation_id: None,
            offset: None,
            limit: None,
        },
        timeout,
    )
}

pub fn send_pack_status(
    socket_path: &Path,
    operation_id: &str,
    timeout: Duration,
) -> Result<PackReplyEnvelope, String> {
    if operation_id.is_empty() {
        return Err("pack operation ID is empty".to_owned());
    }
    send_pack_envelope(
        socket_path,
        PackRequestEnvelope {
            version: PACK_PROTOCOL_VERSION,
            kind: "pack".to_owned(),
            command: "status".to_owned(),
            request: None,
            operation_id: Some(operation_id.to_owned()),
            offset: None,
            limit: None,
        },
        timeout,
    )
}

pub fn control_socket_is_live(path: &Path, timeout: Duration) -> bool {
    send_command(path, "ping", None, timeout)
        .map(|response| response.ok)
        .unwrap_or(false)
}

fn bind_private_socket(path: &Path) -> Result<UnixListener, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("control socket {} has no parent", path.display()))?;
    lifecycle::validate_directory(parent, true)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "control socket {} must not be a symlink",
                path.display()
            ));
        }
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "control socket {} exists but is not a Unix socket",
                path.display()
            ));
        }
        if metadata.uid() != lifecycle::effective_uid()
            || metadata.permissions().mode() & 0o777 != SOCKET_MODE
        {
            return Err(format!(
                "control socket {} is not private to the current user",
                path.display()
            ));
        }
        match connect_private_socket(path, Instant::now() + Duration::from_millis(150)) {
            Ok(_) => return Err("another desktop-pet daemon is already running".to_owned()),
            Err(error) if is_stale_socket_error(&error) => {
                fs::remove_file(path).map_err(|remove| {
                    format!(
                        "cannot remove stale control socket {}: {remove}",
                        path.display()
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("cannot bind control socket {}: {error}", path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot make control listener nonblocking: {error}"))?;
    fs::set_permissions(path, Permissions::from_mode(SOCKET_MODE))
        .map_err(|error| format!("cannot secure control socket {}: {error}", path.display()))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect control socket {}: {error}", path.display()))?;
    if metadata.uid() != lifecycle::effective_uid()
        || metadata.permissions().mode() & 0o777 != SOCKET_MODE
    {
        return Err(format!(
            "control socket {} is not private to the current user",
            path.display()
        ));
    }
    Ok(listener)
}

fn connect_private_socket(path: &Path, deadline: Instant) -> Result<UnixStream, String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink()
            || !metadata.file_type().is_socket()
            || metadata.uid() != lifecycle::effective_uid()
            || metadata.permissions().mode() & 0o777 != SOCKET_MODE
        {
            return Err(format!(
                "control socket {} is not a private user socket",
                path.display()
            ));
        }
    }
    socket::connect(path, deadline).map_err(|error| {
        format!(
            "cannot connect to control socket {}: {error}",
            path.display()
        )
    })
}

fn is_stale_socket_error(error: &str) -> bool {
    error.contains("Connection refused")
        || error.contains("No such file")
        || error.contains("not found")
}

fn remove_owned_socket(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_socket()
        || metadata.uid() != lifecycle::effective_uid()
    {
        return Ok(());
    }
    fs::remove_file(path).map_err(|error| error.to_string())
}

fn wake_listener(path: &Path) {
    let _ = socket::connect(path, Instant::now() + Duration::from_millis(100));
}

#[cfg(target_os = "macos")]
fn verify_peer_uid(stream: &UnixStream) -> bool {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    unsafe {
        libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) == 0 && uid == libc::geteuid()
    }
}

#[cfg(not(target_os = "macos"))]
fn verify_peer_uid(_stream: &UnixStream) -> bool {
    false
}

fn write_response(
    stream: &mut UnixStream,
    response: &ControlResponse,
    deadline: Instant,
) -> io::Result<()> {
    let bytes = response_json(response)?;
    write_frame(stream, &bytes, deadline)
}

fn write_pack_response(
    stream: &mut UnixStream,
    response: &PackReplyEnvelope,
    deadline: Instant,
) -> io::Result<()> {
    let bytes = pack_response_json(response)?;
    write_frame(stream, &bytes, deadline)
}

fn lock_unpoisoned<'a, T>(mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character_types::{CharacterRef, PackRecord};
    #[test]
    fn lifecycle_frames_reject_duplicate_unknown_and_wrong_typed_mutations() {
        for frame in [
            r#"{"version":1,"kind":"lifecycle","operation":"set","key":"auto_start","value":false,"value":true}"#,
            r#"{"version":1,"kind":"lifecycle","operation":"set","key":"auto_start","value":false,"unexpected":0}"#,
            r#"{"version":1,"kind":"lifecycle","operation":"set","key":"auto_start","value":"off"}"#,
            r#"{"version":1,"kind":"lifecycle","operation":"set","key":"enabled","value":false}"#,
            r#"{"version":1,"kind":"lifecycle","operation":"set","key":"auto_start"}"#,
            r#"{"version":1,"kind":"lifecycle","operation":"get","key":"auto_start"}"#,
            r#"{"version":2,"kind":"lifecycle","operation":"get"}"#,
        ] {
            let parsed = reject_duplicate_json_keys(frame.as_bytes())
                .and_then(|()| {
                    serde_json::from_str::<LifecycleRequest>(frame)
                        .map_err(|error| error.to_string())
                })
                .and_then(|request| validate_lifecycle_request(&request));
            assert!(parsed.is_err(), "{frame}");
        }
    }

    fn list_reply(packs: Vec<PackRecord>) -> PackReplyEnvelope {
        PackReplyEnvelope {
            version: PACK_PROTOCOL_VERSION,
            kind: "pack".to_owned(),
            command: "list".to_owned(),
            ok: true,
            listing: Some(PackListing {
                generation: 1,
                selected: CharacterRef::builtin(),
                active: None,
                override_active: false,
                packs,
                error: None,
            }),
            operation: None,
            error: None,
        }
    }

    fn list_record(index: usize) -> PackRecord {
        PackRecord {
            id: format!("pack-{index}"),
            name: format!("Pack {index}"),
            head: 1,
            revisions: vec![1],
        }
    }

    #[test]
    fn pack_envelope_rejects_duplicate_wire_fields() {
        let error = serde_json::from_slice::<PackRequestEnvelope>(
            br#"{"version":1,"kind":"pack","command":"list","request":null,"operation_id":null,"offset":0,"limit":1,"limit":1}"#,
        );
        assert!(error.is_err());
    }

    #[test]
    fn raw_frames_reject_duplicate_keys_before_dispatch() {
        assert!(
            reject_duplicate_json_keys(br#"{"version":1,"command":"show","command":"hide"}"#)
                .is_err()
        );
        assert!(reject_duplicate_json_keys(
            br#"{"version":1,"kind":"pack","command":"list","limit":1,"limit":2}"#
        )
        .is_err());
        assert!(reject_duplicate_json_keys(
            br#"{"version":1,"command":"show","endpoint":"/tmp/herdr.sock"}"#
        )
        .is_ok());
    }
    #[test]
    fn ipc_pack_source_paths_are_bounded() {
        let nul = PackRequest {
            operation_id: "op".to_owned(),
            expected_generation: None,
            action: PackAction::Import {
                path: PathBuf::from("/tmp/pack\0source"),
            },
        };
        assert!(validate_pack_request(&nul).is_err());

        let relative = PackRequest {
            operation_id: "op".to_owned(),
            expected_generation: None,
            action: PackAction::Import {
                path: PathBuf::from("relative/source"),
            },
        };
        assert!(validate_pack_request(&relative).is_err());

        let empty = PackRequest {
            operation_id: "op".to_owned(),
            expected_generation: None,
            action: PackAction::Import {
                path: PathBuf::new(),
            },
        };
        assert!(validate_pack_request(&empty).is_err());

        let oversized = PackRequest {
            operation_id: "op".to_owned(),
            expected_generation: None,
            action: PackAction::Update {
                id: "pack".to_owned(),
                path: PathBuf::from(format!("/{}", "x".repeat(MAX_SOURCE_PATH_BYTES))),
            },
        };
        assert!(validate_pack_request(&oversized).is_err());
    }

    #[test]
    fn pack_list_accepts_maximum_records_with_bounded_pages() {
        let records: Vec<_> = (0..MAX_PACK_LIST_RECORDS).map(list_record).collect();
        let mut offsets = Vec::new();
        let reply = collect_pack_list_pages(|offset, limit| {
            offsets.push(offset);
            let end = (offset + limit).min(records.len());
            Ok(list_reply(records[offset..end].to_vec()))
        })
        .expect("maximum valid listing");
        assert_eq!(offsets, vec![0, 8, 16, 24, 32]);
        assert_eq!(
            reply.listing.expect("listing").packs.len(),
            MAX_PACK_LIST_RECORDS
        );
    }

    #[test]
    fn pack_list_rejects_duplicate_and_overflow_pages() {
        let duplicate = collect_pack_list_pages(|offset, limit| {
            if offset == 0 {
                Ok(list_reply((0..limit).map(list_record).collect()))
            } else {
                Ok(list_reply(vec![list_record(0)]))
            }
        });
        assert!(duplicate.is_err());

        let overflow = collect_pack_list_pages(|offset, limit| {
            Ok(list_reply(
                (offset..offset + limit).map(list_record).collect(),
            ))
        });
        assert!(overflow.is_err());
    }

    #[test]
    fn pack_envelope_rejects_unknown_fields() {
        assert!(serde_json::from_str::<PackRequestEnvelope>(
            r#"{"version":1,"kind":"pack","command":"list","request":null,"operation_id":null,"offset":0,"limit":1,"extra":true}"#,
        )
        .is_err());
    }

    #[test]
    fn oversized_pack_reply_is_rejected_before_writing() {
        let listing = PackListing {
            generation: 1,
            selected: CharacterRef::builtin(),
            active: None,
            override_active: false,
            packs: (0..32)
                .map(|index| PackRecord {
                    id: format!("pack-{index}"),
                    name: "quote\\\"".repeat(128),
                    head: u64::MAX,
                    revisions: vec![u64::MAX; 8],
                })
                .collect(),
            error: None,
        };
        let response = PackReplyEnvelope {
            version: PACK_PROTOCOL_VERSION,
            kind: "pack".to_owned(),
            command: "list".to_owned(),
            ok: true,
            listing: Some(listing),
            operation: None,
            error: None,
        };
        assert!(pack_response_json(&response).is_err());
    }
}
