use super::*;
    fn record(terminal_id: &str, pane_id: &str, status: AgentStatus) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal_id.to_owned(),
            pane_id: pane_id.to_owned(),
            status,
            metadata: desktop_pet_contracts::SessionMetadata::default(),
            outcome_authoritative: false,
            outcome: None,
        }
    }

    fn snapshot(panes: &[&str], agents: Vec<AgentRecord>) -> Snapshot {
        Snapshot {
            panes: panes.iter().map(|pane| (*pane).to_owned()).collect(),
            agents,
        }
    }

    #[test]
    fn startup_and_reconnect_snapshots_are_baselines() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Done)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Done)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn only_active_to_done_transition_is_observed_once() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: true,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: false,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Unknown)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: false,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
    }

    #[test]
    fn ambiguous_pane_events_do_not_update_or_complete() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![
                record("terminal-a", "pane-a", AgentStatus::Working),
                record("terminal-b", "pane-a", AgentStatus::Blocked),
            ],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Ambiguous
        );
        let counts = endpoint.counts();
        assert_eq!(counts.working, 1);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.done, 0);
    }

    #[test]
    fn duplicate_terminal_baseline_is_disarmed_until_unique_baseline() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![
                record("terminal-a", "pane-a", AgentStatus::Working),
                record("terminal-a", "pane-a", AgentStatus::Working),
            ],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Ambiguous
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        let counts = source_counts(endpoint.counts());
        assert_eq!(
            apply_event_records(
                &mut endpoint.agents,
                &endpoint.ambiguous_terminal_ids,
                "pane-a",
                AgentStatus::Done,
                counts,
            ),
            EventRecordUpdate::Applied {
                terminal_id: "terminal-a",
                completion: true,
                counts: SourceCounts {
                    sessions: 1,
                    done: 1,
                    ..SourceCounts::default()
                },
            }
        );
    }
    #[test]
    fn snapshot_remap_and_new_or_unknown_sessions_do_not_complete() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a", "pane-b"],
            vec![record("terminal-a", "pane-a", AgentStatus::Working)],
        ));
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a", "pane-b"],
                vec![record("terminal-a", "pane-b", AgentStatus::Done)],
            ))
            .is_empty());
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a", "pane-b"],
                vec![record("terminal-b", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn coherent_snapshot_transition_completes_once() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.replace_snapshot(snapshot(
            &["pane-a"],
            vec![record("terminal-a", "pane-a", AgentStatus::Blocked)],
        ));
        assert_eq!(
            endpoint.reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            )),
            vec![("terminal-a".to_owned(), "pane-a".to_owned())]
        );
        assert!(endpoint
            .reconcile_snapshot(snapshot(
                &["pane-a"],
                vec![record("terminal-a", "pane-a", AgentStatus::Done)],
            ))
            .is_empty());
    }

    #[test]
    fn metadata_changes_never_complete_or_fabricate_omp_outcomes() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        let mut original = record("terminal", "pane", AgentStatus::Working);
        original.outcome_authoritative = true;
        original.metadata.title = Some("Split README".to_owned());
        endpoint.replace_snapshot(snapshot(&["pane"], vec![original.clone()]));
        let mut changed = original.clone();
        changed.metadata.title = Some("Different title".to_owned());
        changed.metadata.workspace_label = Some("Idea".to_owned());
        assert!(endpoint
            .reconcile_snapshot(snapshot(&["pane"], vec![changed.clone()]))
            .is_empty());
        assert_eq!(endpoint.agents["terminal"].metadata, changed.metadata);
        changed.status = AgentStatus::Done;
        assert!(endpoint
            .reconcile_snapshot(snapshot(&["pane"], vec![changed]))
            .is_empty());
        assert!(endpoint.agents["terminal"].outcome.is_none());
    }

    #[test]
    fn endpoint_counts_do_not_use_provider_specific_fields() {
        let mut endpoint = Endpoint::with_registry(
            PathBuf::from("/tmp/herdr.sock"),
            crate::harness::integration().0.shared.clone(),
        );
        endpoint.agents.insert(
            "terminal-a".to_owned(),
            AgentRecord {
                terminal_id: "terminal-a".to_owned(),
                pane_id: "pane-a".to_owned(),
                status: AgentStatus::Working,
                metadata: desktop_pet_contracts::SessionMetadata::default(),
                outcome_authoritative: false,
                outcome: None,
            },
        );
        endpoint.agents.insert(
            "terminal-b".to_owned(),
            AgentRecord {
                terminal_id: "terminal-b".to_owned(),
                pane_id: "pane-b".to_owned(),
                status: AgentStatus::Done,
                metadata: desktop_pet_contracts::SessionMetadata::default(),
                outcome_authoritative: false,
                outcome: None,
            },
        );
        let counts = endpoint.counts();
        assert_eq!(counts.sessions, 2);
        assert_eq!(counts.working, 1);
        assert_eq!(counts.done, 1);
    }

