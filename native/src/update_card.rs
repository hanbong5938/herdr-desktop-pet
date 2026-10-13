use crate::i18n::{text, Message, UiLocale};
use crate::ui::MenuTarget;
use herdr_update_coordinator::{CheckStatus, ReleaseCandidate, UpdateAction};
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBezelStyle, NSBezierPath, NSBox, NSBoxType, NSButton, NSButtonType, NSColor, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSFont, NSLineBreakMode, NSSwitch, NSTextField,
    NSView,
};
use objc2_foundation::{
    NSDate, NSDateFormatter, NSDateFormatterStyle, NSLocale, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString, NSTimeZone,
};

// SAFETY: AppKit owns this decorative view and all calls are on the main thread.
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "OMPetAppUpdateCardView"]
    struct CardView;
    unsafe impl NSObjectProtocol for CardView {}
    impl CardView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let bounds = self.bounds();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.161, 0.161, 0.176, 1.0).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 10.0, 10.0).fill();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.28, 0.28, 0.32, 0.5).setStroke();
            let border = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                NSRect::new(
                    NSPoint::new(0.5, 0.5),
                    NSSize::new((bounds.size.width - 1.0).max(1.0), (bounds.size.height - 1.0).max(1.0)),
                ),
                9.5,
                9.5,
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

#[derive(Clone)]
pub(crate) struct UpdateCardModel {
    pub(crate) auto_check: bool,
    pub(crate) receive_beta_updates: bool,
    pub(crate) busy: bool,
    pub(crate) settings_locked: bool,
    // Tint only; status_text carries the precise localized operation/check outcome.
    pub(crate) status: CheckStatus,
    pub(crate) status_text: String,
    pub(crate) running_version: String,
    pub(crate) source: String,
    pub(crate) detail: String,
    pub(crate) history: String,
    pub(crate) diagnostics: String,
    pub(crate) last_checked: Option<u64>,
    pub(crate) action: Option<UpdateAction>,
    pub(crate) release: Option<ReleaseCandidate>,
    pub(crate) download_url: Option<String>,
}

pub(crate) struct AppUpdateCard {
    view: Retained<CardView>,
    heading: Retained<NSTextField>,
    auto_label: Retained<NSTextField>,
    auto_help: Retained<NSTextField>,
    auto_switch: Retained<NSSwitch>,
    beta_label: Retained<NSTextField>,
    beta_help: Retained<NSTextField>,
    beta_switch: Retained<NSSwitch>,
    first_separator: Retained<NSBox>,
    status_icon: Retained<NSTextField>,
    status: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    running: Retained<NSTextField>,
    source: Retained<NSTextField>,
    history: Retained<NSTextField>,
    diagnostics_toggle: Retained<NSButton>,
    diagnostics: Retained<NSTextField>,
    diagnostics_expanded: bool,
    second_separator: Retained<NSBox>,
    check: Retained<NSButton>,
    apply: Retained<NSButton>,
    download: Retained<NSButton>,
    third_separator: Retained<NSBox>,
    last_checked: Retained<NSTextField>,
    locale: UiLocale,
    // Keep the last presented data so language switches re-render the entire card.
    model: Option<UpdateCardModel>,
}

