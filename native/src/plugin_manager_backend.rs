//! Explicit, on-demand manager operations. The host owns mutations and durable results;
//! this module owns admission, authenticating the selected server and observing journals.
use crate::{bundle, lifecycle::Paths, preferences::Preferences};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{CString, OsStr};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const FRAME_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 128 * 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const PATCH_REQUIRED: &str = "Herdr plugin-manager host patch is missing or incompatible (requires manager safety protocol 1); no stock-CLI mutation fallback";
const MANAGER_DIR: &str = "plugin-manager";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Enable,
    Disable,
    Unlink,
    Uninstall,
    Update,
}
impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Unlink => "unlink",
            Self::Uninstall => "uninstall",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub root: PathBuf,
    pub source: Value,
    pub warnings: Vec<String>,
    pub raw: Value,
    pub registration: Option<Value>,
}
impl Plugin {
    pub fn protected(&self) -> bool {
        self.id == crate::lifecycle::PLUGIN_ID
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub socket: PathBuf,
    pub host: PathBuf,
    socket_dev: u64,
    socket_ino: u64,
    host_dev: u64,
    host_ino: u64,
    peer_pid: u32,
    protected_roots: Vec<PathBuf>,
}
impl Target {
    pub fn from_selected_socket(socket: PathBuf) -> Result<Self, String> {
        if !socket.is_absolute() {
            return Err("selected socket must be absolute".into());
        }
        let (socket_dev, socket_ino) = socket_identity(&socket)?;
        let stream = connect_bounded(&socket)?;
        if socket_identity(&socket)? != (socket_dev, socket_ino) {
            return Err("selected socket changed during authentication".into());
        }
        let (peer_pid, host) = peer_identity(&stream)?;
        let host_metadata = fs::metadata(&host).map_err(|e| e.to_string())?;
        if !host_metadata.is_file() {
            return Err("host peer executable must be a regular file".into());
        }
        // Never accept a caller-supplied Pet root. This protects the current image even
        // when an old Herdr server omitted its registration from plugin.list.
        let image =
            bundle::executable().map_err(|e| format!("cannot identify Pet executable: {e}"))?;
        let root = if let Some(contents) = bundle::contents_dir() {
            contents
                .parent()
                .ok_or("Pet bundle has no root")?
                .to_path_buf()
        } else {
            image
                .parent()
                .ok_or("Pet executable has no parent")?
                .to_path_buf()
        };
        Ok(Self {
            socket,
            host,
            peer_pid,
            socket_dev,
            socket_ino,
            host_dev: host_metadata.dev(),
            host_ino: host_metadata.ino(),
            protected_roots: vec![canonical_existing(&root)?, image],
        })
    }

    pub fn list(&self) -> Result<Vec<Plugin>, String> {
        let legacy = || parse_inventory(&socket_request(self, "plugin.list")?.0);
        let (context, pid) = match socket_request(self, "plugin.manager.context") {
            Ok(context) => context,
            Err(error)
                if matches!(
                    host_api_error_code(&error),
                    Some("invalid_request" | "method_not_found")
                ) =>
            {
                return legacy();
            }
            Err(error) => return Err(error),
        };
        if context.get("type").and_then(Value::as_str) != Some("plugin_manager_context")
            || context.get("protocol").and_then(Value::as_u64) != Some(2)
        {
            return Err("invalid manager inventory context".into());
        }
        if context
            .get("manager_safety_protocol")
            .and_then(Value::as_u64)
            != Some(1)
        {
            return legacy();
        }
        validate_context(&context)?;
        if pid != self.peer_pid || context.get("pid").and_then(Value::as_u64) != Some(pid as u64) {
            return Err("host inventory context PID differs from authenticated peer".into());
        }
        parse_manager_inventory(&socket_request(self, "plugin.manager.inventory")?.0)
    }
    pub fn protection_reason(&self, plugin: &Plugin, inventory: &[Plugin]) -> Option<String> {
        match protected_roots(inventory, &self.protected_roots) {
            Ok(roots) => reject_protected(plugin, &roots).err(),
            Err(e) => Some(format!("cannot establish protected Pet paths: {e}")),
        }
    }
    pub fn mutation_support(&self) -> Result<(), String> {
        self.bound_context().map(|_| ())
    }
    fn ensure_host_image(&self) -> Result<(), String> {
        let meta = fs::metadata(&self.host)
            .map_err(|e| format!("cannot inspect selected host executable: {e}"))?;
        if !meta.is_file() || (meta.dev(), meta.ino()) != (self.host_dev, self.host_ino) {
            return Err("selected host executable was replaced; refresh before retrying".into());
        }
        Ok(())
    }

    fn bound_context(&self) -> Result<Value, String> {
        let (context, pid) = socket_request(self, "plugin.manager.context")
            .map_err(|e| format!("{PATCH_REQUIRED}: {e}"))?;
        validate_context(&context)?;
        if pid != self.peer_pid || context.get("pid").and_then(Value::as_u64) != Some(pid as u64) {
            return Err("host context PID differs from the authenticated selected peer".into());
        }
        self.ensure_host_image()?;
        let caps = host_command(
            &self.host,
            &["plugin", "manager", "capabilities", "--json"],
            None,
        )?;
        validate_capabilities(&caps)?;
        let socket = absolute_string(&self.socket)?;
        self.ensure_host_image()?;
        let cli = host_command(
            &self.host,
            &[
                "plugin", "manager", "context", "--socket", &socket, "--json",
            ],
            Some(&context),
        )?;
        validate_cli_context(&context, &cli, &socket, &self.host)?;
        Ok(context)
    }

