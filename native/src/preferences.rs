use crate::bubble::BubblePlacement;
use crate::dialogue::{DialogueOverrides, DialogueSlot, DialogueTarget};
use crate::i18n::LanguagePreference;
use crate::lifecycle::config_directory;
use crate::sources::ObservationPreferences;
use crate::state::{normalize_scale, DEFAULT_SCALE};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
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

#[derive(Clone, Debug)]
pub struct Preferences {
    visible: bool,
    passthrough: bool,
    alpha_passthrough: bool,
    bubble_visible: bool,
    show_status_indicators: bool,
    menu_bar_mode: MenuBarMode,
    bubble_placement: BubblePlacement,
    scale: f64,
    position: Option<(f64, f64)>,
    standalone_bubble_position: Option<(f64, f64)>,
    position_space: PositionSpace,
    language: LanguagePreference,
    bubble_appearance: BubbleAppearance,
    observation: ObservationPreferences,
    dialogue_overrides: DialogueOverrides,
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
    #[serde(default)]
    menu_bar_mode: MenuBarMode,
    #[serde(default)]
    bubble_placement: BubblePlacement,
    scale: f64,
    position: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    standalone_bubble_position: Option<[f64; 2]>,
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
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn default_bubble_visible() -> bool {
    true
}

fn default_show_status_indicators() -> bool {
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
            menu_bar_mode: MenuBarMode::default(),
            bubble_placement: BubblePlacement::default(),
            scale: DEFAULT_SCALE,
            position: None,
            standalone_bubble_position: None,
            position_space: PositionSpace::Display,
            language: LanguagePreference::default(),
            bubble_appearance: BubbleAppearance::default(),
            observation: ObservationPreferences::default(),
            dialogue_overrides: DialogueOverrides::default(),
            extra: Map::new(),
        }
    }
}
impl Preferences {
    /// Strict load for read-only consumers: an unusable file is reported as an
    /// error and left untouched.
    pub fn load() -> Result<Self, String> {
        let directory = preferences_directory()?;
        Self::load_in_directory(&directory, legacy_preferences_path().as_deref(), false)
    }