impl AppUpdateCard {
    pub(crate) fn new(locale: UiLocale, target: &MenuTarget, mtm: MainThreadMarker) -> Self {
        let view = CardView::new(mtm);
        unsafe {
            let _: () = msg_send![&*view, setAccessibilityElement: false];
        }
        let heading = label(12.5, true, false, mtm);
        let auto_label = label(12.0, true, false, mtm);
        let auto_help = label(10.5, false, true, mtm);
        let auto_switch: Retained<NSSwitch> = unsafe {
            msg_send![NSSwitch::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(46.0, 30.0))]
        };
        auto_switch.setControlSize(NSControlSize::Small);
        auto_switch.setRefusesFirstResponder(false);
        unsafe {
            auto_switch.setTarget(Some(target));
            auto_switch.setAction(Some(sel!(toggleUpdateAutoCheck:)));
        }
        let beta_label = label(12.0, true, false, mtm);
        let beta_help = label(10.5, false, true, mtm);
        let beta_switch: Retained<NSSwitch> = unsafe {
            msg_send![NSSwitch::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(46.0, 30.0))]
        };
        beta_switch.setControlSize(NSControlSize::Small);
        beta_switch.setRefusesFirstResponder(false);
        unsafe {
            beta_switch.setTarget(Some(target));
            beta_switch.setAction(Some(sel!(toggleUpdateBeta:)));
        }
        let first_separator = separator(mtm);
        let status_icon = label(13.0, true, false, mtm);
        unsafe {
            let _: () = msg_send![&*status_icon, setAccessibilityElement: false];
        }
        let status = label(11.5, true, false, mtm);
        let detail = label(10.5, false, true, mtm);
        let running = label(10.5, false, true, mtm);
        let source = label(10.5, false, true, mtm);
        let history = label(10.0, false, true, mtm);
        history.setHidden(true);
        let diagnostics_toggle = button(target, sel!(toggleUpdateDiagnostics:), mtm);
        diagnostics_toggle.setHidden(true);
        let diagnostics = label(10.0, false, true, mtm);
        diagnostics.setSelectable(true);
        diagnostics.setRefusesFirstResponder(false);
        diagnostics.setLineBreakMode(NSLineBreakMode::ByCharWrapping);
        diagnostics.setHidden(true);
        let second_separator = separator(mtm);
        let check = button(target, sel!(checkAppUpdate:), mtm);
        let apply = button(target, sel!(applyAppUpdate:), mtm);
        let download = button(target, sel!(downloadAppUpdate:), mtm);
        download.setHidden(true);
        let third_separator = separator(mtm);
        let last_checked = label(10.0, false, true, mtm);
        for child in [
            &*heading as &NSView,
            &*auto_label,
            &*auto_help,
            &*auto_switch,
            &*first_separator,
            &*beta_label,
            &*beta_help,
            &*beta_switch,
            &*status_icon,
            &*status,
            &*detail,
            &*running,
            &*source,
            &*history,
            &*diagnostics_toggle,
            &*diagnostics,
            &*second_separator,
            &*check,
            &*apply,
            &*download,
            &*third_separator,
            &*last_checked,
        ] {
            view.addSubview(child);
        }
        let mut card = Self {
            view,
            heading,
            auto_label,
            auto_help,
            auto_switch,
            beta_label,
            beta_help,
            beta_switch,
            first_separator,
            status_icon,
            status,
            detail,
            running,
            source,
            second_separator,
            history,
            diagnostics_toggle,
            diagnostics,
            diagnostics_expanded: false,
            check,
            apply,
            download,
            third_separator,
            last_checked,
            locale,
            model: None,
        };
        card.set_locale(locale);
        card
    }

    pub(crate) fn view(&self) -> &NSView {
        &self.view
    }
    pub(crate) fn diagnostics_toggle(&self) -> &NSButton {
        &self.diagnostics_toggle
    }

    pub(crate) fn diagnostics(&self) -> &NSTextField {
        &self.diagnostics
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.locale = locale;
        set_text(&self.heading, text(locale, Message::AppUpdateTitle));
        set_text(&self.auto_label, text(locale, Message::AppUpdateAutoCheck));
        set_text(
            &self.auto_help,
            text(locale, Message::AppUpdateAutomaticHelp),
        );
        set_text(
            &self.beta_label,
            text(locale, Message::AppUpdateReceiveBeta),
        );
        set_text(&self.beta_help, text(locale, Message::AppUpdateBetaHelp));
        self.check.setTitle(&NSString::from_str(text(
            locale,
            Message::AppUpdateCheckNow,
        )));
        self.download.setTitle(&NSString::from_str(text(
            locale,
            Message::AppUpdateDownloadRelease,
        )));
        accessibility(
            &self.download,
            text(locale, Message::AppUpdateDownloadRelease),
        );
        accessibility(&self.heading, text(locale, Message::AppUpdateTitle));
        accessibility(&self.auto_switch, text(locale, Message::AppUpdateAutoCheck));
        accessibility(
            &self.beta_switch,
            text(locale, Message::AppUpdateReceiveBeta),
        );
        accessibility(&self.beta_help, text(locale, Message::AppUpdateBetaHelp));
        accessibility_help(&self.beta_switch, text(locale, Message::AppUpdateBetaHelp));
        accessibility_help(
            &self.auto_switch,
            text(locale, Message::AppUpdateAutomaticHelp),
        );
        accessibility(
            &self.auto_help,
            text(locale, Message::AppUpdateAutomaticHelp),
        );
        accessibility(&self.check, text(locale, Message::AppUpdateCheckNow));
        if let Some(model) = self.model.clone() {
            self.refresh(&model);
        } else {
            self.render_status(
                &CheckStatus::NotChecked,
                text(self.locale, Message::AppUpdateNotChecked),
                "",
                "",
                "",
                None,
                None,
                None,
            );
        }
    }

    pub(crate) fn refresh(&mut self, model: &UpdateCardModel) {
        self.auto_switch.setState(if model.auto_check {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        self.beta_switch.setState(if model.receive_beta_updates {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        let check_title = text(
            self.locale,
            if model.last_checked.is_some() || model.status != CheckStatus::NotChecked {
                Message::AppUpdateCheckAgain
            } else {
                Message::AppUpdateCheckNow
            },
        );
        self.check.setTitle(&NSString::from_str(check_title));
        accessibility(&self.check, check_title);
        let busy = model.busy;
        self.auto_switch.setEnabled(!busy && !model.settings_locked);
        self.beta_switch.setEnabled(!busy && !model.settings_locked);
        self.check.setEnabled(!busy);
        self.render_status(
            &model.status,
            &model.status_text,
            &model.running_version,
            &model.source,
            &model.detail,
            model.last_checked,
            model.action.as_ref(),
            model.release.as_ref(),
        );
        set_text(&self.history, &model.history);
        self.history.setHidden(model.history.is_empty());
        accessibility(&self.history, &model.history);
        if self
            .model
            .as_ref()
            .is_none_or(|old| old.diagnostics != model.diagnostics)
        {
            set_text(&self.diagnostics, &model.diagnostics);
        }
        accessibility(&self.diagnostics, &model.diagnostics);
        self.diagnostics_toggle
            .setHidden(model.diagnostics.is_empty());
        self.diagnostics
            .setHidden(model.diagnostics.is_empty() || !self.diagnostics_expanded);
        self.localize_diagnostics_toggle();
        self.apply.setEnabled(!busy && model.action.is_some());
        self.download.setHidden(model.download_url.is_none());
        self.download
            .setEnabled(!busy && !model.settings_locked && model.download_url.is_some());
        self.model = Some(model.clone());
    }

    pub(crate) fn toggle_diagnostics(&mut self) {
        self.diagnostics_expanded = !self.diagnostics_expanded;
        self.diagnostics.setHidden(
            !self.diagnostics_expanded
                || self
                    .model
                    .as_ref()
                    .is_none_or(|model| model.diagnostics.is_empty()),
        );
        self.localize_diagnostics_toggle();
    }

    fn localize_diagnostics_toggle(&self) {
        let title = text(
            self.locale,
            if self.diagnostics_expanded {
                Message::AppUpdateHideDiagnostics
            } else {
                Message::AppUpdateShowDiagnostics
            },
        );
        self.diagnostics_toggle.setTitle(&NSString::from_str(title));
        accessibility(&self.diagnostics_toggle, title);
    }

    fn render_status(
        &self,
        status: &CheckStatus,
        status_text: &str,
        running_version: &str,
        source: &str,
        detail: &str,
        last_checked: Option<u64>,
        action: Option<&UpdateAction>,
        release: Option<&ReleaseCandidate>,
    ) {
        let (symbol, tint) = match status {
            CheckStatus::Current | CheckStatus::Installed | CheckStatus::Applied => {
                ("✓", NSColor::systemGreenColor())
            }
            CheckStatus::Available | CheckStatus::Local => ("●", NSColor::systemBlueColor()),
            CheckStatus::Failed | CheckStatus::Unsupported => ("!", NSColor::systemOrangeColor()),
            CheckStatus::NotChecked | CheckStatus::Checking => ("○", muted()),
        };
        set_text(&self.status_icon, symbol);
        self.status_icon.setTextColor(Some(&tint));
        set_text(&self.status, status_text);
        accessibility(&self.status, status_text);
        set_text(&self.detail, detail);
        self.detail.setHidden(detail.is_empty());
        let detail_tint = if *status == CheckStatus::Failed {
            NSColor::systemRedColor()
        } else {
            muted()
        };
        self.detail.setTextColor(Some(&detail_tint));
        accessibility(&self.detail, detail);
        let running = if running_version.is_empty() {
            String::new()
        } else if let Some(release) = release {
            format!(
                "{}: {} → {} ({})",
                text(self.locale, Message::AppUpdateRunning),
                running_version,
                release.version,
                release.tag,
            )
        } else {
            format!(
                "{}: {}",
                text(self.locale, Message::AppUpdateRunning),
                running_version
            )
        };
        let origin = if source.is_empty() {
            String::new()
        } else {
            format!(
                "{}: {}",
                text(self.locale, Message::AppUpdateSource),
                source
            )
        };
        set_text(&self.running, &running);
        set_text(&self.source, &origin);
        self.running.setHidden(running.is_empty());
        self.source.setHidden(origin.is_empty());
        accessibility(&self.running, &running);
        accessibility(&self.source, &origin);
        let action_message = match action {
            Some(UpdateAction::HerdrReinstall | UpdateAction::HomebrewUpgrade) => {
                Some(Message::AppUpdateInstall)
            }
            Some(UpdateAction::LocalRebuild) => Some(Message::AppUpdateRebuild),
            Some(UpdateAction::ApplyInstalled) => Some(Message::AppUpdateApply),
            None => None,
        };
        self.apply.setHidden(action_message.is_none());
        if let Some(message) = action_message {
            let title = text(self.locale, message);
            self.apply.setTitle(&NSString::from_str(title));
            accessibility(&self.apply, title);
        }
        let checked = match last_checked.and_then(|seconds| format_checked_at(seconds, self.locale))
        {
            Some(date) => format!(
                "{}: {}",
                text(self.locale, Message::AppUpdateLastChecked),
                date
            ),
            None => text(self.locale, Message::AppUpdateNeverChecked).to_owned(),
        };
        set_text(&self.last_checked, &checked);
        accessibility(&self.last_checked, &checked);
    }

    pub(crate) fn height(&self, width: f64) -> f64 {
        self.layout_contents(width, false)
    }

    pub(crate) fn layout(&self, frame: NSRect) {
        self.view.setFrame(frame);
        self.layout_contents(frame.size.width, true);
    }

    fn layout_contents(&self, width: f64, apply: bool) -> f64 {
        let full = (width - 28.0).max(1.0);
        let beside_switch = (width - 82.0).max(1.0);
        let mut y = 12.0;
        place(&self.heading, 14.0, full, 19.0, &mut y, apply);
        y += 5.0;
        let row_height = measured(&self.auto_label, beside_switch).max(30.0);
        if apply {
            self.auto_label
                .setFrame(rect(14.0, y, beside_switch, row_height));
            self.auto_switch
                .setFrame(rect((width - 58.0).max(0.0), y, 46.0, 30.0));
        }
        y += row_height + 3.0;
        place(&self.auto_help, 14.0, full, 14.0, &mut y, apply);
        let beta_row_height = measured(&self.beta_label, beside_switch).max(30.0);
        if apply {
            self.beta_label
                .setFrame(rect(14.0, y, beside_switch, beta_row_height));
            self.beta_switch
                .setFrame(rect((width - 58.0).max(0.0), y, 46.0, 30.0));
        }
        y += beta_row_height + 3.0;
        place(&self.beta_help, 14.0, full, 14.0, &mut y, apply);
        y += 7.0;
        if apply {
            self.first_separator.setFrame(rect(14.0, y, full, 1.0));
        }
        y += 6.0;
        let status_width = (full - 25.0).max(1.0);
        let status_height = measured(&self.status, status_width).max(18.0);
        if apply {
            self.status_icon.setFrame(rect(14.0, y, 19.0, 20.0));
            self.status
                .setFrame(rect(38.0, y, status_width, status_height));
        }
        y += status_height.max(20.0) + 3.0;
        for field in [&self.detail, &self.running, &self.source] {
            if !field.isHidden() {
                place(field, 38.0, (width - 52.0).max(1.0), 14.0, &mut y, apply);
                y += 3.0;
            }
        }
        y += 5.0;
        if apply {
            self.second_separator.setFrame(rect(14.0, y, full, 1.0));
        }
        y += 7.0;
        let check_width = button_width(&self.check, full);
        let apply_width = if self.apply.isHidden() {
            0.0
        } else {
            button_width(&self.apply, full)
        };
        let download_width = if self.download.isHidden() {
            0.0
        } else {
            button_width(&self.download, full)
        };
        let mut x = 14.0;
        let mut row_y = y;
        for (button, button_width) in [
            (&self.check, check_width),
            (&self.apply, apply_width),
            (&self.download, download_width),
        ] {
            if button_width == 0.0 {
                continue;
            }
            if x > 14.0 && x + button_width > 14.0 + full {
                x = 14.0;
                row_y += 31.0;
            }
            if apply {
                button.setFrame(rect(x, row_y, button_width, 25.0));
            }
            x += button_width + 8.0;
        }
        y = row_y + 25.0;
        y += 7.0;
        if apply {
            self.third_separator.setFrame(rect(14.0, y, full, 1.0));
        }
        y += 7.0;
        place(&self.last_checked, 14.0, full, 14.0, &mut y, apply);
        if !self.history.isHidden() {
            y += 5.0;
            place(&self.history, 14.0, full, 14.0, &mut y, apply);
        }
        if !self.diagnostics_toggle.isHidden() {
            y += 6.0;
            let toggle_width = button_width(&self.diagnostics_toggle, full);
            if apply {
                self.diagnostics_toggle
                    .setFrame(rect(14.0, y, toggle_width, 25.0));
            }
            y += 25.0;
            if !self.diagnostics.isHidden() {
                y += 5.0;
                place(&self.diagnostics, 14.0, full, 14.0, &mut y, apply);
            }
        }
        y + 14.0
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn place(field: &NSTextField, x: f64, width: f64, minimum: f64, y: &mut f64, apply: bool) {
    let height = measured(field, width).max(minimum);
    if apply {
        field.setFrame(rect(x, *y, width, height));
    }
    *y += height;
}

fn measured(field: &NSTextField, width: f64) -> f64 {
    field
        .cell()
        .expect("wrapping label cell")
        .cellSizeForBounds(rect(0.0, 0.0, width.max(1.0), 100_000.0))
        .height
        .ceil()
}

fn button_width(button: &NSButton, available: f64) -> f64 {
    (button.cell().expect("button cell").cellSize().width + 18.0)
        .max(82.0)
        .min(available)
}

fn separator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let line = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::default());
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.0);
    line.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.28, 0.28, 0.32, 0.35,
    ));
    unsafe {
        let _: () = msg_send![&*line, setAccessibilityElement: false];
    }
    line
}

