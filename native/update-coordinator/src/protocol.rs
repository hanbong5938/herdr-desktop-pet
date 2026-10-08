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
pub const UPDATER_PROTOCOL: u32 = 2;

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
        let scope = reservation(&owned_private_file(&marker, 4096)?)?;
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

fn lock_legacy_key(
    global: &Path,
    origin: &InstallOrigin,
    key: &str,
) -> Result<Option<File>, String> {
    legacy_key(origin)
        .filter(|old| old != key)
        .map(|old| {
            lock_with_mode(
                &global.join(format!("{old}.lock")),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        })
        .transpose()
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
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    operation_id: String,
    installation_key: String,
    state_dir: PathBuf,
    stop_generation: u64,
    control_token: String,
    startup_token: Option<String>,
}

fn reservation(bytes: &[u8]) -> Result<Reservation, String> {
    let marker: Reservation = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if marker.version != UPDATER_PROTOCOL {
        return Err("unsupported legacy reservation; preserve original marker".into());
    }
    Ok(marker)
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
    if matches!(origin, InstallOrigin::Unknown { .. }) {
        let current = generation(state_dir)?;
        if expected_generation.is_some_and(|generation| generation != current) {
            return Err("user requested stop after updater prepared startup".into());
        }
        let reservation = profile_marker(state_dir)?;
        if !missing_file(&reservation) {
            owned_private_file(&reservation, 4096)?;
            return Err("unsupported installation cannot start a reserved profile".into());
        }
        return Ok(());
    }
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
    let key = match installation_key(origin) {
        Ok(key) => key,
        Err(_) if missing_file(&profile_path) => return Ok(()),
        Err(error) => return Err(error),
    };
    reject_legacy_marker(global, origin)?;
    if !missing_file(&profile_path) {
        let active = reservation(&owned_private_file(&profile_path, 4096)?)?;
        if active.state_dir != state_dir
            || active.installation_key != key
            || expected_generation != Some(active.stop_generation)
            || updater_token.is_none()
            || updater_token != active.startup_token.as_deref()
        {
            return Err("profile reserved by updater".into());
        }
    }
    let path = global.join(format!("{key}.reservation.json"));
    if !missing_file(&path) {
        let reservation = reservation(&owned_private_file(&path, 4096)?)?;
        if reservation.state_dir != state_dir
            || reservation.installation_key != key
            || expected_generation != Some(reservation.stop_generation)
            || updater_token.is_none()
            || updater_token != reservation.startup_token.as_deref()
        {
            return Err("installation reserved by updater".into());
        }
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

/// Control requests must also match expected live instance and same-UID peer, checked by daemon.
pub fn verify_control(
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
    token: &str,
) -> Result<(), String> {
    verify_control_at(&global_updates()?, origin, state_dir, operation_id, token)
}
fn verify_control_at(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
    token: &str,
) -> Result<(), String> {
    validate_operation_id(operation_id)?;
    let _gate = lock(&global.join("gate.lock"))?;
    reject_legacy_marker(global, origin)?;
    let key = installation_key(origin)?;
    let marker = global.join(format!("{key}.reservation.json"));
    let data = reservation(&owned_private_file(&marker, 4096)?)?;
    if data.operation_id != operation_id
        || data.installation_key != key
        || data.state_dir != state_dir
        || data.control_token != token
        || generation(state_dir)? != data.stop_generation
    {
        return Err("updater request has no valid same-profile reservation".into());
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
/// Durable reservation survives helper death. Only explicit recovery after disk reconciliation
/// releases it; dropping the lock never clears the marker.
pub struct InstallationReservation {
    _lock: File,
    _legacy_lock: Option<File>,
    _profile_lock: File,
    key: String,
    operation_id: String,
    state_dir: PathBuf,
    global_dir: PathBuf,
    control_token: String,
    pub stop_generation: u64,
}
pub fn reserve_install(
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    reserve_install_at(&global_updates()?, origin, state_dir, operation_id)
}
fn reserve_install_at(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    validate_operation_id(operation_id)?;
    check_dir(global, false)?;
    let key = installation_key(origin)?;
    let profile_lock = lock_with_mode(
        &private_updates(state_dir)?.join("profile.lock"),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let install_lock = lock_with_mode(
        &global.join(format!("{key}.lock")),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let legacy_lock = lock_legacy_key(global, origin, &key)?;
    let _gate = lock(&global.join("gate.lock"))?;
    reject_legacy_marker(global, origin)?;
    let marker = global.join(format!("{key}.reservation.json"));
    if !missing_file(&marker) {
        return Err("installation already reserved; inspect/recover previous operation".into());
    }
    let scoped_marker = profile_marker(state_dir)?;
    if !missing_file(&scoped_marker) {
        return Err("profile already reserved; inspect/recover previous operation".into());
    }
    if active_for_origin(global, origin)?
        .iter()
        .any(|e| e.state_dir != state_dir && live(e.pid))
    {
        return Err("another active profile shares the installation".into());
    }
    let stop_generation = generation(state_dir)?;
    let mut random = [0u8; 32];
    use std::io::Read;
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| e.to_string())?;
    let control_token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let data = serde_json::to_vec(&Reservation {
        version: UPDATER_PROTOCOL,
        operation_id: operation_id.into(),
        installation_key: key.clone(),
        state_dir: state_dir.into(),
        stop_generation,
        control_token: control_token.clone(),
        startup_token: None,
    })
    .map_err(|e| e.to_string())?;
    write_private_bytes(&scoped_marker, &data, 0o600)?;
    if let Err(error) = write_private_bytes(&marker, &data, 0o600) {
        let _ = fs::remove_file(&scoped_marker);
        return Err(error);
    }
    Ok(InstallationReservation {
        _lock: install_lock,
        _legacy_lock: legacy_lock,
        _profile_lock: profile_lock,
        key,
        operation_id: operation_id.into(),
        state_dir: state_dir.into(),
        global_dir: global.into(),
        control_token,
        stop_generation,
    })
}
/// Reattach to a marker after coordinator death. This does not infer that disk state is safe:
/// caller must reconcile journal, manager process and actual registered installation.
pub fn recover_reservation(
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    recover_reservation_at(&global_updates()?, origin, state_dir, operation_id)
}
fn recover_reservation_at(
    global: &Path,
    origin: &InstallOrigin,
    state_dir: &Path,
    operation_id: &str,
) -> Result<InstallationReservation, String> {
    validate_operation_id(operation_id)?;
    let key = installation_key(origin)?;
    let profile_lock = lock_with_mode(
        &private_updates(state_dir)?.join("profile.lock"),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let install_lock = lock_with_mode(
        &global.join(format!("{key}.lock")),
        libc::LOCK_EX | libc::LOCK_NB,
    )?;
    let legacy_lock = lock_legacy_key(global, origin, &key)?;
    let _gate = lock(&global.join("gate.lock"))?;
    reject_legacy_marker(global, origin)?;
    let marker = global.join(format!("{key}.reservation.json"));
    let scoped_marker = profile_marker(state_dir)?;
    let scoped_bytes = owned_private_file(&scoped_marker, 4096)?;
    let scoped = reservation(&scoped_bytes)?;
    if scoped.operation_id != operation_id
        || scoped.installation_key != key
        || scoped.state_dir != state_dir
    {
        return Err("profile recovery marker does not match installation".into());
    }
    if !missing_file(&marker) {
        let marker_bytes = owned_private_file(&marker, 4096)?;
        let data = reservation(&marker_bytes)?;
        if data.operation_id != operation_id
            || data.installation_key != key
            || data.state_dir != state_dir
            || data.control_token != scoped.control_token
            || data.stop_generation != scoped.stop_generation
        {
            return Err("recovery scope does not match durable installation reservation".into());
        }
        if data.startup_token != scoped.startup_token {
            if scoped.startup_token.is_some() {
                return Err("startup authorization scopes conflict during recovery".into());
            }
            write_private_bytes(&scoped_marker, &marker_bytes, 0o600)?;
        }
    } else {
        write_private_bytes(&marker, &scoped_bytes, 0o600)?;
    }
    Ok(InstallationReservation {
        _lock: install_lock,
        _legacy_lock: legacy_lock,
        _profile_lock: profile_lock,
        key,
        operation_id: operation_id.into(),
        state_dir: state_dir.into(),
        global_dir: global.into(),
        control_token: scoped.control_token,
        stop_generation: scoped.stop_generation,
    })
}

impl InstallationReservation {
    pub fn control_token(&self) -> &str {
        &self.control_token
    }
    pub fn user_stop_requested(&self) -> Result<bool, String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        Ok(generation(&self.state_dir)? != self.stop_generation)
    }

    /// Explicit recovery may reuse an issued token only while both scopes still agree.
    pub fn issued_start_token(&self) -> Result<Option<String>, String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        if generation(&self.state_dir)? != self.stop_generation {
            return Err("user stop superseded updater start".into());
        }
        let marker = self
            .global_dir
            .join(format!("{}.reservation.json", self.key));
        let data = reservation(&owned_private_file(&marker, 4096)?)?;
        let scoped = reservation(&owned_private_file(
            &profile_marker(&self.state_dir)?,
            4096,
        )?)?;
        if data.operation_id != self.operation_id
            || data.installation_key != self.key
            || data.state_dir != self.state_dir
            || data.control_token != self.control_token
            || data.stop_generation != self.stop_generation
            || scoped.operation_id != data.operation_id
            || scoped.installation_key != data.installation_key
            || scoped.state_dir != data.state_dir
            || scoped.control_token != data.control_token
            || scoped.stop_generation != data.stop_generation
            || scoped.startup_token != data.startup_token
        {
            return Err("startup reservation scopes do not match".into());
        }
        Ok(data.startup_token)
    }
    /// After successful manager validation, authorize exactly this profile's updater startup.
    pub fn authorize_start(&self) -> Result<String, String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        if generation(&self.state_dir)? != self.stop_generation {
            return Err("user stop superseded updater start".into());
        }
        let marker = self
            .global_dir
            .join(format!("{}.reservation.json", self.key));
        let mut data = reservation(&owned_private_file(&marker, 4096)?)?;
        if data.operation_id != self.operation_id
            || data.installation_key != self.key
            || data.state_dir != self.state_dir
        {
            return Err("reservation ownership mismatch".into());
        }
        if data.startup_token.is_some() {
            return Err("startup authorization already issued".into());
        }
        let mut random = [0u8; 32];
        use std::io::Read;
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        data.startup_token = Some(token.clone());
        let bytes = serde_json::to_vec(&data).map_err(|e| e.to_string())?;
        write_private_bytes(&marker, &bytes, 0o600)?;
        write_private_bytes(&profile_marker(&self.state_dir)?, &bytes, 0o600)?;
        Ok(token)
    }
    /// Release only after durable final result and reconciled installation state.
    pub fn release(self) -> Result<(), String> {
        let _gate = lock(&self.global_dir.join("gate.lock"))?;
        let marker = self
            .global_dir
            .join(format!("{}.reservation.json", self.key));
        let scoped_marker = profile_marker(&self.state_dir)?;
        let scoped = reservation(&owned_private_file(&scoped_marker, 4096)?)?;
        if scoped.operation_id != self.operation_id
            || scoped.installation_key != self.key
            || scoped.state_dir != self.state_dir
            || scoped.control_token != self.control_token
        {
            return Err("profile reservation ownership mismatch".into());
        }
        if !missing_file(&marker) {
            let data = reservation(&owned_private_file(&marker, 4096)?)?;
            if data.operation_id != self.operation_id
                || data.installation_key != self.key
                || data.state_dir != self.state_dir
                || data.control_token != scoped.control_token
                || data.stop_generation != scoped.stop_generation
            {
                return Err("installation reservation ownership mismatch".into());
            }
        }
        if !missing_file(&marker) {
            fs::remove_file(&marker).map_err(|e| e.to_string())?;
            sync_parent(&marker)?;
        }
        fs::remove_file(&scoped_marker).map_err(|e| e.to_string())?;
        sync_parent(&scoped_marker)?;
        Ok(())
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

    #[test]
    fn reservation_blocks_other_profiles_and_user_stop_defeats_authorized_start() {
        let (_dir, global, state, origin) = fixture();
        let unknown = InstallOrigin::Unknown {
            reason: "no receipt".into(),
        };
        start_allowed_at(&global, &unknown, &state, None, None).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let reservation = reserve_install_at(&global, &origin, &state, id).unwrap();
        let different_install = InstallOrigin::Local {
            root: state.parent().unwrap().join("other-checkout"),
        };
        assert!(start_allowed_at(&global, &different_install, &state, None, None).is_err());
        assert!(reserve_install_at(
            &global,
            &different_install,
            &state,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        assert!(start_allowed_at(&global, &unknown, &state, None, None).is_err());
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(verify_control_at(&global, &origin, &state, id, "invalid").is_err());
        verify_control_at(&global, &origin, &state, id, reservation.control_token()).unwrap();
        let startup = reservation.authorize_start().unwrap();
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
        assert!(reservation.issued_start_token().is_err());
        assert!(start_allowed_at(&global, &origin, &state, Some(0), Some(&startup)).is_err());
        assert!(
            verify_control_at(&global, &origin, &state, id, reservation.control_token()).is_err()
        );
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
        assert!(reserve_install_at(&global, &origin, &state, id).is_err());
        assert!(active(&global, &installation_key(&origin).unwrap())
            .unwrap()
            .iter()
            .any(|e| e.state_dir == other));
    }

    #[test]
    fn crashed_owner_keeps_reservation_until_explicit_recovery() {
        let (_dir, global, state, origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &origin, &state, id).unwrap();
        lease.authorize_start().unwrap();
        drop(lease);
        fs::remove_file(global.join(format!(
            "{}.reservation.json",
            installation_key(&origin).unwrap()
        )))
        .unwrap();
        let different_install = InstallOrigin::Local {
            root: state.parent().unwrap().join("other-checkout"),
        };
        assert!(recover_reservation_at(&global, &different_install, &state, id).is_err());
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(recover_reservation_at(
            &global,
            &origin,
            &state,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        let recovered = recover_reservation_at(&global, &origin, &state, id).unwrap();
        let startup = recovered.issued_start_token().unwrap().unwrap();
        start_allowed_at(&global, &origin, &state, Some(0), Some(&startup)).unwrap();
        assert!(recovered.authorize_start().is_err());
        recovered.release().unwrap();
        start_allowed_at(&global, &origin, &state, None, None).unwrap();
    }

    #[test]
    fn partial_start_authorization_can_resume_without_second_issue() {
        let (_dir, global, state, origin) = fixture();
        let id = "0123456789abcdef0123456789abcdef";
        let lease = reserve_install_at(&global, &origin, &state, id).unwrap();
        lease.authorize_start().unwrap();
        let profile = profile_marker(&state).unwrap();
        let mut interrupted: Reservation =
            serde_json::from_slice(&owned_private_file(&profile, 4096).unwrap()).unwrap();
        interrupted.startup_token = None;
        write_private_bytes(&profile, &serde_json::to_vec(&interrupted).unwrap(), 0o600).unwrap();
        drop(lease);
        let recovered = recover_reservation_at(&global, &origin, &state, id).unwrap();
        let token = recovered.issued_start_token().unwrap().unwrap();
        start_allowed_at(&global, &origin, &state, Some(0), Some(&token)).unwrap();
        assert!(recovered.authorize_start().is_err());
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
        let lease = reserve_install_at(&global, &origin, &state, id).unwrap();
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
        let stable_lease =
            reserve_install_at(&global, &stable, &state, "0123456789abcdef0123456789abcdef")
                .unwrap();
        assert!(reserve_install_at(
            &global,
            &next_keg,
            &other,
            "ffffffffffffffffffffffffffffffff"
        )
        .is_err());
        let beta_lease =
            reserve_install_at(&global, &beta, &other, "ffffffffffffffffffffffffffffffff").unwrap();
        beta_lease.release().unwrap();
        stable_lease.release().unwrap();
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
        assert!(reserve_install_at(&global, &origin, &state, id).is_err());
        drop(old_owner);
        let lease = reserve_install_at(&global, &origin, &state, id).unwrap();
        assert!(lock_with_mode(&old_lock, libc::LOCK_EX | libc::LOCK_NB).is_err());
        lease.release().unwrap();
        let old = global.join(format!("{}.reservation.json", legacy_key(&origin).unwrap()));
        let old_bytes = br#"{"operation_id":"0123456789abcdef0123456789abcdef","version":1}"#;
        fs::write(&old, old_bytes).unwrap();
        assert!(start_allowed_at(&global, &origin, &state, None, None).is_err());
        assert!(reserve_install_at(&global, &origin, &state, id).is_err());
        assert!(recover_reservation_at(&global, &origin, &state, id).is_err());
        assert_eq!(fs::read(&old).unwrap(), old_bytes);
        let (_other_dir, other_global, other_state, other_origin) = fixture();
        let marker = profile_marker(&other_state).unwrap();
        fs::write(&marker, old_bytes).unwrap();
        assert!(start_allowed_at(&other_global, &other_origin, &other_state, None, None).is_err());
        assert!(reserve_install_at(&other_global, &other_origin, &other_state, id).is_err());
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
