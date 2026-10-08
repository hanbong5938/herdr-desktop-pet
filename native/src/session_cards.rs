use crate::herdr_protocol::AgentStatus;
use crate::i18n::{
    display_status_label, offline_status, session_empty, session_filter_label,
    session_list_summary, session_pane, session_source, session_status_label, text, Message,
    SessionFilterLabel, SessionStatusLabel, UiLocale,
};
use crate::preferences::{BubbleAppearance, BubbleColor, BubblePalette, SessionListPreferences};
use crate::session_view::{
    display_value, Availability, CardDisplay, DisplayStatus, SessionFilter, SessionKey,
    SessionListOptions, SessionSnapshot, SessionSort, SessionStatusSummary, SessionView,
};
use crate::state::AppState;
use crate::status_indicator::{semantic_color, StatusIcon, STATUS_ICON_GAP, STATUS_ICON_SIZE};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::Message as _;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSBezierPath, NSBorderType, NSButton, NSColor, NSControlSize,
    NSControlStateValueOn, NSEvent, NSFont, NSLineBreakMode, NSPopUpButton, NSScrollElasticity,
    NSScrollView, NSScrollerStyle, NSSearchField, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSString};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

const DEFAULT_FRAME_WIDTH: f64 = 360.0;
const DEFAULT_FRAME_HEIGHT: f64 = 220.0;
const OUTER_INSET: f64 = 8.0;
const SEARCH_HEIGHT: f64 = 24.0;
const POPUP_HEIGHT: f64 = 24.0;
const TOOLBAR_GAP: f64 = 6.0;
const POPUP_GAP: f64 = 8.0;
const SUMMARY_HEIGHT: f64 = 40.0;
const HEADER_HEIGHT: f64 =
    SEARCH_HEIGHT + TOOLBAR_GAP + POPUP_HEIGHT + TOOLBAR_GAP + SUMMARY_HEIGHT;
const SESSION_SORTS: [SessionSort; 3] = [
    SessionSort::Stable,
    SessionSort::TitleAsc,
    SessionSort::SourceAsc,
];
const HEADER_GAP: f64 = 5.0;
const ROW_HEIGHT: f64 = 46.0;
const EMPTY_HEIGHT: f64 = 20.0;
const LIST_VIEWPORT_MAX_HEIGHT: f64 = 180.0;
const ROW_GAP: f64 = 0.0;
const ROW_HORIZONTAL_INSET: f64 = 12.0;
const ROW_TEXT_HEIGHT: f64 = 16.0;
const ROW_TEXT_INSET: f64 = 5.0;
const ROW_SELECTION_INSET: f64 = 1.0;
const ROW_SELECTION_RADIUS: f64 = 8.0;
const BADGE_GAP: f64 = 8.0;

/// Toolbar, insets and one selectable row; the inline reply adds its measured height.
pub(crate) const fn minimum_selectable_height() -> f64 {
    OUTER_INSET * 2.0 + HEADER_HEIGHT + HEADER_GAP + ROW_HEIGHT
}

pub(crate) const fn maximum_cards_height() -> f64 {
    OUTER_INSET * 2.0 + HEADER_HEIGHT + HEADER_GAP + LIST_VIEWPORT_MAX_HEIGHT
}

#[derive(Default)]
struct CardsIntent {
    reply_composition_active: bool,
    search_composition_active: bool,
    resize_frozen: bool,
    pending_selection: Option<SessionKey>,
    pending_filter: Option<SessionFilter>,
    pending_query: Option<String>,
    pending_query_native: Option<Retained<NSString>>,
    pending_sort: Option<SessionSort>,
    pending_running_first: Option<bool>,
    pending_selection_deferred: bool,
    deferred_refresh: bool,
    deferred_selection_applied: bool,
}
impl CardsIntent {
    fn composing(&self) -> bool {
        self.reply_composition_active || self.search_composition_active
    }

    fn pending_preferences(
        &self,
        applied: SessionListPreferences,
    ) -> Option<SessionListPreferences> {
        if self.pending_sort.is_none() && self.pending_running_first.is_none() {
            return None;
        }
        Some(SessionListPreferences {
            sort: self.pending_sort.unwrap_or(applied.sort),
            running_first: self.pending_running_first.unwrap_or(applied.running_first),
        })
    }

    fn clear_pending_preferences(&mut self) {
        self.pending_sort = None;
        self.pending_running_first = None;
    }
    fn retain_saved_preferences(
        &mut self,
        applied: SessionListPreferences,
        saved: SessionListPreferences,
    ) {
        // A later failed choice cannot discard an earlier, already persisted
        // choice that the frozen native rows have not consumed yet.
        self.pending_sort = (saved.sort != applied.sort).then_some(saved.sort);
        self.pending_running_first =
            (saved.running_first != applied.running_first).then_some(saved.running_first);
        self.deferred_refresh = true;
    }

    fn request_selection(&mut self, key: SessionKey, marked: bool) {
        self.pending_selection = Some(key);
        self.pending_selection_deferred = true;
        self.reply_composition_active |= marked;
        self.deferred_refresh = true;
    }

    fn set_resize_frozen(&mut self, frozen: bool) {
        if self.resize_frozen && !frozen {
            self.deferred_refresh = true;
        }
        self.resize_frozen = frozen;
    }

    fn accept_deferred_selection(&mut self, valid: bool) -> bool {
        let reveal = valid && self.pending_selection_deferred;
        self.deferred_selection_applied |= reveal;
        self.pending_selection_deferred = false;
        reveal
    }
}

struct SessionCardsRootIvars {
    inner: Weak<RefCell<SessionCardsInner>>,
    intent: Rc<RefCell<CardsIntent>>,
    search: Retained<NSSearchField>,
    popup: Retained<NSPopUpButton>,
    sort_popup: Retained<NSPopUpButton>,
    running_toggle: Retained<NSButton>,
    applied_filter: Cell<SessionFilter>,
    applied_options: Cell<SessionListPreferences>,
}

struct SessionCardViewIvars {
    inner: Weak<RefCell<SessionCardsInner>>,
    intent: Rc<RefCell<CardsIntent>>,
    key: SessionKey,
    title: Retained<NSTextField>,
    context: Retained<NSTextField>,
    status: Retained<NSTextField>,
    icon: StatusIcon,
    accessibility_details: RefCell<CardAccessibilityDetails>,
    display_status: Cell<DisplayStatus>,
    show_status_indicators: Cell<bool>,
    selected: Cell<bool>,
    reply_height: Cell<f64>,
    palette: Cell<BubblePalette>,
}

