//! Unix-socket IPC server: handler registry, socket lifecycle, and command dispatch.
//! Port of `src/ipc/ipc_service.{cpp,h}`.
//!
//! Divergences from the C++ (recorded per CLAUDE.md rule 6):
//! - `resolveSocketPath` is defined once here (`resolve_socket_path`, `pub(crate)`) and reused
//!   by `client::send`, instead of being duplicated verbatim in both `ipc_service.cpp` and
//!   `ipc_client.cpp` as in the C++ — a source-organization difference only, not a behavioral one.
//! - Socket setup goes through `std::os::unix::net::UnixListener::bind` (one call covering
//!   `socket()`/`bind()`/`listen()`) instead of the C++'s three explicit syscalls, so a single
//!   generic "bind() failed" warning replaces the C++'s three distinct per-step messages, and the
//!   explicit "socket path too long" pre-check is folded into whatever `bind()` reports for an
//!   over-length path. Diagnostic text only, not tested, not behavior.
//! - `HandlerInfo` owns `String`s instead of borrowing `string_view`s into the registry (which
//!   the C++ comment calls out as "invalidated by the next registerHandler() call"). Rust's
//!   borrow checker would force an artificial lifetime on this purely-diagnostic listing API for
//!   no behavioral gain, so it just clones; call frequency (help text, action editor listing) is
//!   far from hot-path.
//! - The socket read path treats the incoming byte stream as UTF-8 text
//!   (`String::from_utf8_lossy`) rather than the C++'s raw `std::string` byte buffer. Every real
//!   caller composes commands from CLI argv or `serde_json`/`nlohmann::json::dump()` output (both
//!   always valid UTF-8) — verified by reading every `IpcClient::send`/`runCli` call site — so
//!   this is unreachable in practice, unlike the byte-fidelity-load-bearing `/proc/<pid>/cmdline`
//!   case in `noctalia_core::process::matching`.

use std::cell::{Ref, RefCell};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use noctalia_core::log::Logger;

use crate::invocation_context::IpcInvocationContext;

const LOG: Logger = Logger::new("ipc");
const MAX_COMMAND_BYTES: usize = 64 * 1024;
const RECV_TIMEOUT: Duration = Duration::from_millis(100);
const CALLER_CWD_SEPARATOR: char = '\u{1e}';

/// A registered command handler. Receives everything after the first space as `args` and must
/// return a string ending with `\n`.
pub type Handler = Box<dyn Fn(&str) -> String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpVisibility {
    #[default]
    Public,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionEditorVisibility {
    #[default]
    Shown,
    Hidden,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HandlerOptions {
    pub help_visibility: HelpVisibility,
    pub action_editor_visibility: ActionEditorVisibility,
}

struct HandlerEntry {
    handler: Handler,
    args_spec: String,
    description: String,
    help_visibility: HelpVisibility,
    action_editor_visibility: ActionEditorVisibility,
    cycles: bool,
}

/// A registered command, for callers that need to present or validate the command set (e.g. the
/// action editor). Independent, owned snapshot — see the module divergence note on why this
/// clones rather than borrowing like the C++'s `string_view`-based `HandlerInfo`.
#[derive(Debug, Clone)]
pub struct HandlerInfo {
    pub command: String,
    /// Argument spec without the verb, e.g. "<id> [context]". Empty when the command takes none.
    pub args: String,
    pub description: String,
    pub help_visibility: HelpVisibility,
    pub action_editor_visibility: ActionEditorVisibility,
    /// Whether the command moves one position along an ordered set (workspaces, tracks, power
    /// profiles). Bound to a scroll gesture, one of those runs once per flick: the several
    /// notches an eager wheel movement emits are one intent, not a request to skip that many
    /// entries.
    pub cycles: bool,
}

impl HandlerInfo {
    /// "panel-toggle <id> [context]" — the form shown in --help.
    #[must_use]
    pub fn signature(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args)
        }
    }
}

/// Sets the invocation context for its lifetime, restoring the previous value on drop so that a
/// handler which re-enters `execute()` nests correctly.
pub struct InvocationScope<'a> {
    ipc: &'a IpcService,
    previous: Option<IpcInvocationContext>,
}

impl<'a> InvocationScope<'a> {
    pub fn new(ipc: &'a IpcService, context: Option<IpcInvocationContext>) -> Self {
        let previous = ipc.invocation_context.replace(context);
        Self { ipc, previous }
    }
}

impl Drop for InvocationScope<'_> {
    fn drop(&mut self) {
        *self.ipc.invocation_context.borrow_mut() = self.previous.take();
    }
}

