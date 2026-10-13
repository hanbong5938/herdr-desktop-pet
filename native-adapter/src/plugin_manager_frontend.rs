//! Herdr-owned optional-component presentation and availability controller.
use crate::plugin_manager::{self, PluginManagerCard, CARD_HEIGHT};
use crate::plugin_manager_control as control;
use crate::plugin_manager_i18n::{text, Message, UiLocale};
use desktop_pet_contracts::{PreferenceAccess, ProfilePaths, ProfileStore};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAppearance, NSAppearanceNameDarkAqua, NSBackingStoreType,
    NSFloatingWindowLevel, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

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
    window: Retained<NSWindow>,
    _target: Retained<SettingsTarget>,
    plugin_manager_card: PluginManagerCard,
    socket: PathBuf,
    paths: ProfilePaths,
    profile: ProfileStore,
    preferences: Arc<dyn PreferenceAccess>,
    presentation_generation: u64,
    availability_request: Option<u64>,
    id: u64,
    uncertain: Option<UncertainAvailability>,
    confirming_removal: bool,
    availability_error: Option<String>,
}
// SAFETY: All callbacks and retained native objects belong to the AppKit main thread.
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "DesktopPetHerdrManagerAvailabilityTarget"]
    #[ivars = u64]
    struct SettingsTarget;
    unsafe impl NSObjectProtocol for SettingsTarget {}
    unsafe impl NSWindowDelegate for SettingsTarget {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &objc2_foundation::NSNotification) {
            let id = *self.ivars();
            WINDOW.with(|slot| {
                if let Some(window) = slot.borrow_mut().as_mut().filter(|window| window.id == id) {
                    // Fence presentation only. The actual offered operation
                    // still completes into this hidden controller's evidence.
                    window.presentation_generation = window.presentation_generation.wrapping_add(1);
                }
            });
        }
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &objc2_foundation::NSNotification) {
            let id = *self.ivars();
            DispatchQueue::main().exec_async(move || {
                if WINDOW.with(|slot| slot.borrow().as_ref().is_some_and(|window| window.id == id && window.window.isVisible() && !window.confirming_removal)) {
                    request_availability(AvailabilityAction::Refresh { preserve_error: true });
                }
            });
        }
    }
    impl SettingsTarget {
        #[unsafe(method(openPluginManager:))]
        fn open_plugin_manager(&self, _sender: &AnyObject) { if self.is_current() { request_availability(AvailabilityAction::Open); } }
        #[unsafe(method(installPluginManager:))]
        fn install_plugin_manager(&self, _sender: &AnyObject) { if self.is_current() { request_availability(AvailabilityAction::Set(true)); } }
        #[unsafe(method(removePluginManager:))]
        fn remove_plugin_manager(&self, _sender: &AnyObject) { if self.is_current() { confirm_remove_plugin_manager(self.mtm()); } }
    }
);
impl SettingsTarget {
    fn is_current(&self) -> bool {
        WINDOW.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|window| window.id == *self.ivars() && window.window.isVisible())
        })
    }
}

fn confirm_remove_plugin_manager(mtm: MainThreadMarker) {
    let Some((id, generation, locale)) = WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let window = borrow.as_mut()?;
        if window.availability_request.is_some() || window.confirming_removal {
            return None;
        }
        window.confirming_removal = true;
        Some((
            window.id,
            window.presentation_generation,
            window.plugin_manager_card.locale(),
        ))
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
        window.presentation_generation == generation && window.window.isVisible()
    });
    if approved && current {
        request_availability(AvailabilityAction::Set(false));
    }
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
    let Some((preferences, request_id, generation)) = WINDOW.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let window = borrow.as_mut()?;
        if window.availability_request.is_some() {
            return None;
        }
        let request_id = NEXT_AVAILABILITY_REQUEST.fetch_add(1, Ordering::Relaxed);
        window.availability_request = Some(request_id);
        if matches!(action, AvailabilityAction::Set(_)) {
            window.uncertain = None;
            window.availability_error = None;
            window.plugin_manager_card.report_error(None);
        }
        window.plugin_manager_card.set_busy(true);
        Some((
            Arc::clone(&window.preferences),
            request_id,
            window.presentation_generation,
        ))
    }) else {
        return;
    };
    thread::spawn(move || {
        let reply = match action {
            AvailabilityAction::Refresh { .. } | AvailabilityAction::Open => {
                AvailabilityReply::Current(control::get_plugin_manager_availability(
                    preferences.as_ref(),
                ))
            }
            AvailabilityAction::Set(installed) => AvailabilityReply::Commit(
                control::set_plugin_manager_availability(preferences.as_ref(), installed),
            ),
        };
        DispatchQueue::main()
            .exec_async(move || complete_availability(request_id, generation, action, reply));
    });
}

