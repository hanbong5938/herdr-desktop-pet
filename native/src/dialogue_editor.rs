use crate::assets::CharacterMetadata;
use crate::character_types::CharacterRef;
use crate::dialogue::{DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::i18n::{
    default_dialogue, dialogue_slot_message, text, DefaultDialogue, Message, UiLocale,
};
use crate::ui::MenuTarget;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message as ObjcMessage,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBezelStyle, NSBezierPath,
    NSBorderType, NSButton, NSButtonType, NSColor, NSControlSize, NSEvent, NSEventModifierFlags,
    NSFont, NSLineBreakMode, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView, NSTextDelegate,
    NSTextField, NSTextView, NSTextViewDelegate, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSUndoManager};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

const MAX_BYTES: usize = 2048;
const SIDEBAR: f64 = 170.0;

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

fn color(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha)
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

fn card_color() -> Retained<NSColor> {
    color(CARD_RED, CARD_GREEN, CARD_BLUE, 1.0)
}

fn track_color() -> Retained<NSColor> {
    color(TRACK_RED, TRACK_GREEN, TRACK_BLUE, 1.0)
}

type Key = (DialogueTarget, UiLocaleKey, DialogueSlot);

// UiLocale has no ordering; use its stable language token for ordered draft keys.
type UiLocaleKey = &'static str;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialogueChoice {
    pub target: DialogueTarget,
    pub name: String,
    pub reference: Option<CharacterRef>,
    pub generation: u64,
}

#[derive(Default)]
struct Model {
    choices: Vec<DialogueChoice>,
    selected: Option<DialogueChoice>,
    drafts: BTreeMap<Key, String>,
    overrides: DialogueOverrides,
    metadata: Option<CharacterMetadata>,
    ready: bool,
    error: Option<String>,
    reload: bool,
    locale: Option<UiLocale>,
    slot: Option<DialogueSlot>,
}

impl Model {
    fn key(&self) -> Option<Key> {
        Some((
            self.selected.as_ref()?.target.clone(),
            self.locale?.tag(),
            self.slot?,
        ))
    }

    fn valid(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| self.choices.contains(selected))
    }

    fn reference(&self, locale: UiLocale, slot: DialogueSlot) -> Option<String> {
        let original = self.metadata.as_ref().and_then(|metadata| {
            if slot.is_reaction() {
                metadata.dialogue_text("", Some(slot.key()), locale.tag())
            } else {
                metadata.dialogue_text(slot.key(), None, locale.tag())
            }
        });
        original.map(str::to_owned).or_else(|| {
            let key = match slot {
                DialogueSlot::HeadTap => DefaultDialogue::HeadTap,
                DialogueSlot::BodyTap => DefaultDialogue::BodyTap,
                DialogueSlot::Pet => DefaultDialogue::Pet,
                DialogueSlot::Completion => DefaultDialogue::Completion,
                _ => return None,
            };
            Some(default_dialogue(locale, key).to_owned())
        })
    }

    fn baseline(&self, key: &Key) -> String {
        self.overrides
            .entry(&key.0, key.1, key.2)
            .map(str::to_owned)
            .or_else(|| {
                self.reference(
                    if key.1 == "ko" {
                        UiLocale::Ko
                    } else {
                        UiLocale::En
                    },
                    key.2,
                )
            })
            .unwrap_or_default()
    }

    fn value(&self, key: &Key) -> String {
        self.drafts
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.baseline(key))
    }

    fn dirty(&self, key: &Key) -> bool {
        self.drafts
            .get(key)
            .is_some_and(|draft| normalized(draft) != normalized(&self.baseline(key)))
    }

    fn capture(&mut self, value: String) {
        if let Some(key) = self.key() {
            if normalized(&value) == normalized(&self.baseline(&key)) {
                self.drafts.remove(&key);
            } else {
                self.drafts.insert(key, value);
            }
        }
    }
}

fn normalized(value: &str) -> &str {
    if value.trim().is_empty() {
        ""
    } else {
        value
    }
}

struct Feedback {
    model: Rc<RefCell<Model>>,
    count: Retained<NSTextField>,
    error: Retained<NSTextField>,
    save: Retained<NSButton>,
    reset_entry: Retained<NSMenuItem>,
    reset_all: Retained<NSMenuItem>,
    slots: [Retained<NSButton>; 8],
    undo: Retained<NSUndoManager>,
    ui_locale: UiLocale,
    replacing: bool,
}

