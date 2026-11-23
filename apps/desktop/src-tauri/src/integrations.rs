use crate::{
    commands::{CommandError, Result},
    notification_paths,
};
use pet_bridge::hook_install::{
    HookPatch, apply_hook_patch_with_backup, inspect_hook_entries, plan_hook_install,
    plan_hook_remove,
};
use pet_core::{AppPaths, Source, paths};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::Manager;
const PACKAGE: &str = "@codex-pet/deepseek-integration";
#[derive(Default)]
pub struct IntegrationLock(Mutex<()>);
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Remove,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Level {
    Verified,
    Unsupported,
    Unverified,
}
#[derive(Serialize)]
struct Capability {
    name: &'static str,
    status: Level,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    source: Source,
    host: &'static str,
    installation_id: Option<String>,
    version: Option<String>,
    installed: bool,
    capabilities: Vec<Capability>,
    problem: Option<String>,
    target: PathBuf,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    changed: bool,
    backup_path: Option<PathBuf>,
    owned_entry_ids: Vec<String>,
    warnings: Vec<String>,
    bundle_path: Option<PathBuf>,
}
fn failure(e: impl std::fmt::Display) -> CommandError {
    CommandError::new("integration", e)
}
struct Locations {
    paths: AppPaths,
    config: PathBuf,
    profile: PathBuf,
    bridge: PathBuf,
    template: PathBuf,
}

