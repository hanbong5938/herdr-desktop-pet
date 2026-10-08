//! The coordinator is outside the replaceable app and owns its own native window.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CString};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}
    #[link(name = "Foundation", kind = "framework")]
    extern "C" {}
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        fn objc_allocateClassPair(superclass: Obj, name: *const c_char, extra_bytes: usize) -> Obj;
        fn objc_registerClassPair(cls: Obj);
        fn class_addMethod(
            cls: Obj,
            name: Obj,
            implementation: *const c_void,
            types: *const c_char,
        ) -> i8;
    }
    type Obj = *mut c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    fn class(name: &str) -> Obj {
        unsafe { objc_getClass(CString::new(name).unwrap().as_ptr()) }
    }
    fn selector(name: &str) -> Obj {
        unsafe { sel_registerName(CString::new(name).unwrap().as_ptr()) }
    }
    unsafe fn object(receiver: Obj, message: &str) -> Obj {
        let call: unsafe extern "C" fn(Obj, Obj) -> Obj =
            std::mem::transmute(objc_msgSend as *const ());
        call(receiver, selector(message))
    }
    unsafe fn send_void(receiver: Obj, message: &str) {
        let call: unsafe extern "C" fn(Obj, Obj) = std::mem::transmute(objc_msgSend as *const ());
        call(receiver, selector(message));
    }
    unsafe fn send_object(receiver: Obj, message: &str, value: Obj) {
        let call: unsafe extern "C" fn(Obj, Obj, Obj) =
            std::mem::transmute(objc_msgSend as *const ());
        call(receiver, selector(message), value);
    }
    unsafe fn send_bool(receiver: Obj, message: &str, value: bool) {
        let call: unsafe extern "C" fn(Obj, Obj, i8) =
            std::mem::transmute(objc_msgSend as *const ());
        call(receiver, selector(message), value as i8);
    }
    unsafe fn string(value: &str) -> Obj {
        let call: unsafe extern "C" fn(Obj, Obj, *const c_char) -> Obj =
            std::mem::transmute(objc_msgSend as *const ());
        let value = CString::new(value.replace('\0', " ")).unwrap();
        call(
            class("NSString"),
            selector("stringWithUTF8String:"),
            value.as_ptr(),
        )
    }
    static RECOVER_REQUESTED: AtomicBool = AtomicBool::new(false);
    extern "C" fn recover_action(_: Obj, _: Obj, _: Obj) {
        RECOVER_REQUESTED.store(true, Ordering::Release);
    }
    fn recovery_target() -> Result<Obj, String> {
        unsafe {
            let name = CString::new("HerdrUpdateCoordinatorRecoveryTarget").unwrap();
            let mut cls = objc_getClass(name.as_ptr());
            if cls.is_null() {
                cls = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
                if cls.is_null() {
                    return Err("cannot allocate AppKit recovery action".into());
                }
                if class_addMethod(
                    cls,
                    selector("recoverUpdate:"),
                    recover_action as *const c_void,
                    b"v@:@\0".as_ptr().cast(),
                ) == 0
                {
                    return Err("cannot register AppKit recovery action".into());
                }
                objc_registerClassPair(cls);
            }
            Ok(object(object(cls, "alloc"), "init"))
        }
    }
    // Native panel is deliberately nonmodal. A failure stays visible until the user closes it.
    pub struct Panel {
        app: Obj,
        window: Obj,
        label: Obj,
        recovery_button: Obj,
        _target: Obj,
    }
    impl Panel {
        pub fn new(locale: &str) -> Result<Self, String> {
            unsafe {
                let app = object(class("NSApplication"), "sharedApplication");
                if app.is_null() {
                    return Err("AppKit application unavailable".into());
                }
                let policy: unsafe extern "C" fn(Obj, Obj, isize) -> i8 =
                    std::mem::transmute(objc_msgSend as *const ());
                policy(app, selector("setActivationPolicy:"), 1);
                send_void(app, "finishLaunching");
                let alloc = object(class("NSPanel"), "alloc");
                let init: unsafe extern "C" fn(Obj, Obj, Rect, usize, usize, i8) -> Obj =
                    std::mem::transmute(objc_msgSend as *const ());
                let window = init(
                    alloc,
                    selector("initWithContentRect:styleMask:backing:defer:"),
                    Rect {
                        origin: Point { x: 0., y: 0. },
                        size: Size {
                            width: 510.,
                            height: 185.,
                        },
                    },
                    1 | 2 | 128,
                    2,
                    0,
                );
                if window.is_null() {
                    return Err("AppKit window unavailable".into());
                }
                send_bool(window, "setReleasedWhenClosed:", false);
                let title = if locale.starts_with("ko") {
                    "Herdr 앱 업데이트"
                } else {
                    "Herdr app update"
                };
                send_object(window, "setTitle:", string(title));
                let label_alloc = object(class("NSTextField"), "alloc");
                let label_init: unsafe extern "C" fn(Obj, Obj, Rect) -> Obj =
                    std::mem::transmute(objc_msgSend as *const ());
                let label = label_init(
                    label_alloc,
                    selector("initWithFrame:"),
                    Rect {
                        origin: Point { x: 22., y: 52. },
                        size: Size {
                            width: 465.,
                            height: 105.,
                        },
                    },
                );
                send_bool(label, "setEditable:", false);
                send_bool(label, "setSelectable:", true);
                send_bool(label, "setBezeled:", false);
                send_bool(label, "setDrawsBackground:", false);
                send_object(
                    label,
                    "setStringValue:",
                    string(if locale.starts_with("ko") {
                        "업데이트를 준비하고 있습니다…"
                    } else {
                        "Preparing update…"
                    }),
                );
                let content = object(window, "contentView");
                send_object(content, "addSubview:", label);
                let target = recovery_target()?;
                let button_alloc = object(class("NSButton"), "alloc");
                let button = label_init(
                    button_alloc,
                    selector("initWithFrame:"),
                    Rect {
                        origin: Point { x: 305., y: 14. },
                        size: Size {
                            width: 180.,
                            height: 33.,
                        },
                    },
                );
                send_object(
                    button,
                    "setTitle:",
                    string(if locale.starts_with("ko") {
                        "확인 후 다시 시작"
                    } else {
                        "Recover verified start"
                    }),
                );
                send_object(button, "setTarget:", target);
                send_object(button, "setAction:", selector("recoverUpdate:"));
                send_bool(button, "setHidden:", true);
                send_object(content, "addSubview:", button);
                send_void(window, "center");
                send_void(window, "orderFrontRegardless");
                let panel = Self {
                    app,
                    window,
                    label,
                    recovery_button: button,
                    _target: target,
                };
                panel.pump();
                Ok(panel)
            }
        }
        pub fn activate(&self) {
            unsafe {
                send_bool(self.app, "activateIgnoringOtherApps:", true);
                send_object(self.window, "makeKeyAndOrderFront:", std::ptr::null_mut());
            }
            self.pump();
        }
        pub fn show(&self, message: &str) {
            unsafe {
                send_object(self.label, "setStringValue:", string(message));
            }
            self.pump();
        }
        pub fn offer_recovery(&self) {
            RECOVER_REQUESTED.store(false, Ordering::Release);
            unsafe {
                send_bool(self.recovery_button, "setHidden:", false);
            }
            self.pump();
        }
        pub fn recovery_requested(&self) -> bool {
            RECOVER_REQUESTED.swap(false, Ordering::AcqRel)
        }
        pub fn close(&self) {
            unsafe {
                send_object(self.window, "orderOut:", std::ptr::null_mut());
            }
        }
        pub fn pump(&self) {
            unsafe {
                let date = object(class("NSDate"), "distantPast");
                let next: unsafe extern "C" fn(Obj, Obj, usize, Obj, Obj, i8) -> Obj =
                    std::mem::transmute(objc_msgSend as *const ());
                for _ in 0..32 {
                    let event = next(
                        self.app,
                        selector("nextEventMatchingMask:untilDate:inMode:dequeue:"),
                        usize::MAX,
                        date,
                        string("kCFRunLoopDefaultMode"),
                        1,
                    );
                    if event.is_null() {
                        break;
                    }
                    send_object(self.app, "sendEvent:", event);
                }
                send_void(self.app, "updateWindows");
            }
        }
        pub fn visible(&self) -> bool {
            unsafe {
                let visible: unsafe extern "C" fn(Obj, Obj) -> i8 =
                    std::mem::transmute(objc_msgSend as *const ());
                visible(self.window, selector("isVisible")) != 0
            }
        }
    }
    impl Drop for Panel {
        fn drop(&mut self) {
            unsafe {
                send_object(self.window, "orderOut:", std::ptr::null_mut());
                send_void(self.label, "release");
                send_void(self.recovery_button, "release");
                send_void(self._target, "release");
                send_void(self.window, "release");
            }
        }
    }
    pub use Panel as NativePanel;
}
#[cfg(target_os = "macos")]
pub use mac::NativePanel as Panel;
#[cfg(not(target_os = "macos"))]
pub struct Panel;
#[cfg(not(target_os = "macos"))]
impl Panel {
    pub fn new(_: &str) -> Result<Self, String> {
        Err("native updater requires macOS AppKit".into())
    }
    pub fn activate(&self) {}
    pub fn show(&self, _: &str) {}
    pub fn pump(&self) {}
    pub fn visible(&self) -> bool {
        false
    }
    pub fn offer_recovery(&self) {}
    pub fn recovery_requested(&self) -> bool {
        false
    }
    pub fn close(&self) {}
}
