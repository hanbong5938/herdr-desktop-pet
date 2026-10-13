use crate::socket::read_with_deadline;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn enlarge_socket_buffers(stream: &UnixStream) {
    let size: libc::c_int = 64 * 1024;
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        let result = unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&size as *const libc::c_int).cast(),
                std::mem::size_of_val(&size) as libc::socklen_t,
            )
        };
        assert_eq!(
            result,
            0,
            "setsockopt({option}) failed: {}",
            io::Error::last_os_error()
        );
    }
}

fn is_blocking(stream: &UnixStream) -> bool {
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
    assert!(
        flags >= 0,
        "fcntl(F_GETFL) failed: {}",
        io::Error::last_os_error()
    );
    flags & libc::O_NONBLOCK == 0
}

#[test]
fn drains_buffered_payload_after_peer_close_until_eof() {
    let (mut reader, mut writer) = UnixStream::pair().expect("UnixStream::pair");
    enlarge_socket_buffers(&reader);
    enlarge_socket_buffers(&writer);
    reader.set_nonblocking(false).expect("reader blocking mode");

    let payload: Vec<u8> = (0..20_000).map(|index| (index % 251) as u8).collect();
    writer.write_all(&payload).expect("preload payload");
    drop(writer);

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = Vec::with_capacity(payload.len());
    let mut chunk = [0u8; 8192];
    loop {
        let count = read_with_deadline(&mut reader, &mut chunk, deadline)
            .expect("read buffered payload or EOF");
        if count == 0 {
            break;
        }
        received.extend_from_slice(&chunk[..count]);
    }

    assert_eq!(received, payload);
    assert!(
        is_blocking(&reader),
        "successful read changed blocking mode"
    );
}

#[test]
fn timeout_is_bounded_and_restores_original_blocking_mode() {
    let (mut reader, _writer) = UnixStream::pair().expect("UnixStream::pair");
    reader.set_nonblocking(false).expect("reader blocking mode");
    assert!(is_blocking(&reader));

    let started = Instant::now();
    let deadline = started + Duration::from_millis(120);
    let error = read_with_deadline(&mut reader, &mut [0u8; 1], deadline)
        .expect_err("an idle peer must reach the read deadline");
    let elapsed = started.elapsed();

    assert!(
        matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ),
        "unexpected idle-read error: {error:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(60),
        "idle read returned before its deadline: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "idle read exceeded its bounded deadline: {elapsed:?}"
    );
    assert!(is_blocking(&reader), "timed-out read changed blocking mode");
}

#[test]
fn empty_buffer_returns_without_waiting() {
    let (mut reader, _writer) = UnixStream::pair().expect("UnixStream::pair");
    reader.set_nonblocking(false).expect("reader blocking mode");

    let started = Instant::now();
    let count = read_with_deadline(
        &mut reader,
        &mut [],
        Instant::now() + Duration::from_millis(250),
    )
    .expect("empty read");

    assert_eq!(count, 0);
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "empty read waited for socket readiness"
    );
    assert!(is_blocking(&reader), "empty read changed blocking mode");
}

mod prompt {
    use crate::herdr::{PromptError, PromptSender, RequestOrigin};
    use crate::session::{AgentRecord, AgentStatus, SessionMetadata};
    use crate::session_view::{SessionFilter, SessionKey, SessionListOptions};
    use crate::sources::{
        remote_source, MachineInfo, MachineStatus, ObservationPreferences, SourceCatalog,
    };
    use crate::state::{AppState, SourceCounts};
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
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

    fn record(terminal: &str, pane: &str) -> AgentRecord {
        AgentRecord {
            terminal_id: terminal.to_owned(),
            pane_id: pane.to_owned(),
            status: AgentStatus::Idle,
            metadata: SessionMetadata::default(),
            outcome_authoritative: false,
            outcome: None,
        }
    }

    fn publish(
        shared: &Arc<Mutex<AppState>>,
        source: &str,
        generation: u64,
        agents: &[AgentRecord],
    ) -> SessionKey {
        let mut state = shared.lock().unwrap();
        assert!(state.begin_source(source.to_owned(), generation));
        assert!(state.publish_source_snapshot(
            source,
            generation,
            agents,
            SourceCounts::default(),
            Arc::from(
                agents
                    .iter()
                    .map(|agent| agent.pane_id.clone())
                    .collect::<Vec<_>>()
            ),
            &[],
            Instant::now()
        ));
        state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows
            .into_iter()
            .find(|row| {
                row.key.generation == generation && row.key.terminal_id == agents[0].terminal_id
            })
            .unwrap()
            .key
    }

