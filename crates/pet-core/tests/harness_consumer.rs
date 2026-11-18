use pet_core::*;
fn events() -> Vec<EventEnvelope> {
    let default = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../integrations/deepseek-harness/fixtures/host-roundtrip.json"
    );
    let path = std::env::var("PET_HARNESS_EVENTS").unwrap_or(default.into());
    let values: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    values
        .into_iter()
        .map(|value| decode_event(&serde_json::to_vec(&value).unwrap()).unwrap())
        .collect()
}
#[test]
fn normalized_harness_actual_sdk_bridge_and_reload_are_consumable() {
    let events = events();
    assert_eq!(events.len(), 12);
    assert!(
        events
            .iter()
            .all(|e| e.schema_version == 2 && e.identity.source == Source::Deepseek)
    );
    assert_eq!(events[0].identity, events[6].identity);
    assert!(
        events[0]
            .identity
            .instance_id
            .starts_with("session-log:child:")
    );
    let mut store = SessionStore::default();
    for event in &events[..4] {
        store.apply(event);
    }
    assert_eq!(store.snapshot().sessions[0].pending[0].id, "b");
    for event in &events[4..6] {
        store.apply(event);
    }
    let state = store.snapshot();
    assert_eq!(state.sessions[0].status, SessionStatus::NeedsAttention);
    assert_eq!(state.sessions[0].pending.len(), 1);
    assert_eq!(state.sessions[0].pending[0].id, "question:q1");
    assert_eq!(
        state.sessions[0].pending[0].lifetime,
        RequestLifetime::UntilResolved
    );
    for event in &events[6..9] {
        store.apply(event);
    }
    assert_eq!(store.snapshot().sessions[0].pending[0].id, "c");
    for event in &events[9..] {
        store.apply(event);
    }
    let state = store.snapshot();
    assert_eq!(state.sessions[0].turn_id.as_deref(), Some("2"));
    assert_eq!(
        state.sessions[0].parent_session_id.as_deref(),
        Some("parent")
    );
    assert_eq!(state.sessions[0].status, SessionStatus::Running);
    assert!(state.sessions[0].pending.is_empty());
    for event in &events {
        assert!(!store.apply(event).should_notify);
    }
    // 请求先于出生回合开始、原回合结束迟到、解除先于请求，均使用真实转换结果。
    for order in [
        vec![4, 0, 1, 2, 3, 6, 5, 8, 7, 11, 10, 9],
        (0..12).rev().collect(),
    ] {
        let mut unordered = SessionStore::default();
        for index in order {
            unordered.apply(&events[index]);
        }
        assert!(unordered.snapshot().sessions[0].pending.is_empty());
        assert!(!unordered.snapshot().notifications.iter().any(|n| matches!(
            n.kind,
            NotificationKind::Attention {
                resolved: false,
                ..
            }
        )));
    }
}
