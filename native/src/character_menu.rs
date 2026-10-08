use crate::character_selection::{CharacterSelection, OperationStatus};
use crate::character_types::{CharacterRef, PackListing};
use crate::i18n::{self, Message, UiLocale};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSBezierPath, NSButton, NSControlSize, NSFont,
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSMenu, NSMenuItem, NSPopUpButton,
    NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{ns_string, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::path::{Path, PathBuf};

use crate::ui::MenuTarget;

const ROOT_WIDTH: f64 = 328.0;
const ROOT_HEIGHT: f64 = 140.0;
const IDLE_HEIGHT: f64 = 72.0;
const CARD_RADIUS: f64 = 10.0;

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

const CARD_RED: f64 = 0.161;
const CARD_GREEN: f64 = 0.161;
const CARD_BLUE: f64 = 0.176;
const CARD_BORDER_RED: f64 = 0.28;
const CARD_BORDER_GREEN: f64 = 0.28;
const CARD_BORDER_BLUE: f64 = 0.32;
const CARD_BORDER_ALPHA: f64 = 0.50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SummaryStatus<'a> {
    Error(&'a str),
    Stale,
    Operation(OperationStatus),
    Candidate,
}

fn summary_status<'a>(
    listing: &'a PackListing,
    selection: &'a CharacterSelection,
) -> Option<SummaryStatus<'a>> {
    let operation_visible = selection.operation_visible_for_candidate();
    let error = listing.error.as_deref().or_else(|| {
        if operation_visible {
            selection.operation_error().or_else(|| {
                selection
                    .operation()
                    .and_then(|operation| operation.error.as_deref())
            })
        } else {
            None
        }
    });
    if let Some(error) = error {
        return Some(SummaryStatus::Error(error));
    }
    if selection.stale() {
        return Some(SummaryStatus::Stale);
    }
    if operation_visible {
        if let Some(status) = selection.operation_status() {
            return Some(SummaryStatus::Operation(status));
        }
    }
    selection.candidate().map(|_| SummaryStatus::Candidate)
}

