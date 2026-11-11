pub mod platform;
mod windows;

use serde::Serialize;
use std::{
    fs,
    io::Write,
    sync::{Mutex, OnceLock},
};
use tauri::Manager;

#[derive(Default)]
struct DiagnosticState(Mutex<()>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
    os: &'static str,
    arch: &'static str,
    os_version: String,
    webview: String,
    tauri: &'static str,
    app_version: &'static str,
    visible: bool,
    focused: bool,
    scale_factor: f64,
    position: tauri::PhysicalPosition<i32>,
    cursor_position: tauri::PhysicalPosition<f64>,
    size: tauri::PhysicalSize<u32>,
    phase: String,
}

#[tauri::command]
fn runtime_info(window: tauri::WebviewWindow) -> Result<RuntimeInfo, String> {
    Ok(RuntimeInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        os_version: OS_VERSION.get_or_init(os_version).clone(),
        webview: tauri::webview_version().map_err(|e| e.to_string())?,
        tauri: tauri::VERSION,
        app_version: env!("CARGO_PKG_VERSION"),
        visible: window.is_visible().map_err(|e| e.to_string())?
            && !window.is_minimized().map_err(|e| e.to_string())?,
        focused: window.is_focused().map_err(|e| e.to_string())?,
        scale_factor: window.scale_factor().map_err(|e| e.to_string())?,
        position: window.inner_position().map_err(|e| e.to_string())?,
        cursor_position: window
            .app_handle()
            .cursor_position()
            .map_err(|e| e.to_string())?,
        size: window.inner_size().map_err(|e| e.to_string())?,
        phase: std::env::var("PET_VALIDATION_PHASE").unwrap_or_else(|_| "animation".into()),
    })
}

static OS_VERSION: OnceLock<String> = OnceLock::new();

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output();
    #[cfg(target_os = "windows")]
    let output = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW，诊断不能弹出控制台抢焦点。
            .output()
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let output = std::process::Command::new("uname").arg("-r").output();
    output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}

/// 仅显式设置环境变量时把诊断写入指定实验目录，不接触宿主配置。
#[tauri::command]
fn record_diagnostic(
    report: serde_json::Value,
    state: tauri::State<'_, DiagnosticState>,
) -> Result<(), String> {
    let line = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    if line.len() > 1024 * 1024 {
        return Err("诊断报告超过 1 MiB".into());
    }
    if let Some(directory) = std::env::var_os("PET_DIAGNOSTICS_DIR") {
        let _guard = state.0.lock().map_err(|e| e.to_string())?;
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = std::path::PathBuf::from(directory).join("runtime.jsonl");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    builder
        .manage(DiagnosticState::default())
        .invoke_handler(tauri::generate_handler![runtime_info, record_diagnostic])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            platform::create_pet_window(app.handle())?;
            if std::env::var("PET_VALIDATION_PHASE").as_deref() == Ok("hidden") {
                app.get_webview_window("pet").unwrap().hide()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("桌宠窗口启动失败");
}
