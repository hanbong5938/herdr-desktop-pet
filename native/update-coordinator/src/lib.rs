//! Read-only installation discovery and durable, explicitly consented update plans.
//! This crate deliberately has no dependency on the desktop app's build script or rig.
mod observation;
mod origin;
pub mod protocol;
pub mod releases;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use observation::{ObservationMode, OBSERVATION_MODE_ENV};
pub use origin::{
    check, detect_origin, installed_candidate, manager_command, recheck_plan,
    refresh_release_target, restored_original, validate_plan_target, ManagerCommand,
};
pub use protocol::{read_operation, write_operation, OperationPhase, OperationRecord};
pub use releases::{ReleaseCandidate, ReleaseChannel, ReleaseTarget, UpdatePolicy};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UpdateContext {
    pub executable: PathBuf,
    pub version: String,
    pub instance_id: String,
    pub config_dir: PathBuf,
    /// Original injected host plugin config; never an override profile or socket parent.
    pub host_plugin_config_dir: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub herdr_socket: PathBuf,
    /// Snapshot captured once when this daemon starts, never reconstructed from the locator.
    pub running: ExecutableIdentity,
    /// Manager/source captured at daemon startup, before an external same-path replacement.
    #[serde(default)]
    pub running_origin: Option<Box<InstallOrigin>>,
    pub assets_override: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub locale: String,
}

