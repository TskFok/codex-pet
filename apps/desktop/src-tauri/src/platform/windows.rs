use crate::{
    placement::{HEIGHT, PlacementState, WIDTH, WindowPlacement, WorkArea, clamp_placement},
    windows::AlphaMask,
};
use std::{ffi::c_void, io, sync::Mutex};
use tauri::{Manager, PhysicalPosition, PhysicalSize};

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
    fn ExtCreateRegion(transform: *const c_void, bytes: u32, data: *const i32) -> *mut c_void;
    fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
}

#[derive(Default)]
struct InputRegion {
    mask: AlphaMask,
    geometry: Option<(i32, i32, i32, i32)>,
}

fn refresh_region(window: &tauri::WebviewWindow, mask: Option<AlphaMask>) -> tauri::Result<()> {
    let handle = window.hwnd()?.0;
    let state = window.state::<Mutex<InputRegion>>();
    // SetWindowRgn 会同步产生新的 Resized，忽略此递归通知。
    let Ok(mut state) = state.try_lock() else {
        if mask.is_some() {
            return Err(io::Error::other("桌宠区域正在更新").into());
        }
        return Ok(());
    };
    let mut client = Rect::default();
    let mut outer = Rect::default();
    let mut origin = Point::default();
    // SAFETY: HWND 由当前窗口持有，Win32 输出指向有效栈内存。
    unsafe {
        if GetClientRect(handle, &mut client) == 0
            || GetWindowRect(handle, &mut outer) == 0
            || ClientToScreen(handle, &mut origin) == 0
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    let w = client.right - client.left;
    let h = client.bottom - client.top;
    let dx = origin.x - outer.left;
    let dy = origin.y - outer.top;
    let geometry = (w, h, dx, dy);
    if mask.is_none() && state.geometry == Some(geometry) {
        return Ok(());
    }
    let shape = mask.as_ref().unwrap_or(&state.mask);
    let data = shape.region_data(w, h, (dx, dy));
    let region = if data.len() == 8 {
        unsafe { CreateRectRgn(0, 0, 0, 0) }
    } else {
        // SAFETY: 数据连续且 4 字节对齐，长度与头部矩形数匹配。
        unsafe { ExtCreateRegion(std::ptr::null(), data.len() as u32 * 4, data.as_ptr()) }
    };
    if region.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe { SetWindowRgn(handle, region, 1) } == 0 {
        let error = io::Error::last_os_error();
        unsafe {
            DeleteObject(region);
        }
        return Err(error.into());
    }
    // 成功后由 Window Manager 持有 HRGN。
    if let Some(mask) = mask {
        state.mask = mask;
    }
    state.geometry = Some(geometry);
    Ok(())
}

pub(super) fn update_hit_region(
    window: &tauri::WebviewWindow,
    mask: AlphaMask,
) -> tauri::Result<()> {
    refresh_region(window, Some(mask))
}

pub(super) fn install_input(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.manage(Mutex::new(InputRegion::default()));
    refresh_region(window, None)?;
    let target = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
        ) && let Err(error) = refresh_region(&target, None)
        {
            eprintln!("桌宠区域更新失败：{error}");
        }
    });
    Ok(())
}

fn area(monitor: &tauri::Monitor) -> WorkArea {
    let work = monitor.work_area();
    let id = monitor
        .name()
        .cloned()
        .unwrap_or_else(|| format!("{},{}", monitor.position().x, monitor.position().y));
    WorkArea {
        id,
        x: f64::from(work.position.x),
        y: f64::from(work.position.y),
        width: f64::from(work.size.width),
        height: f64::from(work.size.height),
        scale_factor: monitor.scale_factor(),
    }
}

pub(super) fn position(
    app: &tauri::AppHandle,
    saved: Option<WindowPlacement>,
) -> tauri::Result<Option<WindowPlacement>> {
    let Some(window) = app.get_webview_window("pet") else {
        return Ok(None);
    };
    let mut areas: Vec<_> = window.available_monitors()?.iter().map(area).collect();
    if let Some(primary) = window.primary_monitor()? {
        let id = area(&primary).id;
        if let Some(i) = areas.iter().position(|a| a.id == id) {
            areas.swap(0, i);
        }
    }
    let current = window.outer_position()?;
    let requested = match saved {
        Some(value) => value,
        None => {
            let Some(monitor) = window.current_monitor()? else {
                return Ok(None);
            };
            area(&monitor).capture(
                f64::from(current.x),
                f64::from(current.y),
                app.state::<PlacementState>().current().scale,
            )
        }
    };
    let fit = clamp_placement(requested, &areas);
    let Some(area) = areas
        .iter()
        .find(|a| a.usable() && Some(&a.id) == fit.monitor_id.as_ref())
    else {
        return Ok(None);
    };
    let (x, y) = area.position(&fit);
    let position = PhysicalPosition::new(x.round() as i32, y.round() as i32);
    if current != position {
        window.set_position(position)?;
    }
    let size = PhysicalSize::new(
        (WIDTH * fit.scale * area.scale_factor).round() as u32,
        (HEIGHT * fit.scale * area.scale_factor).round() as u32,
    );
    if window.inner_size()? != size {
        window.set_size(size)?;
    }
    Ok(Some(fit))
}
