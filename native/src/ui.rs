use crate::animation::{phase_index, FrameId, Playback};
use crate::assets::CharacterMetadata;
use crate::assets::{AssetPack, ValidatedCharacter};
use crate::behavior::{
    Behavior, EffectKind, Presentation, PresentationIntent, PresentationViewport, Reaction,
};
use crate::bubble::{
    place_bubble, BubbleGeometry, BubblePlacement, Rect as BubbleRect, BUBBLE_RADIUS,
    BUBBLE_WINDOW_INSET,
};
use crate::character_menu::{self, CharacterMenu, MenuCommand};
use crate::character_renderer::{self, CharacterHit, PrepareBuilder, PreparedCharacter};
use crate::character_service::PackService;
use crate::character_types::{
    CharacterRef, PackAction, PackListing, PackOperation, PackRequest, RendererToken,
};
use crate::control;
use crate::dialogue::{effective_metadata, DialogueSlot, DialogueTarget};
use crate::display_geometry::{DisplayGeometry, BASE_HEIGHT, BASE_WIDTH};
use crate::herdr::{PromptError, PromptSender};
use crate::i18n::{
    default_dialogue, disconnected_sources, language_save_failure, resolve_language,
    status_indicator_summary, task_disclosure, task_status, text, LanguagePreference, Message,
    TaskStatus, UiLocale,
};
use crate::interaction::{GestureAction, Interaction, Point, RegionPolicy};
use crate::lifecycle::{LifecycleSetting, Paths};
use crate::menu_panel::MenuPanel;
use crate::preferences::{BubbleAppearance, BubbleColor, BubblePalette, BubbleTheme, Preferences};
use crate::session_cards::{minimum_selectable_height, SessionCards};
use crate::session_view::{SessionKey, SessionStatusSummary};
use crate::sources::ObservationPreferences;
use crate::state::{AppState, Phase, Scene, MAX_SCALE, MIN_SCALE};
use crate::status_indicator::{
    semantic_color, StatusIcon, STATUS_ICON_GAP, STATUS_ICON_SIZE, STATUS_ROW_HEIGHT,
};
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAppearance, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezierPath, NSBorderType, NSButton, NSCell, NSColor,
    NSControlStateValueOn, NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition,
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventTrackingRunLoopMode, NSEventType,
    NSFloatingWindowLevel, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSImageScaling, NSImageView, NSLayoutManager, NSLineBreakMode, NSModalPanelRunLoopMode,
    NSModalResponseOK, NSMutableParagraphStyle, NSOpenPanel, NSPanel,
    NSParagraphStyleAttributeName, NSPopUpButton, NSRunningApplication, NSScreen, NSScrollView,
    NSScrollerStyle, NSStatusBar, NSStatusItem, NSSwitch, NSTextAlignment, NSTextContainer,
    NSTextField, NSTextStorage, NSTextView, NSTrackingArea, NSTrackingAreaOptions,
    NSUserInterfaceItemIdentification, NSVariableStatusItemLength, NSView,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSAttributedString, NSCopying, NSCurrentLocaleDidChangeNotification, NSDate, NSLocale,
    NSMutableAttributedString, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
    NSPoint, NSProcessInfo, NSRange, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSTimer,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
static NEXT_UI_OPERATION: AtomicU64 = AtomicU64::new(1);

enum PendingNative {
    Preparing {
        token: RendererToken,
        builder: PrepareBuilder,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
        deadline: Instant,
    },
    Ready {
        token: RendererToken,
        prepared: PreparedCharacter,
        cancel: Arc<AtomicBool>,
    },
    Applying {
        token: RendererToken,
        prepared: PreparedCharacter,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
        deadline: Instant,
    },
}

enum DeferredBridge {
    Prepare {
        token: RendererToken,
        assets: Box<ValidatedCharacter>,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
    },
    Apply {
        token: RendererToken,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
    },
    Discard {
        token: RendererToken,
    },
}

struct StartupSelection {
    reference: CharacterRef,
    override_active: bool,
    prepared: PreparedCharacter,
    error: Option<String>,
}

static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
static FRAME_RESIZE_CURSOR_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 15
});

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static STARTUP_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    static SCREEN_CHANGE_PENDING: Cell<bool> = const { Cell::new(false) };
    static DEFERRED_BRIDGES: RefCell<VecDeque<DeferredBridge>> = const { RefCell::new(VecDeque::new()) };
}
const BUBBLE_BODY_MIN_WIDTH: f64 = 120.0;
const BUBBLE_COMPACT_MAX_WIDTH: f64 = 260.0;
const BUBBLE_EXPANDED_MIN_WIDTH: f64 = 320.0;
const BUBBLE_EXPANDED_MAX_WIDTH: f64 = 320.0;
const BUBBLE_HORIZONTAL_INSET: f64 = 12.0;
const BUBBLE_VERTICAL_INSET: f64 = 8.0;
const BUBBLE_PRIMARY_FONT_SIZE: f64 = 13.0;
const BUBBLE_SECONDARY_FONT_SIZE: f64 = 11.0;
const BUBBLE_LINE_HEIGHT: f64 = 18.0;
const BUBBLE_CONTROL_HEIGHT: f64 = 20.0;
const BUBBLE_COLLAPSE_WIDTH: f64 = 72.0;
const BUBBLE_CONTENT_GAP: f64 = 4.0;
const BUBBLE_MESSAGE_MAX_HEIGHT: f64 = 116.0;
const BUBBLE_CARDS_MAX_HEIGHT: f64 = 180.0;
const COMPOSER_EDITOR_HEIGHT: f64 = 64.0;
const COMPOSER_TARGET_HEIGHT: f64 = 20.0;
const COMPOSER_STATUS_HEIGHT: f64 = 34.0;
const COMPOSER_MIN_EDITOR_HEIGHT: f64 = 20.0;
const COMPOSER_MIN_STATUS_HEIGHT: f64 = 18.0;

// Tight spacing is reserved for expanded bubbles whose normal one-row layout
// cannot fit the visible screen. OFF keeps the legacy spacing.
#[derive(Clone, Copy)]
struct ExpandedSpacing {
    inset: f64,
    gap: f64,
}

fn expanded_spacing(
    body_height: f64,
    cards_height: f64,
    show_status: bool,
    has_message: bool,
) -> ExpandedSpacing {
    if show_status && expanded_minimum_height(cards_height, true, has_message) > body_height {
        ExpandedSpacing {
            // Recover the three points needed for the fixed status/dialogue gap
            // without reducing the selectable row or composer minimums.
            inset: 2.5,
            gap: 1.0,
        }
    } else {
        ExpandedSpacing {
            inset: BUBBLE_VERTICAL_INSET,
            gap: BUBBLE_CONTENT_GAP,
        }
    }
}

fn expanded_height_budget(
    cards_height: f64,
    show_status: bool,
    has_message: bool,
    spacing: ExpandedSpacing,
) -> f64 {
    spacing.inset * 2.0
        + BUBBLE_CONTROL_HEIGHT
        + spacing.gap
            * (if show_status && !has_message {
                4.0
            } else {
                5.0
            })
        + if show_status {
            STATUS_ROW_HEIGHT + BUBBLE_CONTENT_GAP
        } else {
            0.0
        }
        + COMPOSER_TARGET_HEIGHT
        + COMPOSER_MIN_EDITOR_HEIGHT
        + COMPOSER_MIN_STATUS_HEIGHT
        + cards_height.min(minimum_selectable_height())
        + if has_message { BUBBLE_LINE_HEIGHT } else { 0.0 }
}

// Allocation is shared by sizing and frame layout; the card minimum includes
// its filter, summary and one full row rather than only its scroll viewport.
fn expanded_minimum_height(cards_height: f64, show_status: bool, has_message: bool) -> f64 {
    expanded_height_budget(
        cards_height,
        show_status,
        has_message,
        ExpandedSpacing {
            inset: BUBBLE_VERTICAL_INSET,
            gap: BUBBLE_CONTENT_GAP,
        },
    )
}

fn expanded_message_top(
    status_bottom: f64,
    content_top: f64,
    show_status: bool,
    has_message: bool,
) -> f64 {
    if show_status {
        status_bottom - if has_message { BUBBLE_CONTENT_GAP } else { 0.0 }
    } else {
        content_top
    }
}

struct ExpandedHeights {
    status: f64,
    editor: f64,
    target: f64,
    cards: f64,
    message: f64,
}

fn expanded_heights(
    body_height: f64,
    desired_cards: f64,
    desired_message: f64,
    show_status: bool,
    spacing: ExpandedSpacing,
) -> ExpandedHeights {
    let mut remaining = (body_height
        - spacing.inset * 2.0
        - BUBBLE_CONTROL_HEIGHT
        - spacing.gap
            * (if show_status && desired_message == 0.0 {
                4.0
            } else {
                5.0
            })
        - if show_status {
            STATUS_ROW_HEIGHT + BUBBLE_CONTENT_GAP
        } else {
            0.0
        })
    .max(0.0);
    let target = COMPOSER_TARGET_HEIGHT.min(remaining);
    remaining -= target;
    let editor = COMPOSER_MIN_EDITOR_HEIGHT.min(remaining);
    remaining -= editor;
    let cards = desired_cards
        .min(minimum_selectable_height())
        .min(remaining);
    remaining -= cards;
    let status = COMPOSER_MIN_STATUS_HEIGHT.min(remaining);
    remaining -= status;
    let message = desired_message.min(BUBBLE_LINE_HEIGHT).min(remaining);
    remaining -= message;

    let editor_extra = (COMPOSER_EDITOR_HEIGHT - editor).min(remaining);
    remaining -= editor_extra;
    let status_extra = (COMPOSER_STATUS_HEIGHT - status).min(remaining);
    remaining -= status_extra;
    let cards_extra = (desired_cards - cards).min(remaining);
    remaining -= cards_extra;
    ExpandedHeights {
        status: status + status_extra,
        editor: editor + editor_extra,
        target,
        cards: cards + cards_extra,
        message: message + (desired_message - message).min(remaining),
    }
}
const COMPOSER_DRAFT_LIMIT: usize = 32;

fn bubble_color(color: BubbleColor, alpha: f64) -> Retained<NSColor> {
    let (r, g, b) = color.rgb();
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha)
}

fn bubble_surface_is_dark(color: BubbleColor) -> bool {
    let (r, g, b) = color.rgb();
    0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
}

fn bubble_paragraph_style(
    line_spacing: f64,
    paragraph_spacing: f64,
) -> Retained<NSMutableParagraphStyle> {
    let style = NSMutableParagraphStyle::new();
    style.setLineSpacing(line_spacing);
    style.setParagraphSpacing(paragraph_spacing);
    style.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    style
}

fn set_attributed_field_text(
    field: &NSTextField,
    text: &str,
    font: &NSFont,
    color: &NSColor,
    paragraph_style: &NSMutableParagraphStyle,
) {
    let string = NSString::from_str(text);
    let attributed = NSMutableAttributedString::from_nsstring(&string);
    let length = string.length();
    if length > 0 {
        let range = NSRange::new(0, length);
        unsafe {
            attributed.addAttribute_value_range(NSFontAttributeName, font, range);
            attributed.addAttribute_value_range(NSForegroundColorAttributeName, color, range);
            attributed.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                paragraph_style,
                range,
            );
        }
    }
    if let Some(cell) = field.cell() {
        cell.setAttributedStringValue(&*attributed);
    } else {
        field.setStringValue(&string);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BubbleMode {
    Compact,
    Expanded,
}

#[derive(Clone, Debug)]
struct BubbleLayout {
    primary: String,
    compact_primary: String,
    secondary: String,
    full_message: String,
    overflow: bool,
    body_size: NSSize,
    message_content_height: f64,
}

impl Default for BubbleLayout {
    fn default() -> Self {
        Self {
            primary: String::new(),
            compact_primary: String::new(),
            secondary: String::new(),
            full_message: String::new(),
            overflow: false,
            body_size: NSSize::new(BUBBLE_BODY_MIN_WIDTH, 120.0),
            message_content_height: BUBBLE_LINE_HEIGHT,
        }
    }
}
#[derive(Clone, Debug)]
struct TextMetrics {
    size: NSSize,
    line_ranges: Vec<NSRange>,
    line_widths: Vec<f64>,
    char_wrapping: bool,
}

impl TextMetrics {
    fn line_count(&self) -> usize {
        self.line_ranges.len()
    }
}

#[derive(Clone, Copy, Debug)]
struct BubbleFade {
    generation: u64,
    started: Instant,
}

const GRIP_HIT_SIZE: f64 = 24.0;
const GRIP_DRAW_INSET: f64 = 4.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GestureKind {
    Move,
    Resize,
}

#[derive(Clone, Copy, Debug)]
struct InputOwner {
    backend_epoch: u64,
    input_epoch: u64,
    viewport_epoch: u64,
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    kind: GestureKind,
    start_mouse: NSPoint,
    start_frame: NSRect,
    start_top_left: NSPoint,
    start_scale: f64,
    screen_visible: NSRect,
    expected_frame: NSRect,
    expected_scale: f64,
    expected_visible: bool,
    expected_passthrough: bool,
    expected_alpha_passthrough: bool,
    expected_bubble_visible: bool,
    expected_bubble_placement: BubblePlacement,
    expected_reset_position_revision: u64,
    input_owner: Option<InputOwner>,
}

#[derive(Clone, Copy, Debug)]
struct PointerPress {
    start_mouse: NSPoint,
    start_frame: NSRect,
    presentation: Presentation,
    input_owner: Option<InputOwner>,
}

#[derive(Debug)]
struct GripViewIvars;

#[derive(Debug)]
struct PetViewIvars {
    drag: Cell<Option<DragState>>,
    hover_pet: Cell<bool>,
    hover_grip: Cell<bool>,
    grip: Retained<GripView>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
}

#[derive(Debug)]
struct BubbleViewIvars {
    drag: Cell<Option<DragState>>,
    geometry: RefCell<Option<BubbleGeometry>>,
    path: RefCell<Option<Retained<NSBezierPath>>>,
    native_regions: RefCell<Vec<NSRect>>,
    palette: Cell<BubblePalette>,
    opaque_surface: Cell<bool>,
}

#[derive(Debug)]
struct ComposerViewIvars;

define_class!(
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetComposerView"]
    #[ivars = ComposerViewIvars]
    struct ComposerView;
    unsafe impl NSObjectProtocol for ComposerView {}

    impl ComposerView {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let marked: bool = unsafe { msg_send![self, hasMarkedText] };
            if !marked
                && matches!(event.keyCode(), 36 | 76)
                && event.modifierFlags().contains(NSEventModifierFlags::Command)
            {
                with_ui_mut(|ui| ui.submit_composer());
            } else if event.keyCode() == 53 {
                if marked {
                    // Let Cocoa's input context handle Escape first; some IMEs consume it themselves.
                    let _: () = unsafe { msg_send![super(self), keyDown: event] };
                    self.discard_composition();
                } else {
                    with_ui_mut(|ui| ui.collapse_bubble());
                }
            } else {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, sender: Option<&AnyObject>) {
            if !self.discard_composition() {
                let _: () = unsafe { msg_send![super(self), cancelOperation: sender] };
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                let _ = window.makeFirstResponder(Some(self));
            }
            let _: () = unsafe { msg_send![super(self), mouseDown: event] };
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

impl ComposerView {
    fn discard_composition(&self) -> bool {
        let marked: bool = unsafe { msg_send![self, hasMarkedText] };
        if !marked {
            return false;
        }
        let context: Option<&AnyObject> = unsafe { msg_send![self, inputContext] };
        if let Some(context) = context {
            let _: () = unsafe { msg_send![context, discardMarkedText] };
        }
        let still_marked: bool = unsafe { msg_send![self, hasMarkedText] };
        if still_marked {
            // A discarded conversion can leave the view's preedit range marked.
            // Replace that range rather than accepting the pending characters.
            let range: NSRange = unsafe { msg_send![self, markedRange] };
            let empty = NSString::from_str("");
            let _: () = unsafe { msg_send![self, insertText: &*empty, replacementRange: range] };
            let _: () = unsafe { msg_send![self, unmarkText] };
        }
        true
    }

    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ComposerViewIvars);
        unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0), NSSize::new(320.0, COMPOSER_EDITOR_HEIGHT)
            )]
        }
    }
}
#[derive(Debug)]
struct BubblePanelIvars;

struct AppDelegateIvars {
    shared: Arc<Mutex<AppState>>,
    assets: PathBuf,
    packs: Arc<PackService>,
    prefs: RefCell<Option<Preferences>>,
}

#[derive(Clone, Copy)]
struct AnchorKey {
    frame: Option<FrameId>,
    artwork: NSRect,
    backend_epoch: u64,
    input_epoch: Option<u64>,
    anchor_epoch: Option<u64>,
    ready: bool,
    phase: Phase,
    effect: Option<EffectKind>,
}

struct CachedAnchor {
    key: AnchorKey,
    // Relative to the pet panel; dragging translates without resampling art.
    relative: Option<BubbleRect>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ComposerRenderStamp {
    cards: (Option<u64>, u64, UiLocale),
    locale: UiLocale,
    pending: bool,
    live_revision: Option<u64>,
}

struct Ui {
    mtm: MainThreadMarker,
    shared: Arc<Mutex<AppState>>,
    packs: Arc<PackService>,
    prefs: Preferences,
    lifecycle_paths: Paths,
    panel: Retained<NSPanel>,
    root: Retained<PetView>,
    image_view: Retained<NSImageView>,
    bubble_panel: Retained<BubblePanel>,
    bubble_root: Retained<BubbleView>,
    bubble: Retained<NSTextField>,
    dialogue: Retained<NSTextField>,
    status_icon: StatusIcon,
    status_label: Retained<NSTextField>,
    disclosure: Retained<NSButton>,
    collapse: Retained<NSButton>,
    message_scroll: Retained<NSScrollView>,
    message_view: Retained<NSTextView>,
    composer_scroll: Retained<NSScrollView>,
    composer_view: Retained<ComposerView>,
    composer_target: Retained<NSTextField>,
    composer_status: Retained<NSTextField>,
    composer_send: Retained<NSButton>,
    prompt_sender: PromptSender,
    composer_render_stamp: Option<ComposerRenderStamp>,
    composer_key: Option<SessionKey>,
    composer_pending_key: Option<SessionKey>,
    composer_drafts: VecDeque<(SessionKey, String)>,
    composer_results: VecDeque<(SessionKey, String)>,
    cards: SessionCards,
    menu_panel: MenuPanel,
    locale: UiLocale,
    pending_language: Option<(LanguagePreference, UiLocale)>,
    effective_dialogue: CharacterMetadata,
    dialogue_target: Option<DialogueTarget>,
    dialogue_name: String,
    dialogue_pack_generation: u64,
    dialogue_override_active: bool,
    dialogue_prepared_epoch: u64,
    active: PreparedCharacter,
    display_geometry: DisplayGeometry,
    bubble_appearance: BubbleAppearance,
    cached_anchor: Option<CachedAnchor>,
    displayed_frame: Option<FrameId>,
    viewport: PresentationViewport,
    playback: Playback,
    pending_native: Option<PendingNative>,
    character_menu: CharacterMenu,
    pack_operation: Option<PackOperation>,
    pack_error: Option<String>,
    behavior: Behavior,
    interaction: Interaction,
    launch_time: Instant,
    pointer_press: Option<PointerPress>,
    pointer_event_started_at: Option<f64>,
    timer: Option<Retained<NSTimer>>,
    pointer_timer: Option<Retained<NSTimer>>,
    prepare_timer: Option<Retained<NSTimer>>,
    prepare_timer_operation: Option<String>,
    language_timer: Option<Retained<NSTimer>>,
    timer_target: Retained<TimerTarget>,
    presentation: Presentation,
    status_text: String,
    status_summary: SessionStatusSummary,
    dialogue_text: String,
    disconnect_text: String,
    bubble_mode: BubbleMode,
    bubble_layout: BubbleLayout,
    bubble_geometry: Option<BubbleGeometry>,
    bubble_content_dirty: bool,
    pending_bubble_placement: Option<BubblePlacement>,
    pending_bubble_content: bool,
    bubble_layout_dirty: bool,
    transition_generation: u64,
    bubble_fade: Option<BubbleFade>,
    reduced_motion: bool,
    _status_item: Retained<NSStatusItem>,
    _menu_target: Retained<MenuTarget>,
    _menu_event_monitors: Vec<Retained<AnyObject>>,
    _window_delegate: Retained<WindowDelegate>,
    last_scene: Scene,
    last_reset_position_revision: u64,
    did_present: bool,
    force_image: bool,
}

define_class!(
    // SAFETY:
    // - NSView has no subclassing requirements beyond NSObject lifetime rules.
    // - GripView is main-thread-only and does not implement Drop.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetGripView"]
    #[ivars = GripViewIvars]
    #[derive(Debug)]
    struct GripView;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for GripView {}

    impl GripView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let background = NSColor::colorWithSRGBRed_green_blue_alpha(0.08, 0.08, 0.10, 0.62);
            background.setFill();
            let plate = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                bounds,
                5.0,
                5.0,
            );
            plate.fill();

            let color = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.84);
            color.setStroke();
            let path = NSBezierPath::bezierPath();
            path.setLineWidth(2.0);
            let inset = GRIP_DRAW_INSET;
            for offset in [0.0, 5.0, 10.0] {
                path.moveToPoint(NSPoint::new(
                    bounds.size.width - inset - offset,
                    inset,
                ));
                path.lineToPoint(NSPoint::new(
                    bounds.size.width - inset,
                    inset + offset,
                ));
            }
            path.stroke();
        }
    }
);

define_class!(
    // SAFETY:
    // - TimerTarget is retained by Ui for the lifetime of the scheduled timer.
    // - NSTimer invokes it only on the AppKit run-loop thread.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetTimerTarget"]
    struct TimerTarget;

    unsafe impl NSObjectProtocol for TimerTarget {}

    impl TimerTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            with_ui_mut(|ui| ui.frame_tick());
        }

        #[unsafe(method(prepareTick:))]
        fn prepare_tick(&self, _timer: &NSTimer) {
            with_ui_mut(|ui| ui.prepare_tick());
        }
        #[unsafe(method(pointerTick:))]
        fn pointer_tick(&self, _timer: &NSTimer) {
            with_ui_mut(|ui| ui.pointer_tick());
        }
        #[unsafe(method(languageTick:))]
        fn language_tick(&self, _timer: &NSTimer) {
            with_ui_mut(|ui| {
                ui.apply_pending_language();
                ui.apply_pending_bubble_updates();
                if !ui.pending_ui_updates() {
                    ui.stop_language_timer();
                }
            });
        }
    }
);

define_class!(
    // SAFETY:
    // - NSView has no subclassing requirements beyond NSObject lifetime rules.
    // - PetView is main-thread-only and does not implement Drop.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetPetView"]
    #[ivars = PetViewIvars]
    struct PetView;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for PetView {}

    impl PetView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<&NSView> {
            let bounds = self.bounds();
            if point.x >= bounds.origin.x
                && point.x <= bounds.origin.x + bounds.size.width
                && point.y >= bounds.origin.y
                && point.y <= bounds.origin.y + bounds.size.height
            {
                // Keep all mouse ownership at the root so the image, bubble and grip
                // cannot split a gesture across child views.
                Some(&**self)
            } else {
                None
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, true) { return; }
            let Some(window) = self.window() else {
                return;
            };
            let local_point = event.locationInWindow();
            let screen_point = window.convertPointToScreen(local_point);
            let start_frame = window.frame();
            let resize = grip_hit_test(local_point, self.bounds().size);
            let option = event
                .modifierFlags()
                .contains(NSEventModifierFlags::Option);
            let drag = Cell::new(None);
            with_ui_mut(|ui| {
                drag.set(ui.pointer_down(
                    local_point,
                    screen_point,
                    start_frame,
                    option,
                    resize,
                    event.timestamp(),
                ));
            });
            if let Some(drag) = drag.get() {
                self.ivars().drag.set(Some(drag));
                self.set_gesture_visuals(Some(drag.kind));
            } else {
                self.ivars().drag.set(None);
                self.set_gesture_visuals(None);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) { return; }
            let Some(window) = self.window() else {
                return;
            };
            let local_point = event.locationInWindow();
            let screen_point = window.convertPointToScreen(local_point);
            let current_drag = self.ivars().drag.get();
            let updated = Cell::new(None);
            let processed = Cell::new(false);
            with_ui_mut(|ui| {
                processed.set(true);
                if let Some(drag) = current_drag {
                    updated.set(ui.update_gesture(drag, screen_point));
                } else {
                    updated.set(ui.pointer_motion(local_point, screen_point));
                }
            });
            if processed.get() {
                self.ivars().drag.set(updated.get());
                self.set_gesture_visuals(updated.get().map(|drag| drag.kind));
            }

        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) { return; }
            let local_point = event.locationInWindow();
            let drag = self.ivars().drag.get();
            with_ui_mut(|ui| {
                if let Some(drag) = drag {
                    ui.finish_gesture(drag);
                }
                ui.pointer_end(local_point);
            });
            self.ivars().drag.set(None);
            self.set_gesture_visuals(None);
        }

        #[unsafe(method(mouseCancelled:))]
        fn mouse_cancelled(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) { return; }
            let drag = self.ivars().drag.get();
            with_ui_mut(|ui| {
                ui.cancel_gesture_for(drag);
                ui.cancel_pointer();
            });
            self.ivars().drag.set(None);
            self.set_gesture_visuals(None);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let hovered = grip_hit_test(event.locationInWindow(), self.bounds().size);
            self.ivars().hover_pet.set(true);
            self.ivars().hover_grip.set(hovered);
            self.set_gesture_visuals(self.ivars().drag.get().map(|drag| drag.kind));
            with_ui_mut(|ui| ui.set_hover(true, hovered));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            let hovered = grip_hit_test(event.locationInWindow(), self.bounds().size);
            self.ivars().hover_pet.set(true);
            self.ivars().hover_grip.set(hovered);
            self.set_gesture_visuals(self.ivars().drag.get().map(|drag| drag.kind));
            with_ui_mut(|ui| ui.set_hover(true, hovered));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hover_pet.set(false);
            self.ivars().hover_grip.set(false);
            self.set_gesture_visuals(self.ivars().drag.get().map(|drag| drag.kind));
            with_ui_mut(|ui| ui.set_hover(false, false));
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY:
    // - BubblePanel is main-thread-only and forwards all non-Escape key events
    //   to NSPanel's responder chain.
    #[unsafe(super = NSPanel)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetBubblePanel"]
    #[ivars = BubblePanelIvars]
    struct BubblePanel;

    unsafe impl NSObjectProtocol for BubblePanel {}

    impl BubblePanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if self.isKeyWindow() && event.keyCode() == 53 {
                if self.composing_editor().is_some() {
                    let _: () = unsafe { msg_send![super(self), keyDown: event] };
                } else {
                    with_ui_mut(|ui| ui.collapse_bubble());
                }
                return;
            }
            let _: () = unsafe { msg_send![super(self), keyDown: event] };
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, sender: Option<&AnyObject>) {
            if self.isKeyWindow() {
                if let Some(editor) = self.composing_editor() {
                    let _: () = unsafe { msg_send![editor, cancelOperation: sender] };
                } else {
                    with_ui_mut(|ui| ui.collapse_bubble());
                }
            } else {
                let _: () = unsafe { msg_send![super(self), cancelOperation: sender] };
            }
        }
    }
);
impl BubblePanel {
    fn composing_editor(&self) -> Option<&AnyObject> {
        let responder: Option<&AnyObject> = unsafe { msg_send![self, firstResponder] };
        responder.filter(|responder| {
            let is_editor: bool =
                unsafe { msg_send![*responder, isKindOfClass: ComposerView::class()] };
            is_editor && unsafe { msg_send![*responder, hasMarkedText] }
        })
    }
}

