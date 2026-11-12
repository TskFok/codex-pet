mod support;
use pet_core::*;
use serde_json::json;
use support::{event, load_case};

fn started(id: &str, turn: Option<&str>, seq: Option<u64>) -> EventEnvelope {
    event(id, turn, seq, json!({"type":"turn_started"}))
}
fn requested(id: &str, turn: Option<&str>, seq: Option<u64>, request: &str) -> EventEnvelope {
    event(
        id,
        turn,
        seq,
        json!({"type":"attention_requested","requestId":request,"reason":"answer"}),
    )
}
fn ended(id: &str, turn: Option<&str>, seq: Option<u64>, outcome: &str) -> EventEnvelope {
    event(
        id,
        turn,
        seq,
        json!({"type":"turn_ended","outcome":outcome}),
    )
}
fn key() -> SessionKey {
    started("key", None, None).session_key()
}

#[test]
fn state_replay_cases() {
    for name in [
        "source-isolation",
        "duplicate",
        "resolved-before-requested",
        "late-old-turn",
        "late-start-after-end",
        "child-folding",
        "end-is-unknown",
        "session-sequence-isolation",
    ] {
        let case = load_case(name);
        let mut store = SessionStore::default();
        for event in &case.events {
            store.apply(event);
        }
        assert_eq!(store.snapshot(), case.expected, "{name}");
        for event in &case.events {
            assert!(!store.apply(event).should_notify);
        }
        assert_eq!(store.snapshot(), case.expected, "replay {name}");
    }
}

#[test]
fn new_turn_preserves_unread_history_until_explicitly_read() {
    let mut store = SessionStore::default();
    assert!(
        store
            .apply(&ended("a", Some("t1"), Some(1), "completed"))
            .should_notify
    );
    store.apply(&started("b", Some("t2"), Some(2)));
    let snap = store.snapshot();
    assert_eq!(snap.sessions[0].status, SessionStatus::Running);
    assert!(snap.sessions[0].unread);
    assert_eq!(snap.notifications.len(), 1);
    assert!(store.mark_read(&key()).changed);
    assert!(!store.snapshot().sessions[0].unread);
    assert!(store.snapshot().notifications[0].read);
    assert!(!store.mark_read(&key()).changed);
    assert!(
        store
            .apply(&ended("c", Some("t2"), Some(3), "unknown"))
            .should_notify
    );
    assert!(store.snapshot().sessions[0].unread);
}

#[test]
fn reading_attention_does_not_resolve_it_or_hide_it_from_active() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("t1"), Some(1), "q"));
    store.mark_read(&key());
    let snap = store.snapshot();
    assert!(!snap.sessions[0].unread);
    assert_eq!(snap.sessions[0].status, SessionStatus::NeedsAttention);
    assert_eq!(snap.active, Some(key()));
}

#[test]
fn resolved_request_does_not_clear_other_requests() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("t1"), Some(1), "a"));
    store.apply(&requested("b", Some("t1"), Some(2), "b"));
    let resolution = event(
        "c",
        Some("t1"),
        Some(3),
        json!({"type":"attention_resolved","requestId":"a"}),
    );
    assert!(!store.apply(&resolution).should_notify);
    let snap = store.snapshot();
    assert_eq!(snap.sessions[0].pending.len(), 1);
    assert_eq!(snap.sessions[0].pending[0].id, "b");
    assert!(matches!(
        snap.notifications[0].kind,
        NotificationKind::Attention { resolved: true, .. }
    ));
    assert!(snap.notifications[0].read);
}

#[test]
fn same_request_id_in_new_turn_is_not_suppressed_by_old_tombstone() {
    let mut store = SessionStore::default();
    store.apply(&event(
        "a",
        Some("t1"),
        Some(2),
        json!({"type":"attention_resolved","requestId":"q"}),
    ));
    assert!(
        store
            .apply(&requested("b", Some("t2"), Some(3), "q"))
            .should_notify
    );
    store.apply(&event(
        "c",
        Some("t1"),
        Some(4),
        json!({"type":"attention_resolved","requestId":"q"}),
    ));
    assert_eq!(
        store.snapshot().sessions[0].pending[0].turn_id.as_deref(),
        Some("t2")
    );
}

