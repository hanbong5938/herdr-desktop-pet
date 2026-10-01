use crate::behavior::Reaction;
use std::time::Duration;

const BASE_WIDTH: f64 = 384.0;
const BASE_HEIGHT: f64 = 512.0;

// These distances are expressed in the unscaled artwork coordinate system. The
// UI converts pointer locations back into this space before calling us, so a
// gesture has the same feel at every renderer scale.
const MOVE_THRESHOLD: f64 = 11.0;
const TAP_DRIFT: f64 = MOVE_THRESHOLD;
const TAP_MAX_DURATION: Duration = Duration::from_millis(360);
const PET_MIN_STEP: f64 = f64::EPSILON;
const PET_LEG_DISTANCE: f64 = 18.0;
const PET_REVERSAL_COMMIT: f64 = 3.0;
const PET_MAX_EVENTS: u8 = 8;
const HORIZONTAL_DOMINANCE: f64 = 1.25;

/// A pointer location in the 384x512, top-left-origin artwork coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// The small set of effects the AppKit layer needs to perform for a gesture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureAction {
    None,
    BeginMove,
    BeginResize,
    Reaction(Reaction),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
    Head,
    Body,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionPolicy {
    Legacy,
    Captured(Region),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Owner {
    Candidate,
    Move,
    Resize,
    Pet,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    origin: Point,
    last: Point,
    started_at: Duration,
    region: Region,
    legacy_geometry: bool,
    owner: Owner,
    horizontal_direction: i8,
    horizontal_leg: f64,
    pending_direction: i8,
    pending_leg: f64,
    return_leg_ready: bool,
    pet_events: u8,
    max_displacement: f64,
    head_path_valid: bool,
}

/// Pure gesture classification for the desktop pet.
///
/// `Interaction` deliberately owns no AppKit objects and does not move a
/// window itself. It only turns a press/motion/release stream into move/resize
/// requests and bounded touch reactions. All distances are measured in the
/// fixed artwork space, not in screen points.
#[derive(Debug, Default)]
pub struct Interaction {
    press: Option<Press>,
}

impl Interaction {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a gesture. Resize has precedence over Option-move, and Option
    /// move has precedence over character hit-testing.
    pub fn begin(
        &mut self,
        point: Point,
        now: Duration,
        option: bool,
        resize: bool,
        region_policy: RegionPolicy,
    ) -> GestureAction {
        if !valid_point(point) {
            self.cancel();
            return GestureAction::None;
        }

        // A duplicate mouse-down must not replace the original anchor or
        // ownership. The caller will continue the already-active gesture.
        if self.press.is_some() {
            return GestureAction::None;
        }

        if resize {
            self.press = Some(Press {
                origin: point,
                last: point,
                started_at: now,
                region: Region::Other,
                legacy_geometry: false,
                owner: Owner::Resize,
                horizontal_direction: 0,
                horizontal_leg: 0.0,
                pet_events: 0,
                max_displacement: 0.0,
                pending_direction: 0,
                pending_leg: 0.0,
                return_leg_ready: false,
                head_path_valid: true,
            });
            return GestureAction::BeginResize;
        }

        if option {
            self.press = Some(Press {
                origin: point,
                last: point,
                started_at: now,
                region: Region::Other,
                legacy_geometry: false,
                owner: Owner::Move,
                horizontal_direction: 0,
                horizontal_leg: 0.0,
                pet_events: 0,
                max_displacement: 0.0,
                pending_direction: 0,
                pending_leg: 0.0,
                return_leg_ready: false,
                head_path_valid: true,
            });
            return GestureAction::BeginMove;
        }

        let (region, legacy_geometry) = match region_policy {
            RegionPolicy::Legacy => (classify(point), true),
            RegionPolicy::Captured(region) => (region, false),
        };
        self.press = Some(Press {
            origin: point,
            last: point,
            started_at: now,
            region,
            legacy_geometry,
            owner: Owner::Candidate,
            horizontal_direction: 0,
            horizontal_leg: 0.0,
            pet_events: 0,
            max_displacement: 0.0,
            pending_direction: 0,
            pending_leg: 0.0,
            return_leg_ready: false,
            head_path_valid: true,
        });
        GestureAction::None
    }

    /// Classifies one motion event. The original press location remains the
    /// move threshold anchor; it is never replaced by the latest event.
    pub fn motion(&mut self, point: Point, _now: Duration) -> GestureAction {
        if !finite_point(point) {
            self.cancel();
            return GestureAction::None;
        }

        let Some(mut press) = self.press else {
            return GestureAction::None;
        };

        let dx_from_origin = point.x - press.origin.x;
        let dy_from_origin = point.y - press.origin.y;
        let displacement = dx_from_origin.hypot(dy_from_origin);
        press.max_displacement = press.max_displacement.max(displacement);

        if press.legacy_geometry && press.region == Region::Head && !in_head(point) {
            // Legacy packs retain their original fixed silhouette. New packs
            // capture the renderer's region at down; animation cannot transfer
            // an owned gesture to a different semantic part.
            press.head_path_valid = false;
        }

        let action = match press.owner {
            Owner::Move | Owner::Resize => GestureAction::None,
            Owner::Pet => {
                if press.head_path_valid {
                    pet_motion(&mut press, point)
                } else {
                    GestureAction::None
                }
            }
            Owner::Candidate => match press.region {
                Region::Head => {
                    if press.head_path_valid {
                        pet_motion(&mut press, point)
                    } else {
                        GestureAction::None
                    }
                }
                Region::Body | Region::Other => {
                    if displacement >= MOVE_THRESHOLD {
                        press.owner = Owner::Move;
                        GestureAction::BeginMove
                    } else {
                        GestureAction::None
                    }
                }
            },
        };

        press.last = point;
        self.press = Some(press);
        action
    }

    /// Ends the current gesture. Only a short, nearly stationary press that
    /// began on the head or body produces a tap. Move, resize, pet, invalid,
    /// and off-character releases are all silent.
    pub fn end(&mut self, point: Point, now: Duration) -> GestureAction {
        if !finite_point(point) {
            self.cancel();
            return GestureAction::None;
        }

        let Some(press) = self.press.take() else {
            return GestureAction::None;
        };

        let release_displacement = (point.x - press.origin.x).hypot(point.y - press.origin.y);
        let displacement = press.max_displacement.max(release_displacement);
        let release_region = if press.legacy_geometry {
            classify(point)
        } else {
            press.region
        };
        if press.owner != Owner::Candidate
            || displacement > TAP_DRIFT
            || release_region != press.region
            || !press.head_path_valid
        {
            return GestureAction::None;
        }

        let elapsed = now.saturating_sub(press.started_at);
        if elapsed > TAP_MAX_DURATION {
            return GestureAction::None;
        }

        match press.region {
            Region::Head => GestureAction::Reaction(Reaction::HeadTap),
            Region::Body => GestureAction::Reaction(Reaction::BodyTap),
            Region::Other => GestureAction::None,
        }
    }

    /// Cancels and forgets the current press, including any move anchor or pet
    /// stroke history. Cancellation is idempotent.
    pub fn cancel(&mut self) {
        self.press = None;
    }

    pub fn is_active(&self) -> bool {
        self.press.is_some()
    }
}

fn valid_point(point: Point) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.0
        && point.x <= BASE_WIDTH
        && point.y >= 0.0
        && point.y <= BASE_HEIGHT
}

