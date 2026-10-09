//! Native AppKit dark character browser window and layout.
//!
//! Provides a retained resizable window with search, filtering (All / Installed / Official),
//! reusable character cards, bounded portrait thumbnail loading, and a pinned
//! candidate Apply/Cancel footer.

use crate::character_menu::{self, MenuCommand};
use crate::character_preview::{CharacterPreviews, PreviewKey, PreviewSource, PreviewState};
use crate::character_selection::{CharacterSelection, OperationStatus};
use crate::character_service::DownloadProgress;
use crate::character_types::{CharacterRef, OfficialPackIdentity, PackListing, PackRecord};
use crate::i18n::{self, Message, UiLocale};
use crate::official_catalog::OfficialEntry;
use crate::ui::MenuTarget;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message as ObjcMessage,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezelStyle, NSBezierPath,
    NSButton, NSColor, NSControlSize, NSEvent, NSFont, NSImage, NSImageScaling, NSImageView,
    NSLineBreakMode, NSMenu, NSPopUpButton, NSScrollElasticity, NSScrollView, NSScrollerStyle,
    NSSearchField, NSTextField, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Theme Colors (consistent with menu_panel.rs and dialogue_editor.rs)
// ---------------------------------------------------------------------------

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

const ACCENT_RED: f64 = 0.29;
const ACCENT_GREEN: f64 = 0.56;
const ACCENT_BLUE: f64 = 0.89;

const ERROR_RED: f64 = 0.95;
const ERROR_GREEN: f64 = 0.36;
const ERROR_BLUE: f64 = 0.36;

const CARD_RADIUS: f64 = 8.0;
const THUMB_RADIUS: f64 = 6.0;

fn color(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha)
}

fn bg_color() -> Retained<NSColor> {
    color(BG_RED, BG_GREEN, BG_BLUE, 1.0)
}

fn card_color() -> Retained<NSColor> {
    color(CARD_RED, CARD_GREEN, CARD_BLUE, 1.0)
}

fn card_border_color() -> Retained<NSColor> {
    color(
        CARD_BORDER_RED,
        CARD_BORDER_GREEN,
        CARD_BORDER_BLUE,
        CARD_BORDER_ALPHA,
    )
}

fn track_color() -> Retained<NSColor> {
    color(TRACK_RED, TRACK_GREEN, TRACK_BLUE, 1.0)
}

fn primary_color() -> Retained<NSColor> {
    color(PRIMARY_RED, PRIMARY_GREEN, PRIMARY_BLUE, 1.0)
}

fn secondary_color() -> Retained<NSColor> {
    color(SECONDARY_RED, SECONDARY_GREEN, SECONDARY_BLUE, 1.0)
}

fn accent_color() -> Retained<NSColor> {
    color(ACCENT_RED, ACCENT_GREEN, ACCENT_BLUE, 1.0)
}

fn error_color() -> Retained<NSColor> {
    color(ERROR_RED, ERROR_GREEN, ERROR_BLUE, 1.0)
}

// ---------------------------------------------------------------------------
// Rect Helpers
// ---------------------------------------------------------------------------

fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(width.max(1.0), height.max(1.0)),
    )
}

fn inset_rect(rect: NSRect, inset: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
        NSSize::new(
            (rect.size.width - inset * 2.0).max(0.0),
            (rect.size.height - inset * 2.0).max(0.0),
        ),
    )
}

fn ax(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}

fn ax_id(view: &NSView, identifier: &str) {
    let identifier = NSString::from_str(identifier);
    unsafe {
        let _: () = msg_send![view, setAccessibilityIdentifier: Some(&*identifier)];
    }
}

fn set_tooltip(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setToolTip: Some(&*label)];
    }
}

fn label(
    value: &str,
    size: f64,
    bold: bool,
    color: Retained<NSColor>,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field.setTextColor(Some(&color));
    field
}

fn resolve_builtin_thumbnail_path() -> Option<PathBuf> {
    if let Some(contents) = crate::bundle::contents_dir() {
        let bundle_path = contents.join("Resources/default-thumbnail.png");
        if bundle_path.is_file() {
            return Some(bundle_path);
        }
    }
    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/rubelia-thumbnail.png");
    if dev_path.is_file() {
        return Some(dev_path);
    }
    let cwd_path = Path::new("assets/rubelia-thumbnail.png");
    if cwd_path.is_file() {
        return Some(cwd_path.to_path_buf());
    }
    None
}

fn load_builtin_thumbnail() -> Option<Retained<NSImage>> {
    let path = resolve_builtin_thumbnail_path()?;
    let path_ns = NSString::from_str(&path.to_string_lossy());
    NSImage::initWithContentsOfFile(NSImage::alloc(), &path_ns)
}

// ---------------------------------------------------------------------------
// Public Browser Input DTO & Selectors Interface (consumed by ui.rs)
// ---------------------------------------------------------------------------

pub struct BrowserInput<'a> {
    pub listing: &'a PackListing,
    pub selection: &'a CharacterSelection,
    pub busy: bool,
    pub official_entries: &'a [OfficialEntry],
    pub catalog_revision: u64,
    pub catalog_error: Option<&'a str>,
    pub progress: Option<&'a DownloadProgress>,
    pub previews: &'a CharacterPreviews,
    pub active_preview_key: Option<&'a PreviewKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserOfficialIntent {
    pub identity: OfficialPackIdentity,
    pub catalog_revision: u64,
    pub listing_generation: u64,
}

pub(crate) fn official_from_sender(sender: Option<&AnyObject>) -> Option<BrowserOfficialIntent> {
    let button = sender?.downcast_ref::<NSButton>()?;
    let represented = button.cell()?.representedObject()?;
    let payload = represented.downcast_ref::<NSString>()?.to_string();
    let data = payload.strip_prefix("official:")?;
    if data.len() > 2048 {
        return None;
    }
    let (catalog_revision, listing_generation, identity) =
        serde_json::from_str::<(u64, u64, OfficialPackIdentity)>(data).ok()?;
    Some(BrowserOfficialIntent {
        identity,
        catalog_revision,
        listing_generation,
    })
}

pub(crate) fn cancel_id_from_sender(sender: Option<&AnyObject>) -> Option<String> {
    let button = sender?.downcast_ref::<NSButton>()?;
    let represented = button.cell()?.representedObject()?;
    let payload = represented.downcast_ref::<NSString>()?.to_string();
    let id = payload.strip_prefix("cancel:")?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return None;
    }
    Some(id.to_owned())
}

fn encode_official(intent: &BrowserOfficialIntent) -> String {
    format!(
        "official:{}",
        serde_json::to_string(&(
            intent.catalog_revision,
            intent.listing_generation,
            &intent.identity
        ))
        .expect("official identity is serializable")
    )
}

fn set_button_payload(button: &NSButton, payload: &str) {
    let str_ns = NSString::from_str(payload);
    if let Some(cell) = button.cell() {
        unsafe {
            cell.setRepresentedObject(Some(str_ns.as_ref()));
        }
    }
}

// ---------------------------------------------------------------------------
// AppKit View & Window Classes
// ---------------------------------------------------------------------------

define_class!(
    #[unsafe(super = NSWindow)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserWindow"]
    #[ivars = Cell<bool>]
    struct CharacterBrowserWindow;

    unsafe impl NSObjectProtocol for CharacterBrowserWindow {}

    impl CharacterBrowserWindow {
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            if self.ivars().get() {
                return;
            }
            if !self.search_has_marked_text() {
                self.orderOut(None);
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if self.ivars().get() {
                return;
            }
            if event.keyCode() == 53 && !self.search_has_marked_text() {
                self.orderOut(None);
                return;
            }
            let _: () = unsafe { msg_send![super(self), keyDown: event] };
        }
    }
);

impl CharacterBrowserWindow {
    fn search_has_marked_text(&self) -> bool {
        self.firstResponder()
            .and_then(|responder| {
                responder.downcast_ref::<NSTextView>().map(|text| {
                    let marked: bool = unsafe { msg_send![text, hasMarkedText] };
                    marked
                })
            })
            .unwrap_or(false)
    }
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(false));
        unsafe {
            msg_send![super(this),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::Miniaturizable,
                backing: NSBackingStoreType::Buffered,
                defer: false]
        }
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserDocument"]
    struct CharacterBrowserDocument;

    unsafe impl NSObjectProtocol for CharacterBrowserDocument {}

    impl CharacterBrowserDocument {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl CharacterBrowserDocument {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserCardView"]
    struct CharacterBrowserCardView;

    unsafe impl NSObjectProtocol for CharacterBrowserCardView {}

    impl CharacterBrowserCardView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                bounds,
                CARD_RADIUS,
                CARD_RADIUS,
            )
            .fill();

            card_border_color().setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                (CARD_RADIUS - 0.5).max(0.0),
                (CARD_RADIUS - 0.5).max(0.0),
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl CharacterBrowserCardView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserThumbView"]
    struct CharacterBrowserThumbView;

    unsafe impl NSObjectProtocol for CharacterBrowserThumbView {}

    impl CharacterBrowserThumbView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            track_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                bounds,
                THUMB_RADIUS,
                THUMB_RADIUS,
            )
            .fill();

            card_border_color().setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                (THUMB_RADIUS - 0.5).max(0.0),
                (THUMB_RADIUS - 0.5).max(0.0),
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl CharacterBrowserThumbView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserFooterView"]
    struct CharacterBrowserFooterView;

    unsafe impl NSObjectProtocol for CharacterBrowserFooterView {}

    impl CharacterBrowserFooterView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::fillRect(bounds);

            card_border_color().setStroke();
            let sep_path = NSBezierPath::bezierPath();
            sep_path.moveToPoint(NSPoint::new(0.0, 0.5));
            sep_path.lineToPoint(NSPoint::new(bounds.size.width, 0.5));
            sep_path.setLineWidth(1.0);
            sep_path.stroke();
        }
    }
);

