//! Port of `src/core/deferred_call.{cpp,h}`.
//!
//! The C++ version hand-rolls a self-pipe (`ensureWakePipe`/`wakeMainLoop`) plus a
//! mutex-guarded queue so any thread can schedule a closure to run on the main
//! loop thread. `calloop::channel` is exactly that primitive, already built and
//! already the sanctioned tokio-sidecar-to-main-loop bridge per architecture
//! decision 1 — so there is no port of `wakeFd()`/`drainWakeFd()`:
//! `calloop::channel`'s own internal ping source does that job, invisibly.

use calloop::LoopHandle;
use calloop::channel::{self, Event};

type Job<Data> = Box<dyn FnOnce(&mut Data) + Send>;

/// A queue of closures to run on the loop thread, safe to schedule work onto
/// from any thread. Clone [`DeferredCall::sender`]'s `Sender` and hand it to
/// whichever threads need to defer work — the same "clone and distribute" idiom
/// `calloop::channel` uses everywhere else in this migration (e.g. the tokio
/// sidecar bridge in task 1.5).
pub struct DeferredCall<Data> {
    sender: channel::Sender<Job<Data>>,
}

impl<Data: 'static> DeferredCall<Data> {
    /// Registers the channel's receiving end as a calloop source. Fails only if
    /// calloop can't register the underlying ping source with the poll backend —
    /// essentially unreachable for a freshly created channel against a live event
    /// loop, but propagated rather than unwrapped since it's not provably
    /// impossible.
    pub fn new(handle: &LoopHandle<'static, Data>) -> Result<Self, calloop::Error> {
        let (sender, receiver) = channel::channel::<Job<Data>>();
        handle.insert_source(receiver, |event, (), data| {
            if let Event::Msg(job) = event {
                job(data);
            }
        })?;
        Ok(Self { sender })
    }

    /// A cloneable handle that schedules `f` to run on the loop thread the next
    /// time it dispatches — from any thread, matching `DeferredCall::callLater`.
    pub fn sender(&self) -> channel::Sender<Job<Data>> {
        self.sender.clone()
    }

    pub fn call_later(&self, f: impl FnOnce(&mut Data) + Send + 'static) {
        // An error here only means the receiver (this same `DeferredCall`) was
        // dropped; nothing to run the closure on, so there's nothing to do.
        let _ = self.sender.send(Box::new(f));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use calloop::EventLoop;
    use std::time::Duration;

    #[test]
    fn runs_a_deferred_closure_on_dispatch() {
        let mut event_loop: EventLoop<'static, Vec<u32>> =
            EventLoop::try_new().expect("event loop");
        let mut ran: Vec<u32> = Vec::new();

        let deferred = DeferredCall::new(&event_loop.handle()).expect("register deferred call");
        deferred.call_later(|ran: &mut Vec<u32>| ran.push(1));

        assert!(ran.is_empty(), "should not run before the loop dispatches");
        event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut ran)
            .expect("dispatch");
        assert_eq!(ran, vec![1]);
    }

    #[test]
    fn preserves_fifo_order_across_multiple_calls() {
        let mut event_loop: EventLoop<'static, Vec<u32>> =
            EventLoop::try_new().expect("event loop");
        let mut ran: Vec<u32> = Vec::new();

        let deferred = DeferredCall::new(&event_loop.handle()).expect("register deferred call");
        deferred.call_later(|ran: &mut Vec<u32>| ran.push(1));
        deferred.call_later(|ran: &mut Vec<u32>| ran.push(2));
        deferred.call_later(|ran: &mut Vec<u32>| ran.push(3));

        event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut ran)
            .expect("dispatch");
        assert_eq!(ran, vec![1, 2, 3]);
    }

    #[test]
    fn a_cloned_sender_can_defer_work_from_another_thread() {
        let mut event_loop: EventLoop<'static, Vec<&'static str>> =
            EventLoop::try_new().expect("event loop");
        let mut ran: Vec<&'static str> = Vec::new();

        let deferred = DeferredCall::new(&event_loop.handle()).expect("register deferred call");
        let sender = deferred.sender();

        let worker = std::thread::spawn(move || {
            let _ = sender.send(Box::new(|ran: &mut Vec<&'static str>| {
                ran.push("from-worker-thread")
            }));
        });
        worker.join().expect("worker thread panicked");

        event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut ran)
            .expect("dispatch");
        assert_eq!(ran, vec!["from-worker-thread"]);
    }
}
