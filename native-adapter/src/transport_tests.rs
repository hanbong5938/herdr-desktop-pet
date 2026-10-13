use crate::harness::{self, record};
use crate::{HerdrIntegration, Route};
use desktop_pet_contracts::*;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    struct Server {
        path: PathBuf,
        listener: UnixListener,
    }

    impl Server {
        fn new() -> Self {
            let path = PathBuf::from("/tmp").join(format!(
                "herdr-prompt-{}-{}.sock",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            let listener = UnixListener::bind(&path).unwrap();
            Self { path, listener }
        }

        fn source(&self) -> String {
            fs::canonicalize(&self.path)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        }

        fn exchange(&self, expected_method: &str, result: Value) -> Value {
            let (mut stream, _) = self.listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], expected_method);
            let response = json!({ "id": request["id"], "result": result });
            writeln!(stream, "{response}").unwrap();
            request
        }

        fn snapshot(&self, agents: &[(&str, &str)]) {
            let panes: Vec<_> = agents
                .iter()
                .map(|(_, pane)| json!({"pane_id": pane}))
                .collect();
            let agents: Vec<_> = agents
                .iter()
                .map(|(terminal, pane)| {
                    json!({"terminal_id": terminal, "pane_id": pane, "agent_status": "idle"})
                })
                .collect();
            self.exchange(
                "session.snapshot",
                json!({"type": "session_snapshot", "snapshot": {"panes": panes, "agents": agents}}),
            );
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }
    fn worktree(
        checkout: &str,
        panes: usize,
        tabs: usize,
        linked: bool,
    ) -> Arc<WorkspaceWorktreeInfo> {
        Arc::new(WorkspaceWorktreeInfo {
            repo_key: "repo-key".into(),
            repo_name: "Repo".into(),
            repo_root: "/virtual/repo".into(),
            checkout_path: checkout.into(),
            is_linked_worktree: linked,
            pane_count: panes,
            tab_count: tabs,
        })
    }

    fn workspace(id: &str, info: &WorkspaceWorktreeInfo) -> Value {
        json!({
            "workspace_id": id, "label": "Checkout",
            "pane_count": info.pane_count, "tab_count": info.tab_count,
            "worktree": {
                "repo_key": info.repo_key, "repo_name": info.repo_name,
                "repo_root": info.repo_root, "checkout_path": info.checkout_path,
                "is_linked_worktree": info.is_linked_worktree
            }
        })
    }

    fn agent(terminal: &str, pane: &str, workspace: &str) -> Value {
        json!({
            "terminal_id": terminal, "pane_id": pane,
            "workspace_id": workspace, "agent_status": "idle"
        })
    }

    fn worktree_record(
        terminal: &str,
        pane: &str,
        workspace: &str,
        info: Arc<WorkspaceWorktreeInfo>,
    ) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal.into(),
            pane_id: pane.into(),
            status: AgentStatus::Idle,
            metadata: SessionMetadata {
                workspace_id: Some(workspace.into()),
                worktree: Some(info),
                ..SessionMetadata::default()
            },
            outcome_authoritative: false,
            outcome: None,
        }
    }

fn publish_local(integration: &HerdrIntegration, runtime: &harness::Runtime, server: &Server, records: &[AgentRecord]) -> SessionKey {
    let source = server.source();
    harness::publish(integration,runtime,&source,1,Route::Local(PathBuf::from(&source)),records)
}
fn receive<T>(receiver: mpsc::Receiver<T>) -> T {
    receiver.recv_timeout(Duration::from_secs(5)).expect("bounded adapter receipt")
}

#[test]
fn prompt_routes_selected_source_and_preserves_original_multiline_text() {
    let selected = Server::new(); let other = Server::new();
    let (integration,runtime) = harness::integration();
    publish_local(&integration,&runtime,&other,&[record("other-terminal","same-pane")]);
    let key = publish_local(&integration,&runtime,&selected,&[record("selected-terminal","same-pane")]);
    let worker = thread::spawn(move || {
        selected.snapshot(&[("selected-terminal","same-pane")]);
        let request = selected.exchange("agent.prompt",json!({"type":"agent_prompted","agent":{"terminal_id":"selected-terminal","pane_id":"same-pane"}}));
        assert_eq!(request["params"]["target"],"same-pane");
        assert_eq!(request["params"]["text"],"  first\nsecond  ");
        assert!(request["params"].get("wait").is_none());
    });
    let receipt = integration.submit_prompt(key,"  first\nsecond  ".to_owned()).unwrap();
    assert_eq!(receive(receipt),Ok(PromptReceipt::Acknowledged));
    worker.join().unwrap();
    other.listener.set_nonblocking(true).unwrap();
    assert!(other.listener.accept().is_err());
    assert!(runtime.domain.lock().unwrap().outcomes.is_empty());
    assert!(runtime.domain.lock().unwrap().completions.is_empty());
    integration.shutdown();
}