impl CharacterBrowserFooterView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

// ---------------------------------------------------------------------------
// Internal Target for Search & Filter Controls
// ---------------------------------------------------------------------------

define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetBrowserActionTarget"]
    #[ivars = RefCell<Option<Rc<RefCell<BrowserSharedState>>>>]
    struct BrowserActionTarget;

    unsafe impl NSObjectProtocol for BrowserActionTarget {}

    impl BrowserActionTarget {
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, sender: Option<&AnyObject>) {
            let ivars = self.ivars().borrow();
            let Some(shared_rc) = ivars.as_ref() else { return };
            if let Some(sender) = sender {
                if let Some(field) = sender.downcast_ref::<NSTextField>() {
                    let shared = shared_rc.borrow();
                    if shared.update_frozen {
                        if let Some(original) = shared.frozen_search_value.as_deref() {
                            if field.stringValue().to_string() != original {
                                field.setStringValue(&NSString::from_str(original));
                            }
                        }
                        return;
                    }
                    drop(shared);
                    shared_rc.borrow_mut().change_query(field.stringValue().to_string());
                }
            }
        }

        #[unsafe(method(filterAll:))]
        fn filter_all(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars().borrow();
            let Some(shared_rc) = ivars.as_ref() else { return };
            shared_rc.borrow_mut().change_filter(BrowserFilter::All);
        }

        #[unsafe(method(filterInstalled:))]
        fn filter_installed(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars().borrow();
            let Some(shared_rc) = ivars.as_ref() else { return };
            shared_rc.borrow_mut().change_filter(BrowserFilter::Installed);
        }

        #[unsafe(method(filterOfficial:))]
        fn filter_official(&self, _sender: Option<&AnyObject>) {
            let ivars = self.ivars().borrow();
            let Some(shared_rc) = ivars.as_ref() else { return };
            shared_rc.borrow_mut().change_filter(BrowserFilter::Official);
        }
    }
);

impl BrowserActionTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// Root Container View with Layout Tracking
// ---------------------------------------------------------------------------

struct LayoutRefs {
    search_field: Retained<NSSearchField>,
    filter_all_button: Retained<NSButton>,
    filter_installed_button: Retained<NSButton>,
    filter_official_button: Retained<NSButton>,
    status_label: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    document: Retained<CharacterBrowserDocument>,
    footer: Retained<CharacterBrowserFooterView>,
    candidate_thumb: Retained<CharacterBrowserThumbView>,
    candidate_image: Retained<NSImageView>,
    candidate_title: Retained<NSTextField>,
    candidate_status: Retained<NSTextField>,
    import_button: Retained<NSButton>,
    cancel_button: Retained<NSButton>,
    apply_button: Retained<NSButton>,
    empty_label: Retained<NSTextField>,
    shared: Rc<RefCell<BrowserSharedState>>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterBrowserRoot"]
    #[ivars = RefCell<Option<LayoutRefs>>]
    struct CharacterBrowserRoot;

    unsafe impl NSObjectProtocol for CharacterBrowserRoot {}

    impl CharacterBrowserRoot {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            self.arrange();
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.arrange();
        }
    }
);

impl CharacterBrowserRoot {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn arrange(&self) {
        let ivars = self.ivars().borrow();
        let Some(refs) = ivars.as_ref() else { return };
        let bounds = self.bounds();
        let w = bounds.size.width;
        let h = bounds.size.height;

        let margin_x = 16.0;
        let header_h = 66.0;
        let footer_h = 74.0;

        // 1. Search and Filter row
        let all_w = 52.0;
        let inst_w = 86.0;
        let off_w = 112.0;
        let gap = 6.0;
        let filter_total_w = all_w + inst_w + off_w + gap * 2.0;
        let search_w = (w - margin_x * 2.0 - filter_total_w - 14.0).max(120.0);

        refs.search_field
            .setFrame(frame(margin_x, 10.0, search_w, 26.0));
        let filter_x = margin_x + search_w + 14.0;
        refs.filter_all_button
            .setFrame(frame(filter_x, 10.0, all_w, 26.0));
        refs.filter_installed_button
            .setFrame(frame(filter_x + all_w + gap, 10.0, inst_w, 26.0));
        refs.filter_official_button.setFrame(frame(
            filter_x + all_w + inst_w + gap * 2.0,
            10.0,
            off_w,
            26.0,
        ));

        // 2. Status Label (character count, catalog error)
        refs.status_label
            .setFrame(frame(margin_x, 42.0, (w - margin_x * 2.0).max(10.0), 18.0));

        // 3. Scroll View
        let scroll_y = header_h;
        let scroll_h = (h - scroll_y - footer_h).max(1.0);
        refs.scroll
            .setFrame(frame(0.0, scroll_y, w.max(1.0), scroll_h));

        // 4. Footer
        let footer_y = (h - footer_h).max(0.0);
        refs.footer
            .setFrame(frame(0.0, footer_y, w.max(1.0), footer_h));

        // Footer children
        refs.candidate_thumb.setFrame(frame(16.0, 16.0, 42.0, 42.0));

        let apply_w = refs.apply_button.intrinsicContentSize().width.max(96.0);
        let cancel_w = refs.cancel_button.intrinsicContentSize().width.max(96.0);
        let import_w = refs.import_button.intrinsicContentSize().width.max(96.0);
        let btn_h = 28.0;
        let btn_y = 23.0;
        let apply_x = (w - 16.0 - apply_w).max(0.0);
        let cancel_x = (apply_x - 8.0 - cancel_w).max(0.0);
        let import_x = (cancel_x - 8.0 - import_w).max(0.0);

        refs.apply_button
            .setFrame(frame(apply_x, btn_y, apply_w, btn_h));
        refs.cancel_button
            .setFrame(frame(cancel_x, btn_y, cancel_w, btn_h));
        refs.import_button
            .setFrame(frame(import_x, btn_y, import_w, btn_h));

        let cand_text_x = 16.0 + 42.0 + 12.0;
        let cand_text_w = (import_x - cand_text_x - 12.0).max(40.0);
        refs.candidate_title
            .setFrame(frame(cand_text_x, 16.0, cand_text_w, 20.0));
        refs.candidate_status
            .setFrame(frame(cand_text_x, 38.0, cand_text_w, 18.0));

        // Arrange cards within the document view
        arrange_cards_layout(refs, w);
    }
}

fn visible_range(refs: &LayoutRefs, total: usize) -> std::ops::Range<usize> {
    let visible = refs.scroll.documentVisibleRect();
    let columns = if refs.scroll.contentSize().width >= 620.0 {
        2
    } else {
        1
    };
    let start_row = ((visible.origin.y - 14.0).max(0.0) / 132.0) as usize;
    let start = (start_row * columns).min(total);
    let rows = ((visible.size.height / 132.0).ceil() as usize + 2).min(14 / columns);
    start..(start + rows * columns).min(total)
}

fn arrange_cards_layout(refs: &LayoutRefs, _root_width: f64) {
    let doc_w = refs.scroll.contentSize().width.max(1.0);
    let cols = if doc_w >= 620.0 { 2 } else { 1 };
    let card_w = ((doc_w - 28.0 - if cols == 2 { 10.0 } else { 0.0 }) / cols as f64)
        .floor()
        .max(100.0);
    let (total, cards) = {
        let shared = refs.shared.borrow();
        (
            shared.items.len(),
            shared
                .cards
                .iter()
                .filter_map(|card| card.item_index.map(|index| (card.clone(), index)))
                .collect::<Vec<_>>(),
        )
    };
    let rows = total.div_ceil(cols);
    let doc_h = (28.0 + rows as f64 * 132.0 - if rows == 0 { 0.0 } else { 10.0 })
        .max(refs.scroll.contentSize().height.max(1.0));
    refs.document.setFrame(frame(0.0, 0.0, doc_w, doc_h));
    refs.empty_label.setHidden(total != 0);
    refs.empty_label
        .setFrame(frame(14.0, 40.0, (doc_w - 28.0).max(10.0), 30.0));
    for (card, index) in cards {
        let x = 14.0 + (index % cols) as f64 * (card_w + 10.0);
        let y = 14.0 + (index / cols) as f64 * 132.0;
        card.view.setFrame(frame(x, y, card_w, 122.0));
        layout_card_subviews(&card, card_w, 122.0);
    }
}

