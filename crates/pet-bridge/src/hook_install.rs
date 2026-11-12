use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub type InstallError = String;
const MARKER: &str = "Codex 桌宠通知（pet-bridge/v1）";
const EVENTS: [&str; 4] = ["UserPromptSubmit", "PermissionRequest", "Stop", "Interrupt"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HookPatch {
    pub before_hash: String,
    pub after_bytes: Vec<u8>,
    pub owned_entry_ids: Vec<String>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn entry_id(event: &str, value: &Value) -> String {
    hash(&serde_json::to_vec(&(event, value)).expect("JSON 值可序列化"))
}

fn parse(existing: &[u8]) -> Result<Value, InstallError> {
    let mut root: Value = if existing.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(existing).map_err(|_| "Hook 配置不是有效 JSON")?
    };
    let object = root.as_object_mut().ok_or("Hook 配置必须为对象")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object()
        .ok_or("hooks 必须为对象")?;
    for groups in hooks.values() {
        if !groups.is_array() {
            return Err("Hook 事件必须为数组".into());
        }
    }
    Ok(root)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// 0.145.0 Windows command runner 使用 COMSPEC /C；编码脚本不含 shell 元字符。
/// 真正的二进制路径只在 PowerShell 字面字符串中解释，stdin/stdout 直接继承。
pub fn windows_command(bridge: &str, installation_id: &str) -> Result<String, InstallError> {
    uuid::Uuid::parse_str(installation_id).map_err(|_| "安装 ID 必须为 UUID")?;
    if bridge.contains(['\0', '\r', '\n']) {
        return Err("桥接路径包含控制字符".into());
    }
    let script = format!(
        "$ErrorActionPreference='Stop'; & '{}' codex --installation-id '{}'; exit $LASTEXITCODE",
        bridge.replace('\'', "''"),
        installation_id
    );
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Ok(format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn group(bridge: &str, installation_id: &str) -> Result<Value, InstallError> {
    Ok(
        json!({"hooks":[{"type":"command","command":format!("{} codex --installation-id {}",shell_quote(bridge),installation_id),"commandWindows":windows_command(bridge,installation_id)?,"timeout":1,"statusMessage":MARKER}]}),
    )
}

pub fn plan_hook_install(existing: &[u8], bridge: &Path) -> Result<HookPatch, InstallError> {
    if !bridge.is_absolute() {
        return Err("桥接程序必须使用绝对路径".into());
    }
    let bridge = bridge.to_str().ok_or("桥接路径不是有效 Unicode")?;
    if bridge.contains(['\0', '\r', '\n']) {
        return Err("桥接路径包含控制字符".into());
    }
    let mut root = parse(existing)?;
    let hooks = root["hooks"].as_object_mut().expect("已验证对象");
    let mut installation_id = None;
    let mut previous_entries = Vec::new();
    // 仅识别完整匹配的自有条目。标记相同但内容改过时拒绝覆盖。
    for event in EVENTS {
        if let Some(groups) = hooks.get(event).and_then(Value::as_array) {
            for old in groups {
                if old["hooks"].as_array().is_some_and(|handlers| {
                    handlers
                        .iter()
                        .any(|handler| handler["statusMessage"] == MARKER)
                }) {
                    let id = old["hooks"][0]["command"]
                        .as_str()
                        .and_then(|command| {
                            command.rsplit_once(" --installation-id ").map(|(_, id)| id)
                        })
                        .ok_or("自有 Hook 已修改，请先检查清单")?;
                    uuid::Uuid::parse_str(id).map_err(|_| "自有 Hook 安装 ID 无效")?;
                    let command = old["hooks"][0]["command"].as_str().ok_or("自有命令无效")?;
                    let quoted = command
                        .strip_suffix(&format!(" codex --installation-id {id}"))
                        .ok_or("自有命令已修改")?;
                    let previous_bridge = quoted
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                        .ok_or("自有路径已修改")?
                        .replace("'\"'\"'", "'");
                    if !Path::new(&previous_bridge).is_absolute()
                        || quoted != shell_quote(&previous_bridge)
                        || *old != group(&previous_bridge, id)?
                        || installation_id
                            .as_ref()
                            .is_some_and(|previous| previous != id)
                    {
                        return Err("自有 Hook 内容已修改，请先检查清单".into());
                    }
                    previous_entries.push(entry_id(event, old));
                    installation_id = Some(id.to_owned());
                }
            }
        }
    }
    let id = installation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let owned = group(bridge, &id)?;
    let mut owned_entry_ids = Vec::new();
    for event in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("已验证数组");
        groups.retain(|old| !previous_entries.contains(&entry_id(event, old)));
        if !groups.contains(&owned) {
            groups.push(owned.clone());
        }
        owned_entry_ids.push(entry_id(event, &owned));
    }
    Ok(HookPatch {
        before_hash: hash(existing),
        after_bytes: serde_json::to_vec_pretty(&root).map_err(|_| "配置序列化失败")?,
        owned_entry_ids,
    })
}

pub fn plan_hook_remove(existing: &[u8], installed: &HookPatch) -> Result<HookPatch, InstallError> {
    let mut root = parse(existing)?;
    let hooks = root["hooks"].as_object_mut().expect("已验证对象");
    // 用安装清单的内容哈希匹配，绝不以旧备份覆盖当前文件。
    for (event, groups) in hooks.iter_mut() {
        groups
            .as_array_mut()
            .expect("已验证数组")
            .retain(|value| !installed.owned_entry_ids.contains(&entry_id(event, value)));
    }
    Ok(HookPatch {
        before_hash: hash(existing),
        after_bytes: serde_json::to_vec_pretty(&root).map_err(|_| "配置序列化失败")?,
        owned_entry_ids: Vec::new(),
    })
}

fn private_new(path: &Path) -> Result<fs::File, InstallError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "无法独占创建安装文件".into())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    let mut file = private_new(path)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法持久化安装文件".into())
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn apply_hook_patch(path: &Path, patch: &HookPatch) -> Result<(), InstallError> {
    // 同一安装器互斥；外部编辑在提交前再次检查。拒绝符号链接目标。
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink() || !meta.is_file())
    {
        return Err("Hook 目标必须为普通文件".into());
    }
    let parent = path.parent().ok_or("Hook 路径缺少父目录")?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Hook 文件名无效")?;
    let lock_path = parent.join(format!(".{filename}.pet-bridge.lock"));
    let _lock_file = private_new(&lock_path)?;
    let _lock = Lock(lock_path);
    let read_current = || match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("无法读取 Hook 配置".to_owned()),
    };
    let before = read_current()?;
    if hash(&before) != patch.before_hash {
        return Err("Hook 配置已并发修改，请重新生成计划".into());
    }
    parse(&patch.after_bytes)?;
    if before == patch.after_bytes {
        return Ok(());
    }
    let transaction = uuid::Uuid::new_v4();
    let prefix = format!(".{filename}.pet-bridge.{transaction}");
    write_new(&parent.join(format!("{prefix}.backup.json")), &before)?;
    write_new(
        &parent.join(format!("{prefix}.manifest.json")),
        &serde_json::to_vec_pretty(patch).map_err(|_| "清单序列化失败")?,
    )?;
    let staging = parent.join(format!("{prefix}.tmp"));
    write_new(&staging, &patch.after_bytes)?;
    let result = (|| {
        if hash(&read_current()?) != patch.before_hash {
            return Err("Hook 配置在提交前被修改，请重新生成计划".into());
        }
        fs::rename(&staging, path).map_err(|_| "无法原子替换 Hook 配置".to_owned())?;
        #[cfg(unix)]
        {
            fs::File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| "无法同步配置目录")?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(staging);
    result
}
