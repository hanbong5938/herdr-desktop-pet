use crate::assets::ValidatedCharacter;
use crate::behavior::{EffectKind, EffectSnapshot, PresentationIntent, PresentationViewport};
use crate::character_renderer::{self, PreparedCharacter};
use crate::character_store::PackStore;
use crate::character_types::{CharacterRef, RendererToken};
use crate::pose::{PoseKind, PoseSnapshot};
use crate::state::Phase;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PREVIEW_MILLIS: u64 = 86_400_000;
const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewPhase {
    Idle,
    Running,
    Waiting,
    Unknown,
}

impl PreviewPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Unknown => "unknown",
        }
    }

    fn phase(self) -> Phase {
        match self {
            Self::Idle => Phase::Idle,
            Self::Running => Phase::Running,
            Self::Waiting => Phase::Waiting,
            Self::Unknown => Phase::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewReaction {
    HeadTap,
    BodyTap,
    Pet,
    CompletionObserved,
}

impl PreviewReaction {
    fn as_str(self) -> &'static str {
        match self {
            Self::HeadTap => "head_tap",
            Self::BodyTap => "body_tap",
            Self::Pet => "pet",
            Self::CompletionObserved => "completion_observed",
        }
    }

    fn effect_kind(self) -> EffectKind {
        match self {
            Self::HeadTap => EffectKind::HeadTap,
            Self::BodyTap => EffectKind::BodyTap,
            Self::Pet => EffectKind::Pet,
            Self::CompletionObserved => EffectKind::CompletionObserved,
        }
    }
}

pub(crate) fn parse_phase(value: &str) -> Result<PreviewPhase, String> {
    match value {
        "idle" => Ok(PreviewPhase::Idle),
        "running" => Ok(PreviewPhase::Running),
        "waiting" => Ok(PreviewPhase::Waiting),
        "unknown" => Ok(PreviewPhase::Unknown),
        _ => Err(format!(
            "invalid preview phase {value:?}; expected idle, running, waiting, or unknown"
        )),
    }
}

pub(crate) fn parse_reaction(value: &str) -> Result<PreviewReaction, String> {
    match value {
        "head_tap" => Ok(PreviewReaction::HeadTap),
        "body_tap" => Ok(PreviewReaction::BodyTap),
        "pet" => Ok(PreviewReaction::Pet),
        "completion_observed" => Ok(PreviewReaction::CompletionObserved),
        _ => Err(format!(
            "invalid preview reaction {value:?}; expected head_tap, body_tap, pet, or completion_observed"
        )),
    }
}

pub(crate) fn parse_millis(value: &str, option: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{option} requires a nonnegative millisecond value"));
    }
    let millis = value
        .parse::<u64>()
        .map_err(|_| format!("{option} is out of range"))?;
    if millis > MAX_PREVIEW_MILLIS {
        return Err(format!(
            "{option} must be at most {MAX_PREVIEW_MILLIS} milliseconds"
        ));
    }
    Ok(millis)
}

#[derive(Serialize)]
struct ValidateResult {
    command: &'static str,
    path: String,
    backend: &'static str,
    content_digest: String,
    width: u32,
    height: u32,
    prepared: bool,
}

#[derive(Serialize)]
struct PreviewResult {
    command: &'static str,
    path: String,
    output: String,
    backend: &'static str,
    content_digest: String,
    width: u32,
    height: u32,
    phase: &'static str,
    time_ms: u64,
    reaction: Option<&'static str>,
    reaction_age_ms: Option<u64>,
    pose: Option<PoseKind>,
    hit_overlay: bool,
    bytes: usize,
}

#[derive(Serialize)]
struct ExportResult {
    command: &'static str,
    id: String,
    revision: u64,
    output: String,
    bytes: u64,
}

