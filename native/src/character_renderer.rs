use crate::alpha::AlphaMask;
use crate::animation::{ClipSet, FrameId, Playback};
use crate::assets::{AssetPack, CharacterMetadata, FrameAsset, FrameRegions, ValidatedCharacter};
use crate::behavior::{EffectKind, PresentationIntent};
use crate::bubble::{screen_rect_for_image_bounds, Rect};
use crate::character_types::RendererToken;
use crate::interaction::{legacy_region, Point, Region, RegionPolicy};
use crate::rig_renderer::{PreparedRig, RigIntent, RigPreparation, RigRegion};
use objc2::rc::{autoreleasepool, Retained};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{NSData, NSDate, NSDictionary, NSRunLoop, NSSize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_MASK_STORAGE_BYTES: u64 = 16 * 1024 * 1024;

mod preparation_limits {
    include!(env!("HERDR_RIG_LIMITS_RS"));
}

pub const SURFACE_PREPARATION_TIMEOUT: Duration =
    Duration::from_secs(preparation_limits::DECODE_SECONDS);
// A v5 catalog prepares the ten distinct semantic bindings, not unused models.
pub const MAX_PREPARATION_TIMEOUT: Duration = rig_preparation_timeout(10);

const fn rig_preparation_timeout(model_count: usize) -> Duration {
    let batches = model_count.div_ceil(preparation_limits::CATALOG_DECODE_WORKERS);
    // Account for queued decode waves, then initial renderer/geometry setup.
    Duration::from_secs(preparation_limits::DECODE_SECONDS * (batches as u64 + 1))
}

pub fn preparation_timeout(assets: &ValidatedCharacter) -> Duration {
    match assets {
        ValidatedCharacter::Png(_) => SURFACE_PREPARATION_TIMEOUT,
        ValidatedCharacter::Rig(rig) => {
            rig_preparation_timeout(rig.bindings.as_ref().map_or(1, |bindings| bindings.len()))
        }
    }
}

/// Main-thread-owned native image and hit-test resources for one frame.
///
/// `NSImage` is retained together with the alpha mask produced while loading
/// the source frame.  A frame's image and mask therefore cannot be exchanged
/// independently by the presentation layer.
pub struct NativeFrame {
    pub image: Retained<NSImage>,
    pub mask: AlphaMask,
    anchor_bounds: Option<(f64, f64, f64, f64)>,
    bitmap: Retained<NSBitmapImageRep>,
}

/// A completely decoded character pack.  This value is only constructed and
/// dropped on the AppKit main thread.
pub enum PreparedCharacter {
    Png(PreparedPng),
    Rig(PreparedRig),
}

pub struct CharacterHit {
    pub opaque: bool,
    pub region: RegionPolicy,
}

impl PreparedCharacter {
    pub fn token(&self) -> &RendererToken {
        match self {
            Self::Png(png) => &png.token,
            Self::Rig(rig) => rig.token(),
        }
    }

    pub fn input_epoch(&self) -> Option<u64> {
        match self {
            Self::Png(_) => Some(1),
            Self::Rig(rig) => rig.input_epoch(),
        }
    }

    pub fn input_ready(&self) -> bool {
        match self {
            Self::Png(_) => true,
            Self::Rig(rig) => rig.input_ready(),
        }
    }
    /// Semantic rig pose/model generation, excluding continuously advancing
    /// animation clocks. Pair with input_epoch and input_ready in UI caches.
    pub fn speech_anchor_epoch(&self) -> Option<u64> {
        match self {
            Self::Png(_) => Some(1),
            Self::Rig(rig) => rig.speech_anchor_epoch(),
        }
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        match self {
            Self::Png(png) => (png.width, png.height),
            Self::Rig(rig) => rig.canvas_size(),
        }
    }
    /// Stable normalized top-left source coverage across every prepared frame.
    pub fn display_bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Self::Png(png) => png.display_bounds,
            Self::Rig(rig) => rig.display_bounds(),
        }
    }

    /// Visible head bounds in AppKit screen coordinates (bottom-left origin).
    /// Call when the presented frame or viewport changes, not per animation tick.
    /// A PNG head is clipped to its alpha mask; when absent, visible artwork
    /// provides a padding-free fallback. An unavailable input snapshot has no anchor.
    pub fn speech_anchor(
        &self,
        frame: Option<FrameId>,
        rect: (f64, f64, f64, f64),
    ) -> Option<Rect> {
        let (x, y, width, height) = rect;
        if ![x, y, width, height].iter().all(|value| value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            return None;
        }
        match self {
            Self::Png(png) => {
                let bounds = png.frames.get(frame?.index())?.anchor_bounds?;
                let scale = (width / f64::from(png.width)).min(height / f64::from(png.height));
                let fitted_width = f64::from(png.width) * scale;
                let fitted_height = f64::from(png.height) * scale;
                let fitted = Rect {
                    x: x + (width - fitted_width) * 0.5,
                    y: y + (height - fitted_height) * 0.5,
                    width: fitted_width,
                    height: fitted_height,
                };
                screen_rect_for_image_bounds(fitted, bounds)
            }
            Self::Rig(rig) => {
                let (source_width, source_height) = rig.canvas_size();
                let (x0, y0, x1, y1) = rig.speech_anchor()?;
                let scale =
                    (width / f64::from(source_width)).min(height / f64::from(source_height));
                let fitted_width = f64::from(source_width) * scale;
                let fitted_height = f64::from(source_height) * scale;
                let fitted = Rect {
                    x: x + (width - fitted_width) * 0.5,
                    y: y + (height - fitted_height) * 0.5,
                    width: fitted_width,
                    height: fitted_height,
                };
                screen_rect_for_image_bounds(
                    fitted,
                    (
                        x0 / f64::from(source_width),
                        y0 / f64::from(source_height),
                        x1 / f64::from(source_width),
                        y1 / f64::from(source_height),
                    ),
                )
            }
        }
    }

    pub fn preview_png(
        &mut self,
        intent: PresentationIntent,
        hit_overlay: bool,
    ) -> Result<Vec<u8>, String> {
        match self {
            Self::Rig(rig) => rig.preview_png(rig_intent(intent), hit_overlay),
            Self::Png(png) => {
                let mut playback = Playback::default();
                playback.reset_pack(intent.now.saturating_sub(intent.phase_age), intent.phase);
                let sample = playback
                    .sample(intent.now, intent.phase, true, intent.effect, &png.clips)
                    .ok_or_else(|| "preview has no visible PNG frame".to_owned())?;
                let frame = png
                    .frames
                    .get(sample.frame.index())
                    .ok_or_else(|| "preview frame is unavailable".to_owned())?;
                // Empty properties are valid for PNG; the bitmap is immutable
                // and owned by this main-thread prepared renderer.
                let data = unsafe {
                    frame.bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }
                .ok_or_else(|| "AppKit could not encode the prepared PNG frame".to_owned())?;
                if data.length() > 8 * 1024 * 1024 {
                    return Err("native PNG preview exceeds output budget".to_owned());
                }
                Ok(data.to_vec())
            }
        }
    }

    /// Paints the bounded authoring hit overlay over a normalized RGBA preview.
    ///
    /// Samples are taken from the selected frame's rendered alpha on a 16px
    /// source grid. New PNG packs use their validated per-frame regions;
    /// legacy packs retain the fixed interaction geometry used by input.
    pub fn paint_png_hit_overlay(
        &self,
        intent: PresentationIntent,
        rgba: &mut [u8],
    ) -> Result<(), String> {
        let Self::Png(_) = self else {
            return Err("PNG hit overlay requested for a non-PNG renderer".to_owned());
        };
        paint_png_hit_overlay(self, intent, rgba)
    }
    pub fn metadata(&self) -> Option<&CharacterMetadata> {
        match self {
            Self::Png(png) => png.metadata.as_ref(),
            Self::Rig(rig) => Some(rig.metadata()),
        }
    }

    pub fn hit(
        &self,
        frame: Option<FrameId>,
        x: f64,
        y: f64,
        rect: (f64, f64, f64, f64),
    ) -> Option<CharacterHit> {
        match self {
            Self::Png(png) => {
                let frame = frame?.index();
                let sample = png.frames.get(frame)?.mask.sample(x, y, rect);
                let opaque = sample.is_some_and(|sample| sample.0);
                let region = match png.regions.as_ref() {
                    None => RegionPolicy::Legacy,
                    Some(regions) => {
                        let regions = regions.get(frame)?;
                        let region = match sample.filter(|sample| sample.0) {
                            Some((_, x, y))
                                if x >= regions.head.x0
                                    && x < regions.head.x1
                                    && y >= regions.head.y0
                                    && y < regions.head.y1 =>
                            {
                                Region::Head
                            }
                            Some((_, x, y))
                                if x >= regions.body.x0
                                    && x < regions.body.x1
                                    && y >= regions.body.y0
                                    && y < regions.body.y1 =>
                            {
                                Region::Body
                            }
                            _ => Region::Other,
                        };
                        RegionPolicy::Captured(region)
                    }
                };
                Some(CharacterHit { opaque, region })
            }

            Self::Rig(rig) => {
                let (left, bottom, width, height) = rect;
                if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                    return None;
                }
                let hit = rig.hit((x - left) / width, 1.0 - (y - bottom) / height)?;
                let region = match hit.region {
                    Some(RigRegion::Head) => Region::Head,
                    Some(RigRegion::Body) => Region::Body,
                    None => Region::Other,
                };
                Some(CharacterHit {
                    opaque: hit.alpha >= 8,
                    region: RegionPolicy::Captured(region),
                })
            }
        }
    }
}

