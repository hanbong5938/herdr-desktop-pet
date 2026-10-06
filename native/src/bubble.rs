use serde::{Deserialize, Serialize};

pub(crate) const BUBBLE_WINDOW_INSET: f64 = 12.0;
pub(crate) const BUBBLE_RADIUS: f64 = 15.0;
pub(crate) const TAIL_BASE_WIDTH: f64 = 14.0;
pub(crate) const TAIL_DEPTH: f64 = 8.0;

const BUBBLE_GAP: f64 = 8.0;
const OUTER_GAP: f64 = 4.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BubblePlacement {
    #[default]
    Above,
    Below,
    Left,
    Right,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
/// Maps a normalized top-left image-space box into an AppKit bottom-left rect.
/// Rejects unbounded or degenerate input rather than passing invalid geometry
/// to placement. The image box must describe visible pixels, not canvas bounds.
pub(crate) fn screen_rect_for_image_bounds(
    frame: Rect,
    bounds: (f64, f64, f64, f64),
) -> Option<Rect> {
    let (x0, y0, x1, y1) = bounds;
    if ![frame.x, frame.y, frame.width, frame.height, x0, y0, x1, y1]
        .iter()
        .all(|value| value.is_finite())
        || frame.width <= 0.0
        || frame.height <= 0.0
        || !(0.0..=1.0).contains(&x0)
        || !(0.0..=1.0).contains(&y0)
        || !(0.0..=1.0).contains(&x1)
        || !(0.0..=1.0).contains(&y1)
        || x0 >= x1
        || y0 >= y1
    {
        return None;
    }
    let result = Rect {
        x: frame.x + frame.width * x0,
        y: frame.y + frame.height * (1.0 - y1),
        width: frame.width * (x1 - x0),
        height: frame.height * (y1 - y0),
    };
    let right = result.x + result.width;
    let top = result.y + result.height;
    if [result.x, result.y, result.width, result.height]
        .iter()
        .all(|value| value.is_finite())
        && result.width > 0.0
        && result.height > 0.0
        && right.is_finite()
        && top.is_finite()
        && right > result.x
        && top > result.y
    {
        Some(result)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BubbleGeometry {
    pub window: Rect,
    pub body: Rect,
    pub tail: Option<TailGeometry>,
    pub side: Option<BubbleSide>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TailGeometry {
    pub base_start: (f64, f64),
    pub base_end: (f64, f64),
    pub tip: (f64, f64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BubbleSide {
    Above,
    Below,
    Left,
    Right,
}

impl BubbleSide {
    fn candidate_order(preferred: BubblePlacement) -> [Self; 4] {
        match preferred {
            BubblePlacement::Above => [Self::Above, Self::Below, Self::Left, Self::Right],
            BubblePlacement::Below => [Self::Below, Self::Above, Self::Left, Self::Right],
            BubblePlacement::Left => [Self::Left, Self::Right, Self::Above, Self::Below],
            BubblePlacement::Right => [Self::Right, Self::Left, Self::Above, Self::Below],
            BubblePlacement::Auto => [Self::Above, Self::Below, Self::Left, Self::Right],
        }
    }

    fn has_horizontal_tail(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// Places a bubble outside the pet, preferring the requested side.
///
/// `body_size` is the measured rounded body, excluding the tail and the
/// transparent/stroke/shadow margin reserved by the returned window. The
/// returned `window` is in screen coordinates; `body` and `tail` are local
/// bottom-left coordinates inside that window.
///
/// Placement is selected in two passes. A candidate that fits without any
/// clamp wins before a candidate that needs to be clamped, which preserves an
/// external alternate side when the preferred side is blocked by an edge.
/// If no side can remain external after clamping, the bounded body is returned
/// without a potentially misleading tail.
pub(crate) fn place_bubble(
    pet: Rect,
    body_size: (f64, f64),
    visible: Rect,
    placement: BubblePlacement,
) -> BubbleGeometry {
    let pet = normalize_rect(pet);
    let visible = normalize_rect(visible);
    let body_size = (
        finite_non_negative(body_size.0),
        finite_non_negative(body_size.1),
    );
    let order = BubbleSide::candidate_order(placement);

    for side in order {
        let proposed = candidate_window(side, pet, body_size, visible);
        if !is_within(proposed, visible) {
            continue;
        }

        let geometry = layout(side, proposed, body_size, pet);
        if is_external(side, &geometry, pet) {
            return geometry;
        }
    }

    for side in order {
        let proposed = candidate_window(side, pet, body_size, visible);
        let clamped = clamp_to_visible(proposed, visible);
        let geometry = layout(side, clamped, body_size, pet);
        if is_external(side, &geometry, pet) {
            return geometry;
        }
    }

    let proposed = candidate_window(order[0], pet, body_size, visible);
    let clamped = clamp_to_visible(proposed, visible);
    let geometry = layout(order[0], clamped, body_size, pet);
    BubbleGeometry {
        tail: None,
        side: None,
        ..geometry
    }
}

/// Places a tailless bubble independently of the pet. `body_origin` is the
/// requested screen-space bottom-left of the visible body, not the window.
/// Resizing keeps that origin unless the window must be clamped to the screen.
pub(crate) fn place_standalone_bubble(
    body_origin: (f64, f64),
    body_size: (f64, f64),
    visible: Rect,
) -> BubbleGeometry {
    let visible = normalize_rect(visible);
    let requested_body = (
        finite_non_negative(body_size.0),
        finite_non_negative(body_size.1),
    );
    let (body_width, body_height) = body_size_for_extent(requested_body, visible);
    let (width, height) = window_size(body_width, body_height);
    let window = clamp_to_visible(
        Rect {
            x: saturating_sub(finite_coordinate(body_origin.0), BUBBLE_WINDOW_INSET),
            y: saturating_sub(finite_coordinate(body_origin.1), BUBBLE_WINDOW_INSET),
            width,
            height,
        },
        visible,
    );
    let (width_reserve, height_reserve) = body_reserve();
    let body_width = body_capacity(window.width, width_reserve).min(requested_body.0);
    let body_height = body_capacity(window.height, height_reserve).min(requested_body.1);
    let body = Rect {
        x: clamp_body_cross(BUBBLE_WINDOW_INSET, window.width, body_width),
        y: clamp_body_cross(BUBBLE_WINDOW_INSET, window.height, body_height),
        width: body_width,
        height: body_height,
    };
    BubbleGeometry {
        window,
        body,
        tail: None,
        side: None,
    }
}

fn candidate_window(side: BubbleSide, pet: Rect, body_size: (f64, f64), visible: Rect) -> Rect {
    let (body_width, body_height) = body_size_for_extent(body_size, visible);
    let (window_width, window_height) = window_size(body_width, body_height);

    let centered_x = centered(pet.x, pet.width, body_width);
    let centered_y = centered(pet.y, pet.height, body_height);

    match side {
        BubbleSide::Above => Rect {
            x: saturating_sub(centered_x, BUBBLE_WINDOW_INSET),
            y: saturating_add(top(pet), OUTER_GAP),
            width: window_width,
            height: window_height,
        },
        BubbleSide::Below => Rect {
            x: saturating_sub(centered_x, BUBBLE_WINDOW_INSET),
            y: saturating_sub(saturating_sub(pet.y, OUTER_GAP), window_height),
            width: window_width,
            height: window_height,
        },
        BubbleSide::Left => Rect {
            x: saturating_sub(saturating_sub(pet.x, OUTER_GAP), window_width),
            y: saturating_sub(centered_y, BUBBLE_WINDOW_INSET),
            width: window_width,
            height: window_height,
        },
        BubbleSide::Right => Rect {
            x: saturating_add(right(pet), OUTER_GAP),
            y: saturating_sub(centered_y, BUBBLE_WINDOW_INSET),
            width: window_width,
            height: window_height,
        },
    }
}

fn layout(side: BubbleSide, window: Rect, requested_body: (f64, f64), pet: Rect) -> BubbleGeometry {
    let window = normalize_rect(window);
    let (width_reserve, height_reserve) = body_reserve();
    let body_width = body_capacity(window.width, width_reserve).min(requested_body.0);
    let body_height = body_capacity(window.height, height_reserve).min(requested_body.1);

    let cross_x = saturating_sub(centered(pet.x, pet.width, body_width), window.x);
    let cross_y = saturating_sub(centered(pet.y, pet.height, body_height), window.y);
    let body_x = if side.has_horizontal_tail() {
        clamp_body_cross(BUBBLE_WINDOW_INSET, window.width, body_width)
    } else {
        clamp_body_cross(cross_x, window.width, body_width)
    };
    let body_y = if side.has_horizontal_tail() {
        clamp_body_cross(cross_y, window.height, body_height)
    } else {
        clamp_body_cross(BUBBLE_WINDOW_INSET, window.height, body_height)
    };
    let body = Rect {
        x: body_x,
        y: body_y,
        width: body_width,
        height: body_height,
    };
    let tail = tail_geometry(side, window, body, pet);

    BubbleGeometry {
        window,
        body,
        side: tail.map(|_| side),
        tail,
    }
}

fn clamp_body_cross(target: f64, window_extent: f64, body_extent: f64) -> f64 {
    let inset = BUBBLE_WINDOW_INSET;
    let maximum = saturating_sub(window_extent, saturating_add(inset, body_extent));
    if maximum >= inset {
        clamp_scalar(target, inset, maximum)
    } else {
        clamp_scalar(
            target,
            0.0,
            saturating_sub(window_extent, body_extent).max(0.0),
        )
    }
}

fn tail_geometry(side: BubbleSide, window: Rect, body: Rect, pet: Rect) -> Option<TailGeometry> {
    if body.width <= 0.0 || body.height <= 0.0 {
        return None;
    }

    let (cross_start, cross_extent, pet_center) = if side.has_horizontal_tail() {
        (
            body.y,
            body.height,
            saturating_sub(center(pet.y, pet.height), window.y),
        )
    } else {
        (
            body.x,
            body.width,
            saturating_sub(center(pet.x, pet.width), window.x),
        )
    };
    let radius = BUBBLE_RADIUS.min(cross_extent * 0.5);
    let available_base_width = saturating_sub(cross_extent, radius * 2.0);
    let base_width = TAIL_BASE_WIDTH.min(available_base_width);
    if base_width <= 0.0 || !base_width.is_finite() {
        return None;
    }

    let half_base = base_width * 0.5;
    let lower_center = saturating_add(cross_start, saturating_add(radius, half_base));
    let upper_center = saturating_sub(
        saturating_add(cross_start, cross_extent),
        saturating_add(radius, half_base),
    );
    let center = clamp_scalar(pet_center, lower_center, upper_center);
    let base_start = saturating_sub(center, half_base);
    let base_end = saturating_add(center, half_base);

    let tail = match side {
        BubbleSide::Above => TailGeometry {
            base_start: (base_start, body.y),
            base_end: (base_end, body.y),
            tip: (center, saturating_sub(body.y, TAIL_DEPTH)),
        },
        BubbleSide::Below => TailGeometry {
            base_start: (base_start, top(body)),
            base_end: (base_end, top(body)),
            tip: (center, saturating_add(top(body), TAIL_DEPTH)),
        },
        BubbleSide::Left => TailGeometry {
            base_start: (right(body), base_start),
            base_end: (right(body), base_end),
            tip: (saturating_add(right(body), TAIL_DEPTH), center),
        },
        BubbleSide::Right => TailGeometry {
            base_start: (body.x, base_start),
            base_end: (body.x, base_end),
            tip: (saturating_sub(body.x, TAIL_DEPTH), center),
        },
    };

    Some(TailGeometry {
        base_start: clamp_point(tail.base_start, window),
        base_end: clamp_point(tail.base_end, window),
        tip: clamp_point(tail.tip, window),
    })
}

fn body_reserve() -> (f64, f64) {
    (BUBBLE_WINDOW_INSET * 2.0, BUBBLE_WINDOW_INSET * 2.0)
}

fn body_size_for_extent(requested: (f64, f64), visible: Rect) -> (f64, f64) {
    let (width_reserve, height_reserve) = body_reserve();
    (
        requested.0.min(body_capacity(visible.width, width_reserve)),
        requested
            .1
            .min(body_capacity(visible.height, height_reserve)),
    )
}

fn window_size(body_width: f64, body_height: f64) -> (f64, f64) {
    let (width_reserve, height_reserve) = body_reserve();
    (
        saturating_add(body_width, width_reserve),
        saturating_add(body_height, height_reserve),
    )
}

fn body_capacity(extent: f64, reserve: f64) -> f64 {
    if extent >= reserve {
        saturating_sub(extent, reserve)
    } else {
        0.0
    }
}

fn clamp_point(point: (f64, f64), window: Rect) -> (f64, f64) {
    (
        clamp_scalar(point.0, 0.0, window.width),
        clamp_scalar(point.1, 0.0, window.height),
    )
}

fn normalize_rect(rect: Rect) -> Rect {
    Rect {
        x: finite_coordinate(rect.x),
        y: finite_coordinate(rect.y),
        width: finite_non_negative(rect.width),
        height: finite_non_negative(rect.height),
    }
}

fn finite_coordinate(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_nan() || value <= 0.0 {
        0.0
    } else if value.is_infinite() {
        f64::MAX
    } else {
        value
    }
}

fn centered(origin: f64, extent: f64, length: f64) -> f64 {
    saturating_add(origin, saturating_sub(extent, length) * 0.5)
}

fn center(origin: f64, extent: f64) -> f64 {
    saturating_add(origin, extent * 0.5)
}

fn saturating_add(left: f64, right: f64) -> f64 {
    let result = left + right;
    if result.is_finite() {
        result
    } else if left.is_sign_negative() == right.is_sign_negative() {
        if left.is_sign_negative() {
            -f64::MAX
        } else {
            f64::MAX
        }
    } else {
        0.0
    }
}

fn saturating_sub(left: f64, right: f64) -> f64 {
    saturating_add(left, -right)
}

fn right(rect: Rect) -> f64 {
    saturating_add(rect.x, rect.width)
}

fn top(rect: Rect) -> f64 {
    saturating_add(rect.y, rect.height)
}

fn is_within(rect: Rect, visible: Rect) -> bool {
    rect.x >= visible.x
        && rect.y >= visible.y
        && right(rect) <= right(visible)
        && top(rect) <= top(visible)
}

fn is_external(side: BubbleSide, bubble: &BubbleGeometry, pet: Rect) -> bool {
    let Some(tail) = bubble.tail else {
        return false;
    };
    let tip = (
        saturating_add(bubble.window.x, tail.tip.0),
        saturating_add(bubble.window.y, tail.tip.1),
    );
    match side {
        BubbleSide::Above => tip.1 >= saturating_add(top(pet), BUBBLE_GAP),
        BubbleSide::Below => tip.1 <= saturating_sub(pet.y, BUBBLE_GAP),
        BubbleSide::Left => tip.0 <= saturating_sub(pet.x, BUBBLE_GAP),
        BubbleSide::Right => tip.0 >= saturating_add(right(pet), BUBBLE_GAP),
    }
}

fn clamp_to_visible(rect: Rect, visible: Rect) -> Rect {
    let rect = normalize_rect(rect);
    let visible = normalize_rect(visible);
    let width = rect.width.min(visible.width);
    let height = rect.height.min(visible.height);
    Rect {
        x: clamp_origin(rect.x, width, visible.x, right(visible)),
        y: clamp_origin(rect.y, height, visible.y, top(visible)),
        width,
        height,
    }
}

fn clamp_origin(origin: f64, length: f64, minimum: f64, maximum: f64) -> f64 {
    let origin = finite_coordinate(origin);
    let length = finite_non_negative(length);
    let minimum = finite_coordinate(minimum);
    let maximum = finite_coordinate(maximum);
    if maximum <= minimum {
        return minimum;
    }
    let available = saturating_sub(maximum, minimum);
    if length >= available {
        minimum
    } else {
        clamp_scalar(origin, minimum, saturating_sub(maximum, length))
    }
}

fn clamp_scalar(value: f64, minimum: f64, maximum: f64) -> f64 {
    let value = finite_coordinate(value);
    let minimum = finite_coordinate(minimum);
    let maximum = finite_coordinate(maximum);
    if maximum <= minimum {
        minimum
    } else {
        value.max(minimum).min(maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn image_bounds_remove_padding_and_flip_y_on_negative_screen_origin() {
        let frame = rect(-140.0, -30.0, 200.0, 100.0);
        assert_eq!(
            screen_rect_for_image_bounds(frame, (0.25, 0.125, 0.75, 0.5)),
            Some(rect(-90.0, 20.0, 100.0, 37.5))
        );
        assert_eq!(
            screen_rect_for_image_bounds(frame, (0.75, 0.125, 0.75, 0.5)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(frame, (0.0, 0.0, 1.01, 1.0)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(frame, (0.0, f64::NAN, 1.0, 1.0)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(rect(f64::MAX, 0.0, 200.0, 100.0), (0.0, 0.0, 1.0, 1.0)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(rect(0.0, f64::MAX, 100.0, 200.0), (0.0, 0.0, 1.0, 1.0)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(rect(-f64::MAX, 0.0, 200.0, 100.0), (0.0, 0.0, 1.0, 1.0)),
            None
        );
        assert_eq!(
            screen_rect_for_image_bounds(rect(0.0, -f64::MAX, 100.0, 200.0), (0.0, 0.0, 1.0, 1.0)),
            None
        );
    }

    fn assert_bounded(frame: Rect, visible: Rect) {
        assert!(frame.x.is_finite());
        assert!(frame.y.is_finite());
        assert!(frame.width.is_finite());
        assert!(frame.height.is_finite());
        assert!(frame.width >= 0.0);
        assert!(frame.height >= 0.0);
        assert!(frame.x >= visible.x);
        assert!(frame.y >= visible.y);
        assert!(right(frame) <= right(visible));
        assert!(top(frame) <= top(visible));
    }

    fn assert_local_body(geometry: BubbleGeometry) {
        assert!(geometry.body.x.is_finite());
        assert!(geometry.body.y.is_finite());
        assert!(geometry.body.width.is_finite());
        assert!(geometry.body.height.is_finite());
        assert!(geometry.body.x >= 0.0);
        assert!(geometry.body.y >= 0.0);
        assert!(geometry.body.width >= 0.0);
        assert!(geometry.body.height >= 0.0);
        assert!(right(geometry.body) <= geometry.window.width);
        assert!(top(geometry.body) <= geometry.window.height);

        let (width_reserve, height_reserve) = body_reserve();
        if geometry.window.width >= width_reserve {
            assert!(geometry.body.x + 1e-9 >= BUBBLE_WINDOW_INSET);
            assert!(right(geometry.body) <= geometry.window.width - BUBBLE_WINDOW_INSET + 1e-9);
        }
        if geometry.window.height >= height_reserve {
            assert!(geometry.body.y + 1e-9 >= BUBBLE_WINDOW_INSET);
            assert!(top(geometry.body) <= geometry.window.height - BUBBLE_WINDOW_INSET + 1e-9);
        }

        if let Some(tail) = geometry.tail {
            for point in [tail.base_start, tail.base_end, tail.tip] {
                assert!(point.0.is_finite());
                assert!(point.1.is_finite());
                assert!(point.0 >= 0.0 && point.0 <= geometry.window.width);
                assert!(point.1 >= 0.0 && point.1 <= geometry.window.height);
            }
        }
    }

    fn tip_screen(geometry: BubbleGeometry) -> (f64, f64) {
        let tail = geometry.tail.expect("external geometry has a tail");
        (
            geometry.window.x + tail.tip.0,
            geometry.window.y + tail.tip.1,
        )
    }

    #[test]
    fn standalone_keeps_visible_body_origin_and_has_no_pet_tail() {
        let visible = rect(-900.0, -600.0, 1400.0, 1000.0);
        for size in [(120.0, 40.0), (300.0, 180.0), (80.0, 32.0)] {
            let geometry = place_standalone_bubble((-250.0, -180.0), size, visible);
            assert_eq!(geometry.tail, None);
            assert_eq!(geometry.side, None);
            assert_eq!(geometry.body.width, size.0);
            assert_eq!(geometry.body.height, size.1);
            assert_eq!(
                (
                    geometry.window.x + geometry.body.x,
                    geometry.window.y + geometry.body.y,
                ),
                (-250.0, -180.0)
            );
            assert_bounded(geometry.window, visible);
            assert_local_body(geometry);
        }
    }

    #[test]
    fn standalone_clamps_offscreen_origins_and_shrinks_on_tiny_displays() {
        for visible in [
            rect(-1920.0, -1080.0, 1920.0, 1080.0),
            rect(-15.0, -8.0, 20.0, 15.0),
            rect(-2.0, -3.0, 3.0, 2.0),
        ] {
            for origin in [
                (-f64::MAX, f64::MAX),
                (f64::MAX, -f64::MAX),
                (f64::NAN, f64::INFINITY),
            ] {
                let geometry = place_standalone_bubble(origin, (10_000.0, 50.0), visible);
                assert_eq!(geometry.tail, None);
                assert_eq!(geometry.side, None);
                assert_bounded(geometry.window, visible);
                assert_local_body(geometry);
            }
        }
        let tiny = rect(-2.0, -3.0, 3.0, 2.0);
        let geometry = place_standalone_bubble((-2.0, -3.0), (100.0, 50.0), tiny);
        assert_eq!(geometry.window, tiny);
        assert_eq!(geometry.body.width, 0.0);
        assert_eq!(geometry.body.height, 0.0);
    }

    #[test]
    fn each_side_returns_bounded_body_and_attached_tail() {
        let pet = rect(100.0, 100.0, 80.0, 60.0);
        let visible = rect(-200.0, 0.0, 700.0, 400.0);
        let cases = [
            (BubblePlacement::Above, BubbleSide::Above),
            (BubblePlacement::Below, BubbleSide::Below),
            (BubblePlacement::Left, BubbleSide::Left),
            (BubblePlacement::Right, BubbleSide::Right),
        ];

        for (placement, expected_side) in cases {
            let geometry = place_bubble(pet, (120.0, 40.0), visible, placement);
            assert_eq!(geometry.side, Some(expected_side));
            let tail = geometry.tail.expect("fitting placement should have a tail");
            assert_bounded(geometry.window, visible);
            assert_local_body(geometry);

            let reserve = BUBBLE_WINDOW_INSET * 2.0;
            assert!((geometry.window.width - (geometry.body.width + reserve)).abs() < 1e-9);
            assert!((geometry.window.height - (geometry.body.height + reserve)).abs() < 1e-9);

            let screen_tip = tip_screen(geometry);
            let (tip_margin, outer_gap) = match expected_side {
                BubbleSide::Above => {
                    assert!(screen_tip.1 >= top(pet) + BUBBLE_GAP);
                    assert!((screen_tip.0 - center(pet.x, pet.width)).abs() < 1e-9);
                    (tail.tip.1, geometry.window.y - top(pet))
                }
                BubbleSide::Below => {
                    assert!(screen_tip.1 <= pet.y - BUBBLE_GAP);
                    assert!((screen_tip.0 - center(pet.x, pet.width)).abs() < 1e-9);
                    (
                        geometry.window.height - tail.tip.1,
                        pet.y - top(geometry.window),
                    )
                }
                BubbleSide::Left => {
                    assert!(screen_tip.0 <= pet.x - BUBBLE_GAP);
                    assert!((screen_tip.1 - center(pet.y, pet.height)).abs() < 1e-9);
                    (
                        geometry.window.width - tail.tip.0,
                        pet.x - right(geometry.window),
                    )
                }
                BubbleSide::Right => {
                    assert!(screen_tip.0 >= right(pet) + BUBBLE_GAP);
                    assert!((screen_tip.1 - center(pet.y, pet.height)).abs() < 1e-9);
                    (tail.tip.0, geometry.window.x - right(pet))
                }
            };
            assert!((tip_margin - (BUBBLE_WINDOW_INSET - TAIL_DEPTH)).abs() < 1e-9);
            assert!((outer_gap - OUTER_GAP).abs() < 1e-9);
        }
    }

    #[test]
    fn body_width_capacity_is_uniform_across_sides() {
        let visible = rect(-80.0, -40.0, 200.0, 180.0);
        let pet = rect(-20.0, 20.0, 40.0, 40.0);
        let expected_body_width = visible.width - BUBBLE_WINDOW_INSET * 2.0;

        for placement in [
            BubblePlacement::Above,
            BubblePlacement::Below,
            BubblePlacement::Left,
            BubblePlacement::Right,
        ] {
            let geometry = place_bubble(pet, (500.0, 40.0), visible, placement);
            assert_bounded(geometry.window, visible);
            assert_local_body(geometry);
            assert!((geometry.body.width - expected_body_width).abs() < 1e-9);
            assert!((geometry.window.width - visible.width).abs() < 1e-9);
        }
    }

    #[test]
    fn fitting_alternate_side_wins_before_clamping() {
        let visible = rect(0.0, 0.0, 400.0, 300.0);
        let pet = rect(160.0, 260.0, 80.0, 30.0);
        let geometry = place_bubble(pet, (120.0, 40.0), visible, BubblePlacement::Above);

        assert_eq!(geometry.side, Some(BubbleSide::Below));
        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
        let tip = tip_screen(geometry);
        assert!(tip.1 <= pet.y - BUBBLE_GAP);
    }

    #[test]
    fn cross_axis_clamp_keeps_preferred_side_external() {
        let visible = rect(0.0, 0.0, 400.0, 300.0);
        let pet = rect(0.0, 40.0, 80.0, 60.0);
        let geometry = place_bubble(pet, (300.0, 40.0), visible, BubblePlacement::Above);

        assert_eq!(geometry.side, Some(BubbleSide::Above));
        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
        let tip = tip_screen(geometry);
        assert!(tip.1 >= top(pet) + BUBBLE_GAP);
    }

    #[test]
    fn tail_base_is_clamped_away_from_rounded_corners() {
        let geometry = place_bubble(
            rect(100.0, 100.0, 80.0, 60.0),
            (120.0, 40.0),
            rect(0.0, 0.0, 400.0, 300.0),
            BubblePlacement::Left,
        );
        let tail = geometry.tail.expect("left placement should have a tail");
        let radius = BUBBLE_RADIUS.min(geometry.body.height * 0.5);
        assert!(tail.base_start.1 >= geometry.body.y + radius);
        assert!(tail.base_end.1 <= top(geometry.body) - radius);
        assert!(tail.base_end.1 - tail.base_start.1 <= TAIL_BASE_WIDTH);
    }

    #[test]
    fn oversized_body_and_negative_visible_origin_stay_finite_and_bounded() {
        let visible = rect(-1920.0, -1080.0, 1920.0, 1080.0);
        let geometry = place_bubble(
            rect(-1920.0, -1080.0, 80.0, 80.0),
            (10_000.0, f64::INFINITY),
            visible,
            BubblePlacement::Above,
        );

        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
        assert_eq!(geometry.window, visible);
        assert_eq!(geometry.side, None);
        assert_eq!(geometry.tail, None);
    }

    #[test]
    fn tiny_visible_frame_omits_impossible_tail() {
        let visible = rect(-2.0, -3.0, 3.0, 2.0);
        let geometry = place_bubble(
            rect(-2.0, -3.0, 1.0, 1.0),
            (100.0, 50.0),
            visible,
            BubblePlacement::Above,
        );

        assert_eq!(geometry.window, visible);
        assert_eq!(geometry.side, None);
        assert_eq!(geometry.tail, None);
        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
    }

    #[test]
    fn non_finite_inputs_are_deterministic_and_bounded() {
        let visible = rect(-20.0, -10.0, 100.0, 80.0);
        let geometry = place_bubble(
            rect(f64::NAN, f64::NEG_INFINITY, f64::INFINITY, -5.0),
            (f64::NAN, f64::INFINITY),
            visible,
            BubblePlacement::Auto,
        );

        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
        assert!(geometry.window.x.is_finite());
        assert!(geometry.window.y.is_finite());
    }
}
