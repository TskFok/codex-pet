use super::playback;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSWindow, NSWindowDidMoveNotification,
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObject, NSObjectNSDelayedPerforming,
};
use std::cell::RefCell;

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = tauri::AppHandle]
    struct PetPowerObserver;

    impl PetPowerObserver {
        #[unsafe(method(petRestore:))]
        fn restore_position(&self, _notification:&NSNotification) {
            if let Err(error)=super::restore_placement(self.ivars()) {eprintln!("恢复桌宠位置失败：{error}");}
        }
        #[unsafe(method(petDidMove:))]
        fn did_move(&self,_notification:&NSNotification) {
            // WindowServer 接管拖动后 mouseUp 不一定送达。只在一批移动通知结束时保存。
            unsafe {
                NSObject::cancelPreviousPerformRequestsWithTarget_selector_object(self,sel!(petSave:),None);
                self.performSelector_withObject_afterDelay(sel!(petSave:),None,0.18);
            }
        }
        #[unsafe(method(petSave:))]
        fn save_position(&self,_unused:Option<&NSObject>) {super::finish_drag(self.ivars());}

        #[unsafe(method(petWillSleep:))]
        fn will_sleep(&self, _notification: &NSNotification) {
            playback::set_suspended(self.ivars(), true);
        }

        #[unsafe(method(petDidWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            if let Err(error)=super::restore_placement(self.ivars()) {eprintln!("恢复桌宠位置失败：{error}");}
            playback::set_suspended(self.ivars(), false);
        }
    }
);

struct PowerWatch {
    center: Retained<NSNotificationCenter>,
    observer: Retained<PetPowerObserver>,
}

impl Drop for PowerWatch {
    fn drop(&mut self) {
        // SAFETY: the center and registered observer are both still retained.
        unsafe {
            NSObject::cancelPreviousPerformRequestsWithTarget(&self.observer);
            self.center.removeObserver(&self.observer);
        }
    }
}

thread_local! {
    static WATCH: RefCell<Option<PowerWatch>> = const { RefCell::new(None) };
}

pub(super) fn install(app: &tauri::AppHandle) {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: NSObject's initializer is called exactly once following ivar initialization.
    let allocated = PetPowerObserver::alloc().set_ivars(app.clone());
    let observer: Retained<PetPowerObserver> = unsafe { msg_send![super(allocated), init] };
    // SAFETY: selectors match the class methods, and WATCH retains the observer until removal.
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(petWillSleep:),
            Some(NSWorkspaceWillSleepNotification),
            None,
        );
        center.addObserver_selector_name_object(
            &observer,
            sel!(petDidWake:),
            Some(NSWorkspaceDidWakeNotification),
            None,
        );
    }
    WATCH.with(|watch| *watch.borrow_mut() = Some(PowerWatch { center, observer }));
}

pub(super) fn shutdown() {
    POSITION.with(|watch| watch.borrow_mut().take());
    WATCH.with(|watch| watch.borrow_mut().take());
}

thread_local! { static POSITION: RefCell<Option<PowerWatch>> = const { RefCell::new(None) }; }
pub(super) fn watch_position(app: &tauri::AppHandle, proxy: &NSWindow) {
    let center = NSNotificationCenter::defaultCenter();
    let observer: Retained<PetPowerObserver> = unsafe {
        msg_send![
            super(PetPowerObserver::alloc().set_ivars(app.clone())),
            init
        ]
    };
    // SAFETY: CENTER 和匹配所注册方法的 observer 持有至退出。
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(petDidMove:),
            Some(NSWindowDidMoveNotification),
            Some(proxy),
        );
        center.addObserver_selector_name_object(
            &observer,
            sel!(petRestore:),
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
        );
    }
    POSITION.with(|watch| *watch.borrow_mut() = Some(PowerWatch { center, observer }));
}