fn layout_card_subviews(card: &CharacterCardHolder, card_w: f64, card_h: f64) {
    let pad = 10.0;
    let thumb_size = 72.0;
    card.thumb_view
        .setFrame(frame(pad, pad, thumb_size, thumb_size));

    let content_x = pad + thumb_size + 10.0;
    let content_w = (card_w - content_x - pad).max(10.0);

    card.name_label
        .setFrame(frame(content_x, pad, content_w, 20.0));
    card.meta_label
        .setFrame(frame(content_x, pad + 20.0, content_w, 16.0));
    card.desc_label
        .setFrame(frame(content_x, pad + 38.0, content_w, 32.0));

    // Action buttons at the bottom of the card
    let btn_h = 26.0;
    let btn_y = card_h - pad - btn_h;
    let btn_w = 110.0_f64.min(content_w * 0.55).max(75.0);

    if !card.popup_button.isHidden() {
        let popup_w = 32.0;
        let action_w = (content_w - popup_w - 6.0).min(btn_w).max(60.0);
        card.action_button.setFrame(frame(
            content_x + content_w - popup_w - 6.0 - action_w,
            btn_y,
            action_w,
            btn_h,
        ));
        card.popup_button.setFrame(frame(
            content_x + content_w - popup_w,
            btn_y,
            popup_w,
            btn_h,
        ));
    } else {
        card.action_button.setFrame(frame(
            (content_x + content_w - btn_w).max(content_x),
            btn_y,
            btn_w,
            btn_h,
        ));
    }
    card.progress_label.setFrame(frame(
        content_x,
        btn_y + 4.0,
        (content_w - btn_w - 8.0).max(10.0),
        18.0,
    ));
}

// ---------------------------------------------------------------------------
// Browser Model & Filter Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrowserFilter {
    All,
    Installed,
    Official,
}

#[derive(Clone, Debug)]
enum BrowserItemKind {
    Builtin,
    Local(PackRecord),
    Official {
        entry: OfficialEntry,
        installed: Option<PackRecord>,
    },
}

#[derive(Clone, Debug)]
struct BrowserItem {
    id: String,
    name: String,
    kind: BrowserItemKind,
    preview_key: PreviewKey,
}

impl BrowserItem {
    fn installed_pack(&self) -> Option<&PackRecord> {
        match &self.kind {
            BrowserItemKind::Local(pack) => Some(pack),
            BrowserItemKind::Official { installed, .. } => installed.as_ref(),
            BrowserItemKind::Builtin => None,
        }
    }

    fn selected_revision(&self) -> Option<u64> {
        match &self.kind {
            BrowserItemKind::Builtin => Some(0),
            _ => self.installed_pack().map(|pack| pack.head),
        }
    }
}

fn build_index(
    listing: &PackListing,
    official_entries: &[OfficialEntry],
    catalog_revision: u64,
    locale: UiLocale,
    query: &str,
    filter: BrowserFilter,
) -> Vec<BrowserItem> {
    let query = query.to_lowercase();
    let mut items = Vec::new();
    if filter != BrowserFilter::Official {
        let name = i18n::text(locale, Message::RubeliaBuiltIn);
        if query.is_empty() || name.to_lowercase().contains(&query) || "default".contains(&query) {
            items.push(BrowserItem {
                id: "default".to_owned(),
                name: name.to_owned(),
                kind: BrowserItemKind::Builtin,
                preview_key: PreviewKey {
                    source: PreviewSource::Local {
                        reference: CharacterRef::builtin(),
                        generation: listing.generation,
                    },
                    pixels: 128,
                },
            });
        }
        for pack in &listing.packs {
            if official_entries
                .iter()
                .any(|entry| entry.identity.id == pack.id)
                || !(query.is_empty()
                    || pack.name.to_lowercase().contains(&query)
                    || pack.id.to_lowercase().contains(&query))
            {
                continue;
            }
            items.push(BrowserItem {
                id: pack.id.clone(),
                name: pack.name.clone(),
                kind: BrowserItemKind::Local(pack.clone()),
                preview_key: PreviewKey {
                    source: PreviewSource::Local {
                        reference: CharacterRef {
                            id: pack.id.clone(),
                            revision: pack.head,
                        },
                        generation: listing.generation,
                    },
                    pixels: 128,
                },
            });
        }
    }
    if filter != BrowserFilter::Installed || !listing.packs.is_empty() {
        for entry in official_entries {
            let installed = listing
                .packs
                .iter()
                .find(|pack| pack.id == entry.identity.id);
            if filter == BrowserFilter::Installed && installed.is_none() {
                continue;
            }
            let desc = match locale {
                UiLocale::Ko => &entry.description.ko,
                UiLocale::En => &entry.description.en,
            };
            let variant = match locale {
                UiLocale::Ko => &entry.variant_name.ko,
                UiLocale::En => &entry.variant_name.en,
            };
            if !(query.is_empty()
                || entry.name.to_lowercase().contains(&query)
                || entry.identity.id.to_lowercase().contains(&query)
                || installed.is_some_and(|pack| pack.name.to_lowercase().contains(&query))
                || variant.to_lowercase().contains(&query)
                || desc.to_lowercase().contains(&query)
                || entry
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query)))
            {
                continue;
            }
            let source = if let Some(pack) = installed {
                PreviewSource::Local {
                    reference: CharacterRef {
                        id: pack.id.clone(),
                        revision: pack.head,
                    },
                    generation: listing.generation,
                }
            } else {
                PreviewSource::Official {
                    identity: entry.identity.clone(),
                    catalog_revision,
                    url: entry.preview_idle_url.clone(),
                }
            };
            items.push(BrowserItem {
                id: entry.identity.id.clone(),
                name: entry.name.clone(),
                kind: BrowserItemKind::Official {
                    entry: entry.clone(),
                    installed: installed.cloned(),
                },
                preview_key: PreviewKey {
                    source,
                    pixels: 128,
                },
            });
        }
    }
    items
}

fn selection_message(
    item: &BrowserItem,
    listing: &PackListing,
    selection: &CharacterSelection,
) -> Option<Message> {
    let revision = item.selected_revision()?;
    if selection.candidate().is_some_and(|candidate| {
        candidate.reference.id == item.id && candidate.reference.revision == revision
    }) {
        Some(Message::CharacterBrowserSelected)
    } else if !listing.override_active
        && listing
            .active
            .as_ref()
            .is_some_and(|active| active.id == item.id && active.revision == revision)
    {
        Some(Message::Active)
    } else {
        Some(Message::CharacterBrowserSelect)
    }
}

#[derive(Clone)]
struct CharacterCardHolder {
    view: Retained<CharacterBrowserCardView>,
    thumb_view: Retained<CharacterBrowserThumbView>,
    image_view: Retained<NSImageView>,
    name_label: Retained<NSTextField>,
    meta_label: Retained<NSTextField>,
    desc_label: Retained<NSTextField>,
    action_button: Retained<NSButton>,
    popup_button: Retained<NSPopUpButton>,
    progress_label: Retained<NSTextField>,
    item_index: Option<usize>,
    visible: bool,
}

struct BrowserSharedState {
    query: String,
    filter: BrowserFilter,
    locale: UiLocale,
    cards: Vec<CharacterCardHolder>,
    items: Vec<BrowserItem>,
    update_frozen: bool,
    frozen_search_value: Option<String>,
    dirty: bool,
    // These versions describe items, PackRecord history, and portrait keys
    // installed together by rebuild_index, not the newest incoming input.
    cached_generation: u64,
    cached_catalog_revision: u64,
    // Versions of the input last observed, independent of the rendered index.
    last_input_versions: Option<(u64, u64)>,
    last_preview_revision: u64,
    last_selection: Option<CharacterSelection>,
    last_busy: bool,
    last_progress: Option<DownloadProgress>,
    last_catalog_error: Option<String>,
    last_listing_error: Option<String>,
    last_active_key: Option<PreviewKey>,
}
impl BrowserSharedState {
    fn change_query(&mut self, query: String) {
        if !self.update_frozen && self.query != query {
            self.query = query;
            self.dirty = true;
        }
    }

    fn change_filter(&mut self, filter: BrowserFilter) {
        if !self.update_frozen && self.filter != filter {
            self.filter = filter;
            self.dirty = true;
        }
    }
}
struct FrozenBrowserMenu {
    menu: Retained<NSMenu>,
    autoenables_items: bool,
    item_enabled: Vec<bool>,
}

impl FrozenBrowserMenu {
    fn disable(menu: Retained<NSMenu>) -> Self {
        let autoenables_items: bool = unsafe { msg_send![&*menu, autoenablesItems] };
        let items = menu.itemArray();
        let item_enabled = items.iter().map(|item| item.isEnabled()).collect();
        let _: () = unsafe { msg_send![&*menu, setAutoenablesItems: false] };
        for item in items.iter() {
            item.setEnabled(false);
        }
        Self {
            menu,
            autoenables_items,
            item_enabled,
        }
    }

    fn restore(self) {
        for (item, enabled) in self.menu.itemArray().iter().zip(self.item_enabled) {
            item.setEnabled(enabled);
        }
        let _: () = unsafe { msg_send![&*self.menu, setAutoenablesItems: self.autoenables_items] };
    }
}

