use self::pack_format::{
    known_phase, known_reaction, valid_payload_kind, DialogueLocaleV4, FrameRegionsV4, ManifestV4,
    ManifestV5, MotionFileV4, MotionKeyTrackV4, MotionTrackV4, MotionV4, PngEntryV4, RegionRectV4,
    RigEntryV4, RigEntryV5, V5_POSE_NAMES,
};
use crate::alpha::AlphaMask;
use crate::animation::{ClipSet, FrameId, PhaseClip, ReactionClip};
use crate::behavior::EffectKind;
use crate::character_types::{validate_pack_id, validate_pack_name};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::num::NonZeroU8;
#[cfg(not(unix))]
use std::path::PathBuf;
use std::path::{Component, Path};
use std::sync::Arc;

#[cfg(unix)]
use std::ffi::CString;
#[cfg(not(unix))]
use std::fs;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[path = "pack_archive.rs"]
mod pack_archive;
#[path = "pack_format.rs"]
mod pack_format;

use self::pack_archive::ArchiveSnapshot;
pub(crate) use self::pack_archive::MAX_EXPANDED_BYTES;
pub use self::pack_format::CharacterMetadata;
pub(crate) use self::pack_format::DialogueLocale;
pub const POSE_WIDTH: u32 = 384;
pub const POSE_HEIGHT: u32 = 512;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DECODED_PIXELS: u64 = 1_000_000;
/// The historical v2-v4 PNG and rig aggregate budget.  Keep this stricter
/// limit for all legacy paths even though v5 archives have a larger envelope.
pub const MAX_AGGREGATE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_V5_AGGREGATE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_V3_DECODED_BYTES: u64 = 48 * 1024 * 1024;
pub const V4_MAX_DIMENSION: u32 = 1024;
pub const V4_MAX_PAYLOADS: usize = 128;
pub const V4_MAX_MOTION_BYTES: usize = 64 * 1024;
pub const V4_MAX_OVERRIDES_BYTES: usize = 256 * 1024;
pub const V5_MAX_MODELS: usize = 16;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_JSON_NODES: usize = 100_000;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const PSD_SIGNATURE: &[u8; 4] = b"8BPS";

const POSE_NAMES: [&str; 4] = ["idle", "running", "waiting", "unknown"];
const REACTION_ROLES: [(&str, EffectKind); 4] = [
    ("completion_observed", EffectKind::CompletionObserved),
    ("head_tap", EffectKind::HeadTap),
    ("body_tap", EffectKind::BodyTap),
    ("pet", EffectKind::Pet),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    name: String,
    width: u32,
    height: u32,
    poses: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedManifestV2 {
    version: u32,
    format: String,
    id: String,
    name: String,
    width: u32,
    height: u32,
    poses: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V3PhaseManifest {
    fps: u8,
    frames: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V3ReactionManifest {
    frames: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedManifestV3 {
    version: u32,
    format: String,
    id: String,
    name: String,
    width: u32,
    height: u32,
    phases: BTreeMap<String, V3PhaseManifest>,
    #[serde(default)]
    reactions: Option<BTreeMap<String, V3ReactionManifest>>,
}

#[derive(Debug)]
pub struct FrameAsset {
    pub(crate) name: String,
    pub(crate) png: Arc<Vec<u8>>,
    pub(crate) alpha: AlphaMask,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRegions {
    pub head: RegionRect,
    pub body: RegionRect,
}

#[derive(Debug)]
pub(crate) struct PngParts {
    pub(crate) frames: Box<[FrameAsset]>,
    pub(crate) clips: ClipSet,
    pub(crate) native_decoded_budget: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) regions: Option<Box<[FrameRegions]>>,
    pub(crate) metadata: Option<CharacterMetadata>,
}

#[derive(Debug)]
pub struct AssetPack {
    frames: Box<[FrameAsset]>,
    clips: ClipSet,
    native_decoded_budget: u64,
    width: u32,
    height: u32,
    regions: Option<Box<[FrameRegions]>>,
    metadata: Option<CharacterMetadata>,
    content_digest: String,
}

#[derive(Debug)]
pub struct RigModelAsset {
    pub id: String,
    pub file: Arc<Vec<u8>>,
    pub overrides: Arc<Vec<u8>>,
    pub motion: Arc<Vec<u8>>,
}

#[derive(Debug)]
pub struct RigAsset {
    pub width: u32,
    pub height: u32,
    // Legacy v4 base/alternate fields remain populated for the supported
    // two-pose path. v5 uses `models` and never routes through poseMix.
    pub base: Arc<Vec<u8>>,
    pub pose: Option<Arc<Vec<u8>>>,
    pub overrides: Arc<Vec<u8>>,
    pub motion: Arc<Vec<u8>>,
    pub base_pose_id: String,
    pub pose_id: Option<String>,
    pub metadata: CharacterMetadata,
    pub content_digest: String,
    pub models: Option<Arc<[RigModelAsset]>>,
    /// For v5, each entry is the model index bound to the ten semantic kinds.
    pub bindings: Option<[u8; 10]>,
    pub initial_pose_kind: i32,
}

#[derive(Debug)]
pub enum ValidatedCharacter {
    Png(AssetPack),
    Rig(RigAsset),
}

impl ValidatedCharacter {
    pub fn content_digest(&self) -> &str {
        match self {
            Self::Png(pack) => &pack.content_digest,
            Self::Rig(rig) => &rig.content_digest,
        }
    }

    /// Load the packaged builtin. Both the historical v1 PNG fixture and a
    /// v4 PNG/rig directory are accepted; managed IDs remain reserved outside
    /// this entry point.
    pub fn load_builtin(root: &Path) -> Result<Self, String> {
        if !root.is_dir() {
            return Err("builtin character source must be a directory".to_string());
        }
        let source = AssetRoot::open(root)?;
        let manifest = source.read("manifest.json", MAX_MANIFEST_BYTES, "asset manifest")?;
        let version = serde_json::from_slice::<serde_json::Value>(&manifest)
            .ok()
            .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64));
        if version == Some(1) {
            return AssetPack::load(root).map(Self::Png);
        }
        drop(source);
        ManagedPack::load_internal(root, true).map(|pack| pack.assets)
    }
    pub fn load_managed(path: &Path) -> Result<Self, String> {
        ManagedPack::load(path).map(|pack| pack.assets)
    }
}

impl RigAsset {
    /// True only for the v5 catalog path. Legacy v4 rigs deliberately keep
    /// phase/effect selection semantics even when a live intent carries a
    /// semantic pose kind.
    pub fn supports_independent_models(&self) -> bool {
        self.models.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct PackPayload {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
}

/// A strict managed pack, retaining every source byte needed by the store.
#[derive(Debug)]
pub struct ManagedPack {
    pub id: String,
    pub name: String,
    pub manifest: Vec<u8>,
    pub assets: ValidatedCharacter,
    pub payloads: Vec<PackPayload>,
    pub(crate) aggregate_limit: u64,
}

#[derive(Debug)]
struct V3Descriptor {
    id: String,
    name: String,
    phases: [V3PhaseDescriptor; 4],
    reactions: [Option<Vec<String>>; 4],
    filenames: Vec<String>,
}

#[derive(Debug)]
struct V3PhaseDescriptor {
    fps: NonZeroU8,
    frames: Vec<String>,
}

#[derive(Debug)]
enum ManagedDescriptor {
    V2 {
        id: String,
        name: String,
        filenames: [String; 4],
    },
    V3(V3Descriptor),
    V4(ManifestV4),
    V5(ManifestV5),
}

#[derive(Debug)]
struct SourceSnapshot {
    entries: BTreeMap<String, Arc<Vec<u8>>>,
}

enum PackSource {
    Directory(AssetRoot),
    Snapshot(SourceSnapshot),
}

impl PackSource {
    fn read(&self, name: &str, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Directory(root) => root.read(name, max_bytes, label),
            Self::Snapshot(snapshot) => snapshot
                .entries
                .get(name)
                .filter(|bytes| bytes.len() <= max_bytes)
                .map(|bytes| bytes.as_ref().clone())
                .ok_or_else(|| format!("{label} is unavailable or exceeds the file limit")),
        }
    }

    fn snapshot_managed(self) -> Result<SourceSnapshot, String> {
        match self {
            Self::Directory(root) => root.snapshot_managed(),
            Self::Snapshot(snapshot) => Ok(snapshot),
        }
    }
}

impl AssetPack {
    /// Loads the legacy v1 asset directory used by the builtin/explicit
    /// --assets path.  Legacy fixed dimensions and byte budgets are unchanged.
    pub fn load(root: &Path) -> Result<Self, String> {
        let source = AssetRoot::open(root)?;
        let manifest_bytes = source.read("manifest.json", MAX_MANIFEST_BYTES, "asset manifest")?;
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("asset manifest is invalid: {error}"))?;
        validate_manifest(&manifest)?;
        let filenames = pose_filenames(&manifest.poses)?;
        load_legacy_assets(&source, &filenames, &manifest_bytes)
    }

    pub(crate) fn into_parts(self) -> PngParts {
        PngParts {
            frames: self.frames,
            clips: self.clips,
            native_decoded_budget: self.native_decoded_budget,
            width: self.width,
            height: self.height,
            regions: self.regions,
            metadata: self.metadata,
        }
    }
}

impl ManagedPack {
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_internal(path, false)
    }

    pub(crate) fn load_internal(path: &Path, allow_builtin: bool) -> Result<Self, String> {
        if path.is_dir() {
            let source = PackSource::Directory(AssetRoot::open(path)?);
            Self::load_from_source(source, allow_builtin)
        } else {
            let extension = path.extension().and_then(|value| value.to_str());
            if extension != Some("herdrchar") {
                return Err("managed archive must use the .herdrchar extension".to_string());
            }
            let archive = self::pack_archive::load(path)?;
            Self::load_from_source(
                PackSource::Snapshot(snapshot_from_archive(archive)),
                allow_builtin,
            )
        }
    }

    pub(crate) fn load_from_directory(directory: &File) -> Result<Self, String> {
        let source = PackSource::Directory(AssetRoot::from_directory(directory)?);
        Self::load_from_source(source, false)
    }

    fn load_from_source(source: PackSource, allow_builtin: bool) -> Result<Self, String> {
        let manifest = source.read("manifest.json", MAX_MANIFEST_BYTES, "asset manifest")?;
        let descriptor = parse_managed_descriptor(&manifest, allow_builtin)?;
        match descriptor {
            ManagedDescriptor::V2 {
                id,
                name,
                filenames,
            } => {
                if allow_builtin {
                    return Err("builtin v4 source is required for this identity".to_string());
                }
                let assets = match &source {
                    PackSource::Directory(root) => load_legacy_assets(root, &filenames, &manifest)?,
                    PackSource::Snapshot(snapshot) => {
                        load_legacy_assets_snapshot(snapshot, &filenames, &manifest)?
                    }
                };
                let payloads = unique_png_payloads(&filenames, &assets);
                Ok(Self {
                    id,
                    name,
                    manifest,
                    assets: ValidatedCharacter::Png(assets),
                    payloads,
                    aggregate_limit: MAX_AGGREGATE_BYTES,
                })
            }
            ManagedDescriptor::V3(descriptor) => {
                if allow_builtin {
                    return Err("builtin v4 source is required for this identity".to_string());
                }
                let assets = match &source {
                    PackSource::Directory(root) => load_v3_assets(root, &descriptor, &manifest)?,
                    PackSource::Snapshot(snapshot) => {
                        load_v3_assets_snapshot(snapshot, &descriptor, &manifest)?
                    }
                };
                let payloads = unique_png_payloads(&descriptor.filenames, &assets);
                Ok(Self {
                    id: descriptor.id,
                    name: descriptor.name,
                    manifest,
                    assets: ValidatedCharacter::Png(assets),
                    payloads,
                    aggregate_limit: MAX_AGGREGATE_BYTES,
                })
            }
            ManagedDescriptor::V4(manifest_v4) => {
                let snapshot = source.snapshot_managed()?;
                load_v4_pack(snapshot, manifest, manifest_v4)
            }
            ManagedDescriptor::V5(manifest_v5) => {
                let snapshot = source.snapshot_managed()?;
                load_v5_pack(snapshot, manifest, manifest_v5)
            }
        }
    }

    pub fn export_archive(&self, output: &Path) -> Result<(), String> {
        let mut entries = Vec::with_capacity(self.payloads.len() + 1);
        entries.push(("manifest.json".to_string(), self.manifest.as_slice()));
        for payload in &self.payloads {
            entries.push((payload.name.clone(), payload.bytes.as_slice()));
        }
        self::pack_archive::write(output, &entries)
    }

    /// Check that every exported entry name satisfies the archive rules, so
    /// a newly installed revision can always be exported again.
    pub(crate) fn validate_archive_names(&self) -> Result<(), String> {
        self::pack_archive::validate_entry_names(
            std::iter::once("manifest.json")
                .chain(self.payloads.iter().map(|payload| payload.name.as_str())),
        )
    }
}

fn snapshot_from_archive(archive: ArchiveSnapshot) -> SourceSnapshot {
    SourceSnapshot {
        entries: archive
            .entries
            .into_iter()
            .map(|(name, bytes)| (name, Arc::new(bytes)))
            .collect(),
    }
}

fn unique_png_payloads(filenames: &[String], assets: &AssetPack) -> Vec<PackPayload> {
    let mut seen = BTreeSet::new();
    filenames
        .iter()
        .enumerate()
        .filter_map(|(index, filename)| {
            if !seen.insert(filename.clone()) {
                return None;
            }
            assets.frames.get(index).map(|frame| PackPayload {
                name: filename.clone(),
                bytes: frame.png.clone(),
            })
        })
        .collect()
}

/// Parse and strictly validate a managed manifest without opening or decoding
/// any PNG. v4/v5 return their complete payload inventory, which is reused by
/// store GC to classify a tree without inventing a second schema.
pub(crate) fn managed_manifest_files(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("asset manifest exceeds the 64 KiB limit".to_string());
    }
    match parse_managed_descriptor(bytes, false)? {
        ManagedDescriptor::V2 { filenames, .. } => Ok(filenames.into_iter().collect()),
        ManagedDescriptor::V3(descriptor) => Ok(descriptor.filenames),
        ManagedDescriptor::V4(manifest) => Ok(manifest
            .payloads
            .into_iter()
            .map(|payload| payload.path)
            .collect()),
        ManagedDescriptor::V5(manifest) => Ok(manifest
            .payloads
            .into_iter()
            .map(|payload| payload.path)
            .collect()),
    }
}

