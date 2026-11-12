use crate::windows::region::{RegionUpdate, body_region};
use std::{
    ffi::c_void,
    io,
    sync::{Arc, Mutex},
};
use tauri::{WebviewWindowBuilder, Wry};

pub(super) fn configure(
    builder: WebviewWindowBuilder<'_, Wry, tauri::AppHandle>,
) -> WebviewWindowBuilder<'_, Wry, tauri::AppHandle> {
    // WebView2 无后台节流开关；命中交给系统窗口区域，禁止按光标采样切换整窗输入。
    builder
}

#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
    fn GetWindowRect(window: *mut c_void, rect: *mut Rect) -> i32;
    fn ClientToScreen(window: *mut c_void, point: *mut Point) -> i32;
    fn SetWindowRgn(window: *mut c_void, region: *mut c_void, redraw: i32) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateEllipticRgn(left: i32, top: i32, right: i32, bottom: i32) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
}

fn refresh_region(window: &tauri::WebviewWindow, cache: &Mutex<RegionUpdate>) -> tauri::Result<()> {
    let handle = window.hwnd()?.0;
    let mut client = Rect::default();
    let mut outer = Rect::default();
    let mut origin = Point::default();
    // SAFETY: 有效 HWND；所有输出指针都指向当前栈上正确 Win32 C 布局的结构。
    unsafe {
        if GetClientRect(handle, &mut client) == 0
            || GetWindowRect(handle, &mut outer) == 0
            || ClientToScreen(handle, &mut origin) == 0
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    let Some(shape) = body_region(
        client.right - client.left,
        client.bottom - client.top,
        (origin.x - outer.left, origin.y - outer.top),
    ) else {
        return Ok(());
    };
    let mut state = match cache.try_lock() {
        Ok(state) => state,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        // SetWindowRgn 会同步产生窗口消息；重复通知不能重入同一配置操作。
        Err(std::sync::TryLockError::WouldBlock) => return Ok(()),
    };
    state.apply(shape, |shape| {
        let region = unsafe { CreateEllipticRgn(shape.left, shape.top, shape.right, shape.bottom) };
        if region.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { SetWindowRgn(handle, region, 1) } == 0 {
            let error = io::Error::last_os_error();
            // 失败时所有权仍属调用者；成功时系统接管，不能 DeleteObject。
            unsafe {
                DeleteObject(region);
            }
            return Err(error);
        }
        Ok(())
    })?;
    Ok(())
}

pub(super) fn install_input(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let cache = Arc::new(Mutex::new(RegionUpdate::default()));
    // 初始区域失败时返回初始化错误，不能留下捕获整块透明矩形的可运行窗口。
    refresh_region(window, &cache)?;
    let window = window.clone();
    let target = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::Resized(_)
                | tauri::WindowEvent::ScaleFactorChanged { .. }
                | tauri::WindowEvent::Focused(_)
                | tauri::WindowEvent::ThemeChanged(_)
        ) && let Err(error) = refresh_region(&target, &cache)
        {
            // 不移除事件处理器，也不缓存失败的区域；后续生命周期事件可再次成功。
            eprintln!("桌宠区域更新失败，等待下次窗口事件重试：{error}");
        }
    });
    Ok(())
}