#[test]
fn fresh_snapshot_requires_unique_matching_terminal_and_pane() {
    for pairs in [vec![("replacement","pane")],vec![("terminal","pane"),("terminal","other")],vec![("terminal","pane"),("other","pane")]] {
        let server = Server::new(); let (integration,runtime) = harness::integration();
        let key = publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
        let worker = thread::spawn(move || {
            server.snapshot(&pairs);
            server.listener.set_nonblocking(true).unwrap();
            assert!(server.listener.accept().is_err(),"stale prompt was transmitted");
        });
        assert_eq!(receive(integration.submit_prompt(key,"message".into()).unwrap()),Err(PromptError::StaleTarget));
        worker.join().unwrap(); integration.shutdown();
    }
}

#[test]
fn prompt_maps_server_rejection_and_uncertain_delivery_without_resend() {
    for (code,expected) in [("agent_blocked",PromptError::Blocked),("agent_not_ready",PromptError::NotReady),("unsupported_method",PromptError::Unsupported)] {
        let server=Server::new(); let (integration,runtime)=harness::integration();
        let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
        let worker=thread::spawn(move || {
            server.snapshot(&[("terminal","pane")]);
            let (mut stream,_)=server.listener.accept().unwrap();
            let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
            let request:Value=serde_json::from_str(&line).unwrap();
            writeln!(stream,"{}",json!({"id":request["id"],"error":{"code":code,"message":"rejected"}})).unwrap();
        });
        assert_eq!(receive(integration.submit_prompt(key,"text".into()).unwrap()),Err(expected));
        worker.join().unwrap(); integration.shutdown();
    }
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    let worker=thread::spawn(move || {
        server.snapshot(&[("terminal","pane")]);
        let (stream,_)=server.listener.accept().unwrap();
        let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
        let request:Value=serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"],"agent.prompt"); drop(stream);
        server.listener.set_nonblocking(true).unwrap();
        assert!(server.listener.accept().is_err(),"prompt was resent");
    });
    assert_eq!(receive(integration.submit_prompt(key,"do not duplicate".into()).unwrap()),Err(PromptError::UnknownDelivery));
    worker.join().unwrap(); integration.shutdown();
}

#[test]
fn prompt_does_not_report_success_for_wrong_ack_identity() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    let worker=thread::spawn(move || {
        server.snapshot(&[("terminal","pane")]);
        server.exchange("agent.prompt",json!({"type":"agent_prompted","agent":{"terminal_id":"other","pane_id":"pane"}}));
    });
    assert_eq!(receive(integration.submit_prompt(key,"identity".into()).unwrap()),Err(PromptError::UnknownDelivery));
    worker.join().unwrap(); integration.shutdown();
}

#[test]
fn remote_descriptor_is_read_only_even_when_metadata_and_machine_id_are_local_paths() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let source=server.source();
    let row=worktree_record("terminal","pane","ws",worktree("/virtual/checkout",1,1,true));
    let key=harness::publish(&integration,&runtime,&source,1,Route::Remote(source.clone()),&[row.clone()]);
    assert!(matches!(integration.submit_prompt(key.clone(),"never send".into()),Err(PromptError::ReadOnly)));
    let target=WorktreeRemoveTarget { key,source,pane_id:"pane".into(),workspace_id:"ws".into(),worktree:row.metadata.worktree.unwrap() };
    assert!(matches!(integration.submit_worktree_remove(target),Err(WorktreeRemoveError::ReadOnly)));
    server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
}

#[test]
fn prompt_bounds_full_escaped_frame_and_final_prepare_admission() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    assert!(matches!(integration.submit_prompt(key.clone()," \n ".into()),Err(PromptError::Empty)));
    assert!(matches!(integration.submit_prompt(key.clone(),"\"".repeat(crate::protocol::MAX_FRAME_BYTES/2)),Err(PromptError::TooLarge)));
    runtime.domain.lock().unwrap().gate=false;
    assert!(matches!(integration.submit_prompt(key,"not admitted".into()),Err(PromptError::Other(_))));
    assert!(integration.shared.registry.lock().unwrap().operations.is_empty());
    server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
}

#[test]
fn retirement_during_fresh_snapshot_cancels_queued_prompt_without_contacting_agent() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    let (arrived,arrival)=mpsc::channel(); let (release,released)=mpsc::channel();
    let worker=thread::spawn(move || {
        let (mut stream,_)=server.listener.accept().unwrap();
        let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
        let request:Value=serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"],"session.snapshot"); arrived.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(3)).unwrap();
        writeln!(stream,"{}",json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":{"panes":[{"pane_id":"pane"}],"agents":[{"terminal_id":"terminal","pane_id":"pane","agent_status":"idle"}]}}})).unwrap();
        server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
    });
    let receipt=integration.submit_prompt(key,"queued".into()).unwrap();
    arrival.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(runtime.domain.try_lock().is_ok(),"host wait holds domain");
    runtime.with_domain(&mut |domain| integration.retire(domain)).unwrap();
    assert_eq!(receive(receipt),Err(PromptError::StaleTarget));
    release.send(()).unwrap(); worker.join().unwrap(); integration.shutdown();
}

