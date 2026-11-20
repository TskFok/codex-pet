use super::playback;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};
use std::cell::RefCell;

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = tauri::AppHandle]
    struct PetPowerObserver;

    impl PetPowerObserver {
        #[unsafe(method(petWillSleep:))]
        fn will_sleep(&self, _notification: &NSNotification) {
            playback::set_suspended(self.ivars(), true);
        }

        #[unsafe(method(petDidWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
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
        unsafe { self.center.removeObserver(&self.observer) };
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
    WATCH.with(|watch| watch.borrow_mut().take());
}