fn summary_height(listing: Option<&PackListing>, selection: &CharacterSelection) -> f64 {
    if listing.is_some_and(|listing| summary_status(listing, selection).is_some()) {
        ROOT_HEIGHT
    } else {
        IDLE_HEIGHT
    }
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

pub(crate) struct CharacterMenu {
    root: Retained<CharacterMenuDocument>,
    hero: Retained<CharacterCardView>,
    candidate_card: Retained<CharacterCardView>,
    portrait: Retained<NSImageView>,
    current: Retained<NSTextField>,
    active_status: Retained<NSTextField>,
    candidate: Retained<NSTextField>,
    status: Retained<NSTextField>,
    diagnostics: Retained<NSButton>,
    locale: UiLocale,
    listing: Option<PackListing>,
    selection: CharacterSelection,
    busy: bool,
    builtin_image: Option<Retained<NSImage>>,
}

impl CharacterMenu {
    pub(crate) fn new(target: &MenuTarget, locale: UiLocale, mtm: MainThreadMarker) -> Self {
        let root = CharacterMenuDocument::new(
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(ROOT_WIDTH, IDLE_HEIGHT)),
            mtm,
        );
        let hero = CharacterCardView::new(NSRect::default(), mtm);
        let candidate_card = CharacterCardView::new(NSRect::default(), mtm);
        root.addSubview(&hero);
        root.addSubview(&candidate_card);
        let portrait = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::default());
        portrait.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        portrait.setEditable(false);
        hero.addSubview(&portrait);
        let current = add_label(
            &hero,
            "",
            NSRect::default(),
            LabelStyle::PrimaryBold,
            1,
            mtm,
        );
        let active_status = add_label(&hero, "", NSRect::default(), LabelStyle::Success, 1, mtm);
        let candidate = add_label(
            &candidate_card,
            "",
            NSRect::default(),
            LabelStyle::PrimaryBold,
            1,
            mtm,
        );
        let status = add_label(
            &candidate_card,
            "",
            NSRect::default(),
            LabelStyle::Secondary,
            2,
            mtm,
        );
        let diagnostics = action_button(
            "ⓘ",
            NSRect::default(),
            sel!(packDiagnose:),
            target,
            mtm,
            false,
        );
        set_accessibility_label(
            &diagnostics,
            i18n::text(locale, Message::CharacterDiagnostics),
        );
        hero.addSubview(&diagnostics);
        let builtin_image = load_builtin_thumbnail();
        let mut menu = Self {
            root,
            hero,
            candidate_card,
            portrait,
            current,
            active_status,
            candidate,
            status,
            diagnostics,
            locale,
            listing: None,
            selection: CharacterSelection::new(),
            busy: false,
            builtin_image,
        };
        menu.set_candidate_card_visible(false);
        menu.set_frame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(ROOT_WIDTH, IDLE_HEIGHT),
        ));
        menu
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.root
    }

    pub(crate) fn natural_height(&self) -> f64 {
        summary_height(self.listing.as_ref(), &self.selection)
    }

    fn set_candidate_card_visible(&self, visible: bool) {
        self.candidate_card.setHidden(!visible);
        for field in [&self.candidate, &self.status] {
            unsafe {
                let _: () = msg_send![&**field, setAccessibilityElement: visible];
            }
            if let Some(cell) = field.cell() {
                unsafe {
                    let _: () = msg_send![&*cell, setAccessibilityElement: visible];
                }
            }
        }
    }

    pub(crate) fn set_frame(&mut self, frame: NSRect) {
        self.root.setFrame(frame);
        let content = (frame.size.width - 16.0).max(1.0);
        self.hero.setFrame(NSRect::new(
            NSPoint::new(8.0, 8.0),
            NSSize::new(content, 58.0),
        ));
        self.portrait.setFrame(NSRect::new(
            NSPoint::new(10.0, 9.0),
            NSSize::new(40.0, 40.0),
        ));
        self.current.setFrame(NSRect::new(
            NSPoint::new(58.0, 9.0),
            NSSize::new((content - 98.0).max(1.0), 19.0),
        ));
        self.active_status.setFrame(NSRect::new(
            NSPoint::new(58.0, 31.0),
            NSSize::new((content - 98.0).max(1.0), 17.0),
        ));
        self.diagnostics.setFrame(NSRect::new(
            NSPoint::new((content - 32.0).max(0.0), 17.0),
            NSSize::new(24.0, 24.0),
        ));
        self.candidate_card.setFrame(NSRect::new(
            NSPoint::new(8.0, 74.0),
            NSSize::new(content, 60.0),
        ));
        self.candidate.setFrame(NSRect::new(
            NSPoint::new(10.0, 7.0),
            NSSize::new((content - 20.0).max(1.0), 19.0),
        ));
        self.status.setFrame(NSRect::new(
            NSPoint::new(10.0, 27.0),
            NSSize::new((content - 20.0).max(1.0), 28.0),
        ));
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        if self.locale == locale {
            return;
        }
        self.locale = locale;
        set_accessibility_label(
            &self.diagnostics,
            i18n::text(locale, Message::CharacterDiagnostics),
        );
        self.render();
    }

    pub(crate) fn set_portrait(&mut self, image: Option<&NSImage>) {
        let builtin = self.listing.as_ref().is_some_and(|listing| {
            !listing.override_active
                && listing
                    .active
                    .as_ref()
                    .is_some_and(CharacterRef::is_builtin)
        });
        self.portrait.setImage(image.or_else(|| {
            if builtin {
                self.builtin_image.as_deref()
            } else {
                None
            }
        }));
    }

    pub(crate) fn refresh(
        &mut self,
        listing: &PackListing,
        selection: &CharacterSelection,
        busy: bool,
        _target: &MenuTarget,
        _mtm: MainThreadMarker,
    ) {
        if self.listing.as_ref() == Some(listing)
            && &self.selection == selection
            && self.busy == busy
        {
            return;
        }
        self.listing = Some(listing.clone());
        self.selection = selection.clone();
        self.busy = busy;
        self.render();
    }

    fn render(&self) {
        let Some(listing) = self.listing.as_ref() else {
            return;
        };
        let name = |reference: &CharacterRef| -> String {
            if reference.is_builtin() {
                i18n::text(self.locale, Message::RubeliaBuiltIn).to_owned()
            } else {
                listing
                    .packs
                    .iter()
                    .find(|pack| pack.id == reference.id)
                    .map_or_else(|| reference.id.clone(), |pack| pack.name.clone())
            }
        };
        let active = listing.active.as_ref().unwrap_or(&listing.selected);
        let active_name = if listing.override_active {
            i18n::text(self.locale, Message::ActiveOverride).to_owned()
        } else {
            format!(
                "{} · {}",
                name(active),
                i18n::revision_label(self.locale, active.revision)
            )
        };
        self.current
            .setStringValue(&NSString::from_str(&active_name));
        set_tooltip(&self.current, &active_name);
        set_accessibility_label(&self.current, &active_name);
        let active_state = i18n::text(
            self.locale,
            if listing.override_active {
                Message::ActiveOverride
            } else if listing.active.is_none() {
                Message::ActivePending
            } else {
                Message::Active
            },
        );
        self.active_status
            .setStringValue(&NSString::from_str(active_state));
        set_accessibility_label(&self.active_status, active_state);
        let candidate = self.selection.candidate();
        let summary = summary_status(listing, &self.selection);
        if let Some(summary) = summary {
            let candidate_name = candidate.map_or_else(
                || i18n::text(self.locale, Message::Operation).to_owned(),
                |candidate| {
                    format!(
                        "{} · {}",
                        name(&candidate.reference),
                        i18n::revision_label(self.locale, candidate.reference.revision),
                    )
                },
            );
            self.candidate
                .setStringValue(&NSString::from_str(&candidate_name));
            set_tooltip(&self.candidate, &candidate_name);
            set_accessibility_label(&self.candidate, &candidate_name);
            let status = match summary {
                SummaryStatus::Error(error) => format!("⚠ {error}"),
                SummaryStatus::Stale => {
                    i18n::text(self.locale, Message::CharacterSelectionStale).to_owned()
                }
                SummaryStatus::Operation(operation) => i18n::text(
                    self.locale,
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
                .to_owned(),
                SummaryStatus::Candidate => {
                    i18n::text(self.locale, Message::CharacterCandidate).to_owned()
                }
            };
            self.status.setStringValue(&NSString::from_str(&status));
            set_tooltip(&self.status, &status);
            set_accessibility_label(&self.status, &status);
            self.set_candidate_card_visible(true);
        } else {
            self.set_candidate_card_visible(false);
            self.candidate.setStringValue(ns_string!(""));
            self.status.setStringValue(ns_string!(""));
            for field in [&self.candidate, &self.status] {
                field.setToolTip(None);
                set_accessibility_label(field, "");
            }
        }
        self.portrait
            .setImage(if !listing.override_active && active.is_builtin() {
                self.builtin_image.as_deref()
            } else {
                None
            });
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
    PrimaryBold,
    Secondary,
    Success,
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
        LabelStyle::PrimaryBold => {
            label.setFont(Some(&NSFont::boldSystemFontOfSize(12.0)));
            label.setTextColor(Some(&primary_color()));
        }
        LabelStyle::Secondary => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            label.setTextColor(Some(&secondary_color()));
        }
        LabelStyle::Success => {
            label.setFont(Some(&NSFont::systemFontOfSize(10.5)));
            label.setTextColor(Some(&success_color()));
        }
    }
    parent.addSubview(&label);
    label
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

