use std::time::Duration;

/// Stable semantic indices shared with the native independent-model catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[repr(i32)]
#[serde(rename_all = "kebab-case")]
pub enum PoseKind {
    Waiting = 0,
    Writing = 1,
    Failed = 2,
    Cancelled = 3,
    Disconnected = 4,
    Bored = 5,
    Happy = 6,
    HeadTap = 7,
    TorsoTap = 8,
    HeadPet = 9,
}

impl PoseKind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "waiting" => Self::Waiting,
            "writing" => Self::Writing,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "disconnected" => Self::Disconnected,
            "bored" => Self::Bored,
            "happy" => Self::Happy,
            "head-tap" => Self::HeadTap,
            "torso-tap" => Self::TorsoTap,
            "head-pet" => Self::HeadPet,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PoseSnapshot {
    pub kind: PoseKind,
    pub age: Duration,
}
