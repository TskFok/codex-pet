mod support;

use pet_core::{AppPaths, EventEnvelope, JournalStore, SessionStatus};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;
use support::event;
use tempfile::{TempDir, tempdir};

const DAY_MS: u64 = 86_400_000;
const NOW: u64 = DAY_MS * 10;
const CHILD_ROOT: &str = "PET_CORE_STORAGE_CHILD_ROOT";

fn sandbox() -> (TempDir, AppPaths) {
    let dir = tempdir().unwrap();
    let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
    (dir, paths)
}

fn open(paths: &AppPaths, now: u64) -> JournalStore {
    JournalStore::open_with_clock(paths.clone(), move || now).unwrap()
}

fn happened(id: &str, turn: &str, sequence: Option<u64>, kind: serde_json::Value) -> EventEnvelope {
    let mut value = event(id, Some(turn), sequence, kind);
    value.occurred_at_ms = NOW;
    value
}

fn end(id: &str, sequence: u64) -> EventEnvelope {
    happened(
        id,
        id,
        Some(sequence),
        json!({"type":"turn_ended","outcome":"completed"}),
    )
}

fn persistent(id: &str, turn: &str) -> EventEnvelope {
    happened(
        id,
        turn,
        None,
        json!({
            "type":"attention_requested","requestId":id,"reason":"answer","lifetime":"until_resolved"
        }),
    )
}

#[test]
fn acknowledged_event_survives_restart() {
    let (_dir, paths) = sandbox();
    let event = end("ended", 1);
    let ack = open(&paths, NOW).ingest(event.clone()).unwrap();
    assert!(!ack.duplicate);
    assert_eq!(ack.event_id, "ended");

    let mut reopened = open(&paths, NOW);
    let snapshot = reopened.recover(NOW).unwrap();
    assert_eq!(snapshot.revision, ack.revision);
    assert_eq!(snapshot.notifications[0].id, "ended");
    assert!(reopened.ingest(event).unwrap().duplicate);
}