#[test]
fn different_request_ids_can_arrive_out_of_sequence() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("t1"), Some(5), "a"));
    store.apply(&requested("b", Some("t1"), Some(3), "b"));
    assert_eq!(store.snapshot().sessions[0].pending.len(), 2);
}

#[test]
fn turn_end_resolves_only_current_turn_and_never_reopens() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("t1"), Some(1), "q"));
    store.apply(&ended("b", Some("t1"), Some(3), "interrupted"));
    assert!(
        !store
            .apply(&requested("c", Some("t1"), Some(2), "late"))
            .should_notify
    );
    store.apply(&started("d", Some("t1"), Some(4)));
    let snap = store.snapshot();
    assert_eq!(snap.sessions[0].status, SessionStatus::Interrupted);
    assert!(snap.sessions[0].pending.is_empty());
    assert_eq!(snap.notifications.len(), 2);
}

#[test]
fn missing_turn_order_preserves_requests_and_reports_unknown() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("new"), None, "q"));
    let mut uncertain = ended("b", Some("other"), None, "completed");
    uncertain.occurred_at_ms = 999_999;
    assert!(store.apply(&uncertain).should_notify);
    let snap = store.snapshot();
    assert_eq!(snap.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(snap.sessions[0].turn_id.as_deref(), Some("new"));
    assert_eq!(snap.sessions[0].pending[0].id, "q");
    assert_eq!(snap.notifications.len(), 2);
    assert!(!snap.notifications[0].read);
    assert!(matches!(
        snap.notifications[0].kind,
        NotificationKind::Attention {
            resolved: false,
            ..
        }
    ));
    assert_eq!(snap.notifications[1].turn_id.as_deref(), Some("other"));
    assert_eq!(snap.active, Some(key()));
}

#[test]
fn missing_turn_and_sequence_cannot_reopen_a_finished_turn() {
    let mut store = SessionStore::default();
    store.apply(&ended("a", None, None, "completed"));
    store.apply(&started("b", None, None));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}

#[test]
fn sequence_is_not_compared_across_source_instances() {
    let mut store = SessionStore::default();
    store.apply(&requested("a", Some("t1"), Some(100), "q"));
    let mut restarted = started("b", Some("t2"), Some(101));
    restarted.identity.instance_id = "process-2".into();
    store.apply(&restarted);
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
}

#[test]
fn active_prioritizes_attention_failure_unread_end_and_running() {
    let mut store = SessionStore::default();
    let mut running = started("run", Some("t"), Some(1));
    running.session_id = "running".into();
    let mut finish = ended("end", Some("t"), Some(1), "completed");
    finish.session_id = "ended".into();
    let mut fail = ended("fail", Some("t"), Some(1), "failed");
    fail.session_id = "failed".into();
    let mut need = requested("need", Some("t"), Some(1), "q");
    need.session_id = "attention".into();
    for e in [&running, &finish, &fail, &need] {
        store.apply(e);
    }
    assert_eq!(store.snapshot().active, Some(need.session_key()));
    store.apply(
        &event(
            "resolve",
            Some("t"),
            Some(2),
            json!({"type":"attention_resolved","requestId":"q"}),
        )
        .with_session("attention"),
    );
    assert_eq!(store.snapshot().active, Some(fail.session_key()));
    store.mark_read(&fail.session_key());
    assert_eq!(store.snapshot().active, Some(fail.session_key()));
}

#[test]
fn attention_child_remains_active_when_parent_is_missing() {
    let mut child = requested("child", Some("t"), Some(1), "q");
    child.parent_session_id = Some("missing".into());
    let mut store = SessionStore::default();
    store.apply(&child);
    assert_eq!(store.snapshot().active, Some(child.session_key()));
    assert_eq!(store.snapshot().sessions.len(), 1);
}

