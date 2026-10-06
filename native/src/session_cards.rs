use crate::herdr_protocol::AgentStatus;
use crate::i18n::{
    display_status_label, offline_status, session_empty, session_filter_label,
    session_local_source, session_pane, session_source, session_status_label, session_summary,
    text, Message, SessionFilterLabel, SessionStatusLabel, UiLocale,
};
use crate::preferences::{BubbleAppearance, BubbleColor, BubblePalette};
use crate::session_view::{
    Availability, DisplayStatus, SessionFilter, SessionKey, SessionSnapshot, SessionStatusSummary,
    SessionView,
};
use crate::state::AppState;
use crate::status_indicator::{semantic_color, StatusIcon, STATUS_ICON_GAP, STATUS_ICON_SIZE};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::Message as _;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSBezierPath, NSBorderType, NSColor, NSControlSize, NSEvent, NSFont,
    NSLineBreakMode, NSPopUpButton, NSScrollElasticity, NSScrollView, NSScrollerStyle,
    NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSString};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

const DEFAULT_FRAME_WIDTH: f64 = 360.0;
const DEFAULT_FRAME_HEIGHT: f64 = 220.0;
const OUTER_INSET: f64 = 8.0;
const HEADER_HEIGHT: f64 = 27.0;
const HEADER_GAP: f64 = 5.0;
const SUMMARY_HEIGHT: f64 = 40.0;
const ROW_HEIGHT: f64 = 46.0;
const EMPTY_HEIGHT: f64 = 20.0;
const ROW_GAP: f64 = 0.0;
const ROW_HORIZONTAL_INSET: f64 = 12.0;
const ROW_TEXT_HEIGHT: f64 = 16.0;
const ROW_TEXT_INSET: f64 = 5.0;
const ROW_SELECTION_INSET: f64 = 1.0;
const ROW_SELECTION_RADIUS: f64 = 8.0;
const BADGE_GAP: f64 = 8.0;

/// Height needed to show the filter/summary and one complete selectable row.
pub(crate) const fn minimum_selectable_height() -> f64 {
    OUTER_INSET * 2.0 + SUMMARY_HEIGHT + HEADER_GAP + ROW_HEIGHT
}

#[derive(Default)]
struct CardsIntent {
    composition_active: bool,
    pending_selection: Option<SessionKey>,
    pending_filter: Option<SessionFilter>,
    pending_selection_deferred: bool,
    deferred_refresh: bool,
    deferred_selection_applied: bool,
}
impl CardsIntent {
    fn request_selection(&mut self, key: SessionKey, marked: bool) {
        self.pending_selection = Some(key);
        self.pending_selection_deferred = true;
        self.composition_active |= marked;
        self.deferred_refresh = true;
    }
}

