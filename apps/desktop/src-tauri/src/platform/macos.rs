use tauri::{WebviewWindowBuilder, Wry, utils::config::BackgroundThrottlingPolicy};

pub(super) fn configure(
    builder: WebviewWindowBuilder<'_, Wry, tauri::AppHandle>,
) -> WebviewWindowBuilder<'_, Wry, tauri::AppHandle> {
    // WKWebView 仅 macOS 14+ 支持；13 的后台表现必须另行真机验证。
    builder
        .accept_first_mouse(true)
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
}

pub(super) fn left_button_down() -> bool {
    objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0
}
