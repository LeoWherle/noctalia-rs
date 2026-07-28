//! Port of `src/core/frame_rate_limiter.h`.

use std::time::{Duration, Instant};

use calloop::LoopHandle;

use super::Timer;

/// Caps animation-driven repaints to a fixed rate regardless of the compositor's
/// frame-callback rate — see the C++ header's comment for the full rationale
/// (repaint on `shouldStep()`, which arms a revive timer near the interval edge
/// when it returns `false` so the animation doesn't stall once the frame loop
/// goes idle).
pub struct FrameRateLimiter<Data> {
    interval: Duration,
    last_step_at: Option<Instant>,
    revive: Timer<Data>,
}

const DEFAULT_INTERVAL: Duration = Duration::from_millis(33);
const MIN_REVIVE_DELAY: Duration = Duration::from_millis(1);

impl<Data> FrameRateLimiter<Data> {
    pub fn new(handle: LoopHandle<'static, Data>, interval: Duration) -> Self {
        Self {
            interval,
            last_step_at: None,
            revive: Timer::new(handle),
        }
    }

    pub fn with_default_interval(handle: LoopHandle<'static, Data>) -> Self {
        Self::new(handle, DEFAULT_INTERVAL)
    }

    /// Call from a per-frame tick. Returns `true` when enough time has elapsed
    /// to run another step; otherwise arms `revive_cb` to fire near the interval
    /// edge and returns `false` so the caller skips this frame's update+redraw.
    pub fn should_step(&mut self, revive_cb: impl FnMut(&mut Data) + 'static) -> bool {
        let now = Instant::now();
        // No prior step (or `reset()` since): treat as "arbitrarily overdue", the
        // same effect as the C++ default-constructed `time_point{}` producing a
        // huge `since`.
        let since = self
            .last_step_at
            .map_or(Duration::MAX, |last| now.saturating_duration_since(last));

        if since < self.interval {
            if !self.revive.active() {
                let remaining = self.interval - since;
                self.revive
                    .start(remaining.max(MIN_REVIVE_DELAY), revive_cb);
            }
            return false;
        }

        self.last_step_at = Some(now);
        true
    }

    /// Cancels any pending revive and lets the next `should_step()` run immediately.
    pub fn reset(&mut self) {
        self.last_step_at = None;
        self.revive.stop();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use calloop::EventLoop;

    #[test]
    fn first_step_always_runs() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().expect("event loop");
        let mut limiter = FrameRateLimiter::new(event_loop.handle(), Duration::from_millis(30));
        assert!(limiter.should_step(|_| {}));
    }

    #[test]
    fn rapid_steps_are_throttled_then_allowed_after_the_interval() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut revives = 0u32;
        let mut limiter = FrameRateLimiter::new(event_loop.handle(), Duration::from_millis(30));

        assert!(limiter.should_step(|revives: &mut u32| *revives += 1));
        assert!(
            !limiter.should_step(|revives: &mut u32| *revives += 1),
            "second immediate step should be throttled"
        );

        event_loop
            .dispatch(Some(Duration::from_millis(40)), &mut revives)
            .expect("dispatch");
        assert_eq!(
            revives, 1,
            "the armed revive callback should have fired once the interval passed"
        );

        assert!(
            limiter.should_step(|revives: &mut u32| *revives += 1),
            "should_step should be allowed again"
        );
    }

    #[test]
    fn reset_lets_the_next_step_run_immediately_and_cancels_the_revive() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut revives = 0u32;
        let mut limiter = FrameRateLimiter::new(event_loop.handle(), Duration::from_millis(30));

        assert!(limiter.should_step(|revives: &mut u32| *revives += 1));
        assert!(!limiter.should_step(|revives: &mut u32| *revives += 1));

        limiter.reset();
        assert!(
            limiter.should_step(|revives: &mut u32| *revives += 1),
            "reset should let the next step run"
        );

        event_loop
            .dispatch(Some(Duration::from_millis(40)), &mut revives)
            .expect("dispatch");
        assert_eq!(
            revives, 0,
            "reset should have cancelled the pending revive timer"
        );
    }
}