// NSTextView's own change notification captures drafts synchronously, without entering Ui.
// A dedicated undo manager prevents previous context's edits from leaking into this one.
define_class!(
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetDialogueEditorTextView"]
    #[ivars = RefCell<Option<Feedback>>]
    struct EditorTextView;
    unsafe impl NSObjectProtocol for EditorTextView {}
    unsafe impl NSTextDelegate for EditorTextView {}
    unsafe impl NSTextViewDelegate for EditorTextView {
        #[unsafe(method_id(undoManagerForTextView:))]
        fn delegate_undo(&self, _view: &NSTextView) -> Option<Retained<NSUndoManager>> {
            self.ivars().borrow().as_ref().map(|state| state.undo.retain())
        }
    }
    impl EditorTextView {
        #[unsafe(method_id(undoManager))]
        fn undo_manager(&self) -> Option<Retained<NSUndoManager>> {
            self.ivars().borrow().as_ref().map(|state| state.undo.retain())
        }
        #[unsafe(method(didChangeText))]
        fn did_change_text(&self) {
            let _: () = unsafe { msg_send![super(self), didChangeText] };
            if let Some(feedback) = self.ivars().borrow().as_ref() {
                if !feedback.replacing {
                    let mut model = feedback.model.borrow_mut();
                    model.capture(self.string().to_string());
                    model.error = None;
                }
            }
            self.refresh_feedback();
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            // The editor has no Edit menu, so standard editing key equivalents
            // need to reach the focused text view directly.
            if self
                .window()
                .and_then(|window| window.firstResponder())
                .is_some_and(|responder| Retained::as_ptr(&responder).cast::<()>() == (self as *const Self).cast::<()>())
                && self.handle_edit_shortcut(event)
            {
                return true.into();
            }
            unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // Some window dispatch paths bypass performKeyEquivalent:.
            if !self.handle_edit_shortcut(event) {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }
        #[unsafe(method(insertTab:))]
        fn insert_tab(&self, _sender: Option<&AnyObject>) {
            if let Some(window) = self.window() { window.selectNextKeyView(Some(self)); }
        }
        #[unsafe(method(insertBacktab:))]
        fn insert_backtab(&self, _sender: Option<&AnyObject>) {
            if let Some(window) = self.window() { window.selectPreviousKeyView(Some(self)); }
        }
    }
);

impl EditorTextView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: NSRect::default()] }
    }

    fn handle_edit_shortcut(&self, event: &NSEvent) -> bool {
        let modifiers = event.modifierFlags()
            & (NSEventModifierFlags::Command
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option);
        if !modifiers.contains(NSEventModifierFlags::Command) {
            return false;
        }
        let marked: bool = unsafe { msg_send![self, hasMarkedText] };
        if marked {
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
        match (key.to_ascii_lowercase(), modifiers) {
            (b'z', flags)
                if flags == NSEventModifierFlags::Command
                    || flags == (NSEventModifierFlags::Command | NSEventModifierFlags::Shift) =>
            {
                // Release the feedback borrow before undo invokes didChangeText.
                let undo = self
                    .ivars()
                    .borrow()
                    .as_ref()
                    .map(|state| state.undo.retain());
                if let Some(undo) = undo {
                    if flags.contains(NSEventModifierFlags::Shift) {
                        if undo.canRedo() {
                            undo.redo();
                        }
                    } else if undo.canUndo() {
                        undo.undo();
                    }
                }
                true
            }
            (b'a', flags) if flags == NSEventModifierFlags::Command => {
                unsafe { self.selectAll(None) };
                true
            }
            (b'c', flags) if flags == NSEventModifierFlags::Command => {
                unsafe { self.copy(None) };
                true
            }
            (b'x', flags) if flags == NSEventModifierFlags::Command => {
                unsafe { self.cut(None) };
                true
            }
            (b'v', flags) if flags == NSEventModifierFlags::Command => {
                unsafe { self.paste(None) };
                true
            }
            _ => false,
        }
    }

    fn replace_context(&self, value: &str) {
        self.breakUndoCoalescing();
        if let Some(feedback) = self.ivars().borrow_mut().as_mut() {
            feedback.replacing = true;
        }
        self.setString(&NSString::from_str(value));
        if let Some(feedback) = self.ivars().borrow_mut().as_mut() {
            feedback.undo.removeAllActions();
            feedback.replacing = false;
        }
        self.refresh_feedback();
    }

    fn refresh_feedback(&self) {
        let feedback = self.ivars().borrow();
        let Some(feedback) = feedback.as_ref() else {
            return;
        };
        let model = feedback.model.borrow();
        let locale = feedback.ui_locale;
        let key = model.key();
        let value = self.string().to_string();
        let dirty = key.as_ref().is_some_and(|key| model.dirty(key));
        let bytes = value.len();
        let valid = model.ready && model.valid();
        let status = if !model.valid() {
            text(locale, Message::DialogueTargetUnavailable).to_owned()
        } else if !model.ready {
            text(locale, Message::DialogueLoading).to_owned()
        } else {
            let source = if key
                .as_ref()
                .is_some_and(|key| model.overrides.entry(&key.0, key.1, key.2).is_some())
            {
                Message::DialogueCustomValue
            } else {
                Message::DialogueDefaultValue
            };
            let changes = if dirty {
                Message::DialogueUnsaved
            } else {
                Message::DialogueNoChanges
            };
            format!("{} · {}", text(locale, source), text(locale, changes))
        };
        feedback.count.setStringValue(&NSString::from_str(&format!(
            "{}: {bytes}/{MAX_BYTES} · {status}",
            text(locale, Message::DialogueBytes)
        )));
        let error = if !model.valid() {
            text(locale, Message::DialogueTargetUnavailable)
        } else if !model.ready {
            model
                .error
                .as_deref()
                .unwrap_or(text(locale, Message::DialogueLoading))
        } else if bytes > MAX_BYTES {
            text(locale, Message::DialogueTooLong)
        } else {
            model.error.as_deref().unwrap_or("")
        };
        let error_text = NSString::from_str(error);
        feedback.error.setStringValue(&error_text);
        feedback
            .error
            .setToolTip((!error.is_empty()).then_some(&*error_text));
        ax(&feedback.error, error);
        feedback
            .save
            .setEnabled(valid && dirty && bytes <= MAX_BYTES);
        feedback.reset_entry.setEnabled(
            valid
                && (dirty
                    || key
                        .as_ref()
                        .is_some_and(|key| model.overrides.entry(&key.0, key.1, key.2).is_some())),
        );
        feedback.reset_all.setEnabled(
            valid
                && model.selected.as_ref().is_some_and(|choice| {
                    model.overrides.locales(&choice.target).is_some()
                        || model
                            .drafts
                            .keys()
                            .any(|(target, _, _)| target == &choice.target)
                }),
        );
        for (index, button) in feedback.slots.iter().enumerate() {
            let slot = DialogueSlot::ALL[index];
            let label = text(locale, dialogue_slot_message(slot));
            let marked = model.selected.as_ref().is_some_and(|choice| {
                ["ko", "en"].iter().any(|language| {
                    let key = (choice.target.clone(), *language, slot);
                    model.dirty(&key)
                })
            });
            let title = if marked {
                format!("{label} •")
            } else {
                label.to_owned()
            };
            button.setTitle(&NSString::from_str(&title));
            let accessible = if marked {
                format!("{label}, {}", text(locale, Message::DialogueUnsaved))
            } else {
                label.to_owned()
            };
            ax(button, &accessible);
            button.setEnabled(model.selected.is_some());
        }
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetDialogueEditorCard"]
    struct EditorCard;

    unsafe impl NSObjectProtocol for EditorCard {}

    impl EditorCard {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            card_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0).fill();

            let border = color(
                CARD_BORDER_RED,
                CARD_BORDER_GREEN,
                CARD_BORDER_BLUE,
                CARD_BORDER_ALPHA,
            );
            border.setStroke();
            let border_bounds = NSRect::new(
                NSPoint::new(bounds.origin.x + 0.5, bounds.origin.y + 0.5),
                NSSize::new(
                    (bounds.size.width - 1.0).max(0.0),
                    (bounds.size.height - 1.0).max(0.0),
                ),
            );
            let border_path =
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(border_bounds, 7.5, 7.5);
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl EditorCard {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        unsafe {
            let _: () = msg_send![&*view, setAccessibilityElement: false];
        }
        view
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetDialogueReferenceBox"]
    struct ReferenceBox;

    unsafe impl NSObjectProtocol for ReferenceBox {}

    impl ReferenceBox {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            let bounds = self.bounds();
            track_color().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 6.0, 6.0).fill();

            let border = color(CARD_BORDER_RED, CARD_BORDER_GREEN, CARD_BORDER_BLUE, 0.35);
            border.setStroke();
            let border_bounds = NSRect::new(
                NSPoint::new(bounds.origin.x + 0.5, bounds.origin.y + 0.5),
                NSSize::new(
                    (bounds.size.width - 1.0).max(0.0),
                    (bounds.size.height - 1.0).max(0.0),
                ),
            );
            let border_path =
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(border_bounds, 5.5, 5.5);
            border_path.setLineWidth(1.0);
            border_path.stroke();
        }
    }
);

