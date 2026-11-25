use pet_core::{AppPaths, JournalStore, decode_event, paths::wall_clock_ms};
use pet_desktop::{
    broker::{BrokerHandle, HistoryAction},
    settings::{Preferences, SettingsPatch},
};

#[test]
fn preferences_survive_restart_and_reject_invalid_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(&temp.path().canonicalize().unwrap().join("app"));
    let defaults = Preferences::load(&paths).unwrap();
    assert_eq!(defaults.fps, 30);
    assert!(!defaults.sound && !defaults.autostart);
    let valid: SettingsPatch = serde_json::from_str(r#"{"fps":60,"sound":true}"#).unwrap();
    let updated = defaults.patched(&valid).unwrap();
    updated.save(&paths).unwrap();
    let invalid = serde_json::from_str(r#"{"fps":45,"sound":false}"#).unwrap();
    assert!(updated.patched(&invalid).is_err());
    assert_eq!(Preferences::load(&paths).unwrap(), updated);
}

#[tokio::test]
async fn broker_read_and_clear_are_durable_without_resolving_or_forgetting_event() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(&temp.path().canonicalize().unwrap().join("app"));
    let mut event = decode_event(include_bytes!(
        "../../../../tests/fixtures/protocol/valid-attention.json"
    ))
    .unwrap();
    event.occurred_at_ms = wall_clock_ms();
    JournalStore::open(paths.clone())
        .unwrap()
        .ingest(event.clone())
        .unwrap();
    let broker = BrokerHandle::start(paths.clone()).unwrap();
    let mut updates = broker.subscribe();
    let key = broker.snapshot().sessions[0].key.clone();
    let read = broker.history(HistoryAction::MarkRead(key)).await.unwrap();
    assert!(read.notifications[0].read);
    assert_eq!(read.sessions[0].pending.len(), 1);
    assert_eq!(updates.recv().await.unwrap().revision, read.revision);
    let cleared = broker.history(HistoryAction::Clear).await.unwrap();
    assert!(cleared.notifications.is_empty());
    assert_eq!(cleared.sessions[0].pending.len(), 1);
    assert_eq!(updates.recv().await.unwrap().revision, cleared.revision);
    drop(broker);
    let mut journal = JournalStore::open(paths).unwrap();
    assert!(journal.ingest(event).unwrap().duplicate);
    assert!(journal.snapshot().notifications.is_empty());
}

#[test]
fn changing_scale_keeps_latest_drag_coordinates() {
    use pet_desktop::placement::WindowPlacement;
    let patch: SettingsPatch = serde_json::from_str(r#"{"placement":{"scale":1.5}}"#).unwrap();
    let after_drag = WindowPlacement {
        x: 441.,
        y: 63.,
        scale: 1.,
        monitor_id: Some("external".into()),
    };
    let expected = WindowPlacement {
        scale: 1.5,
        ..after_drag.clone()
    };
    assert_eq!(patch.placement.unwrap().resolve(&after_drag), expected);
}

#[test]
fn deepseek_activity_preferences_validate_persist_and_redact_api_key_over_ipc() {
    use pet_desktop::{
        placement::WindowPlacement,
        settings::{DEEPSEEK_DEFAULT_MODEL, Settings, mask_api_key},
    };

    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(&temp.path().canonicalize().unwrap().join("app"));
    paths.prepare_root().unwrap();

    // 旧版 settings.json 无 DeepSeek 字段时应兼容并填充默认值
    std::fs::write(
        &paths.settings,
        r#"{"revision":2,"fps":30,"sound":false,"autostart":false,"petId":"yueying-cape-dancer","mutedSources":[]}"#,
    )
    .unwrap();
    let legacy = Preferences::load(&paths).unwrap();
    assert!(legacy.deepseek_enabled);
    assert_eq!(legacy.deepseek_model, DEEPSEEK_DEFAULT_MODEL);
    assert!(legacy.deepseek_api_key.is_empty());

    // 配置 API Key 和切换模型后落盘持久化，且 IPC 序列化绝不泄露明文 Key
    let patch: SettingsPatch = serde_json::from_str(
        r#"{"deepseekApiKey":"  sk-secret1234567890abcd  ","deepseekModel":"deepseek-reasoner","deepseekEnabled":true}"#,
    )
    .unwrap();
    let updated = legacy.patched(&patch).unwrap();
    assert_eq!(updated.deepseek_api_key, "sk-secret1234567890abcd");
    assert_eq!(updated.deepseek_model, "deepseek-reasoner");
    assert_eq!(updated.revision, 3);
    updated.save(&paths).unwrap();
    assert_eq!(Preferences::load(&paths).unwrap(), updated);

    let wire = serde_json::to_value(Settings {
        preferences: updated.clone(),
        placement: WindowPlacement::default(),
    })
    .unwrap();
    let raw_json = wire.to_string();
    assert!(!raw_json.contains("sk-secret1234567890abcd"));
    assert_eq!(wire["deepseekApiKeyConfigured"], true);
    assert_eq!(
        wire["deepseekApiKeyHint"],
        mask_api_key("sk-secret1234567890abcd").unwrap()
    );
    assert_eq!(wire["deepseekModel"], "deepseek-reasoner");
    assert_eq!(wire["deepseekEnabled"], true);

    // 拒绝不支持的模型和非法空白字符的 API Key
    let bad_model: SettingsPatch =
        serde_json::from_str(r#"{"deepseekModel":"gpt-4o"}"#).unwrap();
    assert!(updated.patched(&bad_model).is_err());

    let bad_key: SettingsPatch =
        serde_json::from_str(r#"{"deepseekApiKey":"sk-bad key with space"}"#).unwrap();
    assert!(updated.patched(&bad_key).is_err());

    // 支持清空 API Key
    let clear_patch: SettingsPatch = serde_json::from_str(r#"{"deepseekApiKey":""}"#).unwrap();
    let cleared = updated.patched(&clear_patch).unwrap();
    assert!(cleared.deepseek_api_key.is_empty());
    let cleared_wire = serde_json::to_value(Settings {
        preferences: cleared,
        placement: WindowPlacement::default(),
    })
    .unwrap();
    assert_eq!(cleared_wire["deepseekApiKeyConfigured"], false);
    assert!(cleared_wire["deepseekApiKeyHint"].is_null());
}
