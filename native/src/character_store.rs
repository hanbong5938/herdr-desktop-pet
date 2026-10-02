use crate::assets::{
    builtin_dialogue_metadata, managed_dialogue_metadata, managed_manifest_files,
    CharacterMetadata, ManagedPack, ValidatedCharacter, MAX_FILE_BYTES,
};
use crate::character_types::{
    validate_pack_id, validate_pack_name, CharacterRef, PackAction, PackListing, PackOperation,
    PackRecord, PackRequest,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsString};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const STORE_DIR: &str = "characters";
const REGISTRY_FILE: &str = "registry.json";
const PREVIOUS_REGISTRY_FILE: &str = "registry.previous.json";
const LOCK_FILE: &str = ".registry.lock";
const PACKS_DIR: &str = "packs";
const STAGING_DIR: &str = ".staging";
const TRASH_DIR: &str = ".trash";
const INDEX_VERSION: u32 = 1;
const MAX_PACKS: usize = 32;
const MAX_REVISIONS: usize = 8;
const MAX_STORE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REGISTRY_BYTES: usize = 1024 * 1024;
const MAX_OPERATIONS: usize = 64;
const MAX_OPERATION_ID_BYTES: usize = 128;
// Per-directory limits remain useful for recovery, but a tree-wide budget is
// required as well: an attacker can otherwise create thousands of tiny
// directories while staying below every per-directory limit.  This covers
// packs, revisions, staging, and trash in one conservative scan.
const MAX_TOTAL_SCAN_ENTRIES: usize = 16_384;

#[derive(Default)]
struct ScanBudget {
    visited: usize,
}

impl ScanBudget {
    fn visit(&mut self) -> Result<(), String> {
        self.visited = self
            .visited
            .checked_add(1)
            .ok_or_else(|| "character store scan count overflow".to_string())?;
        if self.visited > MAX_TOTAL_SCAN_ENTRIES {
            return Err("character store contains too many entries to scan".to_string());
        }
        Ok(())
    }
}
const MAX_OPERATION_ERROR_BYTES: usize = 4096;
const MAX_STAGE_ENTRIES: usize = 256;
const MAX_READ_BYTES: usize = MAX_REGISTRY_BYTES;

