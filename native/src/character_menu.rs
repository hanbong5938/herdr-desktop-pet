use crate::character_selection::{CharacterSelection, OperationStatus};
use crate::character_types::{CharacterRef, PackListing, PackRecord};
use crate::i18n::{self, Message, UiLocale};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message as _,
};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSBezierPath, NSButton, NSControlSize, NSFont,
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSMenu, NSMenuItem, NSPopUpButton,
    NSScrollElasticity, NSScrollView, NSScrollerStyle, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{ns_string, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::ui::MenuTarget;

const ROOT_WIDTH: f64 = 328.0;
const ROOT_HEIGHT: f64 = 260.0;
const FOOTER_HEIGHT: f64 = 116.0;
const PADDING: f64 = 10.0;
const CARD_GAP: f64 = 8.0;
const CARD_RADIUS: f64 = 10.0;
const HERO_HEIGHT: f64 = 52.0;
const BANNER_HEIGHT: f64 = 30.0;
const ROW_HEIGHT: f64 = 40.0;
const ROW_GAP: f64 = 4.0;
const BUTTON_HEIGHT: f64 = 28.0;

const PRIMARY_RED: f64 = 0.95;
const PRIMARY_GREEN: f64 = 0.95;
const PRIMARY_BLUE: f64 = 0.97;
const SECONDARY_RED: f64 = 0.60;
const SECONDARY_GREEN: f64 = 0.60;
const SECONDARY_BLUE: f64 = 0.63;
const ACCENT_RED: f64 = 0.29;
const ACCENT_GREEN: f64 = 0.56;
const ACCENT_BLUE: f64 = 0.89;
const SUCCESS_RED: f64 = 0.30;
const SUCCESS_GREEN: f64 = 0.78;
const SUCCESS_BLUE: f64 = 0.47;
const WARNING_RED: f64 = 0.96;
const WARNING_GREEN: f64 = 0.65;
const WARNING_BLUE: f64 = 0.14;
const ERROR_RED: f64 = 0.95;
const ERROR_GREEN: f64 = 0.36;
const ERROR_BLUE: f64 = 0.36;

const CARD_RED: f64 = 0.161;
const CARD_GREEN: f64 = 0.161;
const CARD_BLUE: f64 = 0.176;
const CARD_BORDER_RED: f64 = 0.28;
const CARD_BORDER_GREEN: f64 = 0.28;
const CARD_BORDER_BLUE: f64 = 0.32;
const CARD_BORDER_ALPHA: f64 = 0.50;

const ROW_HOVER_RED: f64 = 0.20;
const ROW_HOVER_GREEN: f64 = 0.20;
const ROW_HOVER_BLUE: f64 = 0.22;

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

fn symbol_image(symbol_name: &str) -> Option<Retained<NSImage>> {
    let name = NSString::from_str(symbol_name);
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    Select {
        id: String,
        generation: u64,
    },
    Update {
        id: String,
        generation: u64,
    },
    Restore {
        id: String,
        revision: u64,
        generation: u64,
    },
    Remove {
        id: String,
        generation: u64,
    },
    Inspect {
        id: String,
    },
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterMenuDocument"]
    struct CharacterMenuDocument;

    unsafe impl NSObjectProtocol for CharacterMenuDocument {}

    impl CharacterMenuDocument {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl CharacterMenuDocument {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
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

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterCardView"]
    struct CharacterCardView;

    unsafe impl NSObjectProtocol for CharacterCardView {}

    impl CharacterCardView {
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

impl CharacterCardView {
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
    #[name = "OMPetCharacterFooterView"]
    struct CharacterFooterView;

    unsafe impl NSObjectProtocol for CharacterFooterView {}

    impl CharacterFooterView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::fillRect(bounds);

            let border = card_border_color();
            border.setStroke();
            let sep_path = NSBezierPath::bezierPath();
            sep_path.moveToPoint(NSPoint::new(0.0, 0.5));
            sep_path.lineToPoint(NSPoint::new(bounds.size.width, 0.5));
            sep_path.setLineWidth(1.0);
            sep_path.stroke();
        }
    }
);

impl CharacterFooterView {
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
    #[name = "OMPetCharacterBannerView"]
    #[ivars = Cell<bool>]
    struct CharacterBannerView;

    unsafe impl NSObjectProtocol for CharacterBannerView {}

    impl CharacterBannerView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 6.0, 6.0).fill();

            let border = if self.ivars().get() {
                error_color()
            } else {
                warning_color()
            };
            border.setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                5.5,
                5.5,
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl CharacterBannerView {
    fn new(frame: NSRect, is_error: bool, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(is_error));
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterRowView"]
    #[ivars = Cell<bool>]
    struct CharacterRowView;

    unsafe impl NSObjectProtocol for CharacterRowView {}

    impl CharacterRowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            let is_selected = self.ivars().get();
            let (fill, border, line_width) = if is_selected {
                (row_selected_color(), accent_color(), 1.0)
            } else {
                (row_hover_color(), card_border_color(), 0.5)
            };
            fill.setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 6.0, 6.0).fill();

            border.setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                5.5,
                5.5,
            );
            border_path.setLineWidth(line_width);
            border_path.stroke();
        }
    }
);

impl CharacterRowView {
    fn new(frame: NSRect, selected: bool, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(selected));
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetCharacterThumbView"]
    struct CharacterThumbView;

    unsafe impl NSObjectProtocol for CharacterThumbView {}

