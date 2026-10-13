use crate::{
    ensure_absolute, executable_identity, now, owned_file, releases,
    validate_candidate_for_context, CheckSnapshot, CheckStatus, ExecutableIdentity, InstallOrigin,
    ReleaseCandidate, ReleaseChannel, ReleaseTarget, UpdateAction, UpdateContext, UpdatePlan,
    UpdatePolicy, VerifiedInstallation, OBSERVATION_MODE_ENV,
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
const BETA_APP: &str = "dist/HerdrDesktopPetBeta.app/Contents/MacOS/herdr-desktop-pet";
const MAX_METADATA: u64 = 1024 * 1024;
const OFFICIAL_SOURCE: &str = "hanbong5938/herdr-desktop-pet";
const OFFICIAL_TAP: &str = "hanbong5938/tap/";
const STABLE_FORMULA: &str = "herdr-desktop-pet";
const BETA_FORMULA: &str = "herdr-desktop-pet-beta";

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
    c.observation_mode()?;
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
                | OBSERVATION_MODE_ENV
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
fn app_for_version(root: &Path, release_version: &str) -> Result<PathBuf, String> {
    let parsed = version(release_version)?;
    if !UpdatePolicy::StableAndBeta.allows_version(release_version)? {
        return Err("unsupported managed app release channel".into());
    }
    Ok(root.join(if parsed.pre.is_empty() { APP } else { BETA_APP }))
}

fn installed_app(root: &Path) -> Result<PathBuf, String> {
    app_for_version(root, &app_manifest(root)?)
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
/// The startup image is historical: only its captured canonical path and version
/// select the bundle. An installed image must exist at the current manifest's
/// bundle path inside the physical registered checkout.
enum RegistrationImage<'a> {
    Captured,
    Installed(&'a Path),
}

fn registered(c: &UpdateContext) -> Result<Option<InstallOrigin>, String> {
    registered_with_host(c, true)
}
fn registered_with_host(
    c: &UpdateContext,
    require_live_host: bool,
) -> Result<Option<InstallOrigin>, String> {
    registered_with_host_at(c, require_live_host, RegistrationImage::Captured)
}

fn registered_with_host_at(
    c: &UpdateContext,
    require_live_host: bool,
    image_scope: RegistrationImage<'_>,
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
    let doc = manifest(&plugin_root)?;
    let image = match image_scope {
        RegistrationImage::Captured => {
            let image = app_for_version(&plugin_root, &c.version)?;
            if image != c.running.path || !c.running.path.starts_with(&plugin_root) {
                return Ok(None);
            }
            image
        }
        RegistrationImage::Installed(expected_image) => {
            let image = app_for_version(&plugin_root, &manifest_version(&doc)?)?;
            let physical = normalized(&image)?;
            if physical != image
                || physical.as_path() != expected_image
                || !physical.starts_with(&plugin_root)
            {
                return Ok(None);
            }
            physical
        }
    };
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
        || !image.starts_with(&checkout_root)
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
            InstallOrigin::Local { root } => (path == &app_for_version(root, &c.version)?
                && path.starts_with(root)
                && normalized(root).is_ok_and(|physical| &physical == root)
                && manifest(root).is_ok()
                && normalized(&root.join("scripts/install.sh"))
                    .is_ok_and(|script| script.starts_with(root))
                && owned_file(&root.join("scripts/install.sh"), MAX_METADATA).is_ok())
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
        installed_app(p).is_ok_and(|app| app == *path)
            && p.join("scripts/install.sh").is_file()
            && p.join("herdr-plugin.toml").is_file()
    }) {
        if installed_app(root).is_ok_and(|app| same(&app, path))
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

fn reviewed_launcher(doc: &toml::Value) -> bool {
    doc.get("startup")
        .and_then(toml::Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("command")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|args| {
                        args.len() == 3
                            && args[0].as_str() == Some("bash")
                            && args[1].as_str() == Some("scripts/run-native.sh")
                            && args[2].as_str() == Some("ensure")
                    })
            })
        })
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
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
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
    if !reviewed_launcher(&doc)
        && !startup.iter().any(|entry| {
            entry
                .get("command")
                .and_then(toml::Value::as_array)
                .is_some_and(|args| {
                    args.len() == 2
                        && args[0].as_str()
                            == Some("./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet")
                        && args[1].as_str() == Some("ensure")
                })
        })
    {
        return Err("remote startup command is not the expected bundle ensure".into());
    }
    let release = manifest_version(&doc)?;
    Ok((release, doc))
}

fn official_formula(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => STABLE_FORMULA,
        ReleaseChannel::Beta => BETA_FORMULA,
    }
}

fn brew_metadata(brew: &Path, formula: &str) -> Result<Value, String> {
    let short = formula
        .strip_prefix(OFFICIAL_TAP)
        .ok_or("unofficial formula tap")?;
    if !matches!(short, STABLE_FORMULA | BETA_FORMULA) {
        return Err("unofficial formula".into());
    }
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
    if json.pointer("/formulae/1").is_some()
        || details.get("name").and_then(Value::as_str) != Some(short)
        || details.get("full_name").and_then(Value::as_str) != Some(formula)
    {
        return Err("brew returned a different formula/tap".into());
    }
    Ok(details.clone())
}

fn brew_matches_release(details: &Value, release: &ReleaseCandidate) -> Result<(), String> {
    let version = details.pointer("/versions/stable").and_then(Value::as_str);
    let url = details.pointer("/urls/stable/url").and_then(Value::as_str);
    let digest = details
        .pointer("/urls/stable/checksum")
        .and_then(Value::as_str);
    if version != Some(release.version.as_str())
        || url != Some(release.archive_url.as_str())
        || digest != Some(release.sha256.as_str())
    {
        return Err(
            "cached Homebrew formula version, archive or checksum differs from approved release"
                .into(),
        );
    }
    Ok(())
}

