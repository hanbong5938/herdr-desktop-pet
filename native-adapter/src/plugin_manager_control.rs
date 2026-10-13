//! Herdr projects its opaque optional-component field from the common owner.
//! CAS, owner probing, offline gates, and operation uncertainty stay in core.
use desktop_pet_contracts::{PreferenceAccess, PreferenceCommit};
use serde_json::{Map, Value};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PluginManagerAvailabilityCommit {
    Committed(bool),
    Uncertain {
        instance_id: String,
        operation_id: String,
        current: Option<bool>,
    },
}
fn availability_from_snapshot(saved: &Map<String, Value>) -> Result<bool, String> {
    saved
        .get("plugin_manager_installed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "availability persisted result is missing or invalid".to_owned())
}
pub(crate) fn get_plugin_manager_availability(
    preferences: &dyn PreferenceAccess,
) -> Result<bool, String> {
    availability_from_snapshot(&preferences.read()?)
}
pub(crate) fn set_plugin_manager_availability(
    preferences: &dyn PreferenceAccess,
    installed: bool,
) -> Result<PluginManagerAvailabilityCommit, String> {
    let mut patch = Map::new();
    patch.insert(
        "plugin_manager_installed".to_owned(),
        Value::Bool(installed),
    );
    match preferences.apply(patch)? {
        PreferenceCommit::Committed(saved) => Ok(PluginManagerAvailabilityCommit::Committed(
            availability_from_snapshot(&saved)?,
        )),
        PreferenceCommit::Uncertain {
            instance_id,
            operation_id,
            current,
        } => {
            let current = current
                .as_ref()
                .and_then(|saved| availability_from_snapshot(saved).ok());
            Ok(PluginManagerAvailabilityCommit::Uncertain {
                instance_id,
                operation_id,
                current,
            })
        }
    }
}
