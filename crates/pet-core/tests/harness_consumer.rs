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

#[test]
#[ignore = "由宿主重载集成测试提供真实事件"]
fn reloaded_direct_ask_matches_persisted_attention() {
    let path = std::env::var("PET_HARNESS_DIRECT_EVENTS").expect("需要真桥接事件样本");
    let [before, after]: [Vec<serde_json::Value>; 2] =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let decode = |value| decode_event(&serde_json::to_vec(&value).unwrap()).unwrap();
    let mut before: Vec<EventEnvelope> = before.into_iter().map(decode).collect();
    before.sort_by_key(|event| event.sequence.is_none());
    let after: Vec<EventEnvelope> = after.into_iter().map(decode).collect();
    let now = after
        .iter()
        .map(|event| event.occurred_at_ms)
        .max()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(&directory.path().canonicalize().unwrap().join("app"));
    {
        let mut journal = JournalStore::open_with_clock(paths.clone(), move || now).unwrap();
        for event in before {
            assert!(!journal.ingest(event).unwrap().duplicate);
        }
        assert_eq!(journal.snapshot().sessions[0].pending.len(), 2);
    }
    let mut journal = JournalStore::open_with_clock(paths, move || now).unwrap();
    let replay = after
        .iter()
        .find(|event| matches!(event.kind, EventKind::AttentionRequested { .. }))
        .unwrap();
    assert!(journal.ingest(replay.clone()).unwrap().duplicate);
    assert_eq!(journal.snapshot().sessions[0].pending.len(), 2);
    let idle_cancelled = after.iter().find(|event| event.turn_id.is_none()).unwrap();
    assert!(!journal.ingest(idle_cancelled.clone()).unwrap().duplicate);
    let snapshot = journal.snapshot();
    let [waiting] = snapshot.sessions[0].pending.as_slice() else {
        panic!("只保留原来仍在等待的回合问题");
    };
    assert_eq!(waiting.turn_id.as_deref(), Some("1"));
    let completed = after
        .iter()
        .find(|event| {
            matches!(event.kind, EventKind::AttentionResolved { .. }) && event.turn_id.is_some()
        })
        .unwrap();
    assert!(!journal.ingest(completed.clone()).unwrap().duplicate);
    let snapshot = journal.snapshot();
    assert!(snapshot.sessions[0].pending.is_empty());
    assert_eq!(
        snapshot
            .notifications
            .iter()
            .filter(|note| matches!(
                note.kind,
                NotificationKind::Attention { resolved: true, .. }
            ))
            .count(),
        2
    );
}
