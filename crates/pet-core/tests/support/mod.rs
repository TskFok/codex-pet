#![allow(dead_code)]
use pet_core::{EventEnvelope, PetSnapshot, decode_event};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct EventCase {
    pub events: Vec<EventEnvelope>,
    pub expected: PetSnapshot,
}

pub fn fixture_path(category: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(category)
        .join(format!("{name}.json"))
}

pub fn load_case(name: &str) -> EventCase {
    serde_json::from_slice(&std::fs::read(fixture_path("events", name)).unwrap()).unwrap()
}

pub fn event(
    id: &str,
    turn: Option<&str>,
    sequence: Option<u64>,
    kind: serde_json::Value,
) -> EventEnvelope {
    decode_event(
        &serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "eventId": id,
            "identity": {"source": "codex", "installationId": "local", "instanceId": "process-1"},
            "sessionId": "s", "turnId": turn, "sequence": sequence,
            "occurredAtMs": 100, "kind": kind
        }))
        .unwrap(),
    )
    .unwrap()
}
