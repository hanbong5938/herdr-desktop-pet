#[path = "ui_dialogues.rs"]
mod automation_dialogues;
#[path = "ui_preferences.rs"]
mod automation_preferences;
#[path = "ui_sessions.rs"]
mod automation_sessions;
#[path = "ui_worktrees.rs"]
mod automation_worktrees;

use crate::animation::{phase_index, FrameId, Playback};
use crate::assets::CharacterMetadata;
use crate::assets::{AssetPack, ValidatedCharacter};
use crate::automation::{
    lock_automation, AutomationState, PendingReason, PresentationAction, PresentationCheckpoint,
    PresentationPatch, PresentationTarget, SharedAutomation, WindowFrame,
};
use crate::behavior::{Behavior, Presentation, PresentationIntent, PresentationViewport, Reaction};
use crate::bubble::{
    place_bubble, place_standalone_bubble, BubbleGeometry, BubblePlacement, Rect as BubbleRect,
    BUBBLE_RADIUS, BUBBLE_WINDOW_INSET,
};
use crate::character_menu::{self, CharacterMenu, MenuCommand};
use crate::character_renderer::{
    self, CharacterHit, PrepareBuilder, PreparedCharacter, SpeechAnchorSnapshot, SpeechAnchorStatus,
};
use crate::character_selection::{CharacterSelection, SelectionError};
use crate::character_service::PackService;
use crate::character_types::{
    CharacterRef, PackAction, PackListing, PackOperation, PackRequest, RendererToken,
};
use crate::composer_layout::{
    configure_composer, layout_composer, ComposerMetrics, INPUT_ORIGIN_Y,
};
use crate::control;
use crate::dialogue::{effective_metadata, DialogueSlot, DialogueTarget};
use crate::dialogue_automation::{
    target_overrides_token, DialogueBaseline, DialogueIdentity, DialogueLanguage, DialogueSelection,
};
use crate::dialogue_editor::{
    DialogueChoice, DialogueDraftBaseline, DialogueEditor, HydrationSync,
};
use crate::display_geometry::{DisplayGeometry, BASE_HEIGHT, BASE_WIDTH};
use crate::herdr::{
    PromptError, PromptSender, RequestOrigin, WorktreeRemoveError, WorktreeRemoveSender,
};
use crate::i18n::{
    default_dialogue, disconnected_sources, language_save_failure, resolve_language,
    status_indicator_summary, task_disclosure, task_status, text, worktree_remove_confirmation,
    LanguagePreference, Message, TaskStatus, UiLocale,
};
use crate::interaction::{GestureAction, Interaction, Point, RegionPolicy};
use crate::lifecycle::{LifecycleSetting, Paths};
use crate::menu_bar_icon;
use crate::menu_panel::MenuPanel;
use crate::preferences::{
    BubbleAppearance, BubbleColor, BubblePalette, BubbleTheme, MenuBarIconPreference, MenuBarMode,
    Preferences,
};
use crate::session_cards::{
    card_header_key_at_hit, maximum_cards_height, minimum_selectable_height, SessionCards,
};
use crate::session_view::{SessionKey, SessionStatusSummary, WorktreeRemoveTarget};
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
    MainThreadOnly, Message as ObjcMessage,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAppearance,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSBezierPath, NSButton, NSButtonCell, NSCell, NSColor,
    NSControlStateValueOn, NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition,
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventTrackingRunLoopMode, NSEventType,
    NSFloatingWindowLevel, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSImage,
    NSImageScaling, NSImageView, NSLayoutManager, NSLineBreakMode, NSMenu, NSMenuItem,
    NSModalPanelRunLoopMode, NSModalResponseOK, NSMutableParagraphStyle, NSOpenPanel, NSPanel,
    NSParagraphStyleAttributeName, NSPasteboard, NSPasteboardTypeString, NSPopUpButton,
    NSRunningApplication, NSScreen, NSScrollView, NSScrollerStyle, NSStatusBar, NSStatusItem,
    NSSwitch, NSTextAlignment, NSTextContainer, NSTextField, NSTextStorage, NSTextView,
    NSTrackingArea, NSTrackingAreaOptions, NSUserInterfaceItemIdentification,
    NSVariableStatusItemLength, NSView, NSWindowCollectionBehavior, NSWindowDelegate,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSCopying, NSCurrentLocaleDidChangeNotification, NSDate, NSLocale,
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
const COMPOSER_READONLY_HEIGHT: f64 = 25.0;
const COMPOSER_STATUS_HEIGHT: f64 = 18.0;

fn menu_bar_visible(scene: &Scene, mode: MenuBarMode) -> bool {
    !scene.shutdown && (mode == MenuBarMode::Always || scene.needs_recovery_entry())
}

#[derive(Clone, Copy)]
struct ExpandedSpacing {
    inset: f64,
    gap: f64,
}

fn expanded_spacing(
    body_height: f64,
    cards_height: f64,
    minimum_cards: f64,
    show_status: bool,
    has_message: bool,
) -> ExpandedSpacing {
    let normal = ExpandedSpacing {
        inset: BUBBLE_VERTICAL_INSET,
        gap: BUBBLE_CONTENT_GAP,
    };
    if expanded_height_budget(
        cards_height,
        minimum_cards,
        show_status,
        has_message,
        normal,
    ) > body_height
    {
        ExpandedSpacing {
            inset: 1.0,
            gap: 0.5,
        }
    } else {
        normal
    }
}

fn expanded_height_budget(
    cards_height: f64,
    minimum_cards: f64,
    show_status: bool,
    has_message: bool,
    spacing: ExpandedSpacing,
) -> f64 {
    spacing.inset * 2.0
        + BUBBLE_CONTROL_HEIGHT
        + spacing.gap
            * (if show_status && !has_message {
                1.0
            } else {
                2.0
            })
        + if show_status {
            STATUS_ROW_HEIGHT + if has_message { spacing.gap } else { 0.0 }
        } else {
            0.0
        }
        + cards_height.min(minimum_cards)
        + if has_message { BUBBLE_LINE_HEIGHT } else { 0.0 }
}

fn expanded_message_top(
    status_bottom: f64,
    content_top: f64,
    show_status: bool,
    has_message: bool,
    spacing: ExpandedSpacing,
) -> f64 {
    if show_status {
        status_bottom - if has_message { spacing.gap } else { 0.0 }
    } else {
        content_top
    }
}

struct ExpandedHeights {
    cards: f64,
    message: f64,
}

fn expanded_heights(
    body_height: f64,
    desired_cards: f64,
    desired_message: f64,
    minimum_cards: f64,
    show_status: bool,
    spacing: ExpandedSpacing,
) -> ExpandedHeights {
    let remaining = (body_height
        - spacing.inset * 2.0
        - BUBBLE_CONTROL_HEIGHT
        - spacing.gap
            * (if show_status && desired_message == 0.0 {
                1.0
            } else {
                2.0
            })
        - if show_status {
            STATUS_ROW_HEIGHT
                + if desired_message > 0.0 {
                    spacing.gap
                } else {
                    0.0
                }
        } else {
            0.0
        })
    .max(0.0);
    let cards = desired_cards.min(minimum_cards).min(remaining);
    let message = desired_message.min((remaining - cards).max(0.0));
    ExpandedHeights {
        cards: desired_cards.min((remaining - message).max(0.0)),
        message,
    }
}

// NSTextView indices are UTF-16 offsets. Only CRLF changes their width; preserve
// every other code unit (including surrogate pairs) in the text storage.
fn line_break_ranges(value: &NSString) -> Vec<NSRange> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < value.length() {
        let ch = value.characterAtIndex(index);
        let length = match ch {
            0x0d if index + 1 < value.length() && value.characterAtIndex(index + 1) == 0x0a => 2,
            0x0a | 0x0d | 0x2028 | 0x2029 => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        ranges.push(NSRange::new(index, length));
        index += length;
    }
    ranges
}

fn adjusted_composer_selection(selection: NSRange, breaks: &[NSRange]) -> NSRange {
    if selection.location == usize::MAX {
        return selection;
    }
    let adjust = |position: usize| {
        position
            - breaks
                .iter()
                .filter(|range| range.length == 2 && range.location + 2 <= position)
                .count()
    };
    let start = adjust(selection.location);
    let end = adjust(selection.location + selection.length);
    NSRange::new(start, end - start)
}
const COMPOSER_DRAFT_LIMIT: usize = 32;

fn bubble_color(color: BubbleColor, alpha: f64) -> Retained<NSColor> {
    let (r, g, b) = color.rgb();
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha)
}

fn reply_input_color(palette: BubblePalette) -> Retained<NSColor> {
    let (r, g, b) = palette.surface.rgb();
    let (text_r, text_g, text_b) = palette.text.rgb();
    // NSScrollView paints an opaque background; blend before handing it to AppKit.
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r * 0.9 + text_r * 0.1,
        g * 0.9 + text_g * 0.1,
        b * 0.9 + text_b * 0.1,
        1.0,
    )
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
    feedback_summary: String,
    feedback_summary_height: f64,
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
            feedback_summary: String::new(),
            feedback_summary_height: 0.0,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DragTarget {
    Character,
    StandaloneBubble,
}

#[derive(Clone, Copy, Debug)]
struct InputOwner {
    backend_epoch: u64,
    input_epoch: u64,
    viewport_epoch: u64,
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    target: DragTarget,
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
struct ComposerViewIvars {
    normalizing: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetComposerView"]
    #[ivars = ComposerViewIvars]
    struct ComposerView;
    unsafe impl NSObjectProtocol for ComposerView {}

    impl ComposerView {
        #[unsafe(method(didChangeText))]
        fn did_change_text(&self) {
            let _: () = unsafe { msg_send![super(self), didChangeText] };
            self.normalize_committed_text();
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            // Like the dialogue editor, this panel has no application Edit menu.
            if self
                .window()
                .and_then(|window| window.firstResponder())
                .is_some_and(|responder| {
                    Retained::as_ptr(&responder).cast::<()>()
                        == (self as *const Self).cast::<()>()
                })
                && self.handle_edit_shortcut(event)
            {
                return true.into();
            }
            unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // Some window dispatch paths bypass performKeyEquivalent:.
            if self.handle_edit_shortcut(event) {
                return;
            }
            let marked: bool = unsafe { msg_send![self, hasMarkedText] };
            if !marked && matches!(event.keyCode(), 36 | 76) {
                with_ui_mut(|ui| ui.submit_composer());
            } else if event.keyCode() == 53 {
                if marked {
                    // Let Cocoa's input context handle Escape first; some IMEs consume it themselves.
                    let _: () = unsafe { msg_send![super(self), keyDown: event] };
                    self.discard_composition();
                } else {
                    with_ui_mut(|ui| ui.fold_reply());
                }
            } else {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let _: () = unsafe { msg_send![super(self), unmarkText] };
            self.normalize_committed_text();
            wake();
        }

        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text_replacement_range(&self, text: &AnyObject, range: NSRange) {
            let _: () = unsafe { msg_send![super(self), insertText: text, replacementRange: range] };
            self.normalize_committed_text();
            wake();
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text_selected_range_replacement_range(
            &self,
            text: &AnyObject,
            selected: NSRange,
            replacement: NSRange,
        ) {
            let was_normalizing = self.ivars().normalizing.replace(true);
            let _: () = unsafe {
                msg_send![super(self), setMarkedText: text, selectedRange: selected, replacementRange: replacement]
            };
            self.ivars().normalizing.set(was_normalizing);
            wake();
        }


        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, sender: Option<&AnyObject>) {
            if !self.discard_composition() {
                with_ui_mut(|ui| ui.fold_reply());
            }
        }

        #[unsafe(method(paste:))]
        fn paste(&self, sender: Option<&AnyObject>) {
            let _: () = unsafe { msg_send![super(self), paste: sender] };
            self.normalize_committed_text();
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
    fn handle_edit_shortcut(&self, event: &NSEvent) -> bool {
        let modifiers = event.modifierFlags()
            & (NSEventModifierFlags::Command
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option);
        if modifiers != NSEventModifierFlags::Command || unsafe { msg_send![self, hasMarkedText] } {
            return false;
        }
        let Some(key) = event.charactersIgnoringModifiers() else {
            return false;
        };
        if key.length() != 1 {
            return false;
        }
        let Ok(key) = u8::try_from(key.characterAtIndex(0)) else {
            return false;
        };
        match key.to_ascii_lowercase() {
            b'a' => unsafe { self.selectAll(None) },
            b'c' => unsafe { self.copy(None) },
            b'x' => unsafe { self.cut(None) },
            b'v' => unsafe { msg_send![self, paste: None::<&AnyObject>] },
            _ => return false,
        }
        true
    }

    fn normalize_committed_text(&self) {
        let marked: bool = unsafe { msg_send![self, hasMarkedText] };
        if marked || self.ivars().normalizing.get() {
            return;
        }
        self.ivars().normalizing.set(true);
        // Scan the existing NSString, not a UTF-8 copy of the entire draft.
        let value = self.string();
        let mut breaks = line_break_ranges(&value);
        if !breaks.is_empty() {
            let selection = self.selectedRange();
            let affinity = self.selectionAffinity();
            let storage: Option<&NSTextStorage> = unsafe { msg_send![self, textStorage] };
            if let Some(storage) = storage {
                let space = NSString::from_str(" ");
                for index in (0..breaks.len()).rev() {
                    let range = breaks[index];
                    if self.shouldChangeTextInRange_replacementString(range, Some(&space)) {
                        storage.replaceCharactersInRange_withString(range, &space);
                        self.didChangeText();
                    } else {
                        breaks.remove(index);
                    }
                }
                let adjusted = adjusted_composer_selection(selection, &breaks);
                if !breaks.is_empty()
                    && (self.selectedRange() != adjusted || self.selectionAffinity() != affinity)
                {
                    self.setSelectedRange_affinity_stillSelecting(adjusted, affinity, false);
                }
            }
        }
        self.ivars().normalizing.set(false);
    }

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
        let this = Self::alloc(mtm).set_ivars(ComposerViewIvars {
            normalizing: Cell::new(false),
        });
        unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0), NSSize::new(220.0, BUBBLE_LINE_HEIGHT)
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
    viewport: PresentationViewport,
    scale: f64,
}

#[derive(Clone, Copy)]
struct VisualAnchor {
    key: AnchorKey,
    // Panel-relative so a move translates the last valid attachment.
    relative: BubbleRect,
}

struct CachedAnchor {
    key: AnchorKey,
}

impl AnchorKey {
    fn same_transform(self, other: Self) -> bool {
        self.backend_epoch == other.backend_epoch
            && rect_nearly_equal(self.artwork, other.artwork)
            && self.scale == other.scale
            && self.viewport == other.viewport
    }

    fn same_query(self, other: Self) -> bool {
        self.same_transform(other) && self.frame == other.frame
    }
}

fn visual_relative_for(visual: Option<VisualAnchor>, current: AnchorKey) -> Option<BubbleRect> {
    visual
        .filter(|anchor| anchor.key.same_transform(current))
        .map(|anchor| anchor.relative)
}

// Both the renderer refresh and lifecycle tests use this placement invalidation.
// Comparing raw bounds misses a same-bounds return after a transform change;
// comparing only effective bounds misses a discarded, previously applied visual.
fn accept_visual_anchor(
    previous: Option<VisualAnchor>,
    key: AnchorKey,
    status: SpeechAnchorStatus,
    rig: bool,
) -> (Option<VisualAnchor>, bool) {
    let before = visual_relative_for(previous, key);
    let selected = match status {
        SpeechAnchorStatus::ReadyAnchor(relative)
            if [relative.x, relative.y, relative.width, relative.height]
                .iter()
                .all(|value| value.is_finite())
                && relative.width > 0.0
                && relative.height > 0.0 =>
        {
            Some(VisualAnchor { key, relative })
        }
        SpeechAnchorStatus::TemporarilyUnavailable if rig => {
            previous.filter(|old| old.key.same_transform(key))
        }
        SpeechAnchorStatus::Invalid
        | SpeechAnchorStatus::ReadyAnchorless
        | SpeechAnchorStatus::TemporarilyUnavailable
        | SpeechAnchorStatus::ReadyAnchor(_) => None,
    };
    let invalidated =
        before != visual_relative_for(selected, key) || (previous.is_some() && selected.is_none());
    (selected, invalidated)
}

fn presentation_anchor_status(
    result: &Result<SpeechAnchorSnapshot, String>,
    key: AnchorKey,
    panel_origin: NSPoint,
    rig: bool,
) -> SpeechAnchorStatus {
    match result {
        Ok(snapshot)
            if snapshot.backend_epoch == key.backend_epoch
                && snapshot.viewport == Some(key.viewport)
                && (!rig
                    || (snapshot.input_epoch.is_some() && snapshot.anchor_epoch.is_some())) =>
        {
            match snapshot.status {
                SpeechAnchorStatus::ReadyAnchor(anchor) => {
                    SpeechAnchorStatus::ReadyAnchor(BubbleRect {
                        x: anchor.x - panel_origin.x,
                        y: anchor.y - panel_origin.y,
                        width: anchor.width,
                        height: anchor.height,
                    })
                }
                other => other,
            }
        }
        _ => SpeechAnchorStatus::Invalid,
    }
}

fn anchor_query_needed(cached: Option<&CachedAnchor>, key: AnchorKey, rig: bool) -> bool {
    rig || !cached.is_some_and(|cached| cached.key.same_query(key))
}

fn placed_attached_bubble(
    relative: Option<BubbleRect>,
    pet: BubbleRect,
    body: (f64, f64),
    visible: BubbleRect,
    placement: BubblePlacement,
) -> BubbleGeometry {
    let anchor = relative
        .map(|anchor| BubbleRect {
            x: pet.x + anchor.x,
            y: pet.y + anchor.y,
            width: anchor.width,
            height: anchor.height,
        })
        .unwrap_or(pet);
    place_bubble(anchor, body, visible, placement)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ComposerRenderStamp {
    cards: (Option<u64>, u64, UiLocale, u64),
    locale: UiLocale,
    pending: bool,
    live_revision: Option<u64>,
}

enum WorktreeFeedback {
    Pending(WorktreeRemoveTarget),
    Finished(WorktreeRemoveTarget, Result<(), WorktreeRemoveError>),
}

impl WorktreeFeedback {
    fn compact_summary(&self, locale: UiLocale) -> &'static str {
        let message = match self {
            Self::Pending(_) => Message::WorktreeRemoving,
            Self::Finished(_, Ok(())) => Message::WorktreeRemoved,
            Self::Finished(_, Err(WorktreeRemoveError::UnknownDelivery)) => {
                Message::WorktreeCompactUnknownDelivery
            }
            Self::Finished(_, Err(WorktreeRemoveError::Rejected { .. })) => {
                Message::WorktreeCompactRejected
            }
            Self::Finished(_, Err(WorktreeRemoveError::Other(_))) => Message::WorktreeCompactFailed,
            Self::Finished(_, Err(_)) => Message::WorktreeCompactUnavailable,
        };
        text(locale, message)
    }

    fn full_text(&self, locale: UiLocale) -> String {
        let (target, message) = match self {
            Self::Pending(target) => (target, text(locale, Message::WorktreeRemoving).to_owned()),
            Self::Finished(target, Ok(())) => {
                (target, text(locale, Message::WorktreeRemoved).to_owned())
            }
            Self::Finished(target, Err(error)) => (target, worktree_error_text(locale, error)),
        };
        format!(
            "{} ({}): {message}",
            worktree_display_name(target),
            target.worktree.checkout_path
        )
    }
}

fn worktree_feedback_detail(
    feedback: Option<&WorktreeFeedback>,
    locale: UiLocale,
    offline: &str,
) -> String {
    match feedback {
        Some(feedback) if !offline.is_empty() => {
            format!("{}\n{offline}", feedback.full_text(locale))
        }
        Some(feedback) => feedback.full_text(locale),
        None => offline.to_owned(),
    }
}

fn compact_worktree_secondary(summary: &str, offline: &str) -> String {
    if summary.is_empty() {
        offline.to_owned()
    } else if offline.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary}\n{offline}")
    }
}

fn worktree_menu_key(
    header_key: Option<SessionKey>,
    background: bool,
    selected_key: Option<SessionKey>,
) -> Option<SessionKey> {
    if background {
        selected_key
    } else {
        header_key
    }
}

fn worktree_display_name(target: &WorktreeRemoveTarget) -> String {
    let path = &target.worktree.checkout_path;
    let checkout = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    format!("{} / {checkout}", target.worktree.repo_name)
}

fn worktree_error_text(locale: UiLocale, error: &WorktreeRemoveError) -> String {
    let message = match error {
        WorktreeRemoveError::Busy => Message::WorktreeBusy,
        WorktreeRemoveError::Offline => Message::WorktreeOffline,
        WorktreeRemoveError::ReadOnly => Message::WorktreeReadOnly,
        WorktreeRemoveError::StaleTarget => Message::WorktreeStale,
        WorktreeRemoveError::NotLinkedWorktree => Message::WorktreeNotLinked,
        WorktreeRemoveError::Unsupported => Message::WorktreeUnsupported,
        WorktreeRemoveError::UnknownDelivery => Message::WorktreeUnknownDelivery,
        WorktreeRemoveError::Rejected { .. } => Message::WorktreeRejected,
        WorktreeRemoveError::Other(_) => Message::WorktreeFailed,
    };
    match error {
        WorktreeRemoveError::Rejected { code, message } => {
            format!(
                "{} ({code}: {message})",
                text(locale, Message::WorktreeRejected)
            )
        }
        WorktreeRemoveError::Other(detail) => format!("{}: {detail}", text(locale, message)),
        _ => text(locale, message).to_owned(),
    }
}
struct Ui {
    mtm: MainThreadMarker,
    shared: Arc<Mutex<AppState>>,
    packs: Arc<PackService>,
    automation: SharedAutomation,
    saved_presentation: PresentationTarget,
    presentation_patch: Option<PresentationPatch>,
    presentation_reset: bool,
    presentation_operation: Option<String>,
    checkpoint_save: Option<Result<PresentationTarget, (PresentationTarget, String)>>,
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
    reply_container: Retained<NSView>,
    composer_scroll: Retained<NSScrollView>,
    composer_view: Retained<ComposerView>,
    composer_metrics: ComposerMetrics,
    composer_readonly: bool,
    composer_status: Retained<NSTextField>,
    composer_send: Retained<NSButton>,
    prompt_sender: PromptSender,
    worktree_sender: WorktreeRemoveSender,
    worktree_automation: automation_worktrees::WorktreeAutomation,
    worktree_confirming: bool,
    worktree_feedback: Option<WorktreeFeedback>,
    composer_render_stamp: Option<ComposerRenderStamp>,
    composer_key: Option<SessionKey>,
    reply_open: bool,
    composer_pending_key: Option<SessionKey>,
    composer_drafts: VecDeque<(SessionKey, String)>,
    composer_results: VecDeque<(SessionKey, String)>,
    cards: SessionCards,
    menu_panel: MenuPanel,
    dialogue_editor: DialogueEditor,
    dialogue_automation: automation_dialogues::DialogueAutomation,
    dialogue_choices: Vec<DialogueChoice>,
    editor_cached_choice: Option<DialogueChoice>,
    editor_metadata: Option<CharacterMetadata>,
    editor_ready: bool,
    editor_error: Option<String>,
    editor_content_dirty: bool,
    external_dialogue_target: Option<DialogueTarget>,
    external_dialogue_metadata: Option<CharacterMetadata>,
    locale: UiLocale,
    effective_language: LanguagePreference,
    preference_revision: u64,
    native_show_status_indicators: bool,
    native_menu_bar_mode: MenuBarMode,
    pending_preference_operations: Vec<automation_preferences::PendingPreferenceOperation>,
    cli_preference_feedback: Option<String>,
    pending_language: Option<(LanguagePreference, UiLocale)>,
    effective_dialogue: CharacterMetadata,
    dialogue_target: Option<DialogueTarget>,
    dialogue_override_active: bool,
    dialogue_prepared_epoch: u64,
    active: PreparedCharacter,
    display_geometry: DisplayGeometry,
    bubble_appearance: BubbleAppearance,
    native_bubble_appearance: BubbleAppearance,
    cached_anchor: Option<CachedAnchor>,
    visual_anchor: Option<VisualAnchor>,
    displayed_frame: Option<FrameId>,
    viewport: PresentationViewport,
    playback: Playback,
    pending_native: Option<PendingNative>,
    character_menu: CharacterMenu,
    character_selection: CharacterSelection,
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
    // The last successfully applied geometry can be tailless and still attached.
    bubble_geometry_attached: bool,
    pending_standalone_body_origin: Option<(f64, f64)>,
    standalone_reset_pending: bool,
    standalone_position_unsaved: bool,
    bubble_content_dirty: bool,
    pending_bubble_scene: Option<Scene>,
    pending_bubble_content: bool,
    bubble_layout_dirty: bool,
    transition_generation: u64,
    bubble_fade: Option<BubbleFade>,
    reduced_motion: bool,
    _status_item: Retained<NSStatusItem>,
    status_menu: Retained<NSMenu>,
    menu_bar_icon_image: Retained<NSImage>,
    menu_bar_icon_busy: bool,
    status_menu_tracking: bool,
    context_anchor: Option<NSRect>,
    settings_anchor: Option<NSRect>,
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
                if ui.pending_language.is_some() {
                    ui.apply_pending_language();
                }
                if ui.pending_bubble_content || ui.pending_bubble_scene.is_some() {
                    ui.apply_pending_bubble_updates();
                }
                if ui.has_pending_native_dialogue() {
                    if ui.bubble_content_dirty
                        && !ui.bubble_content_tracking_locked()
                        && !ui.composer_marked()
                        && !ui.cards.is_composing()
                    {
                        let current = ui.shared.lock().ok().map(|state| state.scene());
                        if current.as_ref().is_some_and(|scene| {
                            !scene.shutdown
                                && presentation_matches_scene(scene, &ui.last_scene)
                                && !status_fields_changed(scene, &ui.last_scene)
                        }) {
                            let scene = ui.last_scene.clone();
                            ui.refresh_bubble_content(&scene);
                        }
                    }
                    ui.settle_pending_native_dialogues();
                }
                if ui.dialogue_editor.is_visible()
                    && ui.dialogue_editor.needs_initial_hydration()
                    && ui.editor_content_dirty
                    && !ui.dialogue_editor.has_marked_text()
                {
                    ui.sync_dialogue_editor_content(false);
                }
                if ui.menu_panel.is_visible() {
                    ui.menu_panel.refresh_bubble_color_controls();
                }
                if !ui.pending_preference_operations.is_empty() {
                    ui.settle_preference_operations();
                }
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

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.show_character_context_menu(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                self.show_character_context_menu(event);
                return;
            }
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
            with_ui_mut(|ui| {
                let scene = ui.last_scene.clone();
                ui.update_pointer_policy(&scene);
                ui.publish_presentation_checkpoint(&scene);
            });
            wake();
        }

