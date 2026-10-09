//! Browser portraits are a separate, bounded rendering lane. Only visible
//! sources are loaded; immutable rig inputs are sealed on the worker, while
//! AppKit/native host ownership stays on main and never touches active slots.
use crate::assets::{AssetPack, ValidatedCharacter};
use crate::character_renderer::{preparation_timeout, PrepareBuilder};
use crate::character_service::PackService;
use crate::character_types::{CharacterRef, OfficialPackIdentity, RendererToken};
use crate::official_characters::OfficialCharacters;
use crate::rig_renderer::RigSnapshot;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapImageRep, NSImage};
use objc2_foundation::{NSData, NSSize};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_VISIBLE: usize = 16;
const MAX_CACHE: usize = 32;
const MAX_IN_FLIGHT: usize = 2;
const MAX_REMOTE_ENCODED: usize = 4 * 1024 * 1024;
const MAX_LOCAL_ENCODED: usize = 8 * 1024 * 1024;
const MAX_REMOTE_PIXELS: u64 = 1024 * 1024;
const MAX_LOCAL_PIXELS: u64 = 1024 * 1024;
const MAX_LOCAL_RGBA_BYTES: usize = 4 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageOrigin {
    Local,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewSource {
    Local {
        reference: CharacterRef,
        generation: u64,
    },
    /// The active --assets directory, never the canonical bundled builtin.
    External {
        path: PathBuf,
        content_digest: String,
    },
    Official {
        identity: OfficialPackIdentity,
        catalog_revision: u64,
        url: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewKey {
    pub source: PreviewSource,
    pub pixels: u16,
}

pub enum PreviewState {
    Loading,
    Ready(Retained<NSImage>),
    Error(String),
}

struct CacheEntry {
    key: PreviewKey,
    state: PreviewState,
    cancel: Arc<AtomicBool>,
    dispatched: bool,
    deferred_until: Option<Instant>,
}

impl CacheEntry {
    /// A busy service gate or store flock defers admission, not image validity.
    /// Back off while a mutation or independent store reader owns admission.
    fn defer_admission(&mut self, now: Instant) {
        self.dispatched = false;
        self.deferred_until = Some(now + Duration::from_millis(250));
    }

    fn resume_admission(&mut self) {
        self.deferred_until = None;
    }

    fn can_dispatch(&self, now: Instant) -> bool {
        !self.dispatched
            && matches!(self.state, PreviewState::Loading)
            && self.deferred_until.is_none_or(|deadline| now >= deadline)
    }
}

enum JobKind {
    Load,
    NormalizeRig(Vec<u8>),
}
struct Job {
    key: PreviewKey,
    kind: JobKind,
    cancel: Arc<AtomicBool>,
}
enum WorkerValue {
    Thumbnail(Vec<u8>),
    Rig {
        snapshot: RigSnapshot,
        token: RendererToken,
        timeout: Duration,
    },
    PendingAdmission,
}
struct Response {
    key: PreviewKey,
    cancel: Arc<AtomicBool>,
    result: Result<WorkerValue, String>,
}
struct PendingRig {
    key: PreviewKey,
    builder: PrepareBuilder,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
}

/// Main-thread-owned cache and the sole owner of the preview worker.
pub struct CharacterPreviews {
    mtm: MainThreadMarker,
    entries: Vec<CacheEntry>,
    visible: Vec<PreviewKey>,
    pending_rig: Option<PendingRig>,
    revision: u64,
    sender: Option<mpsc::SyncSender<Job>>,
    receiver: Option<mpsc::Receiver<Response>>,
    worker: Option<JoinHandle<()>>,
    in_flight: usize,
    apply_pending: bool,
}

impl CharacterPreviews {
    pub fn new(
        packs: Arc<PackService>,
        official: Arc<OfficialCharacters>,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        let (sender, jobs) = mpsc::sync_channel::<Job>(MAX_IN_FLIGHT);
        let (results, receiver) = mpsc::sync_channel::<Response>(MAX_IN_FLIGHT);
        let (cleanup_sender, cleanup_paths) = mpsc::channel::<PathBuf>();
        let worker = thread::Builder::new()
            .name("character-portraits".to_owned())
            .spawn(move || {
                loop {
                    while let Ok(path) = cleanup_paths.try_recv() {
                        let _ = std::fs::remove_dir_all(path);
                    }
                    let job = match jobs.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let result = if job.cancel.load(Ordering::Acquire) {
                        Err("portrait request canceled".to_owned())
                    } else {
                        match job.kind {
                            JobKind::Load => {
                                load(&packs, &official, &job.key, &job.cancel, &cleanup_sender)
                            }
                            JobKind::NormalizeRig(png) => normalize_thumbnail(
                                &png,
                                job.key.pixels,
                                &job.cancel,
                                ImageOrigin::Local,
                            )
                            .map(WorkerValue::Thumbnail),
                        }
                    };
                    if results
                        .send(Response {
                            key: job.key,
                            cancel: job.cancel,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                while let Ok(path) = cleanup_paths.try_recv() {
                    let _ = std::fs::remove_dir_all(path);
                }
            })
            .map_err(|error| format!("cannot start character portrait worker: {error}"))?;
        Ok(Self {
            mtm,
            entries: Vec::new(),
            visible: Vec::new(),
            pending_rig: None,
            revision: 0,
            sender: Some(sender),
            receiver: Some(receiver),
            worker: Some(worker),
            in_flight: 0,
            apply_pending: false,
        })
    }

    /// Caller order determines priority. Repeated requests coalesce by exact
    /// identity, generation, catalog revision, URL, and pixel size.
    pub fn request_visible(&mut self, keys: Vec<PreviewKey>) {
        let mut visible = Vec::with_capacity(keys.len().min(MAX_VISIBLE));
        for key in keys {
            if visible.len() == MAX_VISIBLE {
                break;
            }
            if !visible.contains(&key) {
                visible.push(key);
            }
        }
        for entry in &self.entries {
            if !visible.contains(&entry.key) && entry.dispatched {
                entry.cancel.store(true, Ordering::Release);
            }
        }
        if self
            .pending_rig
            .as_ref()
            .is_some_and(|rig| !visible.contains(&rig.key))
        {
            if let Some(rig) = self.pending_rig.take() {
                rig.cancel.store(true, Ordering::Release);
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == rig.key) {
                    entry.dispatched = false;
                }
            }
        }
        self.visible = visible;
        for key in &self.visible {
            if self.entries.iter().any(|entry| entry.key == *key) {
                continue;
            }
            if self.entries.len() == MAX_CACHE {
                if let Some(index) = self
                    .entries
                    .iter()
                    .position(|entry| !self.visible.contains(&entry.key) && !entry.dispatched)
                {
                    self.entries.remove(index);
                } else {
                    continue;
                }
            }
            self.entries.push(CacheEntry {
                key: key.clone(),
                state: PreviewState::Loading,
                cancel: Arc::new(AtomicBool::new(false)),
                dispatched: false,
                deferred_until: None,
            });
        }
        if !self.apply_pending {
            self.schedule();
        }
    }

    pub fn lookup(&self, key: &PreviewKey) -> Option<&PreviewState> {
        self.entries
            .iter()
            .find(|entry| &entry.key == key)
            .map(|entry| &entry.state)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn has_pending_work(&self) -> bool {
        self.pending_rig.is_some()
            || self.in_flight != 0
            || self.visible.iter().any(|key| {
                self.entries
                    .iter()
                    .any(|entry| entry.key == *key && matches!(entry.state, PreviewState::Loading))
            })
    }

    /// Call from the main-loop timer. Never advance rig preview preparation
    /// while the actual character apply/prepare is pending.
    pub fn tick(&mut self, apply_pending: bool) {
        let apply_finished = self.apply_pending && !apply_pending;
        self.apply_pending = apply_pending;
        if apply_finished {
            for entry in &mut self.entries {
                entry.resume_admission();
            }
        }
        if apply_pending {
            if let Some(rig) = self.pending_rig.take() {
                rig.cancel.store(true, Ordering::Release);
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == rig.key) {
                    entry.dispatched = false;
                }
            }
            for entry in &self.entries {
                if entry.dispatched {
                    entry.cancel.store(true, Ordering::Release);
                }
            }
        }
        while let Some(result) = self.receiver.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.key == result.key)
            else {
                continue;
            };
            if response_index(&self.entries, &self.visible, &result.key, &result.cancel).is_none() {
                if Arc::ptr_eq(&self.entries[index].cancel, &result.cancel) {
                    self.entries[index].dispatched = false;
                }
                continue;
            }
            match result.result {
                Ok(WorkerValue::PendingAdmission) => {
                    self.entries[index].defer_admission(Instant::now());
                }
                Ok(WorkerValue::Thumbnail(bytes)) => {
                    self.publish(
                        index,
                        native_image(&bytes, result.key.pixels, self.mtm).map(PreviewState::Ready),
                    );
                }
                Ok(WorkerValue::Rig {
                    snapshot,
                    token,
                    timeout,
                }) => {
                    if apply_pending || self.pending_rig.is_some() {
                        self.entries[index].dispatched = false;
                    } else {
                        match PrepareBuilder::from_preview_snapshot(snapshot, token, self.mtm) {
                            Ok(builder) => {
                                self.pending_rig = Some(PendingRig {
                                    key: result.key,
                                    builder,
                                    cancel: Arc::clone(&self.entries[index].cancel),
                                    deadline: Instant::now() + timeout,
                                })
                            }
                            Err(error) => self.publish(index, Err(error)),
                        }
                    }
                }
                Err(error) => self.publish(index, Err(error)),
            }
        }
        if !apply_pending {
            if let Some(mut rig) = self.pending_rig.take() {
                let outcome = if Instant::now() >= rig.deadline {
                    Err("portrait rig preparation timed out".to_owned())
                } else {
                    rig.builder
                        .step(&rig.cancel)
                        .and_then(|prepared| match prepared {
                            None => Ok(None),
                            Some(mut prepared) => prepared.preview_idle_png().map(Some),
                        })
                };
                match outcome {
                    Ok(None) => self.pending_rig = Some(rig),
                    Ok(Some(png)) if png.len() <= MAX_LOCAL_ENCODED => {
                        if let Some(index) =
                            self.entries.iter().position(|entry| entry.key == rig.key)
                        {
                            if let Some(sender) = &self.sender {
                                match sender.try_send(Job {
                                    key: rig.key,
                                    kind: JobKind::NormalizeRig(png),
                                    cancel: Arc::clone(&rig.cancel),
                                }) {
                                    Ok(()) => self.in_flight += 1,
                                    Err(mpsc::TrySendError::Full(_)) => self
                                        .publish(index, Err("portrait worker is busy".to_owned())),
                                    Err(mpsc::TrySendError::Disconnected(_)) => self
                                        .publish(index, Err("portrait worker stopped".to_owned())),
                                }
                            }
                        }
                    }
                    Ok(Some(_)) => self.publish_key(
                        &rig.key,
                        Err("portrait rig PNG exceeds byte budget".to_owned()),
                    ),
                    Err(error) => self.publish_key(&rig.key, Err(error)),
                }
            }
        }
        if !apply_pending {
            self.schedule();
        }
    }

    fn publish_key(&mut self, key: &PreviewKey, result: Result<PreviewState, String>) {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == *key) {
            self.publish(index, result);
        }
    }

    fn publish(&mut self, index: usize, result: Result<PreviewState, String>) {
        self.entries[index].state = result.unwrap_or_else(PreviewState::Error);
        self.entries[index].dispatched = false;
        self.entries[index].deferred_until = None;
        self.revision = self.revision.wrapping_add(1);
    }

    fn schedule(&mut self) {
        if self.pending_rig.is_some() {
            return;
        }
        let now = Instant::now();
        for priority in 0..self.visible.len() {
            if self.in_flight >= MAX_IN_FLIGHT {
                break;
            }
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.key == self.visible[priority])
            else {
                continue;
            };
            let entry = &mut self.entries[index];
            if !entry.can_dispatch(now) {
                continue;
            }
            if !(32..=192).contains(&self.visible[priority].pixels) {
                self.publish(
                    index,
                    Err("portrait size must be 32..=192 pixels".to_owned()),
                );
                continue;
            }
            entry.cancel = Arc::new(AtomicBool::new(false));
            let Some(sender) = &self.sender else {
                return;
            };
            match sender.try_send(Job {
                key: self.visible[priority].clone(),
                kind: JobKind::Load,
                cancel: Arc::clone(&entry.cancel),
            }) {
                Ok(()) => {
                    entry.dispatched = true;
                    self.in_flight += 1;
                }
                Err(mpsc::TrySendError::Full(_)) => return,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.publish(index, Err("portrait worker stopped".to_owned()));
                }
            }
        }
    }

    pub fn shutdown(&mut self) {
        for entry in &self.entries {
            entry.cancel.store(true, Ordering::Release);
        }
        self.pending_rig.take();
        self.receiver.take();
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.entries.clear();
        self.visible.clear();
        self.in_flight = 0;
    }
}

