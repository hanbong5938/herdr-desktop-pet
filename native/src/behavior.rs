use crate::agent_outcome::AgentOutcome;
use crate::pose::{PoseKind, PoseSnapshot};
use crate::state::{Phase, Scene};
use std::time::Duration;

/// A small user interaction that can produce a cosmetic response from the pet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reaction {
    HeadTap,
    BodyTap,
    Pet,
}

/// The semantic effect currently accepted by `Behavior`.
///
/// This is intentionally independent of dialogue and geometry.  Consumers can
/// select an optional asset reaction without reconstructing event state from
/// the rendered transform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectKind {
    CompletionObserved,
    HeadTap,
    BodyTap,
    Pet,
}

impl EffectKind {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::CompletionObserved => 0,
            Self::HeadTap => 1,
            Self::BodyTap => 2,
            Self::Pet => 3,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::CompletionObserved => "completion_observed",
            Self::HeadTap => "head_tap",
            Self::BodyTap => "body_tap",
            Self::Pet => "pet",
        }
    }

    pub const fn duration(self) -> Duration {
        match self {
            Self::CompletionObserved => COMPLETION_DURATION,
            Self::HeadTap => HEAD_TAP_DURATION,
            Self::BodyTap => BODY_TAP_DURATION,
            Self::Pet => PET_DURATION,
        }
    }
}

impl From<Reaction> for EffectKind {
    fn from(reaction: Reaction) -> Self {
        match reaction {
            Reaction::HeadTap => Self::HeadTap,
            Reaction::BodyTap => Self::BodyTap,
            Reaction::Pet => Self::Pet,
        }
    }
}

/// A frame-local snapshot of a semantically accepted transient effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectSnapshot {
    pub kind: EffectKind,
    pub started: Duration,
    pub elapsed: Duration,
    pub duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationViewport {
    pub width: f64,
    pub height: f64,
    pub backing_scale: f64,
    pub epoch: u64,
}

/// Native-owned semantic time and geometry, independent of the selected backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationIntent {
    pub now: Duration,
    pub phase: Phase,
    pub phase_age: Duration,
    pub effect: Option<EffectSnapshot>,
    pub pose: Option<PoseSnapshot>,
    pub visible: bool,
    pub pointer: Option<(f64, f64)>,
    pub viewport: PresentationViewport,
    pub frozen: bool,
}

/// The frame-local presentation of the character artwork.
///
/// Offsets are expressed in the artwork's 384x512 base coordinate system.  The
/// scale is relative to the normal artwork scale (1.0 is the unmodified frame);
/// the scene's persistent scale remains owned by the native UI geometry code.
/// `effect` is the semantic transient accepted by this same update; it is not
/// inferred from the dialogue or geometric transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presentation {
    pub offset_x: f64,
    pub offset_y: f64,
    pub scale: f64,
    pub dialogue: Option<crate::i18n::DefaultDialogue>,
    pub animate: bool,
    pub effect: Option<EffectSnapshot>,
}

impl Presentation {
    const fn identity(
        dialogue: Option<crate::i18n::DefaultDialogue>,
        animate: bool,
        effect: Option<EffectSnapshot>,
    ) -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            dialogue,
            animate,
            effect,
        }
    }
}

const IDLE_PERIOD: Duration = Duration::from_millis(3_600);
const COMPLETION_DURATION: Duration = Duration::from_millis(900);
const COMPLETION_COOLDOWN: Duration = Duration::from_millis(1_000);
const HEAD_TAP_DURATION: Duration = Duration::from_millis(460);
const BODY_TAP_DURATION: Duration = Duration::from_millis(520);
const PET_DURATION: Duration = Duration::from_millis(900);
const REACTION_COOLDOWN: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug)]
enum Active {
    Completion { started: Duration },
    Reaction { kind: Reaction, started: Duration },
}

impl Active {
    fn expired(self, now: Duration) -> bool {
        let (started, duration) = match self {
            Self::Completion { started } => (started, COMPLETION_DURATION),
            Self::Reaction { kind, started } => (started, reaction_duration(kind)),
        };
        !within(now, started, duration)
    }
    fn effect(self, now: Duration) -> EffectSnapshot {
        let (kind, started, duration) = match self {
            Self::Completion { started } => {
                (EffectKind::CompletionObserved, started, COMPLETION_DURATION)
            }
            Self::Reaction { kind, started } => {
                (EffectKind::from(kind), started, reaction_duration(kind))
            }
        };
        let elapsed = now.checked_sub(started).unwrap_or_default();
        EffectSnapshot {
            kind,
            started,
            elapsed,
            duration,
        }
    }
}