    fn receive(sender: &mut PromptSender) -> crate::herdr::PromptResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = sender.try_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "prompt request did not finish");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn selected_live_remote_is_read_only_without_socket_contact() {
        let server = Server::new();
        let id = server.source();
        let source = remote_source(&id);
        let shared = Arc::new(Mutex::new(AppState::new()));
        {
            let mut state = shared.lock().unwrap();
            state.set_observation_catalog(SourceCatalog {
                initialized: true,
                machines: vec![MachineInfo {
                    id: id.clone(),
                    label: "Remote".to_owned(),
                    remote_session: "observation".to_owned(),
                    enabled: true,
                    status: MachineStatus::Online,
                    error: None,
                }],
                error: None,
            });
            state.apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: true,
                machines: vec![id],
            });
        }
        let key = publish(&shared, &source, 1, &[record("remote-terminal", "pane")]);
        assert_eq!(
            shared.lock().unwrap().prompt_available(&key),
            Err(PromptError::ReadOnly)
        );
        let mut sender = PromptSender::new(Arc::clone(&shared));
        assert_eq!(
            sender.submit(key.clone(), "must not send".into(), RequestOrigin::Gui),
            Err(PromptError::ReadOnly)
        );
        assert!(shared
            .lock()
            .unwrap()
            .update_source(&source, 1, false, SourceCounts::default()));
        assert_eq!(
            sender.submit(
                key,
                "offline remote is still read-only".into(),
                RequestOrigin::Gui
            ),
            Err(PromptError::ReadOnly)
        );
        assert!(!sender.is_pending());
        server.listener.set_nonblocking(true).unwrap();
        assert!(
            server.listener.accept().is_err(),
            "remote prompt contacted a socket"
        );
    }

    #[test]
    fn hidden_local_key_cannot_send_and_reselection_restores_current_key() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(&shared, &server.source(), 1, &[record("terminal", "pane")]);
        let mut sender = PromptSender::new(Arc::clone(&shared));
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences {
                local: false,
                ..ObservationPreferences::default()
            });
        assert!(shared
            .lock()
            .unwrap()
            .session_snapshot(&SessionListOptions::default(), Some(&key))
            .rows
            .is_empty());
        assert_eq!(
            shared.lock().unwrap().prompt_available(&key),
            Err(PromptError::StaleTarget)
        );
        assert_eq!(
            sender.submit(key.clone(), "hidden".into(), RequestOrigin::Gui),
            Err(PromptError::StaleTarget)
        );
        assert!(!sender.is_pending());
        server.listener.set_nonblocking(true).unwrap();
        assert!(
            server.listener.accept().is_err(),
            "hidden local prompt contacted a socket"
        );
        shared
            .lock()
            .unwrap()
            .apply_observation_preferences(ObservationPreferences::default());
        let state = shared.lock().unwrap();
        let snapshot =
            state.session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None);
        assert_eq!(snapshot.rows[0].key, key);
        assert_eq!(state.prompt_available(&snapshot.rows[0].key), Ok(()));
    }

    #[test]
    fn noncanonical_local_source_never_contacts_its_alias_socket() {
        let server = Server::new();
        let canonical = PathBuf::from(server.source());
        let parent = canonical.parent().unwrap();
        let alias = parent
            .join("..")
            .join(parent.file_name().unwrap())
            .join(canonical.file_name().unwrap());
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(
            &shared,
            alias.to_str().unwrap(),
            1,
            &[record("terminal", "pane")],
        );
        let mut sender = PromptSender::new(shared);
        sender
            .submit(key, "never deliver".into(), RequestOrigin::Gui)
            .unwrap();
        assert_eq!(receive(&mut sender).result, Err(PromptError::StaleTarget));
        server.listener.set_nonblocking(true).unwrap();
        assert!(
            server.listener.accept().is_err(),
            "alias contacted local socket"
        );
    }

    #[test]
    fn prompt_rejects_stale_offline_and_ambiguous_mapping_without_contacting_server() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(&shared, &server.source(), 1, &[record("terminal", "pane")]);
        let mut sender = PromptSender::new(Arc::clone(&shared));
        assert_eq!(
            sender.submit(key.clone(), " \n ".into(), RequestOrigin::Gui),
            Err(PromptError::Empty)
        );
        assert_eq!(
            sender.submit(
                key.clone(),
                "x".repeat(crate::herdr_protocol::MAX_FRAME_BYTES),
                RequestOrigin::Gui,
            ),
            Err(PromptError::TooLarge)
        );
        assert_eq!(
            sender.submit(
                key.clone(),
                "\"".repeat(crate::herdr_protocol::MAX_FRAME_BYTES / 2),
                RequestOrigin::Gui,
            ),
            Err(PromptError::TooLarge),
            "escaped frame must fit the full 512 KiB wire limit"
        );
        shared
            .lock()
            .unwrap()
            .update_source(&server.source(), 1, false, SourceCounts::default());
        assert_eq!(
            sender.submit(key.clone(), "hi".into(), RequestOrigin::Gui),
            Err(PromptError::Offline)
        );
        shared.lock().unwrap().begin_source(server.source(), 2);
        assert_eq!(
            sender.submit(key.clone(), "hi".into(), RequestOrigin::Gui),
            Err(PromptError::StaleTarget)
        );
        let current = publish_after_generation(
            &shared,
            &server.source(),
            2,
            &[record("terminal", "pane"), record("other", "pane")],
        );
        assert_eq!(
            sender.submit(current, "hi".into(), RequestOrigin::Gui),
            Err(PromptError::StaleTarget)
        );
        shared.lock().unwrap().request_shutdown();
        assert_eq!(
            sender.submit(key, "hi".into(), RequestOrigin::Gui),
            Err(PromptError::Offline)
        );
        assert!(!sender.is_pending());
    }

    fn publish_after_generation(
        shared: &Arc<Mutex<AppState>>,
        source: &str,
        generation: u64,
        agents: &[AgentRecord],
    ) -> SessionKey {
        let mut state = shared.lock().unwrap();
        assert!(state.publish_source_snapshot(
            source,
            generation,
            agents,
            SourceCounts::default(),
            Arc::from(
                agents
                    .iter()
                    .map(|agent| agent.pane_id.clone())
                    .collect::<Vec<_>>()
            ),
            &[],
            Instant::now()
        ));
        state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows
            .into_iter()
            .find(|row| row.key.terminal_id == agents[0].terminal_id)
            .unwrap()
            .key
    }

    #[test]
    fn prompt_routes_selected_source_and_preserves_original_multiline_text() {
        let selected = Server::new();
        let other = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        publish(
            &shared,
            &other.source(),
            1,
            &[record("other-terminal", "same-pane")],
        );
        let key = publish(
            &shared,
            &selected.source(),
            1,
            &[record("selected-terminal", "same-pane")],
        );
        let worker = thread::spawn(move || {
            selected.snapshot(&[("selected-terminal", "same-pane")]);
            let request = selected.exchange(
                "agent.prompt",
                json!({"type": "agent_prompted", "agent": {
                    "terminal_id": "selected-terminal", "pane_id": "same-pane"
                }}),
            );
            assert_eq!(request["params"]["target"], "same-pane");
            assert_eq!(request["params"]["text"], "  first\nsecond  ");
            assert!(request["params"].get("wait").is_none());
        });
        let mut sender = PromptSender::new(shared);
        sender
            .submit(
                key.clone(),
                "  first\nsecond  ".to_owned(),
                RequestOrigin::Gui,
            )
            .unwrap();
        assert_eq!(
            sender.submit(
                key.clone(),
                "again".into(),
                RequestOrigin::Cli {
                    operation_id: "op-busy".into(),
                },
            ),
            Err(PromptError::Busy)
        );
        let result = receive(&mut sender);
        assert_eq!(result.submission.key, key);
        assert_eq!(result.submission.text, "  first\nsecond  ");
        assert_eq!(result.submission.origin, RequestOrigin::Gui);
        assert_eq!(result.result, Ok(crate::herdr::PromptReceipt::Acknowledged));
        assert!(!sender.is_pending());
        worker.join().unwrap();
    }

    #[test]
    fn prompt_rejects_changed_live_identity_before_submission() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(&shared, &server.source(), 1, &[record("original", "pane")]);
        let worker = thread::spawn(move || {
            server.snapshot(&[("replacement", "pane")]);
            server.listener.set_nonblocking(true).unwrap();
            assert!(
                server.listener.accept().is_err(),
                "stale prompt was transmitted"
            );
        });
        let mut sender = PromptSender::new(shared);
        sender
            .submit(key, "message".into(), RequestOrigin::Gui)
            .unwrap();
        assert_eq!(receive(&mut sender).result, Err(PromptError::StaleTarget));
        worker.join().unwrap();
    }

    #[test]
    fn prompt_maps_server_rejection_and_uncertain_delivery_without_resend() {
        for (code, expected) in [
            ("agent_blocked", PromptError::Blocked),
            ("agent_not_ready", PromptError::NotReady),
            ("unsupported_method", PromptError::Unsupported),
        ] {
            let server = Server::new();
            let shared = Arc::new(Mutex::new(AppState::new()));
            let key = publish(&shared, &server.source(), 1, &[record("terminal", "pane")]);
            let worker = thread::spawn(move || {
                server.snapshot(&[("terminal", "pane")]);
                let (mut stream, _) = server.listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                writeln!(
                    stream,
                    "{}",
                    json!({"id": request["id"], "error": {"code": code, "message": "rejected"}})
                )
                .unwrap();
            });
            let mut sender = PromptSender::new(shared);
            sender
                .submit(key, "text".into(), RequestOrigin::Gui)
                .unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.result, Err(expected));
            assert_eq!(result.submission.text, "text");
            worker.join().unwrap();
        }
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(&shared, &server.source(), 1, &[record("terminal", "pane")]);
        let worker = thread::spawn(move || {
            server.snapshot(&[("terminal", "pane")]);
            let (stream, _) = server.listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "agent.prompt");
            drop(stream);
            server.listener.set_nonblocking(true).unwrap();
            assert!(server.listener.accept().is_err(), "prompt was resent");
        });
        let mut sender = PromptSender::new(shared);
        let origin = RequestOrigin::Cli {
            operation_id: "prompt-unknown".into(),
        };
        sender
            .submit(key.clone(), "do not duplicate".into(), origin.clone())
            .unwrap();
        let result = receive(&mut sender);
        assert_eq!(result.result, Err(PromptError::UnknownDelivery));
        assert_eq!(result.submission.key, key);
        assert_eq!(result.submission.origin, origin);
        assert_eq!(result.submission.text, "do not duplicate");
        worker.join().unwrap();
    }

    #[test]
    fn prompt_does_not_report_success_for_wrong_ack_identity() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let key = publish(&shared, &server.source(), 1, &[record("terminal", "pane")]);
        let worker = thread::spawn(move || {
            server.snapshot(&[("terminal", "pane")]);
            server.exchange(
                "agent.prompt",
                json!({
                    "type": "agent_prompted",
                    "agent": {"terminal_id": "other", "pane_id": "pane"}
                }),
            );
        });
        let mut sender = PromptSender::new(shared);
        sender
            .submit(key, "identity".into(), RequestOrigin::Gui)
            .unwrap();
        assert_eq!(
            receive(&mut sender).result,
            Err(PromptError::UnknownDelivery)
        );
        worker.join().unwrap();
    }
}

mod worktree_remove {
    use crate::herdr::{
        RequestOrigin, WorktreeRemoveError, WorktreeRemoveResult, WorktreeRemoveSender,
    };
    use crate::session::{AgentRecord, AgentStatus, SessionMetadata, WorkspaceWorktreeInfo};
    use crate::session_view::{SessionFilter, SessionKey, SessionListOptions};
    use crate::state::{AppState, SourceCounts};
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

    struct Server {
        path: PathBuf,
        listener: UnixListener,
    }