    fn prepare(
        &self,
        action: Action,
        expected: &Plugin,
    ) -> Result<(Value, Vec<PathBuf>, Value, Value), String> {
        let inventory = self.list()?;
        let mut matches = inventory.iter().filter(|item| item.id == expected.id);
        let selected = matches
            .next()
            .ok_or("plugin changed since the displayed snapshot; refresh before retrying")?;
        if matches.next().is_some()
            || selected.raw != expected.raw
            || selected.registration != expected.registration
        {
            return Err(
                "plugin changed since the displayed snapshot; refresh before retrying".into(),
            );
        }
        let registration = selected.registration.as_ref().ok_or(PATCH_REQUIRED)?;
        let roots = protected_roots(&inventory, &self.protected_roots)?;
        reject_protected(selected, &roots)?;
        eligible(action, selected)?;
        let context = self.bound_context()?;
        let current = self.list()?;
        if current
            .iter()
            .filter(|p| {
                p.id == selected.id
                    && p.raw == selected.raw
                    && p.registration.as_ref() == Some(registration)
            })
            .count()
            != 1
        {
            return Err("plugin changed before mutation; refresh before retrying".into());
        }
        let current_roots = protected_roots(&current, &self.protected_roots)?;
        reject_protected(selected, &current_roots)?;
        if roots != current_roots {
            return Err(
                "protected Pet paths changed before mutation; refresh before retrying".into(),
            );
        }
        let (latest, pid) = socket_request(self, "plugin.manager.context")?;
        validate_context(&latest)?;
        if pid != self.peer_pid || latest != context {
            return Err("host context changed before mutation; refresh before retrying".into());
        }
        Ok((context, roots, selected.raw.clone(), registration.clone()))
    }
}

fn eligible(action: Action, plugin: &Plugin) -> Result<(), String> {
    let kind = plugin.source.get("kind").and_then(Value::as_str);
    match action {
        Action::Enable | Action::Disable => Ok(()),
        Action::Unlink if kind == Some("local") => Ok(()),
        Action::Update | Action::Uninstall
            if kind == Some("github")
                && plugin
                    .source
                    .get("managed_path")
                    .and_then(Value::as_str)
                    .is_some_and(|s| Path::new(s).is_absolute()) =>
        {
            Ok(())
        }
        _ => Err(format!(
            "{} is not available for this plugin source",
            action.as_str()
        )),
    }
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("invalid response: missing {key}"))
}
fn parse_inventory(value: &Value) -> Result<Vec<Plugin>, String> {
    if value.get("type").and_then(Value::as_str) != Some("plugin_list") {
        return Err("invalid response: expected plugin_list".into());
    }
    let entries = value
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or("invalid plugin_list entries")?;
    parse_entries(entries, false)
}
fn parse_manager_inventory(value: &Value) -> Result<Vec<Plugin>, String> {
    if value.get("type").and_then(Value::as_str) != Some("plugin_manager_inventory") {
        return Err("invalid response: expected plugin_manager_inventory".into());
    }
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("invalid manager inventory entries")?;
    parse_entries(entries, true)
}
fn parse_entries(entries: &[Value], manager: bool) -> Result<Vec<Plugin>, String> {
    if entries.len() > 4096 {
        return Err("plugin inventory exceeds limit".into());
    }
    let mut seen = HashSet::new();
    entries
        .iter()
        .map(|entry| {
            let raw = if manager {
                entry.get("display").ok_or("missing displayed plugin")?
            } else {
                entry
            };
            let id = field(raw, "plugin_id")?.to_owned();
            if !seen.insert(id.clone()) {
                return Err("duplicate plugin ID in inventory".into());
            }
            let root = PathBuf::from(field(raw, "plugin_root")?);
            if !root.is_absolute() {
                return Err("plugin_root is not absolute".into());
            }
            let manifest = field(raw, "manifest_path")?;
            if !Path::new(manifest).is_absolute() {
                return Err("plugin manifest_path is not absolute".into());
            }
            let source = raw.get("source").cloned().unwrap_or(Value::Null);
            validate_managed_path(&source)?;
            let enabled = raw
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("invalid plugin enabled flag")?;
            let registration = if manager {
                let registered = entry
                    .get("registration")
                    .ok_or("missing raw registration")?;
                if field(registered, "plugin_id")? != id
                    || field(registered, "plugin_root")? != root.to_str().ok_or("invalid root")?
                    || field(registered, "manifest_path")? != manifest
                    || registered.get("source") != raw.get("source")
                    || registered.get("enabled").and_then(Value::as_bool) != Some(enabled)
                {
                    return Err("display and original registration disagree".into());
                }
                validate_managed_path(registered.get("source").unwrap_or(&Value::Null))?;
                Some(registered.clone())
            } else {
                None
            };
            let warnings = match raw.get("warnings") {
                None => Vec::new(),
                Some(v) => v
                    .as_array()
                    .ok_or("invalid plugin warnings")?
                    .iter()
                    .map(|s| {
                        s.as_str()
                            .map(str::to_owned)
                            .ok_or("invalid plugin warning".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            Ok(Plugin {
                id,
                name: field(raw, "name")?.to_owned(),
                version: field(raw, "version")?.to_owned(),
                enabled,
                root,
                source,
                warnings,
                raw: raw.clone(),
                registration,
            })
        })
        .collect()
}
fn validate_managed_path(source: &Value) -> Result<(), String> {
    if let Some(path) = source.get("managed_path").filter(|v| !v.is_null()) {
        if !path.as_str().is_some_and(|s| Path::new(s).is_absolute()) {
            return Err("plugin source.managed_path is not absolute".into());
        }
    }
    Ok(())
}

fn canonical_existing(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("unsafe relative path: {}", path.display()));
    }
    let mut missing = Vec::new();
    let mut prefix = path;
    while !prefix.exists() {
        if fs::symlink_metadata(prefix).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("unresolvable path symlink: {}", prefix.display()));
        }
        let leaf = prefix
            .file_name()
            .ok_or_else(|| format!("unresolvable path: {}", path.display()))?;
        missing.push(leaf.to_os_string());
        prefix = prefix.parent().ok_or("path has no existing ancestor")?;
    }
    let mut resolved = fs::canonicalize(prefix)
        .map_err(|e| format!("cannot resolve {}: {e}", prefix.display()))?;
    for leaf in missing.iter().rev() {
        resolved.push(leaf);
    }
    let mut normalized = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn protected_roots(inventory: &[Plugin], extra: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut roots = extra.to_vec();
    for plugin in inventory.iter().filter(|p| p.protected()) {
        for snapshot in std::iter::once(&plugin.raw).chain(plugin.registration.iter()) {
            roots.push(canonical_existing(Path::new(field(
                snapshot,
                "plugin_root",
            )?))?);
            roots.push(canonical_existing(Path::new(field(
                snapshot,
                "manifest_path",
            )?))?);
            if let Some(path) = snapshot
                .get("source")
                .and_then(|s| s.get("managed_path"))
                .and_then(Value::as_str)
            {
                roots.push(canonical_existing(Path::new(path))?);
            }
        }
        let bundle = plugin.root.join("dist/HerdrDesktopPet.app");
        if bundle.exists() {
            roots.push(canonical_existing(&bundle)?);
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}
fn reject_protected(plugin: &Plugin, roots: &[PathBuf]) -> Result<(), String> {
    if plugin.protected() {
        return Err("desktop-pet is read-only in Plugin Manager".into());
    }
    let mut paths = Vec::new();
    for snapshot in std::iter::once(&plugin.raw).chain(plugin.registration.iter()) {
        paths.push(PathBuf::from(field(snapshot, "plugin_root")?));
        paths.push(PathBuf::from(field(snapshot, "manifest_path")?));
        if let Some(path) = snapshot
            .get("source")
            .and_then(|s| s.get("managed_path"))
            .and_then(Value::as_str)
        {
            paths.push(PathBuf::from(path));
        }
    }
    for path in paths {
        let resolved = canonical_existing(&path)?;
        if roots.iter().any(|root| overlap(&resolved, root)) {
            return Err(format!(
                "plugin path {} overlaps protected desktop-pet files",
                path.display()
            ));
        }
    }
    Ok(())
}
fn validate_context(value: &Value) -> Result<(), String> {
    if value.get("type").and_then(Value::as_str) != Some("plugin_manager_context")
        || value.get("protocol").and_then(Value::as_u64) != Some(2)
        || value.get("manager_safety_protocol").and_then(Value::as_u64) != Some(1)
    {
        return Err(PATCH_REQUIRED.into());
    }
    for key in ["config_dir", "state_dir"] {
        canonical_existing(Path::new(field(value, key)?))?;
    }
    value
        .get("pid")
        .and_then(Value::as_u64)
        .filter(|pid| *pid > 0)
        .ok_or("invalid server PID")?;
    Ok(())
}
fn validate_capabilities(value: &Value) -> Result<(), String> {
    if value.get("product_id").and_then(Value::as_str) != Some("herdr-plugin-manager-host")
        || value.get("protocol").and_then(Value::as_u64) != Some(2)
        || value.get("durable_result_protocol").and_then(Value::as_u64) != Some(1)
        || value.get("admission_fd_protocol").and_then(Value::as_u64) != Some(1)
        || value.get("manager_safety_protocol").and_then(Value::as_u64) != Some(1)
        || !["enable", "disable", "unlink", "uninstall", "update"]
            .iter()
            .all(|action| {
                value
                    .get("actions")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(action)))
            })
    {
        return Err(PATCH_REQUIRED.into());
    }
    Ok(())
}
fn validate_cli_context(
    server: &Value,
    cli: &Value,
    socket: &str,
    host: &Path,
) -> Result<(), String> {
    if cli.get("protocol").and_then(Value::as_u64) != Some(2)
        || cli.get("manager_safety_protocol").and_then(Value::as_u64) != Some(1)
        || field(cli, "socket")? != socket
        || canonical_existing(Path::new(field(cli, "host")?))? != host
        || cli.get("pid") != server.get("pid")
        || ["config_dir", "state_dir"]
            .iter()
            .any(|key| field(server, key).ok() != field(cli, key).ok())
    {
        return Err("host context disagrees with selected live socket; refusing mutation".into());
    }
    Ok(())
}
fn absolute_string(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("expected absolute path".into());
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or("path is not valid UTF-8".into())
}

fn poll_until(
    fd: RawFd,
    events: libc::c_short,
    deadline: Instant,
    operation: &str,
) -> Result<(), String> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("{operation} timed out"));
        }
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut pollfd,
                1,
                remaining.as_millis().min(i32::MAX as u128).max(1) as i32,
            )
        };
        if result > 0 {
            return Ok(());
        }
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(format!(
                "{operation} poll failed: {}",
                io::Error::last_os_error()
            ));
        }
    }
}
fn connect_bounded(path: &Path) -> Result<UnixStream, String> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(format!(
            "invalid or excessively long Unix socket path: {}",
            path.display()
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dst, byte) in address.sun_path.iter_mut().zip(bytes) {
        *dst = *byte as libc::c_char;
    }
    let len = (std::mem::size_of::<libc::sockaddr_un>() - address.sun_path.len() + bytes.len() + 1)
        as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = len as u8;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(format!(
            "cannot create Unix socket: {}",
            io::Error::last_os_error()
        ));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(format!(
            "cannot make socket nonblocking: {}",
            io::Error::last_os_error()
        ));
    }
    let deadline = Instant::now() + SOCKET_TIMEOUT;
    let status = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            len,
        )
    };
    if status < 0 {
        if io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(format!(
                "cannot connect to selected socket {}: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
        poll_until(fd.as_raw_fd(), libc::POLLOUT, deadline, "socket connection")?;
        let mut error: libc::c_int = 0;
        let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut size,
            )
        } < 0
            || size as usize != std::mem::size_of_val(&error)
            || error != 0
        {
            return Err(format!(
                "cannot connect to selected socket {}: {}",
                path.display(),
                io::Error::from_raw_os_error(error)
            ));
        }
    }
    Ok(UnixStream::from(fd))
}
fn socket_identity(path: &Path) -> Result<(u64, u64), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("cannot inspect selected socket: {e}"))?;
    if !metadata.file_type().is_socket() || metadata.uid() != crate::lifecycle::effective_uid() {
        return Err("selected socket is not an owned Unix socket".into());
    }
    Ok((metadata.dev(), metadata.ino()))
}
fn socket_request(target: &Target, method: &str) -> Result<(Value, u32), String> {
    if socket_identity(&target.socket)? != (target.socket_dev, target.socket_ino) {
        return Err("selected socket was replaced; refresh before retrying".into());
    }
    let mut stream = connect_bounded(&target.socket)?;
    if socket_identity(&target.socket)? != (target.socket_dev, target.socket_ino) {
        return Err("selected socket changed during connection".into());
    }
    let (pid, executable) = peer_identity(&stream)?;
    if pid != target.peer_pid || executable != target.host {
        return Err("selected host peer changed; refresh before retrying".into());
    }
    let id = format!("plugin-manager-{}", std::process::id());
    let mut bytes = serde_json::to_vec(&json!({"id":id,"method":method,"params":{}}))
        .map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let deadline = Instant::now() + SOCKET_TIMEOUT;
    let mut pending = bytes.as_slice();
    while !pending.is_empty() {
        match stream.write(pending) {
            Ok(0) => return Err(format!("socket closed while sending {method}")),
            Ok(n) => pending = &pending[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => poll_until(
                stream.as_raw_fd(),
                libc::POLLOUT,
                deadline,
                "socket request",
            )?,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("socket request failed: {e}")),
        }
        if Instant::now() >= deadline {
            return Err("socket request timed out".into());
        }
    }
    let mut data = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return Err("socket closed before response".into()),
            Ok(n) => {
                data.extend_from_slice(&chunk[..n]);
                if data.len() > FRAME_LIMIT {
                    return Err("socket frame exceeds size limit".into());
                }
                if let Some(end) = data.iter().position(|b| *b == b'\n') {
                    data.truncate(end);
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => poll_until(
                stream.as_raw_fd(),
                libc::POLLIN,
                deadline,
                "socket response",
            )?,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("socket response read failed: {e}")),
        }
        if Instant::now() >= deadline {
            return Err("socket response timed out".into());
        }
    }
    if socket_identity(&target.socket)? != (target.socket_dev, target.socket_ino) {
        return Err("selected socket changed while answering".into());
    }
    let envelope: Value =
        serde_json::from_slice(&data).map_err(|e| format!("invalid host response: {e}"))?;
    if envelope.get("id").and_then(Value::as_str) != Some(id.as_str()) {
        return Err("socket response ID mismatch".into());
    }
    if let Some(error) = envelope.get("error") {
        return Err(format!(
            "host API {}: {}",
            error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
        ));
    }
    let result = envelope
        .get("result")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or("missing host result")?;
    Ok((result, pid))
}
fn host_api_error_code(error: &str) -> Option<&str> {
    error
        .strip_prefix("host API ")?
        .split_once(": ")
        .map(|(code, _)| code)
}
#[cfg(target_os = "macos")]
fn peer_identity(stream: &UnixStream) -> Result<(u32, PathBuf), String> {
    #[link(name = "proc")]
    extern "C" {
        fn proc_pidpath(pid: libc::c_int, buffer: *mut libc::c_void, size: u32) -> libc::c_int;
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    } != 0
        || len as usize != std::mem::size_of_val(&pid)
        || pid <= 0
    {
        return Err("cannot identify socket peer PID".into());
    }
    let (mut uid, mut gid) = (0, 0);
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || uid != unsafe { libc::geteuid() }
    {
        return Err("selected socket peer is not the current user".into());
    }
    let mut buffer = [0; 4096];
    let n = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if n <= 0 {
        return Err("cannot identify live socket peer executable".into());
    }
    let end = buffer[..n as usize]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(n as usize);
    let host = canonical_existing(Path::new(OsStr::from_bytes(&buffer[..end])))?;
    Ok((pid as u32, host))
}
#[cfg(target_os = "linux")]
fn peer_identity(stream: &UnixStream) -> Result<(u32, PathBuf), String> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&cred)
        || cred.pid <= 0
        || cred.uid != unsafe { libc::geteuid() }
    {
        return Err("cannot identify same-user selected socket peer".into());
    }
    let host = fs::canonicalize(format!("/proc/{}/exe", cred.pid))
        .map_err(|e| format!("cannot identify socket peer executable: {e}"))?;
    Ok((cred.pid as u32, host))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_identity(_: &UnixStream) -> Result<(u32, PathBuf), String> {
    Err("socket peer identity unavailable".into())
}

