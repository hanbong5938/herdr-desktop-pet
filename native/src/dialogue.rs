use crate::assets::{CharacterMetadata, DialogueLocale};
use crate::character_types::validate_pack_id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const MAX_TEXT_BYTES: usize = 2048;
const MAX_PATH_BYTES: usize = 4096;
const MAX_TARGETS: usize = 128;
const MAX_ENTRIES: usize = 512;
const MAX_AGGREGATE_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum DialogueTarget {
    Character(String),
    ExternalAssets(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum DialogueSlot {
    Idle,
    Running,
    Waiting,
    Unknown,
    HeadTap,
    BodyTap,
    Pet,
    Completion,
}

impl DialogueSlot {
    pub(crate) const ALL: [Self; 8] = [
        Self::Idle,
        Self::Running,
        Self::Waiting,
        Self::Unknown,
        Self::HeadTap,
        Self::BodyTap,
        Self::Pet,
        Self::Completion,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Unknown => "unknown",
            Self::HeadTap => "head_tap",
            Self::BodyTap => "body_tap",
            Self::Pet => "pet",
            Self::Completion => "completion_observed",
        }
    }

    pub(crate) fn is_reaction(self) -> bool {
        matches!(
            self,
            Self::HeadTap | Self::BodyTap | Self::Pet | Self::Completion
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueLocaleOverrides {
    #[serde(default)]
    pub(crate) phases: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) reactions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueOverrides {
    #[serde(default)]
    pub(crate) characters: BTreeMap<String, BTreeMap<String, DialogueLocaleOverrides>>,
    #[serde(default)]
    pub(crate) external_assets: BTreeMap<String, BTreeMap<String, DialogueLocaleOverrides>>,
}

impl DialogueOverrides {
    pub(crate) fn locales(
        &self,
        target: &DialogueTarget,
    ) -> Option<&BTreeMap<String, DialogueLocaleOverrides>> {
        match target {
            DialogueTarget::Character(id) => self.characters.get(id),
            DialogueTarget::ExternalAssets(path) => self.external_assets.get(path),
        }
    }

    pub(crate) fn entry(
        &self,
        target: &DialogueTarget,
        locale: &str,
        slot: DialogueSlot,
    ) -> Option<&str> {
        let entry = self.locales(target)?.get(locale)?;
        let entries = if slot.is_reaction() {
            &entry.reactions
        } else {
            &entry.phases
        };
        entries.get(slot.key()).map(String::as_str)
    }

    fn locales_mut(
        &mut self,
        target: &DialogueTarget,
    ) -> &mut BTreeMap<String, BTreeMap<String, DialogueLocaleOverrides>> {
        match target {
            DialogueTarget::Character(_) => &mut self.characters,
            DialogueTarget::ExternalAssets(_) => &mut self.external_assets,
        }
    }

    pub(crate) fn prune_empty(&mut self) {
        for records in [&mut self.characters, &mut self.external_assets] {
            records.retain(|_, locales| {
                locales
                    .retain(|_, values| !values.phases.is_empty() || !values.reactions.is_empty());
                !locales.is_empty()
            });
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut targets = 0usize;
        let mut entries = 0usize;
        let mut bytes = 0usize;
        for (namespace, records) in [(&self.characters, false), (&self.external_assets, true)] {
            for (id, locales) in namespace {
                if records {
                    validate_external_path(id)?;
                } else {
                    validate_character_id(id)?;
                }
                targets += 1;
                bytes += id.len();
                for (locale, values) in locales {
                    validate_locale(locale)?;
                    bytes += locale.len();
                    for (keys, reaction) in [(&values.phases, false), (&values.reactions, true)] {
                        for (key, value) in keys {
                            if !DialogueSlot::ALL
                                .iter()
                                .any(|slot| slot.is_reaction() == reaction && slot.key() == key)
                            {
                                return Err(format!("unknown dialogue slot: {key}"));
                            }
                            validate_text(value)?;
                            if value.trim().is_empty() {
                                return Err("dialogue override cannot be blank".to_string());
                            }
                            entries += 1;
                            bytes += key.len() + value.len();
                        }
                    }
                }
            }
        }
        if targets > MAX_TARGETS || entries > MAX_ENTRIES || bytes > MAX_AGGREGATE_BYTES {
            return Err("dialogue overrides exceed storage limits".to_string());
        }
        Ok(())
    }

    pub(crate) fn set_entry(
        &mut self,
        target: &DialogueTarget,
        locale: &str,
        slot: DialogueSlot,
        value: Option<String>,
    ) -> Result<(), String> {
        validate_target(target)?;
        validate_locale(locale)?;
        let value = match value {
            Some(value) if !value.trim().is_empty() => {
                validate_text(&value)?;
                Some(value)
            }
            _ => None,
        };
        let id = match target {
            DialogueTarget::Character(id) | DialogueTarget::ExternalAssets(id) => id,
        };
        let records = self.locales_mut(target);
        if let Some(value) = value {
            let entries = records.entry(id.clone()).or_default();
            let overrides = entries.entry(locale.to_string()).or_default();
            if slot.is_reaction() {
                overrides.reactions.insert(slot.key().to_string(), value);
            } else {
                overrides.phases.insert(slot.key().to_string(), value);
            }
        } else if let Some(entries) = records.get_mut(id) {
            if let Some(overrides) = entries.get_mut(locale) {
                if slot.is_reaction() {
                    overrides.reactions.remove(slot.key());
                } else {
                    overrides.phases.remove(slot.key());
                }
                if overrides.phases.is_empty() && overrides.reactions.is_empty() {
                    entries.remove(locale);
                }
            }
            if entries.is_empty() {
                records.remove(id);
            }
        }
        self.validate()
    }

    pub(crate) fn remove_target(&mut self, target: &DialogueTarget) -> Result<(), String> {
        validate_target(target)?;
        let id = match target {
            DialogueTarget::Character(id) | DialogueTarget::ExternalAssets(id) => id,
        };
        self.locales_mut(target).remove(id);
        Ok(())
    }
}

pub(crate) fn validate_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT_BYTES
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(
            "dialogue text must be at most 2048 UTF-8 bytes and contain no control characters"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_locale(locale: &str) -> Result<(), String> {
    if matches!(locale, "ko" | "en") {
        Ok(())
    } else {
        Err("dialogue language must be ko or en".to_string())
    }
}

fn validate_character_id(id: &str) -> Result<(), String> {
    if id == "default" {
        Ok(())
    } else {
        validate_pack_id(id)
    }
}

fn validate_external_path(path: &str) -> Result<(), String> {
    if path.len() > MAX_PATH_BYTES || path.is_empty() || path.chars().any(char::is_control) {
        return Err("external asset path is invalid".to_string());
    }
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.components().collect::<PathBuf>().as_path() != path
    {
        return Err("external asset path must be absolute and normalized".to_string());
    }
    Ok(())
}

fn validate_target(target: &DialogueTarget) -> Result<(), String> {
    match target {
        DialogueTarget::Character(id) => validate_character_id(id),
        DialogueTarget::ExternalAssets(path) => validate_external_path(path),
    }
}

/// Merge at activation/save time, retaining the pack's locale and reaction precedence.
pub(crate) fn effective_metadata(
    base: Option<&CharacterMetadata>,
    overrides: Option<&BTreeMap<String, DialogueLocaleOverrides>>,
) -> CharacterMetadata {
    let mut result = base
        .cloned()
        .unwrap_or(CharacterMetadata { dialogue: None });
    if let Some(overrides) = overrides {
        for (locale, values) in overrides {
            let entry = result
                .dialogue
                .get_or_insert_with(BTreeMap::new)
                .entry(locale.clone())
                .or_insert_with(|| DialogueLocale {
                    phases: None,
                    reactions: None,
                });
            if !values.phases.is_empty() {
                entry
                    .phases
                    .get_or_insert_with(BTreeMap::new)
                    .extend(values.phases.clone());
            }
            if !values.reactions.is_empty() {
                entry
                    .reactions
                    .get_or_insert_with(BTreeMap::new)
                    .extend(values.reactions.clone());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_reaction_beats_custom_phase_and_exact_phase_beats_fallback_reaction() {
        let base = CharacterMetadata {
            dialogue: Some(BTreeMap::from([
                (
                    "ko".to_string(),
                    DialogueLocale {
                        phases: Some(BTreeMap::from([(
                            "idle".to_string(),
                            "pack idle".to_string(),
                        )])),
                        reactions: Some(BTreeMap::from([(
                            "pet".to_string(),
                            "pack pet".to_string(),
                        )])),
                    },
                ),
                (
                    "en".to_string(),
                    DialogueLocale {
                        phases: None,
                        reactions: Some(BTreeMap::from([(
                            "head_tap".to_string(),
                            "English tap".to_string(),
                        )])),
                    },
                ),
            ])),
        };
        let overrides = BTreeMap::from([(
            "ko".to_string(),
            DialogueLocaleOverrides {
                phases: BTreeMap::from([("idle".to_string(), "custom idle".to_string())]),
                reactions: BTreeMap::new(),
            },
        )]);
        let effective = effective_metadata(Some(&base), Some(&overrides));
        assert_eq!(
            effective.dialogue_text("idle", Some("pet"), "ko"),
            Some("pack pet")
        );
        assert_eq!(
            effective.dialogue_text("idle", Some("head_tap"), "ko"),
            Some("custom idle")
        );
        assert_eq!(
            base.dialogue_text("idle", Some("pet"), "ko"),
            Some("pack pet")
        );
    }

    #[test]
    fn no_metadata_and_locale_fallback_resolve_user_dialogue() {
        let overrides = BTreeMap::from([(
            "ko".to_string(),
            DialogueLocaleOverrides {
                phases: BTreeMap::from([("waiting".to_string(), "기다려요".to_string())]),
                reactions: BTreeMap::new(),
            },
        )]);
        let effective = effective_metadata(None, Some(&overrides));
        assert_eq!(
            effective.dialogue_text("waiting", None, "en"),
            Some("기다려요")
        );
        assert_eq!(effective.dialogue_text("running", None, "en"), None);
        assert!(effective_metadata(None, None)
            .dialogue_text("waiting", None, "ko")
            .is_none());
    }
}
