use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const REMOTE_PREFIX: &str = "machine:";
const MAX_MACHINES: usize = 64;
const MAX_MACHINE_ID_BYTES: usize = 4096;

fn default_local() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct ObservationPreferences {
    #[serde(default = "default_local")]
    pub local: bool,
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub machines: Vec<String>,
}

impl Default for ObservationPreferences {
    fn default() -> Self {
        Self {
            local: true,
            remote: false,
            machines: Vec::new(),
        }
    }
}

impl ObservationPreferences {
    pub(crate) fn includes(&self, source: &str) -> bool {
        match remote_machine_id(source) {
            Some(id) => self.remote && self.machines.iter().any(|selected| selected == id),
            None if source.starts_with(REMOTE_PREFIX) => false,
            None => self.local,
        }
    }

    pub(crate) fn sanitize(&mut self) {
        let mut seen = HashSet::new();
        self.machines.retain(|id| {
            !id.is_empty()
                && id.len() <= MAX_MACHINE_ID_BYTES
                && !id.contains('\0')
                && seen.len() < MAX_MACHINES
                && seen.insert(id.clone())
        });
    }
}

pub(crate) fn remote_source(id: &str) -> String {
    format!("{REMOTE_PREFIX}{id}")
}

pub(crate) fn remote_machine_id(source: &str) -> Option<&str> {
    source
        .strip_prefix(REMOTE_PREFIX)
        .filter(|id| !id.is_empty())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MachineStatus {
    #[default]
    NotSelected,
    Connecting,
    Online,
    Offline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MachineInfo {
    pub id: String,
    pub label: String,
    pub remote_session: String,
    pub enabled: bool,
    pub status: MachineStatus,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceCatalog {
    pub machines: Vec<MachineInfo>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_machine_ids_survive_sanitization_without_remote_enabled() {
        let opaque = " machine:/🌲 /key=abc ".to_owned();
        let boundary = "x".repeat(MAX_MACHINE_ID_BYTES);
        let mut preferences = ObservationPreferences {
            local: false,
            remote: false,
            machines: vec![
                String::new(),
                opaque.clone(),
                opaque.clone(),
                "x".repeat(MAX_MACHINE_ID_BYTES + 1),
                boundary.clone(),
                "invalid\0id".to_owned(),
            ],
        };
        preferences.sanitize();
        assert_eq!(preferences.machines, vec![opaque.clone(), boundary]);
        assert!(!preferences.includes(&remote_source(&opaque)));
        preferences.remote = true;
        assert!(preferences.includes(&remote_source(&opaque)));
        assert!(!preferences.includes("machine:"));
        assert!(!preferences.includes("/tmp/local.sock"));
    }

    #[test]
    fn selection_cap_retains_first_distinct_machine_ids() {
        let mut preferences = ObservationPreferences {
            machines: (0..MAX_MACHINES + 2)
                .map(|index| format!("opaque-{index}"))
                .collect(),
            ..ObservationPreferences::default()
        };
        preferences.sanitize();
        assert_eq!(preferences.machines.len(), MAX_MACHINES);
        assert_eq!(
            preferences.machines.first().map(String::as_str),
            Some("opaque-0")
        );
        assert_eq!(
            preferences.machines.last().map(String::as_str),
            Some("opaque-63")
        );
    }
}