struct FrozenBrowserCard {
    action_enabled: bool,
    popup_enabled: bool,
    menu: Option<FrozenBrowserMenu>,
}

struct FrozenBrowserControls {
    search_enabled: bool,
    search_editable: bool,
    search_selectable: bool,
    editor: Option<(Retained<AnyObject>, bool, bool)>,
    filter_enabled: [bool; 3],
    import_enabled: bool,
    cancel_enabled: bool,
    apply_enabled: bool,
    cards: Vec<FrozenBrowserCard>,
    ignores_mouse: bool,
}

fn observe_input_versions(last: &mut Option<(u64, u64)>, incoming: (u64, u64)) -> bool {
    let changed = *last != Some(incoming);
    *last = Some(incoming);
    changed
}

// ---------------------------------------------------------------------------
// CharacterBrowser Implementation
// ---------------------------------------------------------------------------

pub(crate) struct CharacterBrowser {
    window: Retained<CharacterBrowserWindow>,
    root: Retained<CharacterBrowserRoot>,
    _action_target: Retained<BrowserActionTarget>,
    menu_target: Retained<MenuTarget>,
    shared: Rc<RefCell<BrowserSharedState>>,
    builtin_image: Option<Retained<NSImage>>,
    mtm: MainThreadMarker,
    update_frozen_previous: RefCell<Option<FrozenBrowserControls>>,
}

impl CharacterBrowser {
    pub(crate) fn new(target: &MenuTarget, locale: UiLocale, mtm: MainThreadMarker) -> Self {
        let initial_size = NSSize::new(740.0, 560.0);
        let window = CharacterBrowserWindow::new(
            frame(0.0, 0.0, initial_size.width, initial_size.height),
            mtm,
        );
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setMinSize(NSSize::new(600.0, 480.0));
        window.setMaxSize(NSSize::new(1100.0, 820.0));
        window.setBackgroundColor(Some(&bg_color()));
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            let _: () = unsafe { msg_send![&*window, setAppearance: Some(&*appearance)] };
        }
        window.setTitle(&NSString::from_str(i18n::text(
            locale,
            Message::CharacterBrowserTitle,
        )));

        let action_target = BrowserActionTarget::new(mtm);
        let shared = Rc::new(RefCell::new(BrowserSharedState {
            query: String::new(),

            filter: BrowserFilter::All,
            locale,
            cards: Vec::new(),
            items: Vec::new(),
            dirty: true,
            update_frozen: false,
            frozen_search_value: None,
            cached_generation: u64::MAX,
            cached_catalog_revision: u64::MAX,
            last_input_versions: None,
            last_preview_revision: u64::MAX,
            last_selection: None,
            last_busy: false,
            last_progress: None,
            last_catalog_error: None,
            last_listing_error: None,
            last_active_key: None,
        }));
        *action_target.ivars().borrow_mut() = Some(shared.clone());

        let root = CharacterBrowserRoot::new(
            frame(0.0, 0.0, initial_size.width, initial_size.height),
            mtm,
        );
        root.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        ax_id(&root, "herdr.character.browser");
        window
            .contentView()
            .expect("character browser window content")
            .addSubview(&root);

        // Header controls
        let search_field = NSSearchField::new(mtm);
        search_field.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        search_field.setTextColor(Some(&primary_color()));
        search_field.setEditable(true);
        search_field.setSelectable(true);
        search_field.setBezeled(true);
        search_field.setRefusesFirstResponder(false);
        search_field.setPlaceholderString(Some(&NSString::from_str(i18n::text(
            locale,
            Message::CharacterBrowserSearchPlaceholder,
        ))));
        unsafe {
            let _: () = msg_send![&*search_field, setContinuous: true];
            let _: () = msg_send![&*search_field, setTarget: Some(&*action_target)];
            let _: () = msg_send![&*search_field, setAction: Some(sel!(searchChanged:))];
        }
        ax(
            &search_field,
            i18n::text(locale, Message::CharacterBrowserSearchPlaceholder),
        );
        ax_id(&search_field, "herdr.character.search");
        root.addSubview(&search_field);

        let filter_all_button = make_control_button(
            i18n::text(locale, Message::CharacterBrowserFilterAll),
            &*action_target,
            sel!(filterAll:),
            true,
            mtm,
        );
        let filter_installed_button = make_control_button(
            i18n::text(locale, Message::CharacterBrowserFilterInstalled),
            &*action_target,
            sel!(filterInstalled:),
            false,
            mtm,
        );
        let filter_official_button = make_control_button(
            i18n::text(locale, Message::CharacterBrowserFilterOfficial),
            &*action_target,
            sel!(filterOfficial:),
            false,
            mtm,
        );
        ax_id(&filter_all_button, "herdr.character.filter.all");
        ax_id(&filter_installed_button, "herdr.character.filter.installed");
        ax_id(&filter_official_button, "herdr.character.filter.official");
        root.addSubview(&filter_all_button);
        root.addSubview(&filter_installed_button);
        root.addSubview(&filter_official_button);

        let status_label = label("", 11.0, false, secondary_color(), mtm);
        root.addSubview(&status_label);

