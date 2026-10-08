use crate::{
    ensure_absolute, executable_identity, now, owned_file, validate_candidate, CheckSnapshot,
    CheckStatus, ExecutableIdentity, InstallOrigin, UpdateAction, UpdateContext, UpdatePlan,
};
use semver::Version;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const APP: &str = "dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet";
const MAX_METADATA: u64 = 1024 * 1024;

fn same(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a)
        .ok()
        .zip(fs::canonicalize(b).ok())
        .is_some_and(|(a, b)| a == b)
}
fn normalized(path: &Path) -> Result<PathBuf, String> {
    ensure_absolute(path)?;
    fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))
}
fn source_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn parse_source(source: &str) -> Result<(), String> {
    let parts: Vec<_> = source.split('/').collect();
    if source.starts_with('-')
        || !(2..=8).contains(&parts.len())
        || !parts.iter().all(|s| source_component(s))
    {
        return Err("invalid recorded GitHub owner/repo/subdirectory".into());
    }
    Ok(())
}
fn safe_ref(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 200
        && !s.starts_with('-')
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("..")
        && !s.contains("@{")
        && !s.contains("//")
        && !s.contains('\\')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'.'))
}
fn commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn version(s: &str) -> Result<Version, String> {
    Version::parse(s.trim_start_matches('v')).map_err(|e| format!("invalid version {s}: {e}"))
}

fn context_valid(c: &UpdateContext) -> Result<(), String> {
    for p in [
        &c.executable,
        &c.running.path,
        &c.config_dir,
        &c.state_dir,
        &c.herdr_socket,
    ] {
        ensure_absolute(p)?;
    }
    if let Some(host_config) = &c.host_plugin_config_dir {
        ensure_absolute(host_config)?;
    }
    if let Some(p) = &c.assets_override {
        ensure_absolute(p)?;
    }
    if c.instance_id.is_empty()
        || c.instance_id.len() > 128
        || c.instance_id
            .chars()
            .any(|ch| matches!(ch, '\n' | '\r' | '/'))
    {
        return Err("invalid instance id".into());
    }
    if !c.running.sha256.bytes().all(|b| b.is_ascii_hexdigit()) || c.running.sha256.len() != 64 {
        return Err("invalid immutable running identity".into());
    }
    version(&c.version)?;
    for (key, value) in &c.environment {
        if !matches!(
            key.as_str(),
            "HERDR_BIN_PATH"
                | "HERDR_PLUGIN_ID"
                | "HERDR_ENV"
                | "HERDR_PLUGIN_CONFIG_DIR"
                | "HERDR_PLUGIN_STATE_DIR"
                | "HERDR_SOCKET_PATH"
                | "XDG_CONFIG_HOME"
                | "XDG_STATE_HOME"
                | "HOME"
        ) || value.len() > 4096
            || value.contains('\0')
        {
            return Err(format!(
                "unapproved host routing environment variable {key}"
            ));
        }
    }
    if c.running_origin.is_none() && executable_identity(&c.executable)? != c.running {
        return Err("initial running executable does not match captured image".into());
    }
    Ok(())
}

