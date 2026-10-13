use crate::helper_ui::{Attention, Phase};
use coordinator::protocol::{
    self, ExecutionFence, InstallationReservation, OperationLease, UpdateCommand,
    UpdateControlReply, UpdateControlRequest,
};
use coordinator::{
    ExecutableIdentity, InstallOrigin, ObservationMode, OperationPhase, OperationRecord,
    UpdatePlan, VerifiedInstallation, OBSERVATION_MODE_ENV,
};
use herdr_update_coordinator as coordinator;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Eq, PartialEq)]
enum StartupPurpose {
    Candidate,
    Original,
}

fn selected_origin(
    snapshot: Option<&protocol::ReservationSnapshot>,
    installation: &VerifiedInstallation,
    purpose: StartupPurpose,
    plan: &UpdatePlan,
) -> Result<(), String> {
    match snapshot.and_then(|scope| scope.startup_origin.as_ref()) {
        Some(selected) if selected == &installation.origin => Ok(()),
        Some(_) => Err("durable startup selection belongs to another installation".into()),
        None if purpose == StartupPurpose::Original && installation.origin == plan.origin => Ok(()),
        None if purpose == StartupPurpose::Candidate => Ok(()),
        None => Err("restored original origin differs from captured original".into()),
    }
}

fn pin_state(
    pinned_image: Option<&ExecutableIdentity>,
    snapshot: Option<&protocol::ReservationSnapshot>,
    installed: &VerifiedInstallation,
    purpose: StartupPurpose,
    plan: &UpdatePlan,
) -> Result<(), String> {
    selected_origin(snapshot, installed, purpose, plan)?;
    if purpose == StartupPurpose::Candidate {
        match (
            pinned_image,
            snapshot.and_then(|scope| scope.startup_origin.as_ref()),
        ) {
            (Some(image), Some(_)) if image == &installed.image => Ok(()),
            (Some(_), Some(_)) => Err("installed candidate differs from durable image pin".into()),
            (Some(_), None) => {
                Err("candidate image has no durable paired origin; reservation retained".into())
            }
            (None, Some(_)) => Err(
                "selected candidate origin has no durable paired image; reservation retained"
                    .into(),
            ),
            (None, None) => Ok(()),
        }
    } else {
        Ok(())
    }
}

// A historical candidate image is not a candidate origin pin. A separately
// verified original may reconcile it only before a candidate was selected, or
// while resuming an original (never candidate) restoration.
fn historical_original_allowed(
    plan: &UpdatePlan,
    record: &OperationRecord,
    scope: Option<&protocol::ReservationSnapshot>,
) -> bool {
    let Some(image) = record.candidate.as_ref() else {
        return false;
    };
    match scope.and_then(|scope| scope.startup_origin.as_ref()) {
        None => {
            // An unselected reproducible Local candidate can be byte-for-byte A.
            // Equality is not evidence that its purpose was Original.
            !matches!(plan.action, coordinator::UpdateAction::LocalRebuild)
                || image != &plan.running
        }
        Some(selected) => {
            selected == &plan.origin
                && record.phase == OperationPhase::Failed
                && !record.installed
                && !record.applied
        }
    }
}

fn historical_original(
    plan: &UpdatePlan,
    record: &OperationRecord,
    scope: Option<&protocol::ReservationSnapshot>,
) -> Option<VerifiedInstallation> {
    historical_original_allowed(plan, record, scope).then_some(())?;
    let original = coordinator::restored_original(plan).ok()?;
    selected_origin(scope, &original, StartupPurpose::Original, plan)
        .ok()
        .map(|()| original)
}

fn complete_stopped_candidate(
    image: Option<&ExecutableIdentity>,
    scope: Option<&protocol::ReservationSnapshot>,
    installed: &VerifiedInstallation,
) -> bool {
    image == Some(&installed.image)
        && scope.is_some_and(|scope| {
            scope.markers_complete && scope.startup_origin.as_ref() == Some(&installed.origin)
        })
}

fn snapshot(plan: &UpdatePlan) -> Result<Option<protocol::ReservationSnapshot>, String> {
    protocol::reservation_snapshot(
        &plan.origin,
        plan.target.as_ref().map(|target| &target.origin),
        &plan.context.state_dir,
        &plan.operation_id,
    )
}

fn recheck_stopped_candidate(
    plan: &UpdatePlan,
    image: Option<&ExecutableIdentity>,
    installed: &VerifiedInstallation,
) -> Result<(), String> {
    let current = candidate(plan)?;
    if current != *installed
        || !complete_stopped_candidate(image, snapshot(plan)?.as_ref(), &current)
    {
        return Err(
            "stopped candidate image/origin or reservation changed; reservation retained".into(),
        );
    }
    Ok(())
}

