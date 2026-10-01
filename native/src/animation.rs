use crate::behavior::EffectSnapshot;
use crate::state::Phase;
use std::num::NonZeroU8;
use std::time::Duration;

/// An index into the validated frame array owned by a character pack.
///
/// The index is intentionally private: only validation code can mint a frame
/// identifier, while consumers can use it to index the prepared frame array.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameId(usize);

impl FrameId {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// Return the stable phase slot used by phase clips and prepared frames.
pub(crate) const fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Idle => 0,
        Phase::Running => 1,
        Phase::Waiting => 2,
        Phase::Unknown => 3,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhaseClip {
    pub frames: Box<[FrameId]>,
    pub fps: NonZeroU8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionClip {
    pub frames: Box<[FrameId]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipSet {
    pub phases: [PhaseClip; 4],
    pub reactions: [Option<ReactionClip>; 4],
}

/// The frame selected by a playback sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackSample {
    pub frame: FrameId,
    pub use_procedural_transform: bool,
    pub needs_tick: bool,
}

/// Absolute-time playback state for one prepared clip set.
///
/// This type owns no resources and performs no I/O.  `sample` uses the supplied
/// monotonic elapsed time directly; timer delivery frequency therefore cannot
/// change which frame is selected or replay an old event.
#[derive(Debug, Default)]
pub struct Playback {
    phase: Option<Phase>,
    phase_started: Duration,
    last_now: Option<Duration>,
    was_visible: bool,
}

impl Playback {
    /// Rebase playback at a pack replacement boundary.
    ///
    /// The effect snapshot is deliberately not stored here.  Behavior remains
    /// the authority for an active effect, so a pack swap cannot restart it.
    pub fn reset_pack(&mut self, now: Duration, phase: Phase) {
        self.phase = Some(phase);
        self.phase_started = now;
        self.last_now = Some(now);
        self.was_visible = true;
    }

    /// Select a frame for `now`, or return `None` while hidden.
    pub fn sample(
        &mut self,
        now: Duration,
        phase: Phase,
        visible: bool,
        effect: Option<EffectSnapshot>,
        clips: &ClipSet,
    ) -> Option<PlaybackSample> {
        self.observe_clock(now, phase);

        if !visible {
            // A subsequent show starts the base loop at its show time rather
            // than replaying time that elapsed while the pet was hidden.
            self.was_visible = false;
            return None;
        }

        if self.phase != Some(phase) {
            self.phase = Some(phase);
            self.phase_started = now;
        } else if !self.was_visible {
            self.phase_started = now;
        }
        self.was_visible = true;

        let phase_clip = &clips.phases[phase_index(phase)];
        if phase_clip.frames.is_empty() {
            return None;
        }

        if let Some(snapshot) = effect {
            if let Some(clip) = clips
                .reactions
                .get(snapshot.kind.index())
                .and_then(Option::as_ref)
                .filter(|clip| !clip.frames.is_empty())
            {
                let frame = sample_one_shot(clip.frames.as_ref(), snapshot);
                return Some(PlaybackSample {
                    frame,
                    use_procedural_transform: false,
                    needs_tick: snapshot.elapsed < snapshot.duration || phase_clip.frames.len() > 1,
                });
            }
        }

        let frame = sample_loop(
            phase_clip.frames.as_ref(),
            phase_clip.fps,
            now.checked_sub(self.phase_started).unwrap_or_default(),
        );
        Some(PlaybackSample {
            frame,
            // A one-frame v1/v2 phase keeps the established procedural idle /
            // reaction presentation.  Missing reaction assets likewise keep
            // the procedural effect even when the base phase is animated.
            use_procedural_transform: effect.is_some() || phase_clip.frames.len() == 1,
            needs_tick: phase_clip.frames.len() > 1
                || effect
                    .map(|snapshot| snapshot.elapsed < snapshot.duration)
                    .unwrap_or(false),
        })
    }

    fn observe_clock(&mut self, now: Duration, phase: Phase) {
        if self.last_now.is_some_and(|previous| now < previous) {
            // A reset/backward clock must never turn into a huge elapsed value
            // or a wrapped frame index.  Effects are not owned here and remain
            // represented by their immutable snapshot.
            self.phase = Some(phase);
            self.phase_started = now;
            self.was_visible = false;
        }
        self.last_now = Some(now);
    }
}

fn sample_loop(frames: &[FrameId], fps: NonZeroU8, elapsed: Duration) -> FrameId {
    debug_assert!(!frames.is_empty());
    let elapsed_ns = elapsed.as_nanos();
    let frame_number = elapsed_ns
        .checked_mul(u128::from(fps.get()))
        .map(|ticks| ticks / 1_000_000_000)
        .unwrap_or(u128::MAX);
    frames[(frame_number % frames.len() as u128) as usize]
}

fn sample_one_shot(frames: &[FrameId], effect: EffectSnapshot) -> FrameId {
    debug_assert!(!frames.is_empty());
    let index = if effect.duration.is_zero() {
        frames.len() - 1
    } else {
        let elapsed_ns = effect.elapsed.as_nanos();
        let duration_ns = effect.duration.as_nanos();
        let scaled = elapsed_ns
            .checked_mul(frames.len() as u128)
            .map(|value| value / duration_ns)
            .unwrap_or(u128::MAX);
        scaled.min((frames.len() - 1) as u128) as usize
    };
    frames[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::EffectKind;

    fn clips() -> ClipSet {
        let phase = |frames: &[usize], fps: u8| PhaseClip {
            frames: frames.iter().copied().map(FrameId::new).collect(),
            fps: NonZeroU8::new(fps).unwrap(),
        };
        let reaction = |frames: &[usize]| ReactionClip {
            frames: frames.iter().copied().map(FrameId::new).collect(),
        };
        ClipSet {
            phases: [
                phase(&[0, 1, 2], 2),
                phase(&[3, 10], 1),
                phase(&[4, 5], 4),
                phase(&[6], 1),
            ],
            reactions: [Some(reaction(&[7, 8, 9])), None, None, None],
        }
    }

    fn completion(elapsed: u64) -> EffectSnapshot {
        EffectSnapshot {
            kind: EffectKind::CompletionObserved,
            started: Duration::from_millis(0),
            elapsed: Duration::from_millis(elapsed),
            duration: Duration::from_millis(900),
        }
    }

    #[test]
    fn loop_sampling_uses_absolute_floor_boundaries() {
        let clips = clips();
        let mut playback = Playback::default();
        assert_eq!(
            playback
                .sample(Duration::ZERO, Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            0
        );
        assert_eq!(
            playback
                .sample(Duration::from_millis(499), Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            0
        );
        assert_eq!(
            playback
                .sample(Duration::from_millis(500), Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            1
        );
        assert_eq!(
            playback
                .sample(Duration::from_secs(2), Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            1
        );
    }

    #[test]
    fn delayed_ticks_and_backward_clock_do_not_replay_or_wrap() {
        let clips = clips();
        let mut playback = Playback::default();
        let initial = playback
            .sample(Duration::ZERO, Phase::Idle, true, None, &clips)
            .unwrap();
        assert_eq!(initial.frame.index(), 0);
        let delayed = playback
            .sample(Duration::from_secs(10), Phase::Idle, true, None, &clips)
            .unwrap();
        assert_eq!(delayed.frame.index(), 2);
        let reset = playback
            .sample(Duration::from_millis(1), Phase::Idle, true, None, &clips)
            .unwrap();
        assert_eq!(reset.frame.index(), 0);
    }

    #[test]
    fn phase_change_rebases_base_but_effect_uses_its_elapsed_time() {
        let clips = clips();
        let mut playback = Playback::default();
        let effect = completion(450);
        let sample = playback
            .sample(
                Duration::from_millis(2_000),
                Phase::Waiting,
                true,
                Some(effect),
                &clips,
            )
            .unwrap();
        assert_eq!(sample.frame.index(), 8);
        let next = playback
            .sample(
                Duration::from_millis(2_001),
                Phase::Waiting,
                true,
                None,
                &clips,
            )
            .unwrap();
        assert_eq!(next.frame.index(), 4);
        playback.reset_pack(Duration::from_secs(3), Phase::Waiting);
        let swapped = playback
            .sample(
                Duration::from_millis(3_001),
                Phase::Waiting,
                true,
                Some(effect),
                &clips,
            )
            .unwrap();
        assert_eq!(swapped.frame.index(), 8);
    }

    #[test]
    fn hidden_show_and_pack_reset_start_the_base_loop_at_zero() {
        let clips = clips();
        let mut playback = Playback::default();
        assert!(playback
            .sample(Duration::from_secs(4), Phase::Idle, false, None, &clips)
            .is_none());
        assert_eq!(
            playback
                .sample(Duration::from_secs(9), Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            0
        );
        playback.reset_pack(Duration::from_secs(20), Phase::Idle);
        assert_eq!(
            playback
                .sample(Duration::from_secs(20), Phase::Idle, true, None, &clips)
                .unwrap()
                .frame
                .index(),
            0
        );
        assert_eq!(
            playback
                .sample(
                    Duration::from_millis(20_500),
                    Phase::Idle,
                    true,
                    None,
                    &clips,
                )
                .unwrap()
                .frame
                .index(),
            1
        );
    }

    #[test]
    fn missing_reaction_keeps_procedural_transform_and_active_clip_suppresses_it() {
        let clips = clips();
        let mut playback = Playback::default();
        let missing_effect = EffectSnapshot {
            kind: EffectKind::BodyTap,
            started: Duration::ZERO,
            elapsed: Duration::ZERO,
            duration: Duration::from_millis(520),
        };
        let missing = playback
            .sample(
                Duration::ZERO,
                Phase::Running,
                true,
                Some(missing_effect),
                &clips,
            )
            .unwrap();
        assert!(missing.use_procedural_transform);
        let active = playback
            .sample(
                Duration::ZERO,
                Phase::Idle,
                true,
                Some(completion(0)),
                &clips,
            )
            .unwrap();
        assert!(!active.use_procedural_transform);
        assert_eq!(active.frame.index(), 7);
    }
}