        #[unsafe(method(mouseCancelled:))]
        fn mouse_cancelled(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) { return; }
            let drag = self.ivars().drag.get();
            with_ui_mut(|ui| {
                ui.cancel_gesture_for(drag, true);
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
    // - BubblePanel is main-thread-only and forwards unhandled pointer events
    //   and all non-Escape key events to NSPanel's responder chain.
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

        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            let context_click = event.r#type() == NSEventType::RightMouseDown
                || (event.r#type() == NSEventType::LeftMouseDown
                    && event.modifierFlags().contains(NSEventModifierFlags::Control));
            if context_click && self.show_bubble_context_menu(event) {
                return;
            }
            let _: () = unsafe { msg_send![super(self), sendEvent: event] };
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            if self.search_edit_shortcut(event) {
                return true.into();
            }
            unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if self.isKeyWindow() && self.search_edit_shortcut(event) {
                return;
            }
            if self.isKeyWindow() && event.keyCode() == 53 {
                if with_ui_read(|ui| ui.cards.search_has_focus()).unwrap_or(false) {
                    with_ui_mut(|ui| ui.cards.search_escape());
                } else if self.composing_editor().is_some() {
                    let _: () = unsafe { msg_send![super(self), keyDown: event] };
                } else {
                    with_ui_mut(|ui| ui.escape_reply_or_collapse());
                }
                return;
            }
            let _: () = unsafe { msg_send![super(self), keyDown: event] };
        }
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, sender: Option<&AnyObject>) {
            if self.isKeyWindow() {
                if with_ui_read(|ui| ui.cards.search_has_focus()).unwrap_or(false) {
                    with_ui_mut(|ui| ui.cards.search_escape());
                } else if let Some(editor) = self.composing_editor() {
                    let _: () = unsafe { msg_send![editor, cancelOperation: sender] };
                } else {
                    with_ui_mut(|ui| ui.escape_reply_or_collapse());
                }
            } else {
                let _: () = unsafe { msg_send![super(self), cancelOperation: sender] };
            }
        }
    }
);
impl BubblePanel {
    fn search_edit_shortcut(&self, event: &NSEvent) -> bool {
        if !self.isKeyWindow() {
            return false;
        }
        let modifiers = event.modifierFlags()
            & (NSEventModifierFlags::Command
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option);
        if modifiers != NSEventModifierFlags::Command {
            return false;
        }
        let Some(key) = event.charactersIgnoringModifiers() else {
            return false;
        };
        if key.length() != 1 {
            return false;
        }
        let Ok(key) = u8::try_from(key.characterAtIndex(0)) else {
            return false;
        };
        if !matches!(key.to_ascii_lowercase(), b'a' | b'c' | b'x' | b'v') {
            return false;
        }
        let Some(editor) =
            with_ui_read(|ui| ui.cards.search_editor().map(|editor| editor.retain())).flatten()
        else {
            return false;
        };
        let marked: bool = unsafe { msg_send![&*editor, hasMarkedText] };
        if marked {
            return false;
        }
        match key.to_ascii_lowercase() {
            b'a' => {
                let _: () = unsafe { msg_send![&*editor, selectAll: None::<&AnyObject>] };
            }
            b'c' => {
                let _: () = unsafe { msg_send![&*editor, copy: None::<&AnyObject>] };
            }
            b'x' => {
                let _: () = unsafe { msg_send![&*editor, cut: None::<&AnyObject>] };
            }
            b'v' => {
                let _: () = unsafe { msg_send![&*editor, paste: None::<&AnyObject>] };
            }
            _ => unreachable!(),
        }
        true
    }

    fn composing_editor(&self) -> Option<&AnyObject> {
        let responder: Option<&AnyObject> = unsafe { msg_send![self, firstResponder] };
        responder.filter(|responder| {
            let is_editor: bool =
                unsafe { msg_send![*responder, isKindOfClass: ComposerView::class()] };
            is_editor && unsafe { msg_send![*responder, hasMarkedText] }
        })
    }
    fn show_bubble_context_menu(&self, event: &NSEvent) -> bool {
        let Some(root) = self.contentView() else {
            return false;
        };
        let location = event.locationInWindow();
        let Some(hit) = root.hitTest(location) else {
            return false;
        };
        let is_background = hit.downcast_ref::<BubbleView>().is_some();
        let header_key = card_header_key_at_hit(&hit, location);
        if !is_background && header_key.is_none() {
            return false;
        }
        // Tracking invokes actions synchronously: snapshot and release the UI borrow first.
        let Some((locale, visible, composing, target, mtm, worktree)) = with_ui_read(|ui| {
            // Resolve only the card captured by this opening event, never a later selection.
            let selected = if is_background {
                ui.cards.selected_target().map(|(key, _)| key)
            } else {
                None
            };
            let key = worktree_menu_key(header_key, is_background, selected);
            let worktree = key.and_then(|key| {
                if ui.worktree_confirming || ui.worktree_sender.is_pending() {
                    return None;
                }
                ui.shared.lock().ok()?.worktree_remove_target(&key).ok()
            });
            (
                ui.locale,
                ui.last_scene.visible,
                ui.composer_marked() || ui.cards.is_composing(),
                ui._menu_target.clone(),
                ui.mtm,
                worktree,
            )
        }) else {
            return false;
        };
        let anchor = NSRect::new(self.convertPointToScreen(location), NSSize::new(1.0, 1.0));
        with_ui_mut(|ui| ui.context_anchor = Some(anchor));
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
        menu.setAutoenablesItems(false);
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            Message::ContextSettings,
            sel!(openContextSettings:),
            !composing,
        );
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            if visible {
                Message::ContextHideCharacter
            } else {
                Message::ContextShowCharacter
            },
            if visible { sel!(hide:) } else { sel!(show:) },
            true,
        );
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            Message::CloseBubbleWindow,
            sel!(closeBubbleWindow:),
            !composing,
        );
        if let Some(worktree) = worktree {
            let title = format!(
                "{} · {}",
                text(locale, Message::WorktreeRemove),
                worktree_display_name(&worktree)
            );
            let worktree_target = WorktreeMenuTarget::new(worktree, mtm);
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&title),
                    Some(sel!(removeWorktree:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { item.setTarget(Some(&*worktree_target)) };
            item.setEnabled(!composing);
            menu.addItem(&item);
            // NSMenuItem does not own its target; keep it alive through synchronous tracking.
            NSMenu::popUpContextMenu_withEvent_forView(&menu, event, &root);
            drop(worktree_target);
            return true;
        }
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, &root);
        true
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
            with_ui_mut(|ui| {
                let scene = ui.last_scene.clone();
                ui.update_pointer_policy(&scene);
                ui.publish_presentation_checkpoint(&scene);
            });
            wake();
        }

        #[unsafe(method(mouseCancelled:))]
        fn mouse_cancelled(&self, event: &NSEvent) {
            if !pointer_event_allowed(event, false) {
                return;
            }
            let drag = self.ivars().drag.get();
            with_ui_mut(|ui| ui.cancel_gesture_for(drag, true));
            self.ivars().drag.set(None);
            self.set_drag_visuals(false);
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

struct WorktreeMenuTargetIvars {
    target: WorktreeRemoveTarget,
}

define_class!(
    // SAFETY: Main-thread-only NSObject retains an immutable menu-opening descriptor.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetWorktreeMenuTarget"]
    #[ivars = WorktreeMenuTargetIvars]
    struct WorktreeMenuTarget;

    unsafe impl NSObjectProtocol for WorktreeMenuTarget {}

    impl WorktreeMenuTarget {
        #[unsafe(method(removeWorktree:))]
        fn remove_worktree(&self, _sender: Option<&AnyObject>) {
            let target = self.ivars().target.clone();
            // Tracking invokes this action inside a nested run loop. Confirm only after
            // the menu has returned, without retaining any Ui borrow across the modal.
            DispatchQueue::main().exec_async(move || begin_worktree_remove(target));
        }
    }
);

impl WorktreeMenuTarget {
    fn new(target: WorktreeRemoveTarget, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WorktreeMenuTargetIvars { target });
        unsafe { msg_send![super(this), init] }
    }
}

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

        #[unsafe(method(closeBubbleWindow:))]
        fn close_bubble_window(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| {
                if ui.composer_marked() || ui.cards.is_composing() {
                    return;
                }
                if ui.apply_control("hide_bubble").is_err() {
                    ui.refresh();
                }
            });
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

        #[unsafe(method(setMenuBarMode:))]
        fn set_menu_bar_mode(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let Some(item) = popup.selectedItem() else {
                return;
            };
            let mode = match item.tag() {
                0 => MenuBarMode::Always,
                1 => MenuBarMode::RecoveryOnly,
                _ => return,
            };
            with_ui_mut(|ui| ui.set_menu_bar_mode(mode));
        }

        #[unsafe(method(chooseMenuBarIcon:))]
        fn choose_menu_bar_icon(&self, _sender: Option<&AnyObject>) {
            choose_menu_bar_icon();
        }

        #[unsafe(method(resetMenuBarIcon:))]
        fn reset_menu_bar_icon(&self, _sender: Option<&AnyObject>) {
            reset_menu_bar_icon();
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

        #[unsafe(method(showObservationHelp:))]
        fn show_observation_help(&self, _sender: Option<&AnyObject>) {
            let Some((mtm, locale)) = with_ui_read(|ui| (ui.mtm, ui.locale)) else { return };
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(text(locale, Message::ObservationHelpTitle)));
            alert.setInformativeText(&NSString::from_str(text(locale, Message::ObservationHelpDetails)));
            alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Close)));
            let _ = alert.runModal();
        }

        #[unsafe(method(toggleObservationCatalogError:))]
        fn toggle_observation_catalog_error(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.menu_panel.toggle_observation_error(None));
        }

        #[unsafe(method(toggleObservationMachineError:))]
        fn toggle_observation_machine_error(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else { return };
            let Some(id) = button.identifier().map(|id| id.to_string()) else { return };
            with_ui_mut(|ui| ui.menu_panel.toggle_observation_error(Some(&id)));
        }

        #[unsafe(method(copyObservationReconnect:))]
        fn copy_observation_reconnect(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else { return };
            let Some(id) = button.identifier().map(|id| id.to_string()) else { return };
            let Some(command) = with_ui_read(|ui| ui.menu_panel.observation_reconnect_command(&id)).flatten() else { return };
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            let success = pasteboard.setString_forType(
                &NSString::from_str(&command),
                unsafe { NSPasteboardTypeString },
            );
            with_ui_mut(|ui| ui.menu_panel.observation_copy_feedback(&id, success));
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
            with_ui_mut(|ui| ui.commit_bubble_appearance(BubbleAppearance::default(), true));
        }

        #[unsafe(method(reloadBubbleColors:))]
        fn reload_bubble_colors(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| { ui.menu_panel.reload_bubble_colors(); });
        }

        #[unsafe(method(rebaseBubbleColors:))]
        fn rebase_bubble_colors(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| { ui.menu_panel.rebase_bubble_colors(); });
        }

        #[unsafe(method(openContextSettings:))]
        fn open_context_settings(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| {
                if !ui.composer_marked() && !ui.cards.is_composing() {
                    if let Some(anchor) = ui.context_anchor {
                        ui.open_settings_at(anchor);
                    }
                }
            });
        }

        #[unsafe(method(activateStatusItem:))]
        fn activate_status_item(&self, _sender: Option<&AnyObject>) {
            let Some((button, event, action)) = with_ui_read(|ui| {
                let button = ui._status_item.button(ui.mtm)?;
                let event = NSApplication::sharedApplication(ui.mtm).currentEvent();
                let action = status_item_action(event.as_deref().filter(|event| {
                    matches!(
                        event.r#type(),
                        NSEventType::LeftMouseUp | NSEventType::RightMouseUp
                    )
                }).map(|event| {
                    let same_window = event.window(ui.mtm).is_some_and(|window| {
                        button.window().is_some_and(|status_window| {
                            window.windowNumber() == status_window.windowNumber()
                        })
                    });
                    StatusItemEvent {
                        kind: event.r#type(),
                        modifiers: event.modifierFlags(),
                        age: NSProcessInfo::processInfo().systemUptime() - event.timestamp(),
                        same_window,
                        hits_button: same_window
                            && point_in_rect(
                                event.locationInWindow(),
                                button.convertRect_toView(button.bounds(), None),
                            ),
                    }
                }));
                Some((button, event, action))
            }).flatten() else { return };
            let Some(anchor) = status_button_rect(&button) else {
                return;
            };

            match action {
                StatusItemAction::Primary => with_ui_mut(|ui| {
                    if !ui.composer_marked() && !ui.cards.is_composing() {
                        ui.open_settings_at(anchor);
                    }
                }),
                StatusItemAction::Context => {
                    let Some(event) = event else { return };
                    let mut menu = None;
                    with_ui_mut(|ui| {
                        let context_menu = ui.status_menu.clone();
                        if let Some(settings) = context_menu.itemAtIndex(1) {
                            settings.setEnabled(!ui.composer_marked() && !ui.cards.is_composing());
                        }
                        ui.menu_panel.hide();
                        if let Ok(state) = ui.shared.lock() {
                            ui.sync_recover_item(&state.scene());
                        }
                        ui.context_anchor = Some(anchor);
                        ui.status_menu_tracking = true;
                        menu = Some(context_menu);
                    });
                    let Some(menu) = menu else { return };
                    NSMenu::popUpContextMenu_withEvent_forView(&menu, &event, &button);
                    with_ui_mut(|ui| {
                        ui.status_menu_tracking = false;
                        ui.refresh();
                        if let Ok(state) = ui.shared.lock() {
                            ui.sync_status_menu(&state.scene());
                        }
                    });
                }
            }
        }

        #[unsafe(method(recoverCharacterInteraction:))]
        fn recover_character_interaction(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| {
                if let Ok(mut state) = ui.shared.lock() {
                    state.recover_interaction();
                }
                ui.refresh();
            });
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
            with_ui_mut(|ui| ui.dialogue_editor.select_locale(locale));
        }

        #[unsafe(method(setDialogueSlot:))]
        fn set_dialogue_slot(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else {
                return;
            };
            let Ok(index) = usize::try_from(button.tag()) else { return };
            let Some(&slot) = DialogueSlot::ALL.get(index) else { return };
            with_ui_mut(|ui| ui.dialogue_editor.select_slot(slot));
        }

        #[unsafe(method(openDialogueEditor:))]
        fn open_dialogue_editor(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| {
                let listing = ui.packs.cached_list();
                ui.update_dialogue_choices(&listing);
                ui.menu_panel.hide();
                ui.dialogue_editor.show();
                ui.sync_dialogue_editor_content(true);
            });
        }

        #[unsafe(method(setDialogueTarget:))]
        fn set_dialogue_target(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else { return };
            let Ok(index) = usize::try_from(popup.indexOfSelectedItem()) else { return };
            with_ui_mut(|ui| {
                let previous = ui.dialogue_editor.choice();
                ui.dialogue_editor.select_target(index);
                let changed = ui.dialogue_editor.choice() != previous;
                ui.sync_dialogue_editor_content(changed);
            });
        }

        #[unsafe(method(toggleDialogueOriginal:))]
        fn toggle_dialogue_original(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.dialogue_editor.toggle_original());
        }

        #[unsafe(method(reloadDialogueDraft:))]
        fn reload_dialogue_draft(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| { ui.dialogue_editor.reload_saved_draft(); });
        }

        #[unsafe(method(rebaseDialogueDraft:))]
        fn rebase_dialogue_draft(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| { ui.dialogue_editor.rebase_draft(); });
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

        #[unsafe(method(packApply:))]
        fn pack_apply(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.apply_candidate());
        }

        #[unsafe(method(packCancelSelection:))]
        fn pack_cancel_selection(&self, _sender: Option<&AnyObject>) {
            with_ui_mut(|ui| ui.cancel_candidate());
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

    fn show_character_context_menu(&self, event: &NSEvent) {
        if !pointer_event_allowed(event, true) || self.ivars().drag.get().is_some() {
            return;
        }
        let local = event.locationInWindow();
        if grip_hit_test(local, self.bounds().size) {
            return;
        }
        let Some(window) = self.window() else { return };
        let anchor = NSRect::new(window.convertPointToScreen(local), NSSize::new(1.0, 1.0));
        let Some((locale, bubble_visible, composing, target, mtm)) = with_ui_read(|ui| {
            if !ui.last_scene.visible
                || ui.last_scene.passthrough
                || !ui.character_hit(local).is_some_and(|hit| hit.opaque)
            {
                return None;
            }
            Some((
                ui.locale,
                ui.last_scene.bubble_visible,
                ui.composer_marked(),
                ui._menu_target.clone(),
                ui.mtm,
            ))
        })
        .flatten() else {
            return;
        };
        with_ui_mut(|ui| ui.context_anchor = Some(anchor));
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
        menu.setAutoenablesItems(false);
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            Message::ContextSettings,
            sel!(openContextSettings:),
            !composing,
        );
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            if bubble_visible {
                Message::ContextHideBubble
            } else {
                Message::ContextShowBubble
            },
            if bubble_visible {
                sel!(closeBubbleWindow:)
            } else {
                sel!(showBubble:)
            },
            !bubble_visible || !composing,
        );
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            Message::ContextHideCharacter,
            sel!(hide:),
            true,
        );
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_context_item(
            &menu,
            mtm,
            &target,
            locale,
            Message::MenuQuit,
            sel!(quit:),
            true,
        );
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
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