define_class!(
    // SAFETY:
    // - SessionCardsDocument is a main-thread-only NSView used solely as the
    //   NSScrollView document view.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetSessionCardsDocument"]
    struct SessionCardsDocument;

    unsafe impl NSObjectProtocol for SessionCardsDocument {}

    impl SessionCardsDocument {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY:
    // - SessionCardsRoot is only created and used on the AppKit main thread.
    // - Its ivars are retained or weak Rust state and follow NSObject lifetime rules.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetSessionCardsRoot"]
    #[ivars = SessionCardsRootIvars]
    struct SessionCardsRoot;

    unsafe impl NSObjectProtocol for SessionCardsRoot {}

    impl SessionCardsRoot {
        #[unsafe(method(filterChanged:))]
        fn filter_changed(&self, _sender: Option<&AnyObject>) {
            let index: isize = unsafe { msg_send![&*self.ivars().popup, indexOfSelectedItem] };
            let Some(filter) = SessionFilter::ALL.get(index.max(0) as usize).copied() else {
                return;
            };
            let mut intent = self.ivars().intent.borrow_mut();
            intent.pending_filter = Some(filter);
            intent.deferred_refresh = true;
            drop(intent);
            self.restore_popup();
            crate::ui::cards_content_changed();
        }

        #[unsafe(method(sortChanged:))]
        fn sort_changed(&self, _sender: Option<&AnyObject>) {
            let index: isize = unsafe { msg_send![&*self.ivars().sort_popup, indexOfSelectedItem] };
            let Some(sort) = SESSION_SORTS.get(index.max(0) as usize).copied() else {
                return;
            };
            let mut intent = self.ivars().intent.borrow_mut();
            intent.pending_sort = Some(sort);
            intent.deferred_refresh = true;
            drop(intent);
            self.restore_options();
            crate::ui::cards_options_changed();
        }

        #[unsafe(method(runningChanged:))]
        fn running_changed(&self, _sender: Option<&AnyObject>) {
            let mut intent = self.ivars().intent.borrow_mut();
            intent.pending_running_first =
                Some(self.ivars().running_toggle.state() == NSControlStateValueOn);
            intent.deferred_refresh = true;
            drop(intent);
            self.restore_options();
            crate::ui::cards_options_changed();
        }

        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, _sender: Option<&AnyObject>) {
            self.search_text_changed();
        }

        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &AnyObject) {
            self.search_text_changed();
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _notification: &AnyObject) {
            self.search_text_changed();
            crate::ui::wake();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command(
            &self,
            _control: &AnyObject,
            _editor: &AnyObject,
            command: objc2::runtime::Sel,
        ) -> bool {
            if command == sel!(insertNewline:) || command == sel!(insertNewlineIgnoringFieldEditor:) {
                return true.into();
            }
            if command == sel!(cancelOperation:) {
                self.search_escape();
                return true.into();
            }
            false
        }


        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

impl SessionCardsRoot {
    fn restore_popup(&self) {
        let index = SessionFilter::ALL
            .iter()
            .position(|filter| *filter == self.ivars().applied_filter.get())
            .unwrap_or(0) as isize;
        let selected: isize = unsafe { msg_send![&*self.ivars().popup, indexOfSelectedItem] };
        if selected != index {
            let _: () = unsafe { msg_send![&*self.ivars().popup, selectItemAtIndex: index] };
        }
    }

    fn restore_options(&self) {
        let options = self.ivars().applied_options.get();
        let index = SESSION_SORTS
            .iter()
            .position(|sort| *sort == options.sort)
            .unwrap_or(0) as isize;
        let selected: isize = unsafe { msg_send![&*self.ivars().sort_popup, indexOfSelectedItem] };
        if selected != index {
            let _: () = unsafe { msg_send![&*self.ivars().sort_popup, selectItemAtIndex: index] };
        }
        let state = if options.running_first {
            NSControlStateValueOn
        } else {
            objc2_app_kit::NSControlStateValueOff
        };
        if self.ivars().running_toggle.state() != state {
            self.ivars().running_toggle.setState(state);
        }
    }

    fn search_marked(&self) -> bool {
        let editor: Option<&AnyObject> = unsafe { msg_send![&*self.ivars().search, currentEditor] };
        editor.is_some_and(|editor| unsafe { msg_send![editor, hasMarkedText] })
    }

    fn search_text_changed(&self) {
        let value = self.ivars().search.stringValue();
        let marked = self.search_marked();
        let mut intent = self.ivars().intent.borrow_mut();
        if intent
            .pending_query_native
            .as_deref()
            .is_none_or(|pending| !native_string_equal(&value, pending))
        {
            intent.pending_query = Some(value.to_string());
            intent.pending_query_native = Some(value);
            intent.deferred_refresh = true;
        }
        if intent.search_composition_active != marked {
            intent.search_composition_active = marked;
            intent.deferred_refresh = true;
        }
        drop(intent);
        crate::ui::cards_content_changed();
    }

    fn search_escape(&self) {
        if self.search_marked() {
            let editor: Option<&AnyObject> =
                unsafe { msg_send![&*self.ivars().search, currentEditor] };
            if let Some(editor) = editor {
                let context: Option<&AnyObject> = unsafe { msg_send![editor, inputContext] };
                if let Some(context) = context {
                    let _: () = unsafe { msg_send![context, discardMarkedText] };
                }
                let still_marked: bool = unsafe { msg_send![editor, hasMarkedText] };
                if still_marked {
                    let range: objc2_foundation::NSRange =
                        unsafe { msg_send![editor, markedRange] };
                    let empty = NSString::from_str("");
                    let _: () =
                        unsafe { msg_send![editor, insertText: &*empty, replacementRange: range] };
                    let _: () = unsafe { msg_send![editor, unmarkText] };
                }
            }
            crate::ui::wake();
        } else if self.ivars().search.stringValue().length() > 0 {
            let empty = NSString::from_str("");
            self.ivars().search.setStringValue(&empty);
            let editor: Option<&AnyObject> =
                unsafe { msg_send![&*self.ivars().search, currentEditor] };
            if let Some(editor) = editor {
                let _: () = unsafe { msg_send![editor, setString: &*empty] };
            }
            self.search_text_changed();
        } else if let Some(window) = self.window() {
            let _ = window.makeFirstResponder(None);
        }
    }
}

define_class!(
    // SAFETY:
    // - SessionCardView is only created and used on the AppKit main thread.
    // - Each card owns its immutable key and label and weakly references the component state.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetSessionCardView"]
    #[ivars = SessionCardViewIvars]
    struct SessionCardView;

    unsafe impl NSObjectProtocol for SessionCardView {}

    impl SessionCardView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let palette = self.ivars().palette.get();
            if self.ivars().selected.get() {
                palette_color(palette.accent, 0.14).setFill();
                let bounds = inset_rect(self.bounds(), ROW_SELECTION_INSET);
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    bounds, ROW_SELECTION_RADIUS, ROW_SELECTION_RADIUS,
                ).fill();
            }
            palette_color(palette.border, 0.7).setStroke();
            let bounds = self.bounds();
            let path = NSBezierPath::bezierPath();
            path.setLineWidth(0.5);
            path.moveToPoint(NSPoint::new(ROW_HORIZONTAL_INSET, bounds.size.height - 0.5));
            path.lineToPoint(NSPoint::new(
                (bounds.size.width - ROW_HORIZONTAL_INSET).max(ROW_HORIZONTAL_INSET),
                bounds.size.height - 0.5,
            ));
            path.stroke();
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<&NSView> {
            // Keep the header selectable, including its noninteractive labels,
            // but let AppKit route events to controls inside the reply slot.
            let frame = self.frame();
            if point.x < frame.origin.x
                || point.x > frame.origin.x + frame.size.width
                || point.y < frame.origin.y
                || point.y > frame.origin.y + frame.size.height
            {
                return None;
            }
            if self.ivars().reply_height.get() > 0.0
                && point.y >= frame.origin.y + ROW_HEIGHT
            {
                return unsafe { msg_send![super(self), hitTest: point] };
            }
            Some(&**self)
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            self.select();
        }

        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            self.select();
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Retained<NSString> {
            NSString::from_str("AXButton")
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }

);

impl SessionCardView {
    fn select(&self) {
        let marked = crate::ui::composer_is_composing()
            || self.ivars().intent.borrow().search_composition_active;
        let Some(inner) = self.ivars().inner.upgrade() else {
            return;
        };
        {
            let mut intent = self.ivars().intent.borrow_mut();
            intent.request_selection(self.ivars().key.clone(), marked);
        }
        if marked || self.ivars().intent.borrow().resize_frozen {
            return;
        }
        let ready = if let Ok(mut cards) = inner.try_borrow_mut() {
            if !self.ivars().intent.borrow().composing()
                && !self.ivars().intent.borrow().resize_frozen
            {
                self.ivars().intent.borrow_mut().pending_selection_deferred = false;
                cards.reveal_selection = true;
                cards.refresh();
            }
            true
        } else {
            false
        };
        if ready {
            crate::ui::cards_selection_changed();
        } else {
            crate::ui::wake();
        }
    }
}

impl SessionCardsDocument {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl SessionCardsRoot {
    fn new(
        frame: NSRect,
        search: Retained<NSSearchField>,
        popup: Retained<NSPopUpButton>,
        sort_popup: Retained<NSPopUpButton>,
        running_toggle: Retained<NSButton>,
        options: SessionListPreferences,
        inner: Weak<RefCell<SessionCardsInner>>,
        intent: Rc<RefCell<CardsIntent>>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SessionCardsRootIvars {
            inner,
            intent,
            search,
            popup,
            sort_popup,
            running_toggle,
            applied_filter: Cell::new(SessionFilter::All),
            applied_options: Cell::new(options),
        });
        // SAFETY: NSView's initWithFrame: has the expected signature.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl SessionCardView {
    fn new(
        frame: NSRect,
        display: &CardDisplay,
        view: &SessionView,
        locale: UiLocale,
        selected: bool,
        show_status_indicators: bool,
        palette: BubblePalette,
        inner: Weak<RefCell<SessionCardsInner>>,
        intent: Rc<RefCell<CardsIntent>>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let title = card_field(&display.title, true, mtm);
        let context = card_field(&display.context, false, mtm);
        let status_text = card_display_status(locale, view, show_status_indicators);
        let status = card_field(&status_text, false, mtm);
        status.setAlignment(NSTextAlignment::Right);
        let icon = StatusIcon::new(mtm);

        let accessibility_details = CardAccessibilityDetails::new(view, display);
        let this = Self::alloc(mtm).set_ivars(SessionCardViewIvars {
            inner,
            intent,
            key: view.key.clone(),
            title,
            context,
            status,
            icon,
            accessibility_details: RefCell::new(accessibility_details),
            display_status: Cell::new(view.display_status()),
            show_status_indicators: Cell::new(show_status_indicators),
            selected: Cell::new(selected),
            reply_height: Cell::new(0.0),
            palette: Cell::new(palette),
        });
        // SAFETY: NSView's initWithFrame: has the expected signature.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        unsafe {
            let _: () = msg_send![&*this, setAccessibilityElement: true];
        }
        let accessibility = this
            .ivars()
            .accessibility_details
            .borrow()
            .label(locale, &status_text);
        let accessibility = NSString::from_str(&accessibility);
        this.setToolTip(Some(&accessibility));
        this.addSubview(&this.ivars().title);
        this.addSubview(&this.ivars().context);
        this.addSubview(&this.ivars().status);
        this.addSubview(this.ivars().icon.view());
        this.update_accessibility(&accessibility);
        this.layout_fields(frame.size);
        this.set_palette(palette);
        this.setNeedsDisplay(true);
        this
    }

    fn update(
        &self,
        locale: UiLocale,
        view: &SessionView,
        display: &CardDisplay,
        selected: bool,
        show_status_indicators: bool,
    ) {
        self.ivars()
            .title
            .setStringValue(&NSString::from_str(&display.title));
        self.ivars()
            .context
            .setStringValue(&NSString::from_str(&display.context));
        let status_text = card_display_status(locale, view, show_status_indicators);
        self.ivars()
            .status
            .setStringValue(&NSString::from_str(&status_text));
        self.ivars().display_status.set(view.display_status());
        self.ivars()
            .show_status_indicators
            .set(show_status_indicators);
        self.ivars().selected.set(selected);
        self.layout_fields(self.frame().size);
        self.ivars()
            .accessibility_details
            .borrow_mut()
            .refresh(view, display);
        let accessibility = self
            .ivars()
            .accessibility_details
            .borrow()
            .label(locale, &status_text);
        self.update_accessibility(&NSString::from_str(&accessibility));
        self.update_status_tint();
        self.setNeedsDisplay(true);
    }
    fn update_offline(&self, locale: UiLocale, show_status_indicators: bool) {
        let status = display_status_label(locale, DisplayStatus::Offline);
        self.ivars()
            .status
            .setStringValue(&NSString::from_str(status));
        self.ivars().display_status.set(DisplayStatus::Offline);
        self.ivars()
            .show_status_indicators
            .set(show_status_indicators);
        self.layout_fields(self.frame().size);
        let label = self
            .ivars()
            .accessibility_details
            .borrow()
            .label(locale, status);
        self.update_accessibility(&NSString::from_str(&label));
        self.update_status_tint();
        self.setNeedsDisplay(true);
    }

    fn update_accessibility(&self, accessibility: &NSString) {
        self.setToolTip(Some(accessibility));
        set_accessibility_label(self, accessibility);
        let show_status_indicators = self.ivars().show_status_indicators.get();
        for field in [
            &self.ivars().title,
            &self.ivars().context,
            &self.ivars().status,
        ] {
            if show_status_indicators {
                // The row announces the full status once; the icon is decorative.
                field.setToolTip(None);
                unsafe {
                    let _: () = msg_send![&**field, setAccessibilityElement: false];
                }
            } else {
                field.setToolTip(Some(accessibility));
                set_accessibility_label(field, accessibility);
                unsafe {
                    let _: () = msg_send![&**field, setAccessibilityElement: true];
                }
            }
            if let Some(cell) = field.cell() {
                unsafe {
                    let _: () = msg_send![&*cell, setAccessibilityElement: !show_status_indicators];
                }
            }
        }
    }
    fn layout_fields(&self, size: objc2_foundation::NSSize) {
        // Rows are non-flipped inside the flipped document: the unchanged
        // header lives at the top and the reply slot occupies the bottom.
        let reply_height = (size.height - ROW_HEIGHT).max(0.0);
        self.ivars().reply_height.set(reply_height);
        let header = objc2_foundation::NSSize::new(size.width, ROW_HEIGHT);
        let mut full = row_text_frame(header, ROW_HEIGHT - ROW_TEXT_INSET - ROW_TEXT_HEIGHT);
        full.origin.y += reply_height;
        let measured = self
            .ivars()
            .status
            .cell()
            .map(|cell| cell.cellSize().width)
            .unwrap_or(0.0)
            .max(0.0);
        if self.ivars().show_status_indicators.get() {
            let icon_width = STATUS_ICON_SIZE.min(full.size.width * 0.45);
            let gap = STATUS_ICON_GAP.min((full.size.width * 0.45 - icon_width).max(0.0));
            // Reserve 55% for the title even after accounting for the title/badge gap.
            let badge_width =
                (measured + icon_width + gap).min((full.size.width * 0.45 - BADGE_GAP).max(0.0));
            let badge_x = full.origin.x + full.size.width - badge_width;
            self.ivars().icon.view().setHidden(false);
            self.ivars().icon.view().setFrame(NSRect::new(
                NSPoint::new(
                    badge_x,
                    full.origin.y + (full.size.height - STATUS_ICON_SIZE) / 2.0,
                ),
                objc2_foundation::NSSize::new(icon_width.min(badge_width), STATUS_ICON_SIZE),
            ));
            let label_x = badge_x
                + icon_width.min(badge_width)
                + gap.min((badge_width - icon_width).max(0.0));
            self.ivars().status.setFrame(NSRect::new(
                NSPoint::new(label_x, full.origin.y),
                objc2_foundation::NSSize::new(
                    (full.origin.x + full.size.width - label_x).max(0.0),
                    full.size.height,
                ),
            ));
            self.ivars().title.setFrame(NSRect::new(
                full.origin,
                objc2_foundation::NSSize::new(
                    (badge_x - BADGE_GAP - full.origin.x).max(0.0),
                    full.size.height,
                ),
            ));
        } else {
            self.ivars().icon.view().setHidden(true);
            let badge_width = measured.min(full.size.width);
            let badge_x = full.origin.x + full.size.width - badge_width;
            self.ivars().status.setFrame(NSRect::new(
                NSPoint::new(badge_x, full.origin.y),
                objc2_foundation::NSSize::new(badge_width, full.size.height),
            ));
            self.ivars().title.setFrame(NSRect::new(
                full.origin,
                objc2_foundation::NSSize::new(
                    (full.size.width - badge_width - BADGE_GAP).max(0.0),
                    full.size.height,
                ),
            ));
        }
        let mut context = row_text_frame(header, ROW_TEXT_INSET);
        context.origin.y += reply_height;
        self.ivars().context.setFrame(context);
        self.setNeedsDisplay(true);
    }
    fn set_palette(&self, palette: BubblePalette) {
        self.ivars().palette.set(palette);
        self.ivars()
            .title
            .setTextColor(Some(&palette_color(palette.text, 1.0)));
        self.ivars()
            .context
            .setTextColor(Some(&palette_color(palette.muted, 1.0)));
        self.update_status_tint();
        self.setNeedsDisplay(true);
    }

    fn update_status_tint(&self) {
        let palette = self.ivars().palette.get();
        if self.ivars().show_status_indicators.get() {
            let background = selected_surface(palette, self.ivars().selected.get());
            let status = self.ivars().display_status.get();
            self.ivars()
                .status
                .setTextColor(Some(&semantic_color(status, background)));
            self.ivars().icon.update(status, background);
        } else {
            self.ivars()
                .status
                .setTextColor(Some(&palette_color(palette.muted, 1.0)));
        }
    }

    fn key(&self) -> &SessionKey {
        &self.ivars().key
    }
}
/// A row handles its header itself; its reply slot is native content and must
/// keep AppKit's ordinary context-menu behavior.
pub(crate) fn card_header_key_at_hit(hit: &NSView, window_point: NSPoint) -> Option<SessionKey> {
    let row = hit.downcast_ref::<SessionCardView>()?;
    let point = row.convertPoint_fromView(window_point, None);
    (point.y >= row.ivars().reply_height.get()
        && point.y <= row.bounds().size.height
        && point.x >= 0.0
        && point.x <= row.bounds().size.width)
        .then(|| row.key().clone())
}

struct ReplySlot {
    key: SessionKey,
    view: Retained<NSView>,
    height: f64,
}

struct SessionCardsInner {
    shared: Arc<Mutex<AppState>>,
    intent: Rc<RefCell<CardsIntent>>,
    locale: UiLocale,
    mtm: MainThreadMarker,
    root: Retained<SessionCardsRoot>,
    summary: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    document: Retained<SessionCardsDocument>,
    empty: Retained<NSTextField>,
    filter: SessionFilter,
    query: String,
    query_native: Retained<NSString>,
    sort: SessionSort,
    running_first: bool,
    matched: usize,
    omitted: usize,
    options_epoch: u64,
    rendered_options_epoch: Option<u64>,
    palette: BubblePalette,
    show_status_indicators: bool,
    rendered_show_status_indicators: Option<bool>,
    status_summary: SessionStatusSummary,
    selected: Option<SessionKey>,
    last_revision: Option<u64>,
    selection_epoch: u64,
    selected_target_cache: Option<((Option<u64>, u64, UiLocale), String)>,
    rendered_locale: Option<UiLocale>,
    controls_locale: Option<UiLocale>,
    rendered_filter: Option<SessionFilter>,
    rendered_selected: Option<SessionKey>,
    reply: Option<ReplySlot>,
    reveal_selection: bool,
    reset_scroll: bool,
    rows: Vec<Retained<SessionCardView>>,
}

impl SessionCardsInner {
    fn content_height(&self) -> f64 {
        OUTER_INSET * 2.0
            + HEADER_HEIGHT
            + HEADER_GAP
            + self.rows_height().min(LIST_VIEWPORT_MAX_HEIGHT)
    }

    fn document_content_height(&self) -> f64 {
        OUTER_INSET * 2.0 + HEADER_HEIGHT + HEADER_GAP + self.rows_height()
    }

    fn minimum_content_width(&self) -> f64 {
        // Keep each native popup at its 82pt minimum beside the other.
        OUTER_INSET * 2.0 + 82.0 * 2.0 + POPUP_GAP
    }

    fn rows_height(&self) -> f64 {
        if self.rows.is_empty() {
            EMPTY_HEIGHT
        } else {
            rows_height(
                self.rows.len(),
                self.reply.as_ref().map_or(0.0, |reply| reply.height),
            )
        }
    }

    fn set_frame(&mut self, frame: NSRect) {
        // Keep the existing clip-view origin across panel reflow.  Capture it
        // before changing either the root frame or the document geometry.
        let scroll_origin = self.scroll.contentView().bounds().origin;
        self.root.setFrame(frame);
        self.layout(frame.size, scroll_origin);
    }

    fn layout(&self, size: objc2_foundation::NSSize, scroll_origin: NSPoint) {
        let width = size.width.max(1.0);
        let height = size.height.max(1.0);
        let inset = OUTER_INSET.min(width / 2.0).min(height / 2.0);
        let header_y = (height - inset - HEADER_HEIGHT).max(0.0);
        let header_width = (width - 2.0 * inset).max(1.0);
        let search_y = header_y + SUMMARY_HEIGHT + TOOLBAR_GAP + POPUP_HEIGHT + TOOLBAR_GAP;
        self.root.ivars().search.setFrame(NSRect::new(
            NSPoint::new(inset, search_y),
            objc2_foundation::NSSize::new(header_width, SEARCH_HEIGHT),
        ));
        let popup_width = ((header_width - POPUP_GAP) / 2.0).max(1.0);
        let popup_y = header_y + SUMMARY_HEIGHT + TOOLBAR_GAP;
        self.root.ivars().popup.setFrame(NSRect::new(
            NSPoint::new(inset, popup_y),
            objc2_foundation::NSSize::new(popup_width, POPUP_HEIGHT),
        ));
        self.root.ivars().sort_popup.setFrame(NSRect::new(
            NSPoint::new(inset + popup_width + POPUP_GAP, popup_y),
            objc2_foundation::NSSize::new(popup_width, POPUP_HEIGHT),
        ));
        let toggle_width = 150.0_f64.min(header_width);
        self.root.ivars().running_toggle.setFrame(NSRect::new(
            NSPoint::new(inset, header_y + (SUMMARY_HEIGHT - 18.0) / 2.0),
            objc2_foundation::NSSize::new(toggle_width, 18.0),
        ));
        self.summary.setFrame(NSRect::new(
            NSPoint::new(inset + toggle_width + POPUP_GAP, header_y),
            objc2_foundation::NSSize::new(
                (header_width - toggle_width - POPUP_GAP).max(1.0),
                SUMMARY_HEIGHT,
            ),
        ));
        let scroll_y = inset;
        let scroll_height = (header_y - HEADER_GAP - scroll_y).max(1.0);
        let scroll_width = (width - 2.0 * inset).max(1.0);
        let scroll_frame = NSRect::new(
            NSPoint::new(inset, scroll_y),
            objc2_foundation::NSSize::new(scroll_width, scroll_height),
        );
        self.scroll.setFrame(scroll_frame);
        self.empty.setFrame(NSRect::new(
            NSPoint::new(inset, scroll_y),
            objc2_foundation::NSSize::new(scroll_width, scroll_height),
        ));

        let document_height = if self.rows.is_empty() {
            scroll_height
        } else {
            self.rows_height().max(scroll_height)
        };
        // Set the new document height before measuring contentSize.  AppKit
        // only decides whether the vertical scroller consumes width during
        // tiling, so measuring before this step leaves stale old-width rows
        // during filter transitions.
        self.document.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(scroll_width, document_height),
        ));
        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();
        let document_width = self.scroll.documentVisibleRect().size.width.max(1.0);
        self.document.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(document_width, document_height),
        ));
        let reply_index = self
            .reply
            .as_ref()
            .and_then(|reply| self.rows.iter().position(|row| row.key() == &reply.key));
        for (index, row) in self.rows.iter().enumerate() {
            // The flipped document uses a top-left origin. Only the row
            // following the selected row and its successors shift downward.
            let frame = row_frame(
                index,
                reply_index,
                self.reply.as_ref().map_or(0.0, |reply| reply.height),
                document_width,
            );
            row.setFrame(frame);
            row.layout_fields(frame.size);
            if reply_index == Some(index) {
                if let Some(reply) = &self.reply {
                    reply.view.setFrame(NSRect::new(
                        NSPoint::new(ROW_HORIZONTAL_INSET, 0.0),
                        objc2_foundation::NSSize::new(
                            (document_width - 2.0 * ROW_HORIZONTAL_INSET).max(1.0),
                            reply.height,
                        ),
                    ));
                }
            }
        }
        // Tiling after the final document width lets AppKit settle the
        // scroller geometry before restoring the old origin.  Constrain the
        // proposed bounds so shrinking/reflowing the list cannot leave the
        // clip view beyond the new document range.
        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();
        let clip_view = self.scroll.contentView();
        let mut proposed_bounds = clip_view.bounds();
        proposed_bounds.origin = scroll_origin;
        let constrained_bounds = clip_view.constrainBoundsRect(proposed_bounds);
        clip_view.scrollToPoint(constrained_bounds.origin);
        self.scroll.reflectScrolledClipView(&clip_view);
        if self.reveal_selection {
            if let Some(row) = self
                .rows
                .iter()
                .find(|row| Some(row.key()) == self.selected.as_ref())
            {
                let visible_height = clip_view.bounds().size.height;
                let target = NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(
                        row.frame().size.width,
                        row.frame().size.height.min(visible_height),
                    ),
                );
                row.scrollRectToVisible(target);
            }
        }
    }
    fn set_palette(&mut self, palette: BubblePalette) {
        self.palette = palette;
        self.summary
            .setTextColor(Some(&palette_color(palette.muted, 1.0)));
        self.empty
            .setTextColor(Some(&palette_color(palette.muted, 1.0)));
        let (red, green, blue) = palette.surface.rgb();
        let dark = 0.2126 * red + 0.7152 * green + 0.0722 * blue < 0.45;
        let appearance_name = unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        };
        let appearance = NSAppearance::appearanceNamed(appearance_name);
        for popup in [&self.root.ivars().popup, &self.root.ivars().sort_popup] {
            popup.setAppearance(appearance.as_deref());
            popup.setContentTintColor(Some(&palette_color(palette.text, 1.0)));
        }
        self.root
            .ivars()
            .search
            .setAppearance(appearance.as_deref());
        self.root
            .ivars()
            .running_toggle
            .setAppearance(appearance.as_deref());
        self.root
            .ivars()
            .running_toggle
            .setContentTintColor(Some(&palette_color(palette.text, 1.0)));
        self.root
            .ivars()
            .search
            .setTextColor(Some(&palette_color(palette.text, 1.0)));
        for row in &self.rows {
            row.set_palette(palette);
        }
    }

    fn set_locale(&mut self, locale: UiLocale) {
        self.locale = locale;
        self.rendered_locale = None;
        self.refresh();
    }

    fn refresh(&mut self) {
        // A resize only reflows the already displayed hierarchy. In particular
        // do not consume a pending selection or detach its reply during a drag.
        if self.intent.borrow().resize_frozen {
            self.intent.borrow_mut().deferred_refresh = true;
            return;
        }
        let scroll_origin = self.scroll.contentView().bounds().origin;
        let search_marked = self.root.search_marked();
        let search_value = self.root.ivars().search.stringValue();
        let (marked, pending_filter, pending_query, pending_selection, deferred) = {
            let mut intent = self.intent.borrow_mut();
            if intent.search_composition_active && !search_marked {
                intent.deferred_refresh = true;
            }
            intent.search_composition_active = search_marked;
            let current = intent
                .pending_query_native
                .as_deref()
                .unwrap_or(&self.query_native);
            if !native_string_equal(&search_value, current) {
                intent.pending_query = Some(search_value.to_string());
                intent.pending_query_native = Some(search_value.clone());
                intent.deferred_refresh = true;
            }
            let composing = intent.composing();
            (
                composing,
                intent.pending_filter,
                if composing {
                    None
                } else {
                    intent.pending_query.clone()
                },
                if composing {
                    None
                } else {
                    intent.pending_selection.clone()
                },
                intent.deferred_refresh,
            )
        };
        let filter = if marked {
            self.filter
        } else {
            pending_filter.unwrap_or(self.filter)
        };
        let query = if marked {
            &self.query
        } else {
            pending_query.as_ref().unwrap_or(&self.query)
        };
        let (revision, snapshot, marked_views, marked_displays, selection_valid) =
            match self.shared.lock() {
                Ok(state) => {
                    let revision = state.session_revision();
                    if (marked || !deferred)
                        && self.last_revision == Some(revision)
                        && self.rendered_locale == Some(self.locale)
                        && self.rendered_options_epoch == Some(self.options_epoch)
                        && self.rendered_filter == Some(filter)
                        && self.rendered_selected == self.selected
                        && self.rendered_show_status_indicators == Some(self.show_status_indicators)
                    {
                        drop(state);
                        if !marked && self.reveal_selection {
                            self.layout(self.root.frame().size, scroll_origin);
                            self.reveal_selection = false;
                        }
                        return;
                    }
                    let mut snapshot = state.session_snapshot(
                        &SessionListOptions {
                            filter,
                            query,
                            sort: self.sort,
                            running_first: self.running_first,
                            locale: self.locale,
                        },
                        self.selected.as_ref(),
                    );
                    let selection_valid = pending_selection
                        .as_ref()
                        .is_some_and(|key| selection_visible(&snapshot, key));
                    if !marked && selection_valid {
                        snapshot.selected = pending_selection.clone();
                    }
                    let (marked_views, marked_displays) = if marked {
                        let views: Vec<_> = self
                            .rows
                            .iter()
                            .filter_map(|row| state.session_view_for_key(row.key()))
                            .collect();
                        let keys: Vec<_> = views.iter().map(|view| &view.key).collect();
                        let displays = state.session_displays_for_keys(self.locale, &keys);
                        (views, displays)
                    } else {
                        (Vec::new(), Vec::new())
                    };
                    (
                        revision,
                        snapshot,
                        marked_views,
                        marked_displays,
                        selection_valid,
                    )
                }
                Err(_) => return,
            };

        if marked {
            // Preserve every displayed row's identity and its attached editor;
            // update live status without accepting a new row order or cap.
            // Titles are still disambiguated against every session, not just
            // the frozen rows, so they do not shift while composing.
            self.status_summary = snapshot.status_summary;
            self.matched = snapshot.matched;
            self.omitted = snapshot.omitted;
            self.update_controls();
            self.update_summary(&snapshot, !self.rows.is_empty());
            let displays = &marked_displays;
            for row in &self.rows {
                if let Some(index) = marked_views.iter().position(|view| view.key == *row.key()) {
                    let view = &marked_views[index];
                    let Some(display) = displays[index].as_ref() else {
                        continue;
                    };
                    row.update(
                        self.locale,
                        view,
                        display,
                        self.selected.as_ref() == Some(row.key()),
                        self.show_status_indicators,
                    );
                } else {
                    row.update_offline(self.locale, self.show_status_indicators);
                }
            }
            self.last_revision = Some(revision);
            self.rendered_locale = Some(self.locale);
            self.rendered_filter = Some(self.filter);
            self.rendered_options_epoch = Some(self.options_epoch);
            self.rendered_selected = self.selected.clone();
            self.rendered_show_status_indicators = Some(self.show_status_indicators);
            self.intent.borrow_mut().deferred_refresh = true;
            return;
        }

        let conditions_changed = self.filter != filter || self.query != *query;
        {
            let mut intent = self.intent.borrow_mut();
            intent.pending_filter = None;
            intent.pending_query = None;
            intent.pending_query_native = None;
            intent.pending_selection = None;
            intent.deferred_refresh = false;
            self.reveal_selection |= intent.accept_deferred_selection(selection_valid);
        }
        self.filter = filter;
        if self.query != *query {
            self.query = pending_query.expect("changed query has a pending search value");
            self.query_native = search_value;
        }
        if conditions_changed {
            self.options_epoch = self.options_epoch.wrapping_add(1);
        }
        if self.selected != snapshot.selected {
            self.selection_epoch = self.selection_epoch.saturating_add(1);
        }
        self.selected = snapshot.selected.clone();
        if self.reply.as_ref().is_some_and(|reply| {
            self.selected.as_ref() != Some(&reply.key)
                || !snapshot.rows.iter().any(|row| row.key == reply.key)
        }) {
            self.detach_reply();
        }
        self.status_summary = snapshot.status_summary;
        self.matched = snapshot.matched;
        self.omitted = snapshot.omitted;
        self.update_controls();
        self.update_summary(&snapshot, !snapshot.rows.is_empty());
        self.update_rows(&snapshot);
        self.last_revision = Some(revision);
        self.rendered_locale = Some(self.locale);
        self.rendered_filter = Some(self.filter);
        self.rendered_options_epoch = Some(self.options_epoch);
        self.rendered_show_status_indicators = Some(self.show_status_indicators);
        self.rendered_selected = self.selected.clone();
        self.layout(
            self.root.frame().size,
            if conditions_changed || self.reset_scroll {
                NSPoint::new(0.0, 0.0)
            } else {
                scroll_origin
            },
        );
        self.reset_scroll = false;
        self.reveal_selection = false;
    }

    fn update_controls(&mut self) {
        if self.controls_locale != Some(self.locale) {
            for (index, filter) in SessionFilter::ALL.into_iter().enumerate() {
                if let Some(item) = self.root.ivars().popup.itemAtIndex(index as isize) {
                    item.setTitle(&NSString::from_str(filter_label(self.locale, filter)));
                }
            }
            for (index, sort) in SESSION_SORTS.into_iter().enumerate() {
                if let Some(item) = self.root.ivars().sort_popup.itemAtIndex(index as isize) {
                    item.setTitle(&NSString::from_str(sort_label(self.locale, sort)));
                }
            }
            let search_label = NSString::from_str(text(self.locale, Message::SessionSearch));
            self.root
                .ivars()
                .search
                .setPlaceholderString(Some(&search_label));
            set_accessibility_label(&self.root.ivars().search, &search_label);
            set_accessibility_label(
                &self.root.ivars().popup,
                &NSString::from_str(text(self.locale, Message::SessionStatusFilter)),
            );
            set_accessibility_label(
                &self.root.ivars().sort_popup,
                &NSString::from_str(text(self.locale, Message::SessionSortLabel)),
            );
            let running_label = NSString::from_str(text(self.locale, Message::SessionRunningFirst));
            self.root.ivars().running_toggle.setTitle(&running_label);
            set_accessibility_label(&self.root.ivars().running_toggle, &running_label);
            self.controls_locale = Some(self.locale);
        }
        self.root.ivars().applied_filter.set(self.filter);
        self.root.restore_popup();
        self.root
            .ivars()
            .applied_options
            .set(SessionListPreferences {
                sort: self.sort,
                running_first: self.running_first,
            });
        self.root.restore_options();
    }

    fn update_summary(&self, snapshot: &SessionSnapshot, visible_rows: bool) {
        let summary = session_list_summary(self.locale, snapshot.matched, snapshot.omitted);
        self.summary.setStringValue(&NSString::from_str(&summary));

        let empty_text = session_empty(self.locale, snapshot.total, snapshot.matched);
        self.empty.setStringValue(&NSString::from_str(empty_text));
        self.empty.setHidden(visible_rows);
    }

    fn detach_reply(&mut self) {
        if let Some(reply) = self.reply.take() {
            reply.view.removeFromSuperview();
        }
    }

    fn attach_reply(&mut self, key: &SessionKey, view: &NSView, height: f64) -> bool {
        if self.selected.as_ref() != Some(key) || !self.rows.iter().any(|row| row.key() == key) {
            return false;
        }
        let height = height.max(0.0);
        if let Some(reply) = &mut self.reply {
            if reply.key == *key && std::ptr::eq(&*reply.view, view) {
                let row = self
                    .rows
                    .iter()
                    .find(|row| row.key() == key)
                    .expect("visible selected row");
                // SAFETY: Cards and their retained reply view are created with the main-thread marker.
                let orphaned = !unsafe { reply.view.superview() }
                    .as_deref()
                    .is_some_and(|parent| std::ptr::eq(parent, &***row));
                if orphaned {
                    row.addSubview(&reply.view);
                }
                if reply.height != height || orphaned {
                    reply.height = height;
                    let origin = self.scroll.contentView().bounds().origin;
                    self.layout(self.root.frame().size, origin);
                }
                return true;
            }
        }
        self.detach_reply();
        let row = self
            .rows
            .iter()
            .find(|row| row.key() == key)
            .expect("visible selected row");
        row.addSubview(view);
        self.reply = Some(ReplySlot {
            key: key.clone(),
            view: view.retain(),
            height,
        });
        self.reveal_selection = true;
        let origin = self.scroll.contentView().bounds().origin;
        self.layout(self.root.frame().size, origin);
        self.reveal_selection = false;
        true
    }

    fn remove_row(&self, old: &SessionCardView) {
        if let Some(reply) = &self.reply {
            // SAFETY: Cards and their retained reply view are created with the main-thread marker.
            if unsafe { reply.view.superview() }
                .as_deref()
                .is_some_and(|parent| std::ptr::eq(parent, &**old))
            {
                reply.view.removeFromSuperview();
            }
        }
        old.removeFromSuperview();
    }

    fn update_rows(&mut self, snapshot: &SessionSnapshot) {
        for (index, (view, display)) in snapshot.rows.iter().zip(&snapshot.displays).enumerate() {
            let selected = self.selected.as_ref() == Some(&view.key);
            if let Some(row) = self.rows.get(index) {
                if row.key() == &view.key {
                    row.update(
                        self.locale,
                        view,
                        display,
                        selected,
                        self.show_status_indicators,
                    );
                    continue;
                }
            }
            if let Some(old) = self.rows.get(index) {
                self.remove_row(old);
            }
            let row = SessionCardView::new(
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(1.0, ROW_HEIGHT),
                ),
                display,
                view,
                self.locale,
                selected,
                self.show_status_indicators,
                self.palette,
                self.root.ivars().inner.clone(),
                Rc::clone(&self.intent),
                self.mtm,
            );
            self.document.addSubview(&row);
            if index < self.rows.len() {
                self.rows[index] = row;
            } else {
                self.rows.push(row);
            }
        }
        while self.rows.len() > snapshot.rows.len() {
            if let Some(old) = self.rows.pop() {
                self.remove_row(&old);
            }
        }
        if let Some(reply) = &self.reply {
            if let Some(row) = self.rows.iter().find(|row| row.key() == &reply.key) {
                // SAFETY: Cards and their retained reply view are created with the main-thread marker.
                if !unsafe { reply.view.superview() }
                    .as_deref()
                    .is_some_and(|parent| std::ptr::eq(parent, &***row))
                {
                    row.addSubview(&reply.view);
                }
            }
        }
    }
}
fn selection_visible(snapshot: &SessionSnapshot, key: &SessionKey) -> bool {
    snapshot.rows.iter().any(|row| &row.key == key)
}