#[test]
fn offered_prompt_retirement_is_unknown_delivery_and_never_replayed() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    let (arrived,arrival)=mpsc::channel(); let (release,released)=mpsc::channel();
    let worker=thread::spawn(move || {
        server.snapshot(&[("terminal","pane")]);
        let (mut stream,_)=server.listener.accept().unwrap();
        let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
        let request:Value=serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"],"agent.prompt"); arrived.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(3)).unwrap();
        let _=writeln!(stream,"{}",json!({"id":request["id"],"result":{"type":"agent_prompted","agent":{"terminal_id":"terminal","pane_id":"pane"}}}));
        server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err(),"retired ask was replayed");
    });
    let receipt=integration.submit_prompt(key,"one ask".into()).unwrap();
    arrival.recv_timeout(Duration::from_secs(3)).unwrap();
    runtime.with_domain(&mut |domain| integration.retire(domain)).unwrap();
    assert_eq!(receive(receipt),Err(PromptError::UnknownDelivery));
    runtime.with_domain(&mut |domain| integration.set_enabled(true,domain)).unwrap();
    release.send(()).unwrap(); worker.join().unwrap(); integration.shutdown();
}

#[test]
fn worktree_force_false_and_exact_response_path_are_required() {
    for result in [
        json!({"type":"worktree_removed","workspace_id":"ws","path":"/virtual/checkout","forced":false}),
        json!({"type":"worktree_removed","workspace_id":"ws","path":"/virtual/other","forced":false}),
        json!({"type":"worktree_removed","workspace_id":"ws","path":"/virtual/checkout","forced":true}),
    ] {
        let server=Server::new(); let (integration,runtime)=harness::integration();
        let info=worktree("/virtual/checkout",2,2,true);
        let key=publish_local(&integration,&runtime,&server,&[worktree_record("terminal","pane","ws",info.clone())]);
        let target=runtime.domain.lock().unwrap().worktree_remove_target(&key).unwrap();
        let expected=if result["path"]=="/virtual/checkout" && result["forced"]==false { Ok(()) } else { Err(WorktreeRemoveError::UnknownDelivery) };
        let worker=thread::spawn(move || {
            server.exchange("session.snapshot",json!({"type":"session_snapshot","snapshot":{"workspaces":[workspace("ws",&info)],"panes":[{"pane_id":"pane"},{"pane_id":"second"}],"agents":[agent("terminal","pane","ws"),agent("other","second","ws")]}}));
            let request=server.exchange("worktree.remove",result);
            assert_eq!(request["params"],json!({"workspace_id":"ws","force":false}));
        });
        assert_eq!(receive(integration.submit_worktree_remove(target).unwrap()),expected);
        worker.join().unwrap(); assert!(runtime.domain.lock().unwrap().sources.values().any(|source| source.id == key.source_id));
        integration.shutdown();
    }
}

#[test]
fn worktree_fresh_snapshot_impact_change_aborts_before_remove() {
    for (terminal,info) in [("replacement",worktree("/virtual/checkout",1,1,true)),("terminal",worktree("/virtual/checkout",1,2,true)),("terminal",worktree("/virtual/checkout",1,1,false))] {
        let server=Server::new(); let (integration,runtime)=harness::integration();
        let key=publish_local(&integration,&runtime,&server,&[worktree_record("terminal","pane","ws",worktree("/virtual/checkout",1,1,true))]);
        let target=runtime.domain.lock().unwrap().worktree_remove_target(&key).unwrap();
        let worker=thread::spawn(move || {
            server.exchange("session.snapshot",json!({"type":"session_snapshot","snapshot":{"workspaces":[workspace("ws",&info)],"panes":[{"pane_id":"pane"}],"agents":[agent(terminal,"pane","ws")]}}));
            server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
        });
        assert_eq!(receive(integration.submit_worktree_remove(target).unwrap()),Err(WorktreeRemoveError::StaleTarget));
        worker.join().unwrap(); integration.shutdown();
    }
}

#[test]
fn poisoned_domain_is_source_failure_without_callback_or_host_contact() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let key=publish_local(&integration,&runtime,&server,&[record("terminal","pane")]);
    let poisoned=runtime.clone();
    let _=thread::spawn(move || { let _guard=poisoned.domain.lock().unwrap(); panic!("poison fixture"); }).join();
    assert!(matches!(integration.submit_prompt(key,"never send".into()),Err(PromptError::Other(_))));
    assert!(integration.shared.registry.lock().unwrap().failure.as_ref().is_some_and(|error|error.contains("poisoned")));
    server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
}