#[cfg(test)]
thread_local! {
    pub(crate) static WAKE_OBSERVER: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

pub fn wake() {
    #[cfg(test)]
    WAKE_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow().as_ref() {
            observer();
        }
    });
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
    timeout: Duration,
) -> Result<RendererToken, String> {
    let deadline = Instant::now() + timeout;
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
    let timeout = character_renderer::preparation_timeout(&assets)
        + character_renderer::SURFACE_PREPARATION_TIMEOUT;
    let (sender, receiver) = mpsc::sync_channel(1);
    enqueue_bridge(DeferredBridge::Prepare {
        token,
        assets: Box::new(assets),
        cancel: Arc::clone(&cancel),
        sender,
    });
    wait_for_main_result(receiver, &cancel, timeout)
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
    wait_for_main_result(
        receiver,
        &cancel,
        character_renderer::SURFACE_PREPARATION_TIMEOUT,
    )
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

fn selection_error_message(error: SelectionError) -> Message {
    match error {
        SelectionError::NoListing | SelectionError::Unavailable => {
            Message::CharacterOperationUnavailable
        }
        SelectionError::Busy => Message::CharacterOperationQueued,
        SelectionError::NoCandidate => Message::CharacterSelectionPrompt,
        SelectionError::Stale => Message::CharacterSelectionStale,
        SelectionError::AlreadyApplied => Message::CharacterOperationCompleted,
        SelectionError::InvalidOperationId => Message::CharacterOperationUnknown,
    }
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
fn add_context_item(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    target: &MenuTarget,
    locale: UiLocale,
    title: Message,
    action: objc2::runtime::Sel,
    enabled: bool,
) {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(text(locale, title)),
            Some(action),
            &NSString::from_str(""),
        )
    };
    unsafe { item.setTarget(Some(target)) };
    item.setEnabled(enabled);
    menu.addItem(&item);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusItemAction {
    Primary,
    Context,
}

struct StatusItemEvent {
    kind: NSEventType,
    modifiers: NSEventModifierFlags,
    age: f64,
    same_window: bool,
    hits_button: bool,
}

fn status_item_action(event: Option<StatusItemEvent>) -> StatusItemAction {
    let Some(event) = event else {
        return StatusItemAction::Primary;
    };
    if event.age.abs() >= 2.0 || !event.age.is_finite() || !event.same_window || !event.hits_button
    {
        return StatusItemAction::Primary;
    }
    match event.kind {
        NSEventType::RightMouseUp => StatusItemAction::Context,
        NSEventType::LeftMouseUp if event.modifiers.contains(NSEventModifierFlags::Control) => {
            StatusItemAction::Context
        }
        _ => StatusItemAction::Primary,
    }
}

fn status_button_rect(button: &NSView) -> Option<NSRect> {
    let window = button.window()?;
    Some(window.convertRectToScreen(button.convertRect_toView(button.bounds(), None)))
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

pub(crate) fn composer_is_composing() -> bool {
    UI.with(|cell| match cell.try_borrow() {
        Ok(slot) => slot
            .as_ref()
            .is_some_and(|ui| ui.composer_marked() || ui.cards.is_composing()),
        Err(_) => {
            wake();
            true
        }
    })
}

pub(crate) fn cards_content_changed() {
    with_ui_mut(|ui| {
        ui.cards.set_composition_active(ui.composer_marked());
        ui.composer_render_stamp = None;
        ui.sync_composer();
        if ui.bubble_mode == BubbleMode::Expanded {
            ui.bubble_content_dirty = true;
            let scene = ui.last_scene.clone();
            ui.refresh_bubble_content(&scene);
        }
    });
}

pub(crate) fn cards_options_changed() {
    with_ui_mut(|ui| {
        ui.cards.set_composition_active(ui.composer_marked());
        ui.cards.sync_search_composition();
        if ui.composer_marked() || ui.cards.is_composing() {
            return;
        }
        ui.apply_pending_cards_options();
        ui.composer_render_stamp = None;
        ui.sync_composer();
        if ui.bubble_mode == BubbleMode::Expanded {
            ui.bubble_content_dirty = true;
            let scene = ui.last_scene.clone();
            ui.refresh_bubble_content(&scene);
        }
    });
}

pub(crate) fn cards_selection_changed() {
    with_ui_mut(|ui| {
        if ui.bubble_mode != BubbleMode::Expanded || ui.composer_marked() || ui.cards.is_composing()
        {
            return;
        }
        ui.reply_open = true;
        ui.composer_render_stamp = None;
        ui.sync_composer();
        ui.bubble_content_dirty = true;
        let scene = ui.last_scene.clone();
        ui.refresh_bubble_content(&scene);
        if ui.reply_open {
            if ui.composer_input_visible() {
                ui.bubble_panel.makeKeyAndOrderFront(None);
                let _ = ui.bubble_panel.makeFirstResponder(Some(&ui.composer_view));
            } else if ui.bubble_panel.isKeyWindow() && !ui.composer_marked() {
                let _ = ui.bubble_panel.makeFirstResponder(None);
            }
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

// Local monitors retain their handler. Drop the token before interpreting a
// modal answer so neither its alert controls nor its window outlive the modal.
struct AlertEscapeMonitor {
    token: Retained<AnyObject>,
    _main_thread: MainThreadMarker,
}

impl Drop for AlertEscapeMonitor {
    fn drop(&mut self) {
        unsafe { NSEvent::removeMonitor(&*self.token) };
    }
}

fn install_alert_escape_monitor(
    mtm: MainThreadMarker,
    alert: &NSAlert,
    cancel: &NSButton,
) -> Option<AlertEscapeMonitor> {
    let window = alert.window();
    let cancel = objc2::Message::retain(cancel);
    let app = NSApplication::sharedApplication(mtm);
    let local: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> =
        RcBlock::new(move |event: NonNull<NSEvent>| {
            let pointer = event.as_ptr();
            let event = unsafe { event.as_ref() };
            let modifiers = NSEventModifierFlags::Command
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Shift;
            if event.keyCode() == 53
                && !event.modifierFlags().intersects(modifiers)
                && event
                    .window(mtm)
                    .is_some_and(|current| current.windowNumber() == window.windowNumber())
                && window.isKeyWindow()
                && app
                    .modalWindow()
                    .is_some_and(|current| current.windowNumber() == window.windowNumber())
            {
                unsafe { cancel.performClick(None) };
                std::ptr::null_mut()
            } else {
                pointer
            }
        });
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &*local) }
        .map(|token| AlertEscapeMonitor {
            token,
            _main_thread: mtm,
        })
}

fn begin_worktree_remove(target: WorktreeRemoveTarget) {
    let mut reserved = false;
    with_ui_mut(|ui| {
        if ui.worktree_confirming || ui.worktree_sender.is_pending() || ui.composer_marked() {
            return;
        }
        if !ui.worktree_target_matches(&target) {
            ui.set_worktree_feedback(WorktreeFeedback::Finished(
                target.clone(),
                Err(WorktreeRemoveError::StaleTarget),
            ));
            return;
        }
        ui.worktree_confirming = true;
        reserved = true;
    });
    if !reserved {
        return;
    }
    let Some((mtm, locale, valid)) = with_ui_read(|ui| {
        (
            ui.mtm,
            ui.locale,
            !ui.composer_marked() && ui.worktree_target_matches(&target),
        )
    }) else {
        with_ui_mut(|ui| ui.worktree_confirming = false);
        return;
    };
    if !valid {
        with_ui_mut(|ui| {
            ui.worktree_confirming = false;
            if !ui.composer_marked() {
                ui.set_worktree_feedback(WorktreeFeedback::Finished(
                    target.clone(),
                    Err(WorktreeRemoveError::StaleTarget),
                ));
            }
        });
        return;
    }
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(
        locale,
        Message::WorktreeRemoveConfirm,
    )));
    alert.setInformativeText(&NSString::from_str(&worktree_remove_confirmation(
        locale,
        &target.worktree.repo_name,
        &target.worktree.checkout_path,
        target.worktree.tab_count,
        target.worktree.pane_count,
    )));
    let cancel = alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
    alert.addButtonWithTitle(&NSString::from_str(text(
        locale,
        Message::WorktreeRemoveAction,
    )));
    // Keep Cancel as the default for Return, including when Delete has focus.
    alert.layout();
    let Some(cell) = cancel
        .cell()
        .and_then(|cell| cell.downcast::<NSButtonCell>().ok())
    else {
        with_ui_mut(|ui| {
            ui.worktree_confirming = false;
            ui.set_worktree_feedback(WorktreeFeedback::Finished(
                target,
                Err(WorktreeRemoveError::Other(
                    text(locale, Message::WorktreeConfirmUnavailable).to_owned(),
                )),
            ));
        });
        return;
    };
    alert.window().setDefaultButtonCell(Some(&cell));
    let Some(monitor) = install_alert_escape_monitor(mtm, &alert, &cancel) else {
        with_ui_mut(|ui| {
            ui.worktree_confirming = false;
            ui.set_worktree_feedback(WorktreeFeedback::Finished(
                target,
                Err(WorktreeRemoveError::Other(
                    text(locale, Message::WorktreeConfirmUnavailable).to_owned(),
                )),
            ));
        });
        return;
    };
    let answer = alert.runModal();
    drop(monitor);
    with_ui_mut(|ui| {
        ui.worktree_confirming = false;
        if answer != NSAlertSecondButtonReturn || ui.composer_marked() {
            return;
        }
        if !ui.worktree_target_matches(&target) {
            ui.set_worktree_feedback(WorktreeFeedback::Finished(
                target.clone(),
                Err(WorktreeRemoveError::StaleTarget),
            ));
            return;
        }
        let feedback = match ui
            .worktree_sender
            .submit(target.clone(), RequestOrigin::Gui)
        {
            Ok(()) => WorktreeFeedback::Pending(target),
            Err(error) => WorktreeFeedback::Finished(target, Err(error)),
        };
        ui.set_worktree_feedback(feedback);
    });
}

fn begin_menu_bar_icon_operation(
    hide_settings: bool,
) -> Option<(MainThreadMarker, UiLocale, Option<NSRect>)> {
    let mut started = None;
    with_ui_mut(|ui| {
        if ui.menu_bar_icon_busy
            || !ui.menu_panel.is_visible()
            || !ui.menu_bar_icon_operation_allowed()
        {
            return;
        }
        ui.menu_bar_icon_busy = true;
        ui.menu_panel.set_menu_bar_icon_busy(true);
        let anchor = if hide_settings {
            ui.settings_anchor
        } else {
            None
        };
        if hide_settings {
            ui.menu_panel.hide();
        }
        started = Some((ui.mtm, ui.locale, anchor));
    });
    started
}

fn choose_menu_bar_icon() {
    let Some((mtm, locale, anchor)) = begin_menu_bar_icon_operation(true) else {
        return;
    };
    // No UI RefCell borrow is held across the modal loop or image decoding/publishing.
    let result = match choose_menu_bar_icon_source(mtm, locale) {
        Ok(Some(path)) => {
            if with_ui_read(|ui| ui.menu_bar_icon_busy && ui.menu_bar_icon_operation_allowed())
                != Some(true)
            {
                None
            } else {
                Some(
                    menu_bar_icon::prepare_source(&path, mtm)
                        .and_then(|prepared| {
                            prepared.publish()?;
                            Ok((prepared.preference, prepared.image))
                        })
                        .map_err(|error| (Message::MenuBarIconImportFailure, error)),
                )
            }
        }
        Ok(None) => None,
        Err(error) => Some(Err((Message::MenuBarIconImportFailure, error))),
    };
    with_ui_mut(|ui| {
        if !ui.menu_bar_icon_busy {
            return;
        }
        if ui.menu_bar_icon_operation_allowed() {
            if let Some(result) = result {
                match result {
                    Ok((preference, image)) => ui.commit_menu_bar_icon(preference, image),
                    Err((summary, detail)) => ui.show_menu_bar_icon_error(summary, &detail),
                }
            }
        }
        ui.menu_bar_icon_busy = false;
        ui.menu_panel.set_menu_bar_icon_busy(false);
        if !ui.menu_bar_icon_shutdown() {
            if let Some(anchor) = anchor {
                ui.open_settings_at(anchor);
            }
        }
    });
}

fn reset_menu_bar_icon() {
    let Some((mtm, locale, _)) = begin_menu_bar_icon_operation(false) else {
        return;
    };
    let image = menu_bar_icon::default_image(mtm, text(locale, Message::MenuBarMenuTitle));
    with_ui_mut(|ui| {
        if !ui.menu_bar_icon_busy {
            return;
        }
        if ui.menu_bar_icon_operation_allowed() {
            match image {
                Ok(image) => ui.commit_default_menu_bar_icon(image),
                Err(error) => {
                    ui.show_menu_bar_icon_error(Message::MenuBarIconImportFailure, &error)
                }
            }
        }
        ui.menu_bar_icon_busy = false;
        ui.menu_panel.set_menu_bar_icon_busy(false);
    });
}

fn choose_menu_bar_icon_source(
    mtm: MainThreadMarker,
    locale: UiLocale,
) -> Result<Option<PathBuf>, String> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    // This file-extension filter is supported on the app's oldest macOS target;
    // the decoder still validates the actual bytes and image limits.
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&NSArray::arrayWithObject(&*NSString::from_str("png"))));
    panel.setTitle(Some(&NSString::from_str(text(
        locale,
        Message::MenuBarIconChoose,
    ))));
    panel.setPrompt(Some(&NSString::from_str(text(locale, Message::Choose))));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    panel
        .URL()
        .and_then(|url| url.to_file_path())
        .map(Some)
        .ok_or_else(|| "the selected PNG has no local file path".to_owned())
}

fn pack_menu_context() -> Option<(MainThreadMarker, Arc<PackService>, UiLocale)> {
    with_ui_read(|ui| (ui.mtm, Arc::clone(&ui.packs), ui.locale))
}

