mod support;
use pet_core::*;
use serde_json::json;
use support::event;

fn apply(
    store: &mut SessionStore,
    id: &str,
    turn: &str,
    seq: Option<u64>,
    kind: serde_json::Value,
) {
    store.apply(&event(id, Some(turn), seq, kind));
}
fn until(id: &str) -> serde_json::Value {
    json!({"type":"attention_requested","requestId":id,"reason":"answer","lifetime":"until_resolved"})
}
fn start() -> serde_json::Value {
    json!({"type":"turn_started"})
}
fn end() -> serde_json::Value {
    json!({"type":"turn_ended","outcome":"completed"})
}
fn resolved(id: &str) -> serde_json::Value {
    json!({"type":"attention_resolved","requestId":id})
}

#[test]
fn timed_request_survives_end_new_turn_snapshot_and_resolves_by_birth_turn() {
    let mut store = SessionStore::default();
    apply(&mut store, "start1", "t1", Some(1), start());
    apply(&mut store, "q", "t1", None, until("q"));
    apply(&mut store, "end1", "t1", Some(3), end());
    assert_eq!(
        store.snapshot().sessions[0].status,
        SessionStatus::NeedsAttention
    );
    apply(&mut store, "start2", "t2", Some(4), start());
    let mut projection = store.snapshot().sessions[0].clone();
    projection.pending.clear();
    projection.status = SessionStatus::Running;
    let snapshot = event(
        "snapshot",
        Some("t2"),
        Some(5),
        json!({"type":"source_snapshot","sessions":[projection]}),
    );
    store.apply(&snapshot);
    assert_eq!(
        store.snapshot().sessions[0].pending[0].turn_id.as_deref(),
        Some("t1")
    );
    apply(&mut store, "late-start1", "t1", Some(1), start());
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
    apply(&mut store, "answer", "t1", Some(7), resolved("q"));
    let state = store.snapshot();
    assert!(state.sessions[0].pending.is_empty());
    assert_eq!(state.sessions[0].turn_id.as_deref(), Some("t2"));
    assert_eq!(state.sessions[0].status, SessionStatus::Running);
    assert!(
        state
            .notifications
            .iter()
            .any(|n| matches!(n.kind, NotificationKind::Attention { resolved: true, .. }))
    );
}
#[test]
fn resolved_and_terminal_before_until_request_cannot_resurrect_it() {
    let mut store = SessionStore::default();
    apply(&mut store, "s", "t1", Some(1), start());
    apply(&mut store, "end", "t1", Some(3), end());
    apply(&mut store, "late-q", "t1", None, until("q"));
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
    apply(&mut store, "answer", "t1", None, resolved("q"));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Ended);
    apply(&mut store, "duplicate", "t1", None, until("q"));
    assert!(store.snapshot().sessions[0].pending.is_empty());
    let mut other = SessionStore::default();
    apply(&mut other, "resolve-first", "t1", None, resolved("q"));
    apply(&mut other, "end", "t1", Some(3), end());
    apply(&mut other, "q-last", "t1", None, until("q"));
    assert!(other.snapshot().sessions[0].pending.is_empty());
}
#[test]
fn concurrent_lifetimes_keep_precise_tombstones_and_ordinary_turn_cleanup() {
    let mut store = SessionStore::default();
    apply(&mut store, "s", "t1", Some(1), start());
    apply(&mut store, "a", "t1", None, until("a"));
    apply(&mut store, "b", "t1", None, until("b"));
    apply(
        &mut store,
        "normal",
        "t1",
        Some(2),
        json!({"type":"attention_requested","requestId":"normal","reason":"approval"}),
    );
    apply(&mut store, "e", "t1", Some(3), end());
    assert_eq!(store.snapshot().sessions[0].pending.len(), 2);
    apply(&mut store, "s2", "t2", Some(4), start());
    apply(&mut store, "wrong-turn", "t2", None, resolved("a"));
    assert_eq!(store.snapshot().sessions[0].pending.len(), 2);
    apply(&mut store, "a-resolved", "t1", None, resolved("a"));
    assert_eq!(store.snapshot().sessions[0].pending[0].id, "b");
}
#[test]
fn unsequenced_future_turn_request_before_start_is_deferred_then_consumed() {
    let mut store = SessionStore::default();
    apply(&mut store, "s1", "t1", Some(1), start());
    apply(
        &mut store,
        "q2",
        "t2",
        None,
        json!({"type":"attention_requested","requestId":"q2","reason":"answer"}),
    );
    assert!(store.snapshot().sessions[0].pending.is_empty());
    apply(&mut store, "s2", "t2", Some(5), start());
    assert_eq!(store.snapshot().sessions[0].pending[0].id, "q2");
    apply(&mut store, "resolved-before", "t3", None, resolved("q3"));
    apply(
        &mut store,
        "q3",
        "t3",
        None,
        json!({"type":"attention_requested","requestId":"q3","reason":"answer"}),
    );
    apply(&mut store, "s3", "t3", Some(9), start());
    assert!(store.snapshot().sessions[0].pending.is_empty());
}
#[test]
fn until_request_before_any_start_does_not_prevent_later_native_turn_anchor() {
    let mut store = SessionStore::default();
    apply(&mut store, "q-old", "t1", None, until("q"));
    apply(&mut store, "s2", "t2", Some(6), start());
    assert_eq!(store.snapshot().sessions[0].turn_id.as_deref(), Some("t2"));
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
}
#[test]
fn lifetime_json_defaults_keep_legacy_bytes_and_accept_cross_turn_snapshot_pending() {
    let legacy = event(
        "legacy",
        Some("t1"),
        Some(1),
        json!({"type":"attention_requested","requestId":"q","reason":"answer"}),
    );
    assert!(!serde_json::to_string(&legacy).unwrap().contains("lifetime"));
    let mut store = SessionStore::default();
    apply(&mut store, "s", "t1", Some(1), start());
    apply(&mut store, "q", "t1", None, until("q"));
    apply(&mut store, "s2", "t2", Some(4), start());
    let snapshot = event(
        "snap",
        Some("t2"),
        Some(5),
        json!({"type":"source_snapshot","sessions":store.snapshot().sessions}),
    );
    assert!(decode_event(&serde_json::to_vec(&snapshot).unwrap()).is_ok());
    let mut invalid = serde_json::to_value(snapshot).unwrap();
    invalid["kind"]["sessions"][0]["pending"][0]["lifetime"] = json!("turn");
    assert!(decode_event(&serde_json::to_vec(&invalid).unwrap()).is_err());
}

