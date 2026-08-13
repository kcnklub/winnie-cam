//! Turns a raw per-sample "is the masked region moving right now" boolean
//! into discrete `motion_started`/`motion_stopped` events, debounced so a
//! flickering score doesn't machine-gun events.
//!
//! Pure state machine: `now` is passed in by the caller on every call
//! rather than read from the clock internally, following the same pattern
//! as `detect::Throttle::should_run` - it's what makes this trivially
//! unit-testable with fake time instead of real sleeps.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct DebounceConfig {
    /// How long motion must persist before `Started` fires.
    pub sustain: Duration,
    /// How long stillness must persist (after the last moving sample)
    /// before `Stopped` fires.
    pub quiet: Duration,
    /// Minimum gap after a `Stopped` before a new episode can start.
    pub cooldown: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    Started,
    Stopped { duration_ms: u64 },
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Still,
    Rising { since: Instant },
    Active { since: Instant, last_moving: Instant },
}

/// One monitored subject's motion state. `motion::worker_loop` owns exactly
/// one of these for the lifetime of the process.
pub struct MotionDetector {
    phase: Phase,
    last_stopped: Option<Instant>,
    cfg: DebounceConfig,
}

impl MotionDetector {
    pub fn new(cfg: DebounceConfig) -> Self {
        Self { phase: Phase::Still, last_stopped: None, cfg }
    }

    /// Feed one sample's motion verdict (`moving`, from a masked frame
    /// diff). Returns `Some` exactly on the sample that crosses a
    /// start/stop boundary.
    pub fn observe(&mut self, now: Instant, moving: bool) -> Option<MotionKind> {
        match self.phase {
            Phase::Still => {
                if !moving {
                    return None;
                }
                // A sustain timer that starts *during* cooldown would let a
                // continuously-moving subject promote to `Active` the
                // instant cooldown expires, rather than needing a fresh
                // `sustain` window on top of it - see this module's doc
                // comment on why that's the intended behaviour, not a gap.
                self.phase = Phase::Rising { since: now };
                None
            }
            Phase::Rising { since } => {
                if !moving {
                    self.phase = Phase::Still;
                    return None;
                }
                if now.duration_since(since) < self.cfg.sustain {
                    return None;
                }
                let in_cooldown =
                    self.last_stopped.is_some_and(|t| now < t + self.cfg.cooldown);
                if in_cooldown {
                    // Sustain met, but still cooling down from the last
                    // episode: stay in `Rising` (not `Still`) so continued
                    // motion promotes the instant cooldown lapses, instead
                    // of needing another full `sustain` window on top of it.
                    return None;
                }
                self.phase = Phase::Active { since, last_moving: now };
                Some(MotionKind::Started)
            }
            Phase::Active { since, last_moving } => {
                if moving {
                    self.phase = Phase::Active { since, last_moving: now };
                    return None;
                }
                if now.duration_since(last_moving) < self.cfg.quiet {
                    return None;
                }
                let duration_ms = last_moving.duration_since(since).as_millis() as u64;
                self.phase = Phase::Still;
                self.last_stopped = Some(now);
                Some(MotionKind::Stopped { duration_ms })
            }
        }
    }

    /// Force-ends an in-flight episode without waiting out the `quiet`
    /// window - used when the subject leaves the frame entirely (no person
    /// box to sample motion against) or at shutdown. A `Rising` episode
    /// that hasn't reached `sustain` yet just resets, since it was never
    /// reported as `Started`.
    pub fn close(&mut self, now: Instant) -> Option<MotionKind> {
        match self.phase {
            Phase::Active { since, .. } => {
                let duration_ms = now.duration_since(since).as_millis() as u64;
                self.phase = Phase::Still;
                self.last_stopped = Some(now);
                Some(MotionKind::Stopped { duration_ms })
            }
            Phase::Rising { .. } => {
                self.phase = Phase::Still;
                None
            }
            Phase::Still => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DebounceConfig {
        DebounceConfig {
            sustain: Duration::from_millis(700),
            quiet: Duration::from_millis(4000),
            cooldown: Duration::from_millis(3000),
        }
    }

    #[test]
    fn motion_shorter_than_sustain_fires_nothing() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        assert_eq!(d.observe(t0, true), None);
        assert_eq!(d.observe(t0 + Duration::from_millis(300), true), None);
        // Goes still before sustain is met.
        assert_eq!(d.observe(t0 + Duration::from_millis(500), false), None);
    }

    #[test]
    fn sustained_motion_fires_started_exactly_once() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        assert_eq!(d.observe(t0, true), None);
        assert_eq!(
            d.observe(t0 + Duration::from_millis(750), true),
            Some(MotionKind::Started)
        );
        // Still moving afterwards - no repeat event.
        assert_eq!(d.observe(t0 + Duration::from_millis(900), true), None);
    }