    impl Server {
        fn new() -> Self {
            let path = PathBuf::from("/tmp").join(format!(
                "herdr-remove-test-{}-{}.sock",
                std::process::id(),
                NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
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

        fn exchange(&self, method: &str, result: Value) -> Value {
            let (mut stream, _) = self.listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], method);
            writeln!(stream, "{}", json!({"id": request["id"], "result": result})).unwrap();
            request
        }

        fn snapshot(&self, workspaces: Value, agents: Value, panes: Value) {
            self.exchange(
                "session.snapshot",
                json!({
                    "type": "session_snapshot",
                    "snapshot": { "workspaces": workspaces, "agents": agents, "panes": panes }
                }),
            );
        }

        fn assert_no_request(&self) {
            self.listener.set_nonblocking(true).unwrap();
            assert!(
                self.listener.accept().is_err(),
                "unexpected destructive request"
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

    fn record(
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

    fn publish(
        shared: &Arc<Mutex<AppState>>,
        source: &str,
        generation: u64,
        agents: &[AgentRecord],
    ) -> SessionKey {
        let mut state = shared.lock().unwrap();
        assert!(state.begin_source(source.into(), generation));
        assert!(state.publish_source_snapshot(
            source,
            generation,
            agents,
            SourceCounts::default(),
            Arc::from(
                agents
                    .iter()
                    .map(|agent| agent.pane_id.clone())
                    .collect::<Vec<_>>()
            ),
            &[],
            Instant::now()
        ));
        state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows
            .into_iter()
            .filter(|row| {
                row.key.generation == generation && row.key.terminal_id == agents[0].terminal_id
            })
            .max_by_key(|row| row.key.source_id)
            .unwrap()
            .key
    }

    fn receive(sender: &mut WorktreeRemoveSender) -> WorktreeRemoveResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = sender.try_result() {
                assert!(!sender.is_pending());
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "worktree remove worker did not finish"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn routes_confirmed_source_with_duplicate_ids_and_allows_multiple_workspace_panes() {
        let selected = Server::new();
        let other = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let info = worktree("/virtual/checkout", 2, 2, true);
        publish(
            &shared,
            &other.source(),
            1,
            &[record(
                "same",
                "same-pane",
                "same-ws",
                worktree("/virtual/other", 1, 1, true),
            )],
        );
        let key = publish(
            &shared,
            &selected.source(),
            1,
            &[record("same", "same-pane", "same-ws", Arc::clone(&info))],
        );
        let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        let expected = target.clone();
        let server_info = Arc::clone(&info);
        let worker = thread::spawn(move || {
            selected.snapshot(
                json!([workspace("same-ws", &server_info)]),
                json!([
                    agent("same", "same-pane", "same-ws"),
                    agent("other", "second-pane", "same-ws")
                ]),
                json!([{"pane_id": "same-pane"}, {"pane_id": "second-pane"}]),
            );
            let request = selected.exchange(
                "worktree.remove",
                json!({
                    "type": "worktree_removed", "workspace_id": "same-ws",
                    "path": "/virtual/checkout", "forced": false
                }),
            );
            assert_eq!(
                request["params"],
                json!({"workspace_id": "same-ws", "force": false})
            );
            selected
        });
        let mut sender = WorktreeRemoveSender::new(Arc::clone(&shared));
        let origin = RequestOrigin::Cli {
            operation_id: "remove-1".into(),
        };
        sender.submit(target.clone(), origin.clone()).unwrap();
        assert_eq!(
            sender.submit(target, RequestOrigin::Gui),
            Err(WorktreeRemoveError::Busy)
        );
        let result = receive(&mut sender);
        assert_eq!(result.target, expected);
        assert_eq!(result.origin, origin);
        assert_eq!(result.result, Ok(()));
        assert!(
            shared.lock().unwrap().session_view_for_key(&key).is_some(),
            "never optimistically purge"
        );
        worker.join().unwrap().assert_no_request();
        other.assert_no_request();
    }

    #[test]
    fn frozen_target_change_and_ineligible_metadata_never_contact_server() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let info = worktree("/virtual/checkout", 1, 1, true);
        let key = publish(
            &shared,
            &server.source(),
            1,
            &[record("terminal", "pane", "ws", Arc::clone(&info))],
        );
        let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        let mut sender = WorktreeRemoveSender::new(Arc::clone(&shared));
        let changed = record(
            "terminal",
            "pane",
            "ws",
            worktree("/virtual/checkout", 2, 1, true),
        );
        assert!(shared.lock().unwrap().publish_source_snapshot(
            &server.source(),
            1,
            &[changed],
            SourceCounts::default(),
            Arc::from(vec!["pane".to_owned()]),
            &[],
            Instant::now()
        ));
        assert_eq!(
            sender.submit(target, RequestOrigin::Gui),
            Err(WorktreeRemoveError::StaleTarget)
        );
        assert!(!sender.is_pending());
        let current = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        assert_eq!(current.worktree.pane_count, 2);
        assert!(shared.lock().unwrap().publish_source_snapshot(
            &server.source(),
            1,
            &[record(
                "terminal",
                "pane",
                "ws",
                worktree("/virtual/checkout", 1, 1, false)
            )],
            SourceCounts::default(),
            Arc::from(vec!["pane".to_owned()]),
            &[],
            Instant::now()
        ));
        assert_eq!(
            sender.submit(current, RequestOrigin::Gui),
            Err(WorktreeRemoveError::NotLinkedWorktree)
        );
        server.assert_no_request();
    }

    #[test]
    fn fresh_snapshot_identity_or_impact_change_aborts_before_remove() {
        for (agents, panes, info) in [
            (
                json!([agent("replacement", "pane", "ws")]),
                json!([{"pane_id": "pane"}]),
                worktree("/virtual/checkout", 1, 1, true),
            ),
            (
                json!([
                    agent("terminal", "pane", "ws"),
                    agent("terminal", "other", "ws")
                ]),
                json!([{"pane_id": "pane"}, {"pane_id": "other"}]),
                worktree("/virtual/checkout", 1, 1, true),
            ),
            (
                json!([agent("terminal", "pane", "other-ws")]),
                json!([{"pane_id": "pane"}]),
                worktree("/virtual/checkout", 1, 1, true),
            ),
            (
                json!([agent("terminal", "pane", "ws")]),
                json!([{"pane_id": "pane"}]),
                worktree("/virtual/checkout", 1, 2, true),
            ),
            (
                json!([agent("terminal", "pane", "ws")]),
                json!([{"pane_id": "pane"}]),
                worktree("/virtual/checkout", 1, 1, false),
            ),
        ] {
            let server = Server::new();
            let shared = Arc::new(Mutex::new(AppState::new()));
            let key = publish(
                &shared,
                &server.source(),
                1,
                &[record(
                    "terminal",
                    "pane",
                    "ws",
                    worktree("/virtual/checkout", 1, 1, true),
                )],
            );
            let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
            let worker = thread::spawn(move || {
                server.snapshot(json!([workspace("ws", &info)]), agents, panes);
                server
            });
            let mut sender = WorktreeRemoveSender::new(shared);
            sender.submit(target.clone(), RequestOrigin::Gui).unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.target, target);
            assert_eq!(result.result, Err(WorktreeRemoveError::StaleTarget));
            worker.join().unwrap().assert_no_request();
        }
    }

    #[test]
    fn state_change_during_snapshot_aborts_before_remove_without_holding_lock() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let info = worktree("/virtual/checkout", 1, 1, true);
        let key = publish(
            &shared,
            &server.source(),
            1,
            &[record("terminal", "pane", "ws", Arc::clone(&info))],
        );
        let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        let (read_tx, read_rx) = mpsc::sync_channel(1);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let (mut stream, _) = server.listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "session.snapshot");
            read_tx.send(()).unwrap();
            reply_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            writeln!(
                stream,
                "{}",
                json!({"id": request["id"], "result": {
                    "type": "session_snapshot", "snapshot": {
                        "workspaces": [workspace("ws", &info)],
                        "panes": [{"pane_id": "pane"}],
                        "agents": [agent("terminal", "pane", "ws")]
                    }
                }})
            )
            .unwrap();
            server
        });
        let mut sender = WorktreeRemoveSender::new(Arc::clone(&shared));
        sender.submit(target.clone(), RequestOrigin::Gui).unwrap();
        read_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(shared.lock().unwrap().publish_source_snapshot(
            &target.source,
            1,
            &[record(
                "terminal",
                "pane",
                "ws",
                worktree("/virtual/checkout", 2, 1, true)
            )],
            SourceCounts::default(),
            Arc::from(vec!["pane".to_owned()]),
            &[],
            Instant::now()
        ));
        reply_tx.send(()).unwrap();
        let result = receive(&mut sender);
        assert_eq!(result.target, target);
        assert_eq!(result.result, Err(WorktreeRemoveError::StaleTarget));
        worker.join().unwrap().assert_no_request();
    }

    #[test]
    fn remote_offline_main_and_ambiguous_targets_are_ineligible() {
        use crate::session_view::WorktreeRemoveTargetError;
        use crate::sources::{
            remote_source, MachineInfo, MachineStatus, ObservationPreferences, SourceCatalog,
        };
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let linked = worktree("/virtual/checkout", 1, 1, true);
        let key = publish(
            &shared,
            &server.source(),
            1,
            &[record("terminal", "pane", "ws", Arc::clone(&linked))],
        );
        let mut sender = WorktreeRemoveSender::new(Arc::clone(&shared));
        let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        assert!(shared.lock().unwrap().update_source(
            &server.source(),
            1,
            false,
            SourceCounts::default()
        ));
        assert_eq!(
            sender.submit(target.clone(), RequestOrigin::Gui),
            Err(WorktreeRemoveError::Offline)
        );
        shared.lock().unwrap().begin_source(server.source(), 2);
        assert_eq!(
            sender.submit(target, RequestOrigin::Gui),
            Err(WorktreeRemoveError::StaleTarget)
        );

        let main_key = publish(
            &shared,
            &server.source(),
            3,
            &[record(
                "main",
                "main-pane",
                "main-ws",
                worktree("/virtual/repo", 1, 1, false),
            )],
        );
        assert_eq!(
            shared.lock().unwrap().worktree_remove_target(&main_key),
            Err(WorktreeRemoveTargetError::NotLinkedWorktree)
        );
        let ambiguous = publish(
            &shared,
            &server.source(),
            4,
            &[
                record("one", "shared-pane", "ws", Arc::clone(&linked)),
                record("two", "shared-pane", "ws", Arc::clone(&linked)),
            ],
        );
        assert_eq!(
            shared.lock().unwrap().worktree_remove_target(&ambiguous),
            Err(WorktreeRemoveTargetError::Stale)
        );
        server.assert_no_request();

        let remote_id = server.source();
        let remote = remote_source(&remote_id);
        {
            let mut state = shared.lock().unwrap();
            state.set_observation_catalog(SourceCatalog {
                initialized: true,
                machines: vec![MachineInfo {
                    id: remote_id.clone(),
                    label: "Remote".into(),
                    remote_session: "observation".into(),
                    enabled: true,
                    status: MachineStatus::Online,
                    error: None,
                }],
                error: None,
            });
            state.apply_observation_preferences(ObservationPreferences {
                local: true,
                remote: true,
                machines: vec![remote_id],
            });
        }
        let remote_key = publish(
            &shared,
            &remote,
            1,
            &[record("remote", "pane", "ws", linked)],
        );
        assert_eq!(
            shared.lock().unwrap().worktree_remove_target(&remote_key),
            Err(WorktreeRemoveTargetError::ReadOnly)
        );
        assert!(!sender.is_pending());
    }

