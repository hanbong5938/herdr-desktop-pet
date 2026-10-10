use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerSource {
    Pet,
    Bubble,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EventIdentity {
    pub(crate) source: PointerSource,
    pub(crate) window_number: i64,
    pub(crate) event_number: i64,
    pub(crate) timestamp: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PointerCapture {
    pub(crate) id: CaptureId,
    pub(crate) event: EventIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DownDecision {
    Start,
    Replace(PointerCapture),
    Duplicate,
    Reject,
}

// Keep a real mouse-up ahead of watchdog recovery, including during event tracking.
// A zero sample alone cannot distinguish a lost release from an up awaiting delivery.
const RELEASE_GRACE: Duration = Duration::from_millis(30);

#[derive(Default)]
pub(crate) struct CaptureState {
    active: Option<PointerCapture>,
    last_down: Option<EventIdentity>,
    // Earliest excluded down since the active capture began. A later excluded
    // down must not make the old capture eligible for newer terminal events.
    active_terminal_fence: Option<f64>,
    generation: u64,
    release_candidate_at: Option<Duration>,
}

impl CaptureState {
    pub(crate) fn current(&self) -> Option<PointerCapture> {
        self.active
    }

    pub(crate) fn down_decision(&self, event: EventIdentity) -> DownDecision {
        if !event.timestamp.is_finite() || event.timestamp < 0.0 {
            return DownDecision::Reject;
        }
        // An excluded newer down advances last_down, but a redelivery of the
        // original capture down is still an exact duplicate, not a new down.
        if self.active.is_some_and(|capture| capture.event == event) {
            return DownDecision::Duplicate;
        }
        if let Some(previous) = self.last_down {
            if event == previous {
                return DownDecision::Duplicate;
            }
            // Event numbers are identity metadata, not an ordering clock. A different
            // down at the same instant is ambiguous, even on another window/root.
            if event.timestamp <= previous.timestamp {
                return DownDecision::Reject;
            }
        }
        match self.active {
            Some(old) => DownDecision::Replace(old),
            None => DownDecision::Start,
        }
    }

    // Record a noncapturing left down without replacing the active owner.
    // The UI validates the excluded event's source/window against its receiver;
    // the event may legitimately come from another window than the active one.
    pub(crate) fn exclude_down(&mut self, event: EventIdentity) {
        if !event.timestamp.is_finite() || event.timestamp < 0.0 {
            return;
        }
        if self
            .last_down
            .is_some_and(|previous| event.timestamp <= previous.timestamp)
        {
            return;
        }
        self.last_down = Some(event);
        if self.active.is_some() && self.active_terminal_fence.is_none() {
            self.active_terminal_fence = Some(event.timestamp);
        }
        self.release_candidate_at = None;
    }

    // A rejected down through the latest excluded down can displace the old
    // owner's route, even though its genuine up is still before the FIRST
    // excluded down. Route coverage and terminal eligibility have different
    // upper bounds when several noncapturing downs occur.
    pub(crate) fn excluded_down_covers(&self, event: EventIdentity) -> bool {
        self.active
            .zip(self.active_terminal_fence)
            .zip(self.last_down)
            .is_some_and(|((capture, _), latest)| {
                event.timestamp.is_finite()
                    && event.timestamp >= 0.0
                    && event.timestamp > capture.event.timestamp
                    && event.timestamp <= latest.timestamp
            })
    }

    // The caller has admitted this down and ended any previous capture first.
    pub(crate) fn begin(&mut self, event: EventIdentity) -> PointerCapture {
        debug_assert!(self.active.is_none());
        debug_assert_eq!(self.down_decision(event), DownDecision::Start);
        self.generation = self
            .generation
            .checked_add(1)
            .expect("capture generation exhausted");
        let capture = PointerCapture {
            id: CaptureId(self.generation),
            event,
        };
        self.active = Some(capture);
        self.last_down = Some(event);
        self.active_terminal_fence = None;
        self.release_candidate_at = None;
        capture
    }

    pub(crate) fn matches(&self, id: CaptureId, event: EventIdentity) -> bool {
        self.active.is_some_and(|capture| {
            capture.id == id
                && capture.event.source == event.source
                && capture.event.window_number == event.window_number
                && event.timestamp.is_finite()
                && event.timestamp >= capture.event.timestamp
                && self
                    .active_terminal_fence
                    .is_none_or(|fence| event.timestamp < fence)
        })
    }

    pub(crate) fn take(&mut self, id: CaptureId) -> Option<PointerCapture> {
        if self.active.is_some_and(|capture| capture.id == id) {
            self.clear()
        } else {
            None
        }
    }

    pub(crate) fn clear(&mut self) -> Option<PointerCapture> {
        self.release_candidate_at = None;
        self.active_terminal_fence = None;
        self.active.take()
    }

    pub(crate) fn observe_left_button(
        &mut self,
        now: Duration,
        left_pressed: bool,
        queued_matching_up: bool,
    ) -> Option<CaptureId> {
        let Some(capture) = self.active else {
            self.release_candidate_at = None;
            return None;
        };
        if left_pressed || queued_matching_up {
            self.release_candidate_at = None;
            return None;
        }
        match self.release_candidate_at {
            Some(first_zero) if now >= first_zero => {
                if now > first_zero && now - first_zero >= RELEASE_GRACE {
                    Some(capture.id)
                } else {
                    None
                }
            }
            _ => {
                // Reset on clock rollback; never carry an old zero into this epoch.
                self.release_candidate_at = Some(now);
                None
            }
        }
    }

    // Recheck a deferred watchdog request at dispatch: a real up or a
    // repressed LEFT can arrive after observe_left_button returned this id.
    pub(crate) fn recovery_ready(
        &self,
        id: CaptureId,
        now: Duration,
        left_pressed: bool,
        queued_up: bool,
    ) -> bool {
        !left_pressed
            && !queued_up
            && self.active.is_some_and(|capture| capture.id == id)
            && self
                .release_candidate_at
                .and_then(|first_zero| now.checked_sub(first_zero))
                .is_some_and(|elapsed| elapsed >= RELEASE_GRACE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        source: PointerSource,
        window_number: i64,
        event_number: i64,
        timestamp: f64,
    ) -> EventIdentity {
        EventIdentity {
            source,
            window_number,
            event_number,
            timestamp,
        }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn down_admission_uses_timestamp_not_event_number() {
        let mut state = CaptureState::default();
        let first = event(PointerSource::Pet, 4, 900, 0.0);
        assert_eq!(state.current(), None);
        assert_eq!(state.down_decision(first), DownDecision::Start);
        let old = state.begin(first);
        assert_eq!(state.down_decision(first), DownDecision::Duplicate);
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 901, 0.0)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Bubble, 5, 900, 0.0)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 999, -0.1)),
            DownDecision::Reject
        );
        let fresh = event(PointerSource::Bubble, 5, 1, 0.001);
        assert_eq!(state.down_decision(fresh), DownDecision::Replace(old));
        assert_eq!(state.take(old.id), Some(old));
        assert_eq!(state.down_decision(fresh), DownDecision::Start);
        let next = state.begin(fresh);
        assert_ne!(next.id, old.id);
        assert!(next.id.0 > old.id.0);
        assert_eq!(state.down_decision(first), DownDecision::Reject);
        assert_eq!(state.down_decision(fresh), DownDecision::Duplicate);
    }

    #[test]
    fn duplicate_down_does_not_interrupt_a_subsequent_real_release() {
        let mut state = CaptureState::default();
        let down = event(PointerSource::Bubble, 3, 42, 7.0);
        let capture = state.begin(down);
        assert_eq!(state.down_decision(down), DownDecision::Duplicate);
        assert_eq!(state.current(), Some(capture));
        assert!(state.matches(capture.id, event(PointerSource::Bubble, 3, 43, 7.5)));
        assert_eq!(state.take(capture.id), Some(capture));
        assert_eq!(state.current(), None);
        assert_eq!(state.down_decision(down), DownDecision::Duplicate);
    }

    #[test]
    fn released_down_stays_duplicate_and_other_same_time_down_is_rejected() {
        let mut state = CaptureState::default();
        let down = event(PointerSource::Pet, 1, 7, 12.0);
        let capture = state.begin(down);
        assert_eq!(state.take(capture.id), Some(capture));
        assert_eq!(state.current(), None);
        assert_eq!(state.down_decision(down), DownDecision::Duplicate);
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 1, 8, 12.0)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 1, 8, 11.0)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 1, -2, 13.0)),
            DownDecision::Start
        );
    }

    #[test]
    fn invalid_down_timestamps_are_rejected_without_replacing_capture() {
        let mut state = CaptureState::default();
        for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(
                state.down_decision(event(PointerSource::Pet, 1, 1, time)),
                DownDecision::Reject
            );
        }
        let capture = state.begin(event(PointerSource::Pet, 1, 1, 2.0));
        for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(
                state.down_decision(event(PointerSource::Bubble, 2, 2, time)),
                DownDecision::Reject
            );
            assert_eq!(state.current(), Some(capture));
        }
    }

    #[test]
    fn matching_requires_current_generation_root_and_nonstale_timestamp() {
        let mut state = CaptureState::default();
        let down = event(PointerSource::Pet, 24, 30, 50.0);
        let old = state.begin(down);
        assert!(state.matches(old.id, event(PointerSource::Pet, 24, -100, 50.0)));
        assert!(state.matches(old.id, event(PointerSource::Pet, 24, 30, 51.0)));
        assert!(!state.matches(old.id, event(PointerSource::Bubble, 24, 30, 51.0)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 25, 30, 51.0)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 30, 49.99)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 30, f64::NAN)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 30, f64::INFINITY)));
        assert_eq!(state.take(CaptureId(old.id.0 + 1)), None);
        assert_eq!(state.current(), Some(old));
        assert_eq!(state.take(old.id), Some(old));
        assert!(!state.matches(old.id, down));
        let next_down = event(PointerSource::Pet, 24, 1, 52.0);
        let next = state.begin(next_down);
        assert!(!state.matches(old.id, next_down));
        assert_eq!(state.take(old.id), None);
        assert_eq!(state.current(), Some(next));
    }

    #[test]
    fn excluded_down_fences_only_later_terminals_without_stealing_old_capture() {
        let mut state = CaptureState::default();
        let original = event(PointerSource::Pet, 24, 30, 10.0);
        let old = state.begin(original);
        let control = event(PointerSource::Bubble, 25, 1, 13.0);
        state.exclude_down(control);

        assert_eq!(state.current(), Some(old));
        assert_eq!(state.down_decision(original), DownDecision::Duplicate);
        assert_eq!(state.down_decision(control), DownDecision::Duplicate);
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 24, 31, 12.0)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 24, 31, 13.0)),
            DownDecision::Reject
        );

        // An old real release can arrive after the newer Control-down was
        // observed. Its timestamp, not its delivery time or current modifiers,
        // determines whether it belongs to the original owner.
        assert!(state.matches(old.id, event(PointerSource::Pet, 24, 31, 12.99)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 32, 13.0)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 33, 13.01)));
        assert!(!state.matches(old.id, event(PointerSource::Bubble, 25, 2, 12.99)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 25, 34, 12.99)));
        assert!(state.excluded_down_covers(event(PointerSource::Pet, 24, 40, 12.0)));
        assert!(state.excluded_down_covers(control));
        assert!(!state.excluded_down_covers(original));
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 24, 40, 13.01)));
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 24, 40, f64::NAN)));
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 24, 40, f64::INFINITY)));
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 24, 40, -1.0)));

        assert_eq!(state.take(old.id), Some(old));
        assert!(!state.excluded_down_covers(control));
        let fresh = event(PointerSource::Pet, 24, 1, 14.0);
        assert_eq!(state.down_decision(fresh), DownDecision::Start);
        let next = state.begin(fresh);
        assert_ne!(next.id, old.id);
        assert_eq!(next.id.0, old.id.0 + 1);
        assert!(state.matches(next.id, event(PointerSource::Pet, 24, 2, 14.5)));
        assert!(!state.matches(old.id, event(PointerSource::Pet, 24, 2, 14.5)));
    }

    #[test]
    fn first_excluded_down_keeps_the_terminal_fence_when_more_downs_arrive() {
        let mut state = CaptureState::default();
        let old = state.begin(event(PointerSource::Bubble, 5, 1, 1.0));
        let first = event(PointerSource::Pet, 4, 2, 2.0);
        state.exclude_down(first);
        // Same-time and stale identities do not advance the watermark.
        state.exclude_down(event(PointerSource::Pet, 4, 3, 2.0));
        state.exclude_down(event(PointerSource::Bubble, 5, 4, 1.5));
        for timestamp in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            state.exclude_down(event(PointerSource::Pet, 4, 5, timestamp));
        }
        assert!(state.matches(old.id, event(PointerSource::Bubble, 5, 6, 1.99)));
        assert!(!state.matches(old.id, event(PointerSource::Bubble, 5, 6, 2.0)));
        assert_eq!(
            state.down_decision(event(PointerSource::Bubble, 5, 7, 2.5)),
            DownDecision::Replace(old)
        );

        state.exclude_down(event(PointerSource::Bubble, 5, 8, 3.0));
        assert_eq!(state.current(), Some(old));
        assert!(state.matches(old.id, event(PointerSource::Bubble, 5, 9, 1.99)));
        assert!(!state.matches(old.id, event(PointerSource::Bubble, 5, 9, 2.5)));
        assert!(state.excluded_down_covers(event(PointerSource::Bubble, 5, 9, 2.5)));
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 10, 2.5)),
            DownDecision::Reject
        );
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 11, 3.0)),
            DownDecision::Reject
        );
        assert!(state.excluded_down_covers(event(PointerSource::Pet, 4, 11, 3.0)));
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 12, 3.01)),
            DownDecision::Replace(old)
        );
    }

    #[test]
    fn later_control_down_extends_route_coverage_without_reopening_old_capture() {
        let mut state = CaptureState::default();
        let old = state.begin(event(PointerSource::Pet, 4, 1, 10.0));
        state.exclude_down(event(PointerSource::Pet, 4, 2, 20.0));
        state.exclude_down(event(PointerSource::Pet, 4, 3, 30.0));

        let old_up = event(PointerSource::Pet, 4, 4, 15.0);
        let first_control_up = event(PointerSource::Pet, 4, 5, 25.0);
        assert!(state.matches(old.id, old_up));
        assert!(!state.matches(old.id, first_control_up));
        assert!(state.excluded_down_covers(first_control_up));
        assert_eq!(state.down_decision(first_control_up), DownDecision::Reject);
        assert!(!state.matches(old.id, event(PointerSource::Pet, 4, 6, 30.0)));
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 7, 31.0)),
            DownDecision::Replace(old)
        );
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 4, 8, 31.0)));
    }

    #[test]
    fn exclusion_without_capture_preserves_watermark_but_never_installs_a_fence() {
        let mut state = CaptureState::default();
        let control = event(PointerSource::Bubble, 5, 1, 5.0);
        state.exclude_down(control);
        assert_eq!(state.current(), None);
        assert!(!state.excluded_down_covers(control));
        assert_eq!(state.down_decision(control), DownDecision::Duplicate);
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 2, 5.0)),
            DownDecision::Reject
        );
        let primary = event(PointerSource::Pet, 4, 2, 6.0);
        assert_eq!(state.down_decision(primary), DownDecision::Start);
        let capture = state.begin(primary);
        assert!(state.matches(capture.id, event(PointerSource::Pet, 4, 3, 7.0)));
        state.exclude_down(event(PointerSource::Pet, 4, 4, 8.0));
        assert!(!state.matches(capture.id, event(PointerSource::Pet, 4, 5, 8.0)));
        assert_eq!(state.clear(), Some(capture));
        assert!(!state.excluded_down_covers(event(PointerSource::Pet, 4, 5, 7.0)));
        assert_eq!(
            state.down_decision(event(PointerSource::Pet, 4, 6, 8.0)),
            DownDecision::Reject
        );
        let next = event(PointerSource::Pet, 4, 7, 9.0);
        let restarted = state.begin(next);
        assert!(state.matches(restarted.id, event(PointerSource::Pet, 4, 8, 10.0)));
    }

    #[test]
    fn excluded_down_cancels_queued_recovery_until_new_grace_has_elapsed() {
        let mut state = CaptureState::default();
        let old = state.begin(event(PointerSource::Pet, 4, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(0), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(30), false, false),
            Some(old.id)
        );
        assert!(state.recovery_ready(old.id, ms(30), false, false));
        let control = event(PointerSource::Bubble, 5, 2, 2.0);
        state.exclude_down(control);
        assert_eq!(state.current(), Some(old));
        assert!(!state.recovery_ready(old.id, ms(31), false, false));
        assert_eq!(state.observe_left_button(ms(31), false, false), None);
        state.exclude_down(control);
        state.exclude_down(event(PointerSource::Bubble, 5, 3, f64::NAN));
        assert!(!state.recovery_ready(old.id, ms(60), false, false));
        assert!(state.recovery_ready(old.id, ms(61), false, false));
        assert_eq!(
            state.observe_left_button(ms(61), false, false),
            Some(old.id)
        );
    }

    #[test]
    fn zero_observation_needs_a_later_sample_and_grace_with_no_held_left() {
        let mut state = CaptureState::default();
        assert_eq!(state.observe_left_button(ms(0), false, false), None);
        let capture = state.begin(event(PointerSource::Pet, 1, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(100), true, false), None);
        assert_eq!(state.observe_left_button(ms(101), false, false), None);
        assert_eq!(state.observe_left_button(ms(101), false, false), None);
        assert_eq!(state.observe_left_button(ms(129), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(131), false, false),
            Some(capture.id)
        );
        assert_eq!(state.current(), Some(capture)); // policy only requests termination
        assert_eq!(state.take(capture.id), Some(capture));
        assert_eq!(state.observe_left_button(ms(999), false, false), None);
    }

    #[test]
    fn held_left_and_queued_real_up_cancel_a_staged_synthetic_release() {
        let mut state = CaptureState::default();
        let capture = state.begin(event(PointerSource::Pet, 1, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(100), false, false), None);
        assert_eq!(state.observe_left_button(ms(200), true, false), None);
        assert_eq!(state.observe_left_button(ms(201), false, false), None);
        assert_eq!(state.observe_left_button(ms(240), false, true), None);
        assert_eq!(state.observe_left_button(ms(241), false, false), None);
        assert_eq!(state.observe_left_button(ms(270), false, false), None);
        assert_eq!(state.observe_left_button(ms(271), false, true), None);
        assert_eq!(state.current(), Some(capture));
        assert_eq!(state.observe_left_button(ms(272), true, false), None);
        assert_eq!(state.observe_left_button(ms(1_000_000), true, false), None);
        assert_eq!(state.current(), Some(capture));
    }

    #[test]
    fn clock_rollback_and_new_generation_reset_release_candidate() {
        let mut state = CaptureState::default();
        let first = state.begin(event(PointerSource::Bubble, 9, 4, 2.0));
        assert_eq!(state.observe_left_button(ms(100), false, false), None);
        assert_eq!(state.observe_left_button(ms(90), false, false), None);
        assert_eq!(state.observe_left_button(ms(110), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(120), false, false),
            Some(first.id)
        );
        assert_eq!(state.clear(), Some(first));
        assert_eq!(state.clear(), None);
        let next = state.begin(event(PointerSource::Bubble, 9, 5, 3.0));
        assert_ne!(first.id, next.id);
        assert_eq!(state.observe_left_button(ms(121), false, false), None);
        assert_eq!(state.observe_left_button(ms(150), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(151), false, false),
            Some(next.id)
        );
    }

    #[test]
    fn deferred_recovery_rechecks_physical_left_and_pending_real_up() {
        let mut state = CaptureState::default();
        let capture = state.begin(event(PointerSource::Pet, 1, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(0), false, false), None);
        assert!(!state.recovery_ready(capture.id, ms(29), false, false));
        assert_eq!(
            state.observe_left_button(ms(30), false, false),
            Some(capture.id)
        );
        assert!(state.recovery_ready(capture.id, ms(30), false, false));
        // The timer queued Lost, but native state changed before its dispatch.
        assert!(!state.recovery_ready(capture.id, ms(31), true, false));
        assert!(!state.recovery_ready(capture.id, ms(31), false, true));
        assert_eq!(state.current(), Some(capture));
    }

    #[test]
    fn deferred_recovery_requires_the_original_staged_candidate() {
        let mut state = CaptureState::default();
        let capture = state.begin(event(PointerSource::Bubble, 2, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(0), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(30), false, false),
            Some(capture.id)
        );
        assert_eq!(state.observe_left_button(ms(31), true, false), None);
        assert!(!state.recovery_ready(capture.id, ms(100), false, false));
        assert_eq!(state.observe_left_button(ms(100), false, false), None);
        assert!(!state.recovery_ready(capture.id, ms(100), false, false));
        assert!(!state.recovery_ready(capture.id, ms(99), false, false));
        assert!(!state.recovery_ready(capture.id, ms(129), false, false));
        assert!(state.recovery_ready(capture.id, ms(130), false, false));

        // A pending real up also clears the staged candidate, even when
        // physical LEFT remains released.
        assert_eq!(state.observe_left_button(ms(131), false, true), None);
        assert!(!state.recovery_ready(capture.id, ms(200), false, false));
    }

    #[test]
    fn deferred_recovery_cannot_end_a_newer_capture() {
        let mut state = CaptureState::default();
        let first = state.begin(event(PointerSource::Pet, 1, 1, 1.0));
        assert_eq!(state.observe_left_button(ms(0), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(30), false, false),
            Some(first.id)
        );
        assert_eq!(state.take(first.id), Some(first));
        assert!(!state.recovery_ready(first.id, ms(30), false, false));
        let second = state.begin(event(PointerSource::Pet, 1, 2, 2.0));
        assert_eq!(state.observe_left_button(ms(31), false, false), None);
        assert_eq!(
            state.observe_left_button(ms(61), false, false),
            Some(second.id)
        );
        assert!(!state.recovery_ready(first.id, ms(61), false, false));
        assert!(state.recovery_ready(second.id, ms(61), false, false));
        assert_eq!(state.current(), Some(second));
    }
}
