use pet_bridge::{
    delivery::{DeliveryOutcome, Discovery, deliver},
    outbox::Outbox,
};
use pet_core::{AppPaths, EventEnvelope, decode_event, paths::wall_clock_ms};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener},
    thread,
    time::{Duration, Instant},
};

fn event() -> EventEnvelope {
    decode_event(br#"{"schemaVersion":2,"eventId":"test-queued","identity":{"source":"deepseek","installationId":"test-install","instanceId":"test-instance"},"sessionId":"test-session","turnId":"test-turn","occurredAtMs":100,"kind":{"type":"turn_started"}}"#)
        .unwrap()
}

fn paths(root: &tempfile::TempDir) -> AppPaths {
    AppPaths::under(&root.path().canonicalize().unwrap().join("app"))
}

#[test]
fn only_complete_matching_ack_deletes_the_durable_queue_entry() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let event = event();
    for (reply, expected) in [
        (
            br#"{"eventId":"different","duplicate":false,"revision":9}"#.as_slice(),
            DeliveryOutcome::Queued,
        ),
        (
            br#"{"eventId":"test-queued""#.as_slice(),
            DeliveryOutcome::Queued,
        ),
        (
            br#"{"eventId":"test-queued","duplicate":true,"revision":9}"#.as_slice(),
            DeliveryOutcome::Delivered,
        ),
    ] {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let discovery = Discovery::new(listener.local_addr().unwrap().port());
        discovery.publish(&paths).unwrap();

        let server_paths = paths.clone();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut socket = BufReader::new(socket);
            let mut line = String::new();
            let mut length = 0;
            loop {
                line.clear();
                assert!(socket.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            assert_eq!(decode_event(&body).unwrap().event_id, "test-queued");
            let outbox = Outbox::new(server_paths).unwrap();
            assert_eq!(outbox.pending(wall_clock_ms()).unwrap().len(), 1);
            write!(
                socket.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            )
            .unwrap();
            socket.get_mut().write_all(reply).unwrap();
        });

        assert_eq!(deliver(&event, &paths).unwrap(), expected);
        server.join().unwrap();
        let outbox = Outbox::new(paths.clone()).unwrap();
        assert_eq!(
            outbox.pending(wall_clock_ms()).unwrap().len(),
            usize::from(expected == DeliveryOutcome::Queued)
        );
    }
}

#[test]
fn slowly_arriving_ack_uses_one_network_budget() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    Discovery::new(listener.local_addr().unwrap().port())
        .publish(&paths)
        .unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let ack = br#"{"eventId":"test-queued","duplicate":false,"revision":1}"#;
        thread::sleep(Duration::from_millis(110));
        if write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            ack.len()
        )
        .is_ok()
        {
            let _ = socket.write_all(&ack[..10]);
            thread::sleep(Duration::from_millis(130));
            let _ = socket.write_all(&ack[10..]);
        }
    });
    let started = Instant::now();
    assert_eq!(deliver(&event(), &paths).unwrap(), DeliveryOutcome::Queued);
    assert!(started.elapsed() < Duration::from_millis(350));
    assert_eq!(
        Outbox::new(paths)
            .unwrap()
            .pending(wall_clock_ms())
            .unwrap()
            .len(),
        1
    );
    server.join().unwrap();
}

#[test]
fn stale_discovery_keeps_queue_and_invalid_outbox_reports_failure() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(&root);
    let stale = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    Discovery::new(stale.local_addr().unwrap().port())
        .publish(&paths)
        .unwrap();
    drop(stale);
    assert_eq!(deliver(&event(), &paths).unwrap(), DeliveryOutcome::Queued);
    assert_eq!(
        Outbox::new(paths.clone())
            .unwrap()
            .pending(wall_clock_ms())
            .unwrap()
            .len(),
        1
    );
    let blocked = AppPaths::under(&paths.root.join("blocked"));
    fs::write(&blocked.root, b"file").unwrap();
    assert!(deliver(&event(), &blocked).is_err());
    assert!(!blocked.outbox.exists());
}
