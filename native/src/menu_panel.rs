use crate::assets::CharacterMetadata;
use crate::bubble::BubblePlacement;
use crate::dialogue::{DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::i18n::{default_dialogue, DefaultDialogue};
use crate::i18n::{text, LanguagePreference, Message, UiLocale};
use crate::lifecycle::LifecycleSettings;
use crate::lifecycle_settings_ui::{LifecycleSettingsCard, CARD_HEIGHT};
use crate::preferences::{BubbleAppearance, BubbleColor, BubblePalette, BubbleTheme};
use crate::sources::{MachineStatus, ObservationPreferences, SourceCatalog};
use crate::state::Scene;
use crate::ui::MenuTarget;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message as ObjcMessage,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezelStyle, NSBezierPath, NSBox,
    NSBoxType, NSButton, NSButtonType, NSCellImagePosition, NSColor, NSControl, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSEventModifierFlags,
    NSFloatingWindowLevel, NSFont, NSImage, NSImageScaling, NSPanel, NSPopUpButton, NSScreen,
    NSScrollElasticity, NSScrollView, NSScrollerStyle, NSSwitch, NSTextAlignment, NSTextDelegate,
    NSTextField, NSTextView, NSTextViewDelegate, NSUserInterfaceItemIdentification, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSUndoManager};
use std::cell::RefCell;
use std::collections::BTreeMap;

const PANEL_WIDTH: f64 = 352.0;
const PANEL_HEIGHT: f64 = 540.0;
const PANEL_EDGE_INSET: f64 = 10.0;
const PANEL_RADIUS: f64 = 16.0;
const CARD_RADIUS: f64 = 10.0;
const SCROLL_TOP: f64 = 86.0;
const FOOTER_HEIGHT: f64 = 50.0;
const CHARACTER_CONTENT_HEIGHT: f64 = 404.0;
const DIALOGUE_CARD_HEIGHT: f64 = 414.0;
const DIALOGUE_CHOOSER_HEIGHT: f64 = 246.0;
const DIALOGUE_MAX_BYTES: usize = 2048;
const BUBBLE_CONTENT_HEIGHT: f64 = 578.0;
const SETTINGS_BASE_HEIGHT: f64 = 334.0 + CARD_HEIGHT + 14.0;
const MACHINE_ROW_HEIGHT: f64 = 94.0;
const MIN_SCALE: f64 = 0.35;
const MAX_SCALE: f64 = 1.25;

const BG_RED: f64 = 0.114;
const BG_GREEN: f64 = 0.114;
const BG_BLUE: f64 = 0.125;
const CARD_RED: f64 = 0.161;
const CARD_GREEN: f64 = 0.161;
const CARD_BLUE: f64 = 0.176;
const CARD_BORDER_RED: f64 = 0.28;
const CARD_BORDER_GREEN: f64 = 0.28;
const CARD_BORDER_BLUE: f64 = 0.32;
const CARD_BORDER_ALPHA: f64 = 0.50;
const TRACK_RED: f64 = 0.082;
const TRACK_GREEN: f64 = 0.082;
const TRACK_BLUE: f64 = 0.094;
const PRIMARY_RED: f64 = 0.95;
const PRIMARY_GREEN: f64 = 0.95;
const PRIMARY_BLUE: f64 = 0.97;
const SECONDARY_RED: f64 = 0.60;
const SECONDARY_GREEN: f64 = 0.61;
const SECONDARY_BLUE: f64 = 0.66;

struct DialogueFeedback {
    baseline: String,
    has_override: bool,
    locale: UiLocale,
    count: Retained<NSTextField>,
    error: Retained<NSTextField>,
    save: Retained<NSButton>,
    reset: Retained<NSButton>,
    storage_error: Option<String>,
    undo: Retained<NSUndoManager>,
}

define_class!(
    // SAFETY: AppKit creates and uses this view on the main thread only.
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetDialogueTextView"]
    #[ivars = RefCell<Option<DialogueFeedback>>]
    struct DialogueTextView;

    unsafe impl NSObjectProtocol for DialogueTextView {}
    unsafe impl NSTextDelegate for DialogueTextView {}

    unsafe impl NSTextViewDelegate for DialogueTextView {
        #[unsafe(method_id(undoManagerForTextView:))]
        fn undo_manager_for_text_view(&self, _view: &NSTextView) -> Option<Retained<NSUndoManager>> {
            self.dialogue_undo_manager()
        }
    }

    impl DialogueTextView {
        #[unsafe(method_id(undoManager))]
        fn undo_manager(&self) -> Option<Retained<NSUndoManager>> {
            self.dialogue_undo_manager()
        }

        #[unsafe(method(didChangeText))]
        fn did_change_text(&self) {
            let _: () = unsafe { msg_send![super(self), didChangeText] };
            if let Some(feedback) = self.ivars().borrow_mut().as_mut() {
                feedback.storage_error = None;
            }
            self.update_feedback();
        }

        #[unsafe(method(insertTab:))]
        fn insert_tab(&self, _sender: Option<&AnyObject>) {
            if let Some(window) = self.window() {
                window.selectNextKeyView(Some(self));
            }
        }

        #[unsafe(method(insertBacktab:))]
        fn insert_backtab(&self, _sender: Option<&AnyObject>) {
            if let Some(window) = self.window() {
                window.selectPreviousKeyView(Some(self));
            }
        }
    }
);

impl DialogueTextView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn dialogue_undo_manager(&self) -> Option<Retained<NSUndoManager>> {
        self.ivars()
            .borrow()
            .as_ref()
            .map(|feedback| feedback.undo.retain())
    }

    fn replace_dialogue(&self, value: &str) {
        self.breakUndoCoalescing();
        self.setString(&NSString::from_str(value));
        if let Some(undo) = self.dialogue_undo_manager() {
            undo.removeAllActions();
        }
    }

    fn update_feedback(&self) {
        let feedback = self.ivars().borrow();
        let Some(feedback) = feedback.as_ref() else {
            return;
        };
        let value = self.string().to_string();
        let bytes = value.len();
        let dirty = normalized_dialogue(&value) != normalized_dialogue(&feedback.baseline);
        let status = if dirty {
            Message::DialogueUnsaved
        } else {
            Message::DialogueUnchanged
        };
        feedback.count.setStringValue(&NSString::from_str(&format!(
            "{}: {bytes}/{DIALOGUE_MAX_BYTES} · {}",
            text(feedback.locale, Message::DialogueBytes),
            text(feedback.locale, status),
        )));
        let error = if bytes > DIALOGUE_MAX_BYTES {
            text(feedback.locale, Message::DialogueTooLong)
        } else {
            feedback.storage_error.as_deref().unwrap_or("")
        };
        feedback.error.setStringValue(&NSString::from_str(error));
        set_accessibility_label(&*feedback.error, error);
        set_tooltip(&*feedback.error, error);
        feedback
            .save
            .setEnabled(bytes <= DIALOGUE_MAX_BYTES && dirty);
        feedback.reset.setEnabled(feedback.has_override);
    }
}

fn normalized_dialogue(value: &str) -> &str {
    if value.trim().is_empty() {
        ""
    } else {
        value
    }
}

define_class!(
    // SAFETY:
    // - MenuPanelWindow is created and used only on AppKit's main thread.
    // - It owns no Rust references and forwards normal responder messages to NSPanel.
    #[unsafe(super = NSPanel)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuPanel"]
    struct MenuPanelWindow;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for MenuPanelWindow {}

    impl MenuPanelWindow {
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
            if event.keyCode() == 53 {
                self.resignKeyWindow();
                self.orderOut(None);
                return;
            }
            if event.keyCode() == 48 {
                if event
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Shift)
                {
                    self.selectPreviousKeyView(None);
                } else {
                    self.selectNextKeyView(None);
                }
                return;
            }
            let _: () = unsafe { msg_send![super(self), keyDown: event] };
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, sender: Option<&AnyObject>) {
            self.resignKeyWindow();
            self.orderOut(None);
            let _ = sender;
        }

        #[unsafe(method(resignKeyWindow))]
        fn resign_key_window(&self) {
            // Keep the panel ordered while AppKit changes key windows. A
            // status-item click can resign this panel before dispatching its
            // toggle action; hiding here would turn a close click into reopen.
            let _: () = unsafe { msg_send![super(self), resignKeyWindow] };
        }
    }
);

define_class!(
    // SAFETY: MenuPanelRoot is main-thread-only and has no Rust-owned ivars.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuPanelRoot"]
    struct MenuPanelRoot;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for MenuPanelRoot {}

    impl MenuPanelRoot {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let fill = color(BG_RED, BG_GREEN, BG_BLUE, 1.0);
            fill.setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                bounds,
                PANEL_RADIUS,
                PANEL_RADIUS,
            )
            .fill();

            let border = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.45);
            border.setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                (PANEL_RADIUS - 0.5).max(0.0),
                (PANEL_RADIUS - 0.5).max(0.0),
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();

            let highlight = color(1.0, 1.0, 1.0, 0.08);
            highlight.setStroke();
            let highlight_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                inset_rect(bounds, 1.0),
                (PANEL_RADIUS - 1.0).max(0.0),
                (PANEL_RADIUS - 1.0).max(0.0),
            );
            highlight_path.setLineWidth(1.0);
            highlight_path.stroke();

            // Footer separator hairline
            let sep_y = bounds.size.height - FOOTER_HEIGHT;
            if sep_y > 0.0 {
                let sep = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.35);
                sep.setStroke();
                let sep_path = NSBezierPath::bezierPath();
                sep_path.moveToPoint(NSPoint::new(14.0, sep_y));
                sep_path.lineToPoint(NSPoint::new((bounds.size.width - 14.0).max(14.0), sep_y));
                sep_path.setLineWidth(1.0);
                sep_path.stroke();
            }
        }
    }
);