impl Drop for CharacterPreviews {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Only the source currently attached to a visible cell may publish. In
/// particular a completed old generation or catalog image cannot fill a
/// newly reused card even when its pack ID is unchanged.
fn response_index(
    entries: &[CacheEntry],
    visible: &[PreviewKey],
    key: &PreviewKey,
    cancel: &Arc<AtomicBool>,
) -> Option<usize> {
    visible
        .contains(key)
        .then(|| {
            entries.iter().position(|entry| {
                &entry.key == key
                    && Arc::ptr_eq(&entry.cancel, cancel)
                    && !cancel.load(Ordering::Acquire)
            })
        })
        .flatten()
}

fn local_reference(key: &PreviewKey) -> CharacterRef {
    match &key.source {
        PreviewSource::Local { reference, .. } => reference.clone(),
        PreviewSource::External { .. } => CharacterRef::builtin(),
        PreviewSource::Official { .. } => unreachable!("official portraits never prepare a rig"),
    }
}

fn loaded_local(
    assets: ValidatedCharacter,
    key: &PreviewKey,
    cancel: &AtomicBool,
    cleanup: &mpsc::Sender<PathBuf>,
) -> Result<WorkerValue, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("portrait request canceled".to_owned());
    }
    let timeout = preparation_timeout(&assets);
    match assets {
        ValidatedCharacter::Png(pack) => {
            let png = pack
                .idle_first_png()
                .ok_or_else(|| "validated character has no idle frame".to_owned())?;
            normalize_thumbnail(png, key.pixels, cancel, ImageOrigin::Local)
                .map(WorkerValue::Thumbnail)
        }
        ValidatedCharacter::Rig(rig) => {
            let token = RendererToken::new(
                "portrait-preview".to_owned(),
                local_reference(key),
                rig.content_digest.clone(),
            )?;
            let snapshot = RigSnapshot::for_preview(rig, cleanup.clone())?;
            if cancel.load(Ordering::Acquire) {
                return Err("portrait request canceled".to_owned());
            }
            Ok(WorkerValue::Rig {
                snapshot,
                token,
                timeout,
            })
        }
    }
}