const HIT_OVERLAY_GRID: usize = 16;
const HIT_OVERLAY_MARK_RADIUS: usize = 1;
const HIT_OVERLAY_BLEND: u32 = 160;

fn paint_png_hit_overlay(
    character: &PreparedCharacter,
    intent: PresentationIntent,
    rgba: &mut [u8],
) -> Result<(), String> {
    let PreparedCharacter::Png(png) = character else {
        return Err("PNG hit overlay requested for a non-PNG renderer".to_owned());
    };
    let width = usize::try_from(png.width)
        .map_err(|_| "PNG hit overlay width overflows usize".to_owned())?;
    let height = usize::try_from(png.height)
        .map_err(|_| "PNG hit overlay height overflows usize".to_owned())?;
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "PNG hit overlay pixel storage overflows".to_owned())?;
    if rgba.len() != expected {
        return Err("PNG hit overlay pixel storage does not match its dimensions".to_owned());
    }

    let mut playback = Playback::default();
    playback.reset_pack(intent.now.saturating_sub(intent.phase_age), intent.phase);
    let sample = playback
        .sample(intent.now, intent.phase, true, intent.effect, &png.clips)
        .ok_or_else(|| "preview has no visible PNG frame".to_owned())?;
    let source_rect = (0.0, 0.0, f64::from(png.width), f64::from(png.height));

    // A 16px sampled grid bounds work to 4096 hit samples at the 1024x1024
    // authoring canvas limit. Each sample paints only opaque rendered pixels.
    for top in (0..height).step_by(HIT_OVERLAY_GRID) {
        for left in (0..width).step_by(HIT_OVERLAY_GRID) {
            let index = (top * width + left) * 4;
            let output_alpha = rgba[index + 3];
            let source_x = left as f64 + 0.5;
            let source_y = top as f64 + 0.5;
            let Some(hit) = character.hit(
                Some(sample.frame),
                source_x,
                f64::from(png.height) - source_y,
                source_rect,
            ) else {
                continue;
            };
            if output_alpha < 8 || !hit.opaque {
                continue;
            }

            let region = match hit.region {
                RegionPolicy::Captured(region) => region,
                RegionPolicy::Legacy => legacy_region(Point {
                    x: source_x,
                    y: source_y,
                }),
            };
            let tint = match region {
                Region::Head => [255, 64, 64],
                Region::Body => [64, 224, 96],
                Region::Other => [64, 144, 255],
            };
            let x0 = left.saturating_sub(HIT_OVERLAY_MARK_RADIUS);
            let x1 = left
                .saturating_add(HIT_OVERLAY_MARK_RADIUS)
                .min(width.saturating_sub(1));
            let y0 = top.saturating_sub(HIT_OVERLAY_MARK_RADIUS);
            let y1 = top
                .saturating_add(HIT_OVERLAY_MARK_RADIUS)
                .min(height.saturating_sub(1));
            for mark_y in y0..=y1 {
                for mark_x in x0..=x1 {
                    let mark_index = (mark_y * width + mark_x) * 4;
                    tint_straight_rgba(&mut rgba[mark_index..mark_index + 4], tint);
                }
            }
        }
    }
    Ok(())
}