fn parse_managed_descriptor(
    bytes: &[u8],
    allow_builtin: bool,
) -> Result<ManagedDescriptor, String> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("asset manifest exceeds the 64 KiB limit".to_string());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("asset manifest is invalid: {error}"))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "managed character manifest version is invalid".to_string())?;
    match version {
        2 => {
            let parsed: ManagedManifestV2 = serde_json::from_slice(bytes)
                .map_err(|error| format!("asset manifest is invalid: {error}"))?;
            validate_managed_manifest_v2(&parsed)?;
            let filenames = pose_filenames(&parsed.poses)?;
            Ok(ManagedDescriptor::V2 {
                id: parsed.id,
                name: parsed.name,
                filenames,
            })
        }
        3 => {
            reject_duplicate_json_keys(bytes)?;
            if matches!(value.get("reactions"), Some(serde_json::Value::Null)) {
                return Err("managed v3 reactions must be omitted, not null".to_string());
            }
            let parsed: ManagedManifestV3 = serde_json::from_value(value)
                .map_err(|error| format!("asset manifest is invalid: {error}"))?;
            validate_managed_manifest_v3(&parsed)
        }
        4 => {
            reject_duplicate_json_keys(bytes)?;
            reject_explicit_nulls(&value)?;
            let parsed: ManifestV4 = serde_json::from_value(value)
                .map_err(|error| format!("asset manifest is invalid: {error}"))?;
            validate_v4_manifest(&parsed, allow_builtin)?;
            Ok(ManagedDescriptor::V4(parsed))
        }
        5 => {
            reject_duplicate_json_keys(bytes)?;
            reject_explicit_nulls(&value)?;
            let parsed: ManifestV5 = serde_json::from_value(value)
                .map_err(|error| format!("asset manifest is invalid: {error}"))?;
            validate_v5_manifest(&parsed, allow_builtin)?;
            Ok(ManagedDescriptor::V5(parsed))
        }
        _ => Err("unsupported managed character manifest".to_string()),
    }
}

fn validate_managed_manifest_v2(manifest: &ManagedManifestV2) -> Result<(), String> {
    if manifest.version != 2 || manifest.format != "herdr.character" {
        return Err("unsupported managed character manifest".to_string());
    }
    validate_pack_id(&manifest.id)?;
    validate_pack_name(&manifest.name)?;
    if manifest.width != POSE_WIDTH || manifest.height != POSE_HEIGHT {
        return Err(format!(
            "managed character dimensions must be {POSE_WIDTH}x{POSE_HEIGHT}"
        ));
    }
    if manifest.poses.len() != POSE_NAMES.len()
        || POSE_NAMES
            .iter()
            .any(|name| !manifest.poses.contains_key(*name))
    {
        return Err("managed character manifest must contain exactly idle, running, waiting, and unknown poses".to_string());
    }
    for filename in manifest.poses.values() {
        validate_filename(filename)?;
    }
    Ok(())
}

fn validate_managed_manifest_v3(manifest: &ManagedManifestV3) -> Result<ManagedDescriptor, String> {
    if manifest.version != 3 || manifest.format != "herdr.character" {
        return Err("unsupported managed character manifest".to_string());
    }
    validate_pack_id(&manifest.id)?;
    validate_pack_name(&manifest.name)?;
    if manifest.width != POSE_WIDTH || manifest.height != POSE_HEIGHT {
        return Err(format!(
            "managed character dimensions must be {POSE_WIDTH}x{POSE_HEIGHT}"
        ));
    }
    if manifest.phases.len() != POSE_NAMES.len()
        || POSE_NAMES
            .iter()
            .any(|name| !manifest.phases.contains_key(*name))
    {
        return Err(
            "managed v3 manifest must contain exactly idle, running, waiting, and unknown phases"
                .to_string(),
        );
    }
    let mut total_references = 0usize;
    let mut seen = BTreeSet::new();
    let mut filenames = Vec::new();
    let mut phases = Vec::with_capacity(POSE_NAMES.len());
    for phase_name in POSE_NAMES {
        let phase = manifest
            .phases
            .get(phase_name)
            .ok_or_else(|| format!("managed v3 manifest is missing phase {phase_name}"))?;
        let fps = NonZeroU8::new(phase.fps)
            .filter(|fps| fps.get() <= 30)
            .ok_or_else(|| format!("managed v3 phase {phase_name} fps must be 1..30"))?;
        if phase.frames.is_empty() || phase.frames.len() > 16 {
            return Err(format!(
                "managed v3 phase {phase_name} must contain 1..16 frames"
            ));
        }
        total_references = total_references
            .checked_add(phase.frames.len())
            .ok_or_else(|| "managed v3 reference count overflow".to_string())?;
        for filename in &phase.frames {
            validate_v3_filename(filename)?;
            if seen.insert(filename.clone()) {
                filenames.push(filename.clone());
            }
        }
        phases.push(V3PhaseDescriptor {
            fps,
            frames: phase.frames.clone(),
        });
    }
    let mut reactions = [None, None, None, None];
    if let Some(manifest_reactions) = &manifest.reactions {
        for &(reaction_name, effect_kind) in &REACTION_ROLES {
            let Some(reaction) = manifest_reactions.get(reaction_name) else {
                continue;
            };
            if reaction.frames.is_empty() || reaction.frames.len() > 12 {
                return Err(format!(
                    "managed v3 reaction {reaction_name} must contain 1..12 frames"
                ));
            }
            total_references = total_references
                .checked_add(reaction.frames.len())
                .ok_or_else(|| "managed v3 reference count overflow".to_string())?;
            for filename in &reaction.frames {
                validate_v3_filename(filename)?;
                if seen.insert(filename.clone()) {
                    filenames.push(filename.clone());
                }
            }
            reactions[effect_kind.index()] = Some(reaction.frames.clone());
        }
        if manifest_reactions.len() > REACTION_ROLES.len()
            || manifest_reactions.keys().any(|name| {
                !REACTION_ROLES
                    .iter()
                    .any(|(known_name, _)| *known_name == name.as_str())
            })
        {
            return Err("managed v3 manifest contains an unknown reaction".to_string());
        }
    }
    if total_references > 112 {
        return Err("managed v3 manifest exceeds the total frame reference limit".to_string());
    }
    if filenames.len() > 32 {
        return Err("managed v3 manifest exceeds the unique frame limit".to_string());
    }
    let phases: [V3PhaseDescriptor; 4] = phases
        .try_into()
        .map_err(|_| "managed v3 manifest did not produce four phase clips".to_string())?;
    Ok(ManagedDescriptor::V3(V3Descriptor {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        phases,
        reactions,
        filenames,
    }))
}

fn pose_filenames(poses: &BTreeMap<String, String>) -> Result<[String; 4], String> {
    if poses.len() != POSE_NAMES.len() || POSE_NAMES.iter().any(|name| !poses.contains_key(*name)) {
        return Err(
            "asset manifest must contain exactly idle, running, waiting, and unknown poses"
                .to_string(),
        );
    }
    if poses.values().collect::<BTreeSet<_>>().len() != POSE_NAMES.len() {
        return Err("legacy asset poses must use distinct filenames".to_string());
    }
    POSE_NAMES
        .iter()
        .map(|pose_name| {
            let filename = poses
                .get(*pose_name)
                .ok_or_else(|| format!("manifest is missing pose {pose_name}"))?;
            validate_filename(filename)?;
            Ok(filename.clone())
        })
        .collect::<Result<Vec<_>, String>>()?
        .try_into()
        .map_err(|_| "asset manifest did not produce four poses".to_string())
}

fn load_legacy_assets(
    source: &AssetRoot,
    filenames: &[String; 4],
    manifest: &[u8],
) -> Result<AssetPack, String> {
    let mut aggregate_file_bytes = 0u64;
    let mut aggregate_decoded_bytes = 0u64;
    let mut loaded = Vec::with_capacity(POSE_NAMES.len());
    for (index, pose_name) in POSE_NAMES.iter().enumerate() {
        let filename = &filenames[index];
        let bytes = source.read(filename, MAX_FILE_BYTES, &format!("pose {pose_name}"))?;
        let header = parse_png_header(&bytes)
            .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        let decoded_bytes = validate_png_budget(
            header,
            bytes.len(),
            aggregate_file_bytes,
            aggregate_decoded_bytes,
        )
        .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        let alpha = decode_png_mask(&bytes, header, false)
            .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        aggregate_file_bytes = aggregate_file_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "asset file size overflow".to_string())?;
        aggregate_decoded_bytes = aggregate_decoded_bytes
            .checked_add(decoded_bytes)
            .ok_or_else(|| "asset decoded size overflow".to_string())?;
        loaded.push(FrameAsset {
            name: pose_name.to_string(),
            png: Arc::new(bytes),
            alpha,
        });
    }
    let frames = loaded.into_boxed_slice();
    let content_digest = digest_parts(
        manifest,
        filenames
            .iter()
            .zip(frames.iter())
            .map(|(name, frame)| (name.as_str(), frame.png.as_slice())),
    );
    Ok(AssetPack {
        clips: legacy_clip_set(),
        frames,
        native_decoded_budget: MAX_AGGREGATE_BYTES,
        width: POSE_WIDTH,
        height: POSE_HEIGHT,
        regions: None,
        metadata: None,
        content_digest,
    })
}

fn load_legacy_assets_snapshot(
    source: &SourceSnapshot,
    filenames: &[String; 4],
    manifest: &[u8],
) -> Result<AssetPack, String> {
    let mut loaded = Vec::with_capacity(4);
    let mut aggregate_file_bytes = 0;
    let mut aggregate_decoded_bytes = 0;
    for (index, pose_name) in POSE_NAMES.iter().enumerate() {
        let filename = &filenames[index];
        let bytes = source
            .entries
            .get(filename)
            .ok_or_else(|| format!("pose {pose_name} is unavailable"))?
            .clone();
        let header = parse_png_header(bytes.as_slice())
            .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        let decoded = validate_png_budget(
            header,
            bytes.len(),
            aggregate_file_bytes,
            aggregate_decoded_bytes,
        )
        .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        let alpha = decode_png_mask(bytes.as_slice(), header, false)
            .map_err(|error| format!("pose {pose_name} is invalid: {error}"))?;
        aggregate_file_bytes += bytes.len() as u64;
        aggregate_decoded_bytes += decoded;
        loaded.push(FrameAsset {
            name: pose_name.to_string(),
            png: bytes,
            alpha,
        });
    }
    let frames = loaded.into_boxed_slice();
    let content_digest = digest_parts(
        manifest,
        filenames
            .iter()
            .zip(frames.iter())
            .map(|(name, frame)| (name.as_str(), frame.png.as_slice())),
    );
    Ok(AssetPack {
        clips: legacy_clip_set(),
        frames,
        native_decoded_budget: MAX_AGGREGATE_BYTES,
        width: POSE_WIDTH,
        height: POSE_HEIGHT,
        regions: None,
        metadata: None,
        content_digest,
    })
}

