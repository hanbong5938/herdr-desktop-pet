use crate::session_view::DisplayStatus;
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSColor, NSFontWeightRegular, NSImage, NSImageScaling, NSImageSymbolConfiguration, NSImageView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};

pub(crate) const STATUS_ICON_SIZE: f64 = 13.0;
pub(crate) const STATUS_ICON_GAP: f64 = 4.0;
pub(crate) const STATUS_ROW_HEIGHT: f64 = 18.0;

const MIN_CONTRAST: f64 = 4.5;
#[cfg(test)]
const STATUS_COUNT: usize = 10;
const SYMBOL_COUNT: usize = 9;

type Rgb = (f64, f64, f64);

thread_local! {
    // NSImage is AppKit-owned and must not be shared across threads. Every
    // StatusIcon on the main thread borrows the same lazily populated symbols.
    static SYMBOL_IMAGES: RefCell<[Option<Retained<NSImage>>; SYMBOL_COUNT]> =
        const { RefCell::new([const { None }; SYMBOL_COUNT]) };
}

fn channel_luminance(channel: f64) -> f64 {
    let channel = if channel.is_finite() {
        channel.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance((r, g, b): Rgb) -> f64 {
    0.2126 * channel_luminance(r) + 0.7152 * channel_luminance(g) + 0.0722 * channel_luminance(b)
}

fn contrast(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn blend(from: Rgb, to: f64, fraction: f64) -> Rgb {
    (
        from.0 + (to - from.0) * fraction,
        from.1 + (to - from.1) * fraction,
        from.2 + (to - from.2) * fraction,
    )
}

/// Return a semantic color meeting WCAG AA text contrast on the actual opaque surface.
/// Adjust only when the original hue cannot meet that threshold.
pub(crate) fn semantic_rgb(status: DisplayStatus, background: Rgb) -> Rgb {
    let surface = luminance(background);
    let light_surface = contrast(surface, 0.0) >= contrast(surface, 1.0);
    let base = match (status, light_surface) {
        (DisplayStatus::Running, true) => (0x0B, 0x62, 0xC4),
        (DisplayStatus::Running, false) => (0x58, 0xA6, 0xFF),
        (DisplayStatus::Waiting, true) => (0xA3, 0x5C, 0x00),
        (DisplayStatus::Waiting, false) => (0xE3, 0xB3, 0x41),
        (DisplayStatus::Succeeded, true) => (0x13, 0x7A, 0x38),
        (DisplayStatus::Succeeded, false) => (0x3F, 0xB9, 0x50),
        (DisplayStatus::Failed, true) => (0xC9, 0x2A, 0x2A),
        (DisplayStatus::Failed, false) => (0xF8, 0x51, 0x49),
        (DisplayStatus::Idle | DisplayStatus::Cancelled | DisplayStatus::Completed, true) => {
            (0x4A, 0x4D, 0x68)
        }
        (DisplayStatus::Idle | DisplayStatus::Cancelled | DisplayStatus::Completed, false) => {
            (0xC9, 0xD1, 0xD9)
        }
        (DisplayStatus::NoSessions | DisplayStatus::Unknown | DisplayStatus::Offline, true) => {
            (0x57, 0x60, 0x6A)
        }
        (DisplayStatus::NoSessions | DisplayStatus::Unknown | DisplayStatus::Offline, false) => {
            (0x8B, 0x94, 0x9E)
        }
    };
    let base = (
        base.0 as f64 / 255.0,
        base.1 as f64 / 255.0,
        base.2 as f64 / 255.0,
    );
    if contrast(luminance(base), surface) >= MIN_CONTRAST {
        return base;
    }

    // An endpoint with the greater reachable contrast always exceeds 4.5:1
    // for any opaque sRGB surface. In particular, #AAAAAA must move toward
    // black, not white, despite being below half linear luminance.
    let endpoint = if contrast(surface, 0.0) >= contrast(surface, 1.0) {
        0.0
    } else {
        1.0
    };
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..32 {
        let mid = (low + high) / 2.0;
        if contrast(luminance(blend(base, endpoint, mid)), surface) >= MIN_CONTRAST {
            high = mid;
        } else {
            low = mid;
        }
    }
    blend(base, endpoint, high)
}

pub(crate) fn semantic_color(status: DisplayStatus, background: Rgb) -> Retained<NSColor> {
    let (r, g, b) = semantic_rgb(status, background);
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}

fn symbol(status: DisplayStatus) -> (&'static str, usize) {
    match status {
        DisplayStatus::NoSessions => ("questionmark.circle", 0),
        DisplayStatus::Idle => ("pause.circle", 1),
        DisplayStatus::Running => ("arrow.triangle.2.circlepath", 2),
        DisplayStatus::Waiting => ("exclamationmark.triangle.fill", 3),
        DisplayStatus::Succeeded => ("checkmark.circle.fill", 4),
        DisplayStatus::Failed => ("xmark.circle.fill", 5),
        DisplayStatus::Cancelled => ("slash.circle", 6),
        DisplayStatus::Completed => ("circle.dashed", 7),
        DisplayStatus::Unknown => ("questionmark.circle", 0),
        DisplayStatus::Offline => ("wifi.slash", 8),
    }
}

pub(crate) struct StatusIcon {
    view: Retained<NSImageView>,
    last: Cell<Option<(DisplayStatus, Rgb)>>,
}

impl StatusIcon {
    pub(crate) fn new(mtm: MainThreadMarker) -> Self {
        let view = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(STATUS_ICON_SIZE, STATUS_ICON_SIZE),
            ),
        );
        view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
        view.setEditable(false);
        // SAFETY: An icon beside a labelled status is purely decorative.
        unsafe {
            let _: () = msg_send![&*view, setAccessibilityElement: false];
            // AppKit exposes the image cell even when its control is ignored.
            if let Some(cell) = view.cell() {
                let _: () = msg_send![&*cell, setAccessibilityElement: false];
            }
            view.setSymbolConfiguration(Some(
                &NSImageSymbolConfiguration::configurationWithPointSize_weight(
                    STATUS_ICON_SIZE,
                    NSFontWeightRegular,
                ),
            ));
        }
        Self {
            view,
            last: Cell::new(None),
        }
    }

    pub(crate) fn view(&self) -> &NSImageView {
        &self.view
    }

    pub(crate) fn update(&self, status: DisplayStatus, background: Rgb) {
        MainThreadMarker::new().expect("status icons must update on the AppKit main thread");
        if self.last.get() == Some((status, background)) {
            return;
        }
        let (name, index) = symbol(status);
        SYMBOL_IMAGES.with(|images| {
            let mut images = images.borrow_mut();
            if images[index].is_none() {
                images[index] = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(name),
                    None,
                );
            }
            self.view.setImage(images[index].as_deref());
        });
        self.view
            .setContentTintColor(Some(&semantic_color(status, background)));
        self.last.set(Some((status, background)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUSES: [DisplayStatus; STATUS_COUNT] = [
        DisplayStatus::NoSessions,
        DisplayStatus::Idle,
        DisplayStatus::Running,
        DisplayStatus::Waiting,
        DisplayStatus::Succeeded,
        DisplayStatus::Failed,
        DisplayStatus::Cancelled,
        DisplayStatus::Completed,
        DisplayStatus::Unknown,
        DisplayStatus::Offline,
    ];

    #[test]
    fn semantic_colors_meet_text_contrast_on_actual_and_custom_surfaces() {
        let surfaces = [
            (0xF5, 0xEE, 0xE5), // light preset
            (0xF0, 0xE1, 0xE6), // pink preset
            (0x29, 0x29, 0x36), // dark preset
            (0xAA, 0xAA, 0xAA), // white cannot reach 4.5 here
            (0x77, 0x77, 0x77), // medium gray
            (0x00, 0xEF, 0x20), // bright green
            (0x00, 0x00, 0x00),
            (0xFF, 0xFF, 0xFF),
        ];
        for surface in surfaces {
            let background = (
                surface.0 as f64 / 255.0,
                surface.1 as f64 / 255.0,
                surface.2 as f64 / 255.0,
            );
            for status in STATUSES {
                let foreground = semantic_rgb(status, background);
                assert!(
                    [foreground.0, foreground.1, foreground.2]
                        .iter()
                        .all(|channel| (0.0..=1.0).contains(channel)),
                    "out of gamut: {status:?} on {surface:?}: {foreground:?}"
                );
                assert!(
                    contrast(luminance(foreground), luminance(background)) >= MIN_CONTRAST,
                    "insufficient contrast: {status:?} on {surface:?}: {foreground:?}"
                );
            }
        }
    }

    #[test]
    fn mid_gray_selects_reachable_endpoint() {
        let background = (
            0xAA as f64 / 255.0,
            0xAA as f64 / 255.0,
            0xAA as f64 / 255.0,
        );
        let foreground = semantic_rgb(DisplayStatus::Succeeded, background);
        assert!(luminance(foreground) < luminance(background));
        assert!(contrast(luminance(foreground), luminance(background)) >= MIN_CONTRAST);
    }
}
