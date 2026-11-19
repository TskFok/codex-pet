use pet_core::{EventEnvelope, SessionStatus, SessionStore, decode_event};
use serde_json::{Value, json};

const STREAM: &str = "11111111-1111-4111-8111-111111111111";
const OLD: &str = "22222222-2222-4222-8222-222222222222";
const NEW: &str = "33333333-3333-4333-8333-333333333333";

fn value(epoch: &str, id: &str, turn: Option<&str>, sequence: Option<u64>, kind: Value) -> Value {
    let (generation, ancestors) = if epoch == OLD {
        (1, vec![])
    } else {
        (
            2,
            vec![json!({"epochId": OLD, "generation": 1, "firstSequence": 3})],
        )
    };
    json!({
        "schemaVersion": 3,
        "eventId": format!("{epoch}:{id}"),
        "identity": { "source": "deepseek", "installationId": "installation", "instanceId": epoch },
        "sessionId": "s",
        "turnId": turn,
        "sequence": sequence,
        "occurredAtMs": 100,
        "observation": { "version": 1, "epochId": epoch, "streamId": STREAM,
            "generation": generation, "recoveredEventCount": 3, "ancestors": ancestors },
        "kind": kind,
    })
}

fn event(value: Value) -> EventEnvelope {
    decode_event(&serde_json::to_vec(&value).unwrap()).expect("恢复协议有效")
}

fn recovered(
    epoch: &str,
    count: u64,
    turn: Option<&str>,
    timed: Option<Vec<Value>>,
) -> EventEnvelope {
    event(value(
        epoch,
        "snapshot",
        turn,
        count.checked_sub(1),
        json!({
            "type": "source_recovery", "snapshotEventCount": count, "running": turn.is_some(), "timed": timed,
        }),
    ))
}

#[test]
fn later_writer_reuses_turn_and_positions_without_reusing_old_terminal_facts() {
    let mut store = SessionStore::default();
    let old_end = event(value(
        OLD,
        "end",
        Some("2"),
        Some(4),
        json!({"type": "turn_ended", "outcome": "completed"}),
    ));
    assert!(store.apply(&old_end).should_notify);
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Ended);
    let new_start = event(value(
        NEW,
        "start",
        Some("2"),
        Some(3),
        json!({"type": "turn_started"}),
    ));
    store.apply(&new_start);
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
    store.apply(&recovered(NEW, 3, None, Some(vec![])));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
    let late_old = event(value(
        OLD,
        "old-delayed",
        Some("2"),
        Some(6),
        json!({"type": "turn_ended", "outcome": "failed"}),
    ));
    assert!(!store.apply(&late_old).should_notify);
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
    let new_end = event(value(
        NEW,
        "end",
        Some("2"),
        Some(4),
        json!({"type": "turn_ended", "outcome": "interrupted"}),
    ));
    assert!(store.apply(&new_end).should_notify);
    assert!(!store.apply(&new_end).should_notify);
    assert_eq!(
        store.snapshot().sessions[0].status,
        SessionStatus::Interrupted
    );
    assert_eq!(store.snapshot().notifications.len(), 2);
}

#[test]
fn retained_timed_birth_survives_epoch_change_and_late_old_resolution_cannot_remove_new_birth() {
    let mut store = SessionStore::default();
    let request = |epoch, id, birth| {
        let mut json = value(
            epoch,
            id,
            Some("2"),
            None,
            json!({
                "type": "attention_requested", "requestId": id, "reason": "answer", "lifetime": "until_resolved",
            }),
        );
        json["questionBirth"] = json!(birth);
        event(json)
    };
    store.apply(&request(OLD, "old-tail", 3));
    store.apply(&request(OLD, "retained", 2));
    store.apply(&recovered(
        NEW,
        4,
        None,
        Some(vec![
            json!({"requestId": "retained", "turnId": "2", "birthSequence": 2}),
        ]),
    ));
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions[0].pending.len(), 1);
    assert_eq!(snapshot.sessions[0].pending[0].id, "retained");
    assert_eq!(snapshot.notifications.len(), 2);
    store.apply(&request(NEW, "fresh", 4));
    let old_resolution = event(value(
        OLD,
        "resolution",
        Some("2"),
        Some(5),
        json!({
            "type": "attention_resolved", "requestId": "fresh",
        }),
    ));
    store.apply(&old_resolution);
    assert_eq!(store.snapshot().sessions[0].pending.len(), 2);
    let resolution = event(value(
        NEW,
        "resolution",
        Some("2"),
        Some(5),
        json!({
            "type": "attention_resolved", "requestId": "retained",
        }),
    ));
    store.apply(&resolution);
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
    assert_eq!(store.snapshot().sessions[0].pending[0].id, "fresh");
}