pub(crate) struct SessionCards {
    inner: Rc<RefCell<SessionCardsInner>>,
    intent: Rc<RefCell<CardsIntent>>,
    root: Retained<SessionCardsRoot>,
}

impl SessionCards {
    pub(crate) fn new(
        shared: Arc<Mutex<AppState>>,
        locale: UiLocale,
        options: SessionListPreferences,
        mtm: MainThreadMarker,
    ) -> Self {
        let default_frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(DEFAULT_FRAME_WIDTH, DEFAULT_FRAME_HEIGHT),
        );
        let palette = BubbleAppearance::default().palette();
        let intent = Rc::new(RefCell::new(CardsIntent::default()));
        let inner = Rc::new_cyclic(|weak| {
            let popup = make_filter_popup(locale, mtm);
            let sort_popup = make_sort_popup(locale, mtm);
            let search: Retained<NSSearchField> = unsafe {
                msg_send![NSSearchField::alloc(mtm), initWithFrame: NSRect::new(
                    NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(DEFAULT_FRAME_WIDTH, SEARCH_HEIGHT)
                )]
            };
            search.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            search.setRefusesFirstResponder(false);
            search.setPlaceholderString(Some(&NSString::from_str(text(
                locale,
                Message::SessionSearch,
            ))));
            let running_toggle: Retained<NSButton> = unsafe {
                msg_send![NSButton::alloc(mtm), initWithFrame: NSRect::new(
                    NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(150.0, 18.0)
                )]
            };
            running_toggle.setButtonType(objc2_app_kit::NSButtonType::Switch);
            running_toggle.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            running_toggle.setRefusesFirstResponder(false);
            running_toggle.setTitle(&NSString::from_str(text(
                locale,
                Message::SessionRunningFirst,
            )));
            popup.setRefusesFirstResponder(false);
            sort_popup.setRefusesFirstResponder(false);
            let root = SessionCardsRoot::new(
                default_frame,
                search,
                popup,
                sort_popup,
                running_toggle,
                options,
                weak.clone(),
                Rc::clone(&intent),
                mtm,
            );
            unsafe {
                let _: () = msg_send![&*root.ivars().popup, setTarget: Some(&*root)];
                let _: () = msg_send![&*root.ivars().popup, setAction: sel!(filterChanged:)];
                let _: () = msg_send![&*root.ivars().sort_popup, setTarget: Some(&*root)];
                let _: () = msg_send![&*root.ivars().sort_popup, setAction: sel!(sortChanged:)];
                let _: () = msg_send![&*root.ivars().running_toggle, setTarget: Some(&*root)];
                let _: () =
                    msg_send![&*root.ivars().running_toggle, setAction: sel!(runningChanged:)];
                let _: () = msg_send![&*root.ivars().search, setTarget: Some(&*root)];
                let _: () = msg_send![&*root.ivars().search, setAction: sel!(searchChanged:)];
                let _: () = msg_send![&*root.ivars().search, setDelegate: Some(&*root)];
            }
            root.restore_options();
            let summary = label(&session_list_summary(locale, 0, 0), 10.0, mtm);
            summary.setAlignment(NSTextAlignment::Right);
            summary.setTextColor(Some(&palette_color(palette.muted, 1.0)));

            let empty = label(session_empty(locale, 0, 0), 11.0, mtm);
            empty.setAlignment(NSTextAlignment::Center);
            empty.setTextColor(Some(&palette_color(palette.muted, 1.0)));
            empty.setHidden(true);

            let document = SessionCardsDocument::new(
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(DEFAULT_FRAME_WIDTH, DEFAULT_FRAME_HEIGHT),
                ),
                mtm,
            );
            document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            document.setAutoresizesSubviews(true);
            let scroll: Retained<NSScrollView> = unsafe {
                msg_send![
                    NSScrollView::alloc(mtm),
                    initWithFrame: NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        objc2_foundation::NSSize::new(DEFAULT_FRAME_WIDTH, DEFAULT_FRAME_HEIGHT),
                    )
                ]
            };
            unsafe {
                let _: () = msg_send![&*scroll, setDocumentView: Some(&*document)];
                let _: () = msg_send![&*scroll, setHasVerticalScroller: true];
                let _: () = msg_send![&*scroll, setAutohidesScrollers: true];
                let _: () = msg_send![&*scroll, setDrawsBackground: false];
            }
            scroll.setBorderType(NSBorderType::NoBorder);
            scroll.setScrollerStyle(NSScrollerStyle::Overlay);
            scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
            scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
            scroll.contentView().setDrawsBackground(false);
            scroll.setAutoresizesSubviews(true);
            root.addSubview(&scroll);
            root.addSubview(&empty);
            root.addSubview(&root.ivars().popup);
            root.addSubview(&summary);
            root.addSubview(&root.ivars().search);
            root.addSubview(&root.ivars().sort_popup);
            root.addSubview(&root.ivars().running_toggle);
            // The root retains these controls for the lifetime of their unretained links.
            unsafe {
                root.ivars()
                    .search
                    .setNextKeyView(Some(&root.ivars().popup));
                root.ivars()
                    .popup
                    .setNextKeyView(Some(&root.ivars().sort_popup));
                root.ivars()
                    .sort_popup
                    .setNextKeyView(Some(&root.ivars().running_toggle));
                root.ivars()
                    .running_toggle
                    .setNextKeyView(Some(&root.ivars().search));
            }