#[test]
fn disconnection_is_scoped_to_installation_and_instance_and_keeps_requests() {
    let mut store = SessionStore::default();
    let mut a = requested("a", Some("t"), Some(1), "q");
    a.session_id = "a".into();
    let mut b = started("b", Some("t"), Some(1));
    b.session_id = "b".into();
    b.identity.instance_id = "other".into();
    for e in [&a, &b] {
        store.apply(e);
    }
    let disconnect = event("d", None, None, json!({"type":"source_disconnected"}));
    store.apply(&disconnect);
    let snap = store.snapshot();
    assert_eq!(snap.sessions.len(), 2);
    assert_eq!(snap.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(snap.sessions[0].pending.len(), 1);
    assert_eq!(snap.sessions[1].status, SessionStatus::Running);
}

#[test]
fn restore_downgrades_running_preserves_unread_and_suppresses_retries() {
    let a = started("a", Some("t"), Some(1));
    let mut b = ended("b", Some("t"), Some(1), "unknown");
    b.session_id = "finished".into();
    let mut store = SessionStore::restore(&[a.clone(), b], 1000);
    assert_eq!(
        store
            .snapshot()
            .sessions
            .iter()
            .find(|s| s.key.session_id == "s")
            .unwrap()
            .status,
        SessionStatus::Unknown
    );
    assert!(!store.snapshot().notifications[0].read);
    assert!(!store.apply(&a).changed);
}

#[test]
fn clear_history_keeps_pending_and_deduplication() {
    let a = requested("a", Some("t"), Some(1), "q");
    let mut store = SessionStore::default();
    store.apply(&a);
    assert!(store.clear_history().changed);
    assert!(store.snapshot().notifications.is_empty());
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
    assert!(!store.snapshot().sessions[0].unread);
    assert!(!store.apply(&a).should_notify);
    assert!(!store.clear_history().changed);
}

#[test]
fn history_is_bounded_without_forgetting_deduplication() {
    let mut store = SessionStore::default();
    for index in 0..1005 {
        store.apply(&ended(
            &format!("e{index}"),
            Some(&format!("t{index}")),
            Some(index),
            "completed",
        ));
    }
    let snap = store.snapshot();
    assert_eq!(snap.notifications.len(), 1000);
    assert_eq!(snap.notifications[0].id, "e5");
    assert!(
        !store
            .apply(&ended("e0", Some("t0"), Some(0), "completed"))
            .should_notify
    );
}

#[test]
fn restore_prunes_seven_day_history_without_forgetting_ids_or_pending() {
    let old = ended("old", Some("t1"), Some(1), "unknown");
    let pending = requested("pending", Some("t2"), Some(2), "q");
    let mut store = SessionStore::restore(&[old.clone(), pending], 8 * 24 * 60 * 60 * 1000);
    assert!(store.snapshot().notifications.is_empty());
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
    assert!(!store.apply(&old).should_notify);
}

#[test]
fn source_snapshot_cannot_override_newer_other_session_without_watermark() {
    let mut store = SessionStore::default();
    let a = requested("a", Some("new"), Some(100), "q");
    store.apply(&a);
    let stale = SessionProjection {
        key: key(),
        parent_session_id: None,
        turn_id: Some("old".into()),
        status: SessionStatus::Ended,
        pending: vec![],
        project: None,
        summary: None,
        unread: false,
        last_event_at_ms: 999,
    };
    let mut snapshot = event(
        "snap",
        None,
        Some(200),
        json!({"type":"source_snapshot","sessions":[stale]}),
    );
    snapshot.session_id = "another".into();
    assert!(!store.apply(&snapshot).should_notify);
    let snap = store.snapshot();
    assert_eq!(snap.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(snap.sessions[0].pending.len(), 1);
    assert_eq!(snap.sessions[0].turn_id.as_deref(), Some("new"));
}

#[test]
fn fresh_same_session_snapshot_corrects_cached_state_without_notifying() {
    let a = started("a", Some("t"), Some(1));
    let mut store = SessionStore::restore(&[a], 1000);
    let projection = SessionProjection {
        key: key(),
        parent_session_id: None,
        turn_id: Some("t".into()),
        status: SessionStatus::Running,
        pending: vec![],
        project: None,
        summary: None,
        unread: false,
        last_event_at_ms: 100,
    };
    let snapshot = event(
        "snap",
        Some("t"),
        Some(2),
        json!({"type":"source_snapshot","sessions":[projection]}),
    );
    assert!(!store.apply(&snapshot).should_notify);
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
}

trait EventExt {
    fn with_session(self, session: &str) -> Self;
}
impl EventExt for EventEnvelope {
    fn with_session(mut self, session: &str) -> Self {
        self.session_id = session.into();
        self
    }
}

#[test]
fn resolved_tombstone_also_prevents_snapshot_resurrection() {
    let mut store = SessionStore::default();
    store.apply(&requested("ask", Some("t"), Some(1), "q"));
    let mut stale = store.snapshot().sessions[0].clone();
    store.apply(&event(
        "resolve",
        Some("t"),
        Some(2),
        json!({"type":"attention_resolved","requestId":"q"}),
    ));
    stale.last_event_at_ms = 200;
    store.apply(&event(
        "snap",
        Some("t"),
        Some(3),
        json!({"type":"source_snapshot","sessions":[stale]}),
    ));
    assert!(store.snapshot().sessions[0].pending.is_empty());
    assert_ne!(
        store.snapshot().sessions[0].status,
        SessionStatus::NeedsAttention
    );
}

#[test]
fn imported_unread_end_is_active_without_synthetic_notification() {
    let projection = SessionProjection {
        key: key(),
        parent_session_id: None,
        turn_id: Some("t".into()),
        status: SessionStatus::Ended,
        pending: vec![],
        project: None,
        summary: None,
        unread: true,
        last_event_at_ms: 100,
    };
    let mut store = SessionStore::default();
    store.apply(&event(
        "snap",
        Some("t"),
        Some(1),
        json!({"type":"source_snapshot","sessions":[projection]}),
    ));
    assert_eq!(store.snapshot().active, Some(key()));
    assert!(store.snapshot().notifications.is_empty());
    store.mark_read(&key());
    assert_eq!(store.snapshot().active, None);
}

#[test]
fn stale_snapshot_omission_does_not_degrade_its_newer_session() {
    let mut store = SessionStore::default();
    store.apply(&requested("ask", Some("t"), Some(10), "q"));
    store.apply(&event(
        "stale",
        None,
        Some(2),
        json!({"type":"source_snapshot","sessions":[]}),
    ));
    assert_eq!(
        store.snapshot().sessions[0].status,
        SessionStatus::NeedsAttention
    );
}

#[test]
fn stale_start_cannot_restore_running_after_restart() {
    let a = started("initial", Some("t"), Some(5));
    let mut store = SessionStore::restore(&[a], 1000);
    store.apply(&started("old-start", Some("t"), Some(1)));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}

#[test]
fn unrelated_resolution_cannot_claim_running_after_disconnection() {
    let mut store = SessionStore::default();
    store.apply(&started("fresh-start", Some("t"), Some(6)));
    store.apply(&event(
        "disconnect",
        None,
        None,
        json!({"type":"source_disconnected"}),
    ));
    store.apply(&event(
        "unrelated-resolution",
        Some("t"),
        Some(7),
        json!({"type":"attention_resolved","requestId":"absent"}),
    ));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}

#[test]
fn fresh_snapshot_cannot_reopen_a_confirmed_terminal() {
    let mut store = SessionStore::default();
    store.apply(&ended("end", Some("old"), Some(2), "completed"));
    let mut old_projection = store.snapshot().sessions[0].clone();
    old_projection.status = SessionStatus::Running;
    store.apply(&event(
        "snap1",
        Some("old"),
        Some(3),
        json!({"type":"source_snapshot","sessions":[old_projection.clone()]}),
    ));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Ended);
}

#[test]
fn fresh_snapshot_cannot_restore_a_retired_turn() {
    let mut store = SessionStore::default();
    store.apply(&ended("end", Some("old"), Some(2), "completed"));
    let old_projection = store.snapshot().sessions[0].clone();
    store.apply(&requested("ask", Some("new"), Some(4), "q"));
    store.apply(&event(
        "snap2",
        Some("old"),
        Some(5),
        json!({"type":"source_snapshot","sessions":[old_projection]}),
    ));
    assert_eq!(store.snapshot().sessions[0].turn_id.as_deref(), Some("new"));
    assert_eq!(store.snapshot().sessions[0].pending.len(), 1);
}

#[test]
fn snapshot_does_not_import_other_sessions_turn_identity() {
    let projection = SessionProjection {
        key: key(),
        parent_session_id: None,
        turn_id: Some("old".into()),
        status: SessionStatus::Running,
        pending: vec![],
        project: None,
        summary: None,
        unread: false,
        last_event_at_ms: 100,
    };
    let mut store = SessionStore::default();
    let mut snapshot = event(
        "snap",
        Some("unrelated"),
        Some(100),
        json!({"type":"source_snapshot","sessions":[projection]}),
    );
    snapshot.session_id = "another".into();
    store.apply(&snapshot);
    store.apply(&started("watermark", Some("old"), Some(1)));
    assert!(
        store
            .apply(&requested("ask", Some("unrelated"), Some(2), "q"))
            .should_notify
    );
    assert_eq!(
        store.snapshot().sessions[0].turn_id.as_deref(),
        Some("unrelated")
    );
}

#[test]
fn unsequenced_start_cannot_guess_running_after_restore() {
    let initial = started("initial-no-sequence", Some("t1"), None);
    let mut live = SessionStore::default();
    live.apply(&initial);
    assert_eq!(live.snapshot().sessions[0].status, SessionStatus::Running);

    let mut restored = SessionStore::restore(&[initial], 1000);
    assert_eq!(
        restored.snapshot().sessions[0].status,
        SessionStatus::Unknown
    );
    let mut late = started("late-no-sequence", Some("t1"), None);
    late.occurred_at_ms = 10_000;
    assert_eq!(
        restored.apply(&late),
        ApplyResult {
            changed: false,
            should_notify: false
        }
    );
    assert_eq!(
        restored.snapshot().sessions[0].status,
        SessionStatus::Unknown
    );
}

#[test]
fn unsequenced_start_cannot_guess_running_after_disconnection() {
    let mut store = SessionStore::default();
    store.apply(&started("initial", Some("t1"), Some(1)));
    store.apply(&event(
        "disconnect",
        None,
        None,
        json!({"type":"source_disconnected"}),
    ));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
    store.apply(&started("late-no-sequence", Some("t1"), None));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Unknown);
}

