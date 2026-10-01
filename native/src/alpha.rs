#[derive(Debug)]
pub(crate) struct AlphaMask {
    width: u32,
    height: u32,
    /// One byte per source pixel: non-zero means alpha >= 8/255.
    opaque: Vec<u8>,
    bounds: Option<(u32, u32, u32, u32)>,
    /// Pixel-edge display coverage at alpha >= 1, independent of hit policy.
    display_bounds: Option<(u32, u32, u32, u32)>,
}

const ALPHA_THRESHOLD: u8 = 8;

impl AlphaMask {
    /// Builds an immutable hit mask from normalized, row-major RGBA8 pixels.
    pub(crate) fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self, String> {
        Self::from_alpha_channel(width, height, rgba, 4, 3)
    }

    /// Builds a mask from a normalized output channel without making an intermediate RGBA copy.
    pub(crate) fn from_alpha_channel(
        width: u32,
        height: u32,
        pixels: &[u8],
        channels: usize,
        alpha_channel: usize,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("alpha mask dimensions must be non-zero".to_string());
        }
        if channels == 0 || alpha_channel >= channels {
            return Err("alpha mask channel layout is invalid".to_string());
        }
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| "alpha mask pixel count overflows usize".to_string())?;
        let expected_len = pixel_count
            .checked_mul(channels)
            .ok_or_else(|| "alpha mask byte count overflows usize".to_string())?;
        if pixels.len() != expected_len {
            return Err(format!(
                "alpha mask pixel data has {} bytes; expected {expected_len}",
                pixels.len()
            ));
        }
        let mut opaque = Vec::with_capacity(pixel_count);
        let mut bounds = None;
        let mut display_bounds = None;
        for (index, pixel) in pixels.chunks_exact(channels).enumerate() {
            let alpha = pixel[alpha_channel];
            let visible = alpha >= ALPHA_THRESHOLD;
            opaque.push(u8::from(visible));
            if alpha != 0 {
                extend_bounds(
                    &mut display_bounds,
                    (index % width as usize) as u32,
                    (index / width as usize) as u32,
                );
            }
            if visible {
                extend_bounds(
                    &mut bounds,
                    (index % width as usize) as u32,
                    (index / width as usize) as u32,
                );
            }
        }
        Ok(Self {
            width,
            height,
            opaque,
            bounds,
            display_bounds,
        })
    }
    /// Builds a fully opaque mask for PNG formats without an alpha channel.
    pub(crate) fn fully_opaque(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("alpha mask dimensions must be non-zero".to_string());
        }
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| "alpha mask pixel count overflows usize".to_string())?;
        Ok(Self {
            width,
            height,
            opaque: vec![1; pixel_count],
            bounds: Some((0, 0, width, height)),
            display_bounds: Some((0, 0, width, height)),
        })
    }
    /// Normalized top-left pixel-edge coverage, including faint antialiasing.
    pub(crate) fn display_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        self.display_bounds.map(|(x0, y0, x1, y1)| {
            (
                f64::from(x0) / f64::from(self.width),
                f64::from(y0) / f64::from(self.height),
                f64::from(x1) / f64::from(self.width),
                f64::from(y1) / f64::from(self.height),
            )
        })
    }

    /// Pixel-edge bounds in normalized top-left coordinates. A head region is
    /// intersected with actual opaque pixels; an empty intersection yields None.
    /// The whole-art bounds are cached during decode, so fallback never scans.
    pub(crate) fn visible_bounds(
        &self,
        region: Option<(f64, f64, f64, f64)>,
    ) -> Option<(f64, f64, f64, f64)> {
        let bounds = if let Some((x0, y0, x1, y1)) = region {
            if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) || x0 >= x1 || y0 >= y1 {
                return None;
            }
            let min_x = x0.max(0.0).floor() as u32;
            let min_y = y0.max(0.0).floor() as u32;
            let max_x = x1.max(0.0).ceil().min(f64::from(self.width)) as u32;
            let max_y = y1.max(0.0).ceil().min(f64::from(self.height)) as u32;
            let mut bounds = None;
            for row in min_y.min(self.height)..max_y {
                for column in min_x.min(self.width)..max_x {
                    let index = row as usize * self.width as usize + column as usize;
                    if self.opaque[index] != 0
                        && f64::from(column) + 0.5 >= x0
                        && f64::from(column) + 0.5 < x1
                        && f64::from(row) + 0.5 >= y0
                        && f64::from(row) + 0.5 < y1
                    {
                        extend_bounds(&mut bounds, column, row);
                    }
                }
            }
            bounds?
        } else {
            self.bounds?
        };
        Some((
            f64::from(bounds.0) / f64::from(self.width),
            f64::from(bounds.1) / f64::from(self.height),
            f64::from(bounds.2) / f64::from(self.width),
            f64::from(bounds.3) / f64::from(self.height),
        ))
    }

    /// Tests an AppKit logical point against an aspect-fitted source image.
    ///
    /// Coordinates are bottom-left logical points. The source PNG is row-major
    /// top-to-bottom, so the fitted y coordinate is inverted before indexing.
    /// Frame and point boundaries are half-open: the right and top edges miss.
    pub(crate) fn sample(
        &self,
        x: f64,
        y: f64,
        frame: (f64, f64, f64, f64),
    ) -> Option<(bool, f64, f64)> {
        let (frame_x, frame_y, frame_width, frame_height) = frame;
        if !x.is_finite()
            || !y.is_finite()
            || !frame_x.is_finite()
            || !frame_y.is_finite()
            || !frame_width.is_finite()
            || !frame_height.is_finite()
            || frame_width <= 0.0
            || frame_height <= 0.0
        {
            return None;
        }

        let frame_right = frame_x + frame_width;
        let frame_top = frame_y + frame_height;
        if !frame_right.is_finite() || !frame_top.is_finite() {
            return None;
        }
        if x < frame_x || x >= frame_right || y < frame_y || y >= frame_top {
            return None;
        }

        let source_width = f64::from(self.width);
        let source_height = f64::from(self.height);
        let scale = (frame_width / source_width).min(frame_height / source_height);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let fitted_width = source_width * scale;
        let fitted_height = source_height * scale;
        let fitted_x = frame_x + (frame_width - fitted_width) * 0.5;
        let fitted_y = frame_y + (frame_height - fitted_height) * 0.5;
        if !fitted_width.is_finite()
            || !fitted_height.is_finite()
            || fitted_width <= 0.0
            || fitted_height <= 0.0
            || !fitted_x.is_finite()
            || !fitted_y.is_finite()
        {
            return None;
        }

        let fitted_right = fitted_x + fitted_width;
        let fitted_top = fitted_y + fitted_height;
        if !fitted_right.is_finite()
            || !fitted_top.is_finite()
            || x < fitted_x
            || x >= fitted_right
            || y < fitted_y
            || y >= fitted_top
        {
            return None;
        }

        let u = (x - fitted_x) / fitted_width;
        let v = (y - fitted_y) / fitted_height;
        if !u.is_finite() || !v.is_finite() || u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0 {
            return None;
        }
        let column = (u * source_width).floor() as u32;
        let row_from_bottom = (v * source_height).floor() as u32;
        if column >= self.width || row_from_bottom >= self.height {
            return None;
        }
        let row_from_top = self.height - 1 - row_from_bottom;
        let index = usize::try_from(row_from_top)
            .ok()
            .and_then(|row| {
                usize::try_from(self.width)
                    .ok()
                    .and_then(|width| row.checked_mul(width))
            })
            .and_then(|base| base.checked_add(usize::try_from(column).ok()?));
        Some((
            *self.opaque.get(index?)? != 0,
            f64::from(column) + 0.5,
            f64::from(row_from_top) + 0.5,
        ))
    }
}
fn extend_bounds(bounds: &mut Option<(u32, u32, u32, u32)>, x: u32, y: u32) {
    match bounds {
        Some((left, top, right, bottom)) => {
            *left = (*left).min(x);
            *top = (*top).min(y);
            *right = (*right).max(x + 1);
            *bottom = (*bottom).max(y + 1);
        }
        None => *bounds = Some((x, y, x + 1, y + 1)),
    }
}

