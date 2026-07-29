//! Unix-socket IPC protocol and server. Plan Phase 4, task 4.1.
//!
//! Port of `src/ipc/*`: the daemon-side handler registry and socket server
//! (`service`, `poll_source`), the CLI-side client (`client`), the `noctalia msg` entry point
//! (`cli`), the in-process invocation-origin type (`invocation_context`), and the
//! `noctalia::ipc` argument-parsing helpers (`arg_parse`).

pub mod arg_parse;
pub mod cli;
pub mod client;
pub mod invocation_context;
pub mod poll_source;
pub mod service;

pub use invocation_context::IpcInvocationContext;
pub use service::{
    ActionEditorVisibility, Handler, HandlerInfo, HandlerOptions, HelpVisibility, InvocationScope,
    IpcService,
};
