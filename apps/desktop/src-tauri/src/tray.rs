use crate::platform;
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

fn open_panel(app: &tauri::AppHandle, label: &str, title: &str) -> tauri::Result<()> {
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(
            app,
            label,
            WebviewUrl::App(format!("index.html#{label}").into()),
        )
        .title(title)
        .inner_size(520., 440.)
        .visible(false)
        .build()?,
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

fn visibility_title(visible: bool) -> &'static str {
    if visible {
        "隐藏桌宠"
    } else {
        "显示桌宠"
    }
}

struct ToggleItem(MenuItem<tauri::Wry>);
pub(crate) fn sync_visibility(app: &tauri::AppHandle, visible: bool) -> tauri::Result<()> {
    if let Some(toggle) = app.try_state::<ToggleItem>() {
        toggle.0.set_text(visibility_title(visible))?;
    }
    Ok(())
}

fn request_panel(
    app: &tauri::AppHandle,
    label: &'static str,
    title: &'static str,
) -> tauri::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // WebView2 不允许在同步窗口或托盘回调里建窗。同一标签的连续点击共享一个窗口。
        static OPENING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = OPENING.lock().unwrap();
            if let Err(error) = open_panel(&app, label, title) {
                eprintln!("打开{title}失败：{error}");
            }
        });
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    open_panel(app, label, title)
}

pub(crate) fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let item = |id: &str, title: &str| MenuItem::with_id(app, id, title, true, None::<&str>);
    let toggle = item(
        "toggle-pet",
        visibility_title(app.state::<platform::Playback>().snapshot().visible),
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &item("notifications", "通知")?,
            &item("settings", "设置")?,
            &item("quit", "退出")?,
        ],
    )?;
    app.manage(ToggleItem(toggle));
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::other("缺少桌宠托盘图标"))?;
    TrayIconBuilder::with_id("pet-tray")
        .tooltip("Codex 桌宠")
        .icon(icon.clone())
        .menu(&menu)
        .on_menu_event(move |app, event| {
            let result = match event.id.as_ref() {
                "toggle-pet" => {
                    let next = !app.state::<platform::Playback>().snapshot().visible;
                    platform::set_pet_visible(app, next)
                }
                "notifications" => request_panel(app, "notifications", "通知"),
                "settings" => request_panel(app, "settings", "设置"),
                "quit" => {
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("桌宠托盘操作失败：{error}");
            }
        })
        .build(app)?;
    Ok(())
}