struct SessionCardsRootIvars {
    inner: Weak<RefCell<SessionCardsInner>>,
    intent: Rc<RefCell<CardsIntent>>,
    popup: Retained<NSPopUpButton>,
    applied_filter: Cell<SessionFilter>,
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
            let marked = crate::ui::composer_is_composing();
            let Some(inner) = self.ivars().inner.upgrade() else {
                return;
            };
            {
                let mut intent = self.ivars().intent.borrow_mut();
                intent.pending_filter = Some(filter);
                intent.deferred_refresh = true;
                intent.composition_active |= marked;
            }
            // AppKit may reenter while the cards are borrowed.
            self.restore_popup();
            if !marked {
                let ready = if let Ok(mut cards) = inner.try_borrow_mut() {
                    if !self.ivars().intent.borrow().composition_active {
                        cards.refresh();
                    }
                    true
                } else {
                    false
                };
                if ready {
                    crate::ui::cards_content_changed();
                } else {
                    crate::ui::wake();
                }
            }
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
        let marked = crate::ui::composer_is_composing();
        let Some(inner) = self.ivars().inner.upgrade() else {
            return;
        };
        {
            let mut intent = self.ivars().intent.borrow_mut();
            intent.request_selection(self.ivars().key.clone(), marked);
        }
        if marked {
            return;
        }
        let ready = if let Ok(mut cards) = inner.try_borrow_mut() {
            if !self.ivars().intent.borrow().composition_active {
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
        popup: Retained<NSPopUpButton>,
        inner: Weak<RefCell<SessionCardsInner>>,
        intent: Rc<RefCell<CardsIntent>>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SessionCardsRootIvars {
            inner,
            intent,
            popup,
            applied_filter: Cell::new(SessionFilter::All),
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
pub(crate) fn is_card_header_hit(hit: &NSView, window_point: NSPoint) -> bool {
    let Some(row) = hit.downcast_ref::<SessionCardView>() else {
        return false;
    };
    let point = row.convertPoint_fromView(window_point, None);
    point.y >= row.ivars().reply_height.get()
        && point.y <= row.bounds().size.height
        && point.x >= 0.0
        && point.x <= row.bounds().size.width
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
    palette: BubblePalette,
    show_status_indicators: bool,
    rendered_show_status_indicators: Option<bool>,
    status_summary: SessionStatusSummary,
    selected: Option<SessionKey>,
    last_revision: Option<u64>,
    selection_epoch: u64,
    selected_target_cache: Option<((Option<u64>, u64, UiLocale), String)>,
    rendered_locale: Option<UiLocale>,
    rendered_filter: Option<SessionFilter>,
    rendered_selected: Option<SessionKey>,
    reply: Option<ReplySlot>,
    reveal_selection: bool,
    rows: Vec<Retained<SessionCardView>>,
}

impl SessionCardsInner {
    fn content_height(&self) -> f64 {
        let document_height = self.rows_height();
        (OUTER_INSET * 2.0 + SUMMARY_HEIGHT + HEADER_GAP + document_height).min(180.0)
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
        let header_y = (height - inset - SUMMARY_HEIGHT).max(0.0);
        let header_width = (width - 2.0 * inset).max(1.0);
        let popup_width = (header_width * 0.42).clamp(82.0, 154.0).min(header_width);
        let popup_y = header_y + ((SUMMARY_HEIGHT - HEADER_HEIGHT) / 2.0).max(0.0);
        let popup_frame = NSRect::new(
            NSPoint::new(inset, popup_y),
            objc2_foundation::NSSize::new(popup_width, HEADER_HEIGHT),
        );
        self.root.ivars().popup.setFrame(popup_frame);

        let summary_x = (inset + popup_width + HEADER_GAP).min(width - inset);
        let summary_width = (width - inset - summary_x).max(1.0);
        self.summary.setFrame(NSRect::new(
            NSPoint::new(summary_x, header_y),
            objc2_foundation::NSSize::new(summary_width, SUMMARY_HEIGHT),
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
        let popup = &self.root.ivars().popup;
        let (red, green, blue) = palette.surface.rgb();
        let dark = 0.2126 * red + 0.7152 * green + 0.0722 * blue < 0.45;
        let appearance_name = unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        };
        popup.setAppearance(NSAppearance::appearanceNamed(appearance_name).as_deref());
        popup.setContentTintColor(Some(&palette_color(palette.text, 1.0)));
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
        let scroll_origin = self.scroll.contentView().bounds().origin;
        let (marked, pending_filter, pending_selection, deferred) = {
            let intent = self.intent.borrow();
            (
                intent.composition_active,
                intent.pending_filter,
                if intent.composition_active {
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
        let (revision, snapshot, all_rows, marked_views, selection_valid) = match self.shared.lock()
        {
            Ok(state) => {
                let revision = state.session_revision();
                if (marked || !deferred)
                    && self.last_revision == Some(revision)
                    && self.rendered_locale == Some(self.locale)
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
                let mut snapshot = state.session_snapshot(filter, self.selected.as_ref());
                let selection_valid = pending_selection
                    .as_ref()
                    .is_some_and(|key| selection_visible(&snapshot, key));
                if !marked && selection_valid {
                    snapshot.selected = pending_selection.clone();
                }
                let marked_views = if marked {
                    self.rows
                        .iter()
                        .filter_map(|row| state.session_view_for_key(row.key()))
                        .collect()
                } else {
                    Vec::new()
                };
                let all_rows = if filter != SessionFilter::All {
                    Some(state.session_snapshot(SessionFilter::All, None).rows)
                } else {
                    None
                };
                (revision, snapshot, all_rows, marked_views, selection_valid)
            }
            Err(_) => return,
        };

        if marked {
            // Preserve every displayed row's identity and its attached editor;
            // update live status without accepting a new row order or cap.
            // Titles are still disambiguated against every session, not just
            // the frozen rows, so they do not shift while composing.
            self.status_summary = snapshot.status_summary;
            self.update_filter_popup();
            self.update_summary(&snapshot, !self.rows.is_empty());
            let displays = card_displays_against(
                self.locale,
                &marked_views,
                all_rows.as_deref().unwrap_or(&snapshot.rows),
            );
            for row in &self.rows {
                if let Some(index) = marked_views.iter().position(|view| view.key == *row.key()) {
                    let view = &marked_views[index];
                    let display = &displays[index];
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
            self.rendered_selected = self.selected.clone();
            self.rendered_show_status_indicators = Some(self.show_status_indicators);
            self.intent.borrow_mut().deferred_refresh = true;
            return;
        }

        {
            let mut intent = self.intent.borrow_mut();
            intent.pending_filter = None;
            intent.pending_selection = None;
            intent.deferred_refresh = false;
            intent.deferred_selection_applied |=
                selection_valid && intent.pending_selection_deferred;
            intent.pending_selection_deferred = false;
        }
        self.filter = filter;
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
        self.update_filter_popup();
        self.update_summary(&snapshot, !snapshot.rows.is_empty());
        self.update_rows(&snapshot, all_rows.as_deref().unwrap_or(&snapshot.rows));
        self.last_revision = Some(revision);
        self.rendered_locale = Some(self.locale);
        self.rendered_filter = Some(self.filter);
        self.rendered_show_status_indicators = Some(self.show_status_indicators);
        self.rendered_selected = self.selected.clone();
        self.layout(self.root.frame().size, scroll_origin);
        self.reveal_selection = false;
    }

    fn update_filter_popup(&self) {
        self.root.ivars().applied_filter.set(self.filter);
        for (index, filter) in SessionFilter::ALL.into_iter().enumerate() {
            let title = NSString::from_str(filter_label(self.locale, filter));
            if let Some(item) = self.root.ivars().popup.itemAtIndex(index as isize) {
                item.setTitle(&title);
            }
        }
        let index = SessionFilter::ALL
            .iter()
            .position(|filter| *filter == self.filter)
            .unwrap_or(0) as isize;
        let selected_index: isize =
            unsafe { msg_send![&*self.root.ivars().popup, indexOfSelectedItem] };
        if selected_index != index {
            let _: () = unsafe { msg_send![&*self.root.ivars().popup, selectItemAtIndex: index] };
        }
    }

    fn update_summary(&self, snapshot: &SessionSnapshot, visible_rows: bool) {
        let text = session_summary(
            self.locale,
            snapshot.total,
            snapshot.matched,
            snapshot.omitted,
        );
        self.summary.setStringValue(&NSString::from_str(&text));

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

    fn update_rows(&mut self, snapshot: &SessionSnapshot, all_rows: &[SessionView]) {
        let displays = card_displays_against(self.locale, &snapshot.rows, all_rows);
        for (index, (view, display)) in snapshot.rows.iter().zip(&displays).enumerate() {
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
            let root =
                SessionCardsRoot::new(default_frame, popup, weak.clone(), Rc::clone(&intent), mtm);
            unsafe {
                let _: () = msg_send![&*root.ivars().popup, setTarget: Some(&*root)];
                let _: () = msg_send![&*root.ivars().popup, setAction: sel!(filterChanged:)];
            }

            let summary = label(&session_summary(locale, 0, 0, 0), 11.5, mtm);
            summary.setAlignment(NSTextAlignment::Right);
            summary.setTextColor(Some(&palette_color(palette.muted, 1.0)));

            let empty = label(session_empty(locale, 0, 0), 12.0, mtm);
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
                palette,
                show_status_indicators: true,
                rendered_show_status_indicators: None,
                status_summary: SessionStatusSummary::default(),
                selected: None,
                last_revision: None,
                selection_epoch: 0,
                selected_target_cache: None,
                rendered_locale: None,
                rendered_filter: None,
                rendered_selected: None,
                reply: None,
                reveal_selection: false,
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
    pub(crate) fn selection_stamp(&self) -> (Option<u64>, u64, UiLocale) {
        let inner = self.inner.borrow();
        (inner.last_revision, inner.selection_epoch, inner.locale)
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
        let snapshot = {
            let state = shared.lock().ok()?;
            let snapshot = state.session_snapshot(SessionFilter::All, Some(&key));
            if snapshot.selected.as_ref() != Some(&key) {
                return None;
            }
            snapshot
        };
        let title = snapshot
            .rows
            .iter()
            .position(|row| row.key == key)
            .and_then(|index| {
                card_displays(locale, &snapshot.rows)
                    .get(index)
                    .map(|display| display.title.clone())
            })
            .unwrap_or_else(|| key.terminal_id.clone());
        let mut inner = self.inner.borrow_mut();
        if (inner.last_revision, inner.selection_epoch, inner.locale) == stamp {
            inner.selected_target_cache = Some((stamp, title.clone()));
        }
        Some((key, title))
    }

    pub(crate) fn set_frame(&self, frame: NSRect) {
        self.inner.borrow_mut().set_frame(frame);
    }

    pub(crate) fn set_locale(&self, locale: UiLocale) {
        self.inner.borrow_mut().set_locale(locale);
    }
    pub(crate) fn set_composition_active(&self, active: bool) {
        let mut intent = self.intent.borrow_mut();
        if intent.composition_active && !active {
            intent.deferred_refresh = true;
        }
        intent.composition_active = active;
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
    pub(crate) fn attach_reply(&self, key: &SessionKey, view: &NSView, height: f64) -> bool {
        let mut inner = self.inner.borrow_mut();
        if self.intent.borrow().composition_active
            && !inner
                .reply
                .as_ref()
                .is_some_and(|reply| reply.key == *key && std::ptr::eq(&*reply.view, view))
        {
            self.intent.borrow_mut().deferred_refresh = true;
            return false;
        }
        inner.attach_reply(key, view, height)
    }

    pub(crate) fn detach_reply(&self) {
        if self.intent.borrow().composition_active {
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
                objc2_foundation::NSSize::new(140.0, HEADER_HEIGHT),
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

struct CardDisplay {
    title: String,
    context: String,
    directory_fallback: bool,
    directory_context: bool,
}

fn display_value(value: Option<&str>) -> Option<String> {
    let value = value?;
    let mut result = String::with_capacity(value.len());
    let mut space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            space = !result.is_empty();
        } else if !unsafe_identifier_char(character) {
            if space {
                result.push(' ');
                space = false;
            }
            result.push(character);
        }
    }
    (!result.is_empty()).then_some(result)
}

fn meaningful_title(value: Option<&str>) -> Option<String> {
    let title = display_value(value)?;
    if [
        "omp",
        "claude",
        "claude code",
        "codex",
        "openai codex",
        "terminal",
        "shell",
        "bash",
        "zsh",
        "fish",
        "sh",
        "nu",
        "pwsh",
        "powershell",
    ]
    .iter()
    .any(|generic| title.eq_ignore_ascii_case(generic))
    {
        return None;
    }
    Some(title)
}

fn tab_location(locale: UiLocale, view: &SessionView) -> String {
    match display_value(view.metadata.tab_label.as_deref()) {
        Some(label) if label.chars().all(|c| c.is_ascii_digit()) => {
            format!("{} {label}", text(locale, Message::Tab))
        }
        Some(label) => label,
        None => text(locale, Message::UnnamedTab).to_owned(),
    }
}

fn directory_suffix(path: &str, components: usize) -> String {
    let parts: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    parts[parts.len().saturating_sub(components)..].join("/")
}

fn card_source_label<'a>(locale: UiLocale, view: &'a SessionView) -> Cow<'a, str> {
    if view.is_local {
        Cow::Borrowed(session_local_source(locale))
    } else {
        display_value(Some(&view.source_label))
            .map(Cow::Owned)
            .unwrap_or_else(|| Cow::Owned(format!("#{}", view.key.source_id)))
    }
}

/// Card displays for `rows` (in order), disambiguated against every session
/// (`all_rows`, the unfiltered snapshot) rather than only the rows on screen,
/// so titles stay stable across filters and while IME composition freezes rows.
fn card_displays_against(
    locale: UiLocale,
    rows: &[SessionView],
    all_rows: &[SessionView],
) -> Vec<CardDisplay> {
    let mut shared: Vec<Option<CardDisplay>> = card_displays(locale, all_rows)
        .into_iter()
        .map(Some)
        .collect();
    rows.iter()
        .map(|view| {
            all_rows
                .iter()
                .position(|row| row.key == view.key)
                .and_then(|position| shared[position].take())
                // An unfiltered snapshot is capped; filtered rows past that cap remain readable.
                .unwrap_or_else(|| card_displays(locale, std::slice::from_ref(view)).remove(0))
        })
        .collect()
}

fn card_displays(locale: UiLocale, rows: &[SessionView]) -> Vec<CardDisplay> {
    let cwd_paths: Vec<_> = rows
        .iter()
        .map(|view| display_value(view.metadata.cwd.as_deref()))
        .collect();
    let mut displays: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, view)| {
            let workspace = display_value(view.metadata.workspace_label.as_deref());
            let agent = display_value(view.metadata.agent.as_deref());
            let cwd = cwd_paths[index].as_deref();
            let directory = cwd
                .map(|path| directory_suffix(path, 1))
                .filter(|name| !name.is_empty());
            let tab = tab_location(locale, view);
            let title = meaningful_title(view.metadata.title.as_deref()).or_else(|| {
                meaningful_title(view.metadata.tab_label.as_deref())
                    .filter(|label| !label.chars().all(|c| c.is_ascii_digit()))
            });
            let directory_context = workspace.is_none() && directory.is_some();
            let directory_fallback = title.is_none() && directory_context;
            let title = title.unwrap_or_else(|| {
                workspace
                    .clone()
                    .or(directory.clone())
                    .map(|base| format!("{base} · {tab}"))
                    .unwrap_or_else(|| text(locale, Message::UnnamedSession).to_owned())
            });
            let context = [workspace.or(directory), Some(tab), agent]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            CardDisplay {
                title,
                context,
                directory_fallback,
                directory_context,
            }
        })
        .collect();

    // Compare original basenames before changing either display, so both
    // sides of a directory collision acquire sufficient parent context.
    let original: Vec<_> = displays
        .iter()
        .map(|display| (display.title.clone(), display.context.clone()))
        .collect();
    for i in 0..rows.len() {
        if !displays[i].directory_context {
            continue;
        }
        let Some(path) = cwd_paths[i].as_deref() else {
            continue;
        };
        let peers: Vec<_> = (0..rows.len())
            .filter(|&j| j != i && original[j] == original[i] && displays[j].directory_context)
            .collect();
        if peers.is_empty() {
            continue;
        }
        let mut count = 1;
        while peers.iter().any(|&j| {
            cwd_paths[j].as_deref().is_some_and(|other| {
                directory_suffix(other, count) == directory_suffix(path, count)
            })
        }) {
            count += 1;
            if directory_suffix(path, count) == directory_suffix(path, count - 1) {
                break;
            }
        }
        let directory = directory_suffix(path, count);
        if displays[i].directory_fallback {
            displays[i].title = format!("{directory} · {}", tab_location(locale, &rows[i]));
        }
        let basename = directory_suffix(path, 1);
        if let Some(rest) = displays[i].context.strip_prefix(&basename) {
            displays[i].context = format!("{directory}{rest}");
        }
    }
    let collision_names: Vec<_> = displays
        .iter()
        .map(|display| (display.title.clone(), display.context.clone()))
        .collect();
    // Compare against every row (not just the current filter). IDs stay off
    // ordinary cards; only indistinguishable title/context pairs need one.
    // Put the marker first in the muted context: long titles and contexts
    // truncate at their tails, which must never hide the distinction.
    for i in 0..rows.len() {
        let collisions: Vec<_> = (0..rows.len())
            .filter(|&j| j != i && collision_names[j] == collision_names[i])
            .collect();
        // Labels are presentation only; equality above uses original title
        // and context so shared or renamed machine names cannot hide a clash.
        let label = card_source_label(locale, &rows[i]);
        if collision_names[i].0 == text(locale, Message::UnnamedSession) || !collisions.is_empty() {
            let source = rows[i].key.source_id;
            let same_source = collisions.iter().any(|&j| rows[j].key.source_id == source);
            let discriminator = if same_source {
                // A compact rank stays visible even when terminal IDs share
                // an arbitrarily long prefix; sort by stable session key, not
                // current row order or filter position.
                let rank = 1 + collisions
                    .iter()
                    .filter(|&&j| rows[j].key.source_id == source && rows[j].key < rows[i].key)
                    .count();
                format!("#{source} · {rank}")
            } else {
                format!("#{source}")
            };
            displays[i].context = format!("({discriminator}) · {label} · {}", displays[i].context);
        } else {
            displays[i].context = format!("{label} · {}", displays[i].context);
        }
    }
    displays
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

fn unsafe_identifier_char(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr_protocol::SessionMetadata;

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
    fn filtered_and_composing_cards_disambiguate_against_every_session() {
        let all = [
            row(1, "abc", Some("Refactor"), Some("1"), Some("Idea"), None),
            row(1, "abd", Some("Refactor"), Some("1"), Some("Idea"), None),
            row(2, "a", None, Some("1"), None, Some("/alpha/src")),
            row(3, "b", None, Some("1"), None, Some("/beta/src")),
        ];
        let extra = row(4, "late", None, Some("2"), None, Some("/gamma/src"));
        let displayed = [all[2].clone(), all[0].clone(), extra.clone()];
        let full = card_displays(UiLocale::En, &all);
        let visible_only = card_displays(UiLocale::En, &displayed);
        let cards = card_displays_against(UiLocale::En, &displayed, &all);

        assert_eq!(cards.len(), displayed.len());
        assert_eq!(visible_only[0].title, "src · Tab 1");
        assert_eq!(cards[0].title, "alpha/src · Tab 1");
        assert_eq!(cards[0].title, full[2].title);
        assert_eq!(cards[0].context, full[2].context);
        assert!(!visible_only[1].context.starts_with("(#"));
        assert!(cards[1].context.starts_with("(#1 · 1) · "));
        assert_eq!(cards[1].title, full[0].title);
        assert_eq!(cards[1].context, full[0].context);
        let fallback = card_displays(UiLocale::En, std::slice::from_ref(&extra)).remove(0);
        assert_eq!(cards[2].title, fallback.title);
        assert_eq!(cards[2].context, fallback.context);
        assert_eq!(cards[2].title, "src · Tab 2");
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
