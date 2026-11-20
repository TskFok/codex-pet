use objc2::{MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSCompositingOperation, NSEvent,
    NSRectFillUsingOperation, NSResponder, NSView, NSWindow, NSWindowOrderingMode,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use tauri::{WebviewWindowBuilder, Wry, utils::config::BackgroundThrottlingPolicy};

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
            // 暂用原生椭圆命中区。让命中代理尽量透明，月影由 Canvas 子窗绘制。
            NSColor::colorWithSRGBRed_green_blue_alpha(0.0,0.0,0.0,0.01).setFill();
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(28.0,24.0),NSSize::new(104.0,120.0))).fill();
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            if let Some(window) = self.window() {
                let input = crate::windows::PointerInput { phase:crate::windows::PointerPhase::Down, x:point.x, y:point.y, button:event.buttonNumber() as i16 };
                let _ = crate::windows::dispatch_pointer_input(input, || {
                    eprintln!("桌宠原生按下：{},{}",point.x,point.y);
                    window.performWindowDragWithEvent(event);
                    Ok::<_,std::convert::Infallible>(())
                });
            }
        }
    }
);

struct ProxyPair {
    proxy: Retained<PetProxyWindow>,
    renderer: Retained<NSWindow>,
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
        msg_send![PetInputView::alloc(mtm), initWithFrame:NSRect::new(NSPoint::new(0.0,0.0),native.frame().size)]
    };
    proxy.setContentView(Some(&view));
    native.setIgnoresMouseEvents(true);
    // 代理是 parent，WindowServer 拖动 parent 时自动移动 Canvas child。
    unsafe {
        proxy.addChildWindow_ordered(native, NSWindowOrderingMode::Above);
    }
    PROXY.with(|state| *state.borrow_mut() = Some(ProxyPair { proxy, renderer }));
    set_visible(true)?;
    Ok(())
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
            "rendererWindow":native.windowNumber(),"proxyWindow":pair.proxy.windowNumber(),
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