fn observed_endpoint() -> (Arc<crate::HerdrIntegration>, Arc<crate::harness::Runtime>, Endpoint) {
    let (integration,runtime) = crate::harness::integration();
    let mut endpoint=Endpoint::with_registry(PathBuf::from("/tmp/outcome-source.sock"),integration.shared.clone());
    assert!(endpoint.reconcile_observation(&integration.shared,Instant::now()));
    assert!(endpoint.begin_generation(&integration.shared));
    (integration,runtime,endpoint)
}
fn reported(outcome: AgentOutcome, turn: u64, at: u64) -> AgentRecord {
    let mut row=record("terminal","pane",AgentStatus::Working);
    row.outcome_authoritative=true;
    row.outcome=Some(OutcomeReport { session:"a".repeat(64),turn:turn.to_string(),outcome,at_unix_ms:at });
    row
}
fn publish_at(endpoint:&mut Endpoint, integration:&crate::HerdrIntegration, rows:Vec<AgentRecord>, at:u64) {
    let completions=endpoint.reconcile_snapshot(snapshot(&["pane"],rows));
    endpoint.publish_snapshot_at(&integration.shared,&completions,Instant::now(),at);
}

#[test]
fn authoritative_outcomes_do_not_infer_success_or_replay_across_generations() {
    let (integration,runtime,mut endpoint)=observed_endpoint();
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Succeeded,1,1000)],1000);
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Running,2,1100)],1100);
    let mut done=reported(AgentOutcome::Running,2,1100); done.status=AgentStatus::Done;
    publish_at(&mut endpoint,&integration,vec![done],1200);
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Failed,2,1300)],1300);
    assert_eq!(runtime.domain.lock().unwrap().outcomes.len(),1);
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Failed,2,1300)],1400);
    assert_eq!(runtime.domain.lock().unwrap().outcomes.len(),1);
    runtime.domain.lock().unwrap().outcomes.clear();
    assert!(endpoint.begin_generation(&integration.shared));
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Failed,2,1300)],1500);
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
}

#[test]
fn short_turn_outcome_survives_coalesced_running_metadata() {
    let (integration,runtime,mut endpoint)=observed_endpoint();
    let mut plain=record("terminal","pane",AgentStatus::Idle); plain.outcome_authoritative=true;
    publish_at(&mut endpoint,&integration,vec![plain],1000);
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Succeeded,1,1500)],1600);
    assert_eq!(runtime.domain.lock().unwrap().outcomes[0].outcome,AgentOutcome::Succeeded);
}

#[test]
fn ambiguous_terminal_consumption_is_order_independent_and_never_replayed() {
    for reverse in [false,true] {
        let (integration,runtime,mut endpoint)=observed_endpoint();
        publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Running,1,1000)],1000);
        let succeeded=reported(AgentOutcome::Succeeded,1,1200);
        let mut other=succeeded.clone(); other.pane_id="other".into();
        let rows=if reverse {vec![other.clone(),succeeded.clone()]} else {vec![succeeded.clone(),other]};
        endpoint.replace_snapshot(snapshot(&["pane","other"],rows));
        endpoint.publish_snapshot_at(&integration.shared,&[],Instant::now(),1300);
        assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
        endpoint.replace_snapshot(snapshot(&["pane"],vec![succeeded.clone()]));
        endpoint.publish_snapshot_at(&integration.shared,&[],Instant::now(),1400);
        assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
        publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Running,2,1500)],1500);
        publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Succeeded,2,1700)],1800);
        assert_eq!(runtime.domain.lock().unwrap().outcomes.len(),1);
    }
}