fn tint_straight_rgba(pixel: &mut [u8], tint: [u8; 3]) {
    if pixel[3] < 8 {
        return;
    }
    for (channel, tint_channel) in pixel[..3].iter_mut().zip(tint) {
        let original = u32::from(*channel);
        *channel = ((original * (255 - HIT_OVERLAY_BLEND)
            + u32::from(tint_channel) * HIT_OVERLAY_BLEND
            + 127)
            / 255) as u8;
    }
}
pub struct PreparedPng {
    pub frames: Box<[NativeFrame]>,
    pub clips: ClipSet,
    pub width: u32,
    pub height: u32,
    display_bounds: (f64, f64, f64, f64),
    pub regions: Option<Box<[FrameRegions]>>,
    pub metadata: Option<CharacterMetadata>,
    token: RendererToken,
}

pub fn rig_intent(intent: PresentationIntent) -> RigIntent {
    let (pointer_x, pointer_y) = intent.pointer.unwrap_or((f64::NAN, f64::NAN));
    RigIntent {
        now_seconds: intent.now.as_secs_f64(),
        phase_age_seconds: intent.phase_age.as_secs_f64(),
        pose_age_seconds: intent.pose.map_or(0.0, |pose| pose.age.as_secs_f64()),
        effect_age_seconds: intent
            .effect
            .map_or(0.0, |effect| effect.elapsed.as_secs_f64()),
        effect_duration_seconds: intent
            .effect
            .map_or(0.0, |effect| effect.duration.as_secs_f64()),
        pointer_x,
        pointer_y,
        phase: crate::animation::phase_index(intent.phase) as i32,
        pose_kind: intent.pose.map_or(-1, |pose| pose.kind as i32),
        effect: intent.effect.map_or(-1, |effect| match effect.kind {
            EffectKind::HeadTap => 0,
            EffectKind::BodyTap => 1,
            EffectKind::Pet => 2,
            EffectKind::CompletionObserved => 3,
        }),
        visible: u32::from(intent.visible),
        frozen: u32::from(intent.frozen),
    }
}

