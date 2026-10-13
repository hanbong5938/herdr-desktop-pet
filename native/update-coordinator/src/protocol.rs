//! Private journals, per-install reservation and per-profile stop-intent gate.
//! Every ensure/start path must consult `start_allowed`; user stop calls `record_user_stop`
//! *before* touching a daemon, including when the daemon is already offline.
use crate::{ensure_absolute, owned_private_file, InstallOrigin};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    Preparing,
    Stopping,
    Installing,
    Validating,
    Starting,
    Completed,
    Failed,
    Unknown,
    UserStopped,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFence {
    NoSpawn,
    ManagerIntent,
    ManagerExited,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub version: u32,
    pub operation_id: String,
    pub phase: OperationPhase,
    pub execution_fence: ExecutionFence,
    pub detail: String,
    pub installed: bool,
    pub applied: bool,
    pub candidate: Option<crate::ExecutableIdentity>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateControlRequest {
    pub version: u32,
    pub kind: String,
    pub command: UpdateCommand,
    pub instance_id: String,
    pub operation_id: String,
    pub token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCommand {
    Prepare,
    Stop,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateControlReply {
    pub version: u32,
    pub instance_id: String,
    pub operation_id: String,
    pub accepted: bool,
    pub detail: String,
}
pub const UPDATER_PROTOCOL: u32 = 3;

pub fn validate_operation_id(id: &str) -> Result<(), String> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("operation id must be 128-bit lowercase hex".into());
    }
    Ok(())
}
fn check_dir(path: &Path, create: bool) -> Result<(), String> {
    ensure_absolute(path)?;
    if create {
        use std::os::unix::fs::DirBuilderExt;
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o077 != 0
    {
        return Err(format!(
            "update directory must be private and owned: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Single-process lease held through journal creation, reservation, execution and recovery.
pub struct OperationLease {
    _file: File,
}

pub fn lock_operation(state_dir: &Path, op: &str) -> Result<OperationLease, String> {
    validate_operation_id(op)?;
    let file = lock_with_mode(
        &private_updates(state_dir)?.join(format!("{op}.operation.lock")),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    Ok(OperationLease { _file: file })
}
pub fn private_updates(state_dir: &Path) -> Result<PathBuf, String> {
    check_dir(state_dir, false)?;
    let updates = state_dir.join("updates");
    check_dir(&updates, true)?;
    Ok(updates)
}
fn sync_parent(path: &Path) -> Result<(), String> {
    File::open(path.parent().ok_or("missing parent")?)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
/// Atomic bounded replacement; symlinks/hardlinks at the destination are rejected.
pub fn write_private_bytes(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    ensure_absolute(path)?;
    let limit = if mode == 0o700 {
        64 * 1024 * 1024
    } else {
        1024 * 1024
    };
    if data.len() > limit || !matches!(mode, 0o600 | 0o700) {
        return Err("invalid private file size/mode".into());
    }
    check_dir(path.parent().ok_or("missing parent")?, false)?;
    match fs::symlink_metadata(path) {
        Ok(m)
            if !m.is_file()
                || m.file_type().is_symlink()
                || m.uid() != unsafe { libc::geteuid() }
                || m.nlink() != 1
                || m.permissions().mode() & 0o077 != 0 =>
        {
            return Err("unsafe existing update file".into())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let mut random = [0u8; 12];
    use std::io::Read;
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!(
        "tmp-{}",
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(data)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Immutable helper/plan creation: an operation id can never silently replace a prior plan.
pub fn write_private_new(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    ensure_absolute(path)?;
    let limit = if mode == 0o700 {
        64 * 1024 * 1024
    } else {
        1024 * 1024
    };
    if data.len() > limit || !matches!(mode, 0o600 | 0o700) {
        return Err("invalid private file size/mode".into());
    }
    check_dir(path.parent().ok_or("missing parent")?, false)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let result = file
        .write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
        .and_then(|_| sync_parent(path));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
pub fn write_operation(state_dir: &Path, record: &OperationRecord) -> Result<PathBuf, String> {
    validate_operation_id(&record.operation_id)?;
    let updates = private_updates(state_dir)?;
    let target = updates.join(format!("{}.result.json", record.operation_id));
    if !missing_file(&target) {
        let previous = owned_private_file(&target, 16384)?;
        let previous: OperationRecord = serde_json::from_slice(&previous)
            .map_err(|_| "existing operation journal is unsupported; keep original bytes")?;
        if previous.version != UPDATER_PROTOCOL {
            return Err("existing operation journal is unsupported; keep original bytes".into());
        }
    }
    let latest = updates.join("latest-operation.json");
    if !missing_file(&latest) {
        let old_id: String = serde_json::from_slice(&owned_private_file(&latest, 128)?)
            .map_err(|_| "existing latest-operation marker is unsupported")?;
        read_operation(state_dir, &old_id)?
            .ok_or("existing latest-operation marker has no durable journal")?;
    }
    if record.version != UPDATER_PROTOCOL {
        return Err("unsupported operation journal protocol".into());
    }
    if record.detail.len() > 8192 || record.applied && !record.installed {
        return Err("invalid operation outcome".into());
    }
    let path = target;
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    write_private_bytes(&path, &bytes, 0o600)?;
    write_private_bytes(
        &path.with_file_name("latest-operation.json"),
        &serde_json::to_vec(&record.operation_id).map_err(|e| e.to_string())?,
        0o600,
    )?;
    Ok(path)
}
pub fn read_operation(
    state_dir: &Path,
    operation_id: &str,
) -> Result<Option<OperationRecord>, String> {
    validate_operation_id(operation_id)?;
    check_dir(state_dir, false)?;
    let updates = state_dir.join("updates");
    if missing_file(&updates) {
        return Ok(None);
    }
    check_dir(&updates, false)?;
    let path = updates.join(format!("{operation_id}.result.json"));
    let bytes = match owned_private_file(&path, 16384) {
        Ok(v) => v,
        Err(_) if missing_file(&path) => return Ok(None),
        Err(e) => return Err(e),
    };
    let record: OperationRecord = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if record.version != UPDATER_PROTOCOL {
        return Err("unsupported operation journal protocol".into());
    }
    if record.operation_id != operation_id {
        return Err("journal operation id mismatch".into());
    }
    Ok(Some(record))
}

/// A reserved operation takes precedence over the most recently persisted outcome.
/// Read only: an untouched profile does not gain an update directory.
pub fn latest_operation(state_dir: &Path) -> Result<Option<OperationRecord>, String> {
    check_dir(state_dir, false)?;
    let updates = state_dir.join("updates");
    if missing_file(&updates) {
        return Ok(None);
    }
    check_dir(&updates, false)?;
    let marker = updates.join("profile.reservation.json");
    if !missing_file(&marker) {
        let scope = reservation(&owned_private_file(&marker, 16384)?)?;
        if scope.state_dir != state_dir {
            return Err("reserved update belongs to a different profile".into());
        }
        return read_operation(state_dir, &scope.operation_id)?
            .map(Some)
            .ok_or_else(|| {
                "reserved update has no durable outcome; inspect helper status/recover".into()
            });
    }
    let latest = updates.join("latest-operation.json");
    if missing_file(&latest) {
        return Ok(None);
    }
    let operation_id: String =
        serde_json::from_slice(&owned_private_file(&latest, 128)?).map_err(|e| e.to_string())?;
    read_operation(state_dir, &operation_id)?
        .map(Some)
        .ok_or_else(|| "latest update marker has no durable outcome".into())
}

pub fn installation_key(origin: &InstallOrigin) -> Result<String, String> {
    let root = crate::physical_install_root(origin)?;
    let material = match origin {
        InstallOrigin::Herdr { .. } | InstallOrigin::Local { .. } => {
            format!("checkout\0{}", root.display())
        }
        InstallOrigin::Homebrew { formula, .. } => format!("brew\0{}\0{formula}", root.display()),
        InstallOrigin::Manual { .. } => format!("manual\0{}", root.display()),
        InstallOrigin::Unknown { .. } => {
            return Err("installation has no safely reservable owner".into())
        }
    };
    let digest = Sha256::digest(material.as_bytes());
    Ok(format!("{:x}", digest))
}

/// Earlier manager/formula and manual-image keys are never a clean namespace.
fn legacy_key(origin: &InstallOrigin) -> Option<String> {
    let material = match origin {
        InstallOrigin::Homebrew {
            prefix, formula, ..
        } => format!("brew\0{}\0{formula}", prefix.display()),
        InstallOrigin::Manual { locator } => format!("manual\0{}", locator.display()),
        _ => return None,
    };
    Some(format!("{:x}", Sha256::digest(material.as_bytes())))
}

fn reject_legacy_marker(global: &Path, origin: &InstallOrigin) -> Result<(), String> {
    if let Some(key) = legacy_key(origin) {
        if key != installation_key(origin)?
            && !missing_file(&global.join(format!("{key}.reservation.json")))
        {
            return Err(
                "legacy installation reservation remains blocked; preserve old marker".into(),
            );
        }
    }
    Ok(())
}

fn lock_scope(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    keys: &[String],
) -> Result<Vec<File>, String> {
    let mut lock_keys = keys.to_vec();
    for origin in std::iter::once(source).chain(target) {
        if let Some(old) = legacy_key(origin) {
            lock_keys.push(old);
        }
    }
    lock_keys.sort_unstable();
    lock_keys.dedup();
    lock_keys
        .iter()
        .map(|key| {
            lock_with_mode(
                &global.join(format!("{key}.lock")),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        })
        .collect()
}
/// UID-wide durable private root outside replaceable app/checkout, shared by profiles.
pub fn global_updates() -> Result<PathBuf, String> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let uid = unsafe { libc::geteuid() };
    let mut user: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut user,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || user.pw_dir.is_null() {
        return Err("cannot determine user home for updater reservation".into());
    }
    let home = Path::new(std::ffi::OsStr::from_bytes(
        unsafe { CStr::from_ptr(user.pw_dir) }.to_bytes(),
    ));
    ensure_absolute(home)?;
    let path = home.join(".herdr-updates");
    check_dir(&path, true)?;
    Ok(path)
}
fn lock(path: &Path) -> Result<File, String> {
    lock_with_mode(path, libc::LOCK_EX)
}
fn lock_with_mode(path: &Path, mode: libc::c_int) -> Result<File, String> {
    check_dir(path.parent().ok_or("lock without parent")?, false)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.nlink() != 1
        || m.permissions().mode() & 0o077 != 0
    {
        return Err("unsafe update lock".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), mode) } != 0 {
        return Err(format!(
            "installation busy: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(file)
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    operation_id: String,
    source: InstallOrigin,
    target: Option<InstallOrigin>,
    keys: Vec<String>,
    state_dir: PathBuf,
    stop_generation: u64,
    control_token: String,
    startup_origin: Option<InstallOrigin>,
    startup_token: Option<String>,
}

fn selectable_origin(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    origin: &InstallOrigin,
) -> bool {
    if origin == source || target == Some(origin) {
        return true;
    }
    if target.is_some() {
        return false;
    }
    match (source, origin) {
        (
            InstallOrigin::Herdr {
                host,
                host_config_dir,
                checkout_root,
                plugin_root,
                source,
                requested_ref,
                enabled,
                ..
            },
            InstallOrigin::Herdr {
                host: actual_host,
                host_config_dir: actual_config,
                checkout_root: actual_checkout,
                plugin_root: actual_plugin,
                source: actual_source,
                requested_ref: actual_ref,
                resolved_commit: Some(commit),
                enabled: actual_enabled,
            },
        ) => {
            host == actual_host
                && host_config_dir == actual_config
                && checkout_root == actual_checkout
                && plugin_root == actual_plugin
                && source == actual_source
                && requested_ref == actual_ref
                && enabled == actual_enabled
                && commit.len() == 40
                && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        _ => false,
    }
}

fn valid_reservation(marker: &Reservation) -> Result<(), String> {
    if marker.version != UPDATER_PROTOCOL
        || validate_operation_id(&marker.operation_id).is_err()
        || marker.keys != installation_keys(&marker.source, marker.target.as_ref())?
        || marker.startup_origin.is_some() != marker.startup_token.is_some()
        || marker.startup_origin.as_ref().is_some_and(|origin| {
            !selectable_origin(&marker.source, marker.target.as_ref(), origin)
        })
    {
        return Err("unsupported reservation; preserve original marker".into());
    }
    Ok(())
}
fn reservation(bytes: &[u8]) -> Result<Reservation, String> {
    let marker: Reservation = serde_json::from_slice(bytes)
        .map_err(|_| "unsupported reservation; preserve original marker")?;
    valid_reservation(&marker)?;
    Ok(marker)
}

fn installation_keys(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
) -> Result<Vec<String>, String> {
    let mut keys = vec![installation_key(source)?];
    if let Some(target) = target {
        keys.push(installation_key(target)?);
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}

// The atomic index is written before any individual marker and removed last.
// Thus a crash between writes never leaves an unguarded root, even if the
// profile or one of the installation markers has not yet been created.
fn scopes_path(global: &Path) -> PathBuf {
    global.join("installation-scopes.json")
}
fn scopes(global: &Path) -> Result<Vec<Reservation>, String> {
    let path = scopes_path(global);
    if missing_file(&path) {
        return Ok(Vec::new());
    }
    let bytes = owned_private_file(&path, 1024 * 1024)?;
    let entries: Vec<Reservation> = serde_json::from_slice(&bytes)
        .map_err(|_| "unsupported installation scopes; preserve original bytes")?;
    if entries.len() > 256 {
        return Err("installation scope registry exceeds bound".into());
    }
    for entry in &entries {
        valid_reservation(entry)?;
    }
    Ok(entries)
}
fn write_scopes(global: &Path, entries: &[Reservation]) -> Result<(), String> {
    write_private_bytes(
        &scopes_path(global),
        &serde_json::to_vec(entries).map_err(|e| e.to_string())?,
        0o600,
    )
}
fn marker_path(global: &Path, key: &str) -> PathBuf {
    global.join(format!("{key}.reservation.json"))
}
fn marker_at(path: &Path) -> Result<Option<Reservation>, String> {
    if missing_file(path) {
        Ok(None)
    } else {
        reservation(&owned_private_file(path, 16384)?).map(Some)
    }
}
fn markers_match(global: &Path, data: &Reservation) -> Result<bool, String> {
    if marker_at(&profile_marker(&data.state_dir)?)?.as_ref() != Some(data) {
        return Ok(false);
    }
    for key in &data.keys {
        if marker_at(&marker_path(global, key))?.as_ref() != Some(data) {
            return Ok(false);
        }
    }
    Ok(true)
}
fn write_markers(global: &Path, data: &Reservation) -> Result<(), String> {
    let bytes = serde_json::to_vec(data).map_err(|e| e.to_string())?;
    write_private_bytes(&profile_marker(&data.state_dir)?, &bytes, 0o600)?;
    for key in &data.keys {
        write_private_bytes(&marker_path(global, key), &bytes, 0o600)?;
    }
    Ok(())
}
fn scope_for<'a>(
    entries: &'a [Reservation],
    op: &str,
    state_dir: &Path,
) -> Result<&'a Reservation, String> {
    entries
        .iter()
        .find(|entry| entry.operation_id == op && entry.state_dir == state_dir)
        .ok_or_else(|| "operation has no durable installation scope".into())
}
fn missing_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
fn generation_file(state_dir: &Path) -> Result<PathBuf, String> {
    Ok(private_updates(state_dir)?.join("stop-generation.json"))
}
fn generation(state_dir: &Path) -> Result<u64, String> {
    let path = generation_file(state_dir)?;
    match owned_private_file(&path, 32) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(_) if missing_file(&path) => Ok(0),
        Err(e) => Err(e),
    }
}
fn profile_marker(state_dir: &Path) -> Result<PathBuf, String> {
    Ok(private_updates(state_dir)?.join("profile.reservation.json"))
}
/// Consult during both fast live reuse and spawn, while holding startup lease.
/// Only a helper-issued token can start the reserved install; user stop generation wins.
pub fn start_allowed(
    origin: &InstallOrigin,
    state_dir: &Path,
    expected_generation: Option<u64>,
    updater_token: Option<&str>,
) -> Result<(), String> {
    start_allowed_at(
        &global_updates()?,
        origin,
        state_dir,
        expected_generation,
        updater_token,
    )
}
fn start_allowed_at(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    expected_generation: Option<u64>,
    updater_token: Option<&str>,
) -> Result<(), String> {
    let _gate = lock(&global.join("gate.lock"))?;
    start_allowed_locked(
        global,
        origin,
        state_dir,
        expected_generation,
        updater_token,
    )
}
fn start_allowed_locked(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    expected_generation: Option<u64>,
    updater_token: Option<&str>,
) -> Result<(), String> {
    let current = generation(state_dir)?;
    if expected_generation.is_some_and(|g| g != current) {
        return Err("user requested stop after updater prepared startup".into());
    }
    let profile_path = profile_marker(state_dir)?;
    let key = match origin {
        InstallOrigin::Unknown { .. } => None,
        _ => {
            reject_legacy_marker(global, origin)?;
            Some(installation_key(origin)?)
        }
    };
    let entries = scopes(global)?;
    let relevant = entries.iter().filter(|entry| {
        entry.state_dir == state_dir || key.as_ref().is_some_and(|key| entry.keys.contains(key))
    });
    let mut admitted = false;
    for entry in relevant {
        if entry.state_dir != state_dir
            || entry.startup_origin.as_ref() != Some(origin)
            || expected_generation != Some(entry.stop_generation)
            || updater_token.is_none()
            || updater_token != entry.startup_token.as_deref()
            || !markers_match(global, entry)?
        {
            return Err("installation or profile reserved by updater".into());
        }
        admitted = true;
    }
    if let Some(marker) = marker_at(&profile_path)? {
        if !admitted || marker.state_dir != state_dir {
            return Err("profile reservation lacks matching durable scope".into());
        }
    }
    if let Some(key) = key {
        if marker_at(&marker_path(global, &key))?.is_some() && !admitted {
            return Err("installation reservation lacks matching durable scope".into());
        }
    }
    if updater_token.is_some() && !admitted {
        return Err("updater start token has no active full-scope reservation".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveProfile {
    pub state_dir: PathBuf,
    pub pid: u32,
    pub instance_id: String,
}
fn active_path(global: &Path, key: &str) -> PathBuf {
    global.join(format!("{key}.active.json"))
}
fn active(global: &Path, key: &str) -> Result<Vec<ActiveProfile>, String> {
    let path = active_path(global, key);
    if missing_file(&path) {
        return Ok(Vec::new());
    }
    let entries: Vec<ActiveProfile> =
        serde_json::from_slice(&owned_private_file(&path, 16384)?).map_err(|e| e.to_string())?;
    if entries.len() > 64 {
        return Err("active profile registry exceeds bound".into());
    }
    Ok(entries)
}
fn active_for_origin(global: &Path, origin: &InstallOrigin) -> Result<Vec<ActiveProfile>, String> {
    let key = installation_key(origin)?;
    let mut entries = active(global, &key)?;
    if let Some(old) = legacy_key(origin).filter(|old| old != &key) {
        entries.extend(active(global, &old)?);
    }
    Ok(entries)
}
fn live(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    (unsafe { libc::kill(pid as i32, 0) }) == 0
        || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
/// Daemon calls this after startup admission, and removes its exact instance on finalization.
pub fn register_active(
    origin: &InstallOrigin,
    state_dir: &Path,
    pid: u32,
    instance_id: &str,
    expected_generation: Option<u64>,
    updater_token: Option<&str>,
) -> Result<(), String> {
    register_active_at(
        &global_updates()?,
        origin,
        state_dir,
        pid,
        instance_id,
        expected_generation,
        updater_token,
    )
}
fn register_active_at(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    pid: u32,
    instance_id: &str,
    expected_generation: Option<u64>,
    updater_token: Option<&str>,
) -> Result<(), String> {
    if pid == 0 || instance_id.is_empty() || instance_id.len() > 128 {
        return Err("invalid active daemon identity".into());
    }
    let _gate = lock(&global.join("gate.lock"))?;
    start_allowed_locked(
        global,
        origin,
        state_dir,
        expected_generation,
        updater_token,
    )?;
    let key = installation_key(origin)?;
    let mut entries = active(global, &key)?;
    entries.retain(|e| live(e.pid));
    if entries
        .iter()
        .any(|e| e.state_dir == state_dir && (e.pid != pid || e.instance_id != instance_id))
    {
        return Err("profile already has another active instance".into());
    }
    if entries
        .iter()
        .any(|e| e.state_dir == state_dir && e.pid == pid && e.instance_id == instance_id)
    {
        return Ok(());
    }
    if entries.len() >= 64 {
        return Err("too many active profiles for installation".into());
    }
    entries.push(ActiveProfile {
        state_dir: state_dir.into(),
        pid,
        instance_id: instance_id.into(),
    });
    write_private_bytes(
        &active_path(global, &key),
        &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
        0o600,
    )
}
pub fn unregister_active(
    origin: &InstallOrigin,
    state_dir: &Path,
    pid: u32,
    instance_id: &str,
) -> Result<(), String> {
    let global = global_updates()?;
    let _gate = lock(&global.join("gate.lock"))?;
    let key = installation_key(origin)?;
    let mut entries = active(&global, &key)?;
    entries.retain(|e| !(e.state_dir == state_dir && e.pid == pid && e.instance_id == instance_id));
    write_private_bytes(
        &active_path(&global, &key),
        &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
        0o600,
    )
}
pub fn conflicting_active_profiles(
    origin: &InstallOrigin,
    state_dir: &Path,
) -> Result<Vec<ActiveProfile>, String> {
    let global = global_updates()?;
    let _gate = lock(&global.join("gate.lock"))?;
    Ok(active_for_origin(&global, origin)?
        .into_iter()
        .filter(|e| e.state_dir != state_dir && live(e.pid))
        .collect())
}

/// Control requests must match both the original and proposed installation scope.
pub fn verify_control_for(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
    token: &str,
) -> Result<(), String> {
    verify_control_at(
        &global_updates()?,
        source,
        target,
        state_dir,
        operation_id,
        token,
    )
}
fn verify_control_at(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
    token: &str,
) -> Result<(), String> {
    validate_operation_id(operation_id)?;
    let _gate = lock(&global.join("gate.lock"))?;
    let entries = scopes(global)?;
    let data = scope_for(&entries, operation_id, state_dir)?;
    if &data.source != source
        || data.target.as_ref() != target
        || data.keys != installation_keys(source, target)?
        || data.control_token != token
        || generation(state_dir)? != data.stop_generation
        || !markers_match(global, data)?
    {
        return Err("updater request has no valid full-scope reservation".into());
    }
    Ok(())
}
pub fn stop_generation(state_dir: &Path) -> Result<u64, String> {
    let _gate = lock(&global_updates()?.join("gate.lock"))?;
    generation(state_dir)
}
pub fn record_user_stop(state_dir: &Path) -> Result<u64, String> {
    record_user_stop_at(&global_updates()?, state_dir)
}
fn record_user_stop_at(global: &Path, state_dir: &Path) -> Result<u64, String> {
    let _gate = lock(&global.join("gate.lock"))?;
    let next = generation(state_dir)?
        .checked_add(1)
        .ok_or("stop generation overflow")?;
    write_private_bytes(
        &generation_file(state_dir)?,
        &serde_json::to_vec(&next).map_err(|e| e.to_string())?,
        0o600,
    )?;
    Ok(next)
}
/// Durable atomic scope index gates both roots even across partial marker writes.
/// Dropping the locks never clears the scope; release follows reconciliation.
pub struct InstallationReservation {
    _locks: Vec<File>,
    _profile_lock: File,
    source: InstallOrigin,
    target: Option<InstallOrigin>,
    operation_id: String,
    state_dir: PathBuf,
    global_dir: PathBuf,
    control_token: String,
    pub stop_generation: u64,
}
/// Read-only inspection of the authoritative scope index. This never repairs
/// partially written markers or treats selection as startup admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReservationSnapshot {
    pub startup_origin: Option<InstallOrigin>,
    pub markers_complete: bool,
}

pub fn reservation_snapshot(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<Option<ReservationSnapshot>, String> {
    reservation_snapshot_at(&global_updates()?, source, target, state_dir, operation_id)
}

fn reservation_snapshot_at(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<Option<ReservationSnapshot>, String> {
    validate_operation_id(operation_id)?;
    check_dir(state_dir, false)?;
    let updates = state_dir.join("updates");
    if !missing_file(&updates) {
        check_dir(&updates, false)?;
    }
    let keys = installation_keys(source, target)?;
    let _gate = lock(&global.join("gate.lock"))?;
    for origin in std::iter::once(source).chain(target) {
        reject_legacy_marker(global, origin)?;
    }
    let entries = scopes(global)?;
    let found = entries.iter().find(|entry| {
        entry.operation_id == operation_id
            || entry.state_dir == state_dir
            || entry.keys.iter().any(|key| keys.contains(key))
    });
    let paths = std::iter::once(updates.join("profile.reservation.json"))
        .chain(keys.iter().map(|key| marker_path(global, key)));
    match found {
        Some(entry)
            if entry.operation_id == operation_id
                && entry.state_dir == state_dir
                && &entry.source == source
                && entry.target.as_ref() == target
                && entry.keys == keys =>
        {
            let mut markers_complete = true;
            for path in paths {
                match marker_at(&path)? {
                    Some(marker) if compatible_marker(&marker, entry) => {
                        markers_complete &= marker == *entry;
                    }
                    Some(_) => {
                        return Err("reservation evidence conflicts with full scope".into());
                    }
                    None => markers_complete = false,
                }
            }
            Ok(Some(ReservationSnapshot {
                startup_origin: entry.startup_origin.clone(),
                markers_complete,
            }))
        }
        Some(_) => Err("another update has reserved this profile or installation".into()),
        None => {
            for path in paths {
                if !missing_file(&path) {
                    return Err(
                        "reservation marker has no durable scope; preserve original bytes".into(),
                    );
                }
            }
            Ok(None)
        }
    }
}

/// Read-only discovery includes index-only reservations left by an interrupted
/// marker write; incompatible evidence is never interpreted as absence.
pub fn reservation_exists(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<bool, String> {
    reservation_exists_at(&global_updates()?, source, target, state_dir, operation_id)
}
fn reservation_exists_at(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<bool, String> {
    Ok(reservation_snapshot_at(global, source, target, state_dir, operation_id)?.is_some())
}

pub fn reserve_install(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    reserve_install_at(&global_updates()?, source, target, state_dir, operation_id)
}
fn reserve_install_at(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    validate_operation_id(operation_id)?;
    check_dir(global, false)?;
    let keys = installation_keys(source, target)?;
    let profile_lock = lock_with_mode(
        &private_updates(state_dir)?.join("profile.lock"),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let locks = lock_scope(global, source, target, &keys)?;
    let _gate = lock(&global.join("gate.lock"))?;
    let mut entries = scopes(global)?;
    for origin in std::iter::once(source).chain(target) {
        reject_legacy_marker(global, origin)?;
        if active_for_origin(global, origin)?
            .iter()
            .any(|entry| entry.state_dir != state_dir && live(entry.pid))
        {
            return Err("another active profile shares the installation".into());
        }
    }
    if entries.iter().any(|entry| {
        entry.state_dir == state_dir
            || entry.operation_id == operation_id
            || entry.keys.iter().any(|key| keys.contains(key))
    }) || !missing_file(&profile_marker(state_dir)?)
        || keys
            .iter()
            .any(|key| !missing_file(&marker_path(global, key)))
    {
        return Err(
            "installation or profile already reserved; inspect/recover previous operation".into(),
        );
    }
    if entries.len() >= 256 {
        return Err("installation scope registry is full".into());
    }
    let stop_generation = generation(state_dir)?;
    let control_token = new_token()?;
    let data = Reservation {
        version: UPDATER_PROTOCOL,
        operation_id: operation_id.into(),
        source: source.clone(),
        target: target.cloned(),
        keys,
        state_dir: state_dir.into(),
        stop_generation,
        control_token: control_token.clone(),
        startup_origin: None,
        startup_token: None,
    };
    entries.push(data.clone());
    write_scopes(global, &entries)?;
    // Keep the index after a failed marker write: it is the durable barrier
    // for both roots, and recovery completes the missing individual markers.
    write_markers(global, &data)?;
    Ok(InstallationReservation {
        _locks: locks,
        _profile_lock: profile_lock,
        source: source.clone(),
        target: target.cloned(),
        operation_id: operation_id.into(),
        state_dir: state_dir.into(),
        global_dir: global.into(),
        control_token,
        stop_generation,
    })
}
fn new_token() -> Result<String, String> {
    let mut random = [0u8; 32];
    use std::io::Read;
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| e.to_string())?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(64);
    for byte in random {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(token)
}
fn compatible_marker(actual: &Reservation, expected: &Reservation) -> bool {
    actual == expected
        || (actual.startup_origin.is_none()
            && actual.startup_token.is_none()
            && actual.version == expected.version
            && actual.operation_id == expected.operation_id
            && actual.source == expected.source
            && actual.target == expected.target
            && actual.keys == expected.keys
            && actual.state_dir == expected.state_dir
            && actual.stop_generation == expected.stop_generation
            && actual.control_token == expected.control_token)
}
/// Reattach to an atomic scope after coordinator death; callers still reconcile
/// disk state and journal before they start or release anything.
pub fn recover_reservation(
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    recover_reservation_at(&global_updates()?, source, target, state_dir, operation_id)
}
fn recover_reservation_at(
    global: &Path,
    source: &InstallOrigin,
    target: Option<&InstallOrigin>,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    validate_operation_id(operation_id)?;
    let keys = installation_keys(source, target)?;
    let profile_lock = lock_with_mode(
        &private_updates(state_dir)?.join("profile.lock"),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let locks = lock_scope(global, source, target, &keys)?;
    let _gate = lock(&global.join("gate.lock"))?;
    for origin in std::iter::once(source).chain(target) {
        reject_legacy_marker(global, origin)?;
    }
    let entries = scopes(global)?;
    let data = scope_for(&entries, operation_id, state_dir)?;
    if &data.source != source || data.target.as_ref() != target || data.keys != keys {
        return Err("recovery full installation scope does not match".into());
    }
    for path in std::iter::once(profile_marker(state_dir)?)
        .chain(keys.iter().map(|key| marker_path(global, key)))
    {
        if let Some(marker) = marker_at(&path)? {
            if !compatible_marker(&marker, data) {
                return Err("recovery marker conflicts with durable installation scope".into());
            }
        }
    }
    write_markers(global, data)?;
    Ok(InstallationReservation {
        _locks: locks,
        _profile_lock: profile_lock,
        source: source.clone(),
        target: target.cloned(),
        operation_id: operation_id.into(),
        state_dir: state_dir.into(),
        global_dir: global.into(),
        control_token: data.control_token.clone(),
        stop_generation: data.stop_generation,
    })
}

impl InstallationReservation {
    fn checked_scope<'a>(&self, entries: &'a [Reservation]) -> Result<&'a Reservation, String> {
        let data = scope_for(entries, &self.operation_id, &self.state_dir)?;
        if data.source != self.source
            || data.target != self.target
            || data.control_token != self.control_token
            || data.stop_generation != self.stop_generation
        {
            return Err("reservation full-scope ownership mismatch".into());
        }
        Ok(data)
    }
    fn selected(&self, origin: &InstallOrigin) -> Result<(), String> {
        if !selectable_origin(&self.source, self.target.as_ref(), origin) {
            return Err("startup origin is outside reserved installations".into());
        }
        Ok(())
    }
    pub fn control_token(&self) -> &str {
        &self.control_token
    }
    pub fn user_stop_requested(&self) -> Result<bool, String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        Ok(generation(&self.state_dir)? != self.stop_generation)
    }
    pub fn issued_start_token_for(&self, origin: &InstallOrigin) -> Result<Option<String>, String> {
        self.selected(origin)?;
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        if generation(&self.state_dir)? != self.stop_generation {
            return Err("user stop superseded updater start".into());
        }
        let entries = scopes(&self.global_dir)?;
        let data = self.checked_scope(&entries)?;
        if !markers_match(&self.global_dir, data)? {
            return Err("startup reservation scopes do not match".into());
        }
        if data
            .startup_origin
            .as_ref()
            .is_some_and(|selected| selected != origin)
        {
            return Err("startup authorization belongs to another installation".into());
        }
        Ok(data.startup_token.clone())
    }
    /// The selected candidate or verified original is the only admissible start.
    pub fn authorize_start_for(&self, origin: &InstallOrigin) -> Result<String, String> {
        self.selected(origin)?;
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        if generation(&self.state_dir)? != self.stop_generation {
            return Err("user stop superseded updater start".into());
        }
        let mut entries = scopes(&self.global_dir)?;
        let data = self.checked_scope(&entries)?;
        if !markers_match(&self.global_dir, data)? {
            return Err("startup reservation scopes do not match".into());
        }
        if data.startup_token.is_some() {
            return Err("startup authorization already issued".into());
        }
        let token = new_token()?;
        let index = entries
            .iter()
            .position(|entry| {
                entry.operation_id == self.operation_id && entry.state_dir == self.state_dir
            })
            .ok_or("operation has no durable installation scope")?;
        entries[index].startup_origin = Some(origin.clone());
        entries[index].startup_token = Some(token.clone());
        // Atomic index first: a partial authorization cannot admit either root.
        write_scopes(&self.global_dir, &entries)?;
        write_markers(&self.global_dir, &entries[index])?;
        Ok(token)
    }
    /// Release only after durable final result and reconciled installation state.
    pub fn release(self) -> Result<(), String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        let mut entries = scopes(&self.global_dir)?;
        let data = self.checked_scope(&entries)?;
        let paths = std::iter::once(profile_marker(&self.state_dir)?)
            .chain(
                data.keys
                    .iter()
                    .map(|key| marker_path(&self.global_dir, key)),
            )
            .collect::<Vec<_>>();
        for path in &paths {
            if let Some(marker) = marker_at(path)? {
                if !compatible_marker(&marker, &data) {
                    return Err("release marker conflicts with durable installation scope".into());
                }
            }
        }
        for path in &paths {
            if !missing_file(path) {
                fs::remove_file(path).map_err(|e| e.to_string())?;
                sync_parent(path)?;
            }
        }
        entries.retain(|entry| {
            entry.operation_id != self.operation_id || entry.state_dir != self.state_dir
        });
        write_scopes(&self.global_dir, &entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, InstallOrigin) {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global");
        let state = dir.path().join("profile");
        fs::create_dir(&global).unwrap();
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&global, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let origin = InstallOrigin::Local {
            root: dir.path().join("checkout"),
        };
        (dir, global, state, origin)
    }

    fn herdr_source(state: &Path) -> InstallOrigin {
        let root = state.parent().unwrap();
        InstallOrigin::Herdr {
            host: root.join("host"),
            host_config_dir: root.join("host-config"),
            checkout_root: root.join("checkout"),
            plugin_root: root.join("checkout/plugin"),
            source: "owner/repository".into(),
            requested_ref: Some("beta".into()),
            resolved_commit: Some("a".repeat(40)),
            enabled: true,
        }
    }

    fn herdr_commit(source: &InstallOrigin, commit: &str) -> InstallOrigin {
        let mut next = source.clone();
        if let InstallOrigin::Herdr {
            resolved_commit, ..
        } = &mut next
        {
            *resolved_commit = Some(commit.into());
        }
        next
    }

    #[test]
    fn targetless_herdr_selects_exact_actual_commit_and_admits_only_its_token() {
        let (_dir, global, state, _) = fixture();
        let source = herdr_source(&state);
        let candidate = herdr_commit(&source, &"b".repeat(40));
        let other_commit = herdr_commit(&source, &"c".repeat(40));
        let op = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        let token = lease.authorize_start_for(&candidate).unwrap();
        assert_eq!(
            lease.issued_start_token_for(&candidate).unwrap(),
            Some(token.clone())
        );
        assert!(lease.issued_start_token_for(&source).is_err());
        assert!(lease.authorize_start_for(&other_commit).is_err());
        assert!(lease.authorize_start_for(&source).is_err());
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: true,
            })
        );
        start_allowed_at(&global, &candidate, &state, Some(0), Some(&token)).unwrap();
        for origin in [&source, &other_commit] {
            assert!(start_allowed_at(&global, origin, &state, Some(0), Some(&token)).is_err());
        }
        assert!(start_allowed_at(&global, &candidate, &state, None, Some(&token)).is_err());
        assert!(start_allowed_at(&global, &candidate, &state, Some(0), None).is_err());
        assert!(start_allowed_at(&global, &candidate, &state, Some(0), Some("wrong")).is_err());
        lease.release().unwrap();

        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        let old_token = lease.authorize_start_for(&source).unwrap();
        assert_eq!(
            lease.issued_start_token_for(&source).unwrap(),
            Some(old_token.clone())
        );
        assert!(lease.issued_start_token_for(&candidate).is_err());
        assert!(lease.authorize_start_for(&candidate).is_err());
        assert!(start_allowed_at(&global, &candidate, &state, Some(0), Some(&old_token)).is_err());
        lease.release().unwrap();
    }

    #[test]
    fn targetless_herdr_rejects_scope_drift_invalid_commit_and_explicit_target_delta() {
        let (_dir, global, state, _) = fixture();
        let source = herdr_source(&state);
        let candidate = herdr_commit(&source, &"b".repeat(40));
        let op = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        let mut mutations = Vec::new();
        for field in 0..7 {
            let mut drift = candidate.clone();
            if let InstallOrigin::Herdr {
                host,
                host_config_dir,
                checkout_root,
                plugin_root,
                source,
                requested_ref,
                enabled,
                ..
            } = &mut drift
            {
                match field {
                    0 => *host = host.with_file_name("other-host"),
                    1 => *host_config_dir = host_config_dir.with_file_name("other-config"),
                    2 => *checkout_root = checkout_root.with_file_name("other-checkout"),
                    3 => *plugin_root = plugin_root.with_file_name("other-plugin"),
                    4 => *source = "other/repository".into(),
                    5 => *requested_ref = Some("other-ref".into()),
                    _ => *enabled = false,
                }
            }
            mutations.push(drift);
        }
        mutations.push(herdr_commit(&source, "not-a-commit"));
        mutations.push(herdr_commit(&source, &"g".repeat(40)));
        let mut no_commit = candidate.clone();
        if let InstallOrigin::Herdr {
            resolved_commit, ..
        } = &mut no_commit
        {
            *resolved_commit = None;
        }
        mutations.push(no_commit);
        for drift in &mutations {
            assert!(lease.authorize_start_for(drift).is_err(), "{drift:?}");
        }
        let token = lease.authorize_start_for(&candidate).unwrap();
        for drift in &mutations {
            assert!(start_allowed_at(&global, drift, &state, Some(0), Some(&token)).is_err());
        }
        lease.release().unwrap();

        let lease = reserve_install_at(&global, &source, Some(&candidate), &state, op).unwrap();
        assert!(lease
            .authorize_start_for(&herdr_commit(&source, &"c".repeat(40)))
            .is_err());
        let explicit_token = lease.authorize_start_for(&candidate).unwrap();
        start_allowed_at(&global, &candidate, &state, Some(0), Some(&explicit_token)).unwrap();
        lease.release().unwrap();
    }

    #[test]
    fn snapshot_observes_index_before_markers_without_repair_then_reuses_token() {
        let (_dir, global, state, _) = fixture();
        let source = herdr_source(&state);
        let candidate = herdr_commit(&source, &"b".repeat(40));
        let op = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            None
        );
        assert!(!state.join("updates").exists());
        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: None,
                markers_complete: true,
            })
        );
        let mut entries = scopes(&global).unwrap();
        let token = new_token().unwrap();
        entries[0].startup_origin = Some(candidate.clone());
        entries[0].startup_token = Some(token.clone());
        write_scopes(&global, &entries).unwrap();
        let profile = profile_marker(&state).unwrap();
        let root_marker = marker_path(&global, &installation_key(&source).unwrap());
        let old_profile = fs::read(&profile).unwrap();
        fs::remove_file(&root_marker).unwrap();
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: false,
            })
        );
        assert_eq!(fs::read(&profile).unwrap(), old_profile);
        assert!(!root_marker.exists());
        fs::remove_file(&profile).unwrap();
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: false,
            })
        );
        assert!(!profile.exists());
        assert!(start_allowed_at(&global, &candidate, &state, Some(0), Some(&token)).is_err());
        drop(lease);
        let recovered = recover_reservation_at(&global, &source, None, &state, op).unwrap();
        assert_eq!(
            recovered.issued_start_token_for(&candidate).unwrap(),
            Some(token.clone())
        );
        assert!(recovered.authorize_start_for(&candidate).is_err());
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: true,
            })
        );
        start_allowed_at(&global, &candidate, &state, Some(0), Some(&token)).unwrap();
        recovered.release().unwrap();
    }

    #[test]
    fn snapshot_rejects_conflicting_or_malformed_markers_and_stop_preserves_selection() {
        let (_dir, global, state, _) = fixture();
        let source = herdr_source(&state);
        let candidate = herdr_commit(&source, &"b".repeat(40));
        let other = herdr_commit(&source, &"c".repeat(40));
        let op = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        let token = lease.authorize_start_for(&candidate).unwrap();
        drop(lease);
        record_user_stop_at(&global, &state).unwrap();
        let profile = profile_marker(&state).unwrap();
        let original = fs::read(&profile).unwrap();
        let mut conflicting = scopes(&global).unwrap().remove(0);
        conflicting.startup_origin = Some(other);
        let bytes = serde_json::to_vec(&conflicting).unwrap();
        write_private_bytes(&profile, &bytes, 0o600).unwrap();
        assert!(reservation_snapshot_at(&global, &source, None, &state, op).is_err());
        assert!(recover_reservation_at(&global, &source, None, &state, op).is_err());
        assert_eq!(fs::read(&profile).unwrap(), bytes);
        write_private_bytes(&profile, b"{malformed", 0o600).unwrap();
        assert!(reservation_snapshot_at(&global, &source, None, &state, op).is_err());
        assert!(recover_reservation_at(&global, &source, None, &state, op).is_err());
        assert_eq!(fs::read(&profile).unwrap(), b"{malformed");
        write_private_bytes(&profile, &original, 0o600).unwrap();
        let recovered = recover_reservation_at(&global, &source, None, &state, op).unwrap();
        assert!(recovered.user_stop_requested().unwrap());
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: true,
            })
        );
        assert!(recovered.issued_start_token_for(&candidate).is_err());
        assert!(recovered.authorize_start_for(&candidate).is_err());
        assert!(start_allowed_at(&global, &candidate, &state, Some(0), Some(&token)).is_err());
        recovered.release().unwrap();
    }

    #[test]
    fn user_stop_allows_explicit_partial_marker_recovery_and_safe_release() {
        let (_dir, global, state, _) = fixture();
        let source = herdr_source(&state);
        let candidate = herdr_commit(&source, &"b".repeat(40));
        let op = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, None, &state, op).unwrap();
        let token = lease.authorize_start_for(&candidate).unwrap();
        let generation = lease.stop_generation;
        drop(lease);
        let profile = profile_marker(&state).unwrap();
        let original = fs::read(&profile).unwrap();
        let root_marker = marker_path(&global, &installation_key(&source).unwrap());
        fs::remove_file(&root_marker).unwrap();
        record_user_stop_at(&global, &state).unwrap();
        let selection = Some(ReservationSnapshot {
            startup_origin: Some(candidate.clone()),
            markers_complete: false,
        });
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            selection
        );
        assert_eq!(fs::read(&profile).unwrap(), original);
        assert!(!root_marker.exists());
        let recovered = recover_reservation_at(&global, &source, None, &state, op).unwrap();
        assert!(recovered.user_stop_requested().unwrap());
        assert_eq!(
            reservation_snapshot_at(&global, &source, None, &state, op).unwrap(),
            Some(ReservationSnapshot {
                startup_origin: Some(candidate.clone()),
                markers_complete: true,
            })
        );
        assert_eq!(
            scopes(&global).unwrap()[0].startup_origin,
            Some(candidate.clone())
        );
        assert!(recovered.issued_start_token_for(&candidate).is_err());
        assert!(recovered.authorize_start_for(&candidate).is_err());
        assert!(
            start_allowed_at(&global, &candidate, &state, Some(generation), Some(&token)).is_err()
        );
        recovered.release().unwrap();
        assert!(!reservation_exists_at(&global, &source, None, &state, op).unwrap());
    }

    #[test]
    fn snapshot_never_treats_orphan_legacy_or_invalid_index_as_absence() {
        let (_dir, global, state, origin) = fixture();
        let op = "0123456789abcdef0123456789abcdef";
        let marker = marker_path(&global, &installation_key(&origin).unwrap());
        write_private_bytes(&marker, b"orphan marker", 0o600).unwrap();
        assert!(reservation_snapshot_at(&global, &origin, None, &state, op).is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"orphan marker");
        fs::remove_file(&marker).unwrap();

        let index = scopes_path(&global);
        write_private_bytes(&index, b"malformed index", 0o600).unwrap();
        assert!(reservation_snapshot_at(&global, &origin, None, &state, op).is_err());
        assert_eq!(fs::read(&index).unwrap(), b"malformed index");
        fs::remove_file(&index).unwrap();

        let prefix = state.parent().unwrap().join("brew");
        let brew = InstallOrigin::Homebrew {
            brew: prefix.join("bin/brew"),
            prefix: prefix.clone(),
            cellar_formula_root: prefix.join("Cellar/herdr-desktop-pet"),
            formula: "hanbong5938/tap/herdr-desktop-pet".into(),
            locator: prefix.join("opt/herdr-desktop-pet/libexec/app"),
        };
        let legacy = marker_path(&global, &legacy_key(&brew).unwrap());
        write_private_bytes(&legacy, b"legacy evidence", 0o600).unwrap();
        assert!(reservation_snapshot_at(&global, &brew, None, &state, op).is_err());
        assert_eq!(fs::read(legacy).unwrap(), b"legacy evidence");
    }

    #[test]
    fn reservation_blocks_other_profiles_and_user_stop_defeats_authorized_start() {
        let (_dir, global, state, origin) = fixture();
        let unknown = InstallOrigin::Unknown {
            reason: "no receipt".into(),
        };
        start_allowed_at(&global, &unknown, &state, None, None).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let reservation = reserve_install_at(&global, &origin, None, &state, id).unwrap();
        let different_install = InstallOrigin::Local {
            root: state.parent().unwrap().join("other-checkout"),
        };
        assert!(start_allowed_at(&global, &different_install, &state, None, None).is_err());
        assert!(reserve_install_at(
            &global,
            &different_install,
            None,
            &state,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        assert!(start_allowed_at(&global, &unknown, &state, None, None).is_err());
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(verify_control_at(&global, &origin, None, &state, id, "invalid").is_err());
        verify_control_at(
            &global,
            &origin,
            None,
            &state,
            id,
            reservation.control_token(),
        )
        .unwrap();
        let startup = reservation.authorize_start_for(&origin).unwrap();
        assert!(
            start_allowed_at(&global, &different_install, &state, Some(0), Some(&startup)).is_err()
        );
        start_allowed_at(
            &global,
            &origin,
            &state,
            Some(reservation.stop_generation),
            Some(&startup),
        )
        .unwrap();
        let other = state.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(start_allowed_at(&global, &origin, &other, Some(0), Some(&startup)).is_err());
        record_user_stop_at(&global, &state).unwrap();
        assert!(reservation.user_stop_requested().unwrap());
        assert!(reservation.issued_start_token_for(&origin).is_err());
        assert!(start_allowed_at(&global, &origin, &state, Some(0), Some(&startup)).is_err());
        assert!(verify_control_at(
            &global,
            &origin,
            None,
            &state,
            id,
            reservation.control_token()
        )
        .is_err());
        reservation.release().unwrap();
        start_allowed_at(&global, &origin, &state, None, None).unwrap();
    }

    #[test]
    fn other_live_profile_prevents_physical_replacement() {
        let (_dir, global, state, origin) = fixture();
        let other = state.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        register_active_at(
            &global,
            &origin,
            &other,
            std::process::id(),
            "other-instance",
            None,
            None,
        )
        .unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        assert!(reserve_install_at(&global, &origin, None, &state, id).is_err());
        assert!(active(&global, &installation_key(&origin).unwrap())
            .unwrap()
            .iter()
            .any(|e| e.state_dir == other));
    }

    #[test]
    fn crashed_owner_keeps_reservation_until_explicit_recovery() {
        let (_dir, global, state, origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &origin, None, &state, id).unwrap();
        lease.authorize_start_for(&origin).unwrap();
        drop(lease);
        fs::remove_file(global.join(format!(
            "{}.reservation.json",
            installation_key(&origin).unwrap()
        )))
        .unwrap();
        let different_install = InstallOrigin::Local {
            root: state.parent().unwrap().join("other-checkout"),
        };
        assert!(recover_reservation_at(&global, &different_install, None, &state, id).is_err());
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(recover_reservation_at(
            &global,
            &origin,
            None,
            &state,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        let recovered = recover_reservation_at(&global, &origin, None, &state, id).unwrap();
        let startup = recovered.issued_start_token_for(&origin).unwrap().unwrap();
        start_allowed_at(&global, &origin, &state, Some(0), Some(&startup)).unwrap();
        assert!(recovered.authorize_start_for(&origin).is_err());
        recovered.release().unwrap();
        start_allowed_at(&global, &origin, &state, None, None).unwrap();
    }

    #[test]
    fn partial_start_authorization_can_resume_without_second_issue() {
        let (_dir, global, state, origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &origin, None, &state, id).unwrap();
        lease.authorize_start_for(&origin).unwrap();
        let profile = profile_marker(&state).unwrap();
        let mut interrupted: Reservation =
            serde_json::from_slice(&owned_private_file(&profile, 16384).unwrap()).unwrap();
        interrupted.startup_origin = None;
        interrupted.startup_token = None;
        write_private_bytes(&profile, &serde_json::to_vec(&interrupted).unwrap(), 0o600).unwrap();
        assert!(start_allowed_at(&global, &origin, &state, Some(0), None).is_err());
        drop(lease);
        let recovered = recover_reservation_at(&global, &origin, None, &state, id).unwrap();
        let token = recovered.issued_start_token_for(&origin).unwrap().unwrap();
        start_allowed_at(&global, &origin, &state, Some(0), Some(&token)).unwrap();
        assert!(recovered.authorize_start_for(&origin).is_err());
        recovered.release().unwrap();
    }

    #[test]
    fn reconnect_never_reports_old_success_over_reserved_work() {
        let (_dir, global, state, origin) = fixture();
        assert!(latest_operation(&state).unwrap().is_none());
        assert!(!state.join("updates").exists());
        let completed = OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: "0123456789abcdef0123456789abcdef".into(),
            phase: OperationPhase::Completed,
            execution_fence: ExecutionFence::ManagerExited,
            detail: "previous operation".into(),
            installed: true,
            applied: true,
            candidate: None,
        };
        write_operation(&state, &completed).unwrap();
        let id = "ffffffffffffffffffffffffffffffff";
        let lease = reserve_install_at(&global, &origin, None, &state, id).unwrap();
        assert!(latest_operation(&state).is_err());
        let mut active = OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: id.into(),
            phase: OperationPhase::Preparing,
            execution_fence: ExecutionFence::NoSpawn,
            detail: "new operation".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        write_operation(&state, &active).unwrap();
        write_operation(&state, &completed).unwrap();
        assert_eq!(
            latest_operation(&state).unwrap().unwrap().phase,
            OperationPhase::Preparing
        );
        active.phase = OperationPhase::Failed;
        active.detail = "preparation rejected before mutation".into();
        write_operation(&state, &active).unwrap();
        lease.release().unwrap();
        assert_eq!(
            latest_operation(&state).unwrap().unwrap().phase,
            OperationPhase::Failed
        );
    }

    #[test]
    fn private_journal_rejects_symlink_in_parent_and_target() {
        let (_dir, _global, state, _origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let record = OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: id.into(),
            phase: OperationPhase::Unknown,
            execution_fence: ExecutionFence::NoSpawn,
            detail: "recover".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        let outside = state.parent().unwrap().join("outside");
        fs::write(&outside, b"keep").unwrap();
        symlink(&outside, state.join("updates")).unwrap();
        assert!(write_operation(&state, &record).is_err());
        fs::remove_file(state.join("updates")).unwrap();
        private_updates(&state).unwrap();
        symlink(
            &outside,
            state.join("updates").join(format!("{id}.result.json")),
        )
        .unwrap();
        assert!(read_operation(&state, id).is_err());
        assert!(write_operation(&state, &record).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"keep");
    }

    #[test]
    fn immutable_plan_file_cannot_be_replaced_by_same_operation() {
        let (_dir, _global, state, _origin) = fixture();
        let file = private_updates(&state).unwrap().join("plan.json");
        write_private_new(&file, b"original", 0o600).unwrap();
        assert!(write_private_new(&file, b"changed", 0o600).is_err());
        assert_eq!(fs::read(file).unwrap(), b"original");
    }

    #[test]
    fn operation_lease_is_nonblocking_and_retained_until_release() {
        let (_dir, _global, state, _origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let held = lock_operation(&state, id).unwrap();
        assert!(lock_operation(&state, id).is_err());
        drop(held);
        lock_operation(&state, id).unwrap();
    }

    #[test]
    fn formula_reservations_cover_all_kegs_but_not_other_formula() {
        let (_dir, global, state, _) = fixture();
        let prefix = state.parent().unwrap().join("brew");
        let stable = InstallOrigin::Homebrew {
            brew: prefix.join("bin/brew"),
            prefix: prefix.clone(),
            cellar_formula_root: prefix.join("Cellar/herdr-desktop-pet"),
            formula: "hanbong5938/tap/herdr-desktop-pet".into(),
            locator: prefix.join("opt/herdr-desktop-pet/libexec/app"),
        };
        let mut next_keg = stable.clone();
        if let InstallOrigin::Homebrew { locator, .. } = &mut next_keg {
            *locator = locator.with_file_name("next-keg");
        }
        let beta = InstallOrigin::Homebrew {
            brew: prefix.join("bin/brew"),
            prefix: prefix.clone(),
            cellar_formula_root: prefix.join("Cellar/herdr-desktop-pet-beta"),
            formula: "hanbong5938/tap/herdr-desktop-pet-beta".into(),
            locator: prefix.join("opt/herdr-desktop-pet-beta/libexec/app"),
        };
        let other = state.parent().unwrap().join("other-profile");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            installation_key(&stable).unwrap(),
            installation_key(&next_keg).unwrap()
        );
        assert_ne!(
            installation_key(&stable).unwrap(),
            installation_key(&beta).unwrap()
        );
        let stable_lease = reserve_install_at(
            &global,
            &stable,
            None,
            &state,
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert!(reserve_install_at(
            &global,
            &next_keg,
            None,
            &other,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        let beta_lease = reserve_install_at(
            &global,
            &beta,
            None,
            &other,
            "ffffffffffffffffffffffffffffffff",
        )
        .unwrap();
        beta_lease.release().unwrap();
        stable_lease.release().unwrap();
    }

    #[test]
    fn dual_scope_blocks_both_roots_and_only_selected_origin_can_start() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("beta-checkout"),
        };
        let other = state.parent().unwrap().join("other-profile");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, Some(&target), &state, id).unwrap();
        let entries = scopes(&global).unwrap();
        assert_eq!(
            entries[0].keys,
            installation_keys(&source, Some(&target)).unwrap()
        );
        assert_eq!(entries[0].source, source);
        assert_eq!(entries[0].target.as_ref(), Some(&target));
        assert!(start_allowed_at(&global, &source, &other, None, None).is_err());
        assert!(start_allowed_at(&global, &target, &other, None, None).is_err());
        assert!(reserve_install_at(&global, &target, Some(&source), &other, id).is_err());
        assert!(
            verify_control_at(&global, &source, None, &state, id, lease.control_token()).is_err()
        );
        verify_control_at(
            &global,
            &source,
            Some(&target),
            &state,
            id,
            lease.control_token(),
        )
        .unwrap();
        let token = lease.authorize_start_for(&target).unwrap();
        assert!(lease.issued_start_token_for(&source).is_err());
        assert!(start_allowed_at(&global, &source, &state, Some(0), Some(&token)).is_err());
        start_allowed_at(&global, &target, &state, Some(0), Some(&token)).unwrap();
        lease.release().unwrap();
        assert!(start_allowed_at(&global, &target, &state, Some(0), Some(&token)).is_err());
        start_allowed_at(&global, &source, &other, None, None).unwrap();
        start_allowed_at(&global, &target, &other, None, None).unwrap();
    }

    #[test]
    fn dual_scope_restore_selects_original_without_unblocking_target() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("beta-target"),
        };
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, Some(&target), &state, id).unwrap();
        let token = lease.authorize_start_for(&source).unwrap();
        start_allowed_at(&global, &source, &state, Some(0), Some(&token)).unwrap();
        assert!(start_allowed_at(&global, &target, &state, Some(0), Some(&token)).is_err());
        record_user_stop_at(&global, &state).unwrap();
        assert!(start_allowed_at(&global, &source, &state, Some(0), Some(&token)).is_err());
        lease.release().unwrap();
    }

    #[test]
    fn live_profile_on_proposed_target_prevents_replacement() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("beta-checkout"),
        };
        let other = state.parent().unwrap().join("other-profile");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        register_active_at(
            &global,
            &target,
            &other,
            std::process::id(),
            "target-instance",
            None,
            None,
        )
        .unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        assert!(reserve_install_at(&global, &source, Some(&target), &state, id).is_err());
        assert!(scopes(&global).unwrap().is_empty());
        assert!(!profile_marker(&state).unwrap().exists());
    }

    #[test]
    fn same_key_dedup_still_requires_exact_selected_origin() {
        let (_dir, global, state, _) = fixture();
        let root = state.parent().unwrap().join("brew");
        let source = InstallOrigin::Homebrew {
            brew: root.join("bin/brew"),
            prefix: root.clone(),
            cellar_formula_root: root.join("Cellar/herdr"),
            formula: "hanbong5938/tap/herdr-desktop-pet".into(),
            locator: root.join("opt/herdr/old"),
        };
        let mut target = source.clone();
        if let InstallOrigin::Homebrew { locator, .. } = &mut target {
            *locator = root.join("opt/herdr/new");
        }
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, Some(&target), &state, id).unwrap();
        assert_eq!(scopes(&global).unwrap()[0].keys.len(), 1);
        let token = lease.authorize_start_for(&target).unwrap();
        assert!(start_allowed_at(&global, &source, &state, Some(0), Some(&token)).is_err());
        start_allowed_at(&global, &target, &state, Some(0), Some(&token)).unwrap();
        lease.release().unwrap();
    }

    #[test]
    fn partial_dual_markers_keep_both_roots_gated_and_recover_from_index() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("target"),
        };
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, Some(&target), &state, id).unwrap();
        drop(lease);
        fs::remove_file(profile_marker(&state).unwrap()).unwrap();
        fs::remove_file(marker_path(&global, &installation_key(&target).unwrap())).unwrap();
        fs::remove_file(marker_path(&global, &installation_key(&source).unwrap())).unwrap();
        assert!(reservation_exists_at(&global, &source, Some(&target), &state, id).unwrap());
        let other = state.parent().unwrap().join("other-profile");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        for origin in [&source, &target] {
            assert!(start_allowed_at(&global, origin, &state, None, None).is_err());
            assert!(start_allowed_at(&global, origin, &other, None, None).is_err());
            assert!(reserve_install_at(&global, origin, None, &other, id).is_err());
        }
        assert!(recover_reservation_at(&global, &source, None, &state, id).is_err());
        let recovered =
            recover_reservation_at(&global, &source, Some(&target), &state, id).unwrap();
        assert!(markers_match(&global, &scopes(&global).unwrap()[0]).unwrap());
        recovered.release().unwrap();
        assert!(!reservation_exists_at(&global, &source, Some(&target), &state, id).unwrap());
    }

    #[test]
    fn partial_dual_authorization_never_admits_until_recovered() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("target"),
        };
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &source, Some(&target), &state, id).unwrap();
        let token = lease.authorize_start_for(&target).unwrap();
        fs::remove_file(marker_path(&global, &installation_key(&source).unwrap())).unwrap();
        assert!(start_allowed_at(&global, &target, &state, Some(0), Some(&token)).is_err());
        drop(lease);
        let recovered =
            recover_reservation_at(&global, &source, Some(&target), &state, id).unwrap();
        assert_eq!(
            recovered.issued_start_token_for(&target).unwrap(),
            Some(token.clone())
        );
        assert!(recovered.issued_start_token_for(&source).is_err());
        start_allowed_at(&global, &target, &state, Some(0), Some(&token)).unwrap();
        recovered.release().unwrap();
    }

    #[test]
    fn legacy_brew_key_and_version_one_markers_block_without_rewriting_bytes() {
        let (_dir, global, state, _origin) = fixture();
        let prefix = state.parent().unwrap().join("brew");
        let origin = InstallOrigin::Homebrew {
            brew: prefix.join("bin/brew"),
            prefix: prefix.clone(),
            cellar_formula_root: prefix.join("Cellar/herdr-desktop-pet"),
            formula: "hanbong5938/tap/herdr-desktop-pet".into(),
            locator: prefix.join("opt/herdr-desktop-pet/app"),
        };
        assert_ne!(
            installation_key(&origin).unwrap(),
            legacy_key(&origin).unwrap()
        );
        let old_lock = global.join(format!("{}.lock", legacy_key(&origin).unwrap()));
        let old_owner = lock_with_mode(&old_lock, libc::LOCK_EX | libc::LOCK_NB).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        assert!(reserve_install_at(&global, &origin, None, &state, id).is_err());
        drop(old_owner);
        let lease = reserve_install_at(&global, &origin, None, &state, id).unwrap();
        assert!(lock_with_mode(&old_lock, libc::LOCK_EX | libc::LOCK_NB).is_err());
        lease.release().unwrap();
        let old = global.join(format!("{}.reservation.json", legacy_key(&origin).unwrap()));
        let old_bytes = br#"{"operation_id":"0123456789abcdef0123456789abcdef","version":1}"#;
        write_private_new(&old, old_bytes, 0o600).unwrap();
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(reserve_install_at(&global, &origin, None, &state, id).is_err());
        assert!(recover_reservation_at(&global, &origin, None, &state, id).is_err());
        assert_eq!(fs::read(&old).unwrap(), old_bytes);
        let (_other_dir, other_global, other_state, other_origin) = fixture();
        let marker = profile_marker(&other_state).unwrap();
        write_private_new(&marker, old_bytes, 0o600).unwrap();
        assert!(start_allowed_at(&other_global, &other_origin, &other_state, None, None).is_err());
        assert!(reserve_install_at(&other_global, &other_origin, None, &other_state, id).is_err());
        assert!(latest_operation(&other_state).is_err());
        assert_eq!(fs::read(marker).unwrap(), old_bytes);
    }

    #[test]
    fn version_one_journal_and_corrupt_latest_cannot_be_overwritten() {
        let (_dir, _global, state, _origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let journal = private_updates(&state)
            .unwrap()
            .join(format!("{id}.result.json"));
        let old = br#"{"version":1,"operation_id":"0123456789abcdef0123456789abcdef","phase":"preparing","detail":"old","installed":false,"applied":false,"candidate":null}"#;
        write_private_new(&journal, old, 0o600).unwrap();
        let record = OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: id.into(),
            phase: OperationPhase::Preparing,
            execution_fence: ExecutionFence::NoSpawn,
            detail: "new".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        assert!(write_operation(&state, &record).is_err());
        assert_eq!(fs::read(&journal).unwrap(), old);
        assert!(read_operation(&state, id).is_err());
    }

    #[test]
    fn protocol_two_evidence_stays_blocked_and_byte_preserved() {
        let (_dir, global, state, source) = fixture();
        let target = InstallOrigin::Local {
            root: state.parent().unwrap().join("beta-target"),
        };
        let id = "0123456789abcdef0123456789abcdef";
        let old_marker = br#"{"version":2,"operation_id":"0123456789abcdef0123456789abcdef","installation_key":"legacy","state_dir":"/legacy","stop_generation":0,"control_token":"legacy","startup_token":null}"#;
        let target_marker = marker_path(&global, &installation_key(&target).unwrap());
        write_private_new(&target_marker, old_marker, 0o600).unwrap();
        assert!(start_allowed_at(&global, &target, &state, None, None).is_err());
        assert!(reservation_exists_at(&global, &source, Some(&target), &state, id).is_err());
        assert!(reserve_install_at(&global, &source, Some(&target), &state, id).is_err());
        assert!(recover_reservation_at(&global, &source, Some(&target), &state, id).is_err());
        assert_eq!(fs::read(&target_marker).unwrap(), old_marker);

        let old_journal = br#"{"version":2,"operation_id":"0123456789abcdef0123456789abcdef","phase":"preparing","execution_fence":"no_spawn","detail":"old","installed":false,"applied":false,"candidate":null}"#;
        let journal = private_updates(&state)
            .unwrap()
            .join(format!("{id}.result.json"));
        write_private_new(&journal, old_journal, 0o600).unwrap();
        let record = OperationRecord {
            version: UPDATER_PROTOCOL,
            operation_id: id.into(),
            phase: OperationPhase::Preparing,
            execution_fence: ExecutionFence::NoSpawn,
            detail: "new".into(),
            installed: false,
            applied: false,
            candidate: None,
        };
        assert!(read_operation(&state, id).is_err());
        assert!(write_operation(&state, &record).is_err());
        assert_eq!(fs::read(&journal).unwrap(), old_journal);
    }

    #[test]
    fn writerless_private_fifo_rejects_without_waiting_under_gate() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::time::{Duration, Instant};
        let (_dir, global, state, origin) = fixture();
        let path = private_updates(&state)
            .unwrap()
            .join("stop-generation.json");
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(record_user_stop_at(&global, &state).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