            SessionCardsInner {
                shared,
                intent: Rc::clone(&intent),
                locale,
                mtm,
                root,
                summary,
                scroll,
                document,
                empty,
                filter: SessionFilter::All,
                query: String::new(),
                query_native: NSString::from_str(""),
                sort: options.sort,
                running_first: options.running_first,
                matched: 0,
                omitted: 0,
                options_epoch: 0,
                rendered_options_epoch: None,
                palette,
                show_status_indicators: true,
                rendered_show_status_indicators: None,
                status_summary: SessionStatusSummary::default(),
                selected: None,
                last_revision: None,
                selection_epoch: 0,
                selected_target_cache: None,
                controls_locale: None,
                rendered_locale: None,
                rendered_filter: None,
                rendered_selected: None,
                reply: None,
                reveal_selection: false,
                reset_scroll: false,
                rows: Vec::new(),
            }
            .into()
        });
        let root = inner.borrow().root.clone();
        let cards = Self {
            inner,
            intent,
            root,
        };
        cards.set_frame(default_frame);
        cards.refresh();
        cards
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.root
    }
    pub(crate) fn selection_stamp(&self) -> (Option<u64>, u64, UiLocale, u64) {
        let inner = self.inner.borrow();
        (
            inner.last_revision,
            inner.selection_epoch,
            inner.locale,
            inner.options_epoch,
        )
    }

    pub(crate) fn selected_target(&self) -> Option<(SessionKey, String)> {
        let (shared, key, locale, stamp) = {
            let inner = self.inner.borrow();
            let key = inner.selected.as_ref()?;
            let stamp = (inner.last_revision, inner.selection_epoch, inner.locale);
            if let Some((cached_stamp, title)) = &inner.selected_target_cache {
                if *cached_stamp == stamp {
                    return Some((key.clone(), title.clone()));
                }
            }
            (Arc::clone(&inner.shared), key.clone(), inner.locale, stamp)
        };
        let title = {
            let state = shared.lock().ok()?;
            state.session_display_for_key(locale, &key)?.title
        };
        let mut inner = self.inner.borrow_mut();
        if (inner.last_revision, inner.selection_epoch, inner.locale) == stamp {
            inner.selected_target_cache = Some((stamp, title.clone()));
        }
        Some((key, title))
    }
    pub(crate) fn visible_selected_target(&self) -> Option<(SessionKey, String)> {
        {
            let inner = self.inner.borrow();
            if !inner
                .rows
                .iter()
                .any(|row| Some(row.key()) == inner.selected.as_ref())
            {
                return None;
            }
        }
        self.selected_target()
    }

    pub(crate) fn set_frame(&self, frame: NSRect) {
        self.inner.borrow_mut().set_frame(frame);
    }

    pub(crate) fn set_locale(&self, locale: UiLocale) {
        self.inner.borrow_mut().set_locale(locale);
    }
    pub(crate) fn set_composition_active(&self, active: bool) {
        let mut intent = self.intent.borrow_mut();
        if intent.reply_composition_active && !active {
            intent.deferred_refresh = true;
        }
        intent.reply_composition_active = active;
    }

    pub(crate) fn sync_search_composition(&self) {
        let marked = self.root.search_marked();
        let mut intent = self.intent.borrow_mut();
        if intent.search_composition_active && !marked {
            intent.deferred_refresh = true;
        }
        intent.search_composition_active = marked;
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.intent.borrow().composing()
    }

    pub(crate) fn search_has_focus(&self) -> bool {
        let editor: Option<&AnyObject> =
            unsafe { msg_send![&*self.root.ivars().search, currentEditor] };
        let responder = self
            .root
            .window()
            .and_then(|window| window.firstResponder());
        editor.zip(responder).is_some_and(|(editor, responder)| {
            (editor as *const AnyObject).cast::<()>() == Retained::as_ptr(&responder).cast::<()>()
        })
    }

    pub(crate) fn search_editor(&self) -> Option<&AnyObject> {
        if !self.search_has_focus() {
            return None;
        }
        unsafe { msg_send![&*self.root.ivars().search, currentEditor] }
    }

    pub(crate) fn search_escape(&self) {
        self.root.search_escape();
    }

    pub(crate) fn pending_options(&self) -> Option<SessionListPreferences> {
        let intent = self.intent.borrow();
        let inner = self.inner.borrow();
        intent.pending_preferences(SessionListPreferences {
            sort: inner.sort,
            running_first: inner.running_first,
        })
    }

    pub(crate) fn commit_options(&self, options: SessionListPreferences) {
        let mut inner = self.inner.borrow_mut();
        {
            let mut intent = self.intent.borrow_mut();
            intent.clear_pending_preferences();
            intent.deferred_refresh = true;
        }
        if inner.sort != options.sort || inner.running_first != options.running_first {
            inner.sort = options.sort;
            inner.running_first = options.running_first;
            inner.options_epoch = inner.options_epoch.wrapping_add(1);
            inner.reset_scroll = true;
            inner.reveal_selection = false;
        }
    }

    pub(crate) fn reject_options(&self, saved: SessionListPreferences) {
        let applied = {
            let inner = self.inner.borrow();
            SessionListPreferences {
                sort: inner.sort,
                running_first: inner.running_first,
            }
        };
        self.intent
            .borrow_mut()
            .retain_saved_preferences(applied, saved);
        self.root.restore_options();
    }

    pub(crate) fn set_resize_frozen(&self, frozen: bool) {
        self.intent.borrow_mut().set_resize_frozen(frozen);
    }

    pub(crate) fn has_deferred_refresh(&self) -> bool {
        self.intent.borrow().deferred_refresh
    }

    pub(crate) fn take_deferred_selection_applied(&self) -> bool {
        let mut intent = self.intent.borrow_mut();
        std::mem::take(&mut intent.deferred_selection_applied)
    }

    pub(crate) fn set_show_status_indicators(&self, enabled: bool) {
        let mut inner = self.inner.borrow_mut();
        if inner.show_status_indicators == enabled {
            return;
        }
        inner.show_status_indicators = enabled;
        inner.rendered_show_status_indicators = None;
        inner.refresh();
    }

    pub(crate) fn status_summary(&self) -> SessionStatusSummary {
        self.inner.borrow().status_summary
    }

    pub(crate) fn refresh(&self) {
        self.inner.borrow_mut().refresh();
    }

    pub(crate) fn content_height(&self) -> f64 {
        self.inner.borrow().content_height()
    }

    /// Full requested viewport height, before the automatic bubble's 180pt cap.
    pub(crate) fn document_content_height(&self) -> f64 {
        self.inner.borrow().document_content_height()
    }

    pub(crate) fn minimum_content_width(&self) -> f64 {
        self.inner.borrow().minimum_content_width()
    }

    pub(crate) fn attach_reply(&self, key: &SessionKey, view: &NSView, height: f64) -> bool {
        if self.intent.borrow().composing() || self.intent.borrow().resize_frozen {
            self.intent.borrow_mut().deferred_refresh = true;
            return self
                .inner
                .borrow()
                .reply
                .as_ref()
                .is_some_and(|reply| reply.key == *key && std::ptr::eq(&*reply.view, view));
        }
        self.inner.borrow_mut().attach_reply(key, view, height)
    }

    pub(crate) fn detach_reply(&self) {
        if self.intent.borrow().composing() || self.intent.borrow().resize_frozen {
            self.intent.borrow_mut().deferred_refresh = true;
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.reply.is_none() {
            return;
        }
        inner.detach_reply();
        let origin = inner.scroll.contentView().bounds().origin;
        inner.layout(inner.root.frame().size, origin);
    }

    pub(crate) fn set_palette(&mut self, palette: BubblePalette) {
        self.inner.borrow_mut().set_palette(palette);
    }
}

