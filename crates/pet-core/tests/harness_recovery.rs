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

#[test]
fn old_direct_audit_resolves_only_the_original_owner_request_after_handoff() {
    let direct = |epoch, event_id, owner, kind| {
        let mut raw = value(epoch, event_id, None, None, kind);
        raw["directQuestionOwner"] = json!(owner);
        event(raw)
    };
    let asked = |id| json!({"type":"attention_requested","requestId":id,"reason":"answer","lifetime":"until_resolved"});
    let resolved = |id| json!({"type":"attention_resolved","requestId":id});
    let owner = "a1234567-1234-4234-8234-123456789abc";
    let another_owner = "b1234567-1234-4234-8234-123456789abc";
    let mut store = SessionStore::default();
    assert!(
        store
            .apply(&direct(OLD, "original", owner, asked("old-direct")))
            .should_notify
    );
    assert!(
        store
            .apply(&direct(OLD, "reused-old", owner, asked("reused")))
            .should_notify
    );
    store.apply(&recovered(NEW, 8, Some("2"), None));
    store.apply(&direct(NEW, "reused-new", owner, asked("reused")));
    assert!(
        !store
            .apply(&direct(
                OLD,
                "wrong-owner",
                another_owner,
                resolved("old-direct")
            ))
            .changed
    );
    assert!(
        !store
            .apply(&direct(
                OLD,
                "reused-old-resolved",
                owner,
                resolved("reused")
            ))
            .changed
    );
    let matched = store.apply(&direct(
        OLD,
        "original-resolved",
        owner,
        resolved("old-direct"),
    ));
    assert!(matched.changed);
    assert!(!matched.should_notify);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions[0].turn_id.as_deref(), Some("2"));
    assert_eq!(snapshot.sessions[0].pending.len(), 1);
    assert_eq!(snapshot.sessions[0].pending[0].id, "reused");
    assert!(snapshot.notifications.iter().any(|note| matches!(&note.kind,
        pet_core::NotificationKind::Attention { request_id, resolved: true, .. } if request_id == "old-direct")));
}

#[test]
fn an_idle_snapshot_and_a_same_writer_end_agree_in_both_orders() {
    let idle = recovered(NEW, 21, None, None);
    let end = event(value(
        NEW,
        "ended",
        Some("2"),
        Some(20),
        json!({
            "type":"turn_ended","outcome":"completed",
        }),
    ));
    let duplicate_fact = event(value(
        NEW,
        "ended-again",
        Some("2"),
        Some(20),
        json!({
            "type":"turn_ended","outcome":"completed",
        }),
    ));
    for order in [[&end, &idle], [&idle, &end]] {
        let mut store = SessionStore::default();
        for event in order {
            store.apply(event);
        }
        assert!(!store.apply(&duplicate_fact).should_notify);
        let current = store.snapshot();
        assert_eq!(current.sessions[0].turn_id.as_deref(), Some("2"));
        assert_eq!(current.sessions[0].status, SessionStatus::Ended);
        assert_eq!(current.notifications.len(), 1);
    }
}

#[test]
#[ignore = "同一真实 Host 问题服务的跨代测试提供观察事件"]
fn actual_direct_handoff_keeps_the_owner_match_after_compaction() {
    let path = std::env::var("PET_HARNESS_DIRECT_HANDOFF").expect("需要真实宿主直接提问事件");
    let values: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let events: Vec<_> = values.into_iter().map(event).collect();
    let directory = tempfile::tempdir().unwrap();
    let paths = pet_core::AppPaths::under(&directory.path().canonicalize().unwrap().join("app"));
    let now = events
        .iter()
        .map(|event| event.occurred_at_ms)
        .max()
        .unwrap();
    {
        let mut journal =
            pet_core::JournalStore::open_with_clock(paths.clone(), move || now).unwrap();
        for index in [0, 1, 2, 4] {
            assert!(!journal.ingest(events[index].clone()).unwrap().duplicate);
        }
        assert_eq!(journal.snapshot().sessions[0].pending.len(), 2);
        journal.compact(now).unwrap();
    }
    let mut journal = pet_core::JournalStore::open_with_clock(paths, move || now).unwrap();
    assert!(!journal.ingest(events[3].clone()).unwrap().duplicate);
    assert!(journal.ingest(events[3].clone()).unwrap().duplicate);
    let snapshot = journal.snapshot();
    assert_eq!(snapshot.sessions[0].pending.len(), 1);
    let pet_core::EventKind::AttentionRequested { request_id, .. } = &events[4].kind else {
        panic!("真实样本需要新代提问");
    };
    assert_eq!(&snapshot.sessions[0].pending[0].id, request_id);
    assert_eq!(snapshot.notifications.len(), 2);
}

#[test]
fn unproved_writer_gaps_and_salvaged_tail_do_not_settle_known_old_requests() {
    let mut store = SessionStore::default();
    let mut asked = value(
        OLD,
        "original",
        Some("1"),
        None,
        json!({
            "type":"attention_requested", "requestId":"original", "reason":"answer", "lifetime":"until_resolved",
        }),
    );
    asked["questionBirth"] = json!(4);
    assert!(store.apply(&event(asked)).should_notify);
    let mut handoff = value(
        NEW,
        "new-recovery",
        None,
        Some(8),
        json!({
            "type":"source_recovery","snapshotEventCount":9,"running":false,"timed":[],
        }),
    );
    handoff["observation"]["version"] = json!(2);
    handoff["observation"]["recoveredEventCount"] = json!(6);
    handoff["observation"]["firstLiveSequence"] = json!(8);
    store.apply(&event(handoff.clone()));
    let current = store.snapshot();
    assert_eq!(current.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(current.sessions[0].pending[0].id, "original");

    handoff["kind"]["timed"] = json!([{ "requestId":"forged", "turnId":"1", "birthSequence":7 }]);
    assert!(decode_event(&serde_json::to_vec(&handoff).unwrap()).is_err());
    handoff["kind"] = json!({"type":"turn_started"});
    handoff["turnId"] = json!("3");
    handoff["sequence"] = json!(7);
    assert!(decode_event(&serde_json::to_vec(&handoff).unwrap()).is_err());
}