pub struct IpcService {
    listener: Option<UnixListener>,
    socket_path: String,
    caller_cwd: RefCell<Option<String>>,
    invocation_context: RefCell<Option<IpcInvocationContext>>,
    // Registration order is retained; --help output is sorted for display.
    handlers: Vec<(String, HandlerEntry)>,
}

impl Default for IpcService {
    fn default() -> Self {
        Self::new()
    }
}

impl IpcService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            listener: None,
            socket_path: String::new(),
            caller_cwd: RefCell::new(None),
            invocation_context: RefCell::new(None),
            handlers: Vec::new(),
        }
    }

    /// Creates and binds the Unix socket. Returns false if it fails (IPC disabled).
    pub fn start(&mut self) -> bool {
        self.socket_path = resolve_socket_path();
        if self.socket_path.is_empty() {
            LOG.warn(format_args!(
                "IPC disabled: could not determine socket path"
            ));
            return false;
        }

        // Remove stale socket file before binding.
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = match UnixListener::bind(&self.socket_path) {
            Ok(listener) => listener,
            Err(err) => {
                LOG.warn(format_args!("IPC disabled: bind() failed: {err}"));
                return false;
            }
        };
        if let Err(err) = listener.set_nonblocking(true) {
            LOG.warn(format_args!(
                "IPC disabled: failed to set socket non-blocking: {err}"
            ));
            return false;
        }

        self.listener = Some(listener);
        true
    }

    /// Returns the listening fd, or -1 if not started.
    #[must_use]
    pub fn listen_fd(&self) -> RawFd {
        self.listener.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    /// Returns the socket path used.
    #[must_use]
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }

    /// Called when POLLIN fires on the listening fd (see `poll_source`).
    pub fn dispatch(&self) {
        let Some(listener) = &self.listener else {
            return;
        };
        while let Ok((stream, _addr)) = listener.accept() {
            self.handle_connection(stream);
        }
    }

    /// Execute a command line using the same handler registry as socket IPC.
    #[must_use]
    pub fn execute(&self, line: &str) -> String {
        *self.caller_cwd.borrow_mut() = None;

        let mut command_line = line.to_string();
        if let Some(cwd) = parse_caller_cwd_prefix(&mut command_line) {
            *self.caller_cwd.borrow_mut() = Some(cwd);
        }

        let (command, args) = match command_line.find(' ') {
            Some(pos) => (
                command_line[..pos].to_string(),
                command_line[pos + 1..].to_string(),
            ),
            None => (command_line.clone(), String::new()),
        };

        if command == "--help" || command == "-h" {
            return self.build_help();
        }

        self.execute_parsed(&command, &args)
    }

    /// When set, relative paths in IPC handlers should resolve against this directory (the
    /// caller's cwd) instead of the daemon's cwd. Only populated during `execute()`.
    #[must_use]
    pub fn caller_cwd(&self) -> Ref<'_, Option<String>> {
        self.caller_cwd.borrow()
    }

    /// Where the running command was invoked from, when it did not arrive over the socket.
    /// Empty for socket-originated commands.
    #[must_use]
    pub fn invocation_context(&self) -> Ref<'_, Option<IpcInvocationContext>> {
        self.invocation_context.borrow()
    }

    /// Registered commands, sorted by name.
    #[must_use]
    pub fn handlers(&self) -> Vec<HandlerInfo> {
        let mut infos: Vec<HandlerInfo> = self
            .handlers
            .iter()
            .map(|(command, entry)| HandlerInfo {
                command: command.clone(),
                args: entry.args_spec.clone(),
                description: entry.description.clone(),
                help_visibility: entry.help_visibility,
                action_editor_visibility: entry.action_editor_visibility,
                cycles: entry.cycles,
            })
            .collect();
        infos.sort_by(|a, b| a.command.cmp(&b.command));
        infos
    }

    #[must_use]
    pub fn has_handler(&self, command: &str) -> bool {
        self.handlers.iter().any(|(name, _)| name == command)
    }

    /// Register a handler for a command name. The handler receives everything after the first
    /// space as `args`. Must return a string ending with '\n'.
    /// `args_spec` describes the arguments only, without repeating the verb, e.g. "<id>
    /// [context]". `description` is a short human-readable explanation shown in --help.
    pub fn register_handler(
        &mut self,
        command: impl Into<String>,
        handler: Handler,
        args_spec: impl Into<String>,
        description: impl Into<String>,
        options: HandlerOptions,
    ) {
        let command = command.into();
        // Remove existing entry for this command if re-registering.
        self.handlers.retain(|(name, _)| name != &command);
        self.handlers.push((
            command,
            HandlerEntry {
                handler,
                args_spec: args_spec.into(),
                description: description.into(),
                help_visibility: options.help_visibility,
                action_editor_visibility: options.action_editor_visibility,
                cycles: false,
            },
        ));
    }

    /// A command that steps one position along an ordered set (workspaces, tracks, power
    /// profiles). Registered like any other; bound to a scroll gesture it runs once per flick
    /// instead of once per notch, so an eager wheel movement moves one position rather than
    /// several.
    pub fn register_cycle_handler(
        &mut self,
        command: impl Into<String>,
        handler: Handler,
        args_spec: impl Into<String>,
        description: impl Into<String>,
        options: HandlerOptions,
    ) {
        let command = command.into();
        self.register_handler(command.clone(), handler, args_spec, description, options);
        if let Some((_, entry)) = self.handlers.iter_mut().find(|(name, _)| *name == command) {
            entry.cycles = true;
        }
    }

    /// True when `command` was registered with `register_cycle_handler()`.
    #[must_use]
    pub fn handler_cycles(&self, command: &str) -> bool {
        self.handlers
            .iter()
            .find(|(name, _)| name == command)
            .is_some_and(|(_, entry)| entry.cycles)
    }

    fn handle_connection(&self, mut stream: UnixStream) {
        // Set receive timeout so a slow client doesn't stall the main loop.
        let _ = stream.set_read_timeout(Some(RECV_TIMEOUT));

        // Read until the client closes its write side. Newlines are valid command payload, so
        // they cannot be used as the frame delimiter.
        let mut command: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let mut reached_eof = false;
        while command.len() < MAX_COMMAND_BYTES {
            let limit = buf.len().min(MAX_COMMAND_BYTES - command.len());
            match stream.read(&mut buf[..limit]) {
                Ok(0) => {
                    reached_eof = true;
                    break;
                }
                Ok(n) => command.extend_from_slice(&buf[..n]),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    LOG.warn(format_args!("IPC read failed: {err}"));
                    break;
                }
            }
        }

        if command.len() >= MAX_COMMAND_BYTES && !reached_eof {
            let _ = stream.write_all(b"error: IPC command too large\n");
            return;
        }

        // Legacy external clients may still send one newline-terminated command and keep the
        // socket open while waiting for the response.
        if !reached_eof {
            if let Some(pos) = command.iter().position(|&b| b == b'\n') {
                command.truncate(pos);
            }
            if command.last() == Some(&b'\r') {
                command.pop();
            }
        }

        if command.is_empty() {
            // Client closed the connection without sending anything (e.g. a liveness probe).
            return;
        }

        let command = String::from_utf8_lossy(&command).into_owned();

        // A socket command has no in-process origin, even when the dispatch re-enters from a
        // handler that pumped the event loop.
        let _socket_scope = InvocationScope::new(self, None);
        let response = self.execute(&command);
        let _ = stream.write_all(response.as_bytes());
    }

    fn build_help(&self) -> String {
        let mut infos = self.handlers();
        infos.retain(|info| info.help_visibility != HelpVisibility::Hidden);

        let signatures: Vec<String> = infos.iter().map(HandlerInfo::signature).collect();
        let max_signature = signatures.iter().map(String::len).max().unwrap_or(0);

        let mut out = String::from("Usage: noctalia msg <command> [args]\n\nCommands:\n");
        for (info, signature) in infos.iter().zip(signatures.iter()) {
            out.push_str("  ");
            out.push_str(signature);
            if !info.description.is_empty() {
                out.push_str(&" ".repeat(max_signature - signature.len() + 2));
                out.push_str(&info.description);
            }
            out.push('\n');
        }
        out
    }

    fn execute_parsed(&self, command: &str, args: &str) -> String {
        match self.handlers.iter().find(|(name, _)| name == command) {
            Some((_, entry)) => (entry.handler)(args),
            None => "error: unknown command (try: noctalia msg --help)\n".to_string(),
        }
    }
}

impl Drop for IpcService {
    fn drop(&mut self) {
        // The listener's own Drop closes the fd; we still need to remove the socket file, which
        // UnixListener does not do on its own.
        if !self.socket_path.is_empty() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

fn parse_caller_cwd_prefix(line: &mut String) -> Option<String> {
    let separator = line.find(CALLER_CWD_SEPARATOR)?;

    let cwd = line[..separator].to_string();
    line.replace_range(..=separator, "");
    if cwd.is_empty() {
        return None;
    }

    let cwd_path = Path::new(&cwd);
    if !cwd_path.is_absolute() || !cwd_path.is_dir() {
        return None;
    }
    Some(cwd)
}

pub(crate) fn resolve_socket_path() -> String {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/tmp".to_string());
    let display = std::env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "wayland-0".to_string());
    format!("{runtime}/noctalia-{display}.sock")
}
