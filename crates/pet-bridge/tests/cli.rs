mod support;

use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn hook(args: &[&str], bytes: &[u8]) -> std::process::Output {
    hook_at(Command::new(env!("CARGO_BIN_EXE_pet-bridge")), args, bytes)
}

fn hook_at(mut command: Command, args: &[&str], bytes: &[u8]) -> std::process::Output {
    let mut child = command
        .arg("codex")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
fn input() -> Vec<u8> {
    serde_json::to_vec(&json!({"session_id":"test-session","turn_id":"test-turn","hook_event_name":"UserPromptSubmit","prompt":"提示词不可保存","tool_input":{"command":"参数不可保存"}})).unwrap()
}
#[test]
fn hook_probe_is_private_small_and_keeps_identity_across_invocations() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("探测 目录");
    let args = ["--probe-dir", probe.to_str().unwrap()];
    for _ in 0..2 {
        let result = hook(&args, &input());
        assert!(result.status.success());
        assert_eq!(result.stdout, b"{}\n");
    }
    let mut events = Vec::new();
    for file in std::fs::read_dir(&probe).unwrap() {
        let file = file.unwrap();
        if file.path().extension().is_some_and(|ext| ext == "json") {
            let bytes = std::fs::read(file.path()).unwrap();
            let event: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(bytes.len() < 65536);
            assert!(!String::from_utf8(bytes).unwrap().contains("不可保存"));
            events.push(event);
        }
    }
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["identity"], events[1]["identity"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&probe).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}
#[test]
fn malformed_oversize_or_bad_options_do_not_change_host_decision() {
    for (args, bytes) in [
        (vec![], "{\"secret\":\"私密原文\"".as_bytes().to_vec()),
        (vec![], vec![b'x'; 8 * 1024 * 1024 + 1]),
        (vec!["--bad"], input()),
    ] {
        let result = hook(&args, &bytes);
        assert!(result.status.success());
        assert_eq!(result.stdout, b"{}\n");
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(!error.is_empty());
        assert!(!error.contains("私密原文"));
    }
}
#[test]
fn probe_write_failure_still_declines_to_decide() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file");
    std::fs::write(&target, b"occupied").unwrap();
    let result = hook(&["--probe-dir", target.to_str().unwrap()], &input());
    assert!(result.status.success());
    assert_eq!(result.stdout, b"{}\n");
    assert!(!result.stderr.is_empty());
    assert_eq!(std::fs::read(target).unwrap(), b"occupied");
}
#[test]
fn codex_queues_offline_and_keeps_the_hook_stdout_on_disk_failure() {
    let root = tempfile::tempdir().unwrap();
    let (command, paths) = support::isolated_bridge(root.path());
    let args = ["--installation-id", "11111111-1111-4111-8111-111111111111"];
    let result = hook_at(command, &args, &input());
    assert!(result.status.success());
    assert_eq!(result.stdout, b"{}\n");
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let outbox = pet_bridge::outbox::Outbox::new(paths.clone()).unwrap();
    let queued = outbox.pending(pet_core::paths::wall_clock_ms()).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].envelope.session_id, "test-session");
    assert_eq!(queued[0].envelope.summary, None);

    let blocked = tempfile::tempdir().unwrap();
    let (command, blocked_paths) = support::isolated_bridge(blocked.path());
    std::fs::create_dir_all(blocked_paths.root.parent().unwrap()).unwrap();
    std::fs::write(&blocked_paths.root, b"occupied").unwrap();
    let result = hook_at(command, &args, &input());
    assert!(result.status.success());
    assert_eq!(result.stdout, b"{}\n");
    assert!(!result.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("不可保存"));
    assert!(!blocked_paths.outbox.exists());
}

#[test]
fn install_plan_requires_explicit_apply_and_remove_uses_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("hooks.json");
    let plan = dir.path().join("plan.json");
    let remove = dir.path().join("remove.json");
    std::fs::write(&config, b"{}").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_pet-bridge"))
            .arg("hooks")
            .args(args)
            .output()
            .unwrap()
    };
    let config = config.to_str().unwrap();
    let plan = plan.to_str().unwrap();
    let result = run(&[
        "plan-install",
        "--config",
        config,
        "--bridge",
        env!("CARGO_BIN_EXE_pet-bridge"),
        "--plan",
        plan,
    ]);
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(std::fs::read(config).unwrap(), b"{}");
    assert!(
        run(&["apply", "--config", config, "--plan", plan])
            .status
            .success()
    );
    let installed: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 1);
    assert!(
        run(&[
            "plan-remove",
            "--config",
            config,
            "--manifest",
            plan,
            "--plan",
            remove.to_str().unwrap()
        ])
        .status
        .success()
    );
    assert!(
        run(&[
            "apply",
            "--config",
            config,
            "--plan",
            remove.to_str().unwrap()
        ])
        .status
        .success()
    );
    let after: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert!(after["hooks"]["Stop"].as_array().unwrap().is_empty());
}

#[test]
fn default_hook_observation_scope_tracks_actual_child_instead_of_shared_root() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("events");
    for agent in ["child-a", "child-b", "child-a"] {
        let bytes=serde_json::to_vec(&json!({"hook_event_name":"SubagentStart","session_id":"shared-root","agent_id":agent,"turn_id":"child-turn"})).unwrap();
        let result = hook(&["--probe-dir", probe.to_str().unwrap()], &bytes);
        assert!(result.status.success());
        assert_eq!(result.stdout, b"{}\n");
    }
    let events: Vec<Value> = std::fs::read_dir(&probe)
        .unwrap()
        .filter_map(|file| {
            let path = file.unwrap().path();
            (path.extension().is_some_and(|ext| ext == "json"))
                .then(|| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        })
        .collect();
    assert_eq!(events.len(), 3);
    let installation = events[0]["identity"]["installationId"].clone();
    for event in events {
        let agent = event["sessionId"].as_str().unwrap();
        assert!(matches!(agent, "child-a" | "child-b"));
        assert_eq!(
            event["identity"]["instanceId"],
            format!("hook-session:{agent}")
        );
        assert_eq!(event["identity"]["installationId"], installation);
        assert!(event.get("parentSessionId").is_none());
    }
}
