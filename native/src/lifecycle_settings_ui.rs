use crate::control;
use crate::i18n::{resolve_language, text, Message, UiLocale};
use crate::lifecycle::{LifecycleSetting, LifecycleSettings, Paths};
use crate::plugin_manager::{self, PluginManagerCard, CARD_HEIGHT as PLUGIN_MANAGER_HEIGHT};
use crate::preferences::Preferences;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSBackingStoreType, NSBezierPath, NSColor, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSEventType,
    NSFont, NSRunningApplication, NSSwitch, NSTextField, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{NSLocale, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

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
static NEXT_AVAILABILITY_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
enum AvailabilityAction {
    Refresh { preserve_error: bool },
    Open,
    Set(bool),
}

enum AvailabilityReply {
    Current(Result<bool, String>),
    Commit(Result<control::PluginManagerAvailabilityCommit, String>),
}

struct UncertainAvailability {
    instance_id: String,
    operation_id: String,
    current: Option<bool>,
}

struct Standalone {
    _window: Retained<NSWindow>,
    _target: Retained<SettingsTarget>,
    card: LifecycleSettingsCard,
    plugin_manager_card: PluginManagerCard,
    socket: PathBuf,
    paths: Paths,
    settings: LifecycleSettings,
    availability_request: Option<u64>,
    id: u64,
    uncertain: Option<UncertainAvailability>,
    confirming_removal: bool,
    availability_error: Option<String>,
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
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &objc2_foundation::NSNotification) {
            if WINDOW.with(|slot| slot.borrow().as_ref().is_some_and(|window| window.confirming_removal)) {
                return;
            }
            request_availability(AvailabilityAction::Refresh { preserve_error: false });
        }
    }
    impl SettingsTarget {
        #[unsafe(method(toggleLifecycleAutoStart:))]
        fn toggle_auto(&self, _sender: &AnyObject) { save(LifecycleSetting::AutoStart); }
        #[unsafe(method(toggleLifecycleExit:))]
        fn toggle_exit(&self, _sender: &AnyObject) { save(LifecycleSetting::ExitWithHerdr); }
        #[unsafe(method(openPluginManager:))]
        fn open_plugin_manager(&self, _sender: &AnyObject) { open_plugin_manager(); }
        #[unsafe(method(installPluginManager:))]
        fn install_plugin_manager(&self, _sender: &AnyObject) {
            request_availability(AvailabilityAction::Set(true));
        }
        #[unsafe(method(removePluginManager:))]
        fn remove_plugin_manager(&self, _sender: &AnyObject) {
            confirm_remove_plugin_manager(self.mtm());
        }
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

fn confirm_remove_plugin_manager(mtm: MainThreadMarker) {
    let Some((id, locale)) = WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let window = borrow.as_mut()?;
        if window.availability_request.is_some() || window.confirming_removal {
            return None;
        }
        window.confirming_removal = true;
        Some((window.id, window.plugin_manager_card.locale()))
    }) else {
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(text(
        locale,
        Message::PluginManagerTitle,
    )));
    alert.setInformativeText(&NSString::from_str(text(
        locale,
        Message::PluginManagerRemoveConfirmation,
    )));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Remove)));
    alert.addButtonWithTitle(&NSString::from_str(text(locale, Message::Cancel)));
    let approved = alert.runModal() == NSAlertFirstButtonReturn;
    let current = WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(window) = borrow.as_mut().filter(|window| window.id == id) else {
            return false;
        };
        window.confirming_removal = false;
        true
    });
    if approved && current {
        request_availability(AvailabilityAction::Set(false));
    }
}

fn open_plugin_manager() {
    request_availability(AvailabilityAction::Open);
}

fn availability_uncertain_message(
    locale: UiLocale,
    identity: &UncertainAvailability,
    current: Option<bool>,
) -> String {
    let current = match current {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    };
    format!(
        "{} (instance_id={}, operation_id={}, current={current})",
        text(locale, Message::PluginManagerAvailabilityUncertain),
        identity.instance_id,
        identity.operation_id,
    )
}

fn request_availability(action: AvailabilityAction) {
    let Some((paths, request_id)) = WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let window = borrow.as_mut()?;
        if window.availability_request.is_some() {
            return None;
        }
        let request_id = NEXT_AVAILABILITY_REQUEST.fetch_add(1, Ordering::Relaxed);
        window.availability_request = Some(request_id);
        if !matches!(
            action,
            AvailabilityAction::Refresh {
                preserve_error: true
            }
        ) {
            window.uncertain = None;
            window.availability_error = None;
            window.plugin_manager_card.report_error(None);
        }
        window.plugin_manager_card.set_busy(true);
        Some((window.paths.clone(), request_id))
    }) else {
        return;
    };
    thread::spawn(move || {
        let reply = match action {
            AvailabilityAction::Refresh { .. } | AvailabilityAction::Open => {
                AvailabilityReply::Current(control::get_plugin_manager_availability(&paths))
            }
            AvailabilityAction::Set(installed) => AvailabilityReply::Commit(
                control::set_plugin_manager_availability(&paths, installed),
            ),
        };
        DispatchQueue::main().exec_async(move || complete_availability(request_id, action, reply));
    });
}