define_class!(
    // SAFETY: MenuPanelDocument is a main-thread-only flipped document view.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuPanelDocument"]
    struct MenuPanelDocument;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for MenuPanelDocument {}

    impl MenuPanelDocument {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY: MenuPanelCard is a main-thread-only decorative view.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuPanelCard"]
    struct MenuPanelCard;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for MenuPanelCard {}

    impl MenuPanelCard {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let fill = color(CARD_RED, CARD_GREEN, CARD_BLUE, 1.0);
            fill.setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                bounds,
                CARD_RADIUS,
                CARD_RADIUS,
            )
            .fill();

            let border = color(
                CARD_BORDER_RED,
                CARD_BORDER_GREEN,
                CARD_BORDER_BLUE,
                CARD_BORDER_ALPHA,
            );
            border.setStroke();
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                inset_rect(bounds, 0.5),
                (CARD_RADIUS - 0.5).max(0.0),
                (CARD_RADIUS - 0.5).max(0.0),
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

define_class!(
    // SAFETY: MenuTabTrack is a main-thread-only decorative view.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetMenuTabTrack"]
    struct MenuTabTrack;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for MenuTabTrack {}

    impl MenuTabTrack {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let fill = color(TRACK_RED, TRACK_GREEN, TRACK_BLUE, 1.0);
            fill.setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0).fill();

            let border = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.35);
            border.setStroke();
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                inset_rect(bounds, 0.5),
                7.5,
                7.5,
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl MenuPanelWindow {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSPanel's designated initializer is inherited by this
        // borderless activating subclass.
        unsafe {
            msg_send![
                super(this),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        }
    }
}

impl MenuPanelRoot {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl MenuPanelDocument {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl MenuPanelCard {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl MenuTabTrack {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

pub(crate) struct MenuPanel {
    panel: Retained<MenuPanelWindow>,
    root: Retained<MenuPanelRoot>,
    tab_track: Retained<MenuTabTrack>,
    tab_indicator: Retained<NSBox>,
    scroll: Retained<NSScrollView>,
    document: Retained<MenuPanelDocument>,
    character_tab: Retained<MenuPanelDocument>,
    bubble_tab: Retained<MenuPanelDocument>,
    settings_tab: Retained<MenuPanelDocument>,
    tabs: [Retained<NSButton>; 3],
    placement_buttons: [Retained<NSButton>; 5],
    character_view: Retained<NSView>,
    mtm: MainThreadMarker,
    dialogue_card: Retained<MenuPanelCard>,
    dialogue_title: Retained<NSTextField>,
    dialogue_name: Retained<NSTextField>,
    dialogue_language_label: Retained<NSTextField>,
    dialogue_language: Retained<NSPopUpButton>,
    dialogue_slot_label: Retained<NSTextField>,
    dialogue_slot: Retained<NSPopUpButton>,
    dialogue_original_label: Retained<NSTextField>,
    dialogue_original: Retained<NSTextField>,
    dialogue_input_label: Retained<NSTextField>,
    dialogue_scroll: Retained<NSScrollView>,
    dialogue_text: Retained<DialogueTextView>,
    dialogue_count: Retained<NSTextField>,
    dialogue_error: Retained<NSTextField>,
    dialogue_save: Retained<NSButton>,
    dialogue_reset_entry: Retained<NSButton>,
    dialogue_reset_character: Retained<NSButton>,
    dialogue_active: Option<DialogueTarget>,
    dialogue_edit_locale: UiLocale,
    dialogue_saved_pending: bool,
    dialogue_edit_slot: DialogueSlot,
    dialogue_drafts: BTreeMap<(DialogueTarget, String, DialogueSlot), String>,
    dialogue_overrides: DialogueOverrides,
    dialogue_base: Option<CharacterMetadata>,
    target: Retained<MenuTarget>,
    title: Retained<NSTextField>,
    panel_title: Retained<NSTextField>,
    character_visible_label: Retained<NSTextField>,
    character_visible_switch: Retained<NSSwitch>,
    character_manage_label: Retained<NSButton>,
    bubble_visible_label: Retained<NSTextField>,
    bubble_visible_switch: Retained<NSSwitch>,
    status_indicators_label: Retained<NSTextField>,
    status_indicators_switch: Retained<NSSwitch>,
    bubble_placement_label: Retained<NSTextField>,
    bubble_theme_label: Retained<NSTextField>,
    bubble_theme_popup: Retained<NSPopUpButton>,
    bubble_colors_label: Retained<NSTextField>,
    bubble_color_labels: [Retained<NSTextField>; 5],
    bubble_color_fields: [Retained<NSTextField>; 5],
    bubble_color_swatches: [Retained<NSBox>; 5],
    bubble_apply: Retained<NSButton>,
    bubble_reset: Retained<NSButton>,
    settings_appearance_label: Retained<NSTextField>,
    click_behavior_label: Retained<NSTextField>,
    full_passthrough_label: Retained<NSTextField>,
    full_passthrough_help: Retained<NSTextField>,
    full_passthrough_switch: Retained<NSSwitch>,
    alpha_passthrough_label: Retained<NSTextField>,
    alpha_passthrough_help: Retained<NSTextField>,
    alpha_passthrough_switch: Retained<NSSwitch>,
    scale_label: Retained<NSTextField>,
    scale_readout: Retained<NSTextField>,
    scale_down: Retained<NSButton>,
    scale_up: Retained<NSButton>,
    language_label: Retained<NSTextField>,
    language_popup: Retained<NSPopUpButton>,
    lifecycle_card: LifecycleSettingsCard,
    observation_card: Retained<MenuPanelCard>,
    observation_title: Retained<NSTextField>,
    observation_local_label: Retained<NSTextField>,
    observation_remote_label: Retained<NSTextField>,
    observation_local_switch: Retained<NSSwitch>,
    observation_remote_switch: Retained<NSSwitch>,
    observation_help: Retained<NSTextField>,
    observation_notice: Retained<NSTextField>,
    machine_rows: Vec<(
        Retained<NSSwitch>,
        Retained<NSTextField>,
        Retained<NSTextField>,
    )>,
    observation_catalog: Option<SourceCatalog>,
    observation_preferences: Option<ObservationPreferences>,
    locale: UiLocale,
    status: Retained<NSTextField>,
    reset: Retained<NSButton>,
    quit: Retained<NSButton>,
    selected_tab: usize,
}

impl MenuPanel {
    pub(crate) fn new(
        target: &MenuTarget,
        character_view: &NSView,
        locale: UiLocale,
        mtm: MainThreadMarker,
    ) -> Self {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
        );
        let panel = MenuPanelWindow::new(frame, mtm);
        configure_panel(&panel);
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            let _: () = unsafe { msg_send![&*panel, setAppearance: Some(&*appearance)] };
        }

        let root = MenuPanelRoot::new(frame, mtm);
        set_accessibility_element(&*root, false);
        root.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        panel.setContentView(Some(&root));

        let document = MenuPanelDocument::new(
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PANEL_WIDTH, CHARACTER_CONTENT_HEIGHT),
            ),
            mtm,
        );
        set_accessibility_element(&*document, false);
        document.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);

        let scroll: Retained<NSScrollView> = unsafe {
            msg_send![
                NSScrollView::alloc(mtm),
                initWithFrame: NSRect::new(
                    NSPoint::new(0.0, SCROLL_TOP),
                    NSSize::new(PANEL_WIDTH, PANEL_HEIGHT - SCROLL_TOP - FOOTER_HEIGHT),
                )
            ]
        };
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.contentView().setDrawsBackground(false);
        scroll.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        scroll.setDocumentView(Some(&document));
        document.setAutoresizesSubviews(false);
        root.addSubview(&scroll);

        let character_tab = MenuPanelDocument::new(
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PANEL_WIDTH, CHARACTER_CONTENT_HEIGHT),
            ),
            mtm,
        );
        character_tab.setAutoresizesSubviews(false);
        set_accessibility_element(&*character_tab, false);
        let bubble_tab = MenuPanelDocument::new(
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PANEL_WIDTH, BUBBLE_CONTENT_HEIGHT),
            ),
            mtm,
        );
        bubble_tab.setAutoresizesSubviews(false);
        set_accessibility_element(&*bubble_tab, false);
        let settings_tab = MenuPanelDocument::new(
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PANEL_WIDTH, SETTINGS_BASE_HEIGHT + 216.0),
            ),
            mtm,
        );
        settings_tab.setAutoresizesSubviews(false);
        set_accessibility_element(&*settings_tab, false);
        document.addSubview(&character_tab);
        document.addSubview(&bubble_tab);
        document.addSubview(&settings_tab);
        bubble_tab.setHidden(true);
        settings_tab.setHidden(true);

        let title = label("Herdr", 14.0, true, primary(), mtm);
        let panel_title = label(
            text(locale, Message::MenuPanelTitle),
            11.0,
            false,
            secondary(),
            mtm,
        );
        panel.setTitle(&NSString::from_str(text(locale, Message::MenuPanelTitle)));
        root.addSubview(&title);
        root.addSubview(&panel_title);

        let tab_track = MenuTabTrack::new(NSRect::default(), mtm);
        set_accessibility_element(&*tab_track, false);
        root.addSubview(&tab_track);

        let tab_indicator = make_tab_indicator(mtm);
        root.addSubview(&tab_indicator);

        let tabs = [
            make_tab(
                "person.crop.circle.fill",
                text(locale, Message::MenuCharacterTab),
                0,
                target,
                mtm,
            ),
            make_tab(
                "bubble.left.and.bubble.right.fill",
                text(locale, Message::MenuBubbleTab),
                1,
                target,
                mtm,
            ),
            make_tab(
                "gearshape.fill",
                text(locale, Message::MenuSettingsTab),
                2,
                target,
                mtm,
            ),
        ];
        for tab in &tabs {
            root.addSubview(tab);
        }

        let character_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*character_card, false);
        let character_visible_label = label(
            text(locale, Message::MenuCharacterVisible),
            12.5,
            true,
            primary(),
            mtm,
        );
        let character_visible_switch = make_switch(target, sel!(hide:), mtm);
        set_accessibility_label(
            &*character_visible_switch,
            text(locale, Message::MenuCharacterVisible),
        );
        let character_manage_label = make_section_button(
            text(locale, Message::MenuManageCharacter),
            target,
            sel!(focusCharacterManager:),
            mtm,
        );
        let character_view = character_view.retain();
        character_card.addSubview(&character_visible_label);
        character_card.addSubview(&*character_visible_switch);
        character_tab.addSubview(&character_card);
        character_tab.addSubview(&character_manage_label);
        character_tab.addSubview(&character_view);

        let dialogue_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*dialogue_card, false);
        let dialogue_title = label(
            text(locale, Message::DialogueEditor),
            12.5,
            true,
            primary(),
            mtm,
        );
        let dialogue_name = label("", 11.0, false, secondary(), mtm);
        let dialogue_language_label = label(
            text(locale, Message::DialogueLanguage),
            11.0,
            false,
            primary(),
            mtm,
        );
        let dialogue_language = make_selection_popup(
            &[(Message::KoreanLanguage, 0), (Message::EnglishLanguage, 1)],
            locale,
            target,
            sel!(setDialogueLocale:),
            mtm,
        );
        let dialogue_slot_label = label(
            text(locale, Message::DialogueSlot),
            11.0,
            false,
            primary(),
            mtm,
        );
        let dialogue_slot = make_selection_popup(
            &dialogue_slot_messages().map(|(message, index)| (message, index as isize)),
            locale,
            target,
            sel!(setDialogueSlot:),
            mtm,
        );
        let dialogue_original_label = label(
            text(locale, Message::DialogueOriginal),
            11.0,
            false,
            primary(),
            mtm,
        );
        let dialogue_original = label("", 10.5, false, secondary(), mtm);
        dialogue_original.setMaximumNumberOfLines(2);
        let dialogue_input_label = label(
            text(locale, Message::DialogueText),
            11.0,
            false,
            primary(),
            mtm,
        );
        let dialogue_text = DialogueTextView::new(NSRect::default(), mtm);
        dialogue_text.setRichText(false);
        dialogue_text.setImportsGraphics(false);
        dialogue_text.setAllowsUndo(true);
        dialogue_text.setDelegate(Some(ProtocolObject::from_ref(&*dialogue_text)));
        dialogue_text.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        dialogue_text.setTextColor(Some(&primary()));
        dialogue_text.setBackgroundColor(&color(TRACK_RED, TRACK_GREEN, TRACK_BLUE, 1.0));
        dialogue_text.setTextContainerInset(NSSize::new(6.0, 5.0));
        dialogue_text.setVerticallyResizable(true);
        dialogue_text.setHorizontallyResizable(false);
        dialogue_text.setAutomaticQuoteSubstitutionEnabled(false);
        dialogue_text.setAutomaticDashSubstitutionEnabled(false);
        dialogue_text.setAutomaticTextReplacementEnabled(false);
        dialogue_text.setAutomaticSpellingCorrectionEnabled(false);
        if let Some(container) = unsafe { dialogue_text.textContainer() } {
            container.setWidthTracksTextView(true);
            container.setContainerSize(NSSize::new(300.0, 10_000_000.0));
        }
        let dialogue_scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: NSRect::default()] };
        dialogue_scroll.setHasVerticalScroller(true);
        dialogue_scroll.setHasHorizontalScroller(false);
        dialogue_scroll.setAutohidesScrollers(true);
        dialogue_scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        dialogue_scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        dialogue_scroll.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
        dialogue_scroll.setDocumentView(Some(&dialogue_text));
        let dialogue_count = label("", 10.0, false, secondary(), mtm);
        let dialogue_error = label("", 10.0, false, color(1.0, 0.55, 0.52, 1.0), mtm);
        dialogue_error.setMaximumNumberOfLines(2);
        let dialogue_save = make_action_button(
            text(locale, Message::DialogueSave),
            target,
            sel!(saveDialogue:),
            mtm,
        );
        let dialogue_reset_entry = make_action_button(
            text(locale, Message::DialogueResetEntry),
            target,
            sel!(resetDialogueEntry:),
            mtm,
        );
        let dialogue_reset_character = make_action_button(
            text(locale, Message::DialogueResetCharacter),
            target,
            sel!(resetCharacterDialogue:),
            mtm,
        );
        dialogue_reset_character.setEnabled(false);
        *dialogue_text.ivars().borrow_mut() = Some(DialogueFeedback {
            baseline: String::new(),
            has_override: false,
            locale,
            count: dialogue_count.retain(),
            error: dialogue_error.retain(),
            save: dialogue_save.retain(),
            reset: dialogue_reset_entry.retain(),
            storage_error: None,
            undo: NSUndoManager::new(mtm),
        });
        for view in [
            &*dialogue_title as &NSView,
            &*dialogue_name,
            &*dialogue_language_label,
            &*dialogue_language,
            &*dialogue_slot_label,
            &*dialogue_slot,
            &*dialogue_original_label,
            &*dialogue_original,
            &*dialogue_input_label,
            &*dialogue_scroll,
            &*dialogue_count,
            &*dialogue_error,
            &*dialogue_save,
            &*dialogue_reset_entry,
            &*dialogue_reset_character,
        ] {
            dialogue_card.addSubview(view);
        }
        character_tab.addSubview(&dialogue_card);

        let bubble_visibility_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*bubble_visibility_card, false);
        let bubble_visible_label = label(
            text(locale, Message::MenuBubbleVisible),
            12.5,
            true,
            primary(),
            mtm,
        );
        let bubble_visible_switch = make_switch(target, sel!(hideBubble:), mtm);
        set_accessibility_label(
            &*bubble_visible_switch,
            text(locale, Message::MenuBubbleVisible),
        );
        bubble_visibility_card.addSubview(&bubble_visible_label);
        bubble_visibility_card.addSubview(&*bubble_visible_switch);
        bubble_tab.addSubview(&bubble_visibility_card);

        let status_indicators_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*status_indicators_card, false);
        let status_indicators_label = label(
            text(locale, Message::MenuStatusIndicators),
            12.5,
            true,
            primary(),
            mtm,
        );
        let status_indicators_switch = make_switch(target, sel!(toggleStatusIndicators:), mtm);
        status_indicators_switch.setState(NSControlStateValueOn);
        set_accessibility_label(
            &*status_indicators_switch,
            text(locale, Message::MenuStatusIndicators),
        );
        status_indicators_card.addSubview(&status_indicators_label);
        status_indicators_card.addSubview(&*status_indicators_switch);
        bubble_tab.addSubview(&status_indicators_card);

        let bubble_placement_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*bubble_placement_card, false);
        let bubble_placement_label = label(
            text(locale, Message::MenuBubblePlacement),
            12.5,
            true,
            primary(),
            mtm,
        );
        bubble_placement_card.addSubview(&bubble_placement_label);
        let placement_specs = [
            (Message::MenuPlacementAuto, sel!(bubbleAuto:)),
            (Message::MenuPlacementAbove, sel!(bubbleAbove:)),
            (Message::MenuPlacementBelow, sel!(bubbleBelow:)),
            (Message::MenuPlacementLeft, sel!(bubbleLeft:)),
            (Message::MenuPlacementRight, sel!(bubbleRight:)),
        ];
        let placement_buttons = [
            make_segment_button(
                text(locale, placement_specs[0].0),
                target,
                placement_specs[0].1,
                mtm,
            ),
            make_segment_button(
                text(locale, placement_specs[1].0),
                target,
                placement_specs[1].1,
                mtm,
            ),
            make_segment_button(
                text(locale, placement_specs[2].0),
                target,
                placement_specs[2].1,
                mtm,
            ),
            make_segment_button(
                text(locale, placement_specs[3].0),
                target,
                placement_specs[3].1,
                mtm,
            ),
            make_segment_button(
                text(locale, placement_specs[4].0),
                target,
                placement_specs[4].1,
                mtm,
            ),
        ];
        for button in &placement_buttons {
            bubble_placement_card.addSubview(button);
        }
        bubble_tab.addSubview(&bubble_placement_card);
        let bubble_theme_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*bubble_theme_card, false);
        let bubble_theme_label = label(
            text(locale, Message::BubbleTheme),
            12.5,
            true,
            primary(),
            mtm,
        );
        let theme_specs = [
            (Message::ThemeWarmIvory, 0),
            (Message::ThemeDustyRose, 1),
            (Message::ThemeMoonlitInk, 2),
            (Message::ThemeCustom, 3),
        ];
        let bubble_theme_popup =
            make_selection_popup(&theme_specs, locale, target, sel!(setBubbleTheme:), mtm);
        bubble_theme_card.addSubview(&bubble_theme_label);
        bubble_theme_card.addSubview(&bubble_theme_popup);
        let bubble_colors_label = label(
            text(locale, Message::CustomizeBubbleColors),
            11.0,
            false,
            secondary(),
            mtm,
        );
        let color_messages = [
            Message::BubbleSurfaceColor,
            Message::BubbleTextColor,
            Message::BubbleMutedColor,
            Message::BubbleBorderColor,
            Message::BubbleAccentColor,
        ];
        let bubble_color_labels =
            color_messages.map(|message| label(text(locale, message), 11.0, false, primary(), mtm));
        let bubble_color_fields = color_messages.map(|message| {
            let field = NSTextField::new(mtm);
            field.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)));
            field.setTextColor(Some(&primary()));
            field.setEditable(true);
            field.setSelectable(true);
            field.setBezeled(true);
            field.setRefusesFirstResponder(false);
            set_accessibility_label(&field, text(locale, message));
            field
        });
        let bubble_color_swatches = [
            make_color_swatch(mtm),
            make_color_swatch(mtm),
            make_color_swatch(mtm),
            make_color_swatch(mtm),
            make_color_swatch(mtm),
        ];
        let bubble_apply = make_action_button(
            text(locale, Message::ApplyBubbleColors),
            target,
            sel!(applyBubbleColors:),
            mtm,
        );
        let bubble_reset = make_action_button(
            text(locale, Message::ResetBubbleColors),
            target,
            sel!(resetBubbleColors:),
            mtm,
        );
        set_accessibility_label(&bubble_apply, text(locale, Message::ApplyBubbleColors));
        set_accessibility_label(&bubble_reset, text(locale, Message::ResetBubbleColors));
        bubble_theme_card.addSubview(&bubble_colors_label);
        for ((name, field), swatch) in bubble_color_labels
            .iter()
            .zip(&bubble_color_fields)
            .zip(&bubble_color_swatches)
        {
            bubble_theme_card.addSubview(name);
            bubble_theme_card.addSubview(swatch);
            bubble_theme_card.addSubview(field);
        }
        bubble_theme_card.addSubview(&bubble_apply);
        bubble_theme_card.addSubview(&bubble_reset);
        bubble_tab.addSubview(&bubble_theme_card);

        let appearance_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*appearance_card, false);
        let settings_appearance_label = label(
            text(locale, Message::MenuAppearance),
            12.5,
            true,
            primary(),
            mtm,
        );
        let click_behavior_label = label(
            text(locale, Message::MenuClickBehavior),
            11.0,
            false,
            secondary(),
            mtm,
        );
        let full_passthrough_label = label(
            text(locale, Message::MenuFullPassthrough),
            12.0,
            true,
            primary(),
            mtm,
        );
        let full_passthrough_help = label(
            text(locale, Message::MenuFullPassthroughHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        let full_passthrough_switch = make_switch(target, sel!(togglePassthrough:), mtm);
        let alpha_passthrough_label = label(
            text(locale, Message::MenuAlphaPassthrough),
            12.0,
            true,
            primary(),
            mtm,
        );
        let alpha_passthrough_help = label(
            text(locale, Message::MenuAlphaPassthroughHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        let alpha_passthrough_switch = make_switch(target, sel!(toggleAlphaPassthrough:), mtm);
        for view in [
            &*settings_appearance_label as &NSView,
            &*click_behavior_label,
            &*full_passthrough_label,
            &*full_passthrough_help,
            &*full_passthrough_switch,
            &*alpha_passthrough_label,
            &*alpha_passthrough_help,
            &*alpha_passthrough_switch,
        ] {
            appearance_card.addSubview(view);
        }
        set_accessibility_label(
            &*full_passthrough_switch,
            text(locale, Message::MenuFullPassthrough),
        );
        set_accessibility_label(
            &*alpha_passthrough_switch,
            text(locale, Message::MenuAlphaPassthrough),
        );
        settings_tab.addSubview(&appearance_card);

        let scale_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*scale_card, false);
        let scale_label = label(text(locale, Message::MenuScale), 12.5, true, primary(), mtm);
        let scale_down = make_stepper_button("−", target, sel!(scaleDown:), mtm);
        let scale_up = make_stepper_button("+", target, sel!(scaleUp:), mtm);
        let scale_readout = label("0.65×", 12.0, true, primary(), mtm);
        scale_readout.setAlignment(NSTextAlignment::Center);
        set_accessibility_label(&*scale_down, text(locale, Message::ScaleDown));
        set_accessibility_label(&*scale_up, text(locale, Message::ScaleUp));
        set_accessibility_label(&*scale_readout, text(locale, Message::MenuScale));
        scale_card.addSubview(&scale_label);
        scale_card.addSubview(&scale_down);
        scale_card.addSubview(&scale_up);
        scale_card.addSubview(&scale_readout);
        settings_tab.addSubview(&scale_card);

        let language_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*language_card, false);
        let language_label = label(
            text(locale, Message::MenuLanguage),
            12.5,
            true,
            primary(),
            mtm,
        );
        let language_specs = [
            (Message::LanguageSystem, 0),
            (Message::KoreanLanguage, 1),
            (Message::EnglishLanguage, 2),
        ];
        let language_popup =
            make_selection_popup(&language_specs, locale, target, sel!(setLanguage:), mtm);
        language_card.addSubview(&language_label);
        language_card.addSubview(&language_popup);
        settings_tab.addSubview(&language_card);
        let observation_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*observation_card, false);
        let observation_title = label(
            text(locale, Message::ObservationTitle),
            12.5,
            true,
            primary(),
            mtm,
        );
        let observation_local_label = label(
            text(locale, Message::ObservationLocal),
            12.0,
            true,
            primary(),
            mtm,
        );
        let observation_remote_label = label(
            text(locale, Message::ObservationRemote),
            12.0,
            true,
            primary(),
            mtm,
        );
        let observation_local_switch = make_switch(target, sel!(setObservationLocal:), mtm);
        let observation_remote_switch = make_switch(target, sel!(setObservationRemote:), mtm);
        let observation_help = label(
            text(locale, Message::ObservationHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        let observation_notice = label(
            text(locale, Message::ObservationEmpty),
            10.5,
            false,
            secondary(),
            mtm,
        );
        for view in [
            &*observation_title as &NSView,
            &*observation_local_label,
            &*observation_remote_label,
            &*observation_local_switch,
            &*observation_remote_switch,
            &*observation_help,
            &*observation_notice,
        ] {
            observation_card.addSubview(view);
        }
        settings_tab.addSubview(&observation_card);
        let lifecycle_card = LifecycleSettingsCard::new(locale, target, mtm);
        settings_tab.addSubview(lifecycle_card.view());

        let status = label("", 10.5, false, secondary(), mtm);
        status.setMaximumNumberOfLines(2);
        let reset = make_action_button(
            text(locale, Message::ResetPosition),
            target,
            sel!(resetPosition:),
            mtm,
        );
        let quit = make_action_button(text(locale, Message::MenuQuit), target, sel!(quit:), mtm);
        reset.setKeyEquivalent(&NSString::from_str(""));
        quit.setKeyEquivalent(&NSString::from_str(""));
        set_accessibility_label(&*reset, text(locale, Message::ResetPosition));
        set_accessibility_label(&*quit, text(locale, Message::MenuQuit));
        root.addSubview(&status);
        root.addSubview(&reset);
        root.addSubview(&quit);

        let mut panel = Self {
            panel,
            root,
            tab_track,
            tab_indicator,
            scroll,
            document,
            character_tab,
            bubble_tab,
            settings_tab,
            tabs,
            placement_buttons,
            character_view,
            mtm,
            dialogue_card,
            dialogue_title,
            dialogue_name,
            dialogue_language_label,
            dialogue_language,
            dialogue_slot_label,
            dialogue_slot,
            dialogue_original_label,
            dialogue_original,
            dialogue_input_label,
            dialogue_scroll,
            dialogue_text,
            dialogue_count,
            dialogue_error,
            dialogue_save,
            dialogue_reset_entry,
            dialogue_reset_character,
            dialogue_active: None,
            dialogue_edit_locale: locale,
            dialogue_edit_slot: DialogueSlot::Idle,
            dialogue_saved_pending: false,
            dialogue_drafts: BTreeMap::new(),
            dialogue_overrides: DialogueOverrides::default(),
            dialogue_base: None,
            target: target.retain(),
            title,
            panel_title,
            character_visible_label,
            character_visible_switch,
            character_manage_label,
            bubble_visible_label,
            bubble_visible_switch,
            status_indicators_label,
            status_indicators_switch,
            bubble_placement_label,
            bubble_theme_label,
            bubble_theme_popup,
            bubble_colors_label,
            bubble_color_labels,
            bubble_color_fields,
            bubble_color_swatches,
            bubble_apply,
            bubble_reset,
            settings_appearance_label,
            click_behavior_label,
            full_passthrough_label,
            full_passthrough_help,
            full_passthrough_switch,
            alpha_passthrough_label,
            alpha_passthrough_help,
            alpha_passthrough_switch,
            scale_label,
            scale_readout,
            scale_down,
            scale_up,
            language_label,
            language_popup,
            lifecycle_card,
            observation_card,
            observation_title,
            observation_local_label,
            observation_remote_label,
            observation_local_switch,
            observation_remote_switch,
            observation_help,
            observation_notice,
            machine_rows: Vec::new(),
            observation_catalog: None,
            observation_preferences: None,
            locale,
            status,
            reset,
            quit,
            selected_tab: 0,
        };
        panel.select_tab(0);
        panel.update_color_swatches();
        panel.set_locale(locale);
        panel.layout_root();
        panel
    }

    pub(crate) fn toggle(&self, anchor: &NSView) {
        if self.panel.isVisible() {
            self.hide();
            return;
        }
        self.reanchor(anchor);
        self.panel.makeKeyAndOrderFront(None);
        let first = &self.tabs[self.selected_tab];
        self.panel.makeFirstResponder(Some(&**first));
    }

    pub(crate) fn hide(&self) {
        if self.panel.isKeyWindow() {
            self.panel.resignKeyWindow();
        }
        self.panel.orderOut(None);
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.panel.isVisible()
    }

    pub(crate) fn select_tab(&mut self, index: usize) {
        let index = index.min(2);
        if self.selected_tab != index {
            let _ = self.panel.makeFirstResponder(None);
        }
        self.character_tab.setHidden(index != 0);
        self.bubble_tab.setHidden(index != 1);
        self.settings_tab.setHidden(index != 2);
        self.selected_tab = index;
        self.apply_tab_selection();
        self.layout_documents();
        self.panel.recalculateKeyViewLoop();
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, 0.0));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
        if self.panel.isKeyWindow() {
            let tab = &self.tabs[index];
            self.panel.makeFirstResponder(Some(&**tab));
        }
    }

    pub(crate) fn focus_character_manager(&mut self) {
        self.select_tab(0);
        let point = self
            .scroll
            .contentView()
            .constrainBoundsRect(NSRect::new(
                NSPoint::new(0.0, 0.0),
                self.scroll.contentView().bounds().size,
            ))
            .origin;
        self.scroll.contentView().scrollToPoint(point);
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
        if self.panel.isKeyWindow() {
            self.panel
                .makeFirstResponder(Some(&*self.character_visible_switch));
        }
    }
    pub(crate) fn set_bubble_appearance(&mut self, appearance: BubbleAppearance) {
        let tag = match appearance.theme {
            BubbleTheme::WarmIvory => 0,
            BubbleTheme::DustyRose => 1,
            BubbleTheme::MoonlitInk => 2,
            BubbleTheme::Custom => 3,
        };
        self.bubble_theme_popup.selectItemWithTag(tag);
        let palette = appearance.palette();
        for (field, value) in self.bubble_color_fields.iter().zip([
            palette.surface,
            palette.text,
            palette.muted,
            palette.border,
            palette.accent,
        ]) {
            field.setStringValue(&NSString::from_str(&value.to_hex()));
        }
        self.update_color_swatches();
    }

    pub(crate) fn set_show_status_indicators(&self, enabled: bool) {
        self.status_indicators_switch.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    pub(crate) fn custom_bubble_palette(&self) -> Result<BubblePalette, String> {
        let parse = |index: usize| -> Result<BubbleColor, String> {
            BubbleColor::parse_hex(&self.bubble_color_fields[index].stringValue().to_string())
        };
        Ok(BubblePalette {
            surface: parse(0)?,
            text: parse(1)?,
            muted: parse(2)?,
            border: parse(3)?,
            accent: parse(4)?,
        })
    }

    pub(crate) fn sync(
        &mut self,
        scene: &Scene,
        language: LanguagePreference,
        status: &str,
        lifecycle: LifecycleSettings,
        observation: &ObservationPreferences,
        catalog: &SourceCatalog,
    ) {
        self.character_visible_switch.setState(if scene.visible {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        self.bubble_visible_switch
            .setState(if scene.bubble_visible {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        set_action(
            &*self.character_visible_switch,
            &self.target,
            if scene.visible {
                sel!(hide:)
            } else {
                sel!(show:)
            },
        );

        set_action(
            &*self.bubble_visible_switch,
            &self.target,
            if scene.bubble_visible {
                sel!(hideBubble:)
            } else {
                sel!(showBubble:)
            },
        );
        let active_placement = placement_index(scene.bubble_placement);
        for (index, button) in self.placement_buttons.iter().enumerate() {
            button.setState(if index == active_placement {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }

        self.full_passthrough_switch.setState(if scene.passthrough {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        self.alpha_passthrough_switch
            .setState(if scene.alpha_passthrough {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });

        let scale = if scene.scale.is_finite() {
            scene.scale.clamp(MIN_SCALE, MAX_SCALE)
        } else {
            MIN_SCALE
        };
        self.scale_readout
            .setStringValue(&NSString::from_str(&format!("{scale:.2}×")));
        self.scale_down.setEnabled(scale > MIN_SCALE + f64::EPSILON);
        self.scale_up.setEnabled(scale < MAX_SCALE - f64::EPSILON);

        self.set_language_preference(language);
        self.status.setStringValue(&NSString::from_str(status));
        self.lifecycle_card.sync(lifecycle);
        self.sync_observation(observation, catalog);
        self.update_color_swatches();
        self.layout_root();
    }

    pub(crate) fn set_language_preference(&self, language: LanguagePreference) {
        let tag = match language {
            LanguagePreference::System => 0,
            LanguagePreference::Ko => 1,
            LanguagePreference::En => 2,
        };
        self.language_popup.selectItemWithTag(tag);
    }

    pub(crate) fn revert_observation_controls(&mut self) {
        self.observation_preferences = None;
    }

    pub(crate) fn sync_observation(
        &mut self,
        preferences: &ObservationPreferences,
        catalog: &SourceCatalog,
    ) {
        if self.observation_catalog.as_ref() == Some(catalog)
            && self.observation_preferences.as_ref() == Some(preferences)
        {
            return;
        }
        let catalog_changed = self.observation_catalog.as_ref() != Some(catalog);
        self.observation_local_switch
            .setState(if preferences.local {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        self.observation_remote_switch
            .setState(if preferences.remote {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        if catalog_changed {
            for (toggle, title, detail) in self.machine_rows.drain(..) {
                toggle.removeFromSuperview();
                title.removeFromSuperview();
                detail.removeFromSuperview();
            }
            for machine in &catalog.machines {
                let toggle = make_switch(&self.target, sel!(setObservationMachine:), self.mtm);
                let id = NSString::from_str(&machine.id);
                toggle.setIdentifier(Some(&id));
                toggle.setEnabled(machine.enabled);
                let title = label(&machine.label, 11.5, true, primary(), self.mtm);
                let status = if !machine.enabled {
                    text(self.locale, Message::ObservationDisabled)
                } else {
                    match machine.status {
                        MachineStatus::Connecting => {
                            text(self.locale, Message::ObservationConnecting)
                        }
                        MachineStatus::Online => text(self.locale, Message::ObservationOnline),
                        MachineStatus::Offline => text(self.locale, Message::ObservationOffline),
                        MachineStatus::NotSelected => {
                            text(self.locale, Message::ObservationNotSelected)
                        }
                    }
                };
                let mut detail_text = format!("{} · {}", machine.remote_session, status);
                if machine.enabled && machine.status == MachineStatus::Offline {
                    let command = format!(
                        "herdr machine reconnect '{}'",
                        machine.id.replace('\'', "'\\''")
                    );
                    detail_text.push('\n');
                    detail_text.push_str(text(self.locale, Message::ObservationReconnect));
                    detail_text.push(' ');
                    detail_text.push_str(&command);
                }
                if let Some(error) = machine.error.as_deref() {
                    detail_text.push('\n');
                    detail_text.push_str(error);
                }
                let detail = label(&detail_text, 10.0, false, secondary(), self.mtm);
                detail.setMaximumNumberOfLines(4);
                set_accessibility_label(&*toggle, &format!("{} · {}", machine.label, detail_text));
                set_tooltip(&*toggle, &detail_text);
                self.observation_card.addSubview(&toggle);
                self.observation_card.addSubview(&title);
                self.observation_card.addSubview(&detail);
                self.machine_rows.push((toggle, title, detail));
            }
            self.observation_catalog = Some(catalog.clone());
        }
        for (machine, (toggle, _, _)) in catalog.machines.iter().zip(&self.machine_rows) {
            toggle.setState(if preferences.machines.contains(&machine.id) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        let mut notice = String::new();
        if !preferences.local
            && (!preferences.remote
                || !catalog
                    .machines
                    .iter()
                    .any(|machine| machine.enabled && preferences.machines.contains(&machine.id)))
        {
            notice.push_str(text(self.locale, Message::ObservationNone));
        }
        if let Some(error) = &catalog.error {
            if !notice.is_empty() {
                notice.push('\n');
            }
            notice.push_str(text(self.locale, Message::ObservationError));
            notice.push(' ');
            notice.push_str(error);
        } else if catalog.machines.is_empty() {
            if !notice.is_empty() {
                notice.push('\n');
            }
            notice.push_str(text(self.locale, Message::ObservationEmpty));
        }
        self.observation_notice
            .setStringValue(&NSString::from_str(&notice));
        set_accessibility_label(&*self.observation_notice, &notice);
        self.observation_preferences = Some(preferences.clone());
        self.layout_documents();
    }

    pub(crate) fn sync_dialogue(
        &mut self,
        target: DialogueTarget,
        name: &str,
        overrides: &DialogueOverrides,
        base: Option<&CharacterMetadata>,
        initial_locale: UiLocale,
    ) {
        if self.dialogue_active.as_ref() != Some(&target) {
            self.dialogue_saved_pending = false;
            self.capture_dialogue();
            if self.dialogue_active.is_none() {
                self.dialogue_edit_locale = initial_locale;
            }
            self.dialogue_active = Some(target);
            self.dialogue_edit_slot = DialogueSlot::Idle;
            self.dialogue_overrides = overrides.clone();
            self.dialogue_base = base.cloned();
            self.load_dialogue();
        } else {
            self.dialogue_overrides = overrides.clone();
            self.dialogue_base = base.cloned();
            let baseline = self.committed_dialogue();
            if self.dialogue_saved_pending {
                self.dialogue_text.replace_dialogue(&baseline);
                self.dialogue_saved_pending = false;
            }
            let mut feedback = self.dialogue_text.ivars().borrow_mut();
            if let Some(feedback) = feedback.as_mut() {
                feedback.has_override = !baseline.is_empty();
                feedback.baseline = baseline;
            }
            drop(feedback);
            self.dialogue_text.update_feedback();
            self.update_dialogue_reference();
        }
        self.dialogue_name.setStringValue(&NSString::from_str(name));
        set_accessibility_label(&*self.dialogue_name, name);
        self.dialogue_reset_character.setEnabled(true);
    }

    pub(crate) fn select_dialogue_locale(&mut self, locale: UiLocale) {
        if self.dialogue_edit_locale != locale {
            self.capture_dialogue();
            self.dialogue_edit_locale = locale;
            self.load_dialogue();
        }
    }

    pub(crate) fn select_dialogue_slot(&mut self, slot: DialogueSlot) {
        if self.dialogue_edit_slot != slot {
            self.capture_dialogue();
            self.dialogue_edit_slot = slot;
            self.load_dialogue();
        }
    }

    pub(crate) fn dialogue_edit(
        &mut self,
    ) -> Option<(DialogueTarget, UiLocale, DialogueSlot, String)> {
        self.capture_dialogue();
        Some((
            self.dialogue_active.clone()?,
            self.dialogue_edit_locale,
            self.dialogue_edit_slot,
            self.dialogue_text.string().to_string(),
        ))
    }

    pub(crate) fn dialogue_target(&self) -> Option<DialogueTarget> {
        self.dialogue_active.clone()
    }

    pub(crate) fn dialogue_saved(
        &mut self,
        target: &DialogueTarget,
        locale: UiLocale,
        slot: DialogueSlot,
    ) {
        self.dialogue_drafts
            .remove(&(target.clone(), locale.tag().to_owned(), slot));
        if self.dialogue_active.as_ref() == Some(target)
            && self.dialogue_edit_locale == locale
            && self.dialogue_edit_slot == slot
        {
            let value = self.dialogue_text.string().to_string();
            self.dialogue_saved_pending = true;
            let mut feedback = self.dialogue_text.ivars().borrow_mut();
            if let Some(feedback) = feedback.as_mut() {
                feedback.baseline = normalized_dialogue(&value).to_owned();
                feedback.has_override = !feedback.baseline.is_empty();
                feedback.storage_error = None;
            }
            drop(feedback);
            self.dialogue_text.update_feedback();
        }
    }

    pub(crate) fn dialogue_reset(&mut self, target: &DialogueTarget) {
        self.dialogue_saved_pending = false;
        self.dialogue_drafts.retain(|(key, _, _), _| key != target);
        if self.dialogue_active.as_ref() == Some(target) {
            self.dialogue_text.replace_dialogue("");
            if let Some(feedback) = self.dialogue_text.ivars().borrow_mut().as_mut() {
                feedback.baseline.clear();
                feedback.has_override = false;
                feedback.storage_error = None;
            }
            self.dialogue_text.update_feedback();
        }
    }

    pub(crate) fn set_dialogue_error(&mut self, detail: Option<&str>) {
        if let Some(feedback) = self.dialogue_text.ivars().borrow_mut().as_mut() {
            feedback.storage_error = detail.map(str::to_owned);
        }
        self.dialogue_text.update_feedback();
    }

    fn committed_dialogue(&self) -> String {
        self.dialogue_active
            .as_ref()
            .and_then(|target| {
                self.dialogue_overrides.entry(
                    target,
                    self.dialogue_edit_locale.tag(),
                    self.dialogue_edit_slot,
                )
            })
            .unwrap_or("")
            .to_owned()
    }

    fn capture_dialogue(&mut self) {
        let Some(target) = self.dialogue_active.as_ref() else {
            return;
        };
        let key = (
            target.clone(),
            self.dialogue_edit_locale.tag().to_owned(),
            self.dialogue_edit_slot,
        );
        let value = self.dialogue_text.string().to_string();
        if normalized_dialogue(&value) == normalized_dialogue(&self.committed_dialogue()) {
            self.dialogue_drafts.remove(&key);
        } else {
            self.dialogue_drafts.insert(key, value);
        }
    }

    fn load_dialogue(&mut self) {
        let baseline = self.committed_dialogue();
        let value = self
            .dialogue_active
            .as_ref()
            .and_then(|target| {
                self.dialogue_drafts.get(&(
                    target.clone(),
                    self.dialogue_edit_locale.tag().to_owned(),
                    self.dialogue_edit_slot,
                ))
            })
            .unwrap_or(&baseline);
        self.dialogue_text.replace_dialogue(value);
        if let Some(feedback) = self.dialogue_text.ivars().borrow_mut().as_mut() {
            feedback.has_override = !baseline.is_empty();
            feedback.baseline = baseline;
            feedback.storage_error = None;
        }
        self.dialogue_language
            .selectItemWithTag(if self.dialogue_edit_locale == UiLocale::Ko {
                0
            } else {
                1
            });
        if let Some(index) = DialogueSlot::ALL
            .iter()
            .position(|slot| *slot == self.dialogue_edit_slot)
        {
            self.dialogue_slot.selectItemWithTag(index as isize);
        }
        self.dialogue_text.update_feedback();
        self.update_dialogue_reference();
    }

    fn update_dialogue_reference(&self) {
        let locale = self.dialogue_edit_locale;
        let slot = self.dialogue_edit_slot;
        let original = self.dialogue_base.as_ref().and_then(|base| {
            if slot.is_reaction() {
                base.dialogue_text("", Some(slot.key()), locale.tag())
            } else {
                base.dialogue_text(slot.key(), None, locale.tag())
            }
        });
        let reference = original
            .or_else(|| match slot {
                DialogueSlot::HeadTap => Some(default_dialogue(locale, DefaultDialogue::HeadTap)),
                DialogueSlot::BodyTap => Some(default_dialogue(locale, DefaultDialogue::BodyTap)),
                DialogueSlot::Pet => Some(default_dialogue(locale, DefaultDialogue::Pet)),
                DialogueSlot::Completion => {
                    Some(default_dialogue(locale, DefaultDialogue::Completion))
                }
                _ => None,
            })
            .unwrap_or(text(self.locale, Message::DialogueStatusReference));
        self.dialogue_original
            .setStringValue(&NSString::from_str(reference));
        set_accessibility_label(&*self.dialogue_original, reference);
        set_tooltip(&*self.dialogue_original, reference);
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.panel
            .setTitle(&NSString::from_str(text(locale, Message::MenuPanelTitle)));
        self.panel_title
            .setStringValue(&NSString::from_str(text(locale, Message::MenuPanelTitle)));
        set_accessibility_label(&*self.panel_title, text(locale, Message::MenuPanelTitle));
        self.tabs[0].setTitle(&NSString::from_str(text(locale, Message::MenuCharacterTab)));
        self.tabs[1].setTitle(&NSString::from_str(text(locale, Message::MenuBubbleTab)));
        self.tabs[2].setTitle(&NSString::from_str(text(locale, Message::MenuSettingsTab)));
        set_accessibility_label(&*self.tabs[0], text(locale, Message::MenuCharacterTab));
        set_accessibility_label(&*self.tabs[1], text(locale, Message::MenuBubbleTab));
        set_accessibility_label(&*self.tabs[2], text(locale, Message::MenuSettingsTab));
        self.character_visible_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuCharacterVisible,
            )));
        self.character_manage_label
            .setTitle(&NSString::from_str(text(
                locale,
                Message::MenuManageCharacter,
            )));
        self.bubble_visible_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuBubbleVisible,
            )));
        self.status_indicators_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuStatusIndicators,
            )));
        self.bubble_placement_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuBubblePlacement,
            )));
        let placement_titles = [
            Message::MenuPlacementAuto,
            Message::MenuPlacementAbove,
            Message::MenuPlacementBelow,
            Message::MenuPlacementLeft,
            Message::MenuPlacementRight,
        ];
        for (button, message) in self.placement_buttons.iter().zip(placement_titles) {
            let title = text(locale, message);
            button.setTitle(&NSString::from_str(title));
            set_accessibility_label(&*button, title);
        }
        self.bubble_theme_label
            .setStringValue(&NSString::from_str(text(locale, Message::BubbleTheme)));
        localize_popup_items(
            &self.bubble_theme_popup,
            locale,
            &[
                (Message::ThemeWarmIvory, 0),
                (Message::ThemeDustyRose, 1),
                (Message::ThemeMoonlitInk, 2),
                (Message::ThemeCustom, 3),
            ],
        );
        set_accessibility_label(&self.bubble_theme_popup, text(locale, Message::BubbleTheme));
        self.bubble_colors_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::CustomizeBubbleColors,
            )));
        for ((name, field), message) in self
            .bubble_color_labels
            .iter()
            .zip(&self.bubble_color_fields)
            .zip([
                Message::BubbleSurfaceColor,
                Message::BubbleTextColor,
                Message::BubbleMutedColor,
                Message::BubbleBorderColor,
                Message::BubbleAccentColor,
            ])
        {
            name.setStringValue(&NSString::from_str(text(locale, message)));
            set_accessibility_label(field, text(locale, message));
        }
        self.bubble_apply.setTitle(&NSString::from_str(text(
            locale,
            Message::ApplyBubbleColors,
        )));
        self.bubble_reset.setTitle(&NSString::from_str(text(
            locale,
            Message::ResetBubbleColors,
        )));
        set_accessibility_label(&self.bubble_apply, text(locale, Message::ApplyBubbleColors));
        set_accessibility_label(&self.bubble_reset, text(locale, Message::ResetBubbleColors));
        self.settings_appearance_label
            .setStringValue(&NSString::from_str(text(locale, Message::MenuAppearance)));
        self.click_behavior_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuClickBehavior,
            )));
        self.full_passthrough_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuFullPassthrough,
            )));
        self.full_passthrough_help
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuFullPassthroughHelp,
            )));
        self.alpha_passthrough_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuAlphaPassthrough,
            )));
        self.alpha_passthrough_help
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuAlphaPassthroughHelp,
            )));
        self.scale_label
            .setStringValue(&NSString::from_str(text(locale, Message::MenuScale)));
        self.language_label
            .setStringValue(&NSString::from_str(text(locale, Message::MenuLanguage)));
        self.lifecycle_card.set_locale(locale);
        localize_popup_items(
            &self.language_popup,
            locale,
            &[
                (Message::LanguageSystem, 0),
                (Message::KoreanLanguage, 1),
                (Message::EnglishLanguage, 2),
            ],
        );
        set_accessibility_label(&self.language_popup, text(locale, Message::MenuLanguage));
        set_tooltip(
            &self.language_popup,
            text(locale, Message::FollowSystemSettings),
        );
        self.language_popup
            .menu()
            .expect("language popup menu")
            .itemWithTag(0)
            .expect("system language item")
            .setToolTip(Some(&NSString::from_str(text(
                locale,
                Message::FollowSystemSettings,
            ))));
        self.locale = locale;
        for (field, message) in [
            (&self.dialogue_title, Message::DialogueEditor),
            (&self.dialogue_language_label, Message::DialogueLanguage),
            (&self.dialogue_slot_label, Message::DialogueSlot),
            (&self.dialogue_original_label, Message::DialogueOriginal),
            (&self.dialogue_input_label, Message::DialogueText),
        ] {
            field.setStringValue(&NSString::from_str(text(locale, message)));
            set_accessibility_label(field, text(locale, message));
        }
        localize_popup_items(
            &self.dialogue_language,
            locale,
            &[(Message::KoreanLanguage, 0), (Message::EnglishLanguage, 1)],
        );
        localize_popup_items(
            &self.dialogue_slot,
            locale,
            &dialogue_slot_messages().map(|(message, index)| (message, index as isize)),
        );
        set_accessibility_label(
            &self.dialogue_language,
            text(locale, Message::DialogueLanguage),
        );
        set_accessibility_label(&self.dialogue_slot, text(locale, Message::DialogueSlot));
        set_accessibility_label(&*self.dialogue_text, text(locale, Message::DialogueText));
        for (button, message) in [
            (&self.dialogue_save, Message::DialogueSave),
            (&self.dialogue_reset_entry, Message::DialogueResetEntry),
            (
                &self.dialogue_reset_character,
                Message::DialogueResetCharacter,
            ),
        ] {
            button.setTitle(&NSString::from_str(text(locale, message)));
            set_accessibility_label(button, text(locale, message));
        }
        if let Some(feedback) = self.dialogue_text.ivars().borrow_mut().as_mut() {
            feedback.locale = locale;
        }
        self.dialogue_text.update_feedback();
        self.update_dialogue_reference();
        self.observation_catalog = None;
        self.observation_title
            .setStringValue(&NSString::from_str(text(locale, Message::ObservationTitle)));
        self.observation_local_label
            .setStringValue(&NSString::from_str(text(locale, Message::ObservationLocal)));
        self.observation_remote_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::ObservationRemote,
            )));
        self.observation_help
            .setStringValue(&NSString::from_str(text(locale, Message::ObservationHelp)));
        set_accessibility_label(
            &*self.observation_local_switch,
            text(locale, Message::ObservationLocal),
        );
        set_accessibility_label(
            &*self.observation_remote_switch,
            text(locale, Message::ObservationRemote),
        );
        self.reset
            .setTitle(&NSString::from_str(text(locale, Message::ResetPosition)));
        self.quit
            .setTitle(&NSString::from_str(text(locale, Message::MenuQuit)));
        set_accessibility_label(&*self.scale_down, text(locale, Message::ScaleDown));
        set_accessibility_label(&*self.scale_up, text(locale, Message::ScaleUp));
        set_accessibility_label(
            &*self.character_visible_switch,
            text(locale, Message::MenuCharacterVisible),
        );
        set_accessibility_label(
            &*self.bubble_visible_switch,
            text(locale, Message::MenuBubbleVisible),
        );
        set_accessibility_label(
            &*self.status_indicators_switch,
            text(locale, Message::MenuStatusIndicators),
        );
        set_accessibility_label(
            &*self.full_passthrough_switch,
            text(locale, Message::MenuFullPassthrough),
        );
        set_accessibility_label(
            &*self.alpha_passthrough_switch,
            text(locale, Message::MenuAlphaPassthrough),
        );
        set_accessibility_label(&*self.reset, text(locale, Message::ResetPosition));
        set_accessibility_label(&*self.quit, text(locale, Message::MenuQuit));
        self.layout_root();
    }

    pub(crate) fn reanchor(&self, anchor: &NSView) {
        let Some(window) = anchor.window() else {
            return;
        };
        let Some(screen) = window.screen().or_else(|| NSScreen::mainScreen(self.mtm)) else {
            return;
        };
        let visible = screen.visibleFrame();
        let height = PANEL_HEIGHT.min((visible.size.height - 16.0).max(1.0));
        let width = PANEL_WIDTH.min((visible.size.width - 16.0).max(1.0));
        self.panel.setContentSize(NSSize::new(width, height));
        self.root.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        self.layout_root();

        let anchor_window_rect = anchor.convertRect_toView(anchor.bounds(), None);
        let anchor_screen_rect = window.convertRectToScreen(anchor_window_rect);
        let panel_size = self.panel.frame().size;
        let mut x =
            anchor_screen_rect.origin.x + (anchor_screen_rect.size.width - panel_size.width) * 0.5;
        let below = anchor_screen_rect.origin.y - panel_size.height - 6.0;
        let above = anchor_screen_rect.origin.y + anchor_screen_rect.size.height + 6.0;
        let y = if below >= visible.origin.y {
            below
        } else {
            above
        };
        x = x.clamp(
            visible.origin.x,
            (visible.origin.x + visible.size.width - panel_size.width).max(visible.origin.x),
        );
        let y = y.clamp(
            visible.origin.y,
            (visible.origin.y + visible.size.height - panel_size.height).max(visible.origin.y),
        );
        self.panel.setFrameOrigin(NSPoint::new(x, y));
    }

    pub(crate) fn shutdown(&self) {
        self.hide();
        self.panel.close();
    }

    pub(crate) fn window(&self) -> &NSPanel {
        &self.panel
    }
    pub(crate) fn lifecycle_value(&self, key: crate::lifecycle::LifecycleSetting) -> bool {
        self.lifecycle_card.value(key)
    }

    pub(crate) fn lifecycle_saved(&mut self, settings: LifecycleSettings) {
        self.lifecycle_card.saved(settings);
    }

    pub(crate) fn lifecycle_failed(&mut self, settings: LifecycleSettings, detail: String) {
        self.lifecycle_card.failed(settings, detail);
    }

    fn apply_tab_selection(&self) {
        for (index, button) in self.tabs.iter().enumerate() {
            let selected = index == self.selected_tab;
            button.setState(if selected {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            let selected_color = if selected { primary() } else { secondary() };
            button.setContentTintColor(Some(&selected_color));
            let font = if selected {
                NSFont::boldSystemFontOfSize(11.5)
            } else {
                NSFont::systemFontOfSize(11.0)
            };
            button.setFont(Some(&font));
            set_accessibility_selected(&*button, selected);
        }
        self.update_tab_indicator();
    }

    fn update_tab_indicator(&self) {
        let bounds = self.root.bounds();
        let width = bounds.size.width.max(1.0);
        let track_x = 12.0;
        let track_width = (width - 24.0).max(1.0);
        let tab_width = (track_width / 3.0).max(1.0);
        let indicator_x = track_x + 2.0 + tab_width * self.selected_tab as f64;
        self.tab_indicator.setFrame(NSRect::new(
            NSPoint::new(indicator_x, 50.0),
            NSSize::new((tab_width - 4.0).max(1.0), 26.0),
        ));
    }

    fn update_color_swatches(&self) {
        for (swatch, field) in self
            .bubble_color_swatches
            .iter()
            .zip(&self.bubble_color_fields)
        {
            let val = field.stringValue().to_string();
            if let Ok(color_val) = BubbleColor::parse_hex(&val) {
                let (r, g, b) = color_val.rgb();
                let ns_color = color(r, g, b, 1.0);
                swatch.setFillColor(&ns_color);
            }
        }
    }

    fn layout_root(&self) {
        let bounds = self.root.bounds();
        let width = bounds.size.width.max(1.0);
        let height = bounds.size.height.max(1.0);
        self.title.setFrame(NSRect::new(
            NSPoint::new(16.0, 10.0),
            NSSize::new((width - 32.0).max(1.0), 20.0),
        ));
        self.panel_title.setFrame(NSRect::new(
            NSPoint::new(16.0, 28.0),
            NSSize::new((width - 32.0).max(1.0), 16.0),
        ));

        let track_x = 12.0;
        let track_width = (width - 24.0).max(1.0);
        self.tab_track.setFrame(NSRect::new(
            NSPoint::new(track_x, 48.0),
            NSSize::new(track_width, 30.0),
        ));

        let tab_width = (track_width / 3.0).max(1.0);
        for (index, button) in self.tabs.iter().enumerate() {
            button.setFrame(NSRect::new(
                NSPoint::new(track_x + tab_width * index as f64, 48.0),
                NSSize::new(tab_width, 30.0),
            ));
        }
        self.update_tab_indicator();

        let scroll_height = (height - SCROLL_TOP - FOOTER_HEIGHT).max(1.0);
        self.scroll.setFrame(NSRect::new(
            NSPoint::new(PANEL_EDGE_INSET, SCROLL_TOP),
            NSSize::new((width - PANEL_EDGE_INSET * 2.0).max(1.0), scroll_height),
        ));
        self.layout_documents();

        let footer_y = (height - FOOTER_HEIGHT + 10.0)
            .min((height - FOOTER_HEIGHT).max(SCROLL_TOP + 8.0))
            .max(SCROLL_TOP + 8.0);
        let button_h = 28.0;
        let quit_w = 48.0;
        let reset_w = 90.0;
        let right_margin = 14.0;
        let quit_x = width - right_margin - quit_w;
        let reset_x = quit_x - 8.0 - reset_w;
        let status_w = (reset_x - 16.0 - 8.0).max(1.0);

        self.status.setFrame(NSRect::new(
            NSPoint::new(16.0, footer_y),
            NSSize::new(status_w, 32.0),
        ));
        self.reset.setFrame(NSRect::new(
            NSPoint::new(reset_x, footer_y + 2.0),
            NSSize::new(reset_w, button_h),
        ));
        self.quit.setFrame(NSRect::new(
            NSPoint::new(quit_x, footer_y + 2.0),
            NSSize::new(quit_w, button_h),
        ));
    }

    fn layout_documents(&self) {
        let scroll_frame = self.scroll.frame();
        let scroll_width = scroll_frame.size.width.max(1.0);
        let scroll_height = scroll_frame.size.height.max(1.0);

        let char_view_height = DIALOGUE_CHOOSER_HEIGHT;
        let char_content_height =
            (58.0 + DIALOGUE_CARD_HEIGHT + 10.0 + char_view_height + 12.0).max(scroll_height);

        let content_height = match self.selected_tab {
            0 => char_content_height,
            1 => BUBBLE_CONTENT_HEIGHT,
            _ => self.settings_height().max(scroll_height),
        };

        self.document.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_width, content_height),
        ));
        self.character_tab.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_width, char_content_height),
        ));
        self.bubble_tab.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_width, BUBBLE_CONTENT_HEIGHT),
        ));
        self.settings_tab.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_width, self.settings_height().max(scroll_height)),
        ));

        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();

        let clip_width = self
            .scroll
            .contentView()
            .bounds()
            .size
            .width
            .min(scroll_width)
            .max(1.0);
        let visible_width = self
            .scroll
            .documentVisibleRect()
            .size
            .width
            .min(clip_width)
            .max(1.0);

        let is_legacy = visible_width < scroll_width - 1.0;
        let scroller_allowance = if is_legacy { 0.0 } else { 14.0 };
        let outer_margin = 8.0;
        let card_width = (visible_width - outer_margin * 2.0 - scroller_allowance).max(1.0);

        self.layout_character(card_width, clip_width, char_view_height);
        self.layout_bubble(card_width);
        self.layout_settings(card_width);
    }

    fn layout_character(&self, card_width: f64, clip_width: f64, char_view_height: f64) {
        let cards = self.character_tab.subviews();
        let character_card = cards.objectAtIndex(0);
        character_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 6.0),
            NSSize::new(card_width, 46.0),
        ));
        self.character_visible_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 11.0),
            NSSize::new((card_width - 80.0).max(1.0), 24.0),
        ));
        self.character_visible_switch.setFrame(NSRect::new(
            NSPoint::new(card_width - 58.0, 8.0),
            NSSize::new(46.0, 30.0),
        ));
        self.character_manage_label.setHidden(true);
        self.character_manage_label
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)));
        self.dialogue_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 58.0),
            NSSize::new(card_width, DIALOGUE_CARD_HEIGHT),
        ));
        let field_width = (card_width - 28.0).max(1.0);
        for (field, y, height) in [
            (&self.dialogue_title, 9.0, 19.0),
            (&self.dialogue_name, 29.0, 17.0),
            (&self.dialogue_language_label, 51.0, 16.0),
            (&self.dialogue_slot_label, 100.0, 16.0),
            (&self.dialogue_original_label, 149.0, 16.0),
            (&self.dialogue_original, 167.0, 38.0),
            (&self.dialogue_input_label, 209.0, 16.0),
            (&self.dialogue_count, 296.0, 16.0),
            (&self.dialogue_error, 315.0, 32.0),
        ] {
            field.setFrame(NSRect::new(
                NSPoint::new(14.0, y),
                NSSize::new(field_width, height),
            ));
        }
        self.dialogue_language.setFrame(NSRect::new(
            NSPoint::new(14.0, 69.0),
            NSSize::new(field_width, 26.0),
        ));
        self.dialogue_slot.setFrame(NSRect::new(
            NSPoint::new(14.0, 118.0),
            NSSize::new(field_width, 26.0),
        ));
        self.dialogue_scroll.setFrame(NSRect::new(
            NSPoint::new(14.0, 228.0),
            NSSize::new(field_width, 66.0),
        ));
        self.dialogue_text.setMinSize(NSSize::new(0.0, 66.0));
        self.dialogue_text
            .setMaxSize(NSSize::new(field_width, 10_000_000.0));
        self.dialogue_text.setFrameSize(NSSize::new(
            field_width,
            self.dialogue_text.frame().size.height.max(66.0),
        ));
        let half = ((field_width - 8.0) / 2.0).max(1.0);
        self.dialogue_save.setFrame(NSRect::new(
            NSPoint::new(14.0, 353.0),
            NSSize::new(half, 26.0),
        ));
        self.dialogue_reset_entry.setFrame(NSRect::new(
            NSPoint::new(22.0 + half, 353.0),
            NSSize::new(half, 26.0),
        ));
        self.dialogue_reset_character.setFrame(NSRect::new(
            NSPoint::new(14.0, 383.0),
            NSSize::new(field_width, 26.0),
        ));
        self.character_view.setFrame(NSRect::new(
            NSPoint::new(0.0, 58.0 + DIALOGUE_CARD_HEIGHT + 10.0),
            NSSize::new(clip_width, char_view_height),
        ));
    }

    fn layout_bubble(&self, card_width: f64) {
        let cards = self.bubble_tab.subviews();
        let visibility_card = cards.objectAtIndex(0);
        let status_indicators_card = cards.objectAtIndex(1);
        let placement_card = cards.objectAtIndex(2);
        let theme_card = cards.objectAtIndex(3);

        visibility_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 8.0),
            NSSize::new(card_width, 48.0),
        ));
        self.bubble_visible_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 11.0),
            NSSize::new((card_width - 80.0).max(1.0), 24.0),
        ));
        self.bubble_visible_switch.setFrame(NSRect::new(
            NSPoint::new(card_width - 58.0, 8.0),
            NSSize::new(46.0, 30.0),
        ));

        status_indicators_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 64.0),
            NSSize::new(card_width, 48.0),
        ));
        self.status_indicators_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 11.0),
            NSSize::new((card_width - 80.0).max(1.0), 24.0),
        ));
        self.status_indicators_switch.setFrame(NSRect::new(
            NSPoint::new(card_width - 58.0, 8.0),
            NSSize::new(46.0, 30.0),
        ));

        // Placement card (3x3 Cross layout)
        placement_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 120.0),
            NSSize::new(card_width, 126.0),
        ));
        self.bubble_placement_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 8.0),
            NSSize::new((card_width - 28.0).max(1.0), 18.0),
        ));

        let btn_w = 52.0;
        let btn_h = 24.0;
        let col_gap = 6.0;
        let row_gap = 6.0;
        let grid_w = btn_w * 3.0 + col_gap * 2.0;
        let start_x = ((card_width - grid_w) / 2.0).max(14.0);
        let start_y = 30.0;

        // Row 0: Above (col 1)
        self.placement_buttons[1].setFrame(NSRect::new(
            NSPoint::new(start_x + btn_w + col_gap, start_y),
            NSSize::new(btn_w, btn_h),
        ));
        // Row 1: Left (col 0), Auto (col 1), Right (col 2)
        let row1_y = start_y + btn_h + row_gap;
        self.placement_buttons[3].setFrame(NSRect::new(
            NSPoint::new(start_x, row1_y),
            NSSize::new(btn_w, btn_h),
        ));
        self.placement_buttons[0].setFrame(NSRect::new(
            NSPoint::new(start_x + btn_w + col_gap, row1_y),
            NSSize::new(btn_w, btn_h),
        ));
        self.placement_buttons[4].setFrame(NSRect::new(
            NSPoint::new(start_x + (btn_w + col_gap) * 2.0, row1_y),
            NSSize::new(btn_w, btn_h),
        ));
        // Row 2: Below (col 1)
        let row2_y = row1_y + btn_h + row_gap;
        self.placement_buttons[2].setFrame(NSRect::new(
            NSPoint::new(start_x + btn_w + col_gap, row2_y),
            NSSize::new(btn_w, btn_h),
        ));

        // Theme card (native theme popup + 5 color rows + action buttons)
        theme_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 254.0),
            NSSize::new(card_width, 286.0),
        ));
        self.bubble_theme_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 8.0),
            NSSize::new((card_width - 28.0).max(1.0), 18.0),
        ));
        self.bubble_theme_popup.setFrame(NSRect::new(
            NSPoint::new(14.0, 30.0),
            NSSize::new((card_width - 28.0).max(1.0), 26.0),
        ));

        self.bubble_colors_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 60.0),
            NSSize::new((card_width - 28.0).max(1.0), 18.0),
        ));

        let field_width = 86.0;
        let swatch_size = 18.0;
        let swatch_x = card_width - 14.0 - field_width - 8.0 - swatch_size;
        let field_x = card_width - 14.0 - field_width;

        for (index, ((name, field), swatch)) in self
            .bubble_color_labels
            .iter()
            .zip(&self.bubble_color_fields)
            .zip(&self.bubble_color_swatches)
            .enumerate()
        {
            let y = 82.0 + index as f64 * 30.0;
            name.setFrame(NSRect::new(
                NSPoint::new(14.0, y + 2.0),
                NSSize::new((swatch_x - 20.0).max(1.0), 20.0),
            ));
            swatch.setFrame(NSRect::new(
                NSPoint::new(swatch_x, y + 2.0),
                NSSize::new(swatch_size, swatch_size),
            ));
            field.setFrame(NSRect::new(
                NSPoint::new(field_x, y),
                NSSize::new(field_width, 22.0),
            ));
        }

        let button_gap = 8.0;
        let button_width = ((card_width - 28.0 - button_gap) / 2.0).max(1.0);
        self.bubble_apply.setFrame(NSRect::new(
            NSPoint::new(14.0, 240.0),
            NSSize::new(button_width, 28.0),
        ));
        self.bubble_reset.setFrame(NSRect::new(
            NSPoint::new(14.0 + button_width + button_gap, 240.0),
            NSSize::new(button_width, 28.0),
        ));
    }

    fn settings_height(&self) -> f64 {
        SETTINGS_BASE_HEIGHT + 216.0 + self.machine_rows.len() as f64 * MACHINE_ROW_HEIGHT
    }

    fn layout_settings(&self, card_width: f64) {
        let cards = self.settings_tab.subviews();
        let appearance_card = cards.objectAtIndex(0);
        let scale_card = cards.objectAtIndex(1);
        let language_card = cards.objectAtIndex(2);

        appearance_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 8.0),
            NSSize::new(card_width, 146.0),
        ));
        self.settings_appearance_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 10.0),
            NSSize::new((card_width - 28.0).max(1.0), 20.0),
        ));
        self.click_behavior_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 30.0),
            NSSize::new((card_width - 28.0).max(1.0), 16.0),
        ));

        self.full_passthrough_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 52.0),
            NSSize::new((card_width - 80.0).max(1.0), 20.0),
        ));
        self.full_passthrough_switch.setFrame(NSRect::new(
            NSPoint::new(card_width - 58.0, 48.0),
            NSSize::new(46.0, 30.0),
        ));
        self.full_passthrough_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 72.0),
            NSSize::new((card_width - 28.0).max(1.0), 16.0),
        ));

        self.alpha_passthrough_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 94.0),
            NSSize::new((card_width - 80.0).max(1.0), 20.0),
        ));
        self.alpha_passthrough_switch.setFrame(NSRect::new(
            NSPoint::new(card_width - 58.0, 90.0),
            NSSize::new(46.0, 30.0),
        ));
        self.alpha_passthrough_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 114.0),
            NSSize::new((card_width - 28.0).max(1.0), 16.0),
        ));

        scale_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 162.0),
            NSSize::new(card_width, 54.0),
        ));
        self.scale_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 16.0),
            NSSize::new((card_width - 150.0).max(1.0), 22.0),
        ));
        self.scale_down.setFrame(NSRect::new(
            NSPoint::new(card_width - 132.0, 13.0),
            NSSize::new(28.0, 28.0),
        ));
        self.scale_readout.setFrame(NSRect::new(
            NSPoint::new(card_width - 98.0, 16.0),
            NSSize::new(56.0, 22.0),
        ));
        self.scale_up.setFrame(NSRect::new(
            NSPoint::new(card_width - 38.0, 13.0),
            NSSize::new(28.0, 28.0),
        ));

        language_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 224.0),
            NSSize::new(card_width, 68.0),
        ));
        self.language_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 8.0),
            NSSize::new((card_width - 28.0).max(1.0), 18.0),
        ));
        self.language_popup.setFrame(NSRect::new(
            NSPoint::new(14.0, 30.0),
            NSSize::new((card_width - 28.0).max(1.0), 26.0),
        ));
        let observation_height = 216.0 + self.machine_rows.len() as f64 * MACHINE_ROW_HEIGHT;
        self.observation_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 300.0),
            NSSize::new(card_width, observation_height),
        ));
        let label_width = (card_width - 80.0).max(1.0);
        self.observation_title.setFrame(NSRect::new(
            NSPoint::new(14.0, 10.0),
            NSSize::new(label_width, 20.0),
        ));
        for (label, toggle, y) in [
            (
                &self.observation_local_label,
                &self.observation_local_switch,
                34.0,
            ),
            (
                &self.observation_remote_label,
                &self.observation_remote_switch,
                72.0,
            ),
        ] {
            label.setFrame(NSRect::new(
                NSPoint::new(14.0, y),
                NSSize::new(label_width, 26.0),
            ));
            toggle.setFrame(NSRect::new(
                NSPoint::new(card_width - 58.0, y - 4.0),
                NSSize::new(46.0, 30.0),
            ));
        }
        self.observation_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 106.0),
            NSSize::new((card_width - 28.0).max(1.0), 36.0),
        ));
        self.observation_notice.setFrame(NSRect::new(
            NSPoint::new(14.0, 146.0),
            NSSize::new((card_width - 28.0).max(1.0), 62.0),
        ));
        for (index, (toggle, title, detail)) in self.machine_rows.iter().enumerate() {
            let y = 216.0 + index as f64 * MACHINE_ROW_HEIGHT;
            toggle.setFrame(NSRect::new(
                NSPoint::new(12.0, y + 12.0),
                NSSize::new(46.0, 30.0),
            ));
            title.setFrame(NSRect::new(
                NSPoint::new(64.0, y + 3.0),
                NSSize::new((card_width - 78.0).max(1.0), 20.0),
            ));
            detail.setFrame(NSRect::new(
                NSPoint::new(64.0, y + 24.0),
                NSSize::new((card_width - 78.0).max(1.0), 67.0),
            ));
        }
        self.lifecycle_card.layout(NSRect::new(
            NSPoint::new(
                8.0,
                530.0 + self.machine_rows.len() as f64 * MACHINE_ROW_HEIGHT,
            ),
            NSSize::new(card_width, CARD_HEIGHT),
        ));
    }
}