        // Scroll view & Document view
        let scroll_frame = frame(
            0.0,
            66.0,
            initial_size.width,
            initial_size.height - 66.0 - 74.0,
        );
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: scroll_frame] };
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::Automatic);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        scroll.setDrawsBackground(false);
        scroll.contentView().setDrawsBackground(false);

        let document = CharacterBrowserDocument::new(frame(0.0, 0.0, initial_size.width, 1.0), mtm);
        unsafe {
            let _: () = msg_send![&*scroll, setDocumentView: Some(&*document)];
        }
        root.addSubview(&scroll);

        let empty_label = label(
            i18n::text(locale, Message::CharacterBrowserNoResults),
            13.0,
            false,
            secondary_color(),
            mtm,
        );
        empty_label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        empty_label.setHidden(true);
        document.addSubview(&empty_label);

        // Pinned Footer View
        let footer_frame = frame(0.0, initial_size.height - 74.0, initial_size.width, 74.0);
        let footer = CharacterBrowserFooterView::new(footer_frame, mtm);
        root.addSubview(&footer);

        let candidate_thumb = CharacterBrowserThumbView::new(frame(16.0, 16.0, 42.0, 42.0), mtm);
        let pad = 2.0;
        let candidate_image =
            NSImageView::initWithFrame(NSImageView::alloc(mtm), frame(pad, pad, 38.0, 38.0));
        candidate_image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        candidate_image.setEditable(false);
        candidate_thumb.addSubview(&candidate_image);
        footer.addSubview(&candidate_thumb);

        let candidate_title = label("", 12.5, true, primary_color(), mtm);
        candidate_title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        footer.addSubview(&candidate_title);

        let candidate_status = label("", 11.0, false, secondary_color(), mtm);
        candidate_status.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        footer.addSubview(&candidate_status);

        let import_button = make_control_button(
            i18n::text(locale, Message::AddCharacterTitle),
            target.as_ref(),
            sel!(packImport:),
            false,
            mtm,
        );
        footer.addSubview(&import_button);

        let cancel_button = make_control_button(
            i18n::text(locale, Message::CharacterCancelSelection),
            target.as_ref(),
            sel!(packCancelSelection:),
            false,
            mtm,
        );
        cancel_button.setEnabled(false);
        footer.addSubview(&cancel_button);

        let apply_button = make_control_button(
            i18n::text(locale, Message::CharacterApply),
            target.as_ref(),
            sel!(packApply:),
            true,
            mtm,
        );
        apply_button.setEnabled(false);
        footer.addSubview(&apply_button);

        let layout_refs = LayoutRefs {
            search_field,
            filter_all_button,
            filter_installed_button,
            filter_official_button,
            status_label,
            scroll,
            document,
            footer,
            candidate_thumb,
            candidate_image,
            candidate_title,
            candidate_status,
            import_button,
            cancel_button,
            apply_button,
            empty_label,
            shared: shared.clone(),
        };

        *root.ivars().borrow_mut() = Some(layout_refs);
        root.arrange();
        window.center();

        let builtin_image = load_builtin_thumbnail();

        Self {
            window,
            root,
            _action_target: action_target,
            menu_target: target.retain(),
            shared,
            builtin_image,
            mtm,
            update_frozen_previous: RefCell::new(None),
        }
    }

    pub(crate) fn show(&mut self) {
        self.window.makeKeyAndOrderFront(None);
        let ivars = self.root.ivars().borrow();
        if let Some(refs) = ivars.as_ref() {
            let _ = self.window.makeFirstResponder(Some(&refs.search_field));
        }
    }

    /// Prevent native editing and interactions after Prepare has passed its
    /// blocker check; restore the original control state if Prepare is aborted.
    pub(crate) fn set_update_frozen(&self, frozen: bool) {
        let refs = self.root.ivars().borrow();
        let Some(refs) = refs.as_ref() else {
            return;
        };
        let mut previous = self.update_frozen_previous.borrow_mut();
        if frozen {
            if previous.is_some() {
                return;
            }
            {
                let mut shared = self.shared.borrow_mut();
                shared.frozen_search_value = Some(refs.search_field.stringValue().to_string());
                shared.update_frozen = true;
            }
            let editor: Option<&AnyObject> =
                unsafe { msg_send![&*refs.search_field, currentEditor] };
            let editor = editor.map(|editor| {
                let editable: bool = unsafe { msg_send![editor, isEditable] };
                let selectable: bool = unsafe { msg_send![editor, isSelectable] };
                (editor.retain(), editable, selectable)
            });
            let cards = self.shared.borrow().cards.clone();
            let cards = cards
                .iter()
                .map(|card| {
                    let state = FrozenBrowserCard {
                        action_enabled: card.action_button.isEnabled(),
                        popup_enabled: card.popup_button.isEnabled(),
                        menu: card.popup_button.menu().map(FrozenBrowserMenu::disable),
                    };
                    card.action_button.setEnabled(false);
                    card.popup_button.setEnabled(false);
                    state
                })
                .collect();
            *previous = Some(FrozenBrowserControls {
                search_enabled: refs.search_field.isEnabled(),
                search_editable: refs.search_field.isEditable(),
                search_selectable: refs.search_field.isSelectable(),
                editor,
                filter_enabled: [
                    refs.filter_all_button.isEnabled(),
                    refs.filter_installed_button.isEnabled(),
                    refs.filter_official_button.isEnabled(),
                ],
                import_enabled: refs.import_button.isEnabled(),
                cancel_enabled: refs.cancel_button.isEnabled(),
                apply_enabled: refs.apply_button.isEnabled(),
                cards,
                ignores_mouse: self.window.ignoresMouseEvents(),
            });
            self.window.ivars().set(true);
            if let Some((editor, _, _)) = previous.as_ref().and_then(|prior| prior.editor.as_ref())
            {
                let _: () = unsafe { msg_send![&**editor, setEditable: false] };
                let _: () = unsafe { msg_send![&**editor, setSelectable: false] };
            }
            refs.search_field.setEditable(false);
            refs.search_field.setSelectable(false);
            refs.search_field.setEnabled(false);
            refs.filter_all_button.setEnabled(false);
            refs.filter_installed_button.setEnabled(false);
            refs.filter_official_button.setEnabled(false);
            refs.import_button.setEnabled(false);
            refs.cancel_button.setEnabled(false);
            refs.apply_button.setEnabled(false);
            self.window.setIgnoresMouseEvents(true);
        } else if let Some(previous) = previous.take() {
            // AX value setters can bypass isEditable without sending searchChanged:.
            let original = self.shared.borrow().frozen_search_value.clone();
            if let Some(original) = original {
                if refs.search_field.stringValue().to_string() != original {
                    refs.search_field
                        .setStringValue(&NSString::from_str(&original));
                }
            }
            refs.search_field.setEditable(previous.search_editable);
            refs.search_field.setSelectable(previous.search_selectable);
            refs.search_field.setEnabled(previous.search_enabled);
            if let Some((editor, editable, selectable)) = previous.editor {
                // AppKit may share a field editor with another control.
                let current: Option<&AnyObject> =
                    unsafe { msg_send![&*refs.search_field, currentEditor] };
                if current.is_some_and(|current| std::ptr::eq(current, &*editor)) {
                    let _: () = unsafe { msg_send![&*editor, setEditable: editable] };
                    let _: () = unsafe { msg_send![&*editor, setSelectable: selectable] };
                }
            }
            for (button, enabled) in [
                &refs.filter_all_button,
                &refs.filter_installed_button,
                &refs.filter_official_button,
            ]
            .into_iter()
            .zip(previous.filter_enabled)
            {
                button.setEnabled(enabled);
            }
            refs.import_button.setEnabled(previous.import_enabled);
            refs.cancel_button.setEnabled(previous.cancel_enabled);
            refs.apply_button.setEnabled(previous.apply_enabled);
            let cards = self.shared.borrow().cards.clone();
            for (card, state) in cards.iter().zip(previous.cards) {
                if let Some(menu) = state.menu {
                    menu.restore();
                }
                card.action_button.setEnabled(state.action_enabled);
                card.popup_button.setEnabled(state.popup_enabled);
            }
            self.window.setIgnoresMouseEvents(previous.ignores_mouse);
            self.window.ivars().set(false);
            let mut shared = self.shared.borrow_mut();
            shared.update_frozen = false;
            shared.frozen_search_value = None;
        }
    }

    pub(crate) fn shutdown(&self) {
        self.window.orderOut(None);
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.shared.borrow_mut().locale = locale;
        self.window.setTitle(&NSString::from_str(i18n::text(
            locale,
            Message::CharacterBrowserTitle,
        )));

        let ivars = self.root.ivars().borrow();
        let Some(refs) = ivars.as_ref() else { return };

        refs.search_field
            .setPlaceholderString(Some(&NSString::from_str(i18n::text(
                locale,
                Message::CharacterBrowserSearchPlaceholder,
            ))));
        ax(
            &refs.search_field,
            i18n::text(locale, Message::CharacterBrowserSearchPlaceholder),
        );

        refs.filter_all_button
            .setTitle(&NSString::from_str(i18n::text(
                locale,
                Message::CharacterBrowserFilterAll,
            )));
        refs.filter_installed_button
            .setTitle(&NSString::from_str(i18n::text(
                locale,
                Message::CharacterBrowserFilterInstalled,
            )));
        refs.filter_official_button
            .setTitle(&NSString::from_str(i18n::text(
                locale,
                Message::CharacterBrowserFilterOfficial,
            )));

        refs.import_button.setTitle(&NSString::from_str(i18n::text(
            locale,
            Message::AddCharacterTitle,
        )));
        refs.cancel_button.setTitle(&NSString::from_str(i18n::text(
            locale,
            Message::CharacterCancelSelection,
        )));
        refs.apply_button.setTitle(&NSString::from_str(i18n::text(
            locale,
            Message::CharacterApply,
        )));
        refs.empty_label
            .setStringValue(&NSString::from_str(i18n::text(
                locale,
                Message::CharacterBrowserNoResults,
            )));
        for (button, message) in [
            (&refs.filter_all_button, Message::CharacterBrowserFilterAll),
            (
                &refs.filter_installed_button,
                Message::CharacterBrowserFilterInstalled,
            ),
            (
                &refs.filter_official_button,
                Message::CharacterBrowserFilterOfficial,
            ),
            (&refs.import_button, Message::AddCharacterTitle),
            (&refs.cancel_button, Message::CharacterCancelSelection),
            (&refs.apply_button, Message::CharacterApply),
        ] {
            ax(button, i18n::text(locale, message));
        }

        // Rebuild cards to update localized text
        self.shared.borrow_mut().dirty = true;
        self.root.setNeedsLayout(true);
    }

    pub(crate) fn has_marked_text(&self) -> bool {
        let ivars = self.root.ivars().borrow();
        let Some(refs) = ivars.as_ref() else {
            return false;
        };
        if let Some(editor) = refs.search_field.currentEditor() {
            let marked: bool = unsafe { msg_send![&*editor, hasMarkedText] };
            if marked {
                return true;
            }
        }
        false
    }

    /// Use AppKit's live search value: the filtered index can lag field-editor edits.
    pub(crate) fn has_pending_update_intent(&self) -> bool {
        let native_text = {
            let ivars = self.root.ivars().borrow();
            let Some(refs) = ivars.as_ref() else {
                return true;
            };
            let editor_text = refs.search_field.currentEditor().is_some_and(|editor| {
                editor
                    .downcast_ref::<NSTextView>()
                    .is_some_and(|text| text.string().length() > 0)
            });
            refs.search_field.stringValue().length() > 0 || editor_text
        };
        native_text || self.has_marked_text()
    }

    pub(crate) fn visible_preview_requests(&self) -> Vec<PreviewKey> {
        if !self.is_visible() {
            return Vec::new();
        }
        let refs = self.root.ivars().borrow();
        let Some(refs) = refs.as_ref() else {
            return Vec::new();
        };
        let shared = self.shared.borrow();
        shared.items[visible_range(refs, shared.items.len())]
            .iter()
            .take(14)
            .map(|item| item.preview_key.clone())
            .collect()
    }

    pub(crate) fn refresh(&mut self, input: BrowserInput<'_>) {
        // Neither reenable nor rebind controls during Prepare.
        if self.shared.borrow().update_frozen {
            return;
        }
        let marked = self.has_marked_text();
        let (rebuild, state_changed) = {
            let mut shared = self.shared.borrow_mut();
            let versions = (input.listing.generation, input.catalog_revision);
            let versions_changed =
                observe_input_versions(&mut shared.last_input_versions, versions);
            let rebuild = !marked
                && (shared.dirty
                    || shared.cached_generation != versions.0
                    || shared.cached_catalog_revision != versions.1);
            let changed = rebuild
                || versions_changed
                || shared.last_selection.as_ref() != Some(input.selection)
                || shared.last_busy != input.busy
                || shared.last_progress.as_ref() != input.progress
                || shared.last_catalog_error.as_deref() != input.catalog_error
                || shared.last_listing_error != input.listing.error
                || shared.last_active_key.as_ref() != input.active_preview_key;
            if changed {
                shared.last_selection = Some(input.selection.clone());
                shared.last_busy = input.busy;
                shared.last_progress = input.progress.cloned();
                shared.last_catalog_error = input.catalog_error.map(str::to_owned);
                shared.last_listing_error = input.listing.error.clone();
                shared.last_active_key = input.active_preview_key.cloned();
            }
            (rebuild, changed)
        };
        if rebuild {
            self.rebuild_index(&input);
        }
        let preview_changed = self.render_visible(&input, state_changed);
        if state_changed || preview_changed {
            self.update_header_and_footer(&input);
        }
    }

    fn rebuild_index(&mut self, input: &BrowserInput<'_>) {
        let shared = self.shared.borrow();
        let items = build_index(
            input.listing,
            input.official_entries,
            input.catalog_revision,
            shared.locale,
            &shared.query,
            shared.filter,
        );
        drop(shared);
        let mut shared = self.shared.borrow_mut();
        shared.items = items;
        // Install the index and its version stamps as one rendered snapshot.
        shared.cached_generation = input.listing.generation;
        shared.cached_catalog_revision = input.catalog_revision;
        shared.dirty = false;
        for card in &mut shared.cards {
            card.item_index = None;
        }
        drop(shared);
        let refs = self.root.ivars().borrow();
        if let Some(refs) = refs.as_ref() {
            arrange_cards_layout(refs, self.root.bounds().size.width);
        }
    }

    fn create_card_holder(&self) -> CharacterCardHolder {
        let card = CharacterBrowserCardView::new(frame(0.0, 0.0, 300.0, 122.0), self.mtm);
        let thumb = CharacterBrowserThumbView::new(frame(10.0, 10.0, 72.0, 72.0), self.mtm);
        let image =
            NSImageView::initWithFrame(NSImageView::alloc(self.mtm), frame(4.0, 4.0, 64.0, 64.0));
        image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        image.setEditable(false);
        thumb.addSubview(&image);
        card.addSubview(&thumb);
        let name = label("", 13.0, true, primary_color(), self.mtm);
        let meta = label("", 10.5, false, secondary_color(), self.mtm);
        let desc = label("", 10.5, false, secondary_color(), self.mtm);
        desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        desc.setMaximumNumberOfLines(2);
        card.addSubview(&name);
        card.addSubview(&meta);
        card.addSubview(&desc);
        let action = make_control_button(
            "",
            self.menu_target.as_ref(),
            sel!(packSelect:),
            false,
            self.mtm,
        );
        card.addSubview(&action);
        let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm), &NSString::from_str(""));
        let popup = popup_button(menu, frame(0.0, 0.0, 32.0, 26.0), self.mtm);
        popup.setHidden(true);
        card.addSubview(&popup);
        let progress = label("", 10.5, false, accent_color(), self.mtm);
        progress.setHidden(true);
        card.addSubview(&progress);
        CharacterCardHolder {
            view: card,
            thumb_view: thumb,
            image_view: image,
            name_label: name,
            meta_label: meta,
            desc_label: desc,
            action_button: action,
            popup_button: popup,
            progress_label: progress,
            item_index: None,
            visible: false,
        }
    }

    fn render_visible(&mut self, input: &BrowserInput<'_>, actions_changed: bool) -> bool {
        let refs = self.root.ivars().borrow();
        let Some(refs) = refs.as_ref() else {
            return false;
        };
        if self.shared.borrow().cards.is_empty() {
            let mut pool = Vec::with_capacity(14);
            for _ in 0..14 {
                let card = self.create_card_holder();
                card.view.setHidden(true);
                refs.document.addSubview(&card.view);
                pool.push(card);
            }
            self.shared.borrow_mut().cards = pool;
        }
        let range = visible_range(refs, self.shared.borrow().items.len());
        let (locale, rendered_generation, rendered_catalog_revision) = {
            let shared = self.shared.borrow();
            (
                shared.locale,
                shared.cached_generation,
                shared.cached_catalog_revision,
            )
        };
        let preview_revision = input.previews.revision();
        let preview_changed = {
            let mut shared = self.shared.borrow_mut();
            let changed = shared.last_preview_revision != preview_revision;
            shared.last_preview_revision = preview_revision;
            changed
        };
        let mut geometry_changed = false;
        for slot in 0..14 {
            let binding = {
                let mut shared = self.shared.borrow_mut();
                let index = range.start.checked_add(slot).filter(|i| *i < range.end);
                let card = &mut shared.cards[slot];
                let rebind = card.item_index != index || (index.is_some() && !card.visible);
                let was_visible = card.visible;
                card.item_index = index;
                card.visible = index.is_some();
                if rebind
                    || (index.is_none() && was_visible)
                    || (index.is_some() && (actions_changed || preview_changed))
                {
                    Some((card.clone(), index, rebind))
                } else {
                    None
                }
            };
            let Some((card, index, rebind)) = binding else {
                continue;
            };
            let Some(index) = index else {
                card.view.setHidden(true);
                card.action_button.setEnabled(false);
                set_button_payload(&card.action_button, "");
                card.image_view.setImage(None);
                card.popup_button.setHidden(true);
                card.popup_button.setMenu(None);
                continue;
            };
            let item = self.shared.borrow().items[index].clone();
            geometry_changed |= rebind;
            card.view.setHidden(false);
            if rebind {
                self.bind_card(
                    &card,
                    &item,
                    locale,
                    rendered_generation,
                    input.busy || rendered_generation != input.listing.generation,
                );
            } else if actions_changed {
                if let Some(pack) = item.installed_pack() {
                    card.popup_button.setMenu(Some(&character_menu::pack_menu(
                        pack,
                        rendered_generation,
                        input.busy || rendered_generation != input.listing.generation,
                        locale,
                        &self.menu_target,
                        self.mtm,
                    )));
                }
            }
            if rebind || preview_changed {
                let state = input.previews.lookup(&item.preview_key);
                let image = match state {
                    Some(PreviewState::Ready(image)) => Some(image.as_ref()),
                    Some(PreviewState::Error(_)) => None,
                    _ if matches!(item.kind, BrowserItemKind::Builtin) => {
                        self.builtin_image.as_deref()
                    }
                    _ => None,
                };
                card.image_view.setImage(image);
                let portrait_status = match state {
                    Some(PreviewState::Error(error)) => error.as_str(),
                    _ => item.name.as_str(),
                };
                ax(&card.image_view, portrait_status);
            }
            if rebind || actions_changed {
                self.update_card_action(
                    &card,
                    &item,
                    input,
                    locale,
                    rendered_generation,
                    rendered_catalog_revision,
                    rebind,
                );
            }
        }
        // Card bookkeeping was committed before invoking AppKit; no state borrow
        // remains across a menu creation, setter, or layout callback.
        if geometry_changed {
            arrange_cards_layout(refs, self.root.bounds().size.width);
        }
        preview_changed
    }

    fn bind_card(
        &self,
        card: &CharacterCardHolder,
        item: &BrowserItem,
        locale: UiLocale,
        rendered_generation: u64,
        mutations_blocked: bool,
    ) {
        let badge = match &item.kind {
            BrowserItemKind::Builtin => Message::BuiltInTag,
            BrowserItemKind::Local(_) => Message::ManagedTag,
            BrowserItemKind::Official { .. } => Message::CharacterBrowserOfficialTag,
        };
        let title = format!("{} · {}", item.name, i18n::text(locale, badge));
        card.name_label.setStringValue(&NSString::from_str(&title));
        ax(&card.name_label, &title);
        let (metadata, description) = match &item.kind {
            BrowserItemKind::Builtin => (
                format!("{} · {}", item.id, i18n::revision_label(locale, 0)),
                i18n::text(locale, Message::BuiltInTag).to_owned(),
            ),
            BrowserItemKind::Local(pack) => (
                format!("{} · {}", pack.id, i18n::revision_label(locale, pack.head)),
                i18n::text(locale, Message::ManagedTag).to_owned(),
            ),
            BrowserItemKind::Official { entry, installed } => {
                let description = match locale {
                    UiLocale::Ko => &entry.description.ko,
                    UiLocale::En => &entry.description.en,
                };
                let tags = entry.tags.join(", ");
                let details = if installed.is_some() {
                    format!(
                        "{} · {description} · {tags} · {}",
                        i18n::text(locale, Message::CharacterBrowserFilterInstalled),
                        i18n::format_bytes(entry.download_bytes)
                    )
                } else {
                    format!(
                        "{description} · {tags} · {}",
                        i18n::format_bytes(entry.download_bytes)
                    )
                };
                let metadata = format!(
                    "{} · {} · {} · {}",
                    entry.identity.id, entry.identity.version, entry.render_mode, entry.author
                );
                (
                    if let Some(pack) = installed {
                        format!("{metadata} · {}", i18n::revision_label(locale, pack.head))
                    } else {
                        metadata
                    },
                    details,
                )
            }
        };
        card.meta_label
            .setStringValue(&NSString::from_str(&metadata));
        ax(&card.meta_label, &metadata);
        card.desc_label
            .setStringValue(&NSString::from_str(&description));
        set_tooltip(&card.desc_label, &description);
        ax(&card.desc_label, &description);
        if let Some(pack) = item.installed_pack() {
            card.popup_button.setHidden(false);
            card.popup_button.setMenu(Some(&character_menu::pack_menu(
                pack,
                rendered_generation,
                mutations_blocked,
                locale,
                &self.menu_target,
                self.mtm,
            )));
            ax(
                &card.popup_button,
                &format!(
                    "{}: {}",
                    i18n::text(locale, Message::MenuManageCharacter),
                    pack.name,
                ),
            );
        } else {
            card.popup_button.setHidden(true);
            card.popup_button.setMenu(None);
        }
        layout_card_subviews(card, card.view.frame().size.width.max(1.0), 122.0);
    }

    fn update_card_action(
        &self,
        card: &CharacterCardHolder,
        item: &BrowserItem,
        input: &BrowserInput<'_>,
        locale: UiLocale,
        rendered_generation: u64,
        rendered_catalog_revision: u64,
        rebind: bool,
    ) {
        card.progress_label.setHidden(true);
        let (title, enabled, selector, payload) = match &item.kind {
            BrowserItemKind::Builtin
            | BrowserItemKind::Local(_)
            | BrowserItemKind::Official {
                installed: Some(_), ..
            } => {
                let message = selection_message(item, input.listing, input.selection)
                    .expect("selectable card has a revision");
                (
                    i18n::text(locale, message).to_owned(),
                    !input.busy
                        && message != Message::CharacterBrowserSelected
                        && rendered_generation == input.listing.generation,
                    sel!(packSelect:),
                    character_menu::command_payload(&MenuCommand::Select {
                        id: item.id.clone(),
                        generation: rendered_generation,
                    }),
                )
            }
            BrowserItemKind::Official {
                entry,
                installed: None,
            } => {
                let op_id = input.selection.official_operation_id();
                let downloading = op_id.is_some()
                    && input.selection.is_busy()
                    && input
                        .selection
                        .operation_status()
                        .is_some_and(|status| !status.is_terminal())
                    && !input
                        .selection
                        .last_observation()
                        .is_some_and(|operation| operation.committed)
                    && input.selection.official_pack_id() == Some(entry.identity.id.as_str());
                if downloading {
                    if let Some(progress) = input
                        .progress
                        .filter(|progress| op_id == Some(progress.operation_id.as_str()))
                    {
                        let phase = match progress.phase {
                            crate::character_service::DownloadPhase::Resolving => {
                                Message::CharacterOperationQueued
                            }
                            crate::character_service::DownloadPhase::Downloading => {
                                Message::CharacterBrowserDownloading
                            }
                            crate::character_service::DownloadPhase::Validating => {
                                Message::CharacterOperationPreparing
                            }
                            crate::character_service::DownloadPhase::Installing => {
                                Message::CharacterOperationApplying
                            }
                        };
                        let text = if progress.total > 0 {
                            format!(
                                "{} · {}%",
                                i18n::text(locale, phase),
                                progress.received.saturating_mul(100) / progress.total
                            )
                        } else {
                            i18n::text(locale, phase).to_owned()
                        };
                        card.progress_label
                            .setStringValue(&NSString::from_str(&text));
                        ax(&card.progress_label, &text);
                        card.progress_label.setHidden(false);
                    }
                    (
                        i18n::text(locale, Message::CharacterBrowserCancelDownload).to_owned(),
                        true,
                        sel!(cancelOfficialDownload:),
                        format!("cancel:{}", op_id.unwrap_or_default()),
                    )
                } else if !entry.install_supported {
                    (
                        i18n::text(locale, Message::CharacterBrowserUnsupportedFormat).to_owned(),
                        false,
                        sel!(officialDownloadAndApply:),
                        String::new(),
                    )
                } else {
                    (
                        i18n::text(locale, Message::CharacterBrowserDownloadAndApply).to_owned(),
                        !input.busy
                            && rendered_generation == input.listing.generation
                            && rendered_catalog_revision == input.catalog_revision,
                        sel!(officialDownloadAndApply:),
                        encode_official(&BrowserOfficialIntent {
                            identity: entry.identity.clone(),
                            catalog_revision: rendered_catalog_revision,
                            listing_generation: rendered_generation,
                        }),
                    )
                }
            }
        };
        let title_changed = card.action_button.title().to_string() != title;
        if title_changed {
            card.action_button.setTitle(&NSString::from_str(&title));
        }
        if rebind || title_changed {
            ax(&card.action_button, &format!("{}: {title}", item.name));
        }
        card.action_button.setEnabled(enabled);
        unsafe {
            let target: &AnyObject = &*self.menu_target;
            let _: () = msg_send![&*card.action_button, setTarget: Some(target)];
            card.action_button.setAction(Some(selector));
        }
        set_button_payload(&card.action_button, &payload);
    }
    fn update_header_and_footer(&mut self, input: &BrowserInput<'_>) {
        let refs = self.root.ivars().borrow();
        let Some(refs) = refs.as_ref() else { return };
        let shared = self.shared.borrow();
        let locale = shared.locale;
        let filter = shared.filter;
        for (button, selected) in [
            (&refs.filter_all_button, filter == BrowserFilter::All),
            (
                &refs.filter_installed_button,
                filter == BrowserFilter::Installed,
            ),
            (
                &refs.filter_official_button,
                filter == BrowserFilter::Official,
            ),
        ] {
            let tint = if selected {
                accent_color()
            } else {
                secondary_color()
            };
            button.setContentTintColor(Some(&tint));
            button.setState(if selected {
                objc2_app_kit::NSControlStateValueOn
            } else {
                objc2_app_kit::NSControlStateValueOff
            });
        }
        let mut status = i18n::character_count_label(locale, shared.items.len());
        if let Some(error) = input.catalog_error {
            status.push_str(" · ⚠ ");
            status.push_str(error);
        }
        refs.status_label
            .setStringValue(&NSString::from_str(&status));
        ax(&refs.status_label, &status);
        let candidate = input.selection.candidate();
        let reference = candidate
            .map(|candidate| &candidate.reference)
            .or(input.listing.active.as_ref())
            .unwrap_or(&input.listing.selected);
        let name = if reference.is_builtin() {
            i18n::text(locale, Message::RubeliaBuiltIn)
        } else {
            input
                .listing
                .packs
                .iter()
                .find(|pack| pack.id == reference.id)
                .map_or(reference.id.as_str(), |pack| pack.name.as_str())
        };
        let title = if candidate.is_none() && input.listing.override_active {
            i18n::text(locale, Message::ActiveOverride).to_owned()
        } else {
            format!(
                "{} · {}",
                name,
                i18n::revision_label(locale, reference.revision)
            )
        };
        refs.candidate_title
            .setStringValue(&NSString::from_str(&title));
        ax(&refs.candidate_title, &title);
        set_tooltip(&refs.candidate_title, &title);
        let key = PreviewKey {
            source: PreviewSource::Local {
                reference: reference.clone(),
                generation: input.listing.generation,
            },
            pixels: 128,
        };
        let portrait_key = if candidate.is_none() {
            input.active_preview_key.unwrap_or(&key)
        } else {
            &key
        };
        let portrait_state = input.previews.lookup(portrait_key);
        let ready = match portrait_state {
            Some(PreviewState::Ready(image)) => Some(image.as_ref()),
            _ => None,
        };
        refs.candidate_image.setImage(ready.or_else(|| {
            if reference.is_builtin()
                && !(candidate.is_none() && input.listing.override_active)
                && !matches!(portrait_state, Some(PreviewState::Error(_)))
            {
                self.builtin_image.as_deref()
            } else {
                None
            }
        }));
        let error = input.listing.error.as_deref().or_else(|| {
            if input.selection.operation_visible_for_candidate() {
                input.selection.operation_error().or_else(|| {
                    input
                        .selection
                        .operation()
                        .and_then(|op| op.error.as_deref())
                })
            } else {
                None
            }
        });
        let status = if let Some(error) = error {
            format!("⚠ {error}")
        } else if input.selection.stale() {
            i18n::text(locale, Message::CharacterSelectionStale).to_owned()
        } else if let Some(progress) = input.progress {
            let phase = match progress.phase {
                crate::character_service::DownloadPhase::Resolving => {
                    Message::CharacterOperationQueued
                }
                crate::character_service::DownloadPhase::Downloading => {
                    Message::CharacterBrowserDownloading
                }
                crate::character_service::DownloadPhase::Validating => {
                    Message::CharacterOperationPreparing
                }
                crate::character_service::DownloadPhase::Installing => {
                    Message::CharacterOperationApplying
                }
            };
            if progress.total > 0 {
                format!(
                    "{} · {}%",
                    i18n::text(locale, phase),
                    progress.received.saturating_mul(100) / progress.total
                )
            } else {
                i18n::text(locale, phase).to_owned()
            }
        } else if let Some(operation) = if input.selection.operation_visible_for_candidate() {
            input.selection.operation_status()
        } else {
            None
        } {
            i18n::text(
                locale,
                match operation {
                    OperationStatus::AwaitingSubmission | OperationStatus::Accepted => {
                        Message::CharacterOperationQueued
                    }
                    OperationStatus::Preparing => Message::CharacterOperationPreparing,
                    OperationStatus::Applying => Message::CharacterOperationApplying,
                    OperationStatus::Completed => Message::CharacterOperationCompleted,
                    OperationStatus::Failed => Message::CharacterOperationFailed,
                    OperationStatus::Canceled => Message::CharacterOperationCanceled,
                    OperationStatus::CommittedPendingApply => {
                        Message::CharacterOperationPendingApply
                    }
                    OperationStatus::MissingStatus
                    | OperationStatus::DurabilityUnknown
                    | OperationStatus::Unknown => Message::CharacterOperationUnknown,
                },
            )
            .to_owned()
        } else if input.listing.override_active && candidate.is_none() {
            i18n::text(locale, Message::ActiveOverride).to_owned()
        } else if candidate.is_some() {
            i18n::text(locale, Message::CharacterCandidate).to_owned()
        } else {
            i18n::text(locale, Message::CharacterSelectionPrompt).to_owned()
        };
        refs.candidate_status
            .setStringValue(&NSString::from_str(&status));
        let tint = if error.is_some() {
            error_color()
        } else {
            secondary_color()
        };
        refs.candidate_status.setTextColor(Some(&tint));
        ax(&refs.candidate_status, &status);
        set_tooltip(&refs.candidate_status, &status);
        refs.cancel_button
            .setEnabled(candidate.is_some() && !input.busy);
        refs.apply_button
            .setEnabled(input.selection.can_apply() && !input.busy);
        refs.import_button.setEnabled(!input.busy);
    }
}