fn manifest(root: &Path) -> Result<toml::Value, String> {
    let data = owned_file(&root.join("herdr-plugin.toml"), MAX_METADATA)?;
    let data = std::str::from_utf8(&data).map_err(|e| e.to_string())?;
    let doc: toml::Value = data
        .parse()
        .map_err(|e| format!("invalid plugin manifest: {e}"))?;
    if doc.get("id").and_then(toml::Value::as_str) != Some("desktop-pet") {
        return Err("wrong plugin id".into());
    }
    if !doc
        .get("platforms")
        .and_then(toml::Value::as_array)
        .is_some_and(|p| p.iter().any(|x| x.as_str() == Some("macos")))
    {
        return Err("manifest does not support macOS".into());
    }
    Ok(doc)
}
fn manifest_version(doc: &toml::Value) -> Result<String, String> {
    let v = doc
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or("missing manifest version")?;
    version(v)?;
    Ok(v.to_string())
}
fn app_manifest(root: &Path) -> Result<String, String> {
    manifest_version(&manifest(root)?)
}
fn check_host(c: &UpdateContext, host: &Path, doc: &toml::Value) -> Result<(), String> {
    if !c
        .environment
        .get("HERDR_BIN_PATH")
        .is_some_and(|p| same(Path::new(p), host))
    {
        return Err("Herdr host binary was not injected for this running instance".into());
    }
    if c.environment
        .get("HERDR_PLUGIN_ID")
        .is_some_and(|id| id != "desktop-pet")
        || !c
            .environment
            .get("HERDR_PLUGIN_CONFIG_DIR")
            .is_some_and(|p| {
                same(Path::new(p), &c.config_dir)
                    || c.host_plugin_config_dir
                        .as_deref()
                        .is_some_and(|original| same(Path::new(p), original))
            })
        || !c
            .environment
            .get("HERDR_PLUGIN_STATE_DIR")
            .is_some_and(|p| same(Path::new(p), &c.state_dir))
    {
        return Err("Herdr profile injection differs from running profile".into());
    }
    if c.environment
        .get("HERDR_SOCKET_PATH")
        .is_some_and(|p| !same(Path::new(p), &c.herdr_socket))
    {
        return Err("injected Herdr socket differs from active connection".into());
    }
    // Socket peer identity and version come from the same live connection. A newer CLI
    // on disk does NOT prove an old, still-running server supports this manifest.
    let (server, host_version) = live_host(&c.herdr_socket)?;
    if !same(&server, host) {
        return Err("connected Herdr server executable differs from injected host".into());
    }
    let required = doc
        .get("min_herdr_version")
        .and_then(toml::Value::as_str)
        .ok_or("missing min_herdr_version")?;
    if host_version < version(required)? || host_version < Version::new(0, 9, 3) {
        return Err("connected Herdr server version is too old for updater install".into());
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn live_host(socket: &Path) -> Result<(PathBuf, Version), String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    #[link(name = "proc")]
    extern "C" {
        fn proc_pidpath(
            pid: libc::c_int,
            buffer: *mut libc::c_void,
            buffersize: u32,
        ) -> libc::c_int;
    }
    let mut stream =
        UnixStream::connect(socket).map_err(|e| format!("cannot connect to Herdr server: {e}"))?;
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || uid != unsafe { libc::geteuid() }
    {
        return Err("Herdr server peer UID differs from updater user".into());
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || pid <= 0 || len as usize != std::mem::size_of_val(&pid) {
        return Err("Herdr peer PID unavailable".into());
    }
    let mut buf = [0u8; 4096];
    let size = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if size <= 0 {
        return Err("Herdr peer binary unavailable".into());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(size as usize);
    use std::os::unix::ffi::OsStrExt;
    let peer = normalized(Path::new(std::ffi::OsStr::from_bytes(&buf[..end])))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(
            b"{\"id\":\"updater-host-proof\",\"method\":\"session.snapshot\",\"params\":{}}\n",
        )
        .map_err(|_| "Herdr live version query failed".to_owned())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut reply = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if reply.len() >= 2 * 1024 * 1024 {
            return Err("Herdr snapshot exceeds size limit".into());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Herdr live version query timed out")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let n = stream
            .read(&mut chunk)
            .map_err(|_| "Herdr live version query failed".to_owned())?;
        if n == 0 {
            return Err("Herdr closed live version query".into());
        }
        reply.extend_from_slice(&chunk[..n]);
        if let Some(end) = reply.iter().position(|b| *b == b'\n') {
            let response: Value = serde_json::from_slice(&reply[..end])
                .map_err(|_| "invalid Herdr live version response")?;
            if response.get("id").and_then(Value::as_str) != Some("updater-host-proof") {
                return Err("unrelated Herdr response".into());
            }
            let snap = response
                .pointer("/result/snapshot")
                .ok_or("missing live Herdr snapshot")?;
            if snap
                .get("protocol")
                .and_then(Value::as_u64)
                .is_none_or(|p| p == 0)
            {
                return Err("live Herdr protocol missing".into());
            }
            let live = snap
                .get("version")
                .and_then(Value::as_str)
                .ok_or("live Herdr version missing")?;
            return Ok((peer, version(live)?));
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn live_host(_: &Path) -> Result<(PathBuf, Version), String> {
    Err("Herdr peer identity proof requires macOS".into())
}

/// The registry belongs to the injected host, not to a session socket or override profile.
fn host_config_root(c: &UpdateContext) -> Result<Option<PathBuf>, String> {
    let Some(injected) = &c.host_plugin_config_dir else {
        return Ok(None);
    };
    let original = normalized(injected)?;
    if original
        .file_name()
        .is_none_or(|name| name != "desktop-pet")
        || original.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("config"))
        || original
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            != Some(std::ffi::OsStr::new("plugins"))
    {
        return Err("injected plugin config does not identify a host registry root".into());
    }
    let host = original
        .ancestors()
        .nth(3)
        .ok_or("missing injected host config root")?;
    if !matches!(
        host.file_name().and_then(|name| name.to_str()),
        Some("herdr" | "herdr-dev")
    ) {
        return Err("unrecognized host config root".into());
    }
    let base = if let Some(xdg) = c.environment.get("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(c.environment.get("HOME").ok_or("missing captured HOME")?).join(".config")
    };
    if !same(
        host,
        &base.join(host.file_name().ok_or("missing host root name")?),
    ) {
        return Err("injected host registry root differs from manager routing".into());
    }
    Ok(Some(host.to_path_buf()))
}

fn inside_managed_checkout(c: &UpdateContext, image: &Path) -> Result<bool, String> {
    let within = |root: &Path| image.starts_with(root) || c.executable.starts_with(root);
    if let Some(host) = host_config_root(c)? {
        return Ok(within(&host.join("plugins/github")));
    }
    // Missing provenance still excludes BOTH host flavors from Local fallback.
    let base = c
        .environment
        .get("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            c.environment
                .get("HOME")
                .map(|home| Path::new(home).join(".config"))
        });
    Ok(base.is_some_and(|base| {
        ["herdr", "herdr-dev"]
            .iter()
            .any(|name| within(&base.join(name).join("plugins/github")))
    }))
}
fn registered(c: &UpdateContext) -> Result<Option<InstallOrigin>, String> {
    registered_with_host(c, true)
}
fn registered_with_host(
    c: &UpdateContext,
    require_live_host: bool,
) -> Result<Option<InstallOrigin>, String> {
    let host = match c.environment.get("HERDR_BIN_PATH") {
        Some(p) => normalized(Path::new(p))?,
        None => return Ok(None),
    };
    let Some(host_config_dir) = host_config_root(c)? else {
        return Ok(None);
    };
    let registry = host_config_dir.join("plugins.json");
    if !registry.exists() {
        return Ok(None);
    }
    let records: Vec<Value> = serde_json::from_slice(&owned_file(&registry, MAX_METADATA)?)
        .map_err(|e| format!("Herdr registry: {e}"))?;
    let mut entries = records
        .iter()
        .filter(|v| v.get("plugin_id").and_then(Value::as_str) == Some("desktop-pet"));
    let Some(record) = entries.next() else {
        return Ok(None);
    };
    if entries.next().is_some() {
        return Err("ambiguous Herdr desktop-pet registrations".into());
    }
    let plugin_root = normalized(Path::new(
        record
            .get("plugin_root")
            .and_then(Value::as_str)
            .ok_or("missing registered plugin root")?,
    ))?;
    let recorded_manifest = Path::new(
        record
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or("missing registered manifest path")?,
    );
    if !same(recorded_manifest, &plugin_root.join("herdr-plugin.toml")) {
        return Err("registered manifest does not belong to registered root".into());
    }
    let image = plugin_root.join(APP);
    if !same(&image, &c.running.path) {
        return Ok(None);
    }
    let doc = manifest(&plugin_root)?;
    let source = record
        .get("source")
        .ok_or("missing recorded Herdr source")?;
    let enabled = record
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or("missing enabled flag")?;
    if source.get("kind").and_then(Value::as_str) == Some("local") {
        owned_file(&plugin_root.join("scripts/install.sh"), MAX_METADATA)?;
        return Ok(Some(InstallOrigin::Local { root: plugin_root }));
    }
    if source.get("kind").and_then(Value::as_str) != Some("github") {
        return Err("unsupported Herdr source kind".into());
    }
    let owner = source
        .get("owner")
        .and_then(Value::as_str)
        .ok_or("missing GitHub owner")?;
    let repo = source
        .get("repo")
        .and_then(Value::as_str)
        .ok_or("missing GitHub repo")?;
    let subdir = source.get("subdir").and_then(Value::as_str);
    let source_name = if let Some(subdir) = subdir {
        format!("{owner}/{repo}/{subdir}")
    } else {
        format!("{owner}/{repo}")
    };
    parse_source(&source_name)?;
    let managed = source
        .get("managed_path")
        .and_then(Value::as_str)
        .ok_or("missing managed checkout path")?;
    let checkout_root = normalized(Path::new(managed))?;
    let relative = source_name.split('/').skip(2).collect::<Vec<_>>().join("/");
    let expected_plugin = if relative.is_empty() {
        checkout_root.clone()
    } else {
        normalized(&checkout_root.join(relative))?
    };
    let managed_base = normalized(&host_config_dir.join("plugins/github"))?;
    if checkout_root == managed_base
        || !checkout_root.starts_with(&managed_base)
        || expected_plugin != plugin_root
        || !plugin_root.starts_with(&checkout_root)
    {
        return Err(
            "registered managed checkout or plugin subdirectory differs from source".into(),
        );
    }
    let requested_ref = source
        .get("requested_ref")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if requested_ref.as_deref().is_some_and(|r| !safe_ref(r)) {
        return Err("unsafe recorded GitHub ref".into());
    }
    let resolved_commit = source
        .get("resolved_commit")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    if resolved_commit.as_deref().is_some_and(|r| !commit(r)) {
        return Err("invalid installed revision".into());
    }
    if require_live_host {
        check_host(c, &host, &doc)?;
    }
    Ok(Some(InstallOrigin::Herdr {
        host,
        host_config_dir,
        checkout_root,
        plugin_root,
        source: source_name,
        requested_ref,
        resolved_commit,
        enabled,
    }))
}

fn brew_origin(path: &Path) -> Result<Option<InstallOrigin>, String> {
    let segments: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some(index) = segments.iter().position(|s| s == "Cellar") else {
        return Ok(None);
    };
    let formula = segments.get(index + 1).ok_or("incomplete Cellar path")?;
    if !matches!(
        formula.as_str(),
        "herdr-desktop-pet" | "herdr-desktop-pet-beta"
    ) {
        return Ok(None);
    }
    let keg = segments
        .get(index + 2)
        .ok_or("incomplete Cellar keg path")?;
    let root = PathBuf::from("/").join(segments[1..index].join("/"));
    let keg_path = root.join("Cellar").join(formula).join(keg);
    let bundle = if formula.ends_with("-beta") {
        "HerdrDesktopPetBeta.app/Contents/MacOS/herdr-desktop-pet"
    } else {
        "HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"
    };
    if !same(path, &keg_path.join("libexec").join(bundle)) {
        return Err("binary is not the formula app inside its keg".into());
    }
    let receipt: Value = serde_json::from_slice(&owned_file(
        &keg_path.join("INSTALL_RECEIPT.json"),
        MAX_METADATA,
    )?)
    .map_err(|e| e.to_string())?;
    let tap = receipt
        .pointer("/source/tap")
        .and_then(Value::as_str)
        .ok_or("receipt has no tap")?;
    if tap != "hanbong5938/tap"
        || receipt.pointer("/source/spec").and_then(Value::as_str) != Some("stable")
    {
        return Err("unrecognized formula source/channel".into());
    }
    let receipt_formula = keg_path.join(".brew").join(format!("{formula}.rb"));
    owned_file(&receipt_formula, MAX_METADATA)?;
    let brew = root.join("bin/brew");
    executable_identity(&brew)?;
    let locator = root.join("opt").join(formula).join("libexec").join(bundle);
    if !same(&locator, path) {
        return Err("running keg is not the active formula locator".into());
    }
    Ok(Some(InstallOrigin::Homebrew {
        brew,
        cellar_formula_root: root.join("Cellar").join(formula),
        prefix: root,
        formula: format!("{tap}/{formula}"),
        locator,
    }))
}

pub fn detect_origin(c: &UpdateContext) -> Result<InstallOrigin, String> {
    context_valid(c)?;
    let path = &c.running.path;
    if let Some(captured) = &c.running_origin {
        let current = match captured.as_ref() {
            InstallOrigin::Homebrew {
                brew,
                prefix,
                cellar_formula_root,
                formula,
                locator,
            } => {
                if !path.starts_with(cellar_formula_root) {
                    return Ok(InstallOrigin::Unknown {
                        reason: "running image does not belong to captured Homebrew formula".into(),
                    });
                }
                let latest = brew_origin(&fs::canonicalize(locator).map_err(|e| e.to_string())?)?;
                if let Some(InstallOrigin::Homebrew {
                    brew: active_brew,
                    prefix: active_prefix,
                    cellar_formula_root: active_cellar,
                    formula: active_formula,
                    locator: active_locator,
                }) = latest
                {
                    if &active_brew == brew
                        && &active_prefix == prefix
                        && &active_cellar == cellar_formula_root
                        && &active_formula == formula
                        && &active_locator == locator
                    {
                        Some((**captured).clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            InstallOrigin::Herdr {
                checkout_root,
                plugin_root,
                host_config_dir,
                host,
                source,
                requested_ref,
                enabled,
                ..
            } => {
                if !path.starts_with(checkout_root) {
                    return Ok(InstallOrigin::Unknown {
                        reason: "running image is outside original managed checkout".into(),
                    });
                }
                match registered_with_host(c, false) {
                    Ok(Some(InstallOrigin::Herdr {
                        checkout_root: r,
                        plugin_root: p,
                        host_config_dir: d,
                        host: h,
                        source: s,
                        requested_ref: rr,
                        enabled: e,
                        ..
                    })) if r == *checkout_root
                        && p == *plugin_root
                        && d == *host_config_dir
                        && h == *host
                        && s == *source
                        && rr == *requested_ref
                        && e == *enabled =>
                    {
                        Some((**captured).clone())
                    }
                    _ => None,
                }
            }
            InstallOrigin::Local { root } => (path == &root.join(APP)
                && root.join("scripts/install.sh").is_file()
                && manifest(root).is_ok())
            .then(|| (**captured).clone()),
            InstallOrigin::Manual { locator } => {
                (path == locator || c.executable == *locator).then(|| (**captured).clone())
            }
            InstallOrigin::Unknown { .. } => None,
        };
        return Ok(current.unwrap_or(InstallOrigin::Unknown {
            reason: "installation ownership/source changed since daemon startup".into(),
        }));
    }
    if let Some(brew) = brew_origin(path)? {
        return Ok(brew);
    }
    match registered_with_host(c, false) {
        Ok(Some(origin)) => return Ok(origin),
        Err(reason) => return Ok(InstallOrigin::Unknown { reason }),
        Ok(None) => {}
    }
    if inside_managed_checkout(c, path)? || c.host_plugin_config_dir.is_some() {
        return Ok(InstallOrigin::Unknown {
            reason: "unregistered managed checkout cannot be reclassified as local source".into(),
        });
    }
    if let Some(root) = path.ancestors().find(|p| {
        p.join(APP).as_path() == path
            && p.join("scripts/install.sh").is_file()
            && p.join("herdr-plugin.toml").is_file()
    }) {
        if same(&root.join(APP), path)
            && app_manifest(root).is_ok()
            && owned_file(&root.join("scripts/install.sh"), MAX_METADATA).is_ok()
        {
            return Ok(InstallOrigin::Local {
                root: root.to_path_buf(),
            });
        }
    }
    if path
        .ancestors()
        .any(|p| p.extension().is_some_and(|x| x == "app"))
    {
        return Ok(InstallOrigin::Manual {
            locator: c.running.path.clone(),
        });
    }
    Ok(InstallOrigin::Unknown {
        reason: "running executable does not belong to a verified installation".into(),
    })
}

/// Read-only metadata runs in its own process group; exit AND both pipe EOFs
/// must occur before the same deadline. No reader thread can retain an unbounded join.
pub(crate) fn command_output(
    binary: &Path,
    args: &[&str],
    max: usize,
    seconds: u64,
) -> Result<String, String> {
    fn nonblocking(fd: libc::c_int) -> Result<(), String> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    fn stop(mut child: std::process::Child) {
        let _ = unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
        // Never block the metadata deadline on a wedged process's wait().
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    fn drain<R: Read>(
        pipe: &mut Option<R>,
        output: &mut Vec<u8>,
        max: usize,
    ) -> Result<(), String> {
        let Some(reader) = pipe.as_mut() else {
            return Ok(());
        };
        let mut buf = [0; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    *pipe = None;
                    return Ok(());
                }
                Ok(n) => {
                    if n > max.saturating_sub(output.len()) {
                        return Err("read-only metadata output exceeds bound".into());
                    }
                    output.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(format!("metadata pipe read: {e}")),
            }
        }
    }
    let mut child = Command::new(binary)
        .args(args)
        .process_group(0)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    if let Err(error) =
        nonblocking(stdout.as_raw_fd()).and_then(|()| nonblocking(stderr.as_raw_fd()))
    {
        stop(child);
        return Err(error);
    }
    let mut stdout = Some(stdout);
    let mut stderr = Some(stderr);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut status = None;
    loop {
        if status.is_none() {
            status = match child.try_wait() {
                Ok(value) => value,
                Err(e) => {
                    stop(child);
                    return Err(format!("metadata process wait: {e}"));
                }
            };
        }
        let drained =
            drain(&mut stdout, &mut out, max).and_then(|()| drain(&mut stderr, &mut err, max));
        if let Err(error) = drained {
            if status.is_none() {
                stop(child);
            }
            return Err(error);
        }
        if let Some(exit) = status {
            if stdout.is_none() && stderr.is_none() {
                if !exit.success() {
                    return Err(format!(
                        "metadata query failed: {}",
                        String::from_utf8_lossy(&err)
                    ));
                }
                return String::from_utf8(out).map_err(|e| e.to_string());
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if status.is_none() {
                stop(child);
            }
            return Err("read-only metadata command timed out before exit and pipe EOF".into());
        }
        let mut pollfds = [
            libc::pollfd {
                fd: stdout.as_ref().map_or(-1, |f| f.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stderr.as_ref().map_or(-1, |f| f.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let timeout = remaining.as_millis().min(25) as libc::c_int;
        let rc =
            unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout) };
        if rc < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            if status.is_none() {
                stop(child);
            }
            return Err("read-only metadata poll failed".into());
        }
    }
}

fn ls_remote(
    binary: &Path,
    remote: &str,
    requested_ref: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    let reference = requested_ref.unwrap_or("HEAD");
    if !safe_ref(reference) {
        return Err("invalid ref".into());
    }
    if commit(reference) {
        return Ok((Some(reference.to_ascii_lowercase()), true));
    }
    let branches = reference.strip_prefix("refs/heads/");
    let tags = reference.strip_prefix("refs/tags/");
    if branches.is_some_and(str::is_empty)
        || tags.is_some_and(str::is_empty)
        || (reference.starts_with("refs/") && branches.is_none() && tags.is_none())
    {
        return Err("unsupported qualified Git ref".into());
    }
    let name = branches.or(tags).unwrap_or(reference);
    let branch = format!("refs/heads/{name}");
    let tag = format!("refs/tags/{name}");
    let peeled = format!("{tag}^{{}}");
    let patterns: Vec<&str> = if reference == "HEAD" {
        vec!["HEAD"]
    } else if branches.is_some() {
        vec![&branch]
    } else if tags.is_some() {
        vec![&tag, &peeled]
    } else {
        vec![&branch, &tag, &peeled]
    };
    let mut args = vec!["ls-remote", remote];
    args.extend(patterns);
    let output = command_output(binary, &args, 16384, 12)?;
    resolve_ref(&output, requested_ref)
}

fn observed_herdr(
    source: &str,
    requested_ref: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    parse_source(source)?;
    let (owner, repo) = source.split_once('/').ok_or("invalid GitHub source")?;
    let repo = repo.split('/').next().ok_or("invalid repo")?;
    let remote = format!("https://github.com/{owner}/{repo}.git");
    ls_remote(Path::new("/usr/bin/git"), &remote, requested_ref)
}

fn resolve_ref(
    result: &str,
    requested_ref: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    let refs: Vec<(&str, &str)> = result
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(sha, _)| commit(sha))
        .collect();
    let Some(refname) = requested_ref.filter(|r| *r != "HEAD") else {
        return Ok((
            refs.iter()
                .find(|(_, r)| *r == "HEAD")
                .map(|(s, _)| (*s).into()),
            false,
        ));
    };
    if commit(refname) {
        return Ok((Some(refname.to_ascii_lowercase()), true));
    }
    let (name, branch_only, tag_only) = if let Some(name) = refname.strip_prefix("refs/heads/") {
        (name, true, false)
    } else if let Some(name) = refname.strip_prefix("refs/tags/") {
        (name, false, true)
    } else {
        (refname, false, false)
    };
    let head = format!("refs/heads/{name}");
    let tag = format!("refs/tags/{name}");
    let branch = (!tag_only)
        .then(|| refs.iter().find(|(_, r)| *r == head).map(|(s, _)| *s))
        .flatten();
    let tagged = (!branch_only)
        .then(|| refs.iter().find(|(_, r)| *r == tag).map(|(s, _)| *s))
        .flatten();
    if branch.is_some() && tagged.is_some() {
        return Err("ref resolves ambiguously as branch and tag".into());
    }
    let peeled = format!("{tag}^{{}}");
    Ok((
        branch
            .or_else(|| refs.iter().find(|(_, r)| *r == peeled).map(|(s, _)| *s))
            .or(tagged)
            .map(str::to_owned),
        tagged.is_some(),
    ))
}

fn remote_manifest(source: &str, revision: &str) -> Result<(String, toml::Value), String> {
    parse_source(source)?;
    if !commit(revision) {
        return Err("remote revision is not a full commit".into());
    }
    let mut parts = source.split('/');
    let owner = parts.next().ok_or("missing owner")?;
    let repo = parts.next().ok_or("missing repo")?;
    let subdir = parts.collect::<Vec<_>>().join("/");
    let suffix = if subdir.is_empty() {
        String::new()
    } else {
        format!("{subdir}/")
    };
    let url = format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{revision}/{suffix}herdr-plugin.toml"
    );
    let text = command_output(
        Path::new("/usr/bin/curl"),
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "12",
            &url,
        ],
        65536,
        15,
    )?;
    let doc: toml::Value = text
        .parse()
        .map_err(|e| format!("remote manifest invalid: {e}"))?;
    if doc.get("id").and_then(toml::Value::as_str) != Some("desktop-pet")
        || !doc
            .get("platforms")
            .and_then(toml::Value::as_array)
            .is_some_and(|p| p.iter().any(|x| x.as_str() == Some("macos")))
    {
        return Err("remote manifest does not describe the macOS desktop pet".into());
    }
    let build = doc
        .get("build")
        .and_then(toml::Value::as_array)
        .ok_or("remote manifest has no build section")?;
    if !build.iter().any(|b| {
        b.get("command")
            .and_then(toml::Value::as_array)
            .is_some_and(|args| {
                args.len() == 2
                    && args[0].as_str() == Some("bash")
                    && args[1].as_str() == Some("scripts/install.sh")
            })
    }) {
        return Err("remote build command is not reviewed installer path".into());
    }
    let startup = doc
        .get("startup")
        .and_then(toml::Value::as_array)
        .ok_or("remote manifest has no startup")?;
    if !startup.iter().any(|entry| {
        entry
            .get("command")
            .and_then(toml::Value::as_array)
            .is_some_and(|args| {
                args.len() == 2
                    && args[0].as_str()
                        == Some("./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet")
                    && args[1].as_str() == Some("ensure")
            })
    }) {
        return Err("remote startup command is not the expected bundle ensure".into());
    }
    let release = manifest_version(&doc)?;
    Ok((release, doc))
}

fn new_plan(
    c: &UpdateContext,
    origin: &InstallOrigin,
    action: UpdateAction,
    candidate: Option<ExecutableIdentity>,
) -> Result<UpdatePlan, String> {
    let mut random = [0u8; 16];
    let mut f = fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    f.read_exact(&mut random).map_err(|e| e.to_string())?;
    Ok(UpdatePlan {
        version: crate::protocol::UPDATER_PROTOCOL,
        operation_id: random.iter().map(|b| format!("{b:02x}")).collect(),
        context: c.clone(),
        origin: origin.clone(),
        action,
        running: c.running.clone(),
        candidate,
    })
}

fn changed_candidate(
    c: &UpdateContext,
    locator: &Path,
) -> Result<Option<ExecutableIdentity>, String> {
    let image = match executable_identity(locator) {
        Ok(image) => image,
        Err(_) => return Ok(None),
    };
    if image.sha256 == c.running.sha256 {
        return Ok(None);
    }
    let verified = validate_candidate(locator)?;
    let output = command_output(locator, &["--version"], 4096, 5)?;
    let candidate_version = output
        .split_whitespace()
        .find_map(|word| version(word).ok())
        .ok_or("candidate version unavailable")?;
    if candidate_version < version(&c.version)? {
        return Err("candidate would downgrade the running app".into());
    }
    Ok(Some(verified))
}

pub fn check(c: &UpdateContext) -> Result<CheckSnapshot, String> {
    let origin = detect_origin(c)?;
    let at = now()?;
    let mut snapshot = CheckSnapshot {
        status: CheckStatus::Unsupported,
        origin: origin.clone(),
        installed_version: None,
        observed_revision: None,
        detail: String::new(),
        plan: None,
        checked_at: Some(at),
    };
    match &origin {
        InstallOrigin::Herdr {
            host,
            plugin_root,
            source,
            requested_ref,
            resolved_commit,
            enabled,
            ..
        } => {
            let local = app_manifest(plugin_root)?;
            snapshot.installed_version = Some(local);
            let active = registered(c)?;
            let installed_commit = match active {
                Some(InstallOrigin::Herdr {
                    resolved_commit, ..
                }) => resolved_commit,
                _ => None,
            };
            snapshot.observed_revision = installed_commit.clone();
            if !enabled {
                snapshot.detail =
                    "Disabled Herdr registration cannot be reinstalled without enabling it".into();
                return Ok(snapshot);
            }
            if c.running_origin.is_some() && installed_commit != *resolved_commit {
                if let Some(candidate) = changed_candidate(c, &plugin_root.join(APP))? {
                    snapshot.status = CheckStatus::Installed;
                    snapshot.detail = "Same registered source/ref has an externally installed binary; apply explicitly after validating its exact image".into();
                    snapshot.plan = Some(new_plan(
                        c,
                        &origin,
                        UpdateAction::ApplyInstalled,
                        Some(candidate),
                    )?);
                    return Ok(snapshot);
                }
            }
            if c.running_origin.is_some()
                && executable_identity(&plugin_root.join(APP))
                    .is_ok_and(|now| now.sha256 != c.running.sha256)
            {
                snapshot.detail = "Managed binary changed without a new registered revision; no verified replacement provenance".into();
                return Ok(snapshot);
            }
            let (observed, immutable_ref) = observed_herdr(source, requested_ref.as_deref())?;
            snapshot.observed_revision = observed.clone();
            let Some(revision) = observed else {
                snapshot.status = CheckStatus::Failed;
                snapshot.detail = "Recorded GitHub ref is not resolvable".into();
                return Ok(snapshot);
            };
            if immutable_ref {
                if Some(&revision) != installed_commit.as_ref() {
                    snapshot.detail = "Pinned ref moved; automatic reinstall blocked".into();
                    return Ok(snapshot);
                }
            }
            let (remote_version, remote_doc) = remote_manifest(source, &revision)?;
            if let Err(reason) = check_host(c, host, &remote_doc) {
                snapshot.detail = reason;
                return Ok(snapshot);
            }
            if version(&remote_version)? < version(&c.version)? {
                snapshot.detail =
                    "Remote manifest is older than running app; downgrade blocked".into();
                return Ok(snapshot);
            }
            if Some(&revision) == installed_commit.as_ref() {
                snapshot.status = CheckStatus::Current;
                snapshot.detail = "Installed revision matches requested ref policy (not a global latest release check)".into();
            } else {
                snapshot.status = CheckStatus::Available;
                snapshot.detail = format!("Remote manifest {remote_version}, commit {revision}: advisory only. Build may install a pinned release binary rather than that commit; explicit trust of recorded source and ref is required.");
                snapshot.plan = Some(new_plan(c, &origin, UpdateAction::HerdrReinstall, None)?);
            }
        }
        InstallOrigin::Local { root } => {
            snapshot.installed_version = Some(app_manifest(root)?);
            if c.running_origin.is_some() {
                if let Some(candidate) = changed_candidate(c, &root.join(APP))? {
                    snapshot.status = CheckStatus::Installed;
                    snapshot.detail = "Current local checkout already contains a different verified app image; apply only this pinned image without rebuilding or pulling".into();
                    snapshot.plan = Some(new_plan(
                        c,
                        &origin,
                        UpdateAction::ApplyInstalled,
                        Some(candidate),
                    )?);
                    return Ok(snapshot);
                }
            }
            snapshot.status = CheckStatus::Local;
            snapshot.detail = "Local checkout: rebuild only these on-disk sources, without fetching or changing git refs".into();
            snapshot.plan = Some(new_plan(c, &origin, UpdateAction::LocalRebuild, None)?);
        }
        InstallOrigin::Homebrew {
            brew,
            prefix,
            formula,
            locator,
            ..
        } => {
            let short = formula.rsplit('/').next().ok_or("invalid formula")?;
            let output = command_output(
                brew,
                &["info", "--json=v2", "--formula", formula],
                65536,
                15,
            )?;
            let json: Value = serde_json::from_str(&output).map_err(|e| e.to_string())?;
            let details = json
                .pointer("/formulae/0")
                .ok_or("brew has no formula metadata")?;
            if details.get("name").and_then(Value::as_str) != Some(short)
                || details.get("full_name").and_then(Value::as_str) != Some(formula.as_str())
            {
                return Err("brew returned a different formula/tap".into());
            }
            if !prefix
                .join("opt")
                .join(short)
                .join("INSTALL_RECEIPT.json")
                .exists()
            {
                return Err("active formula has no installation receipt".into());
            }
            let installed = details
                .pointer("/installed/0/version")
                .and_then(Value::as_str)
                .ok_or("brew has no installed version")?;
            let latest = details
                .pointer("/versions/stable")
                .and_then(Value::as_str)
                .ok_or("brew has no cached stable version")?;
            snapshot.installed_version = Some(installed.into());
            if let Some(candidate) = changed_candidate(c, locator)? {
                snapshot.status = CheckStatus::Installed;
                snapshot.detail = "Same-formula installed binary differs; explicit apply verifies its bundle and protocol".into();
                snapshot.plan = Some(new_plan(
                    c,
                    &origin,
                    UpdateAction::ApplyInstalled,
                    Some(candidate),
                )?);
                return Ok(snapshot);
            }
            if version(latest)? > version(installed)? {
                snapshot.status = CheckStatus::Available;
                snapshot.detail = format!("Cached Homebrew metadata lists {latest}; no brew update was run. Verify same formula and confirm before upgrade.");
                snapshot.plan = Some(new_plan(c, &origin, UpdateAction::HomebrewUpgrade, None)?);
            } else {
                snapshot.status = CheckStatus::Current;
                snapshot.detail = "No newer version in cached formula metadata (not a guarantee of latest upstream release)".into();
            }
        }
        InstallOrigin::Manual { locator } => {
            if c.running_origin.is_some() {
                if let Some(candidate) = changed_candidate(c, locator)? {
                    snapshot.status = CheckStatus::Installed;
                    snapshot.detail = "Previously captured manual app locator contains a different verified image; explicit apply only, no automatic download".into();
                    snapshot.plan = Some(new_plan(
                        c,
                        &origin,
                        UpdateAction::ApplyInstalled,
                        Some(candidate),
                    )?);
                    return Ok(snapshot);
                }
            }
            snapshot.detail = format!("Manual installation at {} has no maintained release feed or verified installation manager", locator.display());
        }
        InstallOrigin::Unknown { reason } => snapshot.detail = reason.clone(),
    }
    Ok(snapshot)
}

#[derive(Clone, Debug)]
pub struct ManagerCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub environment: Vec<(String, String)>,
}

fn resolved_parent_path(path: &Path) -> Result<PathBuf, String> {
    ensure_absolute(path)?;
    if let Ok(real) = fs::canonicalize(path) {
        return Ok(real);
    }
    let parent = path.parent().ok_or("path has no parent")?;
    let name = path.file_name().ok_or("path has no basename")?;
    Ok(fs::canonicalize(parent)
        .map_err(|e| e.to_string())?
        .join(name))
}

pub fn recheck_plan(plan: &UpdatePlan) -> Result<(), String> {
    crate::protocol::validate_operation_id(&plan.operation_id)?;
    if plan.version != crate::protocol::UPDATER_PROTOCOL {
        return Err("unsupported update plan protocol".into());
    }
    context_valid(&plan.context)?;
    if plan.running != plan.context.running {
        return Err("running image differs from immutable context".into());
    }
    let current = detect_origin(&plan.context)?;
    if current != plan.origin {
        return Err("installation source/manager/profile changed since check".into());
    }
    // Discovery can retain a managed origin while Herdr is offline. Physical
    // reinstallation must independently attest the ACTUAL original live host
    // and the current registration immediately before installer admission.
    if let (
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
        UpdateAction::HerdrReinstall,
    ) = (&plan.origin, &plan.action)
    {
        match registered(&plan.context)? {
            Some(InstallOrigin::Herdr {
                host: actual_host,
                host_config_dir: actual_config,
                checkout_root: actual_checkout,
                plugin_root: actual_plugin,
                source: actual_source,
                requested_ref: actual_ref,
                enabled: actual_enabled,
                ..
            }) if actual_host == *host
                && actual_config == *host_config_dir
                && actual_checkout == *checkout_root
                && actual_plugin == *plugin_root
                && actual_source == *source
                && actual_ref == *requested_ref
                && actual_enabled == *enabled => {}
            _ => {
                return Err(
                    "live Herdr host or managed source/ref changed before installation".into(),
                )
            }
        }
    }
    let locator = match &plan.origin {
        InstallOrigin::Herdr { plugin_root, .. } => plugin_root.join(APP),
        InstallOrigin::Local { root } => root.join(APP),
        InstallOrigin::Homebrew { locator, .. } | InstallOrigin::Manual { locator } => {
            locator.clone()
        }
        InstallOrigin::Unknown { .. } => return Err("unknown origin cannot be applied".into()),
    };
    let physical_root = crate::physical_install_root(&plan.origin)?;
    for p in [
        &plan.context.config_dir,
        &plan.context.state_dir,
        &plan.context.herdr_socket,
    ] {
        if resolved_parent_path(p)?.starts_with(physical_root) {
            return Err("profile path is inside replaceable installation".into());
        }
    }
    if let Some(p) = &plan.context.assets_override {
        if resolved_parent_path(p)?.starts_with(physical_root) {
            return Err("explicit asset override is inside replaceable installation".into());
        }
    }
    for key in ["HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME"] {
        if let Some(value) = plan.context.environment.get(key) {
            if resolved_parent_path(Path::new(value))?.starts_with(physical_root) {
                return Err(format!("{key} is inside replaceable installation"));
            }
        }
    }
    if matches!(plan.action, UpdateAction::ApplyInstalled) {
        if plan.context.running_origin.is_none() {
            return Err("apply requires startup-captured installation ownership".into());
        }
        let candidate = plan
            .candidate
            .as_ref()
            .ok_or("apply requires exact candidate image")?;
        if changed_candidate(&plan.context, &locator)?.as_ref() != Some(candidate) {
            return Err("installed candidate changed, unsupported or equals running image".into());
        }
    } else {
        if plan.candidate.is_some() {
            return Err("manager plan cannot inject a candidate".into());
        }
        let current = executable_identity(&locator)?;
        if current.sha256 != plan.running.sha256 {
            return Err(
                "installation changed outside updater; check again and apply existing candidate"
                    .into(),
            );
        }
    }
    match (&plan.origin, &plan.action) {
        (InstallOrigin::Herdr { enabled: true, .. }, UpdateAction::HerdrReinstall)
        | (InstallOrigin::Homebrew { .. }, UpdateAction::HomebrewUpgrade)
        | (InstallOrigin::Local { .. }, UpdateAction::LocalRebuild)
        | (InstallOrigin::Local { .. }, UpdateAction::ApplyInstalled)
        | (InstallOrigin::Herdr { enabled: true, .. }, UpdateAction::ApplyInstalled)
        | (InstallOrigin::Homebrew { .. }, UpdateAction::ApplyInstalled)
        | (InstallOrigin::Manual { .. }, UpdateAction::ApplyInstalled) => Ok(()),
        _ => Err("action does not match verified installation manager".into()),
    }
}

/// Resolve the actual post-install image at the previously captured installation,
/// never from PATH or a caller-selected path. A manager may change the active keg
/// or checked-out revision; the policy/source/formula must remain the same.
pub fn installed_candidate(plan: &UpdatePlan) -> Result<ExecutableIdentity, String> {
    installed_image(plan, false)
}

/// Explicit recovery may restart only the original, fully verified image when
/// a failed installer left the captured source/profile intact. This is NOT an update.
pub fn restored_original(plan: &UpdatePlan) -> Result<ExecutableIdentity, String> {
    if matches!(plan.action, UpdateAction::ApplyInstalled) {
        return Err("pinned installed candidate cannot be treated as original image".into());
    }
    let image = installed_image(plan, true)?;
    if image != plan.running {
        return Err(
            "original image changed after failed manager; cannot restore automatically".into(),
        );
    }
    if let InstallOrigin::Herdr {
        resolved_commit, ..
    } = &plan.origin
    {
        let actual = registered_with_host(&plan.context, false)?;
        if !matches!(actual, Some(InstallOrigin::Herdr { resolved_commit: current, .. }) if &current == resolved_commit)
        {
            return Err(
                "managed original registration revision changed after failed manager".into(),
            );
        }
    }
    if let InstallOrigin::Local { root } = &plan.origin {
        if version(&app_manifest(root)?)? != version(&plan.context.version)? {
            return Err("local manifest changed while original app image was retained".into());
        }
    }
    Ok(image)
}

fn installed_image(plan: &UpdatePlan, allow_original: bool) -> Result<ExecutableIdentity, String> {
    crate::protocol::validate_operation_id(&plan.operation_id)?;
    if plan.running != plan.context.running
        || plan.context.running_origin.as_deref() != Some(&plan.origin)
    {
        return Err("installed candidate does not belong to the captured live origin".into());
    }
    let locator = match &plan.origin {
        InstallOrigin::Herdr {
            host,
            host_config_dir,
            checkout_root,
            plugin_root,
            source,
            requested_ref,
            enabled,
            ..
        } => {
            let installed = registered_with_host(&plan.context, false)?
                .ok_or("managed registration disappeared after installation")?;
            match installed {
                InstallOrigin::Herdr {
                    host: actual_host,
                    host_config_dir: actual_config,
                    checkout_root: actual_checkout,
                    plugin_root: actual_plugin,
                    source: actual_source,
                    requested_ref: actual_ref,
                    enabled: actual_enabled,
                    resolved_commit,
                } if actual_host == *host
                    && actual_config == *host_config_dir
                    && actual_checkout == *checkout_root
                    && actual_plugin == *plugin_root
                    && actual_source == *source
                    && actual_ref == *requested_ref
                    && actual_enabled == *enabled
                    && resolved_commit.as_deref().is_some_and(commit) => {}
                _ => {
                    return Err(
                        "installed Herdr registration changed source/ref/channel or lacks revision"
                            .into(),
                    )
                }
            }
            manifest(plugin_root)?;
            plugin_root.join(APP)
        }
        InstallOrigin::Local { root } => {
            owned_file(&root.join("scripts/install.sh"), MAX_METADATA)?;
            manifest(root)?;
            root.join(APP)
        }
        InstallOrigin::Homebrew {
            brew,
            prefix,
            formula,
            locator,
            cellar_formula_root,
        } => {
            executable_identity(brew)?;
            let path = fs::canonicalize(locator).map_err(|e| e.to_string())?;
            match brew_origin(&path)? {
                Some(InstallOrigin::Homebrew {
                    brew: installed_brew,
                    prefix: installed_prefix,
                    formula: installed_formula,
                    locator: installed_locator,
                    cellar_formula_root: installed_cellar,
                }) if installed_brew == *brew
                    && installed_prefix == *prefix
                    && installed_cellar == *cellar_formula_root
                    && installed_formula == *formula
                    && installed_locator == *locator => {}
                _ => return Err("active Homebrew keg/formula or receipt changed origin".into()),
            }
            path
        }
        InstallOrigin::Manual { locator } => locator.clone(),
        InstallOrigin::Unknown { .. } => return Err("unknown installation has no candidate".into()),
    };
    let candidate = validate_candidate(&locator)?;
    // An explicit local rebuild may reproducibly produce the same image.
    // Application is proved by a fresh, profile-bound ready instance, not a hash delta.
    if candidate.sha256 == plan.running.sha256
        && !allow_original
        && !matches!(plan.action, UpdateAction::LocalRebuild)
    {
        return Err("installed executable has not changed".into());
    }
    if matches!(plan.action, UpdateAction::ApplyInstalled)
        && plan.candidate.as_ref() != Some(&candidate)
    {
        return Err("pinned installed candidate changed after confirmation".into());
    }
    let output = command_output(&candidate.path, &["--version"], 4096, 5)?;
    let installed_version = output
        .split_whitespace()
        .find_map(|word| version(word).ok())
        .ok_or("candidate version unavailable")?;
    if installed_version < version(&plan.context.version)? {
        return Err("candidate would downgrade the running app".into());
    }
    let manifest_root = match &plan.origin {
        InstallOrigin::Herdr { plugin_root, .. } => Some(plugin_root.as_path()),
        InstallOrigin::Local { root } => Some(root.as_path()),
        _ => None,
    };
    if let Some(root) = manifest_root {
        if version(&app_manifest(root)?)? < version(&plan.context.version)? {
            return Err("installed plugin manifest would downgrade the running app".into());
        }
    }
    Ok(candidate)
}

/// Fixed argv only. Execute once in helper after lifecycle reservation and fresh recheck.
pub fn manager_command(plan: &UpdatePlan) -> Result<Option<ManagerCommand>, String> {
    recheck_plan(plan)?;
    let c = &plan.context;
    let mut routing = c.environment.clone();
    routing.remove("HERDR_BIN_PATH");
    routing.insert(
        "HERDR_PLUGIN_CONFIG_DIR".into(),
        c.config_dir.to_string_lossy().into_owned(),
    );
    routing.insert(
        "HERDR_PLUGIN_STATE_DIR".into(),
        c.state_dir.to_string_lossy().into_owned(),
    );
    routing.insert(
        "HERDR_SOCKET_PATH".into(),
        c.herdr_socket.to_string_lossy().into_owned(),
    );
    let home = c
        .environment
        .get("HOME")
        .ok_or("captured HOME routing required for manager")?;
    ensure_absolute(Path::new(home))?;
    let path = format!("{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    routing.insert("PATH".into(), path);
    let routing: Vec<_> = routing.into_iter().collect();
    match (&plan.origin, &plan.action) {
        (
            InstallOrigin::Herdr {
                host,
                source,
                requested_ref,
                ..
            },
            UpdateAction::HerdrReinstall,
        ) => {
            parse_source(source)?;
            let mut args = vec!["plugin".into(), "install".into(), "--yes".into()];
            if let Some(r) = requested_ref {
                if !safe_ref(r) {
                    return Err("unsafe requested ref".into());
                }
                args.extend(["--ref".into(), r.clone()]);
            }
            args.push(source.clone());
            Ok(Some(ManagerCommand {
                program: host.clone(),
                args,
                cwd: c.state_dir.clone(),
                environment: routing,
            }))
        }
        (InstallOrigin::Local { root }, UpdateAction::LocalRebuild) => Ok(Some(ManagerCommand {
            program: PathBuf::from("/bin/bash"),
            args: vec![
                root.join("scripts/install.sh")
                    .to_string_lossy()
                    .into_owned(),
                "--source".into(),
            ],
            cwd: c.state_dir.clone(),
            environment: routing,
        })),
        (InstallOrigin::Homebrew { brew, formula, .. }, UpdateAction::HomebrewUpgrade) => {
            Ok(Some(ManagerCommand {
                program: brew.clone(),
                args: vec!["upgrade".into(), "--formula".into(), formula.clone()],
                cwd: c.state_dir.clone(),
                environment: routing,
            }))
        }
        (_, UpdateAction::ApplyInstalled) => Ok(None),
        _ => Err("unsupported manager command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn source_and_ref_cannot_inject_manager_arguments() {
        for source in [
            "-evil/repo",
            "owner/../repo",
            "owner/repo/./sub",
            "owner/repo\n--yes",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
        for reference in [
            "--upload-pack=malicious",
            "../main",
            "a@{1",
            "feature//other",
            "a\nb",
        ] {
            assert!(!safe_ref(reference), "{reference}");
        }
        assert!(parse_source("known-owner/app/path").is_ok());
        assert!(safe_ref("feature/next"));
    }

    #[test]
    fn annotated_tag_is_peeled_and_ambiguous_tag_branch_is_rejected() {
        let tag_object = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let ref_lines =
            format!("{tag_object}\trefs/tags/beta/0.2.0\n{commit}\trefs/tags/beta/0.2.0^{{}}\n");
        assert_eq!(
            resolve_ref(&ref_lines, Some("beta/0.2.0")).unwrap(),
            (Some(commit.into()), true)
        );
        let conflict = format!("{ref_lines}{tag_object}\trefs/heads/beta/0.2.0\n");
        assert!(resolve_ref(&conflict, Some("beta/0.2.0")).is_err());
        assert_eq!(
            resolve_ref(&format!("{commit}\tHEAD\n"), None).unwrap(),
            (Some(commit.into()), false)
        );
    }

    #[test]
    fn actual_git_annotated_and_lightweight_refs_are_resolved_as_checkout_commits() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("remote");
        let git = Path::new("/usr/bin/git");
        let run = |args: &[&str]| {
            let output = Command::new(git).args(args).output().unwrap();
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        run(&["init", "-q", repo.to_str().unwrap()]);
        fs::write(repo.join("readme"), b"commit contents").unwrap();
        let path = repo.to_str().unwrap();
        run(&["-C", path, "add", "readme"]);
        run(&[
            "-C",
            path,
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=f@example.test",
            "commit",
            "-qm",
            "fixture",
        ]);
        let sha = run(&["-C", path, "rev-parse", "HEAD"]).trim().to_owned();
        run(&["-C", path, "tag", "-a", "-m", "annotated", "release"]);
        run(&["-C", path, "tag", "light"]);
        assert_eq!(
            ls_remote(git, path, Some("release")).unwrap(),
            (Some(sha.clone()), true)
        );
        assert_eq!(
            ls_remote(git, path, Some("light")).unwrap(),
            (Some(sha.clone()), true)
        );
        assert_eq!(
            ls_remote(git, path, Some("HEAD")).unwrap(),
            (Some(sha.clone()), false)
        );
        assert_eq!(
            ls_remote(git, path, None).unwrap(),
            (Some(sha.clone()), false)
        );
        assert_eq!(
            ls_remote(git, path, Some(&sha)).unwrap(),
            (Some(sha.clone()), true)
        );
        run(&["-C", path, "branch", "release"]);
        assert!(ls_remote(git, path, Some("release")).is_err());
        assert_eq!(
            ls_remote(git, path, Some("refs/tags/release")).unwrap(),
            (Some(sha.clone()), true)
        );
        assert_eq!(
            ls_remote(git, path, Some("refs/heads/release")).unwrap(),
            (Some(sha), false)
        );
    }

    #[test]
    fn metadata_deadline_covers_descendant_pipe_and_oversized_output() {
        let begin = Instant::now();
        assert!(command_output(
            Path::new("/bin/sh"),
            &["-c", "sleep 3 >&2 & printf ready"],
            32,
            1
        )
        .is_err());
        assert!(
            begin.elapsed() < Duration::from_secs(2),
            "descendant stderr pipe exceeded deadline"
        );
        let begin = Instant::now();
        assert!(command_output(
            Path::new("/bin/sh"),
            &[
                "-c",
                "while :; do printf '0123456789' >&2; printf '0123456789'; done"
            ],
            1024,
            1
        )
        .is_err());
        assert!(
            begin.elapsed() < Duration::from_secs(2),
            "oversized concurrent output blocked metadata"
        );
    }

    #[test]
    fn global_host_registry_owns_subdirectory_even_with_named_socket_and_override_profile() {
        let dir = tempfile::tempdir().unwrap();
        let xdg = dir.path().join("xdg");
        let host_config = xdg.join("herdr-dev");
        let injected = host_config.join("plugins/config/desktop-pet");
        let checkout = host_config.join("plugins/github/checkout");
        let plugin = checkout.join("examples/pet");
        fs::create_dir_all(&injected).unwrap();
        fs::create_dir_all(plugin.join("dist/HerdrDesktopPet.app/Contents/MacOS")).unwrap();
        fs::write(
            plugin.join("herdr-plugin.toml"),
            b"id = 'desktop-pet'\nversion = '0.2.0'\nplatforms = ['macos']\n",
        )
        .unwrap();
        let binary = plugin.join(APP);
        fs::write(&binary, b"test image").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let host_binary = dir.path().join("herdr-host");
        fs::write(&host_binary, b"test host").unwrap();
        let expected_host_binary = fs::canonicalize(&host_binary).unwrap();
        let config = dir.path().join("overridden-profile");
        let state = dir.path().join("overridden-state");
        let socket = dir.path().join("sessions/named/server.sock");
        fs::write(host_config.join("plugins.json"), serde_json::to_vec(&serde_json::json!([{
            "plugin_id": "desktop-pet", "plugin_root": plugin, "manifest_path": plugin.join("herdr-plugin.toml"),
            "enabled": true, "source": {"kind":"github","owner":"owner","repo":"repo",
                "subdir":"examples/pet", "managed_path":checkout, "requested_ref":"release",
                "resolved_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        }])).unwrap()).unwrap();
        let expected_host_config = fs::canonicalize(&host_config).unwrap();
        let expected_checkout = fs::canonicalize(&checkout).unwrap();
        let expected_plugin = fs::canonicalize(&plugin).unwrap();
        let registry_bytes = fs::read(host_config.join("plugins.json")).unwrap();
        let mut environment = BTreeMap::new();
        environment.insert(
            "HERDR_BIN_PATH".into(),
            host_binary.to_string_lossy().into(),
        );
        environment.insert("XDG_CONFIG_HOME".into(), xdg.to_string_lossy().into());
        environment.insert("HOME".into(), dir.path().to_string_lossy().into());
        let mut context = UpdateContext {
            executable: binary.clone(),
            version: "0.2.0".into(),
            instance_id: "fixture".into(),
            config_dir: config,
            host_plugin_config_dir: Some(injected),
            state_dir: state,
            herdr_socket: socket,
            running: executable_identity(&binary).unwrap(),
            running_origin: None,
            assets_override: None,
            environment,
            locale: "en".into(),
        };
        let origin = detect_origin(&context).unwrap();
        match origin {
            InstallOrigin::Herdr {
                host,
                checkout_root,
                plugin_root,
                host_config_dir,
                ..
            } => {
                assert_eq!(
                    host, expected_host_binary,
                    "registered host binary must be physical"
                );
                assert_eq!(
                    checkout_root, expected_checkout,
                    "registered checkout must be physical"
                );
                assert_eq!(
                    plugin_root, expected_plugin,
                    "registered plugin must be physical"
                );
                assert_eq!(
                    host_config_dir, expected_host_config,
                    "host registry must be physical"
                );
            }
            other => panic!("expected registered Herdr origin, got {other:?}"),
        }
        let assert_unknown = |context: &UpdateContext| {
            let origin = detect_origin(context).unwrap();
            assert!(
                matches!(origin, InstallOrigin::Unknown { .. }),
                "expected Unknown origin, got {origin:?}"
            );
        };
        context.environment.insert(
            "XDG_CONFIG_HOME".into(),
            dir.path().join("wrong-xdg").to_string_lossy().into(),
        );
        assert_unknown(&context);
        context
            .environment
            .insert("XDG_CONFIG_HOME".into(), xdg.to_string_lossy().into());
        fs::write(host_config.join("plugins.json"), b"{bad").unwrap();
        assert_unknown(&context);
        fs::remove_file(host_config.join("plugins.json")).unwrap();
        assert_unknown(&context);
        let registry = host_config.join("plugins.json");
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(registry.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert_unknown(&context);
        assert!(start.elapsed() < Duration::from_secs(1));
        fs::remove_file(registry).unwrap();
        // The same recorded checkout cannot smuggle a symlinked plugin outside
        // its replaceable physical root.
        fs::write(host_config.join("plugins.json"), registry_bytes).unwrap();
        let escaped = dir.path().join("escaped-plugin");
        fs::rename(&plugin, &escaped).unwrap();
        std::os::unix::fs::symlink(&escaped, &plugin).unwrap();
        context.running = executable_identity(&binary).unwrap();
        assert_unknown(&context);
    }

    #[test]
    fn manually_launched_checkout_with_host_binary_hint_remains_local_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let xdg = base.join("xdg");
        let local = base.join("source-checkout");
        let make_checkout = |root: &Path| {
            fs::create_dir_all(root).unwrap();
            assert!(Command::new("/usr/bin/git")
                .args(["init", "-q"])
                .arg(root)
                .status()
                .unwrap()
                .success());
            fs::create_dir_all(root.join("native")).unwrap();
            fs::create_dir_all(root.join("scripts")).unwrap();
            fs::write(root.join("scripts/install.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
            fs::write(
                root.join("herdr-plugin.toml"),
                b"id = 'desktop-pet'\nversion = '0.2.0'\nplatforms = ['macos']\n",
            )
            .unwrap();
            let binary = root.join(APP);
            fs::create_dir_all(binary.parent().unwrap()).unwrap();
            fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            binary
        };
        let binary = make_checkout(&local);
        let host_binary = base.join("herdr-host");
        fs::write(&host_binary, b"host").unwrap();
        let mut environment = BTreeMap::new();
        environment.insert("HOME".into(), base.to_string_lossy().into());
        environment.insert("XDG_CONFIG_HOME".into(), xdg.to_string_lossy().into());
        let mut context = UpdateContext {
            executable: binary.clone(),
            version: "0.2.0".into(),
            instance_id: "manual-checkout".into(),
            config_dir: base.join("profile"),
            host_plugin_config_dir: None,
            state_dir: base.join("state"),
            herdr_socket: base.join("server.sock"),
            running: executable_identity(&binary).unwrap(),
            running_origin: None,
            assets_override: None,
            environment,
            locale: "en".into(),
        };
        for hinted in [false, true] {
            if hinted {
                context.environment.insert(
                    "HERDR_BIN_PATH".into(),
                    host_binary.to_string_lossy().into(),
                );
            }
            assert!(
                matches!(detect_origin(&context).unwrap(), InstallOrigin::Local { root } if root == local)
            );
            let snapshot = check(&context).unwrap();
            assert_eq!(snapshot.status, CheckStatus::Local);
            assert!(matches!(
                snapshot.plan,
                Some(UpdatePlan {
                    action: UpdateAction::LocalRebuild,
                    ..
                })
            ));
        }

        // A physically managed checkout cannot become Local just because its registry
        // is absent, regardless of whether an independently selected host binary exists.
        let managed = xdg.join("herdr/plugins/github/checkout");
        let managed_binary = make_checkout(&managed);
        context.executable = managed_binary.clone();
        context.running = executable_identity(&managed_binary).unwrap();
        for hinted in [true, false] {
            if hinted {
                context.environment.insert(
                    "HERDR_BIN_PATH".into(),
                    host_binary.to_string_lossy().into(),
                );
            } else {
                context.environment.remove("HERDR_BIN_PATH");
            }
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Unknown { .. }
            ));
        }

        // Original captured injection remains authoritative even for a source
        // outside the physical managed subtree, with no or malformed registry.
        context.executable = binary.clone();
        context.running = executable_identity(&binary).unwrap();
        let injected = xdg.join("herdr/plugins/config/desktop-pet");
        fs::create_dir_all(&injected).unwrap();
        context.host_plugin_config_dir = Some(injected);
        context.environment.insert(
            "HERDR_BIN_PATH".into(),
            host_binary.to_string_lossy().into(),
        );
        assert!(matches!(
            detect_origin(&context).unwrap(),
            InstallOrigin::Unknown { .. }
        ));
        fs::write(xdg.join("herdr/plugins.json"), b"{bad").unwrap();
        assert!(matches!(
            detect_origin(&context).unwrap(),
            InstallOrigin::Unknown { .. }
        ));
    }

    #[test]
    fn replaced_same_path_does_not_rewrite_running_identity_or_claim_current() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir
            .path()
            .join("HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let running = executable_identity(&binary).unwrap();
        let mut context = UpdateContext {
            executable: binary.clone(),
            version: "0.2.0".into(),
            instance_id: "run-1".into(),
            host_plugin_config_dir: None,
            config_dir: dir.path().join("config"),
            state_dir: dir.path().join("state"),
            herdr_socket: dir.path().join("socket"),
            assets_override: None,
            running: running.clone(),
            running_origin: Some(Box::new(InstallOrigin::Manual {
                locator: binary.clone(),
            })),
            environment: BTreeMap::new(),
            locale: "en".into(),
        };
        assert_eq!(check(&context).unwrap().status, CheckStatus::Unsupported);
        fs::write(&binary, b"#!/bin/sh\nexit 1\n").unwrap();
        let installed = executable_identity(&binary).unwrap();
        assert_ne!(installed.sha256, context.running.sha256);
        // Unsigned/unsupported replacement is rejected instead of assuming this daemon
        // is already current just because its former locator now points to new bytes.
        assert!(check(&context).is_err());
        assert_eq!(context.running, running);
        context.running_origin = Some(Box::new(InstallOrigin::Unknown {
            reason: "no owner".into(),
        }));
        assert_eq!(check(&context).unwrap().status, CheckStatus::Unsupported);
    }
}