pub(crate) fn pack_menu(
    pack: &crate::character_types::PackRecord,
    generation: u64,
    mutations_blocked: bool,
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
    update.setEnabled(!mutations_blocked);
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
            restore.setEnabled(!mutations_blocked);
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
    remove.setEnabled(!mutations_blocked);
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
    let payload = NSString::from_str(&command_payload(command));
    unsafe {
        item.setRepresentedObject(Some(payload.as_ref()));
    }
    item
}

pub(crate) fn command_payload(command: &MenuCommand) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character_types::{PackOperation, PackRecord};

    fn listing() -> PackListing {
        PackListing {
            generation: 7,
            selected: CharacterRef::builtin(),
            active: Some(CharacterRef::builtin()),
            override_active: false,
            packs: vec![PackRecord {
                id: "cat".into(),
                name: "Cat".into(),
                head: 3,
                revisions: vec![1, 3],
            }],
            error: None,
        }
    }

    fn operation(state: &str, error: Option<&str>) -> PackOperation {
        PackOperation {
            operation_id: "attempt".into(),
            state: state.into(),
            committed: false,
            ui_applied: false,
            generation: None,
            error: error.map(str::to_owned),
        }
    }

    fn assert_summary(
        listing: &PackListing,
        selection: &CharacterSelection,
        expected: Option<SummaryStatus<'_>>,
    ) {
        assert_eq!(summary_status(listing, selection), expected);
        assert_eq!(
            summary_height(Some(listing), selection),
            if expected.is_some() {
                ROOT_HEIGHT
            } else {
                IDLE_HEIGHT
            }
        );
    }

    #[test]
    fn idle_and_hero_only_states_are_compact() {
        let mut listing = listing();
        let mut selection = CharacterSelection::new();
        assert_eq!(summary_height(None, &selection), IDLE_HEIGHT);
        selection.reconcile(&listing);
        assert!(!selection.is_busy());
        assert_summary(&listing, &selection, None);
        listing.error = Some(String::new());
        assert_summary(&listing, &selection, Some(SummaryStatus::Error("")));
        listing.error = None;
        listing.active = None;
        selection.reconcile(&listing);
        assert_summary(&listing, &selection, None);
        listing.override_active = true;
        selection.reconcile(&listing);
        assert_summary(&listing, &selection, None);
        listing.override_active = false;
        listing.active = Some(CharacterRef {
            id: "cat".into(),
            revision: 3,
        });
        selection.reconcile(&listing);
        assert_summary(&listing, &selection, None);
    }

    #[test]
    fn candidate_is_visible_even_when_apply_is_ineligible() {
        let listing = listing();
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing);
        selection.stage_head("default").unwrap();
        assert!(!selection.can_apply());
        assert_summary(&listing, &selection, Some(SummaryStatus::Candidate));
        selection.stage_head("cat").unwrap();
        assert!(selection.can_apply());
        assert_summary(&listing, &selection, Some(SummaryStatus::Candidate));
    }

    #[test]
    fn errors_and_stale_keep_their_precedence_including_empty_errors() {
        let mut listing = listing();
        let mut rejected = CharacterSelection::new();
        rejected.reconcile(&listing);
        rejected.stage_head("cat").unwrap();
        rejected.request_apply("attempt".into()).unwrap();
        rejected.submission_failed("attempt", String::new());
        listing.generation += 1;
        rejected.reconcile(&listing);
        assert!(rejected.stale());
        assert_summary(&listing, &rejected, Some(SummaryStatus::Error("")));
        listing.error = Some("listing error".into());
        assert_summary(
            &listing,
            &rejected,
            Some(SummaryStatus::Error("listing error")),
        );

        listing.error = None;
        let mut observed = CharacterSelection::new();
        observed.reconcile(&listing);
        observed.stage_head("cat").unwrap();
        observed.request_apply("attempt".into()).unwrap();
        observed.record_operation(&operation("failed", Some("operation error")));
        listing.generation += 1;
        observed.reconcile(&listing);
        assert_summary(
            &listing,
            &observed,
            Some(SummaryStatus::Error("operation error")),
        );
        listing.error = Some("listing error".into());
        assert_summary(
            &listing,
            &observed,
            Some(SummaryStatus::Error("listing error")),
        );
        listing.error = None;
        observed.record_operation(&operation("failed", None));
        assert_summary(&listing, &observed, Some(SummaryStatus::Stale));
    }

    #[test]
    fn visible_operation_states_are_retained_without_a_candidate() {
        let listing = listing();
        let states = [
            ("accepted", OperationStatus::Accepted),
            ("preparing", OperationStatus::Preparing),
            ("applying", OperationStatus::Applying),
            ("completed", OperationStatus::Completed),
            ("failed", OperationStatus::Failed),
            ("canceled", OperationStatus::Canceled),
            (
                "committed_pending_apply",
                OperationStatus::CommittedPendingApply,
            ),
            ("durability_unknown", OperationStatus::DurabilityUnknown),
            ("unrecognized", OperationStatus::Unknown),
        ];
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing);
        selection.reserve_other("attempt".into());
        assert!(selection.is_busy());
        assert_summary(
            &listing,
            &selection,
            Some(SummaryStatus::Operation(
                OperationStatus::AwaitingSubmission,
            )),
        );
        selection.reconcile_operation(None);
        assert_summary(
            &listing,
            &selection,
            Some(SummaryStatus::Operation(OperationStatus::MissingStatus)),
        );
        for (state, expected) in states {
            selection.reserve_other("attempt".into());
            selection.record_operation(&operation(state, None));
            assert_summary(
                &listing,
                &selection,
                Some(SummaryStatus::Operation(expected)),
            );
        }
        selection.reserve_other("attempt".into());
        selection.record_operation(&operation("failed", None));
        selection.cancel().unwrap();
        assert_summary(
            &listing,
            &selection,
            Some(SummaryStatus::Operation(OperationStatus::Failed)),
        );
    }

    #[test]
    fn new_candidate_hides_unowned_old_result_but_not_owned_pending_apply() {
        let listing = listing();
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing);
        selection.reserve_other("attempt".into());
        selection.record_operation(&operation("failed", Some("old failure")));
        selection.stage_head("cat").unwrap();
        assert!(!selection.operation_visible_for_candidate());
        assert_summary(&listing, &selection, Some(SummaryStatus::Candidate));

        selection.request_apply("owned".into()).unwrap();
        assert!(selection.operation_visible_for_candidate());
        assert_summary(
            &listing,
            &selection,
            Some(SummaryStatus::Operation(
                OperationStatus::AwaitingSubmission,
            )),
        );
    }

    #[test]
    fn missing_current_observation_never_reuses_its_error() {
        let listing = listing();
        let mut selection = CharacterSelection::new();
        selection.reconcile(&listing);
        selection.reserve_other("attempt".into());
        selection.record_operation(&operation("preparing", Some("previous error")));
        selection.reconcile_operation(None);
        assert!(selection.operation().is_none());
        assert!(selection.last_observation().is_some());
        assert_summary(
            &listing,
            &selection,
            Some(SummaryStatus::Operation(OperationStatus::MissingStatus)),
        );
    }
}
