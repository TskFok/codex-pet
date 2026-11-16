use pet_bridge::hook_install::{apply_hook_patch, plan_hook_install, plan_hook_remove};
use serde_json::{Value, json};
use std::path::Path;

#[test]
fn merge_preserves_foreign_hooks() {
    let original = json!({"description":"用户配置","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo 第三方"}]}],"Other":[{"custom":true}]}});
    let patch = plan_hook_install(
        &serde_json::to_vec(&original).unwrap(),
        &std::env::temp_dir().join("中文 空格/pet-bridge"),
    )
    .unwrap();
    let second = plan_hook_install(
        &patch.after_bytes,
        &std::env::temp_dir().join("中文 空格/pet-bridge"),
    )
    .unwrap();
    assert_eq!(patch.after_bytes, second.after_bytes);
    let mut current: Value = serde_json::from_slice(&second.after_bytes).unwrap();
    current["用户新增"] = json!(true);
    current["hooks"]["Stop"].as_array_mut().unwrap()[0]["hooks"][0]["command"] =
        json!("echo 用户更新");
    let removal = plan_hook_remove(&serde_json::to_vec(&current).unwrap(), &patch).unwrap();
    let after: Value = serde_json::from_slice(&removal.after_bytes).unwrap();
    assert_eq!(after["用户新增"], true);
    assert_eq!(
        after["hooks"]["Stop"][0]["hooks"][0]["command"],
        "echo 用户更新"
    );
    assert_eq!(after["hooks"]["Other"], original["hooks"]["Other"]);
    assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 1);
}
#[test]
fn changed_owned_hook_is_preserved_on_removal() {
    let patch = plan_hook_install(b"{}", &std::env::temp_dir().join("pet-bridge")).unwrap();
    let mut current: Value = serde_json::from_slice(&patch.after_bytes).unwrap();
    current["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(2);
    let removal = plan_hook_remove(&serde_json::to_vec(&current).unwrap(), &patch).unwrap();
    let after: Value = serde_json::from_slice(&removal.after_bytes).unwrap();
    assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["timeout"], 2);
}
#[test]
fn apply_rejects_concurrent_changes_and_backs_up_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hooks.json");
    std::fs::write(&path, b"{}").unwrap();
    let patch = plan_hook_install(b"{}", &std::env::temp_dir().join("pet-bridge")).unwrap();
    std::fs::write(&path, b"{\"changed\":true}").unwrap();
    assert!(apply_hook_patch(&path, &patch).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"changed\":true}");
    std::fs::write(&path, b"{}").unwrap();
    apply_hook_patch(&path, &patch).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), patch.after_bytes);
    assert!(
        std::fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("backup"))
    );
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .contains("manifest")
    }));
}
#[cfg(unix)]
#[test]
fn chinese_space_and_shell_metacharacters_are_one_argument() {
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
    let patch = plan_hook_install(b"{}", &binary).unwrap();
    let value: Value = serde_json::from_slice(&patch.after_bytes).unwrap();
    let command = value["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("PET_TEST_ARGS", &output)
            .status()
            .unwrap()
            .success()
    );
    let args = std::fs::read_to_string(output).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert_eq!(args[0], "codex");
    assert_eq!(args[1], "--installation-id");
    assert_eq!(args.len(), 3);
}
#[test]
fn malformed_config_and_relative_bridge_are_rejected() {
    for bytes in [
        b"[]".as_slice(),
        b"{\"hooks\":[]}",
        b"{\"hooks\":{\"Stop\":false}}",
    ] {
        assert!(plan_hook_install(bytes, &std::env::temp_dir().join("pet-bridge")).is_err());
    }
    assert!(plan_hook_install(b"{}", Path::new("relative")).is_err());
}
#[cfg(unix)]
#[test]
fn installer_refuses_symlink_and_existing_lock() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.json");
    std::fs::write(&real, b"{}").unwrap();
    let path = dir.path().join("hooks.json");
    symlink(&real, &path).unwrap();
    let patch = plan_hook_install(b"{}", &std::env::temp_dir().join("pet-bridge")).unwrap();
    assert!(apply_hook_patch(&path, &patch).is_err());
    assert_eq!(std::fs::read(&real).unwrap(), b"{}");
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"{}").unwrap();
    std::fs::write(dir.path().join(".hooks.json.pet-bridge.lock"), b"busy").unwrap();
    assert!(apply_hook_patch(&path, &patch).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"{}");
}
#[cfg(windows)]
#[test]
fn windows_encoded_hook_preserves_native_stdin_stdout_and_quoted_path() {
    use std::{
        io::Write,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("中文 空格 '$` bridge.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_pet-bridge"), &path).unwrap();
    let command = pet_bridge::hook_install::windows_command(
        path.to_str().unwrap(),
        "11111111-1111-4111-8111-111111111111",
    )
    .unwrap();
    let mut child = Command::new(std::env::var("COMSPEC").unwrap_or("cmd.exe".into()))
        .arg("/C")
        .raw_arg(format!("\"{command}\""))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input=serde_json::to_vec(&json!({"hook_event_name":"UserPromptSubmit","session_id":"test-session","turn_id":"test-turn","prompt":"仅测试".repeat(30000)})).unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("投递尚未接入"));
}
#[test]
fn binary_upgrade_changes_path_without_changing_installation_identity() {
    let first=plan_hook_install(br#"{"description":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo foreign"}]}]}}"#,&std::env::temp_dir().join("旧版本/pet-bridge")).unwrap();
    let before: Value = serde_json::from_slice(&first.after_bytes).unwrap();
    let command = before["hooks"]["Interrupt"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let old_id = command.rsplit_once(" --installation-id ").unwrap().1;
    let upgraded = plan_hook_install(
        &first.after_bytes,
        &std::env::temp_dir().join("新 版本/pet-bridge"),
    )
    .unwrap();
    let after: Value = serde_json::from_slice(&upgraded.after_bytes).unwrap();
    let command = after["hooks"]["Interrupt"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("新 版本"));
    assert_eq!(
        command.rsplit_once(" --installation-id ").unwrap().1,
        old_id
    );
    assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(
        after["hooks"]["Stop"][0]["hooks"][0]["command"],
        "echo foreign"
    );
}

// 固定v1旧配置形状，事件范围故意不随当前安装器常量变化。
fn legacy_v1_config(bridge: &Path) -> Value {
    let id = "11111111-1111-4111-8111-111111111111";
    let path = bridge.to_str().unwrap();
    use base64::Engine;
    let legacy_script = format!(
        "$ErrorActionPreference='Stop'; & '{}' codex --installation-id '{}'; exit $LASTEXITCODE",
        path.replace('\'', "''"),
        id
    );
    let legacy_bytes: Vec<_> = legacy_script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let legacy_windows = format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(legacy_bytes)
    );
    let group = json!({"hooks":[{"type":"command","command":format!("'{}' codex --installation-id {id}",path.replace('\'',"'\"'\"'")),"commandWindows":legacy_windows,"timeout":1,"statusMessage":"Codex 桌宠通知（pet-bridge/v1）"}]});
    json!({"description":"旧版用户配置","hooks":{"UserPromptSubmit":[group.clone()],"PermissionRequest":[group.clone(),{"hooks":[{"type":"command","command":"foreign approval hook"}]}],"Stop":[group.clone()],"Interrupt":[group]}})
}

#[test]
fn legacy_install_migrates_to_handled_lifecycles_without_orphaning_old_permission_hook() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hooks.json");
    let old_bridge = dir.path().join("旧 版本/pet-bridge");
    let legacy = legacy_v1_config(&old_bridge);
    let old = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &old).unwrap();
    let new_bridge = dir.path().join("新版本/pet-bridge");
    let patch = plan_hook_install(&old, &new_bridge).unwrap();
    apply_hook_patch(&path, &patch).unwrap();
    let migrated: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        migrated["hooks"]["PermissionRequest"],
        json!([{"hooks":[{"type":"command","command":"foreign approval hook"}]}])
    );
    for name in [
        "UserPromptSubmit",
        "Stop",
        "Interrupt",
        "SubagentStart",
        "SubagentStop",
    ] {
        let command = migrated["hooks"][name][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.contains("新版本"));
        assert!(command.ends_with(" --installation-id 11111111-1111-4111-8111-111111111111"));
        let mut input = json!({"hook_event_name":name,"session_id":"root","turn_id":"own-turn"});
        if name.starts_with("Subagent") {
            input["agent_id"] = json!("child");
        }
        let identity = pet_core::SourceIdentity {
            source: pet_core::Source::Codex,
            installation_id: "i".into(),
            instance_id: "p".into(),
        };
        assert_eq!(
            pet_bridge::codex::normalize_codex(&input, &identity, 1)
                .unwrap()
                .len(),
            1
        );
    }
    let second = plan_hook_install(&patch.after_bytes, &new_bridge).unwrap();
    assert_eq!(second.after_bytes, patch.after_bytes);
    let removal = plan_hook_remove(&second.after_bytes, &second).unwrap();
    apply_hook_patch(&path, &removal).unwrap();
    let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        after["hooks"]["PermissionRequest"],
        migrated["hooks"]["PermissionRequest"]
    );
    for groups in after["hooks"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(name, _)| name.as_str() != "PermissionRequest")
        .map(|(_, value)| value)
    {
        assert!(groups.as_array().unwrap().is_empty());
    }
}

