use crate::socket;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PLUGIN_ID: &str = "desktop-pet";
pub const LIFECYCLE_FILE: &str = "lifecycle.json";
pub const CONTROL_SOCKET_FILE: &str = "control.sock";
pub const LOCK_FILE: &str = "desktop-pet.lock";
pub const LOG_FILE: &str = "desktop-pet.log";

const CONFIG_ROOT_NAME: &str = "herdr";
const MAX_PERSISTED_ENDPOINTS: usize = 64;
const LOCK_MODE: u32 = 0o600;
const STARTUP_KEY: &str = "startup";
const STARTUP_STALE_AFTER_SECS: u64 = 45;

/// Settings writes are serialized by the caller's lifecycle lock or daemon writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LifecycleSettings {
    pub auto_start: bool,
    pub exit_with_herdr: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleSetting {
    AutoStart,
    ExitWithHerdr,
}

impl std::str::FromStr for LifecycleSetting {
    type Err = String;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        match key {
            "auto_start" => Ok(Self::AutoStart),
            "exit_with_herdr" => Ok(Self::ExitWithHerdr),
            _ => Err(format!("unknown lifecycle setting: {key}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartupLease {
    pub token: String,
}
const DIRECTORY_MODE: u32 = 0o700;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub control_socket: PathBuf,
    pub log_file: PathBuf,
}

impl Paths {
    pub fn resolve(
        explicit_config: Option<&Path>,
        explicit_state: Option<&Path>,
    ) -> Result<Self, String> {
        let config_dir = resolve_dir(
            "HERDR_PLUGIN_CONFIG_DIR",
            explicit_config,
            plugin_config_dir,
        )?;
        let state_dir = resolve_dir("HERDR_PLUGIN_STATE_DIR", explicit_state, plugin_state_dir)?;
        Ok(Self {
            control_socket: state_dir.join(CONTROL_SOCKET_FILE),
            log_file: state_dir.join(LOG_FILE),
            config_dir,
            state_dir,
        })
    }

    pub fn export_environment(&self) {
        // These are process-local overrides.  The daemon receives them explicitly
        // from its launcher and never falls back to OMP_* compatibility variables.
        env::set_var("HERDR_PLUGIN_CONFIG_DIR", &self.config_dir);
        env::set_var("HERDR_PLUGIN_STATE_DIR", &self.state_dir);
    }
}

/// Return the plugin's Herdr-managed config directory.
///
/// Herdr injects this variable for installed plugins.  Standalone invocation
/// derives the same path under the active Herdr config root without invoking a
/// command or consulting any legacy OMPet variables.
pub(crate) fn config_directory() -> Result<PathBuf, String> {
    resolve_dir("HERDR_PLUGIN_CONFIG_DIR", None, plugin_config_dir)
}

fn resolve_dir(
    variable: &str,
    explicit: Option<&Path>,
    fallback: fn() -> PathBuf,
) -> Result<PathBuf, String> {
    let path = match env::var_os(variable) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        Some(_) => return Err(format!("{variable} is set to an empty path")),
        None => explicit.map(Path::to_path_buf).unwrap_or_else(fallback),
    };
    validate_directory(&path, true)
}

fn config_root() -> PathBuf {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(root).join(CONFIG_ROOT_NAME);
    }
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join(CONFIG_ROOT_NAME))
        .unwrap_or_else(|| env::temp_dir().join(CONFIG_ROOT_NAME))
}

fn state_root() -> PathBuf {
    if let Some(root) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(root).join(CONFIG_ROOT_NAME);
    }
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".local").join("state").join(CONFIG_ROOT_NAME))
        .unwrap_or_else(|| env::temp_dir().join(format!("{CONFIG_ROOT_NAME}-state")))
}

fn plugin_config_dir() -> PathBuf {
    config_root().join("plugins").join("config").join(PLUGIN_ID)
}

fn plugin_state_dir() -> PathBuf {
    state_root().join("plugins").join(PLUGIN_ID)
}

pub fn default_herdr_socket() -> PathBuf {
    if let Some(value) = env::var_os("HERDR_SOCKET_PATH").filter(|value| !value.is_empty()) {
        return PathBuf::from(value);
    }
    config_root().join("herdr.sock")
}