/// Owns candidate resources exclusively on the AppKit main thread.
pub struct PrepareBuilder {
    backend: PreparingBackend,
}

enum PreparingBackend {
    Png(Box<PngPreparation>),
    Rig(RigPreparation),
}

impl PrepareBuilder {
    pub fn new(
        assets: ValidatedCharacter,
        token: RendererToken,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        if assets.content_digest() != token.content_digest {
            return Err("renderer token does not match validated content".to_owned());
        }
        let backend = match assets {
            ValidatedCharacter::Png(assets) => {
                PreparingBackend::Png(Box::new(PngPreparation::new(assets, token, mtm)))
            }
            ValidatedCharacter::Rig(assets) => {
                PreparingBackend::Rig(RigPreparation::new(assets, token, mtm)?)
            }
        };
        Ok(Self { backend })
    }

    /// Returns the completed candidate, or None while preparation is pending.
    pub fn step(&mut self, cancel: &AtomicBool) -> Result<Option<PreparedCharacter>, String> {
        match &mut self.backend {
            PreparingBackend::Png(png) => png.step(cancel),
            PreparingBackend::Rig(rig) => Ok(rig.poll(cancel)?.map(PreparedCharacter::Rig)),
        }
    }
}

struct PngPreparation {
    mtm: MainThreadMarker,
    frames: std::vec::IntoIter<FrameAsset>,
    clips: Option<ClipSet>,
    native_frames: Vec<NativeFrame>,
    decoded_bytes: u64,
    decoded_budget: u64,
    mask_bytes: u64,
    finished: bool,
    width: u32,
    height: u32,
    regions: Option<Box<[FrameRegions]>>,
    metadata: Option<CharacterMetadata>,
    token: Option<RendererToken>,
}