#[test]
fn noncanonical_route_never_contacts_alias_for_prompt_or_worktree() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let canonical=PathBuf::from(server.source());
    let parent=canonical.parent().unwrap();
    let alias=parent.join("..").join(parent.file_name().unwrap()).join(canonical.file_name().unwrap());
    let source=alias.to_str().unwrap();
    let row=worktree_record("terminal","pane","ws",worktree("/virtual/checkout",1,1,true));
    let key=harness::publish(&integration,&runtime,source,1,Route::Local(alias.clone()),&[row]);
    assert_eq!(receive(integration.submit_prompt(key.clone(),"never send".into()).unwrap()),Err(PromptError::StaleTarget));
    let target=runtime.domain.lock().unwrap().worktree_remove_target(&key).unwrap();
    assert_eq!(receive(integration.submit_worktree_remove(target).unwrap()),Err(WorktreeRemoveError::StaleTarget));
    server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err()); integration.shutdown();
}

#[test]
fn worktree_state_change_during_snapshot_aborts_without_holding_domain() {
    let server=Server::new(); let (integration,runtime)=harness::integration();
    let info=worktree("/virtual/checkout",1,1,true);
    let key=publish_local(&integration,&runtime,&server,&[worktree_record("terminal","pane","ws",info.clone())]);
    let target=runtime.domain.lock().unwrap().worktree_remove_target(&key).unwrap();
    let (arrived,arrival)=mpsc::channel(); let (release,released)=mpsc::channel();
    let worker=thread::spawn(move || {
        let (mut stream,_)=server.listener.accept().unwrap(); let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
        let request:Value=serde_json::from_str(&line).unwrap(); assert_eq!(request["method"],"session.snapshot");
        arrived.send(()).unwrap(); released.recv_timeout(Duration::from_secs(3)).unwrap();
        writeln!(stream,"{}",json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":{"workspaces":[workspace("ws",&info)],"panes":[{"pane_id":"pane"}],"agents":[agent("terminal","pane","ws")]}}})).unwrap();
        server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err());
    });
    let receipt=integration.submit_worktree_remove(target.clone()).unwrap(); arrival.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(runtime.domain.try_lock().is_ok());
    let changed=worktree_record("terminal","pane","ws",worktree("/virtual/checkout",2,1,true));
    runtime.with_domain(&mut |domain| {
        assert!(domain.publish_source_snapshot(&target.source,1,&mut [&changed].into_iter(),SourceCounts::default(),Arc::from(vec!["pane".into()]),&[],Instant::now()));
    }).unwrap();
    release.send(()).unwrap(); assert_eq!(receive(receipt),Err(WorktreeRemoveError::StaleTarget)); worker.join().unwrap(); integration.shutdown();
}

#[test]
fn worktree_server_rejection_and_uncertain_disconnect_are_never_replayed() {
    for (code,expected) in [
        (Some("dirty_worktree"),WorktreeRemoveError::Rejected {code:"dirty_worktree".into(),message:"rejected".into()}),
        (Some("unsupported_method"),WorktreeRemoveError::Unsupported),
        (None,WorktreeRemoveError::UnknownDelivery),
    ] {
        let server=Server::new(); let (integration,runtime)=harness::integration(); let info=worktree("/virtual/checkout",1,1,true);
        let key=publish_local(&integration,&runtime,&server,&[worktree_record("terminal","pane","ws",info.clone())]);
        let target=runtime.domain.lock().unwrap().worktree_remove_target(&key).unwrap();
        let worker=thread::spawn(move || {
            server.exchange("session.snapshot",json!({"type":"session_snapshot","snapshot":{"workspaces":[workspace("ws",&info)],"panes":[{"pane_id":"pane"}],"agents":[agent("terminal","pane","ws")]}}));
            let (mut stream,_)=server.listener.accept().unwrap(); let mut line=String::new(); BufReader::new(&stream).read_line(&mut line).unwrap();
            let request:Value=serde_json::from_str(&line).unwrap(); assert_eq!(request["method"],"worktree.remove");
            if let Some(code)=code { writeln!(stream,"{}",json!({"id":request["id"],"error":{"code":code,"message":"rejected"}})).unwrap(); }
            drop(stream); server.listener.set_nonblocking(true).unwrap(); assert!(server.listener.accept().is_err(),"destructive request replayed");
        });
        assert_eq!(receive(integration.submit_worktree_remove(target).unwrap()),Err(expected)); worker.join().unwrap(); integration.shutdown();
    }
}