define_class!(
    // SAFETY:
    // - NSView has no subclassing requirements beyond NSObject lifetime rules.
    // - BubbleView is main-thread-only and does not implement Drop.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetBubbleView"]
    #[ivars = BubbleViewIvars]
    struct BubbleView;
    unsafe impl NSObjectProtocol for BubbleView {}

    impl BubbleView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let path_ref = self.ivars().path.borrow();
            let Some(path) = path_ref.as_ref() else {
                return;
            };
            let palette = self.ivars().palette.get();
            let alpha = if self.ivars().opaque_surface.get() { 1.0 } else { 0.98 };
            bubble_color(palette.surface, alpha).setFill();
            path.fill();
            bubble_color(palette.border, 1.0).setStroke();
            path.setLineWidth(0.75);
            path.stroke();
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<&NSView> {
            let path_ref = self.ivars().path.borrow();
            let Some(path) = path_ref.as_ref() else {
                return None;
            };
            if !path.containsPoint(point) {
                return None;
            }
            if self
                .ivars()
                .native_regions
                .borrow()
                .iter()
                .any(|region| point_in_rect(point, *region))
            {
                // SAFETY: This lets AppKit route events to actual native
                // controls and the SessionCards scroll hierarchy.
                return unsafe { msg_send![super(self), hitTest: point] };
            }
            Some(&**self)
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, true) {
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            let local_point = event.locationInWindow();
            let screen_point = window.convertPointToScreen(local_point);
            let drag = Cell::new(None);
            with_ui_mut(|ui| drag.set(ui.bubble_pointer_down(screen_point, event.timestamp())));
            self.ivars().drag.set(drag.get());
            self.set_drag_visuals(drag.get().is_some());
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) {
                return;
            }
            let Some(window) = self.window() else {
                return;
            };
            let screen_point = window.convertPointToScreen(event.locationInWindow());
            let current = self.ivars().drag.get();
            let updated = Cell::new(None);
            with_ui_mut(|ui| {
                if let Some(drag) = current {
                    updated.set(ui.update_gesture(drag, screen_point));
                }
            });
            self.ivars().drag.set(updated.get());
            self.set_drag_visuals(updated.get().is_some());
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) {
                return;
            }
            if let Some(drag) = self.ivars().drag.get() {
                with_ui_mut(|ui| ui.finish_gesture(drag));
            }
            self.ivars().drag.set(None);
            self.set_drag_visuals(false);
        }

        #[unsafe(method(mouseCancelled:))]
        fn mouse_cancelled(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) {
                return;
            }
            let drag = self.ivars().drag.get();
            with_ui_mut(|ui| ui.cancel_gesture_for(drag));
            self.ivars().drag.set(None);
            self.set_drag_visuals(false);
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuTarget"]
    pub(crate) struct MenuTarget;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(show:))]
        fn show(&self, _sender: Option<&AnyObject>) {
            with_ui_action("show");
        }

        #[unsafe(method(hide:))]
        fn hide(&self, _sender: Option<&AnyObject>) {
            with_ui_action("hide");
        }

        #[unsafe(method(togglePassthrough:))]
        fn toggle_passthrough(&self, _sender: Option<&AnyObject>) {
            with_ui_action("toggle_passthrough");
        }

        #[unsafe(method(toggleStatusIndicators:))]
        fn toggle_status_indicators(&self, sender: Option<&AnyObject>) {
            let Some(toggle) = sender.and_then(|sender| sender.downcast_ref::<NSSwitch>()) else {
                return;
            };
            with_ui_mut(|ui| ui.set_show_status_indicators(toggle.state() == NSControlStateValueOn));
        }
        #[unsafe(method(toggleAlphaPassthrough:))]
        fn toggle_alpha_passthrough(&self, _sender: Option<&AnyObject>) {
            with_ui_action("alpha_passthrough");
        }

        #[unsafe(method(showBubble:))]
        fn show_bubble(&self, _sender: Option<&AnyObject>) {
            with_ui_action("show_bubble");
        }

        #[unsafe(method(hideBubble:))]
        fn hide_bubble(&self, _sender: Option<&AnyObject>) {
            with_ui_action("hide_bubble");
        }
        #[unsafe(method(expandBubble:))]
        fn expand_bubble(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.expand_bubble());
        }

        #[unsafe(method(collapseBubble:))]
        fn collapse_bubble(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.collapse_bubble());
        }

        #[unsafe(method(sendComposer:))]
        fn send_composer(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.submit_composer());
        }

        #[unsafe(method(bubbleAbove:))]
        fn bubble_above(&self, _sender: Option<&AnyObject>) {
            with_ui_action("bubble_above");
        }

        #[unsafe(method(bubbleBelow:))]
        fn bubble_below(&self, _sender: Option<&AnyObject>) {
            with_ui_action("bubble_below");
        }

        #[unsafe(method(bubbleLeft:))]
        fn bubble_left(&self, _sender: Option<&AnyObject>) {
            with_ui_action("bubble_left");
        }

        #[unsafe(method(bubbleRight:))]
        fn bubble_right(&self, _sender: Option<&AnyObject>) {
            with_ui_action("bubble_right");
        }

        #[unsafe(method(bubbleAuto:))]
        fn bubble_auto(&self, _sender: Option<&AnyObject>) {
            with_ui_action("bubble_auto");
        }


        #[unsafe(method(resetPosition:))]
        fn reset_position(&self, _sender: Option<&AnyObject>) {
            with_ui_action("reset_position");
        }

        #[unsafe(method(scaleUp:))]
        fn scale_up(&self, _sender: Option<&AnyObject>) {
            with_ui_action("scale_up");
        }

        #[unsafe(method(scaleDown:))]
        fn scale_down(&self, _sender: Option<&AnyObject>) {
            with_ui_action("scale_down");
        }

        #[unsafe(method(toggleLifecycleAutoStart:))]
        fn toggle_lifecycle_auto_start(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.save_lifecycle_setting(LifecycleSetting::AutoStart));
        }

        #[unsafe(method(toggleLifecycleExit:))]
        fn toggle_lifecycle_exit(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.save_lifecycle_setting(LifecycleSetting::ExitWithHerdr));
        }

        #[unsafe(method(setLanguage:))]
        fn set_language(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let Some(item) = popup.selectedItem() else {
                return;
            };
            let preference = match item.tag() {
                0 => LanguagePreference::System,
                1 => LanguagePreference::Ko,
                2 => LanguagePreference::En,
                _ => return,
            };
            with_ui_mut(|ui| ui.set_language_preference(preference));
        }

        #[unsafe(method(setObservationLocal:))]
        fn set_observation_local(&self, sender: Option<&AnyObject>) {
            let Some(toggle) = sender.and_then(|sender| sender.downcast_ref::<NSSwitch>()) else { return };
            with_ui_mut(|ui| ui.change_observation(|candidate| candidate.local = toggle.state() == NSControlStateValueOn));
        }

        #[unsafe(method(setObservationRemote:))]
        fn set_observation_remote(&self, sender: Option<&AnyObject>) {
            let Some(toggle) = sender.and_then(|sender| sender.downcast_ref::<NSSwitch>()) else { return };
            with_ui_mut(|ui| ui.change_observation(|candidate| candidate.remote = toggle.state() == NSControlStateValueOn));
        }

        #[unsafe(method(setObservationMachine:))]
        fn set_observation_machine(&self, sender: Option<&AnyObject>) {
            let Some(toggle) = sender.and_then(|sender| sender.downcast_ref::<NSSwitch>()) else { return };
            let Some(id) = toggle.identifier().map(|id| id.to_string()) else { return };
            with_ui_mut(|ui| ui.change_observation_machine(id, toggle.state() == NSControlStateValueOn));
        }

        #[unsafe(method(setBubbleTheme:))]
        fn set_bubble_theme(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let Some(item) = popup.selectedItem() else {
                return;
            };
            let theme = match item.tag() {
                0 => BubbleTheme::WarmIvory,
                1 => BubbleTheme::DustyRose,
                2 => BubbleTheme::MoonlitInk,
                3 => BubbleTheme::Custom,
                _ => return,
            };
            with_ui_mut(|ui| ui.set_bubble_theme(theme));
        }

        #[unsafe(method(applyBubbleColors:))]
        fn apply_bubble_colors(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.apply_custom_bubble_colors());
        }

        #[unsafe(method(resetBubbleColors:))]
        fn reset_bubble_colors(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.commit_bubble_appearance(BubbleAppearance::default()));
        }

        #[unsafe(method(toggleMenuPanel:))]
        fn toggle_menu_panel(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.toggle_menu_panel());
        }

        #[unsafe(method(selectMenuTab:))]
        fn select_menu_tab(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else {
                return;
            };
            let Ok(index) = usize::try_from(button.tag()) else {
                return;
            };
            if index >= 3 {
                return;
            }

            with_ui_mut(|ui| ui.select_menu_tab(index));
        }
        #[unsafe(method(setDialogueLocale:))]
        fn set_dialogue_locale(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let locale = match popup.selectedItem().map(|item| item.tag()) {
                Some(0) => UiLocale::Ko,
                Some(1) => UiLocale::En,
                _ => return,
            };
            with_ui_mut(|ui| ui.menu_panel.select_dialogue_locale(locale));
        }

        #[unsafe(method(setDialogueSlot:))]
        fn set_dialogue_slot(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let Some(slot) = popup
                .selectedItem()
                .and_then(|item| usize::try_from(item.tag()).ok())
                .and_then(|index| DialogueSlot::ALL.get(index))
            else {
                return;
            };
            with_ui_mut(|ui| ui.menu_panel.select_dialogue_slot(*slot));
        }

        #[unsafe(method(saveDialogue:))]
        fn save_dialogue(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.save_dialogue_entry(false));
        }

        #[unsafe(method(resetDialogueEntry:))]
        fn reset_dialogue_entry(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.save_dialogue_entry(true));
        }

        #[unsafe(method(resetCharacterDialogue:))]
        fn reset_character_dialogue(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.confirm_reset_character_dialogue());
        }

        #[unsafe(method(status:))]
        fn status(&self, _sender: Option<&AnyObject>) {
            with_ui_action("status");
        }
        #[unsafe(method(focusCharacterManager:))]
        fn focus_character_manager(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.focus_character_manager());
        }

        #[unsafe(method(packImport:))]
        fn pack_import(&self, _sender: Option<&AnyObject>) {
            begin_pack_import();
        }

        #[unsafe(method(packSelect:))]
        fn pack_select(&self, sender: Option<&AnyObject>) {
            if let Some(command) = character_menu::command_from_sender(sender) {
                with_ui_mut(|ui| ui.handle_pack_command(command));
            }
        }

        #[unsafe(method(packUpdate:))]
        fn pack_update(&self, sender: Option<&AnyObject>) {
            if let Some(command) = character_menu::command_from_sender(sender) {
                begin_pack_update(command);
            }
        }

        #[unsafe(method(packRestore:))]
        fn pack_restore(&self, sender: Option<&AnyObject>) {
            if let Some(command) = character_menu::command_from_sender(sender) {
                with_ui_mut(|ui| ui.handle_pack_command(command));
            }
        }

        #[unsafe(method(packRemove:))]
        fn pack_remove(&self, sender: Option<&AnyObject>) {
            if let Some(command) = character_menu::command_from_sender(sender) {
                begin_pack_remove(command);
            }
        }

        #[unsafe(method(packInspect:))]
        fn pack_inspect(&self, sender: Option<&AnyObject>) {
            if let Some(command) = character_menu::command_from_sender(sender) {
                show_pack_inspection(command);
            }
        }

        #[unsafe(method(packDiagnose:))]
        fn pack_diagnose(&self, _sender: Option<&AnyObject>) {
            show_character_diagnostics();
        }


        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.quit());
        }
    }
);

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetWindowDelegate"]
    struct WindowDelegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for WindowDelegate {}

    // SAFETY: NSWindowDelegate has no safety requirements.
    unsafe impl NSWindowDelegate for WindowDelegate {
        #[unsafe(method(windowDidChangeScreen:))]
        fn window_did_change_screen(&self, _notification: &NSNotification) {
            SCREEN_CHANGE_PENDING.with(|pending| pending.set(true));
            with_ui_mut(|ui| ui.handle_screen_change());
        }
    }

    impl WindowDelegate {
        #[unsafe(method(localeDidChange:))]
        fn locale_did_change(&self, _notification: &NSNotification) {
            with_ui_mut(|ui| ui.system_locale_changed());
        }
    }
);

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetAppDelegate"]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for AppDelegate {}

    // SAFETY: NSApplicationDelegate has no safety requirements.
    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let shared = self.ivars().shared.clone();
            let assets = self.ivars().assets.clone();
            let packs = self.ivars().packs.clone();
            let Some(prefs) = self.ivars().prefs.take() else {
                return;
            };
            if let Err(error) = launch_ui(shared.clone(), &assets, packs, prefs, self.mtm()) {
                STARTUP_ERROR.with(|slot| *slot.borrow_mut() = Some(error));
                if let Ok(mut state) = shared.lock() {
                    state.request_shutdown();
                }
                stop_application(self.mtm());
            }
        }
        #[unsafe(method(applicationDidChangeScreenParameters:))]
        fn did_change_screen_parameters(&self, _notification: &NSNotification) {
            SCREEN_CHANGE_PENDING.with(|pending| pending.set(true));
            with_ui_mut(|ui| ui.handle_screen_change());
        }
        #[unsafe(method(applicationDidResignActive:))]
        fn did_resign_active(&self, _notification: &NSNotification) {
            with_ui_mut(|ui| ui.menu_panel.hide());
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            with_ui_mut(|ui| ui.shutdown());
            if let Ok(mut state) = self.ivars().shared.lock() {
                state.request_shutdown();
            }
        }
    }
);
impl GripView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(GripViewIvars);
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl TimerTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init has the expected signature.
        unsafe { msg_send![super(this), init] }
    }
}

impl PetView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let grip = GripView::new(grip_hit_rect(frame.size), mtm);
        let this = Self::alloc(mtm).set_ivars(PetViewIvars {
            drag: Cell::new(None),
            hover_pet: Cell::new(false),
            hover_grip: Cell::new(false),
            grip,
            tracking_area: RefCell::new(None),
        });
        // SAFETY: NSView's initWithFrame: has the expected signature.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect
            | NSTrackingAreaOptions::EnabledDuringMouseDrag;
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                frame,
                options,
                Some(this.as_ref()),
                None,
            )
        };
        this.addTrackingArea(&tracking);
        this.ivars().tracking_area.replace(Some(tracking));
        this.ivars().grip.setHidden(true);
        this
    }

    fn set_gesture_visuals(&self, kind: Option<GestureKind>) {
        if let Some(layer) = self.layer() {
            layer.setOpacity(if kind.is_some() { 0.92 } else { 1.0 });
        }
        let resizing = matches!(kind, Some(GestureKind::Resize));
        self.ivars()
            .grip
            .setHidden(!(resizing || self.ivars().hover_pet.get()));
        let cursor = if resizing || self.ivars().hover_grip.get() {
            if *FRAME_RESIZE_CURSOR_AVAILABLE {
                NSCursor::frameResizeCursorFromPosition_inDirections(
                    NSCursorFrameResizePosition::BottomRight,
                    NSCursorFrameResizeDirections::All,
                )
            } else {
                // The resize gesture also works on macOS 13/14, which lack the
                // system frame-resize cursor introduced in macOS 15.
                NSCursor::crosshairCursor()
            }
        } else if matches!(kind, Some(GestureKind::Move)) {
            NSCursor::closedHandCursor()
        } else {
            NSCursor::arrowCursor()
        };
        cursor.set();
    }

    fn grip(&self) -> &GripView {
        &self.ivars().grip
    }
}
impl BubblePanel {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BubblePanelIvars);
        // SAFETY: NSPanel's designated initializer is inherited by this
        // borderless nonactivating subclass.
        unsafe {
            msg_send![
                super(this),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        }
    }
}

impl BubbleView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BubbleViewIvars {
            drag: Cell::new(None),
            geometry: RefCell::new(None),
            path: RefCell::new(None),
            native_regions: RefCell::new(Vec::new()),
            palette: Cell::new(BubbleAppearance::default().palette()),
            opaque_surface: Cell::new(false),
        });
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn set_opaque_surface(&self, opaque: bool) {
        if self.ivars().opaque_surface.replace(opaque) != opaque {
            self.setNeedsDisplay(true);
        }
    }

    fn set_geometry(&self, geometry: BubbleGeometry) {
        let unchanged = self.ivars().geometry.borrow().as_ref().is_some_and(|old| {
            old.body == geometry.body && old.tail == geometry.tail && old.side == geometry.side
        });
        self.ivars().geometry.replace(Some(geometry));
        if unchanged {
            return;
        }
        let path = bubble_path(geometry.body, geometry.tail);
        self.ivars().path.replace(Some(path));
        self.setNeedsDisplay(true);
    }

    fn set_native_regions(&self, regions: &[NSRect]) {
        let mut current = self.ivars().native_regions.borrow_mut();
        if current.as_slice() != regions {
            current.clear();
            current.extend_from_slice(regions);
        }
    }

    fn contains_local_point(&self, point: NSPoint) -> bool {
        self.ivars()
            .path
            .borrow()
            .as_ref()
            .is_some_and(|path| path.containsPoint(point))
    }

    fn set_drag_visuals(&self, active: bool) {
        if let Some(layer) = self.layer() {
            layer.setOpacity(if active { 0.92 } else { 1.0 });
        }
        if active {
            NSCursor::closedHandCursor().set();
        } else {
            NSCursor::arrowCursor().set();
        }
    }
}

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init has the expected signature.
        unsafe { msg_send![super(this), init] }
    }
}

impl WindowDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init has the expected signature.
        unsafe { msg_send![super(this), init] }
    }
}

impl AppDelegate {
    fn new(
        shared: Arc<Mutex<AppState>>,
        assets: PathBuf,
        packs: Arc<PackService>,
        prefs: Preferences,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars {
            shared,
            assets,
            packs,
            prefs: RefCell::new(Some(prefs)),
        });
        // SAFETY: NSObject's init has the expected signature.
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(
    shared: Arc<Mutex<AppState>>,
    assets: &Path,
    packs: Arc<PackService>,
    prefs: Preferences,
) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "native UI must run on the AppKit main thread".to_string())?;
    STARTUP_ERROR.with(|slot| slot.borrow_mut().take());
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = AppDelegate::new(shared, assets.to_path_buf(), packs, prefs, mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    UI.with(|cell| {
        cell.borrow_mut().take();
    });
    STARTUP_ERROR.with(|slot| slot.borrow_mut().take().map_or(Ok(()), Err))
}
fn stop_application(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    // `-stop:` inside a `runModal` loop ends only that modal session. Abort
    // the modal and retry once its loop has returned to the main run loop.
    if app.modalWindow().is_some() {
        app.abortModal();
        DispatchQueue::main().exec_async(|| {
            if let Some(mtm) = MainThreadMarker::new() {
                stop_application(mtm);
            }
        });
        return;
    }
    app.stop(None);
    if let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    ) {
        app.postEvent_atStart(&event, true);
    }
}

pub fn wake() {
    if WAKE_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    DispatchQueue::main().exec_async(|| {
        WAKE_PENDING.store(false, Ordering::Release);
        with_ui_mut(|ui| {
            if SCREEN_CHANGE_PENDING.with(|pending| pending.replace(false)) {
                ui.handle_screen_change();
            }
            ui.refresh();
        });
    });
}
fn wait_for_main_result(
    receiver: mpsc::Receiver<Result<RendererToken, String>>,
    cancel: &AtomicBool,
) -> Result<RendererToken, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("character operation canceled".to_owned());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // The request may still be queued on the main dispatch queue.
            // Expire it before returning so that a delayed bridge callback
            // cannot apply a character after the worker reported failure.
            cancel.store(true, Ordering::Release);
            return Err("timed out waiting for AppKit".to_owned());
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(25))) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                cancel.store(true, Ordering::Release);
                return Err("AppKit bridge disconnected".to_owned());
            }
        }
    }
}

fn enqueue_bridge(request: DeferredBridge) {
    DispatchQueue::main().exec_async(move || {
        if let Some(request) = run_bridge(request) {
            DEFERRED_BRIDGES.with(|pending| pending.borrow_mut().push_back(request));
        }
    });
}

fn run_bridge(request: DeferredBridge) -> Option<DeferredBridge> {
    let mut request = Some(request);
    let borrowed = UI.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(request) = request.take() else {
            return true;
        };
        match request {
            DeferredBridge::Prepare {
                token,
                assets,
                cancel,
                sender,
            } => {
                if cancel.load(Ordering::Acquire) {
                    let _ = sender.send(Err("character operation canceled".to_owned()));
                } else if let Some(ui) = slot.as_mut() {
                    ui.prepare_native(token, *assets, cancel, sender);
                } else {
                    let _ = sender.send(Err("native UI is unavailable".to_owned()));
                }
            }
            DeferredBridge::Apply {
                token,
                cancel,
                sender,
            } => {
                if cancel.load(Ordering::Acquire) {
                    let _ = sender.send(Err("character operation canceled".to_owned()));
                } else if let Some(ui) = slot.as_mut() {
                    ui.apply_native(token, cancel, sender);
                } else {
                    let _ = sender.send(Err("native UI is unavailable".to_owned()));
                }
            }
            DeferredBridge::Discard { token } => {
                if let Some(ui) = slot.as_mut() {
                    ui.discard_native(&token);
                }
            }
        }
        true
    });
    if borrowed {
        None
    } else {
        request
    }
}

fn drain_deferred_bridges() {
    loop {
        let request = DEFERRED_BRIDGES.with(|pending| pending.borrow_mut().pop_front());
        let Some(request) = request else {
            return;
        };
        if let Some(request) = run_bridge(request) {
            DEFERRED_BRIDGES.with(|pending| pending.borrow_mut().push_front(request));
            return;
        }
    }
}

pub fn prepare_character(
    token: RendererToken,
    assets: ValidatedCharacter,
    cancel: Arc<AtomicBool>,
) -> Result<RendererToken, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    enqueue_bridge(DeferredBridge::Prepare {
        token,
        assets: Box::new(assets),
        cancel: Arc::clone(&cancel),
        sender,
    });
    wait_for_main_result(receiver, &cancel)
}

pub fn apply_character(
    token: RendererToken,
    cancel: Arc<AtomicBool>,
) -> Result<RendererToken, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    enqueue_bridge(DeferredBridge::Apply {
        token,
        cancel: Arc::clone(&cancel),
        sender,
    });
    wait_for_main_result(receiver, &cancel)
}

pub fn discard_character(token: RendererToken) {
    enqueue_bridge(DeferredBridge::Discard { token });
}

fn launch_ui(
    shared: Arc<Mutex<AppState>>,
    assets_path: &Path,
    packs: Arc<PackService>,
    prefs: Preferences,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    shared
        .lock()
        .map_err(|_| "native state lock is poisoned".to_string())?
        .set_preferences(&prefs);
    let selection = choose_startup_selection(&packs, assets_path, mtm)?;
    let reference = selection.reference.clone();
    let override_active = selection.override_active;
    let startup_error = selection.error.clone();
    let ui = Ui::new(
        shared.clone(),
        packs.clone(),
        selection.prepared,
        prefs,
        override_active,
        mtm,
    )?;
    UI.with(|cell| {
        *cell.borrow_mut() = Some(ui);
    });
    packs.set_active(reference, override_active, startup_error);
    {
        let mut state = shared
            .lock()
            .map_err(|_| "native state lock is poisoned".to_string())?;
        state.set_ui_ready();
    }
    println!("HERDR_DESKTOP_PET_UI_READY");
    Ok(())
}

fn choose_startup_selection(
    packs: &PackService,
    legacy_assets: &Path,
    mtm: MainThreadMarker,
) -> Result<StartupSelection, String> {
    let mut errors = Vec::new();
    if let Some(error) = packs.cached_list().error {
        errors.push(error);
    }
    let candidates = packs.startup_candidates().unwrap_or_else(|error| {
        errors.push(format!("managed character startup unavailable: {error}"));
        Vec::new()
    });
    for (reference, path, override_active) in candidates {
        let loaded = if override_active {
            AssetPack::load(&path).map(ValidatedCharacter::Png)
        } else if reference.is_builtin() {
            ValidatedCharacter::load_builtin(&path)
        } else {
            packs.load_startup_managed(&reference)
        };
        match loaded.and_then(|assets| {
            let token = RendererToken::new(
                "startup".to_owned(),
                reference.clone(),
                assets.content_digest().to_owned(),
            )?;
            character_renderer::prepare(assets, token, mtm)
        }) {
            Ok(prepared) => {
                return Ok(StartupSelection {
                    reference,
                    override_active,
                    prepared,
                    error: (!errors.is_empty()).then(|| errors.join("; ")),
                });
            }
            Err(error) => errors.push(format!(
                "character {}@{} failed native validation: {error}",
                reference.id, reference.revision
            )),
        }
    }
    let prepared = ValidatedCharacter::load_builtin(legacy_assets)
        .and_then(|assets| {
            let token = RendererToken::new(
                "startup-fallback".to_owned(),
                CharacterRef::builtin(),
                assets.content_digest().to_owned(),
            )?;
            character_renderer::prepare(assets, token, mtm)
        })
        .map_err(|error| {
            if errors.is_empty() {
                error
            } else {
                format!("{}; builtin fallback failed: {error}", errors.join("; "))
            }
        })?;
    Ok(StartupSelection {
        reference: CharacterRef::builtin(),
        override_active: false,
        prepared,
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    })
}
fn current_ui_locale(preference: LanguagePreference) -> UiLocale {
    let preferred = NSLocale::preferredLanguages();
    let tags = preferred
        .iter()
        .map(|tag| tag.to_string())
        .collect::<Vec<_>>();
    let tags = tags.iter().map(|tag| tag.as_str()).collect::<Vec<_>>();
    resolve_language(preference, &tags)
}