#[test]
fn partial_tail_preserves_prior_records() {
    let (_dir, paths) = sandbox();
    open(&paths, NOW).ingest(end("first", 1)).unwrap();
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(&paths.journal)
        .unwrap()
        .write_all(b"{\"type\":\"event")
        .unwrap();

    let mut reopened = open(&paths, NOW);
    assert_eq!(reopened.recover(NOW).unwrap().notifications[0].id, "first");
    reopened.ingest(end("second", 2)).unwrap();
    drop(reopened);
    let recovered = open(&paths, NOW).recover(NOW).unwrap();
    assert_eq!(
        recovered
            .notifications
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn compaction_keeps_dedup_for_24_hours() {
    let (_dir, paths) = sandbox();
    let first = end("ended-0", 1);
    let mut store = open(&paths, NOW);
    for index in 0..=1000 {
        store
            .ingest(end(&format!("ended-{index}"), index + 1))
            .unwrap();
    }
    store.compact(NOW + DAY_MS - 1).unwrap();
    drop(store);

    let mut reopened = open(&paths, NOW + DAY_MS - 1);
    let view = reopened.recover(NOW + DAY_MS - 1).unwrap();
    assert_eq!(view.notifications.len(), 1000);
    assert_eq!(view.notifications[0].id, "ended-1");
    assert!(reopened.ingest(first.clone()).unwrap().duplicate);
    reopened.compact(NOW + DAY_MS + 1).unwrap();
    drop(reopened);
    assert!(
        !open(&paths, NOW + DAY_MS + 1)
            .ingest(first)
            .unwrap()
            .duplicate
    );
}

#[test]
fn dedup_uses_local_receipt_time_and_history_expires_on_recovery() {
    let (_dir, paths) = sandbox();
    let mut first = end("first", 1);
    first.received_at_ms = Some(u64::MAX);
    first.occurred_at_ms = 0;
    let mut store = open(&paths, NOW);
    store.ingest(first.clone()).unwrap();
    store.ingest(end("second", 2)).unwrap();
    assert_eq!(store.recover(NOW).unwrap().notifications[0].id, "second");
    store.compact(NOW + DAY_MS + 1).unwrap();
    drop(store);
    let much_later = NOW + 7 * DAY_MS + 1;
    let mut store = open(&paths, much_later);
    assert!(store.recover(much_later).unwrap().notifications.is_empty());
    assert!(!store.ingest(first).unwrap().duplicate);
}

#[test]
fn read_and_clear_survive_restart() {
    let (_dir, paths) = sandbox();
    let request = persistent("question", "old-turn");
    let mut store = open(&paths, NOW);
    store.ingest(request.clone()).unwrap();
    let read = store.mark_read(&request.session_key()).unwrap();
    assert!(read.notifications[0].read);
    assert_eq!(read.sessions[0].pending[0].id, "question");
    drop(store);

    let mut store = open(&paths, NOW);
    assert_eq!(store.recover(NOW).unwrap(), read);
    let cleared = store.clear_history().unwrap();
    assert!(cleared.notifications.is_empty());
    assert!(cleared.revision > read.revision);
    store.compact(NOW).unwrap();
    drop(store);

    let mut store = open(&paths, NOW);
    assert_eq!(store.recover(NOW).unwrap(), cleared);
    assert!(store.ingest(request).unwrap().duplicate);
    assert_eq!(cleared.sessions[0].pending[0].id, "question");
}

#[test]
fn checkpoint_retains_terminal_tombstones_and_deferred_requests() {
    let (_dir, paths) = sandbox();
    let mut store = open(&paths, NOW);
    store.ingest(end("old", 1)).unwrap();
    store
        .ingest(happened(
            "start",
            "current",
            Some(2),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    store
        .ingest(happened(
            "tombstone",
            "current",
            Some(3),
            json!({"type":"attention_resolved","requestId":"cancelled"}),
        ))
        .unwrap();
    store
        .ingest(happened(
            "deferred",
            "future",
            None,
            json!({
                "type":"attention_requested","requestId":"later","reason":"approval"
            }),
        ))
        .unwrap();
    store.clear_history().unwrap();
    store.compact(NOW).unwrap();
    drop(store);

    let mut store = open(&paths, NOW);
    store
        .ingest(happened(
            "late-start",
            "old",
            Some(5),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    store
        .ingest(happened(
            "late-request",
            "current",
            Some(4),
            json!({
                "type":"attention_requested","requestId":"cancelled","reason":"approval"
            }),
        ))
        .unwrap();
    let view = store.recover(NOW).unwrap();
    assert_eq!(view.sessions[0].turn_id.as_deref(), Some("current"));
    assert!(view.sessions[0].pending.is_empty());
    assert!(view.notifications.is_empty());

    store
        .ingest(happened(
            "future-start",
            "future",
            Some(6),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    let view = store.recover(NOW).unwrap();
    assert_eq!(view.sessions[0].turn_id.as_deref(), Some("future"));
    assert_eq!(view.sessions[0].pending[0].id, "later");
    assert_eq!(view.notifications[0].id, "deferred");
}

#[test]
fn recovery_boundary_replays_before_later_accepted_events() {
    let (_dir, paths) = sandbox();
    let mut store = open(&paths, NOW);
    let running = store
        .ingest(happened(
            "start",
            "t1",
            Some(1),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    drop(store);

    let mut store = open(&paths, NOW);
    let first_restart = store.recover(NOW).unwrap();
    assert_eq!(first_restart.sessions[0].status, SessionStatus::Unknown);
    assert!(first_restart.revision > running.revision);
    let ack = store
        .ingest(happened(
            "new-start",
            "t1",
            Some(2),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    assert!(ack.revision > first_restart.revision);
    drop(store);

    let recovered = open(&paths, NOW).recover(NOW).unwrap();
    assert_eq!(recovered.sessions[0].status, SessionStatus::Unknown);
    assert!(recovered.revision > ack.revision);
}

#[test]
fn checkpoint_does_not_restore_running_hidden_by_pending_requests() {
    for lifetime in ["turn", "until_resolved"] {
        check_cached_running_is_discarded(lifetime);
    }
}

fn check_cached_running_is_discarded(lifetime: &str) {
    let (_dir, paths) = sandbox();
    let mut store = open(&paths, NOW);
    store
        .ingest(happened(
            "start",
            "t1",
            Some(1),
            json!({"type":"turn_started"}),
        ))
        .unwrap();
    store
        .ingest(happened(
            "still-waiting",
            "t1",
            None,
            json!({ "type": "attention_requested", "requestId": "still-waiting",
                    "reason": "answer", "lifetime": lifetime }),
        ))
        .unwrap();
    store.compact(NOW).unwrap();
    drop(store);

    let mut store = open(&paths, NOW);
    let waiting = store.recover(NOW).unwrap();
    assert_eq!(waiting.sessions[0].status, SessionStatus::NeedsAttention);
    store
        .ingest(happened(
            "resolve",
            "t1",
            None,
            json!({
                "type":"attention_resolved","requestId":"still-waiting"
            }),
        ))
        .unwrap();
    assert_eq!(
        store.recover(NOW).unwrap().sessions[0].status,
        SessionStatus::Unknown,
        "等待请求类型：{lifetime}"
    );
}

#[test]
fn child_accepts_event_without_sending_response() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let paths = AppPaths::under(std::path::Path::new(&root));
    let mut store = open(&paths, NOW);
    assert!(
        !store
            .ingest(persistent("unanswered-response", "t1"))
            .unwrap()
            .duplicate
    );
    // 模拟持久化以后、响应发送以前退出，绕过 Rust 析构。
    std::process::exit(42);
}

#[test]
fn retry_after_process_exits_before_response_is_duplicate() {
    let (_dir, paths) = sandbox();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_accepts_event_without_sending_response"])
        .env(CHILD_ROOT, &paths.root)
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(42), "{:?}", child);
    let mut store = open(&paths, NOW);
    let ack = store
        .ingest(persistent("unanswered-response", "t1"))
        .unwrap();
    assert!(ack.duplicate);
    assert_eq!(
        store.recover(NOW).unwrap().sessions[0].pending[0].id,
        "unanswered-response"
    );
}

#[test]
fn rejects_another_writer_and_paths_outside_root() {
    let (_dir, paths) = sandbox();
    let store = open(&paths, NOW);
    assert!(JournalStore::open(paths.clone()).is_err());
    drop(store);
    let mut changed = paths.clone();
    changed.journal = paths.root.join("../escaped.jsonl");
    assert!(JournalStore::open(changed).is_err());
    assert!(JournalStore::open(paths).is_ok());
}

#[cfg(unix)]
#[test]
fn permission_denial_does_not_acknowledge_or_mutate_the_view() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, paths) = sandbox();
    let mut store = open(&paths, NOW);
    let ack = store.ingest(end("prior", 1)).unwrap();
    fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o400)).unwrap();
    if OpenOptions::new().write(true).open(&paths.journal).is_ok() {
        // 以 root 运行时仍可写入权限为 0400 的文件。
        fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();
        return;
    }
    assert!(store.ingest(end("denied", 2)).is_err());
    assert!(store.clear_history().is_err());
    assert_eq!(store.snapshot().revision, ack.revision);
    assert_eq!(store.snapshot().notifications[0].id, "prior");
    drop(store);
    assert!(JournalStore::open(paths.clone()).is_err());
    fs::set_permissions(&paths.journal, fs::Permissions::from_mode(0o600)).unwrap();
    let mut store = open(&paths, NOW);
    let view = store.recover(NOW).unwrap();
    assert_eq!(view.notifications[0].id, "prior");
    assert_eq!(view.notifications.len(), 1);
    assert!(!store.ingest(end("denied", 2)).unwrap().duplicate);
}

#[test]
fn a_complete_corrupt_line_is_never_treated_as_a_partial_tail() {
    let (_dir, paths) = sandbox();
    open(&paths, NOW).ingest(end("prior", 1)).unwrap();
    let original = fs::read(&paths.journal).unwrap();
    let mut damaged = original.clone();
    damaged.extend_from_slice(b"broken record\n");
    fs::write(&paths.journal, &damaged).unwrap();
    assert!(JournalStore::open(paths.clone()).is_err());
    assert_eq!(fs::read(&paths.journal).unwrap(), damaged);

    fs::write(&paths.journal, &original).unwrap();
    assert_eq!(
        open(&paths, NOW).recover(NOW).unwrap().notifications[0].id,
        "prior"
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let (_dir, paths) = sandbox();
    let outside = tempdir().unwrap();
    let destination = outside.path().join("log");
    fs::write(&destination, b"outside").unwrap();
    paths.prepare_root().unwrap();
    symlink(&destination, &paths.journal).unwrap();
    assert!(JournalStore::open(paths).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"outside");
}