/// Pure, deterministic presentation state for the desktop pet.
///
/// The caller supplies monotonic elapsed time rather than this type reading a
/// clock.  This keeps all animation reproducible and leaves AppKit lifecycle
/// and timer ownership to the UI layer.
pub struct Behavior {
    last_now: Option<Duration>,
    phase: Option<Phase>,
    phase_started: Duration,
    completion_signal: bool,
    active: Option<Active>,
    last_completion_at: Option<Duration>,
    last_reaction_at: Option<Duration>,
    outcome: Option<(AgentOutcome, Duration)>,
    pose: Option<PoseKind>,
    pose_started: Duration,
}

impl Default for Behavior {
    fn default() -> Self {
        Self::new()
    }
}

impl Behavior {
    pub fn new() -> Self {
        Self {
            last_now: None,
            phase: None,
            phase_started: Duration::ZERO,
            completion_signal: false,
            active: None,
            last_completion_at: None,
            last_reaction_at: None,
            outcome: None,
            pose: None,
            pose_started: Duration::ZERO,
        }
    }

    pub fn phase_age(&self, now: Duration) -> Duration {
        now.checked_sub(self.phase_started).unwrap_or_default()
    }

    /// Only the watcher’s session-bound terminal facts enter this path.
    pub fn accept_outcome(&mut self, now: Duration, scene: &Scene, outcome: AgentOutcome) {
        if !outcome_capable(scene) || outcome == AgentOutcome::Running {
            return;
        }
        self.outcome = Some((outcome, now));
        self.active =
            (outcome == AgentOutcome::Succeeded).then_some(Active::Completion { started: now });
        self.last_completion_at = Some(now);
    }

    pub fn pose_snapshot(&mut self, now: Duration, scene: &Scene) -> PoseSnapshot {
        let kind = if let Some((outcome, _)) = self.outcome {
            match outcome {
                AgentOutcome::Failed => PoseKind::Failed,
                AgentOutcome::Cancelled => PoseKind::Cancelled,
                AgentOutcome::Succeeded => PoseKind::Happy,
                AgentOutcome::Running => unreachable!("running is not a terminal outcome"),
            }
        } else if let Some(active) = self.active {
            match active.effect(now).kind {
                EffectKind::CompletionObserved => PoseKind::Happy,
                EffectKind::HeadTap => PoseKind::HeadTap,
                EffectKind::BodyTap => PoseKind::TorsoTap,
                EffectKind::Pet => PoseKind::HeadPet,
            }
        } else {
            match scene.phase {
                Phase::Running => PoseKind::Writing,
                Phase::Idle if self.phase_age(now) >= Duration::from_secs(30) => PoseKind::Bored,
                Phase::Unknown
                    if scene.connected_sources == 0 && scene.disconnected_sources > 0 =>
                {
                    PoseKind::Disconnected
                }
                _ => PoseKind::Waiting,
            }
        };
        if self.pose != Some(kind) {
            self.pose = Some(kind);
            self.pose_started = now;
        }
        PoseSnapshot {
            kind,
            age: now.checked_sub(self.pose_started).unwrap_or_default(),
        }
    }

