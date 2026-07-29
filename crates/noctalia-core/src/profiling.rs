//! Port of `src/core/scoped_timer.h`.
//!
//! Lightweight, opt-in wall-clock profiling. Output is gated behind the
//! `NOCTALIA_PROFILE` env var so normal runs stay silent; set it to any
//! non-empty value to surface timing lines (e.g. `NOCTALIA_PROFILE=1 noctalia`).

use std::sync::OnceLock;
use std::time::Instant;

use crate::log::Logger;

/// Evaluated once on first use, matching the C++'s function-local `static const`.
pub fn enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| std::env::var_os("NOCTALIA_PROFILE").is_some_and(|v| !v.is_empty()))
}

/// Elapsed milliseconds since construction (or last [`reset`](StopWatch::reset)).
pub struct StopWatch {
    start: Instant,
}

impl StopWatch {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    pub fn reset(&mut self) {
        self.start = Instant::now();
    }
}

impl Default for StopWatch {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII timer: logs `"<label>: <ms> ms"` at info level on drop when profiling is
/// enabled, and is a near-free no-op otherwise. Matches `ScopedTimer`.
pub struct ScopedTimer {
    log: Logger,
    label: String,
    active: bool,
    watch: StopWatch,
}

impl ScopedTimer {
    pub fn new(log: Logger, label: impl Into<String>) -> Self {
        Self {
            log,
            label: label.into(),
            active: enabled(),
            watch: StopWatch::new(),
        }
    }
}

impl Drop for ScopedTimer {
    fn drop(&mut self) {
        if self.active {
            self.log.info(format_args!(
                "{}: {:.1} ms",
                self.label,
                self.watch.elapsed_ms()
            ));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn stopwatch_elapsed_is_nonnegative_and_reset_restarts_it() {
        let mut watch = StopWatch::new();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let elapsed = watch.elapsed_ms();
        assert!(elapsed >= 5.0);
        watch.reset();
        assert!(watch.elapsed_ms() < elapsed);
    }

    #[test]
    fn scoped_timer_does_not_panic_on_drop() {
        let _timer = ScopedTimer::new(Logger::new("test"), "op");
    }
}
