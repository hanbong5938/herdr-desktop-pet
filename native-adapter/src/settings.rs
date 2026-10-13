use crate::socket;
use desktop_pet_contracts::{profile_store, ProfileStore};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::env;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ObservationPreferences {
    #[serde(default = "default_local")]
    pub local: bool,
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub machines: Vec<String>,
}
fn default_local() -> bool { true }
impl Default for ObservationPreferences {
    fn default() -> Self { Self { local: true, remote: false, machines: Vec::new() } }
}
impl ObservationPreferences {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        if self.machines.len() > 64 || self.machines.iter().any(|id| id.is_empty() || id.len() > 4096 || id.contains('\0') || !seen.insert(id)) {
            return Err("machine IDs must be unique nonempty NUL-free strings of at most 4096 bytes; at most 64 may be selected".to_owned());
        }
        Ok(())
    }
    fn sanitize(&mut self) {
        let mut seen = HashSet::new();
        self.machines.retain(|id| !id.is_empty() && id.len() <= 4096 && !id.contains('\0') && seen.len() < 64 && seen.insert(id.clone()));
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MachineStatus { #[default] NotSelected, Connecting, Online, Offline }
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct MachineInfo {
    pub id: String,
    pub label: String,
    pub remote_session: String,
    pub enabled: bool,
    pub status: MachineStatus,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct SourceCatalog { pub initialized: bool, pub machines: Vec<MachineInfo>, pub error: Option<String> }

pub(crate) fn default_herdr_socket() -> PathBuf {
    if let Some(value) = env::var_os("HERDR_SOCKET_PATH").filter(|value| !value.is_empty()) { return PathBuf::from(value); }
    let root = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| env::var_os("HOME").map(PathBuf::from).unwrap_or_else(env::temp_dir).join(".config"));
    root.join("herdr").join("herdr.sock")
}
fn endpoints_from_object(store: &ProfileStore, object: &Map<String, Value>) -> Result<Vec<PathBuf>, String> {
    let Some(value) = object.get("endpoints") else { return Ok(Vec::new()); };
    let values = value.as_array().ok_or_else(|| format!("invalid lifecycle state {}: endpoints must be an array", store.config_dir().join("lifecycle.json").display()))?;
    if values.len() > 64 { return Err("invalid lifecycle state: too many endpoints".to_owned()); }
    let mut endpoints = Vec::with_capacity(values.len());
    for value in values { endpoints.push(socket::canonical_endpoint(Path::new(value.as_str().ok_or("endpoint must be a string")?))?); }
    endpoints.sort(); endpoints.dedup(); Ok(endpoints)
}
pub(crate) fn read_endpoints(store: &ProfileStore) -> Result<Vec<PathBuf>, String> {
    let (object, _) = store.read_object_with_presence()?;
    endpoints_from_object(store, &object)
}
pub(crate) fn add_endpoint(store: &ProfileStore, endpoint: &Path) -> Result<bool, String> {
    let endpoint = socket::canonical_endpoint(endpoint)?;
    store.update(|object, exists| {
        let mut endpoints = endpoints_from_object(store, object)?;
        if endpoints.contains(&endpoint) { return Ok(false); }
        if endpoints.len() >= 64 { return Err("cannot persist more than 64 Herdr endpoints".to_owned()); }
        endpoints.push(endpoint.clone()); endpoints.sort(); endpoints.dedup();
        if !exists { object.insert("exit_with_herdr".to_owned(), Value::Bool(true)); }
        object.insert("endpoints".to_owned(), serde_json::to_value(endpoints).map_err(|error| error.to_string())?);
        Ok(true)
    })
}
pub(crate) fn read_preferences_object(store: &ProfileStore) -> Result<Map<String, Value>, String> {
    let path = store.config_dir().join("preferences.json");
    let mut file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != profile_store::effective_uid() { return Err("preferences must be a current-user-owned regular file".to_owned()); }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| format!("invalid preferences: {error}"))?;
    value.as_object().cloned().ok_or_else(|| "preferences must be a JSON object".to_owned())
}
pub(crate) fn preferences_from_object(object: &Map<String, Value>) -> Result<ObservationPreferences, String> {
    let mut preferences: ObservationPreferences = match object.get("observation") {
        None => ObservationPreferences::default(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| format!("invalid Herdr observation preferences: {error}"))?,
    };
    // Existing disk selection normalization differs deliberately from strict
    // new CLI/control patches: old duplicate/invalid machine IDs are discarded.
    preferences.sanitize();
    Ok(preferences)
}
pub(crate) fn read_preferences(store: &ProfileStore) -> Result<ObservationPreferences, String> {
    store.with_writer_lock(|| preferences_from_object(&read_preferences_object(store)?))
}
pub(crate) fn patch_preferences(object: &mut Map<String, Value>, local: Option<bool>, remote: Option<bool>, machines: Option<&[String]>) -> Result<ObservationPreferences, String> {
    let mut preferences = preferences_from_object(object)?;
    if let Some(value) = local { preferences.local = value; }
    if let Some(value) = remote { preferences.remote = value; }
    if let Some(value) = machines { preferences.machines = value.to_vec(); }
    preferences.validate()?;
    let observation = object.entry("observation".to_owned()).or_insert_with(|| Value::Object(Map::new())).as_object_mut().ok_or("observation must be an object")?;
    observation.insert("local".to_owned(), Value::Bool(preferences.local));
    observation.insert("remote".to_owned(), Value::Bool(preferences.remote));
    observation.insert("machines".to_owned(), serde_json::to_value(&preferences.machines).map_err(|error| error.to_string())?);
    Ok(preferences)
}