// ---------------------------------------------------------------------------
// Action & Control Button Builder
// ---------------------------------------------------------------------------

fn make_control_button(
    title: &str,
    target: &AnyObject,
    action: objc2::runtime::Sel,
    primary: bool,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(true);
    button.setControlSize(NSControlSize::Small);
    let font = if primary {
        NSFont::boldSystemFontOfSize(11.5)
    } else {
        NSFont::systemFontOfSize(11.0)
    };
    button.setFont(Some(&font));
    let color = if primary {
        accent_color()
    } else {
        primary_color()
    };
    button.setContentTintColor(Some(&color));
    button.setRefusesFirstResponder(false);
    ax(&button, title);
    button
}

fn popup_button(
    menu: Retained<NSMenu>,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let popup: Retained<NSPopUpButton> = unsafe {
        msg_send![
            NSPopUpButton::alloc(mtm),
            initWithFrame: frame,
            pullsDown: true
        ]
    };
    popup.setMenu(Some(&menu));
    popup.setControlSize(NSControlSize::Small);
    popup.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    popup.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    popup.setBordered(false);
    let color = secondary_color();
    popup.setContentTintColor(Some(&color));
    popup
}

#[cfg(test)]
mod tests {
    use super::{
        build_index, observe_input_versions, selection_message, BrowserFilter, BrowserItemKind,
        BrowserSharedState,
    };
    use crate::character_preview::PreviewSource;
    use crate::character_selection::CharacterSelection;
    use crate::character_types::{CharacterRef, OfficialPackIdentity, PackListing, PackRecord};
    use crate::i18n::{Message, UiLocale};
    use crate::official_catalog::{LocalizedText, OfficialEntry};