fn release_target(
    c: &UpdateContext,
    origin: &InstallOrigin,
    release: &ReleaseCandidate,
    policy: UpdatePolicy,
) -> Result<Option<ReleaseTarget>, String> {
    release.validate()?;
    if !policy.allows_version(&release.version)? {
        return Err("release violates selected update policy".into());
    }
    let (target_origin, resolved_commit) = match origin {
        InstallOrigin::Herdr {
            host,
            host_config_dir,
            checkout_root,
            plugin_root,
            source,
            enabled: true,
            ..
        } if source == OFFICIAL_SOURCE => {
            let (resolved, immutable) = observed_herdr(source, Some(&release.tag))?;
            let revision = resolved.ok_or("approved release tag cannot be resolved")?;
            if !immutable || !commit(&revision) {
                return Err("approved release tag does not resolve to immutable commit".into());
            }
            let (manifest_release, doc) = remote_manifest(source, &revision)?;
            if manifest_release != release.version || !reviewed_launcher(&doc) {
                return Err(
                    "approved Herdr commit lacks exact release manifest or channel-aware launcher"
                        .into(),
                );
            }
            check_host(c, host, &doc)?;
            (
                InstallOrigin::Herdr {
                    host: host.clone(),
                    host_config_dir: host_config_dir.clone(),
                    checkout_root: checkout_root.clone(),
                    plugin_root: plugin_root.clone(),
                    source: source.clone(),
                    requested_ref: Some(revision.clone()),
                    resolved_commit: Some(revision.clone()),
                    enabled: true,
                },
                Some(revision),
            )
        }
        InstallOrigin::Homebrew { brew, prefix, .. } => {
            let formula = official_formula(release.channel);
            let full = format!("{OFFICIAL_TAP}{formula}");
            let bundle = if release.channel == ReleaseChannel::Beta {
                "HerdrDesktopPetBeta.app"
            } else {
                "HerdrDesktopPet.app"
            };
            (
                InstallOrigin::Homebrew {
                    brew: brew.clone(),
                    prefix: prefix.clone(),
                    cellar_formula_root: prefix.join("Cellar").join(formula),
                    formula: full,
                    locator: prefix
                        .join("opt")
                        .join(formula)
                        .join("libexec")
                        .join(bundle)
                        .join("Contents/MacOS/herdr-desktop-pet"),
                },
                None,
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(ReleaseTarget {
        policy,
        release: release.clone(),
        origin: target_origin,
        resolved_commit,
    }))
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
        target: None,
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
    let verified = validate_candidate_for_context(locator, c)?;
    let output = command_output(locator, &["--version"], 4096, 5)?;
    let candidate_version = output
        .split_whitespace()
        .find_map(|word| version(word).ok())
        .ok_or("candidate version unavailable")?;
    if candidate_version < version(&c.version)? {
        return Err("candidate would downgrade the running app".into());
    }
    if executable_identity(locator)? != verified {
        return Err("candidate changed after version verification".into());
    }
    Ok(Some(verified))
}

pub fn check(c: &UpdateContext, policy: UpdatePolicy) -> Result<CheckSnapshot, String> {
    let origin = detect_origin(c)?;
    let at = now()?;
    let mut snapshot = CheckSnapshot {
        status: CheckStatus::Unsupported,
        origin: origin.clone(),
        installed_version: None,
        observed_revision: None,
        detail: String::new(),
        plan: None,
        policy,
        release: None,
        checked_at: Some(at),
    };
    if let Ok(candidates) = releases::fetch_candidates() {
        snapshot.release = releases::select_candidate(&candidates, &c.version, policy)?;
    }
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
                if let Some(candidate) = changed_candidate(c, &installed_app(plugin_root)?)? {
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
                && executable_identity(&installed_app(plugin_root)?)
                    .is_ok_and(|now| now.sha256 != c.running.sha256)
            {
                snapshot.detail = "Managed binary changed without a new registered revision; no verified replacement provenance".into();
                return Ok(snapshot);
            }
            if source == OFFICIAL_SOURCE {
                if let Some(release) = &snapshot.release {
                    match release_target(c, &origin, release, policy) {
                        Ok(Some(target)) => {
                            let mut plan =
                                new_plan(c, &origin, UpdateAction::HerdrReinstall, None)?;
                            plan.target = Some(target);
                            snapshot.status = CheckStatus::Available;
                            snapshot.detail = format!(
                                "Official release {} ({}) requires explicit recorded ref -> pinned commit change",
                                release.version, release.tag
                            );
                            snapshot.plan = Some(plan);
                            return Ok(snapshot);
                        }
                        Err(reason) => {
                            snapshot.detail = format!(
                                "Official release {} is informational: {reason}",
                                release.version
                            );
                            return Ok(snapshot);
                        }
                        Ok(None) => {}
                    }
                }
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
                if let Some(candidate) = changed_candidate(c, &installed_app(root)?)? {
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
            let details = brew_metadata(brew, formula)?;
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
            if let Some(release) = &snapshot.release {
                match release_target(c, &origin, release, policy) {
                    Ok(Some(target)) => {
                        let mut plan = new_plan(c, &origin, UpdateAction::HomebrewUpgrade, None)?;
                        plan.target = Some(target);
                        snapshot.status = CheckStatus::Available;
                        snapshot.detail = format!(
                            "Official release {} ({}) requires explicit formula change from {} to {}",
                            release.version, release.tag, formula,
                            match plan.target_origin() {
                                InstallOrigin::Homebrew { formula, .. } => formula.as_str(),
                                _ => unreachable!(),
                            }
                        );
                        snapshot.plan = Some(plan);
                        return Ok(snapshot);
                    }
                    Err(reason) => {
                        snapshot.detail = format!(
                            "Official release {} is informational: {reason}",
                            release.version
                        );
                        return Ok(snapshot);
                    }
                    Ok(None) => {}
                }
            }
            snapshot.status = CheckStatus::Current;
            snapshot.detail = format!(
                "Cached Homebrew formula lists {latest}; no matching newer approved release was verified"
            );
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
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut real) => {
                for name in suffix.into_iter().rev() {
                    real.push(name);
                }
                return Ok(real);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Only append genuinely absent components. A dangling symlink
                // must not hide a target inside the replaceable installation.
                match fs::symlink_metadata(ancestor) {
                    Ok(_) => return Err(error.to_string()),
                    Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
                    Err(other) => return Err(other.to_string()),
                }
                suffix.push(ancestor.file_name().ok_or("path has no basename")?);
                ancestor = ancestor.parent().ok_or("path has no parent")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Refresh the approved public identity only on explicitly consented execution,
/// while the original daemon is still running.
pub fn refresh_release_target(plan: &UpdatePlan) -> Result<(), String> {
    recheck_plan(plan)?;
    let Some(target) = &plan.target else {
        return Ok(());
    };
    let catalog = releases::fetch_candidates()?;
    if !catalog.iter().any(|candidate| candidate == &target.release) {
        return Err("approved release metadata changed since check".into());
    }
    match (&plan.origin, &target.origin) {
        (InstallOrigin::Homebrew { brew, .. }, InstallOrigin::Homebrew { formula, .. }) => {
            command_output(brew, &["update"], 65536, 60)?;
            brew_matches_release(&brew_metadata(brew, formula)?, &target.release)?;
        }
        (InstallOrigin::Herdr { source, host, .. }, InstallOrigin::Herdr { requested_ref, .. }) => {
            let (resolved, immutable) = observed_herdr(source, Some(&target.release.tag))?;
            if !immutable || resolved.as_ref() != requested_ref.as_ref() {
                return Err("official release tag moved since consent".into());
            }
            let (manifest_release, doc) = remote_manifest(
                source,
                requested_ref.as_deref().ok_or("missing pinned commit")?,
            )?;
            if manifest_release != target.release.version || !reviewed_launcher(&doc) {
                return Err("pinned Herdr release manifest/launcher changed since consent".into());
            }
            check_host(&plan.context, host, &doc)?;
        }
        _ => return Err("release target changed manager/source".into()),
    }
    recheck_plan(plan)
}

pub fn validate_plan_target(plan: &UpdatePlan) -> Result<(), String> {
    let Some(target) = &plan.target else {
        return Ok(());
    };
    target.release.validate()?;
    if !target.policy.allows_version(&target.release.version)?
        || version(&target.release.version)?.cmp_precedence(&version(&plan.context.version)?)
            != std::cmp::Ordering::Greater
        || plan.candidate.is_some()
    {
        return Err("release target violates policy, precedence or candidate scope".into());
    }
    match (&plan.origin, &target.origin, &plan.action) {
        (
            InstallOrigin::Herdr {
                host,
                host_config_dir,
                checkout_root,
                plugin_root,
                source,
                enabled: true,
                ..
            },
            InstallOrigin::Herdr {
                host: next_host,
                host_config_dir: next_config,
                checkout_root: next_checkout,
                plugin_root: next_plugin,
                source: next_source,
                requested_ref: Some(reference),
                resolved_commit: Some(revision),
                enabled: true,
            },
            UpdateAction::HerdrReinstall,
        ) if source == OFFICIAL_SOURCE
            && next_host == host
            && next_config == host_config_dir
            && next_checkout == checkout_root
            && next_plugin == plugin_root
            && next_source == source
            && commit(reference)
            && reference == revision
            && target.resolved_commit.as_deref() == Some(reference) => {}
        (
            InstallOrigin::Homebrew { brew, prefix, .. },
            InstallOrigin::Homebrew {
                brew: next_brew,
                prefix: next_prefix,
                cellar_formula_root,
                formula,
                locator,
            },
            UpdateAction::HomebrewUpgrade,
        ) if next_brew == brew
            && next_prefix == prefix
            && formula
                == &format!("{OFFICIAL_TAP}{}", official_formula(target.release.channel))
            && cellar_formula_root
                == &prefix
                    .join("Cellar")
                    .join(official_formula(target.release.channel))
            && locator
                == &prefix
                    .join("opt")
                    .join(official_formula(target.release.channel))
                    .join("libexec")
                    .join(if target.release.channel == ReleaseChannel::Beta {
                        "HerdrDesktopPetBeta.app"
                    } else {
                        "HerdrDesktopPet.app"
                    })
                    .join("Contents/MacOS/herdr-desktop-pet")
            && target.resolved_commit.is_none() => {}
        _ => {
            return Err(
                "release target changes unapproved manager, source or physical prefix".into(),
            )
        }
    }
    Ok(())
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
    validate_plan_target(plan)?;
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
        InstallOrigin::Herdr { plugin_root, .. } => installed_app(plugin_root)?,
        InstallOrigin::Local { root } => installed_app(root)?,
        InstallOrigin::Homebrew { locator, .. } | InstallOrigin::Manual { locator } => {
            locator.clone()
        }
        InstallOrigin::Unknown { .. } => return Err("unknown origin cannot be applied".into()),
    };
    let roots = [
        Some(crate::physical_install_root(&plan.origin)?),
        plan.target
            .as_ref()
            .map(|target| crate::physical_install_root(&target.origin))
            .transpose()?,
    ];
    for root in roots.into_iter().flatten() {
        for p in [
            &plan.context.config_dir,
            &plan.context.state_dir,
            &plan.context.herdr_socket,
        ] {
            if resolved_parent_path(p)?.starts_with(root) {
                return Err("profile path is inside replaceable installation".into());
            }
        }
        if let Some(p) = &plan.context.assets_override {
            if resolved_parent_path(p)?.starts_with(root) {
                return Err("explicit asset override is inside replaceable installation".into());
            }
        }
        for key in ["HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME"] {
            if let Some(value) = plan.context.environment.get(key) {
                if resolved_parent_path(Path::new(value))?.starts_with(root) {
                    return Err(format!("{key} is inside replaceable installation"));
                }
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
pub fn installed_candidate(plan: &UpdatePlan) -> Result<VerifiedInstallation, String> {
    installed_image(plan, false)
}

/// Explicit recovery may restart only the original, fully verified image when
/// a failed installer left the captured source/profile intact. This is NOT an update.
pub fn restored_original(plan: &UpdatePlan) -> Result<VerifiedInstallation, String> {
    if matches!(plan.action, UpdateAction::ApplyInstalled) {
        return Err("pinned installed candidate cannot be treated as original image".into());
    }
    let installed = installed_image(plan, true)?;
    if installed.image != plan.running || installed.origin != plan.origin {
        return Err("actual original image or registration changed after failed manager".into());
    }
    Ok(installed)
}

fn matches_installed_origin(
    actual: &InstallOrigin,
    expected: &InstallOrigin,
    allow_original: bool,
    has_target: bool,
) -> bool {
    match (actual, expected) {
        (
            InstallOrigin::Herdr {
                host: actual_host,
                host_config_dir: actual_config,
                checkout_root: actual_checkout,
                plugin_root: actual_plugin,
                source: actual_source,
                requested_ref: actual_ref,
                resolved_commit: actual_commit,
                enabled: actual_enabled,
            },
            InstallOrigin::Herdr {
                host,
                host_config_dir,
                checkout_root,
                plugin_root,
                source,
                requested_ref,
                resolved_commit,
                enabled,
            },
        ) => {
            actual_host == host
            && actual_config == host_config_dir
            && actual_checkout == checkout_root
            && actual_plugin == plugin_root
            && actual_source == source
            && actual_ref == requested_ref
            && actual_enabled == enabled
            && actual_commit.as_deref().is_some_and(commit)
            // A mutable source-only ref can advance; explicit targets, originals,
            // and immutable source refs must retain their exact recorded commit.
            && (if allow_original || has_target || requested_ref.as_deref().is_some_and(commit) {
                actual_commit == resolved_commit
            } else {
                true
            })
        }
        _ => actual == expected,
    }
}

fn installed_image(
    plan: &UpdatePlan,
    allow_original: bool,
) -> Result<VerifiedInstallation, String> {
    crate::protocol::validate_operation_id(&plan.operation_id)?;
    if plan.running != plan.context.running
        || plan.context.running_origin.as_deref() != Some(&plan.origin)
    {
        return Err("installed candidate does not belong to the captured live origin".into());
    }
    let expected = if allow_original {
        &plan.origin
    } else {
        plan.target_origin()
    };
    let (locator, actual_origin) = match expected {
        InstallOrigin::Herdr { plugin_root, .. } => {
            let locator = installed_app(plugin_root)?;
            let actual = registered_with_host_at(
                &plan.context,
                false,
                RegistrationImage::Installed(&locator),
            )?
            .ok_or("managed registration disappeared after installation")?;
            if !matches_installed_origin(&actual, expected, allow_original, plan.target.is_some()) {
                return Err(
                    "installed Herdr registration changed source/ref/root/host or revision".into(),
                );
            }
            (locator, actual)
        }
        InstallOrigin::Local { root } => {
            owned_file(&root.join("scripts/install.sh"), MAX_METADATA)?;
            let locator = installed_app(root)?;
            if normalized(root)? != *root
                || normalized(&locator)? != locator
                || !locator.starts_with(root)
            {
                return Err("installed local app escapes physical checkout".into());
            }
            (locator, expected.clone())
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
            let actual = brew_origin(&path)?.ok_or("active Homebrew keg disappeared")?;
            if !matches!(actual, InstallOrigin::Homebrew {
                brew: ref installed_brew, prefix: ref installed_prefix,
                formula: ref installed_formula, locator: ref installed_locator,
                cellar_formula_root: ref installed_cellar,
            } if installed_brew == brew && installed_prefix == prefix
                && installed_cellar == cellar_formula_root
                && installed_formula == formula && installed_locator == locator)
            {
                return Err("active Homebrew keg/formula or receipt changed origin".into());
            }
            (path, actual)
        }
        InstallOrigin::Manual { locator } => (locator.clone(), expected.clone()),
        InstallOrigin::Unknown { .. } => return Err("unknown installation has no candidate".into()),
    };
    let candidate = validate_candidate_for_context(&locator, &plan.context)?;
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
    if let Some(target) = &plan.target {
        if !allow_original && installed_version != version(&target.release.version)? {
            return Err("installed executable version differs from approved release".into());
        }
    }
    if installed_version < version(&plan.context.version)? {
        return Err("candidate would downgrade the running app".into());
    }
    let manifest_root = match expected {
        InstallOrigin::Herdr { plugin_root, .. } => Some(plugin_root.as_path()),
        InstallOrigin::Local { root } => Some(root.as_path()),
        _ => None,
    };
    if let Some(root) = manifest_root {
        let manifest_version = version(&app_manifest(root)?)?;
        if manifest_version != installed_version {
            return Err("installed executable version differs from installed manifest".into());
        }
        if manifest_version < version(&plan.context.version)? {
            return Err("installed plugin manifest would downgrade the running app".into());
        }
        if allow_original && manifest_version != version(&plan.context.version)? {
            return Err("original manifest changed after failed manager".into());
        }
    }
    if let Some(target) = &plan.target {
        if !allow_original {
            if let InstallOrigin::Homebrew { brew, formula, .. } = &target.origin {
                let details = brew_metadata(brew, formula)?;
                brew_matches_release(&details, &target.release)?;
                if details
                    .pointer("/installed/0/version")
                    .and_then(Value::as_str)
                    != Some(target.release.version.as_str())
                {
                    return Err(
                        "installed Homebrew receipt version differs from approved release".into(),
                    );
                }
            }
            if let InstallOrigin::Herdr { plugin_root, .. } = &target.origin {
                if version(&app_manifest(plugin_root)?)? != version(&target.release.version)? {
                    return Err("installed Herdr manifest differs from approved release".into());
                }
                if !reviewed_launcher(&manifest(plugin_root)?) {
                    return Err(
                        "installed Herdr release lacks channel-aware startup launcher".into(),
                    );
                }
            }
        }
    }
    if executable_identity(&locator)? != candidate {
        return Err("installed executable changed after metadata verification".into());
    }
    if let InstallOrigin::Herdr { .. } = expected {
        let current =
            registered_with_host_at(&plan.context, false, RegistrationImage::Installed(&locator))?
                .ok_or("managed registration disappeared after candidate verification")?;
        if current != actual_origin {
            return Err("managed registration changed during candidate verification".into());
        }
    }
    Ok(VerifiedInstallation {
        image: candidate,
        origin: actual_origin,
    })
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
    match (plan.target_origin(), &plan.action) {
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
            let mut environment = routing;
            if let Some(target) = &plan.target {
                target.release.validate()?;
                environment.insert("HERDR_PET_INSTALL_MODE".into(), "prebuilt".into());
                environment.insert("HERDR_PET_REPOSITORY".into(), OFFICIAL_SOURCE.into());
                environment.insert("HERDR_PET_VERSION".into(), target.release.version.clone());
                environment.insert("HERDR_PET_RELEASE_TAG".into(), target.release.tag.clone());
                environment.insert(
                    "HERDR_PET_ASSET_NAME".into(),
                    target.release.asset_name.clone(),
                );
                environment.insert(
                    "HERDR_PET_RELEASE_URL".into(),
                    target.release.archive_url.clone(),
                );
                environment.insert(
                    "HERDR_PET_CHECKSUM_URL".into(),
                    target.release.checksum_url.clone(),
                );
                environment.insert("HERDR_PET_SHA256".into(), target.release.sha256.clone());
                environment.insert(
                    "HERDR_PET_APP_NAME".into(),
                    if target.release.channel == ReleaseChannel::Beta {
                        "HerdrDesktopPetBeta.app"
                    } else {
                        "HerdrDesktopPet.app"
                    }
                    .into(),
                );
            }
            Ok(Some(ManagerCommand {
                program: host.clone(),
                args,
                cwd: c.state_dir.clone(),
                environment: environment.into_iter().collect(),
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
            environment: routing.into_iter().collect(),
        })),
        (InstallOrigin::Homebrew { brew, formula, .. }, UpdateAction::HomebrewUpgrade) => {
            let installed = if let Some(target) = &plan.target {
                let details = brew_metadata(brew, formula)?;
                brew_matches_release(&details, &target.release)?;
                details
                    .pointer("/installed/0/version")
                    .and_then(Value::as_str)
                    .is_some()
            } else {
                true
            };
            let mut environment = routing;
            if plan.target.is_some() {
                environment.insert("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into());
                environment.insert("HOMEBREW_NO_ANALYTICS".into(), "1".into());
            }
            Ok(Some(ManagerCommand {
                program: brew.clone(),
                args: vec![
                    if installed { "upgrade" } else { "install" }.into(),
                    "--formula".into(),
                    formula.clone(),
                ],
                cwd: c.state_dir.clone(),
                environment: environment.into_iter().collect(),
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
    fn immutable_context_accepts_only_canonical_captured_selection() {
        let mut context = UpdateContext {
            executable: "/tmp/original".into(),
            version: "0.3.3".into(),
            instance_id: "original".into(),
            config_dir: "/tmp/profile".into(),
            host_plugin_config_dir: None,
            state_dir: "/tmp/state".into(),
            herdr_socket: "/tmp/host.sock".into(),
            running: ExecutableIdentity {
                path: "/tmp/original".into(),
                sha256: "a".repeat(64),
            },
            running_origin: Some(Box::new(InstallOrigin::Local {
                root: "/tmp".into(),
            })),
            assets_override: None,
            environment: BTreeMap::new(),
            locale: "en".into(),
        };
        assert!(context_valid(&context).is_ok());
        let default_context = context.clone();
        for mode in ["herdr", "tern", "both"] {
            context
                .environment
                .insert(OBSERVATION_MODE_ENV.into(), mode.into());
            assert!(context_valid(&context).is_ok());
            let bytes = serde_json::to_vec(&context).unwrap();
            let retained: UpdateContext = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(retained, context);
            assert_eq!(serde_json::to_vec(&retained).unwrap(), bytes);
            if mode == "herdr" {
                assert_eq!(
                    retained.observation_mode().unwrap(),
                    crate::ObservationMode::Herdr
                );
                assert_ne!(retained, default_context);
            }
        }
        context
            .environment
            .insert(OBSERVATION_MODE_ENV.into(), "TERN".into());
        assert!(context_valid(&context).is_err());
        context.environment.remove(OBSERVATION_MODE_ENV);
        context
            .environment
            .insert("UNAPPROVED_ROUTING".into(), "value".into());
        assert!(context_valid(&context).is_err());
    }

    #[test]
    fn recheck_preserves_isolation_with_missing_socket_ancestors() {
        let temp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let root = base.join("installation");
        let binary = root.join(APP);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(root.join("scripts/install.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
        fs::write(
            root.join("herdr-plugin.toml"),
            b"id = 'desktop-pet'\nversion = '0.3.3'\nplatforms = ['macos']\n",
        )
        .unwrap();
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let running = executable_identity(&binary).unwrap();
        let origin = InstallOrigin::Local { root: root.clone() };
        let plan = UpdatePlan {
            version: crate::protocol::UPDATER_PROTOCOL,
            operation_id: "0123456789abcdef0123456789abcdef".into(),
            context: UpdateContext {
                executable: binary,
                version: "0.3.3".into(),
                instance_id: "tern-original".into(),
                config_dir: base.join("profile"),
                host_plugin_config_dir: None,
                state_dir: base.join("state"),
                herdr_socket: base.join("missing/parent/herdr.sock"),
                running: running.clone(),
                running_origin: Some(Box::new(origin.clone())),
                assets_override: None,
                environment: BTreeMap::from([(OBSERVATION_MODE_ENV.into(), "tern".into())]),
                locale: "en".into(),
            },
            origin,
            target: None,
            action: UpdateAction::LocalRebuild,
            running,
            candidate: None,
        };
        let original = serde_json::to_vec(&plan).unwrap();
        assert!(recheck_plan(&plan).is_ok());
        assert_eq!(serde_json::to_vec(&plan).unwrap(), original);
        assert!(!base.join("missing").exists());

        let alias = base.join("installation-alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        for socket in [
            root.join("missing/parent/herdr.sock"),
            alias.join("missing/parent/herdr.sock"),
        ] {
            let mut unsafe_plan = plan.clone();
            unsafe_plan.context.herdr_socket = socket;
            assert_eq!(
                recheck_plan(&unsafe_plan).unwrap_err(),
                "profile path is inside replaceable installation"
            );
        }
        let external_alias = base.join("external-alias");
        std::os::unix::fs::symlink(&base, &external_alias).unwrap();
        let mut aliased_plan = plan.clone();
        aliased_plan.context.herdr_socket = external_alias.join("missing/parent/herdr.sock");
        assert!(recheck_plan(&aliased_plan).is_ok());

        // Files and dangling aliases cannot be treated as absent directories.
        let blocked = base.join("not-a-directory");
        fs::write(&blocked, b"file").unwrap();
        let mut blocked_plan = plan.clone();
        blocked_plan.context.herdr_socket = blocked.join("herdr.sock");
        assert!(recheck_plan(&blocked_plan).is_err());
        let dangling = base.join("dangling-alias");
        std::os::unix::fs::symlink(root.join("missing"), &dangling).unwrap();
        blocked_plan.context.herdr_socket = dangling.join("herdr.sock");
        assert!(recheck_plan(&blocked_plan).is_err());
    }

    #[test]
    fn beta_manifest_selects_beta_bundle_and_requires_reviewed_release_launcher() {
        let root = Path::new("/tmp/managed-pet");
        assert_eq!(
            app_for_version(root, "0.3.4-beta.2").unwrap(),
            root.join(BETA_APP)
        );
        assert_eq!(app_for_version(root, "0.3.4").unwrap(), root.join(APP));
        let launcher: toml::Value =
            "[[startup]]\ncommand = ['bash', 'scripts/run-native.sh', 'ensure']\n"
                .parse()
                .unwrap();
        let legacy: toml::Value =
            "[[startup]]\ncommand = ['./dist/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet', 'ensure']\n"
                .parse()
                .unwrap();
        assert!(reviewed_launcher(&launcher));
        assert!(!reviewed_launcher(&legacy));
    }

    #[test]
    fn approved_herdr_target_pins_commit_without_rewriting_original_ref() {
        let commit = "a".repeat(40);
        let original = InstallOrigin::Herdr {
            host: "/tmp/host".into(),
            host_config_dir: "/tmp/config".into(),
            checkout_root: "/tmp/checkout".into(),
            plugin_root: "/tmp/checkout".into(),
            source: OFFICIAL_SOURCE.into(),
            requested_ref: Some("my-feature".into()),
            resolved_commit: Some("b".repeat(40)),
            enabled: true,
        };
        let release = ReleaseCandidate {
            version: "0.3.4-beta.2".into(),
            channel: ReleaseChannel::Beta,
            tag: "beta/0.3.4-beta.2".into(),
            release_id: 1,
            asset_name: "HerdrDesktopPet-v0.3.4-beta.2-macos-arm64.tar.gz".into(),
            archive_url: "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/beta/0.3.4-beta.2/HerdrDesktopPet-v0.3.4-beta.2-macos-arm64.tar.gz".into(),
            archive_size: 100,
            sha256: "0".repeat(64),
            checksum_url: "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/beta/0.3.4-beta.2/HerdrDesktopPet-v0.3.4-beta.2-macos-arm64.tar.gz.sha256".into(),
        };
        let mut target = original.clone();
        if let InstallOrigin::Herdr {
            requested_ref,
            resolved_commit,
            ..
        } = &mut target
        {
            *requested_ref = Some(commit.clone());
            *resolved_commit = Some(commit.clone());
        }
        let image = ExecutableIdentity {
            path: "/tmp/image".into(),
            sha256: "1".repeat(64),
        };
        let context = UpdateContext {
            executable: image.path.clone(),
            version: "0.3.3".into(),
            instance_id: "test".into(),
            config_dir: "/tmp/profile".into(),
            host_plugin_config_dir: None,
            state_dir: "/tmp/state".into(),
            herdr_socket: "/tmp/socket".into(),
            running: image.clone(),
            running_origin: Some(Box::new(original.clone())),
            assets_override: None,
            environment: BTreeMap::new(),
            locale: "en".into(),
        };
        let mut plan = UpdatePlan {
            version: crate::protocol::UPDATER_PROTOCOL,
            operation_id: "0123456789abcdef0123456789abcdef".into(),
            context,
            origin: original.clone(),
            action: UpdateAction::HerdrReinstall,
            running: image,
            candidate: None,
            target: Some(ReleaseTarget {
                policy: UpdatePolicy::StableAndBeta,
                release,
                origin: target,
                resolved_commit: Some(commit.clone()),
            }),
        };
        assert!(validate_plan_target(&plan).is_ok());
        assert_eq!(plan.origin, original);
        assert_eq!(
            plan.target_origin().clone(),
            plan.target.as_ref().unwrap().origin
        );
        plan.target.as_mut().unwrap().resolved_commit = Some("c".repeat(40));
        assert!(validate_plan_target(&plan).is_err());
        plan.target.as_mut().unwrap().resolved_commit = Some(commit);
        plan.target.as_mut().unwrap().policy = UpdatePolicy::StableOnly;
        assert!(validate_plan_target(&plan).is_err());
    }

    #[test]
    fn stale_cached_formula_can_be_proposed_but_execution_requires_exact_refresh() {
        let release = ReleaseCandidate {
            version: "0.3.4".into(), channel: ReleaseChannel::Stable,
            tag: "v0.3.4".into(), release_id: 4,
            asset_name: "HerdrDesktopPet-v0.3.4-macos-arm64.tar.gz".into(),
            archive_url: "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/v0.3.4/HerdrDesktopPet-v0.3.4-macos-arm64.tar.gz".into(),
            archive_size: 100, sha256: "f".repeat(64),
            checksum_url: "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/v0.3.4/HerdrDesktopPet-v0.3.4-macos-arm64.tar.gz.sha256".into(),
        };
        let prefix = PathBuf::from("/tmp/captured-brew-prefix");
        let original = InstallOrigin::Homebrew {
            brew: prefix.join("bin/brew"),
            prefix: prefix.clone(),
            cellar_formula_root: prefix.join("Cellar/herdr-desktop-pet"),
            formula: "hanbong5938/tap/herdr-desktop-pet".into(),
            locator: prefix.join("opt/herdr-desktop-pet/libexec/HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet"),
        };
        let proposal = release_target(
            &UpdateContext {
                executable: "/tmp/captured-brew".into(),
                version: "0.3.3".into(),
                instance_id: "fixture".into(),
                config_dir: "/tmp/captured-profile".into(),
                host_plugin_config_dir: None,
                state_dir: "/tmp/captured-state".into(),
                herdr_socket: "/tmp/captured-host.sock".into(),
                running: ExecutableIdentity {
                    path: "/tmp/captured-brew".into(),
                    sha256: "a".repeat(64),
                },
                running_origin: None,
                assets_override: None,
                environment: BTreeMap::new(),
                locale: "en".into(),
            },
            &original,
            &release,
            UpdatePolicy::StableOnly,
        )
        .unwrap()
        .unwrap();
        assert_eq!(proposal.origin, original);
        assert_eq!(proposal.release, release);
        let stale = serde_json::json!({
            "versions": {"stable": "0.3.3"},
            "urls": {"stable": {"url": "https://example.invalid/stale", "checksum": "0"}}
        });
        assert!(brew_matches_release(&stale, &proposal.release).is_err());
        let metadata = serde_json::json!({
            "versions": {"stable": "0.3.4"},
            "urls": {"stable": {"url": release.archive_url, "checksum": release.sha256}}
        });
        assert!(brew_matches_release(&metadata, &release).is_ok());
        for (key, value) in [
            ("/versions/stable", "0.3.3"),
            ("/urls/stable/url", "https://example.com/other.tar.gz"),
            ("/urls/stable/checksum", "0"),
        ] {
            let mut changed = metadata.clone();
            *changed.pointer_mut(key).unwrap() = Value::String(value.into());
            assert!(brew_matches_release(&changed, &release).is_err(), "{key}");
        }
    }

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
            let snapshot = check(&context, UpdatePolicy::StableOnly).unwrap();
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
        assert_eq!(
            check(&context, UpdatePolicy::StableOnly).unwrap().status,
            CheckStatus::Unsupported
        );
        fs::write(&binary, b"#!/bin/sh\nexit 1\n").unwrap();
        let installed = executable_identity(&binary).unwrap();
        assert_ne!(installed.sha256, context.running.sha256);
        // Unsigned/unsupported replacement is rejected instead of assuming this daemon
        // is already current just because its former locator now points to new bytes.
        assert!(check(&context, UpdatePolicy::StableOnly).is_err());
        assert_eq!(context.running, running);
        context.running_origin = Some(Box::new(InstallOrigin::Unknown {
            reason: "no owner".into(),
        }));
        assert_eq!(
            check(&context, UpdatePolicy::StableOnly).unwrap().status,
            CheckStatus::Unsupported
        );
    }
    fn registry_fixture(
        initial_version: &str,
    ) -> (tempfile::TempDir, UpdateContext, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let xdg = base.join("xdg");
        let host_config = xdg.join("herdr-dev");
        let injected = host_config.join("plugins/config/desktop-pet");
        let checkout = host_config.join("plugins/github/checkout");
        let plugin = checkout.join("examples/pet");
        fs::create_dir_all(&injected).unwrap();
        fs::create_dir_all(&plugin).unwrap();
        fs::create_dir_all(base.join("profile")).unwrap();
        fs::create_dir_all(base.join("state")).unwrap();
        fs::write(
            plugin.join("herdr-plugin.toml"),
            format!("id = 'desktop-pet'\nversion = '{initial_version}'\nplatforms = ['macos']\n"),
        )
        .unwrap();
        let original = app_for_version(&plugin, initial_version).unwrap();
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&original, fs::Permissions::from_mode(0o700)).unwrap();
        let host = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let registry = host_config.join("plugins.json");
        fs::write(
            &registry,
            serde_json::to_vec(&serde_json::json!([{
                "plugin_id": "desktop-pet", "plugin_root": plugin,
                "manifest_path": plugin.join("herdr-plugin.toml"),
                "enabled": true, "source": {
                    "kind": "github", "owner": "owner", "repo": "repo",
                    "subdir": "examples/pet", "managed_path": checkout,
                    "requested_ref": "HEAD",
                    "resolved_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }]))
            .unwrap(),
        )
        .unwrap();
        let mut environment = BTreeMap::new();
        environment.insert("HERDR_BIN_PATH".into(), host.to_string_lossy().into_owned());
        environment.insert("XDG_CONFIG_HOME".into(), xdg.to_string_lossy().into_owned());
        environment.insert("HOME".into(), base.to_string_lossy().into_owned());
        let running = executable_identity(&original).unwrap();
        let mut context = UpdateContext {
            executable: original,
            version: initial_version.into(),
            instance_id: "fixture".into(),
            config_dir: base.join("profile"),
            host_plugin_config_dir: Some(injected),
            state_dir: base.join("state"),
            herdr_socket: base.join("server.sock"),
            running,
            running_origin: None,
            assets_override: None,
            environment,
            locale: "en".into(),
        };
        let captured = detect_origin(&context).unwrap();
        assert!(matches!(captured, InstallOrigin::Herdr { .. }));
        context.running_origin = Some(Box::new(captured));
        (dir, context, registry, plugin)
    }

    fn advance_fixture(registry: &Path, plugin: &Path, next_version: &str) -> PathBuf {
        fs::write(
            plugin.join("herdr-plugin.toml"),
            format!("id = 'desktop-pet'\nversion = '{next_version}'\nplatforms = ['macos']\n"),
        )
        .unwrap();
        let installed = app_for_version(plugin, next_version).unwrap();
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        fs::write(&installed, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o700)).unwrap();
        let mut entries: Value = serde_json::from_slice(&fs::read(registry).unwrap()).unwrap();
        entries[0]["source"]["resolved_commit"] = Value::String("b".repeat(40));
        fs::write(registry, serde_json::to_vec(&entries).unwrap()).unwrap();
        installed
    }

    #[test]
    fn captured_managed_channel_switch_keeps_a_and_installed_registration_proves_b() {
        for (initial, next) in [("0.3.3", "0.3.4-beta.2"), ("0.3.4-beta.2", "0.3.4")] {
            let (_dir, context, registry, plugin) = registry_fixture(initial);
            let original = context.running_origin.as_deref().unwrap().clone();
            let installed = advance_fixture(&registry, &plugin, next);
            for remove_old in [false, true] {
                if remove_old {
                    fs::remove_file(&context.running.path).unwrap();
                }
                assert_eq!(detect_origin(&context).unwrap(), original);
                let actual = registered_with_host_at(
                    &context,
                    false,
                    RegistrationImage::Installed(&installed),
                )
                .unwrap()
                .unwrap();
                assert!(
                    matches!(&actual, InstallOrigin::Herdr { resolved_commit, .. }
                    if resolved_commit.as_deref() == Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
                );
                assert!(matches_installed_origin(&actual, &original, false, false));
                assert!(!matches_installed_origin(&actual, &original, true, false));
                assert!(!matches_installed_origin(&actual, &original, false, true));
                assert!(matches_installed_origin(&actual, &actual, false, true));
            }
        }
    }

    #[test]
    fn installed_immutable_ref_and_explicit_target_require_exact_revision() {
        let (_dir, context, registry, plugin) = registry_fixture("0.3.3");
        let installed = advance_fixture(&registry, &plugin, "0.3.4-beta.2");
        let actual =
            registered_with_host_at(&context, false, RegistrationImage::Installed(&installed))
                .unwrap()
                .unwrap();
        let mut immutable = context.running_origin.as_deref().unwrap().clone();
        let mut pinned_actual = actual.clone();
        if let InstallOrigin::Herdr { requested_ref, .. } = &mut pinned_actual {
            *requested_ref = Some("a".repeat(40));
        }
        if let InstallOrigin::Herdr { requested_ref, .. } = &mut immutable {
            *requested_ref = Some("a".repeat(40));
        }
        assert!(!matches_installed_origin(
            &pinned_actual,
            &immutable,
            false,
            false
        ));
        let mut target = actual.clone();
        if let InstallOrigin::Herdr {
            resolved_commit, ..
        } = &mut target
        {
            *resolved_commit = Some("c".repeat(40));
        }
        assert!(!matches_installed_origin(&actual, &target, false, true));
        assert!(!matches_installed_origin(&actual, &target, true, true));
    }

    #[test]
    fn captured_managed_registration_rejects_changed_scope_and_escaping_installed_bundle() {
        let (dir, context, registry, plugin) = registry_fixture("0.3.3");
        let installed = advance_fixture(&registry, &plugin, "0.3.4-beta.2");
        let original = fs::read(&registry).unwrap();
        for (field, replacement) in [
            ("/0/source/requested_ref", serde_json::json!("other")),
            ("/0/source/owner", serde_json::json!("other")),
            (
                "/0/source/managed_path",
                serde_json::json!(dir.path().join("other")),
            ),
            ("/0/enabled", serde_json::json!(false)),
            (
                "/0/plugin_root",
                serde_json::json!(dir.path().join("other")),
            ),
        ] {
            let mut changed: Value = serde_json::from_slice(&original).unwrap();
            *changed.pointer_mut(field).unwrap() = replacement;
            fs::write(&registry, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(
                matches!(
                    detect_origin(&context).unwrap(),
                    InstallOrigin::Unknown { .. }
                ),
                "{field}"
            );
        }
        fs::write(&registry, &original).unwrap();
        let mut changed_host = context.clone();
        changed_host.environment.insert(
            "HERDR_BIN_PATH".into(),
            dir.path().join("other-host").to_string_lossy().into_owned(),
        );
        assert!(matches!(
            detect_origin(&changed_host).unwrap(),
            InstallOrigin::Unknown { .. }
        ));
        let mut changed_config = context.clone();
        let alternate = dir.path().join("alternate-profile");
        fs::create_dir_all(&alternate).unwrap();
        changed_config.host_plugin_config_dir = Some(alternate);
        assert!(matches!(
            detect_origin(&changed_config).unwrap(),
            InstallOrigin::Unknown { .. }
        ));
        for revision in ["invalid", ""] {
            let mut changed: Value = serde_json::from_slice(&original).unwrap();
            changed[0]["source"]["resolved_commit"] = Value::String(revision.into());
            fs::write(&registry, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(registered_with_host_at(
                &context,
                false,
                RegistrationImage::Installed(&installed),
            )
            .is_err());
        }
        let mut changed: Value = serde_json::from_slice(&original).unwrap();
        changed[0]["source"]["resolved_commit"] = Value::Null;
        fs::write(&registry, serde_json::to_vec(&changed).unwrap()).unwrap();
        let absent_revision =
            registered_with_host_at(&context, false, RegistrationImage::Installed(&installed))
                .unwrap()
                .unwrap();
        assert!(!matches_installed_origin(
            &absent_revision,
            context.running_origin.as_deref().unwrap(),
            false,
            false
        ));
        fs::write(&registry, &original).unwrap();
        let outside = dir.path().join("outside");
        fs::rename(&installed, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &installed).unwrap();
        assert!(
            registered_with_host_at(&context, false, RegistrationImage::Installed(&installed),)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            detect_origin(&context).unwrap(),
            context.running_origin.as_deref().unwrap().clone()
        );
        let mut wrong_channel = context.clone();
        wrong_channel.version = "0.3.4-beta.2".into();
        assert!(matches!(
            detect_origin(&wrong_channel).unwrap(),
            InstallOrigin::Unknown { .. }
        ));
    }

    #[test]
    fn captured_local_channel_switch_survives_deleted_original_only_inside_physical_root() {
        for (initial, next) in [("0.3.3", "0.3.4-beta.2"), ("0.3.4-beta.2", "0.3.4")] {
            let (dir, mut context, _registry, _plugin) = registry_fixture(initial);
            let local = fs::canonicalize(dir.path()).unwrap().join("local");
            let old = app_for_version(&local, initial).unwrap();
            fs::create_dir_all(old.parent().unwrap()).unwrap();
            fs::create_dir_all(local.join("scripts")).unwrap();
            fs::write(local.join("scripts/install.sh"), b"#!/bin/sh\n").unwrap();
            fs::write(
                local.join("herdr-plugin.toml"),
                format!("id = 'desktop-pet'\nversion = '{next}'\nplatforms = ['macos']\n"),
            )
            .unwrap();
            fs::write(&old, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&old, fs::Permissions::from_mode(0o700)).unwrap();
            context.executable = old.clone();
            context.running = executable_identity(&old).unwrap();
            context.version = initial.into();
            context.running_origin = Some(Box::new(InstallOrigin::Local {
                root: local.clone(),
            }));
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Local { .. }
            ));
            fs::remove_file(old).unwrap();
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Local { .. }
            ));
            let scripts = dir.path().join("outside-scripts");
            fs::rename(local.join("scripts"), &scripts).unwrap();
            std::os::unix::fs::symlink(&scripts, local.join("scripts")).unwrap();
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Unknown { .. }
            ));
            fs::remove_file(local.join("scripts")).unwrap();
            fs::rename(scripts, local.join("scripts")).unwrap();
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Local { .. }
            ));
            context.running.path = app_for_version(&local, next).unwrap();
            assert!(matches!(
                detect_origin(&context).unwrap(),
                InstallOrigin::Unknown { .. }
            ));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn prospective_host_requirement_uses_connected_process_version_not_disk_binary() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        let (_dir, mut context, _registry, _plugin) = registry_fixture("0.3.3");
        let mut listener = UnixListener::bind(&context.herdr_socket).unwrap();
        let host = PathBuf::from(context.environment.get("HERDR_BIN_PATH").unwrap());
        context.environment.insert(
            "HERDR_PLUGIN_CONFIG_DIR".into(),
            context.config_dir.to_string_lossy().into_owned(),
        );
        context.environment.insert(
            "HERDR_PLUGIN_STATE_DIR".into(),
            context.state_dir.to_string_lossy().into_owned(),
        );
        for (live, requirement, compatible) in [
            ("0.9.3", Some("0.9.3"), true),
            ("0.9.3", Some("0.9.4"), false),
            ("0.9.4", Some("0.9.4"), true),
            ("0.9.3", Some("invalid"), false),
            ("0.9.3", None, false),
        ] {
            let required = requirement.map_or(String::new(), |value| {
                format!("min_herdr_version = '{value}'\n")
            });
            let doc: toml::Value =
                format!("id = 'desktop-pet'\nversion = '0.3.4'\nplatforms = ['macos']\n{required}")
                    .parse()
                    .unwrap();
            let responder = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 256];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(stream, "{{\"id\":\"updater-host-proof\",\"result\":{{\"snapshot\":{{\"protocol\":1,\"version\":\"{live}\"}}}}}}\n").unwrap();
                listener
            });
            assert_eq!(check_host(&context, &host, &doc).is_ok(), compatible);
            listener = responder.join().unwrap();
        }
        drop(listener);
        fs::remove_file(&context.herdr_socket).unwrap();
        let supported: toml::Value =
            "id = 'desktop-pet'\nversion = '0.3.4'\nplatforms = ['macos']\nmin_herdr_version = '0.9.3'\n"
                .parse().unwrap();
        assert!(check_host(&context, &host, &supported).is_err());
    }
}