const FRAME_LIMIT: u64 = 16384;
// Read-only daemon status uses the native control protocol, not the updater
// Prepare/Stop protocol. The latter is versioned independently.
const NATIVE_STATUS_PROTOCOL: u64 = 1;
const MANAGER_WAIT: Duration = Duration::from_secs(30 * 60);
const START_WAIT: Duration = Duration::from_secs(45);
const STOP_WAIT: Duration = Duration::from_secs(45);
static MANAGER_DRAINING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Deserialize)]
struct Status {
    ok: bool,
    running: bool,
    shutdown: bool,
    ready: bool,
    ui_ready: bool,
    control_ready: bool,
    instance_id: String,
    running_sha256: String,
    pid: u32,
    executable_path: PathBuf,
    config_dir: PathBuf,
    state_dir: PathBuf,
    herdr_socket: PathBuf,
    #[serde(default)]
    observation_mode: Option<ObservationMode>,
    assets_override: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagerEvidence {
    program: PathBuf,
    program_sha256: String,
    // Inherited effective UID and pre-spawn wallclock, NOT kernel-attested process identity.
    // Any still-live recorded PID or PGID (including reuse) conservatively blocks recovery.
    uid: u32,
    started_at_ms: u128,
    pid: Option<u32>,
    pgid: Option<u32>,
    // Persist intent BEFORE spawn: a crash in the spawn/PID journal gap is unrecoverably unknown.
    state: ManagerState,
    exit_code: Option<i32>,
    drained: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ManagerState {
    Intent,
    Running,
    Exited,
    TimedOut,
}
fn group_quiescent(group: Option<u32>) -> Result<bool, String> {
    let group = group.ok_or("manager group identity was never journaled")?;
    if group == 0 || group > i32::MAX as u32 {
        return Err("invalid manager process group".into());
    }
    if unsafe { libc::kill(-(group as i32), 0) } == 0 {
        return Ok(false);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        Some(libc::EPERM) => Ok(false),
        _ => Err("manager process group state cannot be determined".into()),
    }
}
fn manager_quiescent(item: &ManagerEvidence) -> Result<bool, String> {
    let pid = item.pid.ok_or("manager PID identity was never journaled")?;
    Ok(!process_live(pid) && group_quiescent(item.pgid)?)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleEvidence {
    original_pid: u32,
    original_instance_id: String,
    original_image: ExecutableIdentity,
    original_config_dir: PathBuf,
    original_state_dir: PathBuf,
    original_socket: PathBuf,
    original_assets: Option<PathBuf>,
    helper_pid: u32,
    helper_image: ExecutableIdentity,
    stop_generation: u64,
}
fn lifecycle_path(plan: &UpdatePlan) -> Result<PathBuf, String> {
    Ok(protocol::private_updates(&plan.context.state_dir)?
        .join(format!("{}.lifecycle.json", plan.operation_id)))
}
fn lifecycle(plan: &UpdatePlan, live: &Status, stop_generation: u64) -> Result<(), String> {
    let bytes = serde_json::to_vec(&LifecycleEvidence {
        original_pid: live.pid,
        original_instance_id: live.instance_id.clone(),
        original_image: plan.running.clone(),
        original_config_dir: live.config_dir.clone(),
        original_state_dir: live.state_dir.clone(),
        original_socket: live.herdr_socket.clone(),
        original_assets: live.assets_override.clone(),
        helper_pid: std::process::id(),
        helper_image: coordinator::executable_identity(
            &std::env::current_exe().map_err(|e| e.to_string())?,
        )?,
        stop_generation,
    })
    .map_err(|e| e.to_string())?;
    protocol::write_private_bytes(&lifecycle_path(plan)?, &bytes, 0o600)
}
fn read_lifecycle(plan: &UpdatePlan) -> Result<LifecycleEvidence, String> {
    let bytes = coordinator::owned_private_file(&lifecycle_path(plan)?, 4096)?;
    let owner: LifecycleEvidence = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if owner.original_pid == 0
        || owner.original_instance_id != plan.context.instance_id
        || owner.original_image != plan.running
        || owner.original_config_dir != plan.context.config_dir
        || owner.original_state_dir != plan.context.state_dir
        || !same_endpoint(&owner.original_socket, &plan.context.herdr_socket)
        || owner.original_assets != plan.context.assets_override
        || owner.helper_pid == 0
        || owner.helper_image.path
            != fs::canonicalize(
                protocol::private_updates(&plan.context.state_dir)?
                    .join(format!("helper-{}", plan.operation_id)),
            )
            .map_err(|e| e.to_string())?
        || owner.helper_image.sha256.len() != 64
    {
        return Err("original/helper lifecycle evidence does not match operation scope".into());
    }
    Ok(owner)
}
fn same_original(owner: &LifecycleEvidence, live: &Status) -> bool {
    owner.original_pid == live.pid
        && owner.original_instance_id == live.instance_id
        && owner.original_image.path == live.executable_path
        && owner.original_image.sha256 == live.running_sha256
        && owner.original_config_dir == live.config_dir
        && owner.original_state_dir == live.state_dir
        && same_endpoint(&owner.original_socket, &live.herdr_socket)
        && owner.original_assets == live.assets_override
}

fn journal(plan: &UpdatePlan, record: &OperationRecord) -> Result<(), String> {
    protocol::write_operation(&plan.context.state_dir, record).map(|_| ())
}
fn transition(
    plan: &UpdatePlan,
    record: &mut OperationRecord,
    phase: OperationPhase,
    detail: impl Into<String>,
) -> Result<(), String> {
    record.phase = phase;
    record.detail = detail.into();
    journal(plan, record)
}
fn evidence_path(plan: &UpdatePlan) -> Result<PathBuf, String> {
    Ok(protocol::private_updates(&plan.context.state_dir)?
        .join(format!("{}.manager.json", plan.operation_id)))
}
fn evidence(plan: &UpdatePlan, item: &ManagerEvidence) -> Result<(), String> {
    let bytes = serde_json::to_vec(item).map_err(|e| e.to_string())?;
    protocol::write_private_bytes(&evidence_path(plan)?, &bytes, 0o600)
}
fn read_evidence(plan: &UpdatePlan) -> Result<Option<ManagerEvidence>, String> {
    let path = evidence_path(plan)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => (),
    }
    let bytes = coordinator::owned_private_file(&path, 4096)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}
fn manager_artifacts_absent(plan: &UpdatePlan) -> Result<bool, String> {
    let base = protocol::private_updates(&plan.context.state_dir)?;
    for suffix in ["json", "stdout", "stderr"] {
        let path = base.join(format!("{}.manager.{suffix}", plan.operation_id));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
            Ok(_) => return Ok(false),
        }
    }
    Ok(true)
}
fn no_spawn_proved(
    record: &OperationRecord,
    manager: Option<&ManagerEvidence>,
    artifacts_absent: bool,
) -> bool {
    record.execution_fence == ExecutionFence::NoSpawn && manager.is_none() && artifacts_absent
}
fn no_spawn_original_disk_unchanged(plan: &UpdatePlan) -> Result<(), String> {
    // ApplyInstalled pins and rechecks the replacement on disk. The original
    // image is still mapped into its live process, not readable at this path.
    if !matches!(plan.action, coordinator::UpdateAction::ApplyInstalled)
        && coordinator::executable_identity(&plan.running.path)? != plan.running
    {
        return Err("original installation changed since preflight".into());
    }
    Ok(())
}

fn socket(plan: &UpdatePlan) -> PathBuf {
    plan.context.state_dir.join("control.sock")
}
fn exchange<T: Serialize, R: for<'a> Deserialize<'a>>(
    path: &Path,
    request: &T,
    timeout: Duration,
) -> Result<R, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err("control socket is not owned by this user".into());
    }
    let mut stream = UnixStream::connect(path).map_err(|e| format!("control socket: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
            || uid != unsafe { libc::geteuid() }
        {
            return Err("control peer UID differs".into());
        }
    }
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.len() >= FRAME_LIMIT as usize {
        return Err("control request too large".into());
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream
        .take(FRAME_LIMIT)
        .read_to_end_until_newline(&mut response)?;
    serde_json::from_slice(&response).map_err(|e| format!("invalid control reply: {e}"))
}
trait ReadLineBounded: Read {
    fn read_to_end_until_newline(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        let mut byte = [0];
        while out.len() < FRAME_LIMIT as usize {
            self.read_exact(&mut byte)
                .map_err(|e| format!("control response: {e}"))?;
            if byte[0] == b'\n' {
                return Ok(());
            }
            out.push(byte[0]);
        }
        Err("control reply exceeds frame limit".into())
    }
}
impl<T: Read> ReadLineBounded for T {}
fn status(plan: &UpdatePlan) -> Result<Status, String> {
    exchange(
        &socket(plan),
        &serde_json::json!({"version":NATIVE_STATUS_PROTOCOL,"command":"status"}),
        Duration::from_secs(3),
    )
}
fn same_endpoint(actual: &Path, captured: &Path) -> bool {
    if !actual.is_absolute() || !captured.is_absolute() {
        return false;
    }
    if actual == captured {
        return true;
    }
    if let (Ok(actual), Ok(captured)) = (fs::canonicalize(actual), fs::canonicalize(captured)) {
        return actual == captured;
    }
    let canonical_parent = |path: &Path| -> Option<PathBuf> {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    };
    canonical_parent(actual)
        .is_some_and(|actual| canonical_parent(captured).as_ref() == Some(&actual))
}
fn status_mode_matches(plan: &UpdatePlan, response: &Status) -> Result<bool, String> {
    let expected = plan.context.observation_mode()?;
    Ok(response.observation_mode == Some(expected)
        || expected == ObservationMode::Herdr && response.observation_mode.is_none())
}
fn verify_status(plan: &UpdatePlan, response: &Status) -> Result<(), String> {
    let c = &plan.context;
    if !response.ok
        || !response.running
        || response.shutdown
        || response.pid == 0
        || response.instance_id != c.instance_id
        || response.running_sha256 != plan.running.sha256
        || response.executable_path != plan.running.path
        || response.config_dir != c.config_dir
        || response.state_dir != c.state_dir
        || !same_endpoint(&response.herdr_socket, &c.herdr_socket)
        || response.assets_override != c.assets_override
        || !status_mode_matches(plan, response)?
    {
        return Err(
            "live daemon does not match immutable instance/image/profile/assets/observation mode"
                .into(),
        );
    }
    if process_path(response.pid)?.as_deref() != Some(plan.running.path.as_path()) {
        return Err("live daemon process image differs from plan".into());
    }
    Ok(())
}
fn control(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    command: UpdateCommand,
) -> Result<(), String> {
    let request = UpdateControlRequest {
        version: protocol::UPDATER_PROTOCOL,
        kind: "updater".into(),
        command,
        instance_id: plan.context.instance_id.clone(),
        operation_id: plan.operation_id.clone(),
        token: reservation.control_token().into(),
    };
    let reply: UpdateControlReply = exchange(&socket(plan), &request, Duration::from_secs(30))?;
    if reply.version != protocol::UPDATER_PROTOCOL
        || reply.instance_id != plan.context.instance_id
        || reply.operation_id != plan.operation_id
        || !reply.accepted
    {
        return Err(format!(
            "daemon refused update preparation/stop: {}",
            reply.detail
        ));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn process_path(pid: u32) -> Result<Option<PathBuf>, String> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Ok(None);
    }
    let mut buffer = [0i8; 4096];
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
    }
    let len = unsafe { proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if len <= 0 {
        return Ok(None);
    }
    let path = std::ffi::CStr::from_bytes_until_nul(unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len())
    })
    .map_err(|e| e.to_string())?;
    use std::os::unix::ffi::OsStrExt;
    Ok(Some(PathBuf::from(std::ffi::OsStr::from_bytes(
        path.to_bytes(),
    ))))
}
#[cfg(not(target_os = "macos"))]
fn process_path(_pid: u32) -> Result<Option<PathBuf>, String> {
    Ok(None)
}
fn process_live(pid: u32) -> bool {
    pid > 0
        && pid <= i32::MAX as u32
        && (unsafe { libc::kill(pid as i32, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}
fn installation_process_in_scope(path: &Path, root: &Path) -> Result<bool, String> {
    if path
        .file_name()
        .is_none_or(|name| name != "herdr-desktop-pet")
    {
        return Ok(false);
    }
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("live installation process path is not absolute and normalized".into());
    }
    match fs::canonicalize(path) {
        Ok(physical) => Ok(physical.starts_with(root)),
        // An old keg may have been removed while its process still runs.
        // Its kernel-recorded path still identifies the same component-anchored
        // formula subtree; never skip that live installation user.
        Err(_) if path.starts_with(root) => Ok(true),
        Err(error) => Err(format!(
            "cannot resolve live installation process {}: {error}",
            path.display()
        )),
    }
}
// Both sides of a formula/ref switch own live installations until completion.
fn install_roots(plan: &UpdatePlan) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::with_capacity(2);
    for origin in [&plan.origin, plan.target_origin()] {
        let root = coordinator::physical_install_root(origin)?.to_path_buf();
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
}
fn reject_other_active_profiles(plan: &UpdatePlan) -> Result<(), String> {
    let mut keys = Vec::with_capacity(2);
    for origin in [&plan.origin, plan.target_origin()] {
        let key = protocol::installation_key(origin)?;
        if !keys.contains(&key) {
            keys.push(key);
            if !protocol::conflicting_active_profiles(origin, &plan.context.state_dir)?.is_empty() {
                return Err("another profile uses an original or target installation".into());
            }
        }
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn reject_other_install_processes(
    plan: &UpdatePlan,
    original_pid: Option<u32>,
) -> Result<(), String> {
    unsafe extern "C" {
        fn proc_listallpids(buffer: *mut std::ffi::c_void, buffersize: i32) -> i32;
    }
    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if count < 0 || count > 100_000 {
        return Err("cannot enumerate installation users".into());
    }
    let mut pids = vec![0i32; count as usize + 512];
    let read = unsafe {
        proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if read < 0 || read as usize >= pids.len() {
        return Err("process enumeration changed; retry when quiet".into());
    }
    let roots = install_roots(plan)?;
    for pid in pids.into_iter().take(read as usize) {
        if pid <= 0 || Some(pid as u32) == original_pid || pid as u32 == std::process::id() {
            continue;
        }
        let Some(path) = process_path(pid as u32)? else {
            continue;
        };
        for root in &roots {
            if installation_process_in_scope(&path, root)? {
                return Err(format!("another live installation process ({pid}) uses {}; no other profile will be stopped", root.display()));
            }
        }
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn reject_other_install_processes(_: &UpdatePlan, _: Option<u32>) -> Result<(), String> {
    Err("process conflict detection requires macOS".into())
}
fn lock_quiescent(state_dir: &Path) -> Result<bool, String> {
    let path = state_dir.join("desktop-pet.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("unsafe daemon lifecycle lock".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
        Ok(true)
    } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err("cannot inspect daemon lifecycle lock".into())
    }
}
fn startup_quiescent(config_dir: &Path) -> Result<bool, String> {
    let path = config_dir.join("lifecycle.json");
    let bytes = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.to_string()),
        Ok(_) => coordinator::owned_private_file(&path, 65536)?,
    };
    let state: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid startup lifecycle state: {e}"))?;
    let object = state.as_object().ok_or("invalid startup lifecycle state")?;
    // Startup reclamation belongs to the regular launcher under its lifecycle lock.
    Ok(!object.contains_key("startup"))
}
fn stopped(plan: &UpdatePlan, pid: u32) -> Result<bool, String> {
    let socket_gone =
        fs::symlink_metadata(socket(plan)).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    Ok(!process_live(pid)
        && socket_gone
        && lock_quiescent(&plan.context.state_dir)?
        && startup_quiescent(&plan.context.config_dir)?)
}
fn wait_stop(plan: &UpdatePlan, pid: u32, panel: &crate::helper_ui::Panel) -> Result<(), String> {
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if stopped(plan, pid)? {
            return Ok(());
        }
        panel.pump();
        thread::sleep(Duration::from_millis(120));
    }
    Err("original PID/socket/lifetime/startup shutdown could not be proved; no installer was started".into())
}
fn staged_release_path(plan: &UpdatePlan) -> Result<PathBuf, String> {
    Ok(protocol::private_updates(&plan.context.state_dir)?
        .join(format!("{}.staged-release.json", plan.operation_id)))
}
fn candidate(plan: &UpdatePlan) -> Result<VerifiedInstallation, String> {
    let installed = coordinator::installed_candidate(plan)?;
    if plan.target.is_some() {
        let expected: ExecutableIdentity = serde_json::from_slice(
            &coordinator::owned_private_file(&staged_release_path(plan)?, 4096)?,
        )
        .map_err(|e| format!("staged release evidence invalid: {e}"))?;
        // Homebrew's post_install re-signs the Mach-O; the signed installed
        // binary differs byte-for-byte from the approved archive. The origin
        // verifier proves its exact formula/digest/version/receipt instead.
        if !matches!(plan.target_origin(), InstallOrigin::Homebrew { .. })
            && installed.image.sha256 != expected.sha256
        {
            return Err("installed image differs from exact approved staged release".into());
        }
    }
    Ok(installed)
}
fn stage_command(
    program: &Path,
    args: &[&str],
    output: &Path,
    max_bytes: u64,
    seconds: u64,
) -> Result<String, String> {
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(output)
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("release preflight command: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        match child.try_wait() {
            Ok(Some(exit)) if exit.success() => {
                if !group_quiescent(Some(child.id()))? {
                    return Err("release preflight process group still active".into());
                }
                break;
            }
            Ok(Some(exit)) => return Err(format!("release preflight command failed: {exit}")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            outcome => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(match outcome {
                    Ok(None) => "release preflight command timed out".into(),
                    Err(error) => format!("release preflight completion unknown: {error}"),
                    _ => unreachable!(),
                });
            }
        }
    }
    let bytes = coordinator::owned_private_file(output, max_bytes)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
// The signed release is downloaded only once for this operation. This is a
// pre-Stop capability proof, not a manager invocation or a persistent cache.
fn stage_release(plan: &UpdatePlan) -> Result<(), String> {
    let Some(target) = &plan.target else {
        return Ok(());
    };
    let release = &target.release;
    release.validate()?;
    if release.archive_size > 256 * 1024 * 1024 {
        return Err("approved release archive exceeds installation bound".into());
    }
    let updates = protocol::private_updates(&plan.context.state_dir)?;
    let stage = updates.join(format!("{}.release", plan.operation_id));
    fs::create_dir(&stage).map_err(|e| format!("cannot create operation release stage: {e}"))?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    for root in install_roots(plan)? {
        if stage.starts_with(&root) || root.starts_with(&stage) {
            return Err("release staging overlaps replaceable installation".into());
        }
    }
    let archive = stage.join(&release.asset_name);
    let checksum = stage.join("archive.sha256");
    let download = |url: &str, path: &Path, limit: u64| -> Result<(), String> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| e.to_string())?;
        let path = path.to_str().ok_or("non-UTF8 release stage")?;
        let bound = limit.to_string();
        stage_command(
            Path::new("/usr/bin/curl"),
            &[
                "--disable",
                "--fail",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "300",
                "--max-filesize",
                &bound,
                "--silent",
                "--show-error",
                "--output",
                path,
                url,
            ],
            &stage.join(if limit == 4096 {
                "checksum.download.log"
            } else {
                "archive.download.log"
            }),
            4096,
            310,
        )
        .map(|_| ())
    };
    download(&release.checksum_url, &checksum, 4096)?;
    let checksum_text = String::from_utf8(coordinator::owned_private_file(&checksum, 4096)?)
        .map_err(|e| e.to_string())?;
    release.validate_checksum(&checksum_text)?;
    download(&release.archive_url, &archive, release.archive_size)?;
    let mut file = File::open(&archive).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size != release.archive_size {
        return Err("release archive size differs from approved metadata".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 32768];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if format!("{:x}", digest.finalize()) != release.sha256 {
        return Err("release archive digest differs from approved release".into());
    }
    let archive_arg = archive.to_str().ok_or("non-UTF8 release archive")?;
    let listing = stage_command(
        Path::new("/usr/bin/tar"),
        &["-tzf", archive_arg],
        &stage.join("archive.list"),
        1024 * 1024,
        30,
    )?;
    let app = match release.channel {
        coordinator::ReleaseChannel::Stable => "HerdrDesktopPet.app",
        coordinator::ReleaseChannel::Beta => "HerdrDesktopPetBeta.app",
    };
    let mut has_app = false;
    for name in listing.lines() {
        let entry = name.strip_prefix("./").unwrap_or(name);
        if entry == app || entry.starts_with(&format!("{app}/")) {
            if entry.split('/').any(|part| part == ".." || part == ".") {
                return Err("release archive contains traversal".into());
            }
            has_app = true;
        } else if !entry.is_empty() && entry != "." {
            return Err("release archive contains unexpected path".into());
        }
    }
    if !has_app {
        return Err("approved release bundle missing from archive".into());
    }
    let details = stage_command(
        Path::new("/usr/bin/tar"),
        &["-tvzf", archive_arg],
        &stage.join("archive.details"),
        1024 * 1024,
        30,
    )?;
    if details
        .lines()
        .any(|line| !line.starts_with('-') && !line.starts_with('d'))
    {
        return Err("release archive contains links or special files".into());
    }
    let extracted = stage.join("extract");
    fs::create_dir(&extracted).map_err(|e| e.to_string())?;
    stage_command(
        Path::new("/usr/bin/tar"),
        &[
            "-xzf",
            archive_arg,
            "-C",
            extracted.to_str().ok_or("non-UTF8 stage")?,
        ],
        &stage.join("extract.log"),
        4096,
        60,
    )?;
    let binary = extracted.join(app).join("Contents/MacOS/herdr-desktop-pet");
    let image = coordinator::validate_candidate_for_context(&binary, &plan.context)?;
    let actual_version = stage_command(
        &image.path,
        &["--version"],
        &stage.join("version.log"),
        4096,
        5,
    )?;
    if actual_version.trim() != format!("herdr-desktop-pet {}", release.version) {
        return Err("staged signed app version differs from approved release".into());
    }
    let helper = coordinator::helper_path_for_context(&image.path, &plan.context)?;
    #[cfg(target_os = "macos")]
    stage_command(
        Path::new("/usr/bin/codesign"),
        &[
            "--verify",
            "--strict",
            helper.to_str().ok_or("invalid release helper path")?,
        ],
        &stage.join("signature.log"),
        4096,
        10,
    )?;
    let capabilities = stage_command(
        &helper,
        &["update-capabilities"],
        &stage.join("capabilities.log"),
        4096,
        5,
    )?;
    coordinator::validate_helper_capabilities(&capabilities, &plan.context)?;
    let bytes = serde_json::to_vec(&image).map_err(|e| e.to_string())?;
    protocol::write_private_bytes(&staged_release_path(plan)?, &bytes, 0o600)
}
fn bounded_output<R: Read>(mut pipe: R, mut file: File) {
    let mut total = 0usize;
    let mut buffer = [0u8; 8192];
    while let Ok(n) = pipe.read(&mut buffer) {
        if n == 0 {
            break;
        }
        let count = n.min(65536usize.saturating_sub(total));
        if count > 0 && file.write_all(&buffer[..count]).is_ok() {
            total += count;
        }
    }
    let _ = file.sync_all();
}
fn monitor_manager(
    plan: UpdatePlan,
    mut child: Child,
    out: thread::JoinHandle<()>,
    err: thread::JoinHandle<()>,
    mut item: ManagerEvidence,
) {
    MANAGER_DRAINING.store(true, std::sync::atomic::Ordering::Release);
    thread::spawn(move || {
        if let Ok(result) = child.wait() {
            item.state = ManagerState::Exited;
            item.exit_code = result.code();
            let _ = evidence(&plan, &item);
            loop {
                if manager_quiescent(&item).unwrap_or(false)
                    && out.is_finished()
                    && err.is_finished()
                {
                    let _ = out.join();
                    let _ = err.join();
                    item.drained = true;
                    let _ = evidence(&plan, &item);
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        }
        MANAGER_DRAINING.store(false, std::sync::atomic::Ordering::Release);
    });
}
fn manager(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    command: coordinator::ManagerCommand,
    panel: &crate::helper_ui::Panel,
    record: &mut OperationRecord,
) -> Result<(), String> {
    let program = coordinator::executable_identity(&command.program)?;
    let mut item = ManagerEvidence {
        program: program.path,
        program_sha256: program.sha256,
        uid: unsafe { libc::geteuid() },
        started_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis(),
        pid: None,
        pgid: None,
        state: ManagerState::Intent,
        exit_code: None,
        drained: false,
    };
    record.execution_fence = ExecutionFence::ManagerIntent;
    journal(plan, record)?;
    evidence(plan, &item)?;
    let base = protocol::private_updates(&plan.context.state_dir)?;
    let output = |suffix: &str| -> Result<File, String> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(base.join(format!("{}.manager.{suffix}", plan.operation_id)))
            .map_err(|e| e.to_string())
    };
    let stdout_file = output("stdout")?;
    let stderr_file = output("stderr")?;
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .env_clear()
        .envs(command.environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        process.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    if reservation.user_stop_requested()? {
        return Err("user stop superseded manager execution before spawn".into());
    }
    let mut child: Child = process
        .spawn()
        .map_err(|e| format!("manager was not started: {e}"))?;
    item.pid = Some(child.id());
    item.pgid = Some(child.id()); // setsid made the direct child the group/session leader
    item.state = ManagerState::Running;
    let stdout = child.stdout.take().ok_or("missing manager stdout")?;
    let stderr = child.stderr.take().ok_or("missing manager stderr")?;
    let out = thread::spawn(move || bounded_output(stdout, stdout_file));
    let err = thread::spawn(move || bounded_output(stderr, stderr_file));
    if let Err(error) = evidence(plan, &item) {
        monitor_manager(plan.clone(), child, out, err, item);
        return Err(format!(
            "manager spawned but PID evidence could not be journaled: {error}"
        ));
    }
    let deadline = Instant::now() + MANAGER_WAIT;
    let mut exit: Option<ExitStatus> = None;
    loop {
        if exit.is_none() {
            exit = match child.try_wait() {
                Ok(result) => result,
                Err(error) => {
                    monitor_manager(plan.clone(), child, out, err, item);
                    return Err(format!(
                        "manager completion unknown; PID evidence retained: {error}"
                    ));
                }
            };
            if let Some(result) = exit {
                item.state = ManagerState::Exited;
                item.exit_code = result.code();
                if let Err(error) = evidence(plan, &item) {
                    monitor_manager(plan.clone(), child, out, err, item);
                    return Err(format!("manager exit could not be journaled: {error}"));
                }
            }
        }
        if let Some(result) = exit {
            let quiescent = match manager_quiescent(&item) {
                Ok(quiescent) => quiescent,
                Err(error) => {
                    monitor_manager(plan.clone(), child, out, err, item);
                    return Err(format!("manager group completion unknown: {error}"));
                }
            };
            if quiescent && out.is_finished() && err.is_finished() {
                let _ = out.join();
                let _ = err.join();
                item.drained = true;
                evidence(plan, &item)?;
                record.execution_fence = ExecutionFence::ManagerExited;
                journal(plan, record)?;
                if !result.success() {
                    return Err(format!("installation manager exited with {result}; inspect bounded manager output in profile updates directory"));
                }
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            if exit.is_none() {
                item.state = ManagerState::TimedOut;
                if let Err(error) = evidence(plan, &item) {
                    monitor_manager(plan.clone(), child, out, err, item);
                    return Err(format!("manager timeout could not be journaled: {error}"));
                }
            }
            let pid = child.id();
            monitor_manager(plan.clone(), child, out, err, item);
            return Err(format!("manager PID {pid} or its process group/output is still active past timeout; no kill/retry"));
        }
        panel.pump();
        thread::sleep(Duration::from_millis(130));
    }
}
fn validate_new(
    plan: &UpdatePlan,
    old_pid: u32,
    panel: &crate::helper_ui::Panel,
) -> Result<VerifiedInstallation, String> {
    reject_other_install_processes(plan, None)?;
    let installed = candidate(plan)?;
    if !stopped(plan, old_pid)? {
        return Err("old daemon returned or shutdown is incomplete".into());
    }
    panel.pump();
    Ok(installed)
}
fn authorize_pair(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    record: &mut OperationRecord,
    installed: &VerifiedInstallation,
    purpose: StartupPurpose,
) -> Result<(), String> {
    let scope = snapshot(plan)?;
    pin_state(
        record.candidate.as_ref(),
        scope.as_ref(),
        installed,
        purpose,
        plan,
    )?;
    if purpose == StartupPurpose::Candidate && record.candidate.is_none() {
        record.candidate = Some(installed.image.clone());
        journal(plan, record)?;
        // Journal I before B, then prove the SAME I+B before choosing a token.
        if candidate(plan)? != *installed {
            return Err("candidate image/origin changed after image journal".into());
        }
    }
    if reservation.user_stop_requested()? {
        return Err("user stop superseded updater start".into());
    }
    if scope
        .as_ref()
        .and_then(|scope| scope.startup_origin.as_ref())
        .is_none()
    {
        reservation.authorize_start_for(&installed.origin)?;
    }
    reservation
        .issued_start_token_for(&installed.origin)?
        .ok_or("startup token was not durably issued")?;
    Ok(())
}
fn launch(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    installed: &VerifiedInstallation,
) -> Result<Child, String> {
    if reservation.user_stop_requested()? {
        return Err("user stop superseded updater start".into());
    }
    let token = reservation
        .issued_start_token_for(&installed.origin)?
        .ok_or("startup token was not durably issued for installed origin")?;
    start_command(plan, installed, &token, reservation.stop_generation)?
        .spawn()
        .map_err(|e| format!("verified installation start failed: {e}"))
}
fn start_command(
    plan: &UpdatePlan,
    installed: &VerifiedInstallation,
    token: &str,
    generation: u64,
) -> Result<Command, String> {
    let c = &plan.context;
    let mode = c.observation_mode()?;
    let managed_host = if let InstallOrigin::Herdr { host, .. } = &plan.origin {
        let captured = c
            .environment
            .get("HERDR_BIN_PATH")
            .ok_or("managed host route missing from captured profile")?;
        if fs::canonicalize(captured).map_err(|e| e.to_string())? != *host {
            return Err(
                "captured host routing changed; refusing incidental PATH or source conversion"
                    .into(),
            );
        }
        Some(host)
    } else {
        None
    };
    let mut cmd = Command::new(&installed.image.path);
    cmd.arg("start")
        .arg("--config-dir")
        .arg(&c.config_dir)
        .arg("--state-dir")
        .arg(&c.state_dir)
        .arg("--socket")
        .arg(&c.herdr_socket);
    if mode.supports_tern() {
        cmd.arg("--host").arg(mode.as_str());
    }
    if let Some(assets) = &c.assets_override {
        cmd.arg("--assets").arg(assets);
    }
    cmd.env_clear()
        .envs(&c.environment)
        .env("HERDR_PLUGIN_CONFIG_DIR", &c.config_dir)
        .env("HERDR_PLUGIN_STATE_DIR", &c.state_dir)
        .env("HERDR_SOCKET_PATH", &c.herdr_socket)
        .env(OBSERVATION_MODE_ENV, mode.as_str())
        .env("HERDR_DESKTOP_PET_UPDATER_TOKEN", token)
        .env(
            "HERDR_DESKTOP_PET_UPDATER_GENERATION",
            generation.to_string(),
        )
        .current_dir(&c.state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(original_host_config) = &c.host_plugin_config_dir {
        cmd.env(
            "HERDR_DESKTOP_PET_CAPTURED_HOST_PLUGIN_CONFIG_DIR",
            original_host_config,
        );
    } else {
        cmd.env_remove("HERDR_DESKTOP_PET_CAPTURED_HOST_PLUGIN_CONFIG_DIR");
    }
    if let Some(host) = managed_host {
        cmd.env("HERDR_BIN_PATH", host);
    }
    Ok(cmd)
}
fn new_ready(
    plan: &UpdatePlan,
    installed: &VerifiedInstallation,
    purpose: StartupPurpose,
    pinned_image: Option<&ExecutableIdentity>,
    reply: &Status,
) -> Result<bool, String> {
    let image = &installed.image;
    if !(reply.ok
        && reply.running
        && !reply.shutdown
        && reply.ready
        && reply.ui_ready
        && reply.control_ready
        && reply.pid > 0
        && reply.instance_id != plan.context.instance_id
        && reply.running_sha256 == image.sha256
        && reply.executable_path == image.path
        && reply.config_dir == plan.context.config_dir
        && reply.state_dir == plan.context.state_dir
        && same_endpoint(&reply.herdr_socket, &plan.context.herdr_socket)
        && reply.assets_override == plan.context.assets_override
        && status_mode_matches(plan, reply)?
        && process_path(reply.pid)?.as_deref() == Some(image.path.as_path())
        && startup_quiescent(&plan.context.config_dir)?)
    {
        return Ok(false);
    }
    // Purpose is explicit: a reproducible Local rebuild can have original bytes.
    let current = match purpose {
        StartupPurpose::Candidate => candidate(plan)?,
        StartupPurpose::Original => coordinator::restored_original(plan)?,
    };
    let scope = snapshot(plan)?;
    pin_state(pinned_image, scope.as_ref(), &current, purpose, plan)?;
    if purpose == StartupPurpose::Candidate
        && !scope.as_ref().is_some_and(|scope| {
            scope.markers_complete && scope.startup_origin.as_ref() == Some(&installed.origin)
        })
    {
        return Ok(false);
    }
    Ok(current == *installed)
}
fn wait_new(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    installed: &VerifiedInstallation,
    purpose: StartupPurpose,
    pinned_image: Option<&ExecutableIdentity>,
    child: &mut Child,
    panel: &crate::helper_ui::Panel,
) -> Result<(), String> {
    let deadline = Instant::now() + START_WAIT;
    while Instant::now() < deadline {
        if reservation.user_stop_requested()? {
            return Err("user stop superseded updater start".into());
        }
        let start_completed = match child.try_wait().map_err(|e| e.to_string())? {
            Some(exit) if !exit.success() => {
                return Err(format!("candidate start command failed with {exit}"));
            }
            Some(_) => true,
            None => false,
        };
        if let Ok(reply) = status(plan) {
            // The owned start CLI briefly shares the installation while its
            // daemon becomes ready. Require its successful exit before scanning
            // for genuinely independent users of the same installation.
            if start_completed && new_ready(plan, installed, purpose, pinned_image, &reply)? {
                reject_other_install_processes(plan, Some(reply.pid))?;
                reject_other_active_profiles(plan)?;
                return Ok(());
            }
        }
        panel.pump();
        thread::sleep(Duration::from_millis(150));
    }
    Err("new instance/image/profile/UI/control readiness not proved before timeout".into())
}
fn candidate_completion(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    pinned_image: Option<&ExecutableIdentity>,
    installed: &VerifiedInstallation,
) -> Result<(), String> {
    if reservation.user_stop_requested()? {
        return Err("user stop superseded candidate completion".into());
    }
    let current = status(plan)?;
    if !new_ready(
        plan,
        installed,
        StartupPurpose::Candidate,
        pinned_image,
        &current,
    )? {
        return Err(
            "candidate image/origin or fresh UI/control readiness changed before completion".into(),
        );
    }
    reject_other_install_processes(plan, Some(current.pid))?;
    reject_other_active_profiles(plan)?;
    if reservation.user_stop_requested()? {
        return Err("user stop superseded candidate completion".into());
    }
    Ok(())
}
fn record(plan: &UpdatePlan) -> OperationRecord {
    OperationRecord {
        version: protocol::UPDATER_PROTOCOL,
        operation_id: plan.operation_id.clone(),
        phase: OperationPhase::Preparing,
        execution_fence: ExecutionFence::NoSpawn,
        detail: "Preparing update and preserving this profile".into(),
        installed: false,
        applied: false,
        candidate: None,
    }
}
fn fail(
    plan: &UpdatePlan,
    record: &mut OperationRecord,
    error: String,
    uncertain: bool,
    reservation: Option<InstallationReservation>,
) -> Result<(), String> {
    let user_stop = reservation
        .as_ref()
        .is_some_and(|r| r.user_stop_requested().unwrap_or(false));
    let phase = if user_stop {
        OperationPhase::UserStopped
    } else if uncertain {
        OperationPhase::Unknown
    } else {
        OperationPhase::Failed
    };
    transition(plan, record, phase, format!("{error}. No rollback is guaranteed. See update-status/recover for this operation; no manager will be retried automatically."))?;
    // No failed or unknown operation releases a marker without offline reconciliation.
    Ok(())
}
fn preflight(plan: &UpdatePlan) -> Result<LifecycleEvidence, String> {
    coordinator::recheck_plan(plan)?;
    let live = status(plan)?;
    verify_status(plan, &live)?;
    if !live.ui_ready || !live.control_ready {
        return Err("original UI/control is not ready".into());
    }
    reject_other_install_processes(plan, Some(live.pid))?;
    reject_other_active_profiles(plan)?;
    let generation = protocol::stop_generation(&plan.context.state_dir)?;
    lifecycle(plan, &live, generation)?;
    if protocol::stop_generation(&plan.context.state_dir)? != generation {
        return Err("user stopped original during preflight".into());
    }
    read_lifecycle(plan)
}
fn stage_with_panel(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    panel: &crate::helper_ui::Panel,
) -> Result<(), (String, bool)> {
    if plan.target.is_none() {
        return Ok(());
    }
    let worker_plan = plan.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("update-release-stage".into())
        .spawn(move || {
            let result = coordinator::refresh_release_target(&worker_plan)
                .and_then(|_| stage_release(&worker_plan));
            let _ = sender.send(result);
        })
        .map_err(|e| (format!("cannot start release staging worker: {e}"), false))?;
    let mut stopped_by_user = false;
    let mut stop_error = None;
    loop {
        panel.pump();
        match reservation.user_stop_requested() {
            Ok(true) => stopped_by_user = true,
            Ok(false) => {}
            Err(error) => stop_error = Some(error),
        }
        match receiver.recv_timeout(Duration::from_millis(120)) {
            Ok(result) => {
                if worker.join().is_err() {
                    return Err(("release staging worker panicked".into(), true));
                }
                if let Some(error) = stop_error {
                    return Err((
                        format!("cannot verify UserStop during release staging: {error}"),
                        true,
                    ));
                }
                if stopped_by_user || reservation.user_stop_requested().map_err(|e| (e, true))? {
                    return Err(("user stop superseded release staging".into(), false));
                }
                return result.map_err(|error| (error, false));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                return Err((
                    "release staging worker exited without a verified result".into(),
                    true,
                ));
            }
        }
    }
}

fn prepare(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    owner: &LifecycleEvidence,
    panel: &crate::helper_ui::Panel,
    uncertain: &mut bool,
) -> Result<(u32, Option<coordinator::ManagerCommand>), String> {
    if reservation.stop_generation != owner.stop_generation || reservation.user_stop_requested()? {
        return Err("user stop superseded update preflight".into());
    }
    coordinator::recheck_plan(plan)?;
    if !snapshot(plan)?
        .is_some_and(|scope| scope.markers_complete && scope.startup_origin.is_none())
    {
        return Err("original reservation scope changed during release staging".into());
    }
    let command = coordinator::manager_command(plan)?;
    let live = status(plan)?;
    verify_status(plan, &live)?;
    if !same_original(owner, &live) || !live.ui_ready || !live.control_ready {
        return Err("reserved original PID/instance/profile/image changed since preflight".into());
    }
    reject_other_install_processes(plan, Some(live.pid))?;
    reject_other_active_profiles(plan)?;
    if reservation.user_stop_requested()? {
        return Err("user stop superseded update".into());
    }
    // Closing a nonmodal panel is not a stop request. Restore visible ownership
    // before native Prepare so the final ACK can truthfully transfer it.
    panel.activate();
    if !panel.visible() {
        return Err("native updater window cannot be made visible".into());
    }
    if !snapshot(plan)?
        .is_some_and(|scope| scope.markers_complete && scope.startup_origin.is_none())
    {
        return Err("original reservation scope changed before native preparation".into());
    }
    *uncertain = true;
    control(plan, reservation, UpdateCommand::Prepare)?;
    panel.pump();
    if !panel.visible() {
        panel.activate();
    }
    if !panel.visible() {
        return Err("native updater window cannot be made visible after preparation".into());
    }
    if reservation.user_stop_requested()? {
        return Err("user stop superseded preparation".into());
    }
    Ok((owner.original_pid, command))
}
fn ack(
    plan: &UpdatePlan,
    visible: bool,
    prepared: bool,
    error: Option<&str>,
) -> Result<(), String> {
    let mut message = serde_json::json!({"protocol":protocol::UPDATER_PROTOCOL,"operation_id":plan.operation_id,"visible":visible,"prepared":prepared});
    if let Some(error) = error {
        message["error"] = error.into();
    }
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &message).map_err(|e| e.to_string())?;
    stdout
        .write_all(b"\n")
        .and_then(|_| stdout.flush())
        .map_err(|e| e.to_string())
}
fn verify_staged_helper(plan: &UpdatePlan) -> Result<(), String> {
    let expected = protocol::private_updates(&plan.context.state_dir)?
        .join(format!("helper-{}", plan.operation_id));
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if fs::canonicalize(&current).map_err(|e| e.to_string())?
        != fs::canonicalize(&expected).map_err(|e| e.to_string())?
    {
        return Err("updater must execute from the signed private staged copy, outside replaceable installation".into());
    }
    let metadata = fs::symlink_metadata(&expected).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("staged updater is not a private owned executable file".into());
    }
    coordinator::executable_identity(&expected)?;
    #[cfg(target_os = "macos")]
    {
        let mut codesign = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&expected)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot verify staged helper signature: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(exit) = codesign.try_wait().map_err(|e| e.to_string())? {
                if !exit.success() {
                    return Err("staged helper signature verification failed".into());
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err("staged helper signature verification timed out".into());
            }
            thread::sleep(Duration::from_millis(40));
        }
    }
    Ok(())
}

pub fn run(plan: UpdatePlan) -> Result<(), String> {
    if plan.version != protocol::UPDATER_PROTOCOL {
        return Err("unsupported update plan protocol".into());
    }
    let lease = protocol::lock_operation(&plan.context.state_dir, &plan.operation_id)?;
    // The lease serializes this check with recovery. Even a malformed or
    // unsupported journal is an existing operation, never a new NoSpawn run.
    let result_path = protocol::private_updates(&plan.context.state_dir)?
        .join(format!("{}.result.json", plan.operation_id));
    match fs::symlink_metadata(&result_path) {
        Ok(_) => {
            return Err("operation already has a journal; use update-recover, never rerun".into())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot check existing operation journal: {error}")),
    }
    let mut record = record(&plan);
    journal(&plan, &record)?;
    if let Err(error) = verify_staged_helper(&plan) {
        transition(&plan, &mut record, OperationPhase::Failed, error.clone())?;
        let _ = ack(&plan, false, false, Some(&error));
        return Err(error);
    }
    let owner = match preflight(&plan) {
        Ok(owner) => owner,
        Err(error) => {
            transition(&plan, &mut record, OperationPhase::Failed, error.clone())?;
            let _ = ack(&plan, false, false, Some(&error));
            return Err(error);
        }
    };
    let reservation = match protocol::reserve_install(
        &plan.origin,
        plan.target.as_ref().map(|target| &target.origin),
        &plan.context.state_dir,
        &plan.operation_id,
    ) {
        Ok(reservation) => reservation,
        Err(error) => {
            transition(
                &plan,
                &mut record,
                OperationPhase::Failed,
                format!("Update not started: {error}"),
            )?;
            let _ = ack(&plan, false, false, Some(&error));
            return Err(error);
        }
    };
    let panel = match crate::helper_ui::Panel::new(&plan.context.locale, &plan) {
        Ok(panel) => panel,
        Err(error) => {
            fail(&plan, &mut record, error.clone(), false, Some(reservation))?;
            let _ = ack(&plan, false, false, Some(&error));
            return Err(error);
        }
    };
    panel.show(Phase::Preparing, Some(&record.detail));
    let mut prepare_uncertain = false;
    let prepared = stage_with_panel(&plan, &reservation, &panel).and_then(|_| {
        prepare(&plan, &reservation, &owner, &panel, &mut prepare_uncertain)
            .map_err(|e| (e, prepare_uncertain))
    });
    let (old_pid, manager_command) = match prepared {
        Ok(value) => value,
        Err((error, uncertain)) => {
            let diagnostic = journal_diagnostic(
                &error,
                fail(
                    &plan,
                    &mut record,
                    error.clone(),
                    uncertain,
                    Some(reservation),
                ),
            );
            let _ = ack(&plan, false, false, Some(&error));
            panel.show(
                if record.phase == OperationPhase::UserStopped {
                    Phase::UserStopped
                } else {
                    Phase::Attention(Attention::NotStarted)
                },
                Some(&diagnostic),
            );
            if record.phase != OperationPhase::UserStopped {
                panel.offer_recovery();
            }
            if retain(&panel) {
                panel.close();
                let recovered = recover_locked(plan, true, &lease)?;
                return if recovered.phase == OperationPhase::Completed {
                    Ok(())
                } else {
                    Err(format!("update was not applied: {}", recovered.detail))
                };
            }
            return Err(error);
        }
    };
    // ACK is the transfer of visible UI ownership. No Stop is sent until ACK has been flushed.
    panel.pump();
    if !panel.visible() {
        panel.activate();
    }
    if !panel.visible() {
        let error = "native updater window is not visible".to_string();
        fail(&plan, &mut record, error.clone(), true, Some(reservation))?;
        let _ = ack(&plan, false, false, Some(&error));
        return Err(error);
    }
    if let Err(error) = ack(&plan, true, true, None) {
        let diagnostic = journal_diagnostic(
            &error,
            fail(&plan, &mut record, error.clone(), true, Some(reservation)),
        );
        panel.show(
            if record.phase == OperationPhase::UserStopped {
                Phase::UserStopped
            } else {
                Phase::Attention(Attention::Ownership)
            },
            Some(&diagnostic),
        );
        if record.phase != OperationPhase::UserStopped {
            panel.offer_recovery();
        }
        if retain(&panel) {
            panel.close();
            let restored = recover_locked(plan, true, &lease)?;
            return if restored.phase == OperationPhase::Completed {
                Ok(())
            } else {
                Err(format!("update was not applied: {}", restored.detail))
            };
        }
        return Err(error);
    }
    let outcome = perform(
        &plan,
        &reservation,
        &panel,
        old_pid,
        manager_command,
        &mut record,
    );
    match outcome {
        Ok(()) => {
            panel.show(Phase::Finalizing, None);
            let finalized = candidate(&plan)
                .and_then(|installed| {
                    candidate_completion(&plan, &reservation, record.candidate.as_ref(), &installed)
                })
                .and_then(|_| {
                    transition(
                        &plan,
                        &mut record,
                        OperationPhase::Completed,
                        "New instance and UI/control are ready on the original profile",
                    )
                });
            if let Err(error) = finalized {
                let diagnostic = journal_diagnostic(
                    &error,
                    fail(&plan, &mut record, error.clone(), true, Some(reservation)),
                );
                panel.show(
                    if record.phase == OperationPhase::UserStopped {
                        Phase::UserStopped
                    } else {
                        Phase::Attention(Attention::Completion)
                    },
                    Some(&diagnostic),
                );
                if record.phase != OperationPhase::UserStopped {
                    panel.offer_recovery();
                }
                if retain(&panel) {
                    panel.close();
                    let reconciled = recover_locked(plan, true, &lease)?;
                    return if reconciled.phase == OperationPhase::Completed {
                        Ok(())
                    } else {
                        Err(format!("update was not finalized: {}", reconciled.detail))
                    };
                }
                return Err(error);
            }
            let released = candidate(&plan)
                .and_then(|installed| {
                    candidate_completion(&plan, &reservation, record.candidate.as_ref(), &installed)
                })
                .and_then(|_| reservation.release());
            if let Err(error) = released {
                let diagnostic = journal_diagnostic(&error, transition(&plan, &mut record, OperationPhase::Unknown, format!("New UI ready but installation reservation release failed: {error}; inspect/recover offline")));
                panel.show(Phase::Attention(Attention::Release), Some(&diagnostic));
                panel.offer_recovery();
                if retain(&panel) {
                    panel.close();
                    let reconciled = recover_locked(plan, true, &lease)?;
                    return if reconciled.phase == OperationPhase::Completed {
                        Ok(())
                    } else {
                        Err(format!("update was not finalized: {}", reconciled.detail))
                    };
                }
                return Err(error);
            }
            panel.close();
            Ok(())
        }
        Err((error, uncertain)) => {
            let diagnostic = journal_diagnostic(
                &error,
                fail(
                    &plan,
                    &mut record,
                    error.clone(),
                    uncertain,
                    Some(reservation),
                ),
            );
            panel.show(
                if record.phase == OperationPhase::UserStopped {
                    Phase::UserStopped
                } else {
                    Phase::Attention(Attention::Interrupted)
                },
                Some(&diagnostic),
            );
            if record.phase != OperationPhase::UserStopped {
                panel.offer_recovery();
            }
            if retain(&panel) {
                panel.close();
                let recovered = recover_locked(plan, true, &lease)?;
                return if recovered.phase == OperationPhase::Completed {
                    Ok(())
                } else {
                    Err(format!("update was not applied: {}", recovered.detail))
                };
            }
            Err(error)
        }
    }
}
fn perform(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    panel: &crate::helper_ui::Panel,
    old_pid: u32,
    manager_command: Option<coordinator::ManagerCommand>,
    record: &mut OperationRecord,
) -> Result<(), (String, bool)> {
    let step = |result: Result<(), String>, unknown: bool| result.map_err(|e| (e, unknown));
    // Reservations gate new protocol-3 starts, but recheck actual legacy or
    // independent process users immediately before touching the original.
    step(reject_other_install_processes(plan, Some(old_pid)), false)?;
    step(reject_other_active_profiles(plan), false)?;
    let owner = read_lifecycle(plan).map_err(|e| (e, false))?;
    let original = status(plan).map_err(|e| (e, false))?;
    step(verify_status(plan, &original), false)?;
    if !same_original(&owner, &original) || !original.ui_ready || !original.control_ready {
        return Err((
            "prepared original identity or UI/control changed before Stop".into(),
            false,
        ));
    }
    step(coordinator::recheck_plan(plan), false)?;
    if !snapshot(plan)
        .map_err(|e| (e, true))?
        .is_some_and(|scope| scope.markers_complete && scope.startup_origin.is_none())
    {
        return Err((
            "original reservation scope changed before Stop".into(),
            true,
        ));
    }
    if reservation.user_stop_requested().map_err(|e| (e, true))? {
        return Err(("user stop superseded original shutdown".into(), false));
    }
    panel.show(Phase::Stopping, None);
    step(
        transition(
            plan,
            record,
            OperationPhase::Stopping,
            "Stopping prepared original instance",
        ),
        false,
    )?;
    step(control(plan, reservation, UpdateCommand::Stop), true)?;
    step(wait_stop(plan, old_pid, panel), true)?;
    panel.activate();
    step(reject_other_install_processes(plan, None), true)?;
    if reservation.user_stop_requested().map_err(|e| (e, true))? {
        return Err(("user stopped the application".into(), false));
    }
    if let Some(cmd) = manager_command {
        step(
            transition(
                plan,
                record,
                OperationPhase::Installing,
                "Installation manager running once",
            ),
            true,
        )?;
        panel.show(Phase::Installing, None);
        step(manager(plan, reservation, cmd, panel, record), true)?;
    }
    step(
        transition(
            plan,
            record,
            OperationPhase::Validating,
            "Checking installed image/source/signature/version",
        ),
        true,
    )?;
    panel.show(Phase::Validating, None);
    let installed = validate_new(plan, old_pid, panel).map_err(|e| (e, true))?;
    record.installed = true;
    step(
        authorize_pair(
            plan,
            reservation,
            record,
            &installed,
            StartupPurpose::Candidate,
        ),
        true,
    )?;
    if reservation.user_stop_requested().map_err(|e| (e, true))? {
        return Err(("user stopped the application".into(), false));
    }
    step(
        transition(
            plan,
            record,
            OperationPhase::Starting,
            "Starting only the captured profile",
        ),
        true,
    )?;
    panel.show(Phase::Starting, None);
    let mut child = launch(plan, reservation, &installed).map_err(|e| (e, true))?;
    step(
        wait_new(
            plan,
            reservation,
            &installed,
            StartupPurpose::Candidate,
            record.candidate.as_ref(),
            &mut child,
            panel,
        ),
        true,
    )?;
    if reservation.user_stop_requested().map_err(|e| (e, true))? {
        return Err((
            "user stopped the updated application before completion".into(),
            false,
        ));
    }
    step(
        candidate_completion(plan, reservation, record.candidate.as_ref(), &installed),
        true,
    )?;
    record.applied = true;
    step(journal(plan, record), true)?;
    Ok(())
}
fn journal_diagnostic(error: &str, journal: Result<(), String>) -> String {
    match journal {
        Ok(()) => error.to_owned(),
        Err(journal_error) => {
            format!("{error}\n\nCould not save operation result: {journal_error}")
        }
    }
}
fn retain(panel: &crate::helper_ui::Panel) -> bool {
    while panel.visible() || MANAGER_DRAINING.load(std::sync::atomic::Ordering::Acquire) {
        panel.pump();
        if panel.recovery_requested() {
            return true;
        }
        thread::sleep(Duration::from_millis(120));
    }
    false
}
pub fn inspect(plan: &UpdatePlan) -> Result<OperationRecord, String> {
    if plan.version != protocol::UPDATER_PROTOCOL {
        return Err("unsupported update plan protocol; legacy evidence retained".into());
    }
    let mut record = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)?
        .ok_or("operation journal missing")?;
    if record.version != protocol::UPDATER_PROTOCOL {
        return Err("unsupported operation journal protocol".into());
    }
    let durable_image = record.candidate.clone();
    let durable_scope = snapshot(plan)?;
    let updates = protocol::private_updates(&plan.context.state_dir)?;
    let owner = match read_lifecycle(plan) {
        Ok(owner) => Some(owner),
        Err(_)
            if fs::symlink_metadata(
                updates.join(format!("{}.lifecycle.json", plan.operation_id)),
            )
            .is_err_and(|e| e.kind() == io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let expected_helper =
        fs::canonicalize(updates.join(format!("helper-{}", plan.operation_id))).ok();
    let helper_running = owner
        .as_ref()
        .and_then(|owner| process_path(owner.helper_pid).ok().flatten())
        .is_some_and(|path| expected_helper.as_ref() == Some(&path))
        && owner.as_ref().is_some_and(|owner| {
            coordinator::executable_identity(&owner.helper_image.path)
                .is_ok_and(|identity| identity == owner.helper_image)
        });
    if helper_running {
        if let Some(manager) = read_evidence(plan)? {
            if manager.state != ManagerState::Exited || !manager.drained {
                record.detail = format!("Installation manager PID {:?}, PGID {:?}: active or completion not proved; no replay", manager.pid, manager.pgid);
            }
        }
        return Ok(record);
    }
    let reserved = durable_scope.is_some();
    if !reserved
        && matches!(
            record.phase,
            OperationPhase::Completed | OperationPhase::Failed | OperationPhase::UserStopped
        )
    {
        return Ok(record);
    }
    if matches!(
        record.phase,
        OperationPhase::Preparing
            | OperationPhase::Stopping
            | OperationPhase::Installing
            | OperationPhase::Validating
            | OperationPhase::Starting
    ) || record.phase == OperationPhase::Completed && reserved
    {
        record.phase = OperationPhase::Unknown;
        record.detail = "Coordinator is no longer running; reconciling durable manager and installed-image evidence, no replay".into();
    }
    if reserved {
        let selected = durable_scope
            .as_ref()
            .and_then(|scope| scope.startup_origin.as_ref());
        let incomplete = match (&durable_image, selected) {
            (Some(_), None) => true,
            (None, Some(origin)) => origin != &plan.origin,
            _ => false,
        };
        if incomplete && historical_original(plan, &record, durable_scope.as_ref()).is_none() {
            record.phase = OperationPhase::Unknown;
            record.applied = false;
            record.detail = "Durable candidate image/origin pair is incomplete; reservation retained without reconstructing current disk".into();
            if owner.as_ref().is_some_and(|owner| {
                protocol::stop_generation(&plan.context.state_dir)
                    .is_ok_and(|generation| generation != owner.stop_generation)
            }) {
                record.phase = OperationPhase::UserStopped;
            }
            return Ok(record);
        }
    }
    if let (Some(image), Some(scope)) = (durable_image.as_ref(), durable_scope.as_ref()) {
        if scope.startup_origin.is_some()
            && historical_original(plan, &record, durable_scope.as_ref()).is_none()
        {
            let pair_valid = candidate(plan).and_then(|installed| {
                pin_state(
                    Some(image),
                    Some(scope),
                    &installed,
                    StartupPurpose::Candidate,
                    plan,
                )
            });
            if let Err(error) = pair_valid {
                record.phase = OperationPhase::Unknown;
                record.applied = false;
                record.detail = format!("Durably selected candidate image/origin no longer matches installed evidence: {error}; reservation retained");
                if owner.as_ref().is_some_and(|owner| {
                    protocol::stop_generation(&plan.context.state_dir)
                        .is_ok_and(|generation| generation != owner.stop_generation)
                }) {
                    record.phase = OperationPhase::UserStopped;
                }
                return Ok(record);
            }
        }
    }
    let mut manager_safe = false;
    let mut manager_failed_quiet = false;
    match read_evidence(plan)? {
        Some(manager) if record.execution_fence == ExecutionFence::ManagerExited => {
            if manager.uid == unsafe { libc::geteuid() }
                && manager.pid == manager.pgid
                && manager.started_at_ms != 0
                && manager.program_sha256.len() == 64
                && manager.state == ManagerState::Exited
                && manager.drained
                && manager_quiescent(&manager).unwrap_or(false)
            {
                if manager.exit_code == Some(0) {
                    manager_safe = true;
                } else {
                    manager_failed_quiet = true;
                }
            } else {
                record.detail = format!("Manager PID {:?} PGID {:?}: exit/output/group quiescence not proved; installation reserved, no retry", manager.pid, manager.pgid);
            }
        }
        Some(_) => {
            record.detail =
                "Manager evidence conflicts with durable execution fence; reservation retained"
                    .into();
        }
        None if record.execution_fence == ExecutionFence::NoSpawn
            && manager_artifacts_absent(plan)?
            && matches!(plan.action, coordinator::UpdateAction::ApplyInstalled) =>
        {
            manager_safe = true;
        }
        None if record.execution_fence == ExecutionFence::ManagerIntent => {
            record.detail =
                "Manager spawn/PID evidence gap; reservation retained, no replay".into();
        }
        None if record.execution_fence == ExecutionFence::ManagerExited => {
            record.detail = "Manager exit evidence missing; reservation retained, no replay".into();
        }
        None => {}
    }
    let restored_running = record.phase == OperationPhase::Failed
        && !record.installed
        && !record.applied
        && owner.as_ref().is_some_and(|owner| {
            !process_live(owner.original_pid)
                && coordinator::restored_original(plan)
                    .ok()
                    .is_some_and(|original| {
                        selected_origin(
                            durable_scope.as_ref(),
                            &original,
                            StartupPurpose::Original,
                            plan,
                        )
                        .is_ok()
                            && status(plan).ok().is_some_and(|reply| {
                                new_ready(
                                    plan,
                                    &original,
                                    StartupPurpose::Original,
                                    durable_image.as_ref(),
                                    &reply,
                                )
                                .unwrap_or(false)
                            })
                    })
        });
    if restored_running {
        record.detail =
            "Verified fresh original instance is ready; prior failed restoration is NOT an upgrade"
                .into();
    }
    if manager_safe && !restored_running {
        if let Some(owner) = &owner {
            if !process_live(owner.original_pid) {
                if historical_original(plan, &record, durable_scope.as_ref()).is_some() {
                    record.installed = false;
                    record.applied = false;
                    record.detail = "Historical candidate image retained; independently verified CURRENT signed original image/source is unchanged, NOT an upgrade".into();
                } else {
                    match candidate(plan) {
                        Ok(installed) => {
                            match pin_state(
                                durable_image.as_ref(),
                                durable_scope.as_ref(),
                                &installed,
                                StartupPurpose::Candidate,
                                plan,
                            ) {
                                Err(error) => {
                                    record.phase = OperationPhase::Unknown;
                                    record.applied = false;
                                    record.detail = format!("Durable candidate image/origin pin incomplete or changed: {error}; reservation retained");
                                }
                                Ok(()) => {
                                    // An unpinned current installation is an observation only.
                                    record.installed = true;
                                    if let Ok(current) = status(plan) {
                                        if durable_image.is_some()
                                            && durable_scope.as_ref().is_some_and(|scope| {
                                                scope.startup_origin.as_ref()
                                                    == Some(&installed.origin)
                                                    && scope.markers_complete
                                            })
                                            && new_ready(
                                                plan,
                                                &installed,
                                                StartupPurpose::Candidate,
                                                durable_image.as_ref(),
                                                &current,
                                            )?
                                        {
                                            record.applied = true;
                                            record.detail = "Verified pinned new instance and UI/control ready; explicit recovery must reconcile retained reservation".into();
                                        } else {
                                            record.detail = "Candidate observed without complete durable startup proof, or another/unready daemon occupies the profile; explicit recovery required".into();
                                        }
                                    } else if stopped(plan, owner.original_pid)? {
                                        record.detail = "Installed candidate observed; explicit recover --start required unless user stopped".into();
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            if durable_image.is_none()
                                && durable_scope
                                    .as_ref()
                                    .and_then(|scope| scope.startup_origin.as_ref())
                                    .is_none()
                                && coordinator::restored_original(plan).is_ok_and(|original| {
                                    selected_origin(
                                        durable_scope.as_ref(),
                                        &original,
                                        StartupPurpose::Original,
                                        plan,
                                    )
                                    .is_ok()
                                })
                            {
                                record.installed = false;
                                record.applied = false;
                                record.detail = "Manager finished but CURRENT signed original image/source is unchanged; explicit recover --start restores same version, NOT an upgrade".into();
                            } else {
                                record.phase = OperationPhase::Unknown;
                                record.applied = false;
                                record.detail = format!("Pinned candidate cannot be verified: {error}; reservation retained");
                            }
                        }
                    }
                }
            }
        }
    }
    if manager_failed_quiet {
        if let Some(owner) = &owner {
            if stopped(plan, owner.original_pid)?
                && coordinator::restored_original(plan).is_ok_and(|original| {
                    selected_origin(
                        durable_scope.as_ref(),
                        &original,
                        StartupPurpose::Original,
                        plan,
                    )
                    .is_ok()
                })
            {
                record.installed = false;
                record.applied = false;
                record.detail = "Installer reported failure; CURRENT signed original image/source verified, other installer effects unknown. Explicit recover --start restores same version, NOT an upgrade".into();
            } else {
                record.detail = "Installer failed and original image/source or lifecycle cannot be proved safe; reservation retained".into();
            }
        }
    }
    if reserved {
        if let Some(owner) = &owner {
            if protocol::stop_generation(&plan.context.state_dir)? != owner.stop_generation {
                record.phase = OperationPhase::UserStopped;
                record.detail = "User stop superseded updater restart; reserved installation requires explicit recovery, never automatic revival".into();
            }
        }
    }
    // Offline status is read-only; reconciliation journals only while holding
    // the operation lease, never racing the running helper.
    Ok(record)
}
/// Reconcile without replaying a manager. Explicit recovery may start a verified installed image.
pub fn recover(plan: UpdatePlan, start: bool) -> Result<OperationRecord, String> {
    if plan.version != protocol::UPDATER_PROTOCOL {
        return Err("unsupported update plan protocol; legacy evidence retained".into());
    }
    let lease = protocol::lock_operation(&plan.context.state_dir, &plan.operation_id)?;
    recover_locked(plan, start, &lease)
}
fn recover_locked(
    plan: UpdatePlan,
    start: bool,
    _lease: &OperationLease,
) -> Result<OperationRecord, String> {
    {
        verify_staged_helper(&plan)?;
        let durable = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)?
            .ok_or("operation journal missing")?;
        let durable_scope = snapshot(&plan)?;
        let mut record = inspect(&plan)?;
        let exact_historical_original =
            historical_original(&plan, &durable, durable_scope.as_ref()).is_some();
        // Partial pins cannot be reconstructed from today's registry or bytes.
        let pin_check: Result<(), String> = (|| {
            if let Some(scope) = durable_scope.as_ref() {
                match (durable.candidate.as_ref(), scope.startup_origin.as_ref()) {
                    (Some(_), None) if !exact_historical_original => {
                        return Err("candidate image pin lacks a durable origin; recovery cannot reconstruct the missing pair".into());
                    }
                    (None, Some(selected)) if selected != &plan.origin => {
                        return Err("candidate origin pin lacks a durable image; recovery cannot backfill it".into());
                    }
                    (Some(image), Some(_)) if !exact_historical_original => {
                        let installed = candidate(&plan)?;
                        pin_state(
                            Some(image),
                            durable_scope.as_ref(),
                            &installed,
                            StartupPurpose::Candidate,
                            &plan,
                        )?;
                    }
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Err(error) = pin_check {
            let phase = if record.phase == OperationPhase::UserStopped {
                OperationPhase::UserStopped
            } else {
                OperationPhase::Unknown
            };
            transition(
                &plan,
                &mut record,
                phase,
                format!("Recovery retained incompatible or partial candidate evidence: {error}"),
            )?;
            return Err(error);
        }
        if durable_scope.is_none() {
            if matches!(
                record.phase,
                OperationPhase::Completed | OperationPhase::Failed | OperationPhase::UserStopped
            ) {
                return Ok(record);
            }
            return Err(
                "unfinished operation has no reservation; no state will be inferred or replayed"
                    .into(),
            );
        }
        let reservation = protocol::recover_reservation(
            &plan.origin,
            plan.target.as_ref().map(|target| &target.origin),
            &plan.context.state_dir,
            &plan.operation_id,
        )?;
        let prepared_generation = reservation.stop_generation;
        let panel = crate::helper_ui::Panel::new(&plan.context.locale, &plan)?;
        panel.show(Phase::Reconciling, None);
        let result: Result<(), String> = (|| {
            let owner = read_lifecycle(&plan)?;
            if owner.stop_generation != reservation.stop_generation
                || coordinator::executable_identity(&owner.helper_image.path)? != owner.helper_image
            {
                return Err(
                    "durable original/helper identity differs from reserved operation".into(),
                );
            }
            let manager = read_evidence(&plan)?;
            let no_spawn =
                no_spawn_proved(&record, manager.as_ref(), manager_artifacts_absent(&plan)?);
            if no_spawn && durable.candidate.is_none() && !record.installed && !record.applied {
                if let Ok(original) = status(&plan) {
                    if same_original(&owner, &original)
                        && verify_status(&plan, &original).is_ok()
                        && original.ui_ready
                        && original.control_ready
                    {
                        if reservation.user_stop_requested()? {
                            return Err("user stop pending while original daemon is still running; wait for shutdown".into());
                        }
                        coordinator::recheck_plan(&plan)?;
                        no_spawn_original_disk_unchanged(&plan)?;
                        reject_other_install_processes(&plan, Some(original.pid))?;
                        reject_other_active_profiles(&plan)?;
                        if reservation.user_stop_requested()? {
                            return Err(
                                "user stop superseded original preflight reconciliation".into()
                            );
                        }
                        if durable_scope
                            .as_ref()
                            .and_then(|scope| scope.startup_origin.as_ref())
                            .is_some_and(|selected| selected != &plan.origin)
                        {
                            return Err(
                                "candidate startup selection cannot be released as ready original"
                                    .into(),
                            );
                        }
                        record.installed = false;
                        record.applied = false;
                        // Retain historical candidate evidence even on original restoration.
                        transition(&plan, &mut record, OperationPhase::Failed, "No manager was spawned; verified exact original PID/instance/image/source/profile remains ready")?;
                        reservation.release()?;
                        return Ok(());
                    }
                }
            }
            if record.execution_fence == ExecutionFence::ManagerIntent {
                return Err("manager spawn/PID or exit/drain evidence is not durable; reservation retained, no replay".into());
            }
            if manager.is_some() != (record.execution_fence == ExecutionFence::ManagerExited)
                || record.execution_fence == ExecutionFence::NoSpawn && !no_spawn
            {
                return Err(
                    "manager artifacts and durable execution fence disagree; reservation retained"
                        .into(),
                );
            }
            let mut manager_failed = false;
            if let Some(manager) = manager {
                if manager.uid != unsafe { libc::geteuid() }
                    || manager.pid != manager.pgid
                    || manager.program_sha256.len() != 64
                    || manager.started_at_ms == 0
                {
                    return Err("manager execution identity evidence is incomplete".into());
                }
                if manager.state != ManagerState::Exited
                    || !manager.drained
                    || !manager_quiescent(&manager)?
                {
                    return Err("manager or its process group/output has not durably completed; no replay/release".into());
                }
                manager_failed = manager.exit_code != Some(0);
            }
            if no_spawn && !matches!(plan.action, coordinator::UpdateAction::ApplyInstalled) {
                if !stopped(&plan, owner.original_pid)? {
                    return Err("old daemon did not finish shutdown; no manager was run".into());
                }
                reject_other_install_processes(&plan, None)?;
                reject_other_active_profiles(&plan)?;
                let original = coordinator::restored_original(&plan)?;
                selected_origin(
                    durable_scope.as_ref(),
                    &original,
                    StartupPurpose::Original,
                    &plan,
                )?;
                if reservation.user_stop_requested()? {
                    transition(&plan, &mut record, OperationPhase::UserStopped, "User stopped the original daemon before any manager ran; no automatic restart")?;
                } else {
                    transition(&plan, &mut record, OperationPhase::Failed, "No installation manager ran; unchanged original app is stopped. Start it manually if desired")?;
                }
                reservation.release()?;
                return Ok(());
            }
            let old_pid = owner.original_pid;
            if process_live(old_pid) {
                return Err(
                    "original daemon process still exists; no other process will be stopped".into(),
                );
            }
            let restored_running = record.phase == OperationPhase::Failed
                && !record.installed
                && !record.applied
                && coordinator::restored_original(&plan)
                    .ok()
                    .is_some_and(|original| {
                        selected_origin(
                            durable_scope.as_ref(),
                            &original,
                            StartupPurpose::Original,
                            &plan,
                        )
                        .is_ok()
                            && status(&plan).ok().is_some_and(|reply| {
                                new_ready(
                                    &plan,
                                    &original,
                                    StartupPurpose::Original,
                                    durable.candidate.as_ref(),
                                    &reply,
                                )
                                .unwrap_or(false)
                            })
                    });
            let installed = if manager_failed || restored_running || exact_historical_original {
                None
            } else {
                match candidate(&plan) {
                    Ok(candidate) => {
                        pin_state(
                            durable.candidate.as_ref(),
                            durable_scope.as_ref(),
                            &candidate,
                            StartupPurpose::Candidate,
                            &plan,
                        )?;
                        Some(candidate)
                    }
                    Err(error)
                        if durable.candidate.is_some()
                            || durable_scope
                                .as_ref()
                                .and_then(|scope| scope.startup_origin.as_ref())
                                .is_some_and(|origin| origin != &plan.origin) =>
                    {
                        return Err(format!(
                            "durably pinned candidate cannot be verified: {error}"
                        ));
                    }
                    Err(_) => None,
                }
            };
            let Some(installed) = installed else {
                let original = coordinator::restored_original(&plan)?;
                selected_origin(
                    durable_scope.as_ref(),
                    &original,
                    StartupPurpose::Original,
                    &plan,
                )?;
                let detail = if manager_failed {
                    "Installer reported failure; the CURRENT signed original image/source is verified, but other installer effects remain unknown"
                } else {
                    "Installer exited successfully but the CURRENT signed original image is unchanged"
                };
                if let Ok(current) = status(&plan) {
                    if new_ready(
                        &plan,
                        &original,
                        StartupPurpose::Original,
                        durable.candidate.as_ref(),
                        &current,
                    )? {
                        reject_other_install_processes(&plan, Some(current.pid))?;
                        reject_other_active_profiles(&plan)?;
                        if reservation.user_stop_requested()? {
                            return Err("user stop arrived while restored instance was active; wait for its shutdown".into());
                        }
                        record.installed = false;
                        record.applied = false;
                        transition(&plan, &mut record, OperationPhase::Failed, format!("{detail}; verified fresh original instance ready on captured profile, NOT an upgrade"))?;
                        reservation.release()?;
                        return Ok(());
                    }
                    return Err("another or unready daemon occupies original profile; no process will be stopped".into());
                }
                if !stopped(&plan, old_pid)? {
                    return Err("original app lifecycle has not finalized".into());
                }
                reject_other_install_processes(&plan, None)?;
                reject_other_active_profiles(&plan)?;
                record.installed = false;
                record.applied = false;
                if reservation.user_stop_requested()? {
                    transition(
                        &plan,
                        &mut record,
                        OperationPhase::UserStopped,
                        format!("{detail}; user stop forbids revival"),
                    )?;
                    reservation.release()?;
                    return Ok(());
                }
                if !start {
                    transition(&plan, &mut record, OperationPhase::Failed, format!("{detail}; explicit recover --start can restore the same version, NOT an upgrade"))?;
                    return Ok(());
                }
                // Persist Original purpose before selecting A; after a crash,
                // image/origin equality alone cannot distinguish a Local rebuild.
                transition(
                    &plan,
                    &mut record,
                    OperationPhase::Failed,
                    format!("{detail}; restoring verified original version, NOT an upgrade"),
                )?;
                panel.show(Phase::RestoringOriginal, None);
                authorize_pair(
                    &plan,
                    &reservation,
                    &mut record,
                    &original,
                    StartupPurpose::Original,
                )?;
                let mut child = launch(&plan, &reservation, &original)?;
                wait_new(
                    &plan,
                    &reservation,
                    &original,
                    StartupPurpose::Original,
                    durable.candidate.as_ref(),
                    &mut child,
                    &panel,
                )?;
                if reservation.user_stop_requested()? {
                    return Err("user stopped original image during recovery".into());
                }
                transition(&plan, &mut record, OperationPhase::Failed, format!("{detail}; current original version restarted on captured profile, NOT an upgrade or rollback guarantee"))?;
                reservation.release()?;
                return Ok(());
            };
            record.installed = true;
            if durable.candidate.is_none() && !start {
                // Inspection cannot turn a current disk observation into durable pins.
                transition(&plan, &mut record, OperationPhase::Unknown, "Candidate observed but no verified pair has been durably pinned; explicit recover --start required")?;
                return Ok(());
            }
            // A stopped user may settle an independently verified complete pair
            // without acquiring a start token, even when --start was requested.
            // All manager, lifecycle, profile, process and pair checks precede release.
            if reservation.user_stop_requested()? {
                if !complete_stopped_candidate(
                    durable.candidate.as_ref(),
                    snapshot(&plan)?.as_ref(),
                    &installed,
                ) {
                    return Err("user stopped with incomplete candidate startup evidence; reservation retained".into());
                }
                if status(&plan).is_ok() {
                    return Err(
                        "daemon still occupies the stopped profile; wait for shutdown".into(),
                    );
                }
                if !stopped(&plan, old_pid)? {
                    return Err("old profile lifecycle/socket has not fully stopped".into());
                }
                reject_other_install_processes(&plan, None)?;
                reject_other_active_profiles(&plan)?;
                recheck_stopped_candidate(&plan, durable.candidate.as_ref(), &installed)?;
                transition(
                    &plan,
                    &mut record,
                    OperationPhase::UserStopped,
                    "Verified pinned installation is stopped; user stop forbids revival",
                )?;
                reservation.release()?;
                return Ok(());
            }
            if let Ok(current) = status(&plan) {
                if durable.candidate.is_some()
                    && new_ready(
                        &plan,
                        &installed,
                        StartupPurpose::Candidate,
                        record.candidate.as_ref(),
                        &current,
                    )?
                {
                    reject_other_install_processes(&plan, Some(current.pid))?;
                    reject_other_active_profiles(&plan)?;
                    if reservation.user_stop_requested()? {
                        return Err("user stop arrived while updated instance was active; wait for its shutdown".into());
                    }
                    candidate_completion(
                        &plan,
                        &reservation,
                        record.candidate.as_ref(),
                        &installed,
                    )?;
                    record.applied = true;
                    panel.show(Phase::Finalizing, None);
                    transition(
                        &plan,
                        &mut record,
                        OperationPhase::Completed,
                        "Reconciled verified new instance and UI/control readiness",
                    )?;
                    candidate_completion(
                        &plan,
                        &reservation,
                        record.candidate.as_ref(),
                        &installed,
                    )?;
                    reservation.release()?;
                    return Ok(());
                }
                return Err("another or unready live daemon occupies this profile; recovery will not stop it".into());
            }
            if !stopped(&plan, old_pid)? {
                return Err("old profile lifecycle/socket has not fully stopped".into());
            }
            reject_other_install_processes(&plan, None)?;
            reject_other_active_profiles(&plan)?;
            if reservation.user_stop_requested()? {
                recheck_stopped_candidate(&plan, durable.candidate.as_ref(), &installed)?;
                transition(
                    &plan,
                    &mut record,
                    OperationPhase::UserStopped,
                    "User stop superseded update; updater will not restart this profile",
                )?;
                reservation.release()?;
                return Ok(());
            }
            if !start {
                transition(&plan, &mut record, OperationPhase::Failed, "Installation verified but not applied. Explicit recover --start required to start this profile")?;
                return Ok(());
            }
            authorize_pair(
                &plan,
                &reservation,
                &mut record,
                &installed,
                StartupPurpose::Candidate,
            )?;
            panel.show(Phase::Starting, None);
            let mut child = launch(&plan, &reservation, &installed)?;
            wait_new(
                &plan,
                &reservation,
                &installed,
                StartupPurpose::Candidate,
                record.candidate.as_ref(),
                &mut child,
                &panel,
            )?;
            if reservation.user_stop_requested()? {
                return Err("user stop superseded recovered instance before completion".into());
            }
            candidate_completion(&plan, &reservation, record.candidate.as_ref(), &installed)?;
            record.applied = true;
            panel.show(Phase::Finalizing, None);
            transition(
                &plan,
                &mut record,
                OperationPhase::Completed,
                "Verified installed image and new ready profile",
            )?;
            candidate_completion(&plan, &reservation, record.candidate.as_ref(), &installed)?;
            reservation.release()?;
            Ok(())
        })();
        if let Err(error) = &result {
            let stopped_by_user = protocol::stop_generation(&plan.context.state_dir)
                .is_ok_and(|generation| generation != prepared_generation);
            let phase = if stopped_by_user || record.phase == OperationPhase::UserStopped {
                OperationPhase::UserStopped
            } else {
                OperationPhase::Unknown
            };
            let journal_error = transition(&plan, &mut record, phase, format!("Recovery incomplete: {error}. Reservation retained; no automatic retry/rollback")).err();
            if read_lifecycle(&plan)
                .ok()
                .is_some_and(|owner| stopped(&plan, owner.original_pid).unwrap_or(false))
            {
                panel.activate();
            }
            let diagnostic = match journal_error {
                Some(journal_error) => {
                    format!("{error}\n\nCould not save recovery result: {journal_error}")
                }
                None => error.clone(),
            };
            panel.show(
                if phase == OperationPhase::UserStopped {
                    Phase::UserStopped
                } else {
                    Phase::Attention(Attention::Recovery)
                },
                Some(&diagnostic),
            );
            retain(&panel);
        } else if record.phase == OperationPhase::Completed {
            panel.close();
        } else {
            let phase = if record.phase == OperationPhase::UserStopped {
                Phase::UserStopped
            } else if !record.installed {
                Phase::Attention(Attention::OriginalUnchanged)
            } else {
                Phase::Attention(Attention::Interrupted)
            };
            panel.show(phase, Some(&record.detail));
            retain(&panel);
        }
        return result.map(|()| record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coordinator::{UpdateAction, UpdateContext};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    fn plan(root: &Path) -> UpdatePlan {
        let root = fs::canonicalize(root).unwrap();
        let state = root.join("state");
        let config = root.join("config");
        fs::create_dir_all(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
        let running = ExecutableIdentity {
            path: root.join("original/Contents/MacOS/herdr-desktop-pet"),
            sha256: "a".repeat(64),
        };
        let origin = InstallOrigin::Local {
            root: root.join("original"),
        };
        UpdatePlan {
            version: protocol::UPDATER_PROTOCOL,
            operation_id: "0123456789abcdef0123456789abcdef".into(),
            context: UpdateContext {
                executable: running.path.clone(),
                version: "0.2.0".into(),
                instance_id: "original-instance".into(),
                config_dir: config,
                host_plugin_config_dir: None,
                state_dir: state.clone(),
                herdr_socket: root.join("host.sock"),
                running: running.clone(),
                running_origin: Some(Box::new(origin.clone())),
                assets_override: None,
                environment: BTreeMap::new(),
                locale: "en".into(),
            },
            origin,
            target: None,
            action: UpdateAction::LocalRebuild,
            running,
            candidate: None,
        }
    }

    fn live_status(plan: &UpdatePlan) -> Status {
        let c = &plan.context;
        Status {
            ok: true,
            running: true,
            shutdown: false,
            ready: true,
            ui_ready: true,
            control_ready: true,
            instance_id: c.instance_id.clone(),
            running_sha256: plan.running.sha256.clone(),
            pid: std::process::id(),
            executable_path: plan.running.path.clone(),
            config_dir: c.config_dir.clone(),
            state_dir: c.state_dir.clone(),
            herdr_socket: c.herdr_socket.clone(),
            observation_mode: None,
            assets_override: c.assets_override.clone(),
        }
    }

    #[test]
    fn configured_socket_identity_does_not_require_a_listener_or_parent() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("absent/host.sock");
        assert!(same_endpoint(&missing, &missing));
        for invalid in [
            Path::new(""),
            Path::new("host.sock"),
            Path::new("absent/host.sock"),
        ] {
            assert!(!same_endpoint(invalid, invalid));
            assert!(!same_endpoint(invalid, &missing));
            assert!(!same_endpoint(&missing, invalid));
        }
        assert!(!same_endpoint(
            &missing,
            &root.path().join("absent/other.sock")
        ));

        let directory = root.path().join("real");
        fs::create_dir(&directory).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&directory, &alias).unwrap();
        assert!(same_endpoint(
            &directory.join("host.sock"),
            &alias.join("host.sock")
        ));
        assert!(!same_endpoint(
            &directory.join("host.sock"),
            &alias.join("other.sock")
        ));

        let socket = directory.join("host.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let leaf_alias = root.path().join("leaf.sock");
        std::os::unix::fs::symlink(&socket, &leaf_alias).unwrap();
        assert!(same_endpoint(&leaf_alias, &socket));
        assert!(same_endpoint(&socket, &leaf_alias));
    }

    #[test]
    fn tern_status_and_readiness_keep_configured_socket_proof_without_herdr_listener() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        plan.context.herdr_socket = root.path().join("absent/host.sock");
        plan.context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "tern".into());
        // Use the actual test process image to exercise the complete live proof.
        plan.running.path = process_path(std::process::id()).unwrap().unwrap();
        plan.context.executable = plan.running.path.clone();
        plan.context.running = plan.running.clone();
        let mut reply = live_status(&plan);
        reply.observation_mode = Some(ObservationMode::Tern);
        assert!(verify_status(&plan, &reply).is_ok());
        let installed = VerifiedInstallation {
            image: plan.running.clone(),
            origin: plan.origin.clone(),
        };
        reply.instance_id = "fresh-instance".into();
        for invalid in [
            PathBuf::new(),
            PathBuf::from("relative.sock"),
            root.path().join("absent/other.sock"),
        ] {
            reply.herdr_socket = invalid;
            for purpose in [StartupPurpose::Original, StartupPurpose::Candidate] {
                assert!(!new_ready(&plan, &installed, purpose, None, &reply).unwrap());
            }
            reply.instance_id = plan.context.instance_id.clone();
            assert!(verify_status(&plan, &reply).is_err());
            reply.instance_id = "fresh-instance".into();
        }
    }

    #[test]
    fn captured_mode_controls_launch_and_live_proof_without_legacy_tern_fallback() {
        for mode in [
            ObservationMode::Herdr,
            ObservationMode::Tern,
            ObservationMode::Both,
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut plan = plan(root.path());
            if mode != ObservationMode::Herdr {
                plan.context
                    .environment
                    .insert(OBSERVATION_MODE_ENV.into(), mode.as_str().into());
            }
            let installed = VerifiedInstallation {
                image: plan.running.clone(),
                origin: plan.origin.clone(),
            };
            let cmd = start_command(&plan, &installed, "captured-token", 7).unwrap();
            let args: Vec<_> = cmd.get_args().collect();
            if mode.supports_tern() {
                assert!(args.windows(2).any(|pair| {
                    pair[0].to_str() == Some("--host") && pair[1].to_str() == Some(mode.as_str())
                }));
            } else {
                assert!(!args.iter().any(|arg| arg.to_str() == Some("--host")));
            }
            assert_eq!(
                cmd.get_envs()
                    .find(|(key, _)| key.to_str() == Some(OBSERVATION_MODE_ENV))
                    .and_then(|(_, value)| value),
                Some(std::ffi::OsStr::new(mode.as_str()))
            );
            let mut reply = live_status(&plan);
            assert_eq!(
                status_mode_matches(&plan, &reply).unwrap(),
                !mode.supports_tern()
            );
            if mode.supports_tern() {
                assert!(verify_status(&plan, &reply)
                    .unwrap_err()
                    .contains("observation mode"));
                assert!(
                    !new_ready(&plan, &installed, StartupPurpose::Original, None, &reply).unwrap()
                );
            }
            reply.observation_mode = Some(mode);
            assert!(status_mode_matches(&plan, &reply).unwrap());
            reply.observation_mode = Some(if mode == ObservationMode::Herdr {
                ObservationMode::Tern
            } else {
                ObservationMode::Herdr
            });
            assert!(!status_mode_matches(&plan, &reply).unwrap());
            assert!(verify_status(&plan, &reply)
                .unwrap_err()
                .contains("observation mode"));
            assert!(
                !new_ready(&plan, &installed, StartupPurpose::Candidate, None, &reply).unwrap()
            );
        }
    }

    #[test]
    fn invalid_captured_mode_never_defaults_to_legacy_herdr() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        plan.context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "TERN".into());
        let installed = VerifiedInstallation {
            image: plan.running.clone(),
            origin: plan.origin.clone(),
        };
        assert!(start_command(&plan, &installed, "captured-token", 7).is_err());
        assert!(verify_status(&plan, &live_status(&plan)).is_err());
    }

    #[test]
    fn captured_managed_host_drift_cannot_be_replaced_by_socket_or_path() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let captured_host = root.path().join("captured-host");
        let changed_host = root.path().join("changed-host");
        fs::write(&captured_host, b"host").unwrap();
        fs::write(&changed_host, b"host").unwrap();
        let captured_host = fs::canonicalize(captured_host).unwrap();
        plan.origin = InstallOrigin::Herdr {
            host: captured_host.clone(),
            host_config_dir: root.path().join("host-config"),
            checkout_root: root.path().join("checkout"),
            plugin_root: root.path().join("checkout/plugin"),
            source: "owner/repository".into(),
            requested_ref: None,
            resolved_commit: None,
            enabled: true,
        };
        plan.context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "tern".into());
        plan.context
            .environment
            .insert("HERDR_BIN_PATH".into(), changed_host.display().to_string());
        let installed = VerifiedInstallation {
            image: plan.running.clone(),
            origin: plan.origin.clone(),
        };
        assert!(start_command(&plan, &installed, "token", 1)
            .unwrap_err()
            .contains("captured host routing changed"));
        plan.context
            .environment
            .insert("HERDR_BIN_PATH".into(), captured_host.display().to_string());
        let cmd = start_command(&plan, &installed, "token", 1).unwrap();
        assert!(cmd.get_args().collect::<Vec<_>>().windows(2).any(|pair| {
            pair[0].to_str() == Some("--host") && pair[1].to_str() == Some("tern")
        }));
        assert_eq!(
            cmd.get_envs()
                .find(|(key, _)| key.to_str() == Some("HERDR_BIN_PATH"))
                .and_then(|(_, value)| value),
            Some(captured_host.as_os_str())
        );
    }

    #[test]
    fn release_helper_capability_follows_captured_encoding_before_stop() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let legacy = r#"{"protocol":3}"#;
        assert!(coordinator::validate_helper_capabilities(legacy, &plan.context).is_ok());
        assert!(
            coordinator::validate_helper_capabilities(r#"{"protocol":2}"#, &plan.context).is_err()
        );
        for mode in [
            ObservationMode::Herdr,
            ObservationMode::Tern,
            ObservationMode::Both,
        ] {
            plan.context
                .environment
                .insert(OBSERVATION_MODE_ENV.into(), mode.as_str().into());
            assert!(coordinator::validate_helper_capabilities(legacy, &plan.context).is_err());
            assert!(coordinator::validate_helper_capabilities(
                r#"{"protocol":3,"observation_mode":1}"#,
                &plan.context
            )
            .is_ok());
            assert!(coordinator::validate_helper_capabilities(
                r#"{"protocol":2,"observation_mode":1}"#,
                &plan.context
            )
            .is_err());
            for incompatible in [
                r#"{"protocol":3,"observation_mode":0}"#,
                r#"{"protocol":3,"observation_mode":2}"#,
                r#"{"protocol":3,"observation_mode":"1"}"#,
                r#"{"protocol":3,"observation_mode":null}"#,
            ] {
                assert!(
                    coordinator::validate_helper_capabilities(incompatible, &plan.context).is_err()
                );
            }
        }
        plan.context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "TERN".into());
        assert!(coordinator::validate_helper_capabilities(
            r#"{"protocol":3,"observation_mode":1}"#,
            &plan.context
        )
        .is_err());
    }

    fn herdr_pair(plan: &UpdatePlan, commit: char, hash: char) -> VerifiedInstallation {
        let root = plan.context.state_dir.parent().unwrap();
        VerifiedInstallation {
            image: ExecutableIdentity {
                path: root.join("checkout/plugin/Contents/MacOS/herdr-desktop-pet"),
                sha256: hash.to_string().repeat(64),
            },
            origin: InstallOrigin::Herdr {
                host: root.join("host"),
                host_config_dir: root.join("host-config"),
                checkout_root: root.join("checkout"),
                plugin_root: root.join("checkout/plugin"),
                source: "owner/repository".into(),
                requested_ref: Some("beta".into()),
                resolved_commit: Some(commit.to_string().repeat(40)),
                enabled: true,
            },
        }
    }

    #[test]
    fn durable_candidate_pair_rejects_commit_only_image_only_and_path_drift() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let source = herdr_pair(&plan, 'a', 'a');
        let installed = herdr_pair(&plan, 'b', 'b');
        plan.origin = source.origin;
        plan.context.running_origin = Some(Box::new(plan.origin.clone()));
        let mut saved = record(&plan);
        saved.candidate = Some(installed.image.clone());
        journal(&plan, &saved).unwrap();
        let durable = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
            .unwrap()
            .unwrap();
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(installed.origin.clone()),
            markers_complete: true,
        };
        assert!(pin_state(
            durable.candidate.as_ref(),
            Some(&selected),
            &installed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_ok());
        for changed in [
            herdr_pair(&plan, 'c', 'b'),
            herdr_pair(&plan, 'b', 'c'),
            VerifiedInstallation {
                image: ExecutableIdentity {
                    path: root.path().join("different/herdr-desktop-pet"),
                    sha256: installed.image.sha256.clone(),
                },
                origin: installed.origin.clone(),
            },
        ] {
            assert!(pin_state(
                durable.candidate.as_ref(),
                Some(&selected),
                &changed,
                StartupPurpose::Candidate,
                &plan
            )
            .is_err());
        }
        assert_eq!(
            protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
                .unwrap()
                .unwrap()
                .candidate,
            Some(installed.image)
        );
    }

    #[test]
    fn incomplete_candidate_pair_never_reconstructs_origin_or_image() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let source = herdr_pair(&plan, 'a', 'a');
        let installed = herdr_pair(&plan, 'b', 'b');
        plan.origin = source.origin.clone();
        let mut saved = record(&plan);
        saved.candidate = Some(installed.image.clone());
        journal(&plan, &saved).unwrap();
        let pinned = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
            .unwrap()
            .unwrap();
        let unselected = protocol::ReservationSnapshot {
            startup_origin: None,
            markers_complete: true,
        };
        assert!(pin_state(
            pinned.candidate.as_ref(),
            Some(&unselected),
            &installed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(installed.origin.clone()),
            markers_complete: false,
        };
        assert!(pin_state(
            None,
            Some(&selected),
            &installed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        let changed = herdr_pair(&plan, 'c', 'b');
        assert!(pin_state(
            pinned.candidate.as_ref(),
            Some(&unselected),
            &changed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        assert!(pin_state(
            Some(&installed.image),
            Some(&selected),
            &changed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        let old_selection = protocol::ReservationSnapshot {
            startup_origin: Some(source.origin),
            markers_complete: true,
        };
        assert!(pin_state(
            Some(&installed.image),
            Some(&old_selection),
            &installed,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        assert!(selected_origin(
            Some(&selected),
            &VerifiedInstallation {
                image: plan.running.clone(),
                origin: plan.origin.clone(),
            },
            StartupPurpose::Original,
            &plan
        )
        .is_err());
        assert_eq!(
            protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
                .unwrap()
                .unwrap()
                .candidate,
            Some(installed.image)
        );
    }

    #[test]
    fn reproducible_local_image_does_not_select_original_proof() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let repeated = VerifiedInstallation {
            image: plan.running.clone(),
            origin: plan.origin.clone(),
        };
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(plan.origin.clone()),
            markers_complete: true,
        };
        assert!(pin_state(
            None,
            Some(&selected),
            &repeated,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        assert!(pin_state(
            None,
            Some(&selected),
            &repeated,
            StartupPurpose::Original,
            &plan
        )
        .is_ok());
        assert!(pin_state(
            Some(&plan.running),
            Some(&selected),
            &repeated,
            StartupPurpose::Candidate,
            &plan
        )
        .is_ok());
    }

    #[test]
    fn unselected_exact_original_remains_eligible_for_safe_reconciliation() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let source = herdr_pair(&plan, 'a', 'a');
        let candidate = herdr_pair(&plan, 'b', 'b');
        plan.origin = source.origin.clone();
        let original = VerifiedInstallation {
            image: plan.running.clone(),
            origin: source.origin,
        };
        let unselected = protocol::ReservationSnapshot {
            startup_origin: None,
            markers_complete: true,
        };
        assert!(selected_origin(
            Some(&unselected),
            &original,
            StartupPurpose::Original,
            &plan
        )
        .is_ok());
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(candidate.origin),
            markers_complete: false,
        };
        assert!(
            selected_origin(Some(&selected), &original, StartupPurpose::Original, &plan).is_err()
        );
    }

    #[test]
    fn historical_image_only_reconciles_only_independently_verified_original() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let source = herdr_pair(&plan, 'a', 'a');
        let candidate = herdr_pair(&plan, 'b', 'b');
        plan.origin = source.origin.clone();
        plan.context.running_origin = Some(Box::new(plan.origin.clone()));
        let mut saved = record(&plan);
        saved.candidate = Some(candidate.image.clone());
        journal(&plan, &saved).unwrap();
        let unselected = protocol::ReservationSnapshot {
            startup_origin: None,
            markers_complete: true,
        };
        let original = VerifiedInstallation {
            image: plan.running.clone(),
            origin: source.origin.clone(),
        };
        assert!(historical_original_allowed(
            &plan,
            &saved,
            Some(&unselected)
        ));
        assert!(pin_state(
            saved.candidate.as_ref(),
            Some(&unselected),
            &candidate,
            StartupPurpose::Candidate,
            &plan
        )
        .is_err());
        // A matching policy selection alone does not prove the installed original:
        // this fixture has no signed original on disk.
        assert!(historical_original(&plan, &saved, Some(&unselected)).is_none());
        assert!(selected_origin(
            Some(&unselected),
            &original,
            StartupPurpose::Original,
            &plan
        )
        .is_ok());
        let selected_original = protocol::ReservationSnapshot {
            startup_origin: Some(source.origin),
            markers_complete: true,
        };
        assert!(!historical_original_allowed(
            &plan,
            &saved,
            Some(&selected_original)
        ));
        saved.phase = OperationPhase::Failed;
        assert!(historical_original_allowed(
            &plan,
            &saved,
            Some(&selected_original)
        ));
        saved.installed = true;
        assert!(!historical_original_allowed(
            &plan,
            &saved,
            Some(&selected_original)
        ));
        saved.installed = false;
        let selected_candidate = protocol::ReservationSnapshot {
            startup_origin: Some(candidate.origin),
            markers_complete: true,
        };
        assert!(!historical_original_allowed(
            &plan,
            &saved,
            Some(&selected_candidate)
        ));
        assert_eq!(
            protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
                .unwrap()
                .unwrap()
                .candidate,
            Some(candidate.image)
        );
    }

    #[test]
    fn reproducible_local_candidate_never_becomes_original_by_pair_equality() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let mut saved = record(&plan);
        saved.candidate = Some(plan.running.clone());
        let repeated = VerifiedInstallation {
            image: plan.running.clone(),
            origin: plan.origin.clone(),
        };
        let unselected = protocol::ReservationSnapshot {
            startup_origin: None,
            markers_complete: false,
        };
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(plan.origin.clone()),
            markers_complete: true,
        };
        assert!(!historical_original_allowed(
            &plan,
            &saved,
            Some(&unselected)
        ));
        assert!(pin_state(
            saved.candidate.as_ref(),
            Some(&selected),
            &repeated,
            StartupPurpose::Candidate,
            &plan
        )
        .is_ok());
        for phase in [
            OperationPhase::Preparing,
            OperationPhase::Starting,
            OperationPhase::Unknown,
            OperationPhase::UserStopped,
            OperationPhase::Completed,
        ] {
            saved.phase = phase;
            assert!(!historical_original_allowed(&plan, &saved, Some(&selected)));
        }
        saved.phase = OperationPhase::Failed;
        saved.installed = true;
        assert!(!historical_original_allowed(&plan, &saved, Some(&selected)));
        saved.installed = false;
        assert!(historical_original_allowed(&plan, &saved, Some(&selected)));
        // Eligibility still requires independent restored_original verification.
        assert!(historical_original(&plan, &saved, Some(&selected)).is_none());
    }

    #[test]
    fn stopped_candidate_settlement_needs_exact_complete_historical_pair() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        let source = herdr_pair(&plan, 'a', 'a');
        let installed = herdr_pair(&plan, 'b', 'b');
        plan.origin = source.origin;
        let mut saved = record(&plan);
        saved.candidate = Some(installed.image.clone());
        journal(&plan, &saved).unwrap();
        let pinned = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
            .unwrap()
            .unwrap();
        let selected = protocol::ReservationSnapshot {
            startup_origin: Some(installed.origin.clone()),
            markers_complete: true,
        };
        // Both explicit recover modes use the same stop settlement, before
        // either mode can request a startup token.
        for _start in [false, true] {
            assert!(complete_stopped_candidate(
                pinned.candidate.as_ref(),
                Some(&selected),
                &installed
            ));
        }
        assert!(!complete_stopped_candidate(
            None,
            Some(&selected),
            &installed
        ));
        assert!(!complete_stopped_candidate(
            pinned.candidate.as_ref(),
            None,
            &installed
        ));
        assert!(!complete_stopped_candidate(
            pinned.candidate.as_ref(),
            Some(&protocol::ReservationSnapshot {
                markers_complete: false,
                ..selected.clone()
            }),
            &installed
        ));
        assert!(!complete_stopped_candidate(
            pinned.candidate.as_ref(),
            Some(&protocol::ReservationSnapshot {
                startup_origin: None,
                ..selected.clone()
            }),
            &installed
        ));
        for drifted in [herdr_pair(&plan, 'c', 'b'), herdr_pair(&plan, 'b', 'c')] {
            assert!(!complete_stopped_candidate(
                pinned.candidate.as_ref(),
                Some(&selected),
                &drifted
            ));
        }
        assert_eq!(
            protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
                .unwrap()
                .unwrap()
                .candidate,
            Some(installed.image)
        );
    }

    #[test]
    fn native_status_exchange_remains_compatible_with_read_only_control() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let listener = std::os::unix::net::UnixListener::bind(socket(&plan)).unwrap();
        let c = &plan.context;
        let response = serde_json::json!({
            "ok": true,
            "running": true,
            "shutdown": false,
            "ready": true,
            "ui_ready": true,
            "control_ready": true,
            "instance_id": c.instance_id,
            "running_sha256": plan.running.sha256,
            "pid": std::process::id(),
            "executable_path": plan.running.path,
            "config_dir": c.config_dir,
            "state_dir": c.state_dir,
            "herdr_socket": c.herdr_socket,
            "assets_override": c.assets_override,
        });
        let mut tern_response = response.clone();
        tern_response["observation_mode"] = serde_json::json!("tern");
        let mut tern_plan = plan.clone();
        tern_plan
            .context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "tern".into());
        let proof: Status = serde_json::from_value(tern_response.clone()).unwrap();
        assert!(status_mode_matches(&tern_plan, &proof).unwrap());
        tern_response["observation_mode"] = serde_json::json!("TERN");
        assert!(serde_json::from_value::<Status>(tern_response).is_err());
        let missing: Status = serde_json::from_value(response.clone()).unwrap();
        assert!(!status_mode_matches(&tern_plan, &missing).unwrap());
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            (&mut connection)
                .take(FRAME_LIMIT)
                .read_to_end_until_newline(&mut request)
                .unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            // Read-only Native LegacyRequest admits version 1, independently
            // of authenticated updater control frames (protocol 3).
            assert_eq!(request["version"], 1);
            assert_eq!(request["command"], "status");
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            connection.write_all(&bytes).unwrap();
        });
        let reply = status(&plan).unwrap();
        assert!(reply.ok && reply.running && reply.ready && reply.control_ready);
        assert_eq!(reply.observation_mode, None);
        assert!(status_mode_matches(&plan, &reply).unwrap());
        server.join().unwrap();
    }

    #[test]
    fn preflight_fence_rejects_manager_gap_and_conflicting_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let mut state = record(&plan);
        assert!(manager_artifacts_absent(&plan).unwrap());
        assert!(no_spawn_proved(
            &state,
            None,
            manager_artifacts_absent(&plan).unwrap()
        ));
        state.execution_fence = ExecutionFence::ManagerIntent;
        assert!(!no_spawn_proved(
            &state,
            None,
            manager_artifacts_absent(&plan).unwrap()
        ));
        state.execution_fence = ExecutionFence::ManagerExited;
        assert!(!no_spawn_proved(
            &state,
            None,
            manager_artifacts_absent(&plan).unwrap()
        ));
        state.execution_fence = ExecutionFence::NoSpawn;
        let updates = protocol::private_updates(&plan.context.state_dir).unwrap();
        protocol::write_private_bytes(
            &updates.join(format!("{}.manager.stdout", plan.operation_id)),
            b"partial manager output",
            0o600,
        )
        .unwrap();
        assert!(!manager_artifacts_absent(&plan).unwrap());
        assert!(!no_spawn_proved(
            &state,
            None,
            manager_artifacts_absent(&plan).unwrap()
        ));
    }
    #[test]
    fn repeated_run_preserves_every_existing_execution_fence_and_manager_evidence() {
        for fence in [
            ExecutionFence::NoSpawn,
            ExecutionFence::ManagerIntent,
            ExecutionFence::ManagerExited,
        ] {
            let root = tempfile::tempdir().unwrap();
            let plan = plan(root.path());
            let mut previous = record(&plan);
            previous.execution_fence = fence;
            previous.phase = OperationPhase::Failed;
            previous.detail = "retained outcome for explicit recovery".into();
            let journal_path =
                protocol::write_operation(&plan.context.state_dir, &previous).unwrap();
            let latest = journal_path.with_file_name("latest-operation.json");
            let updates = protocol::private_updates(&plan.context.state_dir).unwrap();
            let lifecycle_file = lifecycle_path(&plan).unwrap();
            let staged_helper = updates.join(format!("helper-{}", plan.operation_id));
            protocol::write_private_bytes(&staged_helper, b"private helper image", 0o700).unwrap();
            let owner = LifecycleEvidence {
                original_pid: 777_771,
                original_instance_id: plan.context.instance_id.clone(),
                original_image: plan.running.clone(),
                original_config_dir: plan.context.config_dir.clone(),
                original_state_dir: plan.context.state_dir.clone(),
                original_socket: plan.context.herdr_socket.clone(),
                original_assets: None,
                helper_pid: 777_772,
                helper_image: coordinator::executable_identity(&staged_helper).unwrap(),
                stop_generation: 0,
            };
            let owner_bytes = serde_json::to_vec(&owner).unwrap();
            protocol::write_private_bytes(&lifecycle_file, &owner_bytes, 0o600).unwrap();
            let marker = updates.join("profile.reservation.json");
            protocol::write_private_bytes(&marker, b"retained reservation", 0o600).unwrap();
            let manager_file = evidence_path(&plan).unwrap();
            let manager_bytes = serde_json::to_vec(&ManagerEvidence {
                program: plan.running.path.clone(),
                program_sha256: plan.running.sha256.clone(),
                uid: unsafe { libc::geteuid() },
                started_at_ms: 1,
                pid: Some(777_773),
                pgid: Some(777_773),
                state: if fence == ExecutionFence::ManagerExited {
                    ManagerState::Exited
                } else {
                    ManagerState::Intent
                },
                exit_code: (fence == ExecutionFence::ManagerExited).then_some(1),
                drained: fence == ExecutionFence::ManagerExited,
            })
            .unwrap();
            if fence != ExecutionFence::NoSpawn {
                protocol::write_private_bytes(&manager_file, &manager_bytes, 0o600).unwrap();
            }
            let before = fs::read(&journal_path).unwrap();
            let before_latest = fs::read(&latest).unwrap();
            assert!(run(plan.clone())
                .unwrap_err()
                .contains("already has a journal"));
            assert_eq!(fs::read(&journal_path).unwrap(), before);
            assert_eq!(fs::read(&latest).unwrap(), before_latest);
            assert_eq!(
                protocol::read_operation(&plan.context.state_dir, &plan.operation_id)
                    .unwrap()
                    .unwrap()
                    .execution_fence,
                fence
            );
            assert_eq!(fs::read(&lifecycle_file).unwrap(), owner_bytes);
            assert_eq!(fs::read(&marker).unwrap(), b"retained reservation");
            if fence != ExecutionFence::NoSpawn {
                assert_eq!(fs::read(&manager_file).unwrap(), manager_bytes);
            }
            // Rejected run must release its own lease without affecting durable recovery.
            let _lease =
                protocol::lock_operation(&plan.context.state_dir, &plan.operation_id).unwrap();
        }
    }

    #[test]
    fn existing_unreadable_journal_is_not_replaced_or_treated_as_new() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let path = protocol::private_updates(&plan.context.state_dir)
            .unwrap()
            .join(format!("{}.result.json", plan.operation_id));
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(run(plan).unwrap_err().contains("already has a journal"));
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_fifo());
    }

    #[test]
    fn no_spawn_same_path_apply_keeps_original_process_identity_distinct_from_disk_candidate() {
        let root = tempfile::tempdir().unwrap();
        let mut plan = plan(root.path());
        fs::create_dir_all(plan.running.path.parent().unwrap()).unwrap();
        fs::write(&plan.running.path, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&plan.running.path, fs::Permissions::from_mode(0o700)).unwrap();
        plan.running = coordinator::executable_identity(&plan.running.path).unwrap();
        plan.context.running = plan.running.clone();
        let original = LifecycleEvidence {
            original_pid: std::process::id(),
            original_instance_id: plan.context.instance_id.clone(),
            original_image: plan.running.clone(),
            original_config_dir: plan.context.config_dir.clone(),
            original_state_dir: plan.context.state_dir.clone(),
            original_socket: plan.context.herdr_socket.clone(),
            original_assets: None,
            helper_pid: 123,
            helper_image: plan.running.clone(),
            stop_generation: 0,
        };
        let live = Status {
            ok: true,
            running: true,
            shutdown: false,
            ready: true,
            ui_ready: true,
            control_ready: true,
            instance_id: original.original_instance_id.clone(),
            running_sha256: original.original_image.sha256.clone(),
            pid: original.original_pid,
            executable_path: original.original_image.path.clone(),
            config_dir: original.original_config_dir.clone(),
            state_dir: original.original_state_dir.clone(),
            herdr_socket: original.original_socket.clone(),
            observation_mode: None,
            assets_override: None,
        };
        let staged_replacement = root.path().join("replacement");
        fs::write(&staged_replacement, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&staged_replacement, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(staged_replacement, &plan.running.path).unwrap();
        let replacement = coordinator::executable_identity(&plan.running.path).unwrap();
        assert_ne!(replacement.sha256, plan.running.sha256);
        plan.action = UpdateAction::ApplyInstalled;
        plan.candidate = Some(replacement);
        assert!(same_original(&original, &live));
        assert!(no_spawn_original_disk_unchanged(&plan).is_ok());
        let mut mismatched = live;
        mismatched.running_sha256 = plan.candidate.as_ref().unwrap().sha256.clone();
        assert!(!same_original(&original, &mismatched));
        mismatched.running_sha256 = original.original_image.sha256.clone();
        mismatched.pid += 1;
        assert!(!same_original(&original, &mismatched));
        mismatched.pid = original.original_pid;
        mismatched.instance_id = "another-instance".into();
        assert!(!same_original(&original, &mismatched));
        mismatched.instance_id = original.original_instance_id.clone();
        mismatched.config_dir = root.path().join("another-profile");
        assert!(!same_original(&original, &mismatched));
        let mut stopped = Command::new("/usr/bin/true").spawn().unwrap();
        let stopped_pid = stopped.id();
        assert!(stopped.wait().unwrap().success());
        assert!(!process_live(stopped_pid));
        plan.action = UpdateAction::LocalRebuild;
        plan.candidate = None;
        assert!(no_spawn_original_disk_unchanged(&plan).is_err());
        let mut state = record(&plan);
        assert!(no_spawn_proved(&state, None, true));
        state.execution_fence = ExecutionFence::ManagerIntent;
        assert!(!no_spawn_proved(&state, None, true));
        state.execution_fence = ExecutionFence::ManagerExited;
        assert!(!no_spawn_proved(&state, None, true));
    }
    #[test]
    fn private_evidence_fifo_fails_without_writer_and_startup_marker_blocks_quiescence() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let path = evidence_path(&plan).unwrap();
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(read_evidence(&plan).is_err());
        assert!(startup_quiescent(&plan.context.config_dir).unwrap());
        protocol::write_private_bytes(
            &plan.context.config_dir.join("lifecycle.json"),
            br#"{"startup":{"pid":1,"claimed":true}}"#,
            0o600,
        )
        .unwrap();
        assert!(!startup_quiescent(&plan.context.config_dir).unwrap());
    }

    #[test]
    fn lifecycle_evidence_requires_exact_original_instance_and_profile() {
        let root = tempfile::tempdir().unwrap();
        let plan = plan(root.path());
        let helper = protocol::private_updates(&plan.context.state_dir)
            .unwrap()
            .join(format!("helper-{}", plan.operation_id));
        protocol::write_private_bytes(&helper, b"private staged helper", 0o700).unwrap();
        let original = LifecycleEvidence {
            original_pid: 42,
            original_instance_id: plan.context.instance_id.clone(),
            original_image: plan.running.clone(),
            original_config_dir: plan.context.config_dir.clone(),
            original_state_dir: plan.context.state_dir.clone(),
            original_socket: plan.context.herdr_socket.clone(),
            original_assets: None,
            helper_pid: 77,
            helper_image: ExecutableIdentity {
                path: fs::canonicalize(helper).unwrap(),
                sha256: "b".repeat(64),
            },
            stop_generation: 0,
        };
        let path = lifecycle_path(&plan).unwrap();
        protocol::write_private_bytes(&path, &serde_json::to_vec(&original).unwrap(), 0o600)
            .unwrap();
        assert_eq!(read_lifecycle(&plan).unwrap().original_pid, 42);
        let mut mismatched = original;
        mismatched.original_instance_id = "other-instance".into();
        protocol::write_private_bytes(&path, &serde_json::to_vec(&mismatched).unwrap(), 0o600)
            .unwrap();
        assert!(read_lifecycle(&plan).is_err());
        mismatched.original_instance_id = plan.context.instance_id.clone();
        mismatched.original_state_dir = root.path().join("another-profile");
        protocol::write_private_bytes(&path, &serde_json::to_vec(&mismatched).unwrap(), 0o600)
            .unwrap();
        assert!(read_lifecycle(&plan).is_err());
    }

    #[test]
    fn formula_conflicts_include_other_kegs_not_other_formulas() {
        let directory = tempfile::tempdir().unwrap();
        let cellar = fs::canonicalize(directory.path()).unwrap().join("Cellar");
        let formula = cellar.join("herdr-desktop-pet");
        let image = |keg: &str, app: &str| {
            cellar
                .join(keg)
                .join(app)
                .join("Contents/MacOS/herdr-desktop-pet")
        };
        let stable_old = image("herdr-desktop-pet/0.1", "HerdrDesktopPet.app");
        let stable_new = image("herdr-desktop-pet/0.2", "HerdrDesktopPet.app");
        let beta = image("herdr-desktop-pet-beta/0.2", "HerdrDesktopPetBeta.app");
        let sibling = image("herdr-desktop-pet-old/0.2", "HerdrDesktopPet.app");
        for executable in [&stable_old, &stable_new, &beta, &sibling] {
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::write(executable, b"test-owned image").unwrap();
        }
        let stable = InstallOrigin::Homebrew {
            brew: directory.path().join("bin/brew"),
            prefix: directory.path().to_path_buf(),
            cellar_formula_root: fs::canonicalize(&formula).unwrap(),
            formula: "herdr-desktop-pet".into(),
            locator: stable_new.clone(),
        };
        let root = coordinator::physical_install_root(&stable).unwrap();
        assert!(installation_process_in_scope(&stable_old, root).unwrap());
        assert!(installation_process_in_scope(&stable_new, root).unwrap());
        assert!(!installation_process_in_scope(&beta, root).unwrap());
        let mut migration = plan(directory.path());
        migration.origin = stable.clone();
        migration.target = Some(coordinator::ReleaseTarget {
            policy: coordinator::UpdatePolicy::StableAndBeta,
            release: coordinator::ReleaseCandidate {
                version: "0.3.3-beta.1".into(),
                channel: coordinator::ReleaseChannel::Beta,
                tag: "beta/0.3.3-beta.1".into(),
                release_id: 1,
                asset_name: "HerdrDesktopPet-v0.3.3-beta.1-macos-arm64.tar.gz".into(),
                archive_url: String::new(),
                archive_size: 1,
                sha256: "b".repeat(64),
                checksum_url: String::new(),
            },
            origin: InstallOrigin::Homebrew {
                brew: directory.path().join("bin/brew"),
                prefix: directory.path().to_path_buf(),
                cellar_formula_root: fs::canonicalize(cellar.join("herdr-desktop-pet-beta"))
                    .unwrap(),
                formula: "herdr-desktop-pet-beta".into(),
                locator: beta.clone(),
            },
            resolved_commit: None,
        });
        let roots = install_roots(&migration).unwrap();
        assert_eq!(roots.len(), 2);
        assert!(roots
            .iter()
            .any(|root| installation_process_in_scope(&stable_old, root).unwrap()));
        assert!(roots
            .iter()
            .any(|root| installation_process_in_scope(&beta, root).unwrap()));
        assert!(!roots
            .iter()
            .any(|root| installation_process_in_scope(&sibling, root).unwrap()));
        assert_eq!(install_roots(&plan(directory.path())).unwrap().len(), 1);
        assert!(!installation_process_in_scope(&sibling, root).unwrap());
        let alias = directory.path().join("opt-stable");
        std::os::unix::fs::symlink(formula.join("0.1"), &alias).unwrap();
        assert!(installation_process_in_scope(
            &alias.join("HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"),
            root,
        )
        .unwrap());
        fs::remove_file(&stable_old).unwrap();
        assert!(installation_process_in_scope(&stable_old, root).unwrap());
        assert!(installation_process_in_scope(
            &alias.join("HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"),
            root,
        )
        .is_err());
        assert!(installation_process_in_scope(
            &directory.path().join("missing/herdr-desktop-pet"),
            root,
        )
        .is_err());
    }
}