fn active_dialogue_target(
    prepared: &PreparedCharacter,
    packs: &PackService,
    override_active: bool,
) -> Option<DialogueTarget> {
    if override_active {
        let path = packs.override_assets.as_ref()?.canonicalize().ok()?;
        Some(DialogueTarget::ExternalAssets(path.to_str()?.to_owned()))
    } else {
        Some(DialogueTarget::Character(
            prepared.token().reference.id.clone(),
        ))
    }
}

fn dialogue_display_name(
    target: Option<&DialogueTarget>,
    listing: &PackListing,
    prepared: &PreparedCharacter,
    locale: UiLocale,
    previous: Option<&str>,
) -> String {
    match target {
        Some(DialogueTarget::Character(id)) if id == "default" => {
            text(locale, Message::RubeliaBuiltIn).to_owned()
        }
        Some(DialogueTarget::Character(id)) => listing
            .packs
            .iter()
            .find(|pack| {
                pack.id == *id
                    && pack.head == prepared.token().reference.revision
                    && listing
                        .active
                        .as_ref()
                        .is_none_or(|active| active == &prepared.token().reference)
                    && !listing.override_active
            })
            .map(|pack| pack.name.clone())
            .or_else(|| previous.map(str::to_owned))
            .unwrap_or_else(|| id.clone()),
        Some(DialogueTarget::ExternalAssets(path)) => Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_owned(),
        None => String::new(),
    }
}

fn with_ui_action(action: &str) {
    with_ui_mut(|ui| {
        let result = ui.apply_control(action);
        if result.is_err() {
            ui.refresh();
        }
    });
}

// AppKit timestamps identify the captured event sequence.  A delayed release
// from an older sequence must not end a newer press on the same character.
fn pointer_event_allowed(event: &NSEvent, begins: bool) -> bool {
    let timestamp = event.timestamp();
    timestamp.is_finite()
        && UI.with(|cell| {
            cell.try_borrow().ok().is_some_and(|slot| {
                slot.as_ref().is_some_and(|ui| {
                    if begins {
                        ui.pointer_event_started_at.is_none()
                    } else {
                        ui.pointer_event_started_at
                            .is_some_and(|start| timestamp >= start)
                    }
                })
            })
        })
}
fn menu_event_is_inside(event: &NSEvent, ui: &Ui) -> bool {
    let Some(event_window) = event.window(ui.mtm) else {
        // Events without a window are not actionable outside clicks.
        return true;
    };
    let event_window_number = event_window.windowNumber();
    if event_window_number == ui.menu_panel.window().windowNumber() {
        return true;
    }
    if event_window_number == ui.panel.windowNumber()
        || event_window_number == ui.bubble_panel.windowNumber()
    {
        return false;
    }
    let Some(button) = ui._status_item.button(ui.mtm) else {
        return true;
    };
    let Some(status_window) = button.window() else {
        return true;
    };
    if status_window.windowNumber() == event_window_number {
        return point_in_rect(
            event.locationInWindow(),
            button.convertRect_toView(button.bounds(), None),
        );
    }
    // Other windows in this accessory app (for example an NSPopUpButton
    // menu) continue their own tracking. Global monitoring handles clicks
    // delivered to other applications.
    true
}

fn with_ui_mut<F>(f: F)
where
    F: FnOnce(&mut Ui),
{
    let mut deferred = false;
    UI.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => {
            if let Some(ui) = slot.as_mut() {
                f(ui);
            }
        }
        Err(_) => deferred = true,
    });
    drain_deferred_bridges();
    if deferred {
        wake();
    }
}

pub(crate) fn cards_content_changed() {
    with_ui_mut(|ui| {
        ui.composer_render_stamp = None;
        ui.sync_composer();
        if ui.bubble_mode != BubbleMode::Expanded {
            return;
        }
        if ui.bubble_content_tracking_locked() {
            ui.defer_bubble_content();
            return;
        }
        let scene = ui.last_scene.clone();
        if ui.pending_bubble_content {
            ui.bubble_content_dirty = true;
            ui.refresh_bubble_content(&scene);
        } else {
            ui.remeasure_bubble(&scene);
        }
    });
}

fn with_ui_read<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&Ui) -> R,
{
    UI.with(|cell| {
        let slot = cell.try_borrow().ok()?;
        slot.as_ref().map(f)
    })
}

fn pack_menu_context() -> Option<(MainThreadMarker, Arc<PackService>, UiLocale)> {
    with_ui_read(|ui| (ui.mtm, Arc::clone(&ui.packs), ui.locale))
}

fn begin_pack_import() {
    with_ui_mut(|ui| ui.menu_panel.hide());

    let Some((mtm, packs, locale)) = pack_menu_context() else {
        return;
    };
    let generation = packs.cached_list().generation;
    let Some(path) = choose_character_source(mtm, locale, Message::AddCharacterTitle) else {
        return;
    };
    submit_pack_from_menu(packs, generation, PackAction::Import { path });
}

fn begin_pack_update(command: MenuCommand) {
    let MenuCommand::Update { id, generation } = command else {
        return;
    };
    with_ui_mut(|ui| ui.menu_panel.hide());

    let Some((mtm, packs, locale)) = pack_menu_context() else {
        return;
    };
    let Some(path) = choose_character_source(mtm, locale, Message::UpdateCharacterTitle) else {
        return;
    };
    submit_pack_from_menu(packs, generation, PackAction::Update { id, path });
}

fn begin_pack_remove(command: MenuCommand) {
    let MenuCommand::Remove { id, generation } = command else {
        return;
    };
    with_ui_mut(|ui| ui.menu_panel.hide());
    let Some((mtm, packs, locale)) = pack_menu_context() else {
        return;
    };
    if confirm_remove(mtm, locale, &id) {
        submit_pack_from_menu(packs, generation, PackAction::Remove { id });
    }
}

fn show_pack_inspection(command: MenuCommand) {
    let MenuCommand::Inspect { id } = command else {
        return;
    };
    with_ui_mut(|ui| ui.menu_panel.hide());
    let Some((mtm, packs, locale)) = pack_menu_context() else {
        return;
    };
    let listing = packs.cached_list();
    let Some(pack) = listing.packs.iter().find(|p| p.id == id) else {
        return;
    };
    let detail =
        crate::i18n::pack_inspect_details(locale, &pack.name, &pack.id, pack.head, &pack.revisions);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(locale, Message::InspectPackTitle)));
    alert.setInformativeText(&NSString::from_str(&detail));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Close)));
    let _ = alert.runModal();
}

fn show_character_diagnostics() {
    with_ui_mut(|ui| ui.menu_panel.hide());
    let Some((mtm, packs, locale)) = pack_menu_context() else {
        return;
    };
    let listing = packs.cached_list();
    let selected_str = format!("{}@{}", listing.selected.id, listing.selected.revision);
    let active_str = listing
        .active
        .as_ref()
        .map(|a| format!("{}@{}", a.id, a.revision));
    let op_str = with_ui_read(|ui| {
        ui.pack_operation.as_ref().map(|op| {
            let base = crate::i18n::pack_operation(
                locale,
                &op.operation_id,
                &op.state,
                op.error.as_deref(),
            );
            format!(
                "{base} (committed: {}, ui_applied: {})",
                op.committed, op.ui_applied
            )
        })
    })
    .flatten();
    let detail = crate::i18n::character_diagnostics_details(
        locale,
        &selected_str,
        active_str.as_deref(),
        listing.override_active,
        listing.generation,
        listing.error.as_deref(),
        op_str.as_deref(),
    );
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(
        locale,
        Message::DiagnoseCharacter,
    )));
    alert.setInformativeText(&NSString::from_str(&detail));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Close)));
    let _ = alert.runModal();
}

fn next_pack_operation_id() -> String {
    format!(
        "ui-{}-{}",
        std::process::id(),
        NEXT_UI_OPERATION.fetch_add(1, Ordering::Relaxed)
    )
}

fn submit_pack_from_menu(packs: Arc<PackService>, generation: u64, action: PackAction) {
    let request = PackRequest {
        operation_id: next_pack_operation_id(),
        expected_generation: Some(generation),
        action,
    };
    let result = packs.submit(request);
    with_ui_mut(|ui| {
        if Arc::ptr_eq(&ui.packs, &packs) {
            ui.record_pack_submission(result);
        }
    });
}

impl Ui {
    fn new(
        shared: Arc<Mutex<AppState>>,
        packs: Arc<PackService>,
        prepared: PreparedCharacter,
        mut prefs: Preferences,
        dialogue_override_active: bool,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        let lifecycle_paths = Paths::resolve(None, None)?;
        let scene = shared
            .lock()
            .map_err(|_| "native state lock is poisoned".to_string())?
            .scene();
        let locale = current_ui_locale(prefs.language());
        let bubble_appearance = prefs.bubble_appearance();
        let palette = bubble_appearance.palette();
        let initial_frame = match &prepared {
            PreparedCharacter::Png(png) => {
                Some(png.clips.phases[phase_index(scene.phase)].frames[0])
            }
            PreparedCharacter::Rig(_) => None,
        };
        let dialogue_listing = packs.cached_list();
        let dialogue_target = active_dialogue_target(&prepared, &packs, dialogue_override_active);
        let dialogue_name = dialogue_display_name(
            dialogue_target.as_ref(),
            &dialogue_listing,
            &prepared,
            locale,
            None,
        );
        let effective_dialogue = effective_metadata(
            prepared.metadata(),
            dialogue_target
                .as_ref()
                .and_then(|target| prefs.dialogue_overrides().locales(target)),
        );
        prefs.set_scale(scene.scale);
        prefs.set_visible(scene.visible);
        prefs.set_passthrough(scene.passthrough);
        prefs.set_alpha_passthrough(scene.alpha_passthrough);
        prefs.set_bubble_visible(scene.bubble_visible);
        prefs.set_bubble_placement(scene.bubble_placement);

        let display_geometry = DisplayGeometry::new(
            prepared.display_bounds(),
            prepared.canvas_size(),
            matches!(&prepared, PreparedCharacter::Png(_)),
        );
        let size = display_geometry.size(scene.scale);
        let initial_origin = prefs
            .position()
            .map(|(x, y)| {
                let origin = NSPoint::new(x, y);
                if prefs.position_is_legacy() {
                    display_geometry.window_origin(origin, scene.scale)
                } else {
                    origin
                }
            })
            .unwrap_or_else(|| default_origin(size, mtm));
        prefs.mark_display_position();
        prefs.set_position(Some((initial_origin.x, initial_origin.y)));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(initial_origin, size),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        configure_panel(&panel);

        let root = PetView::new(NSRect::new(NSPoint::new(0.0, 0.0), size), mtm);
        root.setWantsLayer(true);
        // The full source-sized child is translated inside the tight window.
        // Never aspect-fit the artwork to the cropped container's bounds.
        panel.setContentView(Some(&root));
        let image_view = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            display_geometry.canvas_frame(scene.scale),
        );
        if let (PreparedCharacter::Png(png), Some(frame)) = (&prepared, initial_frame) {
            image_view.setImage(Some(&png.frames[frame.index()].image));
        }
        image_view.setHidden(matches!(&prepared, PreparedCharacter::Rig(_)));
        image_view.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        image_view.setEditable(false);
        root.addSubview(&image_view);
        if let PreparedCharacter::Rig(rig) = &prepared {
            rig.view().setHidden(true);
            rig.view()
                .setFrame(display_geometry.canvas_frame(scene.scale));
            root.addSubview(rig.view());
        }
        root.addSubview(root.grip());