fn complete_availability(request_id: u64, action: AvailabilityAction, reply: AvailabilityReply) {
    let mut open = None;
    let mut refresh_conflict = false;
    let mut close_manager = false;
    WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(window) = borrow.as_mut() else {
            return;
        };
        if window.availability_request != Some(request_id) {
            return;
        }
        window.availability_request = None;
        window.plugin_manager_card.set_busy(false);
        let locale = window.plugin_manager_card.locale();
        match (action, reply) {
            (
                AvailabilityAction::Refresh { preserve_error },
                AvailabilityReply::Current(Ok(installed)),
            ) => {
                window.plugin_manager_card.sync(installed);
                close_manager = !installed;
                if preserve_error {
                    if let Some(identity) = window.uncertain.as_mut() {
                        identity.current = Some(installed);
                        window.plugin_manager_card.report_error(Some(
                            availability_uncertain_message(locale, identity, identity.current),
                        ));
                    } else if let Some(error) = &window.availability_error {
                        window.plugin_manager_card.report_error(Some(error.clone()));
                    }
                } else {
                    window.plugin_manager_card.report_error(None);
                }
            }
            (AvailabilityAction::Open, AvailabilityReply::Current(Ok(installed))) => {
                window.plugin_manager_card.sync(installed);
                close_manager = !installed;
                if installed {
                    open = Some((
                        window.id,
                        window.paths.clone(),
                        window.socket.clone(),
                        locale,
                    ));
                }
            }
            (
                AvailabilityAction::Refresh { preserve_error },
                AvailabilityReply::Current(Err(error)),
            ) => {
                if preserve_error {
                    if let Some(identity) = &window.uncertain {
                        window.plugin_manager_card.report_error(Some(format!(
                            "{}; {error}",
                            availability_uncertain_message(locale, identity, identity.current),
                        )));
                    } else if let Some(previous) = &window.availability_error {
                        window
                            .plugin_manager_card
                            .report_error(Some(format!("{previous}; {error}")));
                    } else {
                        window.plugin_manager_card.report_error(Some(error));
                    }
                } else {
                    window.plugin_manager_card.report_error(Some(error));
                }
            }
            (AvailabilityAction::Open, AvailabilityReply::Current(Err(error))) => {
                window.plugin_manager_card.report_error(Some(error));
            }
            (
                AvailabilityAction::Set(_),
                AvailabilityReply::Commit(Ok(control::PluginManagerAvailabilityCommit::Committed(
                    installed,
                ))),
            ) => {
                window.plugin_manager_card.sync(installed);
                close_manager = !installed;
                window.plugin_manager_card.report_error(None);
            }
            (
                AvailabilityAction::Set(_),
                AvailabilityReply::Commit(Ok(
                    control::PluginManagerAvailabilityCommit::Uncertain {
                        instance_id,
                        operation_id,
                        current,
                    },
                )),
            ) => {
                let identity = UncertainAvailability {
                    instance_id,
                    operation_id,
                    current,
                };
                if let Some(installed) = current {
                    window.plugin_manager_card.sync(installed);
                }
                window
                    .plugin_manager_card
                    .report_error(Some(availability_uncertain_message(
                        locale, &identity, current,
                    )));
                window.uncertain = Some(identity);
                refresh_conflict = true;
            }
            (AvailabilityAction::Set(_), AvailabilityReply::Commit(Err(error))) => {
                refresh_conflict = error.contains("revision") || error.contains("conflict");
                window.availability_error = Some(error.clone());
                window.plugin_manager_card.report_error(Some(error));
            }
            _ => unreachable!("availability action and reply must match"),
        }
    });
    if close_manager {
        plugin_manager::close();
    }
    if refresh_conflict {
        request_availability(AvailabilityAction::Refresh {
            preserve_error: true,
        });
    }
    if let Some((id, paths, socket, locale)) = open {
        if let Some(mtm) = MainThreadMarker::new() {
            let result = plugin_manager::open(paths, socket, locale, mtm);
            if let Err(error) = result {
                WINDOW.with(|slot| {
                    if let Some(window) =
                        slot.borrow_mut().as_mut().filter(|window| window.id == id)
                    {
                        window.plugin_manager_card.report_error(Some(error));
                    }
                });
            }
        }
    }
}

pub(crate) fn run(paths: Paths, socket: PathBuf) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("settings must run on the AppKit main thread")?;
    let settings = control::get_lifecycle_settings(&paths)?;
    let preference = Preferences::load(&paths.config_dir)
        .unwrap_or_default()
        .language();
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
                NSSize::new(width, CARD_HEIGHT + PLUGIN_MANAGER_HEIGHT + 48.0),
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
    let plugin_manager_card = PluginManagerCard::new(locale, &target, mtm);
    plugin_manager_card.layout(NSRect::new(
        NSPoint::new(16.0, CARD_HEIGHT + 28.0),
        NSSize::new(width - 32.0, PLUGIN_MANAGER_HEIGHT),
    ));
    window
        .contentView()
        .ok_or("settings window has no content view")?
        .addSubview(card.view());
    window
        .contentView()
        .ok_or("settings window has no content view")?
        .addSubview(plugin_manager_card.view());
    WINDOW.with(|slot| {
        *slot.borrow_mut() = Some(Standalone {
            _window: window.clone(),
            _target: target,
            card,
            plugin_manager_card,
            socket,
            paths,
            settings,
            availability_request: None,
            id: NEXT_AVAILABILITY_REQUEST.fetch_add(1, Ordering::Relaxed),
            uncertain: None,
            confirming_removal: false,
            availability_error: None,
        })
    });
    request_availability(AvailabilityAction::Refresh {
        preserve_error: false,
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
