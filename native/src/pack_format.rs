use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical v4 manifest.  Optional fields are omitted rather than encoded as
/// JSON null; the loader enforces that rule before deserializing this type.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestV4 {
    pub format: String,
    pub version: u32,
    pub render_mode: String,
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub author: AuthorV4,
    pub source: SourceV4,
    pub licenses: Vec<LicenseV4>,
    pub attributions: Vec<String>,
    pub entry: String,
    pub runtime: RuntimeV4,
    pub payloads: Vec<PayloadV4>,
    pub persona: Option<String>,
    pub dialogue: Option<BTreeMap<String, DialogueLocaleV4>>,
}

/// v5 manifests retain the v4 provenance and inventory envelope while the rig
/// entry switches to a catalog of independently authored models.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestV5 {
    pub format: String,
    pub version: u32,
    pub render_mode: String,
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub author: AuthorV4,
    pub source: SourceV4,
    pub licenses: Vec<LicenseV4>,
    pub attributions: Vec<String>,
    pub entry: String,
    pub runtime: RuntimeV4,
    pub payloads: Vec<PayloadV4>,
    pub persona: Option<String>,
    pub dialogue: Option<BTreeMap<String, DialogueLocaleV4>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RigEntryV5 {
    pub version: u32,
    pub initial: String,
    pub models: Vec<RigModelV5>,
    pub bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RigModelV5 {
    pub id: String,
    pub file: String,
    pub overrides: String,
    pub motion: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthorV4 {
    pub name: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceV4 {
    pub method: String,
    pub description: String,
    pub urls: Option<Vec<String>>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LicenseV4 {
    pub expression: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeV4 {
    pub name: String,
    pub version: u32,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadV4 {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueLocaleV4 {
    pub phases: Option<BTreeMap<String, String>>,
    pub reactions: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PngEntryV4 {
    pub version: u32,
    pub phases: BTreeMap<String, PngPhaseV4>,
    pub reactions: BTreeMap<String, PngReactionV4>,
    pub regions: BTreeMap<String, FrameRegionsV4>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PngPhaseV4 {
    pub fps: u8,
    pub frames: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PngReactionV4 {
    pub frames: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameRegionsV4 {
    pub head: RegionRectV4,
    pub body: RegionRectV4,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionRectV4 {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RigEntryV4 {
    pub version: u32,
    pub base: RigPoseV4,
    pub pose: Option<RigPoseV4>,
    pub overrides: String,
    pub motion: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RigPoseV4 {
    pub id: String,
    pub file: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionFileV4 {
    pub version: u32,
    pub phases: BTreeMap<String, MotionV4>,
    pub reactions: BTreeMap<String, MotionV4>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionV4 {
    pub duration_ms: u32,
    pub pose: String,
    pub transition_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<MotionPlaybackV4>,
    pub tracks: BTreeMap<String, MotionTrackV4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<BTreeMap<String, MotionLayerV4>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MotionPlaybackV4 {
    Loop,
    Once,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum MotionTrackV4 {
    Legacy(Vec<MotionKeyV4>),
    V2(MotionKeyTrackV4),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionKeyTrackV4 {
    pub interpolation: MotionInterpolationV4,
    pub keys: Vec<MotionKeyV4>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MotionInterpolationV4 {
    Linear,
    Smoothstep,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MotionLayerV4 {
    pub origin: MotionOriginV4,
    pub influence: MotionInfluenceV4,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_x: Option<MotionKeyTrackV4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_y: Option<MotionKeyTrackV4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<MotionKeyTrackV4>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<MotionKeyTrackV4>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionOriginV4 {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MotionInfluenceV4 {
    pub axis_x: f64,
    pub axis_y: f64,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionKeyV4 {
    pub at_ms: u32,
    pub value: f64,
}

/// Validated presentation metadata. Complete provenance, rights and persona
/// remain byte-preserved in the immutable manifest inventory.
#[derive(Clone, Debug)]
pub struct CharacterMetadata {
    pub dialogue: Option<BTreeMap<String, DialogueLocale>>,
}

#[derive(Clone, Debug)]
pub struct DialogueLocale {
    pub phases: Option<BTreeMap<String, String>>,
    pub reactions: Option<BTreeMap<String, String>>,
}

impl CharacterMetadata {
    /// Resolve a pack-owned dialogue string without allowing it to replace
    /// host-owned status semantics.  Locale selection is exact, then language
    /// prefix, then en/ko, then deterministic first locale.  Reaction text has
    /// precedence over phase text when a reaction is supplied.
    pub fn dialogue_text(&self, phase: &str, reaction: Option<&str>, locale: &str) -> Option<&str> {
        let dialogue = self.dialogue.as_ref()?;
        let mut locales = Vec::<&str>::new();
        if dialogue.contains_key(locale) {
            locales.push(locale);
        }
        if let Some(language) = locale.split('-').next() {
            for key in dialogue.keys() {
                if key != locale && key.split('-').next() == Some(language) {
                    locales.push(key.as_str());
                }
            }
        }
        for fallback in ["en", "ko"] {
            if dialogue.contains_key(fallback) && !locales.contains(&fallback) {
                locales.push(fallback);
            }
        }
        for key in dialogue.keys() {
            if !locales.contains(&key.as_str()) {
                locales.push(key.as_str());
            }
        }
        for key in locales {
            let entry = dialogue.get(key)?;
            if let Some(reaction) = reaction {
                if let Some(text) = entry
                    .reactions
                    .as_ref()
                    .and_then(|values| values.get(reaction))
                {
                    return Some(text.as_str());
                }
            }
            if let Some(text) = entry.phases.as_ref().and_then(|values| values.get(phase)) {
                return Some(text.as_str());
            }
        }
        None
    }
}

impl From<&ManifestV4> for CharacterMetadata {
    fn from(manifest: &ManifestV4) -> Self {
        Self {
            dialogue: manifest.dialogue.as_ref().map(|dialogue| {
                dialogue
                    .iter()
                    .map(|(locale, entry)| {
                        (
                            locale.clone(),
                            DialogueLocale {
                                phases: entry.phases.clone(),
                                reactions: entry.reactions.clone(),
                            },
                        )
                    })
                    .collect()
            }),
        }
    }
}

impl From<&ManifestV5> for CharacterMetadata {
    fn from(manifest: &ManifestV5) -> Self {
        Self {
            dialogue: manifest.dialogue.as_ref().map(|dialogue| {
                dialogue
                    .iter()
                    .map(|(locale, entry)| {
                        (
                            locale.clone(),
                            DialogueLocale {
                                phases: entry.phases.clone(),
                                reactions: entry.reactions.clone(),
                            },
                        )
                    })
                    .collect()
            }),
        }
    }
}

pub(crate) fn known_phase(name: &str) -> bool {
    matches!(name, "idle" | "running" | "waiting" | "unknown")
}

pub(crate) fn known_reaction(name: &str) -> bool {
    matches!(
        name,
        "head_tap" | "body_tap" | "pet" | "completion_observed"
    )
}

pub(crate) fn valid_payload_kind(kind: &str) -> bool {
    matches!(
        kind,
        "png" | "psd" | "entry" | "overrides" | "motion" | "license" | "attribution" | "source"
    )
}

/// Stable semantic selector names carried by the v5 entry bindings.  Their
/// numeric order is part of the native ABI and must not be reordered.
pub(crate) const V5_POSE_NAMES: [&str; 10] = [
    "waiting",
    "writing",
    "failed",
    "cancelled",
    "disconnected",
    "bored",
    "happy",
    "head-tap",
    "torso-tap",
    "head-pet",
];
