use crate::behavior::Presentation;
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub(crate) const BASE_WIDTH: f64 = 384.0;
pub(crate) const BASE_HEIGHT: f64 = 512.0;
const DISPLAY_INSET: f64 = 6.0;

/// Stable pack envelope in the UI's 384x512, top-left source coordinate space.
/// The renderer still uses its full canvas; only its view origin and containing
/// window change. Drawing, alpha sampling and semantic input share that frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DisplayGeometry {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl DisplayGeometry {
    pub(crate) fn new(
        bounds: (f64, f64, f64, f64), canvas: (u32, u32), png_motion: bool,
    ) -> Self {
        let (x0, y0, x1, y1) = bounds;
        // Packs need not have the UI's 3:4 aspect ratio. Match the existing
        // contain transform before removing both source padding and letterbox.
        let fit = (BASE_WIDTH / f64::from(canvas.0))
            .min(BASE_HEIGHT / f64::from(canvas.1));
        let width = f64::from(canvas.0) * fit;
        let height = f64::from(canvas.1) * fit;
        let left = (BASE_WIDTH - width) * 0.5;
        let top = (BASE_HEIGHT - height) * 0.5;
        let mut result = Self {
            left: left + x0 * width,
            top: top + y0 * height,
            right: left + x1 * width,
            bottom: top + y1 * height,
        };
        if png_motion {
            // Behavior's mutually exclusive presentations: scale 0.972..1.055,
            // x -7..7, y-up -11.5..3.5 (completion includes +/-1.5 bounce).
            // Bound both scale endpoints about the *canvas* center, not the crop.
            let edge = |value: f64, center: f64, minimum: bool| {
                let a = center + (value - center) * 0.972;
                let b = center + (value - center) * 1.055;
                if minimum { a.min(b) } else { a.max(b) }
            };
            result.left = edge(result.left, BASE_WIDTH * 0.5, true) - 7.0;
            result.right = edge(result.right, BASE_WIDTH * 0.5, false) + 7.0;
            result.top = edge(result.top, BASE_HEIGHT * 0.5, true) - 3.5;
            result.bottom = edge(result.bottom, BASE_HEIGHT * 0.5, false) + 11.5;
        }
        result
    }

    pub(crate) fn size(self, scale: f64) -> NSSize {
        NSSize::new(
            (self.width() * scale + DISPLAY_INSET * 2.0).ceil(),
            (self.height() * scale + DISPLAY_INSET * 2.0).ceil(),
        )
    }

    pub(crate) fn width(self) -> f64 {
        self.right - self.left
    }

    pub(crate) fn height(self) -> f64 {
        self.bottom - self.top
    }

    pub(crate) fn canvas_frame(self, scale: f64) -> NSRect {
        NSRect::new(
            NSPoint::new(
                DISPLAY_INSET - self.left * scale,
                DISPLAY_INSET - (BASE_HEIGHT - self.bottom) * scale,
            ),
            NSSize::new(BASE_WIDTH * scale, BASE_HEIGHT * scale),
        )
    }

    pub(crate) fn presented_frame(self, scale: f64, presentation: Presentation) -> NSRect {
        let canvas = self.canvas_frame(scale);
        let size = NSSize::new(
            canvas.size.width * presentation.scale,
            canvas.size.height * presentation.scale,
        );
        NSRect::new(
            NSPoint::new(
                canvas.origin.x + (canvas.size.width - size.width) * 0.5
                    + presentation.offset_x * scale,
                canvas.origin.y + (canvas.size.height - size.height) * 0.5
                    + presentation.offset_y * scale,
            ),
            size,
        )
    }

    pub(crate) fn window_origin(self, canvas_origin: NSPoint, scale: f64) -> NSPoint {
        let local = self.canvas_frame(scale).origin;
        NSPoint::new(canvas_origin.x - local.x, canvas_origin.y - local.y)
    }

    pub(crate) fn resize_delta(self, dx: f64, dy: f64) -> f64 {
        let horizontal = dx / self.width();
        let vertical = -dy / self.height();
        if horizontal.abs() >= vertical.abs() { horizontal } else { vertical }
    }

    pub(crate) fn fitting_scale(self, available_width: f64, available_height: f64) -> f64 {
        ((available_width - DISPLAY_INSET * 2.0) / self.width())
            .min((available_height - DISPLAY_INSET * 2.0) / self.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presentation(x: f64, y: f64, scale: f64) -> Presentation {
        Presentation {
            offset_x: x, offset_y: y, scale,
            dialogue: None, animate: false, effect: None,
        }
    }

    #[test]
    fn crop_preserves_source_position_and_scale_for_every_size() {
        let geometry = DisplayGeometry::new((0.2, 0.15, 0.85, 0.9), (384, 512), false);
        let old_origin = NSPoint::new(-540.0, 120.0);
        for scale in [0.35, 0.65, 0.7057291666666666, 1.0, 1.25] {
            let origin = geometry.window_origin(old_origin, scale);
            let canvas = geometry.canvas_frame(scale);
            for source in [NSPoint::new(0.0, 0.0), NSPoint::new(192.0, 340.0)] {
                assert!((origin.x + canvas.origin.x + source.x * scale
                    - (old_origin.x + source.x * scale)).abs() < 1e-9);
                assert!((origin.y + canvas.origin.y + source.y * scale
                    - (old_origin.y + source.y * scale)).abs() < 1e-9);
            }
            let size = geometry.size(scale);
            assert!((size.width - geometry.width() * scale - 12.0).abs() < 1.0);
            assert!((size.height - geometry.height() * scale - 12.0).abs() < 1.0);
        }
    }

    #[test]
    fn png_motion_extremes_remain_inside_stable_window() {
        let bounds = (0.2, 0.15, 0.85, 0.9);
        let geometry = DisplayGeometry::new(bounds, (384, 512), true);
        for scale in [0.35, 0.65, 1.25] {
            let window = geometry.size(scale);
            for transform_scale in [0.972, 1.055] {
                for x in [-7.0, 7.0] {
                    for y in [-11.5, 3.5] {
                        let frame = geometry.presented_frame(scale, presentation(x, y, transform_scale));
                        let left = frame.origin.x + frame.size.width * bounds.0;
                        let right = frame.origin.x + frame.size.width * bounds.2;
                        let bottom = frame.origin.y + frame.size.height * (1.0 - bounds.3);
                        let top = frame.origin.y + frame.size.height * (1.0 - bounds.1);
                        assert!(left >= 6.0 - 1e-9 && bottom >= 6.0 - 1e-9);
                        assert!(right <= window.width - 6.0 + 1e-9);
                        assert!(top <= window.height - 6.0 + 1e-9);
                    }
                }
            }
        }
    }

    #[test]
    fn resize_delta_and_screen_limit_use_envelope_not_canvas() {
        let geometry = DisplayGeometry::new((0.25, 0.25, 0.75, 0.75), (384, 512), false);
        assert!((geometry.resize_delta(19.2, 0.0) - 0.1).abs() < 1e-9);
        assert!((geometry.resize_delta(0.0, -25.6) - 0.1).abs() < 1e-9);
        let scale = geometry.fitting_scale(300.0, 400.0);
        let size = geometry.size(scale);
        assert!(size.width <= 300.0 && size.height <= 400.0);
        assert!((size.width - 300.0).abs() < 1e-9);
    }

    #[test]
    fn square_and_wide_source_letterbox_is_not_reserved_in_window() {
        for (canvas, fitted) in [
            ((1024, 1024), NSSize::new(384.0, 384.0)),
            ((1024, 512), NSSize::new(384.0, 192.0)),
            ((1024, 2048), NSSize::new(256.0, 512.0)),
        ] {
            let geometry = DisplayGeometry::new((0.0, 0.0, 1.0, 1.0), canvas, false);
            for scale in [0.35, 0.65, 1.25] {
                let size = geometry.size(scale);
                assert_eq!(size.width, (fitted.width * scale + 12.0).ceil());
                assert_eq!(size.height, (fitted.height * scale + 12.0).ceil());
                let view = geometry.canvas_frame(scale);
                let left = view.origin.x + (BASE_WIDTH - fitted.width) * scale * 0.5;
                let bottom = view.origin.y + (BASE_HEIGHT - fitted.height) * scale * 0.5;
                assert!((left - 6.0).abs() < 1e-9);
                assert!((bottom - 6.0).abs() < 1e-9);
            }
        }
    }
}