fn load_v3_assets(
    source: &AssetRoot,
    descriptor: &V3Descriptor,
    manifest: &[u8],
) -> Result<AssetPack, String> {
    let mut loaded = Vec::with_capacity(descriptor.filenames.len());
    let mut aggregate_file_bytes = 0u64;
    let mut aggregate_decoded_bytes = 0u64;
    for filename in &descriptor.filenames {
        let bytes = source.read(filename, MAX_FILE_BYTES, &format!("frame {filename}"))?;
        let header = parse_png_header(&bytes)
            .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        let decoded_bytes = validate_png_budget_with_limits(
            header,
            bytes.len(),
            aggregate_file_bytes,
            aggregate_decoded_bytes,
            MAX_AGGREGATE_BYTES,
            MAX_V3_DECODED_BYTES,
        )
        .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        let alpha = decode_png_mask(&bytes, header, true)
            .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        aggregate_file_bytes += bytes.len() as u64;
        aggregate_decoded_bytes += decoded_bytes;
        loaded.push(FrameAsset {
            name: filename.clone(),
            png: Arc::new(bytes),
            alpha,
        });
    }
    build_v3_pack(descriptor, manifest, loaded)
}

fn load_v3_assets_snapshot(
    source: &SourceSnapshot,
    descriptor: &V3Descriptor,
    manifest: &[u8],
) -> Result<AssetPack, String> {
    let mut loaded = Vec::with_capacity(descriptor.filenames.len());
    let mut aggregate_file_bytes = 0u64;
    let mut aggregate_decoded_bytes = 0u64;
    for filename in &descriptor.filenames {
        let bytes = source
            .entries
            .get(filename)
            .ok_or_else(|| format!("frame {filename} is unavailable"))?
            .clone();
        let header = parse_png_header(bytes.as_slice())
            .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        let decoded_bytes = validate_png_budget_with_limits(
            header,
            bytes.len(),
            aggregate_file_bytes,
            aggregate_decoded_bytes,
            MAX_AGGREGATE_BYTES,
            MAX_V3_DECODED_BYTES,
        )
        .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        let alpha = decode_png_mask(bytes.as_slice(), header, true)
            .map_err(|error| format!("frame {filename} is invalid: {error}"))?;
        aggregate_file_bytes += bytes.len() as u64;
        aggregate_decoded_bytes += decoded_bytes;
        loaded.push(FrameAsset {
            name: filename.clone(),
            png: bytes,
            alpha,
        });
    }
    build_v3_pack(descriptor, manifest, loaded)
}

fn build_v3_pack(
    descriptor: &V3Descriptor,
    manifest: &[u8],
    loaded: Vec<FrameAsset>,
) -> Result<AssetPack, String> {
    let mut indices = BTreeMap::new();
    for (index, filename) in descriptor.filenames.iter().enumerate() {
        indices.insert(filename.clone(), FrameId::new(index));
    }
    let phases: [PhaseClip; 4] = descriptor
        .phases
        .iter()
        .map(|phase| {
            Ok(PhaseClip {
                frames: clip_frame_ids(&phase.frames, &indices)?,
                fps: phase.fps,
            })
        })
        .collect::<Result<Vec<_>, String>>()?
        .try_into()
        .map_err(|_| "managed v3 manifest did not produce four phase clips".to_string())?;
    let mut reactions = [None, None, None, None];
    for &(_, effect_kind) in &REACTION_ROLES {
        let index = effect_kind.index();
        if let Some(frames) = descriptor.reactions[index].as_ref() {
            reactions[index] = Some(ReactionClip {
                frames: clip_frame_ids(frames, &indices)?,
            });
        }
    }
    let content_digest = digest_parts(
        manifest,
        loaded
            .iter()
            .map(|frame| (frame.name.as_str(), frame.png.as_slice())),
    );
    Ok(AssetPack {
        frames: loaded.into_boxed_slice(),
        clips: ClipSet { phases, reactions },
        native_decoded_budget: MAX_V3_DECODED_BYTES,
        width: POSE_WIDTH,
        height: POSE_HEIGHT,
        regions: None,
        metadata: None,
        content_digest,
    })
}

fn clip_frame_ids(
    names: &[String],
    indices: &BTreeMap<String, FrameId>,
) -> Result<Box<[FrameId]>, String> {
    names
        .iter()
        .map(|name| {
            indices
                .get(name)
                .copied()
                .ok_or_else(|| "managed frame reference is missing from the frame set".to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}
fn legacy_clip_set() -> ClipSet {
    let phases = std::array::from_fn(|index| PhaseClip {
        frames: vec![FrameId::new(index)].into_boxed_slice(),
        fps: NonZeroU8::new(1).expect("one is non-zero"),
    });
    ClipSet {
        phases,
        reactions: [None, None, None, None],
    }
}

fn digest_parts<'a, I>(manifest: &[u8], payloads: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    let mut digest = Sha256::new();
    digest.update(b"herdr.character.content\0");
    digest.update((manifest.len() as u64).to_le_bytes());
    digest.update(manifest);
    for (name, bytes) in payloads {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn load_v4_pack(
    snapshot: SourceSnapshot,
    manifest_bytes: Vec<u8>,
    manifest: ManifestV4,
) -> Result<ManagedPack, String> {
    let payloads = validate_v4_inventory(&snapshot, &manifest)?;
    let entry_bytes = snapshot
        .entries
        .get(&manifest.entry)
        .ok_or_else(|| "v4 entry payload is missing".to_string())?;
    let metadata = CharacterMetadata::from(&manifest);
    let digest = digest_parts(
        &manifest_bytes,
        payloads
            .iter()
            .map(|payload| (payload.name.as_str(), payload.bytes.as_slice())),
    );
    let assets = if manifest.render_mode == "png" {
        let entry: PngEntryV4 = parse_json_payload(entry_bytes, "PNG entry")?;
        ValidatedCharacter::Png(build_v4_png(
            &manifest, &entry, &payloads, metadata, digest,
        )?)
    } else {
        let entry: RigEntryV4 = parse_json_payload(entry_bytes, "rig entry")?;
        if entry.pose.is_some()
            && !manifest
                .runtime
                .capabilities
                .iter()
                .any(|capability| capability == "pose-crossfade")
        {
            return Err("rig alternate pose requires pose-crossfade capability".to_string());
        }
        ValidatedCharacter::Rig(build_v4_rig(
            &manifest, &entry, &payloads, metadata, digest,
        )?)
    };
    Ok(ManagedPack {
        id: manifest.id,
        name: manifest.name,
        manifest: manifest_bytes,
        assets,
        payloads,
        aggregate_limit: MAX_AGGREGATE_BYTES,
    })
}

fn load_v5_pack(
    snapshot: SourceSnapshot,
    manifest_bytes: Vec<u8>,
    manifest: ManifestV5,
) -> Result<ManagedPack, String> {
    let payloads = validate_v5_inventory(&snapshot, &manifest)?;
    let entry_bytes = snapshot
        .entries
        .get(&manifest.entry)
        .ok_or_else(|| "v5 entry payload is missing".to_string())?;
    let entry: RigEntryV5 = parse_json_payload(entry_bytes, "v5 rig entry")?;
    let metadata = CharacterMetadata::from(&manifest);
    let digest = digest_parts(
        &manifest_bytes,
        payloads
            .iter()
            .map(|payload| (payload.name.as_str(), payload.bytes.as_slice())),
    );
    let assets = ValidatedCharacter::Rig(build_v5_rig(
        &manifest, &entry, &payloads, metadata, digest,
    )?);
    Ok(ManagedPack {
        id: manifest.id,
        name: manifest.name,
        manifest: manifest_bytes,
        assets,
        payloads,
        aggregate_limit: MAX_V5_AGGREGATE_BYTES,
    })
}

fn validate_v4_inventory(
    snapshot: &SourceSnapshot,
    manifest: &ManifestV4,
) -> Result<Vec<PackPayload>, String> {
    if snapshot.entries.len() > V4_MAX_PAYLOADS + 1 {
        return Err("v4 source contains too many root files".to_string());
    }
    let expected: BTreeSet<&str> = manifest
        .payloads
        .iter()
        .map(|payload| payload.path.as_str())
        .chain(std::iter::once("manifest.json"))
        .collect();
    if snapshot
        .entries
        .keys()
        .any(|name| !expected.contains(name.as_str()))
        || expected
            .iter()
            .any(|name| !snapshot.entries.contains_key(*name))
    {
        return Err("v4 source files do not exactly match the payload inventory".to_string());
    }
    let mut result = Vec::with_capacity(manifest.payloads.len());
    for spec in &manifest.payloads {
        let bytes = snapshot
            .entries
            .get(&spec.path)
            .ok_or_else(|| format!("payload {} is missing", spec.path))?
            .clone();
        if bytes.len() as u64 != spec.size {
            return Err(format!("payload {} has an unexpected size", spec.path));
        }
        if !sha256_hex(bytes.as_slice()).eq_ignore_ascii_case(&spec.sha256) {
            return Err(format!(
                "payload {} has an unexpected SHA-256 digest",
                spec.path
            ));
        }
        let max = match spec.kind.as_str() {
            "entry" => MAX_ENTRY_BYTES,
            "motion" => V4_MAX_MOTION_BYTES,
            "overrides" => V4_MAX_OVERRIDES_BYTES,
            _ => MAX_FILE_BYTES,
        };
        if bytes.len() > max {
            return Err(format!("payload {} exceeds its kind limit", spec.path));
        }
        validate_payload_magic(spec, bytes.as_slice())?;
        result.push(PackPayload {
            name: spec.path.clone(),
            bytes,
        });
    }
    Ok(result)
}

fn validate_v5_inventory(
    snapshot: &SourceSnapshot,
    manifest: &ManifestV5,
) -> Result<Vec<PackPayload>, String> {
    if snapshot.entries.len() > V4_MAX_PAYLOADS + 1 {
        return Err("v5 source contains too many root files".to_string());
    }
    let expected: BTreeSet<&str> = manifest
        .payloads
        .iter()
        .map(|payload| payload.path.as_str())
        .chain(std::iter::once("manifest.json"))
        .collect();
    if snapshot
        .entries
        .keys()
        .any(|name| !expected.contains(name.as_str()))
        || expected
            .iter()
            .any(|name| !snapshot.entries.contains_key(*name))
    {
        return Err("v5 source files do not exactly match the payload inventory".to_string());
    }
    let mut total = 0u64;
    let mut result = Vec::with_capacity(manifest.payloads.len());
    for spec in &manifest.payloads {
        let bytes = snapshot
            .entries
            .get(&spec.path)
            .ok_or_else(|| format!("payload {} is missing", spec.path))?
            .clone();
        if bytes.len() as u64 != spec.size {
            return Err(format!("payload {} has an unexpected size", spec.path));
        }
        if !sha256_hex(bytes.as_slice()).eq_ignore_ascii_case(&spec.sha256) {
            return Err(format!(
                "payload {} has an unexpected SHA-256 digest",
                spec.path
            ));
        }
        let max = match spec.kind.as_str() {
            "entry" => MAX_ENTRY_BYTES,
            "motion" => V4_MAX_MOTION_BYTES,
            "overrides" => V4_MAX_OVERRIDES_BYTES,
            _ => MAX_FILE_BYTES,
        };
        if bytes.len() > max {
            return Err(format!("payload {} exceeds its kind limit", spec.path));
        }
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "v5 source size overflows".to_string())?;
        if total > MAX_V5_AGGREGATE_BYTES {
            return Err("v5 source exceeds the 64 MiB aggregate limit".to_string());
        }
        validate_payload_magic(spec, bytes.as_slice())?;
        result.push(PackPayload {
            name: spec.path.clone(),
            bytes,
        });
    }
    Ok(result)
}

fn validate_payload_magic(spec: &pack_format::PayloadV4, bytes: &[u8]) -> Result<(), String> {
    match spec.kind.as_str() {
        "png"
            if bytes.len() < PNG_SIGNATURE.len()
                || &bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE =>
        {
            Err(format!("payload {} is not a PNG", spec.path))
        }
        "psd"
            if bytes.len() < PSD_SIGNATURE.len()
                || &bytes[..PSD_SIGNATURE.len()] != PSD_SIGNATURE =>
        {
            Err(format!("payload {} is not a PSD", spec.path))
        }
        "png" | "psd" => Ok(()),
        "entry" | "overrides" | "motion" => {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| format!("payload {} is invalid JSON: {error}", spec.path))?;
            reject_duplicate_json_keys(bytes)?;
            reject_explicit_nulls(&value)
        }
        "license" | "attribution" | "source" => {
            std::str::from_utf8(bytes)
                .map_err(|_| format!("payload {} is not UTF-8 text", spec.path))?;
            Ok(())
        }
        _ => Err(format!("payload {} has an unsupported kind", spec.path)),
    }
}

fn build_v4_png(
    manifest: &ManifestV4,
    entry: &PngEntryV4,
    payloads: &[PackPayload],
    metadata: CharacterMetadata,
    digest: String,
) -> Result<AssetPack, String> {
    if entry.version != 1 {
        return Err("PNG entry version must be 1".to_string());
    }
    let declared_png: BTreeSet<&str> = manifest
        .payloads
        .iter()
        .filter(|payload| payload.kind == "png")
        .map(|payload| payload.path.as_str())
        .collect();
    if entry.phases.len() != 4
        || POSE_NAMES
            .iter()
            .any(|name| !entry.phases.contains_key(*name))
    {
        return Err("v4 PNG entry must contain exactly four phases".to_string());
    }
    if entry.reactions.len() != 4
        || REACTION_ROLES
            .iter()
            .any(|(name, _)| !entry.reactions.contains_key(*name))
    {
        return Err("v4 PNG entry must contain exactly four reactions".to_string());
    }
    let mut total = 0usize;
    let mut names = BTreeSet::new();
    let mut ordered_names = Vec::new();
    let mut phases = Vec::new();
    for phase_name in POSE_NAMES {
        let phase = entry
            .phases
            .get(phase_name)
            .ok_or_else(|| format!("v4 PNG phase {phase_name} is missing"))?;
        let fps = NonZeroU8::new(phase.fps)
            .filter(|fps| fps.get() <= 30)
            .ok_or_else(|| format!("v4 PNG phase {phase_name} fps must be 1..30"))?;
        if phase.frames.is_empty() || phase.frames.len() > 16 {
            return Err(format!(
                "v4 PNG phase {phase_name} must contain 1..16 frames"
            ));
        }
        total += phase.frames.len();
        for filename in &phase.frames {
            validate_v4_payload_name(filename)?;
            if names.insert(filename.clone()) {
                ordered_names.push(filename.clone());
            }
        }
        phases.push((phase.frames.clone(), fps));
    }
    let mut reactions = [None, None, None, None];
    for &(reaction_name, kind) in &REACTION_ROLES {
        let reaction = entry
            .reactions
            .get(reaction_name)
            .ok_or_else(|| format!("v4 PNG reaction {reaction_name} is missing"))?;
        if reaction.frames.is_empty() || reaction.frames.len() > 12 {
            return Err(format!(
                "v4 PNG reaction {reaction_name} must contain 1..12 frames"
            ));
        }
        total += reaction.frames.len();
        for filename in &reaction.frames {
            validate_v4_payload_name(filename)?;
            if names.insert(filename.clone()) {
                ordered_names.push(filename.clone());
            }
        }
        reactions[kind.index()] = Some(reaction.frames.clone());
    }
    if total > 112 || names.len() > 32 {
        return Err("v4 PNG frame reference limits exceeded".to_string());
    }
    if entry.regions.len() != names.len() || entry.regions.keys().any(|name| !names.contains(name))
    {
        return Err("v4 PNG regions must cover each unique frame exactly once".to_string());
    }
    let mut regions = Vec::with_capacity(ordered_names.len());
    for name in &ordered_names {
        regions.push(convert_regions(
            entry
                .regions
                .get(name)
                .ok_or_else(|| format!("v4 PNG region for {name} is missing"))?,
            manifest.width,
            manifest.height,
        )?);
    }
    let payload_map: BTreeMap<&str, Arc<Vec<u8>>> = payloads
        .iter()
        .map(|payload| (payload.name.as_str(), payload.bytes.clone()))
        .collect();
    let mut frames = Vec::with_capacity(ordered_names.len());
    let mut aggregate_file_bytes = 0u64;
    if declared_png.len() != names.len()
        || declared_png.iter().any(|name| !names.contains(*name))
        || names
            .iter()
            .any(|name| !declared_png.contains(name.as_str()))
    {
        return Err("v4 PNG inventory must contain exactly the referenced frames".to_string());
    }
    let mut aggregate_decoded_bytes = 0u64;
    for name in &ordered_names {
        let bytes = payload_map
            .get(name.as_str())
            .cloned()
            .ok_or_else(|| format!("v4 PNG frame {name} is not in the inventory"))?;
        let header =
            parse_png_header_for_canvas(bytes.as_slice(), manifest.width, manifest.height)?;
        let decoded = validate_png_budget_with_limits(
            header,
            bytes.len(),
            aggregate_file_bytes,
            aggregate_decoded_bytes,
            MAX_AGGREGATE_BYTES,
            MAX_AGGREGATE_BYTES,
        )?;
        let alpha = decode_png_mask(bytes.as_slice(), header, true)?;
        aggregate_file_bytes += bytes.len() as u64;
        aggregate_decoded_bytes += decoded;
        frames.push(FrameAsset {
            name: name.clone(),
            png: bytes,
            alpha,
        });
    }
    let indices: BTreeMap<String, FrameId> = ordered_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), FrameId::new(index)))
        .collect();
    let phase_clips: [PhaseClip; 4] = phases
        .into_iter()
        .map(|(names, fps)| {
            Ok(PhaseClip {
                frames: clip_frame_ids(&names, &indices)?,
                fps,
            })
        })
        .collect::<Result<Vec<_>, String>>()?
        .try_into()
        .map_err(|_| "v4 PNG phase count is invalid".to_string())?;
    let mut reaction_clips = [None, None, None, None];
    for index in 0..4 {
        if let Some(names) = reactions[index].as_ref() {
            reaction_clips[index] = Some(ReactionClip {
                frames: clip_frame_ids(names, &indices)?,
            });
        }
    }
    Ok(AssetPack {
        frames: frames.into_boxed_slice(),
        clips: ClipSet {
            phases: phase_clips,
            reactions: reaction_clips,
        },
        native_decoded_budget: MAX_AGGREGATE_BYTES,
        width: manifest.width,
        height: manifest.height,
        regions: Some(regions.into_boxed_slice()),
        metadata: Some(metadata),
        content_digest: digest,
    })
}

