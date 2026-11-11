use tauri::{WebviewWindowBuilder, Wry};

pub(super) fn configure(
    builder: WebviewWindowBuilder<'_, Wry, tauri::AppHandle>,
) -> WebviewWindowBuilder<'_, Wry, tauri::AppHandle> {
    // WebView2 不支持 Tauri 的 background_throttling，保留系统行为并实测。
    builder
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(virtual_key: i32) -> i16;
}

pub(super) fn left_button_down() -> bool {
    // VK_LBUTTON；读取瞬时高位，不使用有竞态的「自上次调用后按过」低位。
    unsafe { GetAsyncKeyState(1) < 0 }
}
