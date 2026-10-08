use crate::bubble::{BubblePlacement, BubbleSize};
use crate::dialogue::{DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::i18n::LanguagePreference;
use crate::session_view::SessionSort;
use crate::sources::ObservationPreferences;
use crate::state::{normalize_scale, DEFAULT_SCALE};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PREFERENCES_FILE: &str = "preferences.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BubbleTheme {
    WarmIvory,
    DustyRose,
    MoonlitInk,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MenuBarMode {
    #[default]
    Always,
    RecoveryOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct MenuBarIconPreference {
    pub(crate) asset: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BubbleColor(u8, u8, u8);

impl BubbleColor {
    pub(crate) fn parse_hex(value: &str) -> Result<Self, String> {
        let bytes = value.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err("color must be #RRGGBB".to_owned());
        }
        let channel = |offset| {
            u8::from_str_radix(&value[offset..offset + 2], 16).expect("ASCII hex was checked above")
        };
        Ok(Self(channel(1), channel(3), channel(5)))
    }

    pub(crate) fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    pub(crate) fn rgb(self) -> (f64, f64, f64) {
        (
            f64::from(self.0) / 255.0,
            f64::from(self.1) / 255.0,
            f64::from(self.2) / 255.0,
        )
    }
}

impl Serialize for BubbleColor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for BubbleColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse_hex(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct BubblePalette {
    pub surface: BubbleColor,
    pub text: BubbleColor,
    pub muted: BubbleColor,
    pub border: BubbleColor,
    pub accent: BubbleColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct BubbleAppearance {
    pub theme: BubbleTheme,
    pub custom: BubblePalette,
}

impl Default for BubbleAppearance {
    fn default() -> Self {
        Self {
            theme: BubbleTheme::WarmIvory,
            custom: Self::warm_ivory(),
        }
    }
}

impl BubbleAppearance {
    const fn warm_ivory() -> BubblePalette {
        BubblePalette {
            surface: BubbleColor(0xF5, 0xEE, 0xE5),
            text: BubbleColor(0x47, 0x3B, 0x3C),
            muted: BubbleColor(0x8C, 0x79, 0x77),
            border: BubbleColor(0xDE, 0xD0, 0xC7),
            accent: BubbleColor(0xAA, 0x68, 0x68),
        }
    }

    pub(crate) fn palette(self) -> BubblePalette {
        match self.theme {
            BubbleTheme::WarmIvory => Self::warm_ivory(),
            BubbleTheme::DustyRose => BubblePalette {
                surface: BubbleColor(0xF0, 0xE1, 0xE6),
                text: BubbleColor(0x4F, 0x35, 0x47),
                muted: BubbleColor(0x93, 0x77, 0x88),
                border: BubbleColor(0xDB, 0xC4, 0xD0),
                accent: BubbleColor(0x92, 0x5E, 0x79),
            },
            BubbleTheme::MoonlitInk => BubblePalette {
                surface: BubbleColor(0x29, 0x29, 0x36),
                text: BubbleColor(0xF1, 0xEA, 0xF5),
                muted: BubbleColor(0xAA, 0xA1, 0xB6),
                border: BubbleColor(0x48, 0x43, 0x54),
                accent: BubbleColor(0xC1, 0xAE, 0xD7),
            },
            BubbleTheme::Custom => self.custom,
        }
    }
}

/// A partial settings mutation. Absent fields leave the saved value unchanged;
/// an empty machine list explicitly clears the selected catalog machines.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreferencePatch {
    #[serde(default, deserialize_with = "deserialize_patch_language")]
    pub(crate) language: Option<LanguagePreference>,
    #[serde(default, deserialize_with = "deserialize_patch_appearance")]
    pub(crate) bubble_appearance: Option<BubbleAppearance>,
    pub(crate) show_status_indicators: Option<bool>,
    pub(crate) menu_bar_mode: Option<MenuBarMode>,
    pub(crate) observation_local: Option<bool>,
    pub(crate) observation_remote: Option<bool>,
    pub(crate) observation_machines: Option<Vec<String>>,
}

fn deserialize_patch_language<'de, D>(
    deserializer: D,
) -> Result<Option<LanguagePreference>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let token = Option::<String>::deserialize(deserializer)?;
    token
        .map(|token| match token.as_str() {
            "system" => Ok(LanguagePreference::System),
            "ko" => Ok(LanguagePreference::Ko),
            "en" => Ok(LanguagePreference::En),
            _ => Err(serde::de::Error::unknown_variant(
                &token,
                &["system", "ko", "en"],
            )),
        })
        .transpose()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchPalette {
    surface: BubbleColor,
    text: BubbleColor,
    muted: BubbleColor,
    border: BubbleColor,
    accent: BubbleColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchAppearance {
    theme: BubbleTheme,
    custom: PatchPalette,
}

fn deserialize_patch_appearance<'de, D>(
    deserializer: D,
) -> Result<Option<BubbleAppearance>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<PatchAppearance>::deserialize(deserializer).map(|appearance| {
        appearance.map(|appearance| BubbleAppearance {
            theme: appearance.theme,
            custom: BubblePalette {
                surface: appearance.custom.surface,
                text: appearance.custom.text,
                muted: appearance.custom.muted,
                border: appearance.custom.border,
                accent: appearance.custom.accent,
            },
        })
    })
}

/// Settings reflected by the owner's last successful candidate commit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PreferenceSnapshot {
    pub(crate) language: LanguagePreference,
    pub(crate) bubble_appearance: BubbleAppearance,
    pub(crate) show_status_indicators: bool,
    pub(crate) menu_bar_mode: MenuBarMode,
    pub(crate) observation_local: bool,
    pub(crate) observation_remote: bool,
    pub(crate) observation_machines: Vec<String>,
}

/// Position coordinates persisted relative to the panel's display bounds.
/// A missing on-disk value represents the pre-crop full-canvas panel origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PositionSpace {
    LegacyCanvas,
    Display,
}

impl Default for PositionSpace {
    fn default() -> Self {
        Self::LegacyCanvas
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
pub(crate) struct BubbleSizes {
    #[serde(
        default,
        deserialize_with = "deserialize_tolerant_bubble_size",
        skip_serializing_if = "Option::is_none"
    )]
    pub compact: Option<BubbleSize>,
    #[serde(
        default,
        deserialize_with = "deserialize_tolerant_bubble_size",
        skip_serializing_if = "Option::is_none"
    )]
    pub expanded: Option<BubbleSize>,
}

impl BubbleSizes {
    fn sanitize(&mut self) {
        self.compact = self.compact.filter(|size| size.is_valid());
        self.expanded = self.expanded.filter(|size| size.is_valid());
    }
}

fn deserialize_tolerant_bubble_size<'de, D>(deserializer: D) -> Result<Option<BubbleSize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<TolerantBubbleSize>::deserialize(deserializer)?.and_then(|size| size.0))
}

struct TolerantBubbleSize(Option<BubbleSize>);

#[derive(Deserialize)]
#[serde(field_identifier)]
enum BubbleSizeField {
    #[serde(rename = "width")]
    Width,
    #[serde(rename = "height")]
    Height,
    #[serde(other)]
    Other,
}

struct TolerantDimension(Option<f64>);

impl<'de> Deserialize<'de> for TolerantDimension {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(DimensionVisitor)
    }
}

struct DimensionVisitor;

impl<'de> Visitor<'de> for DimensionVisitor {
    type Value = TolerantDimension;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a positive finite number")
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        self.visit_f64(value as f64)
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        self.visit_f64(value as f64)
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Ok(TolerantDimension(
            (value.is_finite() && value > 0.0).then_some(value),
        ))
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(TolerantDimension(None))
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(TolerantDimension(None))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(TolerantDimension(None))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(TolerantDimension(None))
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TolerantDimension(None))
    }
}

impl<'de> Deserialize<'de> for TolerantBubbleSize {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(BubbleSizeVisitor)
    }
}

struct BubbleSizeVisitor;