fn build_v4_rig(
    manifest: &ManifestV4,
    entry: &RigEntryV4,
    payloads: &[PackPayload],
    metadata: CharacterMetadata,
    digest: String,
) -> Result<RigAsset, String> {
    if entry.version != 1 {
        return Err("rig entry version must be 1".to_string());
    }
    validate_rig_pose_id(&entry.base.id)?;
    if let Some(pose) = &entry.pose {
        validate_rig_pose_id(&pose.id)?;
        if pose.id == entry.base.id {
            return Err("rig base and alternate pose IDs must differ".to_string());
        }
    }
    validate_v4_payload_name(&entry.base.file)?;
    if let Some(pose) = &entry.pose {
        validate_v4_payload_name(&pose.file)?;
        if pose.file == entry.base.file {
            return Err("rig base and alternate payloads must be independent".to_string());
        }
    }
    validate_v4_payload_name(&entry.overrides)?;
    let payload_kind = |name: &str| {
        manifest
            .payloads
            .iter()
            .find(|payload| payload.path == name)
            .map(|payload| payload.kind.as_str())
    };
    if payload_kind(&entry.base.file) != Some("psd")
        || entry
            .pose
            .as_ref()
            .is_some_and(|pose| payload_kind(&pose.file) != Some("psd"))
        || payload_kind(&entry.overrides) != Some("overrides")
        || payload_kind(&entry.motion) != Some("motion")
    {
        return Err("rig entry references payloads with the wrong kinds".to_string());
    }
    validate_v4_payload_name(&entry.motion)?;
    let map: BTreeMap<&str, Arc<Vec<u8>>> = payloads
        .iter()
        .map(|payload| (payload.name.as_str(), payload.bytes.clone()))
        .collect();
    let base = map
        .get(entry.base.file.as_str())
        .cloned()
        .ok_or_else(|| "rig base payload is missing".to_string())?;
    let pose = entry
        .pose
        .as_ref()
        .map(|reference| {
            map.get(reference.file.as_str())
                .cloned()
                .ok_or_else(|| "rig alternate payload is missing".to_string())
        })
        .transpose()?;
    let overrides = map
        .get(entry.overrides.as_str())
        .cloned()
        .ok_or_else(|| "rig overrides payload is missing".to_string())?;
    let motion = map
        .get(entry.motion.as_str())
        .cloned()
        .ok_or_else(|| "rig motion payload is missing".to_string())?;
    let base_dimensions =
        validate_psd(base.as_slice(), manifest.width, manifest.height, "rig base")?;
    if let Some(pose_bytes) = pose.as_ref() {
        validate_psd(
            pose_bytes.as_slice(),
            manifest.width,
            manifest.height,
            "rig alternate",
        )?;
    }
    validate_overrides(overrides.as_slice(), manifest.width, manifest.height)?;
    let motion_file: MotionFileV4 = parse_json_payload(motion.as_slice(), "rig motion")?;
    validate_motion(
        &motion_file,
        &entry.base.id,
        entry.pose.as_ref().map(|pose| pose.id.as_str()),
        manifest.width,
        manifest.height,
    )?;
    let referenced: BTreeSet<&str> = std::iter::once(entry.base.file.as_str())
        .chain(entry.pose.as_ref().map(|pose| pose.file.as_str()))
        .chain(std::iter::once(entry.overrides.as_str()))
        .chain(std::iter::once(entry.motion.as_str()))
        .collect();
    if manifest.payloads.iter().any(|payload| {
        matches!(payload.kind.as_str(), "psd" | "overrides" | "motion")
            && !referenced.contains(payload.path.as_str())
    }) {
        return Err("rig inventory contains an unreferenced executable payload".to_string());
    }
    Ok(RigAsset {
        width: base_dimensions.0,
        height: base_dimensions.1,
        base,
        pose,
        overrides,
        motion,
        base_pose_id: entry.base.id.clone(),
        pose_id: entry.pose.as_ref().map(|pose| pose.id.clone()),
        metadata,
        content_digest: digest,
        models: None,
        bindings: None,
        initial_pose_kind: -1,
    })
}

fn build_v5_rig(
    manifest: &ManifestV5,
    entry: &RigEntryV5,
    payloads: &[PackPayload],
    metadata: CharacterMetadata,
    digest: String,
) -> Result<RigAsset, String> {
    if entry.version != 2 || entry.initial != "waiting" {
        return Err("v5 rig entry version or initial model is invalid".to_string());
    }
    if !(V5_POSE_NAMES.len()..=V5_MAX_MODELS).contains(&entry.models.len()) {
        return Err("v5 rig entry must contain 10..16 models".to_string());
    }
    if entry.bindings.len() != V5_POSE_NAMES.len()
        || V5_POSE_NAMES
            .iter()
            .any(|name| !entry.bindings.contains_key(*name))
    {
        return Err("v5 rig bindings must contain exactly the ten semantic names".to_string());
    }
    let mut model_ids = BTreeSet::new();
    let mut payload_map: BTreeMap<&str, Arc<Vec<u8>>> = payloads
        .iter()
        .map(|payload| (payload.name.as_str(), payload.bytes.clone()))
        .collect();
    let mut model_payloads = BTreeSet::new();
    let mut models = Vec::with_capacity(entry.models.len());
    for model in &entry.models {
        validate_rig_pose_id(&model.id)?;
        if !model_ids.insert(model.id.as_str()) {
            return Err("v5 rig models contain duplicate IDs".to_string());
        }
        validate_v4_payload_name(&model.file)?;
        validate_v4_payload_name(&model.overrides)?;
        validate_v4_payload_name(&model.motion)?;
        if !model_payloads.insert(model.file.as_str())
            || !model_payloads.insert(model.overrides.as_str())
            || !model_payloads.insert(model.motion.as_str())
        {
            return Err(
                "v5 rig models must own distinct file, override, and motion payloads".to_string(),
            );
        }
        let file = payload_map
            .remove(model.file.as_str())
            .ok_or_else(|| format!("v5 model {} PSD payload is missing", model.id))?;
        let overrides = payload_map
            .remove(model.overrides.as_str())
            .ok_or_else(|| format!("v5 model {} overrides payload is missing", model.id))?;
        let motion = payload_map
            .remove(model.motion.as_str())
            .ok_or_else(|| format!("v5 model {} motion payload is missing", model.id))?;
        if manifest
            .payloads
            .iter()
            .find(|payload| payload.path == model.file)
            .map(|payload| payload.kind.as_str())
            != Some("psd")
            || manifest
                .payloads
                .iter()
                .find(|payload| payload.path == model.overrides)
                .map(|payload| payload.kind.as_str())
                != Some("overrides")
            || manifest
                .payloads
                .iter()
                .find(|payload| payload.path == model.motion)
                .map(|payload| payload.kind.as_str())
                != Some("motion")
        {
            return Err(format!(
                "v5 model {} references a payload with the wrong kind",
                model.id
            ));
        }
        validate_psd(
            file.as_slice(),
            manifest.width,
            manifest.height,
            &format!("model {}", model.id),
        )?;
        validate_overrides(overrides.as_slice(), manifest.width, manifest.height)?;
        let motion_file: MotionFileV4 =
            parse_json_payload(motion.as_slice(), &format!("motion for model {}", model.id))?;
        validate_motion(
            &motion_file,
            &model.id,
            None,
            manifest.width,
            manifest.height,
        )?;
        models.push(RigModelAsset {
            id: model.id.clone(),
            file,
            overrides,
            motion,
        });
    }
    let mut bindings = [0u8; 10];
    let mut bound_models = BTreeSet::new();
    for (index, name) in V5_POSE_NAMES.iter().enumerate() {
        let model_id = entry
            .bindings
            .get(*name)
            .ok_or_else(|| format!("v5 binding {name} is missing"))?;
        let model_index = entry
            .models
            .iter()
            .position(|model| model.id == *model_id)
            .ok_or_else(|| format!("v5 binding {name} references an unknown model"))?;
        if !bound_models.insert(model_index) {
            return Err("v5 bindings contain duplicate model references".to_string());
        }
        bindings[index] = u8::try_from(model_index)
            .map_err(|_| "v5 model index does not fit the native binding ABI".to_string())?;
    }
    let referenced: BTreeSet<&str> = entry
        .models
        .iter()
        .flat_map(|model| [&model.file, &model.overrides, &model.motion])
        .map(String::as_str)
        .collect();
    if manifest.payloads.iter().any(|payload| {
        matches!(payload.kind.as_str(), "psd" | "overrides" | "motion")
            && !referenced.contains(payload.path.as_str())
    }) {
        return Err("v5 inventory contains an unreferenced executable payload".to_string());
    }
    let first = models
        .first()
        .ok_or_else(|| "v5 rig entry has no models".to_string())?;
    let base = first.file.clone();
    let overrides = first.overrides.clone();
    let motion = first.motion.clone();
    Ok(RigAsset {
        width: manifest.width,
        height: manifest.height,
        base,
        pose: None,
        overrides,
        motion,
        base_pose_id: first.id.clone(),
        pose_id: None,
        metadata,
        content_digest: digest,
        models: Some(models.into()),
        bindings: Some(bindings),
        initial_pose_kind: 0,
    })
}