    /// Advance the behavior state and return the frame to display.
    ///
    /// `completed` is a level supplied by the completion observer.  Only its
    /// rising edge is accepted, so holding a completion notification across
    /// timer ticks cannot restart the pulse.  `reaction` is an instantaneous
    /// input event; short bursts are bounded by the reaction cooldown.
    pub fn update(
        &mut self,
        now: Duration,
        scene: &Scene,
        completed: bool,
        reaction: Option<Reaction>,
        manipulating: bool,
    ) -> Presentation {
        self.observe_clock(now);
        if self.phase != Some(scene.phase) {
            self.phase = Some(scene.phase);
            self.phase_started = now;
        }

        // Consume the level even while hidden/offline.  An event observed while
        // the pet cannot present it must not replay after the pet is shown.
        let completion_edge = completed && !self.completion_signal;
        self.completion_signal = completed;

        if !scene.visible || scene.shutdown {
            self.active = None;
            self.outcome = None;
            return Presentation::identity(None, false, None);
        }

        // Heuristic completions require a wholly known scene. Session-bound
        // outcomes remain true even when another source is unknown or offline.
        if self.outcome.is_none()
            && !completion_capable(scene)
            && matches!(self.active, Some(Active::Completion { .. }))
        {
            self.active = None;
        }
        if self.outcome.is_some_and(|(outcome, started)| {
            let duration = if outcome == AgentOutcome::Succeeded {
                COMPLETION_DURATION
            } else {
                Duration::from_millis(2400)
            };
            !outcome_capable(scene) || !within(now, started, duration)
        }) {
            self.outcome = None;
            self.active = None;
        }
        if let Some((outcome, _)) = self.outcome {
            return if outcome == AgentOutcome::Succeeded {
                self.presentation(now, scene, manipulating)
            } else {
                Presentation::identity(None, true, None)
            };
        }

        if let Some(active) = self.active {
            if active.expired(now) {
                self.active = None;
            }
        }

        let mut completion_started = false;
        if completion_edge && completion_capable(scene) && !self.in_completion_cooldown(now) {
            self.active = Some(Active::Completion { started: now });
            self.last_completion_at = Some(now);
            completion_started = true;
        }

        // A completion event wins over a simultaneous input event.  If a
        // completion is already visible, do not replace its dialogue with a
        // touch response.  Suppressed completion edges (unknown scene or
        // cooldown) do not prevent a useful touch response.
        if !completion_started && !matches!(self.active, Some(Active::Completion { .. })) {
            if let Some(kind) = reaction {
                if !self.in_reaction_cooldown(now) {
                    self.active = Some(Active::Reaction { kind, started: now });
                    self.last_reaction_at = Some(now);
                }
            }
        }

        self.presentation(now, scene, manipulating)
    }

    fn observe_clock(&mut self, now: Duration) {
        if let Some(previous) = self.last_now {
            if now < previous {
                // A monotonic clock should never go backwards.  If a caller
                // does provide a reset value, discard transient timing rather
                // than manufacturing a huge or negative animation interval.
                self.active = None;
                self.last_completion_at = None;
                self.last_reaction_at = None;
                self.phase = None;
                self.outcome = None;
                self.pose = None;
            }
        }
        self.last_now = Some(now);
    }

    fn in_completion_cooldown(&self, now: Duration) -> bool {
        self.last_completion_at
            .map(|started| within(now, started, COMPLETION_COOLDOWN))
            .unwrap_or(false)
    }

    fn in_reaction_cooldown(&self, now: Duration) -> bool {
        self.last_reaction_at
            .map(|started| within(now, started, REACTION_COOLDOWN))
            .unwrap_or(false)
    }

    fn presentation(&self, now: Duration, scene: &Scene, manipulating: bool) -> Presentation {
        let active = self.active;
        let dialogue = active.and_then(dialogue_for);
        let effect = active.map(|active| active.effect(now));

        // Manipulation owns the artwork frame.  Keep dialogue, if any, but do
        // not move or scale the image.  An active transient still requests a
        // tick so its dialogue can expire; idle motion does not.
        if manipulating {
            // Keep the transient alive long enough for the next UI tick to
            // expire it, while leaving the image itself at its geometry-owned
            // identity frame.  Idle motion is deliberately not requested
            // during manipulation.
            return Presentation::identity(dialogue, active.is_some(), effect);
        }

        let mut presentation = match active {
            Some(Active::Completion { started }) => completion_presentation(now, started),
            Some(Active::Reaction { kind, started }) => reaction_presentation(now, kind, started),
            None if idle_capable(scene) => idle_presentation(now),
            None => Presentation::identity(None, false, None),
        };
        presentation.effect = effect;
        presentation
    }
}

fn outcome_capable(scene: &Scene) -> bool {
    scene.visible && !scene.shutdown && scene.connected_sources > 0
}

fn completion_capable(scene: &Scene) -> bool {
    scene.visible
        && !scene.shutdown
        && scene.connected_sources > 0
        && scene.disconnected_sources == 0
        && scene.unknown == 0
        && scene.phase != Phase::Unknown
}

fn idle_capable(scene: &Scene) -> bool {
    completion_capable(scene) && scene.phase == Phase::Idle
}

fn within(now: Duration, started: Duration, duration: Duration) -> bool {
    match now.checked_sub(started) {
        Some(elapsed) => elapsed < duration,
        None => false,
    }
}

