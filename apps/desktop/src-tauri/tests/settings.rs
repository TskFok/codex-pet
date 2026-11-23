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
