use coordinator::protocol::{
    self, ExecutionFence, InstallationReservation, OperationLease, UpdateCommand,
    UpdateControlReply, UpdateControlRequest,
};
use coordinator::{ExecutableIdentity, InstallOrigin, OperationPhase, OperationRecord, UpdatePlan};
use herdr_update_coordinator as coordinator;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    let canonical_parent = |path: &Path| -> Option<PathBuf> {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    };
    canonical_parent(actual)
        .is_some_and(|actual| canonical_parent(captured).as_ref() == Some(&actual))
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
    {
        return Err("live daemon does not match immutable instance/image/profile/assets".into());
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
    let root = coordinator::physical_install_root(&plan.origin)?;
    for pid in pids.into_iter().take(read as usize) {
        if pid <= 0 || Some(pid as u32) == original_pid || pid as u32 == std::process::id() {
            continue;
        }
        let Some(path) = process_path(pid as u32)? else {
            continue;
        };
        if installation_process_in_scope(&path, root)? {
            return Err(format!("another live installation process ({pid}) uses {}; no other profile will be stopped", root.display()));
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
fn candidate(plan: &UpdatePlan) -> Result<ExecutableIdentity, String> {
    coordinator::installed_candidate(plan)
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
) -> Result<ExecutableIdentity, String> {
    reject_other_install_processes(plan, None)?;
    let image = candidate(plan)?;
    if !stopped(plan, old_pid)? {
        return Err("old daemon returned or shutdown is incomplete".into());
    }
    panel.pump();
    Ok(image)
}
fn launch(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    image: &ExecutableIdentity,
    recovering: bool,
) -> Result<Child, String> {
    if reservation.user_stop_requested()? {
        return Err("user stop superseded updater start".into());
    }
    let c = &plan.context;
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
    let token = if recovering {
        match reservation.issued_start_token()? {
            Some(token) => token,
            None => reservation.authorize_start()?,
        }
    } else {
        reservation.authorize_start()?
    };
    let mut cmd = Command::new(&image.path);
    cmd.arg("start")
        .arg("--config-dir")
        .arg(&c.config_dir)
        .arg("--state-dir")
        .arg(&c.state_dir)
        .arg("--socket")
        .arg(&c.herdr_socket);
    if let Some(assets) = &c.assets_override {
        cmd.arg("--assets").arg(assets);
    }
    cmd.env_clear()
        .envs(&c.environment)
        .env("HERDR_PLUGIN_CONFIG_DIR", &c.config_dir)
        .env("HERDR_PLUGIN_STATE_DIR", &c.state_dir)
        .env("HERDR_SOCKET_PATH", &c.herdr_socket)
        .env("HERDR_DESKTOP_PET_UPDATER_TOKEN", token)
        .env(
            "HERDR_DESKTOP_PET_UPDATER_GENERATION",
            reservation.stop_generation.to_string(),
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
    cmd.spawn()
        .map_err(|e| format!("candidate start failed: {e}"))
}
fn new_ready(
    plan: &UpdatePlan,
    image: &ExecutableIdentity,
    reply: &Status,
) -> Result<bool, String> {
    Ok(reply.ok
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
        && process_path(reply.pid)?.as_deref() == Some(image.path.as_path())
        && startup_quiescent(&plan.context.config_dir)?)
}
fn wait_new(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    image: &ExecutableIdentity,
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
            if start_completed && new_ready(plan, image, &reply)? {
                reject_other_install_processes(plan, Some(reply.pid))?;
                if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?
                    .is_empty()
                {
                    return Err(
                        "another profile shares installation while new instance started".into(),
                    );
                }
                return Ok(());
            }
        }
        panel.pump();
        thread::sleep(Duration::from_millis(150));
    }
    Err("new instance/image/profile/UI/control readiness not proved before timeout".into())
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
    if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?.is_empty() {
        return Err("another profile uses this installation".into());
    }
    let generation = protocol::stop_generation(&plan.context.state_dir)?;
    lifecycle(plan, &live, generation)?;
    if protocol::stop_generation(&plan.context.state_dir)? != generation {
        return Err("user stopped original during preflight".into());
    }
    read_lifecycle(plan)
}
fn prepare(
    plan: &UpdatePlan,
    reservation: &InstallationReservation,
    owner: &LifecycleEvidence,
) -> Result<(u32, Option<coordinator::ManagerCommand>), String> {
    if reservation.stop_generation != owner.stop_generation || reservation.user_stop_requested()? {
        return Err("user stop superseded update preflight".into());
    }
    coordinator::recheck_plan(plan)?;
    let command = coordinator::manager_command(plan)?;
    let live = status(plan)?;
    verify_status(plan, &live)?;
    if !same_original(owner, &live) || !live.ui_ready || !live.control_ready {
        return Err("reserved original PID/instance/profile/image changed since preflight".into());
    }
    reject_other_install_processes(plan, Some(live.pid))?;
    if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?.is_empty() {
        return Err("another profile uses this installation".into());
    }
    if reservation.user_stop_requested()? {
        return Err("user stop superseded update".into());
    }
    control(plan, reservation, UpdateCommand::Prepare)?;
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
    let panel = match crate::helper_ui::Panel::new(&plan.context.locale) {
        Ok(panel) => panel,
        Err(error) => {
            fail(&plan, &mut record, error.clone(), false, Some(reservation))?;
            let _ = ack(&plan, false, false, Some(&error));
            return Err(error);
        }
    };
    let (old_pid, manager_command) = match prepare(&plan, &reservation, &owner) {
        Ok(value) => value,
        Err(error) => {
            let _ = fail(&plan, &mut record, error.clone(), false, Some(reservation));
            let _ = ack(&plan, false, false, Some(&error));
            panel.show(&format!(
                "Update not started: {error}. Review this operation with update-status."
            ));
            panel.offer_recovery();
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
    if !panel.visible() {
        let error = "native updater window is not visible".to_string();
        fail(&plan, &mut record, error.clone(), false, Some(reservation))?;
        let _ = ack(&plan, false, false, Some(&error));
        return Err(error);
    }
    if let Err(error) = ack(&plan, true, true, None) {
        let _ = fail(&plan, &mut record, error.clone(), false, Some(reservation));
        panel.show(&format!(
            "Updater ownership ACK failed: {error}; original UI was not stopped."
        ));
        panel.offer_recovery();
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
            if let Err(error) = transition(
                &plan,
                &mut record,
                OperationPhase::Completed,
                "New instance and UI/control are ready on the original profile",
            ) {
                let _ = fail(&plan, &mut record, error.clone(), true, Some(reservation));
                panel.show(&format!("New UI is ready, but durable completion failed: {error}; reservation retained."));
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
            if let Err(error) = reservation.release() {
                let _ = transition(&plan, &mut record, OperationPhase::Unknown, format!("New UI ready but installation reservation release failed: {error}; inspect/recover offline"));
                panel.show(&format!(
                    "Updated UI ready; reservation release failed: {error}. Recovery required."
                ));
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
            panel.show("Update complete. Your existing profile and preferences were preserved.");
            thread::sleep(Duration::from_secs(3));
            Ok(())
        }
        Err((error, uncertain)) => {
            let _ = fail(
                &plan,
                &mut record,
                error.clone(),
                uncertain,
                Some(reservation),
            );
            panel.show(&format!("Update requires attention: {error}. No automatic retry or rollback. Use update-status / recover for this operation."));
            panel.offer_recovery();
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
    panel.show("Stopping only the prepared application instance…");
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
        panel.show("Installing from the recorded installation source…");
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
    panel.show("Validating the actual installed image and recorded source…");
    let image = validate_new(plan, old_pid, panel).map_err(|e| (e, true))?;
    record.installed = true;
    record.candidate = Some(image.clone());
    step(journal(plan, record), true)?;
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
    panel.show("Starting the updated application with your existing profile…");
    let mut child = launch(plan, reservation, &image, false).map_err(|e| (e, true))?;
    step(wait_new(plan, reservation, &image, &mut child, panel), true)?;
    if reservation.user_stop_requested().map_err(|e| (e, true))? {
        return Err((
            "user stopped the updated application before completion".into(),
            false,
        ));
    }
    record.applied = true;
    step(journal(plan, record), true)?;
    Ok(())
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
    let mut record = protocol::read_operation(&plan.context.state_dir, &plan.operation_id)?
        .ok_or("operation journal missing")?;
    if record.version != protocol::UPDATER_PROTOCOL {
        return Err("unsupported operation journal protocol".into());
    }
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
    let profile_marker = updates.join("profile.reservation.json");
    let global_marker = protocol::global_updates()?.join(format!(
        "{}.reservation.json",
        protocol::installation_key(&plan.origin)?
    ));
    let exists_or_unknown = |path: &Path| {
        !fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    };
    let reserved = exists_or_unknown(&profile_marker) || exists_or_unknown(&global_marker);
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
                    .is_some_and(|image| {
                        status(plan)
                            .ok()
                            .is_some_and(|reply| new_ready(plan, &image, &reply).unwrap_or(false))
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
                match candidate(plan) {
                    Ok(image) => {
                        record.installed = true;
                        record.candidate = Some(image.clone());
                        if let Ok(current) = status(plan) {
                            if new_ready(plan, &image, &current)? {
                                record.applied = true;
                                record.detail = "Verified new instance and UI/control ready; explicit recovery must reconcile retained reservation".into();
                            } else {
                                record.detail = "Another or unready daemon occupies this profile; do not stop it during recovery".into();
                            }
                        } else if stopped(plan, owner.original_pid)? {
                            record.detail = "Installed candidate verified and manager quiescent; explicit recover --start required unless user stopped".into();
                        } else {
                            record.detail = "Installed candidate verified but old socket/lifecycle has not finalized".into();
                        }
                    }
                    Err(error) => {
                        if coordinator::restored_original(plan).is_ok() {
                            record.installed = false;
                            record.applied = false;
                            record.candidate = None;
                            record.detail = "Manager finished but CURRENT signed original image/source is unchanged; explicit recover --start restores same version, NOT an upgrade".into();
                        } else {
                            record.detail = format!("Installed candidate cannot be verified: {error}; reservation retained, no automatic retry");
                        }
                    }
                }
            }
        }
    }
    if manager_failed_quiet {
        if let Some(owner) = &owner {
            if stopped(plan, owner.original_pid)? && coordinator::restored_original(plan).is_ok() {
                record.installed = false;
                record.applied = false;
                record.candidate = None;
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
    let lease = protocol::lock_operation(&plan.context.state_dir, &plan.operation_id)?;
    recover_locked(plan, start, &lease)
}
fn recover_locked(
    plan: UpdatePlan,
    start: bool,
    _lease: &OperationLease,
) -> Result<OperationRecord, String> {
    let mut start = start;
    loop {
        verify_staged_helper(&plan)?;
        let mut record = inspect(&plan)?;
        let scoped =
            protocol::private_updates(&plan.context.state_dir)?.join("profile.reservation.json");
        let key = protocol::installation_key(&plan.origin)?;
        let global = protocol::global_updates()?.join(format!("{key}.reservation.json"));
        let missing = |path: &Path| {
            fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        };
        if missing(&scoped) && missing(&global) {
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
            &plan.context.state_dir,
            &plan.operation_id,
        )?;
        let prepared_generation = reservation.stop_generation;
        let panel = crate::helper_ui::Panel::new(&plan.context.locale)?;
        panel.show("Reconciling manager, installed image and this profile; no installation will be repeated…");
        let result = (|| {
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
            if no_spawn && !record.installed && !record.applied {
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
                        if !protocol::conflicting_active_profiles(
                            &plan.origin,
                            &plan.context.state_dir,
                        )?
                        .is_empty()
                        {
                            return Err("another profile uses this installation".into());
                        }
                        if reservation.user_stop_requested()? {
                            return Err(
                                "user stop superseded original preflight reconciliation".into()
                            );
                        }
                        record.installed = false;
                        record.applied = false;
                        record.candidate = None;
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
                if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?
                    .is_empty()
                {
                    return Err("another live profile shares this installation".into());
                }
                coordinator::restored_original(&plan)?;
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
                        status(&plan).ok().is_some_and(|reply| {
                            new_ready(&plan, &original, &reply).unwrap_or(false)
                        })
                    });
            let image = if manager_failed || restored_running {
                None
            } else {
                candidate(&plan).ok()
            };
            let Some(image) = image else {
                let original = coordinator::restored_original(&plan)?;
                let detail = if manager_failed {
                    "Installer reported failure; the CURRENT signed original image/source is verified, but other installer effects remain unknown"
                } else {
                    "Installer exited successfully but the CURRENT signed original image is unchanged"
                };
                if let Ok(current) = status(&plan) {
                    if new_ready(&plan, &original, &current)? {
                        reject_other_install_processes(&plan, Some(current.pid))?;
                        if !protocol::conflicting_active_profiles(
                            &plan.origin,
                            &plan.context.state_dir,
                        )?
                        .is_empty()
                        {
                            return Err(
                                "another profile uses installation during original restoration"
                                    .into(),
                            );
                        }
                        if reservation.user_stop_requested()? {
                            return Err("user stop arrived while restored instance was active; wait for its shutdown".into());
                        }
                        record.installed = false;
                        record.applied = false;
                        record.candidate = None;
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
                if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?
                    .is_empty()
                {
                    return Err("another live profile shares the installation".into());
                }
                record.installed = false;
                record.applied = false;
                record.candidate = None;
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
                let mut child = launch(&plan, &reservation, &original, true)?;
                wait_new(&plan, &reservation, &original, &mut child, &panel)?;
                if reservation.user_stop_requested()? {
                    return Err("user stopped original image during recovery".into());
                }
                transition(&plan, &mut record, OperationPhase::Failed, format!("{detail}; current original version restarted on captured profile, NOT an upgrade or rollback guarantee"))?;
                reservation.release()?;
                return Ok(());
            };
            record.candidate = Some(image.clone());
            record.installed = true;
            journal(&plan, &record)?;
            if let Ok(current) = status(&plan) {
                if new_ready(&plan, &image, &current)? {
                    reject_other_install_processes(&plan, Some(current.pid))?;
                    if !protocol::conflicting_active_profiles(
                        &plan.origin,
                        &plan.context.state_dir,
                    )?
                    .is_empty()
                    {
                        return Err(
                            "another profile uses installation during reconciliation".into()
                        );
                    }
                    if reservation.user_stop_requested()? {
                        return Err("user stop arrived while updated instance was active; wait for its shutdown".into());
                    }
                    record.applied = true;
                    transition(
                        &plan,
                        &mut record,
                        OperationPhase::Completed,
                        "Reconciled verified new instance and UI/control readiness",
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
            if !protocol::conflicting_active_profiles(&plan.origin, &plan.context.state_dir)?
                .is_empty()
            {
                return Err("another profile uses this installation".into());
            }
            if reservation.user_stop_requested()? {
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
            let mut child = launch(&plan, &reservation, &image, true)?;
            wait_new(&plan, &reservation, &image, &mut child, &panel)?;
            if reservation.user_stop_requested()? {
                return Err("user stop superseded recovered instance before completion".into());
            }
            record.applied = true;
            transition(
                &plan,
                &mut record,
                OperationPhase::Completed,
                "Verified installed image and new ready profile",
            )?;
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
            transition(&plan, &mut record, phase, format!("Recovery incomplete: {error}. Reservation retained; no automatic retry/rollback"))?;
            if read_lifecycle(&plan)
                .ok()
                .is_some_and(|owner| stopped(&plan, owner.original_pid).unwrap_or(false))
            {
                panel.activate();
            }
            panel.show(&record.detail);
            panel.offer_recovery();
            if retain(&panel) {
                panel.close();
                start = true;
                continue;
            }
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
            action: UpdateAction::LocalRebuild,
            running,
            candidate: None,
        }
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
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            (&mut connection)
                .take(FRAME_LIMIT)
                .read_to_end_until_newline(&mut request)
                .unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            // Native LegacyRequest currently admits version 1, unlike the
            // authenticated updater control frames (version 2).
            assert_eq!(request["version"], 1);
            assert_eq!(request["command"], "status");
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            connection.write_all(&bytes).unwrap();
        });
        let reply = status(&plan).unwrap();
        assert!(reply.ok && reply.running && reply.ready && reply.control_ready);
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