impl<'de> Visitor<'de> for BubbleSizeVisitor {
    type Value = TolerantBubbleSize;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a bubble size object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let (mut width, mut height) = (None, None);
        let mut duplicate = false;
        while let Some(key) = map.next_key::<BubbleSizeField>()? {
            match key {
                BubbleSizeField::Width => {
                    let value = map.next_value::<TolerantDimension>()?.0;
                    duplicate |= width.is_some();
                    width = Some(value);
                }
                BubbleSizeField::Height => {
                    let value = map.next_value::<TolerantDimension>()?.0;
                    duplicate |= height.is_some();
                    height = Some(value);
                }
                BubbleSizeField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let size = match (width.flatten(), height.flatten()) {
            (Some(width), Some(height)) if !duplicate => Some(BubbleSize { width, height }),
            _ => None,
        };
        Ok(TolerantBubbleSize(size))
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(TolerantBubbleSize(None))
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TolerantBubbleSize(None))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SessionListPreferences {
    pub(crate) sort: SessionSort,
    pub(crate) running_first: bool,
}

impl Default for SessionListPreferences {
    fn default() -> Self {
        Self {
            sort: SessionSort::Stable,
            running_first: false,
        }
    }
}

impl SessionListPreferences {
    fn from_disk(value: Value) -> (Self, Map<String, Value>) {
        let Value::Object(mut fields) = value else {
            return (Self::default(), Map::new());
        };
        let sort = match fields.remove("sort").as_ref().and_then(Value::as_str) {
            Some("title_asc") => SessionSort::TitleAsc,
            Some("source_asc") => SessionSort::SourceAsc,
            _ => SessionSort::Stable,
        };
        let running_first = fields
            .remove("running_first")
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (
            Self {
                sort,
                running_first,
            },
            fields,
        )
    }

    fn to_disk(self, mut extra: Map<String, Value>) -> Value {
        let sort = match self.sort {
            SessionSort::Stable => "stable",
            SessionSort::TitleAsc => "title_asc",
            SessionSort::SourceAsc => "source_asc",
        };
        extra.insert("sort".to_owned(), Value::String(sort.to_owned()));
        extra.insert("running_first".to_owned(), Value::Bool(self.running_first));
        Value::Object(extra)
    }
}

#[derive(Clone, Debug)]
pub struct Preferences {
    visible: bool,
    passthrough: bool,
    alpha_passthrough: bool,
    bubble_visible: bool,
    show_status_indicators: bool,
    auto_update_check: bool,
    last_update_check: Option<u64>,
    menu_bar_mode: MenuBarMode,
    menu_bar_icon: Option<MenuBarIconPreference>,
    bubble_placement: BubblePlacement,
    scale: f64,
    position: Option<(f64, f64)>,
    standalone_bubble_position: Option<(f64, f64)>,
    bubble_sizes: BubbleSizes,
    position_space: PositionSpace,
    language: LanguagePreference,
    bubble_appearance: BubbleAppearance,
    observation: ObservationPreferences,
    dialogue_overrides: DialogueOverrides,
    session_list: SessionListPreferences,
    session_list_extra: Map<String, Value>,
    /// Top-level keys from a newer build, written back unchanged on save.
    extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DiskPreferences {
    visible: bool,
    passthrough: bool,
    #[serde(default)]
    alpha_passthrough: bool,
    #[serde(default = "default_bubble_visible")]
    bubble_visible: bool,
    #[serde(default = "default_show_status_indicators")]
    show_status_indicators: bool,
    #[serde(default = "default_auto_update_check")]
    auto_update_check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_update_check: Option<u64>,
    #[serde(default)]
    menu_bar_mode: MenuBarMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    menu_bar_icon: Option<MenuBarIconPreference>,
    #[serde(default)]
    bubble_placement: BubblePlacement,
    scale: f64,
    position: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    standalone_bubble_position: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bubble_sizes: Option<BubbleSizes>,
    #[serde(default)]
    position_space: PositionSpace,
    #[serde(default)]
    language: LanguagePreference,
    #[serde(default)]
    bubble_appearance: BubbleAppearance,
    #[serde(default)]
    observation: ObservationPreferences,
    #[serde(default)]
    dialogue_overrides: DialogueOverrides,
    #[serde(default)]
    session_list: Value,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn default_bubble_visible() -> bool {
    true
}

fn default_show_status_indicators() -> bool {
    true
}

fn default_auto_update_check() -> bool {
    true
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            visible: true,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            show_status_indicators: true,
            auto_update_check: true,
            last_update_check: None,
            menu_bar_mode: MenuBarMode::default(),
            menu_bar_icon: None,
            bubble_placement: BubblePlacement::default(),
            scale: DEFAULT_SCALE,
            position: None,
            standalone_bubble_position: None,
            bubble_sizes: BubbleSizes::default(),
            position_space: PositionSpace::Display,
            language: LanguagePreference::default(),
            bubble_appearance: BubbleAppearance::default(),
            observation: ObservationPreferences::default(),
            dialogue_overrides: DialogueOverrides::default(),
            session_list: SessionListPreferences::default(),
            session_list_extra: Map::new(),
            extra: Map::new(),
        }
    }
}
impl Preferences {
    /// Strict load for read-only consumers: an unusable file is reported as an
    /// error and left untouched.
    pub fn load(directory: &Path) -> Result<Self, String> {
        Self::load_in_directory(directory, legacy_preferences_path().as_deref(), false)
    }

    /// Daemon startup load: an unusable file is moved aside to
    /// `preferences.json.invalid-<unix_nanos>` and defaults are used, so the
    /// daemon neither refuses to start nor later overwrites the original bytes.
    /// Fails only when the file cannot be moved aside.
    pub(crate) fn load_for_daemon(directory: &Path) -> Result<Self, String> {
        Self::load_in_directory(directory, legacy_preferences_path().as_deref(), true)
    }

    fn load_in_directory(
        directory: &Path,
        legacy: Option<&Path>,
        quarantine_invalid: bool,
    ) -> Result<Self, String> {
        let path = directory.join(PREFERENCES_FILE);
        let existing = match Self::load_existing_path(&path) {
            Err(error) if quarantine_invalid => return Self::quarantine(directory, &path, &error),
            result => result?,
        };
        if let Some(preferences) = existing {
            return Ok(preferences);
        }
        let Some(preferences) = legacy.and_then(Self::load_legacy_path) else {
            return Ok(Self::default());
        };
        match preferences.save_migrated_in_directory(directory)? {
            true => Ok(preferences),
            false => Self::load_in_directory(directory, None, quarantine_invalid),
        }
    }

    fn quarantine(directory: &Path, path: &Path, error: &str) -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let backup = directory.join(format!("{PREFERENCES_FILE}.invalid-{nanos}"));
        fs::rename(path, &backup).map_err(|rename_error| {
            format!("{error}; unusable preferences cannot be moved aside: {rename_error}")
        })?;
        sync_directory(directory);
        eprintln!(
            "desktop-pet: {error}; moved unusable preferences to {} and started with defaults",
            backup.display()
        );
        Ok(Self::default())
    }

    #[cfg(test)]
    pub(crate) fn load_path(path: &Path) -> Result<Self, String> {
        Ok(Self::load_existing_path(path)?.unwrap_or_default())
    }

    fn load_existing_path(path: &Path) -> Result<Option<Self>, String> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("preferences cannot be read: {error}")),
        };
        let mut disk: DiskPreferences = serde_json::from_slice(&bytes)
            .map_err(|error| format!("preferences are invalid: {error}"))?;
        disk.dialogue_overrides
            .validate()
            .map_err(|error| format!("preferences dialogue overrides are invalid: {error}"))?;
        disk.dialogue_overrides.prune_empty();
        let (session_list, session_list_extra) =
            SessionListPreferences::from_disk(disk.session_list);
        let mut preferences = Self {
            visible: disk.visible,
            passthrough: disk.passthrough,
            alpha_passthrough: disk.alpha_passthrough,
            bubble_visible: disk.bubble_visible,
            show_status_indicators: disk.show_status_indicators,
            auto_update_check: disk.auto_update_check,
            last_update_check: disk.last_update_check,
            menu_bar_mode: disk.menu_bar_mode,
            menu_bar_icon: disk.menu_bar_icon,
            bubble_placement: disk.bubble_placement,
            scale: disk.scale,
            position: disk.position.map(|value| (value[0], value[1])),
            standalone_bubble_position: disk
                .standalone_bubble_position
                .map(|value| (value[0], value[1])),
            bubble_sizes: disk.bubble_sizes.unwrap_or_default(),
            position_space: disk.position_space,
            language: disk.language,
            bubble_appearance: disk.bubble_appearance,
            observation: disk.observation,
            dialogue_overrides: disk.dialogue_overrides,
            session_list,
            session_list_extra,
            extra: disk.extra,
        };
        preferences.sanitize();
        Ok(Some(preferences))
    }

    fn load_legacy_path(path: &Path) -> Option<Self> {
        Self::load_existing_path(path).ok().flatten()
    }

    /// Clone the owner's committed state before staging presentation or geometry.
    /// For example: `let mut next = prefs.candidate(); next.set_visible(false);
    /// prefs.save_candidate(next, directory)?;` Never mutate `prefs` before a runtime-first
    /// presentation attempt; a failed save must not leak into later transactions.
    pub(crate) fn candidate(&self) -> Self {
        self.clone()
    }

    /// Save the complete candidate once, then replace the owner's state only
    /// after the atomic writer succeeds. No live preferences are read from disk.
    pub(crate) fn save_candidate(
        &mut self,
        candidate: Self,
        directory: &Path,
    ) -> Result<(), String> {
        self.save_candidate_in_directory(candidate, directory)
    }

    fn save_candidate_in_directory(
        &mut self,
        candidate: Self,
        directory: &Path,
    ) -> Result<(), String> {
        candidate.save_in_directory(directory)?;
        *self = candidate;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> PreferenceSnapshot {
        PreferenceSnapshot {
            language: self.language,
            bubble_appearance: self.bubble_appearance,
            show_status_indicators: self.show_status_indicators,
            menu_bar_mode: self.menu_bar_mode,
            observation_local: self.observation.local,
            observation_remote: self.observation.remote,
            observation_machines: self.observation.machines.clone(),
        }
    }

    /// Validate the entire patch before writing a single candidate. Unlike
    /// legacy GUI observation saves, automation rejects malformed selections
    /// rather than silently pruning them.
    pub(crate) fn apply_patch(
        &mut self,
        patch: PreferencePatch,
        directory: &Path,
    ) -> Result<PreferenceSnapshot, String> {
        self.apply_patch_in_directory(patch, directory)
    }

    pub(crate) fn apply_patch_in_directory(
        &mut self,
        patch: PreferencePatch,
        directory: &Path,
    ) -> Result<PreferenceSnapshot, String> {
        let PreferencePatch {
            language,
            bubble_appearance,
            show_status_indicators,
            menu_bar_mode,
            observation_local,
            observation_remote,
            observation_machines,
        } = patch;
        if language.is_none()
            && bubble_appearance.is_none()
            && show_status_indicators.is_none()
            && menu_bar_mode.is_none()
            && observation_local.is_none()
            && observation_remote.is_none()
            && observation_machines.is_none()
        {
            return Err("preference patch must contain at least one setting".to_owned());
        }
        let mut candidate = self.candidate();
        if let Some(language) = language {
            candidate.language = language;
        }
        if let Some(appearance) = bubble_appearance {
            candidate.bubble_appearance = appearance;
        }
        if let Some(enabled) = show_status_indicators {
            candidate.show_status_indicators = enabled;
        }
        if let Some(mode) = menu_bar_mode {
            candidate.menu_bar_mode = mode;
        }
        if let Some(local) = observation_local {
            candidate.observation.local = local;
        }
        if let Some(remote) = observation_remote {
            candidate.observation.remote = remote;
        }
        if let Some(machines) = observation_machines {
            let mut checked = ObservationPreferences {
                machines,
                ..candidate.observation.clone()
            };
            let original = checked.machines.clone();
            checked.sanitize();
            if checked.machines != original {
                return Err(
                    "observation machines contain duplicate or invalid IDs, or exceed the selection limit"
                        .to_owned(),
                );
            }
            candidate.observation = checked;
        }
        self.save_candidate_in_directory(candidate, directory)?;
        Ok(self.snapshot())
    }

    pub(crate) fn dialogue_overrides(&self) -> &DialogueOverrides {
        &self.dialogue_overrides
    }

    pub(crate) fn save_dialogue_entry(
        &mut self,
        target: &DialogueTarget,
        locale: &str,
        slot: DialogueSlot,
        value: Option<String>,
        directory: &Path,
    ) -> Result<(), String> {
        self.save_dialogue_entry_in_directory(target, locale, slot, value, directory)
    }

    fn save_dialogue_entry_in_directory(
        &mut self,
        target: &DialogueTarget,
        locale: &str,
        slot: DialogueSlot,
        value: Option<String>,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.candidate();
        candidate
            .dialogue_overrides
            .set_entry(target, locale, slot, value)?;
        self.save_candidate_in_directory(candidate, directory)
    }

    pub(crate) fn reset_character_dialogue(
        &mut self,
        target: &DialogueTarget,
        directory: &Path,
    ) -> Result<(), String> {
        self.reset_character_dialogue_in_directory(target, directory)
    }

    fn reset_character_dialogue_in_directory(
        &mut self,
        target: &DialogueTarget,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.candidate();
        candidate.dialogue_overrides.remove_target(target)?;
        self.save_candidate_in_directory(candidate, directory)
    }

    pub(crate) fn save_menu_bar_icon(
        &mut self,
        preference: Option<MenuBarIconPreference>,
        directory: &Path,
    ) -> Result<(), String> {
        self.save_menu_bar_icon_in_directory(preference, directory)
    }

    pub(crate) fn observation(&self) -> &ObservationPreferences {
        &self.observation
    }

    pub(crate) fn session_list(&self) -> SessionListPreferences {
        self.session_list
    }

    pub(crate) fn save_session_list(
        &mut self,
        preference: SessionListPreferences,
        directory: &Path,
    ) -> Result<(), String> {
        self.save_session_list_in_directory(preference, directory)
    }

    fn save_session_list_in_directory(
        &mut self,
        preference: SessionListPreferences,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.candidate();
        candidate.session_list = preference;
        self.save_candidate_in_directory(candidate, directory)
    }

    fn save_menu_bar_icon_in_directory(
        &mut self,
        preference: Option<MenuBarIconPreference>,
        directory: &Path,
    ) -> Result<(), String> {
        if let Some(icon) = &preference {
            crate::menu_bar_icon::validate_asset_name(&icon.asset)?;
        }
        let previous = self.menu_bar_icon.clone();
        let mut candidate = self.candidate();
        candidate.menu_bar_icon = preference;
        self.save_candidate_in_directory(candidate, directory)?;
        // The normal preference writer deliberately ignores directory sync errors.
        // Only a separately confirmed directory sync permits unlinking the previous asset.
        if let Some(previous) = previous.filter(|old| self.menu_bar_icon.as_ref() != Some(old)) {
            if File::open(directory)
                .and_then(|file| file.sync_all())
                .is_ok()
            {
                crate::menu_bar_icon::cleanup_previous_in_directory(directory, &previous.asset);
            }
        }
        Ok(())
    }

    fn save_in_directory(&self, directory: &Path) -> Result<(), String> {
        self.write_atomically(directory, false).map(|_| ())
    }

    fn save_migrated_in_directory(&self, directory: &Path) -> Result<bool, String> {
        self.write_atomically(directory, true)
    }

    fn write_atomically(&self, directory: &Path, no_clobber: bool) -> Result<bool, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("preferences directory cannot be created: {error}"))?;
        set_private_directory(directory)?;
        let path = directory.join(PREFERENCES_FILE);

        let disk = DiskPreferences {
            visible: self.visible,
            passthrough: self.passthrough,
            alpha_passthrough: self.alpha_passthrough,
            bubble_visible: self.bubble_visible,
            show_status_indicators: self.show_status_indicators,
            auto_update_check: self.auto_update_check,
            last_update_check: self.last_update_check,
            menu_bar_mode: self.menu_bar_mode,
            menu_bar_icon: self.menu_bar_icon.clone(),
            bubble_placement: self.bubble_placement,
            scale: self.scale,
            position: self.position.map(|(x, y)| [x, y]),
            standalone_bubble_position: self.standalone_bubble_position.map(|(x, y)| [x, y]),
            bubble_sizes: Some(self.bubble_sizes),
            position_space: self.position_space,
            language: self.language,
            bubble_appearance: self.bubble_appearance,
            observation: self.observation.clone(),
            dialogue_overrides: self.dialogue_overrides.clone(),
            session_list: self.session_list.to_disk(self.session_list_extra.clone()),
            extra: self.extra.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&disk)
            .map_err(|error| format!("preferences cannot be encoded: {error}"))?;

        let temporary = temporary_path(directory);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("preferences temporary file cannot be created: {error}"))?;
        if let Err(error) = set_private_file(&file).and_then(|()| write_and_sync(&mut file, &bytes))
        {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        drop(file);

        if no_clobber {
            let result = match fs::hard_link(&temporary, &path) {
                Ok(()) => true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(format!("migrated preferences cannot be committed: {error}"));
                }
            };
            let _ = fs::remove_file(&temporary);
            if result {
                sync_directory(directory);
            }
            return Ok(result);
        }

        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("preferences cannot be committed: {error}"));
        }
        sync_directory(directory);
        Ok(true)
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn passthrough(&self) -> bool {
        self.passthrough
    }

    pub fn alpha_passthrough(&self) -> bool {
        self.alpha_passthrough
    }

    pub fn bubble_visible(&self) -> bool {
        self.bubble_visible
    }

    pub fn show_status_indicators(&self) -> bool {
        self.show_status_indicators
    }

    pub fn auto_update_check(&self) -> bool {
        self.auto_update_check
    }

    pub fn last_update_check(&self) -> Option<u64> {
        self.last_update_check
    }

    pub(crate) fn menu_bar_mode(&self) -> MenuBarMode {
        self.menu_bar_mode
    }

    pub(crate) fn menu_bar_icon(&self) -> Option<&MenuBarIconPreference> {
        self.menu_bar_icon.as_ref()
    }

    pub fn bubble_placement(&self) -> BubblePlacement {
        self.bubble_placement
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn position(&self) -> Option<(f64, f64)> {
        self.position
    }

    pub fn standalone_bubble_position(&self) -> Option<(f64, f64)> {
        self.standalone_bubble_position
    }

    pub(crate) fn bubble_sizes(&self) -> BubbleSizes {
        self.bubble_sizes
    }

    pub fn position_is_legacy(&self) -> bool {
        self.position_space == PositionSpace::LegacyCanvas
    }

    /// Mark the coordinate space after the UI migrates geometry, even without
    /// a saved position (a later save must not resurrect legacy geometry).
    pub fn mark_display_position(&mut self) {
        self.position_space = PositionSpace::Display;
    }

    pub fn language(&self) -> LanguagePreference {
        self.language
    }

    pub fn bubble_appearance(&self) -> BubbleAppearance {
        self.bubble_appearance
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn set_passthrough(&mut self, passthrough: bool) {
        self.passthrough = passthrough;
    }

    pub fn set_alpha_passthrough(&mut self, alpha_passthrough: bool) {
        self.alpha_passthrough = alpha_passthrough;
    }

    pub fn set_bubble_visible(&mut self, bubble_visible: bool) {
        self.bubble_visible = bubble_visible;
    }

    pub fn set_auto_update_check(&mut self, enabled: bool) {
        self.auto_update_check = enabled;
    }

    pub fn set_last_update_check(&mut self, checked_at: Option<u64>) {
        self.last_update_check = checked_at;
    }

    pub fn set_bubble_placement(&mut self, bubble_placement: BubblePlacement) {
        self.bubble_placement = bubble_placement;
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = normalize_scale(scale).unwrap_or(DEFAULT_SCALE);
    }

    pub fn set_position(&mut self, position: Option<(f64, f64)>) {
        self.position = position.filter(|(x, y)| x.is_finite() && y.is_finite());
        self.mark_display_position();
    }

    pub fn set_standalone_bubble_position(&mut self, position: Option<(f64, f64)>) {
        self.standalone_bubble_position = position.filter(|(x, y)| x.is_finite() && y.is_finite());
    }

    pub(crate) fn set_bubble_sizes(&mut self, mut sizes: BubbleSizes) {
        sizes.sanitize();
        self.bubble_sizes = sizes;
    }

    fn sanitize(&mut self) {
        self.scale = normalize_scale(self.scale).unwrap_or(DEFAULT_SCALE);
        self.position = self
            .position
            .filter(|(x, y)| x.is_finite() && y.is_finite() && x.abs() < 1.0e7 && y.abs() < 1.0e7);
        self.standalone_bubble_position = self
            .standalone_bubble_position
            .filter(|(x, y)| x.is_finite() && y.is_finite());
        self.bubble_sizes.sanitize();
        self.observation.sanitize();
    }
}

fn legacy_preferences_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("OMPet")
                .join(PREFERENCES_FILE)
        })
}

