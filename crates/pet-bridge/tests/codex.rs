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
fn invalid_identity_missing_turn_or_session_and_subagents_are_rejected() {
    let mut input = fixture("start");
    input["session_id"] = json!("");
    assert!(normalize_codex(&input, &identity(), 42).is_err());
    input = fixture("start");
    input.as_object_mut().unwrap().remove("turn_id");
    assert!(normalize_codex(&input, &identity(), 42).is_err());
    input = fixture("start");
    input["agent_id"] = json!("subagent");
    assert!(normalize_codex(&input, &identity(), 42).unwrap().is_empty());
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
