#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_power;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
mod windows_power;

mod playback;
pub(crate) use playback::{Playback, PlaybackSignal};

use std::fmt;
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[derive(Debug)]
pub struct PlatformError(tauri::Error);
impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for PlatformError {}
impl From<tauri::Error> for PlatformError {
    fn from(value: tauri::Error) -> Self {
        Self(value)
    }
}

pub fn create_pet_window(app: &tauri::AppHandle) -> Result<(), PlatformError> {
    let builder = WebviewWindowBuilder::new(app, "pet", WebviewUrl::App("index.html".into()))
        .title("桌宠窗口实验")
        .inner_size(160.0, 180.0)
        .position(120.0, 180.0)
        .resizable(false)
        .decorations(false)
        .shadow(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false);
    #[cfg(target_os = "macos")]
    let builder = macos::configure(builder);
    #[cfg(target_os = "windows")]
    let builder = windows::configure(builder);
    let window = builder.build()?;
    #[cfg(target_os = "macos")]
    {
        macos::install_input(&window)?;
        macos_power::install(app);
    }
    #[cfg(target_os = "windows")]
    {
        windows::install_input(&window)?;
        windows_power::install(&window)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn route_report(window: &tauri::WebviewWindow) -> Option<serde_json::Value> {
    macos::route_report(window)
}

/// 操作整个桌宠窗口组，调用者不能只隐藏 macOS 的 Canvas 子窗。
pub fn set_pet_visible(app: &tauri::AppHandle, visible: bool) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::set_visible(visible)?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("pet") {
            if visible {
                window.show()?;
            } else {
                window.hide()?;
            }
        }
    }
    playback::set_visible(app, visible);
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn shutdown_input() {
    macos_power::shutdown();
    macos::shutdown_input();
}
