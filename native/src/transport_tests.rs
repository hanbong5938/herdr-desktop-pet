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
    use crate::herdr::{PromptError, PromptSender};
    use crate::herdr_protocol::{AgentRecord, AgentStatus, SessionMetadata};
    use crate::session_view::{SessionFilter, SessionKey};
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
            &[],
            Instant::now()
        ));
        state
            .session_snapshot(SessionFilter::All, None)
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
            sender.submit(key.clone(), "must not send".into()),
            Err(PromptError::ReadOnly)
        );
        assert!(shared
            .lock()
            .unwrap()
            .update_source(&source, 1, false, SourceCounts::default()));
        assert_eq!(
            sender.submit(key, "offline remote is still read-only".into()),
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
            .session_snapshot(SessionFilter::All, Some(&key))
            .rows
            .is_empty());
        assert_eq!(
            shared.lock().unwrap().prompt_available(&key),
            Err(PromptError::StaleTarget)
        );
        assert_eq!(
            sender.submit(key.clone(), "hidden".into()),
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
        let snapshot = state.session_snapshot(SessionFilter::All, None);
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
        sender.submit(key, "never deliver".into()).unwrap();
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
            sender.submit(key.clone(), " \n ".into()),
            Err(PromptError::Empty)
        );
        assert_eq!(
            sender.submit(
                key.clone(),
                "x".repeat(crate::herdr_protocol::MAX_FRAME_BYTES)
            ),
            Err(PromptError::TooLarge)
        );
        shared
            .lock()
            .unwrap()
            .update_source(&server.source(), 1, false, SourceCounts::default());
        assert_eq!(
            sender.submit(key.clone(), "hi".into()),
            Err(PromptError::Offline)
        );
        shared.lock().unwrap().begin_source(server.source(), 2);
        assert_eq!(
            sender.submit(key.clone(), "hi".into()),
            Err(PromptError::StaleTarget)
        );
        let current = publish_after_generation(
            &shared,
            &server.source(),
            2,
            &[record("terminal", "pane"), record("other", "pane")],
        );
        assert_eq!(
            sender.submit(current, "hi".into()),
            Err(PromptError::StaleTarget)
        );
        shared.lock().unwrap().request_shutdown();
        assert_eq!(sender.submit(key, "hi".into()), Err(PromptError::Offline));
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
            &[],
            Instant::now()
        ));
        state
            .session_snapshot(SessionFilter::All, None)
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
            .submit(key.clone(), "  first\nsecond  ".to_owned())
            .unwrap();
        assert_eq!(
            sender.submit(key.clone(), "again".into()),
            Err(PromptError::Busy)
        );
        let result = receive(&mut sender);
        assert_eq!(result.key, key);
        assert_eq!(result.text, "  first\nsecond  ");
        assert_eq!(result.result, Ok(()));
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
        sender.submit(key, "message".into()).unwrap();
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
            sender.submit(key, "text".into()).unwrap();
            let result = receive(&mut sender);
            assert_eq!(result.result, Err(expected));
            assert_eq!(result.text, "text");
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
        sender.submit(key, "do not duplicate".into()).unwrap();
        let result = receive(&mut sender);
        assert_eq!(result.result, Err(PromptError::UnknownDelivery));
        assert_eq!(result.text, "do not duplicate");
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
        sender.submit(key, "identity".into()).unwrap();
        assert_eq!(
            receive(&mut sender).result,
            Err(PromptError::UnknownDelivery)
        );
        worker.join().unwrap();
    }
}