    impl CharacterThumbView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 6.0, 6.0).fill();

            card_border_color().setStroke();
            let border_bounds = inset_rect(bounds, 0.5);
            let border_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                border_bounds,
                5.5,
                5.5,
            );
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl CharacterThumbView {
    fn new(frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizesSubviews(false);
        view
    }
}

pub(crate) struct CharacterMenu {
    root: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    footer: Retained<CharacterFooterView>,
    document: Retained<CharacterMenuDocument>,
    target: Retained<MenuTarget>,
    locale: UiLocale,
    listing: Option<PackListing>,
    busy: bool,
    selection: CharacterSelection,
    last_layout_width: Option<f64>,
    last_layout_height: Option<f64>,
    builtin_image: Option<Retained<NSImage>>,
    mtm: MainThreadMarker,
}

impl CharacterMenu {
    pub(crate) fn new(target: &MenuTarget, locale: UiLocale, mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(ROOT_WIDTH, ROOT_HEIGHT));
        let root = NSView::initWithFrame(NSView::alloc(mtm), frame);
        root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        root.setAutoresizesSubviews(true);

        let document = CharacterMenuDocument::new(frame, mtm);
        document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        document.setAutoresizesSubviews(false);

        let scroll_frame = NSRect::new(
            NSPoint::new(0.0, FOOTER_HEIGHT),
            NSSize::new(ROOT_WIDTH, (ROOT_HEIGHT - FOOTER_HEIGHT).max(1.0)),
        );
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: scroll_frame] };
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        scroll.setDrawsBackground(false);
        scroll.contentView().setDrawsBackground(false);
        scroll.contentView().setAutoresizesSubviews(true);
        scroll.setAutoresizesSubviews(true);
        unsafe {
            let _: () = msg_send![&*scroll, setDocumentView: Some(&*document)];
        }
        scroll.contentView().setAutoresizesSubviews(true);
        root.addSubview(&scroll);
        let footer = CharacterFooterView::new(
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(ROOT_WIDTH, FOOTER_HEIGHT),
            ),
            mtm,
        );
        root.addSubview(&footer);

        let builtin_image = load_builtin_thumbnail();
        let mut menu = Self {
            root,
            scroll,
            footer,
            document,
            target: target.retain(),
            locale,
            listing: None,
            busy: false,
            selection: CharacterSelection::new(),
            last_layout_width: None,
            last_layout_height: None,
            builtin_image,
            mtm,
        };
        menu.set_frame(frame);
        menu
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.root
    }

    pub(crate) fn set_frame(&mut self, frame: NSRect) {
        let scroll_origin = self.scroll.contentView().bounds().origin;
        self.root.setFrame(frame);
        self.scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, FOOTER_HEIGHT),
            NSSize::new(
                frame.size.width,
                (frame.size.height - FOOTER_HEIGHT).max(1.0),
            ),
        ));
        let root_width = self.root.bounds().size.width.max(1.0);
        let mut document_frame = self.document.frame();
        document_frame.size.width = root_width;
        self.document.setFrame(document_frame);
        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();
        let clip_width = self
            .scroll
            .contentView()
            .bounds()
            .size
            .width
            .min(root_width)
            .max(1.0);
        let width = self
            .scroll
            .documentVisibleRect()
            .size
            .width
            .min(clip_width)
            .max(1.0);
        document_frame.size.width = width;
        self.document.setFrame(document_frame);
        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();
        let clip_view = self.scroll.contentView();
        let mut proposed_bounds = clip_view.bounds();
        proposed_bounds.origin = scroll_origin;
        clip_view.scrollToPoint(clip_view.constrainBoundsRect(proposed_bounds).origin);
        self.scroll.reflectScrolledClipView(&clip_view);

        if let Some(listing) = self.listing.clone() {
            self.rebuild(&listing);
        }
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        if self.locale == locale {
            return;
        }
        self.locale = locale;
        if let Some(listing) = self.listing.clone() {
            self.rebuild(&listing);
        }
    }

    pub(crate) fn refresh(
        &mut self,
        listing: &PackListing,
        selection: &CharacterSelection,
        busy: bool,
        _target: &MenuTarget,
        _mtm: MainThreadMarker,
    ) {
        let same_content = self.listing.as_ref() == Some(listing)
            && &self.selection == selection
            && self.busy == busy;
        let root_bounds = self.root.bounds();
        let root_width = root_bounds.size.width.max(1.0);
        let root_height = root_bounds.size.height.max(1.0);
        let width_changed = self
            .last_layout_width
            .map_or(true, |w| (w - root_width).abs() > f64::EPSILON)
            || self
                .last_layout_height
                .map_or(true, |h| (h - root_height).abs() > f64::EPSILON);
        if same_content && !width_changed {
            return;
        }
        self.listing = Some(listing.clone());
        self.selection = selection.clone();
        self.busy = busy;
        self.rebuild(listing);
    }

    fn rebuild(&mut self, listing: &PackListing) {
        let saved_origin = self.scroll.contentView().bounds().origin;
        let footer_children = self.footer.subviews();
        for index in (0..footer_children.count()).rev() {
            footer_children.objectAtIndex(index).removeFromSuperview();
        }
        self.remove_document_subviews();

        let root_bounds = self.root.bounds();
        let root_width = root_bounds.size.width.max(1.0);
        let root_height = root_bounds.size.height.max(1.0);
        self.scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, FOOTER_HEIGHT),
            NSSize::new(root_width, (root_height - FOOTER_HEIGHT).max(1.0)),
        ));
        self.footer.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(root_width, FOOTER_HEIGHT),
        ));

        let operation_visible = self.selection.operation_visible_for_candidate();
        let operation_error = if operation_visible {
            self.selection.operation_error().or_else(|| {
                self.selection
                    .operation()
                    .and_then(|op| op.error.as_deref())
            })
        } else {
            None
        };
        let error_message = listing.error.as_deref().or(operation_error);
        let has_error = error_message.is_some();

        let is_mismatched = listing.override_active
            || listing
                .active
                .as_ref()
                .map_or(true, |active| active != &listing.selected);

        let banner_count = if has_error || is_mismatched { 1 } else { 0 };
        let banner_total_height = if banner_count > 0 {
            BANNER_HEIGHT + CARD_GAP
        } else {
            0.0
        };

        // Total rows = Coding Cat + managed packs
        let total_items = 1 + listing.packs.len();
        let list_card_header_height = 22.0;
        let list_card_padding = 8.0;
        let list_items_height =
            total_items as f64 * ROW_HEIGHT + (total_items.saturating_sub(1)) as f64 * ROW_GAP;
        let list_card_height =
            list_card_padding * 2.0 + list_card_header_height + list_items_height;

        let document_height = (PADDING
            + HERO_HEIGHT
            + CARD_GAP
            + banner_total_height
            + list_card_height
            + CARD_GAP
            + BUTTON_HEIGHT
            + PADDING)
            .max((root_height - FOOTER_HEIGHT).max(1.0));

        self.document.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(root_width, document_height),
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
            .min(root_width)
            .max(1.0);
        let visible_width = self
            .scroll
            .documentVisibleRect()
            .size
            .width
            .min(clip_width)
            .max(1.0);
        let is_legacy = visible_width < root_width - 1.0;
        let scroller_allowance = if is_legacy { 0.0 } else { 14.0 };
        let outer_inset = 6.0_f64.min(visible_width * 0.5);
        let content_width = (visible_width - outer_inset * 2.0 - scroller_allowance).max(1.0);
        let inner_inset = 10.0_f64.min(content_width * 0.5);
        let inner_width = (content_width - inner_inset * 2.0).max(1.0);

        let mut y = PADDING;

        // 1. Hero Card: Current Character Summary & Diagnostics Entry
        let hero = CharacterCardView::new(
            NSRect::new(
                NSPoint::new(outer_inset, y),
                NSSize::new(content_width, HERO_HEIGHT),
            ),
            self.mtm,
        );
        self.document.addSubview(&hero);

        // Find active character name & thumbnail
        let (active_name, active_image) = self.resolve_character_info(
            listing.active.as_ref().unwrap_or(&listing.selected),
            listing,
        );

        // Avatar icon (32x32)
        let avatar_size = 32.0;
        let avatar_x = inner_inset;
        let avatar_y = (HERO_HEIGHT - avatar_size) * 0.5;
        let avatar_box = thumbnail_box(
            active_image,
            NSRect::new(
                NSPoint::new(avatar_x, avatar_y),
                NSSize::new(avatar_size, avatar_size),
            ),
            self.mtm,
        );
        hero.addSubview(&avatar_box);

        // Text summary
        let text_x = avatar_x + avatar_size + 8.0;
        let diag_btn_size = 24.0;
        let text_width = (content_width - inner_inset - text_x - diag_btn_size - 6.0).max(1.0);

        let live_name = if let Some(active) = listing.active.as_ref() {
            format!(
                "{} · {}",
                active_name,
                i18n::revision_label(self.locale, active.revision)
            )
        } else {
            active_name
        };
        let name_label = add_label(
            &hero,
            &bounded(&live_name, 60),
            NSRect::new(NSPoint::new(text_x, 8.0), NSSize::new(text_width, 18.0)),
            LabelStyle::PrimaryBold,
            1,
            self.mtm,
        );
        set_accessibility_label(&name_label, &live_name);

        let status_badge_text = if listing.active.is_none() {
            i18n::operation_pending(self.locale).to_owned()
        } else if listing.override_active {
            i18n::text(self.locale, Message::ActiveOverride).to_owned()
        } else {
            i18n::text(self.locale, Message::Active).to_owned()
        };
        add_label(
            &hero,
            &format!("● {status_badge_text}"),
            NSRect::new(NSPoint::new(text_x, 28.0), NSSize::new(text_width, 16.0)),
            if listing.override_active {
                LabelStyle::Warning
            } else {
                LabelStyle::Success
            },
            1,
            self.mtm,
        );

        // Diagnostics button 'ⓘ'
        let diag_btn_x = content_width - inner_inset - diag_btn_size;
        let diag_btn_y = (HERO_HEIGHT - diag_btn_size) * 0.5;
        let diag_btn = action_button(
            "ⓘ",
            NSRect::new(
                NSPoint::new(diag_btn_x, diag_btn_y),
                NSSize::new(diag_btn_size, diag_btn_size),
            ),
            sel!(packDiagnose:),
            &self.target,
            self.mtm,
            false,
        );
        diag_btn.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        set_accessibility_label(
            &diag_btn,
            i18n::text(self.locale, Message::CharacterDiagnostics),
        );
        hero.addSubview(&diag_btn);

        y += HERO_HEIGHT + CARD_GAP;

        // 2. Inline Warning / Error Banner (only visible when active error or mismatch)
        if let Some(err) = error_message {
            let banner = CharacterBannerView::new(
                NSRect::new(
                    NSPoint::new(outer_inset, y),
                    NSSize::new(content_width, BANNER_HEIGHT),
                ),
                true,
                self.mtm,
            );
            self.document.addSubview(&banner);

            let err_text = format!("⚠ {}", bounded(err, 120));
            add_label(
                &banner,
                &err_text,
                NSRect::new(
                    NSPoint::new(inner_inset, (BANNER_HEIGHT - 16.0) * 0.5),
                    NSSize::new(inner_width, 16.0),
                ),
                LabelStyle::Error,
                1,
                self.mtm,
            );
            y += BANNER_HEIGHT + CARD_GAP;
        } else if is_mismatched {
            let banner = CharacterBannerView::new(
                NSRect::new(
                    NSPoint::new(outer_inset, y),
                    NSSize::new(content_width, BANNER_HEIGHT),
                ),
                false,
                self.mtm,
            );
            self.document.addSubview(&banner);

            let warn_text = format!(
                "⚠ {}",
                i18n::text(
                    self.locale,
                    if listing.override_active {
                        Message::OverrideActiveWarning
                    } else {
                        Message::SelectedMismatchWarning
                    }
                )
            );
            add_label(
                &banner,
                &warn_text,
                NSRect::new(
                    NSPoint::new(inner_inset, (BANNER_HEIGHT - 16.0) * 0.5),
                    NSSize::new(inner_width, 16.0),
                ),
                LabelStyle::Warning,
                1,
                self.mtm,
            );
            y += BANNER_HEIGHT + CARD_GAP;
        }

        // 3. Single Unified Character List Card
        let list_card = CharacterCardView::new(
            NSRect::new(
                NSPoint::new(outer_inset, y),
                NSSize::new(content_width, list_card_height),
            ),
            self.mtm,
        );
        self.document.addSubview(&list_card);

        // Header label
        add_label(
            &list_card,
            i18n::text(self.locale, Message::Characters),
            NSRect::new(
                NSPoint::new(inner_inset, list_card_padding),
                NSSize::new(inner_width, 18.0),
            ),
            LabelStyle::Heading,
            1,
            self.mtm,
        );

        let mut row_y = list_card_padding + list_card_header_height;

        // Built-in character
        let live_default = listing.active.as_ref() == Some(&CharacterRef::builtin());
        self.render_character_row(
            &list_card,
            "default",
            i18n::text(self.locale, Message::RubeliaBuiltIn),
            i18n::text(self.locale, Message::BuiltInTag),
            self.builtin_image
                .as_ref()
                .map(|img| img.retain())
                .or_else(|| symbol_image("sparkles")),
            live_default,
            self.selection
                .candidate()
                .is_some_and(|candidate| candidate.reference.id == "default"),
            listing
                .active
                .as_ref()
                .filter(|reference| reference.id == "default")
                .map(|reference| reference.revision),
            self.selection
                .candidate()
                .filter(|candidate| candidate.reference.id == "default")
                .map(|candidate| candidate.reference.revision),
            None,
            row_y,
            inner_inset,
            content_width,
            listing.generation,
        );
        row_y += ROW_HEIGHT + ROW_GAP;

        // Managed Packs
        for pack in &listing.packs {
            let is_pack_live = listing
                .active
                .as_ref()
                .is_some_and(|active| active.id == pack.id);
            let pack_thumb = symbol_image("cube.box.fill");
            self.render_character_row(
                &list_card,
                &pack.id,
                &pack.name,
                i18n::text(self.locale, Message::ManagedTag),
                pack_thumb,
                is_pack_live,
                self.selection
                    .candidate()
                    .is_some_and(|candidate| candidate.reference.id == pack.id),
                listing
                    .active
                    .as_ref()
                    .filter(|reference| reference.id == pack.id)
                    .map(|reference| reference.revision),
                self.selection
                    .candidate()
                    .filter(|candidate| candidate.reference.id == pack.id)
                    .map(|candidate| candidate.reference.revision),
                Some(pack),
                row_y,
                inner_inset,
                content_width,
                listing.generation,
            );
            row_y += ROW_HEIGHT + ROW_GAP;
        }

        y += list_card_height + CARD_GAP;

        // 4. Import / Add Character Button
        let import = action_button(
            &format!("+ {}", i18n::text(self.locale, Message::AddCharacter)),
            NSRect::new(
                NSPoint::new(outer_inset, y),
                NSSize::new(content_width, BUTTON_HEIGHT),
            ),
            sel!(packImport:),
            &self.target,
            self.mtm,
            true,
        );
        self.document.addSubview(&import);
        import.setEnabled(!self.busy);

        // Finalize geometry and scrolling
        self.document.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(visible_width, document_height),
        ));
        self.scroll.layoutSubtreeIfNeeded();
        self.scroll.tile();
        self.scroll.layoutSubtreeIfNeeded();
        let clip_view = self.scroll.contentView();
        let mut proposed_bounds = clip_view.bounds();
        proposed_bounds.origin = saved_origin;
        clip_view.scrollToPoint(clip_view.constrainBoundsRect(proposed_bounds).origin);
        self.scroll.reflectScrolledClipView(&clip_view);
        self.last_layout_width = Some(root_width);
        self.render_footer(listing, root_width);
        self.last_layout_height = Some(root_height);
    }

    fn resolve_character_info(
        &self,
        reference: &CharacterRef,
        listing: &PackListing,
    ) -> (String, Option<Retained<NSImage>>) {
        if reference.id == "default" {
            (
                i18n::text(self.locale, Message::RubeliaBuiltIn).to_owned(),
                self.builtin_image
                    .as_ref()
                    .map(|img| img.retain())
                    .or_else(|| symbol_image("sparkles")),
            )
        } else if let Some(pack) = listing.packs.iter().find(|p| p.id == reference.id) {
            (pack.name.clone(), symbol_image("cube.box.fill"))
        } else {
            (reference.id.clone(), symbol_image("person.crop.circle"))
        }
    }

    fn render_character_row(
        &self,
        parent: &NSView,
        id: &str,
        name: &str,
        tag: &str,
        thumbnail: Option<Retained<NSImage>>,
        live: bool,
        candidate: bool,
        live_revision: Option<u64>,
        candidate_revision: Option<u64>,
        pack_record: Option<&PackRecord>,
        row_y: f64,
        inner_inset: f64,
        content_width: f64,
        generation: u64,
    ) {
        let row_w = (content_width - inner_inset * 2.0).max(1.0);
        let row_box = CharacterRowView::new(
            NSRect::new(
                NSPoint::new(inner_inset, row_y),
                NSSize::new(row_w, ROW_HEIGHT),
            ),
            candidate,
            self.mtm,
        );
        parent.addSubview(&row_box);

        // Thumbnail (28x28)
        let thumb_size = 28.0;
        let thumb_x = 6.0;
        let thumb_y = (ROW_HEIGHT - thumb_size) * 0.5;
        let thumb_view = thumbnail_box(
            thumbnail,
            NSRect::new(
                NSPoint::new(thumb_x, thumb_y),
                NSSize::new(thumb_size, thumb_size),
            ),
            self.mtm,
        );
        row_box.addSubview(&thumb_view);

        // Labels
        let has_overflow = pack_record.is_some();
        let overflow_btn_width = if has_overflow { 28.0 } else { 0.0 };
        let checkmark_width = if live { 20.0 } else { 0.0 };
        let text_x = thumb_x + thumb_size + 8.0;
        let text_w = (row_w - text_x - overflow_btn_width - checkmark_width - 8.0).max(1.0);

        let name_label = add_label(
            &row_box,
            &bounded(name, 50),
            NSRect::new(NSPoint::new(text_x, 4.0), NSSize::new(text_w, 16.0)),
            LabelStyle::PrimaryBold,
            1,
            self.mtm,
        );
        set_accessibility_label(&name_label, name);

        add_label(
            &row_box,
            tag,
            NSRect::new(NSPoint::new(text_x, 21.0), NSSize::new(text_w, 14.0)),
            LabelStyle::Secondary,
            1,
            self.mtm,
        );

        // Selection Checkmark
        if live {
            let check_x = row_w - overflow_btn_width - checkmark_width - 4.0;
            add_label(
                &row_box,
                "✓",
                NSRect::new(
                    NSPoint::new(check_x, (ROW_HEIGHT - 18.0) * 0.5),
                    NSSize::new(checkmark_width, 18.0),
                ),
                LabelStyle::Accent,
                1,
                self.mtm,
            );
        }

        // Clickable selection button overlay covering the row
        let select_hit_w = (row_w - overflow_btn_width).max(1.0);
        let select_btn = command_button(
            "",
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(select_hit_w, ROW_HEIGHT),
            ),
            sel!(packSelect:),
            &MenuCommand::Select {
                id: id.to_owned(),
                generation,
            },
            &self.target,
            self.mtm,
        );
        select_btn.setEnabled(!self.busy);
        select_btn.setTransparent(true);
        let mut accessibility = name.to_owned();
        if let Some(revision) = live_revision {
            accessibility.push_str(&format!(
                " · {} {}",
                i18n::text(self.locale, Message::Active),
                i18n::revision_label(self.locale, revision)
            ));
        }
        if let Some(revision) = candidate_revision {
            accessibility.push_str(&format!(
                " · {} {}",
                i18n::text(self.locale, Message::CharacterCandidate),
                i18n::revision_label(self.locale, revision)
            ));
        }
        set_accessibility_label(&select_btn, &accessibility);
        row_box.addSubview(&select_btn);

        // Overflow Popup Menu for managed packs
        if let Some(pack) = pack_record {
            let overflow_menu = pack_menu(
                pack,
                generation,
                self.busy,
                self.locale,
                &self.target,
                self.mtm,
            );
            let overflow_x = row_w - overflow_btn_width - 4.0;
            let overflow_y = (ROW_HEIGHT - 24.0) * 0.5;
            let overflow = popup_button(
                overflow_menu,
                NSRect::new(
                    NSPoint::new(overflow_x, overflow_y),
                    NSSize::new(overflow_btn_width, 24.0),
                ),
                self.mtm,
            );
            let accessibility = format!(
                "{}: {}",
                i18n::text(self.locale, Message::MenuManageCharacter),
                bounded(&pack.name, 64),
            );
            set_accessibility_label(&overflow, &accessibility);
            row_box.addSubview(&overflow);
        }
    }

    fn render_footer(&self, listing: &PackListing, width: f64) {
        let inset = 12.0;
        let available = (width - 2.0 * inset).max(1.0);
        let candidate = self.selection.candidate();

        let (header_title, header_style) = if let Some(cand) = candidate {
            let is_cand_live = listing
                .active
                .as_ref()
                .is_some_and(|act| act == &cand.reference);
            if is_cand_live {
                (
                    format!(
                        "{} · {}",
                        i18n::text(self.locale, Message::CharacterCandidate),
                        i18n::text(self.locale, Message::Active)
                    ),
                    LabelStyle::Success,
                )
            } else {
                (
                    format!("● {}", i18n::text(self.locale, Message::CharacterCandidate)),
                    LabelStyle::Accent,
                )
            }
        } else {
            (
                i18n::text(self.locale, Message::CharacterCandidate).to_owned(),
                LabelStyle::Secondary,
            )
        };
        add_label(
            &self.footer,
            &header_title,
            NSRect::new(NSPoint::new(inset, 7.0), NSSize::new(available, 15.0)),
            header_style,
            1,
            self.mtm,
        );

        let candidate_full_name = candidate.map(|candidate| {
            let name = if candidate.reference.id == "default" {
                i18n::text(self.locale, Message::RubeliaBuiltIn)
            } else {
                listing
                    .packs
                    .iter()
                    .find(|pack| pack.id == candidate.reference.id)
                    .map_or(candidate.reference.id.as_str(), |pack| pack.name.as_str())
            };
            format!(
                "{} · {}",
                name,
                i18n::revision_label(self.locale, candidate.reference.revision)
            )
        });
        let display_name = candidate_full_name.as_deref().unwrap_or("—");
        let name_label = add_label(
            &self.footer,
            &bounded(display_name, 45),
            NSRect::new(NSPoint::new(inset, 22.0), NSSize::new(available, 18.0)),
            LabelStyle::PrimaryBold,
            1,
            self.mtm,
        );
        set_tooltip(&name_label, display_name);
        set_accessibility_label(&name_label, display_name);

        let operation_visible = self.selection.operation_visible_for_candidate();
        let status = if operation_visible {
            self.selection.operation_status()
        } else {
            None
        };
        let reported = if operation_visible {
            self.selection.operation()
        } else {
            None
        };
        let (message, style) = match status {
            Some(OperationStatus::AwaitingSubmission | OperationStatus::Accepted) => {
                (Message::CharacterOperationQueued, LabelStyle::Accent)
            }
            Some(OperationStatus::Preparing) => {
                (Message::CharacterOperationPreparing, LabelStyle::Accent)
            }
            Some(OperationStatus::Applying) => {
                (Message::CharacterOperationApplying, LabelStyle::Accent)
            }
            Some(OperationStatus::Completed) => {
                (Message::CharacterOperationCompleted, LabelStyle::Success)
            }
            Some(OperationStatus::Failed) => (Message::CharacterOperationFailed, LabelStyle::Error),
            Some(OperationStatus::Canceled) => {
                (Message::CharacterOperationCanceled, LabelStyle::Warning)
            }
            Some(OperationStatus::CommittedPendingApply) => {
                (Message::CharacterOperationPendingApply, LabelStyle::Warning)
            }
            Some(
                OperationStatus::MissingStatus
                | OperationStatus::DurabilityUnknown
                | OperationStatus::Unknown,
            ) => (Message::CharacterOperationUnknown, LabelStyle::Error),
            None if operation_visible && self.selection.operation_rejected() => {
                (Message::CharacterOperationFailed, LabelStyle::Error)
            }
            None if self.selection.stale() => {
                (Message::CharacterSelectionStale, LabelStyle::Warning)
            }
            None if candidate.is_some() && !self.selection.can_apply() => {
                (Message::Active, LabelStyle::Success)
            }
            None if candidate.is_some() => (Message::CharacterApply, LabelStyle::Accent),
            None => (Message::CharacterSelectionPrompt, LabelStyle::Secondary),
        };
        let mut status_text = i18n::text(self.locale, message).to_owned();
        if self.selection.stale() && status.is_some() {
            status_text.push_str(" · ");
            status_text.push_str(i18n::text(self.locale, Message::CharacterSelectionStale));
        }
        let error = if operation_visible {
            self.selection.operation_error()
        } else {
            None
        }
        .or_else(|| reported.and_then(|op| op.error.as_deref()))
        .or(listing.error.as_deref());
        if let Some(error) = error {
            status_text = format!("⚠ {status_text} · {error}");
        }
        let status_label = add_label(
            &self.footer,
            &bounded(&status_text, 75),
            NSRect::new(NSPoint::new(inset, 42.0), NSSize::new(available, 34.0)),
            if error.is_some() {
                LabelStyle::Error
            } else {
                style
            },
            2,
            self.mtm,
        );
        let tooltip = match (
            operation_visible
                .then(|| self.selection.operation_id())
                .flatten(),
            operation_visible && self.selection.operation_rejected(),
        ) {
            (Some(id), true) => format!(
                "{status_text} · {id} · {}",
                i18n::text(self.locale, Message::CharacterOperationNotSubmitted)
            ),
            (Some(id), false) => format!("{status_text} · {id}"),
            (None, _) => status_text.clone(),
        };
        set_tooltip(&status_label, &tooltip);
        set_accessibility_label(&status_label, &tooltip);

        let button_width = ((available - 8.0) / 2.0).max(1.0);
        let apply = action_button(
            i18n::text(self.locale, Message::CharacterApply),
            NSRect::new(
                NSPoint::new(inset, 80.0),
                NSSize::new(button_width, BUTTON_HEIGHT),
            ),
            sel!(packApply:),
            &self.target,
            self.mtm,
            true,
        );
        apply.setEnabled(self.selection.can_apply() && !self.busy);
        set_tooltip(&apply, i18n::text(self.locale, Message::CharacterApply));
        self.footer.addSubview(&apply);

        let cancel = action_button(
            i18n::text(self.locale, Message::CharacterCancelSelection),
            NSRect::new(
                NSPoint::new(inset + button_width + 8.0, 80.0),
                NSSize::new(button_width, BUTTON_HEIGHT),
            ),
            sel!(packCancelSelection:),
            &self.target,
            self.mtm,
            false,
        );
        cancel.setEnabled(candidate.is_some() && !self.busy);
        set_tooltip(
            &cancel,
            i18n::text(self.locale, Message::CharacterCancelSelection),
        );
        self.footer.addSubview(&cancel);
    }

    fn remove_document_subviews(&self) {
        let subviews = self.document.subviews();
        for index in (0..subviews.count()).rev() {
            subviews.objectAtIndex(index).removeFromSuperview();
        }
    }
}

