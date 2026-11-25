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
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
        .title("月影桌宠")
        .visible(false)
        .inner_size(crate::placement::WIDTH, crate::placement::HEIGHT)
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
    set_pet_visible(
        app,
        std::env::var("PET_VALIDATION_PHASE").as_deref() != Ok("hidden"),
    )?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn route_report(window: &tauri::WebviewWindow) -> Option<serde_json::Value> {
    macos::route_report(window)
}

/// 操作整个桌宠窗口组，调用者不能只隐藏 macOS 的 Canvas 子窗。
pub fn set_pet_visible(app: &tauri::AppHandle, visible: bool) -> tauri::Result<()> {
    flush_placement(app);
    if visible {
        restore_placement(app)?;
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_visible(visible)?;
    }
    #[cfg(target_os = "windows")]
    if let Some(window) = app.get_webview_window("pet") {
        windows::set_visible(&window, visible)?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
    crate::tray::sync_visibility(app, visible)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn shutdown_input() {
    macos_power::shutdown();
    macos::shutdown_input();
}

pub(crate) fn flush_placement(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    if macos_power::take_pending_move() {
        finish_drag(app);
    }
    if let Some(state) = app.try_state::<crate::placement::PlacementState>() {
        state.retry_write();
    }
}

pub(crate) fn restore_placement(app: &tauri::AppHandle) -> tauri::Result<()> {
    let saved = app.state::<crate::placement::PlacementState>().current();
    apply_placement(app, saved)
}

pub(crate) fn apply_placement(
    app: &tauri::AppHandle,
    saved: crate::placement::WindowPlacement,
) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    let position = macos::position(Some(saved))?;
    #[cfg(target_os = "windows")]
    let position = windows::position(app, Some(saved))?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(position) = position {
        app.state::<crate::placement::PlacementState>()
            .remember(position);
    }
    Ok(())
}

/// 查询当前显示器工作区和桌宠逻辑坐标，供自主活动规划使用。
pub(crate) async fn desktop_context(
    app: &tauri::AppHandle,
) -> Option<crate::placement::DesktopContext> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if app
            .run_on_main_thread(move || {
                let _ = tx.send(macos::desktop_context().ok().flatten());
            })
            .is_err()
        {
            return None;
        }
        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .ok()
            .and_then(|res| res.ok())
            .flatten()
    }
    #[cfg(target_os = "windows")]
    {
        let app_handle = app.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        if app
            .run_on_main_thread(move || {
                let _ = tx.send(windows::desktop_context(&app_handle).ok().flatten());
            })
            .is_err()
        {
            return None;
        }
        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .ok()
            .and_then(|res| res.ok())
            .flatten()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        None
    }
}

/// 平滑移动桌宠窗口一步。`persist` 为 false 时仅更新窗口位置，不频繁写盘；
/// 移动结束时以 `persist = true` 写入最终位置。
pub(crate) async fn step_placement(
    app: &tauri::AppHandle,
    target: crate::placement::WindowPlacement,
    persist: bool,
) -> Option<crate::placement::WindowPlacement> {
    #[cfg(target_os = "macos")]
    {
        let app_handle = app.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        if app
            .run_on_main_thread(move || {
                let moved = macos::position(Some(target)).ok().flatten();
                if persist
                    && let Some(ref pos) = moved
                    && let Some(state) = app_handle.try_state::<crate::placement::PlacementState>()
                {
                    state.remember(pos.clone());
                }
                let _ = tx.send(moved);
            })
            .is_err()
        {
            return None;
        }
        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .ok()
            .and_then(|res| res.ok())
            .flatten()
    }
    #[cfg(target_os = "windows")]
    {
        let app_handle = app.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        if app
            .run_on_main_thread(move || {
                let moved = windows::position(&app_handle, Some(target)).ok().flatten();
                if persist
                    && let Some(ref pos) = moved
                    && let Some(state) = app_handle.try_state::<crate::placement::PlacementState>()
                {
                    state.remember(pos.clone());
                }
                let _ = tx.send(moved);
            })
            .is_err()
        {
            return None;
        }
        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .ok()
            .and_then(|res| res.ok())
            .flatten()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, target, persist);
        None
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn finish_drag(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let result = macos::position(None);
    #[cfg(target_os = "windows")]
    let result = windows::position(app, None);
    match result {
        Ok(Some(position)) => app
            .state::<crate::placement::PlacementState>()
            .remember(position),
        Err(error) => eprintln!("桌宠位置更新失败：{error}"),
        _ => {}
    }
}

pub(crate) fn update_hit_region(
    window: &tauri::WebviewWindow,
    frame: &pet_core::pet_package::FrameRect,
    mask: crate::windows::AlphaMask,
    bubble_input: Option<crate::windows::BubbleInput>,
) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        macos::update_hit_region(frame, mask, bubble_input)
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (frame, bubble_input);
        windows::update_hit_region(window, mask)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (window, frame, mask, bubble_input);
        Ok(())
    }
}