#[cfg(target_os = "macos")]
extern "C" {
    fn renameatx_np(
        fromfd: libc::c_int,
        from: *const libc::c_char,
        tofd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}
static NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDisk {
    version: u32,
    generation: u64,
    selected: CharacterRef,
    packs: Vec<PackRecord>,
    next_revision: u64,
    operations: Vec<PackOperation>,
    preserve_orphans: bool,
}

impl RegistryDisk {
    fn empty() -> Self {
        Self {
            version: INDEX_VERSION,
            generation: 0,
            selected: CharacterRef::builtin(),
            packs: Vec::new(),
            next_revision: 1,
            operations: Vec::new(),
            preserve_orphans: false,
        }
    }
}

#[derive(Debug)]
struct LoadedRegistry {
    index: RegistryDisk,
    bytes: Option<Vec<u8>>,
    backup_bytes: Option<Vec<u8>>,
    current_valid: bool,
    backup_valid: bool,
    readonly: bool,
    preserve_orphans: bool,
    warning: Option<String>,
}

#[derive(Debug)]
struct StoreLock {
    file: File,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// The durable result of a registry rename.  A successful rename followed by
/// an unconfirmed directory sync is intentionally reported separately: the
/// caller must not roll back a registry that may already be durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitResult {
    pub generation: u64,
    pub durability_unknown: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PackStore {
    config_dir: PathBuf,
    builtin_assets: Option<PathBuf>,
    root: PathBuf,
}

/// A transaction owns the per-config store flock until it is either finished
/// or dropped.  Dropping deliberately leaves staged/renamed data in place;
pub struct PackTransaction {
    _lock: Option<StoreLock>,
    root: PathBuf,
    root_directory: File,
    old_index: RegistryDisk,
    current_bytes: Option<Vec<u8>>,
    proposed: RegistryDisk,
    operation: PackOperation,
    operation_id: String,
    stage_dir: Option<PathBuf>,
    final_dir: Option<PathBuf>,
    allocated_revision: Option<u64>,
    committed: bool,
    durability_unknown: bool,
    commit_generation: Option<u64>,
    active_reference: Option<CharacterRef>,

    pub candidate_ref: Option<CharacterRef>,
    pub selected: CharacterRef,
    pub candidate: Option<ValidatedCharacter>,
    pub changes_selection: bool,
}

impl PackStore {
    pub fn new(config_dir: PathBuf, builtin_assets: Option<PathBuf>) -> Self {
        let root = config_dir.join(STORE_DIR);
        Self {
            config_dir,
            builtin_assets,
            root,
        }
    }

    pub fn list(&self) -> Result<PackListing, String> {
        let (layout, _lock) = self.open_locked()?;
        let loaded = load_registry(&layout.root_file)?;
        if loaded.readonly {
            return Ok(PackListing {
                generation: loaded.index.generation,
                selected: CharacterRef::builtin(),
                active: None,
                override_active: false,
                packs: Vec::new(),
                error: loaded.warning,
            });
        }
        Ok(PackListing {
            generation: loaded.index.generation,
            selected: loaded.index.selected,
            active: None,
            override_active: false,
            packs: loaded.index.packs,
            error: loaded.warning,
        })
    }

    pub fn startup_candidates(&self) -> Result<Vec<(CharacterRef, PathBuf)>, String> {
        let builtin_assets = self
            .builtin_assets
            .clone()
            .ok_or_else(|| "builtin character assets are not configured".to_string())?;
        let (layout, _lock) = self.open_locked()?;
        let loaded = load_registry(&layout.root_file)?;

        let mut result = Vec::new();
        let selected = loaded.index.selected.clone();
        if !loaded.readonly && selected.id != CharacterRef::builtin().id {
            if let Some(record) = loaded
                .index
                .packs
                .iter()
                .find(|record| record.id == selected.id)
            {
                if record.revisions.contains(&selected.revision) {
                    result.push((
                        selected.clone(),
                        revision_path(&self.root, &record.id, selected.revision),
                    ));
                    for revision in record
                        .revisions
                        .iter()
                        .rev()
                        .copied()
                        .filter(|revision| *revision < selected.revision)
                    {
                        result.push((
                            CharacterRef {
                                id: record.id.clone(),
                                revision,
                            },
                            revision_path(&self.root, &record.id, revision),
                        ));
                    }
                }
            }
        }
        result.push((CharacterRef::builtin(), builtin_assets));
        Ok(result)
    }

    pub fn begin(
        &self,
        request: &PackRequest,
        active: Option<&CharacterRef>,
    ) -> Result<PackTransaction, String> {
        validate_operation_id(&request.operation_id)?;
        if self.builtin_assets.is_none() {
            return Err("builtin character assets are not configured".to_string());
        }
        let (layout, lock) = self.open_locked()?;
        let loaded = load_registry(&layout.root_file)?;
        if loaded.readonly {
            return Err(loaded.warning.unwrap_or_else(|| {
                "character store metadata is corrupt; store is read-only".to_string()
            }));
        }
        recover_owned(&layout.root_file, &self.root, &loaded, active)?;
        if let Some(expected) = request.expected_generation {
            if expected != loaded.index.generation {
                return Err(format!(
                    "character store generation mismatch (expected {expected}, current {})",
                    loaded.index.generation
                ));
            }
        }
        if loaded
            .index
            .operations
            .iter()
            .any(|operation| operation.operation_id == request.operation_id)
        {
            return Err(format!("operation {} already exists", request.operation_id));
        }

        let mut proposed = loaded.index.clone();
        proposed.preserve_orphans |= loaded.preserve_orphans;
        let mut candidate = None;
        let mut candidate_ref = None;
        let mut stage_dir = None;
        let mut final_dir = None;
        let mut allocated_revision = None;
        let current_selected = proposed.selected.clone();

        let root_file = layout
            .root_file
            .try_clone()
            .map_err(|error| format!("cannot pin character store root: {error}"))?;
        let stage_candidate = |managed: ManagedPack,
                               operation_id: &str,
                               root: &Path|
         -> Result<(PathBuf, ValidatedCharacter), String> {
            let incoming = managed_size(&managed)?;
            let used = measure_store(&root_file, root)?;
            if used
                .checked_add(incoming)
                .ok_or_else(|| "character store size overflow".to_string())?
                > MAX_STORE_BYTES
            {
                return Err("character store exceeds the 256 MiB limit".to_string());
            }
            let stage = create_stage_dir_pinned(&root_file, root, operation_id)?;
            if let Err(error) = write_managed_to_stage_pinned(&root_file, root, &stage, &managed) {
                let _ = remove_owned_tree_pinned(&root_file, root, &stage);
                return Err(error);
            }
            let stage_directory =
                open_root_relative_directory(&root_file, root, &stage, "staged pack")?;
            let candidate = match ManagedPack::load_from_directory(&stage_directory) {
                Ok(pack) => pack.assets,
                Err(error) => {
                    let _ = remove_owned_tree_pinned(&root_file, root, &stage);
                    return Err(format!("staged character pack is invalid: {error}"));
                }
            };
            sync_directory_pinned(&root_file, root, &stage)?;
            sync_directory_pinned(&root_file, root, &root.join(STAGING_DIR))?;
            Ok((stage, candidate))
        };

        let action_result: Result<(), String> = (|| {
            match &request.action {
                PackAction::Import { path } => {
                    let managed = ManagedPack::load(path)
                        .map_err(|error| format!("cannot import character pack: {error}"))?;
                    validate_managed_identity(&managed)?;
                    let id = managed.id.clone();
                    let name = managed.name.clone();
                    if id == CharacterRef::builtin().id {
                        return Err("the builtin character cannot be imported".to_string());
                    }
                    if proposed.packs.iter().any(|record| record.id == id) {
                        return Err(format!("character pack {id} already exists"));
                    }
                    if proposed.packs.len() >= MAX_PACKS {
                        return Err("character store already contains 32 packs".to_string());
                    }
                    ensure_pack_directory_pinned(&root_file, &self.root, &id)?;
                    let revision = preview_next_revision(&root_file, &proposed)?;
                    let (stage, assets) =
                        stage_candidate(managed, &request.operation_id, &self.root)?;
                    stage_dir = Some(stage);
                    allocated_revision = Some(revision);
                    final_dir = Some(revision_path(&self.root, &id, revision));
                    proposed.packs.push(PackRecord {
                        id: id.clone(),
                        name,
                        head: revision,
                        revisions: vec![revision],
                    });
                    candidate_ref = Some(CharacterRef { id, revision });
                    candidate = Some(assets);
                }
                PackAction::Update { id, path } => {
                    validate_managed_id(id)?;
                    let record_index = proposed
                        .packs
                        .iter()
                        .position(|record| record.id == *id)
                        .ok_or_else(|| format!("character pack {id} does not exist"))?;
                    if proposed.packs[record_index].revisions.len() >= MAX_REVISIONS {
                        return Err(format!("character pack {id} already has 8 revisions"));
                    }
                    let managed = ManagedPack::load(path)
                        .map_err(|error| format!("cannot update character pack: {error}"))?;
                    validate_managed_identity(&managed)?;
                    if managed.id != *id {
                        return Err(format!(
                            "managed pack id {} does not match {id}",
                            managed.id
                        ));
                    }
                    let name = managed.name.clone();
                    ensure_pack_directory_pinned(&root_file, &self.root, id)?;
                    let revision = preview_next_revision(&root_file, &proposed)?;
                    let (stage, assets) =
                        stage_candidate(managed, &request.operation_id, &self.root)?;
                    stage_dir = Some(stage);
                    allocated_revision = Some(revision);
                    final_dir = Some(revision_path(&self.root, id, revision));
                    let record = &mut proposed.packs[record_index];
                    record.name = name;
                    record.head = revision;
                    record.revisions.push(revision);
                    candidate_ref = Some(CharacterRef {
                        id: id.clone(),
                        revision,
                    });
                    if proposed.selected.id == *id
                        || active.map(|reference| reference.id == *id).unwrap_or(false)
                    {
                        proposed.selected = candidate_ref.clone().unwrap();
                    }
                    candidate = Some(assets);
                }
                PackAction::Select { id } => {
                    if *id == CharacterRef::builtin().id {
                        let path = self.builtin_assets.as_deref().ok_or_else(|| {
                            "builtin character assets are not configured".to_string()
                        })?;
                        let assets = ValidatedCharacter::load_builtin(path)
                            .map_err(|error| format!("builtin character is unhealthy: {error}"))?;
                        candidate_ref = Some(CharacterRef::builtin());
                        proposed.selected = CharacterRef::builtin();
                        candidate = Some(assets);
                    } else {
                        validate_id(id)?;
                        let record = proposed
                            .packs
                            .iter()
                            .find(|record| record.id == *id)
                            .ok_or_else(|| format!("character pack {id} does not exist"))?;
                        let revision = record.head;
                        let source = revision_path(&self.root, id, revision);
                        let source_directory = open_root_relative_directory(
                            &root_file,
                            &self.root,
                            &source,
                            "stored character pack",
                        )?;
                        verify_private_directory(&source_directory, &source)?;
                        let managed = ManagedPack::load_from_directory(&source_directory).map_err(
                            |error| format!("selected character pack is unhealthy: {error}"),
                        )?;
                        if managed.id != *id {
                            return Err(format!(
                                "managed pack id {} does not match {id}",
                                managed.id
                            ));
                        }
                        candidate_ref = Some(CharacterRef {
                            id: id.clone(),
                            revision,
                        });
                        proposed.selected = candidate_ref.clone().unwrap();
                        candidate = Some(managed.assets);
                    }
                }
                PackAction::Restore { id, revision } => {
                    validate_managed_id(id)?;
                    if *revision == 0 {
                        return Err("managed revisions must be non-zero".to_string());
                    }
                    let record = proposed
                        .packs
                        .iter()
                        .find(|record| record.id == *id)
                        .ok_or_else(|| format!("character pack {id} does not exist"))?;
                    if !record.revisions.contains(revision) {
                        return Err(format!("character pack {id} has no revision {revision}"));
                    }
                    let source = revision_path(&self.root, id, *revision);
                    let source_directory = open_root_relative_directory(
                        &root_file,
                        &self.root,
                        &source,
                        "stored character pack",
                    )?;
                    verify_private_directory(&source_directory, &source)?;
                    let managed =
                        ManagedPack::load_from_directory(&source_directory).map_err(|error| {
                            format!("restored character pack is unhealthy: {error}")
                        })?;
                    if managed.id != *id {
                        return Err(format!(
                            "managed pack id {} does not match {id}",
                            managed.id
                        ));
                    }
                    candidate_ref = Some(CharacterRef {
                        id: id.clone(),
                        revision: *revision,
                    });
                    proposed.selected = candidate_ref.clone().unwrap();
                    candidate = Some(managed.assets);
                }
                PackAction::Remove { id } => {
                    validate_managed_id(id)?;
                    let position = proposed
                        .packs
                        .iter()
                        .position(|record| record.id == *id)
                        .ok_or_else(|| format!("character pack {id} does not exist"))?;
                    proposed.packs.remove(position);
                    let changes_selection = proposed.selected.id == *id
                        || active.map(|reference| reference.id == *id).unwrap_or(false);
                    if changes_selection {
                        proposed.selected = CharacterRef::builtin();
                        candidate_ref = Some(CharacterRef::builtin());
                        let path = self.builtin_assets.as_deref().ok_or_else(|| {
                            "builtin character assets are not configured".to_string()
                        })?;
                        candidate =
                            Some(ValidatedCharacter::load_builtin(path).map_err(|error| {
                                format!("builtin character is unhealthy: {error}")
                            })?);
                    }
                }
            }
            Ok(())
        })();

        if let Err(error) = action_result {
            if let Some(stage) = stage_dir {
                let _ = remove_owned_tree_pinned(&root_file, &self.root, &stage);
            }
            return Err(error);
        }

        let operation = PackOperation {
            operation_id: request.operation_id.clone(),
            state: "preparing".to_string(),
            committed: false,
            ui_applied: false,
            generation: None,
            error: None,
        };
        validate_registry(&proposed)?;
        let current_bytes = loaded.bytes.clone();
        let changes_selection = proposed.selected != current_selected;
        let mut transaction = PackTransaction {
            _lock: Some(lock),
            root: self.root.clone(),
            root_directory: layout.root_file,
            old_index: loaded.index,
            current_bytes,
            proposed,
            operation,
            operation_id: request.operation_id.clone(),
            stage_dir,
            final_dir,
            allocated_revision,
            committed: false,
            durability_unknown: false,
            commit_generation: None,
            active_reference: active.cloned(),
            candidate,
            candidate_ref,
            selected: CharacterRef::builtin(),
            changes_selection,
        };
        transaction.selected = transaction.proposed.selected.clone();
        Ok(transaction)
    }

    /// A read-only, generation-pinned lookup. The revision directory and
    /// manifest are opened with the same descriptor-relative security checks
    /// as load_revision, but image and rig payloads are never opened.
    pub(crate) fn dialogue_metadata(
        &self,
        reference: &CharacterRef,
        generation: u64,
    ) -> Result<(String, Option<CharacterMetadata>), String> {
        validate_character_ref(reference)?;
        let (layout, _lock) = self.open_locked()?;
        let loaded = load_registry(&layout.root_file)?;
        if loaded.index.generation != generation {
            return Err(format!(
                "character store generation mismatch (expected {generation}, current {})",
                loaded.index.generation
            ));
        }
        if reference.is_builtin() {
            let path = self
                .builtin_assets
                .as_deref()
                .ok_or_else(|| "builtin character assets are not configured".to_string())?;
            return builtin_dialogue_metadata(path);
        }
        if loaded.readonly {
            return Err(loaded.warning.unwrap_or_else(|| {
                "character store metadata is corrupt; store is read-only".to_string()
            }));
        }
        let record = loaded
            .index
            .packs
            .iter()
            .find(|record| record.id == reference.id)
            .ok_or_else(|| format!("character pack {} is not registered", reference.id))?;
        if !record.revisions.contains(&reference.revision) {
            return Err(format!(
                "character pack {} has no retained revision {}",
                reference.id, reference.revision
            ));
        }
        let packs = open_directory_at(
            &layout.root_file,
            std::ffi::OsStr::new(PACKS_DIR),
            "character packs directory",
        )?;
        let pack_directory = open_directory_at(
            &packs,
            std::ffi::OsStr::new(&reference.id),
            "managed character directory",
        )?;
        verify_private_directory(&pack_directory, Path::new("<managed character directory>"))?;
        let revision_directory = open_directory_at(
            &pack_directory,
            std::ffi::OsStr::new(&reference.revision.to_string()),
            "managed character revision",
        )?;
        verify_private_directory(
            &revision_directory,
            Path::new("<managed character revision>"),
        )?;
        managed_dialogue_metadata(&revision_directory, &reference.id)
    }

    pub fn load_revision(&self, reference: &CharacterRef) -> Result<ValidatedCharacter, String> {
        validate_character_ref(reference)?;
        let (layout, _lock) = self.open_locked()?;
        if reference.id == CharacterRef::builtin().id {
            let path = self
                .builtin_assets
                .as_deref()
                .ok_or_else(|| "builtin character assets are not configured".to_string())?;
            return ValidatedCharacter::load_builtin(path);
        }

        let loaded = load_registry(&layout.root_file)?;
        if loaded.readonly {
            return Err(loaded.warning.unwrap_or_else(|| {
                "character store metadata is corrupt; store is read-only".to_string()
            }));
        }
        let record = loaded
            .index
            .packs
            .iter()
            .find(|record| record.id == reference.id)
            .ok_or_else(|| format!("character pack {} is not registered", reference.id))?;
        if !record.revisions.contains(&reference.revision) {
            return Err(format!(
                "character pack {} has no retained revision {}",
                reference.id, reference.revision
            ));
        }
        let packs = open_directory_at(
            &layout.root_file,
            std::ffi::OsStr::new(PACKS_DIR),
            "character packs directory",
        )?;
        let pack_directory = open_directory_at(
            &packs,
            std::ffi::OsStr::new(&reference.id),
            "managed character directory",
        )?;
        verify_private_directory(&pack_directory, Path::new("<managed character directory>"))?;
        let revision_directory = open_directory_at(
            &pack_directory,
            std::ffi::OsStr::new(&reference.revision.to_string()),
            "managed character revision",
        )?;
        verify_private_directory(
            &revision_directory,
            Path::new("<managed character revision>"),
        )?;
        let managed = ManagedPack::load_from_directory(&revision_directory)?;
        if managed.id != reference.id {
            return Err(format!(
                "managed pack id {} does not match referenced pack {}",
                managed.id, reference.id
            ));
        }
        Ok(managed.assets)
    }
    /// Export an immutable managed revision without touching registry state.
    /// The archive writer refuses an existing destination.
    pub fn export(&self, reference: &CharacterRef, output: &Path) -> Result<(), String> {
        validate_character_ref(reference)?;
        let (layout, _lock) = self.open_locked()?;
        let managed = if reference.id == CharacterRef::builtin().id {
            let path = self
                .builtin_assets
                .as_deref()
                .ok_or_else(|| "builtin character assets are not configured".to_string())?;
            ManagedPack::load_internal(path, true)?
        } else {
            let loaded = load_registry(&layout.root_file)?;
            if loaded.readonly {
                return Err(loaded.warning.unwrap_or_else(|| {
                    "character store metadata is corrupt; store is read-only".to_string()
                }));
            }
            let record = loaded
                .index
                .packs
                .iter()
                .find(|record| record.id == reference.id)
                .ok_or_else(|| format!("character pack {} is not registered", reference.id))?;
            if !record.revisions.contains(&reference.revision) {
                return Err(format!(
                    "character pack {} has no retained revision {}",
                    reference.id, reference.revision
                ));
            }
            let packs = open_directory_at(
                &layout.root_file,
                std::ffi::OsStr::new(PACKS_DIR),
                "character packs directory",
            )?;
            let pack_directory = open_directory_at(
                &packs,
                std::ffi::OsStr::new(&reference.id),
                "managed character directory",
            )?;
            verify_private_directory(&pack_directory, Path::new("<managed character directory>"))?;
            let revision_directory = open_directory_at(
                &pack_directory,
                std::ffi::OsStr::new(&reference.revision.to_string()),
                "managed character revision",
            )?;
            verify_private_directory(
                &revision_directory,
                Path::new("<managed character revision>"),
            )?;
            ManagedPack::load_from_directory(&revision_directory)?
        };
        managed.export_archive(output)
    }

    pub fn operation_status(&self, operation_id: &str) -> Result<Option<PackOperation>, String> {
        validate_operation_id(operation_id)?;
        let (layout, _lock) = self.open_locked()?;
        let loaded = load_registry(&layout.root_file)?;
        if loaded.readonly {
            return Err(loaded.warning.unwrap_or_else(|| {
                "character store metadata is corrupt; store is read-only".to_string()
            }));
        }
        Ok(loaded
            .index
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id))
    }

    fn open_locked(&self) -> Result<(StoreLayout, StoreLock), String> {
        let layout = ensure_layout(&self.config_dir, &self.root)?;
        let lock = acquire_store_lock(&layout.root_file, &layout.root, &layout.lock_path)?;
        Ok((layout, lock))
    }
}

impl PackTransaction {
    /// Publish the staged revision (if any) and replace `registry.json`.
    ///
    /// `Err` always means `registry.json` was not replaced: every failing step
    /// runs before or is the registry rename itself, and a failed rename
    /// changes nothing. A revision directory already renamed into `packs/` is
    /// left as an unreferenced orphan for `recover_owned` to quarantine.
    /// Confirmation failures after the rename are reported via
    /// `CommitResult::durability_unknown`, never as `Err`.
    pub fn commit(&mut self) -> Result<CommitResult, String> {
        if self.committed {
            return Ok(CommitResult {
                generation: self.commit_generation.unwrap_or(self.proposed.generation),
                durability_unknown: self.durability_unknown,
                error: if self.durability_unknown {
                    Some("registry durability could not be confirmed".to_string())
                } else {
                    None
                },
            });
        }

        let next_generation = self
            .old_index
            .generation
            .checked_add(1)
            .ok_or_else(|| "character store generation overflow".to_string())?;
        let mut proposed = self.proposed.clone();
        proposed.generation = next_generation;
        if let Some(revision) = self.allocated_revision {
            proposed.next_revision = proposed.next_revision.max(revision.saturating_add(1));
        }
        validate_registry(&proposed)?;

        let mut committed_operation = self.operation.clone();
        committed_operation.committed = true;
        committed_operation.generation = Some(next_generation);
        committed_operation.state = "committed_pending_apply".to_string();
        committed_operation.error = None;
        proposed.operations.push(committed_operation);
        trim_operations(&mut proposed.operations);
        let operation_index = proposed
            .operations
            .iter_mut()
            .find(|operation| operation.operation_id == self.operation_id)
            .ok_or_else(|| "transaction operation metadata disappeared".to_string())?;
        operation_index.committed = true;
        operation_index.generation = Some(next_generation);
        operation_index.state = "committed_pending_apply".to_string();
        operation_index.error = None;
        let registry_bytes = serde_json::to_vec(&proposed)
            .map_err(|error| format!("registry cannot be encoded: {error}"))?;
        ensure_registry_peak_budget(
            &self.root_directory,
            &self.root,
            self.current_bytes.as_deref(),
            &registry_bytes,
        )?;
        if let Some(stage) = self.stage_dir.as_ref() {
            if let Some(final_dir) = self.final_dir.as_ref() {
                let parent = final_dir
                    .parent()
                    .ok_or_else(|| "final character path has no parent".to_string())?;
                rename_owned_noreplace_pinned(
                    &self.root_directory,
                    &self.root,
                    stage,
                    final_dir,
                    "character revision",
                )?;
                if sync_directory_pinned(
                    &self.root_directory,
                    &self.root,
                    &self.root.join(STAGING_DIR),
                )
                .is_err()
                {
                    self.durability_unknown = true;
                }
                if sync_directory_pinned(&self.root_directory, &self.root, parent).is_err() {
                    self.durability_unknown = true;
                }
                if sync_directory_pinned(
                    &self.root_directory,
                    &self.root,
                    &self.root.join(PACKS_DIR),
                )
                .is_err()
                {
                    self.durability_unknown = true;
                }
                self.stage_dir = None;
            }
        }

        let (bytes, unknown) = match write_registry_pinned(
            &self.root_directory,
            &self.root,
            self.current_bytes.as_deref(),
            &proposed,
        ) {
            Ok(result) => result,
            Err(error) => {
                // registry.json is unchanged. Any renamed revision remains an
                // owned orphan; Drop does not remove it, and the next trusted
                // recovery pass quarantines it.
                return Err(error);
            }
        };
        self.current_bytes = Some(bytes);
        self.proposed = proposed;
        self.committed = true;
        self.commit_generation = Some(next_generation);
        self.durability_unknown |= unknown;
        let error = if self.durability_unknown {
            Some("registry durability could not be confirmed".to_string())
        } else {
            None
        };
        Ok(CommitResult {
            generation: next_generation,
            durability_unknown: self.durability_unknown,
            error,
        })
    }

    /// Record whether the native UI acknowledged the candidate.  The store
    /// never assumes native success from a filesystem commit.
    pub fn set_ui_applied(&mut self, applied: bool) {
        self.operation.ui_applied = applied;
        if let Some(operation) = self
            .proposed
            .operations
            .iter_mut()
            .find(|operation| operation.operation_id == self.operation_id)
        {
            operation.ui_applied = applied;
        }
    }

    /// Mark a native-preflight failure before the transaction crosses the
    /// registry commit boundary.  The service owns its live result; this
    /// method only preserves the reason on the transaction for diagnostics.
    pub fn mark_failed(&mut self, error: impl Into<String>) -> Result<(), String> {
        if self.committed {
            return Err("committed character transactions cannot be failed".to_string());
        }
        self.operation.state = "failed".to_string();
        self.operation.error = Some(error.into());
        Ok(())
    }

    pub fn finish(&mut self, allow_gc: bool) -> Result<Option<String>, String> {
        if !self.committed {
            return Err("cannot finish an uncommitted character transaction".to_string());
        }
        let mut cleanup_warning = None;

        // Persist the logical completion and UI acknowledgement before any
        // checkpoint or physical cleanup. A cleanup failure must never turn a
        // durable commit back into a pending or failed operation.
        let mut completed = self.proposed.clone();
        if let Some(operation) = completed
            .operations
            .iter_mut()
            .find(|operation| operation.operation_id == self.operation_id)
        {
            operation.state = if self.durability_unknown {
                "durability_unknown".to_string()
            } else if allow_gc {
                "completed".to_string()
            } else {
                "committed_pending_apply".to_string()
            };
            operation.ui_applied = self.operation.ui_applied;
            operation.committed = true;
            operation.generation = self.commit_generation;
            operation.error = if self.durability_unknown {
                Some("registry durability could not be confirmed".to_string())
            } else if allow_gc {
                None
            } else {
                Some("native character apply is pending".to_string())
            };
        }
        let completed_bytes = serde_json::to_vec(&completed)
            .map_err(|error| format!("registry cannot be encoded: {error}"))?;
        ensure_registry_peak_budget(
            &self.root_directory,
            &self.root,
            self.current_bytes.as_deref(),
            &completed_bytes,
        )?;
        let (bytes, unknown) = write_registry_pinned(
            &self.root_directory,
            &self.root,
            self.current_bytes.as_deref(),
            &completed,
        )?;
        self.current_bytes = Some(bytes);
        self.proposed = completed;
        self.durability_unknown |= unknown;
        if self.durability_unknown {
            return Err("operation completion durability could not be confirmed".to_string());
        }

        if allow_gc {
            // A checkpoint makes both index slots describe the committed data
            // before any unreferenced revision is moved or deleted.
            let current = self
                .current_bytes
                .as_ref()
                .ok_or_else(|| "committed registry bytes are unavailable".to_string())?;
            match atomic_replace_pinned(
                &self.root_directory,
                &self.root,
                &self.root.join(PREVIOUS_REGISTRY_FILE),
                current,
                "registry-previous",
            ) {
                Ok(true) => {
                    cleanup_warning = Some(
                        "registry backup durability could not be confirmed; garbage collection deferred"
                            .to_string(),
                    );
                }
                Ok(false) => {
                    // Only native replacement acknowledgement releases the
                    // old live reference. An explicit reselect can acknowledge
                    // a replacement without changing the durable selection.
                    // Unselected imports/updates must keep it protected.
                    let active = if self.operation.ui_applied {
                        None
                    } else {
                        self.active_reference.as_ref()
                    };
                    if let Err(error) =
                        gc_owned(&self.root_directory, &self.root, &self.proposed, active)
                    {
                        cleanup_warning =
                            Some(format!("character store cleanup deferred: {error}"));
                    }
                }
                Err(error) => {
                    cleanup_warning = Some(format!("registry backup checkpoint deferred: {error}"));
                }
            }
        }
        Ok(cleanup_warning)
    }
}

impl Drop for PackTransaction {
    fn drop(&mut self) {
        // Deliberately do not remove stage/final paths.  A transaction may have
        // crossed the registry rename boundary, and recovery is the only place
        // allowed to classify an orphan under trusted metadata.
        self._lock.take();
    }
}
#[derive(Debug)]
struct StoreLayout {
    root: PathBuf,
    root_file: File,
    lock_path: PathBuf,
}

fn path_cstring(path: &Path, label: &str) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("{label} contains an embedded NUL"))
}

