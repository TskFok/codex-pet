use pet_bridge::{
    antigravity::{HookEvent, normalize_antigravity},
    antigravity_install::{
        HOOK_NAME, apply_antigravity_patch, inspect_antigravity_hooks, plan_antigravity_install,
        plan_antigravity_remove, windows_command,
    },
};
use pet_core::{EventKind, NotificationKind, SessionStore, Source, SourceIdentity, TurnOutcome};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn identity() -> SourceIdentity {
    SourceIdentity {
        source: Source::Antigravity,
        installation_id: "11111111-1111-4111-8111-111111111111".into(),
        instance_id: "hook-session:ec33ebf9-0cba-4100-8142-c61503f6c587".into(),
    }
}

fn fixture(name: &str) -> (HookEvent, Value) {
    let raw: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "../../integrations/antigravity/fixtures/{name}.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let event = HookEvent::parse(raw["event"].as_str().unwrap()).unwrap();
    (event, raw["input"].clone())
}

#[test]
fn lifecycle_fixtures_map_start_and_stop_outcomes_accurately() {
    for (name, expected_kind, expected_turn) in [
        ("session-start", EventKind::TurnStarted, None),
        ("pre-invocation", EventKind::TurnStarted, None),
        (
            "stop-completed",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Completed,
            },
            Some("execution-1"),
        ),
        (
            "stop-interrupted",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Interrupted,
            },
            Some("execution-2"),
        ),
        (
            "stop-failed",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Failed,
            },
            Some("execution-3"),
        ),
        (
            "stop-max-invocations",
            EventKind::TurnEnded {
                outcome: TurnOutcome::Unknown,
            },
            Some("execution-4"),
        ),
    ] {
        let (event, input) = fixture(name);
        let events = normalize_antigravity(event, &input, &identity(), 100).unwrap();
        assert_eq!(events.len(), 1, "{name}");
        assert_eq!(events[0].kind, expected_kind, "{name}");
        assert_eq!(events[0].turn_id.as_deref(), expected_turn, "{name}");
        assert_eq!(
            events[0].project.as_ref().map(|p| p.name.as_str()),
            Some("codex-pet"),
            "{name}"
        );
        assert_eq!(
            events[0]
                .project
                .as_ref()
                .and_then(|p| p.directory.as_deref()),
            None,
            "{name}"
        );
        assert_eq!(events[0].summary, None, "{name}");
        assert_eq!(events[0].sequence, None, "{name}");
    }
}