impl ReferenceBox {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        unsafe {
            let _: () = msg_send![&*view, setAccessibilityElement: false];
        }
        view
    }
}

struct Layout {
    header_card: Retained<EditorCard>,
    sidebar_card: Retained<EditorCard>,
    editor_card: Retained<EditorCard>,
    footer_card: Retained<EditorCard>,
    reference_box: Retained<ReferenceBox>,
    target_label: Retained<NSTextField>,
    target: Retained<NSPopUpButton>,
    language_label: Retained<NSTextField>,
    language: Retained<NSPopUpButton>,
    slot_label: Retained<NSTextField>,
    slots: [Retained<NSButton>; 8],
    text_label: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    original_button: Retained<NSButton>,
    original: Retained<NSTextField>,
    count: Retained<NSTextField>,
    error: Retained<NSTextField>,
    save: Retained<NSButton>,
    reset: Retained<NSPopUpButton>,
    expanded: bool,
}

// Flipped root makes the top controls stable as the window grows; layout is
// recomputed on each native resize so the bottom actions remain visible.
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetDialogueEditorRoot"]
    #[ivars = RefCell<Option<Layout>>]
    struct EditorRoot;
    unsafe impl NSObjectProtocol for EditorRoot {}
    impl EditorRoot {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
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

fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(width.max(1.0), height.max(1.0)),
    )
}

