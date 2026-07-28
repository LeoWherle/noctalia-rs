//! Port of `src/core/timer_manager.{cpp,h}`'s `Timer` class (see the module doc
//! comment in `timing::mod` for why there is no `TimerManager` singleton here).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer as CalloopTimer};
use calloop::{LoopHandle, RegistrationToken};

struct Shared {
    active: Cell<bool>,
}

/// A single-shot or repeating timer registered against a calloop event loop.
///
/// Dropping a `Timer` stops it, same as the C++ `Timer`'s destructor. Unlike the
/// C++ type, no move constructor is needed: Rust's ordinary move semantics
/// already leave the source `Timer` un-droppable (so its `Drop` impl can't
/// double-stop a registration the destination now owns).
pub struct Timer<Data> {
    handle: LoopHandle<'static, Data>,
    token: Option<RegistrationToken>,
    shared: Rc<Shared>,
}

impl<Data> Timer<Data> {
    pub fn new(handle: LoopHandle<'static, Data>) -> Self {
        Self {
            handle,
            token: None,
            shared: Rc::new(Shared {
                active: Cell::new(false),
            }),
        }
    }

    /// Starts (or restarts, cancelling any prior registration first — matching
    /// `TimerManager::start`'s `existingId != 0 -> cancel(existingId)`) a one-shot
    /// timer. `delay` isn't clamped to non-negative the way the C++ does with
    /// `std::max(delay, 0ms)`: `Duration` can't represent a negative value in the
    /// first place, so the clamp has nothing left to do.
    pub fn start(&mut self, delay: Duration, callback: impl FnMut(&mut Data) + 'static) {
        self.register(delay, None, callback);
    }

    /// Starts (or restarts) a repeating timer. Matching calloop's native
    /// repeat-via-return-value design, the next firing is scheduled
    /// `interval` after the *previous callback returns* (not from its
    /// original due time) — the same drift-permitting behavior as the C++,
    /// which recomputes `dueAt` from `steady_clock::now()` after the callback
    /// runs.
    pub fn start_repeating(
        &mut self,
        interval: Duration,
        callback: impl FnMut(&mut Data) + 'static,
    ) {
        self.register(interval, Some(interval), callback);
    }

    fn register(
        &mut self,
        delay: Duration,
        interval: Option<Duration>,
        mut callback: impl FnMut(&mut Data) + 'static,
    ) {
        self.stop();

        let shared = Rc::new(Shared {
            active: Cell::new(true),
        });
        self.shared = Rc::clone(&shared);

        let source = CalloopTimer::from_duration(delay);
        let inserted = self
            .handle
            .insert_source(source, move |_deadline, (), data| {
                // Reentrant self-stop (the callback called `stop()` on this very
                // `Timer` — see `stop()`'s SAFETY-equivalent reasoning below) means
                // don't run the callback body again or reschedule.
                if !shared.active.get() {
                    return TimeoutAction::Drop;
                }
                callback(data);
                if !shared.active.get() {
                    return TimeoutAction::Drop;
                }
                match interval {
                    Some(interval) => TimeoutAction::ToDuration(interval),
                    None => {
                        shared.active.set(false);
                        TimeoutAction::Drop
                    }
                }
            });

        match inserted {
            Ok(token) => self.token = Some(token),
            Err(_) => {
                // Registering a fresh timer source with a live calloop poll
                // backend essentially never fails; if it somehow does, degrade
                // to "never fires" rather than panicking — mirrors
                // `TimerManager::start`'s `if (!callback) return 0;` early-out
                // to an inactive id.
                self.shared.active.set(false);
            }
        }
    }

    /// Cancels this timer. Safe to call from within the timer's own callback —
    /// this is how a repeating timer can stop itself, matching
    /// `TimerManager::cancel`'s `canceledTimerIds`/`inFlightTimerIds` handling of
    /// self-cancellation. Calloop's dispatch loop clones the dispatcher out of
    /// its slot before invoking the callback, so `handle.remove`'s outer borrow
    /// doesn't panic on reentry; its inner per-source unregister borrow can
    /// still be busy at that point, in which case calloop just retries the
    /// unregister itself right after the callback returns — either way the
    /// registration ends up removed, with no double-fire and no leak. The
    /// `shared.active` flag is the real belt-and-suspenders here: the wrapper
    /// closure rechecks it after the callback runs and returns `Drop` instead
    /// of rescheduling, regardless of exactly when calloop finishes tearing
    /// down the old registration.
    pub fn stop(&mut self) {
        self.shared.active.set(false);
        if let Some(token) = self.token.take() {
            self.handle.remove(token);
        }
    }