/// Later samples of an owned gesture may leave the canvas; only the press must
/// land on it.
fn finite_point(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

pub(crate) fn legacy_region(point: Point) -> Region {
    classify(point)
}

fn classify(point: Point) -> Region {
    if in_head(point) {
        Region::Head
    } else if in_body(point) {
        Region::Body
    } else {
        Region::Other
    }
}

// Outer idle head silhouette from herdr-characters/tools/generate-character.py.
// The small margin in the curved outline is intentionally omitted: taps should
// land on actual painted head pixels rather than on the transparent panel.
const HEAD_OUTLINE: &[Point] = &[
    Point { x: 99.0, y: 144.0 },
    Point { x: 95.0, y: 122.0 },
    Point { x: 103.0, y: 83.0 },
    Point { x: 112.0, y: 61.0 },
    Point { x: 157.0, y: 87.0 },
    Point { x: 192.0, y: 60.0 },
    Point { x: 230.0, y: 87.0 },
    Point { x: 273.0, y: 61.0 },
    Point { x: 284.0, y: 84.0 },
    Point { x: 291.0, y: 122.0 },
    Point { x: 285.0, y: 151.0 },
    Point { x: 283.0, y: 198.0 },
    Point { x: 252.0, y: 226.0 },
    Point { x: 193.0, y: 231.0 },
    Point { x: 137.0, y: 228.0 },
    Point { x: 105.0, y: 202.0 },
];

// Outer idle torso silhouette, taken from body_path in the generator.
const BODY_OUTLINE: &[Point] = &[
    Point { x: 113.0, y: 244.0 },
    Point { x: 93.0, y: 271.0 },
    Point { x: 93.0, y: 355.0 },
    Point { x: 112.0, y: 410.0 },
    Point { x: 126.0, y: 446.0 },
    Point { x: 253.0, y: 446.0 },
    Point { x: 272.0, y: 409.0 },
    Point { x: 291.0, y: 355.0 },
    Point { x: 287.0, y: 271.0 },
    Point { x: 268.0, y: 244.0 },
    Point { x: 239.0, y: 223.0 },
    Point { x: 139.0, y: 223.0 },
];

fn in_head(point: Point) -> bool {
    point_in_polygon(point, HEAD_OUTLINE)
}

fn in_body(point: Point) -> bool {
    if point_in_polygon(point, BODY_OUTLINE) {
        return true;
    }

    // The idle sprite's tail, arms, and feet are painted outside the torso.
    // Treat their stroked paths as body hit regions too, while leaving the
    // surrounding transparent panel available for background dragging.
    const STROKE_RADIUS: f64 = 18.0;
    const TAIL: &[(Point, Point)] = &[
        (Point { x: 116.0, y: 354.0 }, Point { x: 63.0, y: 362.0 }),
        (Point { x: 63.0, y: 362.0 }, Point { x: 54.0, y: 426.0 }),
        (Point { x: 54.0, y: 426.0 }, Point { x: 96.0, y: 430.0 }),
        (Point { x: 96.0, y: 430.0 }, Point { x: 128.0, y: 396.0 }),
    ];
    const ARMS: &[(Point, Point)] = &[
        (Point { x: 109.0, y: 283.0 }, Point { x: 83.0, y: 318.0 }),
        (Point { x: 83.0, y: 318.0 }, Point { x: 81.0, y: 349.0 }),
        (Point { x: 260.0, y: 282.0 }, Point { x: 291.0, y: 316.0 }),
        (Point { x: 291.0, y: 316.0 }, Point { x: 296.0, y: 350.0 }),
    ];
    const LEGS: &[(Point, Point)] = &[
        (Point { x: 145.0, y: 390.0 }, Point { x: 137.0, y: 432.0 }),
        (Point { x: 137.0, y: 432.0 }, Point { x: 132.0, y: 443.0 }),
        (Point { x: 233.0, y: 390.0 }, Point { x: 240.0, y: 432.0 }),
        (Point { x: 240.0, y: 432.0 }, Point { x: 247.0, y: 443.0 }),
    ];

    TAIL.iter()
        .chain(ARMS.iter())
        .chain(LEGS.iter())
        .any(|&(start, end)| distance_to_segment(point, start, end) <= STROKE_RADIUS)
}

fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    // Boundary points count as painted. This also handles a synthetic event
    // landing exactly on the hit outline without a numerical gap.
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if distance_to_segment(point, previous, current) <= f64::EPSILON {
            return true;
        }
        let crosses = (current.y > point.y) != (previous.y > point.y);
        if crosses {
            let x_at_y = (previous.x - current.x) * (point.y - current.y)
                / (previous.y - current.y)
                + current.x;
            if point.x < x_at_y {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}
fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::EPSILON {
        return (point.x - start.x).hypot(point.y - start.y);
    }
    let projection = ((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared;
    let t = projection.clamp(0.0, 1.0);
    let closest_x = start.x + t * dx;
    let closest_y = start.y + t * dy;
    (point.x - closest_x).hypot(point.y - closest_y)
}

fn pet_motion(press: &mut Press, point: Point) -> GestureAction {
    let dx = point.x - press.last.x;
    let dy = point.y - press.last.y;
    let horizontal = dx.abs();

    // Vertical and diagonal movement is not a pet stroke. Tiny horizontal
    // samples are retained through the current/pending leg counters instead
    // of being discarded, so a smooth 1px-at-a-time stroke still qualifies.
    if horizontal <= PET_MIN_STEP || horizontal <= dy.abs() * HORIZONTAL_DOMINANCE {
        return GestureAction::None;
    }

    let direction = if dx.is_sign_positive() { 1 } else { -1 };
    if press.horizontal_direction == 0 {
        press.horizontal_direction = direction;
        press.horizontal_leg = horizontal;
        return GestureAction::None;
    }

    if direction == press.horizontal_direction {
        press.horizontal_leg += horizontal;
        // A short reversal which turns back before the hysteresis threshold is
        // just pointer jitter; the resumed leg continues the original stroke.
        press.pending_direction = 0;
        press.pending_leg = 0.0;
        if press.horizontal_leg >= PET_LEG_DISTANCE {
            return maybe_emit_pet(press);
        }
        return GestureAction::None;
    }

    if press.pending_direction == direction {
        press.pending_leg += horizontal;
    } else {
        press.pending_direction = direction;
        press.pending_leg = horizontal;
    }
    if press.pending_leg < PET_REVERSAL_COMMIT {
        return GestureAction::None;
    }

    let previous_leg = press.horizontal_leg;
    let new_leg = press.pending_leg;
    press.horizontal_direction = direction;
    press.horizontal_leg = new_leg;
    press.pending_direction = 0;
    press.pending_leg = 0.0;
    press.return_leg_ready = previous_leg >= PET_LEG_DISTANCE;
    if press.horizontal_leg >= PET_LEG_DISTANCE {
        return maybe_emit_pet(press);
    }
    GestureAction::None
}

fn maybe_emit_pet(press: &mut Press) -> GestureAction {
    if !press.return_leg_ready || press.pet_events >= PET_MAX_EVENTS {
        return GestureAction::None;
    }
    press.return_leg_ready = false;
    press.pet_events += 1;
    press.owner = Owner::Pet;
    GestureAction::Reaction(Reaction::Pet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn authored_head_region_does_not_use_legacy_silhouette() {
        let mut interaction = Interaction::new();
        let point = at(20.0, 450.0);
        interaction.begin(
            point,
            ms(0),
            false,
            false,
            RegionPolicy::Captured(Region::Head),
        );
        assert_eq!(
            interaction.end(point, ms(120)),
            GestureAction::Reaction(Reaction::HeadTap),
        );
        interaction.begin(point, ms(200), false, false, RegionPolicy::Legacy);
        assert_eq!(interaction.end(point, ms(320)), GestureAction::None);
    }

    #[test]
    fn head_pet_requires_horizontal_reversal() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(220.0, 150.0), ms(40)),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(160.0, 150.0), ms(90)),
            GestureAction::Reaction(Reaction::Pet)
        );
        assert_eq!(
            interaction.end(at(160.0, 150.0), ms(130)),
            GestureAction::None
        );
    }

    #[test]
    fn smooth_small_samples_accumulate_to_a_pet() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );

        let mut saw_pet = false;
        for x in 191..=210 {
            saw_pet |= interaction.motion(at(x as f64, 150.0), ms(x as u64))
                == GestureAction::Reaction(Reaction::Pet);
        }
        for x in (192..=209).rev() {
            saw_pet |= interaction.motion(at(x as f64, 150.0), ms((x + 40) as u64))
                == GestureAction::Reaction(Reaction::Pet);
        }

        assert!(saw_pet);
        assert_eq!(
            interaction.end(at(192.0, 150.0), ms(100)),
            GestureAction::None
        );
    }

    #[test]
    fn short_head_jitter_does_not_pet() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        for x in [191.0, 190.0, 191.0, 190.0, 191.0, 190.0] {
            assert_eq!(
                interaction.motion(at(x, 150.0), ms(10)),
                GestureAction::None
            );
        }
        assert_eq!(
            interaction.end(at(190.0, 150.0), ms(20)),
            GestureAction::Reaction(Reaction::HeadTap)
        );
    }

    #[test]
    fn leaving_head_invalidates_stroke_and_tap() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(320.0, 150.0), ms(20)),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(160.0, 150.0), ms(40)),
            GestureAction::None
        );
        assert_eq!(
            interaction.end(at(160.0, 150.0), ms(60)),
            GestureAction::None
        );
    }

    #[test]
    fn off_character_release_does_not_fake_head_tap() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(100.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(
            interaction.end(at(90.0, 150.0), ms(20)),
            GestureAction::None
        );
    }

    #[test]
    fn stationary_head_hold_is_not_pet() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 150.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(190.0, 150.0), ms(1_000)),
            GestureAction::None
        );
        assert_eq!(
            interaction.end(at(190.0, 150.0), ms(1_100)),
            GestureAction::None
        );
    }

    #[test]
    fn body_drag_commits_move_and_cannot_tap_afterward() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 320.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(220.0, 320.0), ms(30)),
            GestureAction::BeginMove
        );
        assert_eq!(
            interaction.motion(at(150.0, 320.0), ms(50)),
            GestureAction::None
        );
        assert_eq!(
            interaction.end(at(150.0, 320.0), ms(80)),
            GestureAction::None
        );
        assert!(!interaction.is_active());
    }

    #[test]
    fn cancellation_discards_anchor_and_release_is_silent() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(at(190.0, 320.0), ms(0), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        interaction.cancel();
        assert!(!interaction.is_active());
        assert_eq!(
            interaction.end(at(190.0, 320.0), ms(10)),
            GestureAction::None
        );
        assert_eq!(
            interaction.begin(at(20.0, 20.0), ms(20), false, false, RegionPolicy::Legacy),
            GestureAction::None
        );
        assert_eq!(interaction.end(at(20.0, 20.0), ms(40)), GestureAction::None);
    }

    #[test]
    fn edge_press_flick_off_canvas_still_begins_move() {
        let mut interaction = Interaction::new();
        assert_eq!(
            interaction.begin(
                at(8.0, 300.0),
                ms(0),
                false,
                false,
                RegionPolicy::Captured(Region::Body),
            ),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(-30.0, 300.0), ms(16)),
            GestureAction::BeginMove
        );
        assert!(interaction.is_active());

        interaction.cancel();
        assert_eq!(
            interaction.begin(
                at(8.0, 300.0),
                ms(100),
                false,
                false,
                RegionPolicy::Captured(Region::Body),
            ),
            GestureAction::None
        );
        assert_eq!(
            interaction.motion(at(f64::NAN, 300.0), ms(116)),
            GestureAction::None
        );
        assert!(!interaction.is_active());
    }
}