fn validate_v4_manifest(manifest: &ManifestV4, allow_builtin: bool) -> Result<(), String> {
    if manifest.format != "herdr.character"
        || manifest.version != 4
        || !matches!(manifest.render_mode.as_str(), "png" | "rig")
    {
        return Err("unsupported v4 character manifest".to_string());
    }
    if allow_builtin {
        validate_identity(&manifest.id, true)?;
    } else {
        validate_pack_id(&manifest.id)?;
    }
    validate_pack_name(&manifest.name)?;
    if manifest.width == 0
        || manifest.height == 0
        || manifest.width > V4_MAX_DIMENSION
        || manifest.height > V4_MAX_DIMENSION
        || (manifest.render_mode == "png"
            && u64::from(manifest.width) * u64::from(manifest.height) > MAX_DECODED_PIXELS)
    {
        return Err("v4 canvas dimensions exceed the limit".to_string());
    }
    validate_bounded_text(&manifest.author.name, 256, "author name", false)?;
    if let Some(url) = &manifest.author.url {
        validate_bounded_text(url, 2048, "author URL", true)?;
    }
    if !matches!(
        manifest.source.method.as_str(),
        "hand-layered" | "procedural" | "image-generated"
    ) {
        return Err("v4 source method is unsupported".to_string());
    }
    validate_bounded_text(
        &manifest.source.description,
        4096,
        "source description",
        false,
    )?;
    for url in manifest.source.urls.as_deref().unwrap_or(&[]) {
        validate_bounded_text(url, 2048, "source URL", true)?;
    }
    for value in [&manifest.source.provider, &manifest.source.model]
        .into_iter()
        .flatten()
    {
        validate_bounded_text(value, 512, "source field", false)?;
    }
    if manifest.licenses.is_empty() {
        return Err("v4 manifest must declare at least one license".to_string());
    }
    for license in &manifest.licenses {
        validate_bounded_text(&license.expression, 256, "license expression", false)?;
        validate_v4_payload_name(&license.path)?;
    }
    for path in &manifest.attributions {
        validate_v4_payload_name(path)?;
    }
    validate_v4_payload_name_allow_manifest(&manifest.entry)?;
    if manifest.entry == "manifest.json" {
        return Err("v4 entry must not be manifest.json".to_string());
    }
    if manifest.runtime.version != 1 {
        return Err("v4 runtime version must be 1".to_string());
    }
    let required = if manifest.render_mode == "png" {
        (
            "herdr-native-png",
            ["frame-clips", "frame-regions"].as_slice(),
        )
    } else {
        (
            "herdr-native-rig",
            ["mesh-deformation", "eye-stencil"].as_slice(),
        )
    };
    if manifest.runtime.name != required.0 {
        return Err("v4 runtime backend does not match render_mode".to_string());
    }
    let mut capabilities = BTreeSet::new();
    for capability in &manifest.runtime.capabilities {
        if !capabilities.insert(capability.as_str()) {
            return Err("v4 runtime capabilities contain a duplicate".to_string());
        }
    }
    for capability in &capabilities {
        let known = if manifest.render_mode == "png" {
            matches!(*capability, "frame-clips" | "frame-regions")
        } else {
            matches!(
                *capability,
                "mesh-deformation" | "eye-stencil" | "pose-crossfade"
            )
        };
        if !known {
            return Err("v4 runtime contains an unknown capability".to_string());
        }
    }
    let required_capabilities: &[&str] = if manifest.render_mode == "png" {
        &["frame-clips", "frame-regions"]
    } else {
        &["mesh-deformation", "eye-stencil"]
    };
    if required_capabilities
        .iter()
        .any(|required| !capabilities.contains(required))
    {
        return Err("v4 runtime is missing a required backend capability".to_string());
    }
    if manifest.payloads.is_empty() || manifest.payloads.len() > V4_MAX_PAYLOADS {
        return Err("v4 payload inventory must contain 1..128 entries".to_string());
    }
    let mut names = BTreeSet::new();
    let mut case_names = BTreeSet::from([b"manifest.json".to_vec()]);
    let mut license_paths = BTreeSet::new();
    let mut attribution_paths = BTreeSet::new();
    for license in &manifest.licenses {
        license_paths.insert(license.path.as_str());
    }
    for path in &manifest.attributions {
        attribution_paths.insert(path.as_str());
    }
    let mut total = 0u64;
    let mut entry_count = 0usize;
    for payload in &manifest.payloads {
        validate_v4_payload_name(&payload.path)?;
        if payload.path == "manifest.json" || !valid_payload_kind(&payload.kind) {
            return Err(format!(
                "v4 payload {} has an invalid path or kind",
                payload.path
            ));
        }
        let mode_kind = if manifest.render_mode == "png" {
            matches!(
                payload.kind.as_str(),
                "png" | "entry" | "license" | "attribution" | "source"
            )
        } else {
            matches!(
                payload.kind.as_str(),
                "psd" | "entry" | "overrides" | "motion" | "license" | "attribution" | "source"
            )
        };
        if !mode_kind {
            return Err(format!(
                "v4 payload {} is not valid for this backend",
                payload.path
            ));
        }
        if !names.insert(payload.path.as_str())
            || !case_names.insert(
                payload
                    .path
                    .bytes()
                    .map(|byte| byte.to_ascii_lowercase())
                    .collect::<Vec<_>>(),
            )
        {
            return Err("v4 payload paths contain duplicates or case collisions".to_string());
        }
        if payload.sha256.len() != 64
            || !payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "v4 payload {} has an invalid SHA-256 digest",
                payload.path
            ));
        }
        let limit = match payload.kind.as_str() {
            "entry" => MAX_ENTRY_BYTES,
            "motion" => V4_MAX_MOTION_BYTES,
            "overrides" => V4_MAX_OVERRIDES_BYTES,
            _ => MAX_FILE_BYTES,
        } as u64;
        if payload.size > limit {
            return Err(format!(
                "v4 payload {} exceeds its kind limit",
                payload.path
            ));
        }
        total = total
            .checked_add(payload.size)
            .ok_or_else(|| "v4 payload size overflows".to_string())?;
        if total > MAX_AGGREGATE_BYTES {
            return Err("v4 payload inventory exceeds the 16 MiB limit".to_string());
        }
        if payload.kind == "entry" {
            entry_count += 1;
        }
    }
    if manifest
        .payloads
        .iter()
        .filter(|payload| payload.path == manifest.entry && payload.kind == "entry")
        .count()
        != 1
    {
        return Err("v4 manifest entry must refer to an entry payload".to_string());
    }
    if entry_count != 1 || !names.contains(manifest.entry.as_str()) {
        return Err("v4 inventory must contain exactly the declared entry payload".to_string());
    }
    for path in license_paths {
        if manifest
            .payloads
            .iter()
            .filter(|payload| payload.path.as_str() == path && payload.kind == "license")
            .count()
            != 1
        {
            return Err("v4 license references must resolve to license payloads".to_string());
        }
    }
    for path in attribution_paths {
        if manifest
            .payloads
            .iter()
            .filter(|payload| payload.path.as_str() == path && payload.kind == "attribution")
            .count()
            != 1
        {
            return Err(
                "v4 attribution references must resolve to attribution payloads".to_string(),
            );
        }
    }
    if let Some(persona) = &manifest.persona {
        validate_bounded_text(persona, 1024, "persona", false)?;
    }
    validate_dialogue(manifest.dialogue.as_ref())
}

fn validate_v5_manifest(manifest: &ManifestV5, allow_builtin: bool) -> Result<(), String> {
    if manifest.format != "herdr.character"
        || manifest.version != 5
        || manifest.render_mode != "rig"
    {
        return Err("unsupported v5 character manifest".to_string());
    }
    if allow_builtin {
        validate_identity(&manifest.id, true)?;
    } else {
        validate_pack_id(&manifest.id)?;
    }
    validate_pack_name(&manifest.name)?;
    if manifest.width == 0
        || manifest.height == 0
        || manifest.width > V4_MAX_DIMENSION
        || manifest.height > V4_MAX_DIMENSION
    {
        return Err("v5 canvas dimensions exceed the limit".to_string());
    }
    validate_bounded_text(&manifest.author.name, 256, "author name", false)?;
    if let Some(url) = &manifest.author.url {
        validate_bounded_text(url, 2048, "author URL", true)?;
    }
    if !matches!(
        manifest.source.method.as_str(),
        "hand-layered" | "procedural" | "image-generated"
    ) {
        return Err("v5 source method is unsupported".to_string());
    }
    validate_bounded_text(
        &manifest.source.description,
        4096,
        "source description",
        false,
    )?;
    for url in manifest.source.urls.as_deref().unwrap_or(&[]) {
        validate_bounded_text(url, 2048, "source URL", true)?;
    }
    for value in [&manifest.source.provider, &manifest.source.model]
        .into_iter()
        .flatten()
    {
        validate_bounded_text(value, 512, "source field", false)?;
    }
    if manifest.licenses.is_empty() {
        return Err("v5 manifest must declare at least one license".to_string());
    }
    let mut license_paths = BTreeSet::new();
    for license in &manifest.licenses {
        validate_bounded_text(&license.expression, 256, "license expression", false)?;
        validate_v4_payload_name(&license.path)?;
        license_paths.insert(license.path.as_str());
    }
    let mut attribution_paths = BTreeSet::new();
    for path in &manifest.attributions {
        validate_v4_payload_name(path)?;
        attribution_paths.insert(path.as_str());
    }
    validate_v4_payload_name_allow_manifest(&manifest.entry)?;
    if manifest.entry == "manifest.json" {
        return Err("v5 entry must not be manifest.json".to_string());
    }
    if manifest.runtime.name != "herdr-native-rig" || manifest.runtime.version != 2 {
        return Err("v5 runtime backend must be herdr-native-rig version 2".to_string());
    }
    let mut capabilities = BTreeSet::new();
    for capability in &manifest.runtime.capabilities {
        if !capabilities.insert(capability.as_str()) {
            return Err("v5 runtime capabilities contain a duplicate".to_string());
        }
        if !matches!(
            capability.as_str(),
            "mesh-deformation" | "eye-stencil" | "independent-models"
        ) {
            return Err("v5 runtime contains an unknown capability".to_string());
        }
    }
    for required in ["mesh-deformation", "eye-stencil", "independent-models"] {
        if !capabilities.contains(required) {
            return Err("v5 runtime is missing a required capability".to_string());
        }
    }
    if manifest.payloads.is_empty() || manifest.payloads.len() > V4_MAX_PAYLOADS {
        return Err("v5 payload inventory must contain 1..128 entries".to_string());
    }
    let mut names = BTreeSet::new();
    let mut case_names = BTreeSet::from([b"manifest.json".to_vec()]);
    let mut total = 0u64;
    let mut entry_count = 0usize;
    for payload in &manifest.payloads {
        validate_v4_payload_name(&payload.path)?;
        if !valid_payload_kind(&payload.kind)
            || !matches!(
                payload.kind.as_str(),
                "psd" | "entry" | "overrides" | "motion" | "license" | "attribution" | "source"
            )
        {
            return Err(format!("v5 payload {} has an invalid kind", payload.path));
        }
        if !names.insert(payload.path.as_str())
            || !case_names.insert(
                payload
                    .path
                    .bytes()
                    .map(|byte| byte.to_ascii_lowercase())
                    .collect::<Vec<_>>(),
            )
        {
            return Err("v5 payload paths contain duplicates or case collisions".to_string());
        }
        if payload.sha256.len() != 64
            || !payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "v5 payload {} has an invalid SHA-256 digest",
                payload.path
            ));
        }
        let limit = match payload.kind.as_str() {
            "entry" => MAX_ENTRY_BYTES,
            "motion" => V4_MAX_MOTION_BYTES,
            "overrides" => V4_MAX_OVERRIDES_BYTES,
            _ => MAX_FILE_BYTES,
        } as u64;
        if payload.size > limit {
            return Err(format!(
                "v5 payload {} exceeds its kind limit",
                payload.path
            ));
        }
        total = total
            .checked_add(payload.size)
            .ok_or_else(|| "v5 payload size overflows".to_string())?;
        if total > MAX_V5_AGGREGATE_BYTES {
            return Err("v5 payload inventory exceeds the 64 MiB limit".to_string());
        }
        if payload.kind == "entry" {
            entry_count += 1;
        }
    }
    if manifest
        .payloads
        .iter()
        .filter(|payload| payload.path == manifest.entry && payload.kind == "entry")
        .count()
        != 1
        || entry_count != 1
        || !names.contains(manifest.entry.as_str())
    {
        return Err("v5 inventory must contain exactly the declared entry payload".to_string());
    }
    for path in license_paths {
        if manifest
            .payloads
            .iter()
            .filter(|payload| payload.path == path && payload.kind == "license")
            .count()
            != 1
        {
            return Err("v5 license references must resolve to license payloads".to_string());
        }
    }
    for path in attribution_paths {
        if manifest
            .payloads
            .iter()
            .filter(|payload| payload.path == path && payload.kind == "attribution")
            .count()
            != 1
        {
            return Err(
                "v5 attribution references must resolve to attribution payloads".to_string(),
            );
        }
    }
    if let Some(persona) = &manifest.persona {
        validate_bounded_text(persona, 1024, "persona", false)?;
    }
    validate_dialogue(manifest.dialogue.as_ref())
}