impl PngPreparation {
    fn new(assets: AssetPack, token: RendererToken, mtm: MainThreadMarker) -> Self {
        let parts = assets.into_parts();
        Self {
            mtm,
            frames: Vec::from(parts.frames).into_iter(),
            clips: Some(parts.clips),
            native_frames: Vec::new(),
            decoded_bytes: 0,
            decoded_budget: parts.native_decoded_budget,
            mask_bytes: 0,
            finished: false,
            width: parts.width,
            height: parts.height,
            regions: parts.regions,
            metadata: parts.metadata,
            token: Some(token),
        }
    }

    /// Performs one bounded preparation slice.
    ///
    /// The cancellation token is checked before consuming a frame, after the
    /// decoder has returned, and before publishing a completed candidate.
    /// Thus every failure path drops native resources while this builder is
    /// still owned by the main-thread caller.
    pub fn step(&mut self, cancel: &AtomicBool) -> Result<Option<PreparedCharacter>, String> {
        // Keep the marker as part of the builder's ownership contract.  The
        // marker is !Send, so a builder containing native objects cannot cross
        // the worker/main-thread boundary.
        let _ = self.mtm;
        if self.finished {
            return Err("character preparation has already completed".to_owned());
        }
        if cancel.load(Ordering::Acquire) {
            return self.cancelled();
        }

        let Some(frame) = self.frames.next() else {
            if self.native_frames.is_empty() {
                return self.fail("character pack contains no frames".to_owned());
            }
            if cancel.load(Ordering::Acquire) {
                return self.cancelled();
            }
            return self.ready();
        };

        let (native_frame, bitmap_bytes, frame_name) =
            match decode_frame(frame, self.width, self.height, cancel) {
                Ok(decoded) => decoded,
                Err(error) => {
                    self.abort();
                    return Err(error);
                }
            };
        if cancel.load(Ordering::Acquire) {
            drop(native_frame);
            return self.cancelled();
        }

        let next_decoded = match self.decoded_bytes.checked_add(bitmap_bytes) {
            Some(next) if next <= self.decoded_budget => next,
            Some(_) => {
                drop(native_frame);
                return self.fail(format!(
                    "frame {frame_name} exceeds the native decoded storage budget"
                ));
            }
            None => {
                drop(native_frame);
                return self.fail("native decoded storage accounting overflow".to_owned());
            }
        };
        let mask_bytes = match mask_storage_bytes(self.width, self.height) {
            Ok(bytes) => bytes,
            Err(error) => {
                drop(native_frame);
                return self.fail(error);
            }
        };
        let next_mask = match self.mask_bytes.checked_add(mask_bytes) {
            Some(next) if next <= MAX_MASK_STORAGE_BYTES => next,
            Some(_) => {
                drop(native_frame);
                return self.fail("character alpha-mask storage budget exceeded".to_owned());
            }
            None => {
                drop(native_frame);
                return self.fail("alpha-mask storage accounting overflow".to_owned());
            }
        };

        let mut native_frame = native_frame;
        let head = self
            .regions
            .as_ref()
            .and_then(|regions| regions.get(self.native_frames.len()))
            .map(|r| (r.head.x0, r.head.y0, r.head.x1, r.head.y1));
        native_frame.anchor_bounds = head
            .and_then(|region| native_frame.mask.visible_bounds(Some(region)))
            .or_else(|| native_frame.mask.visible_bounds(None));
        self.decoded_bytes = next_decoded;
        self.mask_bytes = next_mask;
        self.native_frames.push(native_frame);
        if self.frames.len() == 0 {
            if cancel.load(Ordering::Acquire) {
                return self.cancelled();
            }
            return self.ready();
        }
        Ok(None)
    }

    fn ready(&mut self) -> Result<Option<PreparedCharacter>, String> {
        let Some(clips) = self.clips.take() else {
            return self.fail("character clips were already consumed".to_owned());
        };
        self.finished = true;
        let frames = std::mem::take(&mut self.native_frames).into_boxed_slice();
        let display_bounds = union_display_bounds(frames.iter().map(|frame| &frame.mask));
        let token = self
            .token
            .take()
            .ok_or_else(|| "renderer token was already consumed".to_owned())?;
        Ok(Some(PreparedCharacter::Png(PreparedPng {
            frames,
            clips,
            width: self.width,
            height: self.height,
            display_bounds,
            regions: self.regions.take(),
            metadata: self.metadata.take(),
            token,
        })))
    }