pub fn validate_directory(path: &Path, create: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("directory path is empty".to_owned());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "directory {} must not be a symlink",
                    path.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!("path {} is not a directory", path.display()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
            fs::create_dir_all(path)
                .map_err(|error| format!("cannot create directory {}: {error}", path.display()))?;
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect directory {}: {error}",
                path.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "path {} is not a regular directory",
            path.display()
        ));
    }
    if metadata.uid() != effective_uid() {
        return Err(format!(
            "directory {} is not owned by the current user",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(path, Permissions::from_mode(DIRECTORY_MODE)).map_err(|error| {
            format!(
                "directory {} is not private and cannot be secured: {error}",
                path.display()
            )
        })?;
    }
    let secured = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if secured.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "directory {} is not private to the current user",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

#[derive(Debug)]
pub struct LifecycleLock {
    file: File,
}

impl LifecycleLock {
    pub fn acquire(state_dir: &Path) -> Result<Self, String> {
        let path = state_dir.join(LOCK_FILE);
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "lifecycle lock {} must not be a symlink",
                    path.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "lifecycle lock {} is not a regular file",
                    path.display()
                ));
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(LOCK_MODE)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|error| format!("cannot open lifecycle lock {}: {error}", path.display()))?;
        let metadata = file.metadata().map_err(|error| {
            format!("cannot inspect lifecycle lock {}: {error}", path.display())
        })?;
        if metadata.uid() != effective_uid() {
            return Err(format!(
                "lifecycle lock {} is not owned by the current user",
                path.display()
            ));
        }
        fs::set_permissions(&path, Permissions::from_mode(LOCK_MODE))
            .map_err(|error| format!("cannot secure lifecycle lock {}: {error}", path.display()))?;
        if file
            .metadata()
            .map_err(|error| error.to_string())?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(format!("lifecycle lock {} is not private", path.display()));
        }
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN)
            {
                return Err("another desktop-pet lifecycle operation is in progress".to_owned());
            }
            return Err(format!(
                "cannot lock lifecycle guard {}: {error}",
                path.display()
            ));
        }
        Ok(Self { file })
    }

    pub fn try_acquire(state_dir: &Path) -> Result<Option<Self>, String> {
        match Self::acquire(state_dir) {
            Ok(lock) => Ok(Some(lock)),
            Err(error) if error.contains("lifecycle operation is in progress") => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for LifecycleLock {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn read_object_with_presence(config_dir: &Path) -> Result<(Map<String, Value>, bool), String> {
    let path = config_dir.join(LIFECYCLE_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Map::new(), false)),
        Err(error) => {
            return Err(format!(
                "cannot inspect lifecycle state {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "lifecycle state {} is not a regular file",
            path.display()
        ));
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read lifecycle state {}: {error}", path.display()))?;
    match serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("invalid lifecycle state {}: {error}", path.display()))?
    {
        Value::Object(object) => Ok((object, true)),
        _ => Err(format!(
            "invalid lifecycle state {}: expected an object",
            path.display()
        )),
    }
}

fn read_object(config_dir: &Path) -> Result<Map<String, Value>, String> {
    read_object_with_presence(config_dir).map(|(object, _)| object)
}

fn setting_bool(
    config_dir: &Path,
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    match object.get(key) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!(
            "invalid lifecycle state {}: {key} must be boolean",
            config_dir.join(LIFECYCLE_FILE).display()
        )),
        None => Ok(None),
    }
}

fn settings_from_object(
    config_dir: &Path,
    object: &Map<String, Value>,
    exists: bool,
) -> Result<LifecycleSettings, String> {
    // Validate legacy data even if its canonical replacement is already present.
    let legacy = setting_bool(config_dir, object, "enabled")?;
    let auto_start = setting_bool(config_dir, object, "auto_start")?;
    let exit_with_herdr = setting_bool(config_dir, object, "exit_with_herdr")?;
    Ok(LifecycleSettings {
        auto_start: auto_start.or(legacy).unwrap_or(true),
        exit_with_herdr: exit_with_herdr.unwrap_or(!exists),
    })
}