fn validate_dialogue(dialogue: Option<&BTreeMap<String, DialogueLocaleV4>>) -> Result<(), String> {
    let Some(dialogue) = dialogue else {
        return Ok(());
    };
    if dialogue.len() > 4 {
        return Err("v4 dialogue supports at most four locales".to_string());
    }
    for (locale, entry) in dialogue {
        if locale.is_empty()
            || locale.len() > 32
            || !locale.is_ascii()
            || locale.chars().any(|ch| ch.is_control())
        {
            return Err("v4 dialogue locale is invalid".to_string());
        }
        for (kind, values, known) in [
            ("phase", entry.phases.as_ref(), true),
            ("reaction", entry.reactions.as_ref(), false),
        ] {
            if let Some(values) = values {
                if values.len() > 4 {
                    return Err(format!("v4 dialogue {kind} map has too many entries"));
                }
                for (key, text) in values {
                    if (known && !known_phase(key)) || (!known && !known_reaction(key)) {
                        return Err(format!("v4 dialogue contains unknown {kind}"));
                    }
                    validate_bounded_text(text, 2048, "dialogue text", false)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_motion(
    motion: &MotionFileV4,
    base_pose: &str,
    alternate_pose: Option<&str>,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<(), String> {
    if !matches!(motion.version, 1 | 2)
        || motion.phases.len() != 4
        || motion.reactions.len() != 4
        || POSE_NAMES
            .iter()
            .any(|name| !motion.phases.contains_key(*name))
        || REACTION_ROLES
            .iter()
            .any(|(name, _)| !motion.reactions.contains_key(*name))
    {
        return Err(
            "rig motion must be version 1 or 2 and contain all four phases and reactions"
                .to_string(),
        );
    }
    let mut total_keys = 0usize;
    for values in motion.phases.values().chain(motion.reactions.values()) {
        total_keys += validate_motion_track(
            values,
            motion.version,
            base_pose,
            alternate_pose,
            canvas_width,
            canvas_height,
        )?;
    }
    if total_keys > 1024 {
        return Err("rig motion exceeds the total key limit".to_string());
    }
    Ok(())
}

fn validate_motion_track(
    motion: &MotionV4,
    version: u32,
    base_pose: &str,
    alternate_pose: Option<&str>,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<usize, String> {
    let layer_track_count = motion.layers.as_ref().map_or(0, |layers| {
        layers
            .values()
            .map(|layer| {
                [
                    layer.translate_x.as_ref(),
                    layer.translate_y.as_ref(),
                    layer.rotation_deg.as_ref(),
                    layer.scale.as_ref(),
                ]
                .into_iter()
                .flatten()
                .count()
            })
            .sum()
    });
    if !(1..=30_000).contains(&motion.duration_ms)
        || motion.transition_ms > motion.duration_ms.min(2_000)
        || motion.tracks.len() + layer_track_count > 40
        || motion.pose != base_pose && Some(motion.pose.as_str()) != alternate_pose
    {
        return Err("rig motion duration, transition, pose, or track limit is invalid".to_string());
    }
    match version {
        1 if motion.playback.is_some() || motion.layers.is_some() => {
            return Err("rig motion v1 contains v2 fields".to_string());
        }
        2 if motion.playback.is_none() || motion.layers.is_none() => {
            return Err("rig motion v2 requires playback and layers".to_string());
        }
        _ => {}
    }
    let mut keys = 0usize;
    for (name, track) in &motion.tracks {
        if name.is_empty()
            || name.len() > 128
            || !name.is_ascii()
            || name == "poseMix"
            || name.chars().any(|ch| ch.is_control())
        {
            return Err("rig motion track name is invalid".to_string());
        }
        let values = match (version, track) {
            (1, MotionTrackV4::Legacy(values)) => values.as_slice(),
            (2, MotionTrackV4::V2(track)) => track.keys.as_slice(),
            (1, MotionTrackV4::V2(_)) => {
                return Err("rig motion v1 uses a v2 track".to_string());
            }
            (2, MotionTrackV4::Legacy(_)) => {
                return Err("rig motion v2 uses a legacy track".to_string());
            }
            _ => return Err("rig motion version is invalid".to_string()),
        };
        validate_motion_keys(values, motion.duration_ms)?;
        keys += values.len();
        let Some((minimum, maximum)) = motion_parameter_bounds(name) else {
            return Err("rig motion track references an unknown parameter".to_string());
        };
        if values
            .iter()
            .any(|key| key.value < minimum || key.value > maximum)
        {
            return Err("rig motion parameter value is out of range".to_string());
        }
    }
    if let Some(layers) = &motion.layers {
        let translation_limit = f64::from(canvas_width.max(canvas_height));
        for (name, layer) in layers {
            if name.is_empty()
                || name.len() > 128
                || !name.is_ascii()
                || name.chars().any(|ch| ch.is_control())
            {
                return Err("rig motion layer name is invalid".to_string());
            }
            if !layer.origin.x.is_finite()
                || !layer.origin.y.is_finite()
                || !(0.0..=1.0).contains(&layer.origin.x)
                || !(0.0..=1.0).contains(&layer.origin.y)
                || !layer.influence.axis_x.is_finite()
                || !layer.influence.axis_y.is_finite()
                || !layer.influence.start.is_finite()
                || !layer.influence.end.is_finite()
                || layer.influence.axis_x.abs() > 1.0
                || layer.influence.axis_y.abs() > 1.0
                || layer.influence.axis_x.hypot(layer.influence.axis_y) < 1e-6
                || layer.influence.start < -2.0
                || layer.influence.end > 2.0
                || layer.influence.start >= layer.influence.end
            {
                return Err("rig motion layer origin or influence is invalid".to_string());
            }
            let tracks = [
                ("translateX", layer.translate_x.as_ref()),
                ("translateY", layer.translate_y.as_ref()),
                ("rotationDeg", layer.rotation_deg.as_ref()),
                ("scale", layer.scale.as_ref()),
            ];
            if tracks.iter().all(|(_, track)| track.is_none()) {
                return Err("rig motion layer has no transform tracks".to_string());
            }
            for (kind, track) in tracks
                .into_iter()
                .filter_map(|(kind, track)| track.map(|track| (kind, track)))
            {
                validate_motion_key_track(track, motion.duration_ms)?;
                if track.keys.iter().any(|key| match kind {
                    "translateX" | "translateY" => key.value.abs() > translation_limit,
                    "rotationDeg" => key.value.abs() > 360.0,
                    "scale" => !(0.0..=4.0).contains(&key.value),
                    _ => true,
                }) {
                    return Err("rig motion layer transform value is out of range".to_string());
                }
                keys += track.keys.len();
            }
        }
    }
    if keys > 256 {
        return Err("rig motion exceeds the per-motion key limit".to_string());
    }
    Ok(keys)
}

fn motion_parameter_bounds(name: &str) -> Option<(f64, f64)> {
    match name {
        "angleX" | "angleY" | "angleZ" | "eyeX" | "eyeY" | "brow" | "browAngL" | "browAngR"
        | "browAngSym" | "mouthForm" | "mouthCY" | "body" | "armY" | "armPos" | "bangL"
        | "bangC" | "bangR" | "eyeCY" | "eyeCAng" | "mouthCAng" => Some((-1.0, 1.0)),
        "eyeOpenL" | "eyeOpenR" | "mouthOpen" | "eyeEase" | "mouthEase" => Some((0.0, 1.0)),
        "irisScale" => Some((0.5, 1.3)),
        "physAmp" | "soft" | "fhAmp" => Some((0.0, 3.0)),
        "bust" => Some((0.0, 4.0)),
        "bustY" => Some((-3.0, 3.0)),
        "fhSoft" => Some((0.0, 2.0)),
        "eyeScaleL" | "eyeScaleR" | "mouthScale" => Some((0.5, 1.5)),
        _ => None,
    }
}

fn validate_motion_key_track(track: &MotionKeyTrackV4, duration_ms: u32) -> Result<(), String> {
    validate_motion_keys(&track.keys, duration_ms)
}

fn validate_motion_keys(
    values: &[pack_format::MotionKeyV4],
    duration_ms: u32,
) -> Result<(), String> {
    if values.is_empty() || values.len() > 32 {
        return Err("rig motion track must contain 1..32 keys".to_string());
    }
    let mut previous = None;
    for key in values {
        if key.at_ms > duration_ms
            || !key.value.is_finite()
            || previous.is_some_and(|at| key.at_ms <= at)
        {
            return Err(
                "rig motion keys must be finite and strictly increasing within duration"
                    .to_string(),
            );
        }
        if previous.is_none() && key.at_ms != 0 {
            return Err("rig motion tracks must begin at time zero".to_string());
        }
        previous = Some(key.at_ms);
    }
    Ok(())
}

fn validate_overrides(bytes: &[u8], width: u32, height: u32) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("rig overrides are invalid JSON: {error}"))?;
    reject_duplicate_json_keys(bytes)?;
    reject_explicit_nulls(&value)?;
    let mut nodes = 0;
    validate_json_tree(&value, 0, &mut nodes)?;
    let object = value
        .as_object()
        .ok_or_else(|| "rig overrides must be an object".to_string())?;
    let allowed = [
        "layerAliases",
        "layerOrder",
        "layerOrderConstraints",
        "maskedLayerOverlays",
        "interpolatedPatchRepairs",
        "depthOverrides",
        "groupOverrides",
        "deformationSources",
        "meshSources",
        "hairAttachments",
        "headFollow",
        "cleanupThresholds",
        "hiddenLayers",
        "excludeAfterMeshResolution",
        "anchorOverrides",
        "interactionAreas",
        "mouthExpressions",
        "blinkRepair",
        "hairSplit",
        "physics",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("rig overrides contain an unknown root field".to_string());
    }
    let areas = object
        .get("interactionAreas")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "rig overrides require interactionAreas".to_string())?;
    for area in ["head", "torso"] {
        let bounds = areas
            .get(area)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("rig overrides require interaction area {area}"))?;
        let x0 = finite_number(bounds, "x0")?;
        let y0 = finite_number(bounds, "y0")?;
        let x1 = finite_number(bounds, "x1")?;
        let y1 = finite_number(bounds, "y1")?;
        if !(x0 >= 0.0
            && y0 >= 0.0
            && x1 > x0
            && y1 > y0
            && x1 <= f64::from(width)
            && y1 <= f64::from(height))
        {
            return Err(format!("rig interaction area {area} is outside the canvas"));
        }
    }
    validate_override_graphs(object)?;
    Ok(())
}

fn finite_number(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<f64, String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("rig interaction area field {key} is invalid"))
}

fn validate_override_graphs(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in ["layerAliases", "deformationSources", "meshSources"] {
        if let Some(values) = object.get(key).and_then(serde_json::Value::as_object) {
            for (from, target) in values {
                let target = target
                    .as_str()
                    .ok_or_else(|| format!("rig override graph {key} has a non-string target"))?;
                edges
                    .entry(from.clone())
                    .or_default()
                    .push(target.to_string());
                edges.entry(target.to_string()).or_default();
            }
        }
    }
    if let Some(values) = object
        .get("layerOrderConstraints")
        .and_then(serde_json::Value::as_array)
    {
        for value in values {
            let constraint = value
                .as_object()
                .ok_or_else(|| "rig layer order constraint must be an object".to_string())?;
            let from = constraint
                .get("behind")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "rig layer order constraint is missing behind".to_string())?;
            let to = constraint
                .get("inFrontOf")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "rig layer order constraint is missing inFrontOf".to_string())?;
            if from == to {
                return Err("rig layer order graph contains a self-cycle".to_string());
            }
            edges
                .entry(from.to_string())
                .or_default()
                .push(to.to_string());
            edges.entry(to.to_string()).or_default();
        }
    }
    if let Some(values) = object
        .get("maskedLayerOverlays")
        .and_then(serde_json::Value::as_array)
    {
        for value in values {
            let overlay = value
                .as_object()
                .ok_or_else(|| "rig masked overlay must be an object".to_string())?;
            let from = overlay.get("inFrontOf").and_then(serde_json::Value::as_str);
            let to = overlay.get("name").and_then(serde_json::Value::as_str);
            if let (Some(from), Some(to)) = (from, to) {
                edges
                    .entry(from.to_string())
                    .or_default()
                    .push(to.to_string());
                edges.entry(to.to_string()).or_default();
            }
        }
    }
    let mut state = BTreeMap::<&str, u8>::new();
    fn visit<'a>(
        node: &'a str,
        edges: &'a BTreeMap<String, Vec<String>>,
        state: &mut BTreeMap<&'a str, u8>,
    ) -> Result<(), String> {
        match state.get(node).copied().unwrap_or(0) {
            1 => return Err("rig override graph contains a cycle".to_string()),
            2 => return Ok(()),
            _ => {}
        }
        state.insert(node, 1);
        if let Some(targets) = edges.get(node) {
            for target in targets {
                visit(target.as_str(), edges, state)?;
            }
        }
        state.insert(node, 2);
        Ok(())
    }
    for node in edges.keys() {
        visit(node.as_str(), &edges, &mut state)?;
    }
    Ok(())
}