fn name_cstring(name: &std::ffi::OsStr, label: &str) -> Result<CString, String> {
    CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an embedded NUL"))
}

/// Open the final path component without following it. Ancestor aliases such
/// as Darwin's /tmp -> /private/tmp remain valid; managed roots and children
/// are pinned by the returned descriptor and cannot be redirected by a later
/// rename of their pathname.
fn open_directory_fd(path: &Path) -> Result<File, String> {
    let c_path = path_cstring(path, "directory path")?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot open directory {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a real directory", path.display()));
    }
    Ok(file)
}

fn open_directory_at(parent: &File, name: &std::ffi::OsStr, label: &str) -> Result<File, String> {
    let c_name = name_cstring(name, label)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot open {label}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn open_or_create_directory_at(
    parent: &File,
    name: &std::ffi::OsStr,
    label: &str,
) -> Result<File, String> {
    if let Ok(directory) = open_directory_at(parent, name, label) {
        return Ok(directory);
    }
    let c_name = name_cstring(name, label)?;
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), c_name.as_ptr(), 0o700) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(format!("cannot create {label}: {error}"));
        }
    }
    open_directory_at(parent, name, label)
}

fn stat_at(
    parent: &File,
    name: &std::ffi::OsStr,
    label: &str,
) -> Result<Option<libc::stat>, String> {
    let c_name = name_cstring(name, label)?;
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            c_name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(Some(metadata));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOENT) {
        Ok(None)
    } else {
        Err(format!("cannot inspect {label}: {error}"))
    }
}

fn verify_private_directory(file: &File, path: &Path) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a real directory", path.display()));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(format!(
            "directory {} is not owned by the current user",
            path.display()
        ));
    }
    // Ownership is checked from the opened descriptor before changing mode;
    // this cannot chmod an attacker-controlled replacement or an ancestor.
    if metadata.permissions().mode() & 0o077 != 0 {
        let result = unsafe { libc::fchmod(file.as_raw_fd(), 0o700) };
        if result != 0 {
            return Err(format!(
                "cannot secure directory {}: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
    }
    let secured = file
        .metadata()
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if secured.file_type().is_symlink()
        || !secured.is_dir()
        || secured.uid() != unsafe { libc::geteuid() }
        || secured.permissions().mode() & 0o077 != 0
    {
        return Err(format!("directory {} is not private", path.display()));
    }
    Ok(())
}

fn open_child_file(
    parent: &File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
    mode: libc::mode_t,
    label: &str,
) -> Result<File, String> {
    let c_name = name_cstring(name, label)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c_name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(format!(
            "cannot open {label}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn unlink_child(
    parent: &File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
    label: &str,
) -> Result<(), String> {
    let c_name = name_cstring(name, label)?;
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), c_name.as_ptr(), flags) };
    if result != 0 {
        return Err(format!(
            "cannot remove {label}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn child_name<'a>(path: &'a Path, label: &str) -> Result<&'a std::ffi::OsStr, String> {
    path.file_name()
        .ok_or_else(|| format!("{label} path has no final component"))
}

fn open_root_relative_parent(
    root_file: &File,
    root_path: &Path,
    path: &Path,
    label: &str,
) -> Result<(File, OsString), String> {
    let relative = path
        .strip_prefix(root_path)
        .map_err(|_| format!("{label} is outside the trusted character store"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| format!("{label} path has no final component"))?
        .to_os_string();
    let mut directory = root_file
        .try_clone()
        .map_err(|error| format!("cannot pin character store root: {error}"))?;
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(format!("{label} contains an unsafe path component"));
            };
            directory = open_directory_at(&directory, component, label)?;
        }
    }
    verify_private_directory(
        &directory,
        path.parent().unwrap_or_else(|| Path::new("<store root>")),
    )?;
    Ok((directory, name))
}

fn open_root_relative_directory(
    root_file: &File,
    root_path: &Path,
    path: &Path,
    label: &str,
) -> Result<File, String> {
    let relative = path
        .strip_prefix(root_path)
        .map_err(|_| format!("{label} is outside the trusted character store"))?;
    let mut directory = root_file
        .try_clone()
        .map_err(|error| format!("cannot pin character store root: {error}"))?;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(format!("{label} contains an unsafe path component"));
        };
        directory = open_directory_at(&directory, component, label)?;
    }
    verify_private_directory(&directory, path)?;
    Ok(directory)
}
fn sync_directory_pinned(root_file: &File, root_path: &Path, path: &Path) -> Result<(), String> {
    let directory = open_root_relative_directory(root_file, root_path, path, "directory")?;
    verify_private_directory(&directory, path)?;
    directory
        .sync_all()
        .map_err(|error| format!("cannot sync directory {}: {error}", path.display()))
}

fn ensure_layout(config_dir: &Path, root: &Path) -> Result<StoreLayout, String> {
    ensure_real_directory(config_dir)?;
    let config_directory = open_directory_fd(config_dir)?;
    let relative_root = root
        .strip_prefix(config_dir)
        .map_err(|_| "character store root is outside its config directory".to_string())?;
    let mut components = relative_root.components();
    let Some(std::path::Component::Normal(root_name)) = components.next() else {
        return Err("character store root has no final component".to_string());
    };
    if components.next().is_some() {
        return Err("character store root must be a direct config child".to_string());
    }
    let root_file = open_or_create_directory_at(&config_directory, root_name, "character store")?;
    verify_private_directory(&root_file, root)?;
    let packs = open_or_create_directory_at(
        &root_file,
        std::ffi::OsStr::new(PACKS_DIR),
        "character packs directory",
    )?;
    verify_private_directory(&packs, &root.join(PACKS_DIR))?;
    let staging = open_or_create_directory_at(
        &root_file,
        std::ffi::OsStr::new(STAGING_DIR),
        "character staging directory",
    )?;
    verify_private_directory(&staging, &root.join(STAGING_DIR))?;
    let trash = open_or_create_directory_at(
        &root_file,
        std::ffi::OsStr::new(TRASH_DIR),
        "character trash directory",
    )?;
    verify_private_directory(&trash, &root.join(TRASH_DIR))?;
    root_file
        .sync_all()
        .map_err(|error| format!("cannot sync directory {}: {error}", root.display()))?;
    Ok(StoreLayout {
        root: root.to_path_buf(),
        root_file,
        lock_path: root.join(LOCK_FILE),
    })
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let directory = match open_directory_fd(path) {
        Ok(directory) => directory,
        Err(error) => {
            let missing = matches!(
                fs::symlink_metadata(path),
                Err(inspect) if inspect.kind() == io::ErrorKind::NotFound
            );
            if !missing {
                return Err(error);
            }
            fs::create_dir(path).map_err(|create| {
                format!("cannot create directory {}: {create}", path.display())
            })?;
            open_directory_fd(path)?
        }
    };
    verify_private_directory(&directory, path)
}
fn acquire_store_lock(
    root_directory: &File,
    root: &Path,
    path: &Path,
) -> Result<StoreLock, String> {
    let name = child_name(path, "store lock")?;
    let file = open_child_file(
        root_directory,
        name,
        libc::O_RDWR | libc::O_CREAT | libc::O_NONBLOCK,
        0o600,
        "store lock",
    )?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect store lock {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1
    {
        return Err(format!(
            "store lock {} is not a private regular file",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        let result = unsafe { libc::fchmod(file.as_raw_fd(), 0o600) };
        if result != 0 {
            return Err(format!(
                "cannot secure store lock {}: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
    }
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EAGAIN)
            || error.raw_os_error() == Some(libc::EWOULDBLOCK)
        {
            return Err(format!(
                "Busy: character store {} is locked",
                root.display()
            ));
        }
        return Err(format!(
            "cannot lock character store {}: {error}",
            root.display()
        ));
    }
    Ok(StoreLock { file })
}

fn load_registry(root_file: &File) -> Result<LoadedRegistry, String> {
    let current = read_registry_slot(root_file, REGISTRY_FILE);
    let previous = read_registry_slot(root_file, PREVIOUS_REGISTRY_FILE);
    let (current_index, current_bytes, current_error) = match current {
        Ok(Some((index, bytes))) => (Some(index), Some(bytes), None),
        Ok(None) => (None, None, None),
        Err(error) => (None, None, Some(error)),
    };
    let (previous_index, previous_bytes, previous_error) = match previous {
        Ok(Some((index, bytes))) => (Some(index), Some(bytes), None),
        Ok(None) => (None, None, None),
        Err(error) => (None, None, Some(error)),
    };

    if current_index.is_none() && previous_index.is_none() {
        let warning = match (current_error.as_deref(), previous_error.as_deref()) {
            (None, None) => None,
            (Some(current), None) => Some(format!("registry.json is invalid: {current}")),
            (None, Some(previous)) => {
                Some(format!("registry.previous.json is invalid: {previous}"))
            }
            (Some(current), Some(previous)) => Some(format!(
                "character store metadata is corrupt; registry.json: {current}; registry.previous.json: {previous}"
            )),
        };
        let preserve_orphans =
            warning.is_some() || scan_max_revision(root_file).unwrap_or_default() > 0;
        return Ok(LoadedRegistry {
            index: RegistryDisk::empty(),
            bytes: None,
            backup_bytes: None,
            current_valid: false,
            backup_valid: false,
            readonly: warning.is_some(),
            preserve_orphans,
            warning,
        });
    }

    let current_valid = current_index.is_some();
    let backup_valid = previous_index.is_some();
    let use_previous = match (&current_index, &previous_index) {
        (Some(current), Some(previous)) => previous.generation > current.generation,
        (None, Some(_)) => true,
        _ => false,
    };
    let preserve_orphans = current_index
        .as_ref()
        .map(|index| index.preserve_orphans)
        .unwrap_or(false)
        || previous_index
            .as_ref()
            .map(|index| index.preserve_orphans)
            .unwrap_or(false)
        || current_error.is_some()
        || previous_error.is_some()
        || (current_index.is_none() && previous_index.is_some());
    let warning = if use_previous {
        match (current_error.as_deref(), &current_index, &previous_index) {
            (Some(error), _, _) => Some(format!(
                "using registry.previous.json after registry.json failure: {error}"
            )),
            (None, Some(current), Some(previous)) if previous.generation > current.generation => {
                Some(format!(
                    "using newer registry.previous.json generation {} over registry.json generation {}",
                    previous.generation, current.generation
                ))
            }
            _ => Some("using registry.previous.json after registry.json failure".to_string()),
        }
    } else {
        previous_error.map(|error| format!("registry.previous.json is invalid: {error}"))
    };
    let (index, bytes) = if use_previous {
        (
            previous_index.expect("previous registry exists when selected"),
            previous_bytes.clone(),
        )
    } else {
        (
            current_index.expect("current registry exists when selected"),
            current_bytes,
        )
    };
    Ok(LoadedRegistry {
        index,
        bytes,
        backup_bytes: previous_bytes,
        current_valid,
        backup_valid,
        readonly: false,
        preserve_orphans,
        warning,
    })
}

fn read_registry_slot(
    root_file: &File,
    name: &str,
) -> Result<Option<(RegistryDisk, Vec<u8>)>, String> {
    let name = std::ffi::OsStr::new(name);
    let Some(metadata) = stat_at(root_file, name, "registry")? else {
        return Ok(None);
    };
    let file_type = metadata.st_mode & libc::S_IFMT;
    if file_type != libc::S_IFREG
        || metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t }
        || metadata.st_nlink != 1
        || metadata.st_mode & 0o077 != 0
    {
        return Err("registry must be a private regular file".to_string());
    }
    let file = open_child_file(
        root_file,
        name,
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
        "registry",
    )?;
    let bytes = read_open_regular(file, MAX_REGISTRY_BYTES, "registry")?;
    let index: RegistryDisk = serde_json::from_slice(&bytes)
        .map_err(|error| format!("registry is invalid JSON: {error}"))?;
    validate_registry(&index)?;
    Ok(Some((index, bytes)))
}

fn validate_registry(index: &RegistryDisk) -> Result<(), String> {
    if index.version != INDEX_VERSION {
        return Err("unsupported character registry version".to_string());
    }
    if index.packs.len() > MAX_PACKS {
        return Err("character registry contains too many packs".to_string());
    }
    if index.next_revision == 0 {
        return Err("character registry revision allocator is exhausted".to_string());
    }
    validate_character_ref(&index.selected)?;
    let mut ids = BTreeSet::new();
    for record in &index.packs {
        validate_pack_id(&record.id)?;
        if record.id == CharacterRef::builtin().id {
            return Err("builtin character cannot be a managed pack".to_string());
        }
        validate_pack_name(&record.name)?;
        if record.revisions.is_empty() || record.revisions.len() > MAX_REVISIONS {
            return Err(format!("pack {} has an invalid revision count", record.id));
        }
        let mut previous = 0;
        for revision in &record.revisions {
            if *revision == 0 || *revision <= previous {
                return Err(format!(
                    "pack {} revisions are not strictly increasing",
                    record.id
                ));
            }
            previous = *revision;
        }
        if record.head != *record.revisions.last().unwrap() {
            return Err(format!(
                "pack {} head is not its newest revision",
                record.id
            ));
        }
        if !ids.insert(record.id.clone()) {
            return Err(format!("duplicate character pack id {}", record.id));
        }
    }
    if index.selected.id == CharacterRef::builtin().id {
        if index.selected.revision != 0 {
            return Err("builtin character revision must be zero".to_string());
        }
    } else {
        let record = index
            .packs
            .iter()
            .find(|record| record.id == index.selected.id)
            .ok_or_else(|| "selected character pack is not registered".to_string())?;
        if !record.revisions.contains(&index.selected.revision) {
            return Err("selected character revision is not retained".to_string());
        }
    }
    if index.operations.len() > MAX_OPERATIONS {
        return Err("character registry contains too many retained operations".to_string());
    }
    let mut operation_ids = BTreeSet::new();
    for operation in &index.operations {
        validate_operation_id(&operation.operation_id)?;
        if !operation_ids.insert(operation.operation_id.clone()) {
            return Err("character registry contains duplicate operation ids".to_string());
        }
        if !matches!(
            operation.state.as_str(),
            "accepted"
                | "preparing"
                | "completed"
                | "failed"
                | "canceled"
                | "committed_pending_apply"
                | "durability_unknown"
        ) || operation.state.len() > 64
            || operation
                .error
                .as_ref()
                .map(|error| error.len())
                .unwrap_or(0)
                > MAX_OPERATION_ERROR_BYTES
        {
            return Err("character registry operation metadata is invalid".to_string());
        }
    }
    let encoded = serde_json::to_vec(index)
        .map_err(|error| format!("registry cannot be encoded: {error}"))?;
    if encoded.len() > MAX_REGISTRY_BYTES {
        return Err("character registry exceeds the 1 MiB limit".to_string());
    }
    Ok(())
}

fn validate_character_ref(reference: &CharacterRef) -> Result<(), String> {
    validate_id(&reference.id)?;
    if reference.id == CharacterRef::builtin().id {
        if reference.revision != 0 {
            return Err("builtin character revision must be zero".to_string());
        }
    } else if reference.revision == 0 {
        return Err("managed character revision must be non-zero".to_string());
    }
    Ok(())
}

fn validate_managed_identity(pack: &ManagedPack) -> Result<(), String> {
    validate_pack_id(&pack.id)?;
    validate_pack_name(&pack.name)?;
    if pack.manifest.len() > MAX_READ_BYTES {
        return Err("managed character manifest exceeds the registry limit".to_string());
    }
    pack.validate_archive_names()?;
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("character id must be 1..64 lowercase ASCII bytes".to_string());
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err("character id must start with lowercase ASCII or a digit".to_string());
    }
    if !bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
    }) {
        return Err("character id contains an unsafe character".to_string());
    }
    Ok(())
}
fn validate_managed_id(id: &str) -> Result<(), String> {
    validate_id(id)?;
    if id == CharacterRef::builtin().id {
        return Err("builtin character cannot be managed".to_string());
    }
    Ok(())
}

