use super::playback;
use std::{ffi::c_void, io};
use tauri::Manager;

type Hwnd = *mut c_void;
type WindowProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

const WM_POWERBROADCAST: u32 = 0x0218;
const WM_NCDESTROY: u32 = 0x0082;
const PBT_APMSUSPEND: usize = 0x0004;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
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
    match message {
        WM_POWERBROADCAST => {
            // SAFETY: the subclass owns its boxed AppHandle until WM_NCDESTROY.
            let app = unsafe { &*(data as *const tauri::AppHandle) };
            match wparam {
                PBT_APMSUSPEND => playback::set_suspended(app, true),
                PBT_APMRESUMEAUTOMATIC => playback::set_suspended(app, false),
                _ => {}
            }
        }
        WM_NCDESTROY => {
            // SAFETY: unhook this window's main-thread subclass before freeing its unique data.
            unsafe {
                RemoveWindowSubclass(window, window_proc, id);
                drop(Box::from_raw(data as *mut tauri::AppHandle));
            }
        }
        _ => {}
    }
    // SAFETY: forward this message unchanged to the remaining subclasses and window procedures.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

pub(super) fn install(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?.0;
    let data = Box::into_raw(Box::new(window.app_handle().clone()));
    // SAFETY: setup runs on the HWND's owning thread, and the callback releases data on destruction.
    if unsafe { SetWindowSubclass(hwnd, window_proc, SUBCLASS_ID, data as usize) } == 0 {
        // SAFETY: failed installation leaves the caller as the sole owner.
        unsafe { drop(Box::from_raw(data)) };
        return Err(io::Error::other("无法监听桌宠窗口的系统电源事件").into());
    }
    Ok(())
}
