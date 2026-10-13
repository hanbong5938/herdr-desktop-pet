use crate::{settings, HerdrIntegration, Route, Shared, DISCONNECTION_GRACE};
use desktop_pet_contracts::*;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::ffi::OsStr;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) struct Registration(pub Arc<HerdrIntegration>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterRequest { kind: String, endpoint: PathBuf, #[serde(default)] readiness_only: bool }
impl ControlExtension for Registration {
    fn kind(&self) -> &'static str { "register" }
    fn max_request_bytes(&self) -> usize { 16 * 1024 }
    fn max_response_bytes(&self) -> usize { 16 * 1024 }
    fn handle_frame(&self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() > self.max_request_bytes() { return Err("Herdr registration exceeds size limit".to_owned()); }
        let request: RegisterRequest = serde_json::from_slice(frame).map_err(|error| error.to_string())?;
        if request.kind != self.kind() { return Err("invalid Herdr registration kind".to_owned()); }
        let accepted = if request.readiness_only {
            self.0.legacy_endpoint_registered(&request.endpoint)?
        } else {
            self.0.register(request.endpoint)?;
            true
        };
        let mut bytes = serde_json::to_vec(&serde_json::json!({"ok":true,"registration_accepted":accepted})).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}
pub(crate) struct Observation(pub Arc<HerdrIntegration>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationRequest {
    kind: String,
    command: String,
    #[serde(default)]
    patch: Option<Map<String, Value>>,
}
impl ControlExtension for Observation {
    fn kind(&self) -> &'static str { "herdr-observation" }
    fn max_request_bytes(&self) -> usize { 256 * 1024 }
    fn max_response_bytes(&self) -> usize { 512 * 1024 }
    fn handle_frame(&self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() > self.max_request_bytes() { return Err("Herdr observation request exceeds size limit".to_owned()); }
        let request: ObservationRequest = serde_json::from_slice(frame).map_err(|error| error.to_string())?;
        if request.kind != self.kind() { return Err("invalid Herdr observation kind".to_owned()); }
        match request.command.as_str() {
            "get" if request.patch.is_none() => {}
            "set" => {
                let patch = request.patch.as_ref().ok_or("Herdr observation set requires a patch")?;
                let prepared = self.0.prepare_preference_patch(patch)?;
                self.0.persist_patch(&self.0.store()?, prepared.as_ref())?;
            }
            _ => return Err("invalid Herdr observation command".to_owned()),
        }
        let registry = self.0.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        let mut encoded = serde_json::to_vec(&serde_json::json!({"ok":true,"preferences":registry.preferences,"catalog":registry.catalog,"failure":registry.failure})).map_err(|error| error.to_string())?;
        if encoded.len().saturating_add(1) > self.max_response_bytes() { return Err("Herdr observation response exceeds size limit".to_owned()); }
        encoded.push(b'\n');
        Ok(encoded)
    }
}
const OPTIONS: &[CliOption] = &[CliOption { name: "--herdr-socket", takes_value: true, help: "Herdr backend Unix socket (absolute path; selected Herdr integration only)" }];
impl CliExtension for HerdrIntegration {
    fn options(&self) -> &'static [CliOption] { OPTIONS }
    fn commands(&self) -> &'static [&'static str] { &["herdr-observation"] }
    fn configure_option(&self, name: &str, value: Option<&OsStr>) -> Result<(), String> {
        if name != "--herdr-socket" { return Err(format!("unsupported Herdr option {name}")); }
        let path = PathBuf::from(value.ok_or("--herdr-socket requires a value")?);
        if !path.is_absolute() { return Err("--herdr-socket must be an absolute path".to_owned()); }
        // Store lexical input only. Disabled/absent Herdr never resolves the
        // endpoint or reads HERDR_SOCKET_PATH as a side effect of CLI parsing.
        self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?.configured_socket = Some(path);
        Ok(())
    }
    fn max_input_bytes(&self, command: &str) -> usize { if command == "herdr-observation" { 256 * 1024 } else { 0 } }
    fn execute(&self, invocation: CliInvocation<'_>) -> Result<CliOutput, String> {
        if invocation.command != "herdr-observation" { return Err("unsupported Herdr command".to_owned()); }
        if invocation.arguments.len() > 1 { return Err("usage: herdr-observation [get|set] (set reads a JSON patch on stdin)".to_owned()); }
        let command = invocation.arguments.first().map(|argument| argument.to_str().ok_or("command is not UTF-8")).transpose()?.unwrap_or("get");
        let mut bytes = match command {
            "get" if invocation.input.is_empty() => br#"{"kind":"herdr-observation","command":"get"}"#.to_vec(),
            "set" if !invocation.input.is_empty() => {
                let prefix = br#"{"kind":"herdr-observation","command":"set","patch":"#;
                if prefix.len().saturating_add(invocation.input.len()).saturating_add(2) > 256 * 1024 { return Err("Herdr observation request exceeds size limit".to_owned()); }
                let mut bytes = Vec::with_capacity(prefix.len() + invocation.input.len() + 2);
                bytes.extend_from_slice(prefix);
                // Preserve raw duplicate keys for the existing common control
                // frame validator instead of normalizing them in this client.
                bytes.extend_from_slice(invocation.input);
                bytes.push(b'}');
                bytes
            }
            _ => return Err("usage: herdr-observation [get|set]".to_owned()),
        };
        if bytes.len().saturating_add(1) > 256 * 1024 { return Err("Herdr observation request exceeds size limit".to_owned()); }
        bytes.push(b'\n');
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = crate::socket::connect(&invocation.profile.control_socket, deadline).map_err(|error| error.to_string())?;
        let mut written = 0;
        while written < bytes.len() {
            stream.set_write_timeout(Some(crate::socket::remaining(deadline).map_err(|error| error.to_string())?)).map_err(|error| error.to_string())?;
            let count = stream.write(&bytes[written..]).map_err(|error| error.to_string())?;
            if count == 0 { return Err("control socket write made no progress".to_owned()); }
            written += count;
        }
        let mut response = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let count = crate::socket::read_with_deadline(&mut stream, &mut chunk, deadline).map_err(|error| error.to_string())?;
            if count == 0 { return Err("control socket closed before Herdr observation reply".to_owned()); }
            response.extend_from_slice(&chunk[..count]);
            if response.len() > 512 * 1024 { return Err("Herdr observation response exceeds size limit".to_owned()); }
            if let Some(end) = response.iter().position(|byte| *byte == b'\n') { response.truncate(end + 1); break; }
        }
        let reply: Value = serde_json::from_slice(&response).map_err(|error| error.to_string())?;
        let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
        Ok(CliOutput { stdout: response, stderr: Vec::new(), exit_code: if ok { 0 } else { 1 } })
    }
}
const PATCH_KEYS: &[&str] = &["observation_local", "observation_remote", "observation_machines"];
const PREFERENCE_OPTIONS: &[PreferencePatchOption] = &[
    PreferencePatchOption { name:"--observation-local", takes_value:true, repeatable:false, help:"Observe local Herdr sessions: on|off" },
    PreferencePatchOption { name:"--observation-remote", takes_value:true, repeatable:false, help:"Observe selected saved Herdr machines: on|off" },
    PreferencePatchOption { name:"--machine", takes_value:true, repeatable:true, help:"Saved Herdr machine ID (repeatable)" },
    PreferencePatchOption { name:"--clear-machines", takes_value:false, repeatable:false, help:"Clear selected saved Herdr machines" },
];
struct PreparedPatch {
    shared: Arc<Shared>,
    local: Option<bool>,
    remote: Option<bool>,
    machines: Option<Vec<String>>,
}
impl PreparedPreferencePatch for PreparedPatch {
    fn apply(&self, candidate: &mut Map<String, Value>) -> Result<(), String> {
        settings::patch_preferences(candidate, self.local, self.remote, self.machines.as_deref())?;
        Ok(())
    }
    fn committed(&self, _saved: &Map<String, Value>) -> Result<(), String> {
        // Callbacks can arrive out of writer order. Serialize live application
        // and refresh the latest owner fields, rather than replaying an older
        // saved candidate over a newer already-applied write. The profile
        // writer is released before entering the domain transaction.
        let _policy_write = self.shared.policy_writes.lock().map_err(|_| "Herdr lifecycle policy is poisoned")?;
        let store = self.shared.store.lock().map_err(|_| "Herdr profile is poisoned")?.clone().ok_or("Herdr profile is not configured")?;
        let preferences = settings::read_preferences(&store)?;
        if self.shared.runtime.get().is_none() {
            self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?.preferences = preferences;
            return Ok(());
        }
        self.shared.transaction(&mut |domain, registry| {
            registry.preferences = preferences.clone();
            let crate::Registry { bindings, operations, .. } = registry;
            for (source, binding) in bindings {
                let selected = match &binding.route {
                    Route::Local(_) => preferences.local,
                    Route::Remote(id) => preferences.remote && preferences.machines.contains(id),
                };
                domain.set_source_observed(source, binding.generation, selected);
                if !selected {
                    operations.retain(|_, operation| { if operation.source == *source { operation.cancel(); false } else { true } });
                    if matches!(binding.route, Route::Remote(_)) {
                        domain.mark_source_offline(source, binding.generation);
                        binding.connected = false;
                        binding.offline_since.get_or_insert(Instant::now());
                    }
                }
            }
            Shared::observe_connections(registry, Instant::now());
        })?;
        self.shared.wake();
        Ok(())
    }
}
impl HerdrIntegration {
    fn persist_patch(&self, store: &ProfileStore, prepared: &dyn PreparedPreferencePatch) -> Result<(), String> {
        if self.shared.runtime.get().is_some() {
            let mut admitted = Err("Herdr integration is unavailable".to_owned());
            self.shared.transaction(&mut |domain, registry| { admitted = domain.mutation_allowed(); if registry.finalized { admitted = Err("desktop-pet is shutting down".to_owned()); } })?;
            admitted?;
        }
        let saved = store.with_writer_lock(|| {
            let mut candidate = settings::read_preferences_object(store)?;
            prepared.apply(&mut candidate)?;
            let bytes = serde_json::to_vec_pretty(&candidate).map_err(|error| error.to_string())?;
            desktop_pet_contracts::profile_store::atomic_write(&store.config_dir().join("preferences.json"), &bytes)?;
            Ok(candidate)
        })?;
        prepared.committed(&saved)
    }
}
impl LifecycleExtension for HerdrIntegration {
    fn provider(&self) -> &'static str { "herdr" }
    fn initialize_lifecycle(&self, candidate: &mut Map<String, Value>) -> Result<(), String> {
        candidate.entry("exit_with_herdr".to_owned()).or_insert(Value::Bool(true));
        Ok(())
    }
    fn settings(&self, store: &ProfileStore, selection: &ObservationSelection, locale: &str) -> Result<Vec<LifecycleSettingView>, String> {
        use crate::i18n::{text, Message};
        let t = |message| text(locale, message);
        let (object, exists) = store.read_object_with_presence()?;
        let exit = desktop_pet_contracts::profile_store::setting_bool(store.config_dir(), &object, "exit_with_herdr")?.unwrap_or(!exists);
        let mut views = vec![LifecycleSettingView {
            key: "exit_with_herdr".to_owned(), label: t(Message::Exit).to_owned(),
            help: t(Message::ExitHelp).to_owned(), value: exit,
            enabled: matches!(selection, ObservationSelection::Provider(id) if id == "herdr"),
        }];
        if !selection.includes_provider("herdr") {
            return Ok(views);
        }
        let preferences = settings::read_preferences(store)?;
        let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        let local_help = if !preferences.local && !preferences.remote { Message::NoSources } else { Message::LocalHelp };
        views.push(LifecycleSettingView {
            key: "observation_local".to_owned(), label: t(Message::Local).to_owned(),
            help: t(local_help).to_owned(), value: preferences.local, enabled: true,
        });
        let remote_state = registry.catalog.error.as_deref().unwrap_or_else(|| {
            if !registry.catalog.initialized { t(Message::Loading) }
            else if registry.catalog.machines.is_empty() { t(Message::Empty) }
            else { "" }
        });
        views.push(LifecycleSettingView {
            key: "observation_remote".to_owned(), label: t(Message::Remote).to_owned(),
            help: if remote_state.is_empty() { t(Message::RemoteHelp).to_owned() }
                else { format!("{}\n{remote_state}", t(Message::RemoteHelp)) },
            value: preferences.remote, enabled: true,
        });
        for machine in &registry.catalog.machines {
            let selected = preferences.machines.contains(&machine.id);
            let status = if !machine.enabled { Message::Disabled }
                else if !preferences.remote { Message::Paused }
                else if !selected { Message::NotSelected }
                else { match machine.status {
                    settings::MachineStatus::NotSelected => Message::NotSelected,
                    settings::MachineStatus::Connecting => Message::Connecting,
                    settings::MachineStatus::Online => Message::Online,
                    settings::MachineStatus::Offline => Message::Offline,
                }};
            views.push(LifecycleSettingView {
                key: format!("observation_machine:{}", machine.id), label: machine.label.clone(),
                help: format!("{} · {}{}", machine.remote_session, t(status), machine.error.as_ref().map(|error| format!(" · {error}")).unwrap_or_default()),
                value: selected, enabled: preferences.remote && machine.enabled,
            });
        }
        Ok(views)
    }
    fn apply_setting(&self, store: &ProfileStore, key: &str, value: bool) -> Result<(), String> {
        if key == "exit_with_herdr" {
            let _policy_write = self.shared.policy_writes.lock().map_err(|_| "Herdr lifecycle policy is poisoned")?;
            if self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?.finalized { return Err("desktop-pet is shutting down".to_owned()); }
            store.update(|object, exists| {
                desktop_pet_contracts::profile_store::setting_bool(store.config_dir(), object, "exit_with_herdr")?.unwrap_or(!exists);
                object.insert("exit_with_herdr".to_owned(), Value::Bool(value)); Ok(())
            })?;
            let mut registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
            if value != registry.exit_with_herdr { registry.deadline = (value && registry.connected == 0).then_some(Instant::now() + DISCONNECTION_GRACE); }
            registry.exit_with_herdr = value;
            return Ok(());
        }
        let mut patch = Map::new();
        match key {
            "observation_local" | "observation_remote" => { patch.insert(key.to_owned(), Value::Bool(value)); }
            _ => {
                let id = key.strip_prefix("observation_machine:").ok_or("unsupported Herdr lifecycle setting")?;
                let preferences = settings::read_preferences(store)?;
                let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
                if !registry.catalog.machines.iter().any(|machine| machine.id == id && machine.enabled) { return Err("machine is not in the enabled saved Herdr catalog".to_owned()); }
                let mut machines = preferences.machines;
                machines.retain(|selected| selected != id);
                if value { machines.push(id.to_owned()); }
                // GUI deltas retain unavailable previously selected IDs; only
                // the chosen machine requires fresh enabled catalog authority.
                drop(registry);
                let prepared = PreparedPatch { shared:Arc::clone(&self.shared), local:None, remote:None, machines:Some(machines) };
                return self.persist_patch(store, &prepared);
            }
        }
        self.persist_patch(store, self.prepare_preference_patch(&patch)?.as_ref())
    }
    fn actions(&self, _store: &ProfileStore, selection: &ObservationSelection, locale: &str) -> Result<Vec<LifecycleActionView>, String> {
        use crate::i18n::{text, Message};
        let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
        Ok(registry.catalog.machines.iter().filter(|machine| selection.includes_provider("herdr") && registry.preferences.remote && registry.preferences.machines.contains(&machine.id) && machine.enabled && machine.status == settings::MachineStatus::Offline).map(|machine| LifecycleActionView { key:format!("reconnect:{}",machine.id), label:format!("{} · {}",text(locale, Message::CopyReconnect),machine.label), help:text(locale, Message::ReconnectHelp).to_owned(), enabled:true, copy_text:Some(format!("herdr machine reconnect '{}'",machine.id.replace('\'',"'\\''"))) }).collect())
    }
    fn preference_patch_keys(&self) -> &'static [&'static str] { PATCH_KEYS }
    fn preference_snapshot(&self, saved: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let preferences = settings::preferences_from_object(saved)?;
        let mut result = Map::new();
        result.insert("observation_local".to_owned(), Value::Bool(preferences.local));
        result.insert("observation_remote".to_owned(), Value::Bool(preferences.remote));
        result.insert("observation_machines".to_owned(), serde_json::to_value(preferences.machines).map_err(|error| error.to_string())?);
        Ok(result)
    }
    fn preference_options(&self) -> &'static [PreferencePatchOption] { PREFERENCE_OPTIONS }
    fn parse_preference_options(&self, arguments: &[PreferencePatchArgument]) -> Result<Map<String, Value>, String> {
        let mut patch = Map::new(); let mut machines = Vec::new(); let mut clear = false;
        for argument in arguments {
            match argument.name.as_str() {
                "--observation-local" | "--observation-remote" => {
                    let value = match argument.value.as_ref().and_then(|value| value.to_str()) { Some("on") => true, Some("off") => false, _ => return Err(format!("{} requires on|off",argument.name)) };
                    let key = if argument.name == "--observation-local" { "observation_local" } else { "observation_remote" };
                    if patch.insert(key.to_owned(),Value::Bool(value)).is_some() { return Err(format!("duplicate {}",argument.name)); }
                }
                "--machine" => machines.push(argument.value.as_ref().and_then(|value| value.to_str()).ok_or("--machine requires UTF-8 ID")?.to_owned()),
                "--clear-machines" if argument.value.is_none() && !clear => clear = true,
                _ => return Err(format!("unsupported or duplicate Herdr preference option {}",argument.name)),
            }
        }
        if clear && !machines.is_empty() { return Err("--machine and --clear-machines cannot be combined".to_owned()); }
        if clear || !machines.is_empty() { patch.insert("observation_machines".to_owned(),serde_json::to_value(machines).map_err(|error| error.to_string())?); }
        Ok(patch)
    }
    fn prepare_preference_patch(&self, patch: &Map<String, Value>) -> Result<Box<dyn PreparedPreferencePatch>, String> {
        if patch.keys().any(|key| !PATCH_KEYS.contains(&key.as_str())) { return Err("unsupported Herdr preference patch field".to_owned()); }
        let boolean = |key| patch.get(key).map(|value| value.as_bool().ok_or_else(|| format!("{key} must be a boolean"))).transpose();
        let local = boolean("observation_local")?;
        let remote = boolean("observation_remote")?;
        let machines: Option<Vec<String>> = patch.get("observation_machines").map(|value| serde_json::from_value(value.clone()).map_err(|_| "observation_machines must be a list of strings".to_owned())).transpose()?;
        if let Some(machines) = &machines {
            settings::ObservationPreferences { local:true, remote:false, machines:machines.clone() }.validate()?;
            let registry = self.shared.registry.lock().map_err(|_| "Herdr registry is poisoned")?;
            if !machines.is_empty() && (!registry.catalog.initialized || machines.iter().any(|id| !registry.catalog.machines.iter().any(|machine| machine.id == *id && machine.enabled))) { return Err("selected machine is not in the enabled saved Herdr catalog".to_owned()); }
        }
        Ok(Box::new(PreparedPatch { shared:Arc::clone(&self.shared), local, remote, machines }))
    }
    fn should_exit(&self, selection: &ObservationSelection, now: Instant) -> bool {
        if self.shared.stop.load(Ordering::Acquire) { return false; }
        let Ok(_policy_write) = self.shared.policy_writes.lock() else { self.shared.fail("Herdr lifecycle policy is poisoned".to_owned()); return false; };
        let Ok(mut registry) = self.shared.registry.lock() else { return false; };
        if registry.policy_selection != *selection {
            registry.policy_selection = selection.clone();
            registry.deadline = (matches!(selection, ObservationSelection::Provider(id) if id == "herdr") && registry.exit_with_herdr && registry.connected == 0).then_some(now + DISCONNECTION_GRACE);
        }
        if !matches!(selection, ObservationSelection::Provider(id) if id == "herdr") || !registry.enabled || registry.finalized { return false; }
        Shared::observe_connections(&mut registry, now);
        let mut local = registry.bindings.values().filter(|binding| matches!(binding.route, Route::Local(_)));
        let all_disabled = local.next().is_some_and(|first| first.disabled && local.all(|binding| binding.disabled));
        if all_disabled || (registry.exit_with_herdr && registry.deadline.is_some_and(|deadline| now >= deadline)) { registry.finalized = true; true } else { false }
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;

    #[test]
    fn registration_and_observation_replies_are_complete_bounded_frames() {
        let integration = HerdrIntegration::new();
        let registration = Registration(integration.clone());
        let observation = Observation(integration);
        for (handler, request) in [
            (&registration as &dyn ControlExtension, br#"{"kind":"register","endpoint":"/absent/herdr.sock","readiness_only":true}"#.as_slice()),
            (&observation as &dyn ControlExtension, br#"{"kind":"herdr-observation","command":"get"}"#.as_slice()),
        ] {
            let response = handler.handle_frame(request).unwrap();
            assert_eq!(response.last(), Some(&b'\n'));
            assert!(!response[..response.len() - 1].contains(&b'\n'));
            assert!(response.len() <= handler.max_response_bytes());
            assert_eq!(serde_json::from_slice::<Value>(&response).unwrap()["ok"], true);
        }
    }
}