pub(crate) fn command_from_sender(sender: Option<&AnyObject>) -> Option<MenuCommand> {
    let sender = sender?;
    if let Some(item) = sender.downcast_ref::<NSMenuItem>() {
        return command_from_item(item);
    }
    if let Some(button) = sender.downcast_ref::<NSButton>() {
        if let Some(cell) = button.cell() {
            if let Some(represented) = cell.representedObject() {
                if let Some(payload) = represented.downcast_ref::<NSString>() {
                    return parse_payload(&payload.to_string());
                }
            }
        }
    }
    if let Some(popup) = sender.downcast_ref::<NSPopUpButton>() {
        if let Some(item) = popup.selectedItem() {
            return command_from_item(&item);
        }
    }
    None
}

fn command_from_item(item: &NSMenuItem) -> Option<MenuCommand> {
    let represented = item.representedObject()?;
    let payload = represented.downcast_ref::<NSString>()?;
    parse_payload(&payload.to_string())
}

fn parse_payload(payload: &str) -> Option<MenuCommand> {
    let mut fields = payload.splitn(4, '\n');
    let action = fields.next()?;
    if action == "inspect" {
        let id = fields.next()?.to_owned();
        return Some(MenuCommand::Inspect { id });
    }
    let generation = fields.next()?.parse().ok()?;
    let id = fields.next()?.to_owned();
    let revision = fields.next().and_then(|value| value.parse().ok());
    match action {
        "select" => Some(MenuCommand::Select { id, generation }),
        "update" => Some(MenuCommand::Update { id, generation }),
        "restore" => Some(MenuCommand::Restore {
            id,
            revision: revision?,
            generation,
        }),
        "remove" => Some(MenuCommand::Remove { id, generation }),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LabelStyle {
    Heading,
    PrimaryBold,
    Secondary,
    Accent,
    Success,
    Warning,
    Error,
}

fn add_label(
    parent: &NSView,
    title: &str,
    frame: NSRect,
    style: LabelStyle,
    max_lines: isize,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(title), mtm);
    label.setFrame(frame);
    label.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
    label.setAlignment(NSTextAlignment::Left);
    label.setDrawsBackground(false);
    label.setBordered(false);
    label.setBezeled(false);
    label.setEditable(false);
    label.setSelectable(false);
    label.setMaximumNumberOfLines(max_lines);
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    set_accessibility_label(&label, title);
    match style {
        LabelStyle::Heading => {
            label.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
            let color = primary_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::PrimaryBold => {
            label.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
            let color = primary_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::Secondary => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            let color = secondary_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::Accent => {
            label.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
            let color = accent_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::Success => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            let color = success_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::Warning => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            let color = warning_color();
            label.setTextColor(Some(&color));
        }
        LabelStyle::Error => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            let color = error_color();
            label.setTextColor(Some(&color));
        }
    }
    parent.addSubview(&label);
    label
}

fn thumbnail_box(
    image: Option<Retained<NSImage>>,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<CharacterThumbView> {
    let thumb_view = CharacterThumbView::new(frame, mtm);
    unsafe {
        let _: () = msg_send![&*thumb_view, setAccessibilityElement: false];
    }
    if let Some(image) = image {
        let pad = 3.0;
        let img_w = (frame.size.width - pad * 2.0).max(1.0);
        let img_h = (frame.size.height - pad * 2.0).max(1.0);
        let img_view = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(NSPoint::new(pad, pad), NSSize::new(img_w, img_h)),
        );
        img_view.setImage(Some(&image));
        img_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        img_view.setEditable(false);
        img_view.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        thumb_view.addSubview(&img_view);
    }
    thumb_view
}

fn action_button(
    title: &str,
    frame: NSRect,
    action: objc2::runtime::Sel,
    target: &MenuTarget,
    mtm: MainThreadMarker,
    primary: bool,
) -> Retained<NSButton> {
    let title_str = NSString::from_str(title);
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &title_str,
            Some(target.as_ref()),
            Some(action),
            mtm,
        )
    };
    button.setFrame(frame);
    button.setControlSize(NSControlSize::Small);
    let font = if primary {
        NSFont::boldSystemFontOfSize(11.5)
    } else {
        NSFont::systemFontOfSize(11.0)
    };
    button.setFont(Some(&font));
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(true);
    let color = if primary {
        accent_color()
    } else {
        primary_color()
    };
    button.setContentTintColor(Some(&color));
    button.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
    set_accessibility_label(&button, title);
    button
}

