use pet_bridge::codex::normalize_codex;
use pet_core::{EventKind, Source, SourceIdentity, TurnOutcome};
use serde_json::{Value, json};

fn identity() -> SourceIdentity {
    SourceIdentity {
        source: Source::Codex,
        installation_id: "test-install".into(),
        instance_id: "test-process".into(),
    }
}
fn fixture(name: &str) -> Value {
    serde_json::from_str::<Value>(
        &std::fs::read_to_string(format!("../../integrations/codex/fixtures/{name}.json")).unwrap(),
    )
    .unwrap()["input"]
        .clone()
}
#[test]
fn known_hook_shapes_keep_stop_outcome_unknown() {
    for (name, kind) in [
        ("start", EventKind::TurnStarted),
        (
            "stop",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Unknown,
            },
        ),
        (
            "interrupt",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Interrupted,
            },
        ),
    ] {
        let events = normalize_codex(&fixture(name), &identity(), 42).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, kind);
        assert_eq!(events[0].turn_id.as_deref(), Some("test-turn"));
        assert_eq!(events[0].identity, identity());
        assert_eq!(events[0].sequence, None);
        assert_eq!(events[0].upstream_event_id, None);
    }
}
#[test]
fn uncorrelated_permission_question_continue_cancel_do_not_invent_requests() {
    for name in ["permission", "question", "continue", "cancel"] {
        assert!(
            normalize_codex(&fixture(name), &identity(), 42)
                .unwrap()
                .is_empty(),
            "{name}"
        );
    }
}
#[test]
fn huge_prompt_and_tool_arguments_never_enter_normalized_event() {
    let mut input = fixture("start");
    input["prompt"] = json!("私密提示".repeat(26000));
    input["tool_input"] = json!({"command":"秘密参数"});
    let bytes = serde_json::to_vec(&normalize_codex(&input, &identity(), 42).unwrap()).unwrap();
    assert!(bytes.len() < 64 * 1024);
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("私密"));
    assert!(!text.contains("秘密"));
}
#[test]
fn invalid_source_identity_or_missing_turn_and_session_are_rejected() {
    let mut input = fixture("start");
    input["session_id"] = json!("");
    assert!(normalize_codex(&input, &identity(), 42).is_err());
    input = fixture("start");
    input.as_object_mut().unwrap().remove("turn_id");
    assert!(normalize_codex(&input, &identity(), 42).is_err());
    let mut wrong = identity();
    wrong.source = Source::Deepseek;
    assert!(normalize_codex(&fixture("start"), &wrong, 42).is_err());
}
#[test]
fn event_ids_do_not_dedupe_distinct_stop_invocations() {
    let a = normalize_codex(&fixture("stop"), &identity(), 42).unwrap();
    let b = normalize_codex(&fixture("stop"), &identity(), 42).unwrap();
    assert_ne!(a[0].event_id, b[0].event_id);
}
#[test]
fn oversized_metadata_cannot_bypass_event_limit() {
    let mut input = fixture("start");
    input["session_id"] = json!("s".repeat(65536));
    assert!(normalize_codex(&input, &identity(), 42).is_err());
}

#[test]
fn normalized_unsequenced_turns_keep_each_end_without_guessing_current_turn() {
    use pet_core::{NotificationKind, SessionStatus, SessionStore};
    let mut store = SessionStore::default();
    for turn in ["turn-a", "turn-b"] {
        for name in ["start", "stop"] {
            let mut input = fixture(name);
            input["turn_id"] = json!(turn);
            let event = normalize_codex(&input, &identity(), 42).unwrap().remove(0);
            assert_eq!(event.sequence, None);
            let result = store.apply(&event);
            assert_eq!(result.should_notify, name == "stop", "{turn}/{name}");
            assert!(!store.apply(&event).should_notify);
        }
    }
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.sessions[0].turn_id.as_deref(), Some("turn-a"));
    assert_eq!(snapshot.sessions[0].status, SessionStatus::Unknown);
    assert!(snapshot.sessions[0].unread);
    assert_eq!(snapshot.notifications.len(), 2);
    assert_eq!(snapshot.notifications[0].turn_id.as_deref(), Some("turn-a"));
    assert_eq!(snapshot.notifications[1].turn_id.as_deref(), Some("turn-b"));
    assert!(snapshot.notifications.iter().all(|note| !note.read
        && matches!(
            note.kind,
            NotificationKind::TurnEnded {
                outcome: TurnOutcome::Unknown
            }
        )));
    for name in ["start", "stop"] {
        let mut input = fixture(name);
        input["turn_id"] = json!("turn-b");
        let event = normalize_codex(&input, &identity(), 999999)
            .unwrap()
            .remove(0);
        assert!(!store.apply(&event).should_notify);
    }
    assert_eq!(store.snapshot(), snapshot);
}