    #[test]
    fn snapshot_duplicate_workspace_or_pane_is_not_removable() {
        for (workspaces, panes) in [
            (
                json!([
                    workspace("ws", &worktree("/virtual/checkout", 1, 1, true)),
                    workspace("ws", &worktree("/virtual/checkout", 1, 1, true))
                ]),
                json!([{"pane_id": "pane"}]),
            ),
            (
                json!([workspace("ws", &worktree("/virtual/checkout", 1, 1, true))]),
                json!([{"pane_id": "pane"}, {"pane_id": "pane"}]),
            ),
        ] {
            let server = Server::new();
            let shared = Arc::new(Mutex::new(AppState::new()));
            let key = publish(
                &shared,
                &server.source(),
                1,
                &[record(
                    "terminal",
                    "pane",
                    "ws",
                    worktree("/virtual/checkout", 1, 1, true),
                )],
            );
            let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
            let worker = thread::spawn(move || {
                server.snapshot(workspaces, json!([agent("terminal", "pane", "ws")]), panes);
                server
            });
            let mut sender = WorktreeRemoveSender::new(shared);
            sender.submit(target.clone(), RequestOrigin::Gui).unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.target, target);
            assert_eq!(result.result, Err(WorktreeRemoveError::StaleTarget));
            worker.join().unwrap().assert_no_request();
        }
    }

    #[test]
    fn rejection_preserves_code_and_message_without_force_trust_or_retry() {
        for (code, expected) in [
            (
                "dirty_worktree_requires_force",
                WorktreeRemoveError::Rejected {
                    code: "dirty_worktree_requires_force".into(),
                    message: "Uncommitted changes".into(),
                },
            ),
            ("unsupported_method", WorktreeRemoveError::Unsupported),
        ] {
            let server = Server::new();
            let shared = Arc::new(Mutex::new(AppState::new()));
            let info = worktree("/virtual/checkout", 1, 1, true);
            let key = publish(
                &shared,
                &server.source(),
                1,
                &[record("terminal", "pane", "ws", Arc::clone(&info))],
            );
            let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
            let worker = thread::spawn(move || {
                server.snapshot(
                    json!([workspace("ws", &info)]),
                    json!([agent("terminal", "pane", "ws")]),
                    json!([{"pane_id": "pane"}]),
                );
                let (mut stream, _) = server.listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], "worktree.remove");
                assert_eq!(
                    request["params"],
                    json!({"workspace_id": "ws", "force": false})
                );
                writeln!(
                    stream,
                    "{}",
                    json!({"id": request["id"], "error": {
                        "code": code, "message": "Uncommitted changes"
                    }})
                )
                .unwrap();
                server
            });
            let mut sender = WorktreeRemoveSender::new(shared);
            sender.submit(target.clone(), RequestOrigin::Gui).unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.target, target);
            assert_eq!(result.result, Err(expected));
            worker.join().unwrap().assert_no_request();
        }
    }

    #[test]
    fn uncertain_delivery_or_wrong_ack_never_reports_success_or_resends() {
        for response in [
            None,
            Some(
                json!({"result": {"type": "worktree_removed", "workspace_id": "other", "path": "/virtual/checkout", "forced": false}}),
            ),
            Some(
                json!({"result": {"type": "worktree_removed", "workspace_id": "ws", "path": "/virtual/other", "forced": false}}),
            ),
            Some(
                json!({"result": {"type": "worktree_removed", "workspace_id": "ws", "path": "/virtual/checkout", "forced": true}}),
            ),
            Some(
                json!({"result": {"type": "worktree_removed", "workspace_id": "ws", "path": "/virtual/checkout", "forced": false}, "bad_id": true}),
            ),
            Some(json!({"malformed": true})),
        ] {
            let server = Server::new();
            let shared = Arc::new(Mutex::new(AppState::new()));
            let info = worktree("/virtual/checkout", 1, 1, true);
            let key = publish(
                &shared,
                &server.source(),
                1,
                &[record("terminal", "pane", "ws", Arc::clone(&info))],
            );
            let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
            let worker = thread::spawn(move || {
                server.snapshot(
                    json!([workspace("ws", &info)]),
                    json!([agent("terminal", "pane", "ws")]),
                    json!([{"pane_id": "pane"}]),
                );
                let (mut stream, _) = server.listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], "worktree.remove");
                assert_eq!(
                    request["params"],
                    json!({"workspace_id": "ws", "force": false})
                );
                if let Some(mut response) = response {
                    if response.get("bad_id").is_some() {
                        response.as_object_mut().unwrap().remove("bad_id");
                        response["id"] = json!("wrong-request");
                    } else {
                        response["id"] = request["id"].clone();
                    }
                    writeln!(stream, "{response}").unwrap();
                }
                drop(stream);
                server
            });
            let mut sender = WorktreeRemoveSender::new(shared);
            let origin = RequestOrigin::Cli {
                operation_id: "remove-unknown".into(),
            };
            sender.submit(target.clone(), origin.clone()).unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.target, target);
            assert_eq!(result.origin, origin);
            assert_eq!(result.result, Err(WorktreeRemoveError::UnknownDelivery));
            worker.join().unwrap().assert_no_request();
        }
    }
    #[test]
    fn remove_request_does_not_hold_app_state_mutex_while_waiting_for_ack() {
        let server = Server::new();
        let shared = Arc::new(Mutex::new(AppState::new()));
        let info = worktree("/virtual/checkout", 1, 1, true);
        let key = publish(
            &shared,
            &server.source(),
            1,
            &[record("terminal", "pane", "ws", Arc::clone(&info))],
        );
        let target = shared.lock().unwrap().worktree_remove_target(&key).unwrap();
        let (received_tx, received_rx) = mpsc::sync_channel(1);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            server.snapshot(
                json!([workspace("ws", &info)]),
                json!([agent("terminal", "pane", "ws")]),
                json!([{"pane_id": "pane"}]),
            );
            let (mut stream, _) = server.listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "worktree.remove");
            received_tx.send(()).unwrap();
            reply_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            writeln!(
                stream,
                "{}",
                json!({"id": request["id"], "result": {
                    "type": "worktree_removed", "workspace_id": "ws",
                    "path": "/virtual/checkout", "forced": false
                }})
            )
            .unwrap();
            server
        });
        let mut sender = WorktreeRemoveSender::new(Arc::clone(&shared));
        sender.submit(target.clone(), RequestOrigin::Gui).unwrap();
        received_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(shared.lock().unwrap().update_source(
            &target.source,
            target.key.generation,
            false,
            SourceCounts::default()
        ));
        reply_tx.send(()).unwrap();
        let result = receive(&mut sender);
        assert_eq!(result.target, target);
        assert_eq!(result.result, Ok(()));
        worker.join().unwrap().assert_no_request();
    }
}

mod watcher_lifecycle {
    use crate::herdr::Watchers;
    use crate::lifecycle::LifecycleSettings;
    use crate::session_view::{Availability, SessionFilter, SessionListOptions};
    use crate::state::AppState;
    use serde_json::{json, Value};
    use std::fs;
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
    // A connected watcher rechecks plugin availability on a five-second timer.
    // Leave room for the full interval and worker scheduling.
    const DEADLINE: Duration = Duration::from_secs(12);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Plugin {
        Missing,
        Enabled,
        Disabled,
        Malformed,
        Error,
    }

    #[derive(Clone, Copy, Debug)]
    struct PendingPluginObservation {
        plugin: Plugin,
        connected_sources: usize,
        disconnected_sources: usize,
        shutdown: bool,
    }

    enum Command {
        Set(Plugin, Sender<()>),
        ArmCapture(
            Arc<Mutex<AppState>>,
            Sender<PendingPluginObservation>,
            Sender<()>,
        ),
        Reconcile(Sender<()>),
        Stop,
    }

    struct Server {
        path: PathBuf,
        commands: Sender<Command>,
        queries: Receiver<Plugin>,
        join: Option<JoinHandle<()>>,
        terminal: String,
        pane: String,
    }

