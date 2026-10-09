//! Typed Rust ownership and lifecycle adapter for the Swift/Metal rig host.
//!
//! The Swift bridge is intentionally a narrow C ABI.  Rust owns immutable pack
//! bytes, tokens, metadata, cancellation, and the lifetime of the retained view;
//! Swift owns AppKit/Metal objects and the decoder child process.

use crate::assets::{CharacterMetadata, RigAsset};
use crate::behavior::PresentationViewport;
use crate::character_types::RendererToken;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::NSView;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::raw::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RigIntent {
    pub now_seconds: f64,
    pub phase_age_seconds: f64,
    pub effect_age_seconds: f64,
    pub effect_duration_seconds: f64,
    pub pointer_x: f64,
    pub pointer_y: f64,
    pub phase: i32,
    pub effect: i32,
    pub visible: u32,
    pub frozen: u32,
    /// -1 keeps legacy v4 phase/effect selection.  v5 uses 0..9 semantic
    /// indices in the stable order declared by the pack bindings.
    pub pose_kind: i32,
    /// Age of the selected v5 semantic pose. Legacy scenes ignore this field.
    pub pose_age_seconds: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RigRegion {
    Head,
    Body,
}

#[derive(Clone, Debug)]
pub struct RigHit {
    pub alpha: u8,
    pub region: Option<RigRegion>,
}

#[repr(C)]
struct CModelInput {
    id: *const c_char,
    file_path: *const c_char,
    overrides_path: *const c_char,
    motion_path: *const c_char,
}

#[repr(C)]
struct CAssetInput {
    width: u32,
    height: u32,
    base_path: *const c_char,
    pose_path: *const c_char,
    overrides_path: *const c_char,
    base_pose_id: *const c_char,
    pose_id: *const c_char,
    models: *const CModelInput,
    model_count: u32,
    bindings: *const u8,
    initial_pose_kind: i32,
    initial_motion_bytes: *const u8,
    initial_motion_length: usize,
}

#[repr(C)]
struct CTokenInput {
    operation_id: *const c_char,
    reference_id: *const c_char,
    reference_revision: u64,
    content_digest: *const c_char,
    backend_epoch: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CHit {
    alpha: u8,
    region: u8,
    reserved: u16,
    source_x: f64,
    source_y: f64,
    frame: u64,
    viewport_epoch: u64,
    input_epoch: u64,
}
#[repr(C)]
#[derive(Default)]
struct CAnchor {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

const RIG_ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CRigABIInfoV1 {
    version: u32,
    asset_size: u32,
    model_size: u32,
    token_size: u32,
    intent_size: u32,
    hit_size: u32,
    anchor_size: u32,
    speech_anchor_snapshot_size: u32,
}

fn expected_rig_abi() -> CRigABIInfoV1 {
    CRigABIInfoV1 {
        version: RIG_ABI_VERSION,
        asset_size: std::mem::size_of::<CAssetInput>() as u32,
        model_size: std::mem::size_of::<CModelInput>() as u32,
        token_size: std::mem::size_of::<CTokenInput>() as u32,
        intent_size: std::mem::size_of::<RigIntent>() as u32,
        hit_size: std::mem::size_of::<CHit>() as u32,
        anchor_size: std::mem::size_of::<CAnchor>() as u32,
        speech_anchor_snapshot_size: std::mem::size_of::<CSpeechAnchorSnapshot>() as u32,
    }
}

fn check_rig_abi(info: CRigABIInfoV1) -> Result<(), String> {
    let expected = expected_rig_abi();
    if info.version != expected.version
        || info.asset_size != expected.asset_size
        || info.model_size != expected.model_size
        || info.token_size != expected.token_size
        || info.intent_size != expected.intent_size
        || info.hit_size != expected.hit_size
        || info.anchor_size != expected.anchor_size
        || info.speech_anchor_snapshot_size != expected.speech_anchor_snapshot_size
    {
        return Err(format!(
            "native rig ABI incompatible: expected version {} and sizes [{}, {}, {}, {}, {}, {}, {}], found version {} and sizes [{}, {}, {}, {}, {}, {}, {}]",
            expected.version, expected.asset_size, expected.model_size, expected.token_size,
            expected.intent_size, expected.hit_size, expected.anchor_size,
            expected.speech_anchor_snapshot_size, info.version, info.asset_size, info.model_size,
            info.token_size, info.intent_size, info.hit_size, info.anchor_size,
            info.speech_anchor_snapshot_size
        ));
    }
    Ok(())
}

fn ensure_rig_abi() -> Result<(), String> {
    let mut info = CRigABIInfoV1::default();
    unsafe { herdr_rig_abi_info_v1(&mut info) };
    check_rig_abi(info)
}

#[repr(C)]
#[derive(Default)]
struct CSpeechAnchorSnapshot {
    status: u32,
    reserved: u32,
    backend_epoch: u64,
    input_epoch: u64,
    anchor_epoch: u64,
    viewport_epoch: u64,
    canvas_width: u32,
    canvas_height: u32,
    viewport_width: f64,
    viewport_height: f64,
    backing_scale: f64,
    anchor: CAnchor,
}

#[derive(Clone, Copy)]
pub(crate) enum RigSpeechAnchorStatus {
    Invalid,
    TemporarilyUnavailable,
    ReadyAnchorless,
    ReadyAnchor((f64, f64, f64, f64)),
}

pub(crate) struct RigSpeechAnchorSnapshot {
    pub status: RigSpeechAnchorStatus,
    pub backend_epoch: u64,
    pub input_epoch: Option<u64>,
    pub anchor_epoch: Option<u64>,
    pub viewport: Option<PresentationViewport>,
    pub canvas_width: u32,
    pub canvas_height: u32,
}

#[link(name = "herdr_rig")]
extern "C" {
    fn herdr_rig_abi_info_v1(out_info: *mut CRigABIInfoV1);
    fn herdr_rig_error_free(error: *mut c_char);
    fn herdr_rig_create(
        asset: *const CAssetInput,
        token: *const CTokenInput,
        out_handle: *mut *mut c_void,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_poll(
        handle: *mut c_void,
        cancel_requested: u32,
        out_state: *mut u32,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_cancel(handle: *mut c_void);
    fn herdr_rig_destroy(handle: *mut c_void);
    fn herdr_rig_view(
        handle: *mut c_void,
        out_view: *mut *mut c_void,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_update(
        handle: *mut c_void,
        intent: *const RigIntent,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_set_viewport_epoch(handle: *mut c_void, viewport_epoch: u64);
    fn herdr_rig_prepare_surface(
        handle: *mut c_void,
        intent: *const RigIntent,
        width: f64,
        height: f64,
        backing_scale: f64,
        viewport_epoch: u64,
        out_ready: *mut u32,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_activate(handle: *mut c_void, out_error: *mut *mut c_char) -> i32;
    fn herdr_rig_set_visible(handle: *mut c_void, visible: u32);
    fn herdr_rig_input_epoch(handle: *mut c_void) -> u64;
    fn herdr_rig_input_ready(handle: *mut c_void) -> u32;
    fn herdr_rig_speech_anchor_snapshot(
        handle: *mut c_void,
        out_snapshot: *mut CSpeechAnchorSnapshot,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_hit(
        handle: *mut c_void,
        x: f64,
        y: f64,
        out_hit: *mut CHit,
        out_has_hit: *mut u32,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_display_bounds(handle: *mut c_void, out_bounds: *mut CAnchor) -> u32;
    fn herdr_rig_preview_png(
        handle: *mut c_void,
        intent: *const RigIntent,
        hit_overlay: u32,
        out_bytes: *mut *mut u8,
        out_length: *mut usize,
        out_error: *mut *mut c_char,
    ) -> i32;
    fn herdr_rig_bytes_free(bytes: *mut u8, length: usize);
    fn herdr_rig_last_error(handle: *mut c_void) -> *mut c_char;
}

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TempSnapshot {
    root: PathBuf,
    // Preview snapshots must be removed by the portrait worker, never AppKit.
    cleanup: Option<mpsc::Sender<PathBuf>>,
}

impl TempSnapshot {
    fn materialize(
        asset: &RigAsset,
        cleanup: Option<mpsc::Sender<PathBuf>>,
    ) -> Result<(Self, SnapshotPaths), String> {
        let root = snapshot_root()?;
        let write = |name: &str, bytes: &[u8]| -> Result<CString, String> {
            let path = root.join(name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options
                .open(&path)
                .map_err(|error| format!("cannot create rig snapshot {name}: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("cannot write rig snapshot {name}: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("cannot sync rig snapshot {name}: {error}"))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
                .map_err(|error| format!("cannot seal rig snapshot {name}: {error}"))?;
            CString::new(path.as_os_str().as_bytes())
                .map_err(|_| format!("rig snapshot {name} path contains NUL"))
        };
        let result = (|| {
            let base = write("base.psd", asset.base.as_slice())?;
            let pose = asset
                .pose
                .as_ref()
                .map(|bytes| write("pose.psd", bytes.as_slice()))
                .transpose()?;
            let overrides = write("overrides.json", asset.overrides.as_slice())?;
            let base_pose_id = CString::new(asset.base_pose_id.as_bytes())
                .map_err(|_| "base pose id contains NUL".to_owned())?;
            let pose_id = asset
                .pose_id
                .as_ref()
                .map(|id| {
                    CString::new(id.as_bytes()).map_err(|_| "pose id contains NUL".to_owned())
                })
                .transpose()?;
            let mut models = Vec::new();
            if let Some(catalog) = asset.models.as_deref() {
                models
                    .try_reserve(catalog.len())
                    .map_err(|_| "cannot reserve v5 model snapshots".to_owned())?;
                for (index, model) in catalog.iter().enumerate() {
                    let file_name = format!("model-{index}.psd");
                    let overrides_name = format!("model-{index}-overrides.json");
                    let motion_name = format!("model-{index}-motion.json");
                    models.push(SnapshotModelPaths {
                        id: CString::new(model.id.as_bytes())
                            .map_err(|_| "v5 model ID contains NUL".to_owned())?,
                        file: write(&file_name, model.file.as_slice())?,
                        overrides: write(&overrides_name, model.overrides.as_slice())?,
                        motion: write(&motion_name, model.motion.as_slice())?,
                    });
                }
            }
            Ok::<_, String>(SnapshotPaths {
                base,
                pose,
                overrides,
                base_pose_id,
                pose_id,
                models,
            })
        })();
        match result {
            Ok(paths) => Ok((Self { root, cleanup }, paths)),
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                Err(error)
            }
        }
    }
}

impl Drop for TempSnapshot {
    fn drop(&mut self) {
        // The native handle is already canceled/destroyed before its paths
        // are released. Preview cleanup is queued to the portrait worker.
        if let Some(cleanup) = &self.cleanup {
            let _ = cleanup.send(std::mem::take(&mut self.root));
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

struct SnapshotModelPaths {
    id: CString,
    file: CString,
    overrides: CString,
    motion: CString,
}

struct SnapshotPaths {
    base: CString,
    pose: Option<CString>,
    overrides: CString,
    base_pose_id: CString,
    pose_id: Option<CString>,
    models: Vec<SnapshotModelPaths>,
}

fn snapshot_root() -> Result<PathBuf, String> {
    let temp = std::env::temp_dir();
    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    for _ in 0..64 {
        let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = temp.join(format!("herdr-rig-{pid}-{now}-{sequence}"));
        match fs::create_dir(&path) {
            Ok(()) => {
                if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o700)) {
                    let _ = fs::remove_dir(&path);
                    return Err(format!(
                        "cannot seal private rig snapshot directory: {error}"
                    ));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot allocate private rig snapshot: {error}")),
        }
    }
    Err("cannot allocate a unique private rig snapshot".to_owned())
}

fn bridge_error(error: *mut c_char) -> String {
    if error.is_null() {
        return "native rig bridge call failed without a diagnostic".to_owned();
    }
    let text = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    unsafe { herdr_rig_error_free(error) };
    text
}

fn call_result(code: i32, error: *mut c_char) -> Result<(), String> {
    if code == 0 {
        if !error.is_null() {
            unsafe { herdr_rig_error_free(error) };
        }
        Ok(())
    } else {
        Err(bridge_error(error))
    }
}

/// Immutable, sealed filesystem inputs. This value contains no native/AppKit
/// objects and can cross from the portrait worker to the main-thread host.
pub(crate) struct RigSnapshot {
    snapshot: TempSnapshot,
    paths: SnapshotPaths,
    metadata: CharacterMetadata,
    content_digest: String,
    width: u32,
    height: u32,
    independent_models: bool,
    bindings: Option<[u8; 10]>,
    initial_pose_kind: i32,
    initial_motion: std::sync::Arc<Vec<u8>>,
}

impl RigSnapshot {
    fn materialize(
        asset: RigAsset,
        cleanup: Option<mpsc::Sender<PathBuf>>,
    ) -> Result<Self, String> {
        let independent_models = asset.supports_independent_models();
        ensure_rig_abi()?;
        let initial_motion = match (&asset.models, &asset.bindings) {
            (Some(models), Some(bindings)) => {
                let index = usize::from(
                    *bindings
                        .get(usize::try_from(asset.initial_pose_kind).map_err(|_| {
                            "independent rig initial pose kind is invalid".to_owned()
                        })?)
                        .ok_or_else(|| "independent rig initial pose kind is invalid".to_owned())?,
                );
                models
                    .get(index)
                    .ok_or_else(|| "independent rig initial binding is invalid".to_owned())?
                    .motion
                    .clone()
            }
            (None, None) => asset.motion.clone(),
            _ => return Err("independent rig bindings are invalid".to_owned()),
        };
        let (snapshot, paths) = TempSnapshot::materialize(&asset, cleanup)?;
        Ok(Self {
            snapshot,
            paths,
            metadata: asset.metadata,
            content_digest: asset.content_digest,
            width: asset.width,
            height: asset.height,
            independent_models,
            bindings: asset.bindings,
            initial_pose_kind: asset.initial_pose_kind,
            initial_motion,
        })
    }

    pub(crate) fn for_preview(
        asset: RigAsset,
        cleanup: mpsc::Sender<PathBuf>,
    ) -> Result<Self, String> {
        Self::materialize(asset, Some(cleanup))
    }

    fn accept_token(self, token: &RendererToken) -> Result<Self, String> {
        if self.content_digest != token.content_digest {
            return Err("renderer token does not match validated content".to_owned());
        }
        Ok(self)
    }
}

pub struct RigPreparation {
    mtm: MainThreadMarker,
    handle: Option<NonNull<c_void>>,
    snapshot: Option<TempSnapshot>,
    metadata: CharacterMetadata,
    token: RendererToken,
    width: u32,
    height: u32,
    independent_models: bool,
}

pub struct PreparedRig {
    // Drop cancels and releases the native host before the input snapshot and
    // retained AppKit view are released.
    handle: NonNull<c_void>,
    _snapshot: TempSnapshot,
    view: Retained<NSView>,
    mtm: MainThreadMarker,
    metadata: CharacterMetadata,
    token: RendererToken,
    width: u32,
    height: u32,
    independent_models: bool,
    display_bounds: (f64, f64, f64, f64),
    viewport_epoch: u64,
    last_error: Option<String>,
}
impl RigPreparation {
    pub fn new(
        asset: RigAsset,
        token: RendererToken,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        if asset.content_digest != token.content_digest {
            return Err("renderer token does not match validated content".to_owned());
        }
        Self::from_snapshot(RigSnapshot::materialize(asset, None)?, token, mtm)
    }

    pub(crate) fn from_snapshot(
        sealed: RigSnapshot,
        token: RendererToken,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        ensure_rig_abi()?;
        let sealed = sealed.accept_token(&token)?;
        let RigSnapshot {
            snapshot,
            paths,
            metadata,
            width,
            height,
            independent_models,
            bindings,
            initial_pose_kind,
            initial_motion,
            ..
        } = sealed;
        let operation_id = CString::new(token.operation_id.as_bytes())
            .map_err(|_| "renderer operation id contains NUL".to_owned())?;
        let reference_id = CString::new(token.reference.id.as_bytes())
            .map_err(|_| "renderer reference id contains NUL".to_owned())?;
        let content_digest = CString::new(token.content_digest.as_bytes())
            .map_err(|_| "renderer content digest contains NUL".to_owned())?;
        let model_inputs: Vec<CModelInput> = paths
            .models
            .iter()
            .map(|model| CModelInput {
                id: model.id.as_ptr(),
                file_path: model.file.as_ptr(),
                overrides_path: model.overrides.as_ptr(),
                motion_path: model.motion.as_ptr(),
            })
            .collect();
        let bindings = bindings.unwrap_or([0; 10]);
        let model_count = u32::try_from(model_inputs.len())
            .map_err(|_| "v5 model count exceeds the native ABI".to_owned())?;
        let c_asset = CAssetInput {
            width,
            height,
            base_path: paths.base.as_ptr(),
            pose_path: paths
                .pose
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            overrides_path: paths.overrides.as_ptr(),
            base_pose_id: paths.base_pose_id.as_ptr(),
            pose_id: paths
                .pose_id
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            models: if model_inputs.is_empty() {
                std::ptr::null()
            } else {
                model_inputs.as_ptr()
            },
            model_count,
            bindings: if model_inputs.is_empty() {
                std::ptr::null()
            } else {
                bindings.as_ptr()
            },
            initial_pose_kind,
            initial_motion_bytes: initial_motion.as_ptr(),
            initial_motion_length: initial_motion.len(),
        };
        let c_token = CTokenInput {
            operation_id: operation_id.as_ptr(),
            reference_id: reference_id.as_ptr(),
            reference_revision: token.reference.revision,
            content_digest: content_digest.as_ptr(),
            backend_epoch: token.backend_epoch,
        };
        let mut raw_handle = std::ptr::null_mut();
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe { herdr_rig_create(&c_asset, &c_token, &mut raw_handle, &mut raw_error) };
        if code != 0 {
            let message = bridge_error(raw_error);
            drop(snapshot);
            return Err(message);
        }
        let handle = NonNull::new(raw_handle)
            .ok_or_else(|| "native rig bridge returned a null preparation handle".to_owned())?;
        Ok(Self {
            mtm,
            handle: Some(handle),
            snapshot: Some(snapshot),
            metadata,
            token,
            width,
            height,
            independent_models,
        })
    }

    pub fn poll(&mut self, cancel: &AtomicBool) -> Result<Option<PreparedRig>, String> {
        let handle = self
            .handle
            .ok_or_else(|| "rig preparation has already completed".to_owned())?;
        let cancel_requested = cancel.load(Ordering::Acquire);
        let mut state = 0_u32;
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe {
            herdr_rig_poll(
                handle.as_ptr(),
                cancel_requested as u32,
                &mut state,
                &mut raw_error,
            )
        };
        if code != 0 {
            let message = bridge_error(raw_error);
            self.abort();
            return Err(message);
        }
        match state as i32 {
            0 => Ok(None),
            2 => {
                self.abort();
                Err("RIG_CANCELLED".to_owned())
            }
            1 => {
                let mut raw_view = std::ptr::null_mut();
                let mut view_error = std::ptr::null_mut();
                let code =
                    unsafe { herdr_rig_view(handle.as_ptr(), &mut raw_view, &mut view_error) };
                if code != 0 {
                    let message = bridge_error(view_error);
                    self.abort();
                    return Err(message);
                }
                let raw_view = NonNull::new(raw_view as *mut NSView)
                    .ok_or_else(|| "native rig bridge returned a null view".to_owned())?;
                let view = unsafe { Retained::from_raw(raw_view.as_ptr()) }
                    .ok_or_else(|| "native rig view retain was invalid".to_owned())?;
                let mut bounds = CAnchor::default();
                let display_bounds =
                    if unsafe { herdr_rig_display_bounds(handle.as_ptr(), &mut bounds) } != 0 {
                        (bounds.x0, bounds.y0, bounds.x1, bounds.y1)
                    } else {
                        (0.0, 0.0, 1.0, 1.0)
                    };
                let prepared = PreparedRig {
                    handle,
                    _snapshot: self
                        .snapshot
                        .take()
                        .ok_or_else(|| "rig snapshot was already consumed".to_owned())?,
                    view,
                    mtm: self.mtm,
                    metadata: self.metadata.clone(),
                    token: self.token.clone(),
                    width: self.width,
                    height: self.height,
                    independent_models: self.independent_models,
                    display_bounds,
                    viewport_epoch: 0,
                    last_error: None,
                };
                self.handle = None;
                Ok(Some(prepared))
            }
            _ => {
                self.abort();
                Err("native rig bridge returned an unknown preparation state".to_owned())
            }
        }
    }

    fn abort(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { herdr_rig_cancel(handle.as_ptr()) };
            unsafe { herdr_rig_destroy(handle.as_ptr()) };
        }
        self.snapshot.take();
    }
}

impl Drop for RigPreparation {
    fn drop(&mut self) {
        self.abort();
    }
}

impl PreparedRig {
    pub fn view(&self) -> &Retained<NSView> {
        &self.view
    }

    pub fn metadata(&self) -> &CharacterMetadata {
        &self.metadata
    }

    pub fn supports_independent_models(&self) -> bool {
        self.independent_models
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    /// Stable top-left source-canvas envelope, including every prepared model
    /// and pose and all bounded animation/physics states.
    pub fn display_bounds(&self) -> (f64, f64, f64, f64) {
        self.display_bounds
    }

    pub fn update(&mut self, intent: RigIntent) -> Result<(), String> {
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe { herdr_rig_update(self.handle.as_ptr(), &intent, &mut raw_error) };
        let result = call_result(code, raw_error);
        self.last_error = result.as_ref().err().cloned();
        result
    }

    /// Main mirrors its PresentationViewport epoch here.  The native host
    /// invalidates any old surface before the next current-intent draw.
    pub fn set_viewport_epoch(&mut self, epoch: u64) {
        self.viewport_epoch = epoch;
        unsafe { herdr_rig_set_viewport_epoch(self.handle.as_ptr(), epoch) };
    }

    pub fn prepare_surface(
        &mut self,
        intent: RigIntent,
        width: f64,
        height: f64,
        backing_scale: f64,
    ) -> Result<bool, String> {
        let mut ready = 0_u32;
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe {
            herdr_rig_prepare_surface(
                self.handle.as_ptr(),
                &intent,
                width,
                height,
                backing_scale,
                self.viewport_epoch,
                &mut ready,
                &mut raw_error,
            )
        };
        let result = call_result(code, raw_error).map(|()| ready != 0);
        if let Err(error) = &result {
            self.last_error = Some(error.clone());
        }
        result
    }

    pub fn activate(&mut self) -> Result<RendererToken, String> {
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe { herdr_rig_activate(self.handle.as_ptr(), &mut raw_error) };
        let result = call_result(code, raw_error).map(|()| self.token.clone());
        if let Err(error) = &result {
            self.last_error = Some(error.clone());
        }
        result
    }

    pub fn set_visible(&mut self, visible: bool) {
        unsafe { herdr_rig_set_visible(self.handle.as_ptr(), visible as u32) };
    }

    pub fn input_epoch(&self) -> Option<u64> {
        let epoch = unsafe { herdr_rig_input_epoch(self.handle.as_ptr()) };
        (epoch != 0).then_some(epoch)
    }

    pub fn input_ready(&self) -> bool {
        unsafe { herdr_rig_input_ready(self.handle.as_ptr()) != 0 }
    }
    /// One native lock acquisition owns freshness, provenance and CPU geometry.
    pub(crate) fn speech_anchor_snapshot(
        &self,
        expected: PresentationViewport,
    ) -> Result<RigSpeechAnchorSnapshot, String> {
        let mut output = CSpeechAnchorSnapshot::default();
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe {
            herdr_rig_speech_anchor_snapshot(self.handle.as_ptr(), &mut output, &mut raw_error)
        };
        call_result(code, raw_error)?;
        let invalid = || RigSpeechAnchorSnapshot {
            status: RigSpeechAnchorStatus::Invalid,
            backend_epoch: self.token.backend_epoch,
            input_epoch: None,
            anchor_epoch: None,
            viewport: None,
            canvas_width: self.width,
            canvas_height: self.height,
        };
        if output.status == 0 {
            return Ok(invalid());
        }
        let viewport = PresentationViewport {
            width: output.viewport_width,
            height: output.viewport_height,
            backing_scale: output.backing_scale,
            epoch: output.viewport_epoch,
        };
        if output.reserved != 0
            || output.backend_epoch != self.token.backend_epoch
            || output.input_epoch == 0
            || output.canvas_width != self.width
            || output.canvas_height != self.height
            || self.width == 0
            || self.height == 0
            || expected.epoch != self.viewport_epoch
            || !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || !viewport.backing_scale.is_finite()
            || viewport.width <= 0.0
            || viewport.height <= 0.0
            || !(1.0..=4.0).contains(&viewport.backing_scale)
            || viewport != expected
        {
            return Ok(invalid());
        }
        let status = match output.status {
            1 => RigSpeechAnchorStatus::TemporarilyUnavailable,
            2 => RigSpeechAnchorStatus::ReadyAnchorless,
            3 => {
                let anchor = output.anchor;
                if ![anchor.x0, anchor.y0, anchor.x1, anchor.y1]
                    .iter()
                    .all(|value| value.is_finite())
                    || anchor.x0 < 0.0
                    || anchor.y0 < 0.0
                    || anchor.x0 >= anchor.x1
                    || anchor.y0 >= anchor.y1
                    || anchor.x1 > f64::from(self.width)
                    || anchor.y1 > f64::from(self.height)
                {
                    return Ok(invalid());
                }
                RigSpeechAnchorStatus::ReadyAnchor((anchor.x0, anchor.y0, anchor.x1, anchor.y1))
            }
            _ => return Ok(invalid()),
        };
        Ok(RigSpeechAnchorSnapshot {
            status,
            backend_epoch: output.backend_epoch,
            input_epoch: Some(output.input_epoch),
            anchor_epoch: Some(output.anchor_epoch),
            viewport: Some(viewport),
            canvas_width: output.canvas_width,
            canvas_height: output.canvas_height,
        })
    }

    pub fn hit(&self, x: f64, y: f64) -> Option<RigHit> {
        let mut output = CHit::default();
        let mut has_hit = 0_u32;
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe {
            herdr_rig_hit(
                self.handle.as_ptr(),
                x,
                y,
                &mut output,
                &mut has_hit,
                &mut raw_error,
            )
        };
        if code != 0 {
            if !raw_error.is_null() {
                unsafe { herdr_rig_error_free(raw_error) };
            }
            return None;
        }
        if has_hit == 0
            || output.frame == 0
            || output.viewport_epoch != self.viewport_epoch
            || self.input_epoch() != Some(output.input_epoch)
        {
            return None;
        }
        let region = match output.region {
            1 => Some(RigRegion::Head),
            2 => Some(RigRegion::Body),
            0 => None,
            _ => return None,
        };
        if region.is_some()
            && (!output.source_x.is_finite()
                || !output.source_y.is_finite()
                || output.source_x < 0.0
                || output.source_x >= f64::from(self.width)
                || output.source_y < 0.0
                || output.source_y >= f64::from(self.height))
        {
            return None;
        }
        Some(RigHit {
            alpha: output.alpha,
            region,
        })
    }

    pub fn preview_png(&mut self, intent: RigIntent, hit_overlay: bool) -> Result<Vec<u8>, String> {
        let mut bytes = std::ptr::null_mut();
        let mut length = 0_usize;
        let mut raw_error = std::ptr::null_mut();
        let code = unsafe {
            herdr_rig_preview_png(
                self.handle.as_ptr(),
                &intent,
                u32::from(hit_overlay),
                &mut bytes,
                &mut length,
                &mut raw_error,
            )
        };
        if code != 0 {
            let message = bridge_error(raw_error);
            self.last_error = Some(message.clone());
            return Err(message);
        }
        let Some(bytes_ptr) = NonNull::new(bytes) else {
            return Err("native rig preview returned no bytes".to_owned());
        };
        let result = unsafe { std::slice::from_raw_parts(bytes_ptr.as_ptr(), length) }.to_vec();
        unsafe { herdr_rig_bytes_free(bytes_ptr.as_ptr(), length) };
        Ok(result)
    }

    pub fn token(&self) -> &RendererToken {
        &self.token
    }

    pub fn error(&self) -> Option<String> {
        if let Some(error) = &self.last_error {
            return Some(error.clone());
        }
        let raw = unsafe { herdr_rig_last_error(self.handle.as_ptr()) };
        if raw.is_null() {
            return None;
        }
        let error = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        unsafe { herdr_rig_error_free(raw) };
        Some(error)
    }
}

impl Drop for PreparedRig {
    fn drop(&mut self) {
        unsafe { herdr_rig_set_visible(self.handle.as_ptr(), 0) };
        unsafe { herdr_rig_destroy(self.handle.as_ptr()) };
        // Keep mtm in the value so accidental Send/Sync derivations cannot make
        // AppKit ownership appear worker-safe.
        let _ = self.mtm;
    }
}

// Rust's public adapter intentionally keeps the token and metadata on the
// prepared value; Main's PreparedCharacter can delegate to these accessors.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character_types::CharacterRef;
    use std::sync::Arc;
    #[test]
    fn rejects_incompatible_native_abi() {
        let valid = expected_rig_abi();
        assert!(check_rig_abi(valid).is_ok());
        let mut wrong_version = valid;
        wrong_version.version += 1;
        assert!(check_rig_abi(wrong_version).is_err());
        for mutate in [
            |info: &mut CRigABIInfoV1| info.asset_size += 1,
            |info: &mut CRigABIInfoV1| info.model_size += 1,
            |info: &mut CRigABIInfoV1| info.token_size += 1,
            |info: &mut CRigABIInfoV1| info.intent_size += 1,
            |info: &mut CRigABIInfoV1| info.hit_size += 1,
            |info: &mut CRigABIInfoV1| info.anchor_size += 1,
            |info: &mut CRigABIInfoV1| info.speech_anchor_snapshot_size += 1,
        ] {
            let mut incompatible = valid;
            mutate(&mut incompatible);
            assert!(check_rig_abi(incompatible).is_err());
        }
    }

    #[test]
    fn worker_sealed_snapshot_is_consumed_without_rematerialization_and_cleaned_off_main() {
        let (cleanup, paths) = mpsc::channel();
        let cleanup_worker = std::thread::spawn(move || {
            let path: PathBuf = paths.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(path.join("base.psd").exists());
            fs::remove_dir_all(&path).unwrap();
            path
        });
        let asset = RigAsset {
            width: 384,
            height: 512,
            base: Arc::new(vec![1, 2, 3, 4]),
            pose: None,
            overrides: Arc::new(b"{}".to_vec()),
            motion: Arc::new(b"{}".to_vec()),
            base_pose_id: "idle".to_owned(),
            pose_id: None,
            metadata: CharacterMetadata { dialogue: None },
            content_digest: "a".repeat(64),
            models: None,
            bindings: None,
            initial_pose_kind: -1,
        };
        let sealed = std::thread::spawn(move || RigSnapshot::for_preview(asset, cleanup).unwrap())
            .join()
            .unwrap();
        let root = sealed.snapshot.root.clone();
        assert_eq!(fs::read(root.join("base.psd")).unwrap(), [1, 2, 3, 4]);
        assert_eq!(
            fs::metadata(root.join("base.psd"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        let token = RendererToken::new(
            "portrait-preview".to_owned(),
            CharacterRef::builtin(),
            "a".repeat(64),
        )
        .unwrap();
        let accepted = sealed.accept_token(&token).unwrap();
        assert_eq!(
            accepted.paths.base.as_bytes(),
            root.join("base.psd").as_os_str().as_bytes()
        );
        drop(accepted); // Sends only a path; worker owns filesystem removal.
        assert_eq!(cleanup_worker.join().unwrap(), root);
        assert!(!root.exists());
    }

    #[test]
    fn sealed_v5_snapshot_keeps_the_motion_of_its_initial_semantic_binding() {
        let (cleanup, paths) = mpsc::channel();
        let first_motion = Arc::new(b"first model".to_vec());
        let idle_motion = Arc::new(b"bound idle model".to_vec());
        let model = |id: &str, motion: Arc<Vec<u8>>| crate::assets::RigModelAsset {
            id: id.to_owned(),
            file: Arc::new(vec![1]),
            overrides: Arc::new(b"{}".to_vec()),
            motion,
        };
        let asset = RigAsset {
            width: 384,
            height: 512,
            base: Arc::new(vec![1]),
            pose: None,
            overrides: Arc::new(b"{}".to_vec()),
            motion: first_motion.clone(),
            base_pose_id: "first".to_owned(),
            pose_id: None,
            metadata: CharacterMetadata { dialogue: None },
            content_digest: "b".repeat(64),
            models: Some(
                (0..10)
                    .map(|index| {
                        let motion = match index {
                            0 => first_motion.clone(),
                            1 => idle_motion.clone(),
                            _ => Arc::new(vec![index]),
                        };
                        model(&format!("model-{index}"), motion)
                    })
                    .collect::<Vec<_>>()
                    .into(),
            ),
            bindings: Some([1, 0, 2, 3, 4, 5, 6, 7, 8, 9]),
            initial_pose_kind: 0,
        };
        let sealed = std::thread::spawn(move || RigSnapshot::for_preview(asset, cleanup).unwrap())
            .join()
            .unwrap();
        assert!(Arc::ptr_eq(&sealed.initial_motion, &idle_motion));
        assert_eq!(sealed.initial_motion.as_slice(), b"bound idle model");
        drop(sealed);
        fs::remove_dir_all(paths.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    }
}