pub(crate) fn validate(path: &Path) -> Result<(), String> {
    let (prepared, digest, backend) = load_and_prepare(path, "pack-validate")?;
    let (width, height) = prepared.canvas_size();
    emit_json(&ValidateResult {
        command: "validate",
        path: path.display().to_string(),
        backend,
        content_digest: digest,
        width,
        height,
        prepared: true,
    })
}
pub(crate) fn preview(
    path: &Path,
    output: &Path,
    phase: PreviewPhase,
    time_ms: u64,
    reaction: Option<PreviewReaction>,
    reaction_age_ms: Option<u64>,
    pose: Option<PoseKind>,
    hit_overlay: bool,
) -> Result<(), String> {
    let output = prepare_output_path(output)?;
    let (mut prepared, digest, backend) = load_and_prepare(path, "pack-preview")?;
    if pose.is_some()
        && !matches!(&prepared, PreparedCharacter::Rig(rig) if rig.supports_independent_models())
    {
        return Err("--pose requires a v5 independent-model rig pack".to_owned());
    }
    let (width, height) = prepared.canvas_size();
    let mut intent = preview_intent(phase, time_ms, reaction, reaction_age_ms, width, height)?;
    intent.pose = pose.map(|kind| PoseSnapshot {
        kind,
        age: intent.phase_age,
    });
    let native_png = prepared.preview_png(intent, hit_overlay)?;
    let overlay = (hit_overlay && matches!(&prepared, PreparedCharacter::Png(_)))
        .then_some((&prepared, intent));
    let png = normalize_preview_png(native_png, (width, height), overlay)?;
    write_new_file(&output, &png)?;
    emit_json(&PreviewResult {
        command: "preview",
        path: path.display().to_string(),
        output: output.display().to_string(),
        backend,
        content_digest: digest,
        width,
        height,
        phase: phase.as_str(),
        time_ms,
        reaction: reaction.map(PreviewReaction::as_str),
        reaction_age_ms: reaction.map(|_| reaction_age_ms.unwrap_or(0)),
        pose,
        hit_overlay,
        bytes: png.len(),
    })
}

pub(crate) fn export(
    config_dir: &Path,
    builtin_assets: Option<&Path>,
    id: &str,
    revision: Option<u64>,
    output: &Path,
) -> Result<(), String> {
    let output = prepare_output_path(output)?;
    let store = PackStore::new(
        config_dir.to_path_buf(),
        builtin_assets.map(|path| path.to_path_buf()),
    );
    let reference = if id == CharacterRef::builtin().id {
        if revision.is_some_and(|revision| revision != 0) {
            return Err("builtin character revision must be zero".to_owned());
        }
        CharacterRef::builtin()
    } else {
        let listing = store.list()?;
        let Some(record) = listing.packs.iter().find(|record| record.id == id) else {
            return Err(listing
                .error
                .unwrap_or_else(|| format!("character pack {id} does not exist")));
        };
        CharacterRef {
            id: id.to_owned(),
            revision: revision.unwrap_or(record.head),
        }
    };
    store.export(&reference, &output)?;
    let metadata = fs::symlink_metadata(&output).map_err(|error| {
        format!(
            "cannot inspect archive export {}: {error}",
            output.display()
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "archive export {} is not a regular file",
            output.display()
        ));
    }
    let bytes = metadata.len();
    emit_json(&ExportResult {
        command: "export",
        id: id.to_owned(),
        revision: reference.revision,
        output: output.display().to_string(),
        bytes,
    })
}

fn initialize_appkit() -> Result<MainThreadMarker, String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "offline pack authoring must run on the AppKit main thread".to_owned())?;
    let app = NSApplication::sharedApplication(mtm);
    // Prohibited keeps this process out of the Dock and does not activate it;
    // no windows or application delegate are installed for authoring commands.
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    app.finishLaunching();
    Ok(mtm)
}

fn load_and_prepare(
    path: &Path,
    operation_id: &str,
) -> Result<(PreparedCharacter, String, &'static str), String> {
    let assets = ValidatedCharacter::load_managed(path)
        .map_err(|error| format!("cannot validate character source: {error}"))?;
    let digest = assets.content_digest().to_owned();
    let backend = match &assets {
        ValidatedCharacter::Png(_) => "png",
        ValidatedCharacter::Rig(_) => "rig",
    };
    let token = RendererToken::new(
        operation_id.to_owned(),
        CharacterRef::builtin(),
        digest.clone(),
    )?;
    let mtm = initialize_appkit()?;
    let prepared = character_renderer::prepare(assets, token, mtm)
        .map_err(|error| format!("native character preparation failed: {error}"))?;
    Ok((prepared, digest, backend))
}

fn preview_intent(
    phase: PreviewPhase,
    time_ms: u64,
    reaction: Option<PreviewReaction>,
    reaction_age_ms: Option<u64>,
    width: u32,
    height: u32,
) -> Result<PresentationIntent, String> {
    let effect_age_ms = match (reaction, reaction_age_ms) {
        (None, Some(_)) => {
            return Err("--reaction-age-ms requires --reaction".to_owned());
        }
        (Some(_), age) => age.unwrap_or(0),
        (None, None) => 0,
    };
    let now_ms = time_ms.max(effect_age_ms);
    let now = Duration::from_millis(now_ms);
    let phase_age = Duration::from_millis(time_ms);
    let effect = reaction.and_then(|reaction| {
        let kind = reaction.effect_kind();
        let elapsed = Duration::from_millis(effect_age_ms);
        (elapsed < kind.duration()).then(|| EffectSnapshot {
            kind,
            started: now.saturating_sub(elapsed),
            elapsed,
            duration: kind.duration(),
        })
    });
    Ok(PresentationIntent {
        now,
        phase: phase.phase(),
        phase_age,
        effect,
        pose: None,
        visible: true,
        pointer: None,
        viewport: PresentationViewport {
            width: f64::from(width),
            height: f64::from(height),
            backing_scale: 1.0,
            epoch: 1,
        },
        frozen: true,
    })
}

