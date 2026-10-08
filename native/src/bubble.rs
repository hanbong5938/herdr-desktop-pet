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

/// Requested visible body dimensions, excluding the transparent window inset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BubbleSize {
    pub width: f64,
    pub height: f64,
}

impl BubbleSize {
    pub(crate) fn is_valid(self) -> bool {
        self.width.is_finite() && self.width > 0.0 && self.height.is_finite() && self.height > 0.0
    }
}

/// The pointer displacement is relative to the captured body, in AppKit's
/// upward-positive screen coordinates. The opposite edge remains fixed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BubbleResizeDirection {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl BubbleResizeDirection {
    pub(crate) fn size_delta(self, delta: (f64, f64)) -> (f64, f64) {
        let dx = finite_coordinate(delta.0);
        let dy = finite_coordinate(delta.1);
        match self {
            Self::Left => (-dx, 0.0),
            Self::Right => (dx, 0.0),
            Self::Top => (0.0, dy),
            Self::Bottom => (0.0, -dy),
            Self::TopLeft => (-dx, dy),
            Self::TopRight => (dx, dy),
            Self::BottomLeft => (-dx, -dy),
            Self::BottomRight => (dx, -dy),
        }
    }
}

/// Preserve a saved request on axes the drag did not actually move, including
/// requests larger than the currently visible screen can display.
pub(crate) fn requested_bubble_resize_size(
    start_body: Rect,
    direction: BubbleResizeDirection,
    delta: (f64, f64),
    minimum: BubbleSize,
    prior: Option<BubbleSize>,
) -> BubbleSize {
    let (dw, dh) = direction.size_delta(delta);
    let requested_axis = |start: f64, change: f64, floor: f64, saved: Option<f64>| {
        let floor = finite_non_negative(floor).max(1.0);
        if change == 0.0 {
            saved
                .map(finite_non_negative)
                .unwrap_or_else(|| finite_non_negative(start).max(floor))
        } else {
            saturating_add(finite_non_negative(start), change).max(floor)
        }
    };
    BubbleSize {
        width: requested_axis(start_body.width, dw, minimum.width, prior.map(|s| s.width)),
        height: requested_axis(
            start_body.height,
            dh,
            minimum.height,
            prior.map(|s| s.height),
        ),
    }
}

/// Resize against the captured opposite edge. On an edge drag, the orthogonal
/// axis keeps its low origin where possible while accommodating a fresh floor.
pub(crate) fn resize_bubble_body(
    start_body: Rect,
    direction: BubbleResizeDirection,
    delta: (f64, f64),
    minimum: BubbleSize,
    visible: Rect,
) -> Rect {
    let start = normalize_rect(start_body);
    let (dw, dh) = direction.size_delta(delta);
    fit_resizing_body(
        start,
        direction,
        BubbleSize {
            width: saturating_add(start.width, dw).max(finite_non_negative(minimum.width)),
            height: saturating_add(start.height, dh).max(finite_non_negative(minimum.height)),
        },
        visible,
    )
}

/// Fit a freshly measured effective size, including natural shrink, without
/// encoding a pointer displacement or a saved size request.
pub(crate) fn fit_resizing_bubble_body(
    start_body: Rect,
    direction: BubbleResizeDirection,
    target_effective: BubbleSize,
    visible: Rect,
) -> Rect {
    fit_resizing_body(
        normalize_rect(start_body),
        direction,
        target_effective,
        visible,
    )
}

