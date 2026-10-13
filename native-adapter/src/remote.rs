use crate::endpoint::Endpoint;
use crate::protocol;
use crate::settings::{MachineInfo, MachineStatus, SourceCatalog};
use crate::{Shared, Route};
use desktop_pet_contracts::*;
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
const INTERVAL: Duration = Duration::from_secs(5);
const DEADLINE: Duration = Duration::from_secs(8);
const TICK: Duration = Duration::from_millis(50);
const CATALOG_BYTES: usize = 256 * 1024;
const STDERR_BYTES: usize = 1024;
const STDERR_DISCARD_PER_PASS: usize = 64 * 1024;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
struct Job { stop: Arc<AtomicBool>, handle: JoinHandle<()> }
fn remote_source(id: &str) -> String { format!("machine:{id}") }
pub(crate) fn run_coordinator(shared: Arc<Shared>) {
    let mut jobs = HashMap::<String, Job>::new();
    let mut retired = Vec::<JoinHandle<()>>::new();
    let mut catalog_job: Option<Job> = None;
    let mut next_catalog = Instant::now();
    let mut epoch = None;
    let mut cli = None;
    let mut selected = HashSet::<String>::new();
    let mut scratch = Vec::<String>::new();
    while !shared.stop.load(Ordering::Acquire) {
        let current = shared.epoch();
        if epoch != current {
            epoch = current;
            selected.clear();
            for (_, job) in jobs.drain() { job.stop.store(true, Ordering::Release); retired.push(job.handle); }
            if let Some(job) = catalog_job.take() { job.stop.store(true, Ordering::Release); retired.push(job.handle); }
            next_catalog = Instant::now();
            cli = current.and_then(|_| resolve_cli());
        }
        if catalog_job.as_ref().is_some_and(|job| job.handle.is_finished()) {
            if let Some(job) = catalog_job.take() { if job.handle.join().is_err() { shared.fail("Herdr discovery worker panicked".to_owned()); } }
        }
        if let Some(current) = current {
            if Instant::now() >= next_catalog && catalog_job.is_none() {
                let worker_shared = Arc::clone(&shared);
                let stop = Arc::new(AtomicBool::new(false));
                let worker_stop = Arc::clone(&stop);
                let worker_cli = cli.clone();
                match thread::Builder::new().name("desktop-pet-herdr-discovery".to_owned()).spawn(move || discover(&worker_shared, &worker_stop, worker_cli.as_ref(), current)) {
                    Ok(handle) => catalog_job = Some(Job { stop, handle }),
                    Err(error) => shared.fail(format!("cannot start Herdr discovery: {error}")),
                }
                next_catalog = Instant::now() + INTERVAL;
            }
            if let Ok(registry) = shared.registry.lock() {
                let mut desired = registry.catalog.machines.iter().filter(|machine| machine.enabled && registry.preferences.remote && registry.preferences.machines.contains(&machine.id));
                if desired.clone().count() != selected.len() || desired.any(|machine| !selected.contains(&machine.id)) {
                    selected.clear();
                    selected.extend(registry.catalog.machines.iter().filter(|machine| machine.enabled && registry.preferences.remote && registry.preferences.machines.contains(&machine.id)).map(|machine| machine.id.clone()));
                }
            } else { selected.clear(); }
            scratch.extend(jobs.keys().filter(|id| !selected.contains(*id)).cloned());
            for id in scratch.drain(..) {
                if let Some(job) = jobs.remove(&id) { job.stop.store(true, Ordering::Release); retired.push(job.handle); }
                set_status(&shared, &id, current, MachineStatus::NotSelected, None);
            }
            scratch.extend(jobs.iter().filter(|(_, job)| job.handle.is_finished()).map(|(id, _)| id.clone()));
            for id in scratch.drain(..) { if let Some(job) = jobs.remove(&id) { if job.handle.join().is_err() { shared.fail("Herdr machine worker panicked".to_owned()); } } }
            for id in &selected {
                if jobs.contains_key(id) { continue; }
                let worker_shared = Arc::clone(&shared);
                let stop = Arc::new(AtomicBool::new(false));
                let worker_stop = Arc::clone(&stop);
                let worker_cli = cli.clone();
                let worker_id = id.clone();
                match thread::Builder::new().name("desktop-pet-herdr-machine".to_owned()).spawn(move || poll_machine(worker_shared, worker_stop, worker_cli, worker_id, current)) {
                    Ok(handle) => { jobs.insert(id.clone(), Job { stop, handle }); }
                    Err(error) => shared.fail(format!("cannot start Herdr machine: {error}")),
                }
            }
        }
        let mut index = 0;
        while index < retired.len() { if retired[index].is_finished() { let _ = retired.swap_remove(index).join(); } else { index += 1; } }
        thread::sleep(TICK);
    }
    for job in jobs.values() { job.stop.store(true, Ordering::Release); }
    if let Some(job) = catalog_job.as_ref() { job.stop.store(true, Ordering::Release); }
    for (_, job) in jobs { let _ = job.handle.join(); }
    if let Some(job) = catalog_job { let _ = job.handle.join(); }
    for worker in retired { let _ = worker.join(); }
}
fn discover(shared: &Arc<Shared>, stop: &AtomicBool, cli: Option<&PathBuf>, epoch: u64) {
    let mut result = Some(cli.ok_or_else(|| "Herdr CLI not found; install Herdr or add its binary to PATH".to_owned())
        .and_then(|cli| run_cli_until(cli, &["machine", "list", "--json"], CATALOG_BYTES, || stop.load(Ordering::Acquire) || !shared.current(epoch)))
        .and_then(|bytes| parse_catalog(&bytes)));
    let _ = shared.with_epoch(epoch, &mut |domain, registry| {
        match result.take().expect("single domain callback") {
            Ok(mut machines) => {
                for machine in &mut machines {
                    if machine.enabled { if let Some(previous) = registry.catalog.machines.iter().find(|previous| previous.id == machine.id && previous.enabled) { machine.status = previous.status; machine.error = previous.error.clone(); } }
                }
                registry.catalog = SourceCatalog { initialized: true, machines, error: None };
                let crate::Registry { bindings, operations, catalog, .. } = registry;
                bindings.retain(|source, binding| {
                    if let Route::Remote(id) = &binding.route {
                        if let Some(machine) = catalog.machines.iter().find(|machine| machine.id == *id && machine.enabled) {
                            domain.set_source_label(source, binding.generation, &machine.label);
                        } else {
                            domain.remove_source(source, binding.generation);
                            operations.retain(|_, operation| { if operation.source == *source { operation.cancel(); false } else { true } });
                            return false;
                        }
                    }
                    true
                });
            }
            Err(error) => { registry.catalog.initialized = true; registry.catalog.error = Some(error); }
        }
    });
    shared.wake();
}
fn parse_catalog(bytes: &[u8]) -> Result<Vec<MachineInfo>, String> {
    let rows: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Herdr machine list returned invalid JSON".to_owned())?;
    let rows = rows
        .as_array()
        .ok_or("Herdr machine list must return an array")?;
    if rows.len() > 64 {
        return Err("Herdr machine list exceeds 64 profiles".to_owned());
    }
    let mut ids = HashSet::new();
    rows.iter()
        .map(|row| {
            let object = row
                .as_object()
                .ok_or("Herdr machine list has invalid profile")?;
            let string = |key| {
                object
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .ok_or_else(|| format!("Herdr machine profile has invalid {key}"))
            };
            let id = string("id")?.to_owned();
            if id.contains('\0') || !ids.insert(id.clone()) {
                return Err("Herdr machine list has invalid or duplicate IDs".to_owned());
            }
            let label = string("label")?.to_owned();
            let session = string("session")?.to_owned();
            // `target` and `selected` are part of the v0.9.2 CLI contract, but
            // neither SSH targets nor Herdr's active selection belong in app state.
            let _ = string("target")?;
            let _ = object
                .get("selected")
                .and_then(serde_json::Value::as_bool)
                .ok_or("Herdr machine profile has invalid selected")?;
            let enabled = object
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .ok_or("Herdr machine profile has invalid enabled")?;
            Ok(MachineInfo {
                id,
                label,
                remote_session: session,
                enabled,
                status: MachineStatus::NotSelected,
                error: None,
            })
        })
        .collect()
}
fn observation_current(shared: &Arc<Shared>, epoch: u64, id: &str) -> bool {
    shared.current(epoch) && shared.registry.lock().is_ok_and(|registry| registry.preferences.remote && registry.preferences.machines.iter().any(|selected| selected == id) && registry.catalog.machines.iter().any(|machine| machine.id == id && machine.enabled))
}
fn poll_machine(shared: Arc<Shared>, stop: Arc<AtomicBool>, cli: Option<PathBuf>, id: String, epoch: u64) {
    let source = remote_source(&id);
    let mut endpoint: Option<Endpoint> = None;
    let mut baseline = true;
    while !stop.load(Ordering::Acquire) && observation_current(&shared, epoch, &id) {
        if baseline { set_status(&shared, &id, epoch, MachineStatus::Connecting, None); }
        let observed_at = Instant::now();
        let result = cli.as_ref().ok_or_else(|| "Herdr CLI not found; install Herdr or add its binary to PATH".to_owned())
            .and_then(|cli| run_cli_until(cli, &["--machine", &id, "api", "snapshot"], protocol::MAX_FRAME_BYTES, || stop.load(Ordering::Acquire) || !observation_current(&shared, epoch, &id)))
            .and_then(|bytes| {
                let line = std::str::from_utf8(&bytes).map_err(|_| "Herdr snapshot was not UTF-8".to_owned())?;
                let response = protocol::parse_response_line(line, "cli:api:snapshot").map_err(|error| error.to_string())?;
                protocol::parse_snapshot(response).map_err(|error| error.to_string())
            });
        if stop.load(Ordering::Acquire) || !observation_current(&shared, epoch, &id) { break; }
        match result {
            Ok(snapshot) => {
                if endpoint.is_none() {
                    let mut generation = 0;
                    let mut admitted = false;
                    let _ = shared.admission_transaction(epoch, &source, NEXT_GENERATION.fetch_add(1, Ordering::Relaxed), &mut |domain, registry, next| {
                        generation = next;
                        admitted = shared.begin(domain, registry, SourceDescriptor { id: source.clone(), provider: "herdr".to_owned(), label: registry.catalog.machines.iter().find(|machine| machine.id == id).map(|machine| machine.label.clone()).unwrap_or_else(|| id.clone()), locality: SourceLocality::Remote, capabilities: SourceCapabilities::default() }, generation, Route::Remote(id.clone()));
                    });
                    if admitted { endpoint = Some(Endpoint::remote(Arc::clone(&shared), source.clone(), generation, epoch)); }
                    else { set_status(&shared, &id, epoch, MachineStatus::Offline, Some("Remote source limit reached or machine is unavailable".to_owned())); }
                }
                if let Some(endpoint) = &mut endpoint {
                    endpoint.remote_snapshot(&shared, snapshot, observed_at, baseline);
                    baseline = false;
                    let connected = shared.registry.lock().is_ok_and(|registry| registry.bindings.get(&source).is_some_and(|binding| binding.connected));
                    if connected { set_status(&shared, &id, epoch, MachineStatus::Online, None); }
                }
            }
            Err(error) => {
                if let Some(endpoint) = &mut endpoint { endpoint.remote_offline(&shared); }
                endpoint = None; baseline = true;
                set_status(&shared, &id, epoch, MachineStatus::Offline, Some(error));
            }
        }
        let until = Instant::now() + INTERVAL;
        while Instant::now() < until && !stop.load(Ordering::Acquire) && observation_current(&shared, epoch, &id) { thread::sleep(TICK); }
    }
}
fn set_status(shared: &Arc<Shared>, id: &str, epoch: u64, status: MachineStatus, error: Option<String>) {
    let _ = shared.with_epoch(epoch, &mut |_, registry| {
        if status != MachineStatus::NotSelected && (!registry.preferences.remote || !registry.preferences.machines.iter().any(|selected| selected == id)) { return; }
        if let Some(machine) = registry.catalog.machines.iter_mut().find(|machine| machine.id == id && machine.enabled) {
            machine.status = status;
            machine.error = error.as_ref().map(|error| error.chars().filter(|c| !c.is_control()).take(200).collect());
        }
    });
    shared.wake();
}
fn resolve_cli() -> Option<PathBuf> {
    if let Some(path) = env::var_os("HERDR_BIN_PATH").filter(|path| !path.is_empty()).map(PathBuf::from) {
        return path.is_absolute().then_some(path);
    }
    let path = env::var_os("PATH").unwrap_or_default();
    let mut locations: Vec<PathBuf> = env::split_paths(&path)
        .filter(|path| path.is_absolute())
        .collect();
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        locations.push(home.join(".local/bin"));
        locations.push(home.join(".cargo/bin"));
    }
    locations.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    locations
        .into_iter()
        .map(|dir| dir.join("herdr"))
        .find(|path| {
            std::fs::metadata(path).is_ok_and(|meta| {
                meta.is_file()
                    && (std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0)
            })
        })
}
#[cfg(test)]
fn run_cli(
    cli: &PathBuf,
    args: &[&str],
    stop: &AtomicBool,
    limit: usize,
) -> Result<Vec<u8>, String> {
    run_cli_until(cli, args, limit, || stop.load(Ordering::Acquire))
}