#[test]
#[ignore = "由真实 JSONL 进程重启专项提供事件"]
fn actual_jsonl_adapter_bridge_and_checkpoint_keep_generations_distinct() {
    let path = std::env::var("PET_HARNESS_RECOVERY_EVENTS").expect("需要真实桥接样本");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let old: Vec<_> = fixture["old"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(event)
        .collect();
    let current: Vec<_> = fixture["current"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(event)
        .collect();
    let new_start = current
        .iter()
        .find(|e| matches!(e.kind, pet_core::EventKind::TurnStarted))
        .unwrap();
    let new_snapshot = current
        .iter()
        .find(|e| matches!(e.kind, pet_core::EventKind::SourceRecovery { .. }))
        .unwrap();
    let answer = current
        .iter()
        .find(|e| matches!(e.kind, pet_core::EventKind::AttentionResolved { .. }))
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let paths = pet_core::AppPaths::under(&directory.path().canonicalize().unwrap().join("app"));
    let now = current
        .iter()
        .chain(&old)
        .map(|e| e.occurred_at_ms)
        .max()
        .unwrap();
    {
        let mut journal =
            pet_core::JournalStore::open_with_clock(paths.clone(), move || now).unwrap();
        journal.ingest(old[0].clone()).unwrap();
        journal.ingest(old[2].clone()).unwrap();
        journal.ingest(new_start.clone()).unwrap();
        journal.ingest(new_snapshot.clone()).unwrap();
        assert_eq!(
            journal.snapshot().sessions[0].pending[0].id,
            "question:retained"
        );
        assert_eq!(journal.snapshot().sessions[0].turn_id.as_deref(), Some("2"));
        assert_eq!(journal.snapshot().notifications.len(), 1);
        journal.compact(now).unwrap();
    }
    let mut journal = pet_core::JournalStore::open_with_clock(paths, move || now).unwrap();
    journal.recover(now).unwrap();
    assert!(!journal.ingest(old[1].clone()).unwrap().duplicate);
    let mut legacy = serde_json::to_value(&old[0]).unwrap();
    legacy["schemaVersion"] = json!(2);
    legacy["eventId"] = json!("old-v2-ordinary");
    legacy["kind"] = json!({"type":"turn_ended","outcome":"failed"});
    legacy["turnId"] = json!("2");
    legacy.as_object_mut().unwrap().remove("observation");
    assert!(!journal.ingest(event(legacy)).unwrap().duplicate);
    assert_eq!(journal.snapshot().notifications.len(), 1);
    assert!(journal.ingest(new_snapshot.clone()).unwrap().duplicate);
    assert!(!journal.ingest(answer.clone()).unwrap().duplicate);
    assert!(journal.snapshot().sessions[0].pending.is_empty());
    assert_eq!(journal.snapshot().notifications.len(), 1);
}

#[test]
fn recovered_timed_request_can_wait_again_after_its_unflushed_resolution_rolls_back() {
    let mut store = SessionStore::default();
    let mut requested = value(
        OLD,
        "request",
        Some("1"),
        None,
        json!({
            "type": "attention_requested", "requestId": "retained", "reason": "answer", "lifetime": "until_resolved",
        }),
    );
    requested["questionBirth"] = json!(2);
    assert!(store.apply(&event(requested)).should_notify);
    let mut resolution = value(
        OLD,
        "provisional-resolution",
        Some("1"),
        Some(4),
        json!({
            "type": "attention_resolved", "requestId": "retained",
        }),
    );
    resolution["questionBirth"] = json!(2);
    store.apply(&event(resolution));
    assert!(store.snapshot().sessions[0].pending.is_empty());
    store.apply(&recovered(
        NEW,
        3,
        None,
        Some(vec![
            json!({"requestId": "retained", "turnId": "1", "birthSequence": 2}),
        ]),
    ));
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions[0].status, SessionStatus::NeedsAttention);
    assert_eq!(snapshot.sessions[0].pending[0].id, "retained");
    assert_eq!(snapshot.notifications.len(), 1);
    assert!(matches!(
        snapshot.notifications[0].kind,
        pet_core::NotificationKind::Attention { resolved: true, .. }
    ));
}

#[test]
fn a_timed_snapshot_only_covers_its_own_domain() {
    let mut store = SessionStore::default();
    store.apply(&recovered(NEW, 8, Some("2"), Some(vec![])));
    let approval = event(value(
        NEW,
        "approval",
        Some("2"),
        Some(5),
        json!({
            "type":"attention_requested","requestId":"approval-1","reason":"approval",
        }),
    ));
    assert!(store.apply(&approval).should_notify);
    let direct = event(value(
        NEW,
        "direct",
        None,
        None,
        json!({
            "type":"attention_requested","requestId":"direct-1","reason":"answer","lifetime":"until_resolved",
        }),
    ));
    assert!(store.apply(&direct).should_notify);
    assert_eq!(store.snapshot().sessions[0].pending.len(), 2);
}

#[test]
fn incomplete_request_coverage_preserves_unconfirmed_previous_requests() {
    let mut store = SessionStore::default();
    let request = event(value(
        OLD,
        "direct",
        None,
        None,
        json!({
            "type": "attention_requested","requestId":"old-direct","reason":"answer","lifetime":"until_resolved",
        }),
    ));
    assert!(store.apply(&request).should_notify);
    let partial = recovered(NEW, 3, None, None);
    store.apply(&partial);
    let latest = store.snapshot();
    assert_eq!(latest.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(latest.sessions[0].pending[0].id, "old-direct");
    assert_eq!(latest.notifications.len(), 1);
    let replayed = SessionStore::restore(&[request, partial], 100).snapshot();
    assert_eq!(replayed.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(replayed.sessions[0].pending, latest.sessions[0].pending);
}

#[test]
fn audit_only_v3_does_not_expose_an_empty_session() {
    let mut store = SessionStore::default();
    store.apply(&event(value(
        NEW,
        "audit",
        None,
        None,
        json!({
            "type":"attention_resolved", "requestId":"settled",
        }),
    )));
    assert!(store.snapshot().sessions.is_empty());
    let pending = event(value(
        NEW,
        "delayed",
        None,
        None,
        json!({
            "type":"attention_requested", "requestId":"settled", "reason":"answer", "lifetime":"until_resolved",
        }),
    ));
    assert!(!store.apply(&pending).should_notify);
    assert!(store.snapshot().sessions.is_empty());
    store.apply(&event(value(
        NEW,
        "turn",
        Some("3"),
        Some(3),
        json!({"type":"turn_started"}),
    )));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
}
