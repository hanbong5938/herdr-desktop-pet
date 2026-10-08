use crate::bubble::BubblePlacement;
use crate::i18n::{text, LanguagePreference, Message, UiLocale};
use crate::lifecycle::LifecycleSettings;
use crate::lifecycle_settings_ui::{LifecycleSettingsCard, CARD_HEIGHT};
use crate::preferences::{BubbleAppearance, BubbleColor, BubblePalette, BubbleTheme, MenuBarMode};
use crate::sources::{MachineStatus, ObservationPreferences, SourceCatalog};
use crate::state::Scene;
use crate::ui::MenuTarget;
use crate::update_card::{AppUpdateCard, UpdateCardModel};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, MainThreadMarker, MainThreadOnly, Message as ObjcMessage,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezelStyle, NSBezierPath, NSBox,
    NSBoxType, NSButton, NSButtonType, NSCellImagePosition, NSColor, NSControl, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSEventModifierFlags,
    NSFloatingWindowLevel, NSFont, NSImage, NSImageScaling, NSImageView, NSPanel, NSPopUpButton,
    NSScrollElasticity, NSScrollView, NSScrollerStyle, NSSwitch, NSTextAlignment, NSTextField,
    NSTextView, NSUserInterfaceItemIdentification, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};

use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::Cell;
const PANEL_WIDTH: f64 = 352.0;
const PANEL_HEIGHT: f64 = 540.0;
const PANEL_EDGE_INSET: f64 = 10.0;
const PANEL_RADIUS: f64 = 16.0;
const CARD_RADIUS: f64 = 10.0;
const SCROLL_TOP: f64 = 86.0;
const FOOTER_HEIGHT: f64 = 50.0;
const CHARACTER_CONTENT_HEIGHT: f64 = 404.0;
const BUBBLE_CONTENT_HEIGHT: f64 = 578.0;
const SETTINGS_BASE_HEIGHT: f64 = 308.0 + CARD_HEIGHT + 14.0;
const MENU_BAR_CARD_TOP: f64 = 300.0;
const MENU_BAR_CARD_GAP: f64 = 8.0;
const MENU_BAR_CARD_BOTTOM: f64 = 14.0;
const MIN_SCALE: f64 = 0.35;
const MAX_SCALE: f64 = 1.25;

#[derive(Default)]
struct BubbleColorDraft {
    latest: Option<BubbleAppearance>,
    baseline: Option<BubbleAppearance>,
}

impl BubbleColorDraft {
    fn receive(&mut self, appearance: BubbleAppearance, protected: bool) {
        let already_conflicted = self.conflicted();
        self.latest = Some(appearance);
        if self.baseline.is_none() || (!protected && !already_conflicted) {
            self.baseline = Some(appearance);
        }
    }

    fn saved(&mut self, appearance: BubbleAppearance) {
        self.latest = Some(appearance);
        self.baseline = Some(appearance);
    }

    fn conflicted(&self) -> bool {
        self.latest.is_some() && self.latest != self.baseline
    }

    fn rebase(&mut self) -> bool {
        let Some(latest) = self.latest else {
            return false;
        };
        self.baseline = Some(latest);
        true
    }
}

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
    character_editing_open: Retained<NSButton>,
    character_browser_open: Retained<NSButton>,
    target: Retained<MenuTarget>,
    title: Retained<NSTextField>,
    panel_title: Retained<NSTextField>,
    character_visible_label: Retained<NSTextField>,
    character_visible_switch: Retained<NSSwitch>,
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
    bubble_reload: Retained<NSButton>,
    bubble_rebase: Retained<NSButton>,
    bubble_draft: BubbleColorDraft,
    bubble_control_state: Option<(bool, bool)>,
    bubble_conflict_label: Option<bool>,
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
    menu_bar_card: Retained<MenuPanelCard>,
    menu_bar_label: Retained<NSTextField>,
    menu_bar_preview_well: Retained<NSBox>,
    menu_bar_preview: Retained<NSImageView>,
    menu_bar_separator: Retained<NSBox>,
    menu_bar_icon_status: Retained<NSTextField>,
    menu_bar_visibility: Retained<NSTextField>,
    menu_bar_popup: Retained<NSPopUpButton>,
    menu_bar_help: Retained<NSTextField>,
    menu_bar_choose: Retained<NSButton>,
    menu_bar_restore: Retained<NSButton>,
    menu_bar_icon_help: Retained<NSTextField>,
    menu_bar_icon_error: Retained<NSTextField>,
    menu_bar_icon_error_detail: Option<(Message, String)>,
    menu_bar_icon_custom: bool,
    menu_bar_icon_busy: Cell<bool>,
    lifecycle_card: LifecycleSettingsCard,
    frozen_controls: Vec<(Retained<NSControl>, bool)>,
    update_card: AppUpdateCard,
    observation_card: Retained<MenuPanelCard>,
    observation_title: Retained<NSTextField>,
    observation_local_label: Retained<NSTextField>,
    observation_local_help: Retained<NSTextField>,
    observation_remote_label: Retained<NSTextField>,
    observation_local_switch: Retained<NSSwitch>,
    observation_remote_switch: Retained<NSSwitch>,
    observation_machines: Retained<NSTextField>,
    observation_registration: Retained<NSButton>,
    observation_notice: Retained<NSTextField>,
    observation_error_toggle: Retained<NSButton>,
    observation_error: Retained<NSTextField>,
    observation_error_expanded: bool,
    machine_rows: Vec<MachineRow>,
    observation_catalog: Option<SourceCatalog>,
    observation_preferences: Option<ObservationPreferences>,
    locale: UiLocale,
    status: Retained<NSTextField>,
    presentation_error: Option<String>,
    status_text: String,
    reset: Retained<NSButton>,
    quit: Retained<NSButton>,
    selected_tab: usize,
}