    /// Daemon startup load: an unusable file is moved aside to
    /// `preferences.json.invalid-<unix_nanos>` and defaults are used, so the
    /// daemon neither refuses to start nor later overwrites the original bytes.
    /// Fails only when the file cannot be moved aside.
    pub(crate) fn load_for_daemon() -> Result<Self, String> {
        let directory = preferences_directory()?;
        Self::load_in_directory(&directory, legacy_preferences_path().as_deref(), true)
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
    fn load_path(path: &Path) -> Result<Self, String> {
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
        let mut preferences = Self {
            visible: disk.visible,
            passthrough: disk.passthrough,
            alpha_passthrough: disk.alpha_passthrough,
            bubble_visible: disk.bubble_visible,
            show_status_indicators: disk.show_status_indicators,
            menu_bar_mode: disk.menu_bar_mode,
            bubble_placement: disk.bubble_placement,
            scale: disk.scale,
            position: disk.position.map(|value| (value[0], value[1])),
            standalone_bubble_position: disk
                .standalone_bubble_position
                .map(|value| (value[0], value[1])),
            position_space: disk.position_space,
            language: disk.language,
            bubble_appearance: disk.bubble_appearance,
            observation: disk.observation,
            dialogue_overrides: disk.dialogue_overrides,
            extra: disk.extra,
        };
        preferences.sanitize();
        Ok(Some(preferences))
    }

    fn load_legacy_path(path: &Path) -> Option<Self> {
        Self::load_existing_path(path).ok().flatten()
    }

    pub fn save(&self) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_in_directory(&directory)
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
    ) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_dialogue_entry_in_directory(target, locale, slot, value, &directory)
    }

    fn save_dialogue_entry_in_directory(
        &mut self,
        target: &DialogueTarget,
        locale: &str,
        slot: DialogueSlot,
        value: Option<String>,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate
            .dialogue_overrides
            .set_entry(target, locale, slot, value)?;
        candidate.save_in_directory(directory)?;
        self.dialogue_overrides = candidate.dialogue_overrides;
        Ok(())
    }

    pub(crate) fn reset_character_dialogue(
        &mut self,
        target: &DialogueTarget,
    ) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.reset_character_dialogue_in_directory(target, &directory)
    }

    fn reset_character_dialogue_in_directory(
        &mut self,
        target: &DialogueTarget,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.dialogue_overrides.remove_target(target)?;
        candidate.save_in_directory(directory)?;
        self.dialogue_overrides = candidate.dialogue_overrides;
        Ok(())
    }

    pub fn save_language(&mut self, preference: LanguagePreference) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_language_in_directory(preference, &directory)
    }

    pub fn save_bubble_appearance(&mut self, appearance: BubbleAppearance) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_bubble_appearance_in_directory(appearance, &directory)
    }

    pub fn save_show_status_indicators(&mut self, enabled: bool) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_show_status_indicators_in_directory(enabled, &directory)
    }

    pub(crate) fn save_menu_bar_mode(&mut self, mode: MenuBarMode) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_menu_bar_mode_in_directory(mode, &directory)
    }

    pub(crate) fn observation(&self) -> &ObservationPreferences {
        &self.observation
    }

    pub(crate) fn save_observation(
        &mut self,
        preference: ObservationPreferences,
    ) -> Result<(), String> {
        let directory = preferences_directory()?;
        self.save_observation_in_directory(preference, &directory)
    }

    fn save_observation_in_directory(
        &mut self,
        mut preference: ObservationPreferences,
        directory: &Path,
    ) -> Result<(), String> {
        preference.sanitize();
        let mut candidate = self.clone();
        candidate.observation = preference;
        candidate.save_in_directory(directory)?;
        self.observation = candidate.observation;
        Ok(())
    }

    fn save_menu_bar_mode_in_directory(
        &mut self,
        mode: MenuBarMode,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.menu_bar_mode = mode;
        candidate.save_in_directory(directory)?;
        self.menu_bar_mode = mode;
        Ok(())
    }

    fn save_in_directory(&self, directory: &Path) -> Result<(), String> {
        self.write_atomically(directory, false).map(|_| ())
    }

    fn save_language_in_directory(
        &mut self,
        preference: LanguagePreference,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.language = preference;
        candidate.save_in_directory(directory)?;
        self.language = preference;
        Ok(())
    }

    fn save_bubble_appearance_in_directory(
        &mut self,
        appearance: BubbleAppearance,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.bubble_appearance = appearance;
        candidate.save_in_directory(directory)?;
        self.bubble_appearance = appearance;
        Ok(())
    }

    fn save_show_status_indicators_in_directory(
        &mut self,
        enabled: bool,
        directory: &Path,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.show_status_indicators = enabled;
        candidate.save_in_directory(directory)?;
        self.show_status_indicators = enabled;
        Ok(())
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
            menu_bar_mode: self.menu_bar_mode,
            bubble_placement: self.bubble_placement,
            scale: self.scale,
            position: self.position.map(|(x, y)| [x, y]),
            standalone_bubble_position: self.standalone_bubble_position.map(|(x, y)| [x, y]),
            position_space: self.position_space,
            language: self.language,
            bubble_appearance: self.bubble_appearance,
            observation: self.observation.clone(),
            dialogue_overrides: self.dialogue_overrides.clone(),
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

    pub(crate) fn menu_bar_mode(&self) -> MenuBarMode {
        self.menu_bar_mode
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

    fn sanitize(&mut self) {
        self.scale = normalize_scale(self.scale).unwrap_or(DEFAULT_SCALE);
        self.position = self
            .position
            .filter(|(x, y)| x.is_finite() && y.is_finite() && x.abs() < 1.0e7 && y.abs() < 1.0e7);
        self.standalone_bubble_position = self
            .standalone_bubble_position
            .filter(|(x, y)| x.is_finite() && y.is_finite());
        self.observation.sanitize();
    }
}

fn preferences_directory() -> Result<PathBuf, String> {
    config_directory()
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
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn sanitizes_invalid_scale_and_position() {
        let mut preferences = Preferences {
            visible: true,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            show_status_indicators: true,
            menu_bar_mode: MenuBarMode::default(),
            bubble_placement: BubblePlacement::Above,
            scale: f64::NAN,
            position: Some((f64::INFINITY, 2.0)),
            standalone_bubble_position: Some((f64::NAN, 3.0)),
            position_space: PositionSpace::LegacyCanvas,
            language: LanguagePreference::default(),
            bubble_appearance: BubbleAppearance::default(),
            observation: ObservationPreferences::default(),
            dialogue_overrides: DialogueOverrides::default(),
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
                .save_menu_bar_mode_in_directory(MenuBarMode::RecoveryOnly, &directory)
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
    fn menu_bar_mode_survives_language_appearance_and_geometry_saves() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        assert_eq!(preferences.menu_bar_mode(), MenuBarMode::Always);
        preferences.set_position(Some((42.0, 64.0)));
        preferences.set_standalone_bubble_position(Some((-1250.25, -310.75)));
        preferences
            .save_menu_bar_mode_in_directory(MenuBarMode::RecoveryOnly, &directory)
            .expect("conditional mode should save");
        let mut reloaded = Preferences::load_path(&path).expect("mode should reload");
        assert_eq!(reloaded.menu_bar_mode(), MenuBarMode::RecoveryOnly);
        reloaded
            .save_language_in_directory(LanguagePreference::En, &directory)
            .expect("language should save");
        reloaded
            .save_bubble_appearance_in_directory(
                BubbleAppearance {
                    theme: BubbleTheme::DustyRose,
                    custom: BubbleAppearance::default().custom,
                },
                &directory,
            )
            .expect("appearance should save");
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
    fn failed_menu_bar_mode_save_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences
            .save_menu_bar_mode_in_directory(MenuBarMode::RecoveryOnly, &directory)
            .expect("initial mode should save");
        let before = fs::read(&path).expect("initial settings should be readable");
        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").expect("blocking file should be written");

        preferences
            .save_menu_bar_mode_in_directory(MenuBarMode::Always, &blocked)
            .expect_err("save through a file path should fail");
        assert_eq!(preferences.menu_bar_mode(), MenuBarMode::RecoveryOnly);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&blocked).unwrap(), b"sentinel");
        assert_eq!(
            Preferences::load_path(&path).unwrap().menu_bar_mode(),
            MenuBarMode::RecoveryOnly
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
            .save_show_status_indicators_in_directory(false, &directory)
            .expect("disabled indicators should save");
        assert!(!preferences.show_status_indicators());
        let mut reloaded =
            Preferences::load_path(&path).expect("disabled indicators should reload");
        assert!(!reloaded.show_status_indicators());
        reloaded
            .save_language_in_directory(LanguagePreference::En, &directory)
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
            .save_language_in_directory(LanguagePreference::En, &directory)
            .expect("language should save before UI migration");
        preferences
            .save_bubble_appearance_in_directory(BubbleAppearance::default(), &directory)
            .expect("appearance should save before UI migration");
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
    fn language_save_roundtrips_canonical_token() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences
            .save_language_in_directory(LanguagePreference::En, &directory)
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
    fn failed_language_save_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.language = LanguagePreference::Ko;
        preferences
            .save_in_directory(&directory)
            .expect("initial preferences should save");
        let before = fs::read(&path).expect("initial preferences should be readable");

        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").expect("blocking file should be written");
        preferences
            .save_language_in_directory(LanguagePreference::En, &blocked)
            .expect_err("language save should fail for a file path");
        assert_eq!(preferences.language(), LanguagePreference::Ko);
        assert_eq!(
            fs::read(&path).expect("preferences should remain readable"),
            before
        );
        assert_eq!(
            fs::read(&blocked).expect("blocking file should remain readable"),
            b"sentinel"
        );

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
            .save_bubble_appearance_in_directory(appearance, &directory)
            .expect("custom appearance should save");
        assert_eq!(preferences.bubble_appearance().palette(), custom);
        preferences
            .save_bubble_appearance_in_directory(
                BubbleAppearance {
                    theme: BubbleTheme::MoonlitInk,
                    custom,
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
            menu_bar_mode: valid.menu_bar_mode,
            bubble_placement: valid.bubble_placement,
            scale: valid.scale,
            position: None,
            standalone_bubble_position: None,
            language: valid.language,
            position_space: valid.position_space,
            bubble_appearance: valid.bubble_appearance,
            observation: valid.observation.clone(),
            dialogue_overrides: DialogueOverrides::default(),
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
    fn failed_appearance_save_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences.save_in_directory(&directory).unwrap();
        let before = fs::read(&path).unwrap();
        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").unwrap();
        let changed = BubbleAppearance {
            theme: BubbleTheme::DustyRose,
            custom: preferences.bubble_appearance().custom,
        };
        preferences
            .save_bubble_appearance_in_directory(changed, &blocked)
            .expect_err("appearance save should fail for a file path");
        assert_eq!(preferences.bubble_appearance(), BubbleAppearance::default());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&blocked).unwrap(), b"sentinel");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_status_indicator_save_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences
            .save_show_status_indicators_in_directory(false, &directory)
            .expect("initial disabled value should save");
        let before = fs::read(&path).expect("saved settings should be readable");
        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").expect("blocking file should be written");

        preferences
            .save_show_status_indicators_in_directory(true, &blocked)
            .expect_err("save through a file path should fail");
        assert!(!preferences.show_status_indicators());
        assert_eq!(
            fs::read(&path).expect("saved settings should remain"),
            before
        );
        assert!(!Preferences::load_path(&path)
            .expect("saved settings should still load")
            .show_status_indicators());
        assert_eq!(
            fs::read(&blocked).expect("blocking file should remain"),
            b"sentinel"
        );
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
            .save_observation_in_directory(
                ObservationPreferences {
                    local: false,
                    remote: true,
                    machines: vec![machine.clone()],
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
    fn disabled_remote_selection_survives_sanitized_save_and_reload() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        let id = "opaque id:/🌲".to_owned();
        preferences
            .save_observation_in_directory(
                ObservationPreferences {
                    local: true,
                    remote: false,
                    machines: vec![id.clone(), id.clone(), String::new(), "x".repeat(4097)],
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
            .save_observation_in_directory(
                ObservationPreferences {
                    remote: true,
                    ..loaded.observation().clone()
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
    fn failed_observation_save_keeps_memory_and_disk_unchanged() {
        let directory = isolated_preferences_directory();
        let path = directory.join(PREFERENCES_FILE);
        let mut preferences = Preferences::default();
        preferences
            .save_in_directory(&directory)
            .expect("initial save");
        let before = fs::read(&path).expect("saved preferences");
        let blocked = directory.join("not-a-directory");
        fs::write(&blocked, b"sentinel").expect("blocking file");
        preferences
            .save_observation_in_directory(
                ObservationPreferences {
                    local: false,
                    remote: true,
                    machines: vec!["opaque".to_owned()],
                },
                &blocked,
            )
            .expect_err("save through a file must fail");
        assert_eq!(
            preferences.observation(),
            &ObservationPreferences::default()
        );
        assert_eq!(fs::read(&path).expect("saved preferences"), before);
        assert_eq!(fs::read(&blocked).expect("blocking file"), b"sentinel");
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
        let fixtures: [&[u8]; 4] = [
            b"{not json",
            br#"{"visible":true,"passthrough":false,"bubble_placement":"diagonal","scale":0.65,"position":null}"#,
            br#"{"visible":true,"passthrough":false,"menu_bar_mode":"sometimes","scale":0.65,"position":null}"#,
            br#"{"visible":true,"passthrough":false,"scale":1,"position":null,"dialogue_overrides":{"characters":{"forest":{"fr":{"phases":{"idle":"wrong locale"}}}}}}"#,
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