    impl Server {
        fn new(initial: Plugin, label: &str) -> Self {
            let path = PathBuf::from("/tmp").join(format!(
                "herdr-watch-{}-{}.sock",
                std::process::id(),
                NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
            ));
            let listener = UnixListener::bind(&path).expect("bind fixture socket");
            let (commands, rx) = mpsc::channel();
            let (query_tx, queries) = mpsc::channel();
            let terminal = format!("{label}-terminal");
            let pane = format!("{label}-pane");
            let server_terminal = terminal.clone();
            let server_pane = pane.clone();
            let join = thread::spawn(move || {
                let mut plugin = initial;
                let mut capture: Option<(Arc<Mutex<AppState>>, Sender<PendingPluginObservation>)> =
                    None;
                let mut subscriptions: Vec<UnixStream> = Vec::new();
                loop {
                    while let Ok(command) = rx.try_recv() {
                        match command {
                            Command::Set(next, ack) => {
                                plugin = next;
                                ack.send(()).expect("fixture mode ack");
                            }
                            Command::ArmCapture(shared, sender, ack) => {
                                assert!(capture.is_none(), "fixture capture already armed");
                                capture = Some((shared, sender));
                                ack.send(()).expect("fixture capture ack");
                            }
                            Command::Reconcile(ack) => {
                                let event = json!({
                                    "event": "pane.agent_status_changed",
                                    "data": {
                                        "pane_id": server_pane,
                                        "workspace_id": "workspace",
                                        "agent_status": "idle"
                                    }
                                });
                                subscriptions
                                    .retain_mut(|stream| writeln!(stream, "{event}").is_ok());
                                assert!(!subscriptions.is_empty(), "fixture lost subscription");
                                ack.send(()).expect("fixture event ack");
                            }
                            Command::Stop => return,
                        }
                    }
                    let (mut stream, _) = listener.accept().expect("accept fixture request");
                    let deadline = Instant::now() + DEADLINE;
                    let mut line = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let available =
                            (crate::herdr_protocol::MAX_FRAME_BYTES - line.len()).min(chunk.len());
                        assert!(available > 0, "fixture request too large");
                        let read = crate::socket::read_with_deadline(
                            &mut stream,
                            &mut chunk[..available],
                            deadline,
                        )
                        .expect("read fixture request before deadline");
                        if read == 0 {
                            assert!(line.is_empty(), "truncated fixture request");
                            break; // command wake-up, not a protocol request
                        }
                        if let Some(newline) = chunk[..read].iter().position(|byte| *byte == b'\n')
                        {
                            assert_eq!(
                                newline + 1,
                                read,
                                "multiple fixture requests on one socket"
                            );
                            line.extend_from_slice(&chunk[..read]);
                            break;
                        }
                        line.extend_from_slice(&chunk[..read]);
                    }
                    if line.is_empty() {
                        continue;
                    }
                    let request: Value = serde_json::from_slice(&line).expect("request JSON");
                    let response = match request["method"].as_str().expect("request method") {
                        "ping" => json!({"type": "pong"}),
                        "session.snapshot" => json!({
                            "type": "session_snapshot",
                            "snapshot": {
                                "panes": [{"pane_id": server_pane}],
                                "agents": [{
                                    "terminal_id": server_terminal,
                                    "pane_id": server_pane,
                                    "agent_status": "idle"
                                }]
                            }
                        }),
                        "plugin.list" => {
                            if let Some((shared, sender)) = capture.take() {
                                let observation = {
                                    let scene = shared.lock().unwrap().scene();
                                    PendingPluginObservation {
                                        plugin,
                                        connected_sources: scene.connected_sources,
                                        disconnected_sources: scene.disconnected_sources,
                                        shutdown: scene.shutdown,
                                    }
                                };
                                sender
                                    .send(observation)
                                    .expect("report captured fixture query");
                            } else {
                                query_tx.send(plugin).expect("report fixture query");
                            }
                            match plugin {
                                Plugin::Missing => json!({"type": "plugin_list", "plugins": []}),
                                Plugin::Enabled | Plugin::Disabled => json!({
                                    "type": "plugin_list",
                                    "plugins": [{"plugin_id": "desktop-pet", "enabled": matches!(plugin, Plugin::Enabled)}]
                                }),
                                Plugin::Malformed => {
                                    json!({"type": "plugin_list", "plugins": [{}]})
                                }
                                Plugin::Error => {
                                    writeln!(stream, "{}", json!({
                                        "id": request["id"],
                                        "error": {"code": "unavailable", "message": "query failed"}
                                    })).expect("write plugin error");
                                    continue;
                                }
                            }
                        }
                        "events.subscribe" => {
                            assert_eq!(
                                request["params"]["subscriptions"],
                                json!([{"type": "pane.agent_status_changed", "pane_id": server_pane}])
                            );
                            writeln!(
                                stream,
                                "{}",
                                json!({
                                    "id": request["id"],
                                    "result": {"type": "subscription_started"}
                                })
                            )
                            .expect("subscription ack");
                            subscriptions.push(stream);
                            continue;
                        }
                        method => panic!("unexpected fixture request: {method}"),
                    };
                    writeln!(
                        stream,
                        "{}",
                        json!({"id": request["id"], "result": response})
                    )
                    .expect("write response");
                }
            });
            Self {
                path,
                commands,
                join: Some(join),
                queries,
                terminal,
                pane,
            }
        }

        fn wake(&self) -> UnixStream {
            let stream = UnixStream::connect(&self.path).expect("wake fixture");
            stream
                .shutdown(std::net::Shutdown::Write)
                .expect("wake EOF");
            stream
        }

        fn command(&self, make: impl FnOnce(Sender<()>) -> Command) {
            let (ack, received) = mpsc::channel();
            self.commands.send(make(ack)).expect("fixture command");
            let _wake = self.wake();
            received
                .recv_timeout(DEADLINE)
                .expect("fixture command ack");
        }

        fn set(&self, mode: Plugin) {
            self.command(|ack| Command::Set(mode, ack));
        }
        fn capture_next_plugin(
            &self,
            shared: &Arc<Mutex<AppState>>,
        ) -> Receiver<PendingPluginObservation> {
            let (sender, receiver) = mpsc::channel();
            self.command(|ack| Command::ArmCapture(Arc::clone(shared), sender, ack));
            receiver
        }

        fn reconcile(&self) {
            self.command(Command::Reconcile);
        }