fn validate_operation_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_OPERATION_ID_BYTES || id.chars().any(char::is_control) {
        return Err("operation id must be 1..128 bytes without controls".to_string());
    }
    Ok(())
}

fn trim_operations(operations: &mut Vec<PackOperation>) {
    if operations.len() > MAX_OPERATIONS {
        let keep_from = operations.len() - MAX_OPERATIONS;
        operations.drain(..keep_from);
    }
}

fn write_registry_pinned(
    root_file: &File,
    root: &Path,
    previous_bytes: Option<&[u8]>,
    index: &RegistryDisk,
) -> Result<(Vec<u8>, bool), String> {
    validate_registry(index)?;
    let bytes = serde_json::to_vec(index)
        .map_err(|error| format!("registry cannot be encoded: {error}"))?;
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err("character registry exceeds the 1 MiB limit".to_string());
    }
    ensure_registry_peak_budget(root_file, root, previous_bytes, &bytes)?;
    let mut unknown = false;
    if let Some(previous) = previous_bytes {
        unknown |= atomic_replace_pinned(
            root_file,
            root,
            &root.join(PREVIOUS_REGISTRY_FILE),
            previous,
            "registry-previous",
        )?;
    }
    unknown |= atomic_replace_pinned(
        root_file,
        root,
        &root.join(REGISTRY_FILE),
        &bytes,
        "registry",
    )?;
    Ok((bytes, unknown))
}

fn atomic_replace_pinned(
    root_file: &File,
    root_path: &Path,
    path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<bool, String> {
    let (parent, destination) = open_root_relative_parent(root_file, root_path, path, label)?;
    let destination_c = name_cstring(&destination, label)?;
    if let Some(existing) = stat_at(&parent, &destination, label)? {
        let file_type = existing.st_mode & libc::S_IFMT;
        if file_type != libc::S_IFREG
            || existing.st_uid != unsafe { libc::geteuid() as libc::uid_t }
            || existing.st_nlink != 1
        {
            return Err(format!(
                "{label} path must be a regular file owned by the current user"
            ));
        }
    }
    let temp_name = OsString::from(format!(".{label}.tmp-{}", unique_nonce()));
    let mut file = open_child_file(
        &parent,
        &temp_name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_TRUNC,
        0o600,
        label,
    )?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write temporary {label}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync temporary {label}: {error}"))?;
    drop(file);
    let temp_c = name_cstring(&temp_name, label)?;
    let rename_result = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temp_c.as_ptr(),
            parent.as_raw_fd(),
            destination_c.as_ptr(),
        )
    };
    if rename_result != 0 {
        let _ = unlink_child(&parent, &temp_name, 0, label);
        return Err(format!(
            "cannot replace {label}: {}",
            io::Error::last_os_error()
        ));
    }
    match parent.sync_all() {
        Ok(()) => Ok(false),
        Err(_) => Ok(true),
    }
}