        let bubble_size = NSSize::new(360.0, 180.0);
        let bubble_panel = BubblePanel::new(NSRect::new(NSPoint::new(0.0, 0.0), bubble_size), mtm);
        configure_panel(&bubble_panel);
        // Borderless nonactivating panels are not automatically exposed as
        // windows; keep the native NSPanel and its responder/click behavior.
        unsafe {
            let _: () = msg_send![&*bubble_panel, setAccessibilityElement: true];
            let _: () = msg_send![
                &*bubble_panel,
                setAccessibilityRole: Some(&*NSString::from_str("AXWindow"))
            ];
            let _: () = msg_send![
                &*bubble_panel,
                setAccessibilitySubrole: Some(&*NSString::from_str("AXStandardWindow"))
            ];
        }
        bubble_panel.setTitle(&NSString::from_str(text(
            locale,
            Message::FullSpeechBubbleMessage,
        )));
        bubble_panel.setHasShadow(false);
        let bubble_root = BubbleView::new(NSRect::new(NSPoint::new(0.0, 0.0), bubble_size), mtm);
        bubble_root.setWantsLayer(true);
        // The drawn bubble is a container, not a second control. AppKit
        // exposes its native text fields, text view and buttons as children.
        unsafe {
            let _: () = msg_send![&*bubble_root, setAccessibilityElement: false];
        }
        let appearance_name = if bubble_surface_is_dark(palette.surface) {
            unsafe { NSAppearanceNameDarkAqua }
        } else {
            unsafe { NSAppearanceNameAqua }
        };
        if let Some(appearance) = NSAppearance::appearanceNamed(appearance_name) {
            let _: () = unsafe { msg_send![&*bubble_panel, setAppearance: Some(&*appearance)] };
            let _: () = unsafe { msg_send![&*bubble_root, setAppearance: Some(&*appearance)] };
        }
        bubble_root.ivars().palette.set(palette);
        bubble_root.set_opaque_surface(prefs.show_status_indicators());
        let menu_target = MenuTarget::new(mtm);
        let character_menu = CharacterMenu::new(&menu_target, locale, mtm);
        let mut cards = SessionCards::new(shared.clone(), locale, mtm);
        cards.set_show_status_indicators(prefs.show_status_indicators());
        cards.set_palette(palette);
        cards.set_frame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)));
        cards.view().setHidden(true);
        bubble_root.addSubview(cards.view());

        let ivory = bubble_color(palette.text, 1.0);
        let muted = bubble_color(palette.muted, 1.0);
        let bubble_text = NSString::from_str("");
        let bubble = NSTextField::wrappingLabelWithString(&bubble_text, mtm);
        bubble.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        bubble.setAlignment(NSTextAlignment::Left);
        bubble.setUsesSingleLineMode(false);
        if let Some(cell) = bubble.cell() {
            cell.setWraps(true);
        }
        bubble.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        bubble.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE)));
        bubble.setMaximumNumberOfLines(4);
        bubble.setTextColor(Some(&ivory));
        bubble.setDrawsBackground(false);
        bubble.setBordered(false);
        bubble.setBezeled(false);
        bubble.setEditable(false);
        bubble.setSelectable(false);
        bubble_root.addSubview(&bubble);
        let status_icon = StatusIcon::new(mtm);
        status_icon.view().setHidden(true);
        bubble_root.addSubview(status_icon.view());
        let status_label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        status_label.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        status_label.setFont(Some(&NSFont::systemFontOfSize_weight(
            BUBBLE_PRIMARY_FONT_SIZE,
            unsafe { objc2_app_kit::NSFontWeightSemibold },
        )));
        status_label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        status_label.setHidden(true);
        bubble_root.addSubview(&status_label);

        let dialogue_text = NSString::from_str("");
        let dialogue = NSTextField::wrappingLabelWithString(&dialogue_text, mtm);
        dialogue.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        dialogue.setAlignment(NSTextAlignment::Left);
        dialogue.setUsesSingleLineMode(false);
        if let Some(cell) = dialogue.cell() {
            cell.setWraps(true);
        }
        dialogue.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        dialogue.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        dialogue.setMaximumNumberOfLines(2);
        dialogue.setTextColor(Some(&muted));
        dialogue.setDrawsBackground(false);
        dialogue.setBordered(false);
        dialogue.setBezeled(false);
        dialogue.setEditable(false);
        dialogue.setSelectable(false);
        dialogue.setHidden(true);
        bubble_root.addSubview(&dialogue);

        let disclosure = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(1.0, BUBBLE_CONTROL_HEIGHT),
            ),
        );
        disclosure.setTitle(&NSString::from_str(text(locale, Message::More)));
        disclosure.setAlignment(NSTextAlignment::Right);
        disclosure.setBordered(false);
        disclosure.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        disclosure.setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        unsafe {
            disclosure.setTarget(Some(menu_target.as_ref()));
            disclosure.setAction(Some(sel!(expandBubble:)));
        }
        set_accessibility_label(&disclosure, text(locale, Message::ExpandSpeechBubble));
        bubble_root.addSubview(&disclosure);

        let collapse = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(BUBBLE_COLLAPSE_WIDTH, BUBBLE_CONTROL_HEIGHT),
            ),
        );
        collapse.setAlignment(NSTextAlignment::Right);
        collapse.setTitle(&NSString::from_str(text(locale, Message::Collapse)));
        collapse.setBordered(false);
        collapse.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        collapse.setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        unsafe {
            collapse.setTarget(Some(menu_target.as_ref()));
            collapse.setAction(Some(sel!(collapseBubble:)));
        }
        set_accessibility_label(&collapse, text(locale, Message::CollapseSpeechBubble));
        collapse.setHidden(true);
        bubble_root.addSubview(&collapse);

        let message_view: Retained<NSTextView> = unsafe {
            msg_send![
                NSTextView::alloc(mtm),
                initWithFrame: NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(320.0, BUBBLE_LINE_HEIGHT),
                )
            ]
        };
        message_view.setEditable(false);
        message_view.setSelectable(true);
        message_view.setRichText(false);
        message_view.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE)));
        let message_style = bubble_paragraph_style(2.4, 5.0);
        message_view.setDefaultParagraphStyle(Some(&*message_style));
        message_view.setTextColor(Some(&ivory));
        message_view.setDrawsBackground(false);
        message_view.setHorizontallyResizable(false);
        if let Some(container) = unsafe { message_view.textContainer() } {
            container.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            container.setLineFragmentPadding(0.0);
        }
        message_view.setTextContainerInset(NSSize::new(0.0, 0.0));
        message_view.setVerticallyResizable(true);
        message_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let message_scroll: Retained<NSScrollView> = unsafe {
            msg_send![
                NSScrollView::alloc(mtm),
                initWithFrame: NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(320.0, BUBBLE_LINE_HEIGHT),
                )
            ]
        };
        message_scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        message_scroll.setDocumentView(Some(&*message_view));
        message_scroll.setHasVerticalScroller(true);
        message_scroll.setAutohidesScrollers(true);
        message_scroll.setDrawsBackground(false);
        message_scroll.setHidden(true);
        set_accessibility_label(
            &message_scroll,
            text(locale, Message::FullSpeechBubbleMessage),
        );
        set_accessibility_label(
            &message_view,
            text(locale, Message::FullSpeechBubbleMessage),
        );
        bubble_root.addSubview(&message_scroll);
        let composer_view = ComposerView::new(mtm);
        composer_view.setEditable(true);
        composer_view.setSelectable(true);
        composer_view.setRichText(false);
        composer_view.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE)));
        composer_view.setTextColor(Some(&ivory));
        composer_view.setDrawsBackground(false);
        composer_view.setHorizontallyResizable(false);
        composer_view.setVerticallyResizable(true);
        composer_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        if let Some(container) = unsafe { composer_view.textContainer() } {
            container.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            container.setLineFragmentPadding(4.0);
        }
        composer_view.setTextContainerInset(NSSize::new(3.0, 3.0));
        set_accessibility_identifier(&composer_view, "pet-message-input");
        set_accessibility_label(&composer_view, text(locale, Message::ComposerInput));
        let composer_scroll: Retained<NSScrollView> = unsafe {
            msg_send![NSScrollView::alloc(mtm), initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0), NSSize::new(320.0, COMPOSER_EDITOR_HEIGHT)
            )]
        };
        composer_scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        composer_scroll.setBorderType(NSBorderType::LineBorder);
        composer_scroll.setDocumentView(Some(&*composer_view));
        composer_scroll.setHasVerticalScroller(true);
        composer_scroll.setAutohidesScrollers(true);
        composer_scroll.setDrawsBackground(false);
        composer_scroll.setHidden(true);
        bubble_root.addSubview(&composer_scroll);
        let composer_target = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        composer_target.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        composer_target.setTextColor(Some(&muted));
        composer_target.setUsesSingleLineMode(true);
        composer_target.setMaximumNumberOfLines(1);
        composer_target.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        set_accessibility_identifier(&composer_target, "pet-message-target");
        composer_target.setHidden(true);
        bubble_root.addSubview(&composer_target);
        let composer_status = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
        composer_status.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        composer_status.setTextColor(Some(&muted));
        set_accessibility_identifier(&composer_status, "pet-message-status");
        composer_status.setHidden(true);
        bubble_root.addSubview(&composer_status);
        let composer_send = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(72.0, BUBBLE_CONTROL_HEIGHT),
            ),
        );
        composer_send.setTitle(&NSString::from_str(text(locale, Message::ComposerSend)));
        composer_send.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        composer_send.setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        set_accessibility_identifier(&composer_send, "pet-message-send");
        set_accessibility_label(&composer_send, text(locale, Message::ComposerSend));
        unsafe {
            composer_send.setTarget(Some(menu_target.as_ref()));
            composer_send.setAction(Some(sel!(sendComposer:)));
        }
        composer_send.setHidden(true);
        bubble_root.addSubview(&composer_send);
        bubble_panel.setContentView(Some(&bubble_root));

        let mut menu_panel = MenuPanel::new(&menu_target, character_menu.view(), locale, mtm);
        menu_panel.set_bubble_appearance(bubble_appearance);
        menu_panel.set_show_status_indicators(prefs.show_status_indicators());
        let status_bar = NSStatusBar::systemStatusBar();
        let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            button.setTitle(&NSString::from_str("Herdr"));
            unsafe {
                button.setTarget(Some(menu_target.as_ref()));
                button.setAction(Some(sel!(toggleMenuPanel:)));
            }
            set_accessibility_label(&button, text(locale, Message::MenuPanelTitle));
        }

        let window_delegate = WindowDelegate::new(mtm);
        panel.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));
        bubble_panel.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));
        let reduced_motion =
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();

        let launch_time = Instant::now();
        let mut playback = Playback::default();
        playback.reset_pack(Duration::ZERO, scene.phase);
        let prompt_sender = PromptSender::new(shared.clone());
        let mut ui = Self {
            mtm,
            shared,
            lifecycle_paths,
            packs,
            prefs,
            panel,
            root,
            image_view,
            bubble_panel,
            bubble_root,
            bubble,
            dialogue,
            status_icon,
            status_label,
            disclosure,
            collapse,
            message_scroll,
            message_view,
            composer_scroll,
            composer_view,
            composer_target,
            composer_status,
            composer_send,
            prompt_sender,
            composer_render_stamp: None,
            composer_key: None,
            composer_pending_key: None,
            composer_drafts: VecDeque::new(),
            composer_results: VecDeque::new(),
            cards,
            menu_panel,
            locale,
            pending_language: None,
            effective_dialogue,
            dialogue_target,
            dialogue_name,
            dialogue_pack_generation: dialogue_listing.generation,
            dialogue_override_active,
            dialogue_prepared_epoch: prepared.token().backend_epoch,
            active: prepared,
            display_geometry,
            bubble_appearance,
            cached_anchor: None,
            displayed_frame: initial_frame,
            viewport: PresentationViewport {
                width: BASE_WIDTH * scene.scale,
                height: BASE_HEIGHT * scene.scale,
                backing_scale: 1.0,
                epoch: 1,
            },
            playback,
            pending_native: None,
            character_menu,
            pack_operation: None,
            pack_error: None,
            behavior: Behavior::new(),
            interaction: Interaction::new(),
            launch_time,
            pointer_press: None,
            pointer_event_started_at: None,
            timer: None,
            pointer_timer: None,
            prepare_timer: None,
            prepare_timer_operation: None,
            language_timer: None,
            timer_target: TimerTarget::new(mtm),
            presentation: Presentation {
                offset_x: 0.0,
                offset_y: 0.0,
                scale: 1.0,
                dialogue: None,
                animate: false,
                effect: None,
            },
            status_text: String::new(),
            status_summary: SessionStatusSummary::default(),
            dialogue_text: String::new(),
            disconnect_text: String::new(),
            bubble_mode: BubbleMode::Compact,
            bubble_layout: BubbleLayout::default(),
            bubble_geometry: None,
            bubble_content_dirty: true,
            pending_bubble_placement: None,
            pending_bubble_content: false,
            bubble_layout_dirty: true,
            transition_generation: 0,
            bubble_fade: None,
            reduced_motion,
            _status_item: status_item,
            _menu_target: menu_target,
            _menu_event_monitors: Vec::new(),
            _window_delegate: window_delegate,
            last_reset_position_revision: scene.reset_position_revision,
            last_scene: scene,
            did_present: false,
            force_image: false,
        };
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &*ui._window_delegate,
                sel!(localeDidChange:),
                Some(NSCurrentLocaleDidChangeNotification),
                None,
            );
        }
        ui.install_menu_event_monitors();
        ui.sync_dialogue_panel();

        ui.clamp_panel();
        ui.update_bubble_frame();
        ui.prepare_initial_rig_surface()?;
        ui.refresh();
        Ok(ui)
    }
    fn set_show_status_indicators(&mut self, enabled: bool) {
        if enabled == self.prefs.show_status_indicators() {
            self.menu_panel.set_show_status_indicators(enabled);
            return;
        }
        if let Err(error) = self.prefs.save_show_status_indicators(enabled) {
            self.menu_panel
                .set_show_status_indicators(self.prefs.show_status_indicators());
            self.queue_bubble_appearance_error(Message::StatusIndicatorsSaveFailure, &error);
            return;
        }
        self.menu_panel.set_show_status_indicators(enabled);
        // Change the backing together with the content, after AppKit releases tracking.
        self.bubble_content_dirty = true;
        let scene = self.last_scene.clone();
        self.refresh_bubble_content(&scene);
    }

    fn queue_bubble_appearance_error(&self, title: Message, detail: &str) {
        let detail = detail.to_owned();
        DispatchQueue::main().exec_async(move || {
            let Some((mtm, locale)) = with_ui_read(|ui| (ui.mtm, ui.locale)) else {
                return;
            };
            // Let the menu action and UI borrow finish before entering AppKit's
            // nested modal loop, as with language-save failures.
            with_ui_mut(|ui| ui.menu_panel.hide());
            show_bubble_appearance_error(mtm, locale, title, &detail);
        });
    }

    fn set_bubble_theme(&mut self, theme: BubbleTheme) {
        self.commit_bubble_appearance(BubbleAppearance {
            theme,
            custom: self.bubble_appearance.custom,
        });
    }

    fn apply_custom_bubble_colors(&mut self) {
        match self.menu_panel.custom_bubble_palette() {
            Ok(custom) => self.commit_bubble_appearance(BubbleAppearance {
                theme: BubbleTheme::Custom,
                custom,
            }),
            Err(error) => self.queue_bubble_appearance_error(Message::InvalidBubbleColor, &error),
        }
    }

    fn commit_bubble_appearance(&mut self, appearance: BubbleAppearance) {
        if appearance == self.bubble_appearance {
            self.menu_panel.set_bubble_appearance(appearance);
            return;
        }
        if let Err(error) = self.prefs.save_bubble_appearance(appearance) {
            self.menu_panel
                .set_bubble_appearance(self.bubble_appearance);
            self.queue_bubble_appearance_error(Message::BubbleAppearanceSaveFailure, &error);
            return;
        }
        self.bubble_appearance = appearance;
        let palette = appearance.palette();
        self.bubble_root.ivars().palette.set(palette);
        self.bubble_root.setNeedsDisplay(true);
        let appearance_name = if bubble_surface_is_dark(palette.surface) {
            unsafe { NSAppearanceNameDarkAqua }
        } else {
            unsafe { NSAppearanceNameAqua }
        };
        if let Some(native_appearance) = NSAppearance::appearanceNamed(appearance_name) {
            let _: () =
                unsafe { msg_send![&*self.bubble_panel, setAppearance: Some(&*native_appearance)] };
            let _: () =
                unsafe { msg_send![&*self.bubble_root, setAppearance: Some(&*native_appearance)] };
        }
        self.disclosure
            .setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        self.collapse
            .setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        self.composer_send
            .setContentTintColor(Some(&bubble_color(palette.accent, 1.0)));
        self.composer_view
            .setTextColor(Some(&bubble_color(palette.text, 1.0)));
        self.composer_target
            .setTextColor(Some(&bubble_color(palette.muted, 1.0)));
        self.composer_status
            .setTextColor(Some(&bubble_color(palette.muted, 1.0)));
        let primary = bubble_color(palette.text, 1.0);
        self.message_view.setTextColor(Some(&primary));
        let length = NSString::from_str(&self.bubble_layout.full_message).length();
        if length > 0 {
            self.message_view
                .setTextColor_range(Some(&primary), NSRange::new(0, length));
        }
        self.cards.set_palette(palette);
        self.menu_panel.set_bubble_appearance(appearance);
        let scene = self.last_scene.clone();
        self.remeasure_bubble(&scene);
        // Width may stay unchanged; refresh the compact truncated label too.
        self.layout_bubble_children();
    }

    fn set_language_preference(&mut self, preference: LanguagePreference) {
        let locale = current_ui_locale(preference);
        if self.language_transition_locked() {
            self.pending_language = Some((preference, locale));
            self.queue_language_apply();
            return;
        }
        self.pending_language = None;
        self.apply_language(preference, locale);
    }
    fn system_locale_changed(&mut self) {
        if self.prefs.language() != LanguagePreference::System {
            return;
        }
        if matches!(
            self.pending_language,
            Some((preference, _)) if preference != LanguagePreference::System
        ) {
            return;
        }
        let locale = current_ui_locale(LanguagePreference::System);
        if self.language_transition_locked() {
            self.pending_language = Some((LanguagePreference::System, locale));
            self.queue_language_apply();
            return;
        }
        self.pending_language = None;
        self.apply_language(LanguagePreference::System, locale);
    }

    fn language_transition_locked(&self) -> bool {
        self.explicit_gesture_active()
            || NSEvent::pressedMouseButtons() != 0
            || appkit_event_tracking_active()
    }
    fn pending_ui_updates(&self) -> bool {
        self.pending_language.is_some()
            || self.pending_bubble_content
            || self.pending_bubble_placement.is_some()
    }

    fn defer_bubble_content(&mut self) {
        self.pending_bubble_content = true;
        self.queue_language_apply();
    }

    // The common-mode timer also drains deferred bubble updates after control tracking.
    fn queue_language_apply(&mut self) {
        if self.language_timer.is_some() {
            return;
        }
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 30.0,
                &self.timer_target,
                sel!(languageTick:),
                None,
                true,
            )
        };
        let run_loop = NSRunLoop::mainRunLoop();
        unsafe {
            run_loop.addTimer_forMode(&timer, NSRunLoopCommonModes);
            run_loop.addTimer_forMode(&timer, NSModalPanelRunLoopMode);
            run_loop.addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
        }
        self.language_timer = Some(timer);
    }

    fn stop_language_timer(&mut self) {
        if let Some(timer) = self.language_timer.take() {
            timer.invalidate();
        }
    }

    fn apply_pending_language(&mut self) {
        if self.language_transition_locked() {
            return;
        }
        let Some((preference, locale)) = self.pending_language.take() else {
            return;
        };
        self.apply_language(preference, locale);
    }

    fn apply_language(&mut self, preference: LanguagePreference, locale: UiLocale) {
        if preference != self.prefs.language() {
            if let Err(error) = self.prefs.save_language(preference) {
                self.menu_panel
                    .set_language_preference(self.prefs.language());
                self.queue_language_save_failure(&error);
                return;
            }
        }
        self.locale = locale;
        self.composer_render_stamp = None;
        self.composer_results.clear();
        self.composer_send
            .setTitle(&NSString::from_str(text(locale, Message::ComposerSend)));
        set_accessibility_label(&self.composer_view, text(locale, Message::ComposerInput));
        if let Some(button) = self._status_item.button(self.mtm) {
            set_accessibility_label(&button, text(locale, Message::MenuPanelTitle));
        }
        self.menu_panel.set_locale(locale);
        self.cards.set_locale(locale);
        self.character_menu.set_locale(locale);
        if matches!(&self.dialogue_target, Some(DialogueTarget::Character(id)) if id == "default") {
            self.dialogue_name = text(locale, Message::RubeliaBuiltIn).to_owned();
        }
        self.sync_dialogue_panel();
        self.bubble_content_dirty = true;
        self.refresh_character_menu();
        self.refresh();
    }

    fn queue_language_save_failure(&self, error: &str) {
        let error = error.to_owned();
        DispatchQueue::main().exec_async(move || {
            let Some((mtm, locale)) = with_ui_read(|ui| (ui.mtm, ui.locale)) else {
                return;
            };
            with_ui_mut(|ui| ui.menu_panel.hide());
            show_language_save_failure(mtm, locale, &error);
        });
    }

    fn prepare_native(
        &mut self,
        token: RendererToken,
        assets: ValidatedCharacter,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
    ) {
        if cancel.load(Ordering::Acquire) {
            let _ = sender.send(Err("character operation canceled".to_owned()));
            return;
        }
        if self.pending_native.is_some() {
            let _ = sender.send(Err("native character candidate is unavailable".to_owned()));
            return;
        }
        let builder = match PrepareBuilder::new(assets, token.clone(), self.mtm) {
            Ok(builder) => builder,
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        };
        let timer_operation = token.operation_id.clone();
        self.pending_native = Some(PendingNative::Preparing {
            token,
            builder,
            cancel,
            sender,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        self.start_prepare_timer(&timer_operation);
    }

    fn prepare_tick(&mut self) {
        let pending = match self.pending_native.take() {
            Some(pending) => pending,
            None => {
                self.stop_prepare_timer();
                return;
            }
        };
        let (token, mut builder, cancel, sender, deadline) = match pending {
            PendingNative::Applying {
                token,
                prepared,
                cancel,
                sender,
                deadline,
            } => {
                self.apply_tick(token, prepared, cancel, sender, deadline);
                return;
            }
            PendingNative::Preparing {
                token,
                builder,
                cancel,
                sender,
                deadline,
            } => (token, builder, cancel, sender, deadline),
            pending => {
                self.pending_native = Some(pending);
                self.stop_prepare_timer();
                return;
            }
        };
        if self.prepare_timer_operation.as_deref() != Some(token.operation_id.as_str()) {
            let timer_operation = token.operation_id.clone();
            self.pending_native = Some(PendingNative::Preparing {
                token,
                builder,
                cancel,
                sender,
                deadline,
            });
            self.stop_prepare_timer();
            self.start_prepare_timer(&timer_operation);
            return;
        }

        if cancel.load(Ordering::Acquire) {
            let _ = sender.send(Err("character operation canceled".to_owned()));
            self.stop_prepare_timer();
            return;
        }
        if Instant::now() >= deadline {
            cancel.store(true, Ordering::Release);
            let _ = sender.send(Err("timed out preparing character".to_owned()));
            self.stop_prepare_timer();
            return;
        }

        match builder.step(&cancel) {
            Ok(None) => {
                self.pending_native = Some(PendingNative::Preparing {
                    token,
                    builder,
                    cancel,
                    sender,
                    deadline,
                });
            }
            Ok(Some(prepared)) => {
                if cancel.load(Ordering::Acquire) {
                    let _ = sender.send(Err("character operation canceled".to_owned()));
                    self.stop_prepare_timer();
                    return;
                }
                if prepared.token() != &token {
                    let _ = sender.send(Err("native Ready token mismatch".to_owned()));
                    self.stop_prepare_timer();
                    return;
                }
                let _ = sender.send(Ok(token.clone()));
                self.pending_native = Some(PendingNative::Ready {
                    token,
                    prepared,
                    cancel,
                });
                self.stop_prepare_timer();
            }
            Err(error) => {
                let _ = sender.send(Err(error));
                self.stop_prepare_timer();
            }
        }
    }

    fn start_prepare_timer(&mut self, operation_id: &str) {
        if self.prepare_timer.is_some() {
            return;
        }
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 120.0,
                &self.timer_target,
                sel!(prepareTick:),
                None,
                true,
            )
        };
        // Keep preparation alive in the default, modal-panel, and event-tracking
        // modes.  Each callback still decodes at most one frame.
        let run_loop = NSRunLoop::mainRunLoop();
        unsafe {
            run_loop.addTimer_forMode(&timer, NSRunLoopCommonModes);
            run_loop.addTimer_forMode(&timer, NSModalPanelRunLoopMode);
            run_loop.addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
        }
        self.prepare_timer = Some(timer);
        self.prepare_timer_operation = Some(operation_id.to_owned());
    }

    fn stop_prepare_timer(&mut self) {
        if let Some(timer) = self.prepare_timer.take() {
            timer.invalidate();
        }
        self.prepare_timer_operation = None;
    }

    fn apply_native(
        &mut self,
        token: RendererToken,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
    ) {
        let Some(pending) = self.pending_native.take() else {
            let _ = sender.send(Err("native character candidate is unavailable".to_owned()));
            return;
        };
        let PendingNative::Ready {
            token: expected,
            prepared,
            cancel: pending_cancel,
        } = pending
        else {
            self.pending_native = Some(pending);
            let _ = sender.send(Err("native character candidate is unavailable".to_owned()));
            return;
        };
        if expected != token || prepared.token() != &token {
            self.pending_native = Some(PendingNative::Ready {
                token: expected,
                prepared,
                cancel: pending_cancel,
            });
            let _ = sender.send(Err(
                "native character candidate token does not match".to_owned()
            ));
            return;
        }
        if cancel.load(Ordering::Acquire) || pending_cancel.load(Ordering::Acquire) {
            let _ = sender.send(Err("character operation canceled".to_owned()));
            return;
        }
        if let PreparedCharacter::Rig(rig) = &prepared {
            rig.view().setHidden(true);
            rig.view()
                .setFrame(self.display_geometry.canvas_frame(self.last_scene.scale));
            self.root.addSubview(rig.view());
            self.root.addSubview(self.root.grip());
        }
        let operation_id = token.operation_id.clone();
        self.pending_native = Some(PendingNative::Applying {
            token,
            prepared,
            cancel,
            sender,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        self.start_prepare_timer(&operation_id);
    }

    fn apply_tick(
        &mut self,
        token: RendererToken,
        mut prepared: PreparedCharacter,
        cancel: Arc<AtomicBool>,
        sender: mpsc::SyncSender<Result<RendererToken, String>>,
        deadline: Instant,
    ) {
        let scene = self
            .shared
            .lock()
            .ok()
            .map(|state| state.scene())
            .unwrap_or_else(|| self.last_scene.clone());
        let result = (|| {
            if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                cancel.store(true, Ordering::Release);
                return Err("character application canceled or timed out".to_owned());
            }
            if prepared.token() != &token {
                return Err("native candidate token changed".to_owned());
            }
            let intent = self.presentation_intent(&scene);
            if let PreparedCharacter::Rig(rig) = &mut prepared {
                rig.view()
                    .setFrame(self.display_geometry.canvas_frame(scene.scale));
                rig.set_viewport_epoch(intent.viewport.epoch);
                if !rig.prepare_surface(
                    character_renderer::rig_intent(intent),
                    intent.viewport.width,
                    intent.viewport.height,
                    intent.viewport.backing_scale,
                )? {
                    return Ok(false);
                }
                if cancel.load(Ordering::Acquire) {
                    return Err("character operation canceled".to_owned());
                }
                if rig.activate()? != token {
                    return Err("native Applied token mismatch".to_owned());
                }
            }
            Ok(true)
        })();
        match result {
            Ok(false) => {
                self.pending_native = Some(PendingNative::Applying {
                    token,
                    prepared,
                    cancel,
                    sender,
                    deadline,
                });
            }
            Ok(true) => {
                self.cancel_gesture();
                self.cancel_pointer();
                if let PreparedCharacter::Rig(old) = &mut self.active {
                    old.set_visible(false);
                    old.view().removeFromSuperview();
                }
                self.image_view
                    .setHidden(matches!(&prepared, PreparedCharacter::Rig(_)));
                if matches!(&prepared, PreparedCharacter::Rig(_)) {
                    self.image_view.setImage(None);
                }
                let previous_canvas = self.display_geometry.canvas_frame(scene.scale);
                let previous_window = self.panel.frame();
                let canvas_origin = NSPoint::new(
                    previous_window.origin.x + previous_canvas.origin.x,
                    previous_window.origin.y + previous_canvas.origin.y,
                );
                self.display_geometry = DisplayGeometry::new(
                    prepared.display_bounds(),
                    prepared.canvas_size(),
                    matches!(&prepared, PreparedCharacter::Png(_)),
                );
                self.active = prepared;
                let frame = NSRect::new(
                    self.display_geometry
                        .window_origin(canvas_origin, scene.scale),
                    self.display_geometry.size(scene.scale),
                );
                self.set_content_frame(frame, scene.scale, false);
                self.clamp_panel_for(scene.bubble_placement);
                self.persist_geometry(&scene, self.panel.frame());
                self.cached_anchor = None;
                self.displayed_frame = None;
                let now = self.launch_time.elapsed();
                self.playback.reset_pack(
                    now.saturating_sub(self.behavior.phase_age(now)),
                    scene.phase,
                );
                self.force_image = true;
                self.stop_prepare_timer();
                self.packs.set_active(token.reference.clone(), false, None);
                self.dialogue_override_active = false;
                self.dialogue_prepared_epoch = token.backend_epoch;
                let previous_name = (self.dialogue_target
                    == Some(DialogueTarget::Character(token.reference.id.clone())))
                .then_some(self.dialogue_name.as_str());
                self.dialogue_target = Some(DialogueTarget::Character(token.reference.id.clone()));
                let listing = self.packs.cached_list();
                self.dialogue_name = dialogue_display_name(
                    self.dialogue_target.as_ref(),
                    &listing,
                    &self.active,
                    self.locale,
                    previous_name,
                );
                self.dialogue_pack_generation = listing.generation;
                self.rebuild_effective_dialogue();
                self.sync_dialogue_panel();
                self.refresh();
                let _ = sender.send(Ok(token));
            }
            Err(error) => {
                if let PreparedCharacter::Rig(rig) = &prepared {
                    rig.view().removeFromSuperview();
                }
                self.stop_prepare_timer();
                let _ = sender.send(Err(error));
            }
        }
    }

    fn prepare_initial_rig_surface(&mut self) -> Result<(), String> {
        if !matches!(&self.active, PreparedCharacter::Rig(_)) {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let scene = self.last_scene.clone();
        self.behavior
            .update(self.launch_time.elapsed(), &scene, false, None, false);
        loop {
            if Instant::now() >= deadline {
                return Err("initial native drawable timed out".to_owned());
            }
            let intent = self.presentation_intent(&scene);
            if let PreparedCharacter::Rig(rig) = &mut self.active {
                rig.set_viewport_epoch(intent.viewport.epoch);
                if rig.prepare_surface(
                    character_renderer::rig_intent(intent),
                    intent.viewport.width,
                    intent.viewport.height,
                    intent.viewport.backing_scale,
                )? {
                    let actual = rig.activate()?;
                    if &actual != rig.token() {
                        return Err("initial native Applied token mismatch".to_owned());
                    }
                    rig.set_visible(intent.visible);
                    return Ok(());
                }
            }
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.005));
        }
    }

    fn presentation_intent(&mut self, scene: &Scene) -> PresentationIntent {
        let now = self.launch_time.elapsed();
        let canvas = self.display_geometry.canvas_frame(scene.scale);
        let size = canvas.size;
        let backing_scale = self.panel.backingScaleFactor();
        if self.viewport.width != size.width
            || self.viewport.height != size.height
            || self.viewport.backing_scale != backing_scale
        {
            self.viewport = PresentationViewport {
                width: size.width,
                height: size.height,
                backing_scale,
                epoch: self.viewport.epoch.saturating_add(1),
            };
        }
        let point = self.panel.convertPointFromScreen(NSEvent::mouseLocation());
        let pointer =
            (size.width > 0.0 && size.height > 0.0 && point_in_rect(point, self.root.bounds()))
                .then_some((
                    (point.x - canvas.origin.x) / size.width,
                    1.0 - (point.y - canvas.origin.y) / size.height,
                ));
        let effect = self
            .presentation
            .effect
            .map(|mut effect| {
                effect.elapsed = now.saturating_sub(effect.started);
                effect
            })
            .filter(|effect| effect.elapsed < effect.duration);
        PresentationIntent {
            now,
            phase: scene.phase,
            phase_age: self.behavior.phase_age(now),
            effect,
            pose: Some(self.behavior.pose_snapshot(now, scene)),
            visible: scene.visible && !scene.shutdown,
            pointer,
            viewport: self.viewport,
            frozen: false,
        }
    }

    fn update_rig(&mut self, scene: &Scene) {
        if !matches!(&self.active, PreparedCharacter::Rig(_)) {
            return;
        }
        let intent = self.presentation_intent(scene);
        if let PreparedCharacter::Rig(rig) = &mut self.active {
            rig.view()
                .setFrame(self.display_geometry.canvas_frame(scene.scale));
            rig.set_viewport_epoch(intent.viewport.epoch);
            rig.set_visible(intent.visible);
            let error = rig
                .update(character_renderer::rig_intent(intent))
                .err()
                .or_else(|| rig.error());
            self.packs.set_renderer_error(error);
        }
    }

    fn character_hit(&self, point: NSPoint) -> Option<CharacterHit> {
        let rect = match &self.active {
            PreparedCharacter::Png(_) => self.image_view.frame(),
            PreparedCharacter::Rig(rig) => rig.view().frame(),
        };
        self.active
            .hit(self.displayed_frame, point.x, point.y, nsrect_tuple(rect))
    }

    fn discard_native(&mut self, token: &RendererToken) {
        let Some(pending) = self.pending_native.take() else {
            return;
        };
        match pending {
            PendingNative::Preparing {
                token: pending_token,
                cancel,
                sender,
                ..
            } if &pending_token == token => {
                cancel.store(true, Ordering::Release);
                let _ = sender.send(Err("character operation canceled".to_owned()));
                self.stop_prepare_timer();
            }
            PendingNative::Applying {
                token: pending_token,
                prepared,
                cancel,
                sender,
                ..
            } if &pending_token == token => {
                cancel.store(true, Ordering::Release);
                if let PreparedCharacter::Rig(rig) = &prepared {
                    rig.view().removeFromSuperview();
                }
                let _ = sender.send(Err("character operation canceled".to_owned()));
                self.stop_prepare_timer();
            }
            PendingNative::Ready {
                token: pending_token,
                ..
            } if &pending_token == token => {
                self.stop_prepare_timer();
            }
            other => {
                self.pending_native = Some(other);
            }
        }
    }

    fn cancel_pending_native(&mut self) {
        let Some(pending) = self.pending_native.take() else {
            self.stop_prepare_timer();
            return;
        };
        match pending {
            PendingNative::Preparing { cancel, sender, .. } => {
                cancel.store(true, Ordering::Release);
                let _ = sender.send(Err("native UI is shutting down".to_owned()));
            }
            PendingNative::Applying {
                prepared,
                cancel,
                sender,
                ..
            } => {
                cancel.store(true, Ordering::Release);
                if let PreparedCharacter::Rig(rig) = &prepared {
                    rig.view().removeFromSuperview();
                }
                let _ = sender.send(Err("native UI is shutting down".to_owned()));
            }
            PendingNative::Ready { cancel, .. } => {
                cancel.store(true, Ordering::Release);
            }
        }
        self.stop_prepare_timer();
    }

    fn refresh_character_menu(&mut self) {
        if let Some(operation_id) = self
            .pack_operation
            .as_ref()
            .map(|operation| operation.operation_id.clone())
        {
            if let Some(operation) = self.packs.cached_status(&operation_id) {
                self.pack_operation = Some(operation);
            }
        }
        let mut listing = self.packs.cached_list();
        if listing.generation != self.dialogue_pack_generation {
            self.dialogue_pack_generation = listing.generation;
            let name = dialogue_display_name(
                self.dialogue_target.as_ref(),
                &listing,
                &self.active,
                self.locale,
                Some(&self.dialogue_name),
            );
            if name != self.dialogue_name {
                self.dialogue_name = name;
                self.sync_dialogue_panel();
            }
        }
        if let Some(error) = self.pack_error.as_ref() {
            listing.error = Some(error.clone());
        }
        if !self.last_scene.shutdown {
            let scene = self.last_scene.clone();
            let status = status_text(&scene, self.locale);
            let snapshot = self.shared.lock().ok().map(|state| {
                (
                    state.lifecycle_settings(),
                    state.observation_preferences().clone(),
                    state.observation_catalog().clone(),
                )
            });
            if let Some((lifecycle, observation, catalog)) = snapshot {
                self.menu_panel.sync(
                    &scene,
                    self.prefs.language(),
                    &status,
                    lifecycle,
                    &observation,
                    &catalog,
                );
            }
        }
        self.character_menu.refresh(
            &listing,
            self.pack_operation.as_ref(),
            self._menu_target.as_ref(),
            self.mtm,
        );
    }

    fn handle_pack_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::Select {
                id,
                generation: command_generation,
            } => {
                self.submit_pack(command_generation, PackAction::Select { id });
            }
            MenuCommand::Restore {
                id,
                revision,
                generation: command_generation,
            } => {
                self.submit_pack(command_generation, PackAction::Restore { id, revision });
            }
            // Dialog-backed commands are started by MenuTarget after releasing
            // the UI borrow, so nested AppKit event loops cannot re-enter it.
            MenuCommand::Update { .. }
            | MenuCommand::Remove { .. }
            | MenuCommand::Inspect { .. } => {}
        }
    }

    fn submit_pack(&mut self, generation: u64, action: PackAction) {
        let request = PackRequest {
            operation_id: next_pack_operation_id(),
            expected_generation: Some(generation),
            action,
        };
        self.record_pack_submission(self.packs.submit(request));
    }

    fn record_pack_submission(&mut self, result: Result<PackOperation, String>) {
        match result {
            Ok(operation) => {
                self.pack_operation = Some(operation);
                self.pack_error = None;
            }
            Err(error) => self.pack_error = Some(error),
        }
        self.refresh_character_menu();
    }

    fn apply_control(&mut self, action: &str) -> Result<(), String> {
        let result = self
            .shared
            .lock()
            .map_err(|_| "native state lock is poisoned".to_string())?
            .apply_control(action);
        self.refresh();
        result
    }

    fn remember_composer_draft(&mut self, key: SessionKey, value: String) {
        self.composer_drafts.retain(|(old, _)| old != &key);
        if !value.is_empty() {
            self.composer_drafts.push_back((key, value));
            if self.composer_drafts.len() > COMPOSER_DRAFT_LIMIT {
                self.composer_drafts.pop_front();
            }
        }
    }

    fn composer_text(&self) -> String {
        self.composer_view.string().to_string()
    }

    fn sync_composer(&mut self) {
        // The cards' rendered revision can lag live state (for example, a source
        // disconnect immediately before a selection callback). Reconcile it
        // before deciding whether the composer can reuse its rendered labels.
        let revision = self
            .shared
            .lock()
            .ok()
            .map(|state| state.session_revision());
        if self.cards.selection_stamp().0 != revision {
            self.cards.refresh();
        }
        let stamp = ComposerRenderStamp {
            cards: self.cards.selection_stamp(),
            locale: self.locale,
            pending: self.prompt_sender.is_pending(),
            live_revision: revision,
        };
        if self.composer_render_stamp == Some(stamp) {
            return;
        }
        self.composer_render_stamp = Some(stamp);
        let selected = self.cards.selected_target();
        let next_key = selected.as_ref().map(|(key, _)| key.clone());
        if self.composer_key != next_key {
            if let Some(old) = self.composer_key.take() {
                let value = self.composer_text();
                self.remember_composer_draft(old, value);
            }
            let draft = next_key
                .as_ref()
                .and_then(|key| {
                    self.composer_drafts
                        .iter()
                        .find(|(old, _)| old == key)
                        .map(|(_, value)| value.as_str())
                })
                .unwrap_or("");
            self.composer_view.setString(&NSString::from_str(draft));
            self.composer_key = next_key.clone();
        }
        let target = selected
            .as_ref()
            .map(|(_, title)| format!("{} {title}", text(self.locale, Message::ComposerTarget)))
            .unwrap_or_else(|| text(self.locale, Message::ComposerSelectSession).to_owned());
        self.composer_target
            .setStringValue(&NSString::from_str(&target));
        set_accessibility_label(&self.composer_target, &target);
        self.composer_target
            .setToolTip(Some(&NSString::from_str(&target)));
        let available = selected.as_ref().map(|(key, _)| {
            self.shared
                .lock()
                .map_err(|_| PromptError::Offline)
                .and_then(|state| state.prompt_available(key))
        });
        let status = if matches!(available.as_ref(), Some(Err(PromptError::ReadOnly))) {
            text(self.locale, Message::ComposerReadOnly).to_owned()
        } else if self.prompt_sender.is_pending()
            && self.composer_pending_key.as_ref() == next_key.as_ref()
        {
            text(self.locale, Message::ComposerSending).to_owned()
        } else if let Some((key, _)) = selected.as_ref() {
            if let Some(Err(error)) = available.as_ref() {
                self.composer_error_text(error)
            } else if let Some((_, previous)) =
                self.composer_results.iter().find(|(old, _)| old == key)
            {
                previous.clone()
            } else if self.prompt_sender.is_pending() {
                text(self.locale, Message::ComposerBusy).to_owned()
            } else {
                text(self.locale, Message::ComposerShortcut).to_owned()
            }
        } else {
            text(self.locale, Message::ComposerSelectSession).to_owned()
        };
        self.composer_status
            .setStringValue(&NSString::from_str(&status));
        set_accessibility_label(&self.composer_status, &status);
        let send = if self.prompt_sender.is_pending() {
            Message::ComposerSending
        } else {
            Message::ComposerSend
        };
        self.composer_send
            .setTitle(&NSString::from_str(text(self.locale, send)));
        set_accessibility_label(&self.composer_send, text(self.locale, send));
        self.composer_send
            .setEnabled(matches!(available, Some(Ok(()))) && !self.prompt_sender.is_pending());
    }

    fn composer_error_text(&self, error: &PromptError) -> String {
        let message = match error {
            PromptError::Empty => Message::ComposerEmpty,
            PromptError::TooLarge => Message::ComposerTooLarge,
            PromptError::Busy => Message::ComposerBusy,
            PromptError::Offline => Message::ComposerOffline,
            PromptError::ReadOnly => Message::ComposerReadOnly,
            PromptError::StaleTarget => Message::ComposerStaleTarget,
            PromptError::Blocked => Message::ComposerBlocked,
            PromptError::NotReady => Message::ComposerNotReady,
            PromptError::Unsupported => Message::ComposerUnsupported,
            PromptError::UnknownDelivery => Message::ComposerUnknownDelivery,
            PromptError::Other(_) => Message::ComposerFailed,
        };
        let label = text(self.locale, message);
        match error {
            PromptError::Other(detail) => format!("{label}: {detail}"),
            _ => label.to_owned(),
        }
    }

    fn remember_composer_result(&mut self, key: SessionKey, status: String) {
        self.composer_results.retain(|(old, _)| old != &key);
        self.composer_results.push_back((key, status));
        if self.composer_results.len() > COMPOSER_DRAFT_LIMIT {
            self.composer_results.pop_front();
        }
    }

    fn submit_composer(&mut self) {
        // A visibility change can invalidate the retained selection between
        // refresh ticks; reconcile before using its key for submission.
        self.sync_composer();
        let Some(key) = self.composer_key.clone() else {
            return;
        };
        let marked: bool = unsafe { msg_send![&*self.composer_view, hasMarkedText] };
        if marked {
            return;
        }
        let value = self.composer_text();
        self.remember_composer_draft(key.clone(), value.clone());
        match self.prompt_sender.submit(key.clone(), value) {
            Ok(()) => {
                self.composer_results.retain(|(old, _)| old != &key);
                self.composer_pending_key = Some(key);
            }
            Err(error) => {
                let status = self.composer_error_text(&error);
                self.remember_composer_result(key, status);
            }
        }
        self.composer_render_stamp = None;
        self.sync_composer();
    }

    fn poll_composer(&mut self) {
        if let Some(result) = self.prompt_sender.try_result() {
            self.composer_pending_key = None;
            let status = match result.result {
                Ok(()) => {
                    let marked: bool = unsafe { msg_send![&*self.composer_view, hasMarkedText] };
                    let current = !marked
                        && self.composer_key.as_ref() == Some(&result.key)
                        && self.composer_text() == result.text;
                    if current {
                        self.composer_view.setString(&NSString::from_str(""));
                    }
                    if self
                        .composer_drafts
                        .iter()
                        .any(|(key, draft)| key == &result.key && draft == &result.text)
                    {
                        self.composer_drafts.retain(|(key, _)| key != &result.key);
                    }
                    text(self.locale, Message::ComposerSent).to_owned()
                }
                Err(error) => self.composer_error_text(&error),
            };
            self.remember_composer_result(result.key, status);
            self.composer_render_stamp = None;
            self.sync_composer();
        }
    }

    fn refresh(&mut self) {
        self.apply_pending_language();
        let (scene, mut completed, outcomes) = match self.shared.lock() {
            Ok(mut state) => {
                let completed = !state.take_completions().is_empty();
                (state.scene(), completed, state.take_outcomes())
            }
            Err(_) => return,
        };
        if let Some(outcome) = outcomes.iter().max_by_key(|observation| {
            let priority = match observation.outcome {
                crate::agent_outcome::AgentOutcome::Failed => 2,
                crate::agent_outcome::AgentOutcome::Cancelled => 1,
                _ => 0,
            };
            (observation.at_unix_ms, priority)
        }) {
            self.behavior
                .accept_outcome(self.launch_time.elapsed(), &scene, outcome.outcome);
            completed = outcome.outcome == crate::agent_outcome::AgentOutcome::Succeeded;
        }
        let cards_height = self.cards.content_height();
        self.cards.refresh();
        self.poll_composer();
        self.sync_composer();
        self.refresh_event(scene, completed);
        if self.bubble_mode == BubbleMode::Expanded && cards_height != self.cards.content_height() {
            if self.bubble_content_tracking_locked() {
                self.defer_bubble_content();
            } else {
                let scene = self.last_scene.clone();
                if self.pending_bubble_content {
                    self.bubble_content_dirty = true;
                    self.refresh_bubble_content(&scene);
                } else {
                    self.remeasure_bubble(&scene);
                }
            }
        }
        self.refresh_character_menu();
    }

    fn refresh_event(&mut self, scene: Scene, completed: bool) {
        let scale_changed = (scene.scale - self.last_scene.scale).abs() > f64::EPSILON;
        let presentation_changed = !self.did_present
            || scene.visible != self.last_scene.visible
            || scene.passthrough != self.last_scene.passthrough
            || scene.alpha_passthrough != self.last_scene.alpha_passthrough
            || scale_changed;
        let bubble_changed = !self.did_present
            || scene.bubble_visible != self.last_scene.bubble_visible
            || scene.bubble_placement != self.last_scene.bubble_placement;
        let reset_position_changed =
            scene.reset_position_revision != self.last_reset_position_revision;
        let phase_changed = scene.phase != self.last_scene.phase;
        let summary = self.cards.status_summary();
        let status_changed = !self.did_present
            || status_fields_changed(&scene, &self.last_scene)
            || (self.prefs.show_status_indicators() && summary != self.status_summary);
        self.status_summary = summary;

        if presentation_changed
            || bubble_changed
            || reset_position_changed
            || (phase_changed && self.active.metadata().is_none())
        {
            self.cancel_gesture();
            self.cancel_pointer();
        }
        if !scene.visible || scene.passthrough {
            self.set_hover(false, false);
        }
        if presentation_changed || bubble_changed {
            self.prefs.set_visible(scene.visible);
            self.prefs.set_passthrough(scene.passthrough);
            self.prefs.set_alpha_passthrough(scene.alpha_passthrough);
            self.prefs.set_scale(scene.scale);
            self.prefs.set_bubble_visible(scene.bubble_visible);
            self.prefs.set_bubble_placement(scene.bubble_placement);
        }
        if reset_position_changed {
            self.reset_position(scene.bubble_placement);

            self.last_reset_position_revision = scene.reset_position_revision;
        }
        if scale_changed {
            self.resize(scene.scale, scene.bubble_placement);
        }
        if status_changed {
            self.bubble_content_dirty = true;
        }

        if scene.shutdown {
            self.shutdown();
            if scale_changed {
                self.persist_geometry(&scene, self.panel.frame());
            } else if presentation_changed || bubble_changed {
                let _ = self.prefs.save();
            }
            self.hide_bubble_panel();
            self.last_scene = scene;
            self.did_present = true;
            stop_application(self.mtm);
            return;
        }

        if presentation_changed || reset_position_changed || scale_changed {
            if scene.visible {
                self.panel.orderFrontRegardless();
            } else {
                self.panel.orderOut(None);
            }
        }
        if bubble_changed || presentation_changed || reset_position_changed || scale_changed {
            self.update_bubble_frame_for(scene.bubble_placement);
        }
        if scene.visible && scene.bubble_visible {
            self.show_bubble_panel();
        } else {
            self.hide_bubble_panel();
            self.reset_bubble_mode();
        }
        if presentation_changed || reset_position_changed {
            self.persist_geometry(&scene, self.panel.frame());
        } else if bubble_changed {
            let _ = self.prefs.save();
        }
        self.last_scene = scene.clone();
        self.did_present = true;
        self.render(&scene, completed, None);
    }

    fn render(&mut self, scene: &Scene, completed: bool, reaction: Option<Reaction>) {
        let manipulating = self.interaction.is_active()
            || self.root.ivars().drag.get().is_some()
            || self.bubble_root.ivars().drag.get().is_some();
        let now = self.launch_time.elapsed();
        let presentation = self
            .behavior
            .update(now, scene, completed, reaction, manipulating);
        let sample = match &self.active {
            PreparedCharacter::Png(png) => self.playback.sample(
                now,
                scene.phase,
                scene.visible && !scene.shutdown,
                presentation.effect,
                &png.clips,
            ),
            PreparedCharacter::Rig(_) => None,
        };
        let use_procedural_transform = self.active.metadata().is_none()
            && sample
                .as_ref()
                .is_none_or(|sample| sample.use_procedural_transform);
        let playback_needs_tick = sample.as_ref().is_some_and(|sample| sample.needs_tick);
        let rendered = if use_procedural_transform {
            presentation
        } else {
            Presentation {
                offset_x: 0.0,
                offset_y: 0.0,
                scale: 1.0,
                ..presentation
            }
        };
        if let (Some(sample), PreparedCharacter::Png(png)) = (sample, &self.active) {
            let changed = self.force_image || self.displayed_frame != Some(sample.frame);
            self.displayed_frame = Some(sample.frame);
            if changed {
                if let Some(frame) = png.frames.get(sample.frame.index()) {
                    self.image_view.setImage(Some(&frame.image));
                    self.force_image = false;
                }
            }
        }
        let semantic_tick = match &self.active {
            PreparedCharacter::Png(png) if png.metadata.is_none() => presentation.animate,
            _ => presentation.effect.is_some(),
        };
        if scene.visible && !scene.shutdown && (semantic_tick || playback_needs_tick) {
            self.start_timer();
        } else {
            self.stop_timer();
        }
        self.apply_presentation(scene, rendered);
        self.refresh_bubble_content(scene);
        self.update_rig(scene);
        if self.refresh_speech_anchor(scene) {
            self.update_bubble_frame_for(scene.bubble_placement);
        }
        self.advance_bubble_fade();
        self.update_pointer_timer(scene);
        self.update_pointer_policy(scene);
    }

    fn frame_tick(&mut self) {
        self.poll_composer();
        let scene = match self.shared.lock() {
            Ok(state) => state.scene(),
            Err(_) => return,
        };
        if scene.visible != self.last_scene.visible
            || scene.passthrough != self.last_scene.passthrough
            || scene.alpha_passthrough != self.last_scene.alpha_passthrough
            || scene.bubble_visible != self.last_scene.bubble_visible
            || scene.bubble_placement != self.last_scene.bubble_placement
            || (scene.scale - self.last_scene.scale).abs() > f64::EPSILON
            || scene.reset_position_revision != self.last_scene.reset_position_revision
            || scene.phase != self.last_scene.phase
            || status_fields_changed(&scene, &self.last_scene)
        {
            self.refresh();
            return;
        }
        if scene.shutdown {
            self.shutdown();
            stop_application(self.mtm);
            return;
        }
        self.render(&scene, false, None);
    }

    fn start_timer(&mut self) {
        if self.timer.is_some() {
            return;
        }
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 30.0,
                &self.timer_target,
                sel!(tick:),
                None,
                true,
            )
        };
        self.timer = Some(timer);
    }

    fn stop_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }

    fn update_pointer_timer(&mut self, scene: &Scene) {
        let needed = self.bubble_fade.is_some()
            || (scene.visible
                && !scene.shutdown
                && !scene.passthrough
                && (scene.alpha_passthrough
                    || scene.bubble_visible
                    || matches!(&self.active, PreparedCharacter::Rig(_))));
        if needed {
            if self.pointer_timer.is_none() {
                let timer = unsafe {
                    NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                        1.0 / 30.0,
                        &self.timer_target,
                        sel!(pointerTick:),
                        None,
                        true,
                    )
                };
                self.pointer_timer = Some(timer);
            }
        } else {
            if let Some(timer) = self.pointer_timer.take() {
                timer.invalidate();
            }
            self.panel.setIgnoresMouseEvents(
                !scene.visible || scene.passthrough || scene.alpha_passthrough,
            );
            self.bubble_panel.setIgnoresMouseEvents(true);
        }
    }

    fn pointer_tick(&mut self) {
        let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) else {
            return;
        };
        if scene.shutdown
            || scene.visible != self.last_scene.visible
            || scene.passthrough != self.last_scene.passthrough
            || scene.alpha_passthrough != self.last_scene.alpha_passthrough
            || scene.bubble_visible != self.last_scene.bubble_visible
            || scene.bubble_placement != self.last_scene.bubble_placement
        {
            self.refresh();
            return;
        }
        self.update_rig(&scene);
        if self.refresh_speech_anchor(&scene) {
            self.update_bubble_frame_for(scene.bubble_placement);
        }
        self.advance_bubble_fade();
        self.apply_pending_bubble_updates();
        self.update_pointer_policy(&scene);
    }

    fn apply_pending_bubble_updates(&mut self) {
        // A deferred drain must not perform even placement-only layout while
        // AppKit is tracking a control or a pet/bubble gesture is active.
        if self.bubble_content_tracking_locked() {
            return;
        }
        if self.pending_bubble_content {
            let scene = self.last_scene.clone();
            self.bubble_content_dirty = true;
            self.refresh_bubble_content(&scene);
        }
        if let Some(placement) = self.pending_bubble_placement.take() {
            self.apply_bubble_frame_for(placement);
        }
    }

    fn update_pointer_policy(&mut self, scene: &Scene) {
        if scene.shutdown || !scene.visible || scene.passthrough {
            self.panel.setIgnoresMouseEvents(true);
            self.bubble_panel.setIgnoresMouseEvents(true);
            return;
        }
        let fade_active = self.bubble_fade.is_some();
        let gesture_locked = self.interaction.is_active()
            || self.root.ivars().drag.get().is_some()
            || self.bubble_root.ivars().drag.get().is_some();
        if gesture_locked {
            self.panel.setIgnoresMouseEvents(false);
            self.bubble_panel
                .setIgnoresMouseEvents(!scene.bubble_visible || fade_active);
            return;
        }
        // Keep the hit-test target stable for the complete native/AppKit
        // tracking sequence.  In particular, changing ignoresMouseEvents
        // while a popup or scroller is pressed would cancel its tracking.
        if NSEvent::pressedMouseButtons() != 0 || appkit_event_tracking_active() {
            return;
        }
        if !scene.alpha_passthrough {
            self.panel.setIgnoresMouseEvents(false);
            let screen = NSEvent::mouseLocation();
            if matches!(&self.active, PreparedCharacter::Rig(_)) {
                let _ = self.character_hit(self.panel.convertPointFromScreen(screen));
            }
            let bubble_hit = scene.bubble_visible && self.bubble_contains_screen(screen);
            self.bubble_panel
                .setIgnoresMouseEvents(!bubble_hit || fade_active);
            return;
        }
        let screen = NSEvent::mouseLocation();
        let panel_point = self.panel.convertPointFromScreen(screen);
        let pet_hit = grip_hit_test(panel_point, self.root.bounds().size)
            || (NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
                && point_in_rect(panel_point, self.root.bounds()))
            || self
                .character_hit(panel_point)
                .is_some_and(|hit| hit.opaque);
        self.panel.setIgnoresMouseEvents(!pet_hit);

        let bubble_hit = scene.bubble_visible && self.bubble_contains_screen(screen);
        self.bubble_panel
            .setIgnoresMouseEvents(!bubble_hit || fade_active);
    }

    fn apply_presentation(&mut self, scene: &Scene, presentation: Presentation) {
        let frame = self
            .display_geometry
            .presented_frame(scene.scale, presentation);
        let changed = !rect_nearly_equal(self.image_view.frame(), frame);
        if changed {
            self.image_view.setFrame(frame);
        }
        self.presentation = presentation;
    }

    fn suspend_transform(&mut self) {
        // Let Behavior own transient expiry while manipulation holds the
        // identity artwork transform. It may keep the timer alive for an
        // active reaction, but never moves the panel or saved geometry.
        self.render_current(false, None);
    }

    fn input_owner(&self) -> Option<InputOwner> {
        Some(InputOwner {
            backend_epoch: self.active.token().backend_epoch,
            input_epoch: self.active.input_epoch()?,
            viewport_epoch: self.viewport.epoch,
        })
    }

    fn input_owner_is_current(&self, owner: Option<InputOwner>) -> bool {
        owner.is_none_or(|owner| {
            self.input_owner().is_some_and(|current| {
                owner.backend_epoch == current.backend_epoch
                    && owner.input_epoch == current.input_epoch
                    && owner.viewport_epoch == current.viewport_epoch
            })
        })
    }

    fn pointer_down(
        &mut self,
        local_point: NSPoint,
        screen_point: NSPoint,
        start_frame: NSRect,
        option: bool,
        resize: bool,
        event_timestamp: f64,
    ) -> Option<DragState> {
        if self.interaction.is_active()
            || self.root.ivars().drag.get().is_some()
            || self.bubble_root.ivars().drag.get().is_some()
        {
            return None;
        }
        let input_owner = if option || resize {
            None
        } else {
            Some(self.input_owner()?)
        };
        let presentation = self.presentation;
        let mut point = self.artwork_point(local_point);
        if resize || option {
            point = clamp_artwork_point(point);
        }
        let region = if option || resize {
            RegionPolicy::Legacy
        } else {
            let hit = self.character_hit(local_point)?;
            if self.active.metadata().is_some() && !hit.opaque {
                return None;
            }
            hit.region
        };
        if !self.input_owner_is_current(input_owner) {
            return None;
        }
        let action =
            self.interaction
                .begin(point, self.launch_time.elapsed(), option, resize, region);
        if !self.interaction.is_active() {
            return None;
        }
        self.pointer_event_started_at = Some(event_timestamp);
        self.pointer_press = Some(PointerPress {
            start_mouse: screen_point,
            start_frame,
            presentation,
            input_owner,
        });
        self.suspend_transform();
        self.handle_gesture_action(action)
    }

    fn bubble_pointer_down(
        &mut self,
        screen_point: NSPoint,
        event_timestamp: f64,
    ) -> Option<DragState> {
        if self.interaction.is_active()
            || self.root.ivars().drag.get().is_some()
            || self.bubble_root.ivars().drag.get().is_some()
        {
            return None;
        }
        let frame = self.panel.frame();
        let drag = self.begin_gesture(GestureKind::Move, screen_point, frame, None)?;
        self.pointer_event_started_at = Some(event_timestamp);
        self.suspend_transform();
        Some(drag)
    }

    fn pointer_motion(&mut self, local_point: NSPoint, screen_point: NSPoint) -> Option<DragState> {
        if !self.interaction.is_active() {
            return None;
        }
        if self
            .pointer_press
            .is_some_and(|press| !self.input_owner_is_current(press.input_owner))
        {
            self.cancel_pointer();
            return None;
        }
        if self
            .pointer_press
            .is_some_and(|press| press.input_owner.is_some())
            && !self.active.input_ready()
        {
            return None;
        }
        let point = self
            .pointer_press
            .map(|press| self.artwork_point_with(local_point, press.presentation))
            .unwrap_or_else(|| self.artwork_point(local_point));
        let action = self.interaction.motion(point, self.launch_time.elapsed());
        match action {
            GestureAction::BeginMove | GestureAction::BeginResize => {
                let kind = match action {
                    GestureAction::BeginMove => GestureKind::Move,
                    GestureAction::BeginResize => GestureKind::Resize,
                    _ => unreachable!(),
                };
                let press = self.pointer_press?;
                let drag = self.begin_gesture(
                    kind,
                    press.start_mouse,
                    press.start_frame,
                    press.input_owner,
                );
                if let Some(drag) = drag {
                    self.pointer_press = None;
                    self.update_gesture(drag, screen_point)
                } else {
                    self.interaction.cancel();
                    self.pointer_press = None;
                    self.render_current(false, None);
                    None
                }
            }
            GestureAction::Reaction(reaction) => {
                self.render_current(false, Some(reaction));
                None
            }
            GestureAction::None => {
                if !self.interaction.is_active() {
                    self.pointer_press = None;
                    self.render_current(false, None);
                }
                None
            }
        }
    }

    fn pointer_end(&mut self, local_point: NSPoint) {
        self.pointer_event_started_at = None;
        if !self.interaction.is_active() {
            self.pointer_press = None;
            return;
        }
        if self.pointer_press.is_some_and(|press| {
            !self.input_owner_is_current(press.input_owner)
                || (press.input_owner.is_some() && !self.active.input_ready())
        }) {
            self.cancel_pointer();
            return;
        }
        let point = self
            .pointer_press
            .map(|press| self.artwork_point_with(local_point, press.presentation))
            .unwrap_or_else(|| self.artwork_point(local_point));
        let action = self.interaction.end(point, self.launch_time.elapsed());
        self.pointer_press = None;
        match action {
            GestureAction::Reaction(reaction) => self.render_current(false, Some(reaction)),
            GestureAction::BeginMove | GestureAction::BeginResize | GestureAction::None => {
                self.render_current(false, None)
            }
        }
    }

    fn handle_gesture_action(&mut self, action: GestureAction) -> Option<DragState> {
        match action {
            GestureAction::BeginMove => {
                let press = self.pointer_press?;
                let drag = self.begin_gesture(
                    GestureKind::Move,
                    press.start_mouse,
                    press.start_frame,
                    press.input_owner,
                );
                if drag.is_some() {
                    self.pointer_press = None;
                } else {
                    self.interaction.cancel();
                    self.pointer_press = None;
                }
                drag
            }
            GestureAction::BeginResize => {
                let press = self.pointer_press?;
                let drag = self.begin_gesture(
                    GestureKind::Resize,
                    press.start_mouse,
                    press.start_frame,
                    press.input_owner,
                );
                if drag.is_some() {
                    self.pointer_press = None;
                } else {
                    self.interaction.cancel();
                    self.pointer_press = None;
                }
                drag
            }
            GestureAction::Reaction(reaction) => {
                self.render_current(false, Some(reaction));
                None
            }
            GestureAction::None => None,
        }
    }
    fn install_menu_event_monitors(&mut self) {
        let mask =
            NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
        let local: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> =
            RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
                let event_ptr = event.as_ptr();
                let event = unsafe { event.as_ref() };
                let inside = with_ui_read(|ui| {
                    ui.menu_panel.is_visible() && menu_event_is_inside(event, ui)
                })
                .unwrap_or(false);
                if !inside {
                    with_ui_mut(|ui| {
                        if ui.menu_panel.is_visible() {
                            ui.menu_panel.hide();
                        }
                    });
                }
                event_ptr
            });
        if let Some(monitor) =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &*local) }
        {
            self._menu_event_monitors.push(monitor);
        }

        let global: RcBlock<dyn Fn(NonNull<NSEvent>)> = RcBlock::new(|_event: NonNull<NSEvent>| {
            with_ui_mut(|ui| {
                if ui.menu_panel.is_visible() {
                    ui.menu_panel.hide();
                }
            });
        });
        if let Some(monitor) =
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &*global)
        {
            self._menu_event_monitors.push(monitor);
        }
    }

    fn remove_menu_event_monitors(&mut self) {
        for monitor in self._menu_event_monitors.drain(..) {
            unsafe { NSEvent::removeMonitor(&*monitor) };
        }
    }

    fn render_current(&mut self, completed: bool, reaction: Option<Reaction>) {
        let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) else {
            return;
        };
        self.render(&scene, completed, reaction);
    }

    fn cancel_pointer(&mut self) {
        self.pointer_event_started_at = None;
        self.interaction.cancel();
        self.pointer_press = None;
        if self.did_present {
            self.render_current(false, None);
        }
    }

    fn shutdown(&mut self) {
        self.stop_timer();
        self.remove_menu_event_monitors();
        self.stop_prepare_timer();
        self.stop_language_timer();
        self.cancel_pending_native();
        if let Some(timer) = self.pointer_timer.take() {
            timer.invalidate();
        }
        self.interaction.cancel();
        self.pointer_press = None;
        self.root.ivars().drag.set(None);
        self.bubble_root.ivars().drag.set(None);
        self.set_hover(false, false);
        self.panel.setIgnoresMouseEvents(true);
        self.bubble_panel.setIgnoresMouseEvents(true);
        self.menu_panel.shutdown();
    }

    fn artwork_point(&self, local_point: NSPoint) -> Point {
        self.artwork_point_with(local_point, self.presentation)
    }

    fn artwork_point_with(&self, local_point: NSPoint, presentation: Presentation) -> Point {
        let scale = self.last_scene.scale * presentation.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return Point {
                x: f64::NAN,
                y: f64::NAN,
            };
        }
        let origin = self
            .display_geometry
            .presented_frame(self.last_scene.scale, presentation)
            .origin;
        Point {
            x: (local_point.x - origin.x) / scale,
            y: BASE_HEIGHT - (local_point.y - origin.y) / scale,
        }
    }

    fn begin_gesture(
        &mut self,
        kind: GestureKind,
        start_mouse: NSPoint,
        start_frame: NSRect,
        input_owner: Option<InputOwner>,
    ) -> Option<DragState> {
        if self.root.ivars().drag.get().is_some() || self.bubble_root.ivars().drag.get().is_some() {
            return None;
        }
        if !self.input_owner_is_current(input_owner) {
            return None;
        }
        let scene = self.shared.lock().ok()?.scene();
        if scene.shutdown || !scene.visible || scene.passthrough {
            return None;
        }
        let frame = self.panel.frame();
        let size_matches_scale = kind != GestureKind::Resize
            || frame_size_matches_scale(self.display_geometry, frame.size, scene.scale);
        if !presentation_matches_scene(&scene, &self.last_scene)
            || !rect_nearly_equal(frame, start_frame)
            || !size_matches_scale
        {
            self.refresh();
            return None;
        }
        let screen_visible = panel_visible_frame(&self.panel, self.mtm)?;
        let start_top_left = NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height);
        if kind == GestureKind::Resize
            && resize_scale_limits(self.display_geometry, start_top_left, screen_visible).is_none()
        {
            return None;
        }
        Some(DragState {
            kind,
            start_mouse,
            start_frame: frame,
            start_top_left,
            start_scale: scene.scale,
            screen_visible,
            expected_frame: frame,
            expected_scale: scene.scale,
            expected_visible: scene.visible,
            expected_passthrough: scene.passthrough,
            expected_alpha_passthrough: scene.alpha_passthrough,
            expected_bubble_visible: scene.bubble_visible,
            expected_bubble_placement: scene.bubble_placement,
            expected_reset_position_revision: scene.reset_position_revision,
            input_owner,
        })
    }

    fn update_gesture(&mut self, drag: DragState, mouse: NSPoint) -> Option<DragState> {
        let scene = match self
            .shared
            .lock()
            .map(|state| state.scene())
            .map_err(|_| ())
        {
            Ok(scene) => scene,
            Err(_) => {
                self.cancel_gesture();
                return None;
            }
        };
        if scene.shutdown
            || !self.input_owner_is_current(drag.input_owner)
            || !scene_matches_drag(&scene, &drag)
            || !rect_nearly_equal(self.panel.frame(), drag.expected_frame)
        {
            self.cancel_gesture();
            self.refresh();
            return None;
        }
        if drag.input_owner.is_some() && !self.active.input_ready() {
            return Some(drag);
        }

        let mut updated = drag;
        match drag.kind {
            GestureKind::Move => {
                let mut frame = drag.start_frame;
                frame.origin = NSPoint::new(
                    drag.start_frame.origin.x + mouse.x - drag.start_mouse.x,
                    drag.start_frame.origin.y + mouse.y - drag.start_mouse.y,
                );
                self.panel.setFrameOrigin(frame.origin);
                self.clamp_panel();
                updated.expected_frame = self.panel.frame();
            }
            GestureKind::Resize => {
                let Some(current_screen) = panel_visible_frame(&self.panel, self.mtm) else {
                    self.handle_screen_change();
                    return None;
                };
                if !rect_nearly_equal(current_screen, drag.screen_visible) {
                    self.handle_screen_change();
                    return None;
                }
                let Some((min_scale, max_scale)) = resize_scale_limits(
                    self.display_geometry,
                    drag.start_top_left,
                    drag.screen_visible,
                ) else {
                    return Some(drag);
                };
                let requested = resize_scale_for_delta(
                    self.display_geometry,
                    drag.start_scale,
                    drag.start_mouse,
                    mouse,
                );
                let target = requested.clamp(min_scale, max_scale);
                let scene = match self
                    .shared
                    .lock()
                    .map(|mut state| {
                        let scene = state.scene();
                        if scene.shutdown || !scene_matches_drag(&scene, &drag) {
                            return None;
                        }
                        let _ = state.set_scale(target);
                        Some(state.scene())
                    })
                    .map_err(|_| ())
                {
                    Ok(Some(scene)) => scene,
                    Ok(None) => {
                        self.cancel_gesture();
                        self.refresh();
                        return None;
                    }
                    Err(_) => {
                        self.cancel_gesture();
                        return None;
                    }
                };
                let frame = resize_frame(self.display_geometry, drag.start_top_left, scene.scale);
                self.set_content_frame(frame, scene.scale, false);
                self.update_bubble_frame_for(scene.bubble_placement);

                updated.expected_frame = self.panel.frame();
                updated.expected_scale = scene.scale;
                self.last_scene.scale = scene.scale;
            }
        }
        Some(updated)
    }

    fn finish_gesture(&mut self, drag: DragState) {
        self.pointer_event_started_at = None;
        let final_state = self.shared.lock().ok().map(|state| state.scene());
        let final_frame = self.panel.frame();
        let valid = final_state.as_ref().is_some_and(|scene| {
            self.input_owner_is_current(drag.input_owner)
                && scene_matches_drag(scene, &drag)
                && rect_nearly_equal(final_frame, drag.expected_frame)
        });
        self.root.ivars().drag.set(None);
        self.bubble_root.ivars().drag.set(None);
        self.root.set_gesture_visuals(None);
        self.bubble_root.set_drag_visuals(false);
        let Some(scene) = final_state else {
            return;
        };
        if !valid {
            self.cancel_gesture_for(Some(drag));
            self.refresh();
            return;
        }
        let changed = !rect_nearly_equal(final_frame, drag.start_frame)
            || (scene.scale - drag.start_scale).abs() > f64::EPSILON;
        if changed {
            self.persist_geometry(&scene, final_frame);
        }
    }

    fn set_hover(&mut self, pet_hovered: bool, grip_hovered: bool) {
        self.root.ivars().hover_pet.set(pet_hovered);
        self.root.ivars().hover_grip.set(grip_hovered);
        let kind = self.root.ivars().drag.get().map(|drag| drag.kind);
        self.root.set_gesture_visuals(kind);
    }

    fn handle_screen_change(&mut self) {
        SCREEN_CHANGE_PENDING.with(|pending| pending.set(false));
        self.reanchor_menu_panel();
        let drag = self
            .root
            .ivars()
            .drag
            .get()
            .or_else(|| self.bubble_root.ivars().drag.get());
        if drag_survives_screen_change(drag, self.panel.frame()) {
            // The drag itself carried the pet onto another display. Each
            // later sample re-clamps to the new screen and mouse-up persists.
            return;
        }
        self.cancel_gesture();
        self.cancel_pointer();
        self.set_hover(false, false);
        // Clamp against the presented scene only. Unseen shared-state changes,
        // including scale, belong to the refresh that their own wake queues.
        self.clamp_panel();
        let origin = self.panel.frame().origin;
        self.prefs.set_position(Some((origin.x, origin.y)));
        let _ = self.prefs.save();
    }

    fn cancel_gesture(&mut self) {
        let drag = self
            .root
            .ivars()
            .drag
            .get()
            .or_else(|| self.bubble_root.ivars().drag.get());
        self.cancel_gesture_for(drag);
    }
    fn cancel_gesture_for(&mut self, drag: Option<DragState>) {
        let accepted_frame = self.panel.frame();
        let scene = self.shared.lock().ok().map(|state| state.scene());
        self.root.ivars().drag.set(None);
        self.bubble_root.ivars().drag.set(None);
        self.root.set_gesture_visuals(None);
        self.bubble_root.set_drag_visuals(false);
        let Some(drag) = drag else {
            return;
        };
        self.pointer_event_started_at = None;
        let Some(scene) = scene else {
            return;
        };
        let changed = !rect_nearly_equal(drag.expected_frame, drag.start_frame)
            || (drag.expected_scale - drag.start_scale).abs() > f64::EPSILON;
        if changed
            && scene_matches_drag(&scene, &drag)
            && rect_nearly_equal(accepted_frame, drag.expected_frame)
        {
            self.persist_geometry(&scene, drag.expected_frame);
        }
    }

    fn persist_geometry(&mut self, scene: &Scene, frame: NSRect) {
        self.prefs.set_visible(scene.visible);
        self.prefs.set_passthrough(scene.passthrough);
        self.prefs.set_alpha_passthrough(scene.alpha_passthrough);
        self.prefs.set_scale(scene.scale);
        self.prefs.set_bubble_visible(scene.bubble_visible);
        self.prefs.set_bubble_placement(scene.bubble_placement);
        self.prefs
            .set_position(Some((frame.origin.x, frame.origin.y)));
        let _ = self.prefs.save();
    }

    fn resize(&mut self, scale: f64, bubble_placement: BubblePlacement) {
        let frame = self.panel.frame();
        let old_canvas = self.display_geometry.canvas_frame(self.last_scene.scale);
        let old_center = NSPoint::new(
            frame.origin.x + old_canvas.origin.x + old_canvas.size.width * 0.5,
            frame.origin.y + old_canvas.origin.y + old_canvas.size.height * 0.5,
        );
        let canvas_origin = NSPoint::new(
            old_center.x - BASE_WIDTH * scale * 0.5,
            old_center.y - BASE_HEIGHT * scale * 0.5,
        );
        let new_frame = NSRect::new(
            self.display_geometry.window_origin(canvas_origin, scale),
            self.display_geometry.size(scale),
        );
        self.set_content_frame(new_frame, scale, true);
        self.clamp_panel_for(bubble_placement);
    }

    fn set_content_frame(&mut self, frame: NSRect, scale: f64, animate: bool) {
        self.panel.setFrame_display_animate(frame, true, animate);
        let size = self.panel.frame().size;
        self.root
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), size));
        let canvas = self.display_geometry.canvas_frame(scale);
        self.image_view.setFrame(canvas);
        if let PreparedCharacter::Rig(rig) = &self.active {
            rig.view().setFrame(canvas);
        }
        self.root.grip().setFrame(grip_hit_rect(size));
        self.bubble_content_dirty = true;
        self.update_bubble_frame_for(self.last_scene.bubble_placement);
        let dialogue = self.presentation.dialogue;
        self.presentation = Presentation {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            dialogue,
            animate: false,
            effect: self.presentation.effect,
        };
    }
    fn reset_position(&mut self, bubble_placement: BubblePlacement) {
        let size = self.panel.frame().size;
        let origin = default_origin(size, self.mtm);
        self.panel.setFrameOrigin(origin);
        self.clamp_panel_for(bubble_placement);
        let origin = self.panel.frame().origin;
        self.prefs.set_position(Some((origin.x, origin.y)));
    }
    fn clamp_panel(&mut self) {
        self.clamp_panel_for(self.last_scene.bubble_placement);
    }

    fn clamp_panel_for(&mut self, bubble_placement: BubblePlacement) {
        let Some(visible) = panel_visible_frame(&self.panel, self.mtm) else {
            return;
        };
        let frame = self.panel.frame();
        let max_x =
            (visible.origin.x + visible.size.width - frame.size.width).max(visible.origin.x);
        let max_y =
            (visible.origin.y + visible.size.height - frame.size.height).max(visible.origin.y);
        let origin = NSPoint::new(
            frame.origin.x.clamp(visible.origin.x, max_x),
            frame.origin.y.clamp(visible.origin.y, max_y),
        );
        if origin.x != frame.origin.x || origin.y != frame.origin.y {
            self.panel.setFrameOrigin(origin);
        }
        self.update_bubble_frame_for(bubble_placement);
    }
    fn update_bubble_frame(&mut self) {
        self.update_bubble_frame_for(self.last_scene.bubble_placement);
    }

    fn update_bubble_frame_for(&mut self, bubble_placement: BubblePlacement) {
        if self.bubble_placement_tracking_locked() {
            self.pending_bubble_placement = Some(bubble_placement);
            self.queue_language_apply();
            return;
        }
        self.pending_bubble_placement = None;
        self.apply_bubble_frame_for(bubble_placement);
    }

    fn refresh_speech_anchor(&mut self, scene: &Scene) -> bool {
        let panel = self.panel.frame();
        let artwork = if matches!(&self.active, PreparedCharacter::Png(_)) {
            self.image_view.frame()
        } else {
            self.display_geometry.canvas_frame(scene.scale)
        };
        let key = AnchorKey {
            frame: self.displayed_frame,
            artwork,
            backend_epoch: self.active.token().backend_epoch,
            input_epoch: self.active.input_epoch(),
            anchor_epoch: self.active.speech_anchor_epoch(),
            ready: self.active.input_ready(),
            phase: scene.phase,
            effect: self.presentation.effect.map(|effect| effect.kind),
        };
        if self.cached_anchor.as_ref().is_some_and(|cached| {
            cached.key.frame == key.frame
                && rect_nearly_equal(cached.key.artwork, key.artwork)
                && cached.key.backend_epoch == key.backend_epoch
                && cached.key.input_epoch == key.input_epoch
                && cached.key.anchor_epoch == key.anchor_epoch
                && cached.key.ready == key.ready
                && cached.key.phase == key.phase
                && cached.key.effect == key.effect
        }) {
            return false;
        }
        let screen_artwork = BubbleRect {
            x: panel.origin.x + artwork.origin.x,
            y: panel.origin.y + artwork.origin.y,
            width: artwork.size.width,
            height: artwork.size.height,
        };
        let relative = self
            .active
            .speech_anchor(
                self.displayed_frame,
                (
                    screen_artwork.x,
                    screen_artwork.y,
                    screen_artwork.width,
                    screen_artwork.height,
                ),
            )
            .map(|anchor| BubbleRect {
                x: anchor.x - panel.origin.x,
                y: anchor.y - panel.origin.y,
                width: anchor.width,
                height: anchor.height,
            });
        self.cached_anchor = Some(CachedAnchor { key, relative });
        true
    }

    fn apply_bubble_frame_for(&mut self, bubble_placement: BubblePlacement) {
        let Some(visible) = panel_visible_frame(&self.panel, self.mtm) else {
            return;
        };
        let pet = self.panel.frame();
        let body = self.bubble_layout.body_size;
        let geometry = place_bubble(
            self.cached_anchor
                .as_ref()
                .and_then(|cached| cached.relative)
                .map(|anchor| BubbleRect {
                    x: pet.origin.x + anchor.x,
                    y: pet.origin.y + anchor.y,
                    width: anchor.width,
                    height: anchor.height,
                })
                .unwrap_or(BubbleRect {
                    x: pet.origin.x,
                    y: pet.origin.y,
                    width: pet.size.width,
                    height: pet.size.height,
                }),
            (body.width.max(0.0), body.height.max(0.0)),
            BubbleRect {
                x: visible.origin.x,
                y: visible.origin.y,
                width: visible.size.width,
                height: visible.size.height,
            },
            bubble_placement,
        );
        let frame = NSRect::new(
            NSPoint::new(geometry.window.x, geometry.window.y),
            NSSize::new(geometry.window.width, geometry.window.height),
        );
        if !rect_nearly_equal(self.bubble_panel.frame(), frame) {
            self.bubble_panel
                .setFrame_display_animate(frame, true, false);
        }
        let local_unchanged = self.bubble_geometry.is_some_and(|old| {
            old.body == geometry.body && old.tail == geometry.tail && old.side == geometry.side
        });
        self.bubble_root
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        self.bubble_root.set_geometry(geometry);
        self.bubble_geometry = Some(geometry);
        if !local_unchanged || self.bubble_layout_dirty {
            self.layout_bubble_children();
        }
    }
    fn refresh_bubble_content(&mut self, scene: &Scene) {
        let dialogue = self
            .effective_dialogue
            .dialogue_text(
                scene.phase.as_str(),
                self.presentation.effect.map(|effect| effect.kind.name()),
                self.locale.tag(),
            )
            .or_else(|| {
                self.presentation
                    .dialogue
                    .map(|key| default_dialogue(self.locale, key))
            })
            .unwrap_or("");
        if self.bubble_content_tracking_locked() {
            if self.bubble_content_dirty
                || self.dialogue_text != dialogue
                || self.pending_bubble_content
            {
                self.defer_bubble_content();
            }
            return;
        }
        if self.pending_bubble_content {
            self.bubble_content_dirty = true;
            self.pending_bubble_content = false;
        }
        if !self.bubble_content_dirty && self.dialogue_text == dialogue {
            return;
        }
        let show_status = self.prefs.show_status_indicators();
        self.cards.set_show_status_indicators(show_status);
        self.bubble_root.set_opaque_surface(show_status);
        let status = if show_status {
            status_indicator_summary(self.locale, self.status_summary)
        } else {
            status_text(scene, self.locale)
        };
        let disconnect = disconnected_text(scene, self.locale);
        if !self.bubble_content_dirty
            && self.dialogue_text == dialogue
            && self.status_text == status
            && self.disconnect_text == disconnect
        {
            return;
        }
        let primary = if show_status {
            dialogue.to_owned()
        } else if dialogue.is_empty() {
            status.clone()
        } else {
            dialogue.to_owned()
        };
        let full_message = if disconnect.is_empty() {
            primary.clone()
        } else if primary.is_empty() {
            disconnect.clone()
        } else {
            format!("{primary}\n{disconnect}")
        };
        self.dialogue_text = dialogue.to_owned();
        self.status_text = status;
        self.disconnect_text = disconnect;
        self.bubble_layout.primary = primary;
        self.bubble_layout.full_message = full_message.clone();
        self.bubble_layout.secondary = self.disconnect_text.clone();
        self.message_view
            .setString(&NSString::from_str(&full_message));
        let message_style = bubble_paragraph_style(2.4, 5.0);
        self.message_view
            .setDefaultParagraphStyle(Some(&*message_style));
        let message_font = NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE);
        self.message_view.setFont(Some(&message_font));
        if !full_message.is_empty() {
            self.message_view.setFont_range(
                &message_font,
                NSRange::new(0, NSString::from_str(&full_message).length()),
            );
            let color = bubble_color(self.bubble_appearance.palette().text, 1.0);
            self.message_view.setTextColor_range(
                Some(&color),
                NSRange::new(0, NSString::from_str(&full_message).length()),
            );
        }
        self.dialogue
            .setStringValue(&NSString::from_str(&self.disconnect_text));
        self.bubble_content_dirty = false;
        self.remeasure_bubble(scene);
    }

    fn remeasure_bubble(&mut self, scene: &Scene) {
        let primary = self.bubble_layout.primary.clone();
        let primary_font = NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE);
        let secondary_font = NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE);
        let primary_style = bubble_paragraph_style(2.4, 0.0);
        let secondary_style = bubble_paragraph_style(1.6, 0.0);
        let palette = self.bubble_appearance.palette();
        let primary_color = bubble_color(palette.text, 1.0);
        let secondary_color = bubble_color(palette.muted, 1.0);
        let show_status = self.prefs.show_status_indicators();
        if show_status {
            let surface = palette.surface.rgb();
            self.status_icon.update(self.status_summary.status, surface);
            self.status_label
                .setTextColor(Some(&semantic_color(self.status_summary.status, surface)));
            self.status_label
                .setStringValue(&NSString::from_str(&self.status_text));
        }
        set_attributed_field_text(
            &self.bubble,
            &self.bubble_layout.primary,
            &primary_font,
            &primary_color,
            &primary_style,
        );
        set_attributed_field_text(
            &self.dialogue,
            &self.disconnect_text,
            &secondary_font,
            &secondary_color,
            &secondary_style,
        );

        let natural_primary = measure_attributed_field(&self.bubble, 10_000.0);
        let natural_secondary = if self.disconnect_text.is_empty() {
            0.0
        } else {
            measure_attributed_field(&self.dialogue, 10_000.0)
                .size
                .width
        };
        self.disclosure
            .setTitle(&NSString::from_str(&task_disclosure(
                self.locale,
                scene.sessions,
            )));
        let disclosure_width = self
            .disclosure
            .cell()
            .map(|cell| cell.cellSize().width)
            .unwrap_or(0.0);
        self.disclosure
            .setTitle(&NSString::from_str(text(self.locale, Message::More)));
        let disclosure_width = disclosure_width.max(
            self.disclosure
                .cell()
                .map(|cell| cell.cellSize().width)
                .unwrap_or(0.0),
        );
        let natural_width = natural_primary
            .size
            .width
            .max(natural_secondary)
            .max(disclosure_width)
            .max(if show_status {
                self.status_label
                    .cell()
                    .map(|cell| cell.cellSize().width + STATUS_ICON_SIZE + STATUS_ICON_GAP)
                    .unwrap_or(0.0)
            } else {
                0.0
            });
        let visible = panel_visible_frame(&self.panel, self.mtm).unwrap_or(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(1000.0, 800.0),
        ));
        let visible_width = visible.size.width.max(1.0);
        let visible_height = visible.size.height.max(1.0);
        let body_width_cap = (visible_width - BUBBLE_WINDOW_INSET * 2.0).max(1.0);
        let body_height_cap = (visible_height - BUBBLE_WINDOW_INSET * 2.0).max(1.0);
        let compact_width = (natural_width + BUBBLE_HORIZONTAL_INSET * 2.0)
            .max(BUBBLE_BODY_MIN_WIDTH)
            .min(BUBBLE_COMPACT_MAX_WIDTH)
            .min(body_width_cap)
            .max(1.0);
        // Expanded bubbles default to 320 points and only grow smaller on a
        // constrained screen; they do not consume the whole display width.
        let expanded_width = BUBBLE_EXPANDED_MIN_WIDTH
            .min(BUBBLE_EXPANDED_MAX_WIDTH)
            .min(body_width_cap)
            .max(1.0);
        let compact_content_width = (compact_width - BUBBLE_HORIZONTAL_INSET * 2.0).max(1.0);
        let full_metrics = measure_attributed_field(&self.bubble, compact_content_width);
        self.bubble.setLineBreakMode(if full_metrics.char_wrapping {
            NSLineBreakMode::ByCharWrapping
        } else {
            NSLineBreakMode::ByWordWrapping
        });
        let full_metrics = measure_attributed_field(&self.bubble, compact_content_width);
        let overflow = full_metrics.line_count() > 4;
        let compact_primary = if overflow {
            truncate_to_lines(
                &primary,
                &self.bubble,
                compact_content_width,
                4,
                &primary_font,
                &primary_color,
                &primary_style,
            )
        } else {
            primary.clone()
        };
        set_attributed_field_text(
            &self.bubble,
            &compact_primary,
            &primary_font,
            &primary_color,
            &primary_style,
        );
        let compact_metrics = measure_attributed_field(&self.bubble, compact_content_width);

        let warning_metrics = if self.disconnect_text.is_empty() {
            None
        } else {
            Some(measure_attributed_field(
                &self.dialogue,
                compact_content_width,
            ))
        };
        let warning_height = warning_metrics
            .as_ref()
            .map(|metrics| {
                self.dialogue.setLineBreakMode(if metrics.char_wrapping {
                    NSLineBreakMode::ByCharWrapping
                } else {
                    NSLineBreakMode::ByWordWrapping
                });
                metrics
                    .size
                    .height
                    .max(BUBBLE_SECONDARY_FONT_SIZE + 2.0)
                    .min((body_height_cap - BUBBLE_LINE_HEIGHT).max(BUBBLE_LINE_HEIGHT))
            })
            .unwrap_or(0.0);
        let compact_text_height = if show_status && primary.is_empty() {
            0.0
        } else {
            compact_metrics.size.height.max(BUBBLE_LINE_HEIGHT)
        };
        let compact_height = (BUBBLE_VERTICAL_INSET
            + if show_status {
                STATUS_ROW_HEIGHT
                    + if compact_text_height > 0.0 || warning_height > 0.0 {
                        BUBBLE_CONTENT_GAP
                    } else {
                        0.0
                    }
            } else {
                0.0
            }
            + compact_text_height
            + if warning_height > 0.0 {
                (if compact_text_height > 0.0 || !show_status {
                    BUBBLE_CONTENT_GAP
                } else {
                    0.0
                }) + warning_height
            } else {
                0.0
            }
            + BUBBLE_CONTENT_GAP
            + BUBBLE_CONTROL_HEIGHT
            + BUBBLE_VERTICAL_INSET)
            .min(body_height_cap)
            .max(1.0);

        let message_metrics = measure_text_view(
            &self.message_view,
            (expanded_width - BUBBLE_HORIZONTAL_INSET * 2.0).max(1.0),
        );
        self.bubble_layout.message_content_height =
            if show_status && self.bubble_layout.full_message.is_empty() {
                0.0
            } else {
                message_metrics.size.height.max(BUBBLE_LINE_HEIGHT)
            };
        let message_height = self
            .bubble_layout
            .message_content_height
            .min(BUBBLE_MESSAGE_MAX_HEIGHT);
        let cards_height = self.cards.content_height().min(BUBBLE_CARDS_MAX_HEIGHT);
        let spacing = expanded_spacing(
            body_height_cap,
            cards_height,
            show_status,
            message_height > 0.0,
        );
        let expanded_height = (spacing.inset
            + if show_status {
                STATUS_ROW_HEIGHT + BUBBLE_CONTENT_GAP
            } else {
                0.0
            }
            + message_height
            + cards_height
            + COMPOSER_TARGET_HEIGHT
            + COMPOSER_EDITOR_HEIGHT
            + COMPOSER_STATUS_HEIGHT
            + spacing.gap
                * (if show_status && message_height == 0.0 {
                    4.0
                } else {
                    5.0
                })
            + BUBBLE_CONTROL_HEIGHT
            + spacing.inset)
            .min(
                (visible_height * 0.78).min(530.0).max(
                    expanded_height_budget(
                        cards_height,
                        show_status,
                        message_height > 0.0,
                        spacing,
                    )
                    .min(body_height_cap),
                ),
            )
            .min(body_height_cap)
            .max(1.0);

        self.bubble_layout.compact_primary = compact_primary;
        self.bubble_layout.overflow = overflow;
        let disclosure_title = if overflow {
            text(self.locale, Message::More).to_owned()
        } else {
            task_disclosure(self.locale, scene.sessions)
        };
        let collapse_title = text(self.locale, Message::Collapse);
        self.disclosure
            .setTitle(&NSString::from_str(&disclosure_title));
        self.collapse.setTitle(&NSString::from_str(collapse_title));
        let expand_accessibility = text(self.locale, Message::ExpandSpeechBubble);
        let collapse_accessibility = text(self.locale, Message::CollapseSpeechBubble);
        let message_accessibility = text(self.locale, Message::FullSpeechBubbleMessage);
        set_accessibility_label(&self.disclosure, expand_accessibility);
        set_accessibility_label(&self.collapse, collapse_accessibility);
        set_accessibility_label(&self.message_scroll, message_accessibility);
        self.bubble_panel
            .setTitle(&NSString::from_str(message_accessibility));
        set_accessibility_label(&self.message_view, message_accessibility);
        if let Some(cell) = self.bubble.cell() {
            set_accessibility_cell_text(&cell, &primary, Some(&primary));
        }
        if let Some(cell) = self.disclosure.cell() {
            set_accessibility_cell_text(&cell, expand_accessibility, Some(&disclosure_title));
        }
        if let Some(cell) = self.collapse.cell() {
            set_accessibility_cell_text(&cell, collapse_accessibility, Some(collapse_title));
        }
        self.bubble_layout.body_size = match self.bubble_mode {
            BubbleMode::Compact => NSSize::new(compact_width, compact_height),
            BubbleMode::Expanded => NSSize::new(expanded_width, expanded_height),
        };
        self.bubble_layout_dirty = true;
        self.update_bubble_frame_for(scene.bubble_placement);
    }

    fn layout_bubble_children(&mut self) {
        let Some(geometry) = self.bubble_geometry else {
            return;
        };
        self.bubble_layout_dirty = false;
        let body = geometry.body;
        let body_frame = NSRect::new(
            NSPoint::new(body.x, body.y),
            NSSize::new(body.width.max(0.0), body.height.max(0.0)),
        );
        let content_x = body.x + BUBBLE_HORIZONTAL_INSET;
        let content_width = (body.width - BUBBLE_HORIZONTAL_INSET * 2.0).max(0.0);
        let show_status = self.prefs.show_status_indicators();
        let spacing = if self.bubble_mode == BubbleMode::Expanded {
            expanded_spacing(
                body.height,
                self.cards.content_height().min(BUBBLE_CARDS_MAX_HEIGHT),
                show_status,
                self.bubble_layout.message_content_height > 0.0,
            )
        } else {
            ExpandedSpacing {
                inset: BUBBLE_VERTICAL_INSET,
                gap: BUBBLE_CONTENT_GAP,
            }
        };
        let content_top = body.y + body.height - spacing.inset;
        let status_row = if show_status {
            let frame = bounded_frame(
                NSRect::new(
                    NSPoint::new(content_x, content_top - STATUS_ROW_HEIGHT),
                    NSSize::new(content_width, STATUS_ROW_HEIGHT),
                ),
                NSRect::new(
                    NSPoint::new(
                        body.x,
                        body.y + spacing.inset + BUBBLE_CONTROL_HEIGHT + spacing.gap,
                    ),
                    NSSize::new(
                        body.width.max(0.0),
                        (content_top
                            - body.y
                            - spacing.inset
                            - BUBBLE_CONTROL_HEIGHT
                            - spacing.gap)
                            .max(0.0),
                    ),
                ),
            );
            let icon_width = STATUS_ICON_SIZE.min(frame.size.width);
            let icon_height = STATUS_ICON_SIZE.min(frame.size.height);
            self.status_icon.view().setFrame(NSRect::new(
                NSPoint::new(
                    frame.origin.x,
                    frame.origin.y + (frame.size.height - icon_height) / 2.0,
                ),
                NSSize::new(icon_width, icon_height),
            ));
            self.status_label.setFrame(NSRect::new(
                NSPoint::new(
                    frame.origin.x + icon_width + STATUS_ICON_GAP,
                    frame.origin.y,
                ),
                NSSize::new(
                    (frame.size.width - icon_width - STATUS_ICON_GAP).max(0.0),
                    frame.size.height,
                ),
            ));
            let visible = icon_width > 0.0
                && icon_height > 0.0
                && frame.size.width > icon_width + STATUS_ICON_GAP;
            self.status_icon.view().setHidden(!visible);
            self.status_label.setHidden(!visible);
            frame
        } else {
            self.status_icon.view().setHidden(true);
            self.status_label.setHidden(true);
            let zero = NSRect::new(NSPoint::new(body.x, body.y), NSSize::new(0.0, 0.0));
            self.status_icon.view().setFrame(zero);
            self.status_label.setFrame(zero);
            zero
        };
        let mut regions = Vec::new();
        match self.bubble_mode {
            BubbleMode::Compact => {
                self.bubble.setHidden(false);
                self.collapse.setHidden(true);
                self.message_scroll.setHidden(true);
                self.cards.view().setHidden(true);
                self.composer_scroll.setHidden(true);
                self.composer_target.setHidden(true);
                self.composer_status.setHidden(true);
                self.composer_send.setHidden(true);
                let footer_width = self
                    .disclosure
                    .cell()
                    .map(|cell| cell.cellSize().width)
                    .unwrap_or(content_width)
                    .min(content_width);
                let footer = bounded_frame(
                    NSRect::new(
                        NSPoint::new(
                            content_x + (content_width - footer_width).max(0.0),
                            body.y + BUBBLE_VERTICAL_INSET,
                        ),
                        NSSize::new(footer_width, BUBBLE_CONTROL_HEIGHT),
                    ),
                    body_frame,
                );
                let primary = if self.bubble_layout.overflow {
                    self.bubble_layout.compact_primary.clone()
                } else {
                    self.bubble_layout.primary.clone()
                };
                let primary_font = NSFont::systemFontOfSize(BUBBLE_PRIMARY_FONT_SIZE);
                let primary_style = bubble_paragraph_style(2.4, 0.0);
                let primary_color = bubble_color(self.bubble_appearance.palette().text, 1.0);
                set_attributed_field_text(
                    &self.bubble,
                    &primary,
                    &primary_font,
                    &primary_color,
                    &primary_style,
                );
                let primary_metrics =
                    measure_attributed_field(&self.bubble, content_width.max(1.0));
                let primary_height = if show_status && primary.is_empty() {
                    0.0
                } else {
                    primary_metrics.size.height.max(BUBBLE_LINE_HEIGHT)
                };
                let warning_height = if self.disconnect_text.is_empty() {
                    0.0
                } else {
                    measure_attributed_field(&self.dialogue, content_width.max(1.0))
                        .size
                        .height
                        .max(BUBBLE_SECONDARY_FONT_SIZE + 2.0)
                };
                let available_top = if show_status {
                    (status_row.origin.y - BUBBLE_CONTENT_GAP).max(body.y)
                } else {
                    content_top.max(body.y)
                };
                let available_bottom =
                    (footer.origin.y + footer.size.height + BUBBLE_CONTENT_GAP).min(available_top);
                let warning_frame = if warning_height > 0.0 {
                    let desired = NSRect::new(
                        NSPoint::new(
                            content_x,
                            available_top - primary_height - BUBBLE_CONTENT_GAP - warning_height,
                        ),
                        NSSize::new(content_width, warning_height),
                    );
                    bounded_frame(
                        desired,
                        NSRect::new(
                            NSPoint::new(body.x, available_bottom),
                            NSSize::new(
                                body.width.max(0.0),
                                (available_top - available_bottom).max(0.0),
                            ),
                        ),
                    )
                } else {
                    NSRect::new(NSPoint::new(body.x, body.y), NSSize::new(0.0, 0.0))
                };
                let primary_bottom = if warning_height > 0.0 {
                    warning_frame.origin.y - BUBBLE_CONTENT_GAP
                } else {
                    available_top
                };
                let primary_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, primary_bottom - primary_height),
                        NSSize::new(content_width, primary_height),
                    ),
                    NSRect::new(
                        NSPoint::new(body.x, available_bottom),
                        NSSize::new(
                            body.width.max(0.0),
                            (available_top - available_bottom).max(0.0),
                        ),
                    ),
                );
                let (primary_frame, warning_frame) = if show_status {
                    let bounds = NSRect::new(
                        NSPoint::new(body.x, available_bottom),
                        NSSize::new(
                            body.width.max(0.0),
                            (available_top - available_bottom).max(0.0),
                        ),
                    );
                    let primary_frame = bounded_frame(
                        NSRect::new(
                            NSPoint::new(content_x, available_top - primary_height),
                            NSSize::new(content_width, primary_height),
                        ),
                        bounds,
                    );
                    let warning_top = if primary_height > 0.0 {
                        primary_frame.origin.y - BUBBLE_CONTENT_GAP
                    } else {
                        available_top
                    };
                    let warning_frame = bounded_frame(
                        NSRect::new(
                            NSPoint::new(content_x, warning_top - warning_height),
                            NSSize::new(content_width, warning_height),
                        ),
                        NSRect::new(
                            NSPoint::new(body.x, available_bottom),
                            NSSize::new(
                                body.width.max(0.0),
                                (warning_top - available_bottom).max(0.0),
                            ),
                        ),
                    );
                    (primary_frame, warning_frame)
                } else {
                    (primary_frame, warning_frame)
                };
                self.bubble.setHidden(
                    (show_status && primary.is_empty()) || primary_frame.size.height <= 0.0,
                );
                self.bubble.setFrame(primary_frame);
                self.dialogue
                    .setHidden(self.disconnect_text.is_empty() || warning_frame.size.height <= 0.0);
                self.dialogue.setFrame(warning_frame);
                self.disclosure
                    .setHidden(footer.size.width <= 0.0 || footer.size.height <= 0.0);
                self.disclosure.setFrame(footer);
                if footer.size.width > 0.0 && footer.size.height > 0.0 {
                    regions.push(footer);
                }
            }
            BubbleMode::Expanded => {
                self.bubble.setHidden(true);
                self.dialogue.setHidden(true);
                self.disclosure.setHidden(true);
                let collapse_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(
                            body.x + body.width - BUBBLE_HORIZONTAL_INSET - BUBBLE_COLLAPSE_WIDTH,
                            body.y + spacing.inset,
                        ),
                        NSSize::new(BUBBLE_COLLAPSE_WIDTH, BUBBLE_CONTROL_HEIGHT),
                    ),
                    body_frame,
                );
                let mut bottom = collapse_frame.origin.y + collapse_frame.size.height + spacing.gap;
                let message_top = expanded_message_top(
                    status_row.origin.y,
                    content_top,
                    show_status,
                    self.bubble_layout.message_content_height > 0.0,
                );
                let desired_message = self
                    .bubble_layout
                    .message_content_height
                    .min(BUBBLE_MESSAGE_MAX_HEIGHT);
                let desired_cards = self.cards.content_height().min(BUBBLE_CARDS_MAX_HEIGHT);
                let heights = expanded_heights(
                    body.height,
                    desired_cards,
                    desired_message,
                    show_status,
                    spacing,
                );
                let status_height = heights.status;
                let status_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, bottom),
                        NSSize::new(content_width, status_height),
                    ),
                    body_frame,
                );
                bottom += status_height + spacing.gap;
                let editor_height = heights.editor;
                let editor_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, bottom),
                        NSSize::new(content_width, editor_height),
                    ),
                    body_frame,
                );
                bottom += editor_height + spacing.gap;
                let target_height = heights.target;
                let send_width = 78.0_f64.min(content_width);
                let target_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, bottom),
                        NSSize::new(
                            (content_width - send_width - spacing.gap).max(0.0),
                            target_height,
                        ),
                    ),
                    body_frame,
                );
                let send_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x + content_width - send_width, bottom),
                        NSSize::new(send_width, target_height),
                    ),
                    body_frame,
                );
                bottom += target_height + spacing.gap;
                let cards_height = heights.cards;
                let message_height = heights.message;
                let composer_visible = editor_frame.size.width > 0.0 && editor_height > 0.0;
                self.composer_status.setFrame(status_frame);
                self.composer_status.setHidden(status_height <= 0.0);
                self.composer_scroll.setFrame(editor_frame);
                self.composer_scroll.setHidden(!composer_visible);
                self.composer_target.setFrame(target_frame);
                self.composer_target.setHidden(target_height <= 0.0);
                self.composer_send.setFrame(send_frame);
                self.composer_send.setHidden(target_height <= 0.0);
                if composer_visible {
                    let width = self
                        .composer_scroll
                        .documentVisibleRect()
                        .size
                        .width
                        .max(1.0);
                    let height = measure_text_view(&self.composer_view, width)
                        .size
                        .height
                        .max(editor_height);
                    self.composer_view.setFrame(NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        NSSize::new(width, height),
                    ));
                }
                for frame in [status_frame, editor_frame, target_frame, send_frame] {
                    if frame.size.width > 0.0 && frame.size.height > 0.0 {
                        regions.push(frame);
                    }
                }
                let message_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, message_top - message_height),
                        NSSize::new(content_width, message_height),
                    ),
                    body_frame,
                );
                let cards_frame = bounded_frame(
                    NSRect::new(
                        NSPoint::new(content_x, bottom),
                        NSSize::new(content_width, cards_height),
                    ),
                    body_frame,
                );
                let message_visible =
                    message_frame.size.width > 0.0 && message_frame.size.height > 0.0;
                let cards_visible = cards_frame.size.width > 0.0 && cards_frame.size.height > 0.0;
                self.message_scroll.setHidden(!message_visible);
                self.message_scroll.setFrame(message_frame);
                let clip_width = if message_visible {
                    self.message_scroll
                        .documentVisibleRect()
                        .size
                        .width
                        .max(1.0)
                        .min(message_frame.size.width.max(1.0))
                } else {
                    0.0
                };
                let document_height = if message_visible {
                    self.message_view.setFrame(NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        NSSize::new(
                            clip_width,
                            self.bubble_layout
                                .message_content_height
                                .max(message_frame.size.height),
                        ),
                    ));
                    let actual_metrics = measure_text_view(&self.message_view, clip_width);
                    let actual_height = actual_metrics.size.height.max(BUBBLE_LINE_HEIGHT);
                    self.bubble_layout.message_content_height = actual_height;
                    actual_height.max(message_frame.size.height)
                } else {
                    0.0
                };
                self.message_view.setFrame(NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(clip_width, document_height),
                ));
                self.cards.view().setHidden(!cards_visible);
                if cards_visible {
                    self.cards.set_frame(cards_frame);
                }
                self.collapse.setHidden(
                    collapse_frame.size.width <= 0.0 || collapse_frame.size.height <= 0.0,
                );
                self.collapse.setFrame(collapse_frame);
                if message_visible {
                    regions.push(message_frame);
                }
                if cards_visible {
                    regions.push(cards_frame);
                }
                if collapse_frame.size.width > 0.0 && collapse_frame.size.height > 0.0 {
                    regions.push(collapse_frame);
                }
            }
        }
        self.bubble_root.set_native_regions(&regions);
    }

    fn expand_bubble(&mut self) {
        if self.bubble_mode == BubbleMode::Expanded {
            return;
        }
        self.bubble_mode = BubbleMode::Expanded;
        self.bubble_content_dirty = true;
        if let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) {
            self.remeasure_bubble(&scene);
        }
    }

    fn collapse_bubble(&mut self) {
        if self.bubble_mode == BubbleMode::Compact {
            return;
        }
        self.bubble_mode = BubbleMode::Compact;
        self.bubble_content_dirty = true;
        if let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) {
            self.remeasure_bubble(&scene);
        }
        if self.bubble_panel.isKeyWindow() {
            let _ = self.bubble_panel.makeFirstResponder(None);
        }
    }

    fn reset_bubble_mode(&mut self) {
        if self.bubble_mode != BubbleMode::Compact {
            self.bubble_mode = BubbleMode::Compact;
            self.bubble_content_dirty = true;
        }
    }

    fn advance_bubble_fade(&mut self) {
        let Some(fade) = self.bubble_fade else {
            return;
        };
        if fade.generation != self.transition_generation || !self.bubble_panel.isVisible() {
            self.bubble_fade = None;
            return;
        }
        let progress = (fade.started.elapsed().as_secs_f64() / 0.16).clamp(0.0, 1.0);
        if progress >= 1.0 || self.reduced_motion {
            self.bubble_panel.setAlphaValue(1.0);
            self.bubble_fade = None;
        } else {
            self.bubble_panel.setAlphaValue(progress);
        }
    }

    fn show_bubble_panel(&mut self) {
        if self.bubble_panel.isVisible() {
            self.advance_bubble_fade();
            if self.bubble_fade.is_none() {
                self.bubble_panel.setAlphaValue(1.0);
            }
            return;
        }
        self.transition_generation = self.transition_generation.saturating_add(1);
        self.bubble_panel.orderFrontRegardless();
        if self.reduced_motion {
            self.bubble_panel.setAlphaValue(1.0);
            self.bubble_fade = None;
        } else {
            self.bubble_panel.setAlphaValue(0.0);
            self.bubble_fade = Some(BubbleFade {
                generation: self.transition_generation,
                started: Instant::now(),
            });
        }
    }

    fn hide_bubble_panel(&mut self) {
        self.transition_generation = self.transition_generation.saturating_add(1);
        self.bubble_fade = None;
        self.bubble_panel.setAlphaValue(0.0);
        self.bubble_panel.setIgnoresMouseEvents(true);
        self.bubble_panel.orderOut(None);
        if self.bubble_panel.isKeyWindow() {
            let _ = self.bubble_panel.makeFirstResponder(None);
            self.bubble_panel.resignKeyWindow();
        }
    }
    fn explicit_gesture_active(&self) -> bool {
        self.interaction.is_active()
            || self.root.ivars().drag.get().is_some()
            || self.bubble_root.ivars().drag.get().is_some()
    }

    fn bubble_placement_tracking_locked(&self) -> bool {
        if self.explicit_gesture_active() {
            return false;
        }
        NSEvent::pressedMouseButtons() != 0 || appkit_event_tracking_active()
    }

    fn bubble_content_tracking_locked(&self) -> bool {
        self.explicit_gesture_active() || self.bubble_placement_tracking_locked()
    }

    fn bubble_contains_screen(&self, screen: NSPoint) -> bool {
        let local = self.bubble_panel.convertPointFromScreen(screen);
        self.bubble_root.contains_local_point(local)
    }
    fn toggle_menu_panel(&mut self) {
        self.sync_dialogue_panel();
        if let Some(button) = self._status_item.button(self.mtm) {
            if !self.menu_panel.is_visible() {
                let app = NSApplication::sharedApplication(self.mtm);
                if !app.isActive() {
                    if app.respondsToSelector(sel!(activate)) {
                        app.activate();
                    } else {
                        // macOS 13 predates NSApplication.activate().
                        let _ = NSRunningApplication::currentApplication()
                            .activateWithOptions(NSApplicationActivationOptions::empty());
                    }
                }
            }
            self.menu_panel.toggle(&button);
        }
    }

    fn sync_dialogue_panel(&mut self) {
        let Some(target) = self.dialogue_target.as_ref() else {
            return;
        };
        self.menu_panel.sync_dialogue(
            target.clone(),
            &self.dialogue_name,
            self.prefs.dialogue_overrides(),
            self.active.metadata(),
            self.locale,
        );
    }

    fn rebuild_effective_dialogue(&mut self) {
        self.effective_dialogue = effective_metadata(
            self.active.metadata(),
            self.dialogue_target
                .as_ref()
                .and_then(|target| self.prefs.dialogue_overrides().locales(target)),
        );
        self.bubble_content_dirty = true;
    }

    fn active_dialogue_matches(&self, target: &DialogueTarget) -> bool {
        self.dialogue_target.as_ref() == Some(target)
            && self.active.token().backend_epoch == self.dialogue_prepared_epoch
            && match target {
                DialogueTarget::Character(id) => {
                    !self.dialogue_override_active && self.active.token().reference.id == *id
                }
                DialogueTarget::ExternalAssets(_) => self.dialogue_override_active,
            }
    }

    fn save_dialogue_entry(&mut self, reset: bool) {
        let Some((target, locale, slot, value)) = self.menu_panel.dialogue_edit() else {
            return;
        };
        if !self.active_dialogue_matches(&target) {
            self.menu_panel
                .set_dialogue_error(Some(text(self.locale, Message::DialogueSaveFailed)));
            return;
        }
        let value = (!reset).then_some(value);
        match self
            .prefs
            .save_dialogue_entry(&target, locale.tag(), slot, value)
        {
            Ok(()) => {
                self.menu_panel.dialogue_saved(&target, locale, slot);
                self.menu_panel.set_dialogue_error(None);
                self.rebuild_effective_dialogue();
                self.sync_dialogue_panel();
                let scene = self.last_scene.clone();
                self.refresh_bubble_content(&scene);
            }
            Err(error) => self.menu_panel.set_dialogue_error(Some(&error)),
        }
    }

    fn confirm_reset_character_dialogue(&mut self) {
        let Some(target) = self.menu_panel.dialogue_target() else {
            return;
        };
        if !self.active_dialogue_matches(&target) {
            self.menu_panel
                .set_dialogue_error(Some(text(self.locale, Message::DialogueSaveFailed)));
            return;
        }
        DispatchQueue::main().exec_async(move || {
            let Some((mtm, locale)) = with_ui_read(|ui| {
                ui.active_dialogue_matches(&target)
                    .then_some((ui.mtm, ui.locale))
            })
            .flatten() else {
                return;
            };
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(text(
                locale,
                Message::DialogueResetConfirm,
            )));
            alert.setInformativeText(&NSString::from_str(text(
                locale,
                Message::DialogueResetConfirmHelp,
            )));
            alert.addButtonWithTitle(&NSString::from_str(text(
                locale,
                Message::DialogueResetCharacter,
            )));
            alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
            if alert.runModal() != NSAlertFirstButtonReturn {
                return;
            }
            with_ui_mut(|ui| {
                if !ui.active_dialogue_matches(&target) {
                    ui.menu_panel
                        .set_dialogue_error(Some(text(ui.locale, Message::DialogueSaveFailed)));
                    return;
                }
                match ui.prefs.reset_character_dialogue(&target) {
                    Ok(()) => {
                        ui.menu_panel.dialogue_reset(&target);
                        ui.menu_panel.set_dialogue_error(None);
                        ui.rebuild_effective_dialogue();
                        ui.sync_dialogue_panel();
                        let scene = ui.last_scene.clone();
                        ui.refresh_bubble_content(&scene);
                    }
                    Err(error) => ui.menu_panel.set_dialogue_error(Some(&error)),
                }
            });
        });
    }

    fn change_observation_machine(&mut self, id: String, selected: bool) {
        let enabled = self.shared.lock().ok().is_some_and(|state| {
            state
                .observation_catalog()
                .machines
                .iter()
                .any(|machine| machine.id == id && machine.enabled)
        });
        if !enabled {
            self.sync_observation_panel();
            return;
        }
        self.change_observation(|candidate| {
            if selected {
                if !candidate.machines.contains(&id) {
                    candidate.machines.push(id);
                }
            } else {
                candidate.machines.retain(|machine| machine != &id);
            }
        });
    }

    fn change_observation(&mut self, change: impl FnOnce(&mut ObservationPreferences)) {
        let mut candidate = self.prefs.observation().clone();
        change(&mut candidate);
        candidate.sanitize();
        if candidate != *self.prefs.observation() {
            if let Err(error) = self.prefs.save_observation(candidate.clone()) {
                self.menu_panel.revert_observation_controls();
                self.sync_observation_panel();
                self.queue_bubble_appearance_error(Message::ObservationSaveFailed, &error);
                return;
            }
            if let Ok(mut state) = self.shared.lock() {
                state.apply_observation_preferences(candidate);
            }
        }
        self.menu_panel.revert_observation_controls();
        self.sync_observation_panel();
        self.refresh();
    }

    fn sync_observation_panel(&mut self) {
        if let Ok(state) = self.shared.lock() {
            self.menu_panel
                .sync_observation(self.prefs.observation(), state.observation_catalog());
        }
    }

    fn select_menu_tab(&mut self, index: usize) {
        self.menu_panel.select_tab(index);
    }

    fn focus_character_manager(&mut self) {
        self.menu_panel.focus_character_manager();
    }
    fn save_lifecycle_setting(&mut self, key: LifecycleSetting) {
        let value = self.menu_panel.lifecycle_value(key);
        let result = control::set_lifecycle_setting(&self.lifecycle_paths, key, value);
        match result {
            Ok(settings) => self.menu_panel.lifecycle_saved(settings),
            Err(error) => {
                if let Ok(state) = self.shared.lock() {
                    self.menu_panel
                        .lifecycle_failed(state.lifecycle_settings(), error);
                }
            }
        }
    }

    fn reanchor_menu_panel(&self) {
        if let Some(button) = self._status_item.button(self.mtm) {
            self.menu_panel.reanchor(&button);
        }
    }

    fn quit(&mut self) {
        self.cancel_gesture();
        self.cancel_pointer();
        self.shutdown();
        if let Ok(mut state) = self.shared.lock() {
            state.request_shutdown();
        }
        stop_application(self.mtm);
    }
}

