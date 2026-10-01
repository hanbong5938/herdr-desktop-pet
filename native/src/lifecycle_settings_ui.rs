use crate::control;
use crate::i18n::{resolve_language, text, Message, UiLocale};
use crate::lifecycle::{LifecycleSetting, LifecycleSettings, Paths};
use crate::preferences::Preferences;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
    NSBackingStoreType, NSBezierPath, NSColor, NSControlSize, NSControlStateValueOff,
    NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSEventType, NSFont,
    NSRunningApplication, NSSwitch, NSTextField, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{NSLocale, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;

pub(crate) const CARD_HEIGHT: f64 = 300.0;

// SAFETY: This decorative AppKit view is constructed and used on the main thread.
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetLifecycleCardView"]
    struct CardView;
    unsafe impl NSObjectProtocol for CardView {}
    impl CardView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let bounds = self.bounds();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.161, 0.161, 0.176, 1.0).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 10.0, 10.0).fill();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.28, 0.28, 0.32, 0.5).setStroke();
            let border = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                NSRect::new(NSPoint::new(0.5, 0.5), NSSize::new((bounds.size.width - 1.0).max(1.0), (bounds.size.height - 1.0).max(1.0))),
                9.5, 9.5,
            );
            border.setLineWidth(1.0);
            border.stroke();
        }
    }
);

impl CardView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: NSRect::default()] }
    }
}

fn label(
    value: &str,
    size: f64,
    bold: bool,
    muted: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    let color = if muted {
        NSColor::colorWithSRGBRed_green_blue_alpha(0.60, 0.61, 0.66, 1.0)
    } else {
        NSColor::colorWithSRGBRed_green_blue_alpha(0.95, 0.95, 0.97, 1.0)
    };
    field.setTextColor(Some(&color));
    field.setMaximumNumberOfLines(3);
    field
}

fn accessibility(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}

pub(crate) struct LifecycleSettingsCard {
    view: Retained<CardView>,
    heading: Retained<NSTextField>,
    auto_label: Retained<NSTextField>,
    auto_help: Retained<NSTextField>,
    auto_switch: Retained<NSSwitch>,
    exit_label: Retained<NSTextField>,
    exit_help: Retained<NSTextField>,
    exit_switch: Retained<NSSwitch>,
    quit_help: Retained<NSTextField>,
    error: Retained<NSTextField>,
    locale: UiLocale,
    error_detail: Option<String>,
}