impl EditorRoot {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        unsafe { msg_send![super(this), initWithFrame: frame(0.0, 0.0, 760.0, 560.0)] }
    }

    fn arrange(&self) {
        let layout = self.ivars().borrow();
        let Some(view) = layout.as_ref() else {
            return;
        };
        let bounds = self.bounds();
        let width = bounds.size.width;
        let height = bounds.size.height;

        let margin_x = 14.0;
        let margin_y = 12.0;
        let gap = 8.0;

        // 1. Header Card (Top)
        let header_h = 44.0;
        let header_w = (width - margin_x * 2.0).max(100.0);
        view.header_card
            .setFrame(frame(margin_x, margin_y, header_w, header_h));

        let lang_popup_w = 110.0;
        let lang_label_w = 104.0;
        let lang_group_w = lang_label_w + 6.0 + lang_popup_w;
        let target_label_w = 100.0;
        let target_x = margin_x + 12.0;
        let lang_x = margin_x + header_w - 12.0 - lang_group_w;
        let target_popup_w = (lang_x - target_x - target_label_w - 18.0).max(80.0);

        view.target_label
            .setFrame(frame(target_x, margin_y + 12.0, target_label_w, 20.0));
        view.target.setFrame(frame(
            target_x + target_label_w + 6.0,
            margin_y + 8.0,
            target_popup_w,
            28.0,
        ));

        view.language_label
            .setFrame(frame(lang_x, margin_y + 12.0, lang_label_w, 20.0));
        view.language.setFrame(frame(
            lang_x + lang_label_w + 6.0,
            margin_y + 8.0,
            lang_popup_w,
            28.0,
        ));

        // 2. Footer Card (Bottom)
        let footer_h = 52.0;
        let footer_top = (height - margin_y - footer_h).max(header_h + margin_y + 80.0);
        let footer_w = header_w;
        view.footer_card
            .setFrame(frame(margin_x, footer_top, footer_w, footer_h));

        let save_w = 96.0;
        let reset_w = 104.0;
        let btn_h = 28.0;
        let btn_y = footer_top + 12.0;
        let save_x = margin_x + footer_w - 12.0 - save_w;
        let reset_x = save_x - 8.0 - reset_w;
        view.save.setFrame(frame(save_x, btn_y, save_w, btn_h));
        view.reset.setFrame(frame(reset_x, btn_y, reset_w, btn_h));

        let info_x = margin_x + 12.0;
        let info_w = (reset_x - info_x - 12.0).max(100.0);
        view.count
            .setFrame(frame(info_x, footer_top + 8.0, info_w, 16.0));
        view.error
            .setFrame(frame(info_x, footer_top + 26.0, info_w, 18.0));

        // 3. Middle Area (Sidebar Card & Editor Card)
        let middle_y = margin_y + header_h + gap;
        let middle_h = (footer_top - middle_y - gap).max(100.0);

        let sidebar_w = SIDEBAR;
        view.sidebar_card
            .setFrame(frame(margin_x, middle_y, sidebar_w, middle_h));

        view.slot_label.setFrame(frame(
            margin_x + 10.0,
            middle_y + 8.0,
            sidebar_w - 20.0,
            18.0,
        ));
        let slot_start_y = middle_y + 30.0;
        let slot_avail_h = (middle_h - 38.0).max(1.0);
        let slot_step = (slot_avail_h / 8.0).min(34.0).max(24.0);
        let slot_btn_h = (slot_step - 4.0).max(18.0);
        for (index, button) in view.slots.iter().enumerate() {
            button.setFrame(frame(
                margin_x + 8.0,
                slot_start_y + index as f64 * slot_step,
                sidebar_w - 16.0,
                slot_btn_h,
            ));
        }

        // 4. Editor Card (Right of Sidebar)
        let editor_x = margin_x + sidebar_w + gap;
        let editor_w = (margin_x + header_w - editor_x).max(100.0);
        view.editor_card
            .setFrame(frame(editor_x, middle_y, editor_w, middle_h));

        view.text_label.setFrame(frame(
            editor_x + 12.0,
            middle_y + 8.0,
            editor_w - 24.0,
            18.0,
        ));

        let editor_inner_x = editor_x + 10.0;
        let editor_inner_w = (editor_w - 20.0).max(60.0);
        let editor_top = middle_y + 30.0;

        let orig_btn_h = 24.0;
        let orig_box_h = if view.expanded { 64.0 } else { 0.0 };
        let orig_total_h = if view.expanded {
            orig_btn_h + 4.0 + orig_box_h
        } else {
            orig_btn_h
        };

        let scroll_h = (middle_h - 30.0 - orig_total_h - 14.0).max(60.0);
        view.scroll
            .setFrame(frame(editor_inner_x, editor_top, editor_inner_w, scroll_h));

        let orig_btn_y = editor_top + scroll_h + 6.0;
        view.original_button
            .setFrame(frame(editor_inner_x, orig_btn_y, 180.0, orig_btn_h));

        let orig_box_y = orig_btn_y + orig_btn_h + 4.0;
        view.reference_box.setFrame(frame(
            editor_inner_x,
            orig_box_y,
            editor_inner_w,
            orig_box_h,
        ));
        view.original.setFrame(frame(
            editor_inner_x + 8.0,
            orig_box_y + 6.0,
            (editor_inner_w - 16.0).max(1.0),
            (orig_box_h - 12.0).max(1.0),
        ));
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

fn set_tooltip(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setToolTip: Some(&*label)];
    }
}