fn load_external(path: &Path, digest: &str) -> Result<ValidatedCharacter, String> {
    let assets = ValidatedCharacter::Png(AssetPack::load(path)?);
    if assets.content_digest() != digest {
        return Err(
            "external portrait content changed since active renderer preparation".to_owned(),
        );
    }
    Ok(assets)
}

fn load(
    packs: &PackService,
    official: &OfficialCharacters,
    key: &PreviewKey,
    cancel: &AtomicBool,
    cleanup: &mpsc::Sender<PathBuf>,
) -> Result<WorkerValue, String> {
    match &key.source {
        PreviewSource::Local {
            reference,
            generation,
        } => {
            let Some(assets) = packs.load_preview(reference, *generation)? else {
                return Ok(WorkerValue::PendingAdmission);
            };
            loaded_local(assets, key, cancel, cleanup)
        }
        PreviewSource::External {
            path,
            content_digest,
        } => loaded_local(load_external(path, content_digest)?, key, cancel, cleanup),
        PreviewSource::Official {
            identity,
            catalog_revision,
            url,
        } => {
            let snapshot = official
                .cached_snapshot()
                .ok_or_else(|| "official catalog is unavailable".to_owned())?;
            if snapshot.revision != *catalog_revision
                || !snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.identity == *identity && entry.preview_idle_url == *url)
            {
                return Err("official portrait catalog revision or identity changed".to_owned());
            }
            let image = official.fetch_preview(url, cancel)?;
            if official.catalog_revision() != *catalog_revision {
                return Err("official portrait catalog changed during fetch".to_owned());
            }
            normalize_thumbnail(&image, key.pixels, cancel, ImageOrigin::Remote)
                .map(WorkerValue::Thumbnail)
        }
    }
}

