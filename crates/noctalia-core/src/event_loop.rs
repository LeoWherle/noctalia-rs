//! `core::event_loop` — task 1.5. No single C++ source file corresponds to this;
//! the C++ codebase drives everything from its own hand-rolled poll-based
//! `MainLoop`. This is where architecture decision 1 (calloop main thread + a
//! tokio runtime sidecar, bridged with `calloop::channel`) actually gets built:
//! a thin wrapper around a [`calloop::EventLoop`] plus a dedicated OS thread
//! running a single-threaded tokio runtime.
//!
//! The tokio side is a *single* current-thread runtime driven by one dedicated
//! thread (not a multi-worker pool) — a desktop shell has no need to spin up
//! several extra OS threads by default, and `tokio::runtime::Handle::spawn` can
//! still be called from any thread (including the main calloop thread) to queue
//! work onto it; the sidecar thread just needs to stay parked in
//! `Runtime::block_on` to keep polling it.
//!
//! The bridge back from tokio to the main loop reuses [`crate::timing::DeferredCall`]
//! — that type already *is* "any thread can post a closure that runs on the main
//! loop next dispatch," which is exactly what a tokio task completing needs.

use std::future::Future;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use calloop::LoopHandle;
use tokio::runtime::{Builder, Handle as TokioHandle};
use tokio::sync::oneshot;

use crate::timing::DeferredCall;

#[derive(Debug, thiserror::Error)]
pub enum EventLoopError {
    #[error("failed to create the calloop event loop: {0}")]
    Calloop(#[source] calloop::Error),
    #[error("failed to spawn the tokio sidecar thread: {0}")]
    SidecarThread(#[source] std::io::Error),
    #[error("failed to build the tokio sidecar runtime: {0}")]
    SidecarRuntime(#[source] std::io::Error),
}

struct TokioSidecar {
    handle: TokioHandle,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl TokioSidecar {
    fn spawn() -> Result<Self, EventLoopError> {
        let (handle_tx, handle_rx) = std_mpsc::channel::<Result<TokioHandle, std::io::Error>>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let join = std::thread::Builder::new()
            .name("noctalia-tokio-sidecar".to_string())
            .spawn(move || {
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = handle_tx.send(Err(err));
                        return;
                    }
                };
                let _ = handle_tx.send(Ok(runtime.handle().clone()));
                runtime.block_on(async move {
                    let _ = shutdown_rx.await;
                });
            })
            .map_err(EventLoopError::SidecarThread)?;

        let build_result = handle_rx.recv().map_err(|_| {
            EventLoopError::SidecarThread(std::io::Error::other(
                "sidecar thread exited before starting",
            ))
        })?;
        let handle = build_result.map_err(EventLoopError::SidecarRuntime)?;

        Ok(Self {
            handle,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        })
    }

    fn handle(&self) -> &TokioHandle {
        &self.handle
    }
}

impl Drop for TokioSidecar {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Owns the main-thread calloop loop plus its tokio sidecar.
pub struct EventLoop<Data: 'static> {
    calloop: calloop::EventLoop<'static, Data>,
    bridge: DeferredCall<Data>,
    tokio: TokioSidecar,
}

impl<Data: 'static> EventLoop<Data> {
    pub fn try_new() -> Result<Self, EventLoopError> {
        let calloop = calloop::EventLoop::try_new().map_err(EventLoopError::Calloop)?;
        let bridge = DeferredCall::new(&calloop.handle()).map_err(EventLoopError::Calloop)?;
        let tokio = TokioSidecar::spawn()?;
        Ok(Self {
            calloop,
            bridge,
            tokio,
        })
    }

    pub fn handle(&self) -> LoopHandle<'static, Data> {
        self.calloop.handle()
    }

    /// The returned handle is not lifetime-tied to `&self`: if a caller retains
    /// it past this `EventLoop`'s drop, `TokioSidecar::drop` has already sent
    /// the shutdown signal and joined the sidecar thread, so `.spawn()` on the
    /// stale handle still succeeds syntactically but no thread is left to poll
    /// the injected task — the returned `JoinHandle` hangs forever. Prefer
    /// `spawn_on_tokio` below, which is scoped to the sidecar's lifetime.
    pub fn tokio_handle(&self) -> TokioHandle {
        self.tokio.handle().clone()
    }

    /// Runs `future` on the tokio sidecar; once it resolves, `on_complete` runs
    /// on the main loop thread (the next time it dispatches) with the future's
    /// output and a `&mut Data`. This is the "tokio task -> calloop callback"
    /// bridge.
    pub fn spawn_on_tokio<F, T>(
        &self,
        future: F,
        on_complete: impl FnOnce(T, &mut Data) + Send + 'static,
    ) where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let sender = self.bridge.sender();
        self.tokio.handle().spawn(async move {
            let result = future.await;
            let _ = sender.send(Box::new(move |data: &mut Data| on_complete(result, data)));
        });
    }

    /// Schedules `f` to run on the main loop thread the next time it dispatches
    /// — safe to call from any thread, including from within a tokio task.
    pub fn defer(&self, f: impl FnOnce(&mut Data) + Send + 'static) {
        self.bridge.call_later(f);
    }

    pub fn dispatch(&mut self, timeout: Option<Duration>, data: &mut Data) -> calloop::Result<()> {
        self.calloop.dispatch(timeout, data)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn round_trips_a_message_through_the_tokio_sidecar_and_back() {
        let mut event_loop: EventLoop<Vec<u32>> = EventLoop::try_new().expect("event loop");
        let mut state: Vec<u32> = Vec::new();

        event_loop.spawn_on_tokio(
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                42u32
            },
            |result, state: &mut Vec<u32>| state.push(result),
        );

        // Give the sidecar time to run the future and send its result back,
        // then dispatch until the main loop actually picks it up.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while state.is_empty() && std::time::Instant::now() < deadline {
            event_loop
                .dispatch(Some(Duration::from_millis(20)), &mut state)
                .expect("dispatch");
        }

        assert_eq!(
            state,
            vec![42],
            "tokio task result should have reached the main-loop callback"
        );
    }

    #[test]
    fn main_thread_can_defer_work_onto_itself() {
        let mut event_loop: EventLoop<Vec<&'static str>> =
            EventLoop::try_new().expect("event loop");
        let mut state: Vec<&'static str> = Vec::new();

        event_loop.defer(|state: &mut Vec<&'static str>| state.push("deferred"));
        event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut state)
            .expect("dispatch");

        assert_eq!(state, vec!["deferred"]);
    }

    #[test]
    fn tokio_handle_can_be_used_to_spawn_from_outside_spawn_on_tokio() {
        let mut event_loop: EventLoop<Vec<u32>> = EventLoop::try_new().expect("event loop");
        let mut state: Vec<u32> = Vec::new();

        let (done_tx, done_rx) = mpsc::channel::<u32>();
        event_loop.tokio_handle().spawn(async move {
            let _ = done_tx.send(7);
        });

        let value = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("tokio task should have run");
        assert_eq!(value, 7);

        // Nothing to dispatch here — this path never touches the calloop
        // bridge — but exercise dispatch once anyway to prove it stays healthy.
        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut state)
            .expect("dispatch");
    }
}