impl UpdateContext {
    /// Use only the selection captured with this context, never the process environment.
    pub fn observation_mode(&self) -> Result<ObservationMode, String> {
        ObservationMode::from_environment(&self.environment)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentity {
    pub path: PathBuf,
    pub sha256: String,
}

/// Freshly verified installed executable and its actual installation registration.
/// This in-memory proof is not part of the plan, journal or wire protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedInstallation {
    pub image: ExecutableIdentity,
    pub origin: InstallOrigin,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallOrigin {
    Herdr {
        host: PathBuf,
        host_config_dir: PathBuf,
        checkout_root: PathBuf,
        plugin_root: PathBuf,
        source: String,
        requested_ref: Option<String>,
        resolved_commit: Option<String>,
        enabled: bool,
    },
    Local {
        root: PathBuf,
    },
    Homebrew {
        brew: PathBuf,
        prefix: PathBuf,
        cellar_formula_root: PathBuf,
        formula: String,
        locator: PathBuf,
    },
    Manual {
        locator: PathBuf,
    },
    Unknown {
        reason: String,
    },
}

/// Immutable replaceable boundary shared by reservations and process scans.
pub fn physical_install_root(origin: &InstallOrigin) -> Result<&Path, String> {
    match origin {
        InstallOrigin::Herdr { checkout_root, .. } => Ok(checkout_root),
        InstallOrigin::Local { root } => Ok(root),
        InstallOrigin::Homebrew {
            cellar_formula_root,
            ..
        } => Ok(cellar_formula_root),
        InstallOrigin::Manual { locator } => locator
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| "invalid manual bundle".into()),
        InstallOrigin::Unknown { .. } => Err("unknown installation has no physical root".into()),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UpdateAction {
    HerdrReinstall,
    HomebrewUpgrade,
    LocalRebuild,
    ApplyInstalled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlan {
    pub version: u32,
    pub operation_id: String,
    pub context: UpdateContext,
    pub origin: InstallOrigin,
    pub action: UpdateAction,
    pub running: ExecutableIdentity,
    pub candidate: Option<ExecutableIdentity>,
    pub target: Option<ReleaseTarget>,
}

impl UpdatePlan {
    /// The original origin remains immutable; this is the verified prospective installation.
    pub fn target_origin(&self) -> &InstallOrigin {
        self.target
            .as_ref()
            .map_or(&self.origin, |target| &target.origin)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    NotChecked,
    Checking,
    Current,
    Applied,
    Available,
    Installed,
    Local,
    Unsupported,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSnapshot {
    pub status: CheckStatus,
    pub origin: InstallOrigin,
    pub installed_version: Option<String>,
    pub observed_revision: Option<String>,
    pub detail: String,
    pub plan: Option<UpdatePlan>,
    pub policy: UpdatePolicy,
    pub release: Option<ReleaseCandidate>,
    pub checked_at: Option<u64>,
}

pub(crate) fn now() -> Result<u64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs())
}

pub(crate) fn ensure_absolute(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "expected absolute normalized path: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn owned_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    read_owned_file(path, limit, false)
}

pub fn owned_private_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    read_owned_file(path, limit, true)
}

fn read_owned_file(path: &Path, limit: u64, private: bool) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    ensure_absolute(path)?;
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.nlink() != 1
        || m.len() > limit
        || (private && m.permissions().mode() & 0o077 != 0)
    {
        return Err(format!(
            "not a bounded private user-owned regular file: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(m.len() as usize);
    f.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("file exceeded size limit".into());
    }
    Ok(bytes)
}

pub fn executable_identity(path: &Path) -> Result<ExecutableIdentity, String> {
    ensure_absolute(path)?;
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let m = fs::metadata(&canonical).map_err(|e| e.to_string())?;
    if !m.is_file() || m.permissions().mode() & 0o111 == 0 || m.len() > 256 * 1024 * 1024 {
        return Err("expected executable regular file within size limit".into());
    }
    let mut file = File::open(&canonical).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let count = file.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buf[..count]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    let at_path = fs::metadata(&canonical).map_err(|e| e.to_string())?;
    let proof = |m: &fs::Metadata| (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec());
    if proof(&m) != proof(&after) || proof(&after) != proof(&at_path) {
        return Err("executable changed while hashing".into());
    }
    Ok(ExecutableIdentity {
        path: canonical,
        sha256: format!("{:x}", hash.finalize()),
    })
}

/// Require the actual bundle and updater capability, not merely a changed hash/version.
pub fn validate_candidate(path: &Path) -> Result<ExecutableIdentity, String> {
    validate_candidate_mode(path, ObservationMode::Herdr)
}

/// Reject a candidate that cannot preserve the captured observation selection.
pub fn validate_candidate_for_context(
    path: &Path,
    context: &UpdateContext,
) -> Result<ExecutableIdentity, String> {
    validate_candidate_mode(path, context.observation_mode()?)
}

fn validate_candidate_mode(
    path: &Path,
    mode: ObservationMode,
) -> Result<ExecutableIdentity, String> {
    let image = executable_identity(path)?;
    let macos = image.path.parent().ok_or("candidate has no parent")?;
    let binary = image.path.file_name().ok_or("candidate has no name")?;
    let app = macos
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .ok_or("missing candidate app")?;
    if macos.file_name().is_none_or(|s| s != "MacOS")
        || macos
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|s| s != "Contents")
        || binary != "herdr-desktop-pet"
        || (app != "HerdrDesktopPet.app" && app != "HerdrDesktopPetBeta.app")
    {
        return Err("candidate is not a supported app bundle executable".into());
    }
    #[cfg(target_os = "macos")]
    origin::command_output(
        Path::new("/usr/bin/codesign"),
        &[
            "--verify",
            "--strict",
            "--deep",
            macos
                .parent()
                .and_then(Path::parent)
                .ok_or("missing app root")?
                .to_str()
                .ok_or("invalid app path")?,
        ],
        4096,
        10,
    )?;
    let response = origin::command_output(&image.path, &["update-capabilities"], 4096, 5)?;
    let capabilities: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| e.to_string())?;
    validate_capabilities(&capabilities, mode)?;
    Ok(image)
}

fn validate_capabilities(
    capabilities: &serde_json::Value,
    mode: ObservationMode,
) -> Result<(), String> {
    if capabilities
        .get("protocol")
        .and_then(serde_json::Value::as_u64)
        != Some(protocol::UPDATER_PROTOCOL.into())
    {
        return Err("candidate does not support updater protocol 3".into());
    }
    if mode.supports_tern()
        && capabilities
            .get("observation_mode")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
    {
        return Err("candidate does not support observation mode capability 1".into());
    }
    Ok(())
}

/// Require the signed helper to understand explicit captured mode keys, including Herdr.
pub fn validate_helper_capabilities(response: &str, context: &UpdateContext) -> Result<(), String> {
    let parsed: serde_json::Value = serde_json::from_str(response).map_err(|e| e.to_string())?;
    if parsed.get("protocol").and_then(serde_json::Value::as_u64)
        != Some(protocol::UPDATER_PROTOCOL.into())
    {
        return Err("release helper does not match updater protocol 3".into());
    }
    context.observation_mode()?;
    if context.environment.contains_key(OBSERVATION_MODE_ENV)
        && parsed
            .get("observation_mode")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
    {
        return Err("release helper does not support observation mode capability 1".into());
    }
    Ok(())
}

pub fn helper_path_for_context(
    executable: &Path,
    context: &UpdateContext,
) -> Result<PathBuf, String> {
    let image = validate_candidate_for_context(executable, context)?;
    let path = image.path.with_file_name("herdr-update-coordinator");
    executable_identity(&path)?;
    Ok(path)
}

/// Copy the signed/bundled helper before any installation can remove the old app.
pub fn stage_helper(plan: &UpdatePlan) -> Result<PathBuf, String> {
    recheck_plan(plan)?;
    let executable = match (&plan.action, &plan.candidate) {
        (UpdateAction::ApplyInstalled, Some(candidate)) => &candidate.path,
        _ => &plan.running.path,
    };
    let source = helper_path_for_context(executable, &plan.context)?;
    #[cfg(target_os = "macos")]
    origin::command_output(
        Path::new("/usr/bin/codesign"),
        &[
            "--verify",
            "--strict",
            source.to_str().ok_or("invalid helper path")?,
        ],
        4096,
        10,
    )?;
    validate_helper_capabilities(
        &origin::command_output(&source, &["update-capabilities"], 4096, 5)?,
        &plan.context,
    )?;
    let source_id = executable_identity(&source)?;
    let bytes = owned_file(&source, 64 * 1024 * 1024)?;
    let base = protocol::private_updates(&plan.context.state_dir)?;
    let target = base.join(format!("helper-{}", plan.operation_id));
    protocol::write_private_new(&target, &bytes, 0o700)?;
    let verified: Result<(), String> = (|| {
        if executable_identity(&target)?.sha256 != source_id.sha256 {
            return Err("staged helper differs from source".into());
        }
        #[cfg(target_os = "macos")]
        origin::command_output(
            Path::new("/usr/bin/codesign"),
            &[
                "--verify",
                "--strict",
                target.to_str().ok_or("invalid staged helper path")?,
            ],
            4096,
            10,
        )?;
        Ok(())
    })();
    if verified.is_err() {
        let _ = fs::remove_file(&target);
    }
    verified?;
    Ok(target)
}

pub fn write_plan(plan: &UpdatePlan) -> Result<PathBuf, String> {
    plan.context.observation_mode()?;
    recheck_plan(plan)?;
    let base = protocol::private_updates(&plan.context.state_dir)?;
    let path = base.join(format!("{}.plan.json", plan.operation_id));
    let content = serde_json::to_vec(plan).map_err(|e| e.to_string())?;
    if content.len() > 65536 {
        return Err("plan exceeds size limit".into());
    }
    protocol::write_private_new(&path, &content, 0o600)?;
    Ok(path)
}

pub fn read_plan(state_dir: &Path, operation_id: &str) -> Result<UpdatePlan, String> {
    protocol::validate_operation_id(operation_id)?;
    let data = owned_private_file(
        &protocol::private_updates(state_dir)?.join(format!("{operation_id}.plan.json")),
        65536,
    )?;
    let plan: UpdatePlan = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    plan.context.observation_mode()?;
    if plan.version != protocol::UPDATER_PROTOCOL
        || plan.operation_id != operation_id
        || plan.context.state_dir != state_dir
    {
        return Err("plan scope mismatch".into());
    }
    // Reading a durable plan remains possible after the installer replaces the source tree.
    // The helper calls recheck_plan immediately before mutation, not during recovery.
    if plan.running != plan.context.running {
        return Err("plan identity mismatch".into());
    }
    Ok(plan)
}

#[cfg(all(test, target_os = "macos"))]
mod candidate_tests {
    use super::*;

    #[test]
    fn unsigned_replacement_is_rejected_without_running_its_code() {
        let root = tempfile::tempdir().unwrap();
        let macos = root.path().join("HerdrDesktopPet.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        let candidate = macos.join("herdr-desktop-pet");
        let canary = root.path().join("executed");
        fs::write(
            &candidate,
            format!(
                "#!/bin/sh\n: > \"{}\"\nprintf '%s' '{{\"protocol\":2}}'\n",
                canary.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_candidate(&candidate).is_err());
        assert!(
            !canary.exists(),
            "checking an unsigned update executed its code"
        );
    }
}

#[cfg(test)]
mod observation_capability_tests {
    use super::*;

    #[test]
    fn old_protocol_three_candidate_remains_herdr_only() {
        let legacy = serde_json::json!({"protocol": 3});
        assert!(validate_capabilities(&legacy, ObservationMode::Herdr).is_ok());
        for mode in [ObservationMode::Tern, ObservationMode::Both] {
            assert!(validate_capabilities(&legacy, mode).is_err());
            for value in [serde_json::json!(0), serde_json::json!("1")] {
                assert!(validate_capabilities(
                    &serde_json::json!({"protocol": 3, "observation_mode": value}),
                    mode
                )
                .is_err());
            }
            assert!(validate_capabilities(
                &serde_json::json!({"protocol": 3, "observation_mode": 1}),
                mode
            )
            .is_ok());
        }
        assert!(validate_capabilities(
            &serde_json::json!({"protocol": 2, "observation_mode": 1}),
            ObservationMode::Tern
        )
        .is_err());
    }
}

#[cfg(test)]
mod plan_protocol_tests {
    use super::*;

    #[test]
    fn old_private_plan_cannot_enter_protocol_three_and_retains_original_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("profile");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let binary = temp
            .path()
            .join("HerdrDesktopPet.app/Contents/MacOS/herdr-desktop-pet");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"fixture").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let identity = executable_identity(&binary).unwrap();
        let origin = InstallOrigin::Manual {
            locator: identity.path.clone(),
        };
        let context = UpdateContext {
            executable: identity.path.clone(),
            version: "0.2.0".into(),
            instance_id: "fixture".into(),
            config_dir: temp.path().join("config"),
            host_plugin_config_dir: None,
            state_dir: state.clone(),
            herdr_socket: temp.path().join("socket"),
            running: identity.clone(),
            running_origin: Some(Box::new(origin.clone())),
            assets_override: None,
            environment: BTreeMap::new(),
            locale: "en".into(),
        };
        for version in [1, 2] {
            let id = format!("{version:032x}");
            let old = UpdatePlan {
                version,
                operation_id: id.clone(),
                context: context.clone(),
                origin: origin.clone(),
                action: UpdateAction::ApplyInstalled,
                running: identity.clone(),
                candidate: None,
                target: None,
            };
            let mut record = serde_json::to_value(&old).unwrap();
            record.as_object_mut().unwrap().remove("target");
            let bytes = serde_json::to_vec(&record).unwrap();
            let path = protocol::private_updates(&state)
                .unwrap()
                .join(format!("{id}.plan.json"));
            protocol::write_private_new(&path, &bytes, 0o600).unwrap();
            assert!(read_plan(&state, &id).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
}