fn run_cli_until(
    cli: &PathBuf,
    args: &[&str],
    limit: usize,
    mut should_cancel: impl FnMut() -> bool,
) -> Result<Vec<u8>, String> {
    if should_cancel() {
        return Err("Herdr command cancelled".to_owned());
    }
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot launch Herdr CLI: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    set_nonblocking(&stdout).map_err(|e| {
        terminate(&mut child);
        e.to_string()
    })?;
    set_nonblocking(&stderr).map_err(|e| {
        terminate(&mut child);
        e.to_string()
    })?;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let start = Instant::now();
    let mut out_open = true;
    let mut err_open = true;
    loop {
        let cancellation = should_cancel();
        if cancellation || start.elapsed() >= DEADLINE {
            terminate(&mut child);
            return Err(if cancellation {
                "Herdr command cancelled"
            } else {
                "Herdr command timed out"
            }
            .to_owned());
        }
        match drain(
            &mut stdout,
            &mut output,
            limit,
            Overflow::Fail,
            &mut out_open,
        ) {
            Ok(()) => {}
            Err(e) => {
                terminate(&mut child);
                return Err(format!("Herdr stdout: {e}"));
            }
        }
        match drain(
            &mut stderr,
            &mut errors,
            STDERR_BYTES,
            Overflow::Truncate,
            &mut err_open,
        ) {
            Ok(()) => {}
            Err(e) => {
                terminate(&mut child);
                return Err(format!("Herdr stderr: {e}"));
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                terminate(&mut child);
                let detail = String::from_utf8_lossy(&errors);
                let detail = detail
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect::<String>();
                return Err(if detail.is_empty() {
                    format!("Herdr command exited with {status}")
                } else {
                    format!("Herdr command failed: {detail}")
                });
            }
            Ok(Some(_)) if !out_open && !err_open => return Ok(output),
            Ok(_) => {}
            Err(e) => {
                terminate(&mut child);
                return Err(format!("Herdr command wait failed: {e}"));
            }
        }
        thread::sleep(TICK);
    }
}