fn choose_character_source(
    mtm: MainThreadMarker,
    locale: UiLocale,
    title_message: Message,
) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(text(
        locale,
        Message::ChooseCharacterSource,
    ))));
    panel.setPrompt(Some(&NSString::from_str(text(locale, Message::Choose))));
    panel.setTitle(Some(&NSString::from_str(text(locale, title_message))));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel.URL().and_then(|url| url.to_file_path())
}

fn confirm_remove(mtm: MainThreadMarker, locale: UiLocale, id: &str) -> bool {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(
        locale,
        Message::RemoveCharacterPack,
    )));
    alert.setInformativeText(&NSString::from_str(
        &crate::i18n::remove_character_confirmation(locale, id),
    ));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Remove)));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
    alert.runModal() == NSAlertFirstButtonReturn
}
fn show_bubble_appearance_error(
    mtm: MainThreadMarker,
    locale: UiLocale,
    title: Message,
    detail: &str,
) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(locale, title)));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
    let _ = alert.runModal();
}

fn show_language_save_failure(mtm: MainThreadMarker, locale: UiLocale, error: &str) {
    let alert = NSAlert::new(mtm);
    let title = NSString::from_str(text(locale, Message::LanguageSaveFailed));
    alert.setMessageText(&title);
    let detail = language_save_failure(locale, error);
    alert.setInformativeText(&NSString::from_str(&detail));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
    let window = alert.window();
    if let Some(content) = window.contentView() {
        set_accessibility_label(
            &content,
            text(locale, Message::LanguageSaveFailedAccessibility),
        );
    }
    let _ = alert.runModal();
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.remove_menu_event_monitors();
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&*self._window_delegate);
        }
        self.stop_timer();
        self.stop_prepare_timer();
        self.stop_language_timer();
        self.cancel_pending_native();
        if let Some(timer) = self.pointer_timer.take() {
            timer.invalidate();
        }
        self.interaction.cancel();
        self.pointer_press = None;
        self.root.ivars().drag.set(None);
        self.bubble_root.ivars().drag.set(None);
        self.panel.setIgnoresMouseEvents(true);
        self.bubble_panel.setIgnoresMouseEvents(true);
        self.panel.orderOut(None);
        self.bubble_panel.orderOut(None);
        self.menu_panel.shutdown();
    }
}

