use pet_bridge::{
    delivery::{DeliveryOutcome, Discovery, deliver},
    outbox::Outbox,
};
use pet_core::{
    AppPaths, AttentionReason, EventEnvelope, EventKind, JournalStore, NotificationKind,
    RequestLifetime, SessionStatus, TurnOutcome, decode_event, paths::wall_clock_ms,
};
use pet_desktop::broker::BrokerHandle;
use std::{
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, TcpStream},
    thread,
    time::{Duration, Instant},
};

fn paths(root: &tempfile::TempDir) -> AppPaths {
    AppPaths::under(&root.path().canonicalize().unwrap().join("app"))
}

fn event(id: &str, kind: EventKind) -> EventEnvelope {
    let mut event = decode_event(br#"{"schemaVersion":2,"eventId":"placeholder","identity":{"source":"deepseek","installationId":"fixture-install","instanceId":"fixture-instance"},"sessionId":"fixture-session","turnId":"fixture-turn","occurredAtMs":100,"kind":{"type":"turn_started"}}"#)
        .unwrap();
    event.event_id = id.to_owned();
    event.kind = kind;
    event.occurred_at_ms = wall_clock_ms();
    event
}

fn send(discovery: &Discovery, token: &str, extra: &str, bytes: &[u8]) -> TcpStream {
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, discovery.port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        socket,
        "POST /v1/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        discovery.port,
        bytes.len()
    )
    .unwrap();
    socket.write_all(bytes).unwrap();
    socket
}

fn response(mut socket: TcpStream) -> String {
    let mut reply = String::new();
    socket.read_to_string(&mut reply).unwrap();
    reply
}

fn end(id: &str) -> EventEnvelope {
    event(
        id,
        EventKind::TurnEnded {
            outcome: TurnOutcome::Completed,
        },
    )
}

fn wait_until(mut done: impl FnMut() -> bool, budget: Duration) {
    let deadline = Instant::now() + budget;
    while !done() {
        assert!(Instant::now() < deadline, "等待事件落盘超时");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rejects_wrong_token_origin_oversize_and_unwritable_journal_without_ack() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let discovery = Discovery::read(&paths).unwrap();
    let bytes = serde_json::to_vec(&end("rejected")).unwrap();
    for (token, extra, body, status) in [
        ("wrong", "", bytes.as_slice(), "401"),
        (
            discovery.token.as_str(),
            "Origin: null\r\n",
            bytes.as_slice(),
            "403",
        ),
        (discovery.token.as_str(), "", &vec![b' '; 65537], "413"),
    ] {
        let reply = response(send(&discovery, token, extra, body));
        assert!(reply.starts_with(&format!("HTTP/1.1 {status}")), "{reply}");
        assert!(!reply.contains("eventId"));
    }
    fs::remove_file(&paths.journal).unwrap();
    fs::create_dir(&paths.journal).unwrap();
    let reply = response(send(&discovery, &discovery.token, "", &bytes));
    assert!(reply.starts_with("HTTP/1.1 503"), "{reply}");
    assert!(!reply.contains("eventId"));
    assert!(broker.snapshot().notifications.is_empty());
}

#[test]
fn response_lost_then_retry_is_deduplicated_across_restart_and_watch_scan() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let first = Discovery::read(&paths).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&paths.discovery).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    let mut updates = broker.subscribe();
    let event = end("ack-lost");
    let bytes = serde_json::to_vec(&event).unwrap();
    let socket = send(&first, &first.token, "", &bytes);
    socket.shutdown(Shutdown::Read).unwrap();
    wait_until(|| updates.try_recv().is_ok(), Duration::from_secs(2));
    assert_eq!(broker.snapshot().notifications.len(), 1);
    drop(socket);
    drop(broker);

    // 重放崩溃遗留的旧发现文件，随后验证新进程轮换令牌。
    first.publish(&paths).unwrap();
    assert_eq!(deliver(&event, &paths).unwrap(), DeliveryOutcome::Queued);
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let second = Discovery::read(&paths).unwrap();
    assert_ne!(first.token, second.token);
    let mut updates = broker.subscribe();
    let old = response(send(&second, &first.token, "", &bytes));
    assert!(old.starts_with("HTTP/1.1 401"));
    let reply = response(send(&second, &second.token, "", &bytes));
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(
        reply.contains(r#""eventId":"ack-lost","duplicate":true"#),
        "{reply}"
    );
    assert_eq!(broker.snapshot().notifications.len(), 1);
    assert!(updates.try_recv().is_err());
    assert!(
        Outbox::new(paths.clone())
            .unwrap()
            .pending(wall_clock_ms())
            .unwrap()
            .is_empty()
    );
    drop(broker);
    assert_eq!(
        JournalStore::open(paths)
            .unwrap()
            .snapshot()
            .notifications
            .len(),
        1
    );
}

#[test]
fn offline_requested_then_resolved_never_bubbles() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let request = event(
        "request",
        EventKind::AttentionRequested {
            request_id: "same-question".into(),
            reason: AttentionReason::Answer,
            lifetime: RequestLifetime::UntilResolved,
        },
    );
    let resolve = event(
        "resolve",
        EventKind::AttentionResolved {
            request_id: "same-question".into(),
        },
    );
    {
        let mut journal = JournalStore::open(paths.clone()).unwrap();
        let mut previous = end("earlier-end");
        previous.session_id = "earlier-session".into();
        previous.turn_id = Some("earlier-turn".into());
        journal.ingest(previous).unwrap();
    }
    for event in [
        event("start", EventKind::TurnStarted),
        request,
        resolve,
        end("end"),
    ] {
        assert_eq!(deliver(&event, &paths).unwrap(), DeliveryOutcome::Queued);
    }
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let snapshot = broker.snapshot();
    let current = snapshot
        .sessions
        .iter()
        .find(|session| session.key.session_id == "fixture-session")
        .expect("启动快照缺少离线会话");
    assert_eq!(current.status, SessionStatus::Ended);
    assert!(current.pending.is_empty());
    assert!(
        snapshot
            .notifications
            .iter()
            .any(|item| item.id == "earlier-end")
    );
    assert!(!snapshot.notifications.iter().any(|item| matches!(
        item.kind,
        NotificationKind::Attention {
            resolved: false,
            ..
        }
    )));
    assert!(
        Outbox::new(paths)
            .unwrap()
            .pending(wall_clock_ms())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn file_watch_and_explicit_rescan_use_the_same_durable_writer() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let mut updates = broker.subscribe();
    let outbox = Outbox::new(paths.clone()).unwrap();
    outbox.enqueue(&end("watch")).unwrap();
    wait_until(|| updates.try_recv().is_ok(), Duration::from_secs(1));
    assert_eq!(broker.snapshot().notifications.len(), 1);
    let mut recovered = end("second-turn");
    recovered.turn_id = Some("second-turn".into());
    outbox.enqueue(&recovered).unwrap();
    broker.rescan();
    wait_until(|| updates.try_recv().is_ok(), Duration::from_secs(1));
    assert_eq!(broker.snapshot().notifications.len(), 2);
    assert!(outbox.pending(wall_clock_ms()).unwrap().is_empty());
}