fn dialogue_slot_messages() -> [(Message, usize); 8] {
    [
        (Message::DialogueIdle, 0),
        (Message::DialogueRunning, 1),
        (Message::DialogueWaiting, 2),
        (Message::DialogueUnknown, 3),
        (Message::DialogueHeadTap, 4),
        (Message::DialogueBodyTap, 5),
        (Message::DialoguePet, 6),
        (Message::DialogueCompletion, 7),
    ]
}

fn configure_panel(panel: &NSPanel) {
    // SAFETY: the panel is not owned by a window controller and therefore
    // must not release itself when closed.
    unsafe { panel.setReleasedWhenClosed(false) };
    // A borderless NSPanel needs an explicit window accessibility identity;
    // the unexposed content view still passes through its native child controls.
    unsafe {
        let _: () = msg_send![panel, setAccessibilityElement: true];
        let _: () = msg_send![
            panel,
            setAccessibilityRole: Some(objc2_app_kit::NSAccessibilityWindowRole)
        ];
        let _: () = msg_send![
            panel,
            setAccessibilitySubrole: Some(objc2_app_kit::NSAccessibilityStandardWindowSubrole)
        ];
    }
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setWorksWhenModal(true);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setOpaque(false);
    let clear = NSColor::clearColor();
    panel.setBackgroundColor(Some(&clear));
    panel.setHasShadow(true);
    // Keep the menu ordered across transient focus changes; the app delegate
    // and explicit event monitors own dismissal.
    panel.setHidesOnDeactivate(false);
    panel.setAcceptsMouseMovedEvents(true);
}