fn command_button(
    title: &str,
    frame: NSRect,
    action: objc2::runtime::Sel,
    command: &MenuCommand,
    target: &MenuTarget,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = action_button(title, frame, action, target, mtm, false);
    let payload = NSString::from_str(&encode(command));
    if let Some(cell) = button.cell() {
        unsafe {
            cell.setRepresentedObject(Some(payload.as_ref()));
        }
    }
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
    popup.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
    popup
}

fn pack_menu(
    pack: &crate::character_types::PackRecord,
    generation: u64,
    busy: bool,
    locale: UiLocale,
    target: &MenuTarget,
    mtm: MainThreadMarker,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(
        NSMenu::alloc(mtm),
        &NSString::from_str(&bounded(&pack.name, 80)),
    );
    menu.setAutoenablesItems(false);
    let trigger = item("⋯", None, mtm);
    menu.addItem(&trigger);

    let update = command_item(
        i18n::text(locale, Message::Update),
        sel!(packUpdate:),
        &MenuCommand::Update {
            id: pack.id.clone(),
            generation,
        },
        target,
        mtm,
    );
    update.setEnabled(!busy);
    menu.addItem(&update);

    if !pack.revisions.is_empty() {
        let restore_title = NSString::from_str(i18n::text(locale, Message::RestoreRevision));
        let restore_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &restore_title);
        restore_menu.setAutoenablesItems(false);
        for revision in pack.revisions.iter().rev() {
            let restore = command_item(
                &i18n::revision_label(locale, *revision),
                sel!(packRestore:),
                &MenuCommand::Restore {
                    id: pack.id.clone(),
                    revision: *revision,
                    generation,
                },
                target,
                mtm,
            );
            restore.setEnabled(!busy);
            restore_menu.addItem(&restore);
        }
        let restore_item = item(i18n::text(locale, Message::RestoreRevision), None, mtm);
        restore_item.setSubmenu(Some(&restore_menu));
        menu.addItem(&restore_item);
    }

    let inspect = command_item(
        i18n::text(locale, Message::InspectPack),
        sel!(packInspect:),
        &MenuCommand::Inspect {
            id: pack.id.clone(),
        },
        target,
        mtm,
    );
    menu.addItem(&inspect);

    let remove = command_item(
        i18n::text(locale, Message::Remove),
        sel!(packRemove:),
        &MenuCommand::Remove {
            id: pack.id.clone(),
            generation,
        },
        target,
        mtm,
    );
    remove.setEnabled(!busy);
    menu.addItem(&remove);

    menu
}

