//! Port of `src/time/time_service.{cpp,h}`. `time_poll_source.h`
//! (`TimePollSource`) has no port — see [`super`]'s module doc comment.
//!
//! `SecondPoint` (C++'s `time_point<Clock, seconds>`, used purely to detect
//! a whole-second boundary crossing) becomes a plain `i64` Unix-seconds
//! count here rather than a second `SystemTime`-derived type: Rust has no
//! second-granularity clock-point type to mirror it with, and an integer
//! comparison is what the C++'s `floored != m_nowSeconds` check reduces to
//! anyway.

use std::time::SystemTime;

use super::system_time_to_unix_seconds;

/// Called every poll iteration. Tracks both full-precision and
/// seconds-precision time; the tick callback fires once per second
/// boundary.
pub struct TimeService {
    callback: Option<Box<dyn FnMut()>>,
    now: SystemTime,
    now_seconds: i64,
}

impl TimeService {
    pub fn new() -> Self {
        let now = SystemTime::now();
        Self {
            callback: None,
            now_seconds: system_time_to_unix_seconds(now),
            now,
        }
    }

    pub fn set_tick_second_callback(&mut self, callback: impl FnMut() + 'static) {
        self.callback = Some(Box::new(callback));
    }

    pub fn poll_timeout_ms(&self) -> i32 {
        let since_epoch = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let remaining_ms = (1_000_000_000i64 - i64::from(since_epoch.subsec_nanos())) / 1_000_000;
        remaining_ms.max(1) as i32
    }

    pub fn tick(&mut self) {
        self.now = SystemTime::now();
        let floored = system_time_to_unix_seconds(self.now);
        if floored != self.now_seconds {
            self.now_seconds = floored;
            if let Some(callback) = &mut self.callback {
                callback();
            }
        }
    }

    pub fn now(&self) -> SystemTime {
        self.now
    }
}

impl Default for TimeService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[test]
    fn poll_timeout_ms_is_within_one_second() {
        let service = TimeService::new();
        let ms = service.poll_timeout_ms();
        assert!((1..=1000).contains(&ms), "ms={ms}");
    }

    #[test]
    fn now_reflects_the_last_tick() {
        let mut service = TimeService::new();
        let before = service.now();
        std::thread::sleep(Duration::from_millis(10));
        service.tick();
        assert!(service.now() > before);
    }

    #[test]
    fn tick_fires_the_callback_once_per_second_boundary_crossed() {
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = Arc::clone(&count);
        let mut service = TimeService::new();
        service.set_tick_second_callback(move || {
            count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Same second as construction: no boundary crossed yet.
        service.tick();
        assert_eq!(count.load(Ordering::SeqCst), 0);

        // Sleep past the next second boundary, then confirm exactly one fire.
        std::thread::sleep(Duration::from_millis(1100));
        service.tick();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // A same-second re-tick doesn't fire again.
        service.tick();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