fn grip_hit_rect(size: NSSize) -> NSRect {
    let width = size.width.max(0.0);
    let height = size.height.max(0.0);
    NSRect::new(
        NSPoint::new((width - GRIP_HIT_SIZE).max(0.0), 0.0),
        NSSize::new(width.min(GRIP_HIT_SIZE), height.min(GRIP_HIT_SIZE)),
    )
}

fn grip_hit_test(point: NSPoint, size: NSSize) -> bool {
    let hit = grip_hit_rect(size);
    point.x >= hit.origin.x
        && point.x <= hit.origin.x + hit.size.width
        && point.y >= hit.origin.y
        && point.y <= hit.origin.y + hit.size.height
}

fn resize_frame(geometry: DisplayGeometry, top_left: NSPoint, scale: f64) -> NSRect {
    let size = geometry.size(scale);
    NSRect::new(NSPoint::new(top_left.x, top_left.y - size.height), size)
}

fn resize_scale_for_delta(
    geometry: DisplayGeometry,
    start_scale: f64,
    start_mouse: NSPoint,
    mouse: NSPoint,
) -> f64 {
    let dx = mouse.x - start_mouse.x;
    let dy = mouse.y - start_mouse.y;
    if !dx.is_finite() || !dy.is_finite() {
        return start_scale;
    }
    // Preserve the pack envelope's aspect ratio, excluding fixed safety padding.
    start_scale + geometry.resize_delta(dx, dy)
}