    fn cancelled<T>(&mut self) -> Result<T, String> {
        self.abort();
        Err("character operation canceled".to_owned())
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.abort();
        Err(error)
    }

    fn abort(&mut self) {
        self.finished = true;
        self.native_frames.clear();
        self.clips.take();
    }
}

/// Take the union once at preparation, including frames used only by reactions.
fn union_display_bounds<'a>(masks: impl Iterator<Item = &'a AlphaMask>) -> (f64, f64, f64, f64) {
    let mut union: Option<(f64, f64, f64, f64)> = None;
    for (x0, y0, x1, y1) in masks.filter_map(AlphaMask::display_bounds) {
        union = Some(match union {
            Some((left, top, right, bottom)) => (
                left.min(x0),
                top.min(y0),
                right.max(x1),
                bottom.max(y1),
            ),
            None => (x0, y0, x1, y1),
        });
    }
    union.unwrap_or((0.0, 0.0, 1.0, 1.0))
}

/// Startup/offline preparation pumps the main run loop while native work runs.
pub fn prepare(
    assets: ValidatedCharacter,
    token: RendererToken,
    mtm: MainThreadMarker,
) -> Result<PreparedCharacter, String> {
    let cancel = AtomicBool::new(false);
    let timeout = preparation_timeout(&assets);
    let mut builder = PrepareBuilder::new(assets, token, mtm)?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            cancel.store(true, Ordering::Release);
            return Err("character preparation timed out".to_owned());
        }
        match builder.step(&cancel)? {
            None => {
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.005));
            }
            Some(prepared) => return Ok(prepared),
        }
    }
}

