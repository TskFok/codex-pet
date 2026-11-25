use crate::{
    commands::{CommandError, Result},
    navigation::{self, Registration, harness, system},
    notification_paths,
};
use pet_bridge::{
    antigravity_install::{
        apply_antigravity_patch_with_backup, inspect_antigravity_hooks, plan_antigravity_install,
        plan_antigravity_remove,
    },
    hook_install::{
        HookPatch, apply_hook_patch_with_backup, inspect_hook_entries, plan_hook_install,
        plan_hook_remove,
    },
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
    antigravity_config: PathBuf,
    dsh_home: PathBuf,
    profiles_dir: PathBuf,
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
        let antigravity_config = env::var_os("ANTIGRAVITY_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".gemini").join("config"))
            .join("hooks.json");
        let dsh_home = profile_home("DSH_HOME", ".dsh");
        let profiles_dir = dsh_home.join("profiles");
        if !config.is_absolute() || !antigravity_config.is_absolute() || !dsh_home.is_absolute() {
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
            antigravity_config,
            dsh_home,
            profiles_dir,
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
fn parse_cli_version(raw: &str) -> Option<String> {
    let token = raw.lines().next()?.split_whitespace().find_map(|part| {
        let trimmed = part.trim_start_matches('v');
        (trimmed.len() <= 80
            && trimmed.starts_with(|c: char| c.is_ascii_digit())
            && trimmed.contains('.')
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)))
        .then_some(trimmed)
    })?;
    Some(token.into())
}
fn antigravity_version() -> Option<String> {
    if let Some(cli) = antigravity_cli_version() {
        return Some(cli);
    }
    #[cfg(target_os = "macos")]
    {
        let desktop = system::desktop(Source::Antigravity)?;
        let plist = desktop.path.join("Contents/Info.plist");
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
            .arg(&plist)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let ver = parse_cli_version(&String::from_utf8_lossy(&output.stdout))?;
        let app_name = desktop
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Antigravity");
        Some(format!("{app_name} {ver}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
fn antigravity_cli_version() -> Option<String> {
    let exe = env::var_os("PATH").and_then(|p| {
        env::split_paths(&p)
            .filter(|d| d.is_absolute())
            .map(|d| d.join(if cfg!(windows) { "agy.exe" } else { "agy" }))
            .find(|p| p.is_file())
    })?;
    let mut cmd = Command::new(&exe);
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
    parse_cli_version(&data)
}
fn caps(items: &[(&'static str, Level)]) -> Vec<Capability> {
    items
        .iter()
        .map(|&(name, status)| Capability { name, status })
        .collect()
}
fn codex_evidence(loc: &Locations) -> Result<(Option<String>, usize, bool)> {
    read_optional(&loc.config).and_then(|bytes| {
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
    })
}
fn codex_status(loc: &Locations, [cli, desktop]: [Option<String>; 2]) -> Vec<IntegrationStatus> {
    let evidence = codex_evidence(loc);
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
fn installation_entry(row: &Value) -> Option<&Value> {
    row.get(0)?
        .get("insert")?
        .as_array()?
        .iter()
        .find(|r| r["id"] == "desktop-pet-notify" && r["name"] == PACKAGE)
}
fn installation_id(row: &Value) -> Option<String> {
    installation_entry(row)?
        .get("config")?
        .get("installationId")?
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(|id| id.to_string())
}
const WEB_APP_BUNDLE: &str = "@deepseek-ai/dsh-web-app";

struct HarnessProfile {
    name: String,
    dir: PathBuf,
    installation_id: Option<String>,
    bridge_path: Option<PathBuf>,
    package_dir: PathBuf,
    manifest: Value,
    has_web_app: bool,
    has_package_dep: bool,
}

fn harness_profile_dirs(profiles_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for preferred in ["desktop", "web"] {
        let candidate = profiles_dir.join(preferred);
        if candidate.is_dir() {
            dirs.push(candidate);
        }
    }
    let entries = match fs::read_dir(profiles_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(dirs),
        Err(e) => return Err(failure(e)),
    };
    let mut extras = Vec::new();
    for entry in entries {
        let entry = entry.map_err(failure)?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if matches!(name.as_str(), "desktop" | "web" | "node_modules") || name.starts_with('.') {
            continue;
        }
        extras.push((name, path));
    }
    extras.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.extend(extras.into_iter().map(|(_, path)| path));
    Ok(dirs)
}

fn inspect_harness_profile(dir: &Path) -> Result<HarnessProfile> {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("desktop")
        .to_owned();
    let manifest = read_json(&dir.join("package.json"))?.unwrap_or(Value::Null);
    let bundles = manifest
        .pointer("/dsh/profile/bundles")
        .and_then(Value::as_array);
    let selected = bundles.is_some_and(|layers| layers.iter().any(|v| v == PACKAGE));
    let has_web_app = bundles.is_some_and(|layers| layers.iter().any(|v| v == WEB_APP_BUNDLE));
    let has_package_dep = !manifest.is_null() && manifest["dependencies"].get(PACKAGE).is_some();
    let package_dir = dir.join("node_modules").join(PACKAGE);
    let metadata = read_json(&package_dir.join("package.json"))?.unwrap_or(Value::Null);
    let (installation_id, bridge_path) = if selected
        && metadata["name"] == PACKAGE
        && metadata
            .pointer("/dsh/bundle/patch")
            .and_then(Value::as_str)
            == Some("./cordis.patch.json")
    {
        match read_json(&package_dir.join("cordis.patch.json"))? {
            Some(row) => {
                let id = installation_id(&row);
                let bridge = installation_entry(&row)
                    .and_then(|entry| entry["config"]["bridgePath"].as_str())
                    .map(PathBuf::from);
                (id, bridge)
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    Ok(HarnessProfile {
        name,
        dir: dir.to_path_buf(),
        installation_id,
        bridge_path,
        package_dir,
        manifest,
        has_web_app,
        has_package_dep,
    })
}

fn harness_registrations(loc: &Locations) -> Result<Vec<HarnessProfile>> {
    harness_profile_dirs(&loc.profiles_dir)?
        .into_iter()
        .map(|dir| inspect_harness_profile(&dir))
        .collect()
}

fn harness_primary_profile(loc: &Locations) -> Result<HarnessProfile> {
    let profiles = harness_registrations(loc)?;
    let chosen = profiles
        .iter()
        .position(|p| p.installation_id.is_some())
        .or_else(|| profiles.iter().position(|p| p.has_package_dep))
        .or_else(|| profiles.iter().position(|p| !p.manifest.is_null()));
    if let Some(index) = chosen {
        return Ok(profiles.into_iter().nth(index).unwrap());
    }
    let dir = loc.profiles_dir.join("desktop");
    Ok(HarnessProfile {
        name: "desktop".into(),
        package_dir: dir.join("node_modules").join(PACKAGE),
        dir,
        installation_id: None,
        bridge_path: None,
        manifest: Value::Null,
        has_web_app: false,
        has_package_dep: false,
    })
}

fn harness_sdk_version(loc: &Locations, profile_dir: &Path) -> Result<Option<String>> {
    for root in [profile_dir, &loc.profiles_dir] {
        let version = read_json(&root.join("node_modules/@deepseek-ai/dsh-session/package.json"))?
            .and_then(|v| v["version"].as_str().map(str::to_owned));
        if version.is_some() {
            return Ok(version);
        }
    }
    Ok(None)
}

fn harness_status(loc: &Locations) -> Result<IntegrationStatus> {
    let profile = harness_primary_profile(loc)?;
    // 这里能证实 profile 中选中的真实配置包；运行期启用和 Fiber 状态须在宿主检查。
    let installed = profile.installation_id.is_some();
    let bridge_missing = installed
        && !profile
            .bridge_path
            .as_deref()
            .is_some_and(|path| path.is_absolute() && path.is_file());
    let label = if profile.name == "desktop" {
        "桌面 profile".to_owned()
    } else {
        format!("{} profile", profile.name)
    };
    let hint = if installed && bridge_missing {
        format!(
            "已在 {label} 选择配置包，但其中配置的 bridgePath 在磁盘上不存在，请重新生成并在宿主更新。"
        )
    } else if installed {
        format!("已在 {label} 选择配置包。运行状态请在宿主插件页核对。")
    } else if profile.has_package_dep {
        "同名依赖已存在但未确认有效配置层。迁移旧 plain 库前，请在宿主插件页保留安装 ID 并移除自有旧层。"
            .into()
    } else if profile.manifest.is_null() {
        "未发现 Harness profile。".into()
    } else {
        "请在宿主插件页安装并启用配置包。".into()
    };
    let sdk_version = harness_sdk_version(loc, &profile.dir)?;
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
        installation_id: profile.installation_id,
        version: sdk_version,
        installed,
        target: profile.dir,
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

fn antigravity_evidence(loc: &Locations) -> Result<(Option<String>, usize, bool)> {
    read_optional(&loc.antigravity_config).and_then(|bytes| {
        let bytes = bytes.unwrap_or_default();
        let (id, mut rows) = inspect_antigravity_hooks(&bytes).map_err(failure)?;
        let complete = if id.is_some() {
            let mut expected = plan_antigravity_install(&bytes, &loc.bridge)
                .map_err(failure)?
                .owned_entry_ids;
            expected.sort();
            rows.sort();
            rows == expected
        } else {
            false
        };
        Ok((id, rows.len(), complete))
    })
}
fn antigravity_status(loc: &Locations, version: Option<String>) -> IntegrationStatus {
    let (id, count, complete, problem) = match antigravity_evidence(loc) {
        Ok((id, count, complete)) => (id, count, complete, None),
        Err(e) => (None, 0, false, Some(e.message)),
    };
    let problem = problem.or_else(|| {
        let mut notes = Vec::new();
        if !loc.antigravity_config.parent().is_some_and(Path::is_dir) {
            notes.push("尚未发现 Antigravity 全局配置目录，安装时将自动创建。");
        }
        if version.is_none() {
            notes.push("未识别到 Antigravity / Gemini 宿主或 agy CLI 版本。");
        }
        if !loc.bridge.is_file() {
            notes.push("缺少配套 pet-bridge。");
        }
        if count > 0 && !complete {
            notes.push("配置中只有部分或旧路径自有条目，请检查后重新安装。");
        }
        notes.push("配置由 Gemini 桌面端 Antigravity、Antigravity 2.0、IDE 与 CLI 共用，仅订阅 PreInvocation 与 Stop。");
        Some(notes.join(""))
    });
    IntegrationStatus {
        source: Source::Antigravity,
        host: "Antigravity（桌面 / IDE / CLI）",
        installation_id: id,
        installed: complete,
        target: loc.antigravity_config.clone(),
        problem,
        capabilities: caps(&[
            ("回合开始", Level::Unverified),
            ("回合结束 / 中断 / 失败", Level::Unverified),
            ("项目工作区识别", Level::Unverified),
            ("精确审批、提问与解除", Level::Unsupported),
            ("精确会话跳转（支持打开应用与复制 ID）", Level::Unsupported),
        ]),
        version,
    }
}
fn antigravity_apply(loc: &Locations, action: Action) -> Result<InstallReport> {
    let bytes = read_optional(&loc.antigravity_config)?.unwrap_or_default();
    let (_id, owned) = inspect_antigravity_hooks(&bytes).map_err(failure)?;
    let patch = match action {
        Action::Install => {
            if !loc.bridge.is_file() {
                return Err(failure("缺少配套 pet-bridge，请修复桌宠安装"));
            }
            if let Some(parent) = loc.antigravity_config.parent() {
                match fs::symlink_metadata(parent) {
                    Ok(meta) => {
                        if meta.file_type().is_symlink() || !meta.is_dir() {
                            return Err(failure("Antigravity 配置目录无效"));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        fs::create_dir_all(parent).map_err(failure)?;
                    }
                    Err(e) => return Err(failure(e)),
                }
            }
            plan_antigravity_install(&bytes, &loc.bridge).map_err(failure)?
        }
        Action::Remove => {
            if owned.is_empty() {
                return Ok(InstallReport::default());
            }
            let parent = loc
                .antigravity_config
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
            plan_antigravity_remove(&bytes, &receipt).map_err(failure)?
        }
    };
    let backup =
        apply_antigravity_patch_with_backup(&loc.antigravity_config, &patch).map_err(failure)?;
    Ok(InstallReport {
        changed: backup.is_some(),
        backup_path: backup,
        owned_entry_ids: patch.owned_entry_ids,
        warnings: if matches!(action, Action::Install) {
            vec!["已写入 Antigravity 全局 Hooks 配置，将在新会话中自动生效。".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    })
}

fn harness_apply(loc: &Locations, action: Action) -> Result<InstallReport> {
    if matches!(action, Action::Remove) {
        return Ok(InstallReport {
            warnings: vec![
                "在 Harness 插件页停用并移除 @codex-pet/deepseek-integration，然后刷新接入状态。"
                    .into(),
            ],
            ..Default::default()
        });
    }
    if !loc.bridge.is_file() || !loc.template.join("dist/index.js").is_file() {
        return Err(failure(
            "缺少配套的 pet-bridge 或 Harness 配置包资源，请修复桌宠安装",
        ));
    }
    let profiles = harness_registrations(loc)?;
    let mut distinct_ids: Vec<String> = Vec::new();
    for id in profiles
        .iter()
        .filter_map(|profile| profile.installation_id.as_ref())
    {
        if !distinct_ids.contains(id) {
            distinct_ids.push(id.clone());
        }
    }
    if distinct_ids.len() > 1 {
        return Err(failure(
            "多个 Harness profile 中的安装 ID 冲突，请先在宿主核对",
        ));
    }
    let actual_id = distinct_ids.into_iter().next();
    let root = loc.paths.root.join("integrations/harness");
    paths::prepare_directory(&root)?;
    let id_file = root.join("installation-id");
    let saved = read_optional(&id_file)?
        .map(|bytes| String::from_utf8(bytes).map_err(failure))
        .transpose()?;
    if actual_id.is_none() && profiles.iter().any(|profile| profile.has_package_dep) {
        return Err(failure(
            "现有同名依赖的安装 ID 未确认，请在宿主核对原 ID 并用正式生成器的 --installation-id 选项迁移",
        ));
    }
    let id = match (actual_id, saved) {
        (Some(actual), Some(previous)) if actual != previous => {
            return Err(failure(
                "Harness profile 与本机保存的安装 ID 冲突，请先在宿主核对",
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
        Ok(InstallReport{bundle_path:Some(dir.clone()),warnings:vec!["配置包已生成。请在 Harness 插件页添加下面的本地目录，核对 bridgePath 和 installationId 后启用。已有 bundle 请核对原启用状态。".into()],..Default::default()})
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}

fn navigation_target_for_locations(
    loc: &Locations,
    source: Source,
    expected_id: &str,
) -> Result<Option<navigation::Target>> {
    match source {
        Source::Codex => {
            let (id, _, complete) = codex_evidence(loc)?;
            Ok(id
                .filter(|id| complete && id == expected_id)
                .map(|installation_id| {
                    navigation::Target::Desktop(Registration {
                        desktop: system::desktop(source),
                        installation_id,
                        source,
                    })
                }))
        }
        Source::Deepseek => {
            let Some(profile) = harness_registrations(loc)?
                .into_iter()
                .find(|p| p.installation_id.as_deref() == Some(expected_id))
            else {
                return Ok(None);
            };
            let installation_id = profile.installation_id.unwrap();
            if profile.name != "desktop" && profile.has_web_app {
                if let Some(web) =
                    harness::web_target(&profile.dir, Some(&loc.dsh_home), &profile.package_dir)
                {
                    return Ok(Some(navigation::Target::Web {
                        source,
                        installation_id,
                        target: Some(web),
                    }));
                }
                if let Some(desktop) = system::desktop(source) {
                    return Ok(Some(navigation::Target::Desktop(Registration {
                        desktop: Some(desktop),
                        installation_id,
                        source,
                    })));
                }
                return Ok(Some(navigation::Target::Web {
                    source,
                    installation_id,
                    target: None,
                }));
            }
            Ok(Some(navigation::Target::Desktop(Registration {
                desktop: system::desktop(source),
                installation_id,
                source,
            })))
        }
        Source::Antigravity => {
            let (id, _, complete) = antigravity_evidence(loc)?;
            Ok(id
                .filter(|id| complete && id == expected_id)
                .map(|installation_id| {
                    navigation::Target::Desktop(Registration {
                        desktop: system::desktop(source),
                        installation_id,
                        source,
                    })
                }))
        }
    }
}

/// 已登记的 Hooks 或已由 Harness profile 选择的配置包才提供安装身份。
pub(crate) fn navigation_registration(
    app: &tauri::AppHandle,
    source: Source,
    expected_id: &str,
) -> Result<Option<navigation::Target>> {
    let lock = app.state::<IntegrationLock>();
    let _guard = lock.0.lock().unwrap();
    let loc = Locations::new(app)?;
    navigation_target_for_locations(&loc, source, expected_id)
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
                target: loc.profiles_dir.join("desktop"),
                installation_id: None,
                version: None,
                installed: false,
                capabilities: vec![],
                problem: Some(err.message),
            }),
        );
        statuses.push(antigravity_status(&loc, antigravity_version()));
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
            Source::Antigravity => antigravity_apply(&loc, action),
        }
    })
    .await
    .map_err(failure)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::{Ipv4Addr, TcpListener};

    fn locations(tmp: &tempfile::TempDir) -> Locations {
        let root = tmp.path().canonicalize().unwrap();
        let config = root.join("codex/hooks.json");
        fs::create_dir(config.parent().unwrap()).unwrap();
        let antigravity_config = root.join("gemini/config/hooks.json");
        let bridge = root.join("真实路径 pet-bridge");
        fs::write(&bridge, b"test").unwrap();
        let dsh_home = root.join("dsh");
        Locations {
            paths: AppPaths::under(&root.join("app")),
            config,
            antigravity_config,
            bridge,
            profiles_dir: dsh_home.join("profiles"),
            dsh_home,
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
        let desktop_profile = loc.profiles_dir.join("desktop");
        fs::create_dir_all(&desktop_profile).unwrap();
        let original =
            serde_json::to_vec(&json!({"dependencies":{PACKAGE:"file:plain-old"}})).unwrap();
        fs::write(desktop_profile.join("package.json"), &original).unwrap();
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
        let saved = loc.paths.root.join("integrations/harness/installation-id");
        fs::write(&saved, "12345678-1234-4234-8234-123456789012").unwrap();
        let error = harness_apply(&loc, Action::Install)
            .err()
            .expect("旧 plain 依赖身份不可确认");
        assert!(
            error.message.contains("现有同名依赖的安装 ID 未确认"),
            "{}",
            error.message
        );
        assert_eq!(
            fs::read_to_string(saved).unwrap(),
            "12345678-1234-4234-8234-123456789012"
        );
        assert_eq!(
            fs::read(desktop_profile.join("package.json")).unwrap(),
            original
        );
    }
    fn harness_bundle_fixture_in(
        loc: &Locations,
        profile_name: &str,
        id: &str,
        layers: &[&str],
    ) -> Value {
        let profile = loc.profiles_dir.join(profile_name);
        let host_bundle = profile.join("node_modules").join(PACKAGE);
        fs::create_dir_all(&host_bundle).unwrap();
        let manifest = json!({"name":PACKAGE,"dsh":{"bundle":{"patch":"./cordis.patch.json"}}});
        let row = json!([{"insert":[{"name":PACKAGE,"id":"desktop-pet-notify","config":{"installationId":id,"bridgePath":loc.bridge}}]}]);
        fs::write(host_bundle.join("package.json"), manifest.to_string()).unwrap();
        fs::write(host_bundle.join("cordis.patch.json"), row.to_string()).unwrap();
        fs::write(
            profile.join("package.json"),
            json!({"dependencies":{PACKAGE:"file:bundle"},"dsh":{"profile":{"bundles":layers}}})
                .to_string(),
        )
        .unwrap();
        fs::create_dir_all(loc.template.join("dist")).unwrap();
        for name in ["README.md", "compatibility.json", "dist/index.js"] {
            fs::write(loc.template.join(name), "").unwrap();
        }
        fs::write(loc.template.join("package.json"), manifest.to_string()).unwrap();
        fs::write(loc.template.join("cordis.patch.json"), row.to_string()).unwrap();
        row
    }

    fn harness_bundle_fixture(loc: &Locations, id: &str, layers: &[&str]) -> Value {
        harness_bundle_fixture_in(loc, "desktop", id, layers)
    }

    #[test]
    fn unselected_replacement_bundle_cannot_supply_the_host_identity() {
        let bundle_id = "12345678-1234-4234-8234-123456789012";
        for cached in [None, Some("abcdef12-1234-4234-8234-123456789012")] {
            let temp = tempfile::tempdir().unwrap();
            let loc = locations(&temp);
            harness_bundle_fixture(&loc, bundle_id, &["base", "web"]);
            let root = loc.paths.root.join("integrations/harness");
            paths::prepare_directory(&root).unwrap();
            let identity = root.join("installation-id");
            if let Some(previous) = cached {
                fs::write(&identity, previous).unwrap();
            }
            let error = harness_apply(&loc, Action::Install)
                .err()
                .expect("未选择替换包的 profile 必须先核对原安装 ID");
            assert!(
                error.message.contains("现有同名依赖的安装 ID 未确认"),
                "{}",
                error.message
            );
            let status = harness_status(&loc).unwrap();
            assert!(!status.installed);
            assert!(status.installation_id.is_none());
            assert_eq!(
                read_optional(&identity).unwrap().as_deref(),
                cached.map(str::as_bytes)
            );
            assert_eq!(
                fs::read_dir(&root).unwrap().count(),
                usize::from(cached.is_some())
            );
        }
    }

    #[test]
    fn matching_registered_bundle_retains_identity_on_upgrade() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        let id = "12345678-1234-4234-8234-123456789012";
        let row = harness_bundle_fixture(&loc, id, &[PACKAGE]);
        let host_bundle = loc.profiles_dir.join("desktop/node_modules").join(PACKAGE);
        let root = loc.paths.root.join("integrations/harness");
        paths::prepare_directory(&root).unwrap();
        fs::write(root.join("installation-id"), id).unwrap();
        assert!(harness_status(&loc).unwrap().installed);
        let generated = harness_apply(&loc, Action::Install)
            .unwrap()
            .bundle_path
            .unwrap();
        let patch = read_json(&generated.join("cordis.patch.json"))
            .unwrap()
            .unwrap();
        assert_eq!(installation_id(&patch).as_deref(), Some(id));
        assert_eq!(
            read_json(&host_bundle.join("cordis.patch.json"))
                .unwrap()
                .unwrap(),
            row
        );
    }

    #[test]
    fn selected_bundle_with_missing_bridge_warns_in_status() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        let id = "12345678-1234-4234-8234-123456789012";
        harness_bundle_fixture(&loc, id, &[PACKAGE]);
        fs::remove_file(&loc.bridge).unwrap();
        let status = harness_status(&loc).unwrap();
        assert!(status.installed);
        assert_eq!(status.installation_id.as_deref(), Some(id));
        assert!(
            status
                .problem
                .as_deref()
                .is_some_and(|text| text.contains("bridgePath 在磁盘上不存在"))
        );
    }

    #[test]
    fn web_profile_is_recognized_for_status_upgrade_and_navigation() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        let id = "fa6a6b01-aa37-4670-bfa7-7a6268e65289";
        harness_bundle_fixture_in(
            &loc,
            "web",
            id,
            &["@deepseek-ai/dsh-base", WEB_APP_BUNDLE, PACKAGE],
        );
        // 共享 profiles/node_modules 中的 SDK 版本和目录忽略逻辑
        let hoisted_sdk = loc
            .profiles_dir
            .join("node_modules/@deepseek-ai/dsh-session");
        fs::create_dir_all(&hoisted_sdk).unwrap();
        fs::write(
            hoisted_sdk.join("package.json"),
            json!({"name": "@deepseek-ai/dsh-session", "version": "0.2.3-alpha.2"}).to_string(),
        )
        .unwrap();
        fs::write(
            loc.profiles_dir.join("web/cordis.patch.yml"),
            format!("- id: webserver\n  config:\n    port: {port}\n"),
        )
        .unwrap();
        let client_js = loc
            .profiles_dir
            .join("web/node_modules")
            .join(PACKAGE)
            .join("dist/client.js");
        fs::create_dir_all(client_js.parent().unwrap()).unwrap();
        fs::write(&client_js, "const h = '#codex-pet-session=';").unwrap();

        let status = harness_status(&loc).unwrap();
        assert!(status.installed);
        assert_eq!(status.installation_id.as_deref(), Some(id));
        assert_eq!(status.version.as_deref(), Some("0.2.3-alpha.2"));
        assert_eq!(status.target, loc.profiles_dir.join("web"));

        let target = navigation_target_for_locations(&loc, Source::Deepseek, id)
            .unwrap()
            .expect("web profile 应通过安装身份校验");
        match target {
            navigation::Target::Web {
                source,
                installation_id,
                target,
            } => {
                assert_eq!(source, Source::Deepseek);
                assert_eq!(installation_id, id);
                let web = target.expect("监听中的本地 Web 入口应可用");
                assert_eq!(web.origin, format!("http://127.0.0.1:{port}"));
                assert!(web.supports_session);
            }
            navigation::Target::Desktop(_) => panic!("应优先返回运行中的本地 Web 入口"),
        }

        assert!(
            navigation_target_for_locations(
                &loc,
                Source::Deepseek,
                "11111111-1111-4111-8111-111111111111"
            )
            .unwrap()
            .is_none()
        );
        drop(listener);
    }

    #[test]
    fn conflicting_installation_ids_across_profiles_are_rejected_on_apply() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        harness_bundle_fixture_in(
            &loc,
            "desktop",
            "11111111-1111-4111-8111-111111111111",
            &[PACKAGE],
        );
        harness_bundle_fixture_in(
            &loc,
            "web",
            "22222222-2222-4222-8222-222222222222",
            &[WEB_APP_BUNDLE, PACKAGE],
        );
        let err = harness_apply(&loc, Action::Install)
            .err()
            .expect("不同 profile 的冲突安装 ID 不能静默覆写");
        assert!(err.message.contains("安装 ID 冲突"), "{}", err.message);
    }

    #[test]
    fn antigravity_install_preserves_foreign_hooks_registers_identity_and_removes_cleanly() {
        let temp = tempfile::tempdir().unwrap();
        let loc = locations(&temp);
        fs::create_dir_all(loc.antigravity_config.parent().unwrap()).unwrap();
        let foreign = json!({
            "lint-checker": {
                "PostToolUse": [{"matcher": "run_command", "hooks": [{"type": "command", "command": "./lint.sh"}]}]
            }
        });
        fs::write(
            &loc.antigravity_config,
            serde_json::to_vec(&foreign).unwrap(),
        )
        .unwrap();

        let status_before = antigravity_status(&loc, Some("0.5.0".into()));
        assert!(!status_before.installed);
        assert_eq!(status_before.version.as_deref(), Some("0.5.0"));

        let report = antigravity_apply(&loc, Action::Install).unwrap();
        assert!(report.changed && report.backup_path.unwrap().is_file());
        assert_eq!(report.owned_entry_ids.len(), 1);
        assert!(!antigravity_apply(&loc, Action::Install).unwrap().changed);

        let status_after = antigravity_status(&loc, None);
        assert!(status_after.installed);
        let id = status_after.installation_id.expect("应生成安装 ID");
        assert!(
            navigation_target_for_locations(&loc, Source::Antigravity, &id)
                .unwrap()
                .is_some()
        );
        assert!(
            navigation_target_for_locations(
                &loc,
                Source::Antigravity,
                "11111111-1111-4111-8111-111111111111"
            )
            .unwrap()
            .is_none()
        );

        let removed = antigravity_apply(&loc, Action::Remove).unwrap();
        assert!(removed.changed);
        let final_config = read_json(&loc.antigravity_config).unwrap().unwrap();
        assert_eq!(final_config, foreign);
        assert!(!antigravity_status(&loc, None).installed);
    }
}