fn verified_xdg_parent(context: &Value, key: &str) -> Result<PathBuf, String> {
    let root = Path::new(field(context, key)?);
    if !root.is_absolute()
        || !matches!(
            root.file_name().and_then(|s| s.to_str()),
            Some("herdr" | "herdr-dev")
        )
    {
        return Err(format!("selected host {key} is not a standard Herdr root"));
    }
    canonical_existing(root.parent().ok_or("host root has no parent")?)
}
fn host_command(host: &Path, args: &[&str], context: Option<&Value>) -> Result<Value, String> {
    let mut command = Command::new(host);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(context) = context {
        command
            .env(
                "XDG_CONFIG_HOME",
                verified_xdg_parent(context, "config_dir")?,
            )
            .env("XDG_STATE_HOME", verified_xdg_parent(context, "state_dir")?);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{PATCH_REQUIRED}: cannot launch selected host: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("missing stdout pipe")?;
    let mut stderr = child.stderr.take().ok_or("missing stderr pipe")?;
    set_nonblocking(stdout.as_raw_fd())?;
    set_nonblocking(stderr.as_raw_fd())?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut pipe_deadline = deadline;
    let (mut output, mut errors) = (Vec::new(), Vec::new());
    let (mut output_truncated, mut errors_truncated) = (false, false);
    let (mut stdout_done, mut stderr_done) = (false, false);
    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) => {
                    status = Some(exit);
                    pipe_deadline = pipe_deadline.min(Instant::now() + Duration::from_millis(250));
                }
                Ok(None) => {}
                Err(e) => return Err(format!("cannot wait for host probe: {e}")),
            }
        }
        if status.is_some() && stdout_done && stderr_done {
            break;
        }
        let remaining = pipe_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err("host probe or output timed out".into());
        }
        let mut pipes = [
            libc::pollfd {
                fd: if stdout_done { -1 } else { stdout.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if stderr_done { -1 } else { stderr.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe {
            libc::poll(
                pipes.as_mut_ptr(),
                2,
                remaining.as_millis().min(20).max(1) as i32,
            )
        };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!(
                "cannot read host probe: {}",
                io::Error::last_os_error()
            ));
        }
        if pipes[0].revents != 0 {
            stdout_done = read_ready(&mut stdout, &mut output, &mut output_truncated)?;
        }
        if pipes[1].revents != 0 {
            stderr_done = read_ready(&mut stderr, &mut errors, &mut errors_truncated)?;
        }
    }
    if output_truncated || errors_truncated {
        return Err("host probe exceeds output limit".into());
    }
    if !status.expect("status set").success() {
        return Err(format!(
            "{PATCH_REQUIRED}: host probe failed: {}",
            String::from_utf8_lossy(&errors)
        ));
    }
    serde_json::from_slice(&output).map_err(|e| format!("invalid host probe JSON: {e}"))
}
fn set_nonblocking(fd: RawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!(
            "cannot configure probe pipe: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}
fn read_ready(
    pipe: &mut impl Read,
    output: &mut Vec<u8>,
    truncated: &mut bool,
) -> Result<bool, String> {
    let mut chunk = [0; 4096];
    match pipe.read(&mut chunk) {
        Ok(0) => Ok(true),
        Ok(n) => {
            let free = OUTPUT_LIMIT - output.len();
            output.extend_from_slice(&chunk[..n.min(free)]);
            if n > free {
                *truncated = true;
            }
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(format!("cannot read host probe: {e}")),
    }
}

// Every manager path is reached through validated private directory descriptors.
// In particular, a symlink/hardlink cannot redirect the admission or lifetime
// lock to an unrelated file and turn a released lock into a false observation.
fn name(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "invalid manager path component".into())
}
fn owned_directory(file: &File) -> Result<(), String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_dir()
        || meta.uid() != crate::lifecycle::effective_uid()
        || meta.permissions().mode() & 0o077 != 0
    {
        return Err("manager profile directory must be private and owned by this user".into());
    }
    Ok(())
}
fn validate_profile(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("manager profile must be absolute".into());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve manager profile {}: {e}", path.display()))?;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("manager profile must not be a symlink".into());
    }
    let file = File::open(&canonical).map_err(|e| e.to_string())?;
    owned_directory(&file)?;
    if !meta.is_dir()
        || meta.ino() != file.metadata().map_err(|e| e.to_string())?.ino()
        || meta.dev() != file.metadata().map_err(|e| e.to_string())?.dev()
    {
        return Err("manager profile changed while opening it".into());
    }
    Ok(canonical)
}
fn directory_at(parent: &File, component: &str, create: bool) -> Result<File, String> {
    let component = name(component)?;
    if create {
        let result = unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(format!(
                "cannot create private manager directory: {}",
                io::Error::last_os_error()
            ));
        }
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            component.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot open private manager directory: {}",
            io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    owned_directory(&file)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let mut at_meta: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            component.as_ptr(),
            &mut at_meta,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
        || meta.ino() != at_meta.st_ino as u64
        || meta.dev() != at_meta.st_dev as u64
    {
        return Err("manager directory changed while opening it".into());
    }
    Ok(file)
}
fn regular_at(parent: &File, component: &str, create: bool) -> Result<Option<File>, String> {
    let component = name(component)?;
    let flags =
        libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | if create { libc::O_CREAT } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags, 0o600) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        if !create && error.kind() == io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("cannot open private manager file: {error}"));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let mut at_meta: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            component.as_ptr(),
            &mut at_meta,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        return Err(format!(
            "cannot inspect manager file: {}",
            io::Error::last_os_error()
        ));
    }
    if !meta.is_file()
        || meta.uid() != crate::lifecycle::effective_uid()
        || meta.nlink() != 1
        || meta.permissions().mode() & 0o077 != 0
        || meta.ino() != at_meta.st_ino as u64
        || meta.dev() != at_meta.st_dev as u64
    {
        return Err("manager file is not private, singly linked, and stable".into());
    }
    Ok(Some(file))
}
fn create_regular(parent: &File, component: &str) -> Result<File, String> {
    let component = name(component)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            component.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot create private manager file: {}",
            io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != crate::lifecycle::effective_uid()
        || meta.permissions().mode() & 0o077 != 0
    {
        return Err("new manager file is not private".into());
    }
    Ok(file)
}
fn lock(file: &File, nonblocking: bool) -> Result<bool, String> {
    let flags = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
    if unsafe { libc::flock(file.as_raw_fd(), flags) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if nonblocking
        && matches!(error.raw_os_error(), Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK)
    {
        return Ok(false);
    }
    Err(format!("cannot inspect or acquire manager lock: {error}"))
}
fn read_small(file: File, limit: usize) -> Result<Value, String> {
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read manager record: {e}"))?;
    if bytes.len() > limit {
        return Err("manager record exceeds size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid manager record: {e}"))
}
fn record(parent: &File, file: &str, limit: usize) -> Result<Option<Value>, String> {
    regular_at(parent, file, false)?
        .map(|file| read_small(file, limit))
        .transpose()
}
fn sync_json(file: &mut File, value: &Value) -> Result<(), String> {
    sync_json_with_limit(file, value, OUTPUT_LIMIT)
}
fn sync_json_with_limit(file: &mut File, value: &Value, limit: usize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("manager record exceeds size limit".into());
    }
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot persist manager record: {e}"))
}
fn publish(parent: &File, filename: &str, value: &Value) -> Result<(), String> {
    let mut random = [0; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| format!("cannot generate record name: {e}"))?;
    let temporary = format!(".{:032x}.tmp", u128::from_be_bytes(random));
    let mut file = create_regular(parent, &temporary)?;
    let outcome = (|| {
        sync_json(&mut file, value)?;
        // Never replace a path whose ownership or type has changed.
        regular_at(parent, filename, false)?;
        let from = name(&temporary)?;
        let to = name(filename)?;
        if unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                from.as_ptr(),
                parent.as_raw_fd(),
                to.as_ptr(),
            )
        } < 0
        {
            return Err(format!(
                "cannot publish manager record: {}",
                io::Error::last_os_error()
            ));
        }
        parent
            .sync_all()
            .map_err(|e| format!("cannot sync manager journal: {e}"))
    })();
    if outcome.is_err() {
        let _ = unsafe { libc::unlinkat(parent.as_raw_fd(), name(&temporary)?.as_ptr(), 0) };
    }
    outcome
}
fn publish_submission<T>(
    config: &File,
    manager: &File,
    operations: &File,
    dir: &File,
    id: &str,
    mut sync: impl FnMut(&File) -> Result<(), String>,
    start: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    for ancestor in [dir, operations, manager, config] {
        sync(ancestor).map_err(|e| format!("cannot sync request journal ancestor: {e}"))?;
    }
    publish(manager, "active.json", &json!({"operation_id":id}))?;
    start()
}