fn make_tab(
    symbol: &str,
    title: &str,
    index: usize,
    target: &MenuTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    button.setButtonType(NSButtonType::Toggle);
    button.setBordered(false);
    button.setEnabled(true);
    button.setRefusesFirstResponder(false);
    button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    button.setImageHugsTitle(true);
    button.setImagePosition(NSCellImagePosition::ImageLeading);
    button.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    let symbol_name = NSString::from_str(symbol);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &symbol_name,
        Some(&NSString::from_str(title)),
    ) {
        button.setImage(Some(&image));
    }
    unsafe {
        let _: () = msg_send![&*button, setTag: index as isize];
    }
    bind_control(&button, target, sel!(selectMenuTab:));
    set_accessibility_label(&*button, title);
    // SAFETY: AppKit exports this immutable NSString constant for the process lifetime.
    set_accessibility_role(&*button, unsafe {
        objc2_app_kit::NSAccessibilityRadioButtonRole
    });
    // SAFETY: AppKit exports this immutable NSString constant for the process lifetime.
    set_accessibility_subrole(&*button, unsafe {
        objc2_app_kit::NSAccessibilityTabButtonSubrole
    });
    button
}
fn make_switch(
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSSwitch> {
    let control: Retained<NSSwitch> = unsafe {
        msg_send![
            NSSwitch::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(46.0, 30.0))
        ]
    };
    control.setControlSize(NSControlSize::Small);
    control.setEnabled(true);
    control.setRefusesFirstResponder(false);
    bind_control(&control, target, action);
    control
}

