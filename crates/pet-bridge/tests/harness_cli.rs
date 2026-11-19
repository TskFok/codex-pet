mod support;

use serde_json::json;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn run(args: &[&str], bytes: &[u8]) -> std::process::Output {
    run_at(Command::new(env!("CARGO_BIN_EXE_pet-bridge")), args, bytes)
}

fn run_at(mut command: Command, args: &[&str], bytes: &[u8]) -> std::process::Output {
    let mut child = command
        .arg("harness")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).ok();
    child.wait_with_output().unwrap()
}
fn valid() -> Vec<u8> {
    serde_json::to_vec(&json!({"schemaVersion":2,"eventId":"safe/event:identity","identity":{"source":"deepseek","installationId":"install","instanceId":"session-log:child:100"},"sessionId":"child","turnId":"1","occurredAtMs":100,"kind":{"type":"attention_requested","requestId":"q","reason":"answer","lifetime":"until_resolved"}})).unwrap()
}
#[test]
fn harness_probe_accepts_valid_v2_privately_and_deduplicates_exact_replays() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("中文 probe");
    for _ in 0..2 {
        let result = run(&["--probe-dir", dir.to_str().unwrap()], &valid());
        assert!(result.status.success(), "{:?}", result);
        assert!(result.stdout.is_empty());
    }
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(files.len(), 1);
    let event = pet_core::decode_event(&std::fs::read(files[0].path()).unwrap()).unwrap();
    assert_eq!(event.session_id, "child");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(files[0].metadata().unwrap().permissions().mode() & 0o077, 0);
    }
}
#[test]
fn harness_rejects_foreign_source_bad_json_and_oversize_without_payload() {
    let mut foreign: serde_json::Value = serde_json::from_slice(&valid()).unwrap();
    foreign["identity"]["source"] = json!("codex");
    let root = tempfile::tempdir().unwrap();
    let args = ["--probe-dir", root.path().to_str().unwrap()];
    for bytes in [
        serde_json::to_vec(&foreign).unwrap(),
        "{\"secret\":\"私密原文\"".as_bytes().to_vec(),
        vec![b'x'; 65537],
    ] {
        let result = run(&args, &bytes);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            !String::from_utf8(result.stderr)
                .unwrap()
                .contains("私密原文")
        );
    }
}

#[test]
fn harness_queues_without_probe_and_explicit_probe_stays_isolated() {
    let root = tempfile::tempdir().unwrap();
    let (command, paths) = support::isolated_bridge(root.path());
    let probe = root.path().join("probe");
    let result = run_at(command, &["--probe-dir", probe.to_str().unwrap()], &valid());
    assert!(result.status.success());
    assert!(!paths.root.exists());

    let (command, _) = support::isolated_bridge(root.path());
    let result = run_at(command, &[], &valid());
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let outbox = pet_bridge::outbox::Outbox::new(paths).unwrap();
    let queued = outbox.pending(pet_core::paths::wall_clock_ms()).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].envelope.event_id, "safe/event:identity");
}