fn native_image(
    bytes: &[u8],
    pixels: u16,
    _mtm: MainThreadMarker,
) -> Result<Retained<NSImage>, String> {
    if bytes.len() > MAX_THUMBNAIL_BYTES {
        return Err("portrait thumbnail exceeds byte budget".to_owned());
    }
    let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(bytes))
        .ok_or_else(|| "portrait thumbnail could not be decoded".to_owned())?;
    if rep.pixelsWide() != pixels as isize
        || rep.pixelsHigh() != pixels as isize
        || rep.bitmapData().is_null()
    {
        return Err("portrait thumbnail has incorrect decoded dimensions".to_owned());
    }
    let image = NSImage::initWithSize(
        NSImage::alloc(),
        NSSize::new(f64::from(pixels), f64::from(pixels)),
    );
    image.addRepresentation(&rep);
    if !image.isValid() {
        return Err("portrait thumbnail is invalid".to_owned());
    }
    Ok(image)
}

/// Decode the entire input, including trailer, on the worker before AppKit
/// ever sees it. Official portraits and locally generated v5 rig canvases
/// support 1024x1024 pixels (at most 4 MiB decoded RGBA).
fn normalize_thumbnail(
    bytes: &[u8],
    pixels: u16,
    cancel: &AtomicBool,
    origin: ImageOrigin,
) -> Result<Vec<u8>, String> {
    let encoded_limit = match origin {
        ImageOrigin::Local => MAX_LOCAL_ENCODED,
        ImageOrigin::Remote => MAX_REMOTE_ENCODED,
    };
    if !(32..=192).contains(&pixels)
        || bytes.len() < 20
        || bytes.len() > encoded_limit
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err("portrait is not a bounded PNG".to_owned());
    }
    check_png_trailer(bytes)?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 16 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("invalid portrait PNG: {error}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    let pixels_in_source = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "portrait PNG has invalid dimensions".to_owned())?;
    let max_pixels = match origin {
        ImageOrigin::Local => MAX_LOCAL_PIXELS,
        ImageOrigin::Remote => MAX_REMOTE_PIXELS,
    };
    if width == 0
        || height == 0
        || pixels_in_source > max_pixels
        || (origin == ImageOrigin::Local && (width > 1024 || height > 1024))
        || reader.info().animation_control.is_some()
    {
        return Err("portrait PNG has invalid dimensions or animation".to_owned());
    }
    let rgba_bytes = pixels_in_source
        .checked_mul(4)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| "portrait PNG decoded size overflows".to_owned())?;
    if rgba_bytes > MAX_LOCAL_RGBA_BYTES {
        return Err("portrait PNG exceeds decoded byte budget".to_owned());
    }
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err("portrait PNG cannot be reduced to 8-bit color".to_owned());
    }
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("portrait PNG retained indexed color".to_owned()),
    };
    let expected = usize::try_from(pixels_in_source)
        .ok()
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| "portrait PNG decoded size overflows".to_owned())?;
    if reader.output_buffer_size() != expected {
        return Err("portrait PNG output size mismatch".to_owned());
    }
    let mut decoded = vec![0; expected];
    let frame = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("portrait PNG decode failed: {error}"))?;
    if frame.buffer_size() != expected {
        return Err("portrait PNG frame is incomplete".to_owned());
    }
    reader
        .finish()
        .map_err(|error| format!("portrait PNG is truncated: {error}"))?;
    if cancel.load(Ordering::Acquire) {
        return Err("portrait request canceled".to_owned());
    }
    let side = u32::from(pixels);
    let scaled_width = if width >= height {
        side
    } else {
        (u64::from(width) * u64::from(side) / u64::from(height)).max(1) as u32
    };
    let scaled_height = if height >= width {
        side
    } else {
        (u64::from(height) * u64::from(side) / u64::from(width)).max(1) as u32
    };
    let left = (side - scaled_width) / 2;
    let top = (side - scaled_height) / 2;
    let mut rgba = vec![0; side as usize * side as usize * 4];
    for y in 0..scaled_height {
        if cancel.load(Ordering::Relaxed) {
            return Err("portrait request canceled".to_owned());
        }
        for x in 0..scaled_width {
            let src_x = ((u64::from(x) * u64::from(width)) / u64::from(scaled_width)) as usize;
            let src_y = ((u64::from(y) * u64::from(height)) / u64::from(scaled_height)) as usize;
            let src = (src_y * width as usize + src_x) * channels;
            let dst = ((top + y) as usize * side as usize + (left + x) as usize) * 4;
            let pixel = &decoded[src..src + channels];
            rgba[dst..dst + 4].copy_from_slice(&match color {
                png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
                png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
                png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
                png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
                png::ColorType::Indexed => unreachable!(),
            });
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("cannot encode portrait: {error}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| format!("cannot finish portrait: {error}"))?;
    }
    if encoded.len() > MAX_THUMBNAIL_BYTES {
        return Err("portrait thumbnail exceeds byte budget".to_owned());
    }
    Ok(encoded)
}

/// Explicit IEND + exact end, independent of decoder tolerance for trailing
/// bytes. The PNG decoder checks chunk CRCs and deflate integrity.
fn check_png_trailer(bytes: &[u8]) -> Result<(), String> {
    let mut offset = 8usize;
    loop {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("portrait PNG chunk is truncated")?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or("portrait PNG chunk length overflow")?;
        if end > bytes.len() {
            return Err("portrait PNG chunk is truncated".to_owned());
        }
        if &header[4..8] == b"IEND" {
            if length != 0 || end != bytes.len() {
                return Err("portrait PNG has trailing data".to_owned());
            }
            return Ok(());
        }
        offset = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255])
                .unwrap();
        }
        bytes
    }
    #[test]
    fn decoded_thumbnail_preserves_actual_pixel_identity_and_alpha() {
        let bytes =
            normalize_thumbnail(&image(), 32, &AtomicBool::new(false), ImageOrigin::Local).unwrap();
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        let left = &pixels[16 * 32 * 4..16 * 32 * 4 + 4];
        let right = &pixels[16 * 32 * 4 + 31 * 4..16 * 32 * 4 + 31 * 4 + 4];
        assert_eq!(left, &[255, 0, 0, 255]);
        assert_eq!(right, &[0, 0, 255, 255]);
    }
    #[test]
    fn truncated_and_trailing_png_are_rejected() {
        let bytes = image();
        assert!(normalize_thumbnail(
            &bytes[..bytes.len() - 1],
            32,
            &AtomicBool::new(false),
            ImageOrigin::Remote
        )
        .is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(
            normalize_thumbnail(&trailing, 32, &AtomicBool::new(false), ImageOrigin::Remote)
                .is_err()
        );
    }
    #[test]
    fn compressed_dimension_bomb_is_rejected_before_frame_allocation() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2000, 2000);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![0; 4_000_000]).unwrap();
        }
        assert!(
            normalize_thumbnail(&bytes, 32, &AtomicBool::new(false), ImageOrigin::Local).is_err()
        );
        assert!(
            normalize_thumbnail(&bytes, 32, &AtomicBool::new(false), ImageOrigin::Remote).is_err()
        );
    }
    #[test]
    fn full_v5_canvas_supports_remote_portraits_with_stricter_encoded_budget() {
        let mut rgba = vec![0; MAX_LOCAL_RGBA_BYTES];
        let mut random = 0x91cc_37a6_f158_d42fu64;
        for chunk in rgba.chunks_exact_mut(8) {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            chunk.copy_from_slice(&random.to_le_bytes());
        }
        let first = rgba[..4].to_vec();
        let mut full_canvas = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut full_canvas, 1024, 1024);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&rgba).unwrap();
        }
        assert!(full_canvas.len() > MAX_REMOTE_ENCODED);
        let thumbnail = normalize_thumbnail(
            &full_canvas,
            32,
            &AtomicBool::new(false),
            ImageOrigin::Local,
        )
        .unwrap();
        let mut reader = png::Decoder::new(Cursor::new(thumbnail))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(&pixels[..4], &first);
        assert!(normalize_thumbnail(
            &full_canvas,
            32,
            &AtomicBool::new(false),
            ImageOrigin::Remote,
        )
        .is_err());
        // Published official portraits use 1024x1024 canvases; accept them
        // when they fit the remote encoded and decoded byte budgets.
        let color = [31, 80, 174, 199];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        let mut compact_canvas = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut compact_canvas, 1024, 1024);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&rgba).unwrap();
        }
        assert!(compact_canvas.len() < MAX_REMOTE_ENCODED);
        assert!(normalize_thumbnail(
            &compact_canvas,
            32,
            &AtomicBool::new(false),
            ImageOrigin::Local,
        )
        .is_ok());
        let remote_thumbnail = normalize_thumbnail(
            &compact_canvas,
            32,
            &AtomicBool::new(false),
            ImageOrigin::Remote,
        )
        .unwrap();
        let mut reader = png::Decoder::new(Cursor::new(remote_thumbnail))
            .read_info()
            .unwrap();
        let mut pixels = vec![255; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, color.repeat(32 * 32));
    }

    #[test]
    fn pending_admission_defers_then_resumes_without_error_or_stale_publish() {
        let key = PreviewKey {
            source: PreviewSource::Local {
                reference: CharacterRef {
                    id: "coding-cat".into(),
                    revision: 2,
                },
                generation: 17,
            },
            pixels: 128,
        };
        let old_cancel = Arc::new(AtomicBool::new(false));
        let mut entry = CacheEntry {
            key: key.clone(),
            state: PreviewState::Loading,
            cancel: Arc::clone(&old_cancel),
            dispatched: true,
            deferred_until: None,
        };
        let now = Instant::now();
        entry.defer_admission(now);
        assert!(matches!(entry.state, PreviewState::Loading));
        assert!(!entry.can_dispatch(now + Duration::from_millis(249)));
        assert!(entry.can_dispatch(now + Duration::from_millis(250)));
        entry.defer_admission(now);
        entry.resume_admission(); // An actual apply just cleared.
        assert!(entry.can_dispatch(now));
        old_cancel.store(true, Ordering::Release);
        entry.cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            response_index(&[entry], &[key.clone()], &key, &old_cancel),
            None
        );
    }

    #[test]
    fn old_generation_and_catalog_revision_cannot_fill_a_reused_card() {
        let old = PreviewKey {
            source: PreviewSource::Local {
                reference: CharacterRef {
                    id: "coding-cat".into(),
                    revision: 3,
                },
                generation: 10,
            },
            pixels: 128,
        };
        let mut current = old.clone();
        if let PreviewSource::Local { generation, .. } = &mut current.source {
            *generation = 11;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = CacheEntry {
            key: old.clone(),
            state: PreviewState::Loading,
            cancel: Arc::clone(&cancel),
            dispatched: true,
            deferred_until: None,
        };
        assert_eq!(response_index(&[entry], &[current], &old, &cancel), None);

        let identity = OfficialPackIdentity {
            id: "coding-cat".into(),
            version: "0.0.2".into(),
            release_tag: "v0.0.2".into(),
            sha256: "a".repeat(64),
        };
        let stale = PreviewKey {
            source: PreviewSource::Official {
                identity: identity.clone(),
                catalog_revision: 5,
                url: "https://raw.githubusercontent.com/hanbong5938/herdr-characters/main/old.png"
                    .into(),
            },
            pixels: 128,
        };
        let fresh = PreviewKey {
            source: PreviewSource::Official {
                identity,
                catalog_revision: 6,
                url: "https://raw.githubusercontent.com/hanbong5938/herdr-characters/main/new.png"
                    .into(),
            },
            pixels: 128,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = CacheEntry {
            key: stale.clone(),
            state: PreviewState::Loading,
            cancel: Arc::clone(&cancel),
            dispatched: true,
            deferred_until: None,
        };
        assert_eq!(response_index(&[entry], &[fresh], &stale, &cancel), None);
    }
    #[test]
    fn external_portrait_uses_its_own_verified_content_not_the_builtin_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("manifest.json"),
            br#"{"version":1,"name":"External","width":384,"height":512,"poses":{"idle":"idle.png","running":"running.png","waiting":"waiting.png","unknown":"unknown.png"}}"#
        ).unwrap();
        let paint = |color: [u8; 4]| {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 384, 512);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&color.repeat(384 * 512)).unwrap();
            }
            bytes
        };
        for pose in ["idle", "running", "waiting", "unknown"] {
            std::fs::write(root.join(format!("{pose}.png")), paint([31, 80, 174, 255])).unwrap();
        }
        let active_digest = ValidatedCharacter::Png(AssetPack::load(&root).unwrap())
            .content_digest()
            .to_owned();
        let external = PreviewKey {
            source: PreviewSource::External {
                path: root.to_path_buf(),
                content_digest: active_digest.clone(),
            },
            pixels: 32,
        };
        let (cleanup, _received) = mpsc::channel();
        let preview = loaded_local(
            load_external(&root, &active_digest).unwrap(),
            &external,
            &AtomicBool::new(false),
            &cleanup,
        )
        .unwrap();
        let WorkerValue::Thumbnail(bytes) = preview else {
            panic!("expected external PNG");
        };
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
        let width = reader.info().width as usize;
        let height = reader.info().height as usize;
        let mut pixels = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        let center = (height / 2 * width + width / 2) * 4;
        assert_eq!(&pixels[center..center + 4], &[31, 80, 174, 255]);
        std::fs::write(root.join("idle.png"), paint([175, 13, 32, 255])).unwrap();
        assert!(load_external(&root, &active_digest)
            .unwrap_err()
            .contains("external portrait content changed"));
        let changed = ValidatedCharacter::Png(AssetPack::load(&root).unwrap())
            .content_digest()
            .to_owned();
        assert_ne!(changed, active_digest);
    }
}
