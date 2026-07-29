//! Origin of an in-process IPC invocation.
//! Port of `src/ipc/ipc_invocation_context.h`.

use std::ffi::c_void;

/// Origin of an IPC command that was invoked in-process rather than over the socket. Handlers
/// that need to know where a command came from, such as opening the settings window at the
/// invoking widget, read it through `IpcService::invocation_context()`, which is only populated
/// for the duration of an `InvocationScope`.
///
/// Bar widget gesture actions are the only producer today. Panel actions do not travel this way:
/// they re-enter through the bar's panel callback so they anchor at the widget.
///
/// `output` stands in for the C++'s `wl_output*` (a forward-declared, never-dereferenced-here
/// opaque pointer in the C++ too): Wayland types have no Rust binding yet (that lands with the
/// Phase 9 wayland crate), so this crate carries the pointer bits without ever reading through
/// them, exactly as the C++ header does with an incomplete type.
#[derive(Debug, Default, Clone)]
pub struct IpcInvocationContext {
    /// `[widget.<name>]` instance id.
    pub widget_name: String,
    pub widget_type: String,
    pub bar_name: String,
    pub output: Option<*mut c_void>,
}