#[test]
fn new_turn_snapshot_preserves_imported_unread_end_until_user_clears_it() {
    for action in ["mark_read", "clear_history"] {
        let mut projection = SessionProjection {
            key: key(),
            parent_session_id: None,
            turn_id: Some("t1".into()),
            status: SessionStatus::Ended,
            pending: vec![],
            project: None,
            summary: None,
            unread: true,
            last_event_at_ms: 100,
        };
        let mut store = SessionStore::default();
        store.apply(&event(
            "end-snapshot",
            Some("t1"),
            Some(1),
            json!({"type":"source_snapshot","sessions":[projection.clone()]}),
        ));
        assert!(store.snapshot().sessions[0].unread);
        projection.turn_id = Some("t2".into());
        projection.status = SessionStatus::Running;
        projection.unread = false;
        assert!(
            !store
                .apply(&event(
                    "new-turn-snapshot",
                    Some("t2"),
                    Some(2),
                    json!({"type":"source_snapshot","sessions":[projection.clone()]})
                ))
                .should_notify
        );
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].turn_id.as_deref(), Some("t2"));
        assert_eq!(snapshot.sessions[0].status, SessionStatus::Running);
        assert!(
            snapshot.sessions[0].unread,
            "{action}: 未读结束不能被新回合快照清除"
        );
        assert!(snapshot.notifications.is_empty());

        let mut competing = started("competing", Some("other-turn"), Some(1)).with_session("other");
        competing.occurred_at_ms = 200;
        store.apply(&competing);
        assert_eq!(
            store.snapshot().active,
            Some(key()),
            "未读结束应优先于更新的运行会话"
        );
        let result = match action {
            "mark_read" => store.mark_read(&key()),
            "clear_history" => store.clear_history(),
            _ => unreachable!(),
        };
        assert!(result.changed);
        assert!(!result.should_notify);
        let snapshot = store.snapshot();
        assert!(
            !snapshot
                .sessions
                .iter()
                .find(|session| session.key == key())
                .unwrap()
                .unread
        );
        assert_eq!(snapshot.active, Some(competing.session_key()));

        projection.turn_id = Some("t3".into());
        store.apply(&event(
            "following-turn-snapshot",
            Some("t3"),
            Some(3),
            json!({"type":"source_snapshot","sessions":[projection]}),
        ));
        assert!(
            !store
                .snapshot()
                .sessions
                .iter()
                .find(|session| session.key == key())
                .unwrap()
                .unread
        );
    }
}

