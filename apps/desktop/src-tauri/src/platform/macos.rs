use crate::placement::{HEIGHT, WIDTH, WindowPlacement, WorkArea, clamp_placement};
use crate::windows::AlphaMask;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSCompositingOperation, NSEvent, NSRectFillList,
    NSRectFillUsingOperation, NSResponder, NSScreen, NSView, NSWindow, NSWindowOrderingMode,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSNumber, NSObject, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use tauri::{Manager, WebviewWindowBuilder, Wry, utils::config::BackgroundThrottlingPolicy};

pub(super) fn configure(
    builder: WebviewWindowBuilder<'_, Wry, tauri::AppHandle>,
) -> WebviewWindowBuilder<'_, Wry, tauri::AppHandle> {
    builder
        .visible(false)
        .accept_first_mouse(true)
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
}

define_class!(
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct PetProxyWindow;
    impl PetProxyWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool { false }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool { false }
    }
);

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<AlphaMask>]
    struct PetInputView;
    impl PetInputView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool { true }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            NSColor::clearColor().setFill();
            NSRectFillUsingOperation(self.bounds(), NSCompositingOperation::Copy);
            let mask = self.ivars().borrow();
            if mask.runs.is_empty() { return; }
            let sx = self.bounds().size.width / f64::from(mask.width);
            let sy = self.bounds().size.height / f64::from(mask.height);
            let mut rects: Vec<_> = mask.runs.iter().map(|&[y, left, right]| {
                NSRect::new(
                    NSPoint::new(f64::from(left) * sx, f64::from(y) * sy),
                    NSSize::new(f64::from(right - left) * sx, sy),
                )
            }).collect();
            NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.01).setFill();
            // SAFETY: 非空连续矩形数组在同步绘制结束前保持有效。
            unsafe {
                NSRectFillList(std::ptr::NonNull::new(rects.as_mut_ptr()).unwrap(), rects.len() as isize);
            }
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                #[cfg(debug_assertions)]
                if std::env::var_os("PET_DIAGNOSTICS_DIR").is_some() {
                    let point = self.convertPoint_fromView(event.locationInWindow(), None);
                    eprintln!("桌宠原生按下：{},{}", point.x, point.y);
                }
                window.performWindowDragWithEvent(event);
            }
        }
    }
);

struct ProxyPair {
    proxy: Retained<PetProxyWindow>,
    renderer: Retained<NSWindow>,
    view: Retained<PetInputView>,
    frame: Option<pet_core::pet_package::FrameRect>,
}
impl Drop for ProxyPair {
    fn drop(&mut self) {
        self.proxy.removeChildWindow(&self.renderer);
        self.proxy.orderOut(None);
        self.proxy.close();
    }
}
thread_local! { static PROXY: RefCell<Option<ProxyPair>> = const { RefCell::new(None) }; }

pub(super) fn install_input(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let mtm = MainThreadMarker::new().expect("输入代理必须在主线程安装");
    // SAFETY: setup 主线程借用 Tauri 拥有的有效 NSWindow，并为窗口组显式增加强引用。
    let native = unsafe { &*window.ns_window()?.cast::<NSWindow>() };
    let renderer = unsafe { Retained::retain(native as *const NSWindow as *mut NSWindow) }
        .expect("有效窗口指针");
    let proxy: Retained<PetProxyWindow> = unsafe {
        msg_send![PetProxyWindow::alloc(mtm), initWithContentRect:native.frame(), styleMask:NSWindowStyleMask::Borderless, backing:NSBackingStoreType::Buffered, defer:false]
    };
    unsafe {
        proxy.setReleasedWhenClosed(false);
    }
    proxy.setOpaque(false);
    proxy.setBackgroundColor(Some(&NSColor::clearColor()));
    proxy.setHasShadow(false);
    proxy.setLevel(native.level());
    let view: Retained<PetInputView> = unsafe {
        msg_send![super(PetInputView::alloc(mtm).set_ivars(RefCell::default())), initWithFrame:NSRect::new(NSPoint::new(0.0,0.0),native.frame().size)]
    };
    proxy.setContentView(Some(&view));
    native.setIgnoresMouseEvents(true);
    // 代理是 parent，WindowServer 拖动 parent 时自动移动 Canvas child。
    unsafe {
        proxy.addChildWindow_ordered(native, NSWindowOrderingMode::Above);
    }
    super::macos_power::watch_position(window.app_handle(), &proxy);
    PROXY.with(|state| {
        *state.borrow_mut() = Some(ProxyPair {
            proxy,
            renderer,
            view,
            frame: None,
        })
    });
    Ok(())
}

pub(super) fn is_visible() -> bool {
    PROXY.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|pair| pair.proxy.isVisible())
    })
}

pub(super) fn set_visible(visible: bool) -> tauri::Result<()> {
    if MainThreadMarker::new().is_none() {
        return Err(std::io::Error::other("窗口组显隐必须在主线程操作").into());
    }
    PROXY.with(|state| {
        if let Some(pair) = state.borrow().as_ref() {
            if visible {
                pair.proxy.orderFrontRegardless();
                pair.renderer.orderFrontRegardless();
            } else {
                pair.proxy.orderOut(None);
            }
        }
    });
    Ok(())
}
pub(super) fn shutdown_input() {
    let pair = PROXY.with(|state| state.borrow_mut().take());
    drop(pair);
}

