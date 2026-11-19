use pet_bridge::outbox::Outbox;
use pet_core::{AppPaths, EventEnvelope, MAX_EVENT_BYTES, ProjectContext, decode_event};
use serde_json::json;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tempfile::{TempDir, tempdir};

const DAY_MS: u64 = 86_400_000;
const NOW: u64 = DAY_MS * 10;
const CHILD_ROOT: &str = "PET_BRIDGE_OUTBOX_CHILD_ROOT";
const CHILD_INDEX: &str = "PET_BRIDGE_OUTBOX_CHILD_INDEX";

fn sandbox() -> (TempDir, AppPaths) {
    let dir = tempdir().unwrap();
    let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
    (dir, paths)
}

fn event(id: &str) -> EventEnvelope {
    decode_event(
        &serde_json::to_vec(&json!({
            "schemaVersion": 2, "eventId": id,
            "identity": {"source":"codex","installationId":"local","instanceId":"hook-session:s"},
            "sessionId": "s", "occurredAtMs": 0, "kind": {"type":"turn_started"}
        }))
        .unwrap(),
    )
    .unwrap()
}

fn fixed(paths: &AppPaths, now: u64) -> Outbox {
    Outbox::with_clock(paths.clone(), move || now).unwrap()
}

fn padded_event(id: &str, size: usize) -> EventEnvelope {
    let mut event = event(id);
    event.project = Some(ProjectContext {
        name: "project".into(),
        directory: Some(String::new()),
    });
    let padding = size - serde_json::to_vec(&event).unwrap().len();
    event.project.as_mut().unwrap().directory = Some("a".repeat(padding));
    event
}

fn corrupt_payload(outbox: &Outbox, id: &str) {
    let events = outbox.pending(NOW).unwrap();
    let slot = events
        .iter()
        .find(|slot| slot.envelope.event_id == id)
        .unwrap();
    fs::write(&slot.path, vec![b'!'; MAX_EVENT_BYTES]).unwrap();
}

fn pending_ids(outbox: &Outbox) -> Vec<String> {
    outbox
        .pending(NOW)
        .unwrap()
        .into_iter()
        .map(|slot| slot.envelope.event_id)
        .collect()
}

#[test]
fn count_limit_evicts_oldest_in_fifo_order_even_in_same_millisecond() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    for index in 0..=2000 {
        outbox.enqueue(&event(&format!("event-{index}"))).unwrap();
    }
    let queued = outbox.pending(NOW).unwrap();
    assert_eq!(queued.len(), 2000);
    assert_eq!(queued[0].envelope.event_id, "event-1");
    assert_eq!(queued[1].envelope.event_id, "event-2");
    assert_eq!(queued[1999].envelope.event_id, "event-2000");
    assert_eq!(queued[0].queued_at_ms, NOW);
    assert_eq!(
        fixed(&paths, NOW).diagnostics().unwrap().overflow_discarded,
        1
    );
}

#[test]
fn byte_limit_counts_durable_files_and_evicts_oldest() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    let directory = "a".repeat(60_000);
    for index in 0..180 {
        let mut envelope = event(&format!("event-{index}"));
        envelope.project = Some(pet_core::ProjectContext {
            name: "project".into(),
            directory: Some(directory.clone()),
        });
        outbox.enqueue(&envelope).unwrap();
    }
    let queued = outbox.pending(NOW).unwrap();
    let total: u64 = queued
        .iter()
        .map(|entry| fs::metadata(&entry.path).unwrap().len())
        .sum();
    assert!(total <= 10 * 1024 * 1024);
    assert!(total > 10 * 1024 * 1024 - 65_536);
    assert_eq!(queued.last().unwrap().envelope.event_id, "event-179");
    assert_eq!(
        outbox.diagnostics().unwrap().overflow_discarded,
        (180 - queued.len()) as u64
    );
}

#[test]
fn expiry_uses_local_queue_time_and_wins_before_pressure_eviction() {
    let (_dir, paths) = sandbox();
    let time = Arc::new(AtomicU64::new(NOW));
    let outbox = {
        let time = time.clone();
        Outbox::with_clock(paths.clone(), move || time.load(Ordering::Relaxed)).unwrap()
    };
    let mut ancient = event("ancient");
    ancient.received_at_ms = Some(0);
    outbox.enqueue(&ancient).unwrap();
    time.store(NOW + 1, Ordering::Relaxed);
    let mut future = event("future");
    future.occurred_at_ms = u64::MAX;
    outbox.enqueue(&future).unwrap();
    assert_eq!(outbox.pending(NOW + DAY_MS - 1).unwrap().len(), 2);
    time.store(NOW + DAY_MS, Ordering::Relaxed);
    for index in 0..1999 {
        outbox.enqueue(&event(&format!("event-{index}"))).unwrap();
    }
    let queued = outbox.pending(NOW + DAY_MS).unwrap();
    assert_eq!(queued.len(), 2000);
    assert_eq!(queued[0].envelope.event_id, "future");
    assert_eq!(queued[0].queued_at_ms, NOW + 1);
    let counts = outbox.diagnostics().unwrap();
    assert_eq!(counts.expired_discarded, 1);
    assert_eq!(counts.overflow_discarded, 0);
}