fn validate_json_tree(
    value: &serde_json::Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("JSON graph exceeds the depth limit".to_string());
    }
    *nodes += 1;
    if *nodes > MAX_JSON_NODES {
        return Err("JSON graph exceeds the node limit".to_string());
    }
    match value {
        serde_json::Value::Array(values) => {
            if values.len() > 4096 {
                return Err("JSON array exceeds the limit".to_string());
            }
            for value in values {
                validate_json_tree(value, depth + 1, nodes)?;
            }
        }
        serde_json::Value::Object(values) => {
            if values.len() > 2048 {
                return Err("JSON object exceeds the key limit".to_string());
            }
            for (key, value) in values {
                if matches!(key.as_str(), "__proto__" | "prototype" | "constructor") {
                    return Err("JSON graph contains a dangerous key".to_string());
                }
                validate_json_tree(value, depth + 1, nodes)?;
            }
        }
        serde_json::Value::String(text) => {
            if text.len() > 8192 || text.contains('\0') {
                return Err("JSON string exceeds the limit".to_string());
            }
        }
        serde_json::Value::Number(number) => {
            if number
                .as_f64()
                .map(|value| !value.is_finite())
                .unwrap_or(true)
            {
                return Err("JSON number is not finite".to_string());
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) => {}
    }
    Ok(())
}

fn validate_psd(bytes: &[u8], width: u32, height: u32, label: &str) -> Result<(u32, u32), String> {
    if bytes.len() < 26 || &bytes[..4] != PSD_SIGNATURE {
        return Err(format!("{label} is not a PSD"));
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    if version != 1 && version != 2 {
        return Err(format!("{label} PSD version is unsupported"));
    }
    let psd_height = u32::from_be_bytes(bytes[14..18].try_into().unwrap());
    let psd_width = u32::from_be_bytes(bytes[18..22].try_into().unwrap());
    if psd_width != width || psd_height != height {
        return Err(format!("{label} dimensions do not match the manifest"));
    }
    Ok((psd_width, psd_height))
}

fn convert_regions(
    regions: &FrameRegionsV4,
    width: u32,
    height: u32,
) -> Result<FrameRegions, String> {
    Ok(FrameRegions {
        head: convert_region(&regions.head, width, height)?,
        body: convert_region(&regions.body, width, height)?,
    })
}

fn convert_region(region: &RegionRectV4, width: u32, height: u32) -> Result<RegionRect, String> {
    let values = [region.x0, region.y0, region.x1, region.y1];
    if values.iter().any(|value| !value.is_finite())
        || region.x0 < 0.0
        || region.y0 < 0.0
        || region.x1 <= region.x0
        || region.y1 <= region.y0
        || region.x1 > f64::from(width)
        || region.y1 > f64::from(height)
    {
        return Err("v4 frame region is outside the canvas".to_string());
    }
    Ok(RegionRect {
        x0: region.x0,
        y0: region.y0,
        x1: region.x1,
        y1: region.y1,
    })
}

fn parse_json_payload<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    label: &str,
) -> Result<T, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("{label} is invalid JSON: {error}"))?;
    reject_duplicate_json_keys(bytes)?;
    reject_explicit_nulls(&value)?;
    serde_json::from_value(value).map_err(|error| format!("{label} is invalid: {error}"))
}

fn reject_explicit_nulls(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Null => {
            Err("v4 JSON fields must omit optional values instead of using null".to_string())
        }
        serde_json::Value::Array(values) => values.iter().try_for_each(reject_explicit_nulls),
        serde_json::Value::Object(values) => values.values().try_for_each(reject_explicit_nulls),
        _ => Ok(()),
    }
}

fn validate_identity(id: &str, allow_builtin: bool) -> Result<(), String> {
    if allow_builtin && id == "default" {
        return Ok(());
    }
    validate_pack_id(id)
}

fn validate_rig_pose_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 32
        || !id.is_ascii()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("rig pose ID must be 1..32 ASCII characters".to_string());
    }
    Ok(())
}

fn validate_v4_payload_name(name: &str) -> Result<(), String> {
    validate_v4_payload_name_allow_manifest(name)?;
    if name == "manifest.json" {
        return Err("v4 payload path cannot be manifest.json".to_string());
    }
    Ok(())
}

fn validate_v4_payload_name_allow_manifest(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || !name.is_ascii()
        || name.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
        || name.contains('/')
        || name.contains('\\')
        || name.starts_with('.')
        || name.ends_with('.')
        || name == "."
        || name == ".."
    {
        return Err("v4 payload path must be a root-only ASCII filename".to_string());
    }
    Ok(())
}

fn validate_bounded_text(value: &str, max: usize, label: &str, url: bool) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        || value.contains('\0')
    {
        return Err(format!("{label} is invalid"));
    }
    if url && value.chars().any(char::is_whitespace) {
        return Err(format!("{label} contains whitespace"));
    }
    Ok(())
}

#[cfg(unix)]
struct AssetRoot {
    fd: OwnedFd,
    private: bool,
}
#[cfg(not(unix))]
struct AssetRoot {
    path: PathBuf,
    private: bool,
}