fn make_segment_button(
    title: &str,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    button.setButtonType(NSButtonType::PushOnPushOff);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    button.setContentTintColor(Some(&primary()));
    bind_control(&button, target, action);
    set_accessibility_label(&*button, title);
    button
}

fn make_selection_popup(
    items: &[(Message, isize)],
    locale: UiLocale,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let popup: Retained<NSPopUpButton> = unsafe {
        msg_send![
            NSPopUpButton::alloc(mtm),
            initWithFrame: NSRect::default(),
            pullsDown: false
        ]
    };
    popup.setControlSize(NSControlSize::Small);
    popup.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    popup.setContentTintColor(Some(&primary()));
    popup.setRefusesFirstResponder(false);
    for &(message, tag) in items {
        popup.addItemWithTitle(&NSString::from_str(text(locale, message)));
        popup.lastItem().expect("new popup item").setTag(tag);
    }
    bind_control(&popup, target, action);
    popup
}

fn localize_popup_items(popup: &NSPopUpButton, locale: UiLocale, items: &[(Message, isize)]) {
    let menu = popup.menu().expect("selection popup menu");
    for &(message, tag) in items {
        menu.itemWithTag(tag)
            .expect("selection popup item")
            .setTitle(&NSString::from_str(text(locale, message)));
    }
}
fn make_section_button(
    title: &str,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    button.setButtonType(NSButtonType::MomentaryPushIn);
    button.setBordered(false);
    button.setEnabled(true);
    button.setRefusesFirstResponder(false);
    button.setFont(Some(&NSFont::boldSystemFontOfSize(11.5)));
    button.setContentTintColor(Some(&secondary()));
    bind_control(&button, target, action);
    set_accessibility_label(&*button, title);
    button
}