fn put_settings(object: &mut Map<String, Value>, settings: LifecycleSettings) {
    object.remove("enabled");
    object.insert("auto_start".to_owned(), Value::Bool(settings.auto_start));
    object.insert(
        "exit_with_herdr".to_owned(),
        Value::Bool(settings.exit_with_herdr),
    );
}

fn store_settings(
    config_dir: &Path,
    mut object: Map<String, Value>,
    settings: LifecycleSettings,
) -> Result<(), String> {
    put_settings(&mut object, settings);
    write_object(config_dir, object)
}

pub(crate) fn read_settings(config_dir: &Path) -> Result<LifecycleSettings, String> {
    let (object, exists) = read_object_with_presence(config_dir)?;
    settings_from_object(config_dir, &object, exists)
}

pub(crate) fn write_settings(config_dir: &Path, settings: LifecycleSettings) -> Result<(), String> {
    let (object, exists) = read_object_with_presence(config_dir)?;
    settings_from_object(config_dir, &object, exists)?;
    store_settings(config_dir, object, settings)
}

pub(crate) fn update_setting(
    config_dir: &Path,
    key: LifecycleSetting,
    value: bool,
) -> Result<LifecycleSettings, String> {
    let (object, exists) = read_object_with_presence(config_dir)?;
    let mut settings = settings_from_object(config_dir, &object, exists)?;
    match key {
        LifecycleSetting::AutoStart => settings.auto_start = value,
        LifecycleSetting::ExitWithHerdr => settings.exit_with_herdr = value,
    }
    store_settings(config_dir, object, settings)?;
    Ok(settings)
}

pub fn read_endpoints(config_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let object = read_object(config_dir)?;
    let Some(value) = object.get("endpoints") else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        format!(
            "invalid lifecycle state {}: endpoints must be an array",
            config_dir.join(LIFECYCLE_FILE).display()
        )
    })?;
    if values.len() > MAX_PERSISTED_ENDPOINTS {
        return Err(format!(
            "invalid lifecycle state {}: too many endpoints",
            config_dir.join(LIFECYCLE_FILE).display()
        ));
    }
    let mut endpoints = Vec::with_capacity(values.len());
    for value in values {
        let endpoint = value.as_str().ok_or_else(|| {
            format!(
                "invalid lifecycle state {}: endpoint must be a string",
                config_dir.join(LIFECYCLE_FILE).display()
            )
        })?;
        endpoints.push(socket::canonical_endpoint(Path::new(endpoint))?);
    }
    endpoints.sort();
    endpoints.dedup();
    Ok(endpoints)
}

pub fn add_endpoint(config_dir: &Path, endpoint: &Path) -> Result<bool, String> {
    let endpoint = socket::canonical_endpoint(endpoint)?;
    validate_directory(config_dir, true)?;
    let (mut object, exists) = read_object_with_presence(config_dir)?;
    let mut endpoints = read_endpoints(config_dir)?;
    if endpoints.iter().any(|known| known == &endpoint) {
        return Ok(false);
    }
    if endpoints.len() >= MAX_PERSISTED_ENDPOINTS {
        return Err("cannot persist more than 64 Herdr endpoints".to_owned());
    }
    endpoints.push(endpoint);
    endpoints.sort();
    endpoints.dedup();
    if !exists {
        put_settings(
            &mut object,
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            },
        );
    }
    object.insert(
        "endpoints".to_owned(),
        Value::Array(
            endpoints
                .iter()
                .map(|path| Value::String(path.display().to_string()))
                .collect(),
        ),
    );
    let path = config_dir.join(LIFECYCLE_FILE);
    let bytes = serde_json::to_vec_pretty(&Value::Object(object))
        .map_err(|error| format!("cannot encode lifecycle state: {error}"))?;
    atomic_write(&path, &bytes)?;
    Ok(true)
}