fn temporary_path(directory: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    directory.join(format!(
        ".{PREFERENCES_FILE}.{}.{}",
        std::process::id(),
        nonce
    ))
}

fn write_and_sync(file: &mut File, bytes: &[u8]) -> Result<(), String> {
    file.write_all(bytes)
        .map_err(|error| format!("preferences cannot be written: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("preferences cannot be synced: {error}"))
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("preferences directory permissions cannot be set: {error}"))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(file: &File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("preferences file permissions cannot be set: {error}"))
}

#[cfg(not(unix))]
fn set_private_file(_file: &File) -> Result<(), String> {
    Ok(())
}

fn sync_directory(directory: &Path) {
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn explicit_profile_load_writes_and_failed_candidate_preserve_other_profile() {
        let root = isolated_preferences_directory();
        let selected = root.join("profile-a");
        let default = root.join("default");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&default).unwrap();
        let sentinel =
            br#"{"visible":false,"passthrough":false,"scale":0.75,"future_default":"untouched"}"#;
        fs::write(default.join(PREFERENCES_FILE), sentinel).unwrap();
        Preferences::default().save_in_directory(&selected).unwrap();

        let mut owner = Preferences::load_for_daemon(&selected).unwrap();
        assert!(owner.visible());
        owner
            .apply_patch(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &selected,
            )
            .unwrap();
        let mut candidate = owner.candidate();
        candidate.set_auto_update_check(false);
        owner.save_candidate(candidate, &selected).unwrap();
        assert!(!Preferences::load(&selected).unwrap().auto_update_check());
        assert_eq!(
            Preferences::load(&selected).unwrap().language(),
            LanguagePreference::En
        );

        let blocked = root.join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        let saved = fs::read(selected.join(PREFERENCES_FILE)).unwrap();
        let mut candidate = owner.candidate();
        candidate.set_visible(false);
        owner.save_candidate(candidate, &blocked).unwrap_err();
        assert!(owner.visible());
        assert_eq!(fs::read(selected.join(PREFERENCES_FILE)).unwrap(), saved);
        assert_eq!(fs::read(default.join(PREFERENCES_FILE)).unwrap(), sentinel);
        assert!(!Preferences::load(&default).unwrap().visible());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn daemon_quarantine_and_legacy_import_write_only_selected_destination() {
        let root = isolated_preferences_directory();
        let selected = root.join("profile-a");
        let other = root.join("default");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&other).unwrap();
        let sentinel =
            br#"{"visible":false,"passthrough":false,"scale":0.75,"future_default":"untouched"}"#;
        fs::write(other.join(PREFERENCES_FILE), sentinel).unwrap();
        let invalid = b"{bad json";
        fs::write(selected.join(PREFERENCES_FILE), invalid).unwrap();

        Preferences::load(&selected).unwrap_err();
        assert_eq!(fs::read(selected.join(PREFERENCES_FILE)).unwrap(), invalid);
        let loaded = Preferences::load_for_daemon(&selected).unwrap();
        assert!(loaded.visible());
        assert!(!selected.join(PREFERENCES_FILE).exists());
        let backups: Vec<_> = fs::read_dir(&selected)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), invalid);

        let legacy = root.join("legacy.json");
        let legacy_bytes =
            br#"{"visible":false,"passthrough":false,"scale":0.875,"future_legacy":{"kept":true}}"#;
        fs::write(&legacy, legacy_bytes).unwrap();
        let migrated = Preferences::load_in_directory(&selected, Some(&legacy), false).unwrap();
        assert!(!migrated.visible());
        assert_eq!(migrated.scale(), 0.875);
        assert_eq!(
            Preferences::load(&selected).unwrap().extra["future_legacy"]["kept"],
            true
        );
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
        assert_eq!(fs::read(other.join(PREFERENCES_FILE)).unwrap(), sentinel);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn convenience_writers_keep_assets_and_preferences_in_selected_profile() {
        let root = isolated_preferences_directory();
        let selected = root.join("profile-a");
        let other = root.join("default");
        let selected_icons = selected.join("menu-bar-icons");
        let other_icons = other.join("menu-bar-icons");
        fs::create_dir_all(&selected_icons).unwrap();
        fs::create_dir_all(&other_icons).unwrap();
        fs::set_permissions(&selected_icons, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&other_icons, fs::Permissions::from_mode(0o700)).unwrap();
        let (icon, selected_icon, bytes) = private_png_asset(&selected_icons, [200, 20, 30, 255]);
        let other_icon = other_icons.join(&icon.asset);
        fs::write(&other_icon, &bytes).unwrap();
        fs::set_permissions(&other_icon, fs::Permissions::from_mode(0o600)).unwrap();
        let sentinel =
            br#"{"visible":false,"passthrough":false,"scale":0.75,"future_default":"untouched"}"#;
        fs::write(other.join(PREFERENCES_FILE), sentinel).unwrap();
        let target = DialogueTarget::Character("forest".to_owned());
        let mut owner = Preferences::default();
        owner
            .save_menu_bar_icon(Some(icon.clone()), &selected)
            .unwrap();
        owner
            .save_dialogue_entry(
                &target,
                "en",
                DialogueSlot::Idle,
                Some("Selected greeting".to_owned()),
                &selected,
            )
            .unwrap();
        let list = SessionListPreferences {
            sort: SessionSort::TitleAsc,
            running_first: true,
        };
        owner.save_session_list(list, &selected).unwrap();
        assert_eq!(Preferences::load(&selected).unwrap().session_list(), list);
        assert_eq!(
            Preferences::load(&selected)
                .unwrap()
                .dialogue_overrides()
                .entry(&target, "en", DialogueSlot::Idle),
            Some("Selected greeting")
        );
        owner.reset_character_dialogue(&target, &selected).unwrap();
        owner.save_menu_bar_icon(None, &selected).unwrap();
        assert!(!selected_icon.exists());
        assert!(Preferences::load(&selected)
            .unwrap()
            .dialogue_overrides()
            .locales(&target)
            .is_none());
        assert_eq!(fs::read(&other_icon).unwrap(), bytes);
        assert_eq!(fs::read(other.join(PREFERENCES_FILE)).unwrap(), sentinel);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn partial_preference_writes_keep_update_settings_and_future_fields() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.extra.insert(
            "future_setting".to_owned(),
            serde_json::json!({"nested": [1, 2]}),
        );
        let mut candidate = preferences.candidate();
        candidate.set_auto_update_check(false);
        candidate.set_last_update_check(Some(1_738_000_000));
        preferences
            .save_candidate_in_directory(candidate, &directory)
            .unwrap();
        let mut reloaded = Preferences::load_path(&path).unwrap();
        reloaded
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let mut candidate = reloaded.candidate();
        candidate.set_visible(false);
        reloaded
            .save_candidate_in_directory(candidate, &directory)
            .unwrap();
        let saved = Preferences::load_path(&path).unwrap();
        assert!(!saved.auto_update_check());
        assert_eq!(saved.last_update_check(), Some(1_738_000_000));
        assert!(!saved.visible());
        assert_eq!(saved.language(), LanguagePreference::En);
        assert_eq!(
            saved.extra["future_setting"],
            serde_json::json!({"nested": [1, 2]})
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_update_setting_candidate_does_not_change_owner_or_saved_preferences() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let blocked = directory.join("not-a-directory");
        let mut preferences = Preferences::default();
        preferences.set_last_update_check(Some(42));
        preferences.save_in_directory(&directory).unwrap();
        let before = fs::read(&path).unwrap();
        fs::write(&blocked, b"sentinel").unwrap();
        let mut candidate = preferences.candidate();
        candidate.set_auto_update_check(false);
        candidate.set_last_update_check(Some(43));
        preferences
            .save_candidate_in_directory(candidate, &blocked)
            .expect_err("unwritable candidate must not commit");
        assert!(preferences.auto_update_check());
        assert_eq!(preferences.last_update_check(), Some(42));
        assert_eq!(fs::read(&path).unwrap(), before);
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let saved = Preferences::load_path(&path).unwrap();
        assert!(saved.auto_update_check());
        assert_eq!(saved.last_update_check(), Some(42));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn malformed_update_fields_reject_file_like_other_typed_settings() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        Preferences::default()
            .save_in_directory(&directory)
            .unwrap();
        let valid: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (field, malformed) in [
            ("auto_update_check", serde_json::json!("yes")),
            ("last_update_check", serde_json::json!(-1)),
            ("last_update_check", serde_json::json!("yesterday")),
        ] {
            let mut disk = valid.clone();
            disk[field] = malformed;
            fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
            assert!(
                Preferences::load_path(&path).is_err(),
                "{field} must reject malformed data"
            );
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn bubble_sizes_roundtrip_independently_and_reset_without_moving_bubble() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let sizes = BubbleSizes {
            compact: Some(BubbleSize {
                width: 275.5,
                height: 140.25,
            }),
            expanded: Some(BubbleSize {
                width: 490.0,
                height: 315.75,
            }),
        };
        let mut preferences = Preferences::default();
        preferences.set_standalone_bubble_position(Some((-52.0, 87.0)));
        preferences.set_bubble_sizes(sizes);
        preferences.save_in_directory(&directory).unwrap();
        let mut loaded = Preferences::load_path(&path).unwrap();
        assert_eq!(loaded.bubble_sizes(), sizes);
        assert_eq!(loaded.standalone_bubble_position(), Some((-52.0, 87.0)));
        loaded.set_standalone_bubble_position(None);
        assert_eq!(loaded.bubble_sizes(), sizes);
        loaded.set_bubble_sizes(BubbleSizes::default());
        loaded.save_in_directory(&directory).unwrap();
        assert_eq!(
            Preferences::load_path(&path).unwrap().bubble_sizes(),
            BubbleSizes::default()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn old_and_null_bubble_sizes_use_automatic_layout() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        for payload in [
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":null}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":null}}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"expanded":null}}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":[]}"#,
        ] {
            fs::write(&path, payload).unwrap();
            assert_eq!(
                Preferences::load_path(&path).unwrap().bubble_sizes(),
                BubbleSizes::default()
            );
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_bubble_size_mode_does_not_discard_valid_peer() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":0,"height":120},"expanded":{"width":420,"height":260}}}"#,
        )
        .unwrap();
        let mut loaded = Preferences::load_path(&path).unwrap();
        assert_eq!(loaded.bubble_sizes().compact, None);
        assert_eq!(
            loaded.bubble_sizes().expanded,
            Some(BubbleSize {
                width: 420.0,
                height: 260.0
            })
        );
        loaded.set_bubble_sizes(BubbleSizes {
            compact: Some(BubbleSize {
                width: f64::NAN,
                height: 120.0,
            }),
            expanded: Some(BubbleSize {
                width: 420.0,
                height: 260.0,
            }),
        });
        assert_eq!(loaded.bubble_sizes().compact, None);
        loaded.save_in_directory(&directory).unwrap();
        assert_eq!(
            Preferences::load_path(&path).unwrap().bubble_sizes(),
            loaded.bubble_sizes()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn malformed_bubble_size_shapes_and_dimensions_preserve_valid_peer_without_quarantine() {
        for invalid_value in [
            "null",
            "false",
            r#""large""#,
            "42",
            "[]",
            r#"[{"width":287,"height":134}]"#,
            r#"{"width":287}"#,
            r#"{"height":134}"#,
            r#"{"width":null,"height":134}"#,
            r#"{"width":"287","height":134}"#,
            r#"{"width":true,"height":134}"#,
            r#"{"width":{"nested":[1,{"deep":true}]},"height":134}"#,
            r#"{"width":[287,{"nested":[1,2]}],"height":134}"#,
            r#"{"width":287,"height":null}"#,
            r#"{"width":287,"height":"134"}"#,
            r#"{"width":287,"height":false}"#,
            r#"{"width":287,"height":{"nested":[1,2]}}"#,
            r#"{"width":287,"height":[134,{"nested":true}]}"#,
            r#"{"width":0,"height":134}"#,
            r#"{"width":-287,"height":134}"#,
            r#"{"width":287,"height":0}"#,
            r#"{"width":287,"height":-134}"#,
            r#"{"width":287,"width":288,"height":134}"#,
            r#"{"width":287,"\u0077idth":288,"height":134}"#,
            r#"{"width":287,"height":134,"height":135}"#,
            r#"{"width":287,"height":134,"\u0068eight":135}"#,
        ] {
            for invalid_mode in ["compact", "expanded"] {
                let valid_mode = if invalid_mode == "compact" {
                    "expanded"
                } else {
                    "compact"
                };
                let directory = isolated_preferences_directory();
                let path = directory.join(PREFERENCES_FILE);
                let payload = format!(
                    r##"{{"visible":false,"passthrough":true,"bubble_visible":false,"scale":1.25,"position":[12.5,34.5],"standalone_bubble_position":[-52,87],"position_space":"display","bubble_appearance":{{"theme":"custom","custom":{{"surface":"#112233","text":"#445566","muted":"#778899","border":"#AABBCC","accent":"#DDEEFF"}}}},"bubble_sizes":{{"{invalid_mode}":{invalid_value},"{valid_mode}":{{"width":420,"height":260}}}},"future_setting":{{"keep":[1,2]}}}}"##
                );
                fs::write(&path, payload.as_bytes()).unwrap();
                let expected = BubbleSize {
                    width: 420.0,
                    height: 260.0,
                };
                for daemon in [false, true] {
                    let loaded = Preferences::load_in_directory(&directory, None, daemon)
                        .expect("a malformed individual mode must not reject preferences");
                    let sizes = loaded.bubble_sizes();
                    if invalid_mode == "compact" {
                        assert_eq!(sizes.compact, None);
                        assert_eq!(sizes.expanded, Some(expected));
                    } else {
                        assert_eq!(sizes.compact, Some(expected));
                        assert_eq!(sizes.expanded, None);
                    }
                    assert!(!loaded.visible());
                    assert!(loaded.passthrough());
                    assert!(!loaded.bubble_visible());
                    assert_eq!(loaded.scale(), 1.25);
                    assert_eq!(loaded.position(), Some((12.5, 34.5)));
                    assert_eq!(loaded.standalone_bubble_position(), Some((-52.0, 87.0)));
                    assert_eq!(loaded.bubble_appearance().theme, BubbleTheme::Custom);
                    assert_eq!(
                        loaded.extra["future_setting"],
                        serde_json::json!({"keep":[1,2]})
                    );
                }
                assert_eq!(fs::read(&path).unwrap(), payload.as_bytes());
                assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    #[test]
    fn escaped_bubble_size_keys_unknown_nested_values_and_unrelated_save_keep_valid_mode() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":1.25,"position":null,"bubble_sizes":{"compact":{"\u0077idth":287,"\u0068eight":134,"Width":{"future":[{"nested":true}]},"future":[[1,2],{"deep":false}]},"expanded":{"width":"bad","height":260}},"future_setting":{"keep":[1,2]}}"#,
        )
        .unwrap();
        let mut loaded = Preferences::load_path(&path).unwrap();
        let expected = BubbleSizes {
            compact: Some(BubbleSize {
                width: 287.0,
                height: 134.0,
            }),
            expanded: None,
        };
        assert_eq!(loaded.bubble_sizes(), expected);
        loaded
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::Ko),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let saved = Preferences::load_path(&path).unwrap();
        assert_eq!(saved.bubble_sizes(), expected);
        assert_eq!(saved.language(), LanguagePreference::Ko);
        assert_eq!(saved.scale(), 1.25);
        assert!(!saved.visible());
        assert!(saved.passthrough());
        assert_eq!(
            saved.extra["future_setting"],
            serde_json::json!({"keep":[1,2]})
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unrelated_patch_preserves_bubble_sizes_and_unknown_fields() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":287,"height":134},"expanded":{"width":487,"height":280}},"future_setting":{"keep":[1,2]}}"#,
        )
        .unwrap();
        let mut preferences = Preferences::load_path(&path).unwrap();
        let sizes = preferences.bubble_sizes();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::Ko),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["future_setting"], serde_json::json!({"keep":[1,2]}));
        assert_eq!(
            raw["bubble_sizes"]["compact"]["width"],
            serde_json::json!(287.0)
        );
        preferences.set_bubble_visible(false);
        preferences.save_in_directory(&directory).unwrap();
        let loaded = Preferences::load_path(&path).unwrap();
        assert_eq!(loaded.bubble_sizes(), sizes);
        assert_eq!(loaded.language(), LanguagePreference::Ko);
        assert!(!loaded.bubble_visible());
        let raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["future_setting"], serde_json::json!({"keep":[1,2]}));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_bubble_size_candidate_does_not_leak_into_later_save() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let backup = directory.join("original-preferences.json");
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":1.25,"position":null,"bubble_sizes":{"compact":{"width":287,"height":134},"expanded":{"width":{"invalid":[1,2]},"height":260}},"future_setting":{"keep":[1,2]}}"#,
        )
        .unwrap();
        let mut preferences = Preferences::load_path(&path).unwrap();
        let original_sizes = BubbleSizes {
            compact: Some(BubbleSize {
                width: 287.0,
                height: 134.0,
            }),
            expanded: None,
        };
        assert_eq!(preferences.bubble_sizes(), original_sizes);
        let before = fs::read(&path).unwrap();
        fs::rename(&path, &backup).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("sentinel"), b"untouched").unwrap();
        let mut candidate = preferences.candidate();
        candidate.set_bubble_sizes(BubbleSizes {
            compact: Some(BubbleSize {
                width: 280.0,
                height: 140.0,
            }),
            expanded: Some(BubbleSize {
                width: 480.0,
                height: 290.0,
            }),
        });
        preferences
            .save_candidate_in_directory(candidate, &directory)
            .expect_err("target-path directory prevents atomic rename");
        assert_eq!(preferences.bubble_sizes(), original_sizes);
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"untouched");
        fs::remove_dir_all(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let saved = Preferences::load_path(&path).unwrap();
        assert_eq!(saved.bubble_sizes(), original_sizes);
        assert_eq!(saved.language(), LanguagePreference::En);
        assert_eq!(saved.scale(), 1.25);
        assert!(!saved.visible());
        assert!(saved.passthrough());
        assert_eq!(
            saved.extra["future_setting"],
            serde_json::json!({"keep":[1,2]})
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn sanitizes_invalid_scale_and_position() {
        let mut preferences = Preferences {
            visible: true,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            show_status_indicators: true,
            auto_update_check: true,
            last_update_check: None,
            menu_bar_mode: MenuBarMode::default(),
            bubble_placement: BubblePlacement::Above,
            scale: f64::NAN,
            menu_bar_icon: None,
            position: Some((f64::INFINITY, 2.0)),
            standalone_bubble_position: Some((f64::NAN, 3.0)),
            bubble_sizes: BubbleSizes::default(),
            position_space: PositionSpace::LegacyCanvas,
            language: LanguagePreference::default(),
            bubble_appearance: BubbleAppearance::default(),
            observation: ObservationPreferences::default(),
            dialogue_overrides: DialogueOverrides::default(),
            session_list: SessionListPreferences::default(),
            session_list_extra: Map::new(),
            extra: Map::new(),
        };
        preferences.sanitize();
        assert_eq!(preferences.scale(), DEFAULT_SCALE);
        assert!(preferences.position().is_none());
        assert_eq!(preferences.standalone_bubble_position(), None);
    }

    fn isolated_preferences_directory() -> PathBuf {
        let sequence = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "omp-pet-preferences-test-{}-{}",
            std::process::id(),
            sequence
        ));
        fs::create_dir(&path).expect("isolated preferences directory should be created");
        path
    }

    #[test]
    fn session_list_old_partial_future_and_malformed_values_do_not_invalidate_preferences() {
        let cases = [
            (None, SessionListPreferences::default()),
            (
                Some(serde_json::json!({"running_first": true})),
                SessionListPreferences {
                    sort: SessionSort::Stable,
                    running_first: true,
                },
            ),
            (
                Some(serde_json::json!({"sort": "title_asc"})),
                SessionListPreferences {
                    sort: SessionSort::TitleAsc,
                    running_first: false,
                },
            ),
            (
                Some(serde_json::json!({"sort": "future_sort", "running_first": true})),
                SessionListPreferences {
                    sort: SessionSort::Stable,
                    running_first: true,
                },
            ),
            (
                Some(serde_json::json!({"sort": 42, "running_first": "yes"})),
                SessionListPreferences::default(),
            ),
            (
                Some(serde_json::json!([1, 2])),
                SessionListPreferences::default(),
            ),
            (Some(Value::Null), SessionListPreferences::default()),
        ];
        for (session_list, expected) in cases {
            let directory = isolated_preferences_directory();
            let path = directory.join(PREFERENCES_FILE);
            let mut disk = serde_json::json!({
                "visible": false, "passthrough": true, "scale": 0.875,
                "position": [13.0, 27.0], "future_setting": {"a": 1}
            });
            if let Some(value) = session_list {
                disk["session_list"] = value;
            }
            fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
            let loaded = Preferences::load_path(&path).expect("old or new fields should load");
            assert_eq!(loaded.session_list(), expected);
            assert!(!loaded.visible());
            assert!(loaded.passthrough());
            assert_eq!(loaded.position(), Some((13.0, 27.0)));
            assert_eq!(loaded.extra["future_setting"], serde_json::json!({"a": 1}));
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn session_list_roundtrip_and_unrelated_save_retain_existing_and_future_values() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "visible": false, "passthrough": true, "scale": 0.875,
                "position": [3.0, 9.0], "menu_bar_mode": "recovery_only",
                "future_setting": {"a": [1, 2]},
                "session_list": {
                    "sort": "future_sort", "running_first": "invalid",
                    "future_field": {"nested": [3, 4]}
                },
                "bubble_sizes": {
                    "compact": {"width": "invalid", "height": 120},
                    "expanded": {"width": 490, "height": 310}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let mut preferences = Preferences::load_path(&path).unwrap();
        let expected_sizes = BubbleSizes {
            compact: None,
            expanded: Some(BubbleSize {
                width: 490.0,
                height: 310.0,
            }),
        };
        assert_eq!(preferences.bubble_sizes(), expected_sizes);
        let requested = SessionListPreferences {
            sort: SessionSort::SourceAsc,
            running_first: true,
        };
        preferences
            .save_session_list_in_directory(requested, &directory)
            .unwrap();
        assert_eq!(preferences.session_list(), requested);
        assert_eq!(preferences.bubble_sizes(), expected_sizes);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["session_list"]["sort"], "source_asc");
        assert_eq!(saved["session_list"]["running_first"], true);
        assert_eq!(
            saved["session_list"]["future_field"],
            serde_json::json!({"nested": [3, 4]})
        );
        assert_eq!(saved["future_setting"], serde_json::json!({"a": [1, 2]}));
        assert_eq!(saved["menu_bar_mode"], "recovery_only");
        assert_eq!(saved["position"], serde_json::json!([3.0, 9.0]));

        assert_eq!(
            Preferences::load_path(&path).unwrap().bubble_sizes(),
            expected_sizes
        );
        let mut reloaded = Preferences::load_path(&path).unwrap();
        assert_eq!(reloaded.session_list(), requested);
        let mut candidate = reloaded.candidate();
        candidate.menu_bar_mode = MenuBarMode::Always;
        reloaded
            .save_candidate_in_directory(candidate, &directory)
            .unwrap();
        let later: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(later["session_list"], saved["session_list"]);
        assert_eq!(later["future_setting"], saved["future_setting"]);
        assert_eq!(
            Preferences::load_path(&path).unwrap().session_list(),
            requested
        );
        assert_eq!(
            Preferences::load_path(&path).unwrap().bubble_sizes(),
            expected_sizes
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_session_list_save_keeps_memory_and_existing_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let blocked = directory.join("not-a-directory");
        let original = SessionListPreferences {
            sort: SessionSort::TitleAsc,
            running_first: true,
        };
        let mut preferences = Preferences::default();
        preferences
            .save_session_list_in_directory(original, &directory)
            .unwrap();
        let before = fs::read(&path).unwrap();
        fs::write(&blocked, b"sentinel").unwrap();
        preferences
            .save_session_list_in_directory(SessionListPreferences::default(), &blocked)
            .expect_err("save through a file path should fail");
        assert_eq!(preferences.session_list(), original);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&blocked).unwrap(), b"sentinel");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn patch_rejects_invalid_language_and_nested_appearance_without_affecting_disk_compatibility() {
        assert!(serde_json::from_value::<PreferencePatch>(
            serde_json::json!({"language": "en", "unknown_setting": true})
        )
        .is_err());
        for language in [
            serde_json::json!("future"),
            serde_json::json!(42),
            serde_json::json!({}),
        ] {
            assert!(
                serde_json::from_value::<PreferencePatch>(serde_json::json!({
                    "language": language, "show_status_indicators": false
                }))
                .is_err()
            );
        }
        assert_eq!(
            serde_json::from_value::<PreferencePatch>(serde_json::json!({"language": "ko"}))
                .unwrap()
                .language,
            Some(LanguagePreference::Ko)
        );
        assert_eq!(
            serde_json::from_value::<PreferencePatch>(serde_json::json!({"language": null}))
                .unwrap()
                .language,
            None
        );
        let appearance = serde_json::to_value(BubbleAppearance::default()).unwrap();
        for invalid in [
            serde_json::json!({"theme":"warm_ivory","custom":appearance["custom"],"future":1}),
            serde_json::json!({"theme":"warm_ivory","custom":{
                "surface":"#F5EEE5","text":"#473B3C","muted":"#8C7977",
                "border":"#DED0C7","accent":"#AA6868","future":true
            }}),
            serde_json::json!({"theme":"warm_ivory","custom":{
                "surface":"invalid","text":"#473B3C","muted":"#8C7977",
                "border":"#DED0C7","accent":"#AA6868"
            }}),
        ] {
            assert!(
                serde_json::from_value::<PreferencePatch>(serde_json::json!({
                    "bubble_appearance": invalid
                }))
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<PreferencePatch>(serde_json::json!({
                "bubble_appearance": appearance
            }))
            .is_ok()
        );
        assert_eq!(
            serde_json::from_str::<LanguagePreference>("\"future\"").unwrap(),
            LanguagePreference::System
        );
    }

    #[test]
    fn preference_patch_commits_all_settings_without_losing_unknown_fields() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.extra.insert(
            "future_setting".to_owned(),
            serde_json::json!({"preserve": [1, 2]}),
        );
        preferences.save_in_directory(&directory).unwrap();
        let mut appearance = preferences.bubble_appearance();
        appearance.theme = BubbleTheme::Custom;
        appearance.custom.accent = BubbleColor::parse_hex("#aBcDeF").unwrap();
        let snapshot = preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::Ko),
                    bubble_appearance: Some(appearance),
                    show_status_indicators: Some(false),
                    menu_bar_mode: Some(MenuBarMode::RecoveryOnly),
                    observation_local: Some(false),
                    observation_remote: Some(true),
                    observation_machines: Some(vec![" machine:/🌲 ".to_owned()]),
                },
                &directory,
            )
            .unwrap();
        assert_eq!(snapshot.language, LanguagePreference::Ko);
        assert_eq!(snapshot.bubble_appearance, appearance);
        assert!(!snapshot.show_status_indicators);
        assert_eq!(snapshot.menu_bar_mode, MenuBarMode::RecoveryOnly);
        assert!(!snapshot.observation_local);
        assert!(snapshot.observation_remote);
        assert_eq!(snapshot.observation_machines, vec![" machine:/🌲 "]);
        assert_eq!(snapshot, preferences.snapshot());
        let saved = Preferences::load_path(&path).unwrap();
        assert_eq!(saved.snapshot(), snapshot);
        assert_eq!(saved.extra, preferences.extra);
        assert_eq!(
            saved.extra["future_setting"],
            serde_json::json!({"preserve": [1, 2]})
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_multi_field_patch_preserves_owner_and_previous_disk_bytes() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let backup = directory.join("previous-preferences.json");
        let mut preferences = Preferences::default();
        preferences.save_in_directory(&directory).unwrap();
        let before = fs::read(&path).unwrap();
        let original = preferences.snapshot();
        fs::rename(&path, &backup).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("sentinel"), b"untouched").unwrap();

        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    observation_remote: Some(true),
                    observation_machines: Some(vec!["opaque-machine".to_owned()]),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect_err("target-path directory prevents the atomic rename");
        assert_eq!(preferences.snapshot(), original);
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"untouched");
        fs::remove_dir_all(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(Preferences::load_path(&path).unwrap().snapshot(), original);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_presentation_candidate_does_not_ghost_commit_on_next_setting_save() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let backup = directory.join("original-preferences.json");
        let mut preferences = Preferences::default();
        preferences.save_in_directory(&directory).unwrap();
        let before = fs::read(&path).unwrap();
        fs::rename(&path, &backup).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("sentinel"), b"untouched").unwrap();

        let mut candidate = preferences.candidate();
        candidate.set_visible(false);
        candidate.set_position(Some((42.0, 64.0)));
        preferences
            .save_candidate_in_directory(candidate, &directory)
            .expect_err("a target-path directory must prevent rename");
        assert!(preferences.visible());
        assert_eq!(preferences.position(), None);
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(fs::read(path.join("sentinel")).unwrap(), b"untouched");

        fs::remove_dir_all(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        assert!(preferences.visible());
        assert_eq!(preferences.position(), None);
        let saved = Preferences::load_path(&path).unwrap();
        assert!(saved.visible());
        assert_eq!(saved.position(), None);
        assert_eq!(saved.language(), LanguagePreference::En);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_multi_field_patch_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.save_in_directory(&directory).unwrap();
        let before = fs::read(&path).unwrap();
        let original = preferences.snapshot();
        for machines in [
            vec!["duplicate".to_owned(), "duplicate".to_owned()],
            vec!["invalid\0id".to_owned()],
            vec!["id".repeat(4097)],
            (0..65).map(|index| index.to_string()).collect(),
        ] {
            preferences
                .apply_patch_in_directory(
                    PreferencePatch {
                        language: Some(LanguagePreference::En),
                        observation_machines: Some(machines),
                        ..PreferencePatch::default()
                    },
                    &directory,
                )
                .expect_err("invalid selection must reject the entire patch");
            assert_eq!(preferences.snapshot(), original);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        preferences
            .apply_patch_in_directory(PreferencePatch::default(), &directory)
            .expect_err("an empty patch is not a mutation");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(preferences.snapshot(), original);
        assert_eq!(Preferences::load_path(&path).unwrap().snapshot(), original);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn saves_and_loads_scale_and_position_with_isolated_path() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let precise_scale = 0.873125;
        let position = Some((123.25, 456.75));

        let mut preferences = Preferences::default();
        preferences.set_scale(precise_scale);
        preferences.set_position(position);
        preferences
            .save_in_directory(&directory)
            .expect("preferences should save");

        let loaded = Preferences::load_path(&path).expect("preferences should load");
        assert_eq!(loaded.scale(), precise_scale);
        assert_eq!(loaded.position(), position);
        assert!(!loaded.position_is_legacy());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn standalone_origin_roundtrips_without_migrating_or_overwriting_pet_position() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":false,"scale":0.8,"position":[42.0,64.0]}"#,
        )
        .expect("legacy preferences should be written");
        let mut preferences =
            Preferences::load_path(&path).expect("legacy preferences should load");
        assert!(preferences.position_is_legacy());
        preferences.set_standalone_bubble_position(Some((-1250.25, -310.75)));
        assert!(preferences.position_is_legacy());
        preferences
            .save_in_directory(&directory)
            .expect("standalone origin should save");
        let loaded = Preferences::load_path(&path).expect("standalone origin should reload");
        assert_eq!(loaded.position(), Some((42.0, 64.0)));
        assert!(loaded.position_is_legacy());
        assert_eq!(
            loaded.standalone_bubble_position(),
            Some((-1250.25, -310.75))
        );

        preferences.set_position(Some((15.0, 25.0)));
        assert_eq!(
            preferences.standalone_bubble_position(),
            Some((-1250.25, -310.75))
        );
        preferences.set_standalone_bubble_position(None);
        assert_eq!(preferences.position(), Some((15.0, 25.0)));
        preferences
            .save_in_directory(&directory)
            .expect("cleared origin should save");
        let raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(raw.get("standalone_bubble_position").is_none());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn standalone_origin_rejects_nonfinite_but_retains_large_finite_coordinates() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.set_standalone_bubble_position(Some((f64::NAN, 3.0)));
        assert_eq!(preferences.standalone_bubble_position(), None);
        preferences.set_standalone_bubble_position(Some((3.0, f64::NEG_INFINITY)));
        assert_eq!(preferences.standalone_bubble_position(), None);
        preferences.set_standalone_bubble_position(Some((-1.0e100, 1.0e100)));
        preferences
            .save_in_directory(&directory)
            .expect("large origin should save");
        let loaded = Preferences::load_path(&path).expect("large origin should reload");
        assert_eq!(
            loaded.standalone_bubble_position(),
            Some((-1.0e100, 1.0e100))
        );
        assert_eq!(loaded.position(), None);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn old_schema_preserves_passthrough_and_geometry_with_new_defaults() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":0.875,"position":[123.25,456.75]}"#,
        )
        .expect("old preferences should be written");

        let loaded = Preferences::load_path(&path).expect("old preferences should load");
        assert!(!loaded.visible());
        assert!(loaded.passthrough());
        assert!(!loaded.alpha_passthrough());
        assert!(loaded.bubble_visible());
        assert_eq!(loaded.bubble_placement(), BubblePlacement::Above);
        assert_eq!(loaded.scale(), 0.875);
        assert_eq!(loaded.position(), Some((123.25, 456.75)));
        assert_eq!(loaded.standalone_bubble_position(), None);
        assert!(loaded.position_is_legacy());

        assert_eq!(loaded.language(), LanguagePreference::System);
        assert_eq!(loaded.bubble_appearance(), BubbleAppearance::default());
        assert_eq!(loaded.observation(), &ObservationPreferences::default());

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn legacy_schema_preserves_existing_values_and_defaults_new_settings() {
        let directory = isolated_preferences_directory();
        let path = directory.join("legacy-preferences.json");
        fs::write(
            &path,
            br#"{"visible":true,"passthrough":true,"scale":0.55,"position":[-12.5,44.0]}"#,
        )
        .expect("legacy preferences should be written");

        let loaded = Preferences::load_legacy_path(&path).expect("legacy preferences should load");
        assert!(loaded.visible());
        assert!(loaded.passthrough());
        assert!(!loaded.alpha_passthrough());
        assert!(loaded.bubble_visible());

        assert_eq!(loaded.language(), LanguagePreference::System);
        assert_eq!(loaded.bubble_placement(), BubblePlacement::Above);
        assert_eq!(loaded.scale(), 0.55);
        assert_eq!(loaded.position(), Some((-12.5, 44.0)));
        assert!(loaded.position_is_legacy());

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_menu_bar_mode_in_current_and_legacy_files_preserves_existing_settings() {
        for legacy_file in [false, true] {
            let directory = isolated_preferences_directory();
            let path = directory.join(PREFERENCES_FILE);
            let legacy = directory.join("legacy-preferences.json");
            let source = if legacy_file { &legacy } else { &path };
            fs::write(
                source,
                br#"{"visible":false,"passthrough":true,"bubble_visible":false,"scale":0.875,"position":[123.25,456.75],"future_setting":{"a":1}}"#,
            )
            .expect("preferences without a menu bar mode should be written");

            let loaded = Preferences::load_in_directory(
                &directory,
                if legacy_file {
                    Some(legacy.as_path())
                } else {
                    None
                },
                false,
            )
            .expect("current or legacy preferences should load");
            assert_eq!(loaded.menu_bar_mode(), MenuBarMode::Always);
            assert!(!loaded.visible());
            assert!(loaded.passthrough());
            assert!(!loaded.bubble_visible());
            assert_eq!(loaded.scale(), 0.875);
            assert_eq!(loaded.position(), Some((123.25, 456.75)));
            assert!(loaded.position_is_legacy());
            assert_eq!(loaded.extra["future_setting"], serde_json::json!({"a": 1}));

            let mut loaded = loaded;
            loaded
                .apply_patch_in_directory(
                    PreferencePatch {
                        menu_bar_mode: Some(MenuBarMode::RecoveryOnly),
                        ..PreferencePatch::default()
                    },
                    &directory,
                )
                .expect("menu bar mode should save with previous settings");
            let disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(disk["menu_bar_mode"], "recovery_only");
            assert_eq!(disk["future_setting"], serde_json::json!({"a": 1}));
            let reloaded = Preferences::load_path(&path).expect("saved preferences should reload");
            assert_eq!(reloaded.menu_bar_mode(), MenuBarMode::RecoveryOnly);
            assert!(!reloaded.visible());
            assert!(reloaded.passthrough());
            assert!(!reloaded.bubble_visible());
            assert_eq!(reloaded.position(), Some((123.25, 456.75)));
            assert!(reloaded.position_is_legacy());
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn missing_and_null_icon_default_without_losing_current_or_legacy_settings() {
        for legacy in [false, true] {
            for field in ["", r#","menu_bar_icon":null"#] {
                let directory = isolated_preferences_directory();
                let current = directory.join(PREFERENCES_FILE);
                let old = directory.join("old.json");
                let source = if legacy { &old } else { &current };
                fs::write(
                    source,
                    format!(r#"{{"visible":false,"passthrough":true,"scale":0.875,"position":null,"future_setting":{{"v":2}}{field}}}"#),
                )
                .unwrap();
                let loaded = Preferences::load_in_directory(
                    &directory,
                    if legacy { Some(&old) } else { None },
                    false,
                )
                .unwrap();
                assert!(loaded.menu_bar_icon().is_none());
                assert!(!loaded.visible());
                assert!(loaded.passthrough());
                assert_eq!(loaded.extra["future_setting"], serde_json::json!({"v": 2}));
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    fn private_png_asset(
        managed: &Path,
        pixel: [u8; 4],
    ) -> (MenuBarIconPreference, PathBuf, Vec<u8>) {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixel)
                .unwrap();
        }
        let preference = MenuBarIconPreference {
            asset: format!("icon-{:x}.png", Sha256::digest(&bytes)),
        };
        let path = managed.join(&preference.asset);
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        (preference, path, bytes)
    }

    fn assert_icon_state(
        preferences: &Preferences,
        path: &Path,
        icon: Option<&MenuBarIconPreference>,
        expected_disk: &Value,
    ) {
        assert_eq!(preferences.menu_bar_icon(), icon);
        assert_eq!(preferences.menu_bar_mode(), MenuBarMode::RecoveryOnly);
        assert_eq!(preferences.language(), LanguagePreference::En);
        assert!(!preferences.visible());
        assert!(preferences.passthrough());
        assert_eq!(
            preferences.extra["future_setting"],
            serde_json::json!({"v": 2})
        );
        let disk: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(&disk, expected_disk);
        let reloaded = Preferences::load_path(path).unwrap();
        assert_eq!(reloaded.menu_bar_icon(), icon);
        assert_eq!(reloaded.menu_bar_mode(), preferences.menu_bar_mode());
        assert_eq!(reloaded.language(), preferences.language());
        assert_eq!(reloaded.visible(), preferences.visible());
        assert_eq!(reloaded.passthrough(), preferences.passthrough());
        assert_eq!(reloaded.extra, preferences.extra);
    }

    #[test]
    fn icon_replacement_and_reset_commit_before_cleaning_previous_asset() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let managed = directory.join("menu-bar-icons");
        fs::create_dir(&managed).unwrap();
        fs::set_permissions(&managed, fs::Permissions::from_mode(0o700)).unwrap();
        let (previous, old_path, old_bytes) = private_png_asset(&managed, [200, 20, 30, 255]);
        let (replacement, new_path, new_bytes) = private_png_asset(&managed, [20, 140, 50, 255]);
        let (unrelated, unrelated_path, unrelated_bytes) =
            private_png_asset(&managed, [40, 60, 220, 255]);
        let mut prefs = Preferences::default();
        prefs.set_visible(false);
        prefs.set_passthrough(true);
        prefs
            .extra
            .insert("future_setting".to_owned(), serde_json::json!({"v": 2}));
        prefs
            .save_menu_bar_icon_in_directory(Some(previous.clone()), &directory)
            .unwrap();
        prefs
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    menu_bar_mode: Some(MenuBarMode::RecoveryOnly),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .unwrap();
        let mut restarted = Preferences::load_path(&path).unwrap();
        let previous_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_icon_state(&restarted, &path, Some(&previous), &previous_disk);

        // Block the actual preferences.json rename in the same root, after publishing
        // the candidate asset. Moving the previous file aside preserves its exact bytes.
        let backup = directory.join("preferences.backup");
        let previous_bytes = fs::read(&path).unwrap();
        fs::rename(&path, &backup).unwrap();
        fs::create_dir(&path).unwrap();
        restarted
            .save_menu_bar_icon_in_directory(Some(replacement.clone()), &directory)
            .unwrap_err();
        assert_eq!(restarted.menu_bar_icon(), Some(&previous));
        assert_eq!(fs::read(&backup).unwrap(), previous_bytes);
        assert_eq!(fs::read(&old_path).unwrap(), old_bytes);
        assert_eq!(fs::read(&new_path).unwrap(), new_bytes);
        fs::remove_dir(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        assert_icon_state(&restarted, &path, Some(&previous), &previous_disk);

        fs::rename(&path, &backup).unwrap();
        fs::create_dir(&path).unwrap();
        restarted
            .save_menu_bar_icon_in_directory(None, &directory)
            .unwrap_err();
        assert_eq!(restarted.menu_bar_icon(), Some(&previous));
        assert_eq!(fs::read(&backup).unwrap(), previous_bytes);
        assert_eq!(fs::read(&old_path).unwrap(), old_bytes);
        assert_eq!(fs::read(&new_path).unwrap(), new_bytes);
        fs::remove_dir(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
        assert_icon_state(&restarted, &path, Some(&previous), &previous_disk);

        restarted
            .save_menu_bar_icon_in_directory(Some(replacement.clone()), &directory)
            .unwrap();
        let mut replacement_disk = previous_disk.clone();
        replacement_disk["menu_bar_icon"] = serde_json::json!({"asset": replacement.asset});
        assert_icon_state(&restarted, &path, Some(&replacement), &replacement_disk);
        assert!(!old_path.exists());
        assert_eq!(fs::read(&new_path).unwrap(), new_bytes);
        assert_eq!(fs::read(&unrelated_path).unwrap(), unrelated_bytes);

        restarted
            .save_menu_bar_icon_in_directory(None, &directory)
            .unwrap();
        let mut reset_disk = replacement_disk.clone();
        reset_disk.as_object_mut().unwrap().remove("menu_bar_icon");
        assert_icon_state(&restarted, &path, None, &reset_disk);
        assert!(!new_path.exists());
        assert_eq!(fs::read(&unrelated_path).unwrap(), unrelated_bytes);

        // A previous asset that is not private is not ours to unlink, even if
        // its name has the right digest and the preferences reset commits.
        restarted
            .save_menu_bar_icon_in_directory(Some(unrelated.clone()), &directory)
            .unwrap();
        fs::set_permissions(&unrelated_path, fs::Permissions::from_mode(0o644)).unwrap();
        restarted
            .save_menu_bar_icon_in_directory(None, &directory)
            .unwrap();
        assert_icon_state(&restarted, &path, None, &reset_disk);
        assert_eq!(fs::read(&unrelated_path).unwrap(), unrelated_bytes);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn menu_bar_mode_survives_language_appearance_and_geometry_saves() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        assert_eq!(preferences.menu_bar_mode(), MenuBarMode::Always);
        preferences.set_position(Some((42.0, 64.0)));
        preferences.set_standalone_bubble_position(Some((-1250.25, -310.75)));
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    menu_bar_mode: Some(MenuBarMode::RecoveryOnly),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("conditional mode should save");
        let mut reloaded = Preferences::load_path(&path).expect("mode should reload");
        assert_eq!(reloaded.menu_bar_mode(), MenuBarMode::RecoveryOnly);
        reloaded
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    bubble_appearance: Some(BubbleAppearance {
                        theme: BubbleTheme::DustyRose,
                        custom: BubbleAppearance::default().custom,
                    }),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("language and appearance should save");
        reloaded.set_position(None);
        reloaded
            .save_in_directory(&directory)
            .expect("position reset should save");

        let restarted = Preferences::load_path(&path).expect("other saves should reload");
        assert_eq!(restarted.menu_bar_mode(), MenuBarMode::RecoveryOnly);
        assert_eq!(restarted.language(), LanguagePreference::En);
        assert_eq!(restarted.bubble_appearance().theme, BubbleTheme::DustyRose);
        assert_eq!(restarted.position(), None);
        assert_eq!(
            restarted.standalone_bubble_position(),
            Some((-1250.25, -310.75))
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn missing_status_indicator_field_migrates_to_enabled_but_explicit_false_survives_saves() {
        let directory = isolated_preferences_directory();
        let legacy = directory.join("legacy-preferences.json");
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &legacy,
            br#"{"visible":false,"passthrough":true,"scale":0.8,"position":[10.0,20.0]}"#,
        )
        .expect("legacy settings should be written");

        let mut preferences =
            Preferences::load_legacy_path(&legacy).expect("missing indicator setting should load");
        assert!(preferences.show_status_indicators());
        preferences
            .save_migrated_in_directory(&directory)
            .expect("legacy settings should migrate");
        assert!(Preferences::load_path(&path)
            .expect("migrated settings should load")
            .show_status_indicators());

        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    show_status_indicators: Some(false),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("disabled indicators should save");
        assert!(!preferences.show_status_indicators());
        let mut reloaded =
            Preferences::load_path(&path).expect("disabled indicators should reload");
        assert!(!reloaded.show_status_indicators());
        reloaded
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("unrelated preference should save");
        let restarted = Preferences::load_path(&path).expect("settings should reload after save");
        assert!(!restarted.show_status_indicators());
        assert_eq!(restarted.language(), LanguagePreference::En);
        assert!(!restarted.visible());
        assert!(restarted.passthrough());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn pre_ui_saves_keep_legacy_position_until_explicit_migration() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":true,"passthrough":false,"scale":0.8,"position":[42.0,64.0]}"#,
        )
        .expect("legacy geometry should be written");
        let mut preferences = Preferences::load_path(&path).expect("legacy geometry should load");
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    bubble_appearance: Some(BubbleAppearance::default()),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("settings should save before UI migration");
        let mut reloaded = Preferences::load_path(&path).expect("pre-UI save should reload");
        assert!(reloaded.position_is_legacy());
        assert_eq!(reloaded.position(), Some((42.0, 64.0)));
        reloaded.set_position(Some((51.0, 70.0)));
        reloaded
            .save_in_directory(&directory)
            .expect("migrated position should save");
        let reloaded = Preferences::load_path(&path).expect("migrated position should reload");
        assert!(!reloaded.position_is_legacy());
        assert_eq!(reloaded.position(), Some((51.0, 70.0)));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn migration_marker_persists_without_a_position() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":true,"passthrough":false,"scale":0.8,"position":null}"#,
        )
        .expect("legacy preferences should be written");
        let mut preferences =
            Preferences::load_path(&path).expect("legacy preferences should load");
        assert!(preferences.position_is_legacy());
        preferences.mark_display_position();
        preferences
            .save_in_directory(&directory)
            .expect("migration should save");
        let reloaded = Preferences::load_path(&path).expect("migration should reload");
        assert!(!reloaded.position_is_legacy());
        assert_eq!(reloaded.position(), None);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn new_bubble_settings_persist_with_alpha_mode() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.set_alpha_passthrough(true);
        preferences.set_bubble_visible(false);
        preferences.set_bubble_placement(BubblePlacement::Right);
        preferences
            .save_in_directory(&directory)
            .expect("new preferences should save");

        let loaded = Preferences::load_path(&path).expect("new preferences should load");
        assert!(loaded.alpha_passthrough());
        assert!(!loaded.bubble_visible());
        assert_eq!(loaded.bubble_placement(), BubblePlacement::Right);

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_bubble_placement_keeps_existing_invalid_preferences_behavior() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":true,"passthrough":false,"bubble_placement":"diagonal","scale":0.65,"position":null}"#,
        )
        .expect("invalid preferences should be written");

        let error = Preferences::load_path(&path).expect_err("invalid placement should fail");
        assert!(error.starts_with("preferences are invalid:"));

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn migration_does_not_clobber_existing_preferences() {
        let directory = isolated_preferences_directory();
        let existing = directory.join(PREFERENCES_FILE);
        let mut current = Preferences::default();
        current.set_scale(0.8);
        current.language = LanguagePreference::Ko;
        current
            .save_in_directory(&directory)
            .expect("current preferences should save");

        let mut migrated = Preferences::default();
        migrated.set_scale(1.1);
        migrated.language = LanguagePreference::En;
        assert!(!migrated
            .save_migrated_in_directory(&directory)
            .expect("migration should complete without clobbering"));
        let loaded = Preferences::load_path(&existing).expect("current preferences should load");
        assert_eq!(loaded.scale(), 0.8);
        assert_eq!(loaded.language(), LanguagePreference::Ko);

        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn invalid_language_defaults_without_resetting_other_settings() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":0.875,"position":[123.25,456.75],"language":"ja"}"#,
        )
        .expect("preferences with invalid language should be written");

        let loaded = Preferences::load_path(&path).expect("preferences should load");
        assert_eq!(loaded.language(), LanguagePreference::System);
        assert!(!loaded.visible());
        assert!(loaded.passthrough());
        assert_eq!(loaded.scale(), 0.875);
        assert_eq!(loaded.position(), Some((123.25, 456.75)));

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn language_patch_roundtrips_canonical_token() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    language: Some(LanguagePreference::En),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("language preference should save");

        let bytes = fs::read(&path).expect("saved preferences should be readable");
        let disk: serde_json::Value =
            serde_json::from_slice(&bytes).expect("saved preferences should be valid JSON");
        assert_eq!(
            disk.get("language").and_then(serde_json::Value::as_str),
            Some("en")
        );

        let loaded = Preferences::load_path(&path).expect("preferences should load");
        assert_eq!(loaded.language(), LanguagePreference::En);
        assert_eq!(preferences.language(), LanguagePreference::En);

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn custom_palette_survives_preset_switch_and_reload() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        let mut custom = preferences.bubble_appearance().custom;
        custom.surface = BubbleColor::parse_hex("#127fA0").unwrap();
        custom.text = BubbleColor::parse_hex("#ffffff").unwrap();
        let appearance = BubbleAppearance {
            theme: BubbleTheme::Custom,
            custom,
        };
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    bubble_appearance: Some(appearance),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("custom appearance should save");
        assert_eq!(preferences.bubble_appearance().palette(), custom);
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    bubble_appearance: Some(BubbleAppearance {
                        theme: BubbleTheme::MoonlitInk,
                        custom,
                    }),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("preset should save without discarding custom colors");

        let loaded = Preferences::load_path(&path).expect("appearance should load");
        assert_eq!(loaded.bubble_appearance().theme, BubbleTheme::MoonlitInk);
        assert_eq!(loaded.bubble_appearance().custom, custom);
        assert_eq!(
            loaded.bubble_appearance().custom.surface.to_hex(),
            "#127FA0"
        );
        assert_ne!(loaded.bubble_appearance().palette(), custom);
        assert_eq!(
            BubbleAppearance {
                theme: BubbleTheme::Custom,
                custom: loaded.bubble_appearance().custom,
            }
            .palette(),
            custom
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn rejects_malformed_colors_in_input_and_on_disk() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let valid = Preferences::default();
        let mut disk = serde_json::to_value(DiskPreferences {
            visible: valid.visible,
            passthrough: valid.passthrough,
            alpha_passthrough: valid.alpha_passthrough,
            bubble_visible: valid.bubble_visible,
            show_status_indicators: valid.show_status_indicators,
            auto_update_check: valid.auto_update_check,
            last_update_check: valid.last_update_check,
            menu_bar_mode: valid.menu_bar_mode,
            menu_bar_icon: valid.menu_bar_icon.clone(),
            bubble_placement: valid.bubble_placement,
            scale: valid.scale,
            position: None,
            standalone_bubble_position: None,
            bubble_sizes: None,
            language: valid.language,
            position_space: valid.position_space,
            bubble_appearance: valid.bubble_appearance,
            observation: valid.observation.clone(),
            dialogue_overrides: DialogueOverrides::default(),
            session_list: valid.session_list.to_disk(Map::new()),
            extra: Map::new(),
        })
        .unwrap();
        for invalid in [
            "#12345",
            "#1234567",
            "123456",
            "#12345G",
            "#12 456",
            "#１２３４５６",
        ] {
            assert!(BubbleColor::parse_hex(invalid).is_err());
            disk["bubble_appearance"]["custom"]["surface"] = invalid.into();
            fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
            assert!(Preferences::load_path(&path).is_err());
        }
        disk["bubble_appearance"]["custom"]["surface"] = serde_json::json!(1e300);
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
        assert!(Preferences::load_path(&path).is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn saving_observation_migrates_old_settings_without_losing_unrelated_values() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":0.875,"position":[123.25,456.75],"language":"ko"}"#,
        )
        .expect("old preferences should be written");
        let mut preferences = Preferences::load_path(&path).expect("old preferences should load");
        let machine = "opaque machine:🍃/A".to_owned();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    observation_local: Some(false),
                    observation_remote: Some(true),
                    observation_machines: Some(vec![machine.clone()]),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("observation should save");
        let loaded = Preferences::load_path(&path).expect("migrated preferences should reload");
        assert_eq!(loaded.observation(), preferences.observation());
        assert_eq!(loaded.observation().machines, vec![machine]);
        assert!(!loaded.visible());
        assert!(loaded.passthrough());
        assert_eq!(loaded.scale(), 0.875);
        assert_eq!(loaded.position(), Some((123.25, 456.75)));
        assert!(loaded.position_is_legacy());
        assert_eq!(loaded.language(), LanguagePreference::Ko);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn disabled_remote_selection_survives_patch_and_reload() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        let id = "opaque id:/🌲".to_owned();
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    observation_local: Some(true),
                    observation_remote: Some(false),
                    observation_machines: Some(vec![id.clone()]),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("selection should save");
        let loaded = Preferences::load_path(&path).expect("selection should reload");
        assert_eq!(loaded.observation().machines, vec![id.clone()]);
        assert!(!loaded.observation().remote);
        assert!(!loaded
            .observation()
            .includes(&crate::sources::remote_source(&id)));
        preferences
            .apply_patch_in_directory(
                PreferencePatch {
                    observation_remote: Some(true),
                    ..PreferencePatch::default()
                },
                &directory,
            )
            .expect("reenabling remote should save");
        let reloaded = Preferences::load_path(&path).expect("enabled selection should reload");
        assert!(reloaded
            .observation()
            .includes(&crate::sources::remote_source(&id)));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn dialogue_roundtrip_reset_and_namespaces_preserve_other_settings() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":0.875,"position":[123.25,456.75]}"#,
        )
        .expect("old settings");
        let mut preferences = Preferences::load_path(&path).expect("old settings load");
        let builtin = DialogueTarget::Character("default".to_string());
        let pack = DialogueTarget::Character("forest".to_string());
        let external = DialogueTarget::ExternalAssets("/tmp/default".to_string());
        for target in [&builtin, &pack, &external] {
            preferences
                .save_dialogue_entry_in_directory(
                    target,
                    "ko",
                    DialogueSlot::Idle,
                    Some("첫 줄\n둘째 줄".to_string()),
                    &directory,
                )
                .expect("dialogue save");
        }
        preferences
            .save_dialogue_entry_in_directory(
                &pack,
                "en",
                DialogueSlot::Pet,
                Some("Pet response".to_string()),
                &directory,
            )
            .expect("second locale saves");
        let loaded = Preferences::load_path(&path).expect("dialogue reload");
        assert_eq!(
            loaded
                .dialogue_overrides()
                .entry(&pack, "ko", DialogueSlot::Idle),
            Some("첫 줄\n둘째 줄")
        );
        assert_eq!(
            loaded
                .dialogue_overrides()
                .entry(&external, "ko", DialogueSlot::Idle),
            Some("첫 줄\n둘째 줄")
        );
        assert!(!loaded.visible());
        assert!(loaded.passthrough());
        assert!(loaded.position_is_legacy());
        assert_eq!(loaded.position(), Some((123.25, 456.75)));
        let mut loaded = loaded;
        loaded
            .reset_character_dialogue_in_directory(&pack, &directory)
            .expect("pack reset");
        assert!(loaded.dialogue_overrides().locales(&pack).is_none());
        assert!(loaded.dialogue_overrides().locales(&builtin).is_some());
        assert!(loaded.dialogue_overrides().locales(&external).is_some());
        loaded
            .save_dialogue_entry_in_directory(
                &external,
                "ko",
                DialogueSlot::Idle,
                Some(" \n\t".to_string()),
                &directory,
            )
            .expect("blank save removes selected entry");
        assert!(loaded.dialogue_overrides().locales(&external).is_none());
        let reloaded = Preferences::load_path(&path).expect("reset persists");
        assert!(reloaded.dialogue_overrides().locales(&pack).is_none());
        assert!(reloaded.dialogue_overrides().locales(&external).is_none());
        assert_eq!(
            reloaded
                .dialogue_overrides()
                .entry(&builtin, "ko", DialogueSlot::Idle),
            Some("첫 줄\n둘째 줄")
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_dialogue_save_and_reset_preserve_memory_and_disk() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let target = DialogueTarget::Character("forest".to_string());
        let mut preferences = Preferences::default();
        preferences
            .save_dialogue_entry_in_directory(
                &target,
                "ko",
                DialogueSlot::Idle,
                Some("original".to_string()),
                &directory,
            )
            .expect("initial save");
        let before = fs::read(&path).expect("saved settings");
        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").expect("blocking file");
        preferences
            .save_dialogue_entry_in_directory(
                &target,
                "ko",
                DialogueSlot::Idle,
                Some("changed".to_string()),
                &blocked,
            )
            .expect_err("save must fail");
        preferences
            .reset_character_dialogue_in_directory(&target, &blocked)
            .expect_err("reset must fail");
        assert_eq!(
            preferences
                .dialogue_overrides()
                .entry(&target, "ko", DialogueSlot::Idle),
            Some("original")
        );
        assert_eq!(fs::read(&path).expect("settings intact"), before);
        assert_eq!(fs::read(&blocked).expect("blocker intact"), b"sentinel");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_dialogue_payload_and_inputs_leave_settings_intact() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let target = DialogueTarget::Character("forest".to_string());
        let mut preferences = Preferences::default();
        preferences
            .save_in_directory(&directory)
            .expect("initial save");
        let initial = fs::read(&path).expect("initial disk");
        for (target, locale, text) in [
            (
                DialogueTarget::Character("../forest".to_string()),
                "ko",
                "valid".to_string(),
            ),
            (target.clone(), "fr", "valid".to_string()),
            (target.clone(), "en", "a".repeat(2049)),
            (target.clone(), "en", "bad\u{0000}text".to_string()),
            (
                DialogueTarget::ExternalAssets("relative/path".to_string()),
                "ko",
                "valid".to_string(),
            ),
        ] {
            preferences
                .save_dialogue_entry_in_directory(
                    &target,
                    locale,
                    DialogueSlot::Idle,
                    Some(text),
                    &directory,
                )
                .expect_err("invalid entry must fail");
        }
        assert!(preferences.dialogue_overrides().locales(&target).is_none());
        assert_eq!(fs::read(&path).expect("disk unchanged"), initial);
        for invalid in [
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"characters":{"forest":{"fr":{"phases":{"idle":"wrong locale"}}}}}}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"characters":{"forest":{"ko":{"reactions":{"idle":"wrong key"}}}}}}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"external_assets":{"relative/path":{"ko":{"phases":{"idle":"wrong path"}}}}}}"#,
            r#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"characters":{"forest":{"ko":{"phases":{"idle":""}}}}}}"#,
        ] {
            fs::write(&path, invalid).expect("invalid disk fixture");
            assert!(Preferences::load_path(&path).is_err());
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unknown_top_level_field_survives_save() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br#"{"visible":false,"passthrough":true,"scale":0.875,"position":[12.5,34.5],"language":"ko","future_setting":{"a":1}}"#,
        )
        .expect("preferences from a newer build should be written");

        let mut preferences =
            Preferences::load_path(&path).expect("unknown top-level fields should load");
        preferences.set_bubble_visible(false);
        preferences.set_standalone_bubble_position(Some((-500.5, 300.25)));
        preferences
            .save_in_directory(&directory)
            .expect("preferences should save");

        let raw: Value =
            serde_json::from_slice(&fs::read(&path).expect("saved preferences should be read"))
                .expect("saved preferences should be JSON");
        assert_eq!(raw["future_setting"], serde_json::json!({"a": 1}));
        assert_eq!(raw["visible"], Value::Bool(false));
        assert_eq!(raw["passthrough"], Value::Bool(true));
        assert_eq!(raw["bubble_visible"], Value::Bool(false));
        assert_eq!(raw["scale"], serde_json::json!(0.875));
        assert_eq!(raw["position"], serde_json::json!([12.5, 34.5]));
        assert_eq!(
            raw["standalone_bubble_position"],
            serde_json::json!([-500.5, 300.25])
        );
        assert_eq!(raw["language"], serde_json::json!("ko"));
        let reloaded = Preferences::load_path(&path).expect("saved preferences should reload");
        assert!(!reloaded.bubble_visible());
        assert_eq!(
            reloaded.standalone_bubble_position(),
            Some((-500.5, 300.25))
        );
        assert_eq!(reloaded.language(), LanguagePreference::Ko);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unknown_nested_fields_are_tolerated() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        fs::write(
            &path,
            br##"{"visible":true,"passthrough":false,"scale":0.875,"position":null,
            "bubble_appearance":{"theme":"custom","shadow":true,"custom":{"surface":"#112233","text":"#445566","muted":"#778899","border":"#AABBCC","accent":"#DDEEFF","glow":"#000000"}},
            "observation":{"local":false,"remote":true,"machines":["studio"],"poll_ms":5}}"##,
        )
        .expect("preferences with nested future fields should be written");

        let loaded = Preferences::load_path(&path).expect("unknown nested fields should load");
        let appearance = loaded.bubble_appearance();
        assert_eq!(appearance.theme, BubbleTheme::Custom);
        assert_eq!(
            appearance.custom,
            BubblePalette {
                surface: BubbleColor::parse_hex("#112233").unwrap(),
                text: BubbleColor::parse_hex("#445566").unwrap(),
                muted: BubbleColor::parse_hex("#778899").unwrap(),
                border: BubbleColor::parse_hex("#AABBCC").unwrap(),
                accent: BubbleColor::parse_hex("#DDEEFF").unwrap(),
            }
        );
        assert_eq!(
            loaded.observation(),
            &ObservationPreferences {
                local: false,
                remote: true,
                machines: vec!["studio".to_owned()],
            }
        );
        assert_eq!(loaded.scale(), 0.875);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unusable_preferences_are_quarantined_only_for_the_daemon() {
        let fixtures: [&[u8]; 10] = [
            b"{not json",
            br#"{"visible":true,"passthrough":false,"bubble_placement":"diagonal","scale":0.65,"position":null}"#,
            br#"{"visible":true,"passthrough":false,"menu_bar_mode":"sometimes","scale":0.65,"position":null}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"characters":{"forest":{"fr":{"phases":{"idle":"wrong locale"}}}}}}"#,
            br#"{"visible":"false","passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":287,"height":134}}}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":true}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":"bad"}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":1e400,"height":134}}}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":NaN,"height":134}}}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"bubble_sizes":{"compact":{"width":Infinity,"height":134}}}"#,
        ];
        for invalid in fixtures {
            let directory = isolated_preferences_directory();
            let path = directory.join(PREFERENCES_FILE);
            fs::write(&path, invalid).expect("unusable preferences should be written");
            let backups = || {
                fs::read_dir(&directory)
                    .expect("preferences directory should be listed")
                    .map(|entry| entry.expect("directory entry should be read").path())
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with("preferences.json.invalid-"))
                    })
                    .collect::<Vec<_>>()
            };

            Preferences::load_in_directory(&directory, None, false)
                .expect_err("strict load must report unusable preferences");
            assert_eq!(
                fs::read(&path).expect("strict load keeps the file"),
                invalid
            );
            assert!(backups().is_empty());

            let loaded = Preferences::load_in_directory(&directory, None, true)
                .expect("daemon load should fall back to defaults");
            let defaults = Preferences::default();
            assert_eq!(loaded.visible(), defaults.visible());
            assert_eq!(loaded.passthrough(), defaults.passthrough());
            assert_eq!(loaded.menu_bar_mode(), defaults.menu_bar_mode());
            assert_eq!(loaded.bubble_placement(), defaults.bubble_placement());
            assert_eq!(loaded.scale(), defaults.scale());
            assert_eq!(loaded.position(), defaults.position());
            assert_eq!(loaded.language(), defaults.language());
            assert_eq!(loaded.bubble_appearance(), defaults.bubble_appearance());
            assert_eq!(loaded.observation(), defaults.observation());
            assert_eq!(loaded.dialogue_overrides(), defaults.dialogue_overrides());
            assert!(loaded.extra.is_empty());
            assert!(!path.exists());
            let backup = match backups().as_slice() {
                [backup] => backup.clone(),
                other => panic!("expected exactly one backup, found {other:?}"),
            };
            assert_eq!(fs::read(&backup).expect("backup should be read"), invalid);

            loaded
                .save_in_directory(&directory)
                .expect("defaults should save over the quarantined path");
            Preferences::load_path(&path).expect("fresh preferences should load");
            assert_eq!(backups(), vec![backup.clone()]);
            assert_eq!(fs::read(&backup).expect("backup stays intact"), invalid);
            let _ = fs::remove_dir_all(directory);
        }
    }
}