fn fit_resizing_body(
    start: Rect,
    direction: BubbleResizeDirection,
    target: BubbleSize,
    visible: Rect,
) -> Rect {
    let visible = normalize_rect(visible);
    let (x, width) = resize_axis(
        start.x,
        start.width,
        visible.x,
        visible.width,
        target.width,
        !matches!(
            direction,
            BubbleResizeDirection::Top | BubbleResizeDirection::Bottom
        ),
        matches!(
            direction,
            BubbleResizeDirection::Left
                | BubbleResizeDirection::TopLeft
                | BubbleResizeDirection::BottomLeft
        ),
    );
    let (y, height) = resize_axis(
        start.y,
        start.height,
        visible.y,
        visible.height,
        target.height,
        !matches!(
            direction,
            BubbleResizeDirection::Left | BubbleResizeDirection::Right
        ),
        matches!(
            direction,
            BubbleResizeDirection::Bottom
                | BubbleResizeDirection::BottomLeft
                | BubbleResizeDirection::BottomRight
        ),
    );
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn resize_axis(
    start: f64,
    length: f64,
    visible_origin: f64,
    visible_extent: f64,
    target: f64,
    grabbed: bool,
    moving_low: bool,
) -> (f64, f64) {
    let capacity = body_capacity(visible_extent, BUBBLE_WINDOW_INSET * 2.0);
    let low = if capacity > 0.0 {
        saturating_add(visible_origin, BUBBLE_WINDOW_INSET)
    } else {
        visible_origin
    };
    let high = if capacity > 0.0 {
        saturating_sub(
            saturating_add(visible_origin, visible_extent),
            BUBBLE_WINDOW_INSET,
        )
    } else {
        saturating_add(visible_origin, visible_extent)
    };
    if capacity == 0.0 {
        return (low, 0.0);
    }
    if !grabbed {
        let length = finite_non_negative(target).min(capacity);
        return (clamp_origin(start, length, low, high), length);
    }
    let fixed = clamp_scalar(
        if moving_low {
            saturating_add(start, length)
        } else {
            start
        },
        low,
        high,
    );
    let maximum = if moving_low {
        saturating_sub(fixed, low)
    } else {
        saturating_sub(high, fixed)
    };
    let length = finite_non_negative(target).min(maximum);
    (
        if moving_low {
            saturating_sub(fixed, length)
        } else {
            fixed
        },
        length,
    )
}

/// Only the painted rounded body is interactive; the tail and transparent
/// window inset are excluded. Corner regions extend inward beyond edge bands.
pub(crate) fn bubble_resize_direction_at(
    body: Rect,
    point: (f64, f64),
) -> Option<BubbleResizeDirection> {
    if !point.0.is_finite()
        || !point.1.is_finite()
        || !body.x.is_finite()
        || !body.y.is_finite()
        || !body.width.is_finite()
        || !body.height.is_finite()
        || body.width <= 0.0
        || body.height <= 0.0
    {
        return None;
    }
    let x = point.0 - body.x;
    let y = point.1 - body.y;
    if x < 0.0 || y < 0.0 || x > body.width || y > body.height {
        return None;
    }
    let radius = BUBBLE_RADIUS.min(body.width * 0.5).min(body.height * 0.5);
    let corner_x = if x < radius {
        radius
    } else {
        body.width - radius
    };
    let corner_y = if y < radius {
        radius
    } else {
        body.height - radius
    };
    if (x < radius || x > body.width - radius)
        && (y < radius || y > body.height - radius)
        && (x - corner_x).powi(2) + (y - corner_y).powi(2) > radius * radius
    {
        return None;
    }
    let left = x <= body.width - x;
    let bottom = y <= body.height - y;
    let dx = x.min(body.width - x);
    let dy = y.min(body.height - y);
    if dx <= 14.0 && dy <= 14.0 {
        return Some(match (left, bottom) {
            (true, true) => BubbleResizeDirection::BottomLeft,
            (true, false) => BubbleResizeDirection::TopLeft,
            (false, true) => BubbleResizeDirection::BottomRight,
            (false, false) => BubbleResizeDirection::TopRight,
        });
    }
    if dx <= 5.0 {
        Some(if left {
            BubbleResizeDirection::Left
        } else {
            BubbleResizeDirection::Right
        })
    } else if dy <= 5.0 {
        Some(if bottom {
            BubbleResizeDirection::Bottom
        } else {
            BubbleResizeDirection::Top
        })
    } else {
        None
    }
}

/// Lay out a resized screen-space body without selecting a different pet side.
/// An attached bubble loses its tail if its original side is no longer external;
/// the caller retains attached intent independently of `side`.
pub(crate) fn place_resizing_bubble(
    body: Rect,
    visible: Rect,
    attached: Option<(Rect, Option<BubbleSide>)>,
) -> BubbleGeometry {
    let body = normalize_rect(body);
    let mut geometry =
        place_standalone_bubble((body.x, body.y), (body.width, body.height), visible);
    if let Some((pet, Some(side))) = attached {
        let pet = normalize_rect(pet);
        let tail = tail_geometry(side, geometry.window, geometry.body, pet);
        geometry.tail = tail;
        if is_external(side, &geometry, pet) {
            geometry.side = Some(side);
        } else {
            geometry.tail = None;
        }
    }
    geometry
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
    fn resize_changes_axes_independently_without_moving_top_left() {
        let visible = rect(-500.0, -400.0, 1000.0, 800.0);
        let start = rect(-200.0, -100.0, 120.0, 60.0);
        let minimum = BubbleSize {
            width: 80.0,
            height: 35.0,
        };
        assert!(minimum.is_valid());
        assert_eq!(
            resize_bubble_body(
                start,
                BubbleResizeDirection::BottomRight,
                (0.0, 0.0),
                minimum,
                visible
            ),
            start
        );
        assert_eq!(
            resize_bubble_body(
                start,
                BubbleResizeDirection::BottomRight,
                (25.0, 0.0),
                minimum,
                visible
            ),
            rect(-200.0, -100.0, 145.0, 60.0)
        );
        assert_eq!(
            resize_bubble_body(
                start,
                BubbleResizeDirection::BottomRight,
                (0.0, -30.0),
                minimum,
                visible
            ),
            rect(-200.0, -130.0, 120.0, 90.0)
        );
        assert_eq!(
            resize_bubble_body(
                start,
                BubbleResizeDirection::BottomRight,
                (-1000.0, 1000.0),
                minimum,
                visible
            ),
            rect(-200.0, -75.0, 80.0, 35.0)
        );
    }

    #[test]
    fn resize_caps_at_screen_inset_even_with_negative_monitor_origin() {
        let visible = rect(-300.0, -200.0, 240.0, 160.0);
        let minimum = BubbleSize {
            width: 90.0,
            height: 40.0,
        };
        let body = resize_bubble_body(
            rect(-250.0, -150.0, 100.0, 50.0),
            BubbleResizeDirection::BottomRight,
            (1000.0, -1000.0),
            minimum,
            visible,
        );
        assert_eq!(body, rect(-250.0, -188.0, 178.0, 88.0));
        let geometry = place_resizing_bubble(body, visible, None);
        assert_eq!(geometry.window.x + geometry.body.x, body.x);
        assert_eq!(geometry.window.y + geometry.body.y, body.y);
        assert_eq!(geometry.body.width, body.width);
        assert_eq!(geometry.body.height, body.height);
        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);

        let tiny = rect(-2.0, -3.0, 3.0, 2.0);
        let tiny_body = resize_bubble_body(
            body,
            BubbleResizeDirection::BottomRight,
            (f64::INFINITY, f64::NAN),
            minimum,
            tiny,
        );
        assert_eq!(tiny_body.width, 0.0);
        assert_eq!(tiny_body.height, 0.0);
        let geometry = place_resizing_bubble(tiny_body, tiny, None);
        assert_eq!(geometry.window, tiny);
        assert_bounded(geometry.window, tiny);
        assert_local_body(geometry);
    }

    #[test]
    fn resize_sanitizes_invalid_inputs_without_nonfinite_geometry() {
        for size in [
            BubbleSize {
                width: f64::NAN,
                height: 50.0,
            },
            BubbleSize {
                width: 10.0,
                height: f64::INFINITY,
            },
            BubbleSize {
                width: -1.0,
                height: 10.0,
            },
        ] {
            assert!(!size.is_valid());
        }
        let visible = rect(-200.0, -100.0, 300.0, 200.0);
        let body = resize_bubble_body(
            rect(f64::NAN, f64::INFINITY, f64::INFINITY, -1.0),
            BubbleResizeDirection::BottomRight,
            (f64::NAN, f64::NEG_INFINITY),
            BubbleSize {
                width: f64::INFINITY,
                height: f64::NAN,
            },
            visible,
        );
        let geometry = place_resizing_bubble(body, visible, None);
        assert_bounded(geometry.window, visible);
        assert_local_body(geometry);
    }

    #[test]
    fn resize_all_eight_directions_keep_opposite_edges_and_return_to_capture() {
        use BubbleResizeDirection::*;
        let visible = rect(-400.0, -300.0, 800.0, 600.0);
        let start = rect(-100.0, -80.0, 120.0, 60.0);
        let minimum = BubbleSize {
            width: 80.0,
            height: 35.0,
        };
        for (direction, expected) in [
            (Left, rect(-80.0, -80.0, 100.0, 60.0)),
            (Right, rect(-100.0, -80.0, 140.0, 60.0)),
            (Top, rect(-100.0, -80.0, 120.0, 45.0)),
            (Bottom, rect(-100.0, -95.0, 120.0, 75.0)),
            (TopLeft, rect(-80.0, -80.0, 100.0, 45.0)),
            (TopRight, rect(-100.0, -80.0, 140.0, 45.0)),
            (BottomLeft, rect(-80.0, -95.0, 100.0, 75.0)),
            (BottomRight, rect(-100.0, -95.0, 140.0, 75.0)),
        ] {
            assert_eq!(
                resize_bubble_body(start, direction, (20.0, -15.0), minimum, visible),
                expected,
                "{direction:?}"
            );
            assert_eq!(
                resize_bubble_body(start, direction, (0.0, 0.0), minimum, visible),
                start,
                "{direction:?}"
            );
            let smaller = resize_bubble_body(start, direction, (-1000.0, 1000.0), minimum, visible);
            let larger = resize_bubble_body(start, direction, (1000.0, -1000.0), minimum, visible);
            for body in [smaller, larger] {
                assert!(body.width <= visible.width - 2.0 * BUBBLE_WINDOW_INSET);
                assert!(body.height <= visible.height - 2.0 * BUBBLE_WINDOW_INSET);
                assert!(body.x >= visible.x + BUBBLE_WINDOW_INSET);
                assert!(body.y >= visible.y + BUBBLE_WINDOW_INSET);
                assert!(right(body) <= right(visible) - BUBBLE_WINDOW_INSET);
                assert!(top(body) <= top(visible) - BUBBLE_WINDOW_INSET);
                if matches!(direction, Left | TopLeft | BottomLeft) {
                    assert_eq!(right(body), right(start));
                } else {
                    assert_eq!(body.x, start.x);
                }
                if matches!(direction, Bottom | BottomLeft | BottomRight) {
                    assert_eq!(top(body), top(start));
                } else {
                    assert_eq!(body.y, start.y);
                }
            }
        }
    }

    #[test]
    fn resize_inactive_axis_grows_to_content_floor_without_changing_saved_request() {
        use BubbleResizeDirection::*;
        let start = rect(-100.0, -80.0, 60.0, 35.0);
        let minimum = BubbleSize {
            width: 100.0,
            height: 70.0,
        };
        let saved = BubbleSize {
            width: 450.0,
            height: 300.0,
        };
        let visible = rect(-300.0, -200.0, 400.0, 300.0);
        for direction in [Top, Bottom] {
            let body = resize_bubble_body(start, direction, (300.0, -10.0), minimum, visible);
            assert_eq!((body.x, body.width), (start.x, minimum.width));
            assert_eq!(
                requested_bubble_resize_size(
                    start,
                    direction,
                    (300.0, -10.0),
                    minimum,
                    Some(saved)
                )
                .width,
                saved.width
            );
        }
        for direction in [Left, Right] {
            let body = resize_bubble_body(start, direction, (10.0, -300.0), minimum, visible);
            assert_eq!((body.y, body.height), (start.y, minimum.height));
            assert_eq!(
                requested_bubble_resize_size(
                    start,
                    direction,
                    (10.0, -300.0),
                    minimum,
                    Some(saved)
                )
                .height,
                saved.height
            );
        }
        assert_eq!(
            requested_bubble_resize_size(start, BottomRight, (0.0, 0.0), minimum, Some(saved)),
            saved
        );
        assert_eq!(
            requested_bubble_resize_size(start, BottomRight, (0.0, 0.0), minimum, None),
            minimum
        );
        assert_eq!(
            requested_bubble_resize_size(start, TopLeft, (-20.0, 10.0), minimum, Some(saved)),
            BubbleSize {
                width: 100.0,
                height: 70.0
            }
        );
    }

    #[test]
    fn resize_wrap_height_floor_grows_on_horizontal_edges_and_clamps_low_origin() {
        use BubbleResizeDirection::*;
        let visible = rect(-300.0, -200.0, 240.0, 160.0);
        let start = rect(-250.0, -150.0, 100.0, 50.0);
        let wrapped = BubbleSize {
            width: 90.0,
            height: 110.0,
        };
        // The saved untouched height may exceed the screen; the rendered body
        // instead grows to the fresh text floor at the captured screen width.
        let saved = BubbleSize {
            width: 440.0,
            height: 310.0,
        };
        for direction in [Left, Right] {
            let body = resize_bubble_body(start, direction, (10.0, -1000.0), wrapped, visible);
            assert_eq!((body.y, body.height), (-162.0, 110.0), "{direction:?}");
            assert_eq!(
                requested_bubble_resize_size(
                    start,
                    direction,
                    (10.0, -1000.0),
                    wrapped,
                    Some(saved),
                )
                .height,
                saved.height
            );
            let geometry = place_resizing_bubble(body, visible, None);
            assert_bounded(geometry.window, visible);
            assert_eq!(geometry.window.y + geometry.body.y, body.y);
            assert_eq!(geometry.body.height, body.height);
        }
        // An inactive axis has the whole inset interval, not just the room
        // beside a corner's fixed opposite edge.
        let wide = BubbleSize {
            width: 210.0,
            height: 40.0,
        };
        for direction in [Top, Bottom] {
            let body = resize_bubble_body(start, direction, (1000.0, 5.0), wide, visible);
            assert_eq!((body.x, body.width), (-282.0, 210.0), "{direction:?}");
            assert_eq!(
                requested_bubble_resize_size(start, direction, (1000.0, 5.0), wide, Some(saved))
                    .width,
                saved.width
            );
            assert_bounded(place_resizing_bubble(body, visible, None).window, visible);
            let overflowing = BubbleSize {
                width: 300.0,
                height: 40.0,
            };
            let clipped = resize_bubble_body(start, direction, (1000.0, 5.0), overflowing, visible);
            assert_eq!((clipped.x, clipped.width), (-288.0, 216.0));
            assert_bounded(
                place_resizing_bubble(clipped, visible, None).window,
                visible,
            );
        }
    }

    #[test]
    fn resize_corner_floors_on_unmoved_axis_keep_both_opposite_edges_fixed() {
        use BubbleResizeDirection::*;
        let visible = rect(-300.0, -200.0, 240.0, 160.0);
        let start = rect(-250.0, -150.0, 100.0, 50.0);
        let taller = BubbleSize {
            width: 90.0,
            height: 110.0,
        };
        let wider = BubbleSize {
            width: 300.0,
            height: 40.0,
        };
        for (direction, horizontal, vertical) in [
            (
                TopLeft,
                rect(-270.0, -150.0, 120.0, 98.0),
                rect(-288.0, -150.0, 138.0, 40.0),
            ),
            (
                TopRight,
                rect(-250.0, -150.0, 120.0, 98.0),
                rect(-250.0, -150.0, 178.0, 40.0),
            ),
            (
                BottomLeft,
                rect(-270.0, -188.0, 120.0, 88.0),
                rect(-288.0, -160.0, 138.0, 60.0),
            ),
            (
                BottomRight,
                rect(-250.0, -188.0, 120.0, 88.0),
                rect(-250.0, -160.0, 178.0, 60.0),
            ),
        ] {
            let dx = if matches!(direction, TopLeft | BottomLeft) {
                -20.0
            } else {
                20.0
            };
            let horizontal_body = resize_bubble_body(start, direction, (dx, 0.0), taller, visible);
            let vertical_body = resize_bubble_body(start, direction, (0.0, -10.0), wider, visible);
            assert_eq!(horizontal_body, horizontal, "{direction:?} x-only");
            assert_eq!(vertical_body, vertical, "{direction:?} y-only");
            for body in [horizontal_body, vertical_body] {
                if matches!(direction, TopLeft | BottomLeft) {
                    assert_eq!(right(body), right(start));
                } else {
                    assert_eq!(body.x, start.x);
                }
                if matches!(direction, BottomLeft | BottomRight) {
                    assert_eq!(top(body), top(start));
                } else {
                    assert_eq!(body.y, start.y);
                }
                let geometry = place_resizing_bubble(body, visible, None);
                assert_bounded(geometry.window, visible);
                assert_eq!(geometry.window.x + geometry.body.x, body.x);
                assert_eq!(geometry.window.y + geometry.body.y, body.y);
            }
            // A reverse preview does not make a captured corner's fixed edge
            // slide to satisfy an impossible content floor.
            let reversed = resize_bubble_body(start, direction, (-dx, 0.0), taller, visible);
            let returned = resize_bubble_body(start, direction, (0.0, 0.0), taller, visible);
            let reversed_x = if matches!(direction, TopLeft | BottomLeft) {
                -240.0
            } else {
                -250.0
            };
            assert_eq!((reversed.x, reversed.width), (reversed_x, 90.0));
            assert_eq!((returned.x, returned.width), (start.x, start.width));
            assert_eq!(
                (returned.y, returned.height),
                (horizontal.y, horizontal.height)
            );
            let captured_floor = BubbleSize {
                width: start.width,
                height: start.height,
            };
            assert_eq!(
                resize_bubble_body(start, direction, (0.0, 0.0), captured_floor, visible),
                start
            );
        }
    }

    #[test]
    fn exact_resize_fit_can_shrink_or_grow_without_changing_saved_request() {
        use BubbleResizeDirection::*;
        let visible = rect(-300.0, -200.0, 240.0, 160.0);
        let start = rect(-250.0, -150.0, 100.0, 50.0);
        let smaller = BubbleSize {
            width: 60.0,
            height: 30.0,
        };
        let larger = BubbleSize {
            width: 300.0,
            height: 110.0,
        };
        for (direction, shrunk, grown) in [
            (
                TopLeft,
                rect(-210.0, -150.0, 60.0, 30.0),
                rect(-288.0, -150.0, 138.0, 98.0),
            ),
            (
                BottomRight,
                rect(-250.0, -130.0, 60.0, 30.0),
                rect(-250.0, -188.0, 178.0, 88.0),
            ),
        ] {
            assert_eq!(
                fit_resizing_bubble_body(start, direction, smaller, visible),
                shrunk
            );
            let body = fit_resizing_bubble_body(start, direction, larger, visible);
            assert_eq!(body, grown);
            assert_bounded(place_resizing_bubble(body, visible, None).window, visible);
            for prior in [
                None,
                Some(BubbleSize {
                    width: 500.0,
                    height: 400.0,
                }),
            ] {
                let saved =
                    requested_bubble_resize_size(start, direction, (0.0, 0.0), larger, prior);
                assert_eq!(saved, prior.unwrap_or(larger));
                assert_ne!((saved.width, saved.height), (body.width, body.height));
            }
        }
        assert_eq!(
            fit_resizing_bubble_body(start, Right, larger, visible),
            rect(-250.0, -162.0, 178.0, 110.0)
        );
        let tiny = rect(-2.0, -3.0, 3.0, 2.0);
        let invalid = BubbleSize {
            width: f64::INFINITY,
            height: f64::NAN,
        };
        for direction in [TopLeft, BottomRight] {
            let body = fit_resizing_bubble_body(start, direction, invalid, tiny);
            assert_eq!((body.width, body.height), (0.0, 0.0));
            assert!(body.x.is_finite() && body.y.is_finite());
            assert_bounded(place_resizing_bubble(body, tiny, None).window, tiny);
            let sanitized = fit_resizing_bubble_body(
                rect(f64::NAN, f64::INFINITY, f64::INFINITY, -1.0),
                direction,
                invalid,
                visible,
            );
            assert!(sanitized.x.is_finite() && sanitized.y.is_finite());
            assert!(sanitized.width.is_finite() && sanitized.height.is_finite());
            assert_bounded(
                place_resizing_bubble(sanitized, visible, None).window,
                visible,
            );
        }
    }

    #[test]
    fn resize_negative_screen_extremes_and_invalid_input_stay_bounded() {
        let visible = rect(-300.0, -200.0, 240.0, 160.0);
        let start = rect(-250.0, -150.0, 100.0, 50.0);
        let minimum = BubbleSize {
            width: 90.0,
            height: 40.0,
        };
        let left_top = resize_bubble_body(
            start,
            BubbleResizeDirection::TopLeft,
            (-1000.0, 1000.0),
            minimum,
            visible,
        );
        assert_eq!(left_top, rect(-288.0, -150.0, 138.0, 98.0));
        let tiny = rect(-2.0, -3.0, 3.0, 2.0);
        for direction in [
            BubbleResizeDirection::TopLeft,
            BubbleResizeDirection::BottomRight,
        ] {
            let collapsed =
                resize_bubble_body(start, direction, (f64::NAN, f64::INFINITY), minimum, tiny);
            assert_eq!((collapsed.width, collapsed.height), (0.0, 0.0));
            assert!(collapsed.x.is_finite() && collapsed.y.is_finite());
            let normalized = resize_bubble_body(
                rect(f64::NAN, f64::INFINITY, f64::INFINITY, -1.0),
                direction,
                (f64::NEG_INFINITY, f64::NAN),
                minimum,
                visible,
            );
            assert!(normalized.x.is_finite() && normalized.y.is_finite());
            assert!(normalized.width.is_finite() && normalized.height.is_finite());
            assert!(normalized.x >= visible.x && right(normalized) <= right(visible));
            assert!(normalized.y >= visible.y && top(normalized) <= top(visible));
        }
    }

    #[test]
    fn resize_hit_classifier_excludes_cutouts_tail_and_interior() {
        use BubbleResizeDirection::*;
        let body = rect(-100.0, -80.0, 120.0, 60.0);
        for (point, expected) in [
            ((-98.0, -50.0), Some(Left)),
            ((18.0, -50.0), Some(Right)),
            ((-40.0, -22.0), Some(Top)),
            ((-40.0, -78.0), Some(Bottom)),
            ((-91.0, -71.0), Some(BottomLeft)),
            ((11.0, -71.0), Some(BottomRight)),
            ((-91.0, -29.0), Some(TopLeft)),
            ((11.0, -29.0), Some(TopRight)),
            ((-40.0, -50.0), None),
            ((-99.0, -79.0), None),
            ((19.0, -21.0), None),
            ((-101.0, -50.0), None),
            ((-40.0, -85.0), None),
            ((f64::NAN, -50.0), None),
        ] {
            assert_eq!(
                bubble_resize_direction_at(body, point),
                expected,
                "{point:?}"
            );
        }
        // Equal-distance edge collisions choose the left/bottom deterministically.
        assert_eq!(
            bubble_resize_direction_at(rect(0.0, 0.0, 12.0, 12.0), (6.0, 6.0)),
            Some(BottomLeft)
        );
        assert_eq!(
            bubble_resize_direction_at(rect(0.0, 0.0, 0.0, 12.0), (0.0, 6.0)),
            None
        );
    }

    #[test]
    fn attached_resize_keeps_original_side_or_drops_tail_on_overlap() {
        let visible = rect(0.0, 0.0, 500.0, 400.0);
        let pet = rect(100.0, 100.0, 80.0, 60.0);
        let start = place_bubble(pet, (120.0, 40.0), visible, BubblePlacement::Above);
        let start_body = rect(
            start.window.x + start.body.x,
            start.window.y + start.body.y,
            start.body.width,
            start.body.height,
        );
        let same = place_resizing_bubble(start_body, visible, Some((pet, start.side)));
        assert_eq!(same.side, Some(BubbleSide::Above));
        assert_eq!(same.body.width, start.body.width);
        assert_eq!(same.window.x + same.body.x, start_body.x);
        assert_eq!(same.window.y + same.body.y, start_body.y);

        let expanded = resize_bubble_body(
            start_body,
            BubbleResizeDirection::BottomRight,
            (80.0, 10.0),
            BubbleSize {
                width: 80.0,
                height: 30.0,
            },
            visible,
        );
        let resized = place_resizing_bubble(expanded, visible, Some((pet, start.side)));
        assert_eq!(resized.side, Some(BubbleSide::Above));
        assert_eq!(resized.body.width, 200.0);
        assert_eq!(resized.body.height, 30.0);
        assert_bounded(resized.window, visible);
        assert_local_body(resized);

        let overlapping = place_resizing_bubble(
            rect(100.0, 120.0, 120.0, 40.0),
            visible,
            Some((pet, Some(BubbleSide::Above))),
        );
        assert_eq!(overlapping.side, None);
        assert_eq!(overlapping.tail, None);
        assert_eq!(
            place_resizing_bubble(start_body, visible, Some((pet, None))).side,
            None
        );
        assert_eq!(place_resizing_bubble(start_body, visible, None).tail, None);

        let edge_pet = rect(160.0, 360.0, 80.0, 30.0);
        let retained = place_resizing_bubble(
            rect(140.0, 352.0, 120.0, 40.0),
            visible,
            Some((edge_pet, Some(BubbleSide::Above))),
        );
        assert_ne!(retained.side, Some(BubbleSide::Below));
        assert_eq!(retained.side, None);
        assert_eq!(retained.tail, None);
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