impl LifecycleSettingsCard {
    pub(crate) fn new(locale: UiLocale, target: &AnyObject, mtm: MainThreadMarker) -> Self {
        let view = CardView::new(mtm);
        unsafe {
            let _: () = msg_send![&*view, setAccessibilityElement: false];
        }
        let heading = label("", 12.5, true, false, mtm);
        let auto_label = label("", 12.0, true, false, mtm);
        let auto_help = label("", 10.5, false, true, mtm);
        let exit_label = label("", 12.0, true, false, mtm);
        let exit_help = label("", 10.5, false, true, mtm);
        let quit_help = label("", 10.5, false, true, mtm);
        let error = label("", 10.5, false, false, mtm);
        error.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            1.0, 0.54, 0.50, 1.0,
        )));
        let make_switch = |action| {
            let switch: Retained<NSSwitch> = unsafe {
                msg_send![NSSwitch::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(46.0, 30.0))]
            };
            switch.setControlSize(NSControlSize::Small);
            switch.setRefusesFirstResponder(false);
            unsafe {
                switch.setTarget(Some(target));
                switch.setAction(Some(action));
            }
            switch
        };
        let auto_switch = make_switch(sel!(toggleLifecycleAutoStart:));
        let exit_switch = make_switch(sel!(toggleLifecycleExit:));
        for child in [
            &*heading as &NSView,
            &*auto_label,
            &*auto_help,
            &*auto_switch,
            &*exit_label,
            &*exit_help,
            &*exit_switch,
            &*quit_help,
            &*error,
        ] {
            view.addSubview(child);
        }
        let mut card = Self {
            view,
            heading,
            auto_label,
            auto_help,
            auto_switch,
            exit_label,
            exit_help,
            exit_switch,
            quit_help,
            error,
            locale,
            error_detail: None,
        };
        card.set_locale(locale);
        card
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }

    pub(crate) fn layout(&self, frame: NSRect) {
        self.view.setFrame(frame);
        let width = frame.size.width;
        let text_width = (width - 82.0).max(1.0);
        let full_width = (width - 28.0).max(1.0);
        self.heading.setFrame(NSRect::new(
            NSPoint::new(14.0, 10.0),
            NSSize::new(full_width, 20.0),
        ));
        self.auto_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 36.0),
            NSSize::new(text_width, 32.0),
        ));
        self.auto_switch.setFrame(NSRect::new(
            NSPoint::new(width - 58.0, 32.0),
            NSSize::new(46.0, 30.0),
        ));
        self.auto_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 70.0),
            NSSize::new(full_width, 40.0),
        ));
        self.exit_label.setFrame(NSRect::new(
            NSPoint::new(14.0, 116.0),
            NSSize::new(text_width, 32.0),
        ));
        self.exit_switch.setFrame(NSRect::new(
            NSPoint::new(width - 58.0, 112.0),
            NSSize::new(46.0, 30.0),
        ));
        self.exit_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 150.0),
            NSSize::new(full_width, 48.0),
        ));
        self.quit_help.setFrame(NSRect::new(
            NSPoint::new(14.0, 204.0),
            NSSize::new(full_width, 36.0),
        ));
        self.error.setFrame(NSRect::new(
            NSPoint::new(14.0, 246.0),
            NSSize::new(full_width, 44.0),
        ));
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.locale = locale;
        for (field, message) in [
            (&self.heading, Message::LifecycleHeading),
            (&self.auto_label, Message::LifecycleAutoStart),
            (&self.auto_help, Message::LifecycleAutoStartHelp),
            (&self.exit_label, Message::LifecycleExit),
            (&self.exit_help, Message::LifecycleExitHelp),
            (&self.quit_help, Message::LifecycleQuitHelp),
        ] {
            field.setStringValue(&NSString::from_str(text(locale, message)));
        }
        accessibility(&self.auto_switch, text(locale, Message::LifecycleAutoStart));
        accessibility(&self.exit_switch, text(locale, Message::LifecycleExit));
        accessibility(
            &self.auto_help,
            text(locale, Message::LifecycleAutoStartHelp),
        );
        accessibility(&self.exit_help, text(locale, Message::LifecycleExitHelp));
        self.display_error();
    }

    pub(crate) fn sync(&self, settings: LifecycleSettings) {
        self.auto_switch.setState(if settings.auto_start {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        self.exit_switch.setState(if settings.exit_with_herdr {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    pub(crate) fn value(&self, key: LifecycleSetting) -> bool {
        match key {
            LifecycleSetting::AutoStart => self.auto_switch.state() == NSControlStateValueOn,
            LifecycleSetting::ExitWithHerdr => self.exit_switch.state() == NSControlStateValueOn,
        }
    }

    pub(crate) fn saved(&mut self, settings: LifecycleSettings) {
        self.error_detail = None;
        self.display_error();
        self.sync(settings);
    }

    pub(crate) fn failed(&mut self, settings: LifecycleSettings, detail: String) {
        self.sync(settings);
        self.error_detail = Some(detail);
        self.display_error();
    }

    fn display_error(&self) {
        let message = self
            .error_detail
            .as_ref()
            .map(|detail| {
                format!(
                    "{}: {detail}",
                    text(self.locale, Message::LifecycleSaveFailure)
                )
            })
            .unwrap_or_default();
        self.error.setStringValue(&NSString::from_str(&message));
        accessibility(&self.error, &message);
    }
}

thread_local! { static WINDOW: RefCell<Option<Standalone>> = const { RefCell::new(None) }; }

struct Standalone {
    _window: Retained<NSWindow>,
    _target: Retained<SettingsTarget>,
    card: LifecycleSettingsCard,
    paths: Paths,
    settings: LifecycleSettings,
}

// SAFETY: All callbacks run on AppKit's main thread and use only main-thread-owned state.
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetLifecycleSettingsTarget"]
    struct SettingsTarget;
    unsafe impl NSObjectProtocol for SettingsTarget {}
    unsafe impl NSWindowDelegate for SettingsTarget {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &objc2_foundation::NSNotification) {
            let app = NSApplication::sharedApplication(self.mtm());
            app.stop(None);
            if let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                NSEventType::ApplicationDefined, NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(), 0.0, 0, None, 0, 0, 0,
            ) { app.postEvent_atStart(&event, true); }
        }
    }
    impl SettingsTarget {
        #[unsafe(method(toggleLifecycleAutoStart:))]
        fn toggle_auto(&self, _sender: &AnyObject) { save(LifecycleSetting::AutoStart); }
        #[unsafe(method(toggleLifecycleExit:))]
        fn toggle_exit(&self, _sender: &AnyObject) { save(LifecycleSetting::ExitWithHerdr); }
    }
);

fn save(key: LifecycleSetting) {
    WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(window) = borrow.as_mut() else {
            return;
        };
        let value = window.card.value(key);
        match control::set_lifecycle_setting(&window.paths, key, value) {
            Ok(settings) => {
                window.settings = settings;
                window.card.saved(settings);
            }
            Err(error) => window.card.failed(window.settings, error),
        }
    });
}