fn make_action_button(
    title: &str,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    button.setButtonType(NSButtonType::MomentaryPushIn);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(true);
    button.setEnabled(true);
    button.setRefusesFirstResponder(false);
    button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    button.setContentTintColor(Some(&primary()));
    bind_control(&button, target, action);
    button
}

fn make_stepper_button(
    title: &str,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    button.setButtonType(NSButtonType::MomentaryPushIn);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(true);
    button.setEnabled(true);
    button.setRefusesFirstResponder(false);
    button.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    button.setContentTintColor(Some(&primary()));
    bind_control(&button, target, action);
    button
}

fn make_tab_indicator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let indicator = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 26.0)),
    );
    indicator.setBoxType(NSBoxType::Custom);
    indicator.setTransparent(false);
    indicator.setBorderWidth(1.0);
    indicator.setCornerRadius(6.0);
    let fill = color(CARD_RED, CARD_GREEN, CARD_BLUE, 0.98);
    let border = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.65);
    indicator.setFillColor(&fill);
    indicator.setBorderColor(&border);
    unsafe {
        let _: () = msg_send![&*indicator, setAccessibilityElement: false];
    }
    indicator
}

fn make_color_swatch(mtm: MainThreadMarker) -> Retained<NSBox> {
    let swatch = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(18.0, 18.0)),
    );
    swatch.setBoxType(NSBoxType::Custom);
    swatch.setTransparent(false);
    swatch.setBorderWidth(1.0);
    swatch.setCornerRadius(4.0);
    let border = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.55);
    swatch.setBorderColor(&border);
    let fill = color(CARD_RED, CARD_GREEN, CARD_BLUE, 1.0);
    swatch.setFillColor(&fill);
    unsafe {
        let _: () = msg_send![&*swatch, setAccessibilityElement: false];
    }
    swatch
}