fn begin_pack_import() {
    if with_ui_read(|ui| ui.ui_mutation_busy()).unwrap_or(true) {
        return;
    }
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
    if with_ui_read(|ui| {
        let listing = ui.packs.cached_list();
        ui.ui_mutation_busy()
            || listing.generation != generation
            || !listing.packs.iter().any(|pack| pack.id == id)
    })
    .unwrap_or(true)
    {
        return;
    }
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
    if with_ui_read(|ui| {
        let listing = ui.packs.cached_list();
        ui.ui_mutation_busy()
            || listing.generation != generation
            || !listing.packs.iter().any(|pack| pack.id == id)
    })
    .unwrap_or(true)
    {
        return;
    }
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
        let selection = &ui.character_selection;
        selection.operation_id().map(|id| {
            if let Some(error) = selection.operation_error() {
                format!(
                    "{id}: {} ({error}; {})",
                    text(locale, Message::CharacterOperationFailed),
                    text(locale, Message::CharacterOperationNotSubmitted)
                )
            } else if let Some(op) = selection.operation() {
                let base = crate::i18n::pack_operation(locale, id, &op.state, op.error.as_deref());
                format!(
                    "{base} (committed: {}, ui_applied: {})",
                    op.committed, op.ui_applied
                )
            } else {
                let last = selection
                    .last_observation()
                    .map(|op| {
                        format!(
                            " · {} ({})",
                            op.state,
                            text(locale, Message::CharacterOperationUnknown)
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "{id}: {}{last}",
                    text(locale, Message::CharacterOperationUnknown)
                )
            }
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
    with_ui_mut(|ui| {
        if !Arc::ptr_eq(&ui.packs, &packs) {
            return;
        }
        ui.refresh_character_menu();
        if ui.ui_mutation_busy() {
            return;
        }
        let listing = ui.packs.cached_list();
        let valid = listing.generation == generation
            && match &action {
                PackAction::Update { id, .. } | PackAction::Remove { id } => {
                    listing.packs.iter().any(|pack| &pack.id == id)
                }
                _ => true,
            };
        if !valid {
            ui.pack_error = Some(text(ui.locale, Message::CharacterSelectionStale).to_owned());
            ui.refresh_character_menu();
            return;
        }
        let operation_id = next_pack_operation_id();
        let request = PackRequest {
            operation_id: operation_id.clone(),
            expected_generation: Some(generation),
            action,
        };
        ui.character_selection.reserve_other(operation_id.clone());
        let result = packs.submit_ui_if_idle(request);
        ui.record_pack_submission(result, &operation_id);
    });
}

fn changed_presentation(old: &Scene, next: &Scene) -> PresentationPatch {
    PresentationPatch {
        visible: (old.visible != next.visible).then_some(next.visible),
        passthrough: (old.passthrough != next.passthrough).then_some(next.passthrough),
        alpha_passthrough: (old.alpha_passthrough != next.alpha_passthrough)
            .then_some(next.alpha_passthrough),
        bubble_visible: (old.bubble_visible != next.bubble_visible).then_some(next.bubble_visible),
        bubble_placement: (old.bubble_placement != next.bubble_placement)
            .then_some(next.bubble_placement),
        scale: (old.scale != next.scale).then_some(next.scale),
    }
}

fn presentation_persistence_patch(
    previous: &Scene,
    scene: &Scene,
    saved: PresentationTarget,
    did_present: bool,
    requested: Option<&PresentationPatch>,
) -> PresentationPatch {
    let mut patch = if did_present {
        changed_presentation(previous, scene)
    } else {
        let mut baseline = scene.clone();
        baseline.visible = saved.visible;
        baseline.passthrough = saved.passthrough;
        baseline.alpha_passthrough = saved.alpha_passthrough;
        baseline.bubble_visible = saved.bubble_visible;
        baseline.bubble_placement = saved.bubble_placement;
        baseline.scale = saved.scale;
        changed_presentation(&baseline, scene)
    };
    if let Some(requested) = requested {
        patch.merge_requested(&requested.requested_scene_values(scene));
    }
    patch
}

fn reduce_presentation_batch(
    state: &mut AppState,
    ledger: &mut AutomationState,
    requested: &mut Option<PresentationPatch>,
    reset: &mut bool,
    operation: &mut Option<String>,
) -> Scene {
    let scene = state.scene();
    if PresentationTarget::from_scene(&scene) != ledger.desired() {
        ledger.ui_change(PresentationTarget::from_scene(&scene));
    }
    for request in ledger.drain_queued() {
        if request
            .expected_revision
            .is_some_and(|revision| revision != ledger.revision())
        {
            let _ = ledger.reject_request(
                &request.operation_id,
                "presentation revision changed".to_owned(),
            );
            continue;
        }
        let target = match PresentationTarget::from_scene(&state.scene()).applying(&request.action)
        {
            Ok(target) => target,
            Err(error) => {
                let _ = ledger.reject_request(&request.operation_id, error);
                continue;
            }
        };
        if let Err(error) = ledger.start_request(&request.operation_id, target) {
            let _ = ledger.reject_request(&request.operation_id, error);
            continue;
        }
        // start_request verifies the same absolute target before mutating AppState.
        state
            .apply_presentation(&request.action)
            .expect("validated presentation action");
        *operation = Some(request.operation_id.clone());
        match request.action {
            PresentationAction::Set { patch } => requested
                .get_or_insert_with(PresentationPatch::default)
                .merge_requested(&patch),
            PresentationAction::ResetPosition => *reset = true,
        }
    }
    state.scene()
}

impl Ui {
    fn new(
        shared: Arc<Mutex<AppState>>,
        packs: Arc<PackService>,
        prepared: PreparedCharacter,
        prefs: Preferences,
        dialogue_override_active: bool,
        mtm: MainThreadMarker,
    ) -> Result<Self, String> {
        let lifecycle_paths = Paths::resolve(None, None)?;
        let state = shared
            .lock()
            .map_err(|_| "native state lock is poisoned".to_owned())?;
        let scene = state.scene();
        let automation = state
            .automation()
            .ok_or("presentation automation was not initialized")?;
        drop(state);
        let saved_presentation = PresentationTarget {
            visible: prefs.visible(),
            passthrough: prefs.passthrough(),
            alpha_passthrough: prefs.alpha_passthrough(),
            bubble_visible: prefs.bubble_visible(),
            bubble_placement: prefs.bubble_placement(),
            scale: prefs.scale(),
            reset_position_revision: 0,
        };
        let locale = current_ui_locale(prefs.language());
        let bubble_appearance = prefs.bubble_appearance();
        let palette = bubble_appearance.palette();
        let initial_frame = match &prepared {
            PreparedCharacter::Png(png) => {
                Some(png.clips.phases[phase_index(scene.phase)].frames[0])
            }
            PreparedCharacter::Rig(_) => None,
        };
        let dialogue_target = active_dialogue_target(&prepared, &packs, dialogue_override_active);
        let effective_dialogue = effective_metadata(
            prepared.metadata(),
            dialogue_target
                .as_ref()
                .and_then(|target| prefs.dialogue_overrides().locales(target)),
        );
        // Keep the loaded snapshot as the saved baseline. Runtime scene state is
        // applied to native windows independently of whether a save can succeed.

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
        let mut cards = SessionCards::new(shared.clone(), locale, prefs.session_list(), mtm);
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
        set_accessibility_identifier(&composer_view, "pet-message-input");
        set_accessibility_label(&composer_view, text(locale, Message::ComposerInput));
        let composer_scroll: Retained<NSScrollView> = unsafe {
            msg_send![NSScrollView::alloc(mtm), initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0), NSSize::new(220.0, BUBBLE_LINE_HEIGHT)
            )]
        };
        let composer_metrics = configure_composer(&composer_view, &composer_scroll);
        composer_scroll.setDrawsBackground(true);
        composer_scroll.setBackgroundColor(&reply_input_color(palette));
        let reply_container: Retained<NSView> = unsafe {
            msg_send![NSView::alloc(mtm), initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0), NSSize::new(270.0, composer_metrics.reply_height)
            )]
        };
        reply_container.addSubview(&composer_scroll);
        let composer_status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        composer_status.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_SECONDARY_FONT_SIZE)));
        composer_status.setTextColor(Some(&muted));
        composer_status.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        set_accessibility_identifier(&composer_status, "pet-message-status");
        composer_status.setHidden(true);
        reply_container.addSubview(&composer_status);
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
        composer_send.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        set_accessibility_identifier(&composer_send, "pet-message-send");
        set_accessibility_label(&composer_send, text(locale, Message::ComposerSend));
        unsafe {
            composer_send.setTarget(Some(menu_target.as_ref()));
            composer_send.setAction(Some(sel!(sendComposer:)));
        }
        composer_send.setHidden(true);
        reply_container.addSubview(&composer_send);
        bubble_panel.setContentView(Some(&bubble_root));

        let dialogue_editor = DialogueEditor::new(&menu_target, locale, mtm);
        let mut menu_panel = MenuPanel::new(&menu_target, character_menu.view(), locale, mtm);
        menu_panel.set_bubble_appearance(bubble_appearance);
        menu_panel.set_show_status_indicators(prefs.show_status_indicators());
        menu_panel.set_menu_bar_mode(prefs.menu_bar_mode());
        let default_icon =
            menu_bar_icon::default_image(mtm, text(locale, Message::MenuBarMenuTitle))?;
        let (menu_bar_icon_image, icon_load_error) = match prefs.menu_bar_icon() {
            Some(preference) => match menu_bar_icon::load_saved(preference, mtm) {
                Ok(image) => (image, None),
                Err(error) => (default_icon, Some(error)),
            },
            None => (default_icon, None),
        };
        menu_panel.set_menu_bar_icon(
            &menu_bar_icon_image,
            prefs.menu_bar_icon().is_some(),
            icon_load_error
                .as_deref()
                .map(|error| (Message::MenuBarIconLoadFailure, error)),
        );
        let status_bar = NSStatusBar::systemStatusBar();
        let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
        let status_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
        status_menu.setAutoenablesItems(false);
        add_context_item(
            &status_menu,
            mtm,
            &menu_target,
            locale,
            Message::RecoverCharacterInteraction,
            sel!(recoverCharacterInteraction:),
            true,
        );
        add_context_item(
            &status_menu,
            mtm,
            &menu_target,
            locale,
            Message::ContextSettings,
            sel!(openContextSettings:),
            true,
        );
        status_menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_context_item(
            &status_menu,
            mtm,
            &menu_target,
            locale,
            Message::MenuQuit,
            sel!(quit:),
            true,
        );
        if let Some(recover) = status_menu.itemAtIndex(0) {
            let needed = scene.needs_recovery_entry();
            recover.setHidden(!needed);
            recover.setEnabled(needed);
        }
        if let Some(button) = status_item.button(mtm) {
            button.setImage(Some(&menu_bar_icon_image));
            unsafe {
                button.setTarget(Some(menu_target.as_ref()));
                button.setAction(Some(sel!(activateStatusItem:)));
                button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
            }
            set_accessibility_label(&button, text(locale, Message::MenuBarMenuTitle));
            set_accessibility_identifier(&button, "herdr-pet-menu");
        }
        status_item.setVisible(menu_bar_visible(&scene, prefs.menu_bar_mode()));

        let window_delegate = WindowDelegate::new(mtm);
        panel.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));
        bubble_panel.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));
        let reduced_motion =
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();

        let launch_time = Instant::now();
        let mut playback = Playback::default();
        playback.reset_pack(Duration::ZERO, scene.phase);
        let prompt_sender = PromptSender::new(shared.clone());
        let worktree_sender = WorktreeRemoveSender::new(shared.clone());
        let effective_language = prefs.language();
        let native_show_status_indicators = prefs.show_status_indicators();
        let native_menu_bar_mode = prefs.menu_bar_mode();
        let mut ui = Self {
            mtm,
            shared,
            automation,
            saved_presentation,
            presentation_patch: None,
            presentation_reset: false,
            presentation_operation: None,
            checkpoint_save: None,
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
            reply_container,
            composer_scroll,
            composer_view,
            composer_metrics,
            composer_readonly: false,
            composer_status,
            composer_send,
            prompt_sender,
            worktree_sender,
            worktree_automation: automation_worktrees::WorktreeAutomation::new(),
            worktree_confirming: false,
            worktree_feedback: None,
            composer_render_stamp: None,
            composer_key: None,
            reply_open: false,
            composer_pending_key: None,
            composer_drafts: VecDeque::new(),
            composer_results: VecDeque::new(),
            cards,
            menu_panel,
            dialogue_editor,
            dialogue_automation: automation_dialogues::DialogueAutomation::new(),
            dialogue_choices: Vec::new(),
            editor_cached_choice: None,
            editor_metadata: None,
            editor_ready: false,
            editor_error: None,
            editor_content_dirty: true,
            locale,
            effective_language,
            preference_revision: 0,
            native_show_status_indicators,
            native_menu_bar_mode,
            pending_preference_operations: Vec::new(),
            cli_preference_feedback: None,
            pending_language: None,
            effective_dialogue,
            external_dialogue_metadata: dialogue_target
                .as_ref()
                .filter(|target| matches!(target, DialogueTarget::ExternalAssets(_)))
                .and_then(|_| prepared.metadata().cloned()),
            external_dialogue_target: dialogue_target
                .as_ref()
                .filter(|target| matches!(target, DialogueTarget::ExternalAssets(_)))
                .cloned(),
            dialogue_target,
            dialogue_override_active,
            dialogue_prepared_epoch: prepared.token().backend_epoch,
            active: prepared,
            display_geometry,
            bubble_appearance,
            native_bubble_appearance: bubble_appearance,
            cached_anchor: None,
            visual_anchor: None,
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
            character_selection: CharacterSelection::new(),
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
            bubble_geometry_attached: false,
            pending_standalone_body_origin: None,
            standalone_reset_pending: false,
            standalone_position_unsaved: false,
            bubble_content_dirty: true,
            pending_bubble_scene: None,
            pending_bubble_content: false,
            bubble_layout_dirty: true,
            transition_generation: 0,
            bubble_fade: None,
            reduced_motion,
            _status_item: status_item,
            status_menu,
            menu_bar_icon_image,
            menu_bar_icon_busy: false,
            status_menu_tracking: false,
            context_anchor: None,
            settings_anchor: None,
            _menu_target: menu_target,
            _menu_event_monitors: Vec::new(),
            _window_delegate: window_delegate,
            last_reset_position_revision: 0,
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

        ui.clamp_panel();
        ui.update_bubble_frame();
        ui.prepare_initial_rig_surface()?;
        ui.refresh();
        Ok(ui)
    }
    fn menu_bar_icon_shutdown(&self) -> bool {
        self.shared
            .lock()
            .map_or(true, |state| state.scene().shutdown)
    }

    fn menu_bar_icon_operation_allowed(&self) -> bool {
        !self.status_menu_tracking && !self.menu_bar_icon_shutdown()
    }

    fn show_menu_bar_icon_error(&mut self, summary: Message, detail: &str) {
        self.menu_panel.set_menu_bar_icon(
            &self.menu_bar_icon_image,
            self.prefs.menu_bar_icon().is_some(),
            Some((summary, detail)),
        );
    }

    fn apply_menu_bar_icon(&mut self, image: Retained<NSImage>) {
        if let Some(button) = self._status_item.button(self.mtm) {
            button.setImage(Some(&image));
        }
        self.menu_panel
            .set_menu_bar_icon(&image, self.prefs.menu_bar_icon().is_some(), None);
        self.menu_bar_icon_image = image;
    }

    // Call only after the prepared asset has been published outside the UI borrow.
    fn commit_menu_bar_icon(
        &mut self,
        preference: MenuBarIconPreference,
        image: Retained<NSImage>,
    ) {
        let changed = self.prefs.menu_bar_icon() != Some(&preference);
        if let Err(error) = self.prefs.save_menu_bar_icon(Some(preference)) {
            self.show_menu_bar_icon_error(Message::MenuBarIconSaveFailure, &error);
            return;
        }
        if changed {
            self.preference_revision = self
                .preference_revision
                .checked_add(1)
                .expect("preference revision exhausted");
        }
        self.apply_menu_bar_icon(image);
        self.settle_preference_operations();
    }

    fn commit_default_menu_bar_icon(&mut self, image: Retained<NSImage>) {
        let changed = self.prefs.menu_bar_icon().is_some();
        if let Err(error) = self.prefs.save_menu_bar_icon(None) {
            self.show_menu_bar_icon_error(Message::MenuBarIconSaveFailure, &error);
            return;
        }
        if changed {
            self.preference_revision = self
                .preference_revision
                .checked_add(1)
                .expect("preference revision exhausted");
        }
        self.apply_menu_bar_icon(image);
        self.settle_preference_operations();
    }

    fn set_menu_bar_mode(&mut self, mode: MenuBarMode) {
        if let Err(error) = self.commit_preference_patch(
            crate::preferences::PreferencePatch {
                menu_bar_mode: Some(mode),
                ..Default::default()
            },
            None,
        ) {
            self.menu_panel
                .set_menu_bar_mode(self.prefs.menu_bar_mode());
            self.queue_bubble_appearance_error(Message::MenuBarModeSaveFailure, &error.detail);
        }
        self.settle_preference_operations();
    }

    fn set_show_status_indicators(&mut self, enabled: bool) {
        if let Err(error) = self.commit_preference_patch(
            crate::preferences::PreferencePatch {
                show_status_indicators: Some(enabled),
                ..Default::default()
            },
            None,
        ) {
            self.menu_panel
                .set_show_status_indicators(self.prefs.show_status_indicators());
            self.queue_bubble_appearance_error(Message::StatusIndicatorsSaveFailure, &error.detail);
        }
        self.settle_preference_operations();
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
        self.commit_bubble_appearance(
            BubbleAppearance {
                theme,
                custom: self.bubble_appearance.custom,
            },
            true,
        );
    }

    fn apply_custom_bubble_colors(&mut self) {
        if self.menu_panel.bubble_colors_conflicted() || self.menu_panel.bubble_colors_marked() {
            // Keep the local draft and field editor intact. The panel displays
            // its inline conflict status until an explicit reload or rebase.
            return;
        }
        match self.menu_panel.custom_bubble_palette() {
            Ok(custom) => self.commit_bubble_appearance(
                BubbleAppearance {
                    theme: BubbleTheme::Custom,
                    custom,
                },
                false,
            ),
            Err(error) => self.queue_bubble_appearance_error(Message::InvalidBubbleColor, &error),
        }
    }

    fn commit_bubble_appearance(&mut self, appearance: BubbleAppearance, reload_colors: bool) {
        if self.menu_panel.bubble_colors_marked() {
            return;
        }
        match self.commit_preference_patch(
            crate::preferences::PreferencePatch {
                bubble_appearance: Some(appearance),
                ..Default::default()
            },
            None,
        ) {
            Ok(_) => {
                self.menu_panel.bubble_colors_saved(appearance);
                if reload_colors {
                    self.menu_panel.reload_bubble_colors();
                }
            }
            Err(error) => {
                self.menu_panel
                    .set_bubble_appearance(self.bubble_appearance);
                self.queue_bubble_appearance_error(
                    if error.code == "revision_conflict" {
                        Message::PreferenceRevisionConflict
                    } else {
                        Message::BubbleAppearanceSaveFailure
                    },
                    &error.detail,
                );
            }
        }
        self.settle_preference_operations();
    }

    fn apply_bubble_appearance(&mut self, appearance: BubbleAppearance) {
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
        self.composer_scroll
            .setBackgroundColor(&reply_input_color(palette));
        self.composer_view
            .setTextColor(Some(&bubble_color(palette.text, 1.0)));
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
        if !self.pending_bubble_content && self.pending_bubble_scene.is_none() {
            self.native_bubble_appearance = appearance;
        }
    }

    fn set_language_preference(&mut self, preference: LanguagePreference) {
        if let Err(error) = self.commit_preference_patch(
            crate::preferences::PreferencePatch {
                language: Some(preference),
                ..Default::default()
            },
            None,
        ) {
            self.menu_panel
                .set_language_preference(self.prefs.language());
            self.queue_language_save_failure(&error.detail);
        }
        self.settle_preference_operations();
    }

    fn schedule_committed_language(&mut self, preference: LanguagePreference) {
        let locale = current_ui_locale(preference);
        if self.language_transition_locked() {
            self.pending_language = Some((preference, locale));
            self.queue_language_apply();
        } else {
            self.pending_language = None;
            self.apply_language(preference, locale);
        }
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
            || self.composer_marked()
            || self.cards.is_composing()
            || NSEvent::pressedMouseButtons() != 0
            || appkit_event_tracking_active()
    }
    fn pending_ui_updates(&self) -> bool {
        self.pending_language.is_some()
            || self.pending_bubble_content
            || self.pending_bubble_scene.is_some()
            || !self.pending_preference_operations.is_empty()
            || self.menu_panel.is_visible()
            || (self.dialogue_editor.is_visible() && self.dialogue_editor.needs_initial_hydration())
            || self.has_pending_native_dialogue()
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
        self.effective_language = preference;
        self.locale = locale;
        self.composer_render_stamp = None;
        self.composer_results.clear();
        self.composer_send
            .setTitle(&NSString::from_str(text(locale, Message::ComposerSend)));
        set_accessibility_label(&self.composer_view, text(locale, Message::ComposerInput));
        if let Some(button) = self._status_item.button(self.mtm) {
            set_accessibility_label(&button, text(locale, Message::MenuBarMenuTitle));
        }
        for (index, title) in [
            (0, Message::RecoverCharacterInteraction),
            (1, Message::ContextSettings),
            (3, Message::MenuQuit),
        ] {
            if let Some(item) = self.status_menu.itemAtIndex(index) {
                item.setTitle(&NSString::from_str(text(locale, title)));
            }
        }
        self.menu_panel.set_locale(locale);
        self.cards.set_composition_active(self.composer_marked());
        self.cards.set_locale(locale);
        self.character_menu.set_locale(locale);
        self.dialogue_editor.set_locale(locale);
        self.editor_content_dirty = true;
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
        let timeout = character_renderer::preparation_timeout(&assets);
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
            deadline: Instant::now() + timeout,
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
            deadline: Instant::now() + character_renderer::SURFACE_PREPARATION_TIMEOUT,
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
                self.prepare_bubble_transition(&scene);
                self.cancel_gesture(false);
                self.cancel_pointer_state();
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
                self.clamp_panel_origin();
                self.save_position_only(None);
                self.cached_anchor = None;
                self.visual_anchor = None;
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
                self.dialogue_target = Some(DialogueTarget::Character(token.reference.id.clone()));
                self.rebuild_effective_dialogue();
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
        let deadline = Instant::now() + character_renderer::SURFACE_PREPARATION_TIMEOUT;
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

    fn update_rig(&mut self, scene: &Scene) -> bool {
        if !matches!(&self.active, PreparedCharacter::Rig(_)) {
            return true;
        }
        let intent = self.presentation_intent(scene);
        let PreparedCharacter::Rig(rig) = &mut self.active else {
            unreachable!();
        };
        rig.view()
            .setFrame(self.display_geometry.canvas_frame(scene.scale));
        rig.set_viewport_epoch(intent.viewport.epoch);
        rig.set_visible(intent.visible);
        let error = rig
            .update(character_renderer::rig_intent(intent))
            .err()
            .or_else(|| rig.error());
        let usable = error.is_none();
        self.packs.set_renderer_error(error);
        usable
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
        if let Some(operation_id) = self.character_selection.operation_id() {
            let operation = self.packs.cached_status(operation_id);
            self.character_selection
                .reconcile_operation(operation.as_ref());
        }
        let mut listing = self.packs.cached_list();
        self.character_selection.reconcile(&listing);
        self.update_dialogue_choices(&listing);
        self.sync_dialogue_editor_content(false);
        if let Some(error) = self.pack_error.as_ref() {
            listing.error = Some(error.clone());
        }
        if self.menu_panel.is_visible() && !self.last_scene.shutdown {
            let scene = self.last_scene.clone();
            let status = status_text(&scene, self.locale);
            if let Ok(state) = self.shared.lock() {
                self.menu_panel.sync(
                    &scene,
                    self.prefs.menu_bar_mode(),
                    self.prefs.language(),
                    &status,
                    state.lifecycle_settings(),
                    state.observation_preferences(),
                    state.observation_catalog(),
                );
            }
        }
        self.character_menu.refresh(
            &listing,
            &self.character_selection,
            self.packs.ui_mutation_busy() || self.character_selection.is_busy(),
            self._menu_target.as_ref(),
            self.mtm,
        );
    }

    fn handle_pack_command(&mut self, command: MenuCommand) {
        self.refresh_character_menu();
        if self.ui_mutation_busy() {
            return;
        }
        let listing = self.packs.cached_list();
        self.character_selection.reconcile(&listing);
        let generation = listing.generation;
        let result = match command {
            MenuCommand::Select {
                id,
                generation: sent,
            } if sent == generation => self.character_selection.stage_head(&id),
            MenuCommand::Restore {
                id,
                revision,
                generation: sent,
            } if sent == generation => self
                .character_selection
                .stage_revision(CharacterRef { id, revision }),
            MenuCommand::Select { .. } | MenuCommand::Restore { .. } => Err(SelectionError::Stale),
            MenuCommand::Update { .. }
            | MenuCommand::Remove { .. }
            | MenuCommand::Inspect { .. } => return,
        };
        self.pack_error = result
            .err()
            .map(|error| text(self.locale, selection_error_message(error)).to_owned());
        self.refresh_character_menu();
    }

    fn apply_candidate(&mut self) {
        self.refresh_character_menu();
        if self.ui_mutation_busy() {
            return;
        }
        let request = match self
            .character_selection
            .request_apply(next_pack_operation_id())
        {
            Ok(request) => request,
            Err(error) => {
                self.pack_error =
                    Some(text(self.locale, selection_error_message(error)).to_owned());
                self.refresh_character_menu();
                return;
            }
        };
        let operation_id = request.operation_id.clone();
        match self.packs.submit_ui_if_idle(request) {
            Ok(operation) => {
                self.character_selection.record_operation(&operation);
                self.pack_error = None;
            }
            Err(error) => {
                self.character_selection
                    .submission_failed(&operation_id, error);
                self.pack_error = None;
            }
        }
        self.refresh_character_menu();
    }

    fn cancel_candidate(&mut self) {
        self.refresh_character_menu();
        if self.ui_mutation_busy() {
            return;
        }
        self.pack_error = self
            .character_selection
            .cancel()
            .err()
            .map(|error| text(self.locale, selection_error_message(error)).to_owned());
        self.refresh_character_menu();
    }

    fn ui_mutation_busy(&self) -> bool {
        self.character_selection.is_busy() || self.packs.ui_mutation_busy()
    }

    fn record_pack_submission(
        &mut self,
        result: Result<PackOperation, String>,
        operation_id: &str,
    ) {
        match result {
            Ok(operation) => {
                self.character_selection.record_operation(&operation);
                self.pack_error = None;
            }
            Err(error) => {
                self.character_selection
                    .submission_failed(operation_id, error);
                self.pack_error = None;
            }
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
    fn composer_marked(&self) -> bool {
        unsafe { msg_send![&*self.composer_view, hasMarkedText] }
    }

    fn composer_has_focus(&self) -> bool {
        if !self.bubble_panel.isKeyWindow() {
            return false;
        }
        let responder: Option<&AnyObject> =
            unsafe { msg_send![&*self.bubble_panel, firstResponder] };
        responder
            .is_some_and(|responder| unsafe { msg_send![responder, isEqual: &*self.composer_view] })
    }

    fn composer_input_visible(&self) -> bool {
        self.reply_open
            && !self.cards.view().isHidden()
            && !self.composer_scroll.isHidden()
            && self.composer_scroll.frame().size.width > 0.0
            && self.composer_scroll.frame().size.height >= self.composer_metrics.input_height
            && self.composer_scroll.contentSize().height >= self.composer_metrics.content_height
    }

    fn restore_composer_focus(&self, was_focused: bool) {
        if !was_focused || !self.bubble_panel.isKeyWindow() || !self.bubble_panel.isVisible() {
            return;
        }
        if self.composer_input_visible() {
            if !self.composer_has_focus() {
                let _ = self
                    .bubble_panel
                    .makeFirstResponder(Some(&self.composer_view));
            }
        } else if self.composer_has_focus() {
            let _ = self.bubble_panel.makeFirstResponder(None);
        }
    }

    fn apply_pending_cards_options(&mut self) {
        self.cards.sync_search_composition();
        if self.composer_marked() || self.cards.is_composing() {
            return;
        }
        let Some(candidate) = self.cards.pending_options() else {
            return;
        };
        if candidate == self.prefs.session_list() {
            self.cards.commit_options(candidate);
            return;
        }
        match self.prefs.save_session_list(candidate) {
            Ok(()) => self.cards.commit_options(candidate),
            Err(error) => {
                self.cards.reject_options();
                let locale = self.locale;
                DispatchQueue::main().exec_async(move || {
                    let Some(mtm) = with_ui_read(|ui| ui.mtm) else {
                        return;
                    };
                    show_bubble_appearance_error(
                        mtm,
                        locale,
                        Message::SessionListSaveFailure,
                        &error,
                    );
                });
            }
        }
    }

    fn refresh_cards(&mut self) -> bool {
        self.cards.set_composition_active(self.composer_marked());
        self.apply_pending_cards_options();
        self.cards.refresh();
        !self.cards.is_composing() && self.cards.take_deferred_selection_applied()
    }

    fn close_reply(&mut self) {
        if !self.reply_open || self.composer_marked() || self.cards.is_composing() {
            return;
        }
        self.reply_open = false;
        if let Some(key) = self.composer_key.clone() {
            let draft = self.composer_text();
            self.remember_composer_draft(key, draft);
        }
        if self.composer_has_focus() {
            let _ = self.bubble_panel.makeFirstResponder(None);
        }
        self.cards.set_composition_active(self.composer_marked());
        self.cards.detach_reply();
        self.composer_render_stamp = None;
        self.bubble_content_dirty = true;
    }

    fn fold_reply(&mut self) {
        if !self.reply_open {
            return;
        }
        if self.composer_marked() || self.cards.is_composing() {
            return;
        }
        if self.bubble_panel.isKeyWindow() {
            let _ = self.bubble_panel.makeFirstResponder(None);
        }
        self.close_reply();
        let scene = self.last_scene.clone();
        self.refresh_bubble_content(&scene);
    }

    fn escape_reply_or_collapse(&mut self) {
        if self.reply_open {
            self.fold_reply();
        } else {
            self.collapse_bubble();
        }
    }

    fn minimum_cards_height(&self) -> f64 {
        minimum_selectable_height()
            + if self.reply_open {
                self.reply_container.frame().size.height
            } else {
                0.0
            }
    }

    fn desired_cards_height(&self) -> f64 {
        let desired = self.cards.content_height().min(maximum_cards_height());
        if self.reply_open {
            desired.max(self.minimum_cards_height())
        } else {
            desired
        }
    }

    fn layout_reply_children(&mut self) {
        if self.composer_marked() || self.cards.is_composing() {
            self.pending_bubble_scene = Some(self.last_scene.clone());
            self.queue_language_apply();
            return;
        }
        let geometry = reply_child_geometry(
            self.reply_container.frame().size,
            self.composer_readonly,
            self.composer_metrics,
        );
        if self.composer_status.frame() != geometry.status {
            self.composer_status.setFrame(geometry.status);
        }
        if self.composer_scroll.frame() != geometry.input {
            self.composer_scroll.setFrame(geometry.input);
        }
        if self.composer_send.frame() != geometry.send {
            self.composer_send.setFrame(geometry.send);
        }
        self.composer_status
            .setHidden(geometry.status.size.height <= 0.0 || geometry.status.size.width <= 0.0);
        self.composer_scroll
            .setHidden(geometry.readonly || !geometry.interactive);
        self.composer_send
            .setHidden(geometry.readonly || !geometry.interactive);
        if geometry.interactive {
            layout_composer(
                &self.composer_view,
                &self.composer_scroll,
                self.composer_metrics,
            );
        }
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
        let marked = self.composer_marked();
        self.cards.set_composition_active(marked);
        self.cards.sync_search_composition();
        let mut composing = marked || self.cards.is_composing();
        let was_focused = self.composer_has_focus();
        let replayed = if self.cards.selection_stamp().0 != revision
            || (!composing && self.cards.has_deferred_refresh())
        {
            self.refresh_cards()
        } else {
            false
        };
        composing = self.composer_marked() || self.cards.is_composing();
        if replayed && self.bubble_mode == BubbleMode::Expanded {
            self.reply_open = true;
            self.composer_render_stamp = None;
            self.bubble_content_dirty = true;
        }
        let stamp = ComposerRenderStamp {
            cards: self.cards.selection_stamp(),
            locale: self.locale,
            pending: self.prompt_sender.is_pending(),
            live_revision: revision,
        };
        if !composing && self.composer_render_stamp == Some(stamp) {
            if !self.composer_scroll.isHidden() {
                layout_composer(
                    &self.composer_view,
                    &self.composer_scroll,
                    self.composer_metrics,
                );
            }
            self.restore_composer_focus(was_focused);
            return;
        }
        self.composer_render_stamp = Some(stamp);
        let selected = self.cards.visible_selected_target();
        if !composing && selected.is_none() && self.reply_open {
            self.close_reply();
            self.composer_render_stamp = Some(stamp);
        }
        let next_key = if composing {
            self.composer_key.clone()
        } else {
            selected.as_ref().map(|(key, _)| key.clone())
        };
        if self.composer_key != next_key {
            self.cards.detach_reply();
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
        let available = next_key.as_ref().map(|key| {
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
        } else if let Some(key) = next_key.as_ref() {
            if let Some(Err(error)) = available.as_ref() {
                self.composer_error_text(error)
            } else if let Some((_, previous)) =
                self.composer_results.iter().find(|(old, _)| old == key)
            {
                previous.clone()
            } else if self.prompt_sender.is_pending() {
                text(self.locale, Message::ComposerBusy).to_owned()
            } else {
                text(
                    self.locale,
                    if composing {
                        Message::ComposerComposingShortcut
                    } else {
                        Message::ComposerShortcut
                    },
                )
                .to_owned()
            }
        } else {
            text(self.locale, Message::ComposerSelectSession).to_owned()
        };
        self.composer_status
            .setStringValue(&NSString::from_str(&status));
        set_accessibility_label(&self.composer_status, &status);
        self.composer_status
            .setToolTip(Some(&NSString::from_str(&status)));
        let send = if self.prompt_sender.is_pending() {
            Message::ComposerSending
        } else {
            Message::ComposerSend
        };
        self.composer_send
            .setTitle(&NSString::from_str(text(self.locale, send)));
        set_accessibility_label(&self.composer_send, text(self.locale, send));
        self.composer_send.setEnabled(
            !composing
                && matches!(available.as_ref(), Some(Ok(())))
                && !self.prompt_sender.is_pending(),
        );
        if composing {
            self.composer_render_stamp = None;
            return;
        }
        if self.reply_open && self.bubble_mode == BubbleMode::Expanded {
            if let Some(key) = next_key.as_ref() {
                let readonly = matches!(available.as_ref(), Some(Err(PromptError::ReadOnly)));
                let height = if readonly {
                    COMPOSER_READONLY_HEIGHT
                } else {
                    self.composer_metrics.reply_height
                };
                if self.cards.attach_reply(key, &self.reply_container, height) {
                    self.composer_readonly = readonly;
                    self.layout_reply_children();
                    self.restore_composer_focus(was_focused);
                    return;
                }
            }
            self.reply_open = false;
        }
        self.cards.detach_reply();
    }

    fn worktree_target_matches(&self, target: &WorktreeRemoveTarget) -> bool {
        self.shared.lock().is_ok_and(|state| {
            state
                .worktree_remove_target(&target.key)
                .is_ok_and(|current| &current == target)
        })
    }

    fn set_worktree_feedback(&mut self, feedback: WorktreeFeedback) {
        self.worktree_feedback = Some(feedback);
        self.bubble_content_dirty = true;
        wake();
    }

    fn poll_worktree(&mut self) {
        if let Some(result) = self
            .worktree_sender
            .try_result()
            .and_then(|result| self.route_cli_worktree_result(result))
        {
            self.set_worktree_feedback(WorktreeFeedback::Finished(result.target, result.result));
        }
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
        if self.composer_marked() || self.cards.is_composing() {
            return;
        }
        let Some(key) = self.composer_key.clone().filter(|key| {
            self.reply_open
                && self.bubble_mode == BubbleMode::Expanded
                && self
                    .cards
                    .selected_target()
                    .is_some_and(|(selected, _)| &selected == key)
                && !self.composer_scroll.isHidden()
        }) else {
            return;
        };
        let marked: bool = unsafe { msg_send![&*self.composer_view, hasMarkedText] };
        if marked {
            return;
        }
        let value = self.composer_text();
        self.remember_composer_draft(key.clone(), value.clone());
        match self
            .prompt_sender
            .submit(key.clone(), value, RequestOrigin::Gui)
        {
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
        if let Some(result) = self
            .prompt_sender
            .try_result()
            .and_then(|result| self.route_cli_prompt_result(result))
        {
            self.composer_pending_key = None;
            let submission = result.submission;
            let status = match result.result {
                Ok(()) => {
                    let marked: bool = unsafe { msg_send![&*self.composer_view, hasMarkedText] };
                    let current = !marked
                        && !self.cards.is_composing()
                        && self.composer_key.as_ref() == Some(&submission.key)
                        && self
                            .cards
                            .selected_target()
                            .is_some_and(|(key, _)| key == submission.key)
                        && self.composer_text() == submission.text;
                    if current {
                        self.composer_view.setString(&NSString::from_str(""));
                        self.close_reply();
                    }
                    if self
                        .composer_drafts
                        .iter()
                        .any(|(key, draft)| key == &submission.key && draft == &submission.text)
                    {
                        self.composer_drafts
                            .retain(|(key, _)| key != &submission.key);
                    }
                    text(self.locale, Message::ComposerSent).to_owned()
                }
                Err(error) => self.composer_error_text(&error),
            };
            self.remember_composer_result(submission.key.clone(), status);
            self.composer_render_stamp = None;
            self.sync_composer();
        }
    }

    fn drain_presentation_requests(&mut self) -> Option<Scene> {
        let mut state = self.shared.lock().ok()?;
        let mut ledger = lock_automation(&self.automation);
        Some(reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut self.presentation_patch,
            &mut self.presentation_reset,
            &mut self.presentation_operation,
        ))
    }

    fn save_presentation(
        &mut self,
        scene: &Scene,
        patch: PresentationPatch,
        position: bool,
        reset: bool,
    ) {
        let mut candidate = self.prefs.candidate();
        let mut saved = self.saved_presentation;
        if let Some(value) = patch.visible {
            candidate.set_visible(value);
            saved.visible = value;
        }
        if let Some(value) = patch.passthrough {
            candidate.set_passthrough(value);
            saved.passthrough = value;
        }
        if let Some(value) = patch.alpha_passthrough {
            candidate.set_alpha_passthrough(value);
            saved.alpha_passthrough = value;
        }
        if let Some(value) = patch.bubble_visible {
            candidate.set_bubble_visible(value);
            saved.bubble_visible = value;
        }
        if let Some(value) = patch.bubble_placement {
            candidate.set_bubble_placement(value);
            saved.bubble_placement = value;
        }
        if let Some(value) = patch.scale {
            candidate.set_scale(value);
            saved.scale = value;
        }
        if reset {
            candidate.set_standalone_bubble_position(None);
            saved.reset_position_revision = scene.reset_position_revision;
        }
        if position {
            let origin = self.panel.frame().origin;
            candidate.set_position(Some((origin.x, origin.y)));
        }
        match self.prefs.save_candidate(candidate) {
            Ok(()) => {
                self.saved_presentation = saved;
                self.checkpoint_save = Some(Ok(saved));
                if reset {
                    self.standalone_reset_pending = false;
                }
                self.menu_panel.set_presentation_error(None);
            }
            Err(error) => {
                self.checkpoint_save = Some(Err((saved, error.clone())));
                self.menu_panel.set_presentation_error(Some(&error));
            }
        }
    }

    fn save_position_only(&mut self, standalone: Option<Option<(f64, f64)>>) {
        let mut candidate = self.prefs.candidate();
        if let Some(position) = standalone {
            candidate.set_standalone_bubble_position(position);
        } else {
            let origin = self.panel.frame().origin;
            candidate.set_position(Some((origin.x, origin.y)));
        }
        match self.prefs.save_candidate(candidate) {
            Ok(()) => {
                if standalone.is_some() {
                    self.standalone_position_unsaved = false;
                    self.standalone_reset_pending = false;
                    self.pending_standalone_body_origin = None;
                }
            }
            Err(error) => self.menu_panel.set_presentation_error(Some(&error)),
        }
    }

    fn publish_presentation_checkpoint(&mut self, scene: &Scene) {
        let mut pending = Vec::new();
        if self.pending_bubble_scene.is_some() || self.pending_bubble_content {
            pending.push(if self.composer_marked() || self.cards.is_composing() {
                PendingReason::ImeComposition
            } else {
                PendingReason::Tracking
            });
        }
        if self.explicit_gesture_active() || self.pointer_press.is_some() {
            pending.push(PendingReason::Gesture);
        }
        if self.status_menu_tracking {
            pending.push(PendingReason::Tracking);
        }
        if self.bubble_placement_tracking_locked() {
            pending.push(PendingReason::Tracking);
        }
        if scene.bubble_visible && self.bubble_geometry.is_none() {
            pending.push(PendingReason::NativeApply);
        }
        let target = PresentationTarget::from_scene(scene);
        let pet_visible = self.panel.isVisible();
        let bubble_visible = self.bubble_panel.isVisible();
        let native_applied = self.did_present
            && !scene.shutdown
            && pending.is_empty()
            && pet_visible == scene.visible
            && bubble_visible == scene.bubble_visible
            && PresentationTarget::from_scene(&self.last_scene) == target;
        if !native_applied {
            pending.push(PendingReason::NativeApply);
        }
        let frame = |rect: NSRect| WindowFrame {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        };
        let (persisted, save_error) = match self.checkpoint_save.take() {
            Some(Ok(saved)) => (Some(saved), None),
            Some(Err((attempted, error))) => (
                None,
                self.presentation_operation
                    .as_ref()
                    .map(|id| (id.clone(), attempted, error)),
            ),
            None => (None, None),
        };
        let mut ledger = lock_automation(&self.automation);
        let revision = ledger.revision();
        let native_applied = native_applied && ledger.desired() == target;
        if !native_applied && !pending.contains(&PendingReason::NativeApply) {
            pending.push(PendingReason::NativeApply);
        }
        ledger.publish_checkpoint(PresentationCheckpoint {
            revision,
            effective: native_applied.then_some(target),
            native_applied,
            persisted,
            save_error,
            pet_window_visible: Some(pet_visible),
            bubble_window_visible: Some(bubble_visible),
            pet_window_frame: Some(frame(self.panel.frame())),
            bubble_window_frame: Some(frame(self.bubble_panel.frame())),
            pending_reasons: pending,
        });
    }

    fn refresh(&mut self) {
        self.apply_pending_language();
        self.drain_domain_requests();
        self.poll_worktree();
        self.poll_cli_dialogues();
        self.poll_cli_worktrees();
        self.settle_preference_operations();
        let Some(scene) = self.drain_presentation_requests() else {
            return;
        };
        let (mut completed, outcomes) = match self.shared.lock() {
            Ok(mut state) => (!state.take_completions().is_empty(), state.take_outcomes()),
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
        let was_focused = self.composer_has_focus();
        let replayed = self.refresh_cards();
        if replayed && self.bubble_mode == BubbleMode::Expanded {
            self.reply_open = true;
            self.composer_render_stamp = None;
        }
        self.poll_composer();
        self.poll_worktree();
        self.sync_composer();
        if self.bubble_mode == BubbleMode::Expanded && cards_height != self.cards.content_height() {
            self.bubble_content_dirty = true;
        }
        self.refresh_event(scene, completed);
        self.restore_composer_focus(was_focused);
        self.refresh_character_menu();
    }

    fn applied_attached_body_origin(&self) -> Option<(f64, f64)> {
        attached_body_origin(self.bubble_geometry, self.bubble_geometry_attached)
    }

    fn prepare_bubble_transition(&mut self, scene: &Scene) {
        if scene.visible
            || scene.shutdown
            || scene.reset_position_revision != self.last_reset_position_revision
        {
            self.pending_standalone_body_origin = None;
        } else if self.pending_standalone_body_origin.is_none()
            && self.prefs.standalone_bubble_position().is_none()
            && self.did_present
            && self.last_scene.visible
            && self.last_scene.bubble_visible
        {
            self.pending_standalone_body_origin = self.applied_attached_body_origin();
        }
    }

    fn refresh_event(&mut self, scene: Scene, completed: bool) {
        let explicit = self.presentation_patch.take();
        let patch = presentation_persistence_patch(
            &self.last_scene,
            &scene,
            self.saved_presentation,
            self.did_present,
            explicit.as_ref(),
        );
        let explicit_reset = std::mem::take(&mut self.presentation_reset);
        let explicit_set = explicit.is_some();
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

        // Read the applied geometry before any cancellation, reset or resize
        // can publish an older pending placement or invalidate its provenance.
        self.prepare_bubble_transition(&scene);

        if presentation_changed
            || bubble_changed
            || reset_position_changed
            || (phase_changed && self.active.metadata().is_none())
        {
            self.cancel_gesture(false);
            self.cancel_pointer_state();
        }
        if !scene.visible || scene.passthrough {
            self.set_hover(false, false);
        }
        // Presentation saves are staged from the last saved snapshot below;
        // native state is never rolled back when config storage is unavailable.
        if reset_position_changed {
            self.reset_position();

            self.last_reset_position_revision = scene.reset_position_revision;
        }
        if scale_changed {
            self.resize(scene.scale);
        }
        if status_changed {
            self.bubble_content_dirty = true;
        }

        if scene.shutdown {
            self.hide_bubble_panel();
            lock_automation(&self.automation).shutdown();
            self.shutdown();
            self.sync_status_menu(&scene);
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
        if !scene.bubble_visible {
            if self.bubble_panel.isVisible() {
                self.hide_bubble_panel();
            }
            self.reset_bubble_mode();
        }
        let position = !self.did_present
            || scale_changed
            || reset_position_changed
            || scene.visible != self.last_scene.visible;
        if position
            || reset_position_changed
            || explicit_reset
            || explicit_set
            || patch.visible.is_some()
            || patch.passthrough.is_some()
            || patch.alpha_passthrough.is_some()
            || patch.bubble_visible.is_some()
            || patch.bubble_placement.is_some()
            || patch.scale.is_some()
        {
            self.save_presentation(
                &scene,
                patch,
                position,
                reset_position_changed || explicit_reset,
            );
        }
        self.sync_status_menu(&scene);
        self.last_scene = scene.clone();
        self.did_present = true;
        self.render(
            &scene,
            completed,
            None,
            bubble_changed || presentation_changed || reset_position_changed || scale_changed,
        );
        self.publish_presentation_checkpoint(&scene);
        self.presentation_operation = None;
    }

    fn render(
        &mut self,
        scene: &Scene,
        completed: bool,
        reaction: Option<Reaction>,
        relayout_bubble: bool,
    ) {
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
        let content_changed = self.measure_bubble_content(scene);
        let rig_usable = self.update_rig(scene);
        let placement_invalidated = if !rig_usable {
            self.cached_anchor = None;
            self.visual_anchor.take().is_some()
        } else {
            scene.visible && self.refresh_speech_anchor(scene)
        };
        if relayout_bubble || content_changed || placement_invalidated {
            self.update_bubble_frame_scene(scene);
        }
        if scene.bubble_visible && !self.bubble_panel.isVisible() {
            self.show_bubble_panel();
        }
        self.advance_bubble_fade();
        self.update_pointer_timer(scene);
        self.update_pointer_policy(scene);
    }

    fn frame_tick(&mut self) {
        self.poll_composer();
        self.poll_worktree();
        self.poll_cli_dialogues();
        self.poll_cli_worktrees();
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
        self.render(&scene, false, None, false);
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
            || (!scene.shutdown
                && !scene.passthrough
                && (scene.bubble_visible
                    || (scene.visible
                        && (scene.alpha_passthrough
                            || matches!(&self.active, PreparedCharacter::Rig(_))))));
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
            if !scene.shutdown
                && (self.bubble_placement_tracking_locked()
                    || self.composer_marked()
                    || self.cards.is_composing())
            {
                return;
            }
            self.panel.setIgnoresMouseEvents(
                !scene.visible || scene.passthrough || scene.alpha_passthrough,
            );
            self.bubble_panel.setIgnoresMouseEvents(true);
        }
    }

    fn pointer_tick(&mut self) {
        self.cards.set_composition_active(self.composer_marked());
        self.cards.sync_search_composition();
        let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) else {
            return;
        };
        if scene.shutdown
            || !presentation_matches_scene(&scene, &self.last_scene)
            || status_fields_changed(&scene, &self.last_scene)
        {
            self.refresh();
            return;
        }
        let placement_invalidated = if scene.visible {
            if self.update_rig(&scene) {
                self.refresh_speech_anchor(&scene)
            } else {
                self.cached_anchor = None;
                self.visual_anchor.take().is_some()
            }
        } else {
            false
        };
        self.advance_bubble_fade();
        self.apply_pending_bubble_updates();
        if placement_invalidated {
            self.update_bubble_frame_scene(&scene);
        }
        if self.reply_open
            && !self.composer_scroll.isHidden()
            && !self.composer_marked()
            && !self.cards.is_composing()
        {
            layout_composer(
                &self.composer_view,
                &self.composer_scroll,
                self.composer_metrics,
            );
        }
        self.update_pointer_policy(&scene);
        self.publish_presentation_checkpoint(&scene);
    }

    fn apply_pending_bubble_updates(&mut self) {
        self.cards.set_composition_active(self.composer_marked());
        self.cards.sync_search_composition();
        // A deferred drain must not perform even placement-only layout while
        // AppKit is tracking a control or a pet/bubble gesture is active.
        if self.bubble_content_tracking_locked()
            || self.composer_marked()
            || self.cards.is_composing()
        {
            return;
        }
        if self.cards.has_deferred_refresh() {
            self.refresh();
            return;
        }
        if !self.pending_bubble_content && self.pending_bubble_scene.is_none() {
            return;
        }
        let Ok(state) = self.shared.lock() else {
            return;
        };
        let current = state.scene();
        drop(state);
        if current.shutdown
            || !presentation_matches_scene(&current, &self.last_scene)
            || status_fields_changed(&current, &self.last_scene)
        {
            return;
        }
        if self.pending_bubble_content {
            let scene = self.last_scene.clone();
            self.bubble_content_dirty = true;
            self.refresh_bubble_content(&scene);
        }
        if self.pending_bubble_scene.take().is_some() {
            let scene = self.last_scene.clone();
            self.apply_bubble_frame_scene(&scene);
            if scene.bubble_visible {
                self.show_bubble_panel();
            } else {
                if self.bubble_panel.isVisible() {
                    self.hide_bubble_panel();
                }
                self.reset_bubble_mode();
            }
        }
        if !self.pending_bubble_content && self.pending_bubble_scene.is_none() {
            self.native_show_status_indicators = self.prefs.show_status_indicators();
            self.native_bubble_appearance = self.bubble_appearance;
        }
    }

    fn update_pointer_policy(&mut self, scene: &Scene) {
        if scene.shutdown {
            self.panel.setIgnoresMouseEvents(true);
            self.bubble_panel.setIgnoresMouseEvents(true);
            return;
        }
        // A native control's mouse target must not change inside its tracking loop.
        if !self.explicit_gesture_active()
            && (NSEvent::pressedMouseButtons() != 0
                || appkit_event_tracking_active()
                || self.composer_marked()
                || self.cards.is_composing())
        {
            return;
        }
        if scene.passthrough {
            self.panel.setIgnoresMouseEvents(true);
            self.bubble_panel.setIgnoresMouseEvents(true);
            return;
        }
        let fade_active = self.bubble_fade.is_some();
        if self.explicit_gesture_active() {
            self.panel.setIgnoresMouseEvents(!scene.visible);
            self.bubble_panel
                .setIgnoresMouseEvents(!scene.bubble_visible || fade_active);
            return;
        }
        // Native controls must retain their target throughout tracking.
        if NSEvent::pressedMouseButtons() != 0 || appkit_event_tracking_active() {
            return;
        }
        let screen = NSEvent::mouseLocation();
        let pet_hit = if !scene.visible {
            false
        } else if !scene.alpha_passthrough {
            if matches!(&self.active, PreparedCharacter::Rig(_)) {
                let _ = self.character_hit(self.panel.convertPointFromScreen(screen));
            }
            true
        } else {
            let panel_point = self.panel.convertPointFromScreen(screen);
            grip_hit_test(panel_point, self.root.bounds().size)
                || (NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
                    && point_in_rect(panel_point, self.root.bounds()))
                || self
                    .character_hit(panel_point)
                    .is_some_and(|hit| hit.opaque)
        };
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
        let target = if self.last_scene.visible {
            DragTarget::Character
        } else {
            DragTarget::StandaloneBubble
        };
        let frame = self.drag_frame(target);
        let drag = self.begin_gesture(GestureKind::Move, screen_point, frame, None, target)?;
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
                    DragTarget::Character,
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
                    DragTarget::Character,
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
                    DragTarget::Character,
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
                let context_opening = (event.r#type() == NSEventType::RightMouseDown
                    || (event.r#type() == NSEventType::LeftMouseDown
                        && event
                            .modifierFlags()
                            .contains(NSEventModifierFlags::Control)))
                    && with_ui_read(|ui| {
                        event.window(ui.mtm).is_some_and(|window| {
                            window.windowNumber() == ui.panel.windowNumber()
                                || window.windowNumber() == ui.bubble_panel.windowNumber()
                        })
                    })
                    .unwrap_or(false);
                with_ui_mut(|ui| {
                    if !context_opening
                        && ui.reply_open
                        && event.window(ui.mtm).is_some_and(|window| {
                            window.windowNumber() != ui.bubble_panel.windowNumber()
                        })
                    {
                        ui.fold_reply();
                    }
                });
                let inside = with_ui_read(|ui| {
                    ui.menu_panel.is_visible() && menu_event_is_inside(event, ui)
                })
                .unwrap_or(false);
                if !inside && !context_opening {
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
            with_ui_mut(|ui| ui.fold_reply());
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
        if scene.shutdown
            || !presentation_matches_scene(&scene, &self.last_scene)
            || status_fields_changed(&scene, &self.last_scene)
        {
            self.refresh();
            if scene.shutdown {
                return;
            }
            let Some(current) = self.shared.lock().ok().map(|state| state.scene()) else {
                return;
            };
            if current.shutdown
                || !presentation_matches_scene(&current, &self.last_scene)
                || status_fields_changed(&current, &self.last_scene)
            {
                return;
            }
            self.render(&current, completed, reaction, false);
        } else {
            self.render(&scene, completed, reaction, false);
        }
    }

    fn cancel_pointer_state(&mut self) {
        self.pointer_event_started_at = None;
        self.interaction.cancel();
        self.pointer_press = None;
    }

    fn cancel_pointer(&mut self) {
        self.cancel_pointer_state();
        if self.did_present {
            self.render_current(false, None);
        }
    }

    fn shutdown(&mut self) {
        self.pending_standalone_body_origin = None;
        self.pending_bubble_scene = None;
        self.pending_bubble_content = false;
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
        self.dialogue_editor.shutdown();
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
        target: DragTarget,
    ) -> Option<DragState> {
        if self.root.ivars().drag.get().is_some() || self.bubble_root.ivars().drag.get().is_some() {
            return None;
        }
        if !self.input_owner_is_current(input_owner) {
            return None;
        }
        let scene = self.shared.lock().ok()?.scene();
        if scene.shutdown
            || scene.passthrough
            || match target {
                DragTarget::Character => !scene.visible,
                DragTarget::StandaloneBubble => {
                    scene.visible || !scene.bubble_visible || kind != GestureKind::Move
                }
            }
        {
            return None;
        }
        let frame = self.drag_frame(target);
        let size_matches_scale = target == DragTarget::StandaloneBubble
            || kind != GestureKind::Resize
            || frame_size_matches_scale(self.display_geometry, frame.size, scene.scale);
        if !presentation_matches_scene(&scene, &self.last_scene)
            || !rect_nearly_equal(frame, start_frame)
            || !size_matches_scale
        {
            self.refresh();
            return None;
        }
        let screen_visible = if target == DragTarget::StandaloneBubble {
            standalone_visible_frame(self.mtm, frame)?
        } else {
            panel_visible_frame(&self.panel, self.mtm)?
        };
        let start_top_left = NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height);
        if kind == GestureKind::Resize
            && resize_scale_limits(self.display_geometry, start_top_left, screen_visible).is_none()
        {
            return None;
        }
        Some(DragState {
            target,
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
                self.cancel_gesture(true);
                return None;
            }
        };
        if scene.shutdown
            || !self.input_owner_is_current(drag.input_owner)
            || !scene_matches_drag(&scene, &drag)
            || !rect_nearly_equal(self.drag_frame(drag.target), drag.expected_frame)
        {
            self.cancel_gesture(false);
            self.refresh();
            return None;
        }
        if drag.input_owner.is_some() && !self.active.input_ready() {
            return Some(drag);
        }

        let mut updated = drag;
        match (drag.target, drag.kind) {
            (target, GestureKind::Move) => {
                let origin = NSPoint::new(
                    drag.start_frame.origin.x + mouse.x - drag.start_mouse.x,
                    drag.start_frame.origin.y + mouse.y - drag.start_mouse.y,
                );
                if target == DragTarget::StandaloneBubble {
                    let frame = NSRect::new(origin, drag.start_frame.size);
                    let visible = standalone_visible_frame(self.mtm, frame)?;
                    let origin = clamp_window_origin(frame, visible);
                    self.bubble_panel.setFrameOrigin(origin);
                } else {
                    self.panel.setFrameOrigin(origin);
                    self.clamp_panel();
                }
                updated.expected_frame = self.drag_frame(target);
            }
            (DragTarget::StandaloneBubble, GestureKind::Resize) => return None,
            (DragTarget::Character, GestureKind::Resize) => {
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
                        self.cancel_gesture(false);
                        self.refresh();
                        return None;
                    }
                    Err(_) => {
                        self.cancel_gesture(true);
                        return None;
                    }
                };
                let frame = resize_frame(self.display_geometry, drag.start_top_left, scene.scale);
                self.set_content_frame(frame, scene.scale, false);
                updated.expected_frame = self.panel.frame();
                updated.expected_scale = scene.scale;
                self.last_scene.scale = scene.scale;
                self.update_bubble_frame_scene(&self.last_scene.clone());
            }
        }
        Some(updated)
    }

    fn finish_gesture(&mut self, drag: DragState) {
        self.pointer_event_started_at = None;
        let final_state = self.shared.lock().ok().map(|state| state.scene());
        let final_frame = self.drag_frame(drag.target);
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
            self.cancel_gesture_for(Some(drag), false);
            self.refresh();
            return;
        }
        let changed = !rect_nearly_equal(final_frame, drag.start_frame)
            || (scene.scale - drag.start_scale).abs() > f64::EPSILON;
        if changed {
            self.persist_drag_geometry(
                &scene,
                drag.target,
                final_frame,
                drag.kind == GestureKind::Resize,
            );
        }
        self.apply_pending_bubble_updates();
        self.update_pointer_policy(&scene);
        self.publish_presentation_checkpoint(&scene);
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
        let Some(scene) = self.shared.lock().ok().map(|state| state.scene()) else {
            return;
        };
        if scene.shutdown
            || !presentation_matches_scene(&scene, &self.last_scene)
            || status_fields_changed(&scene, &self.last_scene)
        {
            self.refresh();
            if scene.shutdown {
                return;
            }
        }
        let drag = self
            .root
            .ivars()
            .drag
            .get()
            .or_else(|| self.bubble_root.ivars().drag.get());
        if drag_survives_screen_change(
            drag,
            drag.map(|drag| self.drag_frame(drag.target))
                .unwrap_or(self.panel.frame()),
        ) {
            // The moving window entered another display; mouse-up commits it.
            return;
        }
        self.cancel_gesture(false);
        self.cancel_pointer_state();
        self.set_hover(false, false);
        // Refresh above reconciles unseen source changes before clamping.
        self.clamp_panel();
        self.save_position_only(None);
    }

    fn cancel_gesture(&mut self, drain_pending: bool) {
        let drag = self
            .root
            .ivars()
            .drag
            .get()
            .or_else(|| self.bubble_root.ivars().drag.get());
        self.cancel_gesture_for(drag, drain_pending);
    }
    fn cancel_gesture_for(&mut self, drag: Option<DragState>, drain_pending: bool) {
        let accepted_frame = drag.map(|drag| self.drag_frame(drag.target));
        let scene = drag.and_then(|_| self.shared.lock().ok().map(|state| state.scene()));
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
            && accepted_frame.is_some_and(|frame| rect_nearly_equal(frame, drag.expected_frame))
        {
            self.persist_drag_geometry(
                &scene,
                drag.target,
                drag.expected_frame,
                drag.kind == GestureKind::Resize,
            );
        }
        if drain_pending {
            self.apply_pending_bubble_updates();
        }
    }

    fn drag_frame(&self, target: DragTarget) -> NSRect {
        match target {
            DragTarget::Character => self.panel.frame(),
            DragTarget::StandaloneBubble => self.bubble_panel.frame(),
        }
    }

    fn persist_drag_geometry(
        &mut self,
        scene: &Scene,
        target: DragTarget,
        frame: NSRect,
        resized: bool,
    ) {
        if target == DragTarget::StandaloneBubble {
            if let Some(body) = self.bubble_geometry {
                let origin = (frame.origin.x + body.body.x, frame.origin.y + body.body.y);
                self.pending_standalone_body_origin = Some(origin);
                self.standalone_position_unsaved = true;
                self.save_position_only(Some(Some(origin)));
            }
        } else if resized {
            self.save_presentation(
                scene,
                PresentationPatch {
                    scale: Some(scene.scale),
                    ..PresentationPatch::default()
                },
                true,
                false,
            );
        } else {
            self.save_position_only(None);
        }
    }

    fn resize(&mut self, scale: f64) {
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
        self.clamp_panel_origin();
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
    fn reset_position(&mut self) {
        let size = self.panel.frame().size;
        let origin = default_origin(size, self.mtm);
        self.panel.setFrameOrigin(origin);
        self.standalone_reset_pending = true;
        self.standalone_position_unsaved = true;
        self.pending_standalone_body_origin = None;
        self.bubble_geometry = None;
        self.bubble_geometry_attached = false;
        self.pending_bubble_scene = None;
        self.bubble_content_dirty = true;
        self.clamp_panel_origin();
        // Position remains a runtime value until the candidate save succeeds.
    }
    fn clamp_panel(&mut self) {
        self.clamp_panel_for(self.last_scene.bubble_placement);
    }

    fn clamp_panel_for(&mut self, bubble_placement: BubblePlacement) {
        self.clamp_panel_origin();
        self.update_bubble_frame_for(bubble_placement);
    }

    fn clamp_panel_origin(&mut self) {
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
    }
    fn update_bubble_frame(&mut self) {
        self.update_bubble_frame_for(self.last_scene.bubble_placement);
    }

    fn update_bubble_frame_for(&mut self, bubble_placement: BubblePlacement) {
        let mut scene = self.last_scene.clone();
        scene.bubble_placement = bubble_placement;
        self.update_bubble_frame_scene(&scene);
    }

    fn update_bubble_frame_scene(&mut self, scene: &Scene) {
        if self.bubble_placement_tracking_locked()
            || self.composer_marked()
            || self.cards.is_composing()
        {
            self.pending_bubble_scene = Some(scene.clone());
            self.queue_language_apply();
            return;
        }
        self.pending_bubble_scene = None;
        self.apply_bubble_frame_scene(scene);
    }

    fn current_anchor_key(&self, scale: f64) -> AnchorKey {
        let artwork = if matches!(&self.active, PreparedCharacter::Png(_)) {
            self.image_view.frame()
        } else {
            self.display_geometry.canvas_frame(scale)
        };
        AnchorKey {
            frame: self.displayed_frame,
            artwork,
            backend_epoch: self.active.token().backend_epoch,
            viewport: PresentationViewport {
                width: artwork.size.width,
                height: artwork.size.height,
                backing_scale: self.panel.backingScaleFactor(),
                epoch: self.viewport.epoch,
            },
            scale,
        }
    }

    fn refresh_speech_anchor(&mut self, scene: &Scene) -> bool {
        let key = self.current_anchor_key(scene.scale);
        let rig = matches!(&self.active, PreparedCharacter::Rig(_));
        if !anchor_query_needed(self.cached_anchor.as_ref(), key, rig) {
            return false;
        }
        let panel = self.panel.frame();
        let rect = (
            panel.origin.x + key.artwork.origin.x,
            panel.origin.y + key.artwork.origin.y,
            key.artwork.size.width,
            key.artwork.size.height,
        );
        let result = self
            .active
            .speech_anchor_snapshot(key.frame, rect, key.viewport);
        let cacheable = result.as_ref().is_ok_and(|snapshot| {
            snapshot.backend_epoch == key.backend_epoch && snapshot.viewport == Some(key.viewport)
        });
        let status = presentation_anchor_status(&result, key, panel.origin, rig);
        let (selected, invalidated) = accept_visual_anchor(self.visual_anchor, key, status, rig);
        self.visual_anchor = selected;
        self.cached_anchor = (!rig && cacheable).then_some(CachedAnchor { key });
        invalidated
    }

    fn attached_bubble_geometry(
        &self,
        placement: BubblePlacement,
        scale: f64,
    ) -> Option<BubbleGeometry> {
        let visible = panel_visible_frame(&self.panel, self.mtm)?;
        let pet = self.panel.frame();
        let body = self.bubble_layout.body_size;
        let relative = visual_relative_for(self.visual_anchor, self.current_anchor_key(scale));
        Some(placed_attached_bubble(
            relative,
            bubble_rect(pet),
            (body.width.max(0.0), body.height.max(0.0)),
            bubble_rect(visible),
            placement,
        ))
    }

    fn apply_bubble_frame_scene(&mut self, scene: &Scene) {
        if !scene.visible && self.bubble_geometry.is_none() && self.bubble_content_dirty {
            // Measure the actual body before seeding an initially hidden bubble.
            return;
        }
        let geometry = if scene.visible {
            self.attached_bubble_geometry(scene.bubble_placement, scene.scale)
        } else {
            let origin = preferred_standalone_origin(
                if self.standalone_reset_pending || self.standalone_position_unsaved {
                    None
                } else {
                    self.prefs.standalone_bubble_position()
                },
                self.pending_standalone_body_origin,
                self.bubble_geometry,
                self.bubble_geometry_attached,
            )
            .or_else(|| {
                self.attached_bubble_geometry(scene.bubble_placement, scene.scale)
                    .map(bubble_body_origin)
            });
            origin.and_then(|origin| {
                let body = self.bubble_layout.body_size;
                let frame = NSRect::new(NSPoint::new(origin.0, origin.1), body);
                let visible = standalone_visible_frame(self.mtm, frame)?;
                Some(place_standalone_bubble(
                    origin,
                    (body.width.max(0.0), body.height.max(0.0)),
                    bubble_rect(visible),
                ))
            })
        };
        let Some(geometry) = geometry else { return };
        // Only a successfully applied, screen-clamped hidden body becomes a
        // preference. Failed placement leaves the captured origin available.
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
        self.bubble_geometry_attached = scene.visible;
        if !scene.visible {
            let body_origin = bubble_body_origin(geometry);
            if self.prefs.standalone_bubble_position() != Some(body_origin)
                && (!self.standalone_position_unsaved
                    || self.pending_standalone_body_origin != Some(body_origin))
            {
                self.pending_standalone_body_origin = Some(body_origin);
                self.standalone_position_unsaved = true;
                self.save_position_only(Some(Some(body_origin)));
            }
            if !self.standalone_position_unsaved {
                self.pending_standalone_body_origin = None;
            }
        }
        if !local_unchanged || self.bubble_layout_dirty {
            self.layout_bubble_children();
        }
    }
    fn refresh_bubble_content(&mut self, scene: &Scene) {
        if self.measure_bubble_content(scene) {
            self.update_bubble_frame_scene(scene);
        }
    }

    fn measure_bubble_content(&mut self, scene: &Scene) -> bool {
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
            return false;
        }
        if self.pending_bubble_content {
            self.bubble_content_dirty = true;
            self.pending_bubble_content = false;
        }
        if !self.bubble_content_dirty && self.dialogue_text == dialogue {
            return false;
        }
        let show_status = self.prefs.show_status_indicators();
        self.cards.set_composition_active(self.composer_marked());
        self.cards.set_show_status_indicators(show_status);
        self.bubble_root.set_opaque_surface(show_status);
        let status = if let Some(feedback) = self.cli_preference_feedback.as_ref() {
            feedback.clone()
        } else if show_status {
            status_indicator_summary(self.locale, self.status_summary)
        } else {
            status_text(scene, self.locale)
        };
        let offline_warning = disconnected_text(scene, self.locale);
        let feedback_summary = self
            .worktree_feedback
            .as_ref()
            .map(|feedback| feedback.compact_summary(self.locale))
            .unwrap_or("");
        let mut disconnect = worktree_feedback_detail(
            self.worktree_feedback.as_ref(),
            self.locale,
            &offline_warning,
        );
        if !show_status && !dialogue.is_empty() {
            if let Some(feedback) = self.cli_preference_feedback.as_ref() {
                disconnect = if disconnect.is_empty() {
                    feedback.clone()
                } else {
                    format!("{feedback}\n{disconnect}")
                };
            }
        }
        let compact_secondary = compact_worktree_secondary(feedback_summary, &offline_warning);
        if !self.bubble_content_dirty
            && self.dialogue_text == dialogue
            && self.status_text == status
            && self.disconnect_text == disconnect
        {
            return false;
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
        self.bubble_layout.feedback_summary = feedback_summary.to_owned();
        self.bubble_layout.secondary = compact_secondary;
        self.dialogue
            .setMaximumNumberOfLines(if feedback_summary.is_empty() { 2 } else { 0 });
        self.dialogue.setToolTip(
            (!feedback_summary.is_empty())
                .then(|| NSString::from_str(&self.disconnect_text))
                .as_deref(),
        );
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
            .setStringValue(&NSString::from_str(&self.bubble_layout.secondary));
        self.bubble_content_dirty = false;
        self.measure_bubble(scene);
        true
    }

    fn remeasure_bubble(&mut self, scene: &Scene) {
        self.measure_bubble(scene);
        self.update_bubble_frame_scene(scene);
    }

    fn measure_bubble(&mut self, scene: &Scene) {
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
            &self.bubble_layout.secondary,
            &secondary_font,
            &secondary_color,
            &secondary_style,
        );

        let natural_primary = measure_attributed_field(&self.bubble, 10_000.0);
        let natural_secondary = if self.bubble_layout.secondary.is_empty() {
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
        let visible = if scene.visible {
            panel_visible_frame(&self.panel, self.mtm)
        } else {
            let body = self.bubble_layout.body_size;
            let origin = preferred_standalone_origin(
                if self.standalone_reset_pending || self.standalone_position_unsaved {
                    None
                } else {
                    self.prefs.standalone_bubble_position()
                },
                self.pending_standalone_body_origin,
                self.bubble_geometry,
                self.bubble_geometry_attached,
            )
            .unwrap_or((self.panel.frame().origin.x, self.panel.frame().origin.y));
            standalone_visible_frame(
                self.mtm,
                NSRect::new(NSPoint::new(origin.0, origin.1), body),
            )
        }
        .unwrap_or(NSRect::new(
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

        let warning_metrics = if self.bubble_layout.secondary.is_empty() {
            None
        } else {
            Some(measure_attributed_field(
                &self.dialogue,
                compact_content_width,
            ))
        };
        let feedback_summary_height = if self.bubble_layout.feedback_summary.is_empty() {
            0.0
        } else {
            set_attributed_field_text(
                &self.dialogue,
                &self.bubble_layout.feedback_summary,
                &secondary_font,
                &secondary_color,
                &secondary_style,
            );
            let summary_metrics = measure_attributed_field(&self.dialogue, compact_content_width);
            set_attributed_field_text(
                &self.dialogue,
                &self.bubble_layout.secondary,
                &secondary_font,
                &secondary_color,
                &secondary_style,
            );
            summary_metrics
                .size
                .height
                .max(BUBBLE_SECONDARY_FONT_SIZE + 2.0)
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
                    .max(feedback_summary_height)
            })
            .unwrap_or(0.0);
        self.bubble_layout.feedback_summary_height = feedback_summary_height;
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
        let cards_height = self.desired_cards_height();
        let minimum_cards = self.minimum_cards_height();
        let spacing = expanded_spacing(
            body_height_cap,
            cards_height,
            minimum_cards,
            show_status,
            message_height > 0.0,
        );
        let expanded_height = (spacing.inset
            + if show_status {
                STATUS_ROW_HEIGHT
                    + if message_height > 0.0 {
                        spacing.gap
                    } else {
                        0.0
                    }
            } else {
                0.0
            }
            + message_height
            + cards_height
            + spacing.gap
                * (if show_status && message_height == 0.0 {
                    1.0
                } else {
                    2.0
                })
            + BUBBLE_CONTROL_HEIGHT
            + spacing.inset)
            .min(
                (visible_height * 0.78).min(530.0).max(
                    expanded_height_budget(
                        cards_height,
                        minimum_cards,
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
        if let Some(cell) = self.dialogue.cell() {
            let detail = if self.bubble_layout.feedback_summary.is_empty() {
                &self.bubble_layout.secondary
            } else {
                &self.disconnect_text
            };
            set_accessibility_cell_text(&cell, detail, Some(detail));
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
        // The caller commits this measurement with its current visual anchor.
    }

    fn layout_bubble_children(&mut self) {
        if self.composer_marked() || self.cards.is_composing() {
            self.bubble_layout_dirty = true;
            self.pending_bubble_scene = Some(self.last_scene.clone());
            self.queue_language_apply();
            return;
        }
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
                self.desired_cards_height(),
                self.minimum_cards_height(),
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
            let feedback_compact = self.bubble_mode == BubbleMode::Compact
                && !self.bubble_layout.feedback_summary.is_empty();
            let status_bottom = body.y + spacing.inset + BUBBLE_CONTROL_HEIGHT + spacing.gap;
            let status_height = if feedback_compact {
                STATUS_ROW_HEIGHT.min(
                    (content_top
                        - status_bottom
                        - self.bubble_layout.feedback_summary_height
                        - spacing.gap)
                        .max(0.0),
                )
            } else {
                STATUS_ROW_HEIGHT
            };
            let frame = bounded_frame(
                NSRect::new(
                    NSPoint::new(
                        content_x,
                        if feedback_compact {
                            status_bottom
                        } else {
                            content_top - status_height
                        },
                    ),
                    NSSize::new(content_width, status_height),
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
                let warning_height = if self.bubble_layout.secondary.is_empty() {
                    0.0
                } else {
                    measure_attributed_field(&self.dialogue, content_width.max(1.0))
                        .size
                        .height
                        .max(BUBBLE_SECONDARY_FONT_SIZE + 2.0)
                };
                let feedback_compact = !self.bubble_layout.feedback_summary.is_empty();
                let available_top = if show_status && !feedback_compact {
                    (status_row.origin.y - BUBBLE_CONTENT_GAP).max(body.y)
                } else {
                    content_top.max(body.y)
                };
                let footer_top = footer.origin.y + footer.size.height + BUBBLE_CONTENT_GAP;
                let available_bottom = if show_status && feedback_compact {
                    (status_row.origin.y + status_row.size.height + BUBBLE_CONTENT_GAP)
                        .max(footer_top)
                        .min(available_top)
                } else {
                    footer_top.min(available_top)
                };
                let (primary_frame, warning_frame) = if feedback_compact {
                    feedback_compact_frames(
                        content_x,
                        content_width,
                        available_bottom,
                        available_top,
                        primary_height,
                        warning_height,
                    )
                } else {
                    let warning_frame = if warning_height > 0.0 {
                        let desired = NSRect::new(
                            NSPoint::new(
                                content_x,
                                available_top
                                    - primary_height
                                    - BUBBLE_CONTENT_GAP
                                    - warning_height,
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
                    if show_status {
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
                    }
                };
                self.bubble.setHidden(
                    (show_status && primary.is_empty()) || primary_frame.size.height <= 0.0,
                );
                self.bubble.setFrame(primary_frame);
                self.dialogue.setHidden(
                    self.bubble_layout.secondary.is_empty() || warning_frame.size.height <= 0.0,
                );
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
                let bottom = collapse_frame.origin.y + collapse_frame.size.height + spacing.gap;
                let message_top = expanded_message_top(
                    status_row.origin.y,
                    content_top,
                    show_status,
                    self.bubble_layout.message_content_height > 0.0,
                    spacing,
                );
                let desired_message = self
                    .bubble_layout
                    .message_content_height
                    .min(BUBBLE_MESSAGE_MAX_HEIGHT);
                let desired_cards = self.desired_cards_height();
                let heights = expanded_heights(
                    body.height,
                    desired_cards,
                    desired_message,
                    self.minimum_cards_height(),
                    show_status,
                    spacing,
                );
                let cards_height = heights.cards;
                let message_height = heights.message;
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
                    self.cards.set_composition_active(self.composer_marked());
                    self.cards.set_frame(cards_frame);
                    self.layout_reply_children();
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
        let scene = self.last_scene.clone();
        self.refresh_bubble_content(&scene);
    }

    fn collapse_bubble(&mut self) {
        if self.bubble_mode == BubbleMode::Compact
            || self.composer_marked()
            || self.cards.is_composing()
        {
            return;
        }
        self.close_reply();
        self.bubble_mode = BubbleMode::Compact;
        self.bubble_content_dirty = true;
        let scene = self.last_scene.clone();
        self.refresh_bubble_content(&scene);
        if self.bubble_panel.isKeyWindow() {
            let _ = self.bubble_panel.makeFirstResponder(None);
        }
    }

    fn reset_bubble_mode(&mut self) {
        if self.composer_marked() || self.cards.is_composing() {
            return;
        }
        self.close_reply();
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
    fn open_settings_at(&mut self, anchor: NSRect) {
        self.settings_anchor = Some(anchor);
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
        if let Some(frame) = anchor_visible_frame(self.mtm, anchor) {
            self.menu_panel.show_at(anchor, frame);
            self.refresh_character_menu();
            if self.menu_panel.is_visible() {
                self.menu_panel.refresh_bubble_color_controls();
                self.queue_language_apply();
            }
        }
    }

    fn sync_status_menu(&self, scene: &Scene) {
        if self.status_menu_tracking {
            return;
        }
        self.sync_recover_item(scene);
        let visible = menu_bar_visible(scene, self.prefs.menu_bar_mode());
        if self._status_item.isVisible() != visible {
            self._status_item.setVisible(visible);
        }
    }

    fn sync_recover_item(&self, scene: &Scene) {
        if let Some(recover) = self.status_menu.itemAtIndex(0) {
            let needed = scene.needs_recovery_entry();
            if recover.isHidden() == needed {
                recover.setHidden(!needed);
            }
            if recover.isEnabled() != needed {
                recover.setEnabled(needed);
            }
        }
    }

    fn update_dialogue_choices(&mut self, listing: &PackListing) {
        let default_name = text(self.locale, Message::RubeliaBuiltIn);
        if !self.dialogue_choices.is_empty()
            && self.dialogue_choices[0].generation == listing.generation
            && self.dialogue_choices[0].name == default_name
        {
            return;
        }
        let mut choices = Vec::with_capacity(
            1 + listing
                .packs
                .iter()
                .map(|pack| pack.revisions.len())
                .sum::<usize>()
                + usize::from(self.external_dialogue_target.is_some()),
        );
        choices.push(DialogueChoice {
            target: DialogueTarget::Character("default".to_owned()),
            name: default_name.to_owned(),
            reference: Some(CharacterRef::builtin()),
            generation: listing.generation,
        });
        for pack in &listing.packs {
            for revision in &pack.revisions {
                choices.push(DialogueChoice {
                    target: DialogueTarget::Character(pack.id.clone()),
                    name: format!("{} #{}", pack.name, revision),
                    reference: Some(CharacterRef {
                        id: pack.id.clone(),
                        revision: *revision,
                    }),
                    generation: listing.generation,
                });
            }
        }
        if let Some(DialogueTarget::ExternalAssets(path)) = &self.external_dialogue_target {
            choices.push(DialogueChoice {
                target: DialogueTarget::ExternalAssets(path.clone()),
                name: Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path)
                    .to_owned(),
                reference: None,
                generation: listing.generation,
            });
        }
        self.dialogue_choices = choices;
        self.dialogue_editor.sync_choices(
            &self.dialogue_choices,
            self.dialogue_target.as_ref(),
            Some(&self.active.token().reference),
        );
    }

    fn sync_dialogue_editor_content(&mut self, retry_metadata: bool) {
        if !self.dialogue_editor.is_visible() {
            return;
        }
        let choice = self.dialogue_editor.choice();
        if choice.as_ref().is_some_and(|selected| {
            !self
                .dialogue_choices
                .iter()
                .any(|available| available == selected)
        }) {
            if self.editor_ready || self.editor_error.is_none() {
                self.editor_ready = false;
                self.editor_error =
                    Some(text(self.locale, Message::DialogueTargetUnavailable).to_owned());
                self.editor_content_dirty = true;
            }
        }
        let identity_changed = match (choice.as_ref(), self.editor_cached_choice.as_ref()) {
            (Some(current), Some(previous)) => {
                current.target != previous.target
                    || current.reference != previous.reference
                    || current.generation != previous.generation
            }
            (None, None) => false,
            _ => true,
        };
        if choice != self.editor_cached_choice {
            self.editor_cached_choice = choice.clone();
        }
        if identity_changed {
            self.editor_metadata = None;
            self.editor_ready = false;
            self.editor_error = None;
            self.editor_content_dirty = true;
            if let Some(choice) = &choice {
                if !self
                    .dialogue_choices
                    .iter()
                    .any(|available| available == choice)
                {
                    self.editor_error =
                        Some(text(self.locale, Message::DialogueTargetUnavailable).to_owned());
                } else if self.active_dialogue_matches(&choice.target)
                    && (choice.reference.is_none()
                        || choice.reference.as_ref() == Some(&self.active.token().reference))
                {
                    self.editor_metadata = self.active.metadata().cloned();
                    self.editor_ready = true;
                } else if self.external_dialogue_target.as_ref() == Some(&choice.target) {
                    self.editor_metadata = self.external_dialogue_metadata.clone();
                    self.editor_ready = true;
                } else if let Some(reference) = choice.reference.as_ref() {
                    if !retry_metadata {
                        if let Err(error) = self
                            .packs
                            .request_dialogue_metadata(reference.clone(), choice.generation)
                        {
                            self.editor_error = Some(format!(
                                "{}: {error}",
                                text(self.locale, Message::DialogueTargetUnavailable)
                            ));
                        }
                    }
                } else {
                    self.editor_error =
                        Some(text(self.locale, Message::DialogueTargetUnavailable).to_owned());
                }
            }
        }
        if retry_metadata && !self.editor_ready {
            if let Some(choice) = choice.as_ref().filter(|choice| {
                self.dialogue_choices
                    .iter()
                    .any(|available| available == *choice)
                    && (!self.active_dialogue_matches(&choice.target)
                        || choice.reference.as_ref() != Some(&self.active.token().reference))
            }) {
                if let Some(reference) = choice.reference.as_ref() {
                    match self
                        .packs
                        .retry_dialogue_metadata(reference.clone(), choice.generation)
                    {
                        Ok(()) => self.editor_error = None,
                        Err(error) => {
                            self.editor_error = Some(format!(
                                "{}: {error}",
                                text(self.locale, Message::DialogueTargetUnavailable)
                            ));
                        }
                    }
                    self.editor_content_dirty = true;
                }
            }
        }
        if !self.editor_ready && self.editor_error.is_none() {
            if let Some(choice) = choice.as_ref() {
                if let Some(reference) = choice.reference.as_ref() {
                    if let Some(result) = self
                        .packs
                        .cached_dialogue_metadata(reference, choice.generation)
                    {
                        match result {
                            Ok(metadata) => {
                                self.editor_metadata = metadata.metadata;
                                self.editor_ready = true;
                            }
                            Err(error) => {
                                self.editor_error = Some(format!(
                                    "{}: {error}",
                                    text(self.locale, Message::DialogueTargetUnavailable)
                                ))
                            }
                        }
                        self.editor_content_dirty = true;
                    } else if let Err(error) = self
                        .packs
                        .request_dialogue_metadata(reference.clone(), choice.generation)
                    {
                        self.editor_error = Some(format!(
                            "{}: {error}",
                            text(self.locale, Message::DialogueTargetUnavailable)
                        ));
                        self.editor_content_dirty = true;
                    }
                }
            }
        }
        if self.editor_content_dirty {
            match self.dialogue_editor.sync_content(
                self.prefs.dialogue_overrides(),
                self.editor_metadata.as_ref(),
                self.editor_ready,
                self.editor_error.as_deref(),
            ) {
                HydrationSync::Settled => self.editor_content_dirty = false,
                HydrationSync::DeferredMarked => self.queue_language_apply(),
            }
        }
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

    fn dialogue_choice_valid(&self, choice: &DialogueChoice) -> bool {
        let listing = self.packs.cached_list();
        choice.generation == listing.generation
            && self.dialogue_choices.iter().any(|available| {
                available.target == choice.target
                    && available.reference == choice.reference
                    && available.generation == choice.generation
            })
            && match &choice.reference {
                Some(reference) => self
                    .packs
                    .dialogue_reference_valid(reference, choice.generation),
                None => self.external_dialogue_target.as_ref() == Some(&choice.target),
            }
    }

    fn gui_dialogue_selection(baseline: &DialogueDraftBaseline) -> DialogueSelection {
        DialogueSelection {
            identity: DialogueIdentity::from(&baseline.choice),
            locale: match baseline.locale {
                UiLocale::Ko => DialogueLanguage::Ko,
                UiLocale::En => DialogueLanguage::En,
            },
            slot: baseline.slot,
        }
    }

    fn gui_dialogue_baseline(&self, draft: &DialogueDraftBaseline) -> DialogueBaseline {
        DialogueBaseline {
            metadata_token: draft.metadata_token.clone(),
            override_entry: draft.override_entry.clone(),
            target_overrides_token: target_overrides_token(
                self.prefs.dialogue_overrides(),
                &draft.choice.target,
            ),
        }
    }

    fn persist_dialogue_entry(
        &mut self,
        draft: &DialogueDraftBaseline,
        baseline: &DialogueBaseline,
        value: Option<String>,
    ) {
        let selection = Self::gui_dialogue_selection(draft);
        match self.commit_dialogue_mutation(&selection, baseline, value) {
            Ok(_) => {
                self.dialogue_editor
                    .saved(&draft.choice.target, draft.locale, draft.slot);
                self.dialogue_editor.set_error(None);
                self.editor_content_dirty = true;
                self.sync_dialogue_editor_content(false);
            }
            Err(error) => self.dialogue_editor.set_error(Some(&format!(
                "{}: {}",
                text(
                    self.locale,
                    if matches!(
                        error.code,
                        "metadata_conflict" | "entry_conflict" | "target_conflict"
                    ) {
                        Message::DialogueDraftConflict
                    } else {
                        Message::DialogueSaveFailed
                    }
                ),
                error.detail
            ))),
        }
    }

    fn save_dialogue_entry(&mut self, reset: bool) {
        let Some((choice, locale, slot, value)) = self.dialogue_editor.edit() else {
            if self.dialogue_editor.has_conflict() {
                self.dialogue_editor
                    .set_error(Some(text(self.locale, Message::DialogueDraftConflict)));
            }
            return;
        };
        let Some(draft) = self.dialogue_editor.mutation_baseline() else {
            return;
        };
        if !self.dialogue_choice_valid(&choice)
            || draft.choice.target != choice.target
            || draft.choice.reference != choice.reference
            || draft.choice.generation != choice.generation
            || draft.locale != locale
            || draft.slot != slot
        {
            self.dialogue_editor
                .set_error(Some(text(self.locale, Message::DialogueTargetUnavailable)));
            return;
        }
        let baseline = self.gui_dialogue_baseline(&draft);
        if reset {
            if self.dialogue_editor.is_dirty() {
                self.confirm_dialogue_reset(draft, baseline, value, false);
            } else {
                self.persist_dialogue_entry(&draft, &baseline, None);
            }
        } else if self.dialogue_editor.is_dirty() && value.len() <= 2048 {
            self.persist_dialogue_entry(&draft, &baseline, Some(value));
        }
    }

    fn confirm_reset_character_dialogue(&mut self) {
        let Some((choice, locale, slot, value)) = self.dialogue_editor.edit() else {
            if self.dialogue_editor.has_conflict() {
                self.dialogue_editor
                    .set_error(Some(text(self.locale, Message::DialogueDraftConflict)));
            }
            return;
        };
        let Some(draft) = self.dialogue_editor.mutation_baseline() else {
            return;
        };
        if !self.dialogue_choice_valid(&choice)
            || draft.choice.target != choice.target
            || draft.choice.reference != choice.reference
            || draft.choice.generation != choice.generation
            || draft.locale != locale
            || draft.slot != slot
        {
            self.dialogue_editor
                .set_error(Some(text(self.locale, Message::DialogueTargetUnavailable)));
            return;
        }
        // Capture every override for the target before opening the nested modal loop.
        let baseline = self.gui_dialogue_baseline(&draft);
        self.confirm_dialogue_reset(draft, baseline, value, true);
    }

    fn confirm_dialogue_reset(
        &self,
        draft: DialogueDraftBaseline,
        baseline: DialogueBaseline,
        value: String,
        whole: bool,
    ) {
        let locale = self.locale;
        DispatchQueue::main().exec_async(move || {
            let Some(mtm) = with_ui_read(|ui| ui.mtm) else {
                return;
            };
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str(text(
                locale,
                if whole {
                    Message::DialogueResetConfirm
                } else {
                    Message::DialogueResetDraftConfirm
                },
            )));
            alert.setInformativeText(&NSString::from_str(text(
                locale,
                if whole {
                    Message::DialogueResetConfirmHelp
                } else {
                    Message::DialogueResetDraftConfirmHelp
                },
            )));
            alert.addButtonWithTitle(&NSString::from_str(text(
                locale,
                if whole {
                    Message::DialogueResetCharacter
                } else {
                    Message::DialogueResetEntry
                },
            )));
            alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
            if alert.runModal() != NSAlertFirstButtonReturn {
                return;
            }
            with_ui_mut(|ui| {
                let current = ui.dialogue_editor.edit();
                let same_edit = current
                    .as_ref()
                    .is_some_and(|(choice, language, event, text)| {
                        choice.target == draft.choice.target
                            && choice.reference == draft.choice.reference
                            && choice.generation == draft.choice.generation
                            && *language == draft.locale
                            && *event == draft.slot
                            && text == &value
                    });
                if !ui.dialogue_choice_valid(&draft.choice)
                    || !same_edit
                    || ui.dialogue_editor.mutation_baseline().as_ref() != Some(&draft)
                {
                    ui.dialogue_editor
                        .set_error(Some(text(ui.locale, Message::DialogueDraftConflict)));
                    return;
                }
                if whole {
                    let identity = DialogueIdentity::from(&draft.choice);
                    match ui.commit_dialogue_character_reset(
                        &identity,
                        &baseline.metadata_token,
                        &baseline.target_overrides_token,
                    ) {
                        Ok(_) => {
                            ui.dialogue_editor.reset(&draft.choice.target);
                            ui.dialogue_editor.set_error(None);
                            ui.editor_content_dirty = true;
                            ui.sync_dialogue_editor_content(false);
                        }
                        Err(error) => ui.dialogue_editor.set_error(Some(&format!(
                            "{}: {}",
                            text(
                                ui.locale,
                                if matches!(
                                    error.code,
                                    "metadata_conflict" | "entry_conflict" | "target_conflict"
                                ) {
                                    Message::DialogueDraftConflict
                                } else {
                                    Message::DialogueSaveFailed
                                }
                            ),
                            error.detail
                        ))),
                    }
                } else {
                    ui.persist_dialogue_entry(&draft, &baseline, None);
                }
            });
        });
    }

    fn change_observation_machine(&mut self, id: String, selected: bool) {
        if selected {
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
        if candidate != *self.prefs.observation() {
            let current = self.prefs.observation();
            let patch = crate::preferences::PreferencePatch {
                observation_local: (candidate.local != current.local).then_some(candidate.local),
                observation_remote: (candidate.remote != current.remote)
                    .then_some(candidate.remote),
                observation_machines: (candidate.machines != current.machines)
                    .then_some(candidate.machines),
                ..Default::default()
            };
            if let Err(error) = self.commit_gui_observation_patch(patch) {
                self.menu_panel.revert_observation_controls();
                self.sync_observation_panel();
                self.queue_bubble_appearance_error(
                    if error.code == "revision_conflict" {
                        Message::PreferenceRevisionConflict
                    } else {
                        Message::ObservationSaveFailed
                    },
                    &error.detail,
                );
                return;
            }
        }
        self.menu_panel.revert_observation_controls();
        self.sync_observation_panel();
        self.settle_preference_operations();
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
        if self.menu_panel.is_visible() {
            if let Some(anchor) = self.settings_anchor {
                if let Some(frame) = anchor_visible_frame(self.mtm, anchor) {
                    self.menu_panel.reanchor_at(anchor, frame);
                }
            }
        }
    }

    fn quit(&mut self) {
        self.pending_standalone_body_origin = None;
        self.cancel_gesture(false);
        self.cancel_pointer_state();
        self.shutdown();
        if let Ok(mut state) = self.shared.lock() {
            state.request_shutdown();
            self.sync_status_menu(&state.scene());
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
        self.dialogue_editor.shutdown();
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

fn bubble_rect(frame: NSRect) -> BubbleRect {
    BubbleRect {
        x: frame.origin.x,
        y: frame.origin.y,
        width: frame.size.width,
        height: frame.size.height,
    }
}

fn bubble_body_origin(geometry: BubbleGeometry) -> (f64, f64) {
    (
        geometry.window.x + geometry.body.x,
        geometry.window.y + geometry.body.y,
    )
}

fn attached_body_origin(geometry: Option<BubbleGeometry>, attached: bool) -> Option<(f64, f64)> {
    geometry.filter(|_| attached).map(bubble_body_origin)
}

fn preferred_standalone_origin(
    saved: Option<(f64, f64)>,
    pending: Option<(f64, f64)>,
    geometry: Option<BubbleGeometry>,
    attached: bool,
) -> Option<(f64, f64)> {
    saved
        .or(pending)
        .or_else(|| attached_body_origin(geometry, attached))
}

fn clamp_window_origin(frame: NSRect, visible: NSRect) -> NSPoint {
    NSPoint::new(
        frame.origin.x.clamp(
            visible.origin.x,
            (visible.origin.x + visible.size.width - frame.size.width).max(visible.origin.x),
        ),
        frame.origin.y.clamp(
            visible.origin.y,
            (visible.origin.y + visible.size.height - frame.size.height).max(visible.origin.y),
        ),
    )
}

fn screen_overlap(frame: NSRect, visible: NSRect) -> f64 {
    let left = frame.origin.x.max(visible.origin.x);
    let right = (frame.origin.x + frame.size.width).min(visible.origin.x + visible.size.width);
    let bottom = frame.origin.y.max(visible.origin.y);
    let top = (frame.origin.y + frame.size.height).min(visible.origin.y + visible.size.height);
    (right - left).max(0.0) * (top - bottom).max(0.0)
}

fn anchor_visible_frame(mtm: MainThreadMarker, anchor: NSRect) -> Option<NSRect> {
    let screens = NSScreen::screens(mtm);
    let center = NSPoint::new(
        anchor.origin.x + anchor.size.width * 0.5,
        anchor.origin.y + anchor.size.height * 0.5,
    );
    for screen in screens.iter() {
        if point_in_rect(center, screen.frame()) {
            return Some(screen.visibleFrame());
        }
    }
    // The originating display was removed. Use a remaining display, not
    // the now offscreen status button or pet window.
    NSScreen::mainScreen(mtm)
        .or_else(|| screens.firstObject())
        .map(|screen| screen.visibleFrame())
}

fn standalone_visible_frame(mtm: MainThreadMarker, frame: NSRect) -> Option<NSRect> {
    let screens = NSScreen::screens(mtm);
    let mut best = None;
    let mut overlap = 0.0;
    for screen in screens.iter() {
        let visible = screen.visibleFrame();
        let area = screen_overlap(frame, visible);
        if area > overlap {
            overlap = area;
            best = Some(visible);
        }
    }
    best.or_else(|| NSScreen::mainScreen(mtm).map(|screen| screen.visibleFrame()))
        .or_else(|| screens.firstObject().map(|screen| screen.visibleFrame()))
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
        && match drag.target {
            DragTarget::Character => scene.visible,
            DragTarget::StandaloneBubble => !scene.visible && scene.bubble_visible,
        }
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

struct ReplyChildGeometry {
    status: NSRect,
    input: NSRect,
    send: NSRect,
    readonly: bool,
    interactive: bool,
}

fn reply_child_geometry(
    size: NSSize,
    readonly: bool,
    metrics: ComposerMetrics,
) -> ReplyChildGeometry {
    let width = size.width.max(0.0);
    let height = size.height.max(0.0);
    let inset = 2.0_f64.min(width / 2.0);
    let content_width = (width - 2.0 * inset).max(0.0);
    let status_y = 2.0_f64.min(height);
    let status = NSRect::new(
        NSPoint::new(inset, status_y),
        NSSize::new(
            content_width,
            COMPOSER_STATUS_HEIGHT.min((height - status_y).max(0.0)),
        ),
    );
    let input_y = INPUT_ORIGIN_Y.min(height);
    let input_height = if readonly {
        0.0
    } else {
        metrics.input_height.min((height - input_y).max(0.0))
    };
    let send_width = if readonly {
        0.0
    } else {
        58.0_f64.min(content_width * 0.35)
    };
    let gap = if readonly {
        0.0
    } else {
        4.0_f64.min((content_width - send_width).max(0.0))
    };
    let input_width = if readonly {
        0.0
    } else {
        (content_width - send_width - gap).max(0.0)
    };
    let input = NSRect::new(
        NSPoint::new(inset, input_y),
        NSSize::new(input_width, input_height),
    );
    let send = NSRect::new(
        NSPoint::new(inset + input_width + gap, input_y),
        NSSize::new(send_width, input_height),
    );
    // A positive outer rectangle alone cannot expose the caret: the row needs
    // the measured clip height and at least a line-sized horizontal viewport.
    ReplyChildGeometry {
        status,
        input,
        send,
        readonly,
        interactive: !readonly
            && input_width >= metrics.content_height
            && send_width > 0.0
            && input_height >= metrics.input_height,
    }
}

fn feedback_compact_frames(
    x: f64,
    width: f64,
    bottom: f64,
    top: f64,
    primary_height: f64,
    warning_height: f64,
) -> (NSRect, NSRect) {
    // The result occupies the first visible lines. Dialogue uses only the
    // remaining height, irrespective of its (possibly unbounded) text.
    let warning_visible = warning_height.min((top - bottom).max(0.0));
    let warning = NSRect::new(
        NSPoint::new(x, top - warning_visible),
        NSSize::new(width, warning_visible),
    );
    let primary_top = (warning.origin.y - BUBBLE_CONTENT_GAP).max(bottom);
    let primary_visible = primary_height.min((primary_top - bottom).max(0.0));
    let primary = NSRect::new(
        NSPoint::new(x, primary_top - primary_visible),
        NSSize::new(width, primary_visible),
    );
    (primary, warning)
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
    use crate::automation::{test_automation, OperationState, PresentationRequest};
    use crate::composer_layout::BOTTOM_INSET;

    fn submit_presentation(
        ledger: &mut AutomationState,
        id: &str,
        expected_revision: Option<u64>,
        action: PresentationAction,
    ) -> Result<(), String> {
        ledger
            .submit(PresentationRequest {
                instance_id: "test".to_owned(),
                operation_id: id.to_owned(),
                expected_revision,
                action,
            })
            .map(|_| ())
    }

    fn observed_presentation(
        revision: u64,
        target: PresentationTarget,
        persisted: Option<PresentationTarget>,
        save_error: Option<(String, PresentationTarget, String)>,
    ) -> PresentationCheckpoint {
        PresentationCheckpoint {
            revision,
            effective: Some(target),
            native_applied: true,
            persisted,
            save_error,
            pet_window_visible: Some(target.visible),
            bubble_window_visible: Some(target.bubble_visible),
            pet_window_frame: None,
            bubble_window_frame: None,
            pending_reasons: Vec::new(),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.0e-9
    }

    fn test_anchor_key() -> AnchorKey {
        AnchorKey {
            frame: None,
            artwork: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 350.0)),
            backend_epoch: 11,
            viewport: PresentationViewport {
                width: 400.0,
                height: 350.0,
                backing_scale: 2.0,
                epoch: 7,
            },
            scale: 1.0,
        }
    }

    fn test_head() -> BubbleRect {
        BubbleRect {
            x: 100.0,
            y: 60.0,
            width: 180.0,
            height: 230.0,
        }
    }

    fn test_panel() -> BubbleRect {
        BubbleRect {
            x: 300.0,
            y: 180.0,
            width: 400.0,
            height: 350.0,
        }
    }

    fn test_visible() -> BubbleRect {
        BubbleRect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        }
    }

    #[test]
    fn rig_transient_keeps_compatible_visual_and_retries_same_generation() {
        let key = test_anchor_key();
        let (initial, changed) = accept_visual_anchor(
            None,
            key,
            SpeechAnchorStatus::ReadyAnchor(test_head()),
            true,
        );
        assert!(changed);
        let body = (320.0, 180.0);
        let initial_geometry = placed_attached_bubble(
            visual_relative_for(initial, key),
            test_panel(),
            body,
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(initial_geometry.side, Some(crate::bubble::BubbleSide::Left));
        let (retained, changed) = accept_visual_anchor(
            initial,
            key,
            SpeechAnchorStatus::TemporarilyUnavailable,
            true,
        );
        assert!(!changed);
        assert_eq!(
            placed_attached_bubble(
                visual_relative_for(retained, key),
                test_panel(),
                body,
                test_visible(),
                BubblePlacement::Left,
            ),
            initial_geometry,
        );
        assert!(anchor_query_needed(Some(&CachedAnchor { key }), key, true));
        let new_head = BubbleRect {
            x: 120.0,
            y: 78.0,
            width: 165.0,
            height: 205.0,
        };
        let (recovered, changed) = accept_visual_anchor(
            retained,
            key,
            SpeechAnchorStatus::ReadyAnchor(new_head),
            true,
        );
        assert!(changed);
        let recovered_geometry = placed_attached_bubble(
            visual_relative_for(recovered, key),
            test_panel(),
            (280.0, 140.0),
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(
            recovered_geometry.side,
            Some(crate::bubble::BubbleSide::Left)
        );
        assert_ne!(recovered_geometry, initial_geometry);
        assert_eq!(
            recovered_geometry,
            placed_attached_bubble(
                Some(new_head),
                test_panel(),
                (280.0, 140.0),
                test_visible(),
                BubblePlacement::Left,
            ),
        );
        let (anchorless, changed) =
            accept_visual_anchor(recovered, key, SpeechAnchorStatus::ReadyAnchorless, true);
        assert!(changed && anchorless.is_none());
        assert_eq!(
            placed_attached_bubble(
                None,
                test_panel(),
                body,
                test_visible(),
                BubblePlacement::Left
            )
            .side,
            Some(crate::bubble::BubbleSide::Above),
        );
    }

    #[test]
    fn equal_bounds_png_recovery_invalidates_fallback_after_backing_change() {
        let key = AnchorKey {
            frame: Some(FrameId::new(0)),
            ..test_anchor_key()
        };
        let (old, _) = accept_visual_anchor(
            None,
            key,
            SpeechAnchorStatus::ReadyAnchor(test_head()),
            false,
        );
        let body = (320.0, 180.0);
        let attached = placed_attached_bubble(
            visual_relative_for(old, key),
            test_panel(),
            body,
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(attached.side, Some(crate::bubble::BubbleSide::Left));
        let current = AnchorKey {
            viewport: PresentationViewport {
                backing_scale: 1.0,
                ..key.viewport
            },
            ..key
        };
        assert_eq!(visual_relative_for(old, current), None);
        let fallback = placed_attached_bubble(
            visual_relative_for(old, current),
            test_panel(),
            body,
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(fallback.side, Some(crate::bubble::BubbleSide::Above));
        let (restored, invalidated) = accept_visual_anchor(
            old,
            current,
            SpeechAnchorStatus::ReadyAnchor(test_head()),
            false,
        );
        assert!(invalidated);
        let final_geometry = placed_attached_bubble(
            visual_relative_for(restored, current),
            test_panel(),
            body,
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(final_geometry, attached);
        assert_eq!(final_geometry.side, Some(crate::bubble::BubbleSide::Left));
        assert_ne!(final_geometry.window, fallback.window);
        assert_ne!(final_geometry.tail, fallback.tail);
        // A later PNG frame with no usable head must clear the restored
        // attachment instead of replaying the cached geometry.
        let next_frame = AnchorKey {
            frame: Some(FrameId::new(1)),
            ..current
        };
        assert!(anchor_query_needed(
            Some(&CachedAnchor { key: current }),
            next_frame,
            false,
        ));
        let (missing, invalidated) = accept_visual_anchor(
            restored,
            next_frame,
            SpeechAnchorStatus::ReadyAnchorless,
            false,
        );
        assert!(invalidated);
        assert_eq!(
            placed_attached_bubble(
                visual_relative_for(missing, next_frame),
                test_panel(),
                body,
                test_visible(),
                BubblePlacement::Left,
            ),
            fallback,
        );
    }

    #[test]
    fn incompatible_visual_loss_invalidates_even_when_current_bounds_are_both_absent() {
        let key = test_anchor_key();
        let (valid, _) = accept_visual_anchor(
            None,
            key,
            SpeechAnchorStatus::ReadyAnchor(test_head()),
            true,
        );
        let changed_keys = [
            AnchorKey {
                backend_epoch: 12,
                ..key
            },
            AnchorKey {
                artwork: NSRect::new(NSPoint::new(5.0, 0.0), key.artwork.size),
                ..key
            },
            AnchorKey {
                viewport: PresentationViewport {
                    epoch: 9,
                    ..key.viewport
                },
                ..key
            },
            AnchorKey {
                viewport: PresentationViewport {
                    backing_scale: 1.0,
                    epoch: 8,
                    ..key.viewport
                },
                ..key
            },
            AnchorKey { scale: 1.25, ..key },
        ];
        let applied = placed_attached_bubble(
            visual_relative_for(valid, key),
            test_panel(),
            (320.0, 180.0),
            test_visible(),
            BubblePlacement::Left,
        );
        for current in changed_keys {
            assert_eq!(visual_relative_for(valid, current), None);
            let (selected, invalidated) = accept_visual_anchor(
                valid,
                current,
                SpeechAnchorStatus::TemporarilyUnavailable,
                true,
            );
            assert!(invalidated && selected.is_none());
            let fallback = placed_attached_bubble(
                visual_relative_for(selected, current),
                test_panel(),
                (320.0, 180.0),
                test_visible(),
                BubblePlacement::Left,
            );
            assert_eq!(fallback.side, Some(crate::bubble::BubbleSide::Above));
            assert_ne!(fallback, applied);
        }
    }

    #[test]
    fn terminal_absence_invalid_geometry_and_errors_clear_but_move_translates_visual() {
        let key = test_anchor_key();
        let (valid, _) = accept_visual_anchor(
            None,
            key,
            SpeechAnchorStatus::ReadyAnchor(test_head()),
            true,
        );
        for status in [
            SpeechAnchorStatus::Invalid,
            SpeechAnchorStatus::ReadyAnchorless,
            SpeechAnchorStatus::ReadyAnchor(BubbleRect {
                width: f64::NAN,
                ..test_head()
            }),
            SpeechAnchorStatus::ReadyAnchor(BubbleRect {
                height: 0.0,
                ..test_head()
            }),
        ] {
            let (removed, invalidated) = accept_visual_anchor(valid, key, status, true);
            assert!(removed.is_none() && invalidated);
        }
        let (png_missing, invalidated) = accept_visual_anchor(
            valid,
            key,
            SpeechAnchorStatus::TemporarilyUnavailable,
            false,
        );
        assert!(png_missing.is_none() && invalidated);
        let (no_history, invalidated) =
            accept_visual_anchor(None, key, SpeechAnchorStatus::TemporarilyUnavailable, true);
        assert!(no_history.is_none() && !invalidated);
        let moved = BubbleRect {
            x: test_panel().x + 75.0,
            y: test_panel().y + 40.0,
            ..test_panel()
        };
        let before = placed_attached_bubble(
            visual_relative_for(valid, key),
            test_panel(),
            (320.0, 180.0),
            test_visible(),
            BubblePlacement::Left,
        );
        let (retained, invalidated) =
            accept_visual_anchor(valid, key, SpeechAnchorStatus::TemporarilyUnavailable, true);
        assert!(!invalidated);
        let after = placed_attached_bubble(
            visual_relative_for(retained, key),
            moved,
            (320.0, 180.0),
            test_visible(),
            BubblePlacement::Left,
        );
        assert_eq!(before.side, after.side);
        assert!(close(after.window.x - before.window.x, 75.0));
        assert!(close(after.window.y - before.window.y, 40.0));
        let changed_body = placed_attached_bubble(
            visual_relative_for(retained, key),
            moved,
            (280.0, 140.0),
            test_visible(),
            BubblePlacement::Left,
        );
        assert_ne!(changed_body.body, after.body);
    }

    #[test]
    fn snapshot_provenance_and_error_select_authoritative_fallback() {
        let key = test_anchor_key();
        let origin = NSPoint::new(test_panel().x, test_panel().y);
        let snapshot = SpeechAnchorSnapshot {
            status: SpeechAnchorStatus::ReadyAnchor(BubbleRect {
                x: origin.x + test_head().x,
                y: origin.y + test_head().y,
                ..test_head()
            }),
            backend_epoch: key.backend_epoch,
            input_epoch: Some(2),
            anchor_epoch: Some(0),
            viewport: Some(key.viewport),
        };
        let status = presentation_anchor_status(&Ok(snapshot), key, origin, true);
        assert_eq!(status, SpeechAnchorStatus::ReadyAnchor(test_head()));
        let (valid, _) = accept_visual_anchor(None, key, status, true);
        let transient = SpeechAnchorSnapshot {
            status: SpeechAnchorStatus::TemporarilyUnavailable,
            input_epoch: Some(3),
            anchor_epoch: Some(4),
            ..snapshot
        };
        let status = presentation_anchor_status(&Ok(transient), key, origin, true);
        let (retained, invalidated) = accept_visual_anchor(valid, key, status, true);
        assert!(!invalidated);
        assert_eq!(visual_relative_for(retained, key), Some(test_head()));
        for result in [
            Err("native snapshot failure".to_owned()),
            Ok(SpeechAnchorSnapshot {
                backend_epoch: key.backend_epoch + 1,
                ..transient
            }),
            Ok(SpeechAnchorSnapshot {
                viewport: None,
                ..transient
            }),
            Ok(SpeechAnchorSnapshot {
                input_epoch: None,
                ..transient
            }),
        ] {
            let status = presentation_anchor_status(&result, key, origin, true);
            assert_eq!(status, SpeechAnchorStatus::Invalid);
            let (removed, invalidated) = accept_visual_anchor(valid, key, status, true);
            assert!(removed.is_none() && invalidated);
            let fallback = placed_attached_bubble(
                visual_relative_for(removed, key),
                test_panel(),
                (320.0, 180.0),
                test_visible(),
                BubblePlacement::Left,
            );
            assert_eq!(fallback.side, Some(crate::bubble::BubbleSide::Above));
        }
    }

    #[test]
    fn changed_presentation_touches_only_changed_fields() {
        let mut original = AppState::new().scene();
        let mut changed = original.clone();
        changed.visible = false;
        changed.bubble_placement = BubblePlacement::Left;
        let patch = changed_presentation(&original, &changed);
        assert_eq!(patch.visible, Some(false));
        assert_eq!(patch.bubble_placement, Some(BubblePlacement::Left));
        assert_eq!(patch.passthrough, None);
        assert_eq!(patch.alpha_passthrough, None);
        assert_eq!(patch.bubble_visible, None);
        assert_eq!(patch.scale, None);
        original.scale = 0.8;
        assert_eq!(
            changed_presentation(&original, &original),
            PresentationPatch::default()
        );
    }

    #[test]
    fn accepted_hide_retry_and_same_target_noop_both_apply_after_save() {
        let mut state = AppState::new();
        let initial = state.scene();
        let automation = test_automation(PresentationTarget::from_scene(&initial));
        let mut ledger = lock_automation(&automation);
        let mut requested = None;
        let mut reset = false;
        let mut operation = None;
        let hide = PresentationAction::Set {
            patch: PresentationPatch {
                visible: Some(false),
                ..Default::default()
            },
        };
        submit_presentation(&mut ledger, "failed-hide", None, hide.clone()).unwrap();
        let hidden = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        let hidden_target = PresentationTarget::from_scene(&hidden);
        let revision = ledger.revision();
        ledger.publish_checkpoint(observed_presentation(
            revision,
            hidden_target,
            None,
            Some((
                "failed-hide".to_owned(),
                hidden_target,
                "disk busy".to_owned(),
            )),
        ));
        assert_eq!(
            ledger.status("test", "failed-hide").unwrap().state,
            OperationState::PersistFailed
        );
        requested = None;
        operation = None;
        submit_presentation(&mut ledger, "retry-hide", None, hide).unwrap();
        submit_presentation(
            &mut ledger,
            "same-target-noop",
            None,
            PresentationAction::Set {
                patch: PresentationPatch::default(),
            },
        )
        .unwrap();
        let retried = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        assert_eq!(
            changed_presentation(&hidden, &retried),
            PresentationPatch::default()
        );
        let persisted = presentation_persistence_patch(
            &hidden,
            &retried,
            PresentationTarget::from_scene(&initial),
            true,
            requested.as_ref(),
        );
        assert_eq!(persisted.visible, Some(false));
        assert_eq!(persisted.bubble_placement, None);
        assert_eq!(operation.as_deref(), Some("same-target-noop"));
        assert!(!reset);
        let revision = ledger.revision();
        ledger.publish_checkpoint(observed_presentation(
            revision,
            PresentationTarget::from_scene(&retried),
            Some(PresentationTarget::from_scene(&retried)),
            None,
        ));
        for id in ["retry-hide", "same-target-noop"] {
            let status = ledger.status("test", id).unwrap();
            assert_eq!(status.state, OperationState::Applied, "{id}");
            assert!(status.native_applied && status.persisted, "{id}");
        }
    }

    #[test]
    fn distinct_final_target_supersedes_prior_operation_but_saves_both_explicit_fields() {
        let mut state = AppState::new();
        let original = state.scene();
        let automation = test_automation(PresentationTarget::from_scene(&original));
        let mut ledger = lock_automation(&automation);
        let mut requested = None;
        let mut reset = false;
        let mut operation = None;
        submit_presentation(
            &mut ledger,
            "hide",
            None,
            PresentationAction::Set {
                patch: PresentationPatch {
                    visible: Some(false),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        submit_presentation(
            &mut ledger,
            "move-bubble",
            None,
            PresentationAction::Set {
                patch: PresentationPatch {
                    bubble_placement: Some(BubblePlacement::Left),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        let scene = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        assert_eq!(
            ledger.status("test", "hide").unwrap().state,
            OperationState::Superseded
        );
        let patch = presentation_persistence_patch(
            &original,
            &scene,
            PresentationTarget::from_scene(&original),
            true,
            requested.as_ref(),
        );
        assert_eq!(patch.visible, Some(false));
        assert_eq!(patch.bubble_placement, Some(BubblePlacement::Left));
        let revision = ledger.revision();
        ledger.publish_checkpoint(observed_presentation(
            revision,
            PresentationTarget::from_scene(&scene),
            Some(PresentationTarget::from_scene(&scene)),
            None,
        ));
        assert_eq!(
            ledger.status("test", "move-bubble").unwrap().state,
            OperationState::Applied
        );
        assert_eq!(
            ledger.status("test", "hide").unwrap().state,
            OperationState::Superseded
        );
    }

    #[test]
    fn bubble_only_after_unsaved_hide_does_not_save_ghost_hide_and_rejected_cas_is_excluded() {
        let mut state = AppState::new();
        let original = state.scene();
        let automation = test_automation(PresentationTarget::from_scene(&original));
        let mut ledger = lock_automation(&automation);
        let mut requested = None;
        let mut reset = false;
        let mut operation = None;
        submit_presentation(
            &mut ledger,
            "unsaved-hide",
            None,
            PresentationAction::Set {
                patch: PresentationPatch {
                    visible: Some(false),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        let hidden = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        let hidden_target = PresentationTarget::from_scene(&hidden);
        let revision = ledger.revision();
        ledger.publish_checkpoint(observed_presentation(
            revision,
            hidden_target,
            None,
            Some((
                "unsaved-hide".to_owned(),
                hidden_target,
                "disk busy".to_owned(),
            )),
        ));
        requested = None;
        operation = None;
        let revision = ledger.revision();
        submit_presentation(
            &mut ledger,
            "bubble-only",
            Some(revision),
            PresentationAction::Set {
                patch: PresentationPatch {
                    bubble_placement: Some(BubblePlacement::Left),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        submit_presentation(
            &mut ledger,
            "stale-cas",
            Some(revision),
            PresentationAction::Set {
                patch: PresentationPatch {
                    visible: Some(true),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        let bubble = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        let patch = presentation_persistence_patch(
            &hidden,
            &bubble,
            PresentationTarget::from_scene(&original),
            true,
            requested.as_ref(),
        );
        assert_eq!(patch.visible, None);
        assert_eq!(patch.bubble_placement, Some(BubblePlacement::Left));
        assert_eq!(
            ledger.status("test", "stale-cas").unwrap().state,
            OperationState::Rejected
        );

        requested = None;
        submit_presentation(
            &mut ledger,
            "reset",
            None,
            PresentationAction::ResetPosition,
        )
        .unwrap();
        submit_presentation(
            &mut ledger,
            "normalized-scale",
            None,
            PresentationAction::Set {
                patch: PresentationPatch {
                    scale: Some(MAX_SCALE * 2.0),
                    ..Default::default()
                },
            },
        )
        .unwrap();
        let final_scene = reduce_presentation_batch(
            &mut state,
            &mut ledger,
            &mut requested,
            &mut reset,
            &mut operation,
        );
        let patch = presentation_persistence_patch(
            &bubble,
            &final_scene,
            PresentationTarget::from_scene(&original),
            true,
            requested.as_ref(),
        );
        assert_eq!(patch.visible, None);
        assert_eq!(patch.scale, Some(MAX_SCALE));
        assert_eq!(
            final_scene.reset_position_revision,
            bubble.reset_position_revision + 1
        );
        assert!(reset);
    }

    #[test]
    fn applied_attachment_provenance_distinguishes_clipped_from_standalone_tailless_body() {
        let visible = BubbleRect {
            x: -500.0,
            y: 100.0,
            width: 240.0,
            height: 180.0,
        };
        let attached = place_bubble(visible, (150.0, 90.0), visible, BubblePlacement::Above);
        assert!(attached.side.is_none() && attached.tail.is_none());
        let body_origin = bubble_body_origin(attached);
        assert_eq!(
            attached_body_origin(Some(attached), true),
            Some(body_origin)
        );
        let standalone = place_standalone_bubble(body_origin, (150.0, 90.0), visible);
        assert!(standalone.side.is_none() && standalone.tail.is_none());
        assert_eq!(attached_body_origin(Some(standalone), false), None);
        assert_eq!(attached_body_origin(None, true), None);
        let saved = (-310.0, 156.0);
        let pending = (-400.0, 180.0);
        assert_eq!(
            preferred_standalone_origin(Some(saved), Some(pending), Some(attached), true),
            Some(saved)
        );
        assert_eq!(
            preferred_standalone_origin(None, Some(pending), Some(attached), true),
            Some(pending)
        );
        assert_eq!(
            preferred_standalone_origin(None, None, Some(attached), true),
            Some(body_origin)
        );
        assert_eq!(
            preferred_standalone_origin(None, None, Some(standalone), false),
            None
        );
    }

    #[test]
    fn worktree_context_targets_the_clicked_header_or_captured_background_selection() {
        let a = SessionKey {
            source_id: 1,
            generation: 1,
            terminal_id: "A".to_owned(),
        };
        let b = SessionKey {
            source_id: 1,
            generation: 1,
            terminal_id: "B".to_owned(),
        };
        assert_eq!(
            worktree_menu_key(Some(b.clone()), false, Some(a.clone())),
            Some(b.clone())
        );
        assert_eq!(
            worktree_menu_key(None, true, Some(a.clone())),
            Some(a.clone())
        );
        assert_eq!(
            worktree_menu_key(Some(b.clone()), true, Some(a.clone())),
            Some(a.clone())
        );
        assert_eq!(worktree_menu_key(None, true, None), None);
        assert_eq!(worktree_menu_key(None, false, Some(a)), None);
    }

    #[test]
    fn worktree_result_stays_concise_while_full_detail_retains_target_and_server_response() {
        let repo = "repository-name-".repeat(12);
        let basename = "checkout-name-".repeat(12);
        let checkout = format!("/parent/{basename}");
        let target = WorktreeRemoveTarget {
            key: SessionKey {
                source_id: 1,
                generation: 1,
                terminal_id: "session".into(),
            },
            source: "/local.sock".into(),
            pane_id: "pane".into(),
            workspace_id: "workspace".into(),
            worktree: Arc::new(crate::herdr_protocol::WorkspaceWorktreeInfo {
                repo_key: "key".into(),
                repo_name: repo.clone(),
                repo_root: "/parent".into(),
                checkout_path: checkout.clone(),
                is_linked_worktree: true,
                pane_count: 1,
                tab_count: 1,
            }),
        };
        let feedback = [
            WorktreeFeedback::Pending(target.clone()),
            WorktreeFeedback::Finished(target.clone(), Ok(())),
            WorktreeFeedback::Finished(
                target.clone(),
                Err(WorktreeRemoveError::Rejected {
                    code: "dirty_worktree".into(),
                    message: "uncommitted files".into(),
                }),
            ),
            WorktreeFeedback::Finished(
                target.clone(),
                Err(WorktreeRemoveError::Other("server not ready".into())),
            ),
            WorktreeFeedback::Finished(target.clone(), Err(WorktreeRemoveError::UnknownDelivery)),
            WorktreeFeedback::Finished(target.clone(), Err(WorktreeRemoveError::Offline)),
        ];
        for locale in [UiLocale::En, UiLocale::Ko] {
            for state in &feedback {
                let compact = state.compact_summary(locale);
                let full = state.full_text(locale);
                assert!(!compact.contains(&repo));
                assert!(!compact.contains(&basename));
                assert!(full.contains(&repo));
                assert!(full.contains(&checkout));
            }
            let rejection = feedback[2].full_text(locale);
            assert!(rejection.contains("dirty_worktree"));
            assert!(rejection.contains("uncommitted files"));
            assert!(feedback[3].full_text(locale).contains("server not ready"));
            let uncertain = feedback[4].compact_summary(locale);
            assert!(feedback[4].full_text(locale).contains(&checkout));
            let warning = format!("{}: disconnected", locale.tag());
            let compact = compact_worktree_secondary(uncertain, &warning);
            let detail = worktree_feedback_detail(Some(&feedback[2]), locale, &warning);
            assert!(compact.starts_with(uncertain));
            assert!(compact.ends_with(&warning));
            assert!(!compact.contains(&repo));
            assert!(detail.contains(&checkout));
            assert!(detail.contains("dirty_worktree"));
            assert!(detail.ends_with(&warning));
        }
    }

    #[test]
    fn compact_result_owns_visible_space_before_dialogue_with_or_without_status() {
        let content_width = BUBBLE_BODY_MIN_WIDTH - 2.0 * BUBBLE_HORIZONTAL_INSET;
        let summary_height = 3.0 * BUBBLE_LINE_HEIGHT;
        for show_status in [false, true] {
            let body_height = 148.0;
            let top = body_height - BUBBLE_VERTICAL_INSET;
            let footer_top = BUBBLE_VERTICAL_INSET + BUBBLE_CONTROL_HEIGHT + BUBBLE_CONTENT_GAP;
            let bottom = footer_top
                + if show_status {
                    STATUS_ROW_HEIGHT + BUBBLE_CONTENT_GAP
                } else {
                    0.0
                };
            let (primary, result) = feedback_compact_frames(
                BUBBLE_HORIZONTAL_INSET,
                content_width,
                bottom,
                top,
                4.0 * BUBBLE_LINE_HEIGHT,
                summary_height,
            );
            assert_eq!(result.size.width, 96.0);
            assert!(result.size.height >= summary_height);
            assert!(result.origin.y >= primary.origin.y + primary.size.height);
            assert!(primary.origin.y >= bottom);
            assert!(result.origin.y + result.size.height <= top);

            let (primary, result) = feedback_compact_frames(
                BUBBLE_HORIZONTAL_INSET,
                content_width,
                bottom,
                top,
                4.0 * BUBBLE_LINE_HEIGHT,
                2000.0,
            );
            assert!(result.size.height >= summary_height);
            assert!(primary.size.height <= (result.origin.y - bottom).max(0.0));
            assert!(result.origin.y >= bottom);
        }
    }

    #[test]
    fn status_item_input_routes_only_valid_context_clicks_to_native_menu() {
        let valid = |kind, modifiers| StatusItemEvent {
            kind,
            modifiers,
            age: 0.1,
            same_window: true,
            hits_button: true,
        };
        let none = NSEventModifierFlags::empty();
        assert_eq!(status_item_action(None), StatusItemAction::Primary);
        assert_eq!(
            status_item_action(Some(valid(NSEventType::LeftMouseUp, none))),
            StatusItemAction::Primary,
        );
        assert_eq!(
            status_item_action(Some(valid(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::Option,
            ))),
            StatusItemAction::Primary,
        );
        assert_eq!(
            status_item_action(Some(valid(NSEventType::RightMouseUp, none))),
            StatusItemAction::Context,
        );
        assert_eq!(
            status_item_action(Some(valid(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::Control,
            ))),
            StatusItemAction::Context,
        );
        assert_eq!(
            status_item_action(Some(valid(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::Control | NSEventModifierFlags::Shift,
            ))),
            StatusItemAction::Context,
        );
        for kind in [
            NSEventType::LeftMouseDown,
            NSEventType::RightMouseDown,
            NSEventType::KeyDown,
        ] {
            assert_eq!(
                status_item_action(Some(valid(kind, NSEventModifierFlags::Control))),
                StatusItemAction::Primary,
            );
        }
    }

    #[test]
    fn status_item_ignores_stale_foreign_and_missed_mouse_events() {
        let valid = || StatusItemEvent {
            kind: NSEventType::RightMouseUp,
            modifiers: NSEventModifierFlags::empty(),
            age: 1.999,
            same_window: true,
            hits_button: true,
        };
        assert_eq!(status_item_action(Some(valid())), StatusItemAction::Context);
        for age in [2.0, -2.0, f64::INFINITY, f64::NAN] {
            assert_eq!(
                status_item_action(Some(StatusItemEvent { age, ..valid() })),
                StatusItemAction::Primary,
            );
        }
        assert_eq!(
            status_item_action(Some(StatusItemEvent {
                same_window: false,
                ..valid()
            })),
            StatusItemAction::Primary,
        );
        assert_eq!(
            status_item_action(Some(StatusItemEvent {
                hits_button: false,
                ..valid()
            })),
            StatusItemAction::Primary,
        );
    }

    #[test]
    fn menu_bar_visibility_covers_interactive_and_recovery_surfaces_in_both_modes() {
        for (character, bubble, passthrough, recovery_only) in [
            (true, true, false, false),
            (true, false, false, false),
            (false, true, false, false),
            (false, false, false, true),
            (true, true, true, true),
            (true, false, true, true),
            (false, true, true, true),
            (false, false, true, true),
        ] {
            let mut state = AppState::new();
            if !character {
                state.apply_control("hide").unwrap();
            }
            if !bubble {
                state.apply_control("hide_bubble").unwrap();
            }
            if passthrough {
                state.apply_control("toggle_passthrough").unwrap();
            }
            for alpha in [false, true] {
                if alpha {
                    state.apply_control("alpha_passthrough").unwrap();
                }
                let scene = state.scene();
                assert_eq!(
                    (
                        scene.visible,
                        scene.bubble_visible,
                        scene.passthrough,
                        scene.alpha_passthrough
                    ),
                    (character, bubble, passthrough, alpha),
                );
                assert!(menu_bar_visible(&scene, MenuBarMode::Always));
                assert_eq!(
                    menu_bar_visible(&scene, MenuBarMode::RecoveryOnly),
                    recovery_only,
                    "character={character}, bubble={bubble}, passthrough={passthrough}, alpha={alpha}",
                );
            }
            state.request_shutdown();
            let scene = state.scene();
            assert!(!menu_bar_visible(&scene, MenuBarMode::Always));
            assert!(!menu_bar_visible(&scene, MenuBarMode::RecoveryOnly));
        }
    }

    #[test]
    fn menu_bar_mode_changes_do_not_mutate_scene_and_recovery_restores_conditional_visibility() {
        let mut state = AppState::new();
        assert!(state.set_scale(0.8));
        state.apply_control("bubble_left").unwrap();
        let original = state.scene();
        assert!(menu_bar_visible(&original, MenuBarMode::Always));
        assert!(!menu_bar_visible(&original, MenuBarMode::RecoveryOnly));
        let after_mode_change = state.scene();
        assert_eq!(
            (
                after_mode_change.visible,
                after_mode_change.bubble_visible,
                after_mode_change.passthrough,
                after_mode_change.alpha_passthrough,
                after_mode_change.bubble_placement,
                after_mode_change.scale,
                after_mode_change.reset_position_revision,
            ),
            (
                original.visible,
                original.bubble_visible,
                original.passthrough,
                original.alpha_passthrough,
                original.bubble_placement,
                original.scale,
                original.reset_position_revision,
            )
        );

        state.apply_control("hide").unwrap();
        assert!(!menu_bar_visible(&state.scene(), MenuBarMode::RecoveryOnly));
        state.apply_control("hide_bubble").unwrap();
        assert!(menu_bar_visible(&state.scene(), MenuBarMode::RecoveryOnly));
        state.recover_interaction();
        let recovered = state.scene();
        assert!(recovered.visible);
        assert!(!recovered.bubble_visible);
        assert!(!recovered.passthrough);
        assert_eq!(recovered.bubble_placement, original.bubble_placement);
        assert_eq!(recovered.scale, original.scale);
        assert!(!menu_bar_visible(&recovered, MenuBarMode::RecoveryOnly));
        assert!(menu_bar_visible(&recovered, MenuBarMode::Always));

        state.apply_control("toggle_passthrough").unwrap();
        assert!(menu_bar_visible(&state.scene(), MenuBarMode::RecoveryOnly));
        state.recover_interaction();
        assert!(!menu_bar_visible(&state.scene(), MenuBarMode::RecoveryOnly));
        state.apply_control("alpha_passthrough").unwrap();
        assert!(!menu_bar_visible(&state.scene(), MenuBarMode::RecoveryOnly));
        assert!(menu_bar_visible(&state.scene(), MenuBarMode::Always));
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
    fn standalone_screen_crossing_and_clamp_keep_negative_display_coordinates() {
        let left = NSRect::new(NSPoint::new(-1200.0, -200.0), NSSize::new(1200.0, 800.0));
        let right = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1400.0, 900.0));
        let crossing = NSRect::new(NSPoint::new(-80.0, 100.0), NSSize::new(300.0, 120.0));
        assert!(screen_overlap(crossing, right) > screen_overlap(crossing, left));

        let leftward = NSRect::new(NSPoint::new(-1250.0, -210.0), NSSize::new(300.0, 120.0));
        let clamped = clamp_window_origin(leftward, left);
        assert_eq!(clamped, NSPoint::new(-1200.0, -200.0));
        let offscreen = NSRect::new(NSPoint::new(-6000.0, 100.0), crossing.size);
        assert_eq!(screen_overlap(offscreen, left), 0.0);
        assert_eq!(screen_overlap(offscreen, right), 0.0);
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
            target: DragTarget::Character,
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
    fn reply_children_stay_in_bounds_across_width_and_height_changes() {
        for (content_height, input_height) in [(20.0, 30.0), (27.0, 50.0)] {
            let metrics = ComposerMetrics {
                content_height,
                input_height,
                reply_height: INPUT_ORIGIN_Y + input_height + BOTTOM_INSET,
            };
            for width in [0.0, 1.0, 4.0, 8.0, 12.0, 32.0, 120.0, 270.0, 600.0] {
                for height in [
                    0.0,
                    1.0,
                    20.0,
                    COMPOSER_READONLY_HEIGHT,
                    metrics.reply_height,
                ] {
                    for readonly in [false, true] {
                        let size = NSSize::new(width, height);
                        let layout = reply_child_geometry(size, readonly, metrics);
                        for rect in [layout.status, layout.input, layout.send] {
                            assert!(rect.origin.x >= 0.0 && rect.origin.y >= 0.0);
                            assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
                            assert!(rect.origin.x + rect.size.width <= width);
                            assert!(rect.origin.y + rect.size.height <= height);
                        }
                        assert!(
                            layout.input.origin.x + layout.input.size.width <= layout.send.origin.x
                        );
                        assert!(
                            layout.status.origin.y + layout.status.size.height
                                <= layout.input.origin.y
                                || layout.input.size.height == 0.0
                        );
                        if layout.interactive {
                            assert!(!readonly && layout.input.size.width >= metrics.content_height);
                            assert!(layout.input.size.height >= metrics.input_height);
                            assert!(layout.send.size.width > 0.0 && layout.send.size.height > 0.0);
                        }
                        if readonly
                            || width < metrics.content_height
                            || height < metrics.reply_height
                        {
                            assert!(!layout.interactive);
                        }
                        if readonly {
                            assert_eq!(layout.input.size, NSSize::new(0.0, 0.0));
                            assert_eq!(layout.send.size, NSSize::new(0.0, 0.0));
                        }
                    }
                }
            }
            assert!(
                reply_child_geometry(NSSize::new(270.0, metrics.reply_height), false, metrics)
                    .interactive
            );
        }
    }

    #[test]
    fn committed_reply_breaks_are_utf16_local_edits() {
        let input = NSString::from_str("first\r\nsecond\nthird\rfourth\u{2028}fifth\u{2029}끝");
        let breaks = line_break_ranges(&input);
        assert_eq!(
            breaks,
            [
                NSRange::new(5, 2),
                NSRange::new(13, 1),
                NSRange::new(19, 1),
                NSRange::new(26, 1),
                NSRange::new(32, 1)
            ]
        );
        let mut units: Vec<u16> = input.to_string().encode_utf16().collect();
        for range in breaks.iter().rev() {
            units.splice(range.location..range.location + range.length, [b' ' as u16]);
        }
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "first second third fourth fifth 끝"
        );
        assert_eq!(
            line_break_ranges(&NSString::from_str("\r\nA\r\n\r\n")),
            [NSRange::new(0, 2), NSRange::new(3, 2), NSRange::new(5, 2)]
        );
        assert!(line_break_ranges(&NSString::from_str("hello 🌎")).is_empty());
    }

    #[test]
    fn committed_reply_preserves_utf16_selection_endpoints() {
        let breaks = line_break_ranges(&NSString::from_str("🌎\r\nA\u{2028}🦊\r\nZ"));
        assert_eq!(
            breaks,
            [NSRange::new(2, 2), NSRange::new(5, 1), NSRange::new(8, 2)]
        );
        assert_eq!(
            adjusted_composer_selection(NSRange::new(4, 6), &breaks),
            NSRange::new(3, 5)
        );
        assert_eq!(
            adjusted_composer_selection(NSRange::new(7, 0), &breaks),
            NSRange::new(6, 0)
        );
        assert_eq!(
            adjusted_composer_selection(NSRange::new(0, 2), &breaks),
            NSRange::new(0, 2)
        );
        assert_eq!(
            adjusted_composer_selection(NSRange::new(usize::MAX, 0), &breaks),
            NSRange::new(usize::MAX, 0)
        );
    }

    #[test]
    fn selected_reply_respects_screen_clamp_and_prioritizes_cards() {
        for (content_height, input_height) in [(20.0, 30.0), (24.0, 43.0), (27.0, 50.0)] {
            let metrics = ComposerMetrics {
                content_height,
                input_height,
                reply_height: INPUT_ORIGIN_Y + input_height + BOTTOM_INSET,
            };
            let reply =
                reply_child_geometry(NSSize::new(270.0, metrics.reply_height), false, metrics);
            assert!(reply.interactive);
            assert_eq!(reply.input.size.height, input_height);
            let cards = minimum_selectable_height() + metrics.reply_height;
            for visible_height in [240.0, 260.0, 300.0] {
                let cap = visible_height - BUBBLE_WINDOW_INSET * 2.0;
                for show_status in [false, true] {
                    for has_message in [false, true] {
                        let spacing = expanded_spacing(cap, cards, cards, show_status, has_message);
                        let minimum =
                            expanded_height_budget(cards, cards, show_status, has_message, spacing);
                        let body_height = minimum.min(cap);
                        let desired_message = if has_message { BUBBLE_LINE_HEIGHT } else { 0.0 };
                        let slots = expanded_heights(
                            body_height,
                            cards,
                            desired_message,
                            cards,
                            show_status,
                            spacing,
                        );

                        // Mirror the expanded frames: collapse below cards, message below
                        // status (when shown), with the same optional status gap as the budget.
                        let cards_bottom = spacing.inset + BUBBLE_CONTROL_HEIGHT + spacing.gap;
                        let content_top = body_height - spacing.inset;
                        let status_bottom =
                            content_top - if show_status { STATUS_ROW_HEIGHT } else { 0.0 };
                        let message_top = expanded_message_top(
                            status_bottom,
                            content_top,
                            show_status,
                            has_message,
                            spacing,
                        );
                        assert!(slots.cards >= 0.0 && slots.cards <= cards);
                        assert!(slots.message >= 0.0 && slots.message <= desired_message);
                        assert!(cards_bottom + slots.cards <= message_top - slots.message + 1e-9);
                        assert!(message_top <= content_top);
                        if show_status && has_message {
                            assert!(close(status_bottom - message_top, spacing.gap));
                        }

                        let overhead =
                            expanded_height_budget(0.0, 0.0, show_status, has_message, spacing)
                                - desired_message;
                        if cards + overhead <= body_height {
                            assert!(close(slots.cards, cards));
                        } else {
                            assert!(close(slots.message, 0.0));
                        }
                        if minimum <= cap {
                            assert!(close(slots.cards, cards));
                            assert!(close(slots.message, desired_message));
                        } else {
                            assert!(
                                slots.cards < cards || slots.message < desired_message,
                                "an infeasible budget cannot fit both chrome and content"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bridge_disconnection_cancels_deferred_request() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let cancel = AtomicBool::new(false);
        assert!(wait_for_main_result(receiver, &cancel, Duration::from_secs(30)).is_err());
        assert!(cancel.load(Ordering::Acquire));
    }

    #[test]
    fn bridge_deadline_cancels_a_still_connected_deferred_request() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let cancel = AtomicBool::new(false);
        assert!(wait_for_main_result(receiver, &cancel, Duration::ZERO).is_err());
        assert!(cancel.load(Ordering::Acquire));
    }
}