fn rows_height(count: usize, reply_height: f64) -> f64 {
    count as f64 * (ROW_HEIGHT + ROW_GAP) - ROW_GAP + reply_height
}

fn row_frame(index: usize, reply_index: Option<usize>, reply_height: f64, width: f64) -> NSRect {
    let y = index as f64 * (ROW_HEIGHT + ROW_GAP)
        + if reply_index.is_some_and(|reply| index > reply) {
            reply_height
        } else {
            0.0
        };
    let height = ROW_HEIGHT
        + if reply_index == Some(index) {
            reply_height
        } else {
            0.0
        };
    NSRect::new(
        NSPoint::new(0.0, y),
        objc2_foundation::NSSize::new(width, height),
    )
}

fn make_filter_popup(locale: UiLocale, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let popup: Retained<NSPopUpButton> = unsafe {
        msg_send![
            NSPopUpButton::alloc(mtm),
            initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(140.0, POPUP_HEIGHT),
            ),
            pullsDown: false
        ]
    };
    popup.setControlSize(NSControlSize::Small);
    popup.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    popup.setContentTintColor(Some(&palette_color(
        BubbleAppearance::default().palette().text,
        1.0,
    )));
    for filter in SessionFilter::ALL {
        let title = NSString::from_str(filter_label(locale, filter));
        let _: () = unsafe { msg_send![&*popup, addItemWithTitle: &*title] };
    }
    let _: () = unsafe { msg_send![&*popup, selectItemAtIndex: 0isize] };
    popup
}

