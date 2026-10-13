use crate::{harness, settings, Completion, HerdrIntegration, PendingOperation, Route, DISCONNECTION_GRACE};
use desktop_pet_contracts::*;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn profile(label: &str) -> (PathBuf, ProfileStore) {
    let root=std::env::temp_dir().join(format!("herdr-{label}-{}-{}",std::process::id(),crate::NEXT_OPERATION.fetch_add(1,Ordering::Relaxed)));
    let store=ProfileStore::acquire(&root.join("config"),&root.join("state")).unwrap();
    (root,store)
}

#[test]
fn fresh_common_lifecycle_writes_retain_herdr_default_after_profile_configuration() {
    for startup in [false, true] {
        let (root, store) = profile("fresh-common-write");
        let integration = HerdrIntegration::new();
        integration.configure_profile(store.clone()).unwrap();
        let selection = ObservationSelection::Provider("herdr".into());
        let before = integration.settings(&store, &selection, "en").unwrap();
        assert!(before.iter().find(|view| view.key == "exit_with_herdr").unwrap().value);
        assert!(!store.config_dir().join("lifecycle.json").exists());

        store.update(|object, exists| {
            assert!(!exists);
            integration.initialize_lifecycle(object)?;
            object.insert("auto_start".into(), Value::Bool(!startup));
            if startup {
                object.insert("startup".into(), json!({"token": "reserved", "pid": 1}));
            }
            object.insert("future_owner".into(), json!({"version": 7}));
            Ok(())
        }).unwrap();

        integration.configure_profile(store.clone()).unwrap();
        assert_eq!(integration.legacy_status_fields().unwrap()["exit_with_herdr"], true);
        let after = integration.settings(&store, &selection, "en").unwrap();
        assert!(after.iter().find(|view| view.key == "exit_with_herdr").unwrap().value);
        let (object, exists) = store.read_object_with_presence().unwrap();
        assert!(exists);
        assert_eq!(object["exit_with_herdr"], true);
        assert_eq!(object["auto_start"], !startup);
        assert_eq!(object["future_owner"], json!({"version": 7}));
        if startup {
            assert_eq!(object["startup"], json!({"token": "reserved", "pid": 1}));
        }
        drop(integration);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn existing_missing_or_explicit_exit_policy_survives_common_write_and_readonly_queries() {
    for explicit in [None, Some(false), Some(true)] {
        let (root, store) = profile("existing-common-write");
        store.update(|object, _| {
            object.insert("auto_start".into(), Value::Bool(false));
            object.insert("future_owner".into(), json!({"version": 8}));
            if let Some(value) = explicit {
                object.insert("exit_with_herdr".into(), Value::Bool(value));
            }
            Ok(())
        }).unwrap();
        let path = store.config_dir().join("lifecycle.json");
        let before = std::fs::read(&path).unwrap();
        let integration = HerdrIntegration::new();
        integration.configure_profile(store.clone()).unwrap();
        let selection = ObservationSelection::Provider("herdr".into());
        let views = integration.settings(&store, &selection, "en").unwrap();
        assert_eq!(
            views.iter().find(|view| view.key == "exit_with_herdr").unwrap().value,
            explicit.unwrap_or(false),
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);

        store.update(|object, exists| {
            if !exists {
                integration.initialize_lifecycle(object)?;
            }
            object.insert("auto_start".into(), Value::Bool(true));
            object.insert("startup".into(), json!({"token": "reserved"}));
            Ok(())
        }).unwrap();
        integration.configure_profile(store.clone()).unwrap();
        assert_eq!(
            integration.legacy_status_fields().unwrap()["exit_with_herdr"],
            explicit.unwrap_or(false),
        );
        let (object, _) = store.read_object_with_presence().unwrap();
        assert_eq!(object.get("exit_with_herdr").and_then(Value::as_bool), explicit);
        assert_eq!(object["auto_start"], true);
        assert_eq!(object["future_owner"], json!({"version": 8}));
        drop(integration);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn concurrent_first_common_and_owner_writes_preserve_explicit_policy_and_unknown_data() {
    let (root, store) = profile("concurrent-first-write");
    let integration = Arc::new(HerdrIntegration::new());
    integration.configure_profile(store.clone()).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            store.update(|object, exists| {
                if !exists {
                    integration.initialize_lifecycle(object)?;
                }
                object.insert("auto_start".into(), Value::Bool(false));
                object.insert("future_owner".into(), json!({"nested": {"kept": 9}}));
                Ok(())
            }).unwrap();
        });
        scope.spawn(|| {
            barrier.wait();
            integration.apply_setting(&store, "exit_with_herdr", false).unwrap();
        });
    });
    let (object, _) = store.read_object_with_presence().unwrap();
    assert_eq!(object["auto_start"], false);
    assert_eq!(object["exit_with_herdr"], false);
    assert_eq!(object["future_owner"], json!({"nested": {"kept": 9}}));
    integration.configure_profile(store.clone()).unwrap();
    assert_eq!(integration.legacy_status_fields().unwrap()["exit_with_herdr"], false);
    drop(integration);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn lifecycle_and_endpoint_writes_preserve_common_and_unknown_keys() {
    let (root,store)=profile("lifecycle");
    store.update(|object,_| { object.insert("auto_start".into(),Value::Bool(false)); object.insert("future_owner".into(),json!({"version":7})); Ok(()) }).unwrap();
    let integration=HerdrIntegration::new(); integration.configure_profile(store.clone()).unwrap();
    integration.apply_setting(&store,"exit_with_herdr",true).unwrap();
    let endpoint=crate::socket::canonical_endpoint(&root.join("endpoint.sock")).unwrap();
    assert!(settings::add_endpoint(&store,&endpoint).unwrap());
    assert!(!settings::add_endpoint(&store,&endpoint).unwrap());
    let (object,_)=store.read_object_with_presence().unwrap();
    assert_eq!(object["auto_start"],false); assert_eq!(object["future_owner"],json!({"version":7})); assert_eq!(object["exit_with_herdr"],true);
    assert_eq!(settings::read_endpoints(&store).unwrap(),vec![endpoint]);
    drop(integration); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn generic_preference_patch_is_pure_save_first_and_retains_opaque_fields() {
    let (root,store)=profile("preferences");
    let integration=HerdrIntegration::new(); integration.configure_profile(store.clone()).unwrap();
    let mut candidate:Map<String,Value>=serde_json::from_value(json!({"visible":false,"future_top":{"kept":true},"observation":{"host":"auto","local":true,"remote":false,"machines":[],"future_scope":{"token":9}}})).unwrap();
    let original=candidate.clone();
    let patch=serde_json::from_value(json!({"observation_remote":true})).unwrap();
    let prepared=integration.prepare_preference_patch(&patch).unwrap();
    prepared.apply(&mut candidate).unwrap();
    assert!(!integration.shared.registry.lock().unwrap().preferences.remote,"prepare/apply changed live policy before save");
    assert_eq!(candidate["visible"],original["visible"]); assert_eq!(candidate["future_top"],original["future_top"]);
    assert_eq!(candidate["observation"]["host"],"auto"); assert_eq!(candidate["observation"]["future_scope"],json!({"token":9}));
    let bytes=serde_json::to_vec_pretty(&candidate).unwrap();
    store.with_writer_lock(|| desktop_pet_contracts::profile_store::atomic_write(&store.config_dir().join("preferences.json"),&bytes)).unwrap();
    prepared.committed(&candidate).unwrap();
    assert!(integration.shared.registry.lock().unwrap().preferences.remote);
    let snapshot=integration.preference_snapshot(&candidate).unwrap(); assert_eq!(snapshot["observation_remote"],true);
    let bad=serde_json::from_value(json!({"observation_remote":"yes"})).unwrap(); assert!(integration.prepare_preference_patch(&bad).is_err());
    assert_eq!(std::fs::read(store.config_dir().join("preferences.json")).unwrap(),bytes);
    drop(prepared); drop(integration); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn legacy_machine_normalization_does_not_loosen_new_patch_validation() {
    let opaque=" machine:/🌲 /key=abc ";
    let saved=serde_json::from_value(json!({"observation":{"machines":["",opaque,opaque,"invalid\0id"]}})).unwrap();
    assert_eq!(settings::preferences_from_object(&saved).unwrap().machines,vec![opaque.to_owned()]);
    let (integration,_)=harness::integration();
    let patch=serde_json::from_value(json!({"observation_machines":[opaque,opaque]})).unwrap();
    assert!(integration.prepare_preference_patch(&patch).is_err());
}
#[test]
fn hidden_local_reselection_keeps_session_key_and_watcher_evidence() {
    let (integration,runtime)=harness::integration(); let source="/tmp/local-observation.sock";
    let (root,store)=profile("visibility"); integration.configure_profile(store.clone()).unwrap();
    let key=harness::publish(&integration,&runtime,source,1,Route::Local(PathBuf::from(source)),&[harness::record("terminal","pane")]);
    for (local,expected) in [(false,false),(true,true)] {
        let patch=serde_json::from_value(json!({"observation_local":local})).unwrap(); let prepared=integration.prepare_preference_patch(&patch).unwrap();
        let mut saved=Map::new(); prepared.apply(&mut saved).unwrap();
        store.with_writer_lock(|| desktop_pet_contracts::profile_store::atomic_write(&store.config_dir().join("preferences.json"),&serde_json::to_vec(&saved).unwrap())).unwrap();
        prepared.committed(&saved).unwrap();
        let domain=runtime.domain.lock().unwrap(); assert_eq!(domain.key(source,"terminal"),key); assert_eq!(domain.sources[source].observed,expected); assert!(domain.sources[source].connected);
        assert_eq!(domain.prompt_route(&key).is_ok(),expected);
    }
    drop(integration); drop(store); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn lifecycle_exit_applies_only_to_selected_herdr_and_reentry_gets_fresh_grace() {
    let (integration,_)=harness::integration(); let start=Instant::now();
    assert!(!integration.should_exit(&ObservationSelection::Auto,start+DISCONNECTION_GRACE*5));
    assert!(!integration.should_exit(&ObservationSelection::Provider("tern".into()),start+DISCONNECTION_GRACE*6));
    let herdr=ObservationSelection::Provider("herdr".into()); let selected=start+DISCONNECTION_GRACE*7;
    assert!(!integration.should_exit(&herdr,selected)); assert!(!integration.should_exit(&herdr,selected+DISCONNECTION_GRACE-Duration::from_millis(1)));
    assert!(integration.should_exit(&herdr,selected+DISCONNECTION_GRACE));
}
#[test]
fn expired_offline_sources_with_pending_work_are_not_reclaimed() {
    let (integration,runtime)=harness::integration(); let source="/tmp/pending-retention.sock";
    let key=harness::publish(&integration,&runtime,source,1,Route::Local(PathBuf::from(source)),&[harness::record("terminal","pane")]);
    runtime.domain.lock().unwrap().mark_source_offline(source,1);
    let (reply,receiver)=mpsc::channel();
    {
        let mut registry=integration.shared.registry.lock().unwrap();
        let epoch=registry.epoch;
        let binding=registry.bindings.get_mut(source).unwrap(); binding.connected=false; binding.offline_since=Some(Instant::now()-DISCONNECTION_GRACE*2);
        registry.operations.insert(1,PendingOperation {source:source.into(),generation:key.generation,epoch,instance:"test-instance".into(),offered:true,cancelled:Arc::new(AtomicBool::new(false)),completion:Completion::Prompt(reply)});
    }
    runtime.with_domain(&mut |domain| assert_eq!(integration.reclaim_expired_sources(Instant::now(),domain),0)).unwrap();
    assert!(runtime.domain.lock().unwrap().sources.contains_key(source));
    integration.shared.registry.lock().unwrap().operations.remove(&1);
    drop(receiver);
    runtime.with_domain(&mut |domain| assert_eq!(integration.reclaim_expired_sources(Instant::now(),domain),1)).unwrap();
    assert!(!runtime.domain.lock().unwrap().sources.contains_key(source));
}
#[test]
fn disabled_start_runs_no_herdr_socket_transport() {
    use std::os::unix::net::UnixListener;
    let (root,store)=profile("dormant-start"); let path=root.join("host.sock"); let listener=UnixListener::bind(&path).unwrap(); listener.set_nonblocking(true).unwrap();
    let integration=HerdrIntegration::new(); integration.configure_profile(store.clone()).unwrap();
    integration.configure_option("--herdr-socket",Some(path.as_os_str())).unwrap();
    let runtime=Arc::new(harness::Runtime::default()); let port:Arc<dyn RuntimePort>=runtime.clone(); integration.start(port).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert!(listener.accept().is_err()); assert!(runtime.domain.lock().unwrap().sources.is_empty());
    assert!(!integration.should_exit(&ObservationSelection::Provider("herdr".into()),Instant::now()+DISCONNECTION_GRACE*2));
    integration.shutdown(); drop(integration); drop(store); drop(listener); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn explicit_local_plugin_disable_overrides_remote_health_only_in_herdr_selection() {
    let (integration,runtime)=harness::integration();
    let local="/tmp/plugin-disabled.sock";
    harness::publish(&integration,&runtime,local,1,Route::Local(PathBuf::from(local)),&[harness::record("local","pane")]);
    let remote="machine:m";
    harness::publish(&integration,&runtime,remote,1,Route::Remote("m".into()),&[harness::record("remote","pane")]);
    {
        let mut registry=integration.shared.registry.lock().unwrap();
        registry.exit_with_herdr=false;
        let binding=registry.bindings.get_mut(local).unwrap(); binding.disabled=true; binding.connected=false;
    }
    assert!(!integration.should_exit(&ObservationSelection::Auto,Instant::now()));
    assert!(!integration.should_exit(&ObservationSelection::Provider("tern".into()),Instant::now()));
    assert!(integration.should_exit(&ObservationSelection::Provider("herdr".into()),Instant::now()));
}

#[test]
fn late_preference_commit_cannot_overwrite_newer_applied_owner_policy() {
    let (root,store)=profile("ordered-commit");
    let (integration,runtime)=harness::integration();
    integration.configure_profile(store.clone()).unwrap();
    let source="/tmp/ordered-policy.sock";
    let key=harness::publish(&integration,&runtime,source,1,Route::Local(PathBuf::from(source)),&[harness::record("terminal","pane")]);
    let older_patch=serde_json::from_value(json!({"observation_local":false})).unwrap();
    let newer_patch=serde_json::from_value(json!({"observation_local":true})).unwrap();
    let older=integration.prepare_preference_patch(&older_patch).unwrap();
    let newer=integration.prepare_preference_patch(&newer_patch).unwrap();
    let mut saved_a=Map::new(); older.apply(&mut saved_a).unwrap();
    let mut saved_b=saved_a.clone(); newer.apply(&mut saved_b).unwrap();
    let persist=|saved:&Map<String,Value>| store.with_writer_lock(|| {
        desktop_pet_contracts::profile_store::atomic_write(&store.config_dir().join("preferences.json"),&serde_json::to_vec(saved).unwrap())
    });
    persist(&saved_a).unwrap();
    persist(&saved_b).unwrap(); newer.committed(&saved_b).unwrap();
    // A's delayed post-save callback arrives after B has saved and applied.
    older.committed(&saved_a).unwrap();
    assert!(integration.shared.registry.lock().unwrap().preferences.local);
    assert!(runtime.domain.lock().unwrap().prompt_route(&key).is_ok());
    assert_eq!(runtime.domain.lock().unwrap().key(source,"terminal"),key);
    assert_eq!(settings::read_preferences(&store).unwrap().local,true);
    drop(older); drop(newer); drop(integration); drop(store); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn selecting_another_provider_hides_herdr_scope_without_changing_policy() {
    let (root, store) = profile("provider-scope");
    let integration = HerdrIntegration::new();
    integration.configure_profile(store.clone()).unwrap();
    integration.apply_setting(&store, "exit_with_herdr", true).unwrap();
    integration.apply_setting(&store, "observation_local", false).unwrap();
    let hidden = integration.settings(&store, &ObservationSelection::Provider("tern".into()), "ja").unwrap();
    assert!(hidden.iter().all(|view| !view.key.starts_with("observation_")));
    let exit = hidden.iter().find(|view| view.key == "exit_with_herdr").unwrap();
    assert!(exit.value && !exit.enabled);
    let restored = integration.settings(&store, &ObservationSelection::Auto, "ja").unwrap();
    let local = restored.iter().find(|view| view.key == "observation_local").unwrap();
    assert!(local.enabled && !local.value);
    drop(integration);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