fn user_home() -> Result<PathBuf> {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| failure("无法确定用户目录"))
}
impl Locations {
    fn new(app: &tauri::AppHandle) -> Result<Self> {
        let home = user_home()?;
        let profile_home = |name: &str, default: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(default))
        };
        let config = profile_home("CODEX_HOME", ".codex").join("hooks.json");
        let profile = profile_home("DSH_HOME", ".dsh").join("profiles/desktop");
        if !config.is_absolute() || !profile.is_absolute() {
            return Err(failure("来源配置根目录必须是绝对路径"));
        }
        let exe = env::current_exe().map_err(failure)?;
        let bridge = exe.with_file_name(if cfg!(windows) {
            "pet-bridge.exe"
        } else {
            "pet-bridge"
        });
        let template = app
            .path()
            .resource_dir()
            .map(|p| p.join("harness-bundle"))
            .unwrap_or_else(|_| exe.with_file_name("harness-bundle"));
        #[cfg(debug_assertions)]
        let template = if template.is_dir() {
            template
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../integrations/deepseek-harness/prepared/harness-bundle")
        };
        Ok(Self {
            paths: notification_paths()?,
            config,
            profile,
            bridge,
            template,
        })
    }
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(failure(e)),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(failure)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(failure("配置文件过大，请到来源应用检查"));
    }
    Ok(Some(bytes))
}
fn read_json(path: &Path) -> Result<Option<Value>> {
    read_optional(path)?
        .map(|b| serde_json::from_slice(&b).map_err(failure))
        .transpose()
}
fn version(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut data = String::new();
    child
        .stdout
        .take()?
        .take(256)
        .read_to_string(&mut data)
        .ok()?;
    let v = data.trim().strip_prefix("codex-cli ")?;
    if v.len() > 80
        || !v.starts_with(|c: char| c.is_ascii_digit())
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
    {
        return None;
    }
    Some(v.into())
}
fn codex_versions() -> [Option<String>; 2] {
    let cli = env::var_os("PATH")
        .and_then(|p| {
            env::split_paths(&p)
                .filter(|d| d.is_absolute())
                .map(|d| d.join(if cfg!(windows) { "codex.exe" } else { "codex" }))
                .find(|p| p.is_file())
        })
        .and_then(|p| version(&p));
    #[cfg(target_os = "macos")]
    let desktop = version(Path::new(
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex",
    ));
    #[cfg(not(target_os = "macos"))]
    let desktop = None;
    [cli, desktop]
}
fn caps(items: &[(&'static str, Level)]) -> Vec<Capability> {
    items
        .iter()
        .map(|&(name, status)| Capability { name, status })
        .collect()
}
fn codex_status(loc: &Locations, [cli, desktop]: [Option<String>; 2]) -> Vec<IntegrationStatus> {
    let evidence = read_optional(&loc.config).and_then(|bytes| {
        let bytes = bytes.unwrap_or_default();
        let (id, mut rows) = inspect_hook_entries(&bytes).map_err(failure)?;
        let complete = if id.is_some() {
            let mut expected = plan_hook_install(&bytes, &loc.bridge)
                .map_err(failure)?
                .owned_entry_ids;
            expected.sort();
            rows.sort();
            rows == expected
        } else {
            false
        };
        Ok((id, rows.len(), complete))
    });
    let (id, count, complete, problem) = match evidence {
        Ok((id, count, complete)) => (id, count, complete, None),
        Err(e) => (None, 0, false, Some(e.message)),
    };
    [("Codex CLI", cli), ("Codex 桌面", desktop)]
        .into_iter()
        .map(|(host, version)| {
            let historical = host == "Codex CLI" && version.as_deref() == Some("0.145.0");
            let start = if cfg!(all(target_os = "macos", target_arch = "aarch64")) && historical {
                Level::Verified
            } else {
                Level::Unverified
            };
            let problem = problem.clone().or_else(|| {
                let mut notes = Vec::new();
                if !loc.config.parent().is_some_and(Path::is_dir) {
                    notes.push("未发现 Codex 配置目录。");
                }
                if version.is_none() {
                    notes.push("未识别宿主版本。");
                }
                if !loc.bridge.is_file() {
                    notes.push("缺少配套 pet-bridge。");
                }
                if count > 0 && !complete {
                    notes.push("配置中只有部分自有条目，请检查后重新安装。");
                }
                if historical && cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                    notes.push("回合开始的验证来自 macOS 27.0.1 arm64 上的隔离 CLI 样本。");
                }
                notes.push("配置由 CLI 和桌面共用，信任须在 Codex 的 Hooks 页面确认。");
                Some(notes.join(""))
            });
            IntegrationStatus {
                source: Source::Codex,
                host,
                installation_id: id.clone(),
                installed: complete,
                target: loc.config.clone(),
                problem,
                capabilities: caps(&[
                    ("回合开始", start),
                    ("回合结束", Level::Unverified),
                    (
                        "根回合中断",
                        if historical {
                            Level::Unsupported
                        } else {
                            Level::Unverified
                        },
                    ),
                    ("子代理开始与结束", Level::Unverified),
                    ("精确审批、提问与解除", Level::Unsupported),
                    ("精确会话跳转", Level::Unverified),
                ]),
                version,
            }
        })
        .collect()
}
fn installation_id(row: &Value) -> Option<String> {
    row.get(0)?
        .get("insert")?
        .as_array()?
        .iter()
        .find(|r| r["id"] == "desktop-pet-notify" && r["name"] == PACKAGE)
        .and_then(|r| r["config"]["installationId"].as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(|id| id.to_string())
}
fn harness_status(loc: &Locations) -> Result<IntegrationStatus> {
    let profile = read_json(&loc.profile.join("package.json"))?.unwrap_or(Value::Null);
    let selected = profile
        .pointer("/dsh/profile/bundles")
        .and_then(Value::as_array)
        .is_some_and(|layers| layers.iter().any(|v| v == PACKAGE));
    let package = loc.profile.join("node_modules").join(PACKAGE);
    let metadata = read_json(&package.join("package.json"))?.unwrap_or(Value::Null);
    let id = if metadata["name"] == PACKAGE
        && metadata
            .pointer("/dsh/bundle/patch")
            .and_then(Value::as_str)
            == Some("./cordis.patch.json")
    {
        read_json(&package.join("cordis.patch.json"))?.and_then(|row| installation_id(&row))
    } else {
        None
    };
    // 这里能证实 profile 中选中的真实配置包；运行期启用和 Fiber 状态须在宿主检查。
    let installed = selected && id.is_some();
    let hint = if installed {
        "已在桌面 profile 选择配置包。运行状态请在宿主插件页核对。"
    } else if !profile.is_null() && profile["dependencies"].get(PACKAGE).is_some() {
        "同名依赖已存在但未确认有效配置层。迁移旧 plain 库前，请在宿主插件页保留安装 ID 并移除自有旧层。"
    } else if profile.is_null() {
        "未发现 Harness 桌面 profile。"
    } else {
        "请在宿主插件页安装并启用配置包。"
    };
    let sdk_version = read_json(
        &loc.profile
            .join("node_modules/@deepseek-ai/dsh-session/package.json"),
    )?
    .and_then(|v| v["version"].as_str().map(str::to_owned));
    let sdk_verified = sdk_version.as_deref() == Some("0.2.2-rc.1")
        && cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let sdk = if sdk_verified {
        Level::Verified
    } else {
        Level::Unverified
    };
    Ok(IntegrationStatus {
        source: Source::Deepseek,
        host: "DeepSeek Harness · 会话 SDK",
        installation_id: id,
        version: sdk_version,
        installed,
        target: loc.profile.clone(),
        capabilities: caps(&[
            ("SDK 连续观察审批与精确解除", sdk),
            ("SDK 连续观察提问与精确解除", sdk),
            ("SDK 定时提问跨回合解除", sdk),
            ("当前宿主重载恢复（需配套补丁）", Level::Unverified),
            ("原生精确跳转（需配套补丁）", Level::Unverified),
        ]),
        problem: Some(format!(
            "{hint} 对照版本 0.2.2-rc.1 在 macOS 有 SDK 至桥接器的待处理与解除实测。"
        )),
    })
}
fn codex_apply(loc: &Locations, action: Action) -> Result<InstallReport> {
    let bytes = read_optional(&loc.config)?.unwrap_or_default();
    let (_id, owned) = inspect_hook_entries(&bytes).map_err(failure)?;
    let patch = match action {
        Action::Install => {
            if !loc.bridge.is_file() {
                return Err(failure("缺少配套 pet-bridge，请修复桌宠安装"));
            }
            plan_hook_install(&bytes, &loc.bridge).map_err(failure)?
        }
        Action::Remove => {
            if owned.is_empty() {
                return Ok(InstallReport::default());
            }
            // 仅撤销安装清单和当前精确自有条目的交集，保护第三方修改。
            let parent = loc
                .config
                .parent()
                .ok_or_else(|| failure("配置路径无父目录"))?;
            let mut verified = Vec::new();
            for entry in fs::read_dir(parent).map_err(failure)? {
                let entry = entry.map_err(failure)?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with(".hooks.json.pet-bridge.")
                    || !name.ends_with(".manifest.json")
                    || !entry.file_type().map_err(failure)?.is_file()
                {
                    continue;
                }
                let Some(data) = read_optional(&entry.path())? else {
                    continue;
                };
                if let Ok(receipt) = serde_json::from_slice::<HookPatch>(&data) {
                    for hash in receipt.owned_entry_ids {
                        if owned.contains(&hash) && !verified.contains(&hash) {
                            verified.push(hash);
                        }
                    }
                }
            }
            if verified.len() != owned.len() {
                return Err(failure("自有配置缺少对应安装清单，请检查备份与来源设置"));
            }
            let receipt = HookPatch {
                before_hash: String::new(),
                after_bytes: vec![],
                owned_entry_ids: verified,
            };
            plan_hook_remove(&bytes, &receipt).map_err(failure)?
        }
    };
    let backup = apply_hook_patch_with_backup(&loc.config, &patch).map_err(failure)?;
    Ok(InstallReport {
        changed: backup.is_some(),
        backup_path: backup,
        owned_entry_ids: patch.owned_entry_ids,
        warnings: if matches!(action, Action::Install) {
            vec!["请在 Codex 的 Hooks 页面审阅并信任自有条目。".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    })
}

fn harness_apply(loc: &Locations, action: Action) -> Result<InstallReport> {
    if matches!(action, Action::Remove) {
        return Ok(InstallReport{warnings:vec!["在 Harness 桌面的插件页停用并移除 @codex-pet/deepseek-integration，然后刷新接入状态。".into()],..Default::default()});
    }
    if !loc.bridge.is_file() || !loc.template.join("dist/index.js").is_file() {
        return Err(failure(
            "缺少配套的 pet-bridge 或 Harness 配置包资源，请修复桌宠安装",
        ));
    }
    let status = harness_status(loc)?;
    let root = loc.paths.root.join("integrations/harness");
    paths::prepare_directory(&root)?;
    let id_file = root.join("installation-id");
    let saved = read_optional(&id_file)?
        .map(|bytes| String::from_utf8(bytes).map_err(failure))
        .transpose()?;
    if saved.is_none()
        && status.installation_id.is_none()
        && read_json(&loc.profile.join("package.json"))?
            .is_some_and(|p| p["dependencies"].get(PACKAGE).is_some())
    {
        return Err(failure(
            "现有同名依赖的安装 ID 未确认，请在宿主核对原 ID 并用正式生成器的 --installation-id 选项迁移",
        ));
    }
    let id = match (status.installation_id, saved) {
        (Some(actual), Some(previous)) if actual != previous => {
            return Err(failure(
                "桌面 profile 与本机保存的安装 ID 冲突，请先在宿主核对",
            ));
        }
        (Some(actual), _) | (_, Some(actual)) => uuid::Uuid::parse_str(&actual).map_err(failure)?,
        _ => uuid::Uuid::new_v4(),
    }
    .to_string();
    if !id_file.exists() {
        paths::atomic_write(&id_file, ".identity-", id.as_bytes())?;
        paths::sync_directory(&root)?;
    }
    let dir = root.join(format!("bundle-{}", uuid::Uuid::new_v4()));
    paths::prepare_directory(&dir)?;
    let result = (|| {
        for file in ["package.json", "README.md", "compatibility.json"] {
            fs::copy(loc.template.join(file), dir.join(file)).map_err(failure)?;
        }
        let dist = dir.join("dist");
        paths::prepare_directory(&dist)?;
        for entry in fs::read_dir(loc.template.join("dist")).map_err(failure)? {
            let entry = entry.map_err(failure)?;
            if !entry.file_type().map_err(failure)?.is_file() {
                return Err(failure("配置包资源结构无效"));
            }
            fs::copy(entry.path(), dist.join(entry.file_name())).map_err(failure)?;
        }
        let mut patch = read_json(&loc.template.join("cordis.patch.json"))?
            .ok_or_else(|| failure("配置包缺少 patch"))?;
        if installation_id(&patch).is_none() {
            return Err(failure("配置包缺少已知的正式自有条目"));
        }
        patch[0]["insert"][0]["config"] =
            serde_json::json!({"installationId":id,"bridgePath":loc.bridge});
        paths::atomic_write(
            &dir.join("cordis.patch.json"),
            ".patch-",
            &serde_json::to_vec_pretty(&patch).map_err(failure)?,
        )?;
        paths::sync_directory(&dir)?;
        Ok(InstallReport{bundle_path:Some(dir.clone()),warnings:vec!["配置包已生成。请在 Harness 桌面插件页添加下面的本地目录，核对 bridgePath 和 installationId 后启用。已有 bundle 请核对原启用状态。".into()],..Default::default()})
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}

#[tauri::command]
pub async fn integration_status(app: tauri::AppHandle) -> Result<Vec<IntegrationStatus>> {
    tauri::async_runtime::spawn_blocking(move || {
        let loc = Locations::new(&app)?;
        let mut statuses = codex_status(&loc, codex_versions());
        statuses.push(
            harness_status(&loc).unwrap_or_else(|err| IntegrationStatus {
                source: Source::Deepseek,
                host: "DeepSeek Harness",
                target: loc.profile.clone(),
                installation_id: None,
                version: None,
                installed: false,
                capabilities: vec![],
                problem: Some(err.message),
            }),
        );
        Ok(statuses)
    })
    .await
    .map_err(failure)?
}
#[tauri::command]
pub async fn apply_integration(
    app: tauri::AppHandle,
    source: Source,
    action: Action,
) -> Result<InstallReport> {
    tauri::async_runtime::spawn_blocking(move || {
        let gate = app.state::<IntegrationLock>();
        let _guard = gate.0.lock().unwrap();
        let loc = Locations::new(&app)?;
        match source {
            Source::Codex => codex_apply(&loc, action),
            Source::Deepseek => harness_apply(&loc, action),
        }
    })
    .await
    .map_err(failure)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn locations(tmp: &tempfile::TempDir) -> Locations {
        let root = tmp.path().canonicalize().unwrap();
        let config = root.join("codex/hooks.json");
        fs::create_dir(config.parent().unwrap()).unwrap();
        let bridge = root.join("真实路径 pet-bridge");
        fs::write(&bridge, b"test").unwrap();
        Locations {
            paths: AppPaths::under(&root.join("app")),
            config,
            bridge,
            profile: root.join("profiles/desktop"),
            template: root.join("template"),
        }
    }
    #[test]
    fn codex_counts_distinct_current_entries_and_keeps_foreign_hooks_on_remove() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        let foreign = json!({"hooks":[{"command":"user command","type":"command"}]});
        fs::write(
            &loc.config,
            serde_json::to_vec(&json!({"hooks":{"Stop":[foreign]}})).unwrap(),
        )
        .unwrap();
        let report = codex_apply(&loc, Action::Install).unwrap();
        assert!(report.changed && report.backup_path.unwrap().is_file());
        assert_eq!(report.owned_entry_ids.len(), 5);
        assert!(!codex_apply(&loc, Action::Install).unwrap().changed);
        let original = fs::read(&loc.config).unwrap();
        let mut edited: Value = serde_json::from_slice(&original).unwrap();
        edited["hooks"]["Interrupt"] = json!([]);
        let duplicate = edited["hooks"]["Stop"][1].clone();
        edited["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        fs::write(&loc.config, serde_json::to_vec(&edited).unwrap()).unwrap();
        assert!(
            !codex_status(&loc, [None, None])
                .iter()
                .any(|host| host.installed)
        );
        fs::write(&loc.config, original).unwrap();
        let r = codex_apply(&loc, Action::Remove).unwrap();
        assert!(r.changed);
        let config = read_json(&loc.config).unwrap().unwrap();
        assert_eq!(config["hooks"]["Stop"], json!([foreign]));
    }
    #[test]
    fn plain_harness_has_no_verified_install_and_does_not_issue_new_identity() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        fs::create_dir_all(&loc.profile).unwrap();
        let original =
            serde_json::to_vec(&json!({"dependencies":{PACKAGE:"file:plain-old"}})).unwrap();
        fs::write(loc.profile.join("package.json"), &original).unwrap();
        fs::create_dir_all(loc.template.join("dist")).unwrap();
        fs::write(loc.template.join("dist/index.js"), "").unwrap();
        assert!(!harness_status(&loc).unwrap().installed);
        assert!(harness_apply(&loc, Action::Install).is_err());
        assert!(
            !loc.paths
                .root
                .join("integrations/harness/installation-id")
                .exists()
        );
        assert_eq!(
            fs::read(loc.profile.join("package.json")).unwrap(),
            original
        );
    }
}