fn progress(now: Duration, started: Duration, duration: Duration) -> f64 {
    let elapsed = now.checked_sub(started).unwrap_or_default();
    (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
}

fn reaction_duration(kind: Reaction) -> Duration {
    match kind {
        Reaction::HeadTap => HEAD_TAP_DURATION,
        Reaction::BodyTap => BODY_TAP_DURATION,
        Reaction::Pet => PET_DURATION,
    }
}

fn dialogue_for(active: Active) -> Option<crate::i18n::DefaultDialogue> {
    Some(match active {
        Active::Completion { .. } => crate::i18n::DefaultDialogue::Completion,
        Active::Reaction {
            kind: Reaction::HeadTap,
            ..
        } => crate::i18n::DefaultDialogue::HeadTap,
        Active::Reaction {
            kind: Reaction::BodyTap,
            ..
        } => crate::i18n::DefaultDialogue::BodyTap,
        Active::Reaction {
            kind: Reaction::Pet,
            ..
        } => crate::i18n::DefaultDialogue::Pet,
    })
}

fn idle_presentation(now: Duration) -> Presentation {
    let elapsed = now.as_secs_f64();
    let period = IDLE_PERIOD.as_secs_f64();
    let phase = elapsed.rem_euclid(period) / period * (std::f64::consts::PI * 2.0);
    let breath = phase.sin();
    Presentation {
        offset_x: (phase * 0.5).sin() * 0.65,
        offset_y: breath * 2.2,
        scale: 1.0 + breath * 0.012,
        dialogue: None,
        animate: true,
        effect: None,
    }
}

fn completion_presentation(now: Duration, started: Duration) -> Presentation {
    let p = progress(now, started, COMPLETION_DURATION);
    let envelope = (std::f64::consts::PI * p).sin().max(0.0);
    let bounce = (std::f64::consts::PI * 2.5 * p).sin() * (1.0 - p) * 1.5;
    Presentation {
        offset_x: 0.0,
        offset_y: -10.0 * envelope + bounce,
        scale: 1.0 + 0.055 * envelope,
        dialogue: Some(crate::i18n::DefaultDialogue::Completion),
        animate: true,
        effect: None,
    }
}

fn reaction_presentation(now: Duration, kind: Reaction, started: Duration) -> Presentation {
    let p = progress(now, started, reaction_duration(kind));
    let envelope = (std::f64::consts::PI * p).sin().max(0.0);
    match kind {
        Reaction::HeadTap => Presentation {
            offset_x: envelope * 4.0,
            offset_y: -envelope * 2.0,
            scale: 1.0 + envelope * 0.018,
            dialogue: Some(crate::i18n::DefaultDialogue::HeadTap),
            animate: true,
            effect: None,
        },
        Reaction::BodyTap => Presentation {
            offset_x: (std::f64::consts::PI * 2.0 * p).sin() * envelope * 1.5,
            offset_y: envelope * 3.5,
            scale: 1.0 - envelope * 0.028,
            dialogue: Some(crate::i18n::DefaultDialogue::BodyTap),
            animate: true,
            effect: None,
        },
        Reaction::Pet => Presentation {
            offset_x: (std::f64::consts::PI * 4.0 * p).sin() * envelope * 7.0,
            offset_y: -envelope * 2.0,
            scale: 1.0 + envelope * 0.015,
            dialogue: Some(crate::i18n::DefaultDialogue::Pet),
            animate: true,
            effect: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(
        phase: Phase,
        connected_sources: usize,
        disconnected_sources: usize,
        unknown: usize,
        visible: bool,
        shutdown: bool,
    ) -> Scene {
        Scene {
            phase,
            sessions: 0,
            working: 0,
            blocked: 0,
            done: 0,
            unknown,
            connected_sources,
            disconnected_sources,
            visible,
            passthrough: false,
            alpha_passthrough: false,
            bubble_visible: true,
            bubble_placement: crate::bubble::BubblePlacement::Above,
            scale: 1.0,
            reset_position_revision: 0,
            shutdown,
        }
    }

    #[test]
    fn degraded_sources_suppress_idle_and_completion() {
        let degraded = scene(Phase::Idle, 1, 1, 0, true, false);
        let mut behavior = Behavior::new();
        let presentation =
            behavior.update(Duration::from_millis(500), &degraded, true, None, false);
        assert_eq!(presentation.offset_x, 0.0);
        assert_eq!(presentation.offset_y, 0.0);
        assert_eq!(presentation.scale, 1.0);
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);

        let unknown_records = scene(Phase::Running, 1, 0, 1, true, false);
        let presentation = behavior.update(
            Duration::from_millis(900),
            &unknown_records,
            true,
            None,
            false,
        );
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);
    }

    #[test]
    fn hidden_scene_discards_completion_without_replay() {
        let visible = scene(Phase::Running, 1, 0, 0, true, false);
        let hidden = scene(Phase::Running, 1, 0, 0, false, false);
        let mut behavior = Behavior::new();

        let presentation = behavior.update(Duration::from_millis(0), &hidden, true, None, false);
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);

        let presentation =
            behavior.update(Duration::from_millis(1_100), &visible, true, None, false);
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);
    }

    #[test]
    fn completion_wins_touch_and_expires_without_replay() {
        let scene = scene(Phase::Running, 1, 0, 0, true, false);
        let mut behavior = Behavior::new();

        let presentation = behavior.update(
            Duration::from_millis(0),
            &scene,
            true,
            Some(Reaction::HeadTap),
            false,
        );
        assert_eq!(
            presentation.effect,
            Some(EffectSnapshot {
                kind: EffectKind::CompletionObserved,
                started: Duration::ZERO,
                elapsed: Duration::ZERO,
                duration: COMPLETION_DURATION,
            })
        );
        assert_eq!(
            presentation.dialogue,
            Some(crate::i18n::DefaultDialogue::Completion)
        );
        assert!(presentation.animate);

        let presentation = behavior.update(
            Duration::from_millis(300),
            &scene,
            true,
            Some(Reaction::BodyTap),
            false,
        );
        assert_eq!(
            presentation
                .effect
                .map(|effect| (effect.kind, effect.started)),
            Some((EffectKind::CompletionObserved, Duration::ZERO))
        );
        assert_eq!(
            presentation.effect.map(|effect| effect.elapsed),
            Some(Duration::from_millis(300))
        );
        assert!(presentation.scale > 1.03);

        let presentation = behavior.update(Duration::from_millis(901), &scene, true, None, false);
        assert!(presentation.effect.is_none());
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);
    }

    #[test]
    fn accepted_reaction_snapshot_survives_unknown_scene() {
        let unknown = scene(Phase::Unknown, 0, 0, 1, true, false);
        let mut behavior = Behavior::new();
        let presentation = behavior.update(
            Duration::from_millis(10),
            &unknown,
            false,
            Some(Reaction::Pet),
            false,
        );
        let effect = presentation
            .effect
            .expect("accepted reaction has a snapshot");
        assert_eq!(effect.kind, EffectKind::Pet);
        assert_eq!(effect.started, Duration::from_millis(10));
        assert_eq!(effect.elapsed, Duration::ZERO);
        assert_eq!(effect.duration, PET_DURATION);
        assert_eq!(
            presentation.dialogue,
            Some(crate::i18n::DefaultDialogue::Pet)
        );
    }

    #[test]
    fn manipulating_keeps_identity_but_ticks_transient_expiry() {
        let scene = scene(Phase::Running, 1, 0, 0, true, false);
        let mut behavior = Behavior::new();

        let presentation = behavior.update(Duration::from_millis(0), &scene, true, None, true);
        assert_eq!(presentation.offset_x, 0.0);
        assert_eq!(presentation.offset_y, 0.0);
        assert_eq!(presentation.scale, 1.0);
        assert_eq!(
            presentation.dialogue,
            Some(crate::i18n::DefaultDialogue::Completion)
        );
        assert!(presentation.animate);

        let presentation = behavior.update(Duration::from_millis(901), &scene, true, None, true);
        assert_eq!(presentation.offset_x, 0.0);
        assert_eq!(presentation.offset_y, 0.0);
        assert_eq!(presentation.scale, 1.0);
        assert!(presentation.dialogue.is_none());
        assert!(!presentation.animate);
    }

    #[test]
    fn backward_clock_does_not_replay_held_completion_level() {
        let scene = scene(Phase::Running, 1, 0, 0, true, false);
        let mut behavior = Behavior::new();

        let first = behavior.update(Duration::from_secs(1), &scene, true, None, false);
        assert_eq!(
            first.effect.map(|effect| (effect.kind, effect.started)),
            Some((EffectKind::CompletionObserved, Duration::from_secs(1)))
        );

        let held_after_reset =
            behavior.update(Duration::from_millis(500), &scene, true, None, false);
        assert!(held_after_reset.effect.is_none());
        assert!(!held_after_reset.animate);

        behavior.update(Duration::from_millis(600), &scene, false, None, false);
        let genuine_edge = behavior.update(Duration::from_millis(700), &scene, true, None, false);
        assert_eq!(
            genuine_edge
                .effect
                .map(|effect| (effect.kind, effect.started)),
            Some((EffectKind::CompletionObserved, Duration::from_millis(700)))
        );
    }

    #[test]
    fn idle_pose_clock_does_not_restart_phase_clock() {
        let idle = scene(Phase::Idle, 1, 0, 0, true, false);
        let mut behavior = Behavior::new();
        behavior.update(Duration::ZERO, &idle, false, None, false);
        assert_eq!(
            behavior.pose_snapshot(Duration::ZERO, &idle).kind,
            PoseKind::Waiting
        );
        let transition = Duration::from_secs(30);
        behavior.update(transition, &idle, false, None, false);
        assert_eq!(
            behavior.pose_snapshot(transition, &idle),
            PoseSnapshot {
                kind: PoseKind::Bored,
                age: Duration::ZERO
            }
        );
        let later = Duration::from_secs(31);
        behavior.update(later, &idle, false, None, false);
        assert_eq!(
            behavior.pose_snapshot(later, &idle).age,
            Duration::from_secs(1)
        );
        assert_eq!(behavior.phase_age(later), later);
        let working = scene(Phase::Running, 1, 0, 0, true, false);
        behavior.update(later, &working, false, None, false);
        assert_eq!(
            behavior.pose_snapshot(later, &working).kind,
            PoseKind::Writing
        );
    }

    #[test]
    fn authoritative_failure_wins_without_claiming_success() {
        let working = scene(Phase::Running, 1, 1, 1, true, false);
        let mut behavior = Behavior::new();
        behavior.update(Duration::ZERO, &working, false, None, false);
        let now = Duration::from_secs(1);
        behavior.accept_outcome(now, &working, AgentOutcome::Failed);
        let frame = behavior.update(now, &working, true, Some(Reaction::HeadTap), false);
        assert_eq!(behavior.pose_snapshot(now, &working).kind, PoseKind::Failed);
        assert!(frame.dialogue.is_none());
        assert!(frame.effect.is_none());
        let expired = Duration::from_millis(3400);
        behavior.update(expired, &working, true, None, false);
        assert_eq!(
            behavior.pose_snapshot(expired, &working).kind,
            PoseKind::Writing
        );
        behavior.accept_outcome(expired, &working, AgentOutcome::Cancelled);
        behavior.update(expired, &working, false, None, false);
        assert_eq!(
            behavior.pose_snapshot(expired, &working).kind,
            PoseKind::Cancelled
        );
        let offline = scene(Phase::Unknown, 0, 1, 1, true, false);
        behavior.update(Duration::from_secs(4), &offline, false, None, false);
        assert_eq!(
            behavior
                .pose_snapshot(Duration::from_secs(4), &offline)
                .kind,
            PoseKind::Disconnected
        );
    }

    #[test]
    fn authoritative_success_survives_unrelated_unknown_sources() {
        let working = scene(Phase::Running, 1, 1, 1, true, false);
        let mut behavior = Behavior::new();
        behavior.update(Duration::ZERO, &working, false, None, false);
        behavior.accept_outcome(Duration::from_secs(1), &working, AgentOutcome::Failed);
        let now = Duration::from_millis(1100);
        behavior.accept_outcome(now, &working, AgentOutcome::Succeeded);
        let frame = behavior.update(now, &working, true, None, false);
        assert_eq!(behavior.pose_snapshot(now, &working).kind, PoseKind::Happy);
        assert_eq!(
            frame.effect.map(|effect| effect.kind),
            Some(EffectKind::CompletionObserved)
        );
        let hidden = scene(Phase::Running, 1, 0, 0, false, false);
        behavior.update(Duration::from_millis(1200), &hidden, true, None, false);
        behavior.update(Duration::from_millis(1300), &working, true, None, false);
        assert_eq!(
            behavior
                .pose_snapshot(Duration::from_millis(1300), &working)
                .kind,
            PoseKind::Writing
        );
    }
}
