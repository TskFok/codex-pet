use pet_bridge::{
    codex::{MAX_HOOK_BYTES, hook_session_id, normalize_codex},
    hook_install::{HookPatch, apply_hook_patch, plan_hook_install, plan_hook_remove},
};
use pet_core::{Source, SourceIdentity};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("codex") {
        // 所有 Hook 失败均保持无决定输出，诊断绝不包含宿主原始 JSON。
        if let Err(error) = run_hook(args.collect()) {
            eprintln!("pet-bridge：{error}");
        }
        println!("{{}}");
    } else {
        let all: Vec<_> = std::env::args().skip(1).collect();
        if let Err(error) = run_install(&all) {
            eprintln!("pet-bridge：{error}");
            std::process::exit(1);
        }
    }
}

fn options(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        if !allowed.contains(&pair[0].as_str())
            || pair[1].is_empty()
            || result.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err("命令行参数无效或重复".into());
        }
    }
    if !pairs.remainder().is_empty() {
        return Err("命令行参数缺少值".into());
    }
    Ok(result)
}
fn required<'a>(values: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, String> {
    values
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("缺少参数：{name}"))
}
fn uuid(value: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| "安装 ID 必须为 UUID".into())
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| "无法独占创建输出文件")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法持久化输出文件".into())
}
fn probe_identity(directory: &Path) -> Result<String, String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory).map_err(|_| "无法创建探测目录")?;
    if fs::symlink_metadata(directory)
        .map_err(|_| "无法检查探测目录")?
        .file_type()
        .is_symlink()
    {
        return Err("探测目录不能是符号链接".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(directory)
            .map_err(|_| "无法检查探测目录")?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("探测目录只允许当前用户访问".into());
        }
    }
    let identity = directory.join(".installation-id");
    if !identity.exists() {
        let id = uuid::Uuid::new_v4().to_string();
        let staging = directory.join(format!(".{id}.tmp"));
        private_write(&staging, id.as_bytes())?;
        let result = fs::hard_link(&staging, &identity);
        let _ = fs::remove_file(staging);
        if result.is_err() && !identity.exists() {
            return Err("无法保存探测安装 ID".into());
        }
    }
    if fs::symlink_metadata(&identity)
        .map_err(|_| "无法检查探测安装 ID")?
        .file_type()
        .is_symlink()
    {
        return Err("安装 ID 不能是符号链接".into());
    }
    uuid(&fs::read_to_string(identity).map_err(|_| "无法读取探测安装 ID")?)
}
fn run_hook(args: Vec<String>) -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_HOOK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 Hook 输入")?;
    if bytes.len() as u64 > MAX_HOOK_BYTES {
        return Err("Hook 输入超过 8 MiB，已忽略".into());
    }
    let options = options(
        &args,
        &["--probe-dir", "--installation-id", "--instance-id"],
    )?;
    let input: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Hook 输入不是完整有效 JSON")?;
    let probe = options.get("--probe-dir").map(PathBuf::from);
    let installation_id = match options.get("--installation-id") {
        Some(id) => {
            if let Some(path) = &probe {
                let _ = probe_identity(path)?;
            }
            uuid(id)?
        }
        None => probe_identity(
            probe
                .as_deref()
                .ok_or("缺少安装 ID；探测请显式指定 --probe-dir")?,
        )?,
    };
    let session = hook_session_id(&input)?;
    // 公开 Hook 不提供宿主进程生命周期；使用已知会话作用域，绝不每次调用随机生成实例。
    let instance_id = options
        .get("--instance-id")
        .cloned()
        .unwrap_or_else(|| format!("hook-session:{session}"));
    let identity = SourceIdentity {
        source: Source::Codex,
        installation_id,
        instance_id,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_millis()
        .try_into()
        .map_err(|_| "系统时间溢出")?;
    let events = normalize_codex(&input, &identity, now)?;
    if let Some(directory) = probe {
        for event in events {
            let bytes = serde_json::to_vec(&event).map_err(|_| "事件序列化失败")?;
            private_write(&directory.join(format!("{}.json", event.event_id)), &bytes)?;
        }
    } else {
        return Err("通知投递尚未接入；当前仅支持显式 --probe-dir 探测".into());
    }
    if input
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        == Some("PermissionRequest")
    {
        eprintln!("pet-bridge：当前适配器不支持 Codex 精确审批/提问等待及解除，未生成待处理提醒");
    }
    Ok(())
}
fn read_config(path: &Path) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("无法读取指定配置".into()),
    }
}
fn read_patch(path: &str) -> Result<HookPatch, String> {
    serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取计划清单")?)
        .map_err(|_| "计划清单不是有效 HookPatch".into())
}
fn run_install(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("hooks") {
        return Err("用法：pet-bridge codex [--probe-dir 路径]；或 pet-bridge hooks plan-install|plan-remove|apply --config 路径 …".into());
    }
    let action = args.get(1).map(String::as_str).ok_or("缺少安装操作")?;
    let allowed = match action {
        "plan-install" => vec!["--config", "--bridge", "--plan"],
        "plan-remove" => vec!["--config", "--manifest", "--plan"],
        "apply" => vec!["--config", "--plan"],
        _ => return Err("未知安装操作".into()),
    };
    let options = options(&args[2..], &allowed)?;
    let config = Path::new(required(&options, "--config")?);
    if !config.is_absolute() {
        return Err("配置必须使用绝对路径".into());
    }
    let plan = required(&options, "--plan")?;
    if action == "apply" {
        apply_hook_patch(config, &read_patch(plan)?)?;
        println!("已应用显式 Hook 计划；请在 Codex 中检查并信任配置。");
        return Ok(());
    }
    let existing = read_config(config)?;
    let patch = if action == "plan-install" {
        plan_hook_install(&existing, Path::new(required(&options, "--bridge")?))?
    } else {
        plan_hook_remove(&existing, &read_patch(required(&options, "--manifest")?)?)?
    };
    private_write(
        Path::new(plan),
        &serde_json::to_vec_pretty(&patch).map_err(|_| "计划序列化失败")?,
    )?;
    println!(
        "{}",
        String::from_utf8(patch.after_bytes).map_err(|_| "计划编码错误")?
    );
    Ok(())
}