fn bind_control(control: &NSControl, target: &MenuTarget, action: objc2::runtime::Sel) {
    unsafe {
        let _: () = msg_send![control, setTarget: Some(target)];
        let _: () = msg_send![control, setAction: Some(action)];
    }
}

fn set_action(control: &NSControl, target: &MenuTarget, action: objc2::runtime::Sel) {
    bind_control(control, target, action);
}

fn label(
    value: &str,
    size: f64,
    bold: bool,
    foreground: Retained<NSColor>,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    field.setAlignment(NSTextAlignment::Left);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field.setTextColor(Some(&foreground));
    field.setDrawsBackground(false);
    field.setBordered(false);
    field.setBezeled(false);
    field.setEditable(false);
    field.setSelectable(false);
    field.setUsesSingleLineMode(false);
    field.setMaximumNumberOfLines(2);
    field.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByWordWrapping);
    field
}

fn set_accessibility_label(view: &NSView, value: &str) {
    let value = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*value)];
    }
}

fn set_tooltip(view: &NSView, value: &str) {
    let value = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setToolTip: Some(&*value)];
    }
}

fn set_accessibility_element(view: &NSView, enabled: bool) {
    unsafe {
        let _: () = msg_send![view, setAccessibilityElement: enabled];
    }
}

fn set_accessibility_selected(view: &NSView, selected: bool) {
    unsafe {
        let _: () = msg_send![view, setAccessibilitySelected: selected];
    }
}