/// Reserve a durable startup handoff before spawning the detached daemon.
///
/// The normal lifecycle lock serializes writers, but it cannot be held across
/// `Command::spawn`: the child must acquire that lock itself.  This marker is
/// the durable ownership handoff that prevents a second ensure invocation from
/// spawning another child during that gap.
pub fn begin_startup(config_dir: &Path) -> Result<Option<StartupLease>, String> {
    validate_directory(config_dir, true)?;
    let (mut object, exists) = read_object_with_presence(config_dir)?;
    if let Some(value) = object.get(STARTUP_KEY).cloned() {
        let marker = value.as_object().ok_or_else(|| {
            format!(
                "invalid lifecycle state {}: startup must be an object",
                config_dir.join(LIFECYCLE_FILE).display()
            )
        })?;
        let pid = marker
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| {
                format!(
                    "invalid lifecycle state {}: startup pid is invalid",
                    config_dir.join(LIFECYCLE_FILE).display()
                )
            })?;
        let started = marker
            .get("started_unix_secs")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!(
                    "invalid lifecycle state {}: startup timestamp is invalid",
                    config_dir.join(LIFECYCLE_FILE).display()
                )
            })?;
        let now = unix_seconds();
        let claimed = marker
            .get("claimed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if now.saturating_sub(started) <= STARTUP_STALE_AFTER_SECS
            && ((claimed && pid_alive(pid) && pid != std::process::id())
                || (!claimed && unclaimed_owner_alive(marker, pid)))
        {
            return Ok(None);
        }
        // A dead launcher or an abandoned same-process attempt cannot own the
        // next startup.  Replacing its marker is safe under LifecycleLock.
        object.remove(STARTUP_KEY);
    }
    let settings = settings_from_object(config_dir, &object, exists)?;
    put_settings(&mut object, settings);
    let token = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut marker = Map::new();
    marker.insert(
        "pid".to_owned(),
        Value::Number(u64::from(std::process::id()).into()),
    );
    marker.insert("token".to_owned(), Value::String(token.clone()));
    marker.insert(
        "started_unix_secs".to_owned(),
        Value::Number(unix_seconds().into()),
    );
    object.insert(STARTUP_KEY.to_owned(), Value::Object(marker));
    write_object(config_dir, object)?;
    Ok(Some(StartupLease { token }))
}

/// Complete the daemon side of a startup handoff while retaining the marker
/// until the private control listener is bound.  Keeping the child pid in the
/// marker closes the gap where a second launcher could otherwise race startup.
pub fn claim_startup(config_dir: &Path, token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("desktop-pet startup handoff token is empty".to_owned());
    }
    let mut object = read_object(config_dir)?;
    let Some(value) = object.get_mut(STARTUP_KEY) else {
        return Err("desktop-pet startup handoff was cancelled".to_owned());
    };
    let marker = value.as_object_mut().ok_or_else(|| {
        format!(
            "invalid lifecycle state {}: startup must be an object",
            config_dir.join(LIFECYCLE_FILE).display()
        )
    })?;
    if marker.get("token").and_then(Value::as_str) != Some(token) {
        return Err("desktop-pet startup handoff token does not match".to_owned());
    }
    marker.insert(
        "pid".to_owned(),
        Value::Number(u64::from(std::process::id()).into()),
    );
    marker.insert("claimed".to_owned(), Value::Bool(true));
    write_object(config_dir, object)
}

/// Record the child pid while the launcher still owns the lifecycle lock.
/// This keeps stop/finalization able to distinguish a spawned child from an
/// abandoned launcher even before the child claims the handoff.
pub fn set_startup_child(config_dir: &Path, token: &str, pid: u32) -> Result<(), String> {
    if token.is_empty() || pid == 0 {
        return Err("desktop-pet startup child is invalid".to_owned());
    }
    let mut object = read_object(config_dir)?;
    let Some(value) = object.get_mut(STARTUP_KEY) else {
        return Err("desktop-pet startup handoff was cancelled".to_owned());
    };
    let marker = value.as_object_mut().ok_or_else(|| {
        format!(
            "invalid lifecycle state {}: startup must be an object",
            config_dir.join(LIFECYCLE_FILE).display()
        )
    })?;
    if marker.get("token").and_then(Value::as_str) != Some(token) {
        return Err("desktop-pet startup handoff token does not match".to_owned());
    }
    marker.insert("child_pid".to_owned(), Value::Number(u64::from(pid).into()));
    write_object(config_dir, object)
}