fn make_sort_popup(locale: UiLocale, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let popup: Retained<NSPopUpButton> = unsafe {
        msg_send![
            NSPopUpButton::alloc(mtm),
            initWithFrame: NSRect::new(
                NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(140.0, POPUP_HEIGHT),
            ),
            pullsDown: false
        ]
    };
    popup.setControlSize(NSControlSize::Small);
    popup.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    for sort in SESSION_SORTS {
        let _: () = unsafe {
            msg_send![&*popup, addItemWithTitle: &*NSString::from_str(sort_label(locale, sort))]
        };
    }
    popup
}

fn sort_label(locale: UiLocale, sort: SessionSort) -> &'static str {
    text(
        locale,
        match sort {
            SessionSort::Stable => Message::SessionSortStable,
            SessionSort::TitleAsc => Message::SessionSortName,
            SessionSort::SourceAsc => Message::SessionSortSource,
        },
    )
}

fn native_string_equal(a: &NSString, b: &NSString) -> bool {
    a.isEqualToString(b)
}

fn filter_label(locale: UiLocale, filter: SessionFilter) -> &'static str {
    let filter = match filter {
        SessionFilter::All => SessionFilterLabel::All,
        SessionFilter::Idle => SessionFilterLabel::Idle,
        SessionFilter::Working => SessionFilterLabel::Working,
        SessionFilter::Waiting => SessionFilterLabel::Waiting,
        SessionFilter::Completed => SessionFilterLabel::Completed,
        SessionFilter::Unknown => SessionFilterLabel::Unknown,
        SessionFilter::Offline => SessionFilterLabel::Offline,
    };
    session_filter_label(locale, filter)
}
fn label(text: &str, size: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setTextColor(Some(&palette_color(
        BubbleAppearance::default().palette().text,
        1.0,
    )));
    field.setDrawsBackground(false);
    field.setBordered(false);
    field.setBezeled(false);
    field.setEditable(false);
    field.setSelectable(false);
    field.setMaximumNumberOfLines(2);
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    field
}

fn card_field(text: &str, primary: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setAlignment(NSTextAlignment::Left);
    let size = if primary { 13.0 } else { 11.0 };
    let palette = BubbleAppearance::default().palette();
    let color = if primary { palette.text } else { palette.muted };
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setTextColor(Some(&palette_color(color, 1.0)));
    field.setDrawsBackground(false);
    field.setBordered(false);
    field.setBezeled(false);
    field.setEditable(false);
    field.setSelectable(false);
    field.setUsesSingleLineMode(true);
    field.setMaximumNumberOfLines(1);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    field
}
fn palette_color(value: BubbleColor, alpha: f64) -> Retained<NSColor> {
    let (red, green, blue) = value.rgb();
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha)
}
fn selected_surface(palette: BubblePalette, selected: bool) -> (f64, f64, f64) {
    let surface = palette.surface.rgb();
    if !selected {
        return surface;
    }
    let accent = palette.accent.rgb();
    let alpha = 0.14;
    (
        accent.0 * alpha + surface.0 * (1.0 - alpha),
        accent.1 * alpha + surface.1 * (1.0 - alpha),
        accent.2 * alpha + surface.2 * (1.0 - alpha),
    )
}

fn set_accessibility_label(view: &NSView, label: &NSString) {
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(label)];
    }
}

fn inset_rect(rect: NSRect, inset: f64) -> NSRect {
    let inset = inset.max(0.0);
    NSRect::new(
        NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        objc2_foundation::NSSize::new(
            (rect.size.width - 2.0 * inset).max(0.0),
            (rect.size.height - 2.0 * inset).max(0.0),
        ),
    )
}