#[test]
fn temporary_authority_loss_retains_outcome_consumption() {
    let (integration,runtime,mut endpoint)=observed_endpoint();
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Running,1,1000)],1000);
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Failed,1,1200)],1200);
    assert_eq!(runtime.domain.lock().unwrap().outcomes.len(),1);
    runtime.domain.lock().unwrap().outcomes.clear();
    publish_at(&mut endpoint,&integration,vec![record("terminal","pane",AgentStatus::Idle)],1300);
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
    publish_at(&mut endpoint,&integration,vec![reported(AgentOutcome::Failed,1,1200)],1400);
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
}

#[test]
fn denied_admission_retries_only_next_coherent_snapshot_without_live_eviction() {
    let (integration,runtime)=crate::harness::integration();
    {
        let mut domain=runtime.domain.lock().unwrap();
        for index in 0..64 {
            let id=format!("other:{index}");
            assert!(domain.begin_source(SourceDescriptor { id:id.clone(),provider:"another-provider".into(),label:id,locality:SourceLocality::Local,capabilities:SourceCapabilities::default() },1));
        }
    }
    let mut endpoint=Endpoint::with_registry(PathBuf::from("/tmp/capacity-source.sock"),integration.shared.clone());
    assert!(endpoint.reconcile_observation(&integration.shared,Instant::now()));
    assert!(endpoint.begin_generation(&integration.shared));
    assert!(endpoint.source_admission_pending);
    publish_at(&mut endpoint,&integration,vec![record("terminal","pane",AgentStatus::Working)],1000);
    publish_at(&mut endpoint,&integration,vec![record("terminal","pane",AgentStatus::Done)],1200);
    assert_eq!(runtime.domain.lock().unwrap().sources.len(),64);
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
    runtime.domain.lock().unwrap().remove_source("other:0",1);
    publish_at(&mut endpoint,&integration,vec![record("terminal","pane",AgentStatus::Done)],1400);
    assert!(!endpoint.source_admission_pending);
    assert_eq!(runtime.domain.lock().unwrap().sources.len(),64);
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
    runtime.domain.lock().unwrap().remove_source(&endpoint.source,endpoint.generation);
    publish_at(&mut endpoint,&integration,vec![record("terminal","pane",AgentStatus::Working)],1600);
    assert!(!runtime.domain.lock().unwrap().sources.contains_key(&endpoint.source));
}

