//! Client-side IPC send, used by `noctalia msg`.
//! Port of `src/ipc/ipc_client.{cpp,h}`.
//!
//! Divergences (module-level):
//! - `UnixStream::connect` bundles the C++'s three separate `socket()`/path-length-check/
//!   `connect()` steps into one fallible call, so every pre-connect failure — including an
//!   over-length socket path, which can't happen here in practice since `resolve_socket_path()`
//!   only ever composes short, XDG-derived paths — surfaces as the same "noctalia is not
//!   running" message instead of the C++'s more specific per-step text. Diagnostic wording only;
//!   not asserted on by any test.
//! - `stream.write_all(...)` uses `io::Write`'s default trait method, which retries
//!   automatically on `ErrorKind::Interrupted` (EINTR). The C++ reference
//!   (`ipc_client.cpp:73-82`) has no such retry: any negative `write()` return, EINTR included,
//!   is a hard failure that prints `"error: write() failed: ..."` and exits 1. So under signal
//!   delivery mid-write (rare — a short local unix-socket write), this client silently finishes
//!   the send where the C++ client would report an error and exit. Accepted deliberately rather
//!   than hand-rolling a raw-`libc::write` loop to reproduce the C++'s EINTR-is-fatal behavior:
//!   the Rust behavior is strictly more correct (the bytes did go out), and bypassing `Write`'s
//!   standard retry semantics for a single call site to reproduce a likely-incidental C++
//!   omission isn't worth the added `unsafe`.
//! - `std::env::current_dir()`'s cwd is rendered via `to_string_lossy()` (replacing invalid
//!   bytes with U+FFFD) rather than the C++'s raw `std::filesystem::path::string()` byte-for-byte
//!   pass-through, and `resolve_socket_path()` (see `service.rs`) reads env vars via
//!   `std::env::var()`, which treats a non-UTF-8 value as unset (falling back to the default)
//!   instead of the C++'s raw `getenv`. Both require a non-UTF-8 cwd or `$XDG_RUNTIME_DIR`/
//!   `$WAYLAND_DISPLAY` to matter, which doesn't happen in practice on this codebase's target
//!   systems — same class of fidelity gap as the command-body `String::from_utf8_lossy` divergence
//!   recorded in `service.rs`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::service::resolve_socket_path;

const CALLER_CWD_SEPARATOR: char = '\u{1e}';
const TIMEOUT: Duration = Duration::from_secs(2);

/// Sends `command` to the running noctalia instance. Prints the response to stdout. Returns 0 on
/// success, 1 on error.
pub fn send(command: &str) -> i32 {
    let path = resolve_socket_path();

    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(_) => {
            eprintln!("error: noctalia is not running");
            return 1;
        }
    };

    // Set connect/send/recv timeout to 2 seconds.
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let _ = stream.set_read_timeout(Some(TIMEOUT));

    // Prefix the caller cwd so the daemon resolves relative paths correctly.
    let mut line = String::new();
    if let Ok(cwd) = std::env::current_dir()
        && cwd.is_absolute()
    {
        line.push_str(&cwd.to_string_lossy());
        line.push(CALLER_CWD_SEPARATOR);
    }
    line.push_str(command);

    if let Err(err) = stream.write_all(line.as_bytes()) {
        eprintln!("error: write() failed: {err}");
        return 1;
    }

    if let Err(err) = stream.shutdown(std::net::Shutdown::Write) {
        eprintln!("error: shutdown() failed: {err}");
        return 1;
    }

    // Read response until EOF (server closes connection after writing).
    let mut response = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => response.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }

    print!("{response}");
    let _ = std::io::stdout().flush();

    // Return 1 if the response indicates an error.
    i32::from(response.starts_with("error:"))
}