fn row_text_frame(size: objc2_foundation::NSSize, y: f64) -> NSRect {
    let width = size.width.max(1.0);
    let inset = ROW_HORIZONTAL_INSET.min(width / 2.0);
    NSRect::new(
        NSPoint::new(inset, y.max(0.0)),
        objc2_foundation::NSSize::new(
            (width - 2.0 * inset).max(1.0),
            ROW_TEXT_HEIGHT.min(size.height.max(1.0)),
        ),
    )
}

struct CardAccessibilityDetails {
    title: String,
    context: String,
    source_id: u64,
    terminal_id: String,
    pane_id: String,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    cwd: Option<String>,
}

impl CardAccessibilityDetails {
    fn new(view: &SessionView, display: &CardDisplay) -> Self {
        Self {
            title: display.title.clone(),
            context: display.context.clone(),
            source_id: view.key.source_id,
            terminal_id: view.key.terminal_id.clone(),
            pane_id: view.pane_id.clone(),
            workspace_id: view.metadata.workspace_id.clone(),
            tab_id: view.metadata.tab_id.clone(),
            cwd: view.metadata.cwd.clone(),
        }
    }

    fn refresh(&mut self, view: &SessionView, display: &CardDisplay) {
        if self.title != display.title {
            self.title.clone_from(&display.title);
        }
        if self.context != display.context {
            self.context.clone_from(&display.context);
        }
        self.source_id = view.key.source_id;
        if self.terminal_id != view.key.terminal_id {
            self.terminal_id.clone_from(&view.key.terminal_id);
        }
        if self.pane_id != view.pane_id {
            self.pane_id.clone_from(&view.pane_id);
        }
        if self.workspace_id != view.metadata.workspace_id {
            self.workspace_id.clone_from(&view.metadata.workspace_id);
        }
        if self.tab_id != view.metadata.tab_id {
            self.tab_id.clone_from(&view.metadata.tab_id);
        }
        if self.cwd != view.metadata.cwd {
            self.cwd.clone_from(&view.metadata.cwd);
        }
    }

    fn label(&self, locale: UiLocale, status: &str) -> String {
        let mut details = format!(
            "{} · {} · {} · {} · {}",
            self.title,
            self.context,
            status,
            session_source(
                locale,
                self.source_id,
                &display_value(Some(&self.terminal_id)).unwrap_or_default()
            ),
            session_pane(
                locale,
                &display_value(Some(&self.pane_id)).unwrap_or_default()
            )
        );
        for (kind, value) in [
            (Message::Workspace, self.workspace_id.as_deref()),
            (Message::Tab, self.tab_id.as_deref()),
        ] {
            if let Some(id) = display_value(value) {
                details.push_str(&format!(" · {} {id}", text(locale, kind)));
            }
        }
        if let Some(cwd) = display_value(self.cwd.as_deref()) {
            details.push_str(&format!(" · {} {cwd}", text(locale, Message::Cwd)));
        }
        details
    }
}

#[cfg(test)]
fn card_accessibility(
    locale: UiLocale,
    view: &SessionView,
    display: &CardDisplay,
    status: &str,
) -> String {
    CardAccessibilityDetails::new(view, display).label(locale, status)
}

fn card_display_status(locale: UiLocale, view: &SessionView, enabled: bool) -> String {
    if enabled {
        display_status_label(locale, view.display_status()).to_owned()
    } else {
        card_status(locale, view)
    }
}

fn card_status(locale: UiLocale, view: &SessionView) -> String {
    let status = localized_status(locale, view.status);
    match view.availability {
        Availability::Live => status.to_owned(),
        Availability::Offline => offline_status(locale, status),
    }
}

