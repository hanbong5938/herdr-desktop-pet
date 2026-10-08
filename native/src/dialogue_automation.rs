use crate::assets::CharacterMetadata;
use crate::character_types::CharacterRef;
use crate::dialogue::{effective_metadata, DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::dialogue_editor::DialogueChoice;
use crate::i18n::{default_dialogue, DefaultDialogue, UiLocale};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueIdentity {
    pub target: DialogueTarget,
    pub reference: Option<CharacterRef>,
    pub generation: u64,
}

impl From<&DialogueChoice> for DialogueIdentity {
    fn from(choice: &DialogueChoice) -> Self {
        Self {
            target: choice.target.clone(),
            reference: choice.reference.clone(),
            generation: choice.generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DialogueLanguage {
    Ko,
    En,
}

impl DialogueLanguage {
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::Ko => "ko",
            Self::En => "en",
        }
    }

    pub(crate) const fn ui_locale(self) -> UiLocale {
        match self {
            Self::Ko => UiLocale::Ko,
            Self::En => UiLocale::En,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueSelection {
    pub identity: DialogueIdentity,
    pub locale: DialogueLanguage,
    pub slot: DialogueSlot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueBaseline {
    pub metadata_token: String,
    pub override_entry: Option<String>,
    pub target_overrides_token: String,
}

fn text(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}

fn finish(hash: Sha256) -> String {
    format!("{:x}", hash.finalize())
}

/// Bind a baseline to the exact pack revision, registry generation and every
/// authored dialogue fact, including locales not currently selected by the UI.
pub(crate) fn metadata_token(
    identity: &DialogueIdentity,
    metadata: Option<&CharacterMetadata>,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"herdr.dialogue.metadata.v1\0");
    match &identity.target {
        DialogueTarget::Character(id) => {
            hash.update([0]);
            text(&mut hash, id);
        }
        DialogueTarget::ExternalAssets(id) => {
            hash.update([1]);
            text(&mut hash, id);
        }
    }
    match identity.reference.as_ref() {
        Some(reference) => {
            hash.update([1]);
            text(&mut hash, &reference.id);
            hash.update(reference.revision.to_le_bytes());
        }
        None => hash.update([0]),
    }
    hash.update(identity.generation.to_le_bytes());
    match metadata {
        Some(metadata) => {
            hash.update([1]);
            match metadata.dialogue.as_ref() {
                Some(locales) => {
                    hash.update([1]);
                    hash.update((locales.len() as u64).to_le_bytes());
                    for (locale, entry) in locales {
                        text(&mut hash, locale);
                        for values in [&entry.phases, &entry.reactions] {
                            match values {
                                Some(values) => {
                                    hash.update([1]);
                                    hash.update((values.len() as u64).to_le_bytes());
                                    for (key, value) in values {
                                        text(&mut hash, key);
                                        text(&mut hash, value);
                                    }
                                }
                                None => hash.update([0]),
                            }
                        }
                    }
                }
                None => hash.update([0]),
            }
        }
        None => hash.update([0]),
    }
    finish(hash)
}

/// Hash only this target's overrides. A change to another character cannot
/// invalidate a reset-character baseline.
pub(crate) fn target_overrides_token(
    overrides: &DialogueOverrides,
    target: &DialogueTarget,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"herdr.dialogue.overrides.v1\0");
    match target {
        DialogueTarget::Character(id) => {
            hash.update([0]);
            text(&mut hash, id);
        }
        DialogueTarget::ExternalAssets(id) => {
            hash.update([1]);
            text(&mut hash, id);
        }
    }
    match overrides.locales(target) {
        Some(locales) => {
            hash.update([1]);
            hash.update((locales.len() as u64).to_le_bytes());
            for (locale, entry) in locales {
                text(&mut hash, locale);
                for values in [&entry.phases, &entry.reactions] {
                    hash.update((values.len() as u64).to_le_bytes());
                    for (key, value) in values {
                        text(&mut hash, key);
                        text(&mut hash, value);
                    }
                }
            }
        }
        None => hash.update([0]),
    }
    finish(hash)
}

/// The pack's locale/reaction precedence is the same resolution used by the
/// editor. Host-owned reaction fallback is effective, never authored text.
pub(crate) fn read_entry(
    selection: &DialogueSelection,
    metadata: Option<&CharacterMetadata>,
    overrides: &DialogueOverrides,
) -> Value {
    let slot = selection.slot;
    let locale = selection.locale;
    let authored = metadata.and_then(|metadata| {
        if slot.is_reaction() {
            metadata.dialogue_text("", Some(slot.key()), locale.tag())
        } else {
            metadata.dialogue_text(slot.key(), None, locale.tag())
        }
    });
    let override_entry = overrides.entry(&selection.identity.target, locale.tag(), slot);
    let fallback = match slot {
        DialogueSlot::HeadTap => Some(DefaultDialogue::HeadTap),
        DialogueSlot::BodyTap => Some(DefaultDialogue::BodyTap),
        DialogueSlot::Pet => Some(DefaultDialogue::Pet),
        DialogueSlot::Completion => Some(DefaultDialogue::Completion),
        _ => None,
    };
    let merged = effective_metadata(metadata, overrides.locales(&selection.identity.target));
    let effective = if slot.is_reaction() {
        merged.dialogue_text("", Some(slot.key()), locale.tag())
    } else {
        merged.dialogue_text(slot.key(), None, locale.tag())
    }
    .or_else(|| fallback.map(|fallback| default_dialogue(locale.ui_locale(), fallback)));
    let baseline = DialogueBaseline {
        metadata_token: metadata_token(&selection.identity, metadata),
        override_entry: override_entry.map(str::to_owned),
        target_overrides_token: target_overrides_token(overrides, &selection.identity.target),
    };
    json!({
        "selection": selection,
        "authored": authored,
        "override": override_entry,
        "effective": effective,
        "baseline": baseline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::DialogueLocale;
    use std::collections::BTreeMap;

    fn selection(slot: DialogueSlot) -> DialogueSelection {
        DialogueSelection {
            identity: DialogueIdentity {
                target: DialogueTarget::Character("default".to_owned()),
                reference: Some(CharacterRef::builtin()),
                generation: 12,
            },
            locale: DialogueLanguage::Ko,
            slot,
        }
    }

    #[test]
    fn read_entry_distinguishes_authored_fallback_from_host_reaction() {
        let mut en = DialogueLocale {
            phases: None,
            reactions: None,
        };
        en.reactions = Some(BTreeMap::from([(
            "head_tap".to_owned(),
            "pack en".to_owned(),
        )]));
        let metadata = CharacterMetadata {
            dialogue: Some(BTreeMap::from([("en".to_owned(), en)])),
        };
        let mut overrides = DialogueOverrides::default();
        let picked = selection(DialogueSlot::HeadTap);
        let read = read_entry(&picked, Some(&metadata), &overrides);
        assert_eq!(read["authored"], "pack en");
        assert_eq!(read["effective"], "pack en");
        overrides
            .set_entry(
                &picked.identity.target,
                "ko",
                picked.slot,
                Some("  custom\n".to_owned()),
            )
            .unwrap();
        let changed = read_entry(&picked, Some(&metadata), &overrides);
        assert_eq!(changed["effective"], "  custom\n");
        assert_ne!(
            read["baseline"]["target_overrides_token"],
            changed["baseline"]["target_overrides_token"]
        );
        assert_eq!(
            read["baseline"]["metadata_token"],
            changed["baseline"]["metadata_token"]
        );
        let empty = read_entry(
            &selection(DialogueSlot::BodyTap),
            None,
            &DialogueOverrides::default(),
        );
        assert!(empty["authored"].is_null());
        assert!(empty["effective"].is_string());
    }

    #[test]
    fn fingerprints_bind_complete_identity_and_target_without_unrelated_overrides() {
        let mut selected = selection(DialogueSlot::Idle);
        let absent = metadata_token(&selected.identity, None);
        let empty = CharacterMetadata { dialogue: None };
        assert_ne!(absent, metadata_token(&selected.identity, Some(&empty)));
        selected.identity.generation += 1;
        assert_ne!(absent, metadata_token(&selected.identity, None));
        let mut overrides = DialogueOverrides::default();
        let original = target_overrides_token(&overrides, &selected.identity.target);
        overrides
            .set_entry(
                &DialogueTarget::Character("other".to_owned()),
                "en",
                DialogueSlot::Idle,
                Some("other".to_owned()),
            )
            .unwrap();
        assert_eq!(
            original,
            target_overrides_token(&overrides, &selected.identity.target)
        );
        overrides
            .set_entry(
                &selected.identity.target,
                "ko",
                DialogueSlot::Idle,
                Some("selected".to_owned()),
            )
            .unwrap();
        assert_ne!(
            original,
            target_overrides_token(&overrides, &selected.identity.target)
        );
    }

    #[test]
    fn stale_entry_baseline_conflicts_but_unrelated_target_does_not() {
        let selected = selection(DialogueSlot::Idle);
        let mut overrides = DialogueOverrides::default();
        let before = read_entry(&selected, None, &overrides);
        overrides
            .set_entry(
                &DialogueTarget::Character("other".to_owned()),
                "ko",
                DialogueSlot::Idle,
                Some("unrelated".to_owned()),
            )
            .unwrap();
        let unrelated = read_entry(&selected, None, &overrides);
        assert_eq!(before["baseline"], unrelated["baseline"]);
        overrides
            .set_entry(
                &selected.identity.target,
                "ko",
                DialogueSlot::Idle,
                Some("new saved value".to_owned()),
            )
            .unwrap();
        let changed = read_entry(&selected, None, &overrides);
        assert_ne!(
            before["baseline"]["override_entry"],
            changed["baseline"]["override_entry"]
        );
        assert_ne!(
            before["baseline"]["target_overrides_token"],
            changed["baseline"]["target_overrides_token"]
        );

        let mut metadata = CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "en".to_owned(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([("idle".to_owned(), "pack en".to_owned())])),
                    reactions: None,
                },
            )])),
        };
        let pack_before = metadata_token(&selected.identity, Some(&metadata));
        metadata
            .dialogue
            .as_mut()
            .unwrap()
            .get_mut("en")
            .unwrap()
            .phases
            .as_mut()
            .unwrap()
            .insert("idle".to_owned(), "new pack en".to_owned());
        assert_ne!(
            pack_before,
            metadata_token(&selected.identity, Some(&metadata))
        );
    }

    #[test]
    fn older_authored_revision_uses_its_own_baseline_but_updates_active_revision_bubble() {
        use crate::dialogue::effective_metadata;

        let authored = CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "ko".to_owned(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([(
                        "idle".to_owned(),
                        "older authored".to_owned(),
                    )])),
                    reactions: None,
                },
            )])),
        };
        let active = CharacterMetadata {
            dialogue: Some(BTreeMap::from([(
                "ko".to_owned(),
                DialogueLocale {
                    phases: Some(BTreeMap::from([(
                        "idle".to_owned(),
                        "newer authored".to_owned(),
                    )])),
                    reactions: None,
                },
            )])),
        };
        let mut selected = selection(DialogueSlot::Idle);
        selected.identity.reference = Some(CharacterRef {
            id: "shared".to_owned(),
            revision: 1,
        });
        selected.identity.target = DialogueTarget::Character("shared".to_owned());
        let mut overrides = DialogueOverrides::default();
        let prior = read_entry(&selected, Some(&authored), &overrides);
        assert_eq!(prior["authored"], "older authored");
        assert_eq!(
            effective_metadata(Some(&active), overrides.locales(&selected.identity.target))
                .dialogue_text("idle", None, "ko"),
            Some("newer authored")
        );

        overrides
            .set_entry(
                &selected.identity.target,
                "ko",
                DialogueSlot::Idle,
                Some("shared edit".to_owned()),
            )
            .unwrap();
        let saved = read_entry(&selected, Some(&authored), &overrides);
        assert_eq!(saved["authored"], "older authored");
        assert_eq!(saved["effective"], "shared edit");
        assert_eq!(
            prior["baseline"]["metadata_token"],
            saved["baseline"]["metadata_token"]
        );
        assert_ne!(
            prior["baseline"]["target_overrides_token"],
            saved["baseline"]["target_overrides_token"]
        );
        assert_eq!(
            effective_metadata(Some(&active), overrides.locales(&selected.identity.target))
                .dialogue_text("idle", None, "ko"),
            Some("shared edit")
        );
        overrides
            .set_entry(&selected.identity.target, "ko", DialogueSlot::Idle, None)
            .unwrap();
        assert_eq!(
            effective_metadata(Some(&active), overrides.locales(&selected.identity.target))
                .dialogue_text("idle", None, "ko"),
            Some("newer authored")
        );
    }

    #[test]
    fn selection_and_target_reject_unknown_fields_and_keep_tuple_constructors() {
        let selected = selection(DialogueSlot::Completion);
        let encoded = serde_json::to_value(&selected).unwrap();
        assert_eq!(
            encoded["identity"]["target"],
            json!({"kind":"character","id":"default"})
        );
        assert_eq!(encoded["slot"], "completion_observed");
        assert_eq!(
            serde_json::from_value::<DialogueSelection>(encoded.clone()).unwrap(),
            selected
        );
        let mut invalid = encoded;
        invalid["extra"] = json!(true);
        assert!(serde_json::from_value::<DialogueSelection>(invalid).is_err());
        assert!(serde_json::from_value::<DialogueTarget>(
            json!({"kind":"character","id":"default","extra":true})
        )
        .is_err());
    }
}