#[cfg(test)]
pub(crate) fn rgba_alpha_index(width: usize, x: usize, y: usize) -> usize {
    (y * width + x) * 4 + 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_and_transparent_hole_are_preserved() {
        let rgba = [
            0, 0, 0, 255, 0, 0, 0, 7, // PNG top row
            0, 0, 0, 8, 0, 0, 0, 0, // PNG bottom row
        ];
        let mask = AlphaMask::from_rgba(2, 2, &rgba).expect("valid RGBA mask");
        let frame = (0.0, 0.0, 2.0, 2.0);
        assert_eq!(mask.sample(0.5, 0.5, frame), Some((true, 0.5, 1.5))); // alpha 8, PNG bottom-left
        assert_eq!(mask.sample(1.5, 0.5, frame), Some((false, 1.5, 1.5))); // transparent hole
        assert_eq!(mask.sample(0.5, 1.5, frame), Some((true, 0.5, 0.5))); // PNG top-left
        assert_eq!(mask.sample(1.5, 1.5, frame), Some((false, 1.5, 0.5))); // alpha 7
    }
    #[test]
    fn faint_display_pixels_extend_coverage_without_changing_hit_threshold() {
        let rgba = [
            0, 0, 0, 1, 0, 0, 0, 0, // top row
            0, 0, 0, 8, 0, 0, 0, 7, // bottom row
        ];
        let mask = AlphaMask::from_rgba(2, 2, &rgba).expect("valid RGBA mask");
        assert_eq!(mask.display_bounds(), Some((0.0, 0.0, 1.0, 1.0)));
        assert_eq!(mask.visible_bounds(None), Some((0.0, 0.5, 0.5, 1.0)));
        assert_eq!(mask.sample(0.5, 1.5, (0.0, 0.0, 2.0, 2.0)), Some((false, 0.5, 0.5)));
    }

    #[test]
    fn head_bounds_intersect_visible_pixels_and_fallback_excludes_padding() {
        let mut rgba = [0_u8; 4 * 4 * 4];
        rgba[rgba_alpha_index(4, 2, 1)] = 255;
        rgba[rgba_alpha_index(4, 1, 3)] = 255;
        let mask = AlphaMask::from_rgba(4, 4, &rgba).expect("valid alpha");
        assert_eq!(
            mask.visible_bounds(Some((2.0, 1.0, 3.0, 2.0))),
            Some((0.5, 0.25, 0.75, 0.5))
        );
        assert_eq!(mask.visible_bounds(Some((0.0, 0.0, 1.0, 1.0))), None);
        assert_eq!(mask.visible_bounds(None), Some((0.25, 0.25, 0.75, 1.0)));
    }

    #[test]
    fn aspect_fit_letterbox_and_edges_are_not_hits() {
        let rgba = [0, 0, 0, 255, 0, 0, 0, 0];
        let mask = AlphaMask::from_rgba(2, 1, &rgba).expect("valid RGBA mask");
        let frame = (10.0, 20.0, 4.0, 4.0);
        assert_eq!(mask.sample(11.0, 20.5, frame), None); // outside fitted image
        assert_eq!(mask.sample(10.5, 21.5, frame), Some((true, 0.5, 0.5)));
        assert_eq!(mask.sample(12.0, 21.5, frame), Some((false, 1.5, 0.5)));
        assert_eq!(mask.sample(10.5, 23.0, frame), None); // fitted image top edge
        assert_eq!(mask.sample(14.0, 22.0, frame), None); // frame right edge
    }

    #[test]
    fn invalid_geometry_is_rejected_without_panicking() {
        let mask = AlphaMask::from_rgba(1, 1, &[0, 0, 0, 255]).expect("valid RGBA mask");
        assert!(mask.sample(f64::NAN, 0.0, (0.0, 0.0, 1.0, 1.0)).is_none());
        assert!(mask.sample(0.0, 0.0, (0.0, 0.0, 0.0, 1.0)).is_none());
        assert!(mask
            .sample(0.0, 0.0, (f64::MAX, 0.0, f64::MAX, 1.0))
            .is_none());
    }
}