/// Remove a claimed handoff after the control listener is accepting requests.
pub fn finish_startup(config_dir: &Path, token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("desktop-pet startup handoff token is empty".to_owned());
    }
    let mut object = read_object(config_dir)?;
    let Some(value) = object.get(STARTUP_KEY) else {
        return Err("desktop-pet startup handoff was cancelled".to_owned());
    };
    let marker = value.as_object().ok_or_else(|| {
        format!(
            "invalid lifecycle state {}: startup must be an object",
            config_dir.join(LIFECYCLE_FILE).display()
        )
    })?;
    if marker.get("token").and_then(Value::as_str) != Some(token) {
        return Err("desktop-pet startup handoff token does not match".to_owned());
    }
    object.remove(STARTUP_KEY);
    write_object(config_dir, object)
}

/// Abandon a launch reservation after spawning fails or a bounded readiness
/// wait proves that this exact child never claimed the handoff.
pub fn abandon_startup(config_dir: &Path, token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Ok(());
    }
    let mut object = read_object(config_dir)?;
    let remove = object
        .get(STARTUP_KEY)
        .and_then(Value::as_object)
        .and_then(|marker| marker.get("token"))
        .and_then(Value::as_str)
        == Some(token);
    if remove {
        object.remove(STARTUP_KEY);
        write_object(config_dir, object)?;
    }
    Ok(())
}