#[test]
fn noncurrent_end_preserves_other_turn_pending_and_unread_end_history() {
    let mut store = SessionStore::default();
    store.apply(&ended("old-end", Some("old"), Some(1), "unknown"));
    store.apply(&requested(
        "current-ask",
        Some("current"),
        Some(2),
        "question",
    ));
    assert!(
        store
            .apply(&ended("unordered-end", Some("other"), None, "completed"))
            .should_notify
    );
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions[0].turn_id.as_deref(), Some("current"));
    assert_eq!(snapshot.sessions[0].status, SessionStatus::Unknown);
    assert_eq!(snapshot.sessions[0].pending.len(), 1);
    assert_eq!(snapshot.sessions[0].pending[0].id, "question");
    assert_eq!(snapshot.notifications.len(), 3);
    assert!(snapshot.notifications.iter().all(|note| !note.read));
    assert!(matches!(
        snapshot.notifications[1].kind,
        NotificationKind::Attention {
            resolved: false,
            ..
        }
    ));
    assert_eq!(snapshot.notifications[0].turn_id.as_deref(), Some("old"));
    assert_eq!(snapshot.notifications[2].turn_id.as_deref(), Some("other"));
}

#[test]
fn retired_turn_end_is_historical_and_keeps_current_attention() {
    let mut store = SessionStore::default();
    store.apply(&started("old-start", Some("old"), Some(1)));
    store.apply(&requested("new-ask", Some("new"), Some(3), "q"));
    let late_end = ended("late-end", Some("old"), Some(2), "interrupted");
    assert!(store.apply(&late_end).should_notify);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions[0].status, SessionStatus::NeedsAttention);
    assert_eq!(
        snapshot.sessions[0].pending[0].turn_id.as_deref(),
        Some("new")
    );
    assert_eq!(snapshot.notifications.len(), 2);
    assert!(snapshot.notifications.iter().all(|note| !note.read));
    assert!(!store.apply(&late_end).should_notify);
    assert!(
        !store
            .apply(&ended("duplicate-end", Some("old"), Some(4), "completed"))
            .should_notify
    );
    assert!(
        !store
            .apply(&started("late-start", Some("old"), Some(5)))
            .changed
    );
    assert_eq!(store.snapshot(), snapshot);
}