        fn query(&self) -> Plugin {
            self.queries
                .recv_timeout(DEADLINE)
                .expect("fixture plugin query")
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.commands.send(Command::Stop);
            // Keep the EOF peer open until the accept loop has consumed Stop.
            let _wake = UnixStream::connect(&self.path).ok().and_then(|stream| {
                stream.shutdown(std::net::Shutdown::Write).ok()?;
                Some(stream)
            });
            let result = self.join.take().unwrap().join();
            let removed = fs::remove_file(&self.path);
            if !thread::panicking() {
                result.expect("fixture thread");
                removed.expect("remove fixture socket");
            }
        }
    }

    fn watchers(exit_with_herdr: bool) -> (Arc<Mutex<AppState>>, Watchers) {
        let shared = Arc::new(Mutex::new(AppState::new()));
        shared
            .lock()
            .unwrap()
            .set_lifecycle_settings(LifecycleSettings {
                auto_start: true,
                exit_with_herdr,
            });
        let watchers = Watchers::new(Arc::clone(&shared));
        (shared, watchers)
    }

    fn until(label: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + DEADLINE;
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {label}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn register_and_capture(
        watchers: &Watchers,
        shared: &Arc<Mutex<AppState>>,
        server: &Server,
        survivor: &Server,
        expected: Plugin,
    ) -> PendingPluginObservation {
        {
            let state = shared.lock().unwrap();
            let scene = state.scene();
            let rows = state
                .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                .rows;
            assert_eq!(scene.connected_sources, 1, "survivor baseline");
            assert_eq!(scene.disconnected_sources, 0, "survivor baseline");
            assert!(!scene.shutdown, "survivor baseline");
            assert!(
                rows.iter()
                    .all(|row| row.key.terminal_id != server.terminal),
                "target already has a row before registration"
            );
            assert!(
                rows.iter().any(|row| {
                    row.key.terminal_id == survivor.terminal
                        && row.pane_id == survivor.pane
                        && row.availability == Availability::Live
                }),
                "survivor has no live row before registration"
            );
        }
        let captured = server.capture_next_plugin(shared);
        watchers
            .register(server.path.clone())
            .expect("register target");
        let observation = captured
            .recv_timeout(DEADLINE)
            .expect("captured target plugin query");
        assert_eq!(observation.plugin, expected, "unexpected captured plugin");
        observation
    }

    fn live(shared: &Arc<Mutex<AppState>>, server: &Server) -> Option<u64> {
        let state = shared.lock().unwrap();
        let scene = state.scene();
        state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows
            .into_iter()
            .find(|row| {
                row.key.terminal_id == server.terminal
                    && row.pane_id == server.pane
                    && row.availability == Availability::Live
                    && scene.connected_sources > 0
                    && !scene.shutdown
            })
            .map(|row| row.key.generation)
    }

    fn until_detached(
        shared: &Arc<Mutex<AppState>>,
        server: &Server,
        survivor: &Server,
        observation: PendingPluginObservation,
    ) {
        assert_eq!(observation.connected_sources, 1, "pending target baseline");
        assert_eq!(observation.disconnected_sources, 1, "target never began");
        assert!(!observation.shutdown, "shutdown before plugin response");
        // The captured pending generation existed before the response; a later
        // missing target row therefore proves removal, not mere non-creation.
        until("target source removed after plugin response", || {
            let scene = shared.lock().unwrap().scene();
            scene.disconnected_sources == 0 || scene.connected_sources != 1 || scene.shutdown
        });
        let state = shared.lock().unwrap();
        let scene = state.scene();
        assert_eq!(scene.connected_sources, 1, "survivor disconnected");
        assert_eq!(
            scene.disconnected_sources, 0,
            "target source remained pending"
        );
        assert!(!scene.shutdown, "watcher shut down");
        let rows = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert!(
            rows.iter()
                .all(|row| row.key.terminal_id != server.terminal),
            "detached target became a card"
        );
        assert!(
            rows.iter().any(|row| {
                row.key.terminal_id == survivor.terminal
                    && row.pane_id == survivor.pane
                    && row.availability == Availability::Live
            }),
            "survivor lost its live row"
        );
    }

    #[test]
    fn missing_plugin_still_publishes_standalone_agent() {
        let server = Server::new(Plugin::Missing, "standalone");
        let (shared, watchers) = watchers(true);
        watchers.register(server.path.clone()).unwrap();
        until("standalone live card", || live(&shared, &server).is_some());
        let scene = shared.lock().unwrap().scene();
        assert_eq!(scene.connected_sources, 1);
        assert_eq!(scene.sessions, 1);
        assert!(!scene.shutdown);
        drop(watchers);
    }

    #[test]
    fn explicit_disable_waits_for_initially_missing_survivor() {
        let disabled = Server::new(Plugin::Disabled, "disabled");
        let missing = Server::new(Plugin::Missing, "survivor");
        let probe = Server::new(Plugin::Missing, "post-disable-probe");
        let (shared, watchers) = watchers(false);
        watchers.register(missing.path.clone()).unwrap();
        until("missing survivor", || live(&shared, &missing).is_some());
        let observation =
            register_and_capture(&watchers, &shared, &disabled, &missing, Plugin::Disabled);
        until_detached(&shared, &disabled, &missing, observation);
        // A fresh registration can only be handled after the disabled target's
        // pass has made its shutdown decision; the existing survivor is no barrier.
        watchers.register(probe.path.clone()).unwrap();
        until("post-disable probe live", || {
            live(&shared, &probe).is_some()
        });
        let state = shared.lock().unwrap();
        let scene = state.scene();
        let rows = state
            .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
            .rows;
        assert_eq!(scene.connected_sources, 2);
        assert_eq!(scene.disconnected_sources, 0);
        assert!(!scene.shutdown);
        assert!(rows
            .iter()
            .all(|row| row.key.terminal_id != disabled.terminal));
        for server in [&missing, &probe] {
            assert!(rows.iter().any(|row| {
                row.key.terminal_id == server.terminal
                    && row.pane_id == server.pane
                    && row.availability == Availability::Live
            }));
        }
        drop(state);
        drop(watchers);
    }

    #[test]
    fn sole_explicit_disable_requests_immediate_shutdown() {
        let server = Server::new(Plugin::Disabled, "sole-disabled");
        let (shared, watchers) = watchers(false);
        watchers.register(server.path.clone()).unwrap();
        assert!(matches!(server.query(), Plugin::Disabled));
        until("confirmed disable shutdown", || {
            shared.lock().unwrap().scene().shutdown
        });
        assert!(live(&shared, &server).is_none());
        assert!(watchers.register(server.path.clone()).is_err());
        drop(watchers);
    }

    #[test]
    fn observed_unlink_detaches_and_re_registration_restores_agent() {
        let server = Server::new(Plugin::Enabled, "unlink");
        let survivor = Server::new(Plugin::Missing, "other");
        let (shared, watchers) = watchers(false);
        watchers.register(survivor.path.clone()).unwrap();
        watchers.register(server.path.clone()).unwrap();
        until("both live cards", || {
            live(&shared, &survivor).is_some() && live(&shared, &server).is_some()
        });
        let generation = live(&shared, &server).unwrap();
        assert_eq!(server.query(), Plugin::Enabled);
        server.set(Plugin::Missing);
        // An idle event schedules reconciliation within a worker tick (100 ms),
        // rather than waiting for the ordinary five-second interval.
        server.reconcile();
        until("unlinked source removed", || {
            let state = shared.lock().unwrap();
            state.scene().connected_sources == 1
                && state
                    .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                    .rows
                    .iter()
                    .all(|row| row.key.terminal_id != server.terminal)
        });
        assert_eq!(server.query(), Plugin::Missing);
        let observation =
            register_and_capture(&watchers, &shared, &server, &survivor, Plugin::Missing);
        until_detached(&shared, &server, &survivor, observation);
        server.set(Plugin::Enabled);
        watchers.register(server.path.clone()).unwrap();
        assert_eq!(server.query(), Plugin::Enabled);
        until("re-registered source restored", || {
            live(&shared, &server).is_some_and(|next| next > generation)
                && live(&shared, &survivor).is_some()
                && shared.lock().unwrap().scene().connected_sources == 2
        });
        drop(watchers);
    }

    #[test]
    fn initial_disable_then_missing_remains_detached_until_enabled() {
        let server = Server::new(Plugin::Disabled, "formerly-disabled");
        let survivor = Server::new(Plugin::Missing, "healthy");
        let (shared, watchers) = watchers(false);
        watchers.register(survivor.path.clone()).unwrap();
        until("healthy source", || live(&shared, &survivor).is_some());
        let disabled =
            register_and_capture(&watchers, &shared, &server, &survivor, Plugin::Disabled);
        until_detached(&shared, &server, &survivor, disabled);
        server.set(Plugin::Missing);
        let missing = register_and_capture(&watchers, &shared, &server, &survivor, Plugin::Missing);
        until_detached(&shared, &server, &survivor, missing);
        server.set(Plugin::Enabled);
        watchers.register(server.path.clone()).unwrap();
        assert_eq!(server.query(), Plugin::Enabled);
        until("enabled re-registration live", || {
            live(&shared, &server).is_some()
                && live(&shared, &survivor).is_some()
                && shared.lock().unwrap().scene().connected_sources == 2
        });
        assert!(!shared.lock().unwrap().scene().shutdown);
        drop(watchers);
    }

    #[test]
    fn malformed_and_api_plugin_queries_retry_instead_of_disabling() {
        for (index, mode) in [Plugin::Malformed, Plugin::Error].into_iter().enumerate() {
            let server = Server::new(mode, &format!("retry-{index}"));
            let (shared, watchers) = watchers(false);
            watchers.register(server.path.clone()).unwrap();
            assert!(matches!(server.query(), Plugin::Malformed | Plugin::Error));
            server.set(Plugin::Enabled);
            until("retry reaches live card", || {
                live(&shared, &server).is_some()
            });
            assert!(!shared.lock().unwrap().scene().shutdown);
            drop(watchers);
        }
    }
}

#[cfg(target_os = "macos")]
mod tern_control_boundary {
    use crate::character_service::PackService;
    use crate::control::{send_command, send_tern_exchange, ControlServer};
    use crate::herdr::{PromptError, PromptReceipt, Watchers};
    use crate::lifecycle::{LifecycleSettings, Paths};
    use crate::session_view::{Availability, SessionFilter, SessionListOptions};
    use crate::state::AppState;
    use crate::tern::{
        TernAgent, TernAgentState, TernExchangeErrorCode, TernExchangeRequest, TernResult,
        TernResultStatus, FRESHNESS, MAX_EXCHANGE_BYTES, MAX_RESPONSE_BYTES,
    };
    use herdr_update_coordinator::{ExecutableIdentity, ObservationMode, UpdateContext};
    use parking_lot::{Condvar, Mutex as GateMutex};
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

    // AppState and Watchers expose std mutexes; retain those API types while
    // keeping fixture-owned coordination on parking_lot.
    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct Fixture {
        root: PathBuf,
        paths: Paths,
        context: UpdateContext,
        shared: Arc<Mutex<AppState>>,
        watchers: Arc<Mutex<Watchers>>,
        control: ControlServer,
    }