fn complete_availability(
    request_id: u64,
    generation: u64,
    action: AvailabilityAction,
    reply: AvailabilityReply,
) {
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
                if installed
                    && window.window.isVisible()
                    && window.presentation_generation == generation
                {
                    open = Some((
                        window.id,
                        window.paths.clone(),
                        window.profile.clone(),
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
    if let Some((id, paths, profile, socket, locale)) = open {
        if let Some(mtm) = MainThreadMarker::new() {
            let result = plugin_manager::open(paths, profile, socket, locale, mtm);
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

pub(crate) fn activate(
    paths: ProfilePaths,
    profile: ProfileStore,
    socket: PathBuf,
    locale: UiLocale,
    key: &str,
    preferences: Arc<dyn PreferenceAccess>,
) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("manager actions require the AppKit main thread")?;
    let action = match key {
        "plugin_manager_open" => AvailabilityAction::Open,
        "plugin_manager_install" => AvailabilityAction::Set(true),
        "plugin_manager_remove" => AvailabilityAction::Set(false),
        _ => return Err("unsupported Herdr manager action".to_owned()),
    };
    profile.verify_owner()?;
    let installed = crate::settings::read_manager_installed(&profile)?;
    if matches!(
        action,
        AvailabilityAction::Open | AvailabilityAction::Set(false)
    ) && !installed
    {
        return Err("manager feature is not installed".to_owned());
    }
    if matches!(action, AvailabilityAction::Set(true)) && installed {
        return Err("manager feature is already installed".to_owned());
    }
    let same = WINDOW.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|window| window.paths == paths && window.socket == socket)
    });
    if !same {
        WINDOW.with(|slot| {
            if let Some(old) = slot.borrow_mut().take() {
                old.window.setDelegate(None);
                old.window.orderOut(None);
            }
        });
        let id = NEXT_AVAILABILITY_REQUEST.fetch_add(1, Ordering::Relaxed);
        let target: Retained<SettingsTarget> =
            unsafe { msg_send![super(SettingsTarget::alloc(mtm).set_ivars(id)), init] };
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(400.0, CARD_HEIGHT + 24.0),
                ),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setLevel(NSFloatingWindowLevel);
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            let _: () = unsafe { msg_send![&*window, setAppearance: Some(&*appearance)] };
        }
        window.setTitle(&NSString::from_str(text(
            locale,
            Message::PluginManagerTitle,
        )));
        window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let mut plugin_manager_card = PluginManagerCard::new(locale, &target, mtm);
        plugin_manager_card.sync(installed);
        plugin_manager_card.layout(NSRect::new(
            NSPoint::new(0.0, 12.0),
            NSSize::new(400.0, CARD_HEIGHT),
        ));
        window
            .contentView()
            .ok_or("manager availability window has no content view")?
            .addSubview(plugin_manager_card.view());
        window.center();
        WINDOW.with(|slot| {
            *slot.borrow_mut() = Some(Standalone {
                window,
                _target: target,
                plugin_manager_card,
                socket,
                paths,
                profile,
                preferences,
                presentation_generation: 0,
                availability_request: None,
                id,
                uncertain: None,
                confirming_removal: false,
                availability_error: None,
            })
        });
    }
    let busy = WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        let window = slot
            .as_mut()
            .expect("manager availability window initialized");
        window.plugin_manager_card.set_locale(locale);
        window.plugin_manager_card.sync(installed);
        window.window.makeKeyAndOrderFront(None);
        window.availability_request.is_some() || window.confirming_removal
    });
    if busy {
        return Err("manager availability request is already pending".to_owned());
    }
    if matches!(action, AvailabilityAction::Set(false)) {
        confirm_remove_plugin_manager(mtm);
    } else {
        request_availability(action);
    }
    Ok(())
}