#[test]
fn duplicate_enqueue_preserves_original_age_and_ack_is_idempotent() {
    let (_dir, paths) = sandbox();
    let mut initial = event("../../elsewhere");
    initial.summary = Some("最初".into());
    fixed(&paths, NOW).enqueue(&initial).unwrap();
    let mut retry = initial.clone();
    retry.summary = Some("再次".into());
    let outbox = fixed(&paths, NOW + 1);
    outbox.enqueue(&retry).unwrap();
    let queued = outbox.pending(NOW + 1).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].queued_at_ms, NOW);
    assert_eq!(queued[0].path.parent(), Some(paths.outbox.as_path()));
    assert_eq!(queued[0].envelope, initial);
    outbox.acknowledge("../../elsewhere").unwrap();
    outbox.acknowledge("../../elsewhere").unwrap();
    assert!(outbox.pending(NOW + 1).unwrap().is_empty());
}

#[test]
fn corruption_is_isolated_and_diagnostic_survives_restart() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("broken")).unwrap();
    outbox.enqueue(&event("valid")).unwrap();
    let queued = outbox.pending(NOW).unwrap();
    let broken = &queued[0].path;
    fs::write(broken, b"partial json").unwrap();
    let after = outbox.pending(NOW).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].envelope.event_id, "valid");
    assert!(!broken.exists());
    assert_eq!(
        fixed(&paths, NOW).diagnostics().unwrap().corrupt_isolated,
        1
    );
    let quarantined = fs::read_dir(paths.outbox.join("quarantine"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(fs::read(quarantined.path()).unwrap(), b"partial json");
}

#[cfg(unix)]
#[test]
fn ordinary_file_with_punctuation_is_quarantined_without_blocking_the_queue() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("first")).unwrap();
    let sample = paths.outbox.join("普通样例:备注.txt.");
    fs::write(&sample, b"sample").unwrap();

    assert_eq!(pending_ids(&outbox), ["first"]);
    assert!(!sample.exists());
    drop(outbox);

    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("second")).unwrap();
    assert_eq!(pending_ids(&outbox), ["first", "second"]);
    assert_eq!(outbox.diagnostics().unwrap().corrupt_isolated, 1);
}

#[test]
fn replay_skips_a_missing_previously_rejected_cleanup_item() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("first")).unwrap();
    drop(outbox);
    fs::write(
        paths.outbox.join(".outbox-state.json"),
        serde_json::to_vec(&json!({
            "diagnostics": {"expiredDiscarded": 0, "overflowDiscarded": 0, "corruptIsolated": 1},
            "cleanup": [{"name": ".OUTBOX-STAGE-orphan", "quarantine_index": 0}]
        }))
        .unwrap(),
    )
    .unwrap();

    let outbox = fixed(&paths, NOW);
    assert_eq!(pending_ids(&outbox), ["first"]);
    outbox.enqueue(&event("second")).unwrap();
    assert_eq!(pending_ids(&outbox), ["first", "second"]);
    assert_eq!(outbox.diagnostics().unwrap().corrupt_isolated, 1);
}

#[test]
fn enqueue_quarantines_bad_payload_before_evicting_valid_events() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    // 160 个 64 KiB 的合法事件恰好占满 10 MiB。
    let ids: Vec<_> = (0..160).map(|index| format!("event-{index}")).collect();
    for id in &ids {
        outbox.enqueue(&padded_event(id, MAX_EVENT_BYTES)).unwrap();
    }
    corrupt_payload(&outbox, "event-80");

    outbox
        .enqueue(&padded_event("latest", MAX_EVENT_BYTES))
        .unwrap();
    let expected: Vec<_> = ids
        .into_iter()
        .filter(|id| id != "event-80")
        .chain(["latest".into()])
        .collect();
    assert_eq!(pending_ids(&outbox), expected);
    let diagnostics = outbox.diagnostics().unwrap();
    assert_eq!(diagnostics.corrupt_isolated, 1);
    assert_eq!(diagnostics.overflow_discarded, 0);
}

