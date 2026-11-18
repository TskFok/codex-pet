mod support;
use pet_core::*;
use serde_json::json;
use support::{event, fixture_path};

#[test]
fn protocol_rejects_invalid_input() {
    for name in [
        "unknown-version",
        "missing-session",
        "unknown-kind",
        "unknown-reason",
        "empty-id",
    ] {
        assert!(
            decode_event(&std::fs::read(fixture_path("protocol", name)).unwrap()).is_err(),
            "{name}"
        );
    }
    assert!(decode_event(&vec![b' '; MAX_EVENT_BYTES + 1]).is_err());
    assert!(decode_event(b"not json").is_err());
}

#[test]
fn size_limit_counts_bytes_and_accepts_exact_boundary() {
    let original = std::fs::read(fixture_path("protocol", "valid-attention")).unwrap();
    let mut bytes = original.clone();
    bytes.resize(MAX_EVENT_BYTES, b' ');
    assert!(decode_event(&bytes).is_ok());
    bytes.push(b' ');
    assert!(decode_event(&bytes).is_err());
}

#[test]
fn summary_limit_counts_unicode_characters_in_envelope_and_snapshot() {
    let base = event("a", Some("t"), Some(1), json!({"type":"turn_started"}));
    let mut value = serde_json::to_value(base).unwrap();
    value["summary"] = json!("鱼".repeat(512));
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["summary"] = json!("鱼".repeat(513));
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("summary");
    value["kind"] = json!({"type":"source_snapshot","sessions":[{
        "key":{"source":"codex","installationId":"local","sessionId":"s"},
        "status":"needs_attention", "pending":[{"id":"q","reason":"answer","summary":"鱼".repeat(513)}],
        "unread":true,"lastEventAtMs":1
    }]});
    assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn snapshot_rejects_foreign_identity_and_inconsistent_requests() {
    for projection in [
        json!({"key":{"source":"deepseek","installationId":"local","sessionId":"s"},"status":"running","pending":[],"unread":false,"lastEventAtMs":1}),
        json!({"key":{"source":"codex","installationId":"local","sessionId":"s"},"status":"needs_attention","pending":[],"unread":false,"lastEventAtMs":1}),
    ] {
        let base = event("a", None, None, json!({"type":"turn_started"}));
        let mut value = serde_json::to_value(base).unwrap();
        value["kind"] = json!({"type":"source_snapshot","sessions":[projection]});
        assert!(decode_event(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn wire_format_is_camel_case_with_tagged_snake_case_kinds() {
    let bytes = std::fs::read(fixture_path("protocol", "valid-attention")).unwrap();
    let decoded = decode_event(&bytes).unwrap();
    assert_eq!(decoded.schema_version, 1);
    assert_eq!(
        decoded.kind,
        EventKind::AttentionRequested {
            request_id: "q".into(),
            reason: AttentionReason::Approval,
            lifetime: RequestLifetime::Turn
        }
    );
    let output = serde_json::to_value(decoded).unwrap();
    assert_eq!(output["kind"]["requestId"], "q");
    assert!(output.get("receivedAtMs").is_none());
    assert_eq!(
        output,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    );
}

#[test]
fn required_identity_strings_and_optional_ids_cannot_be_empty() {
    let base =
        serde_json::to_value(event("a", None, None, json!({"type":"turn_started"}))).unwrap();
    for pointer in [
        "/eventId",
        "/sessionId",
        "/identity/installationId",
        "/identity/instanceId",
        "/turnId",
        "/parentSessionId",
        "/upstreamEventId",
    ] {
        let mut value = base.clone();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = json!(" ");
        assert!(
            decode_event(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn all_event_kinds_have_shared_valid_wire_fixtures() {
    for name in [
        "valid-start",
        "valid-attention",
        "valid-resolved",
        "valid-end",
        "valid-disconnected",
        "valid-snapshot",
    ] {
        let bytes = std::fs::read(fixture_path("protocol", name)).unwrap();
        let decoded = decode_event(&bytes).unwrap();
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            "{name}"
        );
    }
}