fn ax(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}

fn button(
    title: &str,
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let control = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm)
    };
    control.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    control.setBordered(true);
    control.setControlSize(NSControlSize::Small);
    control.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    control.setContentTintColor(Some(&primary_color()));
    control.setRefusesFirstResponder(false);
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(action));
    }
    ax(&control, title);
    control
}

fn popup(
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let control: Retained<NSPopUpButton> = unsafe {
        msg_send![NSPopUpButton::alloc(mtm), initWithFrame: NSRect::default(), pullsDown: false]
    };
    control.setControlSize(NSControlSize::Small);
    control.setFont(Some(&NSFont::systemFontOfSize(11.5)));
    control.setContentTintColor(Some(&primary_color()));
    control.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    control.setRefusesFirstResponder(false);
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(action));
    }
    control
}

pub(crate) struct DialogueEditor {
    window: Retained<NSWindow>,
    root: Retained<EditorRoot>,
    input: Retained<EditorTextView>,
    model: Rc<RefCell<Model>>,
    ui_locale: UiLocale,
}

impl DialogueEditor {
    pub(crate) fn new(target: &MenuTarget, locale: UiLocale, mtm: MainThreadMarker) -> Self {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame(0.0, 0.0, 760.0, 560.0),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setMinSize(NSSize::new(620.0, 480.0));
        window.setBackgroundColor(Some(&color(BG_RED, BG_GREEN, BG_BLUE, 1.0)));
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            let _: () = unsafe { msg_send![&*window, setAppearance: Some(&*appearance)] };
        }
        let root = EditorRoot::new(mtm);
        root.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        window
            .contentView()
            .expect("dialogue window content")
            .addSubview(&root);

        let header_card = EditorCard::new(mtm);
        let sidebar_card = EditorCard::new(mtm);
        let editor_card = EditorCard::new(mtm);
        let footer_card = EditorCard::new(mtm);
        let reference_box = ReferenceBox::new(mtm);
        reference_box.setHidden(true);