    impl Fixture {
        fn new(mode: ObservationMode) -> Self {
            let root = PathBuf::from("/tmp").join(format!(
                "pet-tern-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("isolated fixture directory");
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let paths = Paths {
                config_dir: root.join("config"),
                state_dir: root.join("state"),
                control_socket: root.join("state/control.sock"),
                log_file: root.join("state/pet.log"),
            };
            let executable = root.join("pet");
            let context = UpdateContext {
                executable: executable.clone(),
                version: env!("CARGO_PKG_VERSION").into(),
                instance_id: format!("fixture-{}", root.display()),
                config_dir: paths.config_dir.clone(),
                host_plugin_config_dir: None,
                state_dir: paths.state_dir.clone(),
                // Neither the listener nor its parent exists. This is a launch
                // identity, not an active Herdr observation endpoint.
                herdr_socket: root.join("not-running/herdr.sock"),
                running: ExecutableIdentity {
                    path: executable.clone(),
                    sha256: "a".repeat(64),
                },
                running_origin: None,
                assets_override: None,
                environment: Default::default(),
                locale: "en".into(),
            };
            let shared = Arc::new(Mutex::new(AppState::new()));
            {
                let mut state = lock(&shared);
                state.set_observation_mode(mode);
                state.set_update_context(context.clone());
                state.set_ui_ready();
                state.set_lifecycle_settings(LifecycleSettings {
                    auto_start: true,
                    exit_with_herdr: false,
                });
            }
            let watchers = Arc::new(Mutex::new(Watchers::new(Arc::clone(&shared))));
            let packs = PackService::new(paths.config_dir.clone(), root.join("assets"), None);
            let mut control = ControlServer::bind(
                Arc::clone(&shared),
                Arc::clone(&watchers),
                paths.clone(),
                None,
                executable,
                packs,
            )
            .unwrap();
            control.start().unwrap();
            Self {
                root,
                paths,
                context,
                shared,
                watchers,
                control,
            }
        }

        fn exchange(&self, request: &TernExchangeRequest) -> crate::tern::TernExchangeReply {
            send_tern_exchange(&self.paths.control_socket, request, REQUEST_TIMEOUT).unwrap()
        }

        fn status(&self) -> crate::control::ControlResponse {
            send_command(&self.paths.control_socket, "status", None, REQUEST_TIMEOUT).unwrap()
        }

        fn availability(&self, pane: &str) -> Option<Availability> {
            lock(&self.shared)
                .session_snapshot(&SessionListOptions::with_filter(SessionFilter::All), None)
                .rows
                .into_iter()
                .find(|row| row.pane_id == pane)
                .map(|row| row.availability)
        }

        fn raw_exchange(&self, value: &Value) -> Vec<u8> {
            let mut stream = UnixStream::connect(&self.paths.control_socket).unwrap();
            stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
            writeln!(stream, "{value}").unwrap();
            let mut frame = Vec::new();
            BufReader::new(stream)
                .read_until(b'\n', &mut frame)
                .unwrap();
            assert_eq!(frame.last(), Some(&b'\n'));
            assert!(frame.len() <= MAX_RESPONSE_BYTES);
            frame
        }

        fn peer_reply(&self, reply: Value) -> Result<crate::tern::TernExchangeReply, String> {
            let path = self.root.join("reply.sock");
            let listener = UnixListener::bind(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let peer = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let request: TernExchangeRequest = serde_json::from_str(&line).unwrap();
                assert_eq!(request.window, "wire");
                // An oversize rejection can close the receiver while this
                // write is in progress; that is an expected transport result.
                let _ = writeln!(stream, "{reply}");
            });
            let result = send_tern_exchange(&path, &request("wire", "pane"), REQUEST_TIMEOUT);
            peer.join().unwrap();
            fs::remove_file(path).unwrap();
            result
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.control.shutdown();
            lock(&self.watchers).shutdown();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn request(window: &str, pane: &str) -> TernExchangeRequest {
        TernExchangeRequest {
            version: 1,
            kind: "tern".into(),
            window: window.into(),
            epoch: 1,
            sequence: 1,
            instance_id: None,
            agents: vec![TernAgent {
                pane: pane.into(),
                title: "fixture agent".into(),
                cwd: "/tmp".into(),
                session_id: None,
                session_label: None,
                tab_id: None,
                tab_label: None,
                state: TernAgentState::Idle,
                delivery_error: None,
            }],
            results: vec![],
        }
    }

    fn inventory_with_json_bytes(bytes: usize) -> TernExchangeRequest {
        let mut inventory = request("request-budget", "pane");
        let template = inventory.agents[0].clone();
        inventory.agents = (0..128)
            .map(|index| {
                let mut agent = template.clone();
                agent.pane = format!("pane-{index}");
                agent.title.clear();
                agent
            })
            .collect();
        let overhead = serde_json::to_vec(&inventory).unwrap().len();
        let mut remaining = bytes.checked_sub(overhead).unwrap();
        for agent in &mut inventory.agents {
            let length = remaining.min(4096);
            agent.title = "x".repeat(length);
            remaining -= length;
        }
        assert_eq!(remaining, 0);
        assert_eq!(serde_json::to_vec(&inventory).unwrap().len(), bytes);
        assert!(
            inventory.normalize().is_ok(),
            "inventory must be otherwise valid"
        );
        inventory
    }

    #[test]
    fn tern_request_json_boundary_excludes_optional_lf_in_helper_decoder_and_ipc() {
        let inventory = inventory_with_json_bytes(MAX_EXCHANGE_BYTES);
        let json = serde_json::to_vec(&inventory).unwrap();
        for terminator in [b"".as_slice(), b"\n".as_slice(), b"\r\n".as_slice()] {
            let mut input = json.clone();
            input.extend_from_slice(terminator);
            assert!(crate::control::decode_tern_exchange_request(&input).is_ok());
            assert!(crate::read_tern_exchange_request(input.as_slice()).is_ok());
        }
        let fixture = Fixture::new(ObservationMode::Tern);
        assert!(fixture.exchange(&inventory).ok);
        assert_eq!(fixture.status().sessions, 128);
        assert_eq!(fixture.status().connected_sources, 1);
        let fixture = Fixture::new(ObservationMode::Tern);
        let reply: Value = serde_json::from_slice(
            &fixture.raw_exchange(&serde_json::to_value(&inventory).unwrap()),
        )
        .unwrap();
        assert_eq!(reply["ok"], true);
        assert_eq!(fixture.status().sessions, 128);
    }

    #[test]
    fn oversized_valid_tern_inventory_rejects_before_helper_exchange_or_source_publication() {
        let fixture = Fixture::new(ObservationMode::Tern);
        assert!(fixture.exchange(&request("existing", "existing-pane")).ok);
        let before = fixture.status();
        for bytes in [MAX_EXCHANGE_BYTES + 1, 411_918] {
            let inventory = inventory_with_json_bytes(bytes);
            let json = serde_json::to_vec(&inventory).unwrap();
            let expected = "Tern exchange request JSON exceeds 256 KiB";
            for terminator in [b"".as_slice(), b"\n".as_slice(), b"\r\n".as_slice()] {
                let mut input = json.clone();
                input.extend_from_slice(terminator);
                assert_eq!(
                    crate::control::decode_tern_exchange_request(&input).unwrap_err(),
                    expected
                );
                assert_eq!(
                    crate::read_tern_exchange_request(input.as_slice()).unwrap_err(),
                    expected
                );
            }
            // No socket exists here: the sender must reject before connecting.
            assert_eq!(
                send_tern_exchange(
                    &fixture.root.join("absent.sock"),
                    &inventory,
                    REQUEST_TIMEOUT
                )
                .unwrap_err(),
                expected
            );
            let reply: Value = serde_json::from_slice(
                &fixture.raw_exchange(&serde_json::to_value(&inventory).unwrap()),
            )
            .unwrap();
            assert_eq!(reply["ok"], false);
            assert_eq!(reply["error_code"], "rejected");
            assert_eq!(reply["error"], expected);
            assert_eq!(reply["generation"], 0);
            assert_eq!(reply["command"], Value::Null);
            let after = fixture.status();
            assert_eq!(after.sessions, before.sessions);
            assert_eq!(after.connected_sources, before.connected_sources);
            assert_eq!(after.disconnected_sources, before.disconnected_sources);
            assert_eq!(after.working, before.working);
            assert_eq!(after.blocked, before.blocked);
            assert_eq!(after.done, before.done);
            assert_eq!(after.unknown, before.unknown);
            assert_eq!(fixture.availability("pane-0"), None);
        }
    }

    #[test]
    fn tern_only_ipc_status_preserves_configured_identity_without_registration() {
        let fixture = Fixture::new(ObservationMode::Tern);
        let status = fixture.status();
        assert!(status.ok && status.ready && status.control_ready && status.ui_ready);
        assert_eq!(status.instance_id, fixture.context.instance_id);
        assert_eq!(status.running_sha256, fixture.context.running.sha256);
        assert_eq!(
            status.herdr_socket,
            fixture.context.herdr_socket.display().to_string()
        );
        assert_eq!(
            status.config_dir,
            fixture.paths.config_dir.display().to_string()
        );
        assert_eq!(
            status.state_dir,
            fixture.paths.state_dir.display().to_string()
        );
        assert_eq!(status.observation_mode, "tern");
        assert_eq!(status.connected_sources, 0);
        assert_eq!(status.disconnected_sources, 0);
        assert!(!status.data_connected);
        assert!(!fixture.context.herdr_socket.exists());
        let ready = send_command(
            &fixture.paths.control_socket,
            "ready",
            None,
            REQUEST_TIMEOUT,
        )
        .unwrap();
        assert!(ready.ok && ready.ready);
        let register = send_command(
            &fixture.paths.control_socket,
            "register",
            Some(&fixture.context.herdr_socket),
            REQUEST_TIMEOUT,
        )
        .unwrap();
        assert!(!register.ok && !register.registration_accepted && !register.ready);
        assert_eq!(fixture.status().connected_sources, 0);
    }

    #[test]
    fn actual_exchange_forwards_typed_rejections_and_preserves_receipts_and_size_admission() {
        let fixture = Fixture::new(ObservationMode::Tern);
        let mut poll = request("bound", "pane");
        let first_wire = fixture.raw_exchange(&serde_json::to_value(&poll).unwrap());
        let first: Value = serde_json::from_slice(&first_wire).unwrap();
        assert_eq!(first["ok"], true);
        assert_eq!(first["command"], Value::Null);
        assert!(first.get("error_code").is_none());
        poll.sequence = 2;
        assert_eq!(
            fixture.exchange(&poll).error_code,
            Some(TernExchangeErrorCode::BindingRequired)
        );
        poll.instance_id = Some("other-daemon".into());
        assert_eq!(
            fixture.exchange(&poll).error_code,
            Some(TernExchangeErrorCode::InstanceChanged)
        );
        poll.instance_id = Some(fixture.context.instance_id.clone());
        let mut retired = poll.clone();
        retired.window = "unknown-bound-window".into();
        assert_eq!(
            fixture.exchange(&retired).error_code,
            Some(TernExchangeErrorCode::SourceRetired)
        );
        assert_eq!(
            fixture.status().sessions,
            1,
            "retired identity must not publish"
        );
        let mut stale = poll.clone();
        stale.sequence = 1;
        assert_eq!(
            fixture.exchange(&stale).error_code,
            Some(TernExchangeErrorCode::Rejected)
        );
        let key = lock(&fixture.shared)
            .session_snapshot(&SessionListOptions::default(), None)
            .rows[0]
            .key
            .clone();
        let escaped = "\u{1}".repeat(100_000);
        assert_eq!(
            lock(&fixture.shared)
                .queue_tern_prompt(key.clone(), "bound", 1, "pane", &escaped)
                .unwrap_err(),
            PromptError::TooLarge
        );
        let text = "✓\n".repeat(60_000);
        let (operation, receipt) = lock(&fixture.shared)
            .queue_tern_prompt(key, "bound", 1, "pane", &text)
            .unwrap();
        let offered = fixture.exchange(&poll);
        assert!(offered.ok && offered.error_code.is_none());
        let command = offered.command.unwrap();
        assert_eq!(command.operation_id, operation);
        assert_eq!(command.text, text);
        poll.sequence = 3;
        poll.results = vec![TernResult {
            operation_id: "not-the-offered-operation".into(),
            pane: "pane".into(),
            status: TernResultStatus::Accepted,
            error: None,
        }];
        let rejected = fixture.exchange(&poll);
        assert!(!rejected.ok && rejected.command.is_none() && rejected.generation == 0);
        assert_eq!(
            rejected.error_code,
            Some(TernExchangeErrorCode::ReceiptRejected)
        );
        assert!(matches!(receipt.try_recv(), Err(mpsc::TryRecvError::Empty)));
        poll.results[0].operation_id = operation;
        assert!(fixture.exchange(&poll).ok);
        assert_eq!(
            receipt.recv_timeout(REQUEST_TIMEOUT).unwrap(),
            Ok(PromptReceipt::Accepted)
        );
        poll.sequence = 4;
        poll.results.clear();
        assert!(fixture.exchange(&poll).command.is_none());

        let mut invalid = serde_json::to_value(&poll).unwrap();
        invalid
            .as_object_mut()
            .unwrap()
            .insert("é".repeat(3000), Value::Null);
        let reply: Value = serde_json::from_slice(&fixture.raw_exchange(&invalid)).unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error_code"], "rejected");
        assert_eq!(reply["generation"], 0);
        assert_eq!(reply["command"], Value::Null);
        let detail = reply["error"].as_str().unwrap();
        assert!(!detail.is_empty() && detail.len() <= 4096);
        let oversized = json!({"kind": "tern", "padding": "x".repeat(MAX_EXCHANGE_BYTES)});
        let reply: Value = serde_json::from_slice(&fixture.raw_exchange(&oversized)).unwrap();
        assert_eq!(reply["error_code"], "rejected");
        assert_eq!(reply["generation"], 0);
        assert_eq!(reply["command"], Value::Null);
        let mut oversized_poll = poll;
        oversized_poll.agents[0].title = "x".repeat(MAX_EXCHANGE_BYTES);
        assert!(send_tern_exchange(
            &fixture.paths.control_socket,
            &oversized_poll,
            REQUEST_TIMEOUT,
        )
        .is_err());
    }

    #[test]
    fn helper_transport_rejects_invalid_error_contract_and_honors_response_lf_limit() {
        let fixture = Fixture::new(ObservationMode::Tern);
        let failed = json!({
            "ok": false, "instance_id": "daemon", "generation": 0,
            "command": null, "error": "rejected", "error_code": "source_retired"
        });
        assert_eq!(
            fixture.peer_reply(failed.clone()).unwrap().error_code,
            Some(TernExchangeErrorCode::SourceRetired)
        );
        let mut invalid = Vec::new();
        let mut missing = failed.clone();
        missing.as_object_mut().unwrap().remove("error_code");
        invalid.push(missing);
        for (field, value) in [
            ("error_code", Value::Null),
            ("error_code", json!("unknown_code")),
            ("generation", json!(1)),
            ("error", json!("")),
            ("error", json!("e".repeat(4097))),
            ("command", json!({"operation_id":"o","pane":"p","text":"x"})),
        ] {
            let mut candidate = failed.clone();
            candidate[field] = value;
            invalid.push(candidate);
        }
        let success = json!({
            "ok": true, "instance_id": "daemon", "generation": 1,
            "command": null, "error": null
        });
        for value in [Value::Null, json!("rejected")] {
            let mut candidate = success.clone();
            candidate["error_code"] = value;
            invalid.push(candidate);
        }
        for candidate in invalid {
            assert!(fixture.peer_reply(candidate).is_err());
        }
        let mut largest = success;
        largest["command"] = json!({"operation_id":"operation","pane":"pane","text":""});
        largest["command"]["text"] = json!("x".repeat(300 * 1024));
        assert_eq!(
            fixture
                .peer_reply(largest.clone())
                .unwrap()
                .command
                .unwrap()
                .text
                .len(),
            300 * 1024
        );
        largest["command"]["text"] = json!("");
        let overhead = serde_json::to_vec(&largest).unwrap().len() + 1;
        let text = "x".repeat(MAX_RESPONSE_BYTES - overhead);
        largest["command"]["text"] = json!(text);
        assert_eq!(
            serde_json::to_vec(&largest).unwrap().len() + 1,
            MAX_RESPONSE_BYTES
        );
        assert_eq!(
            fixture
                .peer_reply(largest.clone())
                .unwrap()
                .command
                .unwrap()
                .text,
            text
        );
        largest["command"]["text"] = json!(format!("{text}x"));
        assert!(fixture.peer_reply(largest).is_err());
    }

    struct BlockedHerdrPass {
        gate: Arc<(GateMutex<bool>, Condvar)>,
        joins: Vec<JoinHandle<()>>,
        observed: mpsc::Receiver<()>,
        observations: mpsc::Sender<()>,
    }

    impl BlockedHerdrPass {
        fn new() -> Self {
            let (observations, observed) = mpsc::channel();
            Self {
                gate: Arc::new((GateMutex::new(false), Condvar::new())),
                joins: vec![],
                observed,
                observations,
            }
        }

        fn register(&mut self, fixture: &Fixture, index: usize) {
            let path = fixture.root.join(format!("h{index}.sock"));
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let gate = Arc::clone(&self.gate);
            let observed = self.observations.clone();
            self.joins.push(thread::spawn(move || {
                let mut stream = loop {
                    if *gate.0.lock() {
                        return;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("fake Herdr accept: {error}"),
                    }
                };
                // Accepted sockets can inherit the listener's nonblocking flag.
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert!(request["method"].is_string(), "real Herdr RPC expected");
                observed.send(()).unwrap();
                // Keep a real request unanswered until cleanup. Every endpoint
                // consumes its own native deadline, blocking a whole watcher
                // pass rather than a synthetic copied expiry implementation.
                let mut released = gate.0.lock();
                while !*released {
                    gate.1.wait(&mut released);
                }
            }));
            lock(&fixture.watchers).register(path).unwrap();
        }

        fn observe(&self) {
            self.observed.recv_timeout(Duration::from_secs(3)).unwrap();
        }
    }

    impl Drop for BlockedHerdrPass {
        fn drop(&mut self) {
            *self.gate.0.lock() = true;
            self.gate.1.notify_all();
            for join in self.joins.drain(..) {
                let result = join.join();
                if !thread::panicking() {
                    result.expect("blocked Herdr fixture thread");
                }
            }
        }
    }

    struct ClientFlood {
        stop: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
    }

    impl ClientFlood {
        fn new(path: PathBuf) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let join = thread::spawn(move || {
                let mut held = Vec::new();
                for _ in 0..32 {
                    if let Ok(stream) =
                        crate::socket::connect(&path, Instant::now() + Duration::from_millis(100))
                    {
                        held.push(stream);
                    }
                }
                while !worker_stop.load(Ordering::Acquire) {
                    let _ =
                        crate::socket::connect(&path, Instant::now() + Duration::from_millis(100));
                    thread::sleep(Duration::from_millis(1));
                }
                drop(held);
            });
            Self {
                stop,
                join: Some(join),
            }
        }
    }

    impl Drop for ClientFlood {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            let _ = self.join.take().unwrap().join();
        }
    }