#[test]
fn noncurrent_terminal_survives_history_clear_and_snapshot_replacement() {
    let mut store = SessionStore::default();
    store.apply(&started("current-start", Some("current"), Some(1)));
    assert!(
        store
            .apply(&ended("other-end", Some("other"), None, "unknown"))
            .should_notify
    );
    store.clear_history();
    let mut projection = store.snapshot().sessions[0].clone();
    projection.status = SessionStatus::Running;
    let snapshot_event = event(
        "current-snapshot",
        Some("current"),
        Some(2),
        json!({"type":"source_snapshot","sessions":[projection.clone()]}),
    );
    store.apply(&snapshot_event);
    let before = store.snapshot();
    assert!(
        !store
            .apply(&started("other-late-start", Some("other"), Some(3)))
            .changed
    );
    assert!(
        !store
            .apply(&requested("other-late-ask", Some("other"), Some(4), "q"))
            .changed
    );
    projection.turn_id = Some("other".into());
    assert!(
        !store
            .apply(&event(
                "terminal-snapshot",
                Some("other"),
                Some(5),
                json!({"type":"source_snapshot","sessions":[projection]})
            ))
            .changed
    );
    assert!(
        !store
            .apply(&ended(
                "other-duplicate-end",
                Some("other"),
                Some(6),
                "completed"
            ))
            .should_notify
    );
    assert_eq!(store.snapshot(), before);
}