fn resize_scale_limits(
    geometry: DisplayGeometry,
    top_left: NSPoint,
    visible: NSRect,
) -> Option<(f64, f64)> {
    let min_x = visible.origin.x;
    let min_y = visible.origin.y;
    let max_x = min_x + visible.size.width;
    let max_y = min_y + visible.size.height;
    if !min_x.is_finite()
        || !min_y.is_finite()
        || !max_x.is_finite()
        || !max_y.is_finite()
        || visible.size.width <= 0.0
        || visible.size.height <= 0.0
        || top_left.x < min_x
        || top_left.y > max_y
    {
        return None;
    }
    let max_scale = geometry
        .fitting_scale(max_x - top_left.x, top_left.y - min_y)
        .min(MAX_SCALE);
    if !max_scale.is_finite() || max_scale < MIN_SCALE {
        None
    } else {
        Some((MIN_SCALE, max_scale))
    }
}

fn panel_visible_frame(panel: &NSPanel, mtm: MainThreadMarker) -> Option<NSRect> {
    panel
        .screen()
        .or_else(|| NSScreen::mainScreen(mtm))
        .map(|screen| screen.visibleFrame())
}

fn presentation_matches_scene(scene: &Scene, expected: &Scene) -> bool {
    scene.visible == expected.visible
        && scene.passthrough == expected.passthrough
        && scene.alpha_passthrough == expected.alpha_passthrough
        && scene.bubble_visible == expected.bubble_visible
        && scene.bubble_placement == expected.bubble_placement
        && scene.reset_position_revision == expected.reset_position_revision
        && (scene.scale - expected.scale).abs() <= f64::EPSILON
}

