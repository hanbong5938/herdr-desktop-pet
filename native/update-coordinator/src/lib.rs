//! Read-only installation discovery and durable, explicitly consented update plans.
//! This crate deliberately has no dependency on the desktop app's build script or rig.
mod origin;
pub mod protocol;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use origin::{
    check, detect_origin, installed_candidate, manager_command, recheck_plan, restored_original,
    ManagerCommand,
};
pub use protocol::{read_operation, write_operation, OperationPhase, OperationRecord};

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

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentity {
    pub path: PathBuf,
    pub sha256: String,
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
    if capabilities
        .get("protocol")
        .and_then(serde_json::Value::as_u64)
        != Some(protocol::UPDATER_PROTOCOL.into())
    {
        return Err("candidate does not support updater protocol 2".into());
    }
    Ok(image)
}

pub fn helper_path(executable: &Path) -> Result<PathBuf, String> {
    let image = validate_candidate(executable)?;
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
    let source = helper_path(executable)?;
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
mod plan_protocol_tests {
    use super::*;

    #[test]
    fn old_private_plan_cannot_enter_protocol_two_and_retains_original_bytes() {
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
        let id = "0123456789abcdef0123456789abcdef";
        let old = UpdatePlan {
            version: 1,
            operation_id: id.into(),
            context,
            origin,
            action: UpdateAction::ApplyInstalled,
            running: identity,
            candidate: None,
        };
        let bytes = serde_json::to_vec(&old).unwrap();
        let path = protocol::private_updates(&state)
            .unwrap()
            .join(format!("{id}.plan.json"));
        protocol::write_private_new(&path, &bytes, 0o600).unwrap();
        assert!(read_plan(&state, id).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