#[test]
fn v1_never_admits_until_resolved_semantics_and_v2_accepts_them() {
    let request = event("v2", Some("t1"), Some(1), until("q"));
    let mut value = serde_json::to_value(request).unwrap();
    value["schemaVersion"] = json!(1);
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_err());
    value["schemaVersion"] = json!(2);
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_ok());
    let pending = json!({"key":{"source":"codex","installationId":"local","sessionId":"s"},"turnId":"t2","status":"needs_attention","pending":[{"id":"q","turnId":"t1","reason":"answer","lifetime":"until_resolved"}],"unread":true,"lastEventAtMs":100});
    value["kind"] = json!({"type":"source_snapshot","sessions":[pending]});
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["schemaVersion"] = json!(1);
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn attention_priority_does_not_invent_activity_after_disconnect_or_unordered_turn() {
    for disconnect in [true, false] {
        let mut store = SessionStore::default();
        apply(&mut store, "s1", "t1", Some(1), start());
        apply(&mut store, "q", "t1", None, until("q"));
        if disconnect {
            apply(
                &mut store,
                "disconnect",
                "t1",
                None,
                json!({"type":"source_disconnected"}),
            );
            apply(&mut store, "ambiguous-start", "t1", None, start());
        } else {
            apply(
                &mut store,
                "q2-unknown",
                "t2",
                None,
                json!({"type":"attention_requested","requestId":"q2","reason":"answer"}),
            );
        }
        assert_eq!(
            store.snapshot().sessions[0].status,
            SessionStatus::NeedsAttention
        );
        apply(&mut store, "answer", "t1", None, resolved("q"));
        assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
    }
}

#[test]
fn restore_keeps_attention_but_not_an_unproved_running_fact() {
    let start = event("start", Some("t1"), Some(1), start());
    let request = event("request", Some("t1"), None, until("q"));
    let mut store = SessionStore::restore(&[start, request], 100);
    assert_eq!(
        store.snapshot().sessions[0].status,
        SessionStatus::NeedsAttention
    );
    apply(&mut store, "answer", "t1", None, resolved("q"));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}

#[test]
fn restore_preserves_proven_terminal_under_persistent_attention() {
    let events = [
        event("s", Some("t1"), Some(1), start()),
        event("q", Some("t1"), None, until("q")),
        event("e", Some("t1"), Some(3), end()),
    ];
    let mut store = SessionStore::restore(&events, 100);
    assert_eq!(
        store.snapshot().sessions[0].status,
        SessionStatus::NeedsAttention
    );
    apply(&mut store, "answer", "t1", None, resolved("q"));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Ended);
}
#[test]
fn tombstoned_ordinary_request_does_not_reestablish_running_under_until_overlay() {
    let mut store = SessionStore::default();
    apply(&mut store, "s", "t1", Some(1), start());
    apply(&mut store, "r", "t1", Some(2), resolved("normal"));
    apply(
        &mut store,
        "d",
        "t1",
        None,
        json!({"type":"source_disconnected"}),
    );
    apply(
        &mut store,
        "late-normal",
        "t1",
        Some(1),
        json!({"type":"attention_requested","requestId":"normal","reason":"answer"}),
    );
    apply(&mut store, "q", "t1", None, until("q"));
    apply(&mut store, "a", "t1", None, resolved("q"));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}