    pub fn active(&self) -> bool {
        self.shared.active.get()
    }
}

impl<Data> Drop for Timer<Data> {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use calloop::EventLoop;

    #[test]
    fn fires_ordered_one_shot_timers_deterministically() {
        let mut event_loop: EventLoop<'static, Vec<&'static str>> =
            EventLoop::try_new().expect("event loop");
        let mut order: Vec<&'static str> = Vec::new();

        let mut fast = Timer::new(event_loop.handle());
        let mut slow = Timer::new(event_loop.handle());
        slow.start(
            Duration::from_millis(60),
            |order: &mut Vec<&'static str>| order.push("slow"),
        );
        fast.start(
            Duration::from_millis(20),
            |order: &mut Vec<&'static str>| order.push("fast"),
        );

        event_loop
            .dispatch(Some(Duration::from_millis(30)), &mut order)
            .expect("dispatch");
        assert_eq!(order, vec!["fast"]);

        event_loop
            .dispatch(Some(Duration::from_millis(60)), &mut order)
            .expect("dispatch");
        assert_eq!(order, vec!["fast", "slow"]);
    }

    #[test]
    fn one_shot_timer_becomes_inactive_after_firing() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut count = 0u32;

        let mut timer = Timer::new(event_loop.handle());
        timer.start(Duration::from_millis(10), |count: &mut u32| *count += 1);
        assert!(timer.active());

        event_loop
            .dispatch(Some(Duration::from_millis(20)), &mut count)
            .expect("dispatch");
        assert_eq!(count, 1);
        assert!(!timer.active());
    }

    #[test]
    fn stop_before_due_prevents_firing() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut count = 0u32;

        let mut timer = Timer::new(event_loop.handle());
        timer.start(Duration::from_millis(10), |count: &mut u32| *count += 1);
        timer.stop();
        assert!(!timer.active());

        event_loop
            .dispatch(Some(Duration::from_millis(20)), &mut count)
            .expect("dispatch");
        assert_eq!(count, 0);
    }

    #[test]
    fn repeating_timer_fires_multiple_times() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut count = 0u32;

        let mut timer = Timer::new(event_loop.handle());
        timer.start_repeating(Duration::from_millis(20), |count: &mut u32| *count += 1);

        event_loop
            .dispatch(Some(Duration::from_millis(30)), &mut count)
            .expect("dispatch");
        assert_eq!(count, 1);
        event_loop
            .dispatch(Some(Duration::from_millis(30)), &mut count)
            .expect("dispatch");
        assert_eq!(count, 2);
        event_loop
            .dispatch(Some(Duration::from_millis(30)), &mut count)
            .expect("dispatch");
        assert_eq!(count, 3);
        assert!(timer.active());
    }

    #[test]
    fn repeating_timer_can_stop_itself_from_its_own_callback() {
        let mut event_loop: EventLoop<'static, u32> = EventLoop::try_new().expect("event loop");
        let mut count = 0u32;

        let timer = Rc::new(std::cell::RefCell::new(Timer::new(event_loop.handle())));
        let timer_for_callback = Rc::clone(&timer);
        timer
            .borrow_mut()
            .start_repeating(Duration::from_millis(15), move |count: &mut u32| {
                *count += 1;
                if *count == 2 {
                    timer_for_callback.borrow_mut().stop();
                }
            });

        for _ in 0..5 {
            event_loop
                .dispatch(Some(Duration::from_millis(20)), &mut count)
                .expect("dispatch");
        }

        assert_eq!(
            count, 2,
            "timer should have stopped itself after its second firing"
        );
        assert!(!timer.borrow().active());
    }

    #[test]
    fn restarting_replaces_the_previous_registration() {
        let mut event_loop: EventLoop<'static, Vec<u32>> =
            EventLoop::try_new().expect("event loop");
        let mut fired: Vec<u32> = Vec::new();

        let mut timer = Timer::new(event_loop.handle());
        timer.start(Duration::from_millis(10), |fired: &mut Vec<u32>| {
            fired.push(1)
        });
        timer.start(Duration::from_millis(10), |fired: &mut Vec<u32>| {
            fired.push(2)
        });

        event_loop
            .dispatch(Some(Duration::from_millis(20)), &mut fired)
            .expect("dispatch");
        assert_eq!(
            fired,
            vec![2],
            "restarting should cancel the first registration, not run both"
        );
    }
}