fn set_accessibility_role(view: &NSView, role: &NSString) {
    unsafe {
        let _: () = msg_send![view, setAccessibilityRole: Some(role)];
    }
}

fn set_accessibility_subrole(view: &NSView, subrole: &NSString) {
    unsafe {
        let _: () = msg_send![view, setAccessibilitySubrole: Some(subrole)];
    }
}

fn color(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha)
}

fn primary() -> Retained<NSColor> {
    color(PRIMARY_RED, PRIMARY_GREEN, PRIMARY_BLUE, 1.0)
}

fn secondary() -> Retained<NSColor> {
    color(SECONDARY_RED, SECONDARY_GREEN, SECONDARY_BLUE, 1.0)
}

fn inset_rect(rect: NSRect, inset: f64) -> NSRect {
    let inset = inset.max(0.0);
    NSRect::new(
        NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        NSSize::new(
            (rect.size.width - inset * 2.0).max(0.0),
            (rect.size.height - inset * 2.0).max(0.0),
        ),
    )
}

fn placement_index(placement: BubblePlacement) -> usize {
    match placement {
        BubblePlacement::Auto => 0,
        BubblePlacement::Above => 1,
        BubblePlacement::Below => 2,
        BubblePlacement::Left => 3,
        BubblePlacement::Right => 4,
    }
}
