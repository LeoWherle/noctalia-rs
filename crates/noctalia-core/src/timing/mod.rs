//! Port of `src/core/timer_manager.{cpp,h}`, `src/core/deferred_call.{cpp,h}`, and
//! `src/core/frame_rate_limiter.h` — Phase 1 task 1.4.
//!
//! The C++ `TimerManager` is a hand-rolled scheduler (a due-time map plus
//! `pollTimeoutMs()`/`tick()` driven by the C++ codebase's own poll loop) because
//! C++ had no real event-loop library to delegate to. Architecture decision 1 puts
//! `calloop` on the main thread instead, and calloop's own [`calloop::timer::Timer`]
//! source already *is* a due-time scheduler — reimplementing one on top of it would
//! just be duplicated bookkeeping. So `timer::Timer<Data>` here is a thin wrapper
//! around a single calloop timer registration, and there is no `TimerManager`
//! singleton: each `Timer` owns its own registration against a `LoopHandle`.
//! Likewise `deferred::DeferredCall<Data>` wraps `calloop::channel` instead of a
//! hand-rolled wake-pipe + mutex-guarded queue (the C++ `wakeFd()`/`drainWakeFd()`
//! pair has no port here — `calloop::channel`'s internal ping source already does
//! that job).
//!
//! Every callback here takes `&mut Data`, calloop's shared per-loop state — the
//! C++ callbacks take no arguments because C++ has no such concept (its closures
//! captured whatever ambient state/`this` pointers they needed instead). This is
//! the natural calloop idiom, not a gratuitous change: every consumer of these
//! types further down the migration will be threading the same `Data` through.

mod deferred;
mod frame_rate_limiter;
mod timer;

pub use deferred::DeferredCall;
pub use frame_rate_limiter::FrameRateLimiter;
pub use timer::Timer;