fn rename_owned_noreplace_pinned(
    root_file: &File,
    root_path: &Path,
    old: &Path,
    new: &Path,
    label: &str,
) -> Result<(), String> {
    let (old_parent, old_name) = open_root_relative_parent(root_file, root_path, old, label)?;
    let (new_parent, new_name) = open_root_relative_parent(root_file, root_path, new, label)?;
    verify_private_directory(
        &old_parent,
        old.parent().unwrap_or_else(|| Path::new("<store root>")),
    )?;
    verify_private_directory(
        &new_parent,
        new.parent().unwrap_or_else(|| Path::new("<store root>")),
    )?;
    let source = open_child_file(
        &old_parent,
        &old_name,
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
        label,
    )?;
    verify_private_directory(&source, old)?;
    let old_c = name_cstring(&old_name, label)?;
    let new_c = name_cstring(&new_name, label)?;
    #[cfg(target_os = "macos")]
    let result = unsafe {
        renameatx_np(
            old_parent.as_raw_fd(),
            old_c.as_ptr(),
            new_parent.as_raw_fd(),
            new_c.as_ptr(),
            0x0000_0004,
        )
    };
    #[cfg(not(target_os = "macos"))]
    let result: libc::c_int = -1;
    if result != 0 {
        return Err(format!(
            "cannot publish {label} without replacement: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}
fn rename_owned_noreplace_at(
    old_parent: &File,
    old_name: &std::ffi::OsStr,
    new_parent: &File,
    new_name: &std::ffi::OsStr,
    label: &str,
) -> Result<(), String> {
    let Some(metadata) = stat_at(old_parent, old_name, label)? else {
        return Err(format!("{label} source is missing"));
    };
    let file_type = metadata.st_mode & libc::S_IFMT;
    if file_type != libc::S_IFDIR || metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t } {
        return Err(format!("{label} source is not a private directory"));
    }
    let source = open_directory_at(old_parent, old_name, label)?;
    verify_private_directory(&source, Path::new("<owned source>"))?;
    let old_c = name_cstring(old_name, label)?;
    let new_c = name_cstring(new_name, label)?;
    #[cfg(target_os = "macos")]
    let result = unsafe {
        renameatx_np(
            old_parent.as_raw_fd(),
            old_c.as_ptr(),
            new_parent.as_raw_fd(),
            new_c.as_ptr(),
            0x0000_0004,
        )
    };
    #[cfg(not(target_os = "macos"))]
    let result: libc::c_int = -1;
    if result != 0 {
        return Err(format!(
            "cannot publish {label} without replacement: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn ensure_registry_peak_budget(
    root_file: &File,
    root: &Path,
    previous_bytes: Option<&[u8]>,
    next_bytes: &[u8],
) -> Result<(), String> {
    let used = measure_store(root_file, root)?;
    let reservation = u64::try_from(next_bytes.len())
        .map_err(|_| "registry size overflow".to_string())?
        .checked_add(
            previous_bytes
                .map(|bytes| u64::try_from(bytes.len()))
                .transpose()
                .map_err(|_| "registry size overflow".to_string())?
                .unwrap_or(0),
        )
        .ok_or_else(|| "character store size overflow".to_string())?;
    if used
        .checked_add(reservation)
        .ok_or_else(|| "character store size overflow".to_string())?
        > MAX_STORE_BYTES
    {
        return Err("character store exceeds the 256 MiB limit".to_string());
    }
    Ok(())
}

fn read_open_regular(file: File, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
    let opened = file
        .metadata()
        .map_err(|error| format!("{label} metadata cannot be read: {error}"))?;
    if !opened.is_file()
        || opened.uid() != unsafe { libc::geteuid() }
        || opened.nlink() != 1
        || opened.permissions().mode() & 0o077 != 0
    {
        return Err(format!("{label} must be a private regular file"));
    }
    if opened.len() > max_bytes as u64 {
        return Err(format!("{label} exceeds its limit"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label} cannot be read: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{label} exceeds its limit"));
    }
    Ok(bytes)
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 255
        || filename.contains('/')
        || filename.contains('\\')
        || filename.chars().any(char::is_control)
    {
        return Err("managed payload filename is not a safe same-directory name".to_string());
    }
    let mut components = Path::new(filename).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("managed payload filename must contain one normal path component".to_string()),
    }
}

fn managed_size(pack: &ManagedPack) -> Result<u64, String> {
    let manifest_bytes =
        u64::try_from(pack.manifest.len()).map_err(|_| "managed pack is too large".to_string())?;
    if pack.manifest.len() > MAX_READ_BYTES {
        return Err("managed character manifest exceeds the limit".to_string());
    }
    let mut payload_total = 0u64;
    for payload in &pack.payloads {
        if payload.bytes.len() > MAX_FILE_BYTES {
            return Err(format!(
                "managed payload {} exceeds the file limit",
                payload.name
            ));
        }
        payload_total = payload_total
            .checked_add(
                u64::try_from(payload.bytes.len())
                    .map_err(|_| "managed pack is too large".to_string())?,
            )
            .ok_or_else(|| "managed pack size overflow".to_string())?;
    }
    if payload_total > pack.aggregate_limit {
        return Err("managed payloads exceed the aggregate limit".to_string());
    }
    manifest_bytes
        .checked_add(payload_total)
        .ok_or_else(|| "managed pack size overflow".to_string())
}

fn create_stage_dir_pinned(
    root_file: &File,
    root: &Path,
    operation_id: &str,
) -> Result<PathBuf, String> {
    let staging = root.join(STAGING_DIR);
    let staging_directory =
        open_root_relative_directory(root_file, root, &staging, "staging directory")?;
    let safe_operation: String = operation_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    let stage_name = OsString::from(format!(".stage-{safe_operation}-{}", unique_nonce()));
    let c_name = name_cstring(&stage_name, "staging directory")?;
    let result = unsafe { libc::mkdirat(staging_directory.as_raw_fd(), c_name.as_ptr(), 0o700) };
    if result != 0 {
        return Err(format!(
            "cannot create staging directory: {}",
            io::Error::last_os_error()
        ));
    }
    let stage = open_directory_at(&staging_directory, &stage_name, "staging directory")?;
    verify_private_directory(&stage, &staging.join(&stage_name))?;
    Ok(staging.join(stage_name))
}
fn ensure_pack_directory_pinned(root_file: &File, root: &Path, id: &str) -> Result<(), String> {
    let packs = open_root_relative_directory(
        root_file,
        root,
        &root.join(PACKS_DIR),
        "character packs directory",
    )?;
    let directory = open_or_create_directory_at(
        &packs,
        std::ffi::OsStr::new(id),
        "managed character directory",
    )?;
    verify_private_directory(&directory, &root.join(PACKS_DIR).join(id))
}

fn write_new_file_pinned(
    root_file: &File,
    root: &Path,
    path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<(), String> {
    let (directory, name) = open_root_relative_parent(root_file, root, path, label)?;
    let mut file = open_child_file(
        &directory,
        &name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_TRUNC,
        0o600,
        label,
    )?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write {label}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync {label}: {error}"))
}

fn write_managed_to_stage_pinned(
    root_file: &File,
    root: &Path,
    stage: &Path,
    managed: &ManagedPack,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for payload in &managed.payloads {
        validate_filename(&payload.name)?;
        if !seen.insert(payload.name.as_str()) {
            return Err(format!(
                "managed character filename {} is referenced more than once",
                payload.name
            ));
        }
    }
    write_new_file_pinned(
        root_file,
        root,
        &stage.join("manifest.json"),
        &managed.manifest,
        "staged manifest",
    )?;
    for payload in &managed.payloads {
        write_new_file_pinned(
            root_file,
            root,
            &stage.join(&payload.name),
            payload.bytes.as_slice(),
            "staged payload",
        )?;
    }
    sync_directory_pinned(root_file, root, stage)
}

fn revision_path(root: &Path, id: &str, revision: u64) -> PathBuf {
    root.join(PACKS_DIR).join(id).join(revision.to_string())
}

fn preview_next_revision(root_file: &File, index: &RegistryDisk) -> Result<u64, String> {
    let max_existing = scan_max_revision(root_file)?;
    let candidate = index.next_revision.max(max_existing.saturating_add(1));
    if candidate == 0 || candidate == u64::MAX {
        return Err("global character revision allocator is exhausted".to_string());
    }
    Ok(candidate)
}

fn scan_max_revision(root_file: &File) -> Result<u64, String> {
    let mut max_revision = 0u64;
    let mut budget = ScanBudget::default();
    let packs = open_directory_at(
        root_file,
        std::ffi::OsStr::new(PACKS_DIR),
        "character packs directory",
    )?;
    for id_name in read_dir_bounded(&packs, "character pack directory")? {
        budget.visit()?;
        let Some(metadata) = stat_at(&packs, &id_name, "character pack")? else {
            continue;
        };
        let file_type = metadata.st_mode & libc::S_IFMT;
        if file_type == libc::S_IFLNK
            || file_type != libc::S_IFDIR
            || metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t }
        {
            continue;
        }
        let id_directory = open_directory_at(&packs, &id_name, "character pack")?;
        verify_private_directory(&id_directory, Path::new("<character pack>"))?;
        for revision_name in read_dir_bounded(&id_directory, "character revision directory")? {
            budget.visit()?;
            let Some(metadata) = stat_at(&id_directory, &revision_name, "character revision")?
            else {
                continue;
            };
            let file_type = metadata.st_mode & libc::S_IFMT;
            if file_type == libc::S_IFLNK
                || file_type != libc::S_IFDIR
                || metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t }
            {
                continue;
            }
            if let Some(value) = revision_name
                .to_str()
                .and_then(|name| name.parse::<u64>().ok())
            {
                max_revision = max_revision.max(value);
            }
        }
    }
    let trash = open_directory_at(
        root_file,
        std::ffi::OsStr::new(TRASH_DIR),
        "character trash directory",
    )?;
    // Only quarantine_orphans creates trash entries, always as top-level
    // `.trash-{id}-{revision}-{nonce}`. Ids and payload names may contain
    // digits, so only the revision field is read and nothing is recursed into.
    for name in read_dir_bounded(&trash, "revision history directory")? {
        budget.visit()?;
        let Some(fields) = name.to_str().and_then(|name| name.strip_prefix(".trash-")) else {
            continue;
        };
        let mut parts = fields.rsplitn(3, '-');
        let (Some(_nonce), Some(revision), Some(id)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        if let Ok(value) = revision.parse::<u64>() {
            max_revision = max_revision.max(value);
        }
    }
    Ok(max_revision)
}

fn measure_store(root_file: &File, root: &Path) -> Result<u64, String> {
    let mut budget = ScanBudget::default();
    measure_tree(root_file, root, 0, &mut budget)
}

fn measure_tree(
    directory: &File,
    path: &Path,
    depth: usize,
    budget: &mut ScanBudget,
) -> Result<u64, String> {
    if depth > 32 {
        return Err("character store directory nesting is too deep".to_string());
    }
    let mut total = 0u64;
    for name in read_dir_bounded(directory, "character store directory")? {
        budget.visit()?;
        let Some(metadata) = stat_at(directory, &name, "character store entry")? else {
            continue;
        };
        let file_type = metadata.st_mode & libc::S_IFMT;
        if file_type == libc::S_IFLNK {
            return Err(format!(
                "character store path {} is a symlink",
                path.join(&name).display()
            ));
        }
        if file_type == libc::S_IFREG {
            if metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t } {
                return Err(format!(
                    "character store file {} is not owned by the current user",
                    path.join(&name).display()
                ));
            }
            total = total
                .checked_add(metadata.st_size as u64)
                .ok_or_else(|| "character store size overflow".to_string())?;
        } else if file_type == libc::S_IFDIR {
            let child = open_directory_at(directory, &name, "character store directory")?;
            verify_private_directory(&child, &path.join(&name))?;
            total = total
                .checked_add(measure_tree(&child, &path.join(&name), depth + 1, budget)?)
                .ok_or_else(|| "character store size overflow".to_string())?;
        } else {
            return Err(format!(
                "character store path {} is not regular",
                path.join(&name).display()
            ));
        }
        if total > MAX_STORE_BYTES {
            return Ok(total);
        }
    }
    Ok(total)
}

fn read_dir_bounded(directory: &File, label: &str) -> Result<Vec<OsString>, String> {
    directory_names(directory, label)
}

fn recover_owned(
    root_file: &File,
    root: &Path,
    loaded: &LoadedRegistry,
    active: Option<&CharacterRef>,
) -> Result<(), String> {
    let mut budget = ScanBudget::default();
    let staging = open_directory_at(
        root_file,
        std::ffi::OsStr::new(STAGING_DIR),
        "character staging directory",
    )?;
    for name in read_dir_bounded(&staging, "character staging directory")? {
        budget.visit()?;
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if name_text.starts_with(".stage-") {
            let Some(metadata) = stat_at(&staging, &name, "staging entry")? else {
                continue;
            };
            if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK {
                let _ = remove_entry_at(&staging, &name, &mut budget);
            }
        }
    }
    for name in read_dir_bounded(root_file, "character store root")? {
        budget.visit()?;
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if name_text.starts_with(".registry.tmp-")
            || name_text.starts_with(".registry-previous.tmp-")
        {
            let Some(metadata) = stat_at(root_file, &name, "registry temporary")? else {
                continue;
            };
            if metadata.st_mode & libc::S_IFMT == libc::S_IFREG {
                let _ = remove_entry_at(root_file, &name, &mut budget);
            }
        }
    }

    // Never classify an orphan while either index slot is untrusted. A move
    // to trash preserves bytes, but still changes namespace ownership.
    if loaded.readonly
        || loaded.preserve_orphans
        || !loaded.current_valid
        || (!loaded.backup_valid && loaded.backup_bytes.is_some())
    {
        return Ok(());
    }
    let mut protected = BTreeSet::new();
    collect_references(&loaded.index, &mut protected);
    if let Some(active) = active {
        if active.id != CharacterRef::builtin().id {
            protected.insert((active.id.clone(), active.revision));
        }
    }
    if let Some(previous) = loaded.backup_bytes.as_ref() {
        if let Ok(index) = serde_json::from_slice::<RegistryDisk>(previous) {
            if validate_registry(&index).is_ok() {
                collect_references(&index, &mut protected);
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }
    quarantine_orphans(root_file, root, &protected, &mut budget)
}

fn collect_references(index: &RegistryDisk, protected: &mut BTreeSet<(String, u64)>) {
    for record in &index.packs {
        for revision in &record.revisions {
            protected.insert((record.id.clone(), *revision));
        }
    }
}
fn quarantine_orphans(
    root_file: &File,
    root: &Path,
    protected: &BTreeSet<(String, u64)>,
    budget: &mut ScanBudget,
) -> Result<(), String> {
    let packs = open_directory_at(
        root_file,
        std::ffi::OsStr::new(PACKS_DIR),
        "character packs directory",
    )?;
    let trash = open_directory_at(
        root_file,
        std::ffi::OsStr::new(TRASH_DIR),
        "character trash directory",
    )?;
    for id_name in read_dir_bounded(&packs, "character packs directory")? {
        budget.visit()?;
        let Some(id) = id_name.to_str() else { continue };
        if validate_id(id).is_err() || id == CharacterRef::builtin().id {
            continue;
        }
        let Some(metadata) = stat_at(&packs, &id_name, "character pack")? else {
            continue;
        };
        if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK
            || metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            continue;
        }
        let id_directory = open_directory_at(&packs, &id_name, "character pack")?;
        verify_private_directory(&id_directory, &root.join(PACKS_DIR).join(id))?;
        for revision_name in read_dir_bounded(&id_directory, "character revisions")? {
            budget.visit()?;
            let Some(revision_name_text) = revision_name.to_str() else {
                continue;
            };
            let Ok(revision) = revision_name_text.parse::<u64>() else {
                continue;
            };
            if protected.contains(&(id.to_string(), revision)) {
                continue;
            }
            let Some(metadata) = stat_at(&id_directory, &revision_name, "orphan revision")? else {
                continue;
            };
            if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK
                || metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
            {
                continue;
            }
            let revision_directory =
                open_directory_at(&id_directory, &revision_name, "orphan revision")?;
            verify_private_directory(
                &revision_directory,
                &root.join(PACKS_DIR).join(id).join(revision_name_text),
            )?;
            if !known_revision_tree(&revision_directory, budget) {
                continue;
            }
            let destination = OsString::from(format!(".trash-{id}-{revision}-{}", unique_nonce()));
            if rename_owned_noreplace_at(
                &id_directory,
                &revision_name,
                &trash,
                &destination,
                "orphan revision",
            )
            .is_ok()
            {
                let _ = id_directory.sync_all();
                let _ = trash.sync_all();
                let _ = packs.sync_all();
            }
        }
    }
    Ok(())
}

fn known_revision_tree(directory: &File, budget: &mut ScanBudget) -> bool {
    let manifest = match open_child_file(
        directory,
        std::ffi::OsStr::new("manifest.json"),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
        "trash manifest",
    )
    .and_then(|file| read_open_regular(file, MAX_READ_BYTES, "trash manifest"))
    {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    // Reuse the exact strict v2/v3 parser used by import/restore.  In
    // particular, this keeps GC from inventing a second manifest schema and
    // lets v3's shared frame names describe a pathname set rather than a
    // fixed four-pose tree.
    let filenames = match managed_manifest_files(&manifest) {
        Ok(filenames) => filenames,
        Err(_) => return false,
    };
    let mut allowed = BTreeSet::new();
    allowed.insert("manifest.json".to_string());
    for filename in filenames {
        if filename == "manifest.json"
            || validate_filename(&filename).is_err()
            || !allowed.insert(filename)
        {
            // A repeated v2 filename is intentionally treated as an
            // ambiguous/unrecognized tree.  v3 shared references are
            // normalized to unique first-occurrence filenames by Assets.
            return false;
        }
    }

    let entries = match read_dir_bounded(directory, "revision tree") {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    // Account for every enumerated entry even when cardinality is already
    // wrong.  Otherwise a forest of malformed trees could evade the global
    // scan bound merely by failing this check early.
    if entries.iter().any(|_| budget.visit().is_err()) {
        return false;
    }
    // Comparing cardinality as well as names rejects both truncated trees and
    // trees with unreferenced files.  Shared v3 references were already
    // normalized by the strict Assets helper; duplicate legacy names were
    // rejected above and therefore remain unrecognized.
    if entries.len() != allowed.len() {
        return false;
    }
    entries.into_iter().all(|name| {
        let Some(name_text) = name.to_str() else {
            return false;
        };
        if !allowed.contains(name_text) {
            return false;
        }

        // stat_at is intentionally not enough here: an attacker can replace
        // a pathname after the lstat.  Open each entry with O_NOFOLLOW and
        // validate the descriptor that is actually being classified.  GC
        // never reads or decodes PNG bytes.
        let file = match open_child_file(
            directory,
            &name,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
            "revision tree entry",
        ) {
            Ok(file) => file,
            Err(_) => return false,
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(_) => return false,
        };
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.nlink() == 1
            && metadata.permissions().mode() & 0o077 == 0
    })
}

fn remove_owned_tree_pinned(root_file: &File, root_path: &Path, path: &Path) -> Result<(), String> {
    let (parent, name) = open_root_relative_parent(root_file, root_path, path, "owned tree")?;
    let mut budget = ScanBudget::default();
    remove_entry_at(&parent, &name, &mut budget)
}

fn gc_owned(
    root_file: &File,
    root: &Path,
    index: &RegistryDisk,
    active: Option<&CharacterRef>,
) -> Result<(), String> {
    if index.preserve_orphans {
        return Ok(());
    }
    let mut budget = ScanBudget::default();
    let mut protected = BTreeSet::new();
    collect_references(index, &mut protected);
    if let Some(active) = active {
        if active.id != CharacterRef::builtin().id {
            protected.insert((active.id.clone(), active.revision));
        }
    }
    let previous = read_registry_slot(root_file, PREVIOUS_REGISTRY_FILE)?
        .ok_or_else(|| "registry backup checkpoint is missing".to_string())?;
    collect_references(&previous.0, &mut protected);
    quarantine_orphans(root_file, root, &protected, &mut budget)?;

    let trash = open_directory_at(
        root_file,
        std::ffi::OsStr::new(TRASH_DIR),
        "character trash directory",
    )?;
    for name in read_dir_bounded(&trash, "character trash directory")? {
        budget.visit()?;
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if !name_text.starts_with(".trash-") {
            continue;
        }
        let Some(metadata) = stat_at(&trash, &name, "trash entry")? else {
            continue;
        };
        if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK
            || metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
        {
            continue;
        }
        let trash_tree = open_directory_at(&trash, &name, "trash entry")?;
        verify_private_directory(&trash_tree, Path::new("<trash entry>"))?;
        // Trash is still untrusted same-user filesystem state.  Only delete
        // trees that retain the exact strict manifest/file-set shape accepted
        // on the packs side; unknown or ambiguous trees remain preserved.
        if !known_revision_tree(&trash_tree, &mut budget) {
            continue;
        }
        remove_entry_at(&trash, &name, &mut budget)?;
    }
    trash.sync_all().map_err(|error| {
        format!(
            "cannot sync directory {}: {error}",
            root.join(TRASH_DIR).display()
        )
    })
}

fn directory_names(directory: &File, label: &str) -> Result<Vec<OsString>, String> {
    let dot = CString::new(".").map_err(|_| format!("{label} contains an invalid dot name"))?;
    // `dup` would share the directory's open-file description and therefore
    // its seek offset. Open "." relative to the directory descriptor instead.
    let duplicate = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            dot.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if duplicate < 0 {
        return Err(format!(
            "cannot open independent {label} descriptor: {}",
            io::Error::last_os_error()
        ));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err(format!(
            "cannot enumerate {label}: {}",
            io::Error::last_os_error()
        ));
    }
    let mut names = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        names.push(OsString::from_vec(name.to_bytes().to_vec()));
        if names.len() > MAX_STAGE_ENTRIES {
            unsafe {
                libc::closedir(stream);
            }
            return Err(format!("{label} has too many entries"));
        }
    }
    unsafe {
        libc::closedir(stream);
    }
    names.sort();
    Ok(names)
}

fn remove_entry_at(
    parent: &File,
    name: &std::ffi::OsStr,
    budget: &mut ScanBudget,
) -> Result<(), String> {
    let c_name = name_cstring(name, "owned entry")?;
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            c_name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(format!(
            "cannot inspect owned entry: {}",
            io::Error::last_os_error()
        ));
    }
    let file_type = metadata.st_mode & libc::S_IFMT;
    if file_type == libc::S_IFLNK {
        return Err("refusing to traverse symlink in owned tree".to_string());
    }
    if metadata.st_uid != unsafe { libc::geteuid() as libc::uid_t } {
        return Err("refusing to remove foreign-owned entry".to_string());
    }
    if file_type == libc::S_IFREG {
        return unlink_child(parent, name, 0, "owned file");
    }
    if file_type != libc::S_IFDIR {
        return Err("cannot remove non-regular owned path".to_string());
    }

    let child = open_directory_at(parent, name, "owned directory")?;
    verify_private_directory(&child, Path::new("<owned directory>"))?;
    for child_name in directory_names(&child, "owned directory")? {
        budget.visit()?;
        remove_entry_at(&child, &child_name, budget)?;
    }
    unlink_child(parent, name, libc::AT_REMOVEDIR, "owned directory")
}

fn unique_nonce() -> u128 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = NONCE.fetch_add(1, Ordering::Relaxed) as u128;
    (std::process::id() as u128) << 96 ^ time ^ counter
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_png_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut bytes), 384, 512);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![255u8; 384 * 512]).unwrap();
        }
        bytes
    }

    /// A 384x512 grayscale PNG of xorshift noise, so it barely compresses.
    fn noisy_png_bytes() -> Vec<u8> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let pixels: Vec<u8> = (0..384 * 512)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut bytes), 384, 512);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&pixels).unwrap();
        }
        bytes
    }

    fn write_single_frame_v3_pack(source: &Path, id: &str, frame: &[u8]) {
        fs::create_dir_all(source).unwrap();
        let manifest = serde_json::json!({
            "version": 3,
            "format": "herdr.character",
            "id": id,
            "name": "Fixture Pack",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["frame.png"]},
                "running": {"fps": 8, "frames": ["frame.png"]},
                "waiting": {"fps": 8, "frames": ["frame.png"]},
                "unknown": {"fps": 8, "frames": ["frame.png"]}
            }
        });
        write_fixture_file(
            &source.join("manifest.json"),
            &serde_json::to_vec(&manifest).unwrap(),
        );
        write_fixture_file(&source.join("frame.png"), frame);
    }

    fn write_fixture_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn corrupt_both_registry_slots_is_read_only_without_deleting_data() {
        let root = std::env::temp_dir().join(format!("herdr-store-test-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        fs::write(store_root.join(REGISTRY_FILE), b"not-json").unwrap();
        fs::write(store_root.join(PREVIOUS_REGISTRY_FILE), b"also-not-json").unwrap();
        let marker = store_root.join(PACKS_DIR).join("unknown");
        fs::create_dir_all(&marker).unwrap();
        fs::write(marker.join("keep"), b"data").unwrap();
        let loaded = load_registry(&layout.root_file).unwrap();
        assert!(loaded.readonly);
        recover_owned(&layout.root_file, &store_root, &loaded, None).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn builtin_dialogue_survives_readonly_registry_without_bypassing_reference_or_manifest_checks()
    {
        let root = std::env::temp_dir().join(format!("herdr-store-dialogue-{}", unique_nonce()));
        let store_root = root.join(STORE_DIR);
        ensure_layout(&root, &store_root).unwrap();
        let builtin_root = root.join("builtin");
        fs::create_dir(&builtin_root).unwrap();
        let manifest = builtin_root.join("manifest.json");
        write_fixture_file(
            &manifest,
            include_bytes!("../../assets/rubelia-default/manifest.json"),
        );

        let primary = b"invalid-primary-registry";
        let previous = b"different-invalid-previous-registry";
        let primary_path = store_root.join(REGISTRY_FILE);
        let previous_path = store_root.join(PREVIOUS_REGISTRY_FILE);
        fs::write(&primary_path, primary).unwrap();
        fs::write(&previous_path, previous).unwrap();
        let store = PackStore::new(root.clone(), Some(builtin_root.clone()));
        let listing = store.list().unwrap();
        assert_eq!(listing.generation, 0);
        assert_eq!(listing.selected, CharacterRef::builtin());
        assert!(listing.packs.is_empty());
        assert!(listing.error.is_some());

        let (name, metadata) = store
            .dialogue_metadata(&CharacterRef::builtin(), 0)
            .unwrap();
        assert_eq!(name, "Rubelia");
        assert_eq!(
            metadata.unwrap().dialogue_text("idle", None, "ko"),
            Some("루벨리아가 차분히 다음 일을 살피고 있어요.")
        );
        let stale_error = store
            .dialogue_metadata(&CharacterRef::builtin(), 1)
            .unwrap_err();
        let managed_error = store
            .dialogue_metadata(
                &CharacterRef {
                    id: "sample".to_string(),
                    revision: 1,
                },
                0,
            )
            .unwrap_err();
        assert_ne!(stale_error, managed_error);
        let invalid_ref_error = store
            .dialogue_metadata(
                &CharacterRef {
                    id: CharacterRef::builtin().id,
                    revision: 1,
                },
                0,
            )
            .unwrap_err();
        assert_ne!(invalid_ref_error, managed_error);
        assert_ne!(invalid_ref_error, stale_error);

        fs::remove_file(&manifest).unwrap();
        let missing_manifest_error = store
            .dialogue_metadata(&CharacterRef::builtin(), 0)
            .unwrap_err();
        assert_ne!(missing_manifest_error, managed_error);
        write_fixture_file(&manifest, b"{not valid json");
        let invalid_manifest_error = store
            .dialogue_metadata(&CharacterRef::builtin(), 0)
            .unwrap_err();
        assert_ne!(invalid_manifest_error, managed_error);
        assert_ne!(invalid_manifest_error, missing_manifest_error);
        assert_eq!(fs::read(&primary_path).unwrap(), primary);
        assert_eq!(fs::read(&previous_path).unwrap(), previous);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_operation_history_stays_bounded_before_atomic_replace() {
        let mut index = RegistryDisk::empty();
        let mut operation = PackOperation {
            operation_id: "op".to_string(),
            state: "completed".to_string(),
            committed: true,
            ui_applied: true,
            generation: Some(1),
            error: Some("x".repeat(MAX_OPERATION_ERROR_BYTES)),
        };
        index.operations = (0..MAX_OPERATIONS)
            .map(|number| {
                operation.operation_id = format!("operation-{number}");
                operation.clone()
            })
            .collect();
        assert!(validate_registry(&index).is_ok());
        assert!(serde_json::to_vec(&index).unwrap().len() <= MAX_REGISTRY_BYTES);
    }

    #[test]
    fn revision_preview_uses_existing_namespace_without_mutating_registry() {
        let root = std::env::temp_dir().join(format!("herdr-store-revision-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let path = revision_path(&store_root, "pack", 41);
        fs::create_dir_all(&path).unwrap();
        let mut index = RegistryDisk::empty();
        index.next_revision = 2;
        assert_eq!(
            preview_next_revision(&layout.root_file, &index).unwrap(),
            42
        );
        assert_eq!(index.next_revision, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn revision_preview_reads_only_trash_revision_fields() {
        let root =
            std::env::temp_dir().join(format!("herdr-store-trash-revision-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let trash = root.join(STORE_DIR).join(TRASH_DIR);
        // An all-digit id and digit-only payload names must not be read as
        // revisions; only the `{revision}` field of the top-level name counts.
        let numeric = trash.join(".trash-18446744073709551613-41-123");
        let dashed = trash.join(".trash-my-pack-2-7-999");
        for directory in [&numeric, &dashed] {
            fs::create_dir_all(directory).unwrap();
            let mut permissions = fs::metadata(directory).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(directory, permissions).unwrap();
        }
        write_fixture_file(&numeric.join("18446744073709551000.png"), b"payload");
        let mut index = RegistryDisk::empty();
        index.next_revision = 2;
        assert_eq!(
            preview_next_revision(&layout.root_file, &index).unwrap(),
            42
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_primary_preserves_unlisted_revision_across_recovery_write() {
        let root = std::env::temp_dir().join(format!("herdr-store-recovery-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let previous = RegistryDisk::empty();
        let bytes = serde_json::to_vec(&previous).unwrap();
        fs::write(store_root.join(PREVIOUS_REGISTRY_FILE), &bytes).unwrap();
        fs::write(store_root.join(REGISTRY_FILE), b"corrupt").unwrap();
        let orphan = revision_path(&store_root, "pack", 77);
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("unknown"), b"preserve").unwrap();

        let loaded = load_registry(&layout.root_file).unwrap();
        assert!(loaded.preserve_orphans);
        recover_owned(&layout.root_file, &store_root, &loaded, None).unwrap();

        let mut next = loaded.index.clone();
        next.generation += 1;
        next.preserve_orphans |= loaded.preserve_orphans;
        let _ = write_registry_pinned(
            &layout.root_file,
            &store_root,
            loaded.bytes.as_deref(),
            &next,
        )
        .unwrap();
        let after = load_registry(&layout.root_file).unwrap();
        assert!(after.index.preserve_orphans);
        recover_owned(&layout.root_file, &store_root, &after, None).unwrap();
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn store_budget_counts_trash_bytes_before_new_staging() {
        let root = std::env::temp_dir().join(format!("herdr-store-budget-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let file_path = root.join(STORE_DIR).join(TRASH_DIR).join(".trash-budget");
        let file = File::create(&file_path).unwrap();
        file.set_len(MAX_STORE_BYTES + 1).unwrap();
        assert!(measure_store(&layout.root_file, &store_root).unwrap() > MAX_STORE_BYTES);

        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        let manifest = br#"{
            "version": 3,
            "format": "herdr.character",
            "id": "budget-pack",
            "name": "Budget Pack",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["frame.png"]},
                "running": {"fps": 8, "frames": ["frame.png"]},
                "waiting": {"fps": 8, "frames": ["frame.png"]},
                "unknown": {"fps": 8, "frames": ["frame.png"]}
            }
        }"#;
        write_fixture_file(&source.join("manifest.json"), manifest);
        write_fixture_file(&source.join("frame.png"), &test_png_bytes());
        let store = PackStore::new(root.clone(), Some(source.clone()));
        let error = store
            .begin(
                &PackRequest {
                    operation_id: "budget-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import { path: source },
                },
                None,
            )
            .err()
            .expect("workspace exhaustion must reject new staging");
        assert!(error.contains("256 MiB"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn select_and_restore_reuse_installed_revision_without_staging_budget() {
        let root = std::env::temp_dir().join(format!("herdr-store-reuse-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let source = root.join("source");
        let frame = noisy_png_bytes();
        assert!(frame.len() > 64 * 1024);
        write_single_frame_v3_pack(&source, "reuse-pack", &frame);
        let store = PackStore::new(root.clone(), Some(source.clone()));
        let mut import = store
            .begin(
                &PackRequest {
                    operation_id: "reuse-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import { path: source },
                },
                None,
            )
            .unwrap();
        let reference = import.candidate_ref.clone().unwrap();
        import.commit().unwrap();
        import.finish(true).unwrap();
        drop(import);

        // Leave less headroom than one more copy of the installed pack.
        let used = measure_store(&layout.root_file, &store_root).unwrap();
        let filler_path = store_root.join(TRASH_DIR).join(".trash-budget");
        let filler = File::create(&filler_path).unwrap();
        filler.set_len(MAX_STORE_BYTES - used - 64 * 1024).unwrap();

        for (operation_id, action) in [
            (
                "reuse-select",
                PackAction::Select {
                    id: reference.id.clone(),
                },
            ),
            (
                "reuse-restore",
                PackAction::Restore {
                    id: reference.id.clone(),
                    revision: reference.revision,
                },
            ),
        ] {
            let mut transaction = store
                .begin(
                    &PackRequest {
                        operation_id: operation_id.to_string(),
                        expected_generation: None,
                        action,
                    },
                    None,
                )
                .unwrap();
            assert!(transaction.stage_dir.is_none());
            assert_eq!(transaction.candidate_ref.as_ref(), Some(&reference));
            transaction.commit().unwrap();
            transaction.finish(true).unwrap();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_commit_leaves_registry_and_revision_namespace_unchanged() {
        let root =
            std::env::temp_dir().join(format!("herdr-store-commit-failure-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let mut index = RegistryDisk::empty();
        index.generation = u64::MAX;
        write_registry_pinned(&layout.root_file, &store_root, None, &index).unwrap();
        let source = root.join("source");
        write_single_frame_v3_pack(&source, "overflow-pack", &test_png_bytes());
        let store = PackStore::new(root.clone(), Some(source.clone()));
        let mut transaction = store
            .begin(
                &PackRequest {
                    operation_id: "overflow-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import { path: source },
                },
                None,
            )
            .unwrap();
        let reference = transaction.candidate_ref.clone().unwrap();
        let error = transaction
            .commit()
            .expect_err("generation overflow must fail the commit");
        assert!(error.contains("generation overflow"), "{error}");
        drop(transaction);

        let loaded = load_registry(&layout.root_file).unwrap();
        assert_eq!(loaded.index.generation, u64::MAX);
        assert!(loaded.index.packs.is_empty());
        assert!(store.operation_status("overflow-import").unwrap().is_none());
        assert!(!revision_path(&store_root, &reference.id, reference.revision).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_rejects_v3_frame_names_that_export_cannot_write() {
        let frame = test_png_bytes();
        for (case, (odd_name, expected)) in [
            ("대기.png", "ASCII"),
            (".idle.png", "root-only"),
            ("Idle.png", "case-colliding"),
        ]
        .into_iter()
        .enumerate()
        {
            let root =
                std::env::temp_dir().join(format!("herdr-store-export-names-{}", unique_nonce()));
            let source = root.join("source");
            fs::create_dir_all(&source).unwrap();
            let manifest = serde_json::json!({
                "version": 3,
                "format": "herdr.character",
                "id": "names",
                "name": "Names",
                "width": 384,
                "height": 512,
                "phases": {
                    "idle": {"fps": 8, "frames": ["idle.png", odd_name]},
                    "running": {"fps": 8, "frames": ["idle.png"]},
                    "waiting": {"fps": 8, "frames": ["idle.png"]},
                    "unknown": {"fps": 8, "frames": ["idle.png"]}
                }
            });
            write_fixture_file(
                &source.join("manifest.json"),
                &serde_json::to_vec(&manifest).unwrap(),
            );
            // On a case-insensitive volume both names resolve to one file.
            for filename in ["idle.png", odd_name] {
                write_fixture_file(&source.join(filename), &frame);
            }
            let store = PackStore::new(root.join("config"), Some(source.clone()));
            let error = store
                .begin(
                    &PackRequest {
                        operation_id: format!("export-names-{case}"),
                        expected_generation: None,
                        action: PackAction::Import { path: source },
                    },
                    None,
                )
                .err()
                .expect("unexportable frame names must be rejected at import");
            assert!(error.contains(expected), "{odd_name}: {error}");
            assert_eq!(
                fs::read_dir(store.root.join(STAGING_DIR)).unwrap().count(),
                0
            );
            let _ = fs::remove_dir_all(root);
        }
    }
    #[test]
    fn pinned_registry_read_survives_store_root_path_replacement() {
        let root = std::env::temp_dir().join(format!("herdr-store-root-pin-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let index = RegistryDisk::empty();
        write_registry_pinned(&layout.root_file, &store_root, None, &index).unwrap();

        let moved = root.join("moved-store");
        fs::rename(&store_root, &moved).unwrap();
        fs::create_dir(&store_root).unwrap();
        fs::write(store_root.join(REGISTRY_FILE), b"attacker-controlled").unwrap();

        let loaded = load_registry(&layout.root_file).unwrap();
        assert!(!loaded.readonly);
        assert_eq!(loaded.index.generation, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn child_symlink_replacement_is_rejected_by_pinned_scan() {
        let root = std::env::temp_dir().join(format!("herdr-store-child-pin-{}", unique_nonce()));
        let outside = std::env::temp_dir().join(format!("herdr-store-outside-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let moved = root.join("moved-packs");
        fs::rename(store_root.join(PACKS_DIR), &moved).unwrap();
        fs::create_dir_all(outside.join(PACKS_DIR)).unwrap();
        std::os::unix::fs::symlink(outside.join(PACKS_DIR), store_root.join(PACKS_DIR)).unwrap();

        assert!(scan_max_revision(&layout.root_file).is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn repeated_descriptor_scans_account_for_each_tree_byte_once() {
        let root = std::env::temp_dir().join(format!("herdr-store-scan-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let nested = store_root.join(TRASH_DIR).join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("one"), b"one").unwrap();
        fs::write(nested.join("two"), b"twenty").unwrap();

        let first = measure_store(&layout.root_file, &store_root).unwrap();
        let second = measure_store(&layout.root_file, &store_root).unwrap();
        assert_eq!(first, 9);
        assert_eq!(second, first);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn maximum_revision_cardinality_fits_global_scan_budget() {
        let root = std::env::temp_dir().join(format!("herdr-store-cardinality-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let frame_names: Vec<String> = (0..32).map(|frame| format!("frame-{frame}.png")).collect();
        let manifest = serde_json::json!({
            "version": 3,
            "format": "herdr.character",
            "id": "maximum",
            "name": "Maximum",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": frame_names[0..8].to_vec()},
                "running": {"fps": 8, "frames": frame_names[8..16].to_vec()},
                "waiting": {"fps": 8, "frames": frame_names[16..24].to_vec()},
                "unknown": {"fps": 8, "frames": frame_names[24..32].to_vec()}
            }
        });
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let frame_bytes = test_png_bytes();
        for pack_number in 0..MAX_PACKS {
            let id = format!("pack-{pack_number}");
            let pack_directory = store_root.join(PACKS_DIR).join(&id);
            fs::create_dir_all(&pack_directory).unwrap();
            let mut permissions = fs::metadata(&pack_directory).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&pack_directory, permissions).unwrap();
            for revision in 1..=MAX_REVISIONS {
                let revision_directory = revision_path(&store_root, &id, revision as u64);
                fs::create_dir_all(&revision_directory).unwrap();
                let mut permissions = fs::metadata(&revision_directory).unwrap().permissions();
                permissions.set_mode(0o700);
                fs::set_permissions(&revision_directory, permissions).unwrap();
                write_fixture_file(&revision_directory.join("manifest.json"), &manifest_bytes);
                for filename in &frame_names {
                    write_fixture_file(&revision_directory.join(filename), &frame_bytes);
                }
            }
        }

        // 32 packs * 8 revisions * (manifest + 32 frame entries), plus
        // their directories, stays below the 16,384 tree-wide bound.
        let measured = measure_store(&layout.root_file, &store_root).unwrap();
        assert_eq!(
            measured,
            (MAX_PACKS * MAX_REVISIONS) as u64
                * (manifest_bytes.len() as u64 + 32 * frame_bytes.len() as u64)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v3_import_stages_shared_frames_once_through_store_transaction() {
        let root = std::env::temp_dir().join(format!("herdr-store-v3-import-{}", unique_nonce()));
        let config_dir = root.join("config");
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        let manifest = br#"{
            "version": 3,
            "format": "herdr.character",
            "id": "motion-demo",
            "name": "Motion Demo",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["shared.png", "idle.png"]},
                "running": {"fps": 12, "frames": ["shared.png"]},
                "waiting": {"fps": 6, "frames": ["wait.png", "shared.png"]},
                "unknown": {"fps": 1, "frames": ["shared.png"]}
            }
        }"#;
        let frame = test_png_bytes();
        write_fixture_file(&source.join("manifest.json"), manifest);
        for filename in ["shared.png", "idle.png", "wait.png"] {
            write_fixture_file(&source.join(filename), &frame);
        }

        let store = PackStore::new(config_dir, Some(source.clone()));
        let request = PackRequest {
            operation_id: "v3-import".to_string(),
            expected_generation: None,
            action: PackAction::Import {
                path: source.clone(),
            },
        };
        let mut transaction = store.begin(&request, None).unwrap();
        let stage = transaction.stage_dir.as_ref().unwrap();
        let mut names = fs::read_dir(stage)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec![
                "idle.png".to_string(),
                "manifest.json".to_string(),
                "shared.png".to_string(),
                "wait.png".to_string(),
            ]
        );
        for filename in ["shared.png", "idle.png", "wait.png"] {
            assert_eq!(fs::read(stage.join(filename)).unwrap(), frame);
        }
        let reference = transaction.candidate_ref.clone().unwrap();
        transaction.commit().unwrap();
        transaction.finish(true).unwrap();
        drop(transaction);
        store
            .export(&reference, &root.join("motion-demo.herdrchar"))
            .unwrap();
        let v2_duplicate_manifest = br#"{
            "version": 2,
            "format": "herdr.character",
            "id": "motion-demo",
            "name": "Legacy Motion",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "shared.png",
                "running": "shared.png",
                "waiting": "wait.png",
                "unknown": "unknown.png"
            }
        }"#;
        write_fixture_file(&source.join("manifest.json"), v2_duplicate_manifest);
        write_fixture_file(&source.join("unknown.png"), &frame);
        store
            .begin(
                &PackRequest {
                    operation_id: "v2-duplicate-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Update {
                        id: "motion-demo".to_string(),
                        path: source,
                    },
                },
                None,
            )
            .err()
            .expect("legacy duplicate filenames must remain rejected");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_revision_survives_unselected_import_until_replacement_acknowledged() {
        let root =
            std::env::temp_dir().join(format!("herdr-store-active-protection-{}", unique_nonce()));
        let config_dir = root.join("config");
        let builtin = root.join("builtin");
        let active_source = root.join("active");
        let unrelated_source = root.join("unrelated");
        for directory in [&builtin, &active_source, &unrelated_source] {
            fs::create_dir_all(directory).unwrap();
        }
        let active_frame = test_png_bytes();

        let builtin_manifest = br#"{
            "version": 1,
            "name": "Builtin",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "idle.png",
                "running": "running.png",
                "waiting": "waiting.png",
                "unknown": "unknown.png"
            }
        }"#;
        write_fixture_file(&builtin.join("manifest.json"), builtin_manifest);
        for filename in ["idle.png", "running.png", "waiting.png", "unknown.png"] {
            write_fixture_file(&builtin.join(filename), &active_frame);
        }

        let active_manifest = br#"{
            "version": 2,
            "format": "herdr.character",
            "id": "active-pack",
            "name": "Active Pack",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "active-idle.png",
                "running": "active-running.png",
                "waiting": "active-waiting.png",
                "unknown": "active-unknown.png"
            }
        }"#;
        write_fixture_file(&active_source.join("manifest.json"), active_manifest);
        for filename in [
            "active-idle.png",
            "active-running.png",
            "active-waiting.png",
            "active-unknown.png",
        ] {
            write_fixture_file(&active_source.join(filename), &active_frame);
        }

        let unrelated_manifest = br#"{
            "version": 2,
            "format": "herdr.character",
            "id": "unrelated-pack",
            "name": "Unrelated Pack",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "unrelated-idle.png",
                "running": "unrelated-running.png",
                "waiting": "unrelated-waiting.png",
                "unknown": "unrelated-unknown.png"
            }
        }"#;
        write_fixture_file(&unrelated_source.join("manifest.json"), unrelated_manifest);
        for filename in [
            "unrelated-idle.png",
            "unrelated-running.png",
            "unrelated-waiting.png",
            "unrelated-unknown.png",
        ] {
            write_fixture_file(&unrelated_source.join(filename), &active_frame);
        }

        let store = PackStore::new(config_dir, Some(builtin.clone()));
        let mut import = store
            .begin(
                &PackRequest {
                    operation_id: "active-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import {
                        path: active_source.clone(),
                    },
                },
                None,
            )
            .unwrap();
        let active_reference = import.candidate_ref.clone().unwrap();
        import.commit().unwrap();
        import.set_ui_applied(false);
        import.finish(true).unwrap();
        drop(import);

        let mut select = store
            .begin(
                &PackRequest {
                    operation_id: "active-select".to_string(),
                    expected_generation: None,
                    action: PackAction::Select {
                        id: active_reference.id.clone(),
                    },
                },
                None,
            )
            .unwrap();
        select.commit().unwrap();
        select.set_ui_applied(true);
        select.finish(true).unwrap();
        drop(select);

        // The remove committed, but native apply is still pending.  The old
        // revision is therefore still the live renderer's source.
        let mut remove = store
            .begin(
                &PackRequest {
                    operation_id: "active-remove-pending".to_string(),
                    expected_generation: None,
                    action: PackAction::Remove {
                        id: active_reference.id.clone(),
                    },
                },
                Some(&active_reference),
            )
            .unwrap();
        remove.commit().unwrap();
        remove.set_ui_applied(false);
        remove.finish(false).unwrap();
        drop(remove);

        let active_revision_path =
            revision_path(&store.root, &active_reference.id, active_reference.revision);
        assert!(active_revision_path.is_dir());
        assert_eq!(
            fs::read(active_revision_path.join("active-idle.png")).unwrap(),
            active_frame
        );

        // This import does not change selection.  It must not make the
        // unregistered revision collectible merely because the registry no
        // longer lists it.
        let mut unrelated_import = store
            .begin(
                &PackRequest {
                    operation_id: "unrelated-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import {
                        path: unrelated_source.clone(),
                    },
                },
                Some(&active_reference),
            )
            .unwrap();
        unrelated_import.commit().unwrap();
        unrelated_import.set_ui_applied(false);
        unrelated_import.finish(true).unwrap();
        drop(unrelated_import);
        assert!(active_revision_path.is_dir());
        assert_eq!(
            fs::read(active_revision_path.join("active-idle.png")).unwrap(),
            active_frame
        );

        // The saved selection is already builtin after removal. Acknowledging
        // that same selection still replaces the old live renderer and must
        // release its physical revision for collection.
        let mut replacement = store
            .begin(
                &PackRequest {
                    operation_id: "replacement-select".to_string(),
                    expected_generation: None,
                    action: PackAction::Select {
                        id: CharacterRef::builtin().id,
                    },
                },
                Some(&active_reference),
            )
            .unwrap();
        replacement.commit().unwrap();
        replacement.set_ui_applied(true);
        replacement.finish(true).unwrap();
        drop(replacement);
        assert!(!active_revision_path.exists());
        assert!(!active_revision_path.join("active-idle.png").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mixed_v2_v3_update_restore_and_removal_gc_preserve_transactions() {
        let root =
            std::env::temp_dir().join(format!("herdr-store-mixed-revisions-{}", unique_nonce()));
        let config_dir = root.join("config");
        let builtin = root.join("builtin");
        let v3_source = root.join("v3");
        let v2_source = root.join("v2");
        for directory in [&builtin, &v3_source, &v2_source] {
            fs::create_dir_all(directory).unwrap();
        }
        let frame = test_png_bytes();

        let builtin_manifest = br#"{
            "version": 1,
            "name": "Builtin",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "idle.png",
                "running": "running.png",
                "waiting": "waiting.png",
                "unknown": "unknown.png"
            }
        }"#;
        write_fixture_file(&builtin.join("manifest.json"), builtin_manifest);
        for filename in ["idle.png", "running.png", "waiting.png", "unknown.png"] {
            write_fixture_file(&builtin.join(filename), &frame);
        }

        let v3_manifest = br#"{
            "version": 3,
            "format": "herdr.character",
            "id": "motion-demo",
            "name": "Motion Demo",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["shared.png", "idle.png"]},
                "running": {"fps": 12, "frames": ["shared.png"]},
                "waiting": {"fps": 6, "frames": ["wait.png", "shared.png"]},
                "unknown": {"fps": 1, "frames": ["shared.png"]}
            }
        }"#;
        write_fixture_file(&v3_source.join("manifest.json"), v3_manifest);
        for filename in ["shared.png", "idle.png", "wait.png"] {
            write_fixture_file(&v3_source.join(filename), &frame);
        }

        let v2_manifest = br#"{
            "version": 2,
            "format": "herdr.character",
            "id": "motion-demo",
            "name": "Legacy Motion",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "legacy-idle.png",
                "running": "legacy-running.png",
                "waiting": "legacy-waiting.png",
                "unknown": "legacy-unknown.png"
            }
        }"#;
        write_fixture_file(&v2_source.join("manifest.json"), v2_manifest);
        for filename in [
            "legacy-idle.png",
            "legacy-running.png",
            "legacy-waiting.png",
            "legacy-unknown.png",
        ] {
            write_fixture_file(&v2_source.join(filename), &frame);
        }

        let store_root = config_dir.join("characters");
        let store = PackStore::new(config_dir, Some(builtin.clone()));
        let mut import = store
            .begin(
                &PackRequest {
                    operation_id: "mixed-import".to_string(),
                    expected_generation: None,
                    action: PackAction::Import {
                        path: v3_source.clone(),
                    },
                },
                None,
            )
            .unwrap();
        let v3_reference = import.candidate_ref.clone().unwrap();
        import.commit().unwrap();
        import.set_ui_applied(true);
        import.finish(true).unwrap();
        drop(import);

        let mut update = store
            .begin(
                &PackRequest {
                    operation_id: "mixed-update".to_string(),
                    expected_generation: None,
                    action: PackAction::Update {
                        id: "motion-demo".to_string(),
                        path: v2_source.clone(),
                    },
                },
                None,
            )
            .unwrap();
        let v2_reference = update.candidate_ref.clone().unwrap();
        assert!(v2_reference.revision > v3_reference.revision);
        update.commit().unwrap();
        update.set_ui_applied(true);
        update.finish(true).unwrap();
        drop(update);
        assert!(store.load_revision(&v3_reference).is_ok());
        assert!(store.load_revision(&v2_reference).is_ok());

        let mut restore = store
            .begin(
                &PackRequest {
                    operation_id: "mixed-restore".to_string(),
                    expected_generation: None,
                    action: PackAction::Restore {
                        id: "motion-demo".to_string(),
                        revision: v3_reference.revision,
                    },
                },
                None,
            )
            .unwrap();
        assert_eq!(restore.candidate_ref, Some(v3_reference.clone()));
        restore.commit().unwrap();
        restore.set_ui_applied(true);
        restore.finish(true).unwrap();
        drop(restore);
        assert!(store.load_revision(&v3_reference).is_ok());

        let mut remove = store
            .begin(
                &PackRequest {
                    operation_id: "mixed-remove".to_string(),
                    expected_generation: None,
                    action: PackAction::Remove {
                        id: "motion-demo".to_string(),
                    },
                },
                None,
            )
            .unwrap();
        remove.commit().unwrap();
        remove.set_ui_applied(true);
        remove.finish(true).unwrap();
        drop(remove);

        // Advance a durable checkpoint after removal so either immediate or
        // deferred cleanup is exercised before asserting both revisions are
        // no longer loadable.
        let mut checkpoint = store
            .begin(
                &PackRequest {
                    operation_id: "mixed-checkpoint".to_string(),
                    expected_generation: None,
                    action: PackAction::Select {
                        id: CharacterRef::builtin().id,
                    },
                },
                None,
            )
            .unwrap();
        checkpoint.commit().unwrap();
        checkpoint.set_ui_applied(true);
        checkpoint.finish(true).unwrap();
        let v3_path = revision_path(&store_root, &v3_reference.id, v3_reference.revision);
        let v2_path = revision_path(&store_root, &v2_reference.id, v2_reference.revision);
        assert!(!v3_path.is_dir());
        assert!(!v3_path.join("shared.png").exists());
        assert!(!v2_path.is_dir());
        assert!(!v2_path.join("legacy-idle.png").exists());
        // Registry absence alone is not enough: GC must remove the physical
        // revision trees after both index slots have crossed the checkpoint.
        assert!(store.load_revision(&v3_reference).is_err());
        assert!(store.load_revision(&v2_reference).is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gc_preserves_unknown_extra_and_unsupported_trash_trees_but_collects_known() {
        let root = std::env::temp_dir().join(format!("herdr-store-trash-gc-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let trash = store_root.join(TRASH_DIR);
        let frame = test_png_bytes();
        let v3_manifest = br#"{
            "version": 3,
            "format": "herdr.character",
            "id": "trash-pack",
            "name": "Trash Pack",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["frame.png"]},
                "running": {"fps": 8, "frames": ["frame.png"]},
                "waiting": {"fps": 8, "frames": ["frame.png"]},
                "unknown": {"fps": 8, "frames": ["frame.png"]}
            }
        }"#;
        let make_private_directory = |path: &Path| {
            fs::create_dir_all(path).unwrap();
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        };

        let known = trash.join(".trash-known-1");
        make_private_directory(&known);
        write_fixture_file(&known.join("manifest.json"), v3_manifest);
        write_fixture_file(&known.join("frame.png"), &frame);

        let unknown = trash.join(".trash-unknown");
        make_private_directory(&unknown);
        write_fixture_file(&unknown.join("notes.txt"), b"unrecognized");

        let extra = trash.join(".trash-extra");
        make_private_directory(&extra);
        write_fixture_file(&extra.join("manifest.json"), v3_manifest);
        write_fixture_file(&extra.join("frame.png"), &frame);
        write_fixture_file(&extra.join("extra.png"), b"unexpected");

        let unsupported = trash.join(".trash-unsupported");
        make_private_directory(&unsupported);
        write_fixture_file(
            &unsupported.join("manifest.json"),
            br#"{"version":99,"format":"herdr.character"}"#,
        );

        let previous = serde_json::to_vec(&RegistryDisk::empty()).unwrap();
        write_fixture_file(&store_root.join(PREVIOUS_REGISTRY_FILE), &previous);
        gc_owned(&layout.root_file, &store_root, &RegistryDisk::empty(), None).unwrap();

        assert!(!known.exists());
        assert!(unknown.is_dir());
        assert!(unknown.join("notes.txt").exists());
        assert!(extra.is_dir());
        assert!(extra.join("extra.png").exists());
        assert!(unsupported.is_dir());
        assert!(unsupported.join("manifest.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gc_uses_dynamic_v3_file_sets_and_preserves_legacy_duplicate_names() {
        let root = std::env::temp_dir().join(format!("herdr-store-v3-tree-{}", unique_nonce()));
        let layout = ensure_layout(&root, &root.join(STORE_DIR)).unwrap();
        let store_root = root.join(STORE_DIR);
        let revision = revision_path(&store_root, "motion-demo", 1);
        fs::create_dir_all(&revision).unwrap();

        let v3_manifest = br#"{
            "version": 3,
            "format": "herdr.character",
            "id": "motion-demo",
            "name": "Motion Demo",
            "width": 384,
            "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["shared.png", "idle.png"]},
                "running": {"fps": 12, "frames": ["shared.png"]},
                "waiting": {"fps": 6, "frames": ["wait.png", "shared.png"]},
                "unknown": {"fps": 1, "frames": ["shared.png"]}
            }
        }"#;
        let write_private = |path: &Path, bytes: &[u8]| {
            fs::write(path, bytes).unwrap();
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(path, permissions).unwrap();
        };
        write_private(&revision.join("manifest.json"), v3_manifest);
        let mut frame_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut frame_bytes), 384, 512);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![255u8; 384 * 512]).unwrap();
        }
        // The classifier deliberately does not decode PNGs; valid encoded
        // bytes stand in for the immutable stage snapshot.
        for filename in ["shared.png", "idle.png", "wait.png"] {
            write_private(&revision.join(filename), &frame_bytes);
        }

        let revision_directory =
            open_root_relative_directory(&layout.root_file, &store_root, &revision, "revision")
                .unwrap();
        let mut budget = ScanBudget::default();
        assert_eq!(
            managed_manifest_files(v3_manifest).unwrap(),
            vec![
                "shared.png".to_string(),
                "idle.png".to_string(),
                "wait.png".to_string(),
            ]
        );
        // The repeated source references are intentionally represented by
        // one immutable file each in the managed snapshot.
        assert!(known_revision_tree(&revision_directory, &mut budget));

        write_private(&revision.join("orphan.png"), b"unexpected frame");
        let mut budget = ScanBudget::default();
        assert!(!known_revision_tree(&revision_directory, &mut budget));
        assert!(revision.join("orphan.png").exists());
        fs::remove_file(revision.join("orphan.png")).unwrap();

        let v2_duplicate_manifest = br#"{
            "version": 2,
            "format": "herdr.character",
            "id": "legacy",
            "name": "Legacy",
            "width": 384,
            "height": 512,
            "poses": {
                "idle": "shared.png",
                "running": "shared.png",
                "waiting": "wait.png",
                "unknown": "unknown.png"
            }
        }"#;
        write_private(&revision.join("manifest.json"), v2_duplicate_manifest);
        let mut budget = ScanBudget::default();
        assert!(!known_revision_tree(&revision_directory, &mut budget));
        assert!(revision.join("manifest.json").exists());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn dialogue_lookup_reads_only_registered_manifest_and_rejects_stale_or_bad_sources() {
        let root = std::env::temp_dir().join(format!("herdr-dialogue-{}", unique_nonce()));
        let store_root = root.join(STORE_DIR);
        let layout = ensure_layout(&root, &store_root).unwrap();
        let reference = CharacterRef {
            id: "sample".to_string(),
            revision: 7,
        };
        let revision = revision_path(&store_root, &reference.id, reference.revision);
        fs::create_dir_all(&revision).unwrap();
        for path in [&store_root.join(PACKS_DIR).join(&reference.id), &revision] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        }
        let manifest = br#"{
            "version": 3, "format": "herdr.character", "id": "sample",
            "name": "Own Original", "width": 384, "height": 512,
            "phases": {
                "idle": {"fps": 8, "frames": ["missing.png"]},
                "running": {"fps": 8, "frames": ["missing.png"]},
                "waiting": {"fps": 8, "frames": ["missing.png"]},
                "unknown": {"fps": 8, "frames": ["missing.png"]}
            }
        }"#;
        write_fixture_file(&revision.join("manifest.json"), manifest);
        let mut index = RegistryDisk::empty();
        index.generation = 3;
        index.packs.push(PackRecord {
            id: reference.id.clone(),
            name: "Registry Label".to_string(),
            head: reference.revision,
            revisions: vec![reference.revision],
        });
        write_registry_pinned(&layout.root_file, &store_root, None, &index).unwrap();
        let store = PackStore::new(root.clone(), None);
        let (name, metadata) = store.dialogue_metadata(&reference, 3).unwrap();
        assert_eq!(name, "Own Original");
        assert!(metadata.is_none());
        assert!(store
            .dialogue_metadata(&reference, 2)
            .unwrap_err()
            .contains("generation mismatch"));
        let missing = CharacterRef {
            id: reference.id.clone(),
            revision: 8,
        };
        assert!(store
            .dialogue_metadata(&missing, 3)
            .unwrap_err()
            .contains("no retained revision"));
        write_fixture_file(&revision.join("manifest.json"), b"{bad json");
        assert!(store
            .dialogue_metadata(&reference, 3)
            .unwrap_err()
            .contains("manifest is invalid"));
        let _ = fs::remove_dir_all(root);
    }
}