fn set_nonblocking(file: &impl AsRawFd) -> io::Result<()> {
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// What `drain` does with bytes past `limit`.
#[derive(Clone, Copy)]
enum Overflow {
    /// The payload is unusable once truncated.
    Fail,
    /// Keep the prefix and keep reading (so the child never blocks on a full
    /// pipe), discarding at most `STDERR_DISCARD_PER_PASS` bytes per call.
    Truncate,
}

fn drain(
    file: &mut impl Read,
    output: &mut Vec<u8>,
    limit: usize,
    overflow: Overflow,
    open: &mut bool,
) -> io::Result<()> {
    let mut chunk = [0; 4096];
    let mut discarded = 0;
    while *open {
        match file.read(&mut chunk) {
            Ok(0) => *open = false,
            Ok(n) if output.len() + n > limit => match overflow {
                Overflow::Fail => return Err(io::Error::other("response exceeded size limit")),
                Overflow::Truncate => {
                    let keep = limit.saturating_sub(output.len());
                    output.extend_from_slice(&chunk[..keep]);
                    discarded += n - keep;
                    if discarded >= STDERR_DISCARD_PER_PASS {
                        break;
                    }
                }
            },
            Ok(n) => output.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn terminate(child: &mut Child) {
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_errors_do_not_accept_partial_or_duplicate_profiles() {
        assert!(parse_catalog(br#"[{"id":"a","label":"A","target":"ssh","session":"default","enabled":true,"selected":false},{"id":"a","label":"B","target":"ssh","session":"default","enabled":true,"selected":false}]"#).is_err());
        assert!(parse_catalog(br#"[{"id":"a","label":"A","target":"ssh","session":"default","enabled":"yes","selected":false}]"#).is_err());
        assert!(parse_catalog(br#"{"machines":[]}"#).is_err());
        assert_eq!(parse_catalog(br#"[{"id":"a","label":"Renamed","target":"ssh","session":"work","enabled":false,"selected":true}]"#).unwrap()[0].remote_session, "work");
    }
    #[test]
    fn cli_output_is_bounded_and_cancelled_process_group_is_reaped() {
        let stop = Arc::new(AtomicBool::new(false));
        assert!(run_cli(
            &PathBuf::from("/bin/sh"),
            &["-c", "while :; do printf x; done"],
            &stop,
            1024
        )
        .unwrap_err()
        .contains("size limit"));
        let signal = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            run_cli(
                &PathBuf::from("/bin/sh"),
                &["-c", "sleep 30 & wait"],
                &signal,
                1024,
            )
        });
        thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        stop.store(true, Ordering::Release);
        assert!(handle.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn stderr_overflow_is_truncated_without_failing_or_starving_cancellation() {
        let stop = Arc::new(AtomicBool::new(false));
        let chatty = "head -c 100000 /dev/zero | tr '\\0' e >&2";
        let succeeds = format!("{chatty}; printf ok");
        assert_eq!(
            run_cli(
                &PathBuf::from("/bin/sh"),
                &["-c", succeeds.as_str()],
                &stop,
                1024
            )
            .unwrap(),
            b"ok"
        );
        let fails = format!("{chatty}; exit 3");
        let failure = run_cli(
            &PathBuf::from("/bin/sh"),
            &["-c", fails.as_str()],
            &stop,
            1024,
        )
        .unwrap_err();
        assert!(failure.contains("Herdr command failed:"), "{failure}");
        assert!(!failure.contains("size limit"), "{failure}");

        let signal = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            run_cli(
                &PathBuf::from("/bin/sh"),
                &["-c", "while :; do printf eeeeeeee >&2; done"],
                &signal,
                1024,
            )
        });
        thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        stop.store(true, Ordering::Release);
        assert!(handle.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    fn remote_fixture(label: &str, script: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root = env::temp_dir().join(format!("herdr-{label}-{}-{}",std::process::id(),NEXT_GENERATION.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        let cli=root.join("herdr");
        std::fs::write(&cli,script.replace("@ROOT@",&root.to_string_lossy())).unwrap();
        std::fs::set_permissions(&cli,std::fs::Permissions::from_mode(0o700)).unwrap();
        (root,cli)
    }
    fn select_machine(shared:&Shared) {
        let mut registry=shared.registry.lock().unwrap();
        registry.preferences.remote=true; registry.preferences.machines=vec!["m".into()];
        registry.catalog=SourceCatalog { initialized:true, machines:parse_catalog(br#"[{"id":"m","label":"Machine","target":"ssh","session":"default","enabled":true,"selected":false}]"#).unwrap(),error:None };
    }
    #[test]
    fn disabled_observation_never_launches_cli_and_epoch_change_cancels_descendants() {
        let (root,cli)=remote_fixture("dormant", "#!/bin/sh\necho run >> '@ROOT@/runs'\nsleep 30\n");
        let (integration,runtime)=crate::harness::integration();
        select_machine(&integration.shared);
        let epoch=integration.shared.epoch().unwrap();
        runtime.with_domain(&mut |domain| integration.set_enabled(false,domain)).unwrap();
        discover(&integration.shared,&AtomicBool::new(false),Some(&cli),epoch);
        poll_machine(integration.shared.clone(),Arc::new(AtomicBool::new(false)),Some(cli.clone()),"m".into(),epoch);
        assert!(!root.join("runs").exists());
        runtime.with_domain(&mut |domain| integration.set_enabled(true,domain)).unwrap();
        let epoch=integration.shared.epoch().unwrap();
        let shared=integration.shared.clone();
        let worker=thread::spawn(move || poll_machine(shared,Arc::new(AtomicBool::new(false)),Some(cli),"m".into(),epoch));
        let deadline=Instant::now()+Duration::from_secs(5);
        while !root.join("runs").exists() { assert!(Instant::now()<deadline); thread::sleep(TICK); }
        let started=Instant::now();
        runtime.with_domain(&mut |domain| integration.retire(domain)).unwrap();
        worker.join().unwrap(); assert!(started.elapsed()<Duration::from_secs(2));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejected_remote_source_waits_for_normal_poll_interval_and_never_evicts_live_sources() {
        let (root,cli)=remote_fixture("budget", "#!/bin/sh\necho run >> '@ROOT@/runs'\nprintf '%s\\n' '{\"id\":\"cli:api:snapshot\",\"result\":{\"type\":\"session_snapshot\",\"snapshot\":{\"panes\":[],\"agents\":[]}}}'\n");
        let (integration,runtime)=crate::harness::integration();
        select_machine(&integration.shared);
        {
            let mut domain=runtime.domain.lock().unwrap();
            for index in 0..64 {
                let id=format!("other:{index}");
                domain.begin_source(SourceDescriptor {id:id.clone(),provider:"other".into(),label:id,locality:SourceLocality::Local,capabilities:SourceCapabilities::default()},1);
            }
        }
        let epoch=integration.shared.epoch().unwrap();
        let stop=Arc::new(AtomicBool::new(false)); let cancellation=stop.clone(); let shared=integration.shared.clone();
        let worker=thread::spawn(move || poll_machine(shared,cancellation,Some(cli),"m".into(),epoch));
        let deadline=Instant::now()+Duration::from_secs(5);
        while integration.shared.registry.lock().unwrap().catalog.machines[0].status != MachineStatus::Offline { assert!(Instant::now()<deadline); thread::sleep(TICK); }
        thread::sleep(Duration::from_secs(1)); stop.store(true,Ordering::Release); worker.join().unwrap();
        assert_eq!(std::fs::read_to_string(root.join("runs")).unwrap().lines().count(),1);
        assert_eq!(runtime.domain.lock().unwrap().sources.len(),64);
        assert!(integration.shared.registry.lock().unwrap().bindings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