#[test]
fn mid_turn_pre_invocation_is_ignored_to_avoid_noise() {
    let (event, input) = fixture("pre-invocation-mid-turn");
    assert!(
        normalize_antigravity(event, &input, &identity(), 100)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sensitive_prompt_paths_and_error_text_never_enter_normalized_event() {
    let (event, mut input) = fixture("stop-failed");
    input["lastUserInput"] = json!("绝密用户提示".repeat(20000));
    input["transcriptPath"] = json!("/Users/secret/transcript.jsonl");
    input["artifactDirectoryPath"] = json!("/Users/secret/artifacts");
    input["workspacePaths"] = json!(["/Users/secret/projects/codex-pet"]);
    input["error"] = json!("Bearer sk-secret-token");
    let events = normalize_antigravity(event, &input, &identity(), 100).unwrap();
    let bytes = serde_json::to_vec(&events).unwrap();
    assert!(bytes.len() < 64 * 1024);
    let text = String::from_utf8(bytes).unwrap();
    for forbidden in [
        "绝密",
        "/Users/secret",
        "transcript.jsonl",
        "artifacts",
        "sk-secret-token",
    ] {
        assert!(!text.contains(forbidden), "unexpected leak: {forbidden}");
    }
    assert!(text.contains("codex-pet"));
}

#[test]
fn invalid_identity_missing_conversation_id_or_bad_numbers_are_rejected() {
    let (event, mut input) = fixture("pre-invocation");
    input["conversationId"] = json!(" ");
    assert!(normalize_antigravity(event, &input, &identity(), 1).is_err());

    let (stop_event, mut stop_input) = fixture("stop-completed");
    stop_input["executionNum"] = json!(-1);
    assert!(normalize_antigravity(stop_event, &stop_input, &identity(), 1).is_err());

    let mut wrong = identity();
    wrong.source = Source::Codex;
    let (event, input) = fixture("pre-invocation");
    assert!(normalize_antigravity(event, &input, &wrong, 1).is_err());
}

#[test]
fn multi_turn_antigravity_stops_notify_once_per_execution_and_dedupe_retries() {
    let mut store = SessionStore::default();
    let (pre_event, pre_input) = fixture("pre-invocation");
    let start = normalize_antigravity(pre_event, &pre_input, &identity(), 1)
        .unwrap()
        .remove(0);
    assert!(!store.apply(&start).should_notify);

    for (name, expected_outcome) in [
        ("stop-completed", TurnOutcome::Completed),
        ("stop-failed", TurnOutcome::Failed),
    ] {
        let (event, input) = fixture(name);
        let end = normalize_antigravity(event, &input, &identity(), 2)
            .unwrap()
            .remove(0);
        assert!(store.apply(&end).should_notify, "{name}");
        // 同 executionNum 重复投递不会重复提醒
        let retry = normalize_antigravity(event, &input, &identity(), 3)
            .unwrap()
            .remove(0);
        assert!(!store.apply(&retry).should_notify, "{name}");
        assert!(store.snapshot().notifications.iter().any(|note| matches!(
            note.kind,
            NotificationKind::TurnEnded { outcome } if outcome == expected_outcome
        )));
    }
    assert_eq!(store.snapshot().notifications.len(), 2);
}

#[test]
fn antigravity_subagent_markers_set_is_subagent_flag() {
    let (event, mut input) = fixture("stop-completed");
    assert!(!normalize_antigravity(event, &input, &identity(), 100).unwrap()[0].is_subagent);

    input["isSubagent"] = json!(true);
    assert!(normalize_antigravity(event, &input, &identity(), 101).unwrap()[0].is_subagent);

    let (event2, mut input2) = fixture("stop-completed");
    input2["parentConversationId"] = json!("parent-conversation-id");
    assert!(normalize_antigravity(event2, &input2, &identity(), 102).unwrap()[0].is_subagent);
}

#[test]
fn installer_merges_upgrades_preserves_foreign_hooks_and_blocks_modified_owned_entry() {
    let original = json!({
        "lint-checker": {
            "PostToolUse": [{
                "matcher": "run_command",
                "hooks": [{"type": "command", "command": "./scripts/lint.sh"}]
            }]
        }
    });
    let old_bridge = std::env::temp_dir().join("旧目录/中文 空格/pet-bridge");
    let first =
        plan_antigravity_install(&serde_json::to_vec(&original).unwrap(), &old_bridge).unwrap();
    let second = plan_antigravity_install(&first.after_bytes, &old_bridge).unwrap();
    assert_eq!(first.after_bytes, second.after_bytes);

    let (first_id, ids) = inspect_antigravity_hooks(&first.after_bytes).unwrap();
    let first_id = first_id.expect("应生成安装 ID");
    assert_eq!(ids, first.owned_entry_ids);

    // 路径升级时保留原安装 UUID
    let new_bridge = std::env::temp_dir().join("新目录/pet-bridge");
    let upgraded = plan_antigravity_install(&first.after_bytes, &new_bridge).unwrap();
    let (upgraded_id, _) = inspect_antigravity_hooks(&upgraded.after_bytes).unwrap();
    assert_eq!(upgraded_id.as_deref(), Some(first_id.as_str()));

    // 手动修改自有条目时拒绝覆盖，且撤销时保留改动内容
    let mut edited: Value = serde_json::from_slice(&upgraded.after_bytes).unwrap();
    edited[HOOK_NAME]["Stop"][0]["timeout"] = json!(5);
    let edited_bytes = serde_json::to_vec(&edited).unwrap();
    assert!(inspect_antigravity_hooks(&edited_bytes).is_err());
    assert!(plan_antigravity_install(&edited_bytes, &new_bridge).is_err());
    let keep_edited = plan_antigravity_remove(&edited_bytes, &upgraded).unwrap();
    let after_keep: Value = serde_json::from_slice(&keep_edited.after_bytes).unwrap();
    assert_eq!(after_keep[HOOK_NAME]["Stop"][0]["timeout"], 5);

    // 未改动时按清单精确删除自有键，保留第三方 Hook
    let removed = plan_antigravity_remove(&upgraded.after_bytes, &upgraded).unwrap();
    let after: Value = serde_json::from_slice(&removed.after_bytes).unwrap();
    assert!(after.get(HOOK_NAME).is_none());
    assert_eq!(after["lint-checker"], original["lint-checker"]);
}

#[test]
fn windows_encoded_command_round_trips_through_inspector() {
    let id = "11111111-1111-4111-8111-111111111111";
    let bridge = r"C:\Users\中文 空格\O'Reilly\pet-bridge.exe";
    let config = json!({
        HOOK_NAME: {
            "PreInvocation": [{
                "type": "command",
                "command": windows_command(bridge, "pre-invocation", id).unwrap(),
                "timeout": 1
            }],
            "Stop": [{
                "type": "command",
                "command": windows_command(bridge, "stop", id).unwrap(),
                "timeout": 1
            }]
        }
    });
    let (found_id, entries) =
        inspect_antigravity_hooks(&serde_json::to_vec(&config).unwrap()).unwrap();
    assert_eq!(found_id.as_deref(), Some(id));
    assert_eq!(entries.len(), 1);
}

#[cfg(unix)]
#[test]
fn unix_hook_command_quotes_spaces_and_shell_metacharacters_as_single_argv0() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("中文 空格 '$` bridge");
    let output = dir.path().join("args");
    std::fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PET_TEST_ARGS\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let patch = plan_antigravity_install(b"{}", &binary).unwrap();
    let value: Value = serde_json::from_slice(&patch.after_bytes).unwrap();
    let command = value[HOOK_NAME]["Stop"][0]["command"].as_str().unwrap();
    assert!(
        Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("PET_TEST_ARGS", &output)
            .status()
            .unwrap()
            .success()
    );
    let args = std::fs::read_to_string(output).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert_eq!(args[0], "antigravity");
    assert_eq!(args[1], "--event");
    assert_eq!(args[2], "stop");
    assert_eq!(args[3], "--installation-id");
    assert_eq!(args.len(), 5);
}

#[test]
fn cli_binary_handles_antigravity_hooks_probe_and_non_blocking_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    let (_, stop_input) = fixture("stop-completed");

    // 正常 Stop 探测：写入脱敏事件并返回 {"decision":"allow"}
    let mut child = Command::new(env!("CARGO_BIN_EXE_pet-bridge"))
        .args([
            "antigravity",
            "--event",
            "stop",
            "--probe-dir",
            probe.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&stop_input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "{\"decision\":\"allow\"}"
    );
    assert!(output.stderr.is_empty());
    let files: Vec<_> = std::fs::read_dir(&probe)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .collect();
    assert_eq!(files.len(), 1);

    // 异常输入仍保持退出成功与非阻断 stdout，stderr 不泄露原始输入
    let mut bad = Command::new(env!("CARGO_BIN_EXE_pet-bridge"))
        .args([
            "antigravity",
            "--event",
            "stop",
            "--installation-id",
            "11111111-1111-4111-8111-111111111111",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    bad.stdin
        .take()
        .unwrap()
        .write_all(b"not-valid-json-secret-payload")
        .unwrap();
    let bad_out = bad.wait_with_output().unwrap();
    assert!(bad_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&bad_out.stdout).trim(),
        "{\"decision\":\"allow\"}"
    );
    assert!(
        !String::from_utf8_lossy(&bad_out.stderr).contains("secret-payload"),
        "stderr 不得泄露原始载荷"
    );

    // CLI antigravity-hooks plan-install / apply / plan-remove 完整链路
    let config = dir.path().join("hooks.json");
    let install_plan = dir.path().join("install-plan.json");
    let remove_plan = dir.path().join("remove-plan.json");
    let bridge = Path::new(env!("CARGO_BIN_EXE_pet-bridge"));

    assert!(
        Command::new(bridge)
            .args([
                "antigravity-hooks",
                "plan-install",
                "--config",
                config.to_str().unwrap(),
                "--bridge",
                bridge.to_str().unwrap(),
                "--plan",
                install_plan.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(bridge)
            .args([
                "antigravity-hooks",
                "apply",
                "--config",
                config.to_str().unwrap(),
                "--plan",
                install_plan.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        inspect_antigravity_hooks(&std::fs::read(&config).unwrap())
            .unwrap()
            .0
            .is_some()
    );
    assert!(
        Command::new(bridge)
            .args([
                "antigravity-hooks",
                "plan-remove",
                "--config",
                config.to_str().unwrap(),
                "--manifest",
                install_plan.to_str().unwrap(),
                "--plan",
                remove_plan.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    apply_antigravity_patch(
        &config,
        &serde_json::from_slice(&std::fs::read(&remove_plan).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(
        inspect_antigravity_hooks(&std::fs::read(&config).unwrap())
            .unwrap()
            .0
            .is_none()
    );
}