fn item(
    title: &str,
    action: Option<objc2::runtime::Sel>,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let title = NSString::from_str(title);
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            action,
            ns_string!(""),
        )
    }
}

fn command_item(
    title: &str,
    action: objc2::runtime::Sel,
    command: &MenuCommand,
    target: &MenuTarget,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = item(title, Some(action), mtm);
    unsafe {
        item.setTarget(Some(target.as_ref()));
    }
    let payload = NSString::from_str(&encode(command));
    unsafe {
        item.setRepresentedObject(Some(payload.as_ref()));
    }
    item
}

fn encode(command: &MenuCommand) -> String {
    match command {
        MenuCommand::Select { id, generation } => format!("select\n{generation}\n{id}\n"),
        MenuCommand::Update { id, generation } => format!("update\n{generation}\n{id}\n"),
        MenuCommand::Restore {
            id,
            revision,
            generation,
        } => format!("restore\n{generation}\n{id}\n{revision}"),
        MenuCommand::Remove { id, generation } => format!("remove\n{generation}\n{id}\n"),
        MenuCommand::Inspect { id } => format!("inspect\n{id}"),
    }
}

fn bounded(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        result.push('…');
    }
    result
}

fn set_accessibility_label(view: &NSView, label: &str) {
    let label = NSString::from_str(label);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}