#[test]
fn acknowledge_quarantines_bad_payload_before_evicting_valid_events() {
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("ack-this")).unwrap();
    let mut expected = Vec::new();
    for index in 0..159 {
        let id = format!("event-{index}");
        outbox.enqueue(&padded_event(&id, MAX_EVENT_BYTES)).unwrap();
        expected.push(id);
    }
    outbox
        .enqueue(&padded_event("last-valid", MAX_EVENT_BYTES - 1_024))
        .unwrap();
    expected.push("last-valid".into());
    outbox.enqueue(&event("corrupt-this")).unwrap();
    // 模拟磁盘上的损坏文件增大。删除 ACK 对应文件后仍有表面上的配额压力。
    corrupt_payload(&outbox, "corrupt-this");

    outbox.acknowledge("ack-this").unwrap();
    assert_eq!(pending_ids(&outbox), expected);
    let diagnostics = outbox.diagnostics().unwrap();
    assert_eq!(diagnostics.corrupt_isolated, 1);
    assert_eq!(diagnostics.overflow_discarded, 0);
}

#[cfg(unix)]
#[test]
fn interrupted_isolation_retries_without_double_counting() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, paths) = sandbox();
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("broken")).unwrap();
    outbox.enqueue(&event("valid")).unwrap();
    let broken = outbox.pending(NOW).unwrap().remove(0).path;
    fs::write(&broken, b"partial").unwrap();
    let quarantine = paths.outbox.join("quarantine");
    pet_core::paths::prepare_directory(&quarantine).unwrap();
    fs::set_permissions(&quarantine, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = quarantine.join("probe");
    if fs::write(&probe, b"").is_ok() {
        // 以 root 运行时仍可写入权限为 0500 的目录。
        return;
    }

    assert!(outbox.pending(NOW).is_err());
    assert!(broken.exists());
    drop(outbox);
    fs::set_permissions(&quarantine, fs::Permissions::from_mode(0o700)).unwrap();

    let outbox = fixed(&paths, NOW);
    let queued = outbox.pending(NOW).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].envelope.event_id, "valid");
    assert!(!broken.exists());
    assert_eq!(outbox.diagnostics().unwrap().corrupt_isolated, 1);
}

#[test]
fn child_enqueues_batch() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let index = std::env::var(CHILD_INDEX).unwrap();
    let paths = AppPaths::under(std::path::Path::new(&root));
    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("shared")).unwrap();
    for offset in 0..15 {
        outbox
            .enqueue(&event(&format!("{index}-{offset}")))
            .unwrap();
    }
}

#[test]
fn independent_processes_share_the_queue_lock() {
    let (_dir, paths) = sandbox();
    let binary = std::env::current_exe().unwrap();
    let outbox = fixed(&paths, NOW);
    for index in 0..1950 {
        outbox.enqueue(&event(&format!("seed-{index}"))).unwrap();
    }
    let mut children = Vec::new();
    for index in 0..5 {
        children.push(
            Command::new(&binary)
                .args(["--exact", "child_enqueues_batch"])
                .env(CHILD_ROOT, &paths.root)
                .env(CHILD_INDEX, index.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let queued = outbox.pending(NOW).unwrap();
    assert_eq!(queued.len(), 2000);
    assert_eq!(queued[0].envelope.event_id, "seed-26");
    assert_eq!(
        queued
            .iter()
            .filter(|entry| entry.envelope.event_id == "shared")
            .count(),
        1
    );
    assert_eq!(outbox.diagnostics().unwrap().overflow_discarded, 26);
}

#[cfg(unix)]
#[test]
fn linked_queue_paths_never_affect_the_target() {
    use std::os::unix::fs::symlink;
    let (_dir, paths) = sandbox();
    let outside = tempdir().unwrap();
    let payload = outside.path().join("outside");
    fs::write(&payload, b"outside").unwrap();
    paths.prepare_root().unwrap();
    symlink(outside.path(), &paths.outbox).unwrap();
    assert!(Outbox::with_clock(paths.clone(), || NOW).is_err());
    fs::remove_file(&paths.outbox).unwrap();

    let outbox = fixed(&paths, NOW);
    outbox.enqueue(&event("link")).unwrap();
    let slot = &outbox.pending(NOW).unwrap()[0].path;
    fs::remove_file(slot).unwrap();
    symlink(&payload, slot).unwrap();
    assert!(outbox.pending(NOW).is_err());
    assert_eq!(fs::read(payload).unwrap(), b"outside");
}