#[test]
fn modified_legacy_permission_hook_blocks_migration_and_is_preserved_by_removal() {
    let bridge = std::env::temp_dir().join("pet-bridge");
    let mut legacy = legacy_v1_config(&bridge);
    legacy["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(2);
    let bytes = serde_json::to_vec(&legacy).unwrap();
    assert!(plan_hook_install(&bytes, &bridge).is_err());
    let old = legacy_v1_config(&bridge);
    let canonical = serde_json::to_vec(&old).unwrap();
    let known = plan_hook_install(&canonical, &bridge).unwrap();
    let removal = plan_hook_remove(&bytes, &known).unwrap();
    let after: Value = serde_json::from_slice(&removal.after_bytes).unwrap();
    assert_eq!(
        after["hooks"]["PermissionRequest"],
        legacy["hooks"]["PermissionRequest"]
    );
}

#[test]
fn fresh_defaults_only_subscribe_to_events_the_adapter_handles() {
    let patch = plan_hook_install(b"{}", &std::env::temp_dir().join("pet-bridge")).unwrap();
    let value: Value = serde_json::from_slice(&patch.after_bytes).unwrap();
    assert!(value["hooks"].get("PermissionRequest").is_none());
    let identity = pet_core::SourceIdentity {
        source: pet_core::Source::Codex,
        installation_id: "i".into(),
        instance_id: "p".into(),
    };
    for (name, _) in value["hooks"].as_object().unwrap() {
        let mut input = json!({"hook_event_name":name,"session_id":"root","turn_id":"turn"});
        if name.starts_with("Subagent") {
            input["agent_id"] = json!("child");
        }
        assert_eq!(
            pet_bridge::codex::normalize_codex(&input, &identity, 1)
                .unwrap()
                .len(),
            1,
            "{name}"
        );
    }
}
