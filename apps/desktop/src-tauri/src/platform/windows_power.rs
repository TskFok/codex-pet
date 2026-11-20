use super::playback;
use std::{cell::Cell, ffi::c_void, io};
use tauri::Manager;

type Hwnd = *mut c_void;
type WindowProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

const WM_POWERBROADCAST: u32 = 0x0218;
const WM_NCDESTROY: u32 = 0x0082;
const PBT_APMSUSPEND: usize = 0x0004;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const WM_DPICHANGED: u32 = 0x02e0;
const WM_DISPLAYCHANGE: u32 = 0x007e;
const WM_SETTINGCHANGE: u32 = 0x001a;
const SPI_SETWORKAREA: usize = 0x002f;

struct WindowWatch {
    app: tauri::AppHandle,
    moving: Cell<bool>,
}
const SUBCLASS_ID: usize = 1;

#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(window: Hwnd, proc: WindowProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(window: Hwnd, proc: WindowProc, id: usize) -> i32;
    fn DefSubclassProc(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
}

unsafe extern "system" fn window_proc(
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    data: usize,
) -> isize {
    // SAFETY: 已安装子类持有该对象，直至本调用处理 WM_NCDESTROY。
    let watch = unsafe { &*(data as *const WindowWatch) };
    let app = &watch.app;
    match message {
        WM_ENTERSIZEMOVE => watch.moving.set(true),
        WM_EXITSIZEMOVE => {
            watch.moving.set(false);
            super::finish_drag(app);
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE
            if !watch.moving.get()
                && (message != WM_SETTINGCHANGE || wparam == SPI_SETWORKAREA) =>
        {
            // 先让 WebView2/Tao 应用系统建议的新 DPI 边界，再按目标工作区恢复位置。
            let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
            if let Err(error) = super::restore_placement(app) {
                eprintln!("恢复桌宠位置失败：{error}");
            }
            return result;
        }
        WM_POWERBROADCAST => match wparam {
            PBT_APMSUSPEND => playback::set_suspended(app, true),
            PBT_APMRESUMEAUTOMATIC => {
                if let Err(error) = super::restore_placement(app) {
                    eprintln!("恢复桌宠位置失败：{error}");
                }
                playback::set_suspended(app, false);
            }
            _ => {}
        },
        WM_NCDESTROY => {
            // SAFETY: unhook this window's main-thread subclass before freeing its unique data.
            unsafe {
                RemoveWindowSubclass(window, window_proc, id);
                drop(Box::from_raw(data as *mut WindowWatch));
            }
        }
        _ => {}
    }
    // SAFETY: forward this message unchanged to the remaining subclasses and window procedures.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

pub(super) fn install(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?.0;
    let data = Box::into_raw(Box::new(WindowWatch {
        app: window.app_handle().clone(),
        moving: Cell::new(false),
    }));
    // SAFETY: setup runs on the HWND's owning thread, and the callback releases data on destruction.
    if unsafe { SetWindowSubclass(hwnd, window_proc, SUBCLASS_ID, data as usize) } == 0 {
        // SAFETY: failed installation leaves the caller as the sole owner.
        unsafe { drop(Box::from_raw(data)) };
        return Err(io::Error::other("无法监听桌宠窗口的系统电源事件").into());
    }
    Ok(())
}