        let target_label = label("", 11.0, true, secondary_color(), mtm);
        let target_popup = popup(target, sel!(setDialogueTarget:), mtm);
        let language_label = label("", 11.0, true, secondary_color(), mtm);
        let language = popup(target, sel!(setDialogueLocale:), mtm);
        language.addItemWithTitle(&NSString::from_str(text(locale, Message::KoreanLanguage)));
        language.addItemWithTitle(&NSString::from_str(text(locale, Message::EnglishLanguage)));
        for (index, item) in language
            .menu()
            .expect("language menu")
            .itemArray()
            .iter()
            .enumerate()
        {
            item.setTag(index as isize);
        }
        language.selectItemWithTag(0);
        let slot_label = label("", 11.5, true, primary_color(), mtm);
        let slots = std::array::from_fn(|index| {
            let control = button("", target, sel!(setDialogueSlot:), mtm);
            control.setTag(index as isize);
            control.setButtonType(NSButtonType::PushOnPushOff);
            control
        });
        let text_label = label("", 11.5, true, primary_color(), mtm);
        let input = EditorTextView::new(mtm);
        input.setRichText(false);
        input.setImportsGraphics(false);
        input.setAllowsUndo(true);
        input.setDelegate(Some(ProtocolObject::from_ref(&*input)));
        input.setFont(Some(&NSFont::systemFontOfSize(13.5)));
        input.setTextColor(Some(&primary_color()));
        input.setBackgroundColor(&track_color());
        input.setTextContainerInset(NSSize::new(10.0, 10.0));
        input.setVerticallyResizable(true);
        input.setHorizontallyResizable(false);
        input.setAutomaticQuoteSubstitutionEnabled(false);
        input.setAutomaticDashSubstitutionEnabled(false);
        input.setAutomaticTextReplacementEnabled(false);
        input.setAutomaticSpellingCorrectionEnabled(false);
        if let Some(container) = unsafe { input.textContainer() } {
            container.setWidthTracksTextView(true);
            container.setContainerSize(NSSize::new(500.0, 10_000_000.0));
        }
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![NSScrollView::alloc(mtm), initWithFrame: NSRect::default()] };
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&input));

        let original_button = button("", target, sel!(toggleDialogueOriginal:), mtm);
        original_button.setContentTintColor(Some(&secondary_color()));
        let original = label("", 11.5, false, secondary_color(), mtm);
        original.setMaximumNumberOfLines(3);
        original.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        original.setHidden(true);

        let count = label("", 11.0, false, secondary_color(), mtm);
        let error = label("", 11.0, true, error_color(), mtm);
        error.setMaximumNumberOfLines(1);
        error.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);

        let save = button("", target, sel!(saveDialogue:), mtm);
        save.setFont(Some(&NSFont::boldSystemFontOfSize(11.5)));
        save.setContentTintColor(Some(&accent_color()));
        save.setKeyEquivalent(&NSString::from_str("s"));
        save.setKeyEquivalentModifierMask(objc2_app_kit::NSEventModifierFlags::Command);

        let reset: Retained<NSPopUpButton> = unsafe {
            msg_send![NSPopUpButton::alloc(mtm), initWithFrame: NSRect::default(), pullsDown: true]
        };
        reset.setBezelStyle(NSBezelStyle::AccessoryBarAction);
        reset.setControlSize(NSControlSize::Small);
        reset.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        reset.setContentTintColor(Some(&secondary_color()));
        reset.setRefusesFirstResponder(false);

        let reset_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
        reset_menu.setAutoenablesItems(false);
        for (message, action) in [
            (Message::DialogueResetMenu, None),
            (Message::DialogueResetEntry, Some(sel!(resetDialogueEntry:))),
            (
                Message::DialogueResetCharacter,
                Some(sel!(resetCharacterDialogue:)),
            ),
        ] {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(text(locale, message)),
                    action,
                    &NSString::from_str(""),
                )
            };
            if action.is_some() {
                unsafe {
                    item.setTarget(Some(target));
                }
            }
            reset_menu.addItem(&item);
        }
        reset.setMenu(Some(&reset_menu));
        let reset_entry = reset_menu.itemAtIndex(1).expect("entry reset");
        let reset_all = reset_menu.itemAtIndex(2).expect("character reset");
        let model = Rc::new(RefCell::new(Model {
            locale: Some(UiLocale::Ko),
            slot: Some(DialogueSlot::Idle),
            ..Model::default()
        }));
        *input.ivars().borrow_mut() = Some(Feedback {
            model: model.clone(),
            count: count.retain(),
            error: error.retain(),
            save: save.retain(),
            reset_entry,
            reset_all,
            slots: slots.clone(),
            undo: NSUndoManager::new(mtm),
            ui_locale: locale,
            replacing: false,
        });

        root.addSubview(&*header_card);
        root.addSubview(&*sidebar_card);
        root.addSubview(&*editor_card);
        root.addSubview(&*footer_card);
        root.addSubview(&*reference_box);

        for view in [
            &*target_label as &NSView,
            &*target_popup,
            &*language_label,
            &*language,
            &*slot_label,
            &*text_label,
            &*scroll,
            &*original_button,
            &*original,
            &*count,
            &*error,
            &*save,
            &*reset,
        ] {
            root.addSubview(view);
        }
        for slot in &slots {
            root.addSubview(slot);
        }
        *root.ivars().borrow_mut() = Some(Layout {
            header_card,
            sidebar_card,
            editor_card,
            footer_card,
            reference_box,
            target_label,
            target: target_popup,
            language_label,
            language,
            slot_label,
            slots,
            text_label,
            scroll,
            original_button,
            original,
            count,
            error,
            save,
            reset,
            expanded: false,
        });
        root.arrange();
        window.center();
        let mut editor = Self {
            window,
            root,
            input,
            model,
            ui_locale: locale,
        };
        editor.set_locale(locale);
        editor.refresh();
        editor
    }

    pub(crate) fn show(&mut self) {
        self.window.makeKeyAndOrderFront(None);
        let _ = self.window.makeFirstResponder(Some(&self.input));
    }

    pub(crate) fn shutdown(&self) {
        self.window.orderOut(None);
    }
    pub(crate) fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.ui_locale = locale;
        self.window.setTitle(&NSString::from_str(text(
            locale,
            Message::DialogueWindowTitle,
        )));
        let layout = self.root.ivars().borrow();
        let view = layout.as_ref().expect("editor layout");
        for (field, message) in [
            (&view.target_label, Message::DialogueTarget),
            (&view.language_label, Message::DialogueLanguage),
            (&view.slot_label, Message::DialogueSlot),
            (&view.text_label, Message::DialogueText),
        ] {
            field.setStringValue(&NSString::from_str(text(locale, message)));
            ax(field, text(locale, message));
        }
        ax(&view.target, text(locale, Message::DialogueTarget));
        ax(&view.language, text(locale, Message::DialogueLanguage));
        ax(&*self.input, text(locale, Message::DialogueText));
        let draft_help = NSString::from_str(text(locale, Message::DialogueDraftSessionHelp));
        self.input.setToolTip(Some(&draft_help));
        view.count.setToolTip(Some(&draft_help));
        for (index, message) in [Message::KoreanLanguage, Message::EnglishLanguage]
            .iter()
            .enumerate()
        {
            view.language
                .menu()
                .expect("language menu")
                .itemAtIndex(index as isize)
                .expect("language item")
                .setTitle(&NSString::from_str(text(locale, *message)));
        }
        for (index, message) in [
            Message::DialogueResetMenu,
            Message::DialogueResetEntry,
            Message::DialogueResetCharacter,
        ]
        .iter()
        .enumerate()
        {
            view.reset
                .menu()
                .expect("reset menu")
                .itemAtIndex(index as isize)
                .expect("reset item")
                .setTitle(&NSString::from_str(text(locale, *message)));
        }
        ax(&view.reset, text(locale, Message::DialogueResetMenu));
        view.save
            .setTitle(&NSString::from_str(text(locale, Message::DialogueSave)));
        ax(&view.save, text(locale, Message::DialogueSave));
        if let Some(feedback) = self.input.ivars().borrow_mut().as_mut() {
            feedback.ui_locale = locale;
        }
        drop(layout);
        self.refresh();
    }

    pub(crate) fn sync_choices(
        &mut self,
        choices: &[DialogueChoice],
        initial_target: Option<&DialogueTarget>,
        initial_reference: Option<&CharacterRef>,
    ) {
        let mut model = self.model.borrow_mut();
        if let Some(selected) = model.selected.as_ref() {
            if let Some(updated) = choices.iter().find(|choice| {
                choice.target == selected.target && choice.reference == selected.reference
            }) {
                if updated.generation != selected.generation {
                    model.ready = false;
                    model.metadata = None;
                }
                model.selected = Some(updated.clone());
            }
        } else {
            model.selected = initial_target
                .and_then(|target| {
                    choices.iter().find(|choice| {
                        &choice.target == target
                            && initial_reference.is_some_and(|reference| {
                                choice.reference.as_ref() == Some(reference)
                            })
                    })
                })
                .or_else(|| {
                    initial_target
                        .and_then(|target| choices.iter().find(|choice| &choice.target == target))
                })
                .or_else(|| choices.first())
                .cloned();
        }
        model.choices = choices.to_vec();
        let target = model.selected.clone();
        drop(model);
        let view = self.root.ivars().borrow();
        let popup = &view.as_ref().expect("editor layout").target;
        popup.removeAllItems();
        for choice in choices {
            popup.addItemWithTitle(&NSString::from_str(&choice.name));
        }
        if let Some(selected) = target.as_ref() {
            if let Some(index) = choices.iter().position(|choice| choice == selected) {
                popup.selectItemAtIndex(index as isize);
            } else {
                popup.addItemWithTitle(&NSString::from_str(&selected.name));
                popup.selectItemAtIndex(choices.len() as isize);
            }
        }
        drop(view);
        self.refresh();
    }

    pub(crate) fn select_target(&mut self, index: usize) {
        let mut model = self.model.borrow_mut();
        let Some(choice) = model.choices.get(index).cloned() else {
            return;
        };
        if model.selected.as_ref() == Some(&choice) {
            return;
        }
        model.selected = Some(choice);
        model.metadata = None;
        model.ready = false;
        model.error = None;
        drop(model);
        self.load_context();
    }

    pub(crate) fn select_locale(&mut self, locale: UiLocale) {
        if self.model.borrow().locale == Some(locale) {
            return;
        }
        self.model.borrow_mut().locale = Some(locale);
        self.load_context();
    }

    pub(crate) fn select_slot(&mut self, slot: DialogueSlot) {
        if self.model.borrow().slot == Some(slot) {
            // Clicking an active native toggle turns it Off before the action.
            // Reassert the model selection without replacing its current draft.
            self.refresh();
            return;
        }
        self.model.borrow_mut().slot = Some(slot);
        self.load_context();
    }

    pub(crate) fn toggle_original(&mut self) {
        {
            let mut layout = self.root.ivars().borrow_mut();
            let view = layout.as_mut().expect("editor layout");
            view.expanded = !view.expanded;
            view.original.setHidden(!view.expanded);
            view.reference_box.setHidden(!view.expanded);
        }
        self.root.arrange();
        self.refresh();
    }

    pub(crate) fn choice(&self) -> Option<DialogueChoice> {
        self.model.borrow().selected.clone()
    }
    pub(crate) fn is_dirty(&self) -> bool {
        let model = self.model.borrow();
        model.key().is_some_and(|key| model.dirty(&key))
    }
    pub(crate) fn edit(&mut self) -> Option<(DialogueChoice, UiLocale, DialogueSlot, String)> {
        let model = self.model.borrow();
        let choice = model.selected.as_ref()?.clone();
        let locale = model.locale?;
        let slot = model.slot?;
        if !model.ready || !model.valid() {
            return None;
        }
        Some((choice, locale, slot, self.input.string().to_string()))
    }

    pub(crate) fn sync_content(
        &mut self,
        overrides: &DialogueOverrides,
        metadata: Option<&CharacterMetadata>,
        ready: bool,
        error: Option<&str>,
    ) {
        let mut model = self.model.borrow_mut();
        let previous = model.key().map(|key| model.value(&key));
        model.overrides = overrides.clone();
        model.metadata = metadata.cloned();
        model.ready = ready;
        model.error = error.map(str::to_owned);
        let value = model.key().map(|key| model.value(&key)).unwrap_or_default();
        let should_replace = model.reload
            || (previous.as_ref().is_some_and(|old| old != &value)
                && !model
                    .key()
                    .is_some_and(|key| model.drafts.contains_key(&key)));
        model.reload = false;
        drop(model);
        if should_replace {
            self.input.replace_context(&value);
        }
        self.refresh();
    }

    pub(crate) fn saved(&mut self, target: &DialogueTarget, locale: UiLocale, slot: DialogueSlot) {
        let key = (target.clone(), locale.tag(), slot);
        let mut model = self.model.borrow_mut();
        let current = model.key().as_ref() == Some(&key);
        model.reload |= current;
        model.drafts.remove(&key);
        // The caller follows this with sync_content(new preferences). Do not reload
        // the old committed value during the interval before that snapshot arrives.
    }

    pub(crate) fn reset(&mut self, target: &DialogueTarget) {
        let mut model = self.model.borrow_mut();
        let current = model
            .selected
            .as_ref()
            .is_some_and(|choice| &choice.target == target);
        model.reload |= current;
        model.drafts.retain(|(choice, _, _), _| choice != target);
    }

    pub(crate) fn set_error(&mut self, error: Option<&str>) {
        self.model.borrow_mut().error = error.map(str::to_owned);
        self.refresh();
    }

    fn load_context(&mut self) {
        let model = self.model.borrow();
        let value = model.key().map(|key| model.value(&key)).unwrap_or_default();
        drop(model);
        self.input.replace_context(&value);
        self.refresh();
    }

    fn refresh(&self) {
        let model = self.model.borrow();
        let layout = self.root.ivars().borrow();
        let view = layout.as_ref().expect("editor layout");
        view.language
            .selectItemWithTag(if model.locale == Some(UiLocale::Ko) {
                0
            } else {
                1
            });
        let expanded = view.expanded;
        view.original_button.setTitle(&NSString::from_str(text(
            self.ui_locale,
            if expanded {
                Message::DialogueHideOriginal
            } else {
                Message::DialogueShowOriginal
            },
        )));
        ax(
            &view.original_button,
            text(
                self.ui_locale,
                if expanded {
                    Message::DialogueHideOriginal
                } else {
                    Message::DialogueShowOriginal
                },
            ),
        );
        let original = model
            .locale
            .zip(model.slot)
            .and_then(|(locale, slot)| model.reference(locale, slot))
            .unwrap_or_else(|| text(self.ui_locale, Message::DialogueStatusReference).to_owned());
        view.original.setStringValue(&NSString::from_str(&original));
        ax(&view.original, &original);
        set_tooltip(&view.original, &original);
        let accent = accent_color();
        for (index, button) in view.slots.iter().enumerate() {
            let selected = model.slot == Some(DialogueSlot::ALL[index]);
            button.setState(if selected {
                objc2_app_kit::NSControlStateValueOn
            } else {
                objc2_app_kit::NSControlStateValueOff
            });
            button.setBezelColor(if selected {
                Some(accent.as_ref())
            } else {
                None
            });
            let tint = if selected {
                primary_color()
            } else {
                secondary_color()
            };
            button.setContentTintColor(Some(&tint));
            let font = if selected {
                NSFont::boldSystemFontOfSize(11.5)
            } else {
                NSFont::systemFontOfSize(11.0)
            };
            button.setFont(Some(&font));
        }
        drop(layout);
        drop(model);
        self.input.refresh_feedback();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_remain_isolated_by_target_locale_and_slot() {
        let target = DialogueTarget::Character("default".to_owned());
        let other = DialogueTarget::Character("other".to_owned());
        let mut model = Model {
            selected: Some(DialogueChoice {
                target: target.clone(),
                name: "Default".to_owned(),
                reference: None,
                generation: 1,
            }),
            locale: Some(UiLocale::Ko),
            slot: Some(DialogueSlot::Idle),
            ..Model::default()
        };
        let first = model.key().unwrap();
        model.capture("첫 초안".to_owned());
        model.locale = Some(UiLocale::En);
        let second = model.key().unwrap();
        model.capture("Second draft".to_owned());
        model.selected.as_mut().unwrap().target = other;
        let third = model.key().unwrap();
        model.capture("Third draft".to_owned());
        model.selected.as_mut().unwrap().target = target;
        assert_eq!(model.value(&first), "첫 초안");
        assert_eq!(model.value(&second), "Second draft");
        model.drafts.remove(&second);
        assert!(!model.dirty(&second));
        assert!(model.dirty(&first));
        assert!(model.dirty(&third));
    }

    #[test]
    fn committed_override_remains_baseline_when_metadata_is_missing() {
        let target = DialogueTarget::Character("default".to_owned());
        let key = (target.clone(), "ko", DialogueSlot::HeadTap);
        let mut model = Model {
            selected: Some(DialogueChoice {
                target: target.clone(),
                name: "Default".to_owned(),
                reference: None,
                generation: 1,
            }),
            locale: Some(UiLocale::Ko),
            slot: Some(DialogueSlot::HeadTap),
            ..Model::default()
        };
        model
            .overrides
            .set_entry(
                &target,
                "ko",
                DialogueSlot::HeadTap,
                Some("Saved".to_owned()),
            )
            .unwrap();
        model.capture("Unsaved".to_owned());
        model.metadata = Some(CharacterMetadata { dialogue: None });
        model.ready = false;
        assert_eq!(model.baseline(&key), "Saved");
        assert_eq!(model.value(&key), "Unsaved");
    }
}
