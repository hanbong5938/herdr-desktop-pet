use crate::assets::CharacterMetadata;
use crate::character_types::CharacterRef;
use crate::dialogue::{DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::dialogue_automation::{metadata_token, DialogueIdentity};
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
use objc2_foundation::{
    NSArray, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSUndoManager, NSValue,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialogueDraftBaseline {
    pub choice: DialogueChoice,
    pub locale: UiLocale,
    pub slot: DialogueSlot,
    pub override_entry: Option<String>,
    pub authored_reference: Option<String>,
    pub metadata_token: String,
}

impl DialogueDraftBaseline {
    fn saved_text(&self) -> &str {
        self.override_entry
            .as_deref()
            .or(self.authored_reference.as_deref())
            .or_else(|| {
                match self.slot {
                    DialogueSlot::HeadTap => Some(DefaultDialogue::HeadTap),
                    DialogueSlot::BodyTap => Some(DefaultDialogue::BodyTap),
                    DialogueSlot::Pet => Some(DefaultDialogue::Pet),
                    DialogueSlot::Completion => Some(DefaultDialogue::Completion),
                    _ => None,
                }
                .map(|default| default_dialogue(self.locale, default))
            })
            .unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HydrationSync {
    Settled,
    DeferredMarked,
}

struct HydrationAction {
    replacement: Option<(String, bool)>,
    result: HydrationSync,
}

// The text view owns undo and selection; the model only decides whether an
// incoming saved value may be displayed in its current context.
#[derive(Default)]
struct DisplayContext {
    key: Option<Key>,
    ready_value_installed: bool,
    edited_before_initial_value: bool,
    deferred_marked: bool,
}

impl DisplayContext {
    fn switch_to(&mut self, key: Option<Key>, has_draft: bool) {
        self.key = key;
        self.ready_value_installed = false;
        self.edited_before_initial_value = has_draft;
        self.deferred_marked = false;
    }
}

#[derive(Default)]
struct Model {
    choices: Vec<DialogueChoice>,
    selected: Option<DialogueChoice>,
    drafts: BTreeMap<Key, String>,
    baselines: BTreeMap<Key, DialogueDraftBaseline>,
    saved_keys: BTreeSet<Key>,
    display: DisplayContext,
    pre_metadata_edits: BTreeSet<Key>,
    reset_targets: BTreeSet<DialogueTarget>,
    overrides: DialogueOverrides,
    metadata: Option<CharacterMetadata>,
    latest_metadata_token: Option<(DialogueIdentity, String)>,
    ready: bool,
    // Save/reset/action failure reported via set_error; cleared by edits.
    error: Option<String>,
    // Metadata/target load failure from Ui::sync_dialogue_editor_content; only
    // sync_content records it, so it survives edits made while loading.
    load_error: Option<String>,
    reload: bool,
    locale: Option<UiLocale>,
    slot: Option<DialogueSlot>,
}

fn same_metadata(left: Option<&CharacterMetadata>, right: Option<&CharacterMetadata>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => match (&left.dialogue, &right.dialogue) {
            (None, None) => true,
            (Some(left), Some(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(
                        |((left_locale, left_text), (right_locale, right_text))| {
                            left_locale == right_locale
                                && left_text.phases == right_text.phases
                                && left_text.reactions == right_text.reactions
                        },
                    )
            }
            _ => false,
        },
        _ => false,
    }
}

impl Model {
    fn key(&self) -> Option<Key> {
        Some((
            self.selected.as_ref()?.target.clone(),
            self.locale?.tag(),
            self.slot?,
        ))
    }

    fn reconcile_choices(
        &mut self,
        choices: &[DialogueChoice],
        initial_target: Option<&DialogueTarget>,
        initial_reference: Option<&CharacterRef>,
        ui_locale: UiLocale,
    ) {
        let previous = self.selected.as_ref();
        let requested = previous
            .map(|choice| (&choice.target, choice.reference.as_ref()))
            .or_else(|| {
                initial_target.map(|target| {
                    let reference = match target {
                        DialogueTarget::Character(_) => initial_reference,
                        DialogueTarget::ExternalAssets(_) => None,
                    };
                    (target, reference)
                })
            });
        let exact = requested.and_then(|(target, reference)| {
            choices
                .iter()
                .find(|choice| &choice.target == target && choice.reference.as_ref() == reference)
        });
        let selected = if let Some(choice) = exact {
            Some(choice.clone())
        } else if let Some(choice) = previous {
            Some(choice.clone())
        } else if let Some((target, Some(reference))) = requested {
            let generation = choices
                .iter()
                .find(|choice| &choice.target == target)
                .or_else(|| choices.first())
                .map_or(0, |choice| choice.generation);
            Some(DialogueChoice {
                target: target.clone(),
                name: format!(
                    "{} #{} ({})",
                    reference.id,
                    reference.revision,
                    text(ui_locale, Message::DialogueTargetUnavailable)
                ),
                reference: Some(reference.clone()),
                generation,
            })
        } else {
            requested
                .and_then(|(target, _)| choices.iter().find(|choice| &choice.target == target))
                .or_else(|| choices.first())
                .cloned()
        };
        if previous.is_some_and(|previous| {
            selected
                .as_ref()
                .is_none_or(|next| next.generation != previous.generation)
        }) || selected
            .as_ref()
            .is_some_and(|choice| !choices.contains(choice))
        {
            self.ready = false;
            self.metadata = None;
            self.latest_metadata_token = None;
        }
        self.selected = selected;
        self.choices = choices.to_vec();
    }

    fn switch_display_context(&mut self) {
        let key = self.key();
        let has_draft = key
            .as_ref()
            .is_some_and(|key| self.drafts.contains_key(key));
        self.display.switch_to(key, has_draft);
    }

    fn needs_initial_hydration(&self) -> bool {
        self.ready
            && self.valid()
            && self
                .display
                .key
                .as_ref()
                .is_some_and(|(target, locale, slot)| {
                    self.selected
                        .as_ref()
                        .is_some_and(|choice| &choice.target == target)
                        && self.locale.is_some_and(|current| current.tag() == *locale)
                        && self.slot == Some(*slot)
                })
            && self.display.deferred_marked
            && !self.display.ready_value_installed
    }

    fn hydration_action(
        &mut self,
        marked: bool,
        focused: bool,
        own_save: bool,
        current_text: &str,
    ) -> HydrationAction {
        if self.display.key != self.key() {
            self.switch_display_context();
        }
        let Some(key) = self.display.key.as_ref() else {
            return HydrationAction {
                replacement: None,
                result: HydrationSync::Settled,
            };
        };
        if self.drafts.contains_key(key) || self.pre_metadata_edits.contains(key) {
            self.display.edited_before_initial_value = true;
        }
        let initial = self.ready
            && self.valid()
            && !self.display.ready_value_installed
            && !self.display.edited_before_initial_value;
        if marked {
            self.display.deferred_marked = initial;
            self.reload |= own_save;
            return HydrationAction {
                replacement: None,
                result: if initial {
                    HydrationSync::DeferredMarked
                } else {
                    HydrationSync::Settled
                },
            };
        }
        self.display.deferred_marked = false;
        let intentional_reset = self.reload || own_save;
        let protected = focused
            || self.drafts.contains_key(key)
            || self.display.edited_before_initial_value
            || self.conflicted(key);
        let install = intentional_reset || initial || (self.ready && self.valid() && !protected);
        self.reload = false;
        let replacement = if install {
            let value = self.value(key);
            (value != current_text).then_some((value, intentional_reset))
        } else {
            None
        };
        if install && self.ready && self.valid() {
            self.display.ready_value_installed = true;
            self.display.edited_before_initial_value = false;
        }
        HydrationAction {
            replacement,
            result: HydrationSync::Settled,
        }
    }

    fn valid(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| self.choices.contains(selected))
    }

    fn authored_reference(&self, locale: UiLocale, slot: DialogueSlot) -> Option<&str> {
        self.metadata.as_ref().and_then(|metadata| {
            if slot.is_reaction() {
                metadata.dialogue_text("", Some(slot.key()), locale.tag())
            } else {
                metadata.dialogue_text(slot.key(), None, locale.tag())
            }
        })
    }
    fn cache_metadata_token(&mut self) {
        let Some(choice) = self.selected.as_ref() else {
            return;
        };
        let identity = DialogueIdentity::from(choice);
        if self
            .latest_metadata_token
            .as_ref()
            .is_some_and(|(cached, _)| cached == &identity)
        {
            return;
        }
        let token = metadata_token(&identity, self.metadata.as_ref());
        self.latest_metadata_token = Some((identity, token));
    }

    fn snapshot(&self, key: &Key) -> Option<DialogueDraftBaseline> {
        let choice = self
            .selected
            .as_ref()
            .filter(|choice| choice.target == key.0)?
            .clone();
        let locale = if key.1 == "ko" {
            UiLocale::Ko
        } else {
            UiLocale::En
        };
        let authored_reference = self.authored_reference(locale, key.2).map(str::to_owned);
        Some(DialogueDraftBaseline {
            choice,
            locale,
            slot: key.2,
            override_entry: self
                .overrides
                .entry(&key.0, key.1, key.2)
                .map(str::to_owned),
            metadata_token: self.latest_metadata_token.as_ref()?.1.clone(),
            authored_reference,
        })
    }

    fn update_saved_content(
        &mut self,
        overrides: &DialogueOverrides,
        metadata: Option<&CharacterMetadata>,
        ready: bool,
        error: Option<&str>,
        protected: bool,
    ) {
        if &self.overrides != overrides {
            self.overrides = overrides.clone();
        }
        if !same_metadata(self.metadata.as_ref(), metadata) {
            self.latest_metadata_token = None;
            self.metadata = metadata.cloned();
        }
        self.ready = ready;
        self.load_error = error.map(str::to_owned);
        self.error = None;
        self.reconcile(protected);
    }

    fn reconcile(&mut self, protected: bool) {
        if !self.ready || !self.valid() {
            return;
        }
        self.cache_metadata_token();
        let Some(key) = self.key() else { return };
        let saved = self.saved_keys.remove(&key);
        let reset = self.reset_targets.remove(&key.0);
        let missing = !self.baselines.contains_key(&key);
        let changed = self.conflicted(&key);
        if saved || reset || missing || (!protected && changed) {
            if let Some(latest) = self.snapshot(&key) {
                self.baselines.insert(key, latest);
            }
        }
    }

    fn conflicted(&self, key: &Key) -> bool {
        if !self.ready {
            return false;
        }
        let Some(baseline) = self.baselines.get(key) else {
            return false;
        };
        let Some(choice) = self
            .selected
            .as_ref()
            .filter(|choice| choice.target == key.0)
        else {
            return true;
        };
        baseline.choice.target != choice.target
            || baseline.choice.reference != choice.reference
            || baseline.choice.generation != choice.generation
            || baseline.override_entry.as_deref() != self.overrides.entry(&key.0, key.1, key.2)
            || baseline.authored_reference.as_deref()
                != self.authored_reference(baseline.locale, key.2)
            || baseline.metadata_token
                != self
                    .latest_metadata_token
                    .as_ref()
                    .map_or("", |(_, token)| token)
    }

    fn mutation_baseline(&self) -> Option<DialogueDraftBaseline> {
        if !self.ready || !self.valid() {
            return None;
        }
        let key = self.key()?;
        self.baselines.get(&key).cloned()
    }

    fn reload_current(&mut self) -> Option<String> {
        if !self.ready || !self.valid() {
            return None;
        }
        self.cache_metadata_token();
        let key = self.key()?;
        let latest = self.snapshot(&key)?;
        let value = latest.saved_text().to_owned();
        self.baselines.insert(key.clone(), latest);
        self.pre_metadata_edits.remove(&key);
        if self.display.key.as_ref() == Some(&key) {
            self.display.ready_value_installed = true;
            self.display.edited_before_initial_value = false;
            self.display.deferred_marked = false;
        }
        self.drafts.remove(&key);
        self.error = None;
        Some(value)
    }

    fn rebase_current(&mut self, value: String) -> bool {
        if !self.ready || !self.valid() {
            return false;
        }
        self.cache_metadata_token();
        let Some(key) = self.key() else { return false };
        let Some(latest) = self.snapshot(&key) else {
            return false;
        };
        self.baselines.insert(key.clone(), latest);
        self.pre_metadata_edits.remove(&key);
        if self.display.key.as_ref() == Some(&key) {
            self.display.edited_before_initial_value = !self.display.ready_value_installed;
            self.display.deferred_marked = false;
        }
        self.capture(value);
        self.error = None;
        true
    }

    fn mark_saved(&mut self, key: Key) {
        self.saved_keys.insert(key.clone());
        self.drafts.remove(&key);
        self.pre_metadata_edits.remove(&key);
        if self.display.key.as_ref() == Some(&key) {
            self.display.edited_before_initial_value = false;
            self.display.deferred_marked = false;
        }
    }

    fn clear_target_drafts(&mut self, target: &DialogueTarget) {
        self.reset_targets.insert(target.clone());
        self.drafts.retain(|(choice, _, _), _| choice != target);
        self.baselines.retain(|(choice, _, _), _| choice != target);
        self.pre_metadata_edits
            .retain(|(choice, _, _)| choice != target);
        if self
            .display
            .key
            .as_ref()
            .is_some_and(|key| &key.0 == target)
        {
            self.display.edited_before_initial_value = false;
            self.display.deferred_marked = false;
        }
    }
    fn reference(&self, locale: UiLocale, slot: DialogueSlot) -> Option<String> {
        self.authored_reference(locale, slot)
            .map(str::to_owned)
            .or_else(|| {
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
        self.drafts.get(key).cloned().unwrap_or_else(|| {
            self.baselines
                .get(key)
                .map(|baseline| baseline.saved_text().to_owned())
                .unwrap_or_else(|| self.baseline(key))
        })
    }

    fn dirty(&self, key: &Key) -> bool {
        self.drafts.get(key).is_some_and(|draft| {
            if let Some(baseline) = self.baselines.get(key) {
                normalized(draft) != normalized(baseline.saved_text())
            } else {
                normalized(draft) != normalized(&self.baseline(key))
            }
        })
    }
    fn restart_draft_keys(&self) -> BTreeSet<Key> {
        self.drafts
            .keys()
            .chain(self.pre_metadata_edits.iter())
            .filter(|key| self.dirty(key) || self.pre_metadata_edits.contains(*key))
            .cloned()
            .collect()
    }

    fn capture(&mut self, value: String) {
        if let Some(key) = self.key() {
            if self.display.key.as_ref() == Some(&key) && !self.display.ready_value_installed {
                self.display.edited_before_initial_value = true;
                // Unmark may commit a draft; the deferred sync still owes its settling pass.
            }
            if !self.ready {
                self.pre_metadata_edits.insert(key.clone());
                self.drafts.insert(key, value);
                return;
            }
            if !self.baselines.contains_key(&key) {
                self.cache_metadata_token();
                if let Some(snapshot) = self.snapshot(&key) {
                    self.baselines.insert(key.clone(), snapshot);
                }
            }
            let same = if let Some(baseline) = self.baselines.get(&key) {
                normalized(&value) == normalized(baseline.saved_text())
            } else {
                normalized(&value) == normalized(&self.baseline(&key))
            };
            if same && !self.pre_metadata_edits.contains(&key) {
                self.drafts.remove(&key);
            } else {
                self.drafts.insert(key, value);
            }
        }
    }

    fn record_edit(&mut self, value: String) {
        self.capture(value);
        // Edits clear action errors only; load_error persists until sync_content.
        self.error = None;
    }

    fn feedback_error(&self, locale: UiLocale, bytes: usize) -> &str {
        if !self.valid() {
            text(locale, Message::DialogueTargetUnavailable)
        } else if !self.ready {
            self.load_error
                .as_deref()
                .or(self.error.as_deref())
                .unwrap_or(text(locale, Message::DialogueLoading))
        } else if self.key().is_some_and(|key| self.conflicted(&key)) {
            text(locale, Message::DialogueDraftConflict)
        } else if bytes > MAX_BYTES {
            text(locale, Message::DialogueTooLong)
        } else {
            self.error.as_deref().unwrap_or("")
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

struct FrozenEditorControls {
    editable: bool,
    selectable: bool,
    target: bool,
    language: bool,
    slots: [bool; 8],
    original_button: bool,
    save: bool,
    reset: bool,
    reset_entry: bool,
    reset_all: bool,
    reload_saved: bool,
    rebase: bool,
}

struct Feedback {
    model: Rc<RefCell<Model>>,
    count: Retained<NSTextField>,
    error: Retained<NSTextField>,
    save: Retained<NSButton>,
    reset_entry: Retained<NSMenuItem>,
    reset_all: Retained<NSMenuItem>,
    slots: [Retained<NSButton>; 8],
    reload_saved: Retained<NSButton>,
    rebase: Retained<NSButton>,
    undo: Retained<NSUndoManager>,
    ui_locale: UiLocale,
    update_frozen: bool,
    frozen_value: Option<String>,
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
            let restore = {
                let feedback = self.ivars().borrow();
                match feedback.as_ref() {
                    Some(feedback) if feedback.update_frozen && !feedback.replacing => {
                        feedback.frozen_value.clone()
                    }
                    Some(feedback) if !feedback.replacing => {
                        feedback.model.borrow_mut().record_edit(self.string().to_string());
                        None
                    }
                    _ => None,
                }
            };
            if let Some(value) = restore {
                if self.string().to_string() != value {
                    self.replace_saved_text(&value);
                    return;
                }
            }
            self.refresh_feedback();
        }
        #[unsafe(method(shouldChangeTextInRange:replacementString:))]
        fn should_change_text(&self, range: NSRange, replacement: Option<&NSString>) -> bool {
            if self.update_frozen() {
                return false.into();
            }
            unsafe { msg_send![super(self), shouldChangeTextInRange: range, replacementString: replacement] }
        }
        #[unsafe(method(shouldChangeTextInRanges:replacementStrings:))]
        fn should_change_ranges(
            &self,
            ranges: &NSArray<NSValue>,
            replacements: Option<&NSArray<NSString>>,
        ) -> bool {
            if self.update_frozen() {
                return false.into();
            }
            unsafe { msg_send![super(self), shouldChangeTextInRanges: ranges, replacementStrings: replacements] }
        }
        #[unsafe(method(insertText:))]
        fn insert_text(&self, text: &AnyObject) {
            if !self.update_frozen() {
                let _: () = unsafe { msg_send![super(self), insertText: text] };
            }
        }
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text_replacement_range(&self, text: &AnyObject, range: NSRange) {
            if !self.update_frozen() {
                let _: () = unsafe { msg_send![super(self), insertText: text, replacementRange: range] };
            }
        }
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(
            &self,
            text: &AnyObject,
            selected: NSRange,
            replacement: NSRange,
        ) {
            if !self.update_frozen() {
                let _: () = unsafe {
                    msg_send![super(self), setMarkedText: text, selectedRange: selected, replacementRange: replacement]
                };
            }
        }
        #[unsafe(method(undo:))]
        fn undo_action(&self, _sender: Option<&AnyObject>) {
            self.invoke_undo(false);
        }
        #[unsafe(method(redo:))]
        fn redo_action(&self, _sender: Option<&AnyObject>) {
            self.invoke_undo(true);
        }
        #[unsafe(method(cut:))]
        fn cut_action(&self, sender: Option<&AnyObject>) {
            if !self.update_frozen() {
                let _: () = unsafe { msg_send![super(self), cut: sender] };
            }
        }
        #[unsafe(method(paste:))]
        fn paste_action(&self, sender: Option<&AnyObject>) {
            if !self.update_frozen() {
                let _: () = unsafe { msg_send![super(self), paste: sender] };
            }
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
    fn update_frozen(&self) -> bool {
        self.ivars()
            .borrow()
            .as_ref()
            .is_some_and(|feedback| feedback.update_frozen)
    }

    fn invoke_undo(&self, redo: bool) {
        if self.update_frozen() {
            return;
        }
        // Undo calls didChangeText; do not hold the feedback borrow.
        let undo = self
            .ivars()
            .borrow()
            .as_ref()
            .map(|state| state.undo.retain());
        if let Some(undo) = undo {
            if redo {
                if undo.canRedo() {
                    undo.redo();
                }
            } else if undo.canUndo() {
                undo.undo();
            }
        }
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
        if marked && !self.update_frozen() {
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
                self.invoke_undo(flags.contains(NSEventModifierFlags::Shift));
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
                if !self.update_frozen() {
                    unsafe { self.cut(None) };
                }
                true
            }
            (b'v', flags) if flags == NSEventModifierFlags::Command => {
                if !self.update_frozen() {
                    unsafe { self.paste(None) };
                }
                true
            }
            _ => false,
        }
    }

    fn replace_context(&self, value: &str) {
        self.replace_text(value, true);
    }

    fn replace_saved_text(&self, value: &str) {
        self.replace_text(value, false);
    }

    fn replace_text(&self, value: &str, clear_undo: bool) {
        self.breakUndoCoalescing();
        if let Some(feedback) = self.ivars().borrow_mut().as_mut() {
            feedback.replacing = true;
            if feedback.update_frozen {
                feedback.frozen_value = Some(value.to_owned());
            }
        }
        self.setString(&NSString::from_str(value));
        if let Some(feedback) = self.ivars().borrow_mut().as_mut() {
            if clear_undo {
                feedback.undo.removeAllActions();
            }
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
        let conflict = key.as_ref().is_some_and(|key| model.conflicted(key));
        let marked: bool = unsafe { msg_send![self, hasMarkedText] };
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
        let error = model.feedback_error(locale, bytes);
        let error_text = NSString::from_str(error);
        feedback.error.setStringValue(&error_text);
        feedback
            .error
            .setToolTip((!error.is_empty()).then_some(&*error_text));
        ax(&feedback.error, error);
        if !feedback.update_frozen {
            feedback
                .save
                .setEnabled(valid && !conflict && !marked && dirty && bytes <= MAX_BYTES);
            feedback.reset_entry.setEnabled(
                valid
                    && !conflict
                    && !marked
                    && (dirty
                        || key.as_ref().is_some_and(|key| {
                            model.overrides.entry(&key.0, key.1, key.2).is_some()
                        })),
            );
            feedback.reset_all.setEnabled(
                valid
                    && !conflict
                    && !marked
                    && model.selected.as_ref().is_some_and(|choice| {
                        model.overrides.locales(&choice.target).is_some()
                            || model
                                .drafts
                                .keys()
                                .any(|(target, _, _)| target == &choice.target)
                    }),
            );
            feedback
                .reload_saved
                .setEnabled(valid && conflict && !marked);
            feedback.rebase.setEnabled(valid && conflict && !marked);
        }
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
            if !feedback.update_frozen {
                button.setEnabled(model.selected.is_some());
            }
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
    reload_saved: Retained<NSButton>,
    rebase: Retained<NSButton>,
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
        let footer_h = 82.0;
        let footer_top = (height - margin_y - footer_h).max(header_h + margin_y + 80.0);
        let footer_w = header_w;
        view.footer_card
            .setFrame(frame(margin_x, footer_top, footer_w, footer_h));

        let save_w = 96.0;
        let reset_w = 104.0;
        let btn_h = 28.0;
        let btn_y = footer_top + 46.0;
        let save_x = margin_x + footer_w - 12.0 - save_w;
        let reset_x = save_x - 8.0 - reset_w;
        view.save.setFrame(frame(save_x, btn_y, save_w, btn_h));
        view.reset.setFrame(frame(reset_x, btn_y, reset_w, btn_h));

        let info_x = margin_x + 12.0;
        let info_w = (save_x + save_w - info_x).max(100.0);
        view.count
            .setFrame(frame(info_x, footer_top + 8.0, info_w, 16.0));
        view.error
            .setFrame(frame(info_x, footer_top + 26.0, info_w, 18.0));
        let draft_w = 110.0;
        view.reload_saved
            .setFrame(frame(info_x, btn_y, draft_w, btn_h));
        view.rebase
            .setFrame(frame(info_x + draft_w + 8.0, btn_y, draft_w, btn_h));

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
    update_frozen_previous: RefCell<Option<FrozenEditorControls>>,
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
        let reload_saved = button("", target, sel!(reloadDialogueDraft:), mtm);
        let rebase = button("", target, sel!(rebaseDialogueDraft:), mtm);

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
            reload_saved: reload_saved.retain(),
            rebase: rebase.retain(),
            slots: slots.clone(),
            undo: NSUndoManager::new(mtm),
            ui_locale: locale,
            update_frozen: false,
            frozen_value: None,
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
            &*reload_saved,
            &*rebase,
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
            reload_saved,
            rebase,
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
            update_frozen_previous: RefCell::new(None),
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
        for (button, message) in [
            (&view.reload_saved, Message::DialogueReloadSaved),
            (&view.rebase, Message::DialogueRebaseDraft),
        ] {
            button.setTitle(&NSString::from_str(text(locale, message)));
            ax(button, text(locale, message));
        }
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
        model.reconcile_choices(choices, initial_target, initial_reference, self.ui_locale);
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
        model.load_error = None;
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
    pub(crate) fn restart_draft_keys(&self) -> Vec<String> {
        let model = self.model.borrow();
        let mut keys = model.restart_draft_keys();
        if let Some(key) = model.key() {
            let current = self.input.string().to_string();
            if model.pre_metadata_edits.contains(&key)
                || model.baselines.get(&key).map_or_else(
                    || normalized(&current) != normalized(&model.baseline(&key)),
                    |baseline| normalized(&current) != normalized(baseline.saved_text()),
                )
            {
                keys.insert(key);
            }
        }
        keys.into_iter()
            .map(|key| format!("{:?} / {} / {:?}", key.0, key.1, key.2))
            .collect()
    }

    pub(crate) fn set_update_frozen(&self, frozen: bool) {
        let layout = self.root.ivars().borrow();
        let view = layout.as_ref().expect("editor layout");
        let mut previous = self.update_frozen_previous.borrow_mut();
        if frozen {
            if previous.is_some() {
                return;
            }
            *previous = Some(FrozenEditorControls {
                editable: self.input.isEditable(),
                selectable: self.input.isSelectable(),
                target: view.target.isEnabled(),
                language: view.language.isEnabled(),
                slots: std::array::from_fn(|index| view.slots[index].isEnabled()),
                original_button: view.original_button.isEnabled(),
                save: view.save.isEnabled(),
                reset: view.reset.isEnabled(),
                reset_entry: self
                    .input
                    .ivars()
                    .borrow()
                    .as_ref()
                    .expect("feedback")
                    .reset_entry
                    .isEnabled(),
                reset_all: self
                    .input
                    .ivars()
                    .borrow()
                    .as_ref()
                    .expect("feedback")
                    .reset_all
                    .isEnabled(),
                reload_saved: view.reload_saved.isEnabled(),
                rebase: view.rebase.isEnabled(),
            });
            let frozen_value = self.input.string().to_string();
            {
                let mut feedback = self.input.ivars().borrow_mut();
                let feedback = feedback.as_mut().expect("feedback");
                feedback.frozen_value = Some(frozen_value);
                feedback.update_frozen = true;
            }
            self.input.setEditable(false);
            self.input.setSelectable(false);
            view.target.setEnabled(false);
            view.language.setEnabled(false);
            for button in &view.slots {
                button.setEnabled(false);
            }
            view.original_button.setEnabled(false);
            view.save.setEnabled(false);
            view.reset.setEnabled(false);
            view.reload_saved.setEnabled(false);
            view.rebase.setEnabled(false);
            let feedback = self.input.ivars().borrow();
            let feedback = feedback.as_ref().expect("feedback");
            feedback.reset_entry.setEnabled(false);
            feedback.reset_all.setEnabled(false);
        } else if let Some(previous) = previous.take() {
            self.input.setEditable(previous.editable);
            self.input.setSelectable(previous.selectable);
            view.target.setEnabled(previous.target);
            view.language.setEnabled(previous.language);
            for (button, enabled) in view.slots.iter().zip(previous.slots) {
                button.setEnabled(enabled);
            }
            view.original_button.setEnabled(previous.original_button);
            view.save.setEnabled(previous.save);
            view.reset.setEnabled(previous.reset);
            view.reload_saved.setEnabled(previous.reload_saved);
            view.rebase.setEnabled(previous.rebase);
            let mut feedback = self.input.ivars().borrow_mut();
            let feedback = feedback.as_mut().expect("feedback");
            feedback.reset_entry.setEnabled(previous.reset_entry);
            feedback.reset_all.setEnabled(previous.reset_all);
            feedback.update_frozen = false;
            feedback.frozen_value = None;
        }
    }
    pub(crate) fn mutation_baseline(&self) -> Option<DialogueDraftBaseline> {
        self.model.borrow().mutation_baseline()
    }

    pub(crate) fn has_conflict(&self) -> bool {
        let model = self.model.borrow();
        model.key().is_some_and(|key| model.conflicted(&key))
    }

    pub(crate) fn has_marked_text(&self) -> bool {
        unsafe { msg_send![&*self.input, hasMarkedText] }
    }

    fn input_focused(&self) -> bool {
        self.window.firstResponder().is_some_and(|responder| {
            Retained::as_ptr(&responder).cast::<()>() == Retained::as_ptr(&self.input).cast::<()>()
        })
    }

    pub(crate) fn reload_saved_draft(&mut self) -> bool {
        if self.has_marked_text() {
            return false;
        }
        let Some(value) = self.model.borrow_mut().reload_current() else {
            return false;
        };
        if self.input.string().to_string() != value {
            self.input.replace_context(&value);
        }
        self.refresh();
        true
    }

    pub(crate) fn rebase_draft(&mut self) -> bool {
        if self.has_marked_text() {
            return false;
        }
        let value = self.input.string().to_string();
        if !self.model.borrow_mut().rebase_current(value) {
            return false;
        }
        self.refresh();
        true
    }
    pub(crate) fn edit(&mut self) -> Option<(DialogueChoice, UiLocale, DialogueSlot, String)> {
        if self.has_marked_text() {
            return None;
        }
        let model = self.model.borrow();
        let choice = model.selected.as_ref()?.clone();
        let locale = model.locale?;
        let slot = model.slot?;
        if !model.ready || !model.valid() || model.key().is_some_and(|key| model.conflicted(&key)) {
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
    ) -> HydrationSync {
        let marked = self.has_marked_text();
        let focused = self.input_focused();
        let current_text = (!marked).then(|| self.input.string().to_string());
        let mut model = self.model.borrow_mut();
        if model.display.key != model.key() {
            model.switch_display_context();
        }
        let own_save = ready
            && model
                .key()
                .is_some_and(|key| model.saved_keys.contains(&key));
        let initial_untouched = !model.display.ready_value_installed
            && !model.display.edited_before_initial_value
            && model
                .key()
                .is_some_and(|key| !model.drafts.contains_key(&key));
        let protected = marked
            || (focused && !initial_untouched)
            || model.key().is_some_and(|key| {
                model.drafts.contains_key(&key)
                    || model.pre_metadata_edits.contains(&key)
                    || (!initial_untouched && model.conflicted(&key))
            });
        model.update_saved_content(overrides, metadata, ready, error, protected);
        let action = model.hydration_action(
            marked,
            focused,
            own_save,
            current_text.as_deref().unwrap_or(""),
        );
        drop(model);
        if let Some((value, intentional_reset)) = action.replacement {
            if intentional_reset {
                self.input.replace_context(&value);
            } else {
                self.input.replace_saved_text(&value);
            }
        }
        self.refresh();
        action.result
    }

    pub(crate) fn needs_initial_hydration(&self) -> bool {
        self.is_visible() && self.model.borrow().needs_initial_hydration()
    }

    pub(crate) fn saved(&mut self, target: &DialogueTarget, locale: UiLocale, slot: DialogueSlot) {
        let key = (target.clone(), locale.tag(), slot);
        self.model.borrow_mut().mark_saved(key);
        // The committed snapshot in the subsequent sync advances the baseline.
        // Equal text is never rewritten, preserving selection and undo on save.
    }

    pub(crate) fn reset(&mut self, target: &DialogueTarget) {
        let mut model = self.model.borrow_mut();
        let current = model
            .selected
            .as_ref()
            .is_some_and(|choice| &choice.target == target);
        model.reload |= current;
        model.clear_target_drafts(target);
    }

    pub(crate) fn set_error(&mut self, error: Option<&str>) {
        self.model.borrow_mut().error = error.map(str::to_owned);
        self.refresh();
    }

    fn load_context(&mut self) {
        let mut model = self.model.borrow_mut();
        model.switch_display_context();
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

    fn selectable_model() -> Model {
        let choice = DialogueChoice {
            target: DialogueTarget::Character("default".to_owned()),
            name: "Default".to_owned(),
            reference: None,
            generation: 1,
        };
        Model {
            choices: vec![choice.clone()],
            selected: Some(choice),
            locale: Some(UiLocale::En),
            slot: Some(DialogueSlot::Idle),
            ..Model::default()
        }
    }
    #[test]
    fn restart_snapshot_keeps_offscreen_draft_and_raw_pre_metadata_edit() {
        let mut model = selectable_model();
        model.ready = true;
        let offscreen = model.key().unwrap();
        model.record_edit("unsaved idle".into());
        model.slot = Some(DialogueSlot::Running);
        model.ready = false;
        let unfinished = model.key().unwrap();
        model.record_edit("  ".into());
        model.slot = Some(DialogueSlot::Idle);
        let keys = model.restart_draft_keys();
        assert!(keys.contains(&offscreen));
        assert!(keys.contains(&unfinished));
    }

    #[test]
    fn load_error_survives_edits_until_content_sync() {
        let locale = UiLocale::En;
        let mut model = Model {
            load_error: Some("load failed".to_owned()),
            ..selectable_model()
        };
        let key = model.key().unwrap();
        model.record_edit("Draft while loading".to_owned());
        assert_eq!(model.feedback_error(locale, 0), "load failed");
        assert!(model.dirty(&key));

        // A successful sync_content replaces the load failure and clears action errors.
        model.load_error = None;
        model.error = None;
        assert_eq!(
            model.feedback_error(locale, 0),
            text(locale, Message::DialogueLoading)
        );
        model.ready = true;
        assert_eq!(model.feedback_error(locale, 0), "");
        assert!(model.dirty(&key));
    }

    #[test]
    fn edits_clear_action_error_but_not_load_error() {
        let locale = UiLocale::En;
        let mut model = Model {
            ready: true,
            error: Some("save failed".to_owned()),
            ..selectable_model()
        };
        assert_eq!(model.feedback_error(locale, 0), "save failed");
        assert_eq!(
            model.feedback_error(locale, MAX_BYTES + 1),
            text(locale, Message::DialogueTooLong)
        );
        model.record_edit("Edited".to_owned());
        assert_eq!(model.feedback_error(locale, 0), "");

        model.ready = false;
        model.load_error = Some("load failed".to_owned());
        model.error = Some("save failed".to_owned());
        assert_eq!(model.feedback_error(locale, 0), "load failed");
        model.record_edit("Edited again".to_owned());
        assert_eq!(model.error, None);
        assert_eq!(model.feedback_error(locale, 0), "load failed");
        model.load_error = None;
        assert_eq!(
            model.feedback_error(locale, 0),
            text(locale, Message::DialogueLoading)
        );
    }

    #[test]
    fn external_entry_change_preserves_raw_draft_and_requires_explicit_rebase() {
        let mut model = Model {
            ready: true,
            ..selectable_model()
        };
        let key = model.key().unwrap();
        let target = key.0.clone();
        model
            .overrides
            .set_entry(&target, "en", DialogueSlot::Idle, Some("saved ".into()))
            .unwrap();
        model.reconcile(false);
        let baseline = model.mutation_baseline().unwrap();
        assert_eq!(baseline.override_entry.as_deref(), Some("saved "));
        assert_eq!(baseline.authored_reference, None);
        assert_eq!(baseline.choice.generation, 1);
        model.capture(" local  ".into());

        model
            .overrides
            .set_entry(&target, "en", DialogueSlot::Waiting, Some("other".into()))
            .unwrap();
        model.reconcile(true);
        assert!(!model.conflicted(&key));
        assert_eq!(model.mutation_baseline(), Some(baseline.clone()));

        model
            .overrides
            .set_entry(&target, "en", DialogueSlot::Idle, Some("external".into()))
            .unwrap();
        model.reconcile(true);
        assert!(model.conflicted(&key));
        assert_eq!(model.value(&key), " local  ");
        assert_eq!(model.mutation_baseline(), Some(baseline));
        assert!(model.rebase_current(" local  ".into()));
        assert!(!model.conflicted(&key));
        assert!(model.dirty(&key));
        assert_eq!(model.value(&key), " local  ");
        assert_eq!(
            model.mutation_baseline().unwrap().override_entry.as_deref(),
            Some("external")
        );
    }

    #[test]
    fn reference_generation_and_reload_are_bound_to_selected_entry() {
        let mut model = Model {
            ready: true,
            ..selectable_model()
        };
        let key = model.key().unwrap();
        model.reconcile(false);
        model.capture("my text".into());
        let original = model.mutation_baseline().unwrap();
        let reference = CharacterRef {
            id: "default".into(),
            revision: 2,
        };
        model.selected.as_mut().unwrap().reference = Some(reference);
        model.selected.as_mut().unwrap().generation = 3;
        model.choices[0] = model.selected.as_ref().unwrap().clone();
        model.reconcile(true);
        assert!(model.conflicted(&key));
        assert_eq!(model.mutation_baseline(), Some(original));
        assert_eq!(model.value(&key), "my text");
        let latest = model.snapshot(&key).unwrap();
        assert_eq!(model.reload_current(), Some(latest.saved_text().to_owned()));
        assert!(!model.conflicted(&key));
        assert!(!model.dirty(&key));
        assert_eq!(model.mutation_baseline(), Some(latest));
    }

    #[test]
    fn entry_save_and_character_reset_only_clear_matching_drafts() {
        let mut model = Model {
            ready: true,
            ..selectable_model()
        };
        let first = model.key().unwrap();
        model.reconcile(false);
        model.capture("idle local".into());
        model.slot = Some(DialogueSlot::Waiting);
        let waiting = model.key().unwrap();
        model.reconcile(false);
        model.capture("waiting local".into());
        model.locale = Some(UiLocale::Ko);
        let korean = model.key().unwrap();
        model.reconcile(false);
        model.capture("korean local".into());
        let other = DialogueChoice {
            target: DialogueTarget::Character("other".into()),
            name: "Other".into(),
            reference: None,
            generation: 1,
        };
        model.choices.push(other.clone());
        model.selected = Some(other);
        let unrelated = model.key().unwrap();
        model.reconcile(false);
        model.capture("other local".into());
        model.mark_saved(first.clone());
        assert!(!model.dirty(&first));
        assert!(model.dirty(&waiting));
        assert!(model.dirty(&korean));
        model.clear_target_drafts(&first.0);
        assert!(!model.dirty(&waiting));
        assert!(!model.dirty(&korean));
        assert!(model.dirty(&unrelated));
    }

    #[test]
    fn authored_reference_change_conflicts_even_without_an_override() {
        use crate::assets::DialogueLocale;

        let mut model = Model {
            ready: true,
            ..selectable_model()
        };
        let key = model.key().unwrap();
        let authored = |value: &str| CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "en".to_owned(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([("idle".to_owned(), value.to_owned())])),
                    reactions: None,
                },
            )])),
        };
        let overrides = DialogueOverrides::default();
        model.update_saved_content(&overrides, Some(&authored("pack one")), true, None, false);
        let baseline = model.mutation_baseline().unwrap();
        assert_eq!(baseline.override_entry, None);
        assert_eq!(baseline.authored_reference.as_deref(), Some("pack one"));
        model.capture("local draft".into());
        model.update_saved_content(&overrides, Some(&authored("pack two")), true, None, true);
        assert!(model.conflicted(&key));
        assert_eq!(model.value(&key), "local draft");
        assert_eq!(model.mutation_baseline(), Some(baseline));
    }

    #[test]
    fn renamed_choice_does_not_conflict_with_same_identity_and_entry() {
        let mut model = Model {
            ready: true,
            ..selectable_model()
        };
        let key = model.key().unwrap();
        model.reconcile(false);
        model.capture("local".into());
        model.selected.as_mut().unwrap().name = "Display name changed".into();
        model.choices[0] = model.selected.as_ref().unwrap().clone();
        model.reconcile(true);
        assert!(!model.conflicted(&key));
    }

    #[test]
    fn metadata_token_tracks_other_authored_source_changes_without_rebasing_draft() {
        use crate::assets::DialogueLocale;

        let mut model = selectable_model();
        let key = model.key().unwrap();
        let overrides = DialogueOverrides::default();
        let metadata = |waiting: &str| CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "en".to_owned(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([
                        ("idle".to_owned(), "idle unchanged".to_owned()),
                        ("waiting".to_owned(), waiting.to_owned()),
                    ])),
                    reactions: None,
                },
            )])),
        };
        model.update_saved_content(&overrides, Some(&metadata("old")), true, None, false);
        let baseline = model.mutation_baseline().unwrap();
        model.capture("local draft".into());
        let changed = metadata("new");
        model.update_saved_content(&overrides, Some(&changed), true, None, true);
        assert_eq!(model.value(&key), "local draft");
        assert!(model.conflicted(&key));
        assert_eq!(model.mutation_baseline(), Some(baseline.clone()));
        assert_eq!(
            model.authored_reference(UiLocale::En, DialogueSlot::Idle),
            Some("idle unchanged")
        );
        assert_ne!(
            model.latest_metadata_token.as_ref().unwrap().1,
            baseline.metadata_token
        );
        assert!(model.rebase_current("local draft".into()));
        assert!(!model.conflicted(&key));
        assert!(model.dirty(&key));
    }

    fn revision(revision: u64, generation: u64) -> DialogueChoice {
        DialogueChoice {
            target: DialogueTarget::Character("fox".into()),
            name: format!("Fox #{revision}"),
            reference: Some(CharacterRef {
                id: "fox".into(),
                revision,
            }),
            generation,
        }
    }

    #[test]
    fn listing_refresh_keeps_exact_revision_and_draft_until_explicit_selection() {
        use crate::assets::DialogueLocale;

        let first = revision(1, 8);
        let second = revision(2, 8);
        let mut model = Model {
            choices: vec![first.clone(), second.clone()],
            selected: Some(second.clone()),
            locale: Some(UiLocale::En),
            slot: Some(DialogueSlot::Idle),
            ready: true,
            ..Model::default()
        };
        let key = model.key().unwrap();
        model.reconcile(false);
        model.capture("working on #2".into());
        let original = model.mutation_baseline().unwrap();
        let updated_first = revision(1, 9);
        let updated_second = revision(2, 9);
        model.reconcile_choices(
            &[updated_first.clone(), updated_second.clone()],
            None,
            None,
            UiLocale::En,
        );
        assert_eq!(model.selected, Some(updated_second.clone()));
        assert!(!model.ready);
        assert_eq!(model.value(&key), "working on #2");
        assert_eq!(model.mutation_baseline(), None);
        model.update_saved_content(&DialogueOverrides::default(), None, true, None, true);
        assert_eq!(model.mutation_baseline().as_ref(), Some(&original));
        assert!(model.conflicted(&key));

        model.reconcile_choices(&[updated_first.clone()], None, None, UiLocale::En);
        assert_eq!(model.selected, Some(updated_second));
        assert!(!model.valid());
        assert!(!model.ready);
        assert_eq!(model.mutation_baseline(), None);
        assert_eq!(model.value(&key), "working on #2");
        assert_eq!(model.baselines.get(&key), Some(&original));

        // Explicit popup selection starts a new metadata load; simply changing
        // the selected choice cannot make the previous source ready.
        model.selected = Some(updated_first.clone());
        model.metadata = None;
        model.ready = false;
        model.update_saved_content(&DialogueOverrides::default(), None, false, None, true);
        assert!(model.valid());
        assert_eq!(model.mutation_baseline(), None);
        assert_eq!(model.value(&key), "working on #2");
        assert_eq!(model.baselines.get(&key), Some(&original));

        let authored = CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "en".into(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([("idle".into(), "authored #1".into())])),
                    reactions: None,
                },
            )])),
        };
        model.update_saved_content(
            &DialogueOverrides::default(),
            Some(&authored),
            true,
            None,
            true,
        );
        assert!(model.conflicted(&key));
        assert_eq!(model.value(&key), "working on #2");
        assert_eq!(model.mutation_baseline().as_ref(), Some(&original));
        assert!(model.rebase_current("working on #2".into()));
        assert!(!model.conflicted(&key));
        assert_eq!(model.value(&key), "working on #2");
        let rebased = model.mutation_baseline().unwrap();
        assert_eq!(rebased.choice.reference, updated_first.reference);
        assert_eq!(rebased.choice.generation, 9);
        assert_eq!(rebased.authored_reference.as_deref(), Some("authored #1"));
        assert_eq!(model.reload_current().as_deref(), Some("authored #1"));
        assert!(!model.dirty(&key));
    }

    #[test]
    fn initial_source_must_match_exact_reference_but_external_has_no_reference() {
        let first = revision(1, 9);
        let missing = revision(2, 9);
        let external = DialogueChoice {
            target: DialogueTarget::ExternalAssets("/assets/fox".into()),
            name: "External fox".into(),
            reference: None,
            generation: 9,
        };
        let choices = [first.clone(), external.clone()];
        let mut model = Model::default();
        model.reconcile_choices(
            &choices,
            Some(&missing.target),
            missing.reference.as_ref(),
            UiLocale::En,
        );
        assert_eq!(
            model.selected.as_ref().unwrap().reference,
            missing.reference
        );
        assert!(!model.valid());
        assert_eq!(model.selected.as_ref().unwrap().generation, 9);
        assert!(model.selected.as_ref().unwrap().name.contains("fox #2"));

        let mut model = Model::default();
        model.reconcile_choices(
            &choices,
            Some(&external.target),
            first.reference.as_ref(),
            UiLocale::En,
        );
        assert_eq!(model.selected, Some(external));
        assert!(model.valid());
    }

    #[test]
    fn first_ready_focused_display_installs_saved_values_but_not_later_external_changes() {
        use crate::assets::DialogueLocale;
        let authored = CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "en".into(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([("idle".into(), "authored".into())])),
                    reactions: None,
                },
            )])),
        };
        for (override_text, metadata, expected) in [
            (Some("override"), Some(&authored), "override"),
            (None, Some(&authored), "authored"),
            (None, None, ""),
        ] {
            let mut model = selectable_model();
            model.switch_display_context();
            let mut overrides = DialogueOverrides::default();
            if let Some(value) = override_text {
                overrides
                    .set_entry(
                        &model.key().unwrap().0,
                        "en",
                        DialogueSlot::Idle,
                        Some(value.into()),
                    )
                    .unwrap();
            }
            model.update_saved_content(&overrides, metadata, true, None, false);
            let first = model.hydration_action(false, true, false, "");
            assert_eq!(first.result, HydrationSync::Settled);
            assert_eq!(
                first.replacement.as_ref().map(|(text, _)| text.as_str()),
                (expected != "").then_some(expected)
            );
            assert!(model.display.ready_value_installed);
            assert!(!model.needs_initial_hydration());
            let unchanged = model.hydration_action(false, true, false, expected);
            assert!(unchanged.replacement.is_none());

            overrides
                .set_entry(
                    &model.key().unwrap().0,
                    "en",
                    DialogueSlot::Idle,
                    Some("new external value".into()),
                )
                .unwrap();
            model.update_saved_content(&overrides, metadata, true, None, true);
            let focused = model.hydration_action(false, true, false, expected);
            assert!(focused.replacement.is_none());
            assert!(model.conflicted(&model.key().unwrap()));
        }

        let mut default_model = selectable_model();
        default_model.slot = Some(DialogueSlot::HeadTap);
        default_model.switch_display_context();
        default_model.update_saved_content(&DialogueOverrides::default(), None, true, None, false);
        let default_display = default_model.hydration_action(false, true, false, "");
        assert_eq!(
            default_display.replacement,
            Some((
                default_dialogue(UiLocale::En, DefaultDialogue::HeadTap).into(),
                false
            ))
        );
    }

    #[test]
    fn pre_metadata_raw_edit_survives_ready_source_even_when_provisional_matches() {
        let mut model = selectable_model();
        model.switch_display_context();
        let key = model.key().unwrap();
        // Whitespace normalizes to the provisional empty idle value, but is an
        // actual text-view edit and must remain the raw draft after metadata.
        model.record_edit("  ".into());
        assert_eq!(model.value(&key), "  ");
        assert!(model.pre_metadata_edits.contains(&key));
        let mut overrides = DialogueOverrides::default();
        overrides
            .set_entry(&key.0, "en", DialogueSlot::Idle, Some("saved".into()))
            .unwrap();
        model.update_saved_content(&overrides, None, true, None, true);
        let first = model.hydration_action(false, true, false, "  ");
        assert_eq!(first.result, HydrationSync::Settled);
        assert!(first.replacement.is_none());
        assert_eq!(model.value(&key), "  ");
        assert!(model.dirty(&key));
        assert_eq!(
            model.mutation_baseline().unwrap().override_entry.as_deref(),
            Some("saved")
        );
        model.record_edit("saved".into());
        assert_eq!(model.value(&key), "saved");
        assert!(model.pre_metadata_edits.contains(&key));
        assert_eq!(model.reload_current().as_deref(), Some("saved"));
        assert!(!model.pre_metadata_edits.contains(&key));
    }

    #[test]
    fn only_marked_initial_ready_hydration_needs_a_quiet_retry() {
        let mut model = selectable_model();
        model.switch_display_context();
        let key = model.key().unwrap();
        let mut overrides = DialogueOverrides::default();
        overrides
            .set_entry(&key.0, "en", DialogueSlot::Idle, Some("saved".into()))
            .unwrap();
        model.update_saved_content(&overrides, None, false, None, true);
        assert_eq!(
            model.hydration_action(true, true, false, "").result,
            HydrationSync::Settled
        );
        assert!(!model.needs_initial_hydration());
        model.update_saved_content(&overrides, None, true, None, true);
        assert_eq!(
            model.hydration_action(true, true, false, "").result,
            HydrationSync::DeferredMarked
        );
        assert!(model.needs_initial_hydration());
        let settled = model.hydration_action(false, true, false, "");
        assert_eq!(settled.result, HydrationSync::Settled);
        assert_eq!(settled.replacement, Some(("saved".into(), false)));
        assert!(!model.needs_initial_hydration());
        model.switch_display_context();
        assert!(!model.needs_initial_hydration());

        model.record_edit("composition".into());
        let edited = model.hydration_action(true, true, false, "composition");
        assert_eq!(edited.result, HydrationSync::Settled);
        assert!(!model.needs_initial_hydration());
        assert_eq!(model.value(&key), "composition");
    }

    #[test]
    fn committed_composition_settles_initial_hydration_without_replacing_draft() {
        let mut model = selectable_model();
        model.switch_display_context();
        let key = model.key().unwrap();
        let mut overrides = DialogueOverrides::default();
        overrides
            .set_entry(&key.0, "en", DialogueSlot::Idle, Some("saved".into()))
            .unwrap();
        model.update_saved_content(&overrides, None, true, None, true);
        assert_eq!(
            model
                .hydration_action(true, true, false, "composition")
                .result,
            HydrationSync::DeferredMarked
        );

        // Native unmark delivers didChangeText before the quiet timer settles.
        model.record_edit("composition".into());
        assert!(model.needs_initial_hydration());
        let settled = model.hydration_action(false, true, false, "composition");
        assert_eq!(settled.result, HydrationSync::Settled);
        assert!(settled.replacement.is_none());
        assert_eq!(model.value(&key), "composition");
        assert!(!model.needs_initial_hydration());
    }

    #[test]
    fn context_switch_keeps_other_draft_but_initializes_untouched_slot() {
        let mut model = selectable_model();
        model.switch_display_context();
        model.record_edit("idle draft".into());
        let idle = model.key().unwrap();
        model.slot = Some(DialogueSlot::Waiting);
        model.switch_display_context();
        let waiting = model.key().unwrap();
        let mut overrides = DialogueOverrides::default();
        overrides
            .set_entry(
                &waiting.0,
                "en",
                DialogueSlot::Waiting,
                Some("waiting saved".into()),
            )
            .unwrap();
        model.update_saved_content(&overrides, None, true, None, false);
        let next = model.hydration_action(false, true, false, "");
        assert_eq!(next.replacement, Some(("waiting saved".into(), false)));
        assert_eq!(model.value(&idle), "idle draft");
        model.slot = Some(DialogueSlot::Idle);
        model.switch_display_context();
        let returning = model.hydration_action(false, true, false, "idle draft");
        assert!(returning.replacement.is_none());
        assert_eq!(model.value(&idle), "idle draft");
    }
}