fn label(size: f64, bold: bool, muted_color: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    let color = if muted_color {
        muted()
    } else {
        NSColor::colorWithSRGBRed_green_blue_alpha(0.95, 0.95, 0.97, 1.0)
    };
    field.setTextColor(Some(&color));
    field.setMaximumNumberOfLines(0);
    field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    field
}

fn muted() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(0.60, 0.61, 0.66, 1.0)
}

fn button(
    target: &MenuTarget,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, mtm)
    };
    button.setButtonType(NSButtonType::MomentaryPushIn);
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setBordered(true);
    button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    button.setContentTintColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.95, 0.95, 0.97, 1.0,
    )));
    button.setRefusesFirstResponder(false);
    unsafe {
        button.setTarget(Some(target));
        button.setAction(Some(action));
    }
    button
}

fn set_text(field: &NSTextField, value: &str) {
    field.setStringValue(&NSString::from_str(value));
}

fn accessibility(view: &NSView, value: &str) {
    let label = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityLabel: Some(&*label)];
    }
}
fn accessibility_help(view: &NSView, value: &str) {
    let help = NSString::from_str(value);
    unsafe {
        let _: () = msg_send![view, setAccessibilityHelp: Some(&*help)];
    }
}

fn format_checked_at(seconds: u64, locale: UiLocale) -> Option<String> {
    // A persisted UNIX timestamp is historical evidence, never synthesized from now.
    // NSDate uses local time zone, and the formatter's language follows the card.
    let seconds = i64::try_from(seconds).ok()?;
    let date = NSDate::dateWithTimeIntervalSince1970(seconds as f64);
    let formatter = NSDateFormatter::new();
    formatter.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(
        &NSString::from_str(locale.tag()),
    )));
    formatter.setTimeZone(Some(&NSTimeZone::localTimeZone()));
    formatter.setDateStyle(NSDateFormatterStyle::MediumStyle);
    formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
    Some(formatter.stringFromDate(&date).to_string())
}