/// Return whether a non-expired launcher or daemon still owns a startup
/// handoff.  Callers use this while waiting for the lifecycle lock to become
/// quiescent; stale markers are reclaimed by `begin_startup`.
pub fn startup_pending(config_dir: &Path) -> Result<bool, String> {
    let object = read_object(config_dir)?;
    let Some(marker) = object.get(STARTUP_KEY).and_then(Value::as_object) else {
        return Ok(false);
    };
    let pid = marker
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .unwrap_or(0);
    let started = marker
        .get("started_unix_secs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let claimed = marker
        .get("claimed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let age_valid = unix_seconds().saturating_sub(started) <= STARTUP_STALE_AFTER_SECS;
    Ok(age_valid
        && if claimed {
            pid_alive(pid)
        } else {
            unclaimed_owner_alive(marker, pid)
        })
}

/// An unclaimed reservation stays owned while its recorded child (the only
/// process that can claim it) lives, or, before a child is recorded, while
/// its launcher lives.
fn unclaimed_owner_alive(marker: &Map<String, Value>, launcher: u32) -> bool {
    match marker
        .get("child_pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    {
        Some(child) => pid_alive(child),
        None => pid_alive(launcher),
    }
}

/// Cancel a pending launch while holding the lifecycle lock. The child must
/// reject a missing handoff token before binding its control listener.
pub fn cancel_startup(config_dir: &Path) -> Result<(), String> {
    let mut object = read_object(config_dir)?;
    if object.remove(STARTUP_KEY).is_some() {
        write_object(config_dir, object)?;
    }
    Ok(())
}

fn write_object(config_dir: &Path, object: Map<String, Value>) -> Result<(), String> {
    validate_directory(config_dir, true)?;
    let bytes = serde_json::to_vec_pretty(&Value::Object(object))
        .map_err(|error| format!("cannot encode lifecycle state: {error}"))?;
    atomic_write(&config_dir.join(LIFECYCLE_FILE), &bytes)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("lifecycle path {} has no parent", path.display()))?;
    validate_directory(parent, true)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        nonce
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(LOCK_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)
        .map_err(|error| {
            format!(
                "cannot create lifecycle temporary file {}: {error}",
                temp.display()
            )
        })?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("cannot write lifecycle state {}: {error}", temp.display()))?;
        file.sync_all()
            .map_err(|error| format!("cannot flush lifecycle state {}: {error}", temp.display()))?;
        fs::rename(&temp, path).map_err(|error| {
            format!("cannot replace lifecycle state {}: {error}", path.display())
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = env::temp_dir().join(format!(
                "desktop-pet-lifecycle-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("unique fixture");
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn state(&self) -> PathBuf {
            self.0.join(LIFECYCLE_FILE)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove fixture");
        }
    }

    #[test]
    fn migrated_false_remains_false_when_changing_other_setting_and_preserves_lease() {
        let fixture = Fixture::new();
        fs::write(
            fixture.state(),
            r#"{"enabled":false,"endpoints":["/tmp/herdr.sock"],"other":{"retained":9}}"#,
        )
        .expect("legacy profile");
        let lease = begin_startup(fixture.path())
            .expect("reserve")
            .expect("lease");
        assert_eq!(
            read_settings(fixture.path()).expect("legacy read"),
            LifecycleSettings {
                auto_start: false,
                exit_with_herdr: false,
            }
        );
        assert_eq!(
            update_setting(fixture.path(), LifecycleSetting::ExitWithHerdr, true).expect("update"),
            LifecycleSettings {
                auto_start: false,
                exit_with_herdr: true,
            }
        );
        let object = read_object(fixture.path()).expect("persisted state");
        assert!(!object.contains_key("enabled"));
        assert_eq!(object["auto_start"], false);
        assert_eq!(object["exit_with_herdr"], true);
        assert_eq!(object["endpoints"], serde_json::json!(["/tmp/herdr.sock"]));
        assert_eq!(object["other"], serde_json::json!({"retained":9}));
        assert_eq!(
            object["startup"]["token"].as_str(),
            Some(lease.token.as_str())
        );
        claim_startup(fixture.path(), &lease.token).expect("lease survives update");
        finish_startup(fixture.path(), &lease.token).expect("finish lease");
        assert!(!startup_pending(fixture.path()).expect("lease released"));
    }

    #[test]
    fn fresh_and_existing_files_migrate_with_distinct_exit_policy() {
        let fixture = Fixture::new();
        assert_eq!(
            read_settings(fixture.path()).expect("fresh read"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            }
        );
        assert!(
            !fixture.state().exists(),
            "reads do not create a state file"
        );
        fs::write(fixture.state(), "{}").expect("empty existing profile");
        assert_eq!(
            read_settings(fixture.path()).expect("empty existing read"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: false,
            }
        );
        fs::write(fixture.state(), r#"{"other":1}"#).expect("existing profile");
        assert_eq!(
            read_settings(fixture.path()).expect("existing read"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: false,
            }
        );
        write_settings(
            fixture.path(),
            LifecycleSettings {
                auto_start: false,
                exit_with_herdr: false,
            },
        )
        .expect("canonical write");
        assert_eq!(read_object(fixture.path()).expect("preserved")["other"], 1);
        assert_eq!(
            update_setting(fixture.path(), LifecycleSetting::AutoStart, true).expect("set"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: false,
            }
        );
    }

    #[test]
    fn fresh_startup_and_endpoint_writes_preserve_new_profile_policy() {
        let startup = Fixture::new();
        let lease = begin_startup(startup.path())
            .expect("reserve fresh profile")
            .expect("lease");
        assert_eq!(
            read_settings(startup.path()).expect("settings after startup"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            }
        );
        assert!(startup_pending(startup.path()).expect("lease remains valid"));
        finish_startup(startup.path(), &lease.token).expect("finish lease");

        let endpoint = Fixture::new();
        let address = endpoint.path().join("herdr.sock");
        assert!(add_endpoint(endpoint.path(), &address).expect("new endpoint"));
        assert_eq!(
            read_settings(endpoint.path()).expect("settings after endpoint"),
            LifecycleSettings {
                auto_start: true,
                exit_with_herdr: true,
            }
        );
        assert_eq!(
            read_endpoints(endpoint.path()).expect("endpoint retained"),
            vec![socket::canonical_endpoint(&address).expect("canonical endpoint")]
        );
    }

    #[test]
    fn canonical_precedence_does_not_hide_malformed_legacy_data() {
        let fixture = Fixture::new();
        fs::write(
            fixture.state(),
            r#"{"auto_start":false,"enabled":true,"exit_with_herdr":true}"#,
        )
        .expect("canonical profile");
        assert!(
            !read_settings(fixture.path())
                .expect("canonical precedence")
                .auto_start
        );
        for malformed in [
            r#"{"enabled":"false","auto_start":false}"#,
            r#"{"enabled":false,"auto_start":0}"#,
            r#"{"enabled":false,"exit_with_herdr":null}"#,
            r#"{"enabled":false"#,
        ] {
            fs::write(fixture.state(), malformed).expect("malformed fixture");
            assert!(read_settings(fixture.path()).is_err());
            assert!(update_setting(fixture.path(), LifecycleSetting::AutoStart, true).is_err());
            assert!(write_settings(
                fixture.path(),
                LifecycleSettings {
                    auto_start: true,
                    exit_with_herdr: true,
                },
            )
            .is_err());
            assert_eq!(
                fs::read_to_string(fixture.state()).expect("untouched old file"),
                malformed
            );
        }
    }

    #[test]
    fn setting_keys_accept_only_canonical_names() {
        assert_eq!(
            "auto_start"
                .parse::<LifecycleSetting>()
                .expect("auto start"),
            LifecycleSetting::AutoStart
        );
        assert_eq!(
            "exit_with_herdr"
                .parse::<LifecycleSetting>()
                .expect("exit policy"),
            LifecycleSetting::ExitWithHerdr
        );
        for key in ["enabled", "AutoStart", "exit-with-herdr", "auto_start "] {
            assert!(key.parse::<LifecycleSetting>().is_err());
        }
    }

    #[test]
    fn startup_handoff_remains_owned_until_control_is_bound() {
        let fixture = Fixture::new();
        let lease = begin_startup(fixture.path())
            .expect("startup reservation")
            .expect("first launcher owns reservation");
        assert!(startup_pending(fixture.path()).expect("pending marker"));
        claim_startup(fixture.path(), &lease.token).expect("child claims reservation");
        assert!(startup_pending(fixture.path()).expect("claimed marker"));
        finish_startup(fixture.path(), &lease.token).expect("control listener bound");
        assert!(!startup_pending(fixture.path()).expect("finished marker"));
    }

    #[test]
    fn unclaimed_startup_is_owned_only_while_its_child_or_launcher_lives() {
        use std::process::Command;

        let fixture = Fixture::new();
        let lease = begin_startup(fixture.path())
            .expect("startup reservation")
            .expect("first launcher owns reservation");
        let mut dead = Command::new("/usr/bin/true").spawn().expect("spawn child");
        dead.wait().expect("reap child");
        set_startup_child(fixture.path(), &lease.token, dead.id()).expect("record child");
        assert!(!startup_pending(fixture.path()).expect("dead child marker"));
        let replacement = begin_startup(fixture.path())
            .expect("startup reservation")
            .expect("dead child no longer owns reservation");
        assert_ne!(replacement.token, lease.token);

        let mut live = Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("spawn child");
        set_startup_child(fixture.path(), &replacement.token, live.id()).expect("record child");
        let pending = startup_pending(fixture.path()).expect("live child marker");
        let reserved = begin_startup(fixture.path()).expect("startup reservation");
        let _ = live.kill();
        live.wait().expect("reap child");
        assert!(pending);
        assert!(reserved.is_none());

        fs::write(
            fixture.state(),
            format!(
                r#"{{"startup":{{"pid":{},"token":"orphan","started_unix_secs":{}}}}}"#,
                dead.id(),
                unix_seconds()
            ),
        )
        .expect("orphaned launcher marker");
        assert!(!startup_pending(fixture.path()).expect("dead launcher marker"));
    }

    #[test]
    fn cancelled_startup_cannot_be_claimed_or_published() {
        let fixture = Fixture::new();
        fs::write(fixture.state(), r#"{"enabled":false,"other":7}"#).expect("legacy state");
        let lease = begin_startup(fixture.path())
            .expect("reserve")
            .expect("lease");
        cancel_startup(fixture.path()).expect("cancel");
        assert!(claim_startup(fixture.path(), &lease.token).is_err());
        assert!(set_startup_child(fixture.path(), &lease.token, std::process::id()).is_err());
        assert!(finish_startup(fixture.path(), &lease.token).is_err());
        assert!(!startup_pending(fixture.path()).expect("no pending lease"));
        assert!(
            !read_settings(fixture.path())
                .expect("preserved settings")
                .auto_start
        );
        assert_eq!(
            read_object(fixture.path()).expect("extra field")["other"],
            7
        );
    }
}