    #[test]
    fn a_brief_still_gap_under_quiet_does_not_stop_it() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        d.observe(t0, true);
        d.observe(t0 + Duration::from_millis(750), true); // Started
        // A couple of still samples well inside the 4s quiet window.
        assert_eq!(d.observe(t0 + Duration::from_millis(1200), false), None);
        assert_eq!(d.observe(t0 + Duration::from_millis(1500), false), None);
        // Motion resumes - still the same episode, no new Started.
        assert_eq!(d.observe(t0 + Duration::from_millis(1800), true), None);
    }

    #[test]
    fn quiet_elapsing_fires_stopped_with_the_right_duration() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        d.observe(t0, true); // Rising since t0
        d.observe(t0 + Duration::from_millis(750), true); // Started
        let last_moving_at = t0 + Duration::from_millis(2000);
        d.observe(last_moving_at, true); // last moving sample
        let stop_at = last_moving_at + Duration::from_millis(4000);
        let event = d.observe(stop_at, false);
        match event {
            // Duration is measured from the first observed movement
            // (Rising's `since`, t0) to the last moving sample, not from
            // whenever `Started` happened to fire - that's the more
            // meaningful "how long did motion actually last".
            Some(MotionKind::Stopped { duration_ms }) => assert_eq!(duration_ms, 2000),
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    #[test]
    fn cooldown_blocks_an_immediate_restart_but_allows_one_after_it_expires() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        d.observe(t0, true); // Rising since t0
        d.observe(t0 + Duration::from_millis(750), true); // Started
        // A still sample well inside the 4s quiet window - not a stop yet.
        assert_eq!(d.observe(t0 + Duration::from_millis(800), false), None);
        // Now push past the quiet window (measured from the last moving
        // sample at 750ms): this is what actually stops it.
        let stopped_at = t0 + Duration::from_millis(4800);
        assert!(matches!(d.observe(stopped_at, false), Some(MotionKind::Stopped { .. })));

        // Immediately moving again: sustain elapses, but cooldown (3s) is
        // still active, so no second Started yet.
        let restart_t0 = stopped_at;
        d.observe(restart_t0, true);
        assert_eq!(d.observe(restart_t0 + Duration::from_millis(750), true), None);

        // Once cooldown has elapsed, continued motion promotes immediately
        // (no need to wait out another full sustain window).
        let after_cooldown = restart_t0 + Duration::from_millis(3100);
        assert_eq!(
            d.observe(after_cooldown, true),
            Some(MotionKind::Started)
        );
    }

    #[test]
    fn close_ends_an_active_episode_immediately() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        d.observe(t0, true); // Rising since t0
        d.observe(t0 + Duration::from_millis(750), true); // Started
        let gone_at = t0 + Duration::from_millis(2000);
        let event = d.close(gone_at);
        match event {
            // Measured from t0 (first observed movement), same as the
            // quiet-window path above.
            Some(MotionKind::Stopped { duration_ms }) => assert_eq!(duration_ms, 2000),
            other => panic!("expected Stopped, got {other:?}"),
        }
        // A second close on an already-Still detector is a no-op.
        assert_eq!(d.close(gone_at + Duration::from_millis(10)), None);
    }

    #[test]
    fn close_during_rising_resets_without_emitting() {
        let mut d = MotionDetector::new(cfg());
        let t0 = Instant::now();
        d.observe(t0, true); // Rising, never reached Started
        assert_eq!(d.close(t0 + Duration::from_millis(100)), None);
    }
}