    #[test]
    fn marked_index_version_only_changes_refresh_actions_once() {
        let rendered = (7, 10);
        let mut observed = None;
        assert!(observe_input_versions(&mut observed, rendered));
        assert!(!observe_input_versions(&mut observed, rendered));
        assert!(observe_input_versions(&mut observed, (7, 11)));
        assert!(!observe_input_versions(&mut observed, (7, 11)));
        assert!(observe_input_versions(&mut observed, (8, 11)));
        assert!(!observe_input_versions(&mut observed, (8, 11)));
        assert_eq!(rendered, (7, 10));
    }

    fn fixture() -> (PackListing, Vec<OfficialEntry>) {
        let pack = |id: &str, name: &str, head| PackRecord {
            id: id.into(),
            name: name.into(),
            head,
            revisions: vec![head],
        };
        let listing = PackListing {
            generation: 8,
            selected: CharacterRef::builtin(),
            active: Some(CharacterRef {
                id: "cat".into(),
                revision: 3,
            }),
            override_active: false,
            packs: vec![pack("cat", "Local cat", 4), pack("other", "Other", 2)],
            error: None,
        };
        let entry = |id: &str, name: &str| OfficialEntry {
            identity: OfficialPackIdentity {
                id: id.into(),
                version: "1.0".into(),
                release_tag: "release".into(),
                sha256: "a".repeat(64),
            },
            name: name.into(),
            variant_name: LocalizedText {
                ko: "변형".into(),
                en: "Variant".into(),
            },
            description: LocalizedText {
                ko: "설명".into(),
                en: "Description".into(),
            },
            tags: vec!["fluffy".into()],
            author: "Artist".into(),
            format_version: 5,
            render_mode: "rig".into(),
            download_bytes: 1000,
            preview_idle_url: "https://example.org/preview.png".into(),
            install_supported: true,
            download_url: "https://example.org/pack".into(),
        };
        (
            listing,
            vec![entry("cat", "Catalog cat"), entry("dog", "Catalog dog")],
        )
    }

