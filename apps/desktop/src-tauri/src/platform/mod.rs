#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::{fmt, thread, time::Duration};
use tauri::{Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

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
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false);
    #[cfg(target_os = "macos")]
    let builder = macos::configure(builder);
    #[cfg(target_os = "windows")]
    let builder = windows::configure(builder);
    let window = builder.build()?;
    window.set_ignore_cursor_events(true)?;
    window.on_window_event(|event| {
        if let tauri::WindowEvent::Moved(position) = event {
            eprintln!("桌宠窗口位置：{},{}", position.x, position.y);
        }
    });

    let app = app.clone();
    thread::spawn(move || {
        let mut ignoring = true;
        let mut dragging = crate::windows::DragTracker::default();
        while let Some(window) = app.get_webview_window("pet") {
            // 使用物理坐标差后除以当前 DPI；不要依赖透明像素隐式穿透。
            let result = (|| -> tauri::Result<bool> {
                let visible = window.is_visible()? && !window.is_minimized()?;
                let ignore = if visible {
                    let cursor = app.cursor_position()?;
                    let origin = window.inner_position()?;
                    let scale = window.scale_factor()?;
                    let hit = crate::windows::hits_pet(
                        (cursor.x - f64::from(origin.x)) / scale,
                        (cursor.y - f64::from(origin.y)) / scale,
                    );
                    #[cfg(target_os = "macos")]
                    let down = macos::left_button_down();
                    #[cfg(target_os = "windows")]
                    let down = windows::left_button_down();
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    let down = false;
                    if let Some((x, y)) =
                        dragging.update((cursor.x, cursor.y), down, hit, (origin.x, origin.y))
                        && (x, y) != (origin.x, origin.y)
                    {
                        window.set_position(PhysicalPosition::new(x, y))?;
                    }
                    !hit
                } else {
                    true
                };
                if ignore != ignoring {
                    window.set_ignore_cursor_events(ignore)?;
                    ignoring = ignore;
                    if std::env::var_os("PET_DIAGNOSTICS_DIR").is_some() {
                        eprintln!("桌宠输入忽略：{ignore}");
                    }
                }
                Ok(visible)
            })();
            let visible = match result {
                Ok(visible) => visible,
                Err(error) => {
                    eprintln!("桌宠命中区更新失败：{error}");
                    break;
                }
            };
            thread::sleep(Duration::from_millis(if visible { 16 } else { 250 }));
        }
    });
    Ok(())
}