pub(super) fn route_report(window: &tauri::WebviewWindow) -> Option<serde_json::Value> {
    let mtm = MainThreadMarker::new()?;
    let native = unsafe { &*window.ns_window().ok()?.cast::<NSWindow>() };
    PROXY.with(|state| {
        let state=state.borrow(); let pair=state.as_ref()?;
        let frame=pair.proxy.frame();
        let body=NSPoint::new(frame.origin.x+80.0,frame.origin.y+frame.size.height-84.0);
        let corner=NSPoint::new(frame.origin.x+5.0,frame.origin.y+frame.size.height-5.0);
        let query=|point,below|NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(point,below,mtm);
        let report=serde_json::json!({
            "pose":pair.frame,"runCount":pair.view.ivars().borrow().runs.len(),"rendererWindow":native.windowNumber(),"proxyWindow":pair.proxy.windowNumber(),
            "rendererParent":native.parentWindow().map(|p|p.windowNumber()),
            "bodyPoint":{"x":body.x,"y":body.y},"cornerPoint":{"x":corner.x,"y":corner.y},
            "bodyWindow":query(body,0),"cornerWindow":query(corner,0),
            "bodyBelowProxy":query(body,pair.proxy.windowNumber()),"cornerBelowProxy":query(corner,pair.proxy.windowNumber()),
            "rendererIgnoresMouseEvents":native.ignoresMouseEvents(),"proxyVisible":pair.proxy.isVisible(),"rendererVisible":native.isVisible(),
            "proxyFrame":{"x":frame.origin.x,"y":frame.origin.y,"width":frame.size.width,"height":frame.size.height},
            "rendererFrame":{"x":native.frame().origin.x,"y":native.frame().origin.y,"width":native.frame().size.width,"height":native.frame().size.height}
        });
        Some(report)
    })
}

pub(super) fn update_hit_region(
    frame: &pet_core::pet_package::FrameRect,
    mask: AlphaMask,
) -> tauri::Result<()> {
    let view = PROXY
        .with(|state| {
            let mut state = state.borrow_mut();
            let pair = state.as_mut()?;
            pair.frame = Some(*frame);
            *pair.view.ivars().borrow_mut() = mask;
            Some(pair.view.clone())
        })
        .ok_or_else(|| std::io::Error::other("桌宠代理不可用"))?;
    view.setNeedsDisplay(true);
    view.displayIfNeeded();
    Ok(())
}

fn area(screen: &NSScreen) -> WorkArea {
    let number = screen
        .deviceDescription()
        .objectForKey(&NSString::from_str("NSScreenNumber"))
        .and_then(|v| v.downcast::<NSNumber>().ok())
        .map(|n| n.unsignedIntValue());
    let rect = screen.visibleFrame();
    let scale = screen.backingScaleFactor();
    WorkArea {
        id: number
            .map(|v| v.to_string())
            .unwrap_or_else(|| screen.localizedName().to_string()),
        x: rect.origin.x * scale,
        y: -(rect.origin.y + rect.size.height) * scale,
        width: rect.size.width * scale,
        height: rect.size.height * scale,
        scale_factor: scale,
    }
}

pub(super) fn position(saved: Option<WindowPlacement>) -> tauri::Result<Option<WindowPlacement>> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| std::io::Error::other("桌宠位置必须在主线程恢复"))?;
    let Some((proxy, renderer)) = PROXY.with(|s| {
        s.borrow()
            .as_ref()
            .map(|p| (p.proxy.clone(), p.renderer.clone()))
    }) else {
        return Ok(None);
    };
    let areas: Vec<_> = NSScreen::screens(mtm)
        .iter()
        .map(|screen| area(&screen))
        .collect();
    let current = proxy.frame();
    let requested = match saved {
        Some(p) => p,
        None => {
            let Some(screen) = proxy.screen() else {
                return Ok(None);
            };
            let area = area(&screen);
            area.capture(
                current.origin.x * area.scale_factor,
                -(current.origin.y + current.size.height) * area.scale_factor,
                current.size.width / WIDTH,
            )
        }
    };
    let fit = clamp_placement(requested, &areas);
    if let Some(area) = areas
        .iter()
        .find(|a| a.usable() && Some(&a.id) == fit.monitor_id.as_ref())
    {
        let (x, y) = area.position(&fit);
        let size = NSSize::new(WIDTH * fit.scale, HEIGHT * fit.scale);
        let frame = NSRect::new(
            NSPoint::new(x / area.scale_factor, -y / area.scale_factor - size.height),
            size,
        );
        if current != frame {
            proxy.setFrame_display(frame, true);
        }
        if renderer.frame() != frame {
            renderer.setFrame_display(frame, true);
        }
        return Ok(Some(fit));
    }
    Ok(None)
}