    #[test]
    fn catalog_ids_are_one_card_in_every_filter_and_search_matches_both_names() {
        let (listing, entries) = fixture();
        for (filter, expected) in [
            (BrowserFilter::All, vec!["default", "other", "cat", "dog"]),
            (BrowserFilter::Installed, vec!["default", "other", "cat"]),
            (BrowserFilter::Official, vec!["cat", "dog"]),
        ] {
            let items = build_index(&listing, &entries, 12, UiLocale::En, "", filter);
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            let cat = items.iter().find(|item| item.id == "cat").unwrap();
            assert!(
                matches!(&cat.kind, BrowserItemKind::Official { installed: Some(pack), .. } if pack.head == 4)
            );
            assert_eq!(cat.installed_pack().unwrap().name, "Local cat");
        }
        for (query, expected) in [
            ("Local cat", vec!["cat"]),
            ("Catalog cat", vec!["cat"]),
            ("Description", vec!["cat", "dog"]),
            ("fluffy", vec!["cat", "dog"]),
            ("Variant", vec!["cat", "dog"]),
            ("cat", vec!["cat", "dog"]),
        ] {
            let items = build_index(
                &listing,
                &entries,
                12,
                UiLocale::En,
                query,
                BrowserFilter::Official,
            );
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        let items = build_index(
            &listing,
            &entries,
            12,
            UiLocale::En,
            "Other",
            BrowserFilter::All,
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["other"]
        );
    }

    #[test]
    fn installed_official_uses_local_head_preview_and_selection_status() {
        let (mut listing, entries) = fixture();
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing);
        let items = build_index(
            &listing,
            &entries,
            12,
            UiLocale::En,
            "",
            BrowserFilter::Official,
        );
        let installed = &items[0];
        assert_eq!(installed.selected_revision(), Some(4));
        assert!(
            matches!(&installed.preview_key.source, PreviewSource::Local { reference, generation } if reference.id == "cat" && reference.revision == 4 && *generation == 8)
        );
        assert!(
            matches!(&items[1].preview_key.source, PreviewSource::Official { catalog_revision, .. } if *catalog_revision == 12)
        );
        assert_eq!(
            selection_message(installed, &listing, &selection),
            Some(Message::CharacterBrowserSelect)
        );
        listing.active = Some(CharacterRef {
            id: "cat".into(),
            revision: 4,
        });
        assert_eq!(
            selection_message(installed, &listing, &selection),
            Some(Message::Active)
        );
        listing.override_active = true;
        assert_eq!(
            selection_message(installed, &listing, &selection),
            Some(Message::CharacterBrowserSelect)
        );
        selection.reconcile(&listing);
        selection.stage_head("cat").unwrap();
        assert_eq!(selection.candidate().unwrap().reference.revision, 4);
        assert_eq!(
            selection_message(installed, &listing, &selection),
            Some(Message::CharacterBrowserSelected)
        );
        assert_eq!(items[1].selected_revision(), None);
        assert_eq!(selection_message(&items[1], &listing, &selection), None);
    }

    #[test]
    fn frozen_browser_actions_reject_query_and_filter_changes() {
        let mut shared = BrowserSharedState {
            query: "existing".into(),
            filter: BrowserFilter::Installed,
            locale: UiLocale::En,
            cards: Vec::new(),
            items: Vec::new(),
            update_frozen: true,
            frozen_search_value: Some(String::new()),
            dirty: false,
            cached_generation: 0,
            cached_catalog_revision: 0,
            last_input_versions: None,
            last_preview_revision: 0,
            last_selection: None,
            last_busy: false,
            last_progress: None,
            last_catalog_error: None,
            last_listing_error: None,
            last_active_key: None,
        };
        shared.change_query("ax search".into());
        shared.change_filter(BrowserFilter::Official);
        assert_eq!(shared.query, "existing");
        assert_eq!(shared.filter, BrowserFilter::Installed);
        assert!(!shared.dirty);

        shared.update_frozen = false;
        shared.change_query("after thaw".into());
        shared.change_filter(BrowserFilter::Official);
        assert_eq!(shared.query, "after thaw");
        assert_eq!(shared.filter, BrowserFilter::Official);
        assert!(shared.dirty);
    }
}