#[test]
fn thread_spawn_lifecycle_uses_child_identity_and_own_turn_without_guessing_parent() {
    use pet_core::{AttentionReason, SessionStatus, SessionStore};
    let mut store = SessionStore::default();
    let root = normalize_codex(&json!({"hook_event_name":"UserPromptSubmit","session_id":"root-session","turn_id":"root-turn"}), &identity(), 1).unwrap().remove(0);
    store.apply(&root);
    let mut request = root.clone();
    request.event_id = "root-request".into();
    request.kind = EventKind::AttentionRequested {
        request_id: "root-approval".into(),
        reason: AttentionReason::Approval,
    };
    store.apply(&request);
    let mut child_end = None;
    for name in ["thread-spawn-start-alpha11", "thread-spawn-stop-alpha11"] {
        let mut input = fixture(name);
        input["parent_session_id"] = json!("forged-direct-parent");
        let event = normalize_codex(&input, &identity(), 2).unwrap().remove(0);
        assert_eq!(event.session_id, "child-thread");
        assert_eq!(event.turn_id.as_deref(), Some("child-turn"));
        assert_eq!(event.parent_session_id, None);
        assert_eq!(event.identity, identity());
        assert_eq!(event.sequence, None);
        let is_end = name.contains("stop");
        assert_eq!(store.apply(&event).should_notify, is_end);
        if is_end {
            child_end = Some(event);
        }
    }
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions.len(), 2);
    let root = snapshot
        .sessions
        .iter()
        .find(|session| session.key.session_id == "root-session")
        .unwrap();
    assert_eq!(root.status, SessionStatus::NeedsAttention);
    assert_eq!(root.turn_id.as_deref(), Some("root-turn"));
    assert_eq!(root.pending[0].id, "root-approval");
    assert_eq!(snapshot.notifications.len(), 2);
    assert!(snapshot.notifications.iter().all(|note| !note.read));
    assert!(!store.apply(&child_end.unwrap()).should_notify);
    assert!(
        !store
            .apply(
                &normalize_codex(&fixture("thread-spawn-stop-alpha11"), &identity(), 4)
                    .unwrap()
                    .remove(0)
            )
            .should_notify
    );
    store.apply(
        &normalize_codex(&fixture("thread-spawn-start-alpha11"), &identity(), 5)
            .unwrap()
            .remove(0),
    );
    assert_eq!(store.snapshot(), snapshot);
}

#[test]
fn shared_root_does_not_merge_sibling_or_nested_thread_identities() {
    use pet_core::SessionStore;
    let mut store = SessionStore::default();
    for agent in ["child-a", "child-b", "nested-child"] {
        let mut input = fixture("thread-spawn-start-alpha11");
        input["agent_id"] = json!(agent);
        input["turn_id"] = json!(format!("{agent}-turn"));
        input["parent_thread_id"] = json!("untrusted-field");
        let event = normalize_codex(&input, &identity(), 1).unwrap().remove(0);
        assert_eq!(event.session_id, agent);
        assert_eq!(event.parent_session_id, None);
        store.apply(&event);
    }
    let snapshot = store.snapshot();
    assert_eq!(snapshot.sessions.len(), 3);
    assert!(
        snapshot
            .sessions
            .iter()
            .all(|session| session.pending.is_empty() && session.parent_session_id.is_none())
    );
}

#[test]
fn user_prompt_agent_identity_is_supported_but_root_only_stop_and_interrupt_are_not_child_events() {
    let mut input = fixture("start");
    input["agent_id"] = json!("child-thread");
    input["turn_id"] = json!("child-turn");
    let event = normalize_codex(&input, &identity(), 1).unwrap().remove(0);
    assert_eq!(event.session_id, "child-thread");
    assert_eq!(event.turn_id.as_deref(), Some("child-turn"));
    for name in ["stop", "interrupt"] {
        let mut input = fixture(name);
        input["agent_id"] = json!("child-thread");
        assert!(normalize_codex(&input, &identity(), 2).unwrap().is_empty());
    }
}

#[test]
fn invalid_or_ambiguous_agent_identity_never_falls_back_to_root() {
    for value in [
        json!(""),
        json!("  "),
        json!(7),
        json!(false),
        json!("root-session"),
    ] {
        let mut input = fixture("thread-spawn-start-alpha11");
        input["agent_id"] = value;
        assert!(normalize_codex(&input, &identity(), 1).is_err(), "{input}");
    }
    for field in ["session_id", "agent_id", "turn_id"] {
        let mut input = fixture("thread-spawn-start-alpha11");
        input.as_object_mut().unwrap().remove(field);
        assert!(normalize_codex(&input, &identity(), 1).is_err());
    }
    let mut input = fixture("start");
    input["agent_id"] = json!(42);
    assert!(normalize_codex(&input, &identity(), 1).is_err());
    input["agent_id"] = Value::Null;
    assert_eq!(
        normalize_codex(&input, &identity(), 1).unwrap()[0].session_id,
        "test-session"
    );
}