fn localized_status(locale: UiLocale, status: AgentStatus) -> &'static str {
    let status = match status {
        AgentStatus::Idle => SessionStatusLabel::Idle,
        AgentStatus::Working => SessionStatusLabel::Working,
        AgentStatus::Blocked => SessionStatusLabel::Waiting,
        AgentStatus::Done => SessionStatusLabel::Completed,
        AgentStatus::Unknown => SessionStatusLabel::Unknown,
    };
    session_status_label(locale, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr_protocol::SessionMetadata;
    use crate::session_view::card_displays;

    #[test]
    fn resize_freeze_retains_selection_filter_and_real_composition_intents() {
        let selected = SessionKey {
            source_id: 7,
            generation: 2,
            terminal_id: "terminal".into(),
        };
        let mut intent = CardsIntent::default();
        intent.set_resize_frozen(true);
        intent.request_selection(selected.clone(), false);
        intent.pending_filter = Some(SessionFilter::Working);
        intent.pending_query = Some("needle".into());
        intent.pending_sort = Some(SessionSort::TitleAsc);
        intent.pending_running_first = Some(true);
        intent.search_composition_active = true;
        assert!(intent.pending_selection_deferred);
        assert!(!intent.reply_composition_active);
        assert!(intent.composing());
        intent.set_resize_frozen(false);
        assert_eq!(intent.pending_selection, Some(selected.clone()));
        assert_eq!(intent.pending_filter, Some(SessionFilter::Working));
        assert_eq!(intent.pending_query.as_deref(), Some("needle"));
        assert_eq!(
            intent.pending_preferences(SessionListPreferences::default()),
            Some(SessionListPreferences {
                sort: SessionSort::TitleAsc,
                running_first: true,
            })
        );
        assert!(intent.pending_selection_deferred);
        assert!(intent.deferred_refresh);
        intent.search_composition_active = false;
        assert!(!intent.composing());
        assert!(intent.accept_deferred_selection(true));
        assert!(!intent.pending_selection_deferred);
        assert!(intent.deferred_selection_applied);
        intent.request_selection(selected, true);
        intent.set_resize_frozen(true);
        intent.set_resize_frozen(false);
        assert!(intent.reply_composition_active);
        assert!(intent.composing());
    }

    #[test]
    fn reply_slot_shifts_only_following_rows_and_extends_scroll_tail() {
        let height = 58.0;
        let frames: Vec<_> = (0..3)
            .map(|index| row_frame(index, Some(1), height, 250.0))
            .collect();
        assert_eq!(frames[0].origin.y, 0.0);
        assert_eq!(frames[0].size.height, ROW_HEIGHT);
        assert_eq!(frames[1].origin.y, ROW_HEIGHT);
        assert_eq!(frames[1].size.height, ROW_HEIGHT + height);
        assert_eq!(frames[2].origin.y, 2.0 * ROW_HEIGHT + height);
        assert_eq!(
            frames[2].origin.y + frames[2].size.height,
            rows_height(3, height)
        );
        assert_eq!(row_frame(2, None, 0.0, 250.0).origin.y, 2.0 * ROW_HEIGHT);
        assert_eq!(rows_height(3, 0.0), 3.0 * ROW_HEIGHT);
    }

    fn row(
        source: u64,
        terminal: &str,
        title: Option<&str>,
        tab: Option<&str>,
        workspace: Option<&str>,
        cwd: Option<&str>,
    ) -> SessionView {
        SessionView {
            key: SessionKey {
                source_id: source,
                generation: 1,
                terminal_id: terminal.into(),
            },
            source_label: "Local".into(),
            is_local: true,
            pane_id: format!("pane-{terminal}"),
            status: AgentStatus::Working,
            outcome: None,
            availability: Availability::Live,
            metadata: SessionMetadata {
                title: title.map(str::to_owned),
                tab_label: tab.map(str::to_owned),
                workspace_label: workspace.map(str::to_owned),
                cwd: cwd.map(str::to_owned),
                ..Default::default()
            },
        }
    }
    #[test]
    fn deferred_target_requires_the_exact_displayed_generation_and_filter_row() {
        let old = row(1, "terminal", None, None, None, None);
        let other = row(2, "other", None, None, None, None);
        let snapshot = |rows: Vec<SessionView>, selected: Option<SessionKey>| SessionSnapshot {
            revision: 9,
            total: 2,
            matched: 2,
            omitted: 0,
            status_summary: SessionStatusSummary::default(),
            rows,
            displays: Vec::new(),
            selected,
        };
        let mut intent = CardsIntent::default();
        intent.request_selection(other.key.clone(), true);
        intent.request_selection(old.key.clone(), true);
        assert!(selection_visible(
            &snapshot(vec![other.clone(), old.clone()], Some(other.key.clone())),
            intent.pending_selection.as_ref().unwrap(),
        ));

        let mut replacement = old.clone();
        replacement.key.generation += 1;
        assert!(!selection_visible(
            &snapshot(vec![replacement], None),
            intent.pending_selection.as_ref().unwrap()
        ));
        // The old key can still exist in the store yet be hidden by the
        // chosen filter or the displayed cap; neither permits replay.
        assert!(!selection_visible(
            &snapshot(vec![other.clone()], Some(old.key.clone())),
            &old.key
        ));
        assert!(!selection_visible(&snapshot(vec![other], None), &old.key));
    }

    #[test]
    fn search_and_reply_ime_hold_the_last_toolbar_intent_until_both_end() {
        let applied = SessionListPreferences::default();
        let mut intent = CardsIntent::default();
        intent.search_composition_active = true;
        intent.reply_composition_active = true;
        intent.pending_filter = Some(SessionFilter::Offline);
        intent.pending_query = Some("first".into());
        intent.pending_query = Some("latest".into());
        intent.pending_sort = Some(SessionSort::TitleAsc);
        intent.pending_sort = Some(SessionSort::SourceAsc);
        intent.pending_running_first = Some(true);
        assert!(intent.composing());
        assert_eq!(intent.pending_query.as_deref(), Some("latest"));
        assert_eq!(intent.pending_filter, Some(SessionFilter::Offline));
        assert_eq!(
            intent.pending_preferences(applied),
            Some(SessionListPreferences {
                sort: SessionSort::SourceAsc,
                running_first: true,
            })
        );

        intent.reply_composition_active = false;
        assert!(intent.composing());
        intent.search_composition_active = false;
        assert!(!intent.composing());
        intent.clear_pending_preferences();
        assert_eq!(intent.pending_preferences(applied), None);
        assert_eq!(intent.pending_filter, Some(SessionFilter::Offline));
        assert_eq!(intent.pending_query.as_deref(), Some("latest"));
    }

    #[test]
    fn failed_second_choice_keeps_first_saved_choice_pending_through_resize() {
        let applied = SessionListPreferences::default();
        let saved = SessionListPreferences {
            sort: SessionSort::TitleAsc,
            running_first: false,
        };
        let mut intent = CardsIntent::default();
        intent.set_resize_frozen(true);
        intent.pending_sort = Some(saved.sort);
        assert_eq!(intent.pending_preferences(applied), Some(saved));
        // The next action requests both the saved sort and a new running-first
        // setting, but its preference write fails while native rows are frozen.
        intent.pending_running_first = Some(true);
        assert_eq!(
            intent.pending_preferences(applied),
            Some(SessionListPreferences {
                running_first: true,
                ..saved
            })
        );
        intent.retain_saved_preferences(applied, saved);
        assert_eq!(intent.pending_preferences(applied), Some(saved));
        assert!(intent.deferred_refresh);
        intent.set_resize_frozen(false);
        assert_eq!(intent.pending_preferences(applied), Some(saved));
        intent.clear_pending_preferences();
        assert_eq!(intent.pending_preferences(saved), None);
    }

    #[test]
    fn task_title_outranks_tab_and_numeric_tab_does_not_name_session() {
        let rows = [
            row(
                1,
                "a",
                Some("Split README by Language with Switching"),
                Some("2"),
                Some("Idea"),
                None,
            ),
            row(
                1,
                "b",
                Some("루벨리아 포즈 이미지 허벅지 컷"),
                Some("2"),
                Some("Idea"),
                None,
            ),
            row(2, "c", Some("zsh"), Some("2"), Some("Idea"), None),
            row(3, "d", None, Some("Fix tests"), Some("Idea"), None),
        ];
        let cards = card_displays(UiLocale::En, &rows);
        assert_eq!(cards[0].title, "Split README by Language with Switching");
        assert_eq!(cards[1].title, "루벨리아 포즈 이미지 허벅지 컷");
        assert_eq!(cards[2].title, "Idea · Tab 2");
        assert_eq!(cards[3].title, "Fix tests");
    }

    #[test]
    fn meaningful_symbols_survive_non_omp_titles_and_named_tabs() {
        let mut titled = row(
            1,
            "math",
            Some("π calculus notes"),
            Some("⠿ Braille reference"),
            None,
            None,
        );
        titled.metadata.agent = Some("codex".into());
        let tab_named = row(2, "braille", None, Some("⠿ Braille reference"), None, None);
        let cards = card_displays(UiLocale::En, &[titled, tab_named]);
        assert_eq!(cards[0].title, "π calculus notes");
        assert!(cards[0].context.contains("⠿ Braille reference"));
        assert_eq!(cards[1].title, "⠿ Braille reference");
    }

    #[test]
    fn symbol_only_titles_and_named_tabs_remain_meaningful() {
        let mut emoji = row(1, "emoji", Some("🧪"), Some("2"), None, None);
        emoji.metadata.agent = Some("codex".into());
        let mut braille = row(2, "braille", Some("⠿"), Some("2"), None, None);
        braille.metadata.agent = Some("claude".into());
        let named_tab = row(3, "tab", None, Some("🌿"), None, None);
        let cards = card_displays(UiLocale::En, &[emoji, braille, named_tab]);
        assert_eq!(cards[0].title, "🧪");
        assert_eq!(cards[1].title, "⠿");
        assert_eq!(cards[2].title, "🌿");
    }

    #[test]
    fn joined_emoji_survives_title_and_context() {
        let view = row(1, "emoji", Some("👩‍💻 Build"), Some("👩‍💻"), None, None);
        let card = card_displays(UiLocale::En, &[view]).remove(0);
        assert_eq!(card.title, "👩‍💻 Build");
        assert!(card.context.contains("👩‍💻"));
    }

    #[test]
    fn colliding_titles_differentiate_same_and_cross_source_only() {
        let rows = [
            row(1, "abc", Some("Refactor"), Some("1"), Some("Idea"), None),
            row(1, "abd", Some("Refactor"), Some("1"), Some("Idea"), None),
            row(2, "xy", Some("Refactor"), Some("1"), Some("Idea"), None),
            row(3, "z", Some("Different"), Some("1"), Some("Idea"), None),
        ];
        let cards = card_displays(UiLocale::En, &rows);
        assert!(cards[..3].iter().all(|card| card.title == "Refactor"));
        assert_eq!(cards[3].title, "Different");
        assert!(cards[..3].iter().all(|card| card.context.starts_with("(#")));
        assert_ne!(cards[0].context, cards[1].context);
        assert_ne!(cards[0].context, cards[2].context);
        assert_ne!(cards[1].context, cards[2].context);
        assert!(!cards[3].context.contains('#'));
        assert!(!cards[3].title.contains('#'));
        let reordered = card_displays(
            UiLocale::En,
            &[
                rows[2].clone(),
                rows[3].clone(),
                rows[1].clone(),
                rows[0].clone(),
            ],
        );
        assert_eq!(cards[0].context, reordered[3].context);
        assert_eq!(cards[1].context, reordered[2].context);
        assert_eq!(cards[2].context, reordered[0].context);
    }

    #[test]
    fn long_identical_titles_and_terminal_prefixes_keep_discriminator_visible() {
        let title = "Write a comprehensive review of a multilingual integration and its behavior across all active sessions";
        let common = "a".repeat(120);
        let rows = [
            row(
                1,
                &format!("{common}b"),
                Some(title),
                Some("1"),
                Some("Idea"),
                None,
            ),
            row(
                1,
                &format!("{common}c"),
                Some(title),
                Some("1"),
                Some("Idea"),
                None,
            ),
            row(2, "other", Some(title), Some("1"), Some("Idea"), None),
        ];
        let cards = card_displays(UiLocale::En, &rows);
        assert!(cards.iter().all(|card| card.title == title));
        let leading: Vec<_> = cards
            .iter()
            .map(|card| card.context.split(") · ").next().unwrap())
            .collect();
        assert!(leading.iter().all(|marker| marker.starts_with("(#")));
        assert!(leading.iter().all(|marker| marker.chars().count() <= 12));
        assert_ne!(leading[0], leading[1]);
        assert_ne!(leading[0], leading[2]);
        assert_ne!(leading[1], leading[2]);
    }

    #[test]
    fn directory_basename_collision_uses_just_enough_ancestor() {
        let rows = [
            row(1, "a", None, Some("1"), None, Some("/alpha/src")),
            row(2, "b", None, Some("1"), None, Some("/beta/src")),
        ];
        let cards = card_displays(UiLocale::En, &rows);
        assert_eq!(cards[0].title, "alpha/src · Tab 1");
        assert_eq!(cards[1].title, "beta/src · Tab 1");
        assert!(!cards[0].title.contains("/alpha/"));
    }

    #[test]
    fn titled_directory_collision_uses_shortest_context_ancestor() {
        let rows = [
            row(
                1,
                "a",
                Some("Build feature"),
                Some("1"),
                None,
                Some("/alpha/src"),
            ),
            row(
                2,
                "b",
                Some("Build feature"),
                Some("1"),
                None,
                Some("/beta/src"),
            ),
        ];
        let cards = card_displays(UiLocale::En, &rows);
        assert!(cards.iter().all(|card| card.title == "Build feature"));
        assert!(cards[0].context.contains("alpha/src"));
        assert!(cards[1].context.contains("beta/src"));
    }

    #[test]
    fn remote_titles_keep_machine_labels_and_opaque_identity_distinct() {
        let mut first = row(7, "shared", Some("Build feature"), Some("1"), None, None);
        first.is_local = false;
        first.source_label = "Shared name".into();
        let mut second = row(8, "shared", Some("Build feature"), Some("1"), None, None);
        second.is_local = false;
        second.source_label = "Shared name".into();
        let cards = card_displays(UiLocale::En, &[first.clone(), second.clone()]);
        assert_eq!(cards[0].title, "Build feature");
        assert_eq!(cards[1].title, "Build feature");
        assert!(cards[0].context.starts_with("(#7) · Shared name · "));
        assert!(cards[1].context.starts_with("(#8) · Shared name · "));
        assert_ne!(cards[0].context, cards[1].context);
        let details = card_accessibility(UiLocale::En, &first, &cards[0], "Working");
        assert!(details.contains("Build feature"));
        assert!(details.contains("Shared name"));
        assert!(details.contains("shared"));

        second.source_label = "Renamed".into();
        let renamed = card_displays(UiLocale::En, &[first, second]);
        assert!(renamed[0].context.starts_with("(#7) · Shared name · "));
        assert!(renamed[1].context.starts_with("(#8) · Renamed · "));
    }

    #[test]
    fn missing_metadata_is_unique_and_offline_retains_details() {
        let mut rows = [
            row(1, "abc", None, None, None, None),
            row(1, "abd", None, None, None, None),
        ];
        rows[0].availability = Availability::Offline;
        let cards = card_displays(UiLocale::En, &rows);
        assert_eq!(cards[0].title, cards[1].title);
        assert!(cards[0].title.starts_with("Unnamed session"));
        assert_ne!(cards[0].context, cards[1].context);
        assert!(card_status(UiLocale::En, &rows[0]).contains("Offline"));
        let details = card_accessibility(
            UiLocale::En,
            &rows[0],
            &cards[0],
            &card_status(UiLocale::En, &rows[0]),
        );
        assert!(details.contains("abc"));
        assert!(details.contains("pane-abc"));
    }
    #[test]
    fn full_multilingual_title_and_cwd_survive_tooltip_without_control_characters() {
        let mut view = row(
            1,
            "terminal",
            Some("루벨리아 포즈 이미지 허벅지 컷 — Split README by Language with Switching"),
            Some("2"),
            Some("Idea"),
            Some("/Users/a/projects/worktree-lucky-valley"),
        );
        view.metadata.agent = Some("omp".into());
        view.metadata.tab_id = Some("tab-45".into());
        view.metadata.workspace_id = Some("ws-1".into());
        let card = card_displays(UiLocale::Ko, std::slice::from_ref(&view)).remove(0);
        let detail = card_accessibility(
            UiLocale::Ko,
            &view,
            &card,
            &card_status(UiLocale::Ko, &view),
        );
        assert!(card.title.contains("Switching"));
        assert!(detail.contains("루벨리아 포즈"));
        assert!(detail.contains("/Users/a/projects/worktree-lucky-valley"));
        assert!(detail.contains("tab-45"));
        assert!(detail.contains("ws-1"));
        assert!(!card.title.contains("⠹"));
        assert_eq!(
            display_value(Some("\u{202e}Unsafe\n title")),
            Some("Unsafe title".into())
        );
    }
    #[test]
    fn external_control_characters_cannot_corrupt_card_or_tooltip() {
        let mut view = row(
            1,
            "terminal",
            Some("\u{202e}one\ntwo\u{200b}"),
            Some("π\u{202e} Math\nTab"),
            Some("Team\u{202e}\nWorkspace"),
            Some("/safe\npath"),
        );
        view.metadata.workspace_id = Some("ws\u{202e}\n1".into());
        let card = card_displays(UiLocale::En, std::slice::from_ref(&view)).remove(0);
        assert_eq!(card.title, "one two");
        let detail = card_accessibility(
            UiLocale::En,
            &view,
            &card,
            &card_status(UiLocale::En, &view),
        );
        assert!(!detail.contains('\u{202e}'));
        assert!(!detail.contains('\n'));
        assert!(!detail.contains('\u{200b}'));
        assert!(detail.contains("ws 1"));
    }
}