impl AssetRoot {
    fn open(root: &Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let bytes = root.as_os_str().as_bytes();
            let path =
                CString::new(bytes).map_err(|_| "asset directory path contains NUL".to_string())?;
            let raw_fd = unsafe {
                libc::open(
                    path.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
                )
            };
            if raw_fd < 0 {
                return Err(format!(
                    "asset directory is unavailable: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
            let metadata =
                std::fs::File::from(fd.try_clone().map_err(|error| {
                    format!("asset directory metadata cannot be read: {error}")
                })?)
                .metadata()
                .map_err(|error| format!("asset directory metadata cannot be read: {error}"))?;
            if !metadata.is_dir() {
                return Err("asset directory must be a real directory".to_string());
            }
            Ok(Self { fd, private: false })
        }
        #[cfg(not(unix))]
        {
            let metadata = fs::symlink_metadata(root)
                .map_err(|error| format!("asset directory is unavailable: {error}"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("asset directory must be a real directory".to_string());
            }
            Ok(Self {
                path: root.to_path_buf(),
                private: false,
            })
        }
    }

    fn from_directory(directory: &File) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let raw_fd = unsafe { libc::dup(directory.as_raw_fd()) };
            if raw_fd < 0 {
                return Err(format!(
                    "asset directory descriptor cannot be duplicated: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
            let metadata = unsafe { std::mem::zeroed::<libc::stat>() };
            let mut metadata = metadata;
            if unsafe { libc::fstat(fd.as_raw_fd(), &mut metadata) } != 0
                || metadata.st_mode & libc::S_IFMT != libc::S_IFDIR
            {
                return Err("asset directory must be a real directory".to_string());
            }
            Ok(Self { fd, private: true })
        }
        #[cfg(not(unix))]
        {
            let _ = directory;
            Err("descriptor-relative asset loading is unavailable on this platform".to_string())
        }
    }

    fn read(&self, filename: &str, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
        #[cfg(unix)]
        {
            read_bounded_regular_at(
                self.fd.as_raw_fd(),
                filename,
                max_bytes,
                label,
                self.private,
                false,
            )
        }
        #[cfg(not(unix))]
        {
            read_bounded_regular(&self.path.join(filename), max_bytes, label, false)
        }
    }

    fn snapshot_managed(&self) -> Result<SourceSnapshot, String> {
        let mut entries = BTreeMap::new();
        let mut total = 0usize;
        for name in self.names()? {
            let name_text = name
                .to_str()
                .ok_or_else(|| "managed source filenames must be ASCII".to_string())?;
            validate_v4_payload_name_allow_manifest(name_text)?;
            let limit = MAX_FILE_BYTES.min(MAX_EXPANDED_BYTES.saturating_sub(total));
            #[cfg(unix)]
            let bytes = read_bounded_regular_at(
                self.fd.as_raw_fd(),
                name_text,
                limit,
                "managed payload",
                self.private,
                true,
            )?;
            #[cfg(not(unix))]
            let bytes =
                read_bounded_regular(&self.path.join(name_text), limit, "managed payload", true)?;
            total += bytes.len();
            entries.insert(name_text.to_string(), Arc::new(bytes));
        }
        Ok(SourceSnapshot { entries })
    }

    #[cfg(unix)]
    fn names(&self) -> Result<Vec<std::ffi::OsString>, String> {
        let dot = CString::new(".").unwrap();
        let duplicate = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                dot.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if duplicate < 0 {
            return Err(format!(
                "cannot enumerate asset directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            unsafe {
                libc::close(duplicate);
            }
            return Err("cannot enumerate asset directory".to_string());
        }
        let mut names = Vec::new();
        loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes != b"." && bytes != b".." {
                names.push(std::ffi::OsString::from_vec(bytes.to_vec()));
            }
            if names.len() > V4_MAX_PAYLOADS + 1 {
                unsafe {
                    libc::closedir(stream);
                }
                return Err("v4 source contains too many root files".to_string());
            }
        }
        unsafe {
            libc::closedir(stream);
        }
        Ok(names)
    }

    #[cfg(not(unix))]
    fn names(&self) -> Result<Vec<std::ffi::OsString>, String> {
        fs::read_dir(&self.path)
            .map_err(|error| format!("cannot enumerate asset directory: {error}"))?
            .map(|entry| {
                entry
                    .map(|entry| entry.file_name())
                    .map_err(|error| format!("cannot enumerate asset directory: {error}"))
            })
            .collect()
    }
}

#[cfg(unix)]
fn read_bounded_regular_at(
    directory_fd: std::os::fd::RawFd,
    filename: &str,
    max_bytes: usize,
    label: &str,
    private: bool,
    single_link: bool,
) -> Result<Vec<u8>, String> {
    let filename = CString::new(filename).map_err(|_| format!("{label} has an unsafe filename"))?;
    let raw_fd = unsafe {
        libc::openat(
            directory_fd,
            filename.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw_fd < 0 {
        return Err(format!(
            "{label} is unavailable: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_fd(raw_fd) };
    read_open_regular(file, max_bytes, label, private, single_link)
}

#[cfg(not(unix))]
fn read_bounded_regular(
    path: &Path,
    max_bytes: usize,
    label: &str,
    single_link: bool,
) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{label} is unavailable: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} must be a regular file"));
    }
    if single_link && metadata.file_type().is_symlink() {
        return Err(format!("{label} must not be linked"));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(format!("{label} exceeds the file limit"));
    }
    read_open_regular(
        File::open(path).map_err(|error| format!("{label} cannot be read: {error}"))?,
        max_bytes,
        label,
        false,
        single_link,
    )
}

fn read_open_regular(
    file: File,
    max_bytes: usize,
    label: &str,
    private: bool,
    single_link: bool,
) -> Result<Vec<u8>, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("{label} metadata cannot be read: {error}"))?;
    #[cfg(unix)]
    if private || single_link {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(format!(
                "{label} metadata cannot be read: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG
            || (private
                && (stat.st_uid != unsafe { libc::geteuid() } as libc::uid_t
                    || stat.st_nlink != 1
                    || stat.st_mode & 0o077 != 0))
            || (single_link && stat.st_nlink != 1)
        {
            return Err(format!("{label} must be a regular single-link file"));
        }
        if stat.st_mode & 0o111 != 0 {
            return Err(format!("{label} must not be executable"));
        }
    }
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err(format!("{label} exceeds the file limit or is not regular"));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(max_bytes)
            .min(max_bytes),
    );
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label} cannot be read: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!("{label} exceeds the file limit"));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PngHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.version != 1
        || manifest.name.trim().is_empty()
        || manifest.width != POSE_WIDTH
        || manifest.height != POSE_HEIGHT
    {
        return Err("legacy asset manifest is invalid".to_string());
    }
    if manifest.poses.len() != 4
        || POSE_NAMES
            .iter()
            .any(|name| !manifest.poses.contains_key(*name))
    {
        return Err(
            "asset manifest must contain exactly idle, running, waiting, and unknown poses"
                .to_string(),
        );
    }
    manifest
        .poses
        .values()
        .try_for_each(|filename| validate_filename(filename))
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 255
        || filename.contains('/')
        || filename.contains('\\')
        || filename.chars().any(char::is_control)
    {
        return Err("pose filename is not a safe same-directory name".to_string());
    }
    let mut components = Path::new(filename).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("pose filename must contain one normal path component".to_string()),
    }
}

fn validate_v3_filename(filename: &str) -> Result<(), String> {
    validate_filename(filename)?;
    if filename == "manifest.json" || !filename.ends_with(".png") {
        return Err("managed v3 frame filename must be a .png file, not manifest.json".to_string());
    }
    Ok(())
}

fn parse_png_header(bytes: &[u8]) -> Result<PngHeader, String> {
    parse_png_header_for_canvas(bytes, POSE_WIDTH, POSE_HEIGHT)
}

fn parse_png_header_for_canvas(
    bytes: &[u8],
    width_expected: u32,
    height_expected: u32,
) -> Result<PngHeader, String> {
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE {
        return Err("PNG signature is missing".to_string());
    }
    let ihdr_length = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    if ihdr_length != 13 || &bytes[12..16] != b"IHDR" {
        return Err("PNG IHDR is missing or malformed".to_string());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    let bit_depth = bytes[24];
    let color_type = bytes[25];
    if width == 0 || height == 0 || width != width_expected || height != height_expected {
        return Err("PNG dimensions do not match the declared canvas".to_string());
    }
    if !matches!(bit_depth, 1 | 2 | 4 | 8 | 16) || !matches!(color_type, 0 | 2 | 3 | 4 | 6) {
        return Err("PNG encoding is unsupported".to_string());
    }
    Ok(PngHeader {
        width,
        height,
        bit_depth,
        color_type,
    })
}

fn validate_png_budget(
    header: PngHeader,
    file_bytes: usize,
    aggregate_file_bytes: u64,
    aggregate_decoded_bytes: u64,
) -> Result<u64, String> {
    validate_png_budget_with_limits(
        header,
        file_bytes,
        aggregate_file_bytes,
        aggregate_decoded_bytes,
        MAX_AGGREGATE_BYTES,
        MAX_AGGREGATE_BYTES,
    )
}

fn validate_png_budget_with_limits(
    header: PngHeader,
    file_bytes: usize,
    aggregate_file_bytes: u64,
    aggregate_decoded_bytes: u64,
    encoded_limit: u64,
    decoded_limit: u64,
) -> Result<u64, String> {
    if file_bytes > MAX_FILE_BYTES {
        return Err("file exceeds the size limit".to_string());
    }
    let pixels = u64::from(header.width)
        .checked_mul(u64::from(header.height))
        .ok_or_else(|| "decoded pixel count overflow".to_string())?;
    if pixels > MAX_DECODED_PIXELS {
        return Err("decoded pixel count exceeds the limit".to_string());
    }
    let decoded_upper_bound = pixels
        .checked_mul(8)
        .ok_or_else(|| "decoded byte count overflow".to_string())?;
    if aggregate_decoded_bytes
        .checked_add(decoded_upper_bound)
        .ok_or_else(|| "asset decoded size overflow".to_string())?
        > decoded_limit
    {
        return Err("decoded bytes exceed the aggregate limit".to_string());
    }
    if aggregate_file_bytes
        .checked_add(file_bytes as u64)
        .ok_or_else(|| "asset file size overflow".to_string())?
        > encoded_limit
    {
        return Err("asset files exceed the aggregate limit".to_string());
    }
    Ok(decoded_upper_bound)
}

fn decode_png_mask(
    bytes: &[u8],
    header: PngHeader,
    reject_animation: bool,
) -> Result<AlphaMask, String> {
    if reject_animation {
        reject_png_animation(bytes)?;
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_FILE_BYTES,
    });
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("PNG decoder rejected header: {error}"))?;
    let output_size = reader.output_buffer_size();
    if output_size == 0 || u64::try_from(output_size).unwrap_or(u64::MAX) > MAX_AGGREGATE_BYTES {
        return Err("decoded PNG exceeds the byte limit".to_string());
    }
    let mut output = vec![0u8; output_size];
    let info = reader
        .next_frame(&mut output)
        .map_err(|error| format!("PNG decoder rejected image data: {error}"))?;
    let decoded_len = info.buffer_size();
    if info.width != header.width
        || info.height != header.height
        || decoded_len == 0
        || decoded_len > output.len()
    {
        return Err("decoded PNG dimensions are inconsistent".to_string());
    }
    if info.bit_depth != png::BitDepth::Eight {
        return Err("decoded PNG was not normalized to 8-bit samples".to_string());
    }
    let decoded = &output[..decoded_len];
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err("decoded PNG retained an indexed color type".to_string())
        }
    };
    let expected_len = usize::try_from(header.width)
        .ok()
        .and_then(|width| {
            usize::try_from(header.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| "decoded PNG byte count overflows usize".to_string())?;
    if decoded.len() != expected_len {
        return Err("decoded PNG buffer size is inconsistent".to_string());
    }
    match info.color_type {
        png::ColorType::GrayscaleAlpha => {
            AlphaMask::from_alpha_channel(header.width, header.height, decoded, 2, 1)
        }
        png::ColorType::Rgba => AlphaMask::from_rgba(header.width, header.height, decoded),
        png::ColorType::Grayscale | png::ColorType::Rgb => {
            AlphaMask::fully_opaque(header.width, header.height)
        }
        png::ColorType::Indexed => unreachable!(),
    }
}

fn reject_png_animation(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < PNG_SIGNATURE.len() || &bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err("PNG signature is missing".to_string());
    }
    let mut offset = PNG_SIGNATURE.len();
    while offset < bytes.len() {
        if bytes.len() - offset < 12 {
            return Err("PNG chunk is truncated".to_string());
        }
        let length = usize::try_from(u32::from_be_bytes(
            bytes[offset..offset + 4].try_into().unwrap(),
        ))
        .map_err(|_| "PNG chunk length overflows usize".to_string())?;
        let data_start = offset
            .checked_add(8)
            .ok_or_else(|| "PNG chunk offset overflow".to_string())?;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| "PNG chunk length overflows usize".to_string())?;
        let crc_end = data_end
            .checked_add(4)
            .ok_or_else(|| "PNG chunk offset overflow".to_string())?;
        if crc_end > bytes.len() {
            return Err("PNG chunk is truncated".to_string());
        }
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"acTL" || kind == b"fcTL" || kind == b"fdAT" {
            return Err("animated PNGs are not supported".to_string());
        }
        offset = crc_end;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(())
}

struct DuplicateKeySeed;
struct DuplicateKeyVisitor;
impl<'de> DeserializeSeed<'de> for DuplicateKeySeed {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateKeyVisitor)
    }
}
impl<'de> Visitor<'de> for DuplicateKeyVisitor {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("any JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_borrowed_str<E>(self, _: &'de str) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_bytes<E>(self, _: &[u8]) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_byte_buf<E>(self, _: Vec<u8>) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_none<E>(self) -> Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateKeyVisitor)
    }
    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateKeyVisitor)
    }
    fn visit_seq<A>(self, mut sequence: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(DuplicateKeySeed)?.is_some() {}
        Ok(())
    }
    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            map.next_value_seed(DuplicateKeySeed)?;
        }
        Ok(())
    }
}
fn reject_duplicate_json_keys(bytes: &[u8]) -> Result<(), String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer
        .deserialize_any(DuplicateKeyVisitor)
        .map_err(|error| format!("JSON contains duplicate keys: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("JSON is invalid: {error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_canvas_limit_does_not_inherit_png_pixel_limit() {
        // A manifest-only fixture: validation of dimensions needs no artwork on disk.
        let fixture = |render_mode: &str| {
            let (backend, capabilities) = if render_mode == "rig" {
                ("herdr-native-rig", vec!["mesh-deformation", "eye-stencil"])
            } else {
                ("herdr-native-png", vec!["frame-clips", "frame-regions"])
            };
            serde_json::json!({
                "format": "herdr.character",
                "version": 4,
                "render_mode": render_mode,
                "id": "canvas-fixture",
                "name": "Canvas Fixture",
                "width": 1024,
                "height": 1024,
                "author": {"name": "Fixture"},
                "source": {"method": "procedural", "description": "Generated for dimension validation"},
                "licenses": [{"expression": "MIT", "path": "LICENSE"}],
                "attributions": [],
                "entry": "entry.json",
                "runtime": {"name": backend, "version": 1, "capabilities": capabilities},
                "payloads": [
                    {"path": "entry.json", "size": 0, "sha256": "0".repeat(64), "kind": "entry"},
                    {"path": "LICENSE", "size": 0, "sha256": "0".repeat(64), "kind": "license"}
                ]
            })
        };

        let rig_value = fixture("rig");
        let mut rig: ManifestV4 = serde_json::from_value(rig_value.clone()).unwrap();
        assert!(validate_v4_manifest(&rig, false).is_ok());
        rig.width = 1025;
        assert_eq!(
            validate_v4_manifest(&rig, false).unwrap_err(),
            "v4 canvas dimensions exceed the limit"
        );
        rig.width = 1024;
        rig.height = 1025;
        assert_eq!(
            validate_v4_manifest(&rig, false).unwrap_err(),
            "v4 canvas dimensions exceed the limit"
        );

        let mut catalog_value = rig_value;
        catalog_value["version"] = serde_json::json!(5);
        catalog_value["runtime"]["version"] = serde_json::json!(2);
        catalog_value["runtime"]["capabilities"] =
            serde_json::json!(["mesh-deformation", "eye-stencil", "independent-models"]);
        let mut catalog: ManifestV5 = serde_json::from_value(catalog_value).unwrap();
        assert!(validate_v5_manifest(&catalog, false).is_ok());
        catalog.width = 1025;
        assert_eq!(
            validate_v5_manifest(&catalog, false).unwrap_err(),
            "v5 canvas dimensions exceed the limit"
        );
        catalog.width = 1024;
        catalog.height = 1025;
        assert_eq!(
            validate_v5_manifest(&catalog, false).unwrap_err(),
            "v5 canvas dimensions exceed the limit"
        );

        let mut png: ManifestV4 = serde_json::from_value(fixture("png")).unwrap();
        png.width = 1000;
        png.height = 1000;
        assert!(validate_v4_manifest(&png, false).is_ok());
        png.width = 1024;
        png.height = 1024;
        assert_eq!(
            validate_v4_manifest(&png, false).unwrap_err(),
            "v4 canvas dimensions exceed the limit"
        );
    }
}