    #[test]
    fn control_expiry_is_independent_of_blocked_herdr_passes_busy_clients_and_idle() {
        let fixture = Fixture::new(ObservationMode::Both);
        let mut peers = BlockedHerdrPass::new();
        peers.register(&fixture, 0);
        peers.observe();
        // Queue the rest while the first peer owns the current watcher pass.
        // The next pass drains all registrations before starting its serial,
        // independently bounded Herdr requests.
        for index in 1..=14 {
            peers.register(&fixture, index);
        }
        peers.observe();
        assert!(fixture.exchange(&request("busy-window", "busy-pane")).ok);
        let busy_deadline = Instant::now() + FRESHNESS + Duration::from_millis(750);
        let flood = ClientFlood::new(fixture.paths.control_socket.clone());
        thread::sleep(Duration::from_secs(1));
        // Use AppState's real exchange admission for the second inventory while
        // client slots are deliberately saturated. No expiry is invoked here.
        assert!(
            lock(&fixture.shared)
                .tern_exchange(request("idle-window", "idle-pane"))
                .ok
        );
        while fixture.availability("busy-pane") != Some(Availability::Offline) {
            assert!(
                Instant::now() < busy_deadline,
                "Tern expiry waited for the Herdr pass"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fixture.availability("idle-pane"), Some(Availability::Live));
        drop(flood);
        let idle_deadline = Instant::now() + Duration::from_secs(2);
        while fixture.availability("idle-pane") != Some(Availability::Offline) {
            assert!(
                Instant::now() < idle_deadline,
                "idle control listener did not expire Tern"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let status = fixture.status();
        assert!(status.disconnected_sources >= 2);
        assert!(!status.shutdown);
        drop(peers);
        lock(&fixture.watchers).shutdown();
    }
}