fn operation_id() -> Result<String, String> {
    let mut random = [0; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| format!("cannot generate operation ID: {e}"))?;
    use std::fmt::Write as _;
    let mut id = String::with_capacity(64);
    for byte in random {
        write!(&mut id, "{byte:02x}").map_err(|e| e.to_string())?;
    }
    Ok(id)
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileOperationStore {
    config: PathBuf,
    state: PathBuf,
    config_identity: (u64, u64),
    state_identity: (u64, u64),
}
#[derive(Debug)]
pub(crate) struct AvailabilityAdmission {
    _file: File,
}
impl ProfileOperationStore {
    pub fn new(paths: &Paths) -> Result<Self, String> {
        let config = std::path::absolute(&paths.config_dir).map_err(|error| error.to_string())?;
        let config = validate_profile(&config)?;
        let state = validate_profile(&paths.state_dir)?;
        let config_meta = fs::symlink_metadata(&config).map_err(|e| e.to_string())?;
        let state_meta = fs::symlink_metadata(&state).map_err(|e| e.to_string())?;
        let store = Self {
            config,
            state,
            config_identity: (config_meta.dev(), config_meta.ino()),
            state_identity: (state_meta.dev(), state_meta.ino()),
        };
        let gate = store.gate()?;
        store.ensure_pairing(&gate)?;
        Ok(store)
    }
    fn manager(&self) -> Result<File, String> {
        self.manager_with_parent().map(|(_, manager)| manager)
    }
    fn manager_with_parent(&self) -> Result<(File, File), String> {
        let config = File::open(&self.config).map_err(|e| e.to_string())?;
        owned_directory(&config)?;
        let meta = config.metadata().map_err(|e| e.to_string())?;
        let path_meta = fs::symlink_metadata(&self.config).map_err(|e| e.to_string())?;
        if !path_meta.is_dir()
            || (meta.dev(), meta.ino()) != self.config_identity
            || (path_meta.dev(), path_meta.ino()) != self.config_identity
        {
            return Err("manager config profile changed while in use".into());
        }
        let manager = directory_at(&config, MANAGER_DIR, true)?;
        Ok((config, manager))
    }
    fn gate(&self) -> Result<AvailabilityAdmission, String> {
        let manager = self.manager()?;
        let file = regular_at(&manager, "admission.lock", true)?.ok_or("missing admission lock")?;
        if !lock(&file, true)? {
            return Err("plugin manager profile admission is busy".into());
        }
        Ok(AvailabilityAdmission { _file: file })
    }
    fn ensure_pairing(&self, _gate: &AvailabilityAdmission) -> Result<(), String> {
        let state = validate_profile(&self.state)?;
        let meta = fs::symlink_metadata(&state).map_err(|e| e.to_string())?;
        if state != self.state || (meta.dev(), meta.ino()) != self.state_identity {
            return Err("manager state profile changed while in use".into());
        }
        let manager = self.manager()?;
        match record(&manager, "profile.json", 4096)? {
            Some(pairing)
                if pairing.get("config").and_then(Value::as_str) == self.config.to_str()
                    && pairing.get("state").and_then(Value::as_str) == self.state.to_str() =>
            {
                Ok(())
            }
            Some(_) => Err("plugin manager config/state profile pairing changed".into()),
            None => publish(
                &manager,
                "profile.json",
                &json!({"config":self.config,"state":self.state}),
            ),
        }
    }
    fn operations(&self) -> Result<File, String> {
        directory_at(&self.manager()?, "operations", true)
    }
    fn active_id(&self) -> Result<Option<String>, String> {
        let Some(value) = record(&self.manager()?, "active.json", 4096)? else {
            return Ok(None);
        };
        let id = field(&value, "operation_id")?;
        if !valid_id(id) {
            return Err("unsafe active operation ID".into());
        }
        Ok(Some(id.to_owned()))
    }
    fn observation(&self) -> Result<OperationObservation, String> {
        let Some(id) = self.active_id()? else {
            return Ok(OperationObservation::Idle);
        };
        self.observation_id(id)
    }
    fn observation_id(&self, id: String) -> Result<OperationObservation, String> {
        if !valid_id(&id) {
            return Err("invalid operation ID".into());
        }
        let operations = self.operations()?;
        let dir = directory_at(&operations, &id, false)?;
        let journal = self.config.join(MANAGER_DIR).join("operations").join(&id);
        let request =
            record(&dir, "request.json", FRAME_LIMIT)?.ok_or("active operation request missing")?;
        if request.get("protocol").and_then(Value::as_u64) != Some(2)
            || request.get("operation_id").and_then(Value::as_str) != Some(id.as_str())
        {
            return Err("active operation request identity mismatch".into());
        }
        let operation_lock =
            regular_at(&dir, "operation.lock", false)?.ok_or("active operation lock missing")?;
        if !lock(&operation_lock, true)? {
            return Ok(OperationObservation::Running {
                operation_id: id,
                journal,
            });
        }
        let result = regular_at(&dir, "result.json", false)?;
        match result {
            Some(file) => match read_small(file, OUTPUT_LIMIT) {
                Ok(result) if valid_result(&request, &result) => {
                    dir.sync_all().map_err(|e| format!("unknown operation outcome: durable result directory could not be inspected/synced: {e}"))?;
                    if result.get("status").and_then(Value::as_str) == Some("unknown") {
                        self.unknown(
                            &dir,
                            id,
                            journal,
                            result
                                .get("detail")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                    } else {
                        Ok(OperationObservation::Terminal {
                            operation_id: id,
                            journal,
                            result,
                        })
                    }
                }
                Ok(_) => self.unknown(
                    &dir,
                    id,
                    journal,
                    "invalid or mismatched durable host result",
                ),
                Err(e) if e.starts_with("invalid manager record") => {
                    self.unknown(&dir, id, journal, "malformed durable host result")
                }
                Err(e) => Err(e),
            },
            None => self.unknown(
                &dir,
                id,
                journal,
                "host lock released without durable result",
            ),
        }
    }
    fn unknown(
        &self,
        dir: &File,
        id: String,
        journal: PathBuf,
        detail: &str,
    ) -> Result<OperationObservation, String> {
        let acknowledged = match record(dir, "acknowledged.json", 4096)? {
            None => false,
            Some(value)
                if value.get("operation_id").and_then(Value::as_str) == Some(id.as_str())
                    && value.get("acknowledged").and_then(Value::as_bool) == Some(true) =>
            {
                true
            }
            Some(_) => return Err("unknown-outcome acknowledgement is invalid".into()),
        };
        Ok(OperationObservation::Unknown {
            operation_id: id,
            journal,
            detail: detail.into(),
            acknowledged,
        })
    }
    pub fn observe_active(&self) -> Result<OperationObservation, String> {
        let gate = self.gate()?;
        self.ensure_pairing(&gate)?;
        self.observation()
    }
    fn observe_operation(&self, id: &str) -> Result<OperationObservation, String> {
        let gate = self.gate()?;
        self.ensure_pairing(&gate)?;
        self.observation_id(id.to_owned())
    }
    pub fn acknowledge_unknown(&self, operation_id: &str) -> Result<(), String> {
        if !valid_id(operation_id) {
            return Err("invalid operation ID".into());
        }
        let gate = self.gate()?;
        self.ensure_pairing(&gate)?;
        match self.observation()? {
            OperationObservation::Unknown {
                operation_id: id, ..
            } if id == operation_id => {
                let operations = self.operations()?;
                let dir = directory_at(&operations, operation_id, false)?;
                publish(
                    &dir,
                    "acknowledged.json",
                    &json!({"operation_id": operation_id, "acknowledged":true}),
                )
            }
            _ => Err("only the exact securely ended unknown operation can be acknowledged".into()),
        }
    }
    pub fn submit(
        &self,
        target: &Target,
        action: Action,
        expected: &Plugin,
    ) -> Result<OperationHandle, String> {
        let gate = self.gate()?;
        self.ensure_pairing(&gate)?;
        if !Preferences::plugin_manager_installed_at(&self.config)? {
            return Err("Plugin Manager is not installed in this profile".into());
        }
        match self.observation()? {
            OperationObservation::Running { .. } => {
                return Err("a plugin operation is still running".into())
            }
            OperationObservation::Unknown {
                acknowledged: false,
                ..
            } => {
                return Err(
                    "review and acknowledge the unknown operation before starting another".into(),
                )
            }
            _ => {}
        }
        // No probes while holding a gate: a slow host must not block Remove.
        drop(gate);
        let (context, roots, raw, registration) = target.prepare(action, expected)?;
        let (now, pid) = socket_request(target, "plugin.manager.context")?;
        validate_context(&now)?;
        if now != context || pid != target.peer_pid {
            return Err("selected host context changed before submission".into());
        }
        let gate = self.gate()?;
        self.ensure_pairing(&gate)?;
        if !Preferences::plugin_manager_installed_at(&self.config)? {
            return Err("Plugin Manager was removed before submission".into());
        }
        match self.observation()? {
            OperationObservation::Running { .. } => {
                return Err("a plugin operation is already running".into())
            }
            OperationObservation::Unknown {
                acknowledged: false,
                ..
            } => {
                return Err(
                    "review and acknowledge the unknown operation before starting another".into(),
                )
            }
            _ => {}
        }
        if socket_identity(&target.socket)? != (target.socket_dev, target.socket_ino) {
            return Err("selected socket changed before mutation handoff".into());
        }
        target.ensure_host_image()?;
        let id = operation_id()?;
        let (config, manager) = self.manager_with_parent()?;
        let operations = directory_at(&manager, "operations", true)?;
        let dir = directory_at(&operations, &id, true)?;
        let lock_file = create_regular(&dir, "operation.lock")?;
        if !lock(&lock_file, true)? {
            return Err("new operation lock unexpectedly busy".into());
        }
        let journal = self.config.join(MANAGER_DIR).join("operations").join(&id);
        let socket = absolute_string(&target.socket)?;
        let request = json!({
            "protocol":2, "operation_id":id, "action":action.as_str(), "socket":socket, "host":absolute_string(&target.host)?,
            "context":{"config_dir": canonical_existing(Path::new(field(&context, "config_dir")?))?,
                       "state_dir": canonical_existing(Path::new(field(&context, "state_dir")?))?, "pid":pid},
            "expected":raw,
            "expected_registration":registration,
            "protected_roots":roots.iter().map(|p| absolute_string(p)).collect::<Result<Vec<_>,_>>()?,
        });
        let mut request_file = create_regular(&dir, "request.json")?;
        sync_json_with_limit(&mut request_file, &request, FRAME_LIMIT)?;
        // Each link needs a durability barrier even when its directory already existed.
        // Failed earlier submissions can leave unsynced directory entries behind.
        if socket_identity(&target.socket)? != (target.socket_dev, target.socket_ino) {
            return Err("selected socket changed before mutation handoff".into());
        }
        target.ensure_host_image()?;
        // The durable locator is published only after every ancestor has synced.
        let request_path = journal.join("request.json");
        let inherited = lock_file.as_raw_fd();
        let admission_fd = inherited.to_string();
        let mut command = Command::new(&target.host);
        command
            .args([
                "plugin",
                "manager",
                "apply",
                "--request",
                &absolute_string(&request_path)?,
                "--admission-fd",
                &admission_fd,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env(
                "XDG_CONFIG_HOME",
                verified_xdg_parent(&context, "config_dir")?,
            )
            .env(
                "XDG_STATE_HOME",
                verified_xdg_parent(&context, "state_dir")?,
            );
        // The inherited descriptor cannot collide with the spawn error pipe: it
        // remains open while std allocates that pipe. The host sets CLOEXEC on
        // this same locked OFD immediately upon adoption.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(inherited, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = publish_submission(
            &config,
            &manager,
            &operations,
            &dir,
            &id,
            |file| file.sync_all().map_err(|e| e.to_string()),
            || {
                command.spawn().map_err(|e| {
                    format!(
                        "host apply did not start; outcome retained for review ({}): {e}",
                        journal.display()
                    )
                })
            },
        )?;
        drop(lock_file);
        // Reaping is scoped to this operation; no idle worker/timer and no kill on timeout.
        std::thread::Builder::new().name("plugin-manager-reap".into()).spawn(move || { let mut child = child; let _ = child.wait(); })
            .map_err(|e| format!("host apply started but reaper failed; operation outcome retained for review: {e}"))?;
        Ok(OperationHandle {
            id,
            journal,
            store: self.clone(),
        })
    }
}
pub(crate) fn availability_admission(
    paths: &Paths,
    installed: bool,
) -> Result<AvailabilityAdmission, String> {
    let store = ProfileOperationStore::new(paths)?;
    let gate = store.gate()?;
    store.ensure_pairing(&gate)?;
    if !installed && matches!(store.observation()?, OperationObservation::Running { .. }) {
        return Err("cannot remove Plugin Manager while a plugin operation is running".into());
    }
    Ok(gate)
}
fn valid_result(request: &Value, result: &Value) -> bool {
    result.get("protocol").and_then(Value::as_u64) == Some(2)
        && result.get("operation_id") == request.get("operation_id")
        && result.get("action") == request.get("action")
        && result.get("plugin_id") == request.get("expected").and_then(|v| v.get("plugin_id"))
        && matches!(
            result.get("status").and_then(Value::as_str),
            Some("completed" | "partial" | "failed" | "unknown")
        )
        && result.get("detail").and_then(Value::as_str).is_some()
}
#[derive(Clone, Debug)]
pub(crate) struct OperationHandle {
    pub id: String,
    pub journal: PathBuf,
    store: ProfileOperationStore,
}
impl OperationHandle {
    pub fn wait(&self, timeout: Duration) -> Result<OperationObservation, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let observation = self.store.observe_operation(&self.id)?;
            if !matches!(&observation, OperationObservation::Running { operation_id, .. } if operation_id == &self.id)
            {
                return match &observation {
                    OperationObservation::Terminal { operation_id, .. }
                    | OperationObservation::Unknown { operation_id, .. }
                        if operation_id == &self.id =>
                    {
                        Ok(observation)
                    }
                    _ => Err("operation was superseded; inspect its durable journal".into()),
                };
            }
            if Instant::now() >= deadline {
                return Ok(observation);
            }
            std::thread::sleep(
                Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) enum OperationObservation {
    Idle,
    Running {
        operation_id: String,
        journal: PathBuf,
    },
    Terminal {
        operation_id: String,
        journal: PathBuf,
        result: Value,
    },
    Unknown {
        operation_id: String,
        journal: PathBuf,
        detail: String,
        acknowledged: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(15);

    fn accept_bounded(listener: &UnixListener) -> UnixStream {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture socket accept timed out");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("fixture socket accept failed: {error}"),
            }
        }
    }
    fn read_fixture_line(stream: &mut UnixStream) -> Option<String> {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        let mut bytes = Vec::new();
        loop {
            poll_until(
                stream.as_raw_fd(),
                libc::POLLIN,
                deadline,
                "fixture socket read",
            )
            .unwrap();
            let mut byte = [0];
            match stream.read(&mut byte) {
                Ok(0) if bytes.is_empty() => return None,
                Ok(0) => panic!("fixture socket closed before newline"),
                Ok(_) => {
                    bytes.push(byte[0]);
                    assert!(
                        bytes.len() <= FRAME_LIMIT,
                        "fixture request exceeds frame limit"
                    );
                    if byte[0] == b'\n' {
                        return Some(String::from_utf8(bytes).unwrap());
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => panic!("fixture socket read failed: {error}"),
            }
        }
    }
    struct Fixture {
        dir: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("pm-native-{}-{n}", std::process::id()));
            fs::create_dir(&dir).unwrap();
            Self { dir }
        }
        fn path(&self, name: &str) -> PathBuf {
            self.dir.join(name)
        }
        fn socket(&self, inventory: Value, requests: usize) -> (Target, thread::JoinHandle<()>) {
            self.socket_at(self.path("host.sock"), inventory, requests)
        }
        fn socket_at(
            &self,
            path: PathBuf,
            inventory: Value,
            requests: usize,
        ) -> (Target, thread::JoinHandle<()>) {
            self.socket_responses_at(path, vec![inventory; requests])
        }
        fn socket_responses_at(
            &self,
            path: PathBuf,
            responses: Vec<Value>,
        ) -> (Target, thread::JoinHandle<()>) {
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let context_root = path.parent().unwrap().to_path_buf();
            let server = thread::spawn(move || {
                // The constructor authenticates this connection without sending a frame.
                // Keep the accepted peer alive until credential discovery finishes.
                let mut auth = accept_bounded(&listener);
                assert!(
                    read_fixture_line(&mut auth).is_none(),
                    "authentication connection sent a request"
                );
                drop(auth);
                let safety_context = json!({"type":"plugin_manager_context","protocol":2,
                    "manager_safety_protocol":1,"pid":std::process::id(),
                    "config_dir":context_root,"state_dir":context_root});
                for inventory in responses {
                    let mut stream = accept_bounded(&listener);
                    let line = read_fixture_line(&mut stream)
                        .expect("request connection closed without a frame");
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let response = if request["method"] == "plugin.manager.inventory"
                        && inventory["type"] == "plugin_manager_inventory"
                    {
                        json!({"id": request["id"], "result":inventory})
                    } else if request["method"] == "plugin.manager.context"
                        && inventory["type"] == "plugin_manager_inventory"
                    {
                        json!({"id":request["id"],"result":safety_context})
                    } else if request["method"] == "plugin.manager.context"
                        && inventory["type"] == "plugin_list"
                    {
                        json!({"id":request["id"],"error":{"code":"invalid_request",
                            "message":"invalid request: unknown variant `plugin.manager.context`"}})
                    } else if request["method"] == "plugin.manager.context"
                        && inventory["type"] == "plugin_manager_context"
                    {
                        json!({"id": request["id"], "result":inventory})
                    } else if request["method"] == "plugin.list"
                        && inventory["type"] == "plugin_list"
                    {
                        json!({"id": request["id"], "result":inventory})
                    } else {
                        json!({"id":request["id"],"error":{"code":"method_not_found","message":"plugin-manager patch not installed"}})
                    };
                    writeln!(stream, "{response}").unwrap();
                }
            });
            (Target::from_selected_socket(path).unwrap(), server)
        }
        fn paths(&self) -> Paths {
            let config_dir = self.path("profile");
            let state_dir = self.path("state");
            fs::create_dir(&config_dir).unwrap();
            fs::create_dir(&state_dir).unwrap();
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700)).unwrap();
            Paths {
                control_socket: state_dir.join("control.sock"),
                log_file: state_dir.join("pet.log"),
                config_dir,
                state_dir,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    fn plugin(id: &str, root: &Path, managed: Option<&Path>) -> Value {
        let source = match managed {
            Some(path) => {
                json!({"kind":"github","owner":"example","repo":"plugin","requested_ref":"v1","managed_path":path})
            }
            None => json!({"kind":"local"}),
        };
        json!({"plugin_id":id,"name":id,"version":"1.0.0","enabled":true,
            "plugin_root":root,"manifest_path":root.join("herdr-plugin.toml"),"source":source,"warnings":[]})
    }
    fn inventory(plugins: Vec<Value>) -> Value {
        json!({"type":"plugin_manager_inventory","entries":plugins.into_iter()
            .map(|plugin| json!({"display":plugin,"registration":plugin})).collect::<Vec<_>>()})
    }
    fn legacy_inventory(plugins: Vec<Value>) -> Value {
        json!({"type":"plugin_list","plugins":plugins})
    }
    fn source_rejected(target: &Target, action: Action, expected: &Plugin) {
        assert!(eligible(action, expected).is_err());
        assert!(target.prepare(action, expected).is_err());
    }
    #[test]
    fn pet_plugin_id_is_read_only_without_host_probe() {
        let fixture = Fixture::new();
        let pet = fixture.path("pet");
        fs::create_dir(&pet).unwrap();
        let (target, server) =
            fixture.socket(inventory(vec![plugin("desktop-pet", &pet, None)]), 2);
        let listed = target.list().unwrap();
        let selected = &listed[0];
        assert!(selected.protected());
        assert!(target.protection_reason(selected, &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn managed_pet_plugin_id_is_read_only() {
        let fixture = Fixture::new();
        let pet = fixture.path("pet");
        let checkout = fixture.path("checkout");
        fs::create_dir(&pet).unwrap();
        fs::create_dir(&checkout).unwrap();
        let (target, server) = fixture.socket(
            inventory(vec![plugin("desktop-pet", &pet, Some(&checkout))]),
            2,
        );
        let listed = target.list().unwrap();
        assert!(listed[0].protected());
        assert!(target.protection_reason(&listed[0], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn symlink_alias_to_registered_pet_root_is_rejected() {
        let fixture = Fixture::new();
        let pet = fixture.path("pet");
        fs::create_dir(&pet).unwrap();
        symlink(&pet, fixture.path("alias")).unwrap();
        let other = fixture.path("alias/subplugin");
        let (target, server) = fixture.socket(
            inventory(vec![
                plugin("desktop-pet", &pet, None),
                plugin("foreign", &other, None),
            ]),
            2,
        );
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[1], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn managed_pet_path_and_ancestor_alias_are_protected() {
        let fixture = Fixture::new();
        let pet = fixture.path("pet");
        let managed = fixture.path("managed");
        fs::create_dir(&pet).unwrap();
        fs::create_dir(&managed).unwrap();
        symlink(&fixture.dir, fixture.path("alias")).unwrap();
        let (target, server) = fixture.socket(
            inventory(vec![
                plugin("desktop-pet", &pet, Some(&managed)),
                plugin("foreign", &fixture.path("alias"), None),
            ]),
            2,
        );
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[1], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn foreign_managed_path_aliasing_pet_managed_path_is_read_only() {
        let fixture = Fixture::new();
        let pet = fixture.path("pet");
        let managed = fixture.path("pet-managed");
        let foreign = fixture.path("foreign");
        for dir in [&pet, &managed, &foreign] {
            fs::create_dir(dir).unwrap();
        }
        symlink(&managed, fixture.path("managed-alias")).unwrap();
        let (target, server) = fixture.socket(
            inventory(vec![
                plugin("desktop-pet", &pet, Some(&managed)),
                plugin(
                    "foreign",
                    &foreign,
                    Some(&fixture.path("managed-alias/data")),
                ),
            ]),
            2,
        );
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[1], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn managed_pet_subdirectory_protects_entire_shared_checkout() {
        let fixture = Fixture::new();
        let checkout = fixture.path("checkout");
        let pet = checkout.join("plugins/desktop-pet");
        let other = checkout.join("plugins/foreign");
        fs::create_dir_all(&pet).unwrap();
        fs::create_dir_all(&other).unwrap();
        let (target, server) = fixture.socket(
            inventory(vec![
                plugin("desktop-pet", &pet, Some(&checkout)),
                plugin("foreign", &other, None),
            ]),
            2,
        );
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[1], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn relative_source_managed_path_is_rejected_in_inventory() {
        let fixture = Fixture::new();
        let mut entry = plugin("foreign", &fixture.path("foreign"), None);
        entry["source"]["managed_path"] = json!("relative/checkout");
        let (target, server) = fixture.socket(inventory(vec![entry]), 2);
        assert!(target.list().is_err());
        server.join().unwrap();
    }
    #[test]
    fn current_physical_pet_root_is_protected_without_server_registration() {
        let fixture = Fixture::new();
        let image = bundle::executable().unwrap();
        let (target, server) = fixture.socket(inventory(vec![plugin("foreign", &image, None)]), 2);
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[0], &listed).is_some());
        server.join().unwrap();
    }
    #[test]
    fn stock_host_without_manager_context_rejects_mutation_support() {
        let fixture = Fixture::new();
        let (target, server) = fixture.socket(
            legacy_inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]),
            3,
        );
        let listed = target.list().unwrap();
        assert!(target.protection_reason(&listed[0], &listed).is_none());
        assert!(eligible(Action::Disable, &listed[0]).is_ok());
        assert!(target.mutation_support().is_err());
        server.join().unwrap();
    }
    #[test]
    fn legacy_inventory_remains_read_only_even_for_a_supported_action() {
        let fixture = Fixture::new();
        let (target, server) = fixture.socket(
            legacy_inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]),
            4,
        );
        let selected = target.list().unwrap().remove(0);
        assert!(selected.registration.is_none());
        assert!(eligible(Action::Disable, &selected).is_ok());
        assert!(target.prepare(Action::Disable, &selected).is_err());
        server.join().unwrap();
    }
    #[test]
    fn either_snapshot_becoming_stale_prevents_handoff() {
        for changed_registration in [false, true] {
            let fixture = Fixture::new();
            let original = inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]);
            let mut stale = original.clone();
            if changed_registration {
                stale["entries"][0]["registration"]["version"] = json!("2.0.0");
            } else {
                stale["entries"][0]["display"]["version"] = json!("2.0.0");
            }
            let (target, server) = fixture.socket_responses_at(
                fixture.path("host.sock"),
                vec![original.clone(), original, stale.clone(), stale],
            );
            let selected = target.list().unwrap().remove(0);
            assert!(selected.registration.is_some());
            assert!(target.prepare(Action::Disable, &selected).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn inconsistent_registration_coordinates_are_not_mutable() {
        let fixture = Fixture::new();
        let mut entries = inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]);
        entries["entries"][0]["registration"]["manifest_path"] = json!(fixture.path("other.toml"));
        let (target, server) = fixture.socket(entries, 2);
        assert!(target.list().is_err());
        server.join().unwrap();
    }
    #[test]
    fn safety_one_malformed_manager_inventory_never_downgrades_to_plugin_list() {
        let fixture = Fixture::new();
        let malformed = json!({"type":"plugin_manager_inventory","entries":"invalid"});
        let (target, server) = fixture.socket(malformed, 2);
        assert!(target.list().is_err());
        server.join().unwrap();
    }
    #[test]
    fn old_weak_context_does_not_authorize_mutation_of_readable_inventory() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let weak_context = json!({"type":"plugin_manager_context","protocol":2,
            "pid":std::process::id(),"config_dir":paths.config_dir,"state_dir":paths.state_dir});
        let (target, server) = fixture.socket_responses_at(
            fixture.path("host.sock"),
            vec![
                weak_context.clone(),
                legacy_inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]),
                weak_context,
            ],
        );
        let selected = target.list().unwrap().remove(0);
        assert!(selected.registration.is_none());
        assert!(target.mutation_support().is_err());
        server.join().unwrap();
    }
    #[test]
    fn socket_at_unix_path_limit_accepts_frame_before_peer_close() {
        let fixture = Fixture::new();
        let capacity = unsafe { std::mem::zeroed::<libc::sockaddr_un>() }
            .sun_path
            .len();
        let base = fixture.dir.as_os_str().as_bytes().len();
        let path = fixture.path(&"s".repeat(capacity - base - 2));
        assert_eq!(path.as_os_str().as_bytes().len(), capacity - 1);
        let (target, server) = fixture.socket_at(path, inventory(vec![]), 2);
        assert!(target.list().unwrap().is_empty());
        server.join().unwrap();
    }
    fn active_without_result(store: &ProfileOperationStore) -> (String, File) {
        let id = operation_id().unwrap();
        let ops = store.operations().unwrap();
        let dir = directory_at(&ops, &id, true).unwrap();
        let lifetime = create_regular(&dir, "operation.lock").unwrap();
        assert!(lock(&lifetime, true).unwrap());
        let mut request = create_regular(&dir, "request.json").unwrap();
        sync_json(
            &mut request,
            &json!({"protocol":2,"operation_id":id,"action":"unlink",
            "expected":{"plugin_id":"foreign"},"expected_registration":{"plugin_id":"foreign"}}),
        )
        .unwrap();
        dir.sync_all().unwrap();
        publish(
            &store.manager().unwrap(),
            "active.json",
            &json!({"operation_id":id}),
        )
        .unwrap();
        (id, lifetime)
    }
    #[test]
    fn relative_config_and_absolute_ancestor_alias_share_admission_and_lifetime_exclusion() {
        let root = PathBuf::from(format!("manager-relative-{}", crate::new_operation_id()));
        let config = root.join("physical/config");
        let state = root.join("state");
        for directory in [&config, &state] {
            fs::create_dir_all(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let paths = Paths {
            config_dir: config.clone(),
            state_dir: fs::canonicalize(&state).unwrap(),
            control_socket: state.join(crate::lifecycle::CONTROL_SOCKET_FILE),
            log_file: state.join(crate::lifecycle::LOG_FILE),
        };
        let relative = ProfileOperationStore::new(&paths).unwrap();
        let absolute = Paths {
            config_dir: fs::canonicalize(&config).unwrap(),
            ..paths.clone()
        };
        symlink(
            fs::canonicalize(root.join("physical")).unwrap(),
            root.join("alias"),
        )
        .unwrap();
        let alias = Paths {
            config_dir: std::path::absolute(root.join("alias/config")).unwrap(),
            ..paths.clone()
        };
        assert_eq!(relative.config, absolute.config_dir);
        let admission = relative.gate().unwrap();
        assert!(ProfileOperationStore::new(&absolute).is_err());
        assert!(ProfileOperationStore::new(&alias).is_err());
        drop(admission);
        let alias_store = ProfileOperationStore::new(&alias).unwrap();
        assert_eq!(alias_store.config_identity, relative.config_identity);
        let (id, lifetime) = active_without_result(&relative);
        assert!(matches!(alias_store.observe_active().unwrap(),
            OperationObservation::Running { operation_id, .. } if operation_id == id));
        assert!(availability_admission(&absolute, false).is_err());
        assert!(availability_admission(&paths, false).is_err());
        drop(lifetime);
        assert!(matches!(relative.observe_active().unwrap(),
            OperationObservation::Unknown { operation_id, .. } if operation_id == id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relative_config_normalization_keeps_state_absolute_and_leaf_symlinks_forbidden() {
        let root = PathBuf::from(format!(
            "manager-relative-reject-{}",
            crate::new_operation_id()
        ));
        let config = root.join("config");
        let state = root.join("state");
        for directory in [&config, &state] {
            fs::create_dir_all(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut paths = Paths {
            config_dir: config.clone(),
            state_dir: state.clone(),
            control_socket: state.join(crate::lifecycle::CONTROL_SOCKET_FILE),
            log_file: state.join(crate::lifecycle::LOG_FILE),
        };
        assert!(ProfileOperationStore::new(&paths).is_err());
        assert!(!config.join(MANAGER_DIR).exists());
        paths.state_dir = fs::canonicalize(&state).unwrap();
        symlink(fs::canonicalize(&config).unwrap(), root.join("config-link")).unwrap();
        paths.config_dir = root.join("config-link");
        assert!(ProfileOperationStore::new(&paths).is_err());
        assert!(!config.join(MANAGER_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn independent_stores_cannot_remove_while_lifetime_lock_is_held() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        assert!(
            matches!(ProfileOperationStore::new(&paths).unwrap().observe_active().unwrap(),
            OperationObservation::Running { operation_id, .. } if operation_id == id)
        );
        assert!(availability_admission(&paths, false).is_err());
        drop(lifetime);
        assert!(matches!(
            store.observe_active().unwrap(),
            OperationObservation::Unknown {
                acknowledged: false,
                ..
            }
        ));
        store.acknowledge_unknown(&id).unwrap();
        assert!(matches!(
            ProfileOperationStore::new(&paths)
                .unwrap()
                .observe_active()
                .unwrap(),
            OperationObservation::Unknown {
                acknowledged: true,
                ..
            }
        ));
    }
    #[test]
    fn uninspectable_lock_fails_closed_and_cannot_be_acknowledged() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        drop(lifetime);
        let path = store
            .config
            .join(MANAGER_DIR)
            .join("operations")
            .join(&id)
            .join("operation.lock");
        fs::remove_file(&path).unwrap();
        symlink("/dev/null", &path).unwrap();
        assert!(store.observe_active().is_err());
        assert!(availability_admission(&paths, false).is_err());
        assert!(store.acknowledge_unknown(&id).is_err());
    }
    #[test]
    fn mismatched_host_result_is_unknown_not_terminal() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        publish(
            &dir,
            "result.json",
            &json!({"protocol":2,"operation_id":"wrong","action":"unlink",
            "plugin_id":"foreign","status":"completed","detail":"forged"}),
        )
        .unwrap();
        drop(lifetime);
        assert!(matches!(
            store.observe_active().unwrap(),
            OperationObservation::Unknown {
                acknowledged: false,
                ..
            }
        ));
    }
    #[test]
    fn replacing_selected_socket_is_rejected_even_if_same_process_rebinds() {
        let fixture = Fixture::new();
        let (target, server) = fixture.socket(inventory(vec![]), 2);
        assert!(target.list().unwrap().is_empty());
        server.join().unwrap();
        fs::remove_file(&target.socket).unwrap();
        let _replacement = UnixListener::bind(&target.socket).unwrap();
        assert!(target.list().is_err());
    }
    #[test]
    fn filesystem_mutations_remain_source_specific_before_probes() {
        let fixture = Fixture::new();
        let local = plugin("local", &fixture.path("local"), None);
        let checkout = fixture.path("checkout");
        let managed = plugin("managed", &checkout, Some(&checkout));
        let mut github_without_checkout = plugin("unmanaged", &fixture.path("unmanaged"), None);
        github_without_checkout["source"] = json!({"kind":"github"});
        let mut no_source = plugin("no-source", &fixture.path("no-source"), None);
        no_source.as_object_mut().unwrap().remove("source");
        let (target, server) = fixture.socket(
            inventory(vec![local, managed, github_without_checkout, no_source]),
            18,
        );
        let plugins = target.list().unwrap();
        for action in [Action::Update, Action::Uninstall] {
            source_rejected(&target, action, &plugins[0]);
            source_rejected(&target, action, &plugins[2]);
            source_rejected(&target, action, &plugins[3]);
        }
        source_rejected(&target, Action::Unlink, &plugins[1]);
        source_rejected(&target, Action::Unlink, &plugins[3]);
        server.join().unwrap();
    }
    #[test]
    fn managed_and_no_source_plugins_reach_host_for_toggles() {
        let fixture = Fixture::new();
        let checkout = fixture.path("checkout");
        let managed = plugin("managed", &checkout, Some(&checkout));
        let mut no_source = plugin("no-source", &fixture.path("no-source"), None);
        no_source.as_object_mut().unwrap().remove("source");
        let (target, server) = fixture.socket(inventory(vec![managed, no_source]), 14);
        let plugins = target.list().unwrap();
        for selected in &plugins {
            for action in [Action::Disable, Action::Enable] {
                assert!(target.protection_reason(selected, &plugins).is_none());
                assert!(eligible(action, selected).is_ok());
                assert!(target.prepare(action, selected).is_err());
            }
        }
        server.join().unwrap();
    }
    #[test]
    fn cross_process_lock_survives_observer_process_and_blocks_remove() {
        const PROFILE: &str = "HERDR_MANAGER_TEST_CHILD_PROFILE";
        let child_profile = std::env::var_os(PROFILE);
        if let Some(root) = child_profile {
            let root = PathBuf::from(root);
            let config_dir = root.join("profile");
            let state_dir = root.join("state");
            let paths = Paths {
                control_socket: state_dir.join("control.sock"),
                log_file: state_dir.join("pet.log"),
                config_dir,
                state_dir,
            };
            let store = ProfileOperationStore::new(&paths).unwrap();
            let id = store.active_id().unwrap().unwrap();
            let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
            let lifetime = regular_at(&dir, "operation.lock", false).unwrap().unwrap();
            assert!(lock(&lifetime, true).unwrap());
            fs::write(root.join("ready"), b"locked").unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !root.join("release").exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            assert!(root.join("release").exists());
            return;
        }
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        drop(lifetime);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg("plugin_manager_backend::tests::cross_process_lock_survives_observer_process_and_blocks_remove")
            .env(PROFILE, &fixture.dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.path("ready").exists() && Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before acquiring lock: {status}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            fixture.path("ready").exists(),
            "child did not acquire operation lock"
        );
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Running { operation_id, .. } if operation_id == id));
        assert!(availability_admission(&paths, false).is_err());
        fs::write(fixture.path("release"), b"done").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Unknown { operation_id, acknowledged: false, .. } if operation_id == id));
    }
    #[test]
    fn cross_process_profile_gate_rejects_busy_without_waiting() {
        const PROFILE: &str = "HERDR_MANAGER_GATE_TEST_PROFILE";
        if let Some(root) = std::env::var_os(PROFILE) {
            let root = PathBuf::from(root);
            let config = File::open(root.join("profile")).unwrap();
            let manager = directory_at(&config, MANAGER_DIR, false).unwrap();
            let admission = regular_at(&manager, "admission.lock", false)
                .unwrap()
                .unwrap();
            assert!(lock(&admission, true).unwrap());
            fs::write(root.join("ready"), b"locked").unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !root.join("release").exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            assert!(root.join("release").exists());
            return;
        }
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg("plugin_manager_backend::tests::cross_process_profile_gate_rejects_busy_without_waiting")
            .env(PROFILE, &fixture.dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.path("ready").exists() && Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before acquiring admission lock: {status}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            fixture.path("ready").exists(),
            "child did not acquire admission lock"
        );
        let started = Instant::now();
        let outcome = store.gate();
        let elapsed = started.elapsed();
        fs::write(fixture.path("release"), b"done").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(outcome.is_err());
        assert!(
            elapsed < Duration::from_secs(1),
            "admission waited {elapsed:?}"
        );
    }
    #[test]
    fn durable_cli_unknown_requires_exact_review_but_permits_removal() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        fs::write(
            store.config.join("preferences.json"),
            br#"{"plugin_manager_installed":true}"#,
        )
        .unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        let result = json!({"protocol":2,"operation_id":id,"action":"unlink",
            "plugin_id":"foreign","status":"unknown","detail":"host reported uncertain mutation"});
        publish(&dir, "result.json", &result).unwrap();
        drop(lifetime);
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Unknown { operation_id, detail, acknowledged: false, .. }
            if operation_id == id && detail == "host reported uncertain mutation"));
        let (target, server) = fixture.socket(
            inventory(vec![plugin("foreign", &fixture.path("foreign"), None)]),
            2,
        );
        let selected = target.list().unwrap().remove(0);
        assert!(store.submit(&target, Action::Unlink, &selected).is_err());
        assert_eq!(store.active_id().unwrap().as_deref(), Some(id.as_str()));
        assert!(record(&dir, "request.json", FRAME_LIMIT).unwrap().is_some());
        server.join().unwrap();
        assert!(availability_admission(&paths, false).is_ok());
        assert!(store.acknowledge_unknown("other-operation").is_err());
        store.acknowledge_unknown(&id).unwrap();
        assert_eq!(
            record(&dir, "result.json", OUTPUT_LIMIT).unwrap(),
            Some(result)
        );
        let (next, next_lifetime) = active_without_result(&store);
        assert_ne!(next, id);
        assert!(matches!(store.observation_id(id.clone()).unwrap(),
            OperationObservation::Unknown { operation_id, acknowledged: true, .. }
            if operation_id == id));
        drop(next_lifetime);
    }
    #[test]
    fn request_journal_above_output_limit_remains_observable() {
        let fixture = Fixture::new();
        let store = ProfileOperationStore::new(&fixture.paths()).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        let request = json!({"protocol":2,"operation_id":id,"action":"unlink",
            "expected":{"plugin_id":"foreign","metadata":"x".repeat(OUTPUT_LIMIT)},
            "expected_registration":{"plugin_id":"foreign"}});
        let mut file = regular_at(&dir, "request.json", false).unwrap().unwrap();
        file.set_len(0).unwrap();
        sync_json_with_limit(&mut file, &request, FRAME_LIMIT).unwrap();
        dir.sync_all().unwrap();
        publish(
            &dir,
            "result.json",
            &json!({"protocol":2,"operation_id":id,
            "action":"unlink","plugin_id":"foreign","status":"completed","detail":"known"}),
        )
        .unwrap();
        drop(lifetime);
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Terminal { operation_id, .. } if operation_id == id));
    }
    #[test]
    fn invalid_result_is_never_claimed_as_known_success() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        publish(
            &dir,
            "result.json",
            &json!({"protocol":2,"operation_id":id,"action":"unlink",
            "plugin_id":"foreign","status":"completed","detail":"known"}),
        )
        .unwrap();
        drop(lifetime);
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Terminal { operation_id, result, .. }
            if operation_id == id && result["status"] == "completed"));
        let path = store
            .config
            .join(MANAGER_DIR)
            .join("operations")
            .join(&id)
            .join("result.json");
        fs::write(path, b"invalid JSON").unwrap();
        assert!(matches!(
            store.observe_active().unwrap(),
            OperationObservation::Unknown { .. }
        ));
    }
    #[test]
    fn operation_handle_observes_own_result_after_new_active_reservation() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let store = ProfileOperationStore::new(&paths).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let journal = store.config.join(MANAGER_DIR).join("operations").join(&id);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        publish(
            &dir,
            "result.json",
            &json!({"protocol":2,"operation_id":id,
            "action":"unlink","plugin_id":"foreign","status":"completed","detail":"removed"}),
        )
        .unwrap();
        drop(lifetime);
        let handle = OperationHandle {
            id: id.clone(),
            journal,
            store: store.clone(),
        };
        let (next, next_lifetime) = active_without_result(&store);
        assert_ne!(id, next);
        assert!(matches!(handle.wait(Duration::ZERO).unwrap(),
            OperationObservation::Terminal { operation_id, .. } if operation_id == id));
        drop(next_lifetime);
    }
    #[test]
    fn inclusive_request_file_limit_round_trips_through_active_observer() {
        let fixture = Fixture::new();
        let store = ProfileOperationStore::new(&fixture.paths()).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        let mut request = json!({"protocol":2,"operation_id":id,"action":"unlink",
            "expected":{"plugin_id":"foreign","padding":""},
            "expected_registration":{"plugin_id":"foreign"}});
        let base = serde_json::to_vec(&request).unwrap().len();
        for size in [FRAME_LIMIT - 1, FRAME_LIMIT] {
            request["expected"]["padding"] = json!("x".repeat(size - base));
            let mut file = regular_at(&dir, "request.json", false).unwrap().unwrap();
            file.set_len(0).unwrap();
            sync_json_with_limit(&mut file, &request, FRAME_LIMIT).unwrap();
            assert_eq!(file.metadata().unwrap().len(), size as u64);
            assert_eq!(
                record(&dir, "request.json", FRAME_LIMIT).unwrap(),
                Some(request.clone())
            );
            assert!(matches!(store.observe_active().unwrap(),
                OperationObservation::Running { operation_id, .. } if operation_id == id));
        }
        request["expected"]["padding"] = json!("x".repeat(FRAME_LIMIT + 1 - base));
        let mut oversized = create_regular(&dir, "oversized-request.json").unwrap();
        assert!(sync_json_with_limit(&mut oversized, &request, FRAME_LIMIT).is_err());
        assert_eq!(oversized.metadata().unwrap().len(), 0);
        fs::write(
            store
                .config
                .join(MANAGER_DIR)
                .join("operations")
                .join(&id)
                .join("oversized-request.json"),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        assert!(record(&dir, "oversized-request.json", FRAME_LIMIT).is_err());
        assert_eq!(store.active_id().unwrap().as_deref(), Some(id.as_str()));
        drop(lifetime);
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Unknown { operation_id, .. } if operation_id == id));
    }
    #[test]
    fn inclusive_result_file_limit_round_trips_through_active_observer() {
        let fixture = Fixture::new();
        let store = ProfileOperationStore::new(&fixture.paths()).unwrap();
        let (id, lifetime) = active_without_result(&store);
        let dir = directory_at(&store.operations().unwrap(), &id, false).unwrap();
        let mut result = json!({"protocol":2,"operation_id":id,"action":"unlink",
            "plugin_id":"foreign","status":"completed","detail":""});
        let base = serde_json::to_vec(&result).unwrap().len();
        for size in [OUTPUT_LIMIT - 1, OUTPUT_LIMIT] {
            result["detail"] = json!("x".repeat(size - base));
            publish(&dir, "result.json", &result).unwrap();
            let file = regular_at(&dir, "result.json", false).unwrap().unwrap();
            assert_eq!(file.metadata().unwrap().len(), size as u64);
            assert_eq!(read_small(file, OUTPUT_LIMIT).unwrap(), result);
        }
        result["detail"] = json!("x".repeat(OUTPUT_LIMIT + 1 - base));
        let mut oversized = create_regular(&dir, "oversized-result.json").unwrap();
        assert!(sync_json_with_limit(&mut oversized, &result, OUTPUT_LIMIT).is_err());
        assert_eq!(oversized.metadata().unwrap().len(), 0);
        fs::write(
            store
                .config
                .join(MANAGER_DIR)
                .join("operations")
                .join(&id)
                .join("oversized-result.json"),
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
        assert!(record(&dir, "oversized-result.json", OUTPUT_LIMIT).is_err());
        drop(lifetime);
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Terminal { operation_id, .. } if operation_id == id));
    }
    #[test]
    fn ancestor_sync_failure_preserves_previous_consumer_view_and_never_starts_apply() {
        const CHILD: &str = "HERDR_MANAGER_ANCESTOR_SYNC_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Parallel tests can fork while we hold a CLOEXEC flock descriptor,
            // retaining its open file description until exec even after our drop.
            // Create these profiles after exec so the public consumer assertion
            // checks immediate lease release without unrelated inherited owners.
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plugin_manager_backend::tests::ancestor_sync_failure_preserves_previous_consumer_view_and_never_starts_apply",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(
                status.success(),
                "isolated ancestor-sync test failed: {status}"
            );
            return;
        }
        for failed_at in 0..4 {
            let fixture = Fixture::new();
            let store = ProfileOperationStore::new(&fixture.paths()).unwrap();
            let (previous, lifetime) = active_without_result(&store);
            // Submission holds one admission lease throughout journal publication.
            // Observe through that same lease; observe_active would try to lock it again.
            let admission = store.gate().unwrap();
            store.ensure_pairing(&admission).unwrap();
            let (config, manager) = store.manager_with_parent().unwrap();
            let operations = directory_at(&manager, "operations", true).unwrap();
            let old_locator = fs::read(store.config.join(MANAGER_DIR).join("active.json")).unwrap();
            let next = operation_id().unwrap();
            let dir = directory_at(&operations, &next, true).unwrap();
            let mut request = create_regular(&dir, "request.json").unwrap();
            sync_json_with_limit(
                &mut request,
                &json!({"protocol":2,"operation_id":next,
                "action":"unlink","expected":{"plugin_id":"foreign"},
                "expected_registration":{"plugin_id":"foreign"}}),
                FRAME_LIMIT,
            )
            .unwrap();
            let mut calls = 0;
            let mut started = false;
            let outcome = publish_submission(
                &config,
                &manager,
                &operations,
                &dir,
                &next,
                |file| {
                    if calls == failed_at {
                        calls += 1;
                        return Err("injected fsync failure".into());
                    }
                    calls += 1;
                    file.sync_all().map_err(|e| e.to_string())
                },
                || {
                    started = true;
                    Ok(())
                },
            );
            assert!(outcome.is_err());
            assert_eq!(calls, failed_at + 1);
            assert!(!started);
            assert_eq!(
                fs::read(store.config.join(MANAGER_DIR).join("active.json")).unwrap(),
                old_locator
            );
            assert!(matches!(store.observation().unwrap(),
                OperationObservation::Running { operation_id, .. } if operation_id == previous));
            drop(lifetime);
            assert!(matches!(store.observation().unwrap(),
                OperationObservation::Unknown { operation_id, .. } if operation_id == previous));
            drop(admission);
            assert!(matches!(store.observe_active().unwrap(),
                OperationObservation::Unknown { operation_id, .. } if operation_id == previous));
        }
    }
    #[test]
    fn already_existing_ancestors_are_synced_before_new_active_handoff() {
        let fixture = Fixture::new();
        let store = ProfileOperationStore::new(&fixture.paths()).unwrap();
        let (_previous, lifetime) = active_without_result(&store);
        drop(lifetime);
        let (config, manager) = store.manager_with_parent().unwrap();
        let operations = directory_at(&manager, "operations", true).unwrap();
        let id = operation_id().unwrap();
        let dir = directory_at(&operations, &id, true).unwrap();
        let lifetime = create_regular(&dir, "operation.lock").unwrap();
        assert!(lock(&lifetime, true).unwrap());
        let request = json!({"protocol":2,"operation_id":id,"action":"unlink",
            "expected":{"plugin_id":"foreign"},"expected_registration":{"plugin_id":"foreign"}});
        sync_json_with_limit(
            &mut create_regular(&dir, "request.json").unwrap(),
            &request,
            FRAME_LIMIT,
        )
        .unwrap();
        let mut synced = Vec::new();
        let started = publish_submission(
            &config,
            &manager,
            &operations,
            &dir,
            &id,
            |file| {
                synced.push(file.metadata().unwrap().ino());
                file.sync_all().map_err(|e| e.to_string())
            },
            || {
                assert_eq!(store.active_id().unwrap().as_deref(), Some(id.as_str()));
                assert_eq!(
                    record(&dir, "request.json", FRAME_LIMIT).unwrap(),
                    Some(request)
                );
                Ok(true)
            },
        )
        .unwrap();
        assert!(started);
        assert_eq!(
            synced,
            [&dir, &operations, &manager, &config].map(|file| file.metadata().unwrap().ino())
        );
        assert!(matches!(store.observe_active().unwrap(),
            OperationObservation::Running { operation_id, .. } if operation_id == id));
    }
    #[test]
    fn config_profile_cannot_be_repaired_with_different_state_profile() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        ProfileOperationStore::new(&paths).unwrap();
        let second = fixture.path("other-state");
        fs::create_dir(&second).unwrap();
        fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
        let mut altered = paths.clone();
        altered.state_dir = second;
        assert!(ProfileOperationStore::new(&altered).is_err());
    }
}
