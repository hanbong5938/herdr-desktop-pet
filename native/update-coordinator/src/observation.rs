use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

/// Application-owned, canonical selection captured in the update context.
pub const OBSERVATION_MODE_ENV: &str = "HERDR_PET_OBSERVATION_MODE";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationMode {
    #[default]
    Herdr,
    Tern,
    Both,
}

impl ObservationMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Herdr => "herdr",
            Self::Tern => "tern",
            Self::Both => "both",
        }
    }

    pub const fn supports_herdr(self) -> bool {
        matches!(self, Self::Herdr | Self::Both)
    }

    pub const fn supports_tern(self) -> bool {
        matches!(self, Self::Tern | Self::Both)
    }

    /// Missing key denotes a plan captured before observation selection existed.
    pub fn from_environment(environment: &BTreeMap<String, String>) -> Result<Self, String> {
        environment
            .get(OBSERVATION_MODE_ENV)
            .map_or(Ok(Self::Herdr), |value| value.parse())
    }
}

impl FromStr for ObservationMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "herdr" => Ok(Self::Herdr),
            "tern" => Ok(Self::Tern),
            "both" => Ok(Self::Both),
            _ => Err(format!("invalid observation mode: {value:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_capture_is_legacy_herdr_but_invalid_capture_is_not() {
        let mut environment = BTreeMap::new();
        assert_eq!(
            ObservationMode::from_environment(&environment),
            Ok(ObservationMode::Herdr)
        );
        for invalid in ["", "Herdr", "TERN", "herdr,tern", "unknown", " both "] {
            environment.insert(OBSERVATION_MODE_ENV.into(), invalid.into());
            assert!(ObservationMode::from_environment(&environment).is_err());
            assert!(invalid.parse::<ObservationMode>().is_err());
        }
        for mode in [
            ObservationMode::Herdr,
            ObservationMode::Tern,
            ObservationMode::Both,
        ] {
            environment.insert(OBSERVATION_MODE_ENV.into(), mode.as_str().into());
            assert_eq!(ObservationMode::from_environment(&environment), Ok(mode));
            assert_eq!(
                serde_json::to_string(&mode).unwrap(),
                format!("\"{}\"", mode.as_str())
            );
        }
    }
}