fn normalize_preview_png(
    bytes: Vec<u8>,
    dimensions: (u32, u32),
    overlay: Option<(&PreparedCharacter, PresentationIntent)>,
) -> Result<Vec<u8>, String> {
    if bytes.is_empty() || bytes.len() > MAX_PREVIEW_BYTES {
        return Err("native preview PNG is empty or exceeds the output budget".to_owned());
    }
    let limits = png::Limits {
        bytes: MAX_PREVIEW_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes.as_slice()), limits);
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("native preview PNG is invalid: {error}"))?;
    let (width, height) = dimensions;
    if reader.info().width != width || reader.info().height != height {
        return Err(format!(
            "native preview dimensions do not match the source ({}, {})",
            reader.info().width,
            reader.info().height
        ));
    }
    if overlay.is_none()
        && reader.info().color_type == png::ColorType::Rgba
        && reader.info().bit_depth == png::BitDepth::Eight
    {
        drop(reader);
        return Ok(bytes);
    }
    let (color_type, bit_depth) = reader.output_color_type();
    if bit_depth != png::BitDepth::Eight {
        return Err("native preview did not decode to 8-bit color".to_owned());
    }
    let channels = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err("native preview retained an indexed color type".to_owned());
        }
    };
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("native preview PNG could not be decoded: {error}"))?;
    let source = decoded
        .get(..info.buffer_size())
        .ok_or_else(|| "native preview pixel storage is truncated".to_owned())?;
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| "native preview dimensions overflow".to_owned())?;
    let expected = pixel_count
        .checked_mul(channels)
        .ok_or_else(|| "native preview pixel storage overflows".to_owned())?;
    if source.len() != expected {
        return Err("native preview pixel storage does not match its dimensions".to_owned());
    }
    let mut rgba = if color_type == png::ColorType::Rgba {
        decoded.truncate(expected);
        decoded
    } else {
        let mut rgba = Vec::with_capacity(
            pixel_count
                .checked_mul(4)
                .ok_or_else(|| "native preview RGBA storage overflows".to_owned())?,
        );
        for pixel in source.chunks_exact(channels) {
            match color_type {
                png::ColorType::Grayscale => {
                    rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255])
                }
                png::ColorType::GrayscaleAlpha => {
                    rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]])
                }
                png::ColorType::Rgb => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
                png::ColorType::Rgba | png::ColorType::Indexed => unreachable!(),
            }
        }
        rgba
    };
    if let Some((character, intent)) = overlay {
        character.paint_png_hit_overlay(intent, &mut rgba)?;
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("cannot encode preview PNG: {error}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| format!("cannot encode preview PNG: {error}"))?;
    }
    if encoded.len() > MAX_PREVIEW_BYTES {
        return Err("encoded preview PNG exceeds the output budget".to_owned());
    }
    Ok(encoded)
}

fn prepare_output_path(path: &Path) -> Result<PathBuf, String> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.contains(&0) {
        return Err("output path must be non-empty and must not contain NUL bytes".to_owned());
    }
    if bytes.len() > MAX_PATH_BYTES {
        return Err(format!(
            "output path must be at most {MAX_PATH_BYTES} bytes"
        ));
    }
    if !path.is_absolute() {
        return Err("output path must be absolute".to_owned());
    }
    let Some(file_name) = path.file_name() else {
        return Err("output path must name a new file".to_owned());
    };
    if file_name == std::ffi::OsStr::new(".") || file_name == std::ffi::OsStr::new("..") {
        return Err("output path must name a new file".to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent directory".to_owned())?;
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("cannot resolve output parent {}: {error}", parent.display()))?;
    let metadata = fs::symlink_metadata(&parent)
        .map_err(|error| format!("output parent {} is unavailable: {error}", parent.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "output parent {} is not a directory",
            parent.display()
        ));
    }
    let output = parent.join(file_name);
    match fs::symlink_metadata(&output) {
        Ok(_) => Err(format!(
            "output destination {} already exists",
            output.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(output),
        Err(error) => Err(format!(
            "cannot inspect output destination {}: {error}",
            output.display()
        )),
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create output {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write output {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync output {}: {error}", path.display()))?;
    Ok(())
}

fn emit_json<T: Serialize>(value: &T) -> Result<(), String> {
    let text = serde_json::to_string(value)
        .map_err(|error| format!("cannot encode pack response: {error}"))?;
    println!("{text}");
    Ok(())
}