fn scene_matches_drag(scene: &Scene, drag: &DragState) -> bool {
    scene.visible == drag.expected_visible
        && scene.passthrough == drag.expected_passthrough
        && scene.alpha_passthrough == drag.expected_alpha_passthrough
        && scene.bubble_visible == drag.expected_bubble_visible
        && scene.bubble_placement == drag.expected_bubble_placement
        && scene.reset_position_revision == drag.expected_reset_position_revision
        && (scene.scale - drag.expected_scale).abs() <= f64::EPSILON
}

/// A screen change caused by the pet's own Move drag must not end that drag.
/// Resize drags and windows AppKit moved away from the expected frame still
/// cancel.
fn drag_survives_screen_change(drag: Option<DragState>, frame: NSRect) -> bool {
    drag.is_some_and(|drag| {
        drag.kind == GestureKind::Move && rect_nearly_equal(frame, drag.expected_frame)
    })
}

fn rect_nearly_equal(a: NSRect, b: NSRect) -> bool {
    const EPSILON: f64 = 0.001;
    (a.origin.x - b.origin.x).abs() <= EPSILON
        && (a.origin.y - b.origin.y).abs() <= EPSILON
        && (a.size.width - b.size.width).abs() <= EPSILON
        && (a.size.height - b.size.height).abs() <= EPSILON
}

fn configure_panel(panel: &NSPanel) {
    // SAFETY: windows created outside a controller must disable automatic release on close.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setWorksWhenModal(true);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary,
    );
    panel.setOpaque(false);
    let clear = NSColor::clearColor();
    panel.setBackgroundColor(Some(&clear));
    panel.setHasShadow(true);
    panel.setAcceptsMouseMovedEvents(true);
}

fn bounded_frame(frame: NSRect, bounds: NSRect) -> NSRect {
    let bounds_x = bounds.origin.x;
    let bounds_y = bounds.origin.y;
    let bounds_width = bounds.size.width.max(0.0);
    let bounds_height = bounds.size.height.max(0.0);
    let x = frame.origin.x.clamp(bounds_x, bounds_x + bounds_width);
    let y = frame.origin.y.clamp(bounds_y, bounds_y + bounds_height);
    let width = frame
        .size
        .width
        .max(0.0)
        .min((bounds_x + bounds_width - x).max(0.0));
    let height = frame
        .size
        .height
        .max(0.0)
        .min((bounds_y + bounds_height - y).max(0.0));
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn appkit_event_tracking_active() -> bool {
    let mode = NSRunLoop::currentRunLoop().currentMode();
    mode.is_some_and(|mode| mode.isEqualToString(unsafe { NSEventTrackingRunLoopMode }))
}

fn point_in_rect(point: NSPoint, rect: NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}
fn set_accessibility_label(view: &NSView, label: &str) {
    let label = NSString::from_str(label);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}
fn set_accessibility_identifier(view: &NSView, identifier: &str) {
    let identifier = NSString::from_str(identifier);
    unsafe {
        let _: () = msg_send![view, setAccessibilityIdentifier: Some(&*identifier)];
    }
}

fn set_accessibility_cell_text(cell: &NSCell, label: &str, value: Option<&str>) {
    let label = NSString::from_str(label);
    unsafe {
        let _: () = msg_send![cell, setAccessibilityLabel: Some(&*label)];
    }
    if let Some(value) = value {
        let value = NSString::from_str(value);
        unsafe {
            let _: () = msg_send![cell, setAccessibilityValue: Some(&*value)];
        }
    }
}

fn layout_metrics_for_manager(
    manager: &NSLayoutManager,
    container: &NSTextContainer,
    width: f64,
    padding: f64,
    char_wrapping: bool,
) -> TextMetrics {
    let width = width.max(1.0);
    container.setContainerSize(NSSize::new(width, 1_000_000.0));
    container.setLineFragmentPadding(padding);
    container.setLineBreakMode(if char_wrapping {
        NSLineBreakMode::ByCharWrapping
    } else {
        NSLineBreakMode::ByWordWrapping
    });
    manager.ensureLayoutForTextContainer(container);
    let glyph_range = manager.glyphRangeForTextContainer(container);
    let mut line_ranges = Vec::new();
    let mut line_widths = Vec::new();
    let mut glyph = glyph_range.location;
    let glyph_end = glyph_range.location + glyph_range.length;
    while glyph < glyph_end {
        let mut effective = NSRange::new(glyph, 0);
        let used = unsafe {
            manager.lineFragmentUsedRectForGlyphAtIndex_effectiveRange(glyph, &mut effective)
        };
        if effective.length == 0 {
            break;
        }
        let mut actual = NSRange::new(0, 0);
        let chars =
            unsafe { manager.characterRangeForGlyphRange_actualGlyphRange(effective, &mut actual) };
        line_ranges.push(chars);
        line_widths.push(used.size.width.max(0.0));
        let next = effective.location + effective.length;
        if next <= glyph {
            break;
        }
        glyph = next;
    }
    let used = manager.usedRectForTextContainer(container);
    let width = line_widths
        .iter()
        .copied()
        .fold(used.size.width.max(0.0), f64::max);
    let height = (used.origin.y + used.size.height).max(0.0);
    TextMetrics {
        size: NSSize::new(width, height),
        line_ranges,
        line_widths,
        char_wrapping,
    }
}

fn layout_metrics_for_attributed(attributed: &NSAttributedString, width: f64) -> TextMetrics {
    let storage = NSTextStorage::new();
    let _: () = unsafe { msg_send![&*storage, setAttributedString: attributed] };
    let manager = NSLayoutManager::new();
    manager.setUsesFontLeading(true);
    let container = NSTextContainer::initWithSize(
        NSTextContainer::alloc(),
        NSSize::new(width.max(1.0), 1_000_000.0),
    );
    manager.addTextContainer(&container);
    storage.addLayoutManager(&manager);
    let word = layout_metrics_for_manager(&manager, &container, width, 0.0, false);
    if word.size.width > width + 0.5
        || word
            .line_widths
            .iter()
            .any(|line_width| *line_width > width + 0.5)
    {
        layout_metrics_for_manager(&manager, &container, width, 0.0, true)
    } else {
        word
    }
}

fn text_field_native_inset(field: &NSTextField) -> f64 {
    field
        .cell()
        .map(|cell| {
            let probe = cell.copy();
            probe.setStringValue(&NSString::from_str(""));
            probe.cellSize().width.max(0.0)
        })
        .unwrap_or(0.0)
}

fn measure_attributed_field(field: &NSTextField, width: f64) -> TextMetrics {
    let inset = text_field_native_inset(field);
    let attributed = field.attributedStringValue();
    let mut metrics = layout_metrics_for_attributed(&attributed, (width.max(1.0) - inset).max(1.0));
    metrics.size.width += inset;
    metrics
}

fn measure_text_view(view: &NSTextView, width: f64) -> TextMetrics {
    let Some(container) = (unsafe { view.textContainer() }) else {
        return TextMetrics {
            size: NSSize::new(0.0, 0.0),
            line_ranges: Vec::new(),
            line_widths: Vec::new(),
            char_wrapping: false,
        };
    };
    let Some(manager) = (unsafe { view.layoutManager() }) else {
        return TextMetrics {
            size: NSSize::new(0.0, 0.0),
            line_ranges: Vec::new(),
            line_widths: Vec::new(),
            char_wrapping: false,
        };
    };
    // Measure the live container with the view's own padding, inside its inset.
    let inset = view.textContainerInset();
    let padding = container.lineFragmentPadding();
    let width = (width - 2.0 * inset.width).max(1.0);
    let word = layout_metrics_for_manager(&manager, &container, width, padding, false);
    let mut metrics = if word.size.width > width + 0.5
        || word
            .line_widths
            .iter()
            .any(|line_width| *line_width > width + 0.5)
    {
        layout_metrics_for_manager(&manager, &container, width, padding, true)
    } else {
        word
    };
    metrics.size.width += 2.0 * inset.width;
    metrics.size.height += 2.0 * inset.height;
    metrics
}

fn truncate_to_lines(
    text: &str,
    field: &NSTextField,
    width: f64,
    max_lines: usize,
    font: &NSFont,
    color: &NSColor,
    paragraph_style: &NSMutableParagraphStyle,
) -> String {
    if text.is_empty() {
        return String::new();
    }
    let source = NSString::from_str(text);
    let mut boundaries = vec![0usize];
    let mut index = 0usize;
    while index < source.length() {
        let range = source.rangeOfComposedCharacterSequenceAtIndex(index);
        let next = range.location + range.length;
        if next <= index {
            break;
        }
        boundaries.push(next);
        index = next;
    }
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);
    let mut best = 0usize;
    while low <= high {
        let middle = low + (high - low) / 2;
        let prefix = source.substringToIndex(boundaries[middle]).to_string();
        let candidate = format!("{prefix}…");
        set_attributed_field_text(field, &candidate, font, color, paragraph_style);
        let metrics = measure_attributed_field(field, width);
        if metrics.line_count() <= max_lines {
            best = middle;
            low = middle.saturating_add(1);
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }
    let prefix = source.substringToIndex(boundaries[best]).to_string();
    let result = format!("{prefix}…");
    set_attributed_field_text(field, &result, font, color, paragraph_style);
    result
}

fn append_edge_with_tail(
    path: &NSBezierPath,
    from: NSPoint,
    to: NSPoint,
    tail: Option<((f64, f64), (f64, f64), (f64, f64))>,
) {
    let Some((start, end, tip)) = tail else {
        path.lineToPoint(to);
        return;
    };
    let horizontal = (from.y - to.y).abs() <= 0.5;
    let on_edge = if horizontal {
        (start.1 - from.y).abs() <= 1.0
            && (end.1 - from.y).abs() <= 1.0
            && tip.0 >= from.x.min(to.x)
            && tip.0 <= from.x.max(to.x)
    } else {
        (start.0 - from.x).abs() <= 1.0
            && (end.0 - from.x).abs() <= 1.0
            && tip.1 >= from.y.min(to.y)
            && tip.1 <= from.y.max(to.y)
    };
    if !on_edge {
        path.lineToPoint(to);
        return;
    }
    let forward = if horizontal {
        to.x >= from.x
    } else {
        to.y >= from.y
    };
    let (a, b) = if forward { (start, end) } else { (end, start) };
    let a = NSPoint::new(a.0, a.1);
    let b = NSPoint::new(b.0, b.1);
    let tip = NSPoint::new(tip.0, tip.1);
    path.lineToPoint(a);
    path.lineToPoint(tip);
    path.lineToPoint(b);
    path.lineToPoint(to);
}

fn bubble_path(
    body: BubbleRect,
    tail: Option<crate::bubble::TailGeometry>,
) -> Retained<NSBezierPath> {
    let path = NSBezierPath::bezierPath();
    let radius = BUBBLE_RADIUS
        .min(body.width.max(0.0) * 0.5)
        .min(body.height.max(0.0) * 0.5);
    let k = 0.552_284_8;
    let x0 = body.x;
    let y0 = body.y;
    let x1 = body.x + body.width;
    let y1 = body.y + body.height;
    let tail = tail.map(|tail| (tail.base_start, tail.base_end, tail.tip));
    path.moveToPoint(NSPoint::new(x0 + radius, y0));
    append_edge_with_tail(
        &path,
        NSPoint::new(x0 + radius, y0),
        NSPoint::new(x1 - radius, y0),
        tail,
    );
    path.curveToPoint_controlPoint1_controlPoint2(
        NSPoint::new(x1, y0 + radius),
        NSPoint::new(x1 - radius + k * radius, y0),
        NSPoint::new(x1, y0 + radius - k * radius),
    );
    append_edge_with_tail(
        &path,
        NSPoint::new(x1, y0 + radius),
        NSPoint::new(x1, y1 - radius),
        tail,
    );
    path.curveToPoint_controlPoint1_controlPoint2(
        NSPoint::new(x1 - radius, y1),
        NSPoint::new(x1, y1 - radius + k * radius),
        NSPoint::new(x1 - radius + k * radius, y1),
    );
    append_edge_with_tail(
        &path,
        NSPoint::new(x1 - radius, y1),
        NSPoint::new(x0 + radius, y1),
        tail,
    );
    path.curveToPoint_controlPoint1_controlPoint2(
        NSPoint::new(x0, y1 - radius),
        NSPoint::new(x0 + radius - k * radius, y1),
        NSPoint::new(x0, y1 - radius + k * radius),
    );
    append_edge_with_tail(
        &path,
        NSPoint::new(x0, y1 - radius),
        NSPoint::new(x0, y0 + radius),
        tail,
    );
    path.curveToPoint_controlPoint1_controlPoint2(
        NSPoint::new(x0 + radius, y0),
        NSPoint::new(x0, y0 + radius - k * radius),
        NSPoint::new(x0 + radius - k * radius, y0),
    );
    path.closePath();
    path
}

fn nsrect_tuple(rect: NSRect) -> (f64, f64, f64, f64) {
    (
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}

fn clamp_artwork_point(point: Point) -> Point {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Point {
            x: BASE_WIDTH * 0.5,
            y: BASE_HEIGHT * 0.5,
        };
    }
    Point {
        x: point.x.clamp(0.0, BASE_WIDTH),
        y: point.y.clamp(0.0, BASE_HEIGHT),
    }
}

fn status_fields_changed(scene: &Scene, previous: &Scene) -> bool {
    scene.phase != previous.phase
        || scene.sessions != previous.sessions
        || scene.working != previous.working
        || scene.blocked != previous.blocked
        || scene.done != previous.done
        || scene.unknown != previous.unknown
        || scene.connected_sources != previous.connected_sources
        || scene.disconnected_sources != previous.disconnected_sources
}

fn frame_size_matches_scale(geometry: DisplayGeometry, size: NSSize, scale: f64) -> bool {
    let expected = geometry.size(scale);
    // NSWindow aligns its outer frame to integral logical points, independently
    // of the backing pixel scale.
    const TOLERANCE: f64 = 1.001;
    (size.width - expected.width).abs() <= TOLERANCE
        && (size.height - expected.height).abs() <= TOLERANCE
}

fn default_origin(size: NSSize, mtm: MainThreadMarker) -> NSPoint {
    let Some(screen) = NSScreen::mainScreen(mtm) else {
        return NSPoint::new(24.0, 24.0);
    };
    let visible = screen.visibleFrame();
    NSPoint::new(
        visible.origin.x + (visible.size.width - size.width - 24.0).max(0.0),
        visible.origin.y + 24.0,
    )
}

fn status_text(scene: &Scene, locale: UiLocale) -> String {
    if scene.sessions == 0 && scene.connected_sources == 0 && scene.disconnected_sources == 0 {
        return task_status(locale, TaskStatus::NoConnectedTasks, 0, 0);
    }
    let (status, count) = match scene.phase {
        Phase::Running => (TaskStatus::InProgress, scene.working),
        Phase::Waiting => (TaskStatus::NeedsAttention, scene.blocked),
        Phase::Idle => (TaskStatus::Complete, scene.done),
        Phase::Unknown => (TaskStatus::BeingChecked, scene.unknown),
    };
    task_status(locale, status, count, scene.sessions)
}

fn disconnected_text(scene: &Scene, locale: UiLocale) -> String {
    disconnected_sources(locale, scene.disconnected_sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.0e-9
    }

    #[test]
    fn resize_accepts_appkit_rounded_settled_frame() {
        let geometry = DisplayGeometry::new((0.25, 0.25, 0.75, 0.75), (384, 512), false);
        let expected = geometry.size(0.8);
        assert!(frame_size_matches_scale(
            geometry,
            NSSize::new(expected.width + 1.0, expected.height),
            0.8,
        ));
        assert!(!frame_size_matches_scale(
            geometry,
            NSSize::new(expected.width + 2.0, expected.height),
            0.8,
        ));
    }

    #[test]
    fn resize_keeps_top_left_anchor() {
        let geometry = DisplayGeometry::new((0.2, 0.15, 0.85, 0.9), (384, 512), false);
        let top_left = NSPoint::new(-220.0, 540.0);
        let frame = resize_frame(geometry, top_left, 0.9);
        assert!(close(frame.origin.x, top_left.x));
        assert!(close(frame.origin.y + frame.size.height, top_left.y));
        assert_eq!(frame.size, geometry.size(0.9));
    }

    #[test]
    fn resize_bounds_reject_infeasible_screen() {
        let geometry = DisplayGeometry::new((0.0, 0.0, 1.0, 1.0), (384, 512), false);
        let visible = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 100.0));
        assert!(resize_scale_limits(geometry, NSPoint::new(20.0, 80.0), visible).is_none());

        let visible = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1_000.0, 900.0));
        let (min_scale, max_scale) =
            resize_scale_limits(geometry, NSPoint::new(100.0, 800.0), visible).unwrap();
        assert!(close(min_scale, MIN_SCALE));
        assert!(close(max_scale, MAX_SCALE));
    }

    #[test]
    fn resize_caps_against_negative_screen_edges() {
        let geometry = DisplayGeometry::new((0.0, 0.0, 1.0, 1.0), (384, 512), false);
        let visible = NSRect::new(NSPoint::new(-1_600.0, -900.0), NSSize::new(800.0, 700.0));
        let top_left = NSPoint::new(-1_100.0, -300.0);
        let (_, max_scale) = resize_scale_limits(geometry, top_left, visible).unwrap();
        let frame = resize_frame(geometry, top_left, max_scale);
        assert!(close(
            frame.origin.x + frame.size.width,
            visible.origin.x + visible.size.width
        ));
        assert!(frame.origin.y > visible.origin.y);
    }

    #[test]
    fn resize_uses_initial_pointer_offset_without_jump() {
        let geometry = DisplayGeometry::new((0.25, 0.25, 0.75, 0.75), (384, 512), false);
        let start_scale = 0.65;
        let top_left = NSPoint::new(100.0, 700.0);
        let start_frame = resize_frame(geometry, top_left, start_scale);
        let start_mouse = NSPoint::new(
            start_frame.origin.x + start_frame.size.width - 6.0,
            start_frame.origin.y + 7.0,
        );
        let unchanged = resize_scale_for_delta(geometry, start_scale, start_mouse, start_mouse);
        assert!(close(unchanged, start_scale));

        let moved_mouse = NSPoint::new(start_mouse.x + 10.0, start_mouse.y);
        let moved_scale = resize_scale_for_delta(geometry, start_scale, start_mouse, moved_mouse);
        let moved_frame = resize_frame(geometry, top_left, moved_scale);
        assert!(close(moved_frame.origin.x, start_frame.origin.x));
        assert!(close(
            moved_frame.origin.y + moved_frame.size.height,
            top_left.y
        ));
        assert!(close(moved_frame.size.width - start_frame.size.width, 10.0));
    }

    #[test]
    fn screen_change_keeps_only_matching_move_drag() {
        let frame = NSRect::new(NSPoint::new(1_480.0, 300.0), NSSize::new(240.0, 320.0));
        let move_drag = DragState {
            kind: GestureKind::Move,
            start_mouse: NSPoint::new(1_400.0, 400.0),
            start_frame: NSRect::new(NSPoint::new(1_200.0, 300.0), frame.size),
            start_top_left: NSPoint::new(1_200.0, 620.0),
            start_scale: 0.8,
            screen_visible: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1_440.0, 900.0)),
            expected_frame: frame,
            expected_scale: 0.8,
            expected_visible: true,
            expected_passthrough: false,
            expected_alpha_passthrough: false,
            expected_bubble_visible: true,
            expected_bubble_placement: BubblePlacement::Above,
            expected_reset_position_revision: 0,
            input_owner: None,
        };
        let resize_drag = DragState {
            kind: GestureKind::Resize,
            ..move_drag
        };
        let mut shifted = frame;
        shifted.origin.x += 1.0;

        assert!(drag_survives_screen_change(Some(move_drag), frame));
        assert!(!drag_survives_screen_change(Some(resize_drag), frame));
        assert!(!drag_survives_screen_change(Some(move_drag), shifted));
        assert!(!drag_survives_screen_change(None, frame));
    }
    #[test]
    fn expanded_composer_fits_one_row_on_300_by_260_screen() {
        let body_cap = 260.0 - BUBBLE_WINDOW_INSET * 2.0;
        let cards = minimum_selectable_height();
        for show_status in [false, true] {
            for has_message in [false, true] {
                let normal_minimum = expanded_minimum_height(cards, show_status, has_message);
                let spacing = expanded_spacing(body_cap, cards, show_status, has_message);
                let minimum = expanded_height_budget(cards, show_status, has_message, spacing);
                let body_height = minimum.max(260.0 * 0.78).min(body_cap);
                let desired_message = if has_message { 116.0 } else { 0.0 };
                let slots = expanded_heights(
                    body_height,
                    BUBBLE_CARDS_MAX_HEIGHT,
                    desired_message,
                    show_status,
                    spacing,
                );
                assert!(slots.cards >= cards);
                assert!(slots.editor >= COMPOSER_MIN_EDITOR_HEIGHT);
                assert!(slots.target >= COMPOSER_TARGET_HEIGHT);
                assert!(slots.status >= COMPOSER_MIN_STATUS_HEIGHT);
                if has_message {
                    assert!(slots.message > 0.0);
                    if show_status {
                        assert!(slots.message >= BUBBLE_LINE_HEIGHT);
                    }
                } else {
                    assert_eq!(slots.message, 0.0);
                }
                let content_top = body_height - spacing.inset;
                let status_bottom = content_top - STATUS_ROW_HEIGHT;
                let message_top =
                    expanded_message_top(status_bottom, content_top, show_status, has_message);
                if show_status {
                    assert_eq!(
                        status_bottom - message_top,
                        if has_message { BUBBLE_CONTENT_GAP } else { 0.0 }
                    );
                }
                if show_status {
                    assert!(normal_minimum > body_cap);
                    assert!(minimum <= body_cap);
                } else {
                    assert_eq!(spacing.gap, BUBBLE_CONTENT_GAP);
                }
                let occupied = spacing.inset * 2.0
                    + BUBBLE_CONTROL_HEIGHT
                    + spacing.gap
                        * (if show_status && !has_message {
                            4.0
                        } else {
                            5.0
                        })
                    + if show_status {
                        STATUS_ROW_HEIGHT + BUBBLE_CONTENT_GAP
                    } else {
                        0.0
                    }
                    + slots.status
                    + slots.editor
                    + slots.target
                    + slots.cards
                    + slots.message;
                assert!(occupied <= body_height);
            }
        }
        let desktop = expanded_spacing(530.0, cards, true, true);
        assert_eq!(desktop.inset, BUBBLE_VERTICAL_INSET);
        assert_eq!(desktop.gap, BUBBLE_CONTENT_GAP);

        let width = 300.0 - BUBBLE_WINDOW_INSET * 2.0 - BUBBLE_HORIZONTAL_INSET * 2.0;
        assert!(width > 78.0 + BUBBLE_CONTENT_GAP);
    }

    #[test]
    fn bridge_disconnection_cancels_deferred_request() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let cancel = AtomicBool::new(false);
        assert!(wait_for_main_result(receiver, &cancel).is_err());
        assert!(cancel.load(Ordering::Acquire));
    }
}