fn decode_frame(
    frame: FrameAsset,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<(NativeFrame, u64, String), String> {
    let frame_name = frame.name;
    autoreleasepool(|_| {
        if cancel.load(Ordering::Acquire) {
            return Err("character operation canceled".to_owned());
        }
        let data = NSData::with_bytes(&frame.png);
        let rep = NSBitmapImageRep::imageRepWithData(&data)
            .ok_or_else(|| format!("frame {frame_name} could not be decoded by AppKit"))?;

        // This is an explicit post-decode cancellation point.  It prevents
        // canceled work from constructing another retained native candidate.
        if cancel.load(Ordering::Acquire) {
            return Err("character operation canceled".to_owned());
        }
        let bitmap_bytes = bitmap_storage_bytes(&rep, &frame_name, width, height)?;
        // This checks that decoded bitmap storage is available.  It does not
        // claim that every AppKit image/cache allocation has materialized.
        if rep.bitmapData().is_null() {
            return Err(format!("frame {frame_name} has no decoded bitmap data"));
        }

        let image = NSImage::initWithSize(
            NSImage::alloc(),
            NSSize::new(f64::from(width), f64::from(height)),
        );
        image.addRepresentation(&rep);
        if !image.isValid() {
            return Err(format!(
                "frame {frame_name} produced an invalid AppKit image"
            ));
        }
        Ok((
            NativeFrame {
                image,
                mask: frame.alpha,
                anchor_bounds: None,
                bitmap: rep,
            },
            bitmap_bytes,
            frame_name,
        ))
    })
}

fn bitmap_storage_bytes(
    rep: &NSBitmapImageRep,
    frame_name: &str,
    expected_width: u32,
    expected_height: u32,
) -> Result<u64, String> {
    let width = usize::try_from(rep.pixelsWide())
        .map_err(|_| format!("frame {frame_name} has invalid bitmap width"))?;
    let height = usize::try_from(rep.pixelsHigh())
        .map_err(|_| format!("frame {frame_name} has invalid bitmap height"))?;
    if width != expected_width as usize || height != expected_height as usize {
        return Err(format!("frame {frame_name} has invalid AppKit dimensions"));
    }

    let bits_per_sample = usize::try_from(rep.bitsPerSample())
        .map_err(|_| format!("frame {frame_name} has invalid sample depth"))?;
    let bytes_per_sample = match bits_per_sample {
        8 => 1usize,
        16 => 2usize,
        _ => return Err(format!("frame {frame_name} has unsupported sample depth")),
    };
    let samples_per_pixel = usize::try_from(rep.samplesPerPixel())
        .map_err(|_| format!("frame {frame_name} has invalid sample count"))?;
    if samples_per_pixel == 0 || samples_per_pixel > 4 {
        return Err(format!("frame {frame_name} has unsupported sample layout"));
    }
    let planar = rep.isPlanar();
    let number_of_planes = usize::try_from(rep.numberOfPlanes())
        .map_err(|_| format!("frame {frame_name} has invalid plane count"))?;
    if number_of_planes == 0
        || (!planar && number_of_planes != 1)
        || (planar && number_of_planes != samples_per_pixel)
    {
        return Err(format!("frame {frame_name} has unsupported plane layout"));
    }
    let minimum_row_bytes = width
        .checked_mul(bytes_per_sample)
        .and_then(|bytes| {
            if planar {
                Some(bytes)
            } else {
                bytes.checked_mul(samples_per_pixel)
            }
        })
        .ok_or_else(|| format!("frame {frame_name} bitmap stride overflows"))?;
    let bytes_per_row = usize::try_from(rep.bytesPerRow())
        .map_err(|_| format!("frame {frame_name} has invalid bitmap stride"))?;
    if bytes_per_row < minimum_row_bytes {
        return Err(format!("frame {frame_name} bitmap stride is too small"));
    }
    let minimum_plane_bytes = bytes_per_row
        .checked_mul(height)
        .ok_or_else(|| format!("frame {frame_name} bitmap plane size overflows"))?;
    let bytes_per_plane = usize::try_from(rep.bytesPerPlane())
        .map_err(|_| format!("frame {frame_name} has invalid bitmap plane size"))?;
    if bytes_per_plane < minimum_plane_bytes {
        return Err(format!("frame {frame_name} bitmap plane is too small"));
    }
    let storage = bytes_per_plane
        .checked_mul(number_of_planes)
        .ok_or_else(|| format!("frame {frame_name} bitmap storage overflows"))?;
    u64::try_from(storage).map_err(|_| format!("frame {frame_name} bitmap storage is too large"))
}

fn mask_storage_bytes(width: u32, height: u32) -> Result<u64, String> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "alpha-mask storage accounting overflow".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alpha::rgba_alpha_index;

    #[test]
    fn ten_pose_catalog_preserves_time_for_a_queued_decode_wave() {
        let decode = Duration::from_secs(preparation_limits::DECODE_SECONDS);
        let elapsed = decode * 2;
        assert!(rig_preparation_timeout(10).saturating_sub(elapsed) >= decode);
    }

    #[test]
    fn display_union_includes_frames_only_used_by_other_phases_and_reactions() {
        let mut resting = [0_u8; 4 * 4 * 4];
        resting[rgba_alpha_index(4, 1, 1)] = 255;
        let mut reaction = [0_u8; 4 * 4 * 4];
        reaction[rgba_alpha_index(4, 0, 3)] = 1;
        reaction[rgba_alpha_index(4, 3, 0)] = 7;
        let masks = [
            AlphaMask::from_rgba(4, 4, &resting).unwrap(),
            AlphaMask::from_rgba(4, 4, &reaction).unwrap(),
        ];
        assert_eq!(union_display_bounds(masks.iter()), (0.0, 0.0, 1.0, 1.0));
        assert_eq!(masks[1].visible_bounds(None), None);
    }

    #[test]
    fn empty_frames_do_not_expand_visible_union() {
        let mut pixels = [0_u8; 4 * 4 * 4];
        pixels[rgba_alpha_index(4, 2, 1)] = 1;
        let masks = [
            AlphaMask::from_rgba(4, 4, &[0; 4 * 4 * 4]).unwrap(),
            AlphaMask::from_rgba(4, 4, &pixels).unwrap(),
        ];
        assert_eq!(union_display_bounds(masks.iter()), (0.5, 0.25, 0.75, 0.5));
    }
}