struct MachineRow {
    id: String,
    toggle: Retained<NSSwitch>,
    title: Retained<NSTextField>,
    session: Retained<NSTextField>,
    status: Retained<NSTextField>,
    error_toggle: Retained<NSButton>,
    error: Retained<NSTextField>,
    copy: Retained<NSButton>,
    error_expanded: bool,
    copy_feedback: Option<Message>,
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
        let character_browser_open = make_action_button(
            text(locale, Message::CharacterBrowserOpen),
            target,
            sel!(openCharacterBrowser:),
            mtm,
        );
        set_accessibility_identifier(&character_browser_open, "herdr.character.open-browser");
        set_accessibility_label(
            &character_browser_open,
            text(locale, Message::CharacterBrowserOpen),
        );
        character_browser_open.setFont(Some(&NSFont::systemFontOfSize(11.5)));
        character_browser_open.setImageHugsTitle(true);
        character_browser_open.setImagePosition(NSCellImagePosition::ImageLeading);
        let browser_symbol = NSString::from_str("square.grid.2x2");
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &browser_symbol,
            Some(&NSString::from_str(text(
                locale,
                Message::CharacterBrowserOpen,
            ))),
        ) {
            character_browser_open.setImage(Some(&image));
        }
        set_tooltip(
            &character_browser_open,
            text(locale, Message::CharacterBrowserOpen),
        );
        let character_editing_open = make_action_button(
            text(locale, Message::CharacterEditingOpen),
            target,
            sel!(openDialogueEditor:),
            mtm,
        );
        character_editing_open.setFont(Some(&NSFont::systemFontOfSize(11.5)));
        character_editing_open.setImageHugsTitle(true);
        character_editing_open.setImagePosition(NSCellImagePosition::ImageLeading);
        let symbol_name = NSString::from_str("bubble.left.and.text.bubble.right");
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &symbol_name,
            Some(&NSString::from_str(text(
                locale,
                Message::CharacterEditingOpen,
            ))),
        ) {
            character_editing_open.setImage(Some(&image));
        }
        set_tooltip(
            &character_editing_open,
            text(locale, Message::CharacterEditingOpen),
        );
        let character_view = character_view.retain();
        character_card.addSubview(&character_visible_label);
        character_card.addSubview(&*character_visible_switch);
        character_tab.addSubview(&character_card);
        character_tab.addSubview(&character_browser_open);
        character_tab.addSubview(&character_editing_open);
        character_tab.addSubview(&character_view);

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
        let bubble_reload = make_action_button(
            text(locale, Message::BubbleColorsReload),
            target,
            sel!(reloadBubbleColors:),
            mtm,
        );
        let bubble_rebase = make_action_button(
            text(locale, Message::BubbleColorsRebase),
            target,
            sel!(rebaseBubbleColors:),
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
        bubble_theme_card.addSubview(&bubble_reload);
        bubble_theme_card.addSubview(&bubble_rebase);
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
        let menu_bar_card = MenuPanelCard::new(NSRect::default(), mtm);
        set_accessibility_element(&*menu_bar_card, false);
        let menu_bar_label = label(
            text(locale, Message::MenuBarIcon),
            12.5,
            true,
            primary(),
            mtm,
        );
        let menu_bar_preview_well = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::default());
        menu_bar_preview_well.setBoxType(NSBoxType::Custom);
        menu_bar_preview_well.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
        menu_bar_preview_well.setContentViewMargins(NSSize::new(0.0, 0.0));
        menu_bar_preview_well.setTransparent(false);
        menu_bar_preview_well.setBorderWidth(1.0);
        menu_bar_preview_well.setCornerRadius(6.0);
        menu_bar_preview_well.setFillColor(&NSColor::controlBackgroundColor());
        menu_bar_preview_well.setBorderColor(&color(
            CARD_BORDER_RED,
            CARD_BORDER_GREEN,
            CARD_BORDER_BLUE,
            CARD_BORDER_ALPHA,
        ));
        set_accessibility_element(&*menu_bar_preview_well, false);
        let menu_bar_preview =
            NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::default());
        menu_bar_preview.setImageScaling(NSImageScaling::ScaleProportionallyDown);
        menu_bar_preview.setEditable(false);
        set_accessibility_element(&*menu_bar_preview, false);
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("pawprint.fill"),
            None,
        ) {
            image.setTemplate(true);
            menu_bar_preview.setImage(Some(&image));
        }
        menu_bar_preview_well.addSubview(&menu_bar_preview);
        let menu_bar_icon_status = label(
            text(locale, Message::MenuBarIconDefault),
            11.5,
            true,
            primary(),
            mtm,
        );
        set_accessibility_identifier(&menu_bar_icon_status, "menu-bar-icon-status");
        let menu_bar_visibility = label(
            text(locale, Message::MenuBarVisibility),
            11.0,
            true,
            secondary(),
            mtm,
        );
        set_accessibility_element(&*menu_bar_visibility, false);
        let menu_bar_popup = make_selection_popup(
            &[
                (Message::MenuBarAlways, 0),
                (Message::MenuBarRecoveryOnly, 1),
            ],
            locale,
            target,
            sel!(setMenuBarMode:),
            mtm,
        );
        set_accessibility_identifier(&menu_bar_popup, "menu-bar-visibility");
        let menu_bar_help = label(
            text(locale, Message::MenuBarModeHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        menu_bar_help.setMaximumNumberOfLines(0);
        let menu_bar_separator = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::default());
        menu_bar_separator.setBoxType(NSBoxType::Custom);
        menu_bar_separator.setBorderWidth(0.0);
        menu_bar_separator.setFillColor(&color(
            CARD_BORDER_RED,
            CARD_BORDER_GREEN,
            CARD_BORDER_BLUE,
            0.35,
        ));
        set_accessibility_element(&*menu_bar_separator, false);
        let menu_bar_choose = make_action_button(
            text(locale, Message::MenuBarIconChoose),
            target,
            sel!(chooseMenuBarIcon:),
            mtm,
        );
        set_accessibility_identifier(&menu_bar_choose, "menu-bar-icon-choose");
        let menu_bar_restore = make_action_button(
            text(locale, Message::MenuBarIconRestore),
            target,
            sel!(resetMenuBarIcon:),
            mtm,
        );
        set_accessibility_identifier(&menu_bar_restore, "menu-bar-icon-reset");
        menu_bar_restore.setEnabled(false);
        let menu_bar_icon_help = label(
            text(locale, Message::MenuBarIconHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        menu_bar_icon_help.setMaximumNumberOfLines(0);
        let menu_bar_icon_error = label("", 10.5, false, NSColor::systemRedColor(), mtm);
        menu_bar_icon_error.setMaximumNumberOfLines(0);
        menu_bar_icon_error.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByCharWrapping);
        set_accessibility_identifier(&menu_bar_icon_error, "menu-bar-icon-error");
        menu_bar_icon_error.setHidden(true);
        menu_bar_card.addSubview(&menu_bar_label);
        menu_bar_card.addSubview(&menu_bar_visibility);
        menu_bar_card.addSubview(&menu_bar_popup);
        menu_bar_card.addSubview(&menu_bar_help);
        menu_bar_card.addSubview(&menu_bar_separator);
        menu_bar_card.addSubview(&menu_bar_preview_well);
        menu_bar_card.addSubview(&menu_bar_icon_status);
        menu_bar_card.addSubview(&menu_bar_choose);
        menu_bar_card.addSubview(&menu_bar_restore);
        menu_bar_card.addSubview(&menu_bar_icon_help);
        menu_bar_card.addSubview(&menu_bar_icon_error);
        settings_tab.addSubview(&menu_bar_card);
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
        let observation_local_help = label(
            text(locale, Message::ObservationLocalHelp),
            10.0,
            false,
            secondary(),
            mtm,
        );
        observation_local_help.setMaximumNumberOfLines(0);
        let observation_remote_label = label(
            text(locale, Message::ObservationRemote),
            12.0,
            true,
            primary(),
            mtm,
        );
        observation_local_label.setMaximumNumberOfLines(0);
        observation_remote_label.setMaximumNumberOfLines(0);
        let observation_local_switch = make_switch(target, sel!(setObservationLocal:), mtm);
        let observation_remote_switch = make_switch(target, sel!(setObservationRemote:), mtm);
        let observation_machines = label(
            text(locale, Message::ObservationMachines),
            11.0,
            true,
            primary(),
            mtm,
        );
        observation_machines.setMaximumNumberOfLines(0);
        observation_machines.setHidden(true);
        let observation_registration = make_action_button(
            text(locale, Message::ObservationRegistration),
            target,
            sel!(showObservationHelp:),
            mtm,
        );
        let observation_notice = label("", 10.5, false, secondary(), mtm);
        observation_notice.setMaximumNumberOfLines(0);
        observation_notice.setHidden(true);
        let observation_error_toggle = make_action_button(
            text(locale, Message::ObservationDetails),
            target,
            sel!(toggleObservationCatalogError:),
            mtm,
        );
        observation_error_toggle.setHidden(true);
        let observation_error = label("", 10.0, false, secondary(), mtm);
        observation_error.setMaximumNumberOfLines(0);
        observation_error.setHidden(true);
        for view in [
            &*observation_title as &NSView,
            &*observation_local_label,
            &*observation_local_help,
            &*observation_remote_label,
            &*observation_local_switch,
            &*observation_remote_switch,
            &*observation_machines,
            &*observation_registration,
            &*observation_notice,
            &*observation_error_toggle,
            &*observation_error,
        ] {
            observation_card.addSubview(view);
        }
        settings_tab.addSubview(&observation_card);
        let lifecycle_card = LifecycleSettingsCard::new(locale, target, mtm);
        settings_tab.addSubview(lifecycle_card.view());
        let update_card = AppUpdateCard::new(locale, target, mtm);
        settings_tab.addSubview(update_card.view());

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
            character_editing_open,
            character_browser_open,
            target: target.retain(),
            title,
            panel_title,
            character_visible_label,
            character_visible_switch,
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
            bubble_reload,
            bubble_rebase,
            bubble_draft: BubbleColorDraft::default(),
            bubble_control_state: None,
            bubble_conflict_label: None,
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
            menu_bar_card,
            menu_bar_label,
            menu_bar_preview_well,
            menu_bar_preview,
            menu_bar_separator,
            menu_bar_icon_status,
            menu_bar_visibility,
            menu_bar_popup,
            menu_bar_help,
            menu_bar_choose,
            menu_bar_restore,
            menu_bar_icon_help,
            menu_bar_icon_error,
            menu_bar_icon_error_detail: None,
            menu_bar_icon_custom: false,
            menu_bar_icon_busy: Cell::new(false),
            lifecycle_card,
            frozen_controls: Vec::new(),
            update_card,
            observation_card,
            observation_title,
            observation_local_label,
            observation_local_help,
            observation_remote_label,
            observation_local_switch,
            observation_remote_switch,
            observation_machines,
            observation_registration,
            observation_notice,
            observation_error_toggle,
            observation_error,
            observation_error_expanded: false,
            machine_rows: Vec::new(),
            observation_catalog: None,
            observation_preferences: None,
            presentation_error: None,
            status_text: String::new(),
            locale,
            status,
            reset,
            quit,
            selected_tab: 0,
        };
        panel.set_menu_bar_mode(MenuBarMode::Always);
        panel.select_tab(0);
        panel.update_color_swatches();
        panel.set_locale(locale);
        panel.layout_root();
        panel
    }

    pub(crate) fn show_at(&self, anchor_screen_rect: NSRect, visible_frame: NSRect) {
        self.reanchor_at(anchor_screen_rect, visible_frame);
        if !self.panel.isVisible() {
            self.panel.makeKeyAndOrderFront(None);
            let first = &self.tabs[self.selected_tab];
            self.panel.makeFirstResponder(Some(&**first));
        } else {
            self.panel.makeKeyAndOrderFront(None);
        }
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

    fn bubble_field_editor(&self, index: usize) -> Option<&AnyObject> {
        unsafe { msg_send![&*self.bubble_color_fields[index], currentEditor] }
    }

    pub(crate) fn bubble_colors_marked(&self) -> bool {
        (0..self.bubble_color_fields.len()).any(|index| {
            self.bubble_field_editor(index)
                .is_some_and(|editor| unsafe { msg_send![editor, hasMarkedText] })
        })
    }

    fn bubble_field_dirty(&self, index: usize) -> bool {
        let Some(baseline) = self.bubble_draft.baseline else {
            return false;
        };
        let palette = baseline.palette();
        let saved = [
            palette.surface,
            palette.text,
            palette.muted,
            palette.border,
            palette.accent,
        ][index];
        BubbleColor::parse_hex(&self.bubble_color_fields[index].stringValue().to_string())
            .map_or(true, |value| value != saved)
    }

    pub(crate) fn refresh_bubble_color_controls(&mut self) {
        let conflicted = self.bubble_draft.conflicted();
        let marked = self.bubble_colors_marked();
        if self.bubble_control_state != Some((conflicted, marked)) {
            self.bubble_reload.setEnabled(conflicted && !marked);
            self.bubble_rebase.setEnabled(conflicted && !marked);
            self.bubble_apply.setEnabled(!conflicted && !marked);
            self.bubble_control_state = Some((conflicted, marked));
        }
        if self.bubble_conflict_label != Some(conflicted) {
            let label = if conflicted {
                text(self.locale, Message::BubbleColorDraftConflict)
            } else {
                text(self.locale, Message::CustomizeBubbleColors)
            };
            self.bubble_colors_label
                .setStringValue(&NSString::from_str(label));
            set_accessibility_label(&self.bubble_colors_label, label);
            self.bubble_conflict_label = Some(conflicted);
        }
    }

    pub(crate) fn set_bubble_appearance(&mut self, appearance: BubbleAppearance) {
        let protected: [bool; 5] = std::array::from_fn(|index| {
            self.bubble_field_editor(index).is_some() || self.bubble_field_dirty(index)
        });
        let editing =
            protected.iter().any(|protected| *protected) || self.bubble_draft.conflicted();
        self.bubble_draft.receive(appearance, editing);
        if !editing {
            let tag = match appearance.theme {
                BubbleTheme::WarmIvory => 0,
                BubbleTheme::DustyRose => 1,
                BubbleTheme::MoonlitInk => 2,
                BubbleTheme::Custom => 3,
            };
            self.bubble_theme_popup.selectItemWithTag(tag);
        }
        if !editing {
            let palette = appearance.palette();
            for (field, value) in self.bubble_color_fields.iter().zip([
                palette.surface,
                palette.text,
                palette.muted,
                palette.border,
                palette.accent,
            ]) {
                let hex = value.to_hex();
                if field.stringValue().to_string() != hex {
                    field.setStringValue(&NSString::from_str(&hex));
                }
            }
        }
        self.update_color_swatches();
        self.refresh_bubble_color_controls();
    }

    pub(crate) fn bubble_colors_conflicted(&self) -> bool {
        self.bubble_draft.conflicted()
    }

    pub(crate) fn bubble_colors_saved(&mut self, appearance: BubbleAppearance) {
        self.bubble_draft.saved(appearance);
        let tag = match appearance.theme {
            BubbleTheme::WarmIvory => 0,
            BubbleTheme::DustyRose => 1,
            BubbleTheme::MoonlitInk => 2,
            BubbleTheme::Custom => 3,
        };
        self.bubble_theme_popup.selectItemWithTag(tag);
        self.refresh_bubble_color_controls();
    }

    pub(crate) fn reload_bubble_colors(&mut self) -> bool {
        if self.bubble_colors_marked() {
            return false;
        }
        let Some(appearance) = self.bubble_draft.latest else {
            return false;
        };
        self.bubble_draft.rebase();
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
            let hex = value.to_hex();
            if field.stringValue().to_string() != hex {
                field.setStringValue(&NSString::from_str(&hex));
            }
        }
        self.update_color_swatches();
        self.refresh_bubble_color_controls();
        true
    }

    pub(crate) fn rebase_bubble_colors(&mut self) -> bool {
        if self.bubble_colors_marked() || !self.bubble_draft.rebase() {
            return false;
        }
        self.refresh_bubble_color_controls();
        true
    }

    pub(crate) fn set_show_status_indicators(&self, enabled: bool) {
        self.status_indicators_switch.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    pub(crate) fn custom_bubble_palette(&self) -> Result<BubblePalette, String> {
        if self.bubble_colors_conflicted() {
            return Err(text(self.locale, Message::BubbleColorDraftConflict).to_owned());
        }
        if self.bubble_colors_marked() {
            return Err(text(self.locale, Message::FinishMarkedText).to_owned());
        }
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
        menu_bar_mode: MenuBarMode,
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
        self.set_menu_bar_mode(menu_bar_mode);
        if self.status_text != status {
            self.status_text.clear();
            self.status_text.push_str(status);
        }
        self.update_status_text();
        self.lifecycle_card.sync(lifecycle);
        self.sync_observation(observation, catalog);
        self.update_color_swatches();
        self.layout_root();
        if !self.frozen_controls.is_empty() {
            self.set_update_frozen(true);
        }
    }

    pub(crate) fn set_language_preference(&self, language: LanguagePreference) {
        let tag = match language {
            LanguagePreference::System => 0,
            LanguagePreference::Ko => 1,
            LanguagePreference::En => 2,
        };
        self.language_popup.selectItemWithTag(tag);
    }

    pub(crate) fn set_menu_bar_mode(&self, mode: MenuBarMode) {
        self.menu_bar_popup.selectItemWithTag(match mode {
            MenuBarMode::Always => 0,
            MenuBarMode::RecoveryOnly => 1,
        });
    }

    pub(crate) fn set_presentation_error(&mut self, error: Option<&str>) {
        self.presentation_error = error.map(str::to_owned);
        let color = if error.is_some() {
            NSColor::systemRedColor()
        } else {
            secondary()
        };
        self.status.setTextColor(Some(&color));
        self.update_status_text();
    }

    fn update_status_text(&self) {
        let error = self.presentation_error.as_ref().map(|detail| {
            format!(
                "{}: {detail}",
                text(self.locale, Message::PresentationSaveFailure)
            )
        });
        let value = error.as_deref().unwrap_or(&self.status_text);
        self.status.setStringValue(&NSString::from_str(value));
        set_tooltip(&self.status, value);
    }

    pub(crate) fn set_menu_bar_icon(
        &mut self,
        image: &NSImage,
        custom: bool,
        error: Option<(Message, &str)>,
    ) {
        self.menu_bar_preview.setImage(Some(image));
        self.menu_bar_icon_custom = custom;
        self.menu_bar_icon_error_detail =
            error.map(|(message, detail)| (message, detail.to_owned()));
        self.update_menu_bar_icon_labels();
        self.menu_bar_restore
            .setEnabled(custom && !self.menu_bar_icon_busy.get());
        self.layout_documents();
    }

    pub(crate) fn set_menu_bar_icon_busy(&self, busy: bool) {
        // This setter is called on the main thread; keep the state together
        // with the controls so clearing busy never enables reset for Default.
        self.menu_bar_icon_busy.set(busy);
        self.menu_bar_choose.setEnabled(!busy);
        self.menu_bar_restore
            .setEnabled(self.menu_bar_icon_custom && !busy);
    }

    fn update_menu_bar_icon_labels(&self) {
        self.menu_bar_icon_status
            .setStringValue(&NSString::from_str(text(
                self.locale,
                if self.menu_bar_icon_custom {
                    Message::MenuBarIconCustom
                } else {
                    Message::MenuBarIconDefault
                },
            )));
        if let Some((summary, detail)) = &self.menu_bar_icon_error_detail {
            let value = if detail.is_empty() {
                text(self.locale, *summary).to_owned()
            } else {
                format!("{}: {detail}", text(self.locale, *summary))
            };
            self.menu_bar_icon_error
                .setStringValue(&NSString::from_str(&value));
            self.menu_bar_icon_error.setHidden(false);
        } else {
            self.menu_bar_icon_error
                .setStringValue(&NSString::from_str(""));
            self.menu_bar_icon_error.setHidden(true);
        }
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
        let mut old = std::mem::take(&mut self.machine_rows);
        for machine in &catalog.machines {
            let row = if let Some(index) = old.iter().position(|row| row.id == machine.id) {
                old.remove(index)
            } else {
                let toggle = make_switch(&self.target, sel!(setObservationMachine:), self.mtm);
                toggle.setIdentifier(Some(&NSString::from_str(&machine.id)));
                let title = label("", 11.5, true, primary(), self.mtm);
                let session = label("", 10.0, false, secondary(), self.mtm);
                let status = label("", 10.5, false, secondary(), self.mtm);
                title.setMaximumNumberOfLines(0);
                session.setMaximumNumberOfLines(0);
                status.setMaximumNumberOfLines(0);
                let error_toggle = make_action_button(
                    text(self.locale, Message::ObservationDetails),
                    &self.target,
                    sel!(toggleObservationMachineError:),
                    self.mtm,
                );
                error_toggle.setIdentifier(Some(&NSString::from_str(&machine.id)));
                let error = label("", 10.0, false, secondary(), self.mtm);
                error.setMaximumNumberOfLines(0);
                let copy = make_action_button(
                    text(self.locale, Message::ObservationCopyReconnect),
                    &self.target,
                    sel!(copyObservationReconnect:),
                    self.mtm,
                );
                copy.setIdentifier(Some(&NSString::from_str(&machine.id)));
                for view in [
                    &*toggle as &NSView,
                    &*title,
                    &*session,
                    &*status,
                    &*error_toggle,
                    &*error,
                    &*copy,
                ] {
                    self.observation_card.addSubview(view);
                }
                MachineRow {
                    id: machine.id.clone(),
                    toggle,
                    title,
                    session,
                    status,
                    error_toggle,
                    error,
                    copy,
                    error_expanded: false,
                    copy_feedback: None,
                }
            };
            row.toggle
                .setEnabled(machine.enabled || preferences.machines.contains(&machine.id));
            row.toggle
                .setState(if preferences.machines.contains(&machine.id) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            let name = format!(
                "{} {}",
                text(self.locale, Message::ObservationMachineObserve),
                machine.label
            );
            row.title
                .setStringValue(&NSString::from_str(&machine.label));
            set_tooltip(&row.title, &machine.label);
            set_accessibility_label(&row.title, &machine.label);
            let session = format!(
                "{}: {}",
                text(self.locale, Message::ObservationSession),
                machine.remote_session
            );
            row.session.setStringValue(&NSString::from_str(&session));
            set_tooltip(&row.session, &session);
            let state = if !machine.enabled {
                Message::ObservationDisabled
            } else if !preferences.remote {
                Message::ObservationMachinePaused
            } else if !preferences.machines.contains(&machine.id) {
                Message::ObservationNotSelected
            } else {
                match machine.status {
                    MachineStatus::Connecting => Message::ObservationConnecting,
                    MachineStatus::Online => Message::ObservationOnline,
                    MachineStatus::Offline => Message::ObservationOffline,
                    MachineStatus::NotSelected => Message::ObservationConnecting,
                }
            };
            let status = text(self.locale, state);
            row.status.setStringValue(&NSString::from_str(status));
            set_accessibility_label(&row.toggle, &format!("{name} · {session} · {status}"));
            set_tooltip(&row.toggle, &format!("{name} · {session} · {status}"));
            let error = machine.error.as_deref().unwrap_or("");
            row.error.setStringValue(&NSString::from_str(error));
            set_tooltip(&row.error, error);
            set_accessibility_label(&row.error, error);
            row.error_toggle.setHidden(error.is_empty());
            row.error.setHidden(error.is_empty() || !row.error_expanded);
            row.error_toggle.setTitle(&NSString::from_str(text(
                self.locale,
                if row.error_expanded {
                    Message::ObservationHideDetails
                } else {
                    Message::ObservationDetails
                },
            )));
            set_accessibility_label(
                &row.error_toggle,
                &format!(
                    "{} · {}",
                    name,
                    text(
                        self.locale,
                        if row.error_expanded {
                            Message::ObservationHideDetails
                        } else {
                            Message::ObservationDetails
                        },
                    )
                ),
            );
            row.copy.setHidden(
                !machine.enabled
                    || !preferences.remote
                    || !preferences.machines.contains(&machine.id)
                    || machine.status != MachineStatus::Offline,
            );
            row.copy.setTitle(&NSString::from_str(text(
                self.locale,
                row.copy_feedback
                    .unwrap_or(Message::ObservationCopyReconnect),
            )));
            set_accessibility_label(
                &row.copy,
                &format!(
                    "{} · {}",
                    name,
                    text(
                        self.locale,
                        row.copy_feedback
                            .unwrap_or(Message::ObservationCopyReconnect)
                    )
                ),
            );
            set_tooltip(
                &row.copy,
                &format!(
                    "herdr machine reconnect '{}'",
                    machine.id.replace('\'', "'\\''")
                ),
            );
            self.machine_rows.push(row);
        }
        for row in old {
            for view in [
                &*row.toggle as &NSView,
                &*row.title,
                &*row.session,
                &*row.status,
                &*row.error_toggle,
                &*row.error,
                &*row.copy,
            ] {
                view.removeFromSuperview();
            }
        }
        self.observation_machines
            .setHidden(self.machine_rows.is_empty());
        let notice = if catalog.error.is_some() {
            Some(Message::ObservationError)
        } else if !catalog.initialized {
            Some(Message::ObservationLoading)
        } else if catalog.machines.is_empty() {
            Some(Message::ObservationEmpty)
        } else if !preferences.local && !preferences.remote {
            Some(Message::ObservationNone)
        } else if !preferences.remote {
            Some(Message::ObservationPaused)
        } else if !catalog
            .machines
            .iter()
            .any(|machine| machine.enabled && preferences.machines.contains(&machine.id))
        {
            Some(Message::ObservationSelect)
        } else {
            None
        };
        let mut notice_text = notice
            .map(|message| text(self.locale, message))
            .unwrap_or("")
            .to_owned();
        if catalog.error.is_some() && !catalog.machines.is_empty() {
            notice_text.push_str(" · ");
            notice_text.push_str(text(self.locale, Message::ObservationPreviousResults));
        }
        self.observation_notice
            .setStringValue(&NSString::from_str(&notice_text));
        set_accessibility_label(&self.observation_notice, &notice_text);
        self.observation_notice.setHidden(notice_text.is_empty());
        let error = catalog.error.as_deref().unwrap_or("");
        self.observation_error
            .setStringValue(&NSString::from_str(error));
        set_tooltip(&self.observation_error, error);
        set_accessibility_label(&self.observation_error, error);
        self.observation_error_toggle.setHidden(error.is_empty());
        self.observation_error
            .setHidden(error.is_empty() || !self.observation_error_expanded);
        self.observation_error_toggle
            .setTitle(&NSString::from_str(text(
                self.locale,
                if self.observation_error_expanded {
                    Message::ObservationHideDetails
                } else {
                    Message::ObservationDetails
                },
            )));
        set_accessibility_label(
            &self.observation_error_toggle,
            &format!(
                "{} · {}",
                text(self.locale, Message::ObservationMachines),
                text(
                    self.locale,
                    if self.observation_error_expanded {
                        Message::ObservationHideDetails
                    } else {
                        Message::ObservationDetails
                    },
                )
            ),
        );
        self.observation_catalog = Some(catalog.clone());
        self.observation_preferences = Some(preferences.clone());
        self.layout_documents();
    }

    pub(crate) fn toggle_observation_error(&mut self, id: Option<&str>) {
        if let Some(id) = id {
            if let Some(row) = self.machine_rows.iter_mut().find(|row| row.id == id) {
                row.error_expanded = !row.error_expanded;
            }
        } else {
            self.observation_error_expanded = !self.observation_error_expanded;
        }
        if let (Some(preferences), Some(catalog)) = (
            self.observation_preferences.clone(),
            self.observation_catalog.clone(),
        ) {
            self.observation_catalog = None;
            self.sync_observation(&preferences, &catalog);
        }
    }

    pub(crate) fn observation_reconnect_command(&self, id: &str) -> Option<String> {
        let preferences = self.observation_preferences.as_ref()?;
        let machine = self
            .observation_catalog
            .as_ref()?
            .machines
            .iter()
            .find(|machine| machine.id == id)?;
        (preferences.remote
            && preferences
                .machines
                .iter()
                .any(|selected| selected.as_str() == id)
            && machine.enabled
            && machine.status == MachineStatus::Offline)
            .then(|| format!("herdr machine reconnect '{}'", id.replace('\'', "'\\''")))
    }

    pub(crate) fn observation_copy_feedback(&mut self, id: &str, success: bool) {
        if let Some(row) = self.machine_rows.iter_mut().find(|row| row.id == id) {
            let message = if success {
                Message::ObservationCommandCopied
            } else {
                Message::ObservationCopyFailed
            };
            row.copy_feedback = Some(message);
            row.copy
                .setTitle(&NSString::from_str(text(self.locale, message)));
            set_accessibility_label(
                &row.copy,
                &format!(
                    "{} · {}",
                    row.title.stringValue(),
                    text(self.locale, message)
                ),
            );
            self.layout_documents();
        }
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
        self.character_browser_open
            .setTitle(&NSString::from_str(text(
                locale,
                Message::CharacterBrowserOpen,
            )));
        set_accessibility_label(
            &self.character_browser_open,
            text(locale, Message::CharacterBrowserOpen),
        );
        set_tooltip(
            &self.character_browser_open,
            text(locale, Message::CharacterBrowserOpen),
        );
        let browser_symbol = NSString::from_str("square.grid.2x2");
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &browser_symbol,
            Some(&NSString::from_str(text(
                locale,
                Message::CharacterBrowserOpen,
            ))),
        ) {
            self.character_browser_open.setImage(Some(&image));
        }
        self.character_editing_open
            .setTitle(&NSString::from_str(text(
                locale,
                Message::CharacterEditingOpen,
            )));
        set_accessibility_label(
            &self.character_editing_open,
            text(locale, Message::CharacterEditingOpen),
        );
        set_tooltip(
            &self.character_editing_open,
            text(locale, Message::CharacterEditingOpen),
        );
        let symbol_name = NSString::from_str("bubble.left.and.text.bubble.right");
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &symbol_name,
            Some(&NSString::from_str(text(
                locale,
                Message::CharacterEditingOpen,
            ))),
        ) {
            self.character_editing_open.setImage(Some(&image));
        }
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
        for (button, message) in [
            (&self.bubble_reload, Message::BubbleColorsReload),
            (&self.bubble_rebase, Message::BubbleColorsRebase),
        ] {
            button.setTitle(&NSString::from_str(text(locale, message)));
            set_accessibility_label(button, text(locale, message));
        }
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
        self.update_card.set_locale(locale);
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
        self.menu_bar_label
            .setStringValue(&NSString::from_str(text(locale, Message::MenuBarIcon)));
        set_accessibility_label(&*self.menu_bar_label, text(locale, Message::MenuBarIcon));
        self.menu_bar_visibility
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::MenuBarVisibility,
            )));
        self.menu_bar_help
            .setStringValue(&NSString::from_str(text(locale, Message::MenuBarModeHelp)));
        self.menu_bar_icon_help
            .setStringValue(&NSString::from_str(text(locale, Message::MenuBarIconHelp)));
        self.menu_bar_choose.setTitle(&NSString::from_str(text(
            locale,
            Message::MenuBarIconChoose,
        )));
        self.menu_bar_restore.setTitle(&NSString::from_str(text(
            locale,
            Message::MenuBarIconRestore,
        )));
        set_accessibility_label(
            &*self.menu_bar_choose,
            text(locale, Message::MenuBarIconChoose),
        );
        set_accessibility_label(
            &*self.menu_bar_restore,
            text(locale, Message::MenuBarIconRestore),
        );
        localize_popup_items(
            &self.menu_bar_popup,
            locale,
            &[
                (Message::MenuBarAlways, 0),
                (Message::MenuBarRecoveryOnly, 1),
            ],
        );
        set_accessibility_label(
            &self.menu_bar_popup,
            text(locale, Message::MenuBarVisibility),
        );
        set_accessibility_label(&*self.menu_bar_help, text(locale, Message::MenuBarModeHelp));
        set_tooltip(&self.menu_bar_popup, text(locale, Message::MenuBarModeHelp));
        for tag in [0, 1] {
            self.menu_bar_popup
                .menu()
                .expect("menu bar mode popup menu")
                .itemWithTag(tag)
                .expect("menu bar mode item")
                .setToolTip(Some(&NSString::from_str(text(
                    locale,
                    Message::MenuBarModeHelp,
                ))));
        }
        self.locale = locale;
        self.bubble_conflict_label = None;
        self.refresh_bubble_color_controls();
        self.update_menu_bar_icon_labels();
        self.observation_catalog = None;
        self.observation_title
            .setStringValue(&NSString::from_str(text(locale, Message::ObservationTitle)));
        self.observation_local_label
            .setStringValue(&NSString::from_str(text(locale, Message::ObservationLocal)));
        self.observation_local_help
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::ObservationLocalHelp,
            )));
        self.observation_remote_label
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::ObservationRemote,
            )));
        self.observation_machines
            .setStringValue(&NSString::from_str(text(
                locale,
                Message::ObservationMachines,
            )));
        self.observation_registration
            .setTitle(&NSString::from_str(text(
                locale,
                Message::ObservationRegistration,
            )));
        set_accessibility_label(
            &self.observation_registration,
            text(locale, Message::ObservationRegistration),
        );
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

    pub(crate) fn reanchor_at(&self, anchor_screen_rect: NSRect, visible_frame: NSRect) {
        let height = PANEL_HEIGHT.min((visible_frame.size.height - 16.0).max(1.0));
        let width = PANEL_WIDTH.min((visible_frame.size.width - 16.0).max(1.0));
        self.panel.setContentSize(NSSize::new(width, height));
        self.root.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        self.layout_root();

        let panel_size = self.panel.frame().size;
        let mut x =
            anchor_screen_rect.origin.x + (anchor_screen_rect.size.width - panel_size.width) * 0.5;
        let below = anchor_screen_rect.origin.y - panel_size.height - 6.0;
        let above = anchor_screen_rect.origin.y + anchor_screen_rect.size.height + 6.0;
        let y = if below >= visible_frame.origin.y {
            below
        } else {
            above
        };
        x = x.clamp(
            visible_frame.origin.x,
            (visible_frame.origin.x + visible_frame.size.width - panel_size.width)
                .max(visible_frame.origin.x),
        );
        let y = y.clamp(
            visible_frame.origin.y,
            (visible_frame.origin.y + visible_frame.size.height - panel_size.height)
                .max(visible_frame.origin.y),
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
    pub(crate) fn refresh_app_update(&mut self, model: &UpdateCardModel) {
        self.update_card.refresh(model);
        self.layout_root();
        if !self.frozen_controls.is_empty() {
            self.set_update_frozen(true);
        }
    }
    pub(crate) fn set_update_frozen(&mut self, frozen: bool) {
        for field in &self.bubble_color_fields {
            field.setEditable(!frozen);
        }
        for index in 0..self.bubble_color_fields.len() {
            if let Some(editor) = self
                .bubble_field_editor(index)
                .and_then(|editor| editor.downcast_ref::<NSTextView>())
            {
                editor.setEditable(!frozen);
            }
        }
        if frozen {
            if !self.frozen_controls.is_empty() {
                for (control, _) in &self.frozen_controls {
                    control.setEnabled(false);
                }
                return;
            }
            fn disable(
                view: &NSView,
                quit: &NSButton,
                controls: &mut Vec<(Retained<NSControl>, bool)>,
            ) {
                for child in view.subviews().iter() {
                    if let Some(control) = child.downcast_ref::<NSControl>() {
                        if !std::ptr::eq(&*child, quit as &NSView) {
                            controls.push((control.retain(), control.isEnabled()));
                            control.setEnabled(false);
                        }
                    }
                    disable(&child, quit, controls);
                }
            }
            disable(&self.root, &self.quit, &mut self.frozen_controls);
        } else {
            for (control, enabled) in self.frozen_controls.drain(..) {
                control.setEnabled(enabled);
            }
        }
    }

    pub(crate) fn bubble_restart_draft(&self, saved: BubbleAppearance) -> bool {
        if self.bubble_draft.conflicted()
            || self.bubble_colors_marked()
            || self.bubble_draft.baseline != Some(saved)
        {
            return true;
        }
        let expected_tag = match saved.theme {
            BubbleTheme::WarmIvory => 0,
            BubbleTheme::DustyRose => 1,
            BubbleTheme::MoonlitInk => 2,
            BubbleTheme::Custom => 3,
        };
        if self
            .bubble_theme_popup
            .selectedItem()
            .map(|item| item.tag())
            != Some(expected_tag)
        {
            return true;
        }
        let palette = saved.palette();
        let colors = [
            palette.surface,
            palette.text,
            palette.muted,
            palette.border,
            palette.accent,
        ];
        self.bubble_color_fields
            .iter()
            .zip(colors)
            .any(|(field, color)| {
                let raw = field.stringValue().to_string();
                BubbleColor::parse_hex(&raw).map_or(true, |actual| actual != color)
            })
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

        let char_view_height = (scroll_height - 106.0).max(250.0);
        let char_content_height = (106.0 + char_view_height).max(scroll_height);

        let old_offset = self.scroll.contentView().bounds().origin.y;
        let initial_width = (scroll_width - 30.0).max(1.0);
        let mut settings_height = self.settings_height(initial_width).max(scroll_height);
        let content_height = match self.selected_tab {
            0 => char_content_height,
            1 => BUBBLE_CONTENT_HEIGHT,
            _ => settings_height,
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
            NSSize::new(scroll_width, settings_height),
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

        settings_height = self.settings_height(card_width).max(scroll_height);
        if self.selected_tab == 2 {
            self.document.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(scroll_width, settings_height),
            ));
        }
        self.settings_tab.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_width, settings_height),
        ));
        self.layout_character(card_width, clip_width, char_view_height);
        self.layout_bubble(card_width);
        self.layout_settings(card_width);
        let max_offset = (self.document.frame().size.height
            - self.scroll.contentView().bounds().size.height)
            .max(0.0);
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, old_offset.min(max_offset)));
        self.scroll
            .reflectScrolledClipView(&self.scroll.contentView());
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
        let gap = 8.0;
        let btn_width = ((card_width - gap) * 0.5).max(1.0);
        self.character_browser_open.setFrame(NSRect::new(
            NSPoint::new(8.0, 58.0),
            NSSize::new(btn_width, 36.0),
        ));
        self.character_editing_open.setFrame(NSRect::new(
            NSPoint::new(8.0 + btn_width + gap, 58.0),
            NSSize::new((card_width - btn_width - gap).max(1.0), 36.0),
        ));
        self.character_view.setFrame(NSRect::new(
            NSPoint::new(0.0, 106.0),
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
            NSSize::new(card_width, 318.0),
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
        self.bubble_reload.setFrame(NSRect::new(
            NSPoint::new(14.0, 272.0),
            NSSize::new(button_width, 28.0),
        ));
        self.bubble_rebase.setFrame(NSRect::new(
            NSPoint::new(14.0 + button_width + button_gap, 272.0),
            NSSize::new(button_width, 28.0),
        ));
    }

    fn settings_height(&self, card_width: f64) -> f64 {
        settings_content_height(
            self.menu_bar_card_height(card_width),
            self.observation_height(card_width),
            self.update_card.height(card_width),
        )
    }

    fn menu_bar_card_height(&self, card_width: f64) -> f64 {
        self.layout_menu_bar_card(card_width, false)
    }

    fn layout_menu_bar_card(&self, card_width: f64, apply: bool) -> f64 {
        let width = (card_width - 28.0).max(1.0);
        if apply {
            self.menu_bar_label.setFrame(NSRect::new(
                NSPoint::new(14.0, 8.0),
                NSSize::new(width, 18.0),
            ));
            self.menu_bar_visibility.setFrame(NSRect::new(
                NSPoint::new(14.0, 31.0),
                NSSize::new(width, 16.0),
            ));
            self.menu_bar_popup.setFrame(NSRect::new(
                NSPoint::new(14.0, 51.0),
                NSSize::new(width, 26.0),
            ));
        }
        let mut y = 84.0;
        place_observation_label(&self.menu_bar_help, 14.0, width, 16.0, &mut y, apply);
        y += 10.0;
        if apply {
            self.menu_bar_separator
                .setFrame(NSRect::new(NSPoint::new(14.0, y), NSSize::new(width, 1.0)));
        }
        y += 11.0;
        if apply {
            self.menu_bar_preview_well
                .setFrame(NSRect::new(NSPoint::new(14.0, y), NSSize::new(36.0, 36.0)));
            self.menu_bar_preview
                .setFrame(NSRect::new(NSPoint::new(9.0, 9.0), NSSize::new(18.0, 18.0)));
            self.menu_bar_icon_status.setFrame(NSRect::new(
                NSPoint::new(60.0, y + 9.0),
                NSSize::new((width - 46.0).max(1.0), 18.0),
            ));
        }
        y += 46.0;
        let choose_width = self
            .menu_bar_choose
            .cell()
            .map(|cell| cell.cellSize().width + 8.0)
            .unwrap_or(110.0)
            .max(110.0);
        let restore_width = self
            .menu_bar_restore
            .cell()
            .map(|cell| cell.cellSize().width + 8.0)
            .unwrap_or(156.0)
            .max(156.0);
        let side_by_side = choose_width + 8.0 + restore_width <= width;
        if apply {
            self.menu_bar_choose.setFrame(NSRect::new(
                NSPoint::new(14.0, y),
                NSSize::new(if side_by_side { choose_width } else { width }, 26.0),
            ));
            self.menu_bar_restore.setFrame(NSRect::new(
                NSPoint::new(
                    if side_by_side {
                        22.0 + choose_width
                    } else {
                        14.0
                    },
                    if side_by_side { y } else { y + 32.0 },
                ),
                NSSize::new(
                    if side_by_side {
                        width - choose_width - 8.0
                    } else {
                        width
                    },
                    26.0,
                ),
            ));
        }
        y += if side_by_side { 26.0 } else { 58.0 };
        y += 8.0;
        place_observation_label(&self.menu_bar_icon_help, 14.0, width, 16.0, &mut y, apply);
        if !self.menu_bar_icon_error.isHidden() {
            y += 6.0;
            place_observation_label(&self.menu_bar_icon_error, 14.0, width, 16.0, &mut y, apply);
        }
        y + MENU_BAR_CARD_BOTTOM
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
        let menu_bar_height = self.menu_bar_card_height(card_width);
        self.menu_bar_card.setFrame(NSRect::new(
            NSPoint::new(8.0, MENU_BAR_CARD_TOP),
            NSSize::new(card_width, menu_bar_height),
        ));
        self.layout_menu_bar_card(card_width, true);
        let observation_height = self.layout_observation(card_width, true);
        self.observation_card.setFrame(NSRect::new(
            NSPoint::new(8.0, MENU_BAR_CARD_TOP + menu_bar_height + MENU_BAR_CARD_GAP),
            NSSize::new(card_width, observation_height),
        ));
        self.lifecycle_card.layout(NSRect::new(
            NSPoint::new(
                8.0,
                MENU_BAR_CARD_TOP + menu_bar_height + MENU_BAR_CARD_GAP * 2.0 + observation_height,
            ),
            NSSize::new(card_width, CARD_HEIGHT),
        ));
        let update_height = self.update_card.height(card_width);
        self.update_card.layout(NSRect::new(
            NSPoint::new(
                8.0,
                MENU_BAR_CARD_TOP
                    + menu_bar_height
                    + MENU_BAR_CARD_GAP * 3.0
                    + observation_height
                    + CARD_HEIGHT,
            ),
            NSSize::new(card_width, update_height),
        ));
    }

    fn observation_height(&self, card_width: f64) -> f64 {
        self.layout_observation(card_width, false)
    }

    fn layout_observation(&self, width: f64, apply: bool) -> f64 {
        let content_width = (width - 28.0).max(1.0);
        let row_width = (width - 96.0).max(1.0);
        let mut y = 14.0;
        place_observation_label(
            &self.observation_title,
            14.0,
            content_width,
            18.0,
            &mut y,
            apply,
        );
        y += 8.0;
        for (index, (field, toggle)) in [
            (
                &self.observation_local_label,
                &self.observation_local_switch,
            ),
            (
                &self.observation_remote_label,
                &self.observation_remote_switch,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let field_width = (width - 88.0).max(1.0);
            let height = measured_label_height(field, field_width).max(30.0);
            if apply {
                field.setFrame(NSRect::new(
                    NSPoint::new(14.0, y),
                    NSSize::new(field_width, height),
                ));
                toggle.setFrame(NSRect::new(
                    NSPoint::new(width - 60.0, y + (height - 30.0) / 2.0),
                    NSSize::new(46.0, 30.0),
                ));
            }
            y += height + 8.0;
            if index == 0 {
                place_observation_label(
                    &self.observation_local_help,
                    14.0,
                    content_width,
                    16.0,
                    &mut y,
                    apply,
                );
                y += 8.0;
            }
        }
        if !self.observation_notice.isHidden() {
            place_observation_label(
                &self.observation_notice,
                14.0,
                content_width,
                16.0,
                &mut y,
                apply,
            );
            y += 6.0;
        }
        if !self.observation_error_toggle.isHidden() {
            if apply {
                self.observation_error_toggle.setFrame(NSRect::new(
                    NSPoint::new(14.0, y),
                    NSSize::new(content_width, 26.0),
                ));
            }
            y += 30.0;
            if !self.observation_error.isHidden() {
                place_observation_label(
                    &self.observation_error,
                    14.0,
                    content_width,
                    16.0,
                    &mut y,
                    apply,
                );
                y += 6.0;
            }
        }
        if !self.machine_rows.is_empty() {
            y += 8.0;
            place_observation_label(
                &self.observation_machines,
                30.0,
                content_width - 16.0,
                16.0,
                &mut y,
                apply,
            );
            y += 8.0;
        }
        for row in &self.machine_rows {
            let row_start = y;
            let title_height = measured_label_height(&row.title, row_width).max(17.0);
            if apply {
                row.title.setFrame(NSRect::new(
                    NSPoint::new(30.0, y),
                    NSSize::new(row_width, title_height),
                ));
                row.toggle.setFrame(NSRect::new(
                    NSPoint::new(width - 60.0, y),
                    NSSize::new(46.0, 30.0),
                ));
            }
            y += title_height + 3.0;
            place_observation_label(&row.session, 30.0, row_width, 15.0, &mut y, apply);
            y += 2.0;
            place_observation_label(&row.status, 30.0, row_width, 15.0, &mut y, apply);
            if !row.copy.isHidden() {
                y += 6.0;
                if apply {
                    row.copy.setFrame(NSRect::new(
                        NSPoint::new(30.0, y),
                        NSSize::new((content_width - 16.0).max(1.0), 26.0),
                    ));
                }
                y += 26.0;
            }
            if !row.error_toggle.isHidden() {
                y += 4.0;
                if apply {
                    row.error_toggle.setFrame(NSRect::new(
                        NSPoint::new(30.0, y),
                        NSSize::new((content_width - 16.0).max(1.0), 26.0),
                    ));
                }
                y += 26.0;
                if !row.error.isHidden() {
                    y += 4.0;
                    place_observation_label(
                        &row.error,
                        30.0,
                        content_width - 16.0,
                        16.0,
                        &mut y,
                        apply,
                    );
                }
            }
            y = y.max(row_start + 56.0) + 8.0;
        }
        if apply {
            self.observation_registration.setFrame(NSRect::new(
                NSPoint::new(14.0, y),
                NSSize::new(content_width, 27.0),
            ));
        }
        y += 27.0;
        y + 14.0
    }
}

fn settings_content_height(
    menu_bar_height: f64,
    observation_height: f64,
    update_height: f64,
) -> f64 {
    SETTINGS_BASE_HEIGHT
        + MENU_BAR_CARD_GAP * 2.0
        + menu_bar_height
        + observation_height
        + update_height
}

fn measured_label_height(field: &NSTextField, width: f64) -> f64 {
    // NSCell measures the actual localized glyphs and wraps at the available width.
    let size = field
        .cell()
        .expect("wrapping label cell")
        .cellSizeForBounds(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width.max(1.0), 100_000.0),
        ));
    size.height.ceil()
}

