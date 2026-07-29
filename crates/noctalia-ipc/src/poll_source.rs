//! Calloop wiring for `IpcService`'s listening socket.
//! Port of `src/ipc/ipc_poll_source.h`.

use std::os::fd::{BorrowedFd, RawFd};

/// Registers `listen_fd` (an `IpcService::listen_fd()` obtained after a successful `start()`)
/// for readability on `handle`. `on_readable` should call `IpcService::dispatch()`.
///
/// The C++'s `IpcPollSource` re-checks `listenFd() >= 0` on every dispatch and when adding fds,
/// since it can be constructed before `start()`. This registration function is only ever called
/// once `start()` has already returned true (mirroring `register_config_poll_source`'s
/// already-open-fd contract), so that guard has no equivalent here — there is nothing to
/// register otherwise.
pub fn register_ipc_poll_source<Data: 'static>(
    handle: &calloop::LoopHandle<'_, Data>,
    listen_fd: RawFd,
    mut on_readable: impl FnMut(&mut Data) + 'static,
) -> Result<
    calloop::RegistrationToken,
    calloop::InsertError<calloop::generic::Generic<BorrowedFd<'static>>>,
> {
    // SAFETY: listen_fd is an open, non-blocking listening socket fd owned by an `IpcService`
    // that the caller keeps alive for at least as long as this registration.
    let fd = unsafe { BorrowedFd::borrow_raw(listen_fd) };
    let source = calloop::generic::Generic::new(fd, calloop::Interest::READ, calloop::Mode::Level);
    handle.insert_source(source, move |_, _, data| {
        on_readable(data);
        Ok(calloop::PostAction::Continue)
    })
}