#[test]
fn confirmed_end_deduplication_is_scoped_to_exact_session_and_turn() {
    let mut store = SessionStore::default();
    store.apply(&started("current-start", Some("current"), None));
    assert!(
        store
            .apply(&ended("other-end", Some("other"), None, "unknown"))
            .should_notify
    );
    assert!(
        store
            .apply(
                &ended("different-session-end", Some("other"), None, "unknown")
                    .with_session("another-session")
            )
            .should_notify
    );
    assert!(
        store
            .apply(&ended("current-end", Some("current"), None, "unknown"))
            .should_notify
    );
    assert_eq!(store.snapshot().notifications.len(), 3);
    store.clear_history();
    assert!(
        !store
            .apply(&ended(
                "same-turn-retry-new-event",
                Some("other"),
                None,
                "unknown"
            ))
            .should_notify
    );
    assert!(store.snapshot().notifications.is_empty());
}

#[test]
fn same_turn_lower_sequence_end_keeps_newer_pending_until_valid_end() {
    let mut store = SessionStore::default();
    store.apply(&requested("newer-ask", Some("t"), Some(10), "q"));
    let before = store.snapshot();
    assert_eq!(
        store.apply(&ended("older-end", Some("t"), Some(9), "unknown")),
        ApplyResult::default()
    );
    assert_eq!(store.snapshot(), before);
    assert!(
        store
            .apply(&ended("valid-end", Some("t"), Some(11), "unknown"))
            .should_notify
    );
    let after = store.snapshot();
    assert!(after.sessions[0].pending.is_empty());
    assert_eq!(after.notifications.len(), 2);
    assert!(matches!(
        after.notifications[0].kind,
        NotificationKind::Attention { resolved: true, .. }
    ));
}

#[test]
fn comparable_new_start_can_begin_next_cycle_without_turn_id() {
    let mut store = SessionStore::default();
    assert!(
        store
            .apply(&ended("first-end", None, Some(1), "unknown"))
            .should_notify
    );
    store.apply(&started("next-start", None, Some(2)));
    assert_eq!(store.snapshot().sessions[0].status, SessionStatus::Running);
    assert!(
        store
            .apply(&ended("next-end", None, Some(3), "unknown"))
            .should_notify
    );
    assert_eq!(store.snapshot().notifications.len(), 2);
}

#[test]
fn replay_restores_noncurrent_terminal_facts_after_visible_history_expires() {
    let events = [
        started("current-start", Some("current"), Some(1)),
        ended("other-end", Some("other"), None, "unknown"),
    ];
    let mut live = SessionStore::default();
    for event in &events {
        live.apply(event);
    }
    assert_eq!(live.snapshot().notifications.len(), 1);
    let mut restored = SessionStore::restore(&events, 8 * 24 * 60 * 60 * 1000);
    assert!(restored.snapshot().notifications.is_empty());
    let before = restored.snapshot();
    assert!(
        !restored
            .apply(&started("late-other-start", Some("other"), Some(3)))
            .changed
    );
    assert!(
        !restored
            .apply(&ended(
                "same-turn-new-event",
                Some("other"),
                Some(4),
                "unknown"
            ))
            .should_notify
    );
    assert_eq!(restored.snapshot(), before);
}