fn set_tooltip(view: &NSView, value: &str) {
    let value = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setToolTip: Some(&*value)];
    }
}

fn primary_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        PRIMARY_RED,
        PRIMARY_GREEN,
        PRIMARY_BLUE,
        1.0,
    )
}

fn secondary_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        SECONDARY_RED,
        SECONDARY_GREEN,
        SECONDARY_BLUE,
        1.0,
    )
}

fn accent_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        ACCENT_RED,
        ACCENT_GREEN,
        ACCENT_BLUE,
        1.0,
    )
}

fn success_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        SUCCESS_RED,
        SUCCESS_GREEN,
        SUCCESS_BLUE,
        1.0,
    )
}

fn warning_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        WARNING_RED,
        WARNING_GREEN,
        WARNING_BLUE,
        1.0,
    )
}

fn error_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        ERROR_RED,
        ERROR_GREEN,
        ERROR_BLUE,
        1.0,
    )
}

fn card_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(CARD_RED, CARD_GREEN, CARD_BLUE, 1.0)
}

fn card_border_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        CARD_BORDER_RED,
        CARD_BORDER_GREEN,
        CARD_BORDER_BLUE,
        CARD_BORDER_ALPHA,
    )
}

fn row_hover_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        ROW_HOVER_RED,
        ROW_HOVER_GREEN,
        ROW_HOVER_BLUE,
        0.5,
    )
}

fn row_selected_color() -> Retained<objc2_app_kit::NSColor> {
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        ROW_HOVER_RED,
        ROW_HOVER_GREEN,
        ROW_HOVER_BLUE,
        0.85,
    )
}