fn place_observation_label(
    field: &NSTextField,
    x: f64,
    width: f64,
    minimum: f64,
    y: &mut f64,
    apply: bool,
) {
    let height = measured_label_height(field, width).max(minimum);
    if apply {
        field.setFrame(NSRect::new(
            NSPoint::new(x, *y),
            NSSize::new(width.max(1.0), height),
        ));
    }
    *y += height;
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

fn set_accessibility_identifier(view: &NSView, value: &str) {
    let value = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityIdentifier: Some(&*value)];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_appearance_preserves_editing_baseline_until_explicit_rebase() {
        let original = BubbleAppearance::default();
        let external = BubbleAppearance {
            theme: BubbleTheme::DustyRose,
            ..original
        };
        let mut draft = BubbleColorDraft::default();
        draft.receive(original, false);
        draft.receive(external, true);
        assert_eq!(draft.baseline, Some(original));
        assert_eq!(draft.latest, Some(external));
        assert!(draft.conflicted());
        draft.receive(external, false);
        assert!(draft.conflicted());
        assert_eq!(draft.baseline, Some(original));
        assert!(draft.rebase());
        assert_eq!(draft.baseline, Some(external));
        assert!(!draft.conflicted());
    }

    #[test]
    fn clean_external_update_and_own_save_advance_color_baseline() {
        let original = BubbleAppearance::default();
        let external = BubbleAppearance {
            theme: BubbleTheme::MoonlitInk,
            ..original
        };
        let custom = BubbleAppearance {
            theme: BubbleTheme::Custom,
            ..original
        };
        let mut draft = BubbleColorDraft::default();
        draft.receive(original, false);
        draft.receive(external, false);
        assert_eq!(draft.baseline, Some(external));
        assert!(!draft.conflicted());
        draft.receive(custom, true);
        assert!(draft.conflicted());
        draft.saved(custom);
        assert_eq!(draft.baseline, Some(custom));
        assert!(!draft.conflicted());
    }
}