pub(crate) fn run(paths: Paths) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("settings must run on the AppKit main thread")?;
    let settings = control::get_lifecycle_settings(&paths)?;
    let preference = Preferences::load().unwrap_or_default().language();
    let preferred = NSLocale::preferredLanguages();
    let tags: Vec<_> = preferred.iter().map(|tag| tag.to_string()).collect();
    let tags: Vec<_> = tags.iter().map(String::as_str).collect();
    let locale = resolve_language(preference, &tags);
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // Complete AppKit launch before constructing the standalone window so it
    // participates in the application's normal window and accessibility tree.
    app.finishLaunching();
    let target = SettingsTarget::alloc(mtm).set_ivars(());
    let target: Retained<SettingsTarget> = unsafe { msg_send![super(target), init] };
    let width = 400.0;
    // SAFETY: NSWindow is allocated on AppKit's main thread and initialized once
    // with a valid content rectangle and supported window style.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, CARD_HEIGHT + 40.0),
            ),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: Retained owns this window; AppKit must not release it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(text(
        locale,
        Message::LifecycleWindowTitle,
    )));
    window.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*target)));
    let card = LifecycleSettingsCard::new(locale, &target, mtm);
    card.layout(NSRect::new(
        NSPoint::new(16.0, 20.0),
        NSSize::new(width - 32.0, CARD_HEIGHT),
    ));
    card.sync(settings);
    window
        .contentView()
        .ok_or("settings window has no content view")?
        .addSubview(card.view());
    WINDOW.with(|slot| {
        *slot.borrow_mut() = Some(Standalone {
            _window: window.clone(),
            _target: target,
            card,
            paths,
            settings,
        })
    });
    window.center();
    window.makeKeyAndOrderFront(None);
    if app.respondsToSelector(sel!(activate)) {
        app.activate();
    } else {
        // macOS 13 predates NSApplication.activate().
        let _ = NSRunningApplication::currentApplication()
            .activateWithOptions(NSApplicationActivationOptions::empty());
    }
    app.run();
    WINDOW.with(|slot| {
        slot.borrow_mut().take();
    });
    Ok(())
}