#[test]
fn plugin_socket_membership_missing_enabled_disabled_and_reactivation_are_fenced() {
    use std::io::{BufRead,BufReader,Write};
    use std::os::unix::net::{UnixListener,UnixStream};
    use std::sync::mpsc;
    let (integration,runtime)=crate::harness::integration();
    let path=PathBuf::from(format!("/tmp/herdr-policy-{}-{}.sock",std::process::id(),crate::NEXT_OPERATION.fetch_add(1,Ordering::Relaxed)));
    let listener=UnixListener::bind(&path).unwrap();
    let canonical=crate::socket::canonical_endpoint(&path).unwrap();
    let (ready,started)=mpsc::sync_channel(0);
    let worker=thread::spawn(move || {
        let mut streams=Vec::<UnixStream>::new(); let mut plugin_queries=0;
        ready.send(()).unwrap();
        // Missing initial membership permits observation. Enabled then explicit
        // disable retires it. A later missing membership must not revive it.
        for _ in 0..11 {
            let (mut stream,_)=listener.accept().unwrap();
            let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
            let request:serde_json::Value=serde_json::from_str(&line).unwrap();
            let result=match request["method"].as_str().unwrap() {
                "ping"=>serde_json::json!({"type":"pong"}),
                "session.snapshot"=>serde_json::json!({"type":"session_snapshot","snapshot":{"panes":[{"pane_id":"pane"}],"agents":[{"terminal_id":"terminal","pane_id":"pane","agent_status":"idle"}]}}),
                "events.subscribe"=>serde_json::json!({"type":"subscription_started"}),
                "plugin.list"=> {
                    plugin_queries+=1;
                    let plugins=match plugin_queries {1|4=>serde_json::json!([]),2=>serde_json::json!([{"plugin_id":"desktop-pet","enabled":true}]),_=>serde_json::json!([{"plugin_id":"desktop-pet","enabled":false}])};
                    serde_json::json!({"type":"plugin_list","plugins":plugins})
                }
                method=>panic!("unexpected {method}"),
            };
            writeln!(stream,"{}",serde_json::json!({"id":request["id"],"result":result})).unwrap();
            if request["method"]=="events.subscribe" {streams.push(stream);}
        }
        streams
    });
    started.recv().unwrap();
    let mut endpoint=Endpoint::with_registry(canonical.clone(),integration.shared.clone());
    endpoint.reconcile_observation(&integration.shared,Instant::now());
    let mut counter=0;
    connect_endpoint(&mut endpoint,&integration.shared,&mut counter,Instant::now());
    assert!(runtime.domain.lock().unwrap().sources[&endpoint.source].connected);
    reconcile_endpoint(&mut endpoint,&integration.shared,&mut counter,Instant::now());
    assert!(endpoint.seen_plugin_present);
    reconcile_endpoint(&mut endpoint,&integration.shared,&mut counter,Instant::now());
    assert!(endpoint.disabled);
    assert!(!runtime.domain.lock().unwrap().sources.contains_key(&endpoint.source));
    endpoint.disabled=false;
    connect_endpoint(&mut endpoint,&integration.shared,&mut counter,Instant::now());
    assert!(endpoint.disabled,"missing plugin after prior membership must stay detached");
    let streams=worker.join().unwrap(); drop(streams);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn retired_remote_worker_cannot_publish_or_fail_a_new_binding() {
    let (integration,runtime)=crate::harness::integration();
    let source="machine:m";
    crate::harness::publish(&integration,&runtime,source,1,Route::Remote("m".into()),&[record("old","pane",AgentStatus::Working)]);
    let epoch=integration.shared.epoch().unwrap();
    let mut old=Endpoint::remote(integration.shared.clone(),source.into(),1,epoch);
    runtime.with_domain(&mut |domain| integration.retire(domain)).unwrap();
    runtime.with_domain(&mut |domain| integration.set_enabled(true,domain)).unwrap();
    crate::harness::publish(&integration,&runtime,source,2,Route::Remote("m".into()),&[record("new","pane",AgentStatus::Idle)]);
    old.remote_snapshot(&integration.shared,snapshot(&["pane"],vec![record("old","pane",AgentStatus::Done)]),Instant::now(),false);
    let domain=runtime.domain.lock().unwrap();
    assert_eq!(domain.sources[source].generation,2); assert_eq!(domain.sources[source].records[0].terminal_id,"new"); assert!(domain.sources[source].connected);
    assert!(domain.completions.is_empty()); drop(domain);
    let registry=integration.shared.registry.lock().unwrap(); assert!(registry.bindings[source].connected); assert!(registry.failure.is_none());
}

#[test]
fn remote_first_and_reconnected_snapshots_are_baselines_but_new_transitions_complete() {
    let (integration,runtime)=crate::harness::integration(); let source="machine:m";
    {
        let mut registry=integration.shared.registry.lock().unwrap();
        registry.preferences.remote=true; registry.preferences.machines=vec!["m".into()];
    }
    crate::harness::publish(&integration,&runtime,source,1,Route::Remote("m".into()),&[record("terminal","pane",AgentStatus::Working)]);
    let mut endpoint=Endpoint::remote(integration.shared.clone(),source.into(),1,integration.shared.epoch().unwrap());
    endpoint.remote_snapshot(&integration.shared,snapshot(&["pane"],vec![record("terminal","pane",AgentStatus::Done)]),Instant::now(),true);
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
    endpoint.remote_snapshot(&integration.shared,snapshot(&["pane"],vec![record("terminal","pane",AgentStatus::Working)]),Instant::now(),false);
    endpoint.remote_snapshot(&integration.shared,snapshot(&["pane"],vec![record("terminal","pane",AgentStatus::Done)]),Instant::now(),false);
    assert_eq!(runtime.domain.lock().unwrap().completions.len(),1);
    endpoint.remote_offline(&integration.shared);
    assert!(!runtime.domain.lock().unwrap().sources[source].connected);
    crate::harness::publish(&integration,&runtime,source,2,Route::Remote("m".into()),&[record("terminal","pane",AgentStatus::Working)]);
    let mut reconnected=Endpoint::remote(integration.shared.clone(),source.into(),2,integration.shared.epoch().unwrap());
    reconnected.remote_snapshot(&integration.shared,snapshot(&["pane"],vec![record("terminal","pane",AgentStatus::Done)]),Instant::now(),true);
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
}
