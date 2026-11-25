use crate::hook_install::{HookPatch, InstallError, apply_patch_with_validator};
use base64::Engine;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const HOOK_NAME: &str = "codex-pet";
const EVENTS: [(&str, &str); 2] = [("PreInvocation", "pre-invocation"), ("Stop", "stop")];
const PS_PREFIX: &str = "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ";

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn entry_id(value: &Value) -> String {
    hash(&serde_json::to_vec(&(HOOK_NAME, value)).expect("JSON 值可序列化"))
}

fn parse(existing: &[u8]) -> Result<Value, InstallError> {
    let root: Value = if existing.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(existing).map_err(|_| "Antigravity Hook 配置不是有效 JSON")?
    };
    let object = root.as_object().ok_or("Antigravity Hook 配置必须为对象")?;
    for spec in object.values() {
        if !spec.is_object() {
            return Err("Antigravity Hook 条目必须为对象".into());
        }
    }
    Ok(root)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn unix_command(
    bridge: &str,
    event_flag: &str,
    installation_id: &str,
) -> Result<String, InstallError> {
    validate_command_parts(bridge, installation_id)?;
    Ok(format!(
        "{} antigravity --event {event_flag} --installation-id {installation_id}",
        shell_quote(bridge)
    ))
}

/// Windows 端 Antigravity 通过 cmd /c 执行 command；编码脚本避免 shell 元字符解释。
pub fn windows_command(
    bridge: &str,
    event_flag: &str,
    installation_id: &str,
) -> Result<String, InstallError> {
    validate_command_parts(bridge, installation_id)?;
    let script = format!(
        "$ErrorActionPreference='Stop'; & '{}' antigravity --event '{}' --installation-id '{}' | ForEach-Object {{ $_ }}; exit $LASTEXITCODE",
        bridge.replace('\'', "''"),
        event_flag,
        installation_id
    );
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Ok(format!(
        "{PS_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn validate_command_parts(bridge: &str, installation_id: &str) -> Result<(), InstallError> {
    uuid::Uuid::parse_str(installation_id).map_err(|_| "安装 ID 必须为 UUID")?;
    if bridge.contains(['\0', '\r', '\n']) {
        return Err("桥接路径包含控制字符".into());
    }
    Ok(())
}

fn build_spec(bridge: &str, installation_id: &str, windows: bool) -> Result<Value, InstallError> {
    let mut map = Map::new();
    for (event_key, event_flag) in EVENTS {
        let command = if windows {
            windows_command(bridge, event_flag, installation_id)?
        } else {
            unix_command(bridge, event_flag, installation_id)?
        };
        map.insert(
            event_key.into(),
            json!([{
                "type": "command",
                "command": command,
                "timeout": 1
            }]),
        );
    }
    Ok(Value::Object(map))
}

fn parse_unix_command(command: &str) -> Option<(String, String)> {
    let (prefix, id) = command.rsplit_once(" --installation-id ")?;
    uuid::Uuid::parse_str(id).ok()?;
    let quoted = prefix.strip_suffix(" antigravity --event pre-invocation")?;
    let bridge = quoted
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))?
        .replace("'\"'\"'", "'");
    if !Path::new(&bridge).is_absolute() || quoted != shell_quote(&bridge) {
        return None;
    }
    Some((bridge, id.to_owned()))
}

fn parse_windows_command(command: &str) -> Option<(String, String)> {
    let encoded = command.strip_prefix(PS_PREFIX)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    let script = String::from_utf16(&units).ok()?;
    let rest = script.strip_prefix("$ErrorActionPreference='Stop'; & '")?;
    let rest = rest.strip_suffix("' | ForEach-Object { $_ }; exit $LASTEXITCODE")?;
    let (bridge_escaped, id) =
        rest.rsplit_once("' antigravity --event 'pre-invocation' --installation-id '")?;
    uuid::Uuid::parse_str(id).ok()?;
    let bridge = bridge_escaped.replace("''", "'");
    if bridge.is_empty() || bridge.contains(['\0', '\r', '\n']) {
        return None;
    }
    Some((bridge, id.to_owned()))
}

/// 识别精确匹配的自有 `codex-pet` 条目；若已手动改动则显式报冲突，避免误覆盖。
pub fn inspect_antigravity_hooks(
    existing: &[u8],
) -> Result<(Option<String>, Vec<String>), InstallError> {
    let root = parse(existing)?;
    let Some(spec) = root.get(HOOK_NAME) else {
        return Ok((None, Vec::new()));
    };
    let command = spec
        .get("PreInvocation")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("command"))
        .and_then(Value::as_str)
        .ok_or("自有 Antigravity Hook 内容已修改，请先检查配置")?;
    let parsed = parse_unix_command(command)
        .map(|(bridge, id)| (bridge, id, false))
        .or_else(|| parse_windows_command(command).map(|(bridge, id)| (bridge, id, true)))
        .ok_or("自有 Antigravity Hook 内容已修改，请先检查配置")?;
    let (bridge, id, is_windows) = parsed;
    let expected = build_spec(&bridge, &id, is_windows)?;
    if *spec != expected {
        return Err("自有 Antigravity Hook 内容已修改，请先检查配置".into());
    }
    Ok((Some(id), vec![entry_id(spec)]))
}

pub fn plan_antigravity_install(existing: &[u8], bridge: &Path) -> Result<HookPatch, InstallError> {
    if !bridge.is_absolute() {
        return Err("桥接程序必须使用绝对路径".into());
    }
    let bridge = bridge.to_str().ok_or("桥接路径不是有效 Unicode")?;
    if bridge.contains(['\0', '\r', '\n']) {
        return Err("桥接路径包含控制字符".into());
    }
    let (installation_id, _) = inspect_antigravity_hooks(existing)?;
    let id = installation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut root = parse(existing)?;
    let owned = build_spec(bridge, &id, cfg!(windows))?;
    let owned_id = entry_id(&owned);
    root.as_object_mut()
        .expect("已验证对象")
        .insert(HOOK_NAME.into(), owned);
    Ok(HookPatch {
        before_hash: hash(existing),
        after_bytes: serde_json::to_vec_pretty(&root).map_err(|_| "配置序列化失败")?,
        owned_entry_ids: vec![owned_id],
    })
}

pub fn plan_antigravity_remove(
    existing: &[u8],
    installed: &HookPatch,
) -> Result<HookPatch, InstallError> {
    let mut root = parse(existing)?;
    let object = root.as_object_mut().expect("已验证对象");
    // 仅删除内容哈希仍与安装清单一致的自有条目，保留用户修改和第三方 Hook。
    if object
        .get(HOOK_NAME)
        .is_some_and(|spec| installed.owned_entry_ids.contains(&entry_id(spec)))
    {
        object.remove(HOOK_NAME);
    }
    Ok(HookPatch {
        before_hash: hash(existing),
        after_bytes: serde_json::to_vec_pretty(&root).map_err(|_| "配置序列化失败")?,
        owned_entry_ids: Vec::new(),
    })
}

pub fn apply_antigravity_patch(path: &Path, patch: &HookPatch) -> Result<(), InstallError> {
    apply_antigravity_patch_with_backup(path, patch).map(|_| ())
}

pub fn apply_antigravity_patch_with_backup(
    path: &Path,
    patch: &HookPatch,
) -> Result<Option<PathBuf>, InstallError> {
    apply_patch_with_validator(path, patch, |bytes| parse(bytes).map(|_| ()))
}
