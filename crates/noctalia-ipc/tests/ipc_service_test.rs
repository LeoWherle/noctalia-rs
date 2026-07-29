//! Port of `tests/ipc_service_test.cpp`.
//!
//! Single `#[test]`, matching the C++ test's single `main()`: this whole file is one test binary
//! process, so the `XDG_RUNTIME_DIR`/`WAYLAND_DISPLAY` env var mutations below have no concurrent
//! sibling test to race against (unlike `noctalia_core::process`'s tests, which share a binary
//! with dozens of others and need `ENV_MUTATION_LOCK`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use noctalia_ipc::service::{ActionEditorVisibility, HandlerOptions, HelpVisibility};
use noctalia_ipc::{InvocationScope, IpcInvocationContext, IpcService};

fn make_temp_dir() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("noctalia-ipc-service-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn send_raw(ipc: &IpcService, socket_path: &Path, command: &str) -> String {
    let mut stream = UnixStream::connect(socket_path).expect("connect");
    stream.write_all(command.as_bytes()).expect("write");
    stream.shutdown(Shutdown::Write).expect("shutdown write");

    ipc.dispatch();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    String::from_utf8(response).expect("valid utf8 response")
}

#[test]
fn ipc_service_matches_cpp_reference_behavior() {
    let runtime_dir = make_temp_dir();
    let wayland_display = "noctalia-ipc-service-test";
    // SAFETY: this is the only test in this binary (see module doc comment), so no other test
    // can observe or race these process env var writes.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
        std::env::set_var("WAYLAND_DISPLAY", wayland_display);
    }

    let mut ipc = IpcService::new();
    ipc.register_handler(
        "visible-command",
        Box::new(|args| format!("visible:{args}\n")),
        "<value>",
        "Visible command",
        HandlerOptions::default(),
    );
    ipc.register_handler(
        "hidden-command",
        Box::new(|args| format!("hidden:{args}\n")),
        "<value>",
        "Hidden command",
        HandlerOptions {
            help_visibility: HelpVisibility::Hidden,
            action_editor_visibility: ActionEditorVisibility::Hidden,
        },
    );

    assert_eq!(ipc.execute("visible-command ok"), "visible:ok\n");
    assert_eq!(ipc.execute("hidden-command ok"), "hidden:ok\n");
    assert_eq!(
        ipc.execute("visible-command line1\nline2\nline3"),
        "visible:line1\nline2\nline3\n"
    );

    // Metadata includes every handler, while has_handler() agrees with execute() dispatch.
    {
        let infos = ipc.handlers();
        assert_eq!(infos.len(), 2);
        let visible = infos
            .iter()
            .find(|i| i.command == "visible-command")
            .expect("visible-command present");
        // The registry stores arguments only; the verb is composed back in for display.
        assert_eq!(visible.args, "<value>");
        assert_eq!(visible.signature(), "visible-command <value>");
        assert_eq!(visible.help_visibility, HelpVisibility::Public);
        assert_eq!(
            visible.action_editor_visibility,
            ActionEditorVisibility::Shown
        );
        assert_eq!(visible.description, "Visible command");

        let hidden = infos
            .iter()
            .find(|i| i.command == "hidden-command")
            .expect("hidden-command present");
        assert_eq!(hidden.help_visibility, HelpVisibility::Hidden);
        assert_eq!(
            hidden.action_editor_visibility,
            ActionEditorVisibility::Hidden
        );
        assert!(ipc.has_handler("visible-command"));
        assert!(ipc.has_handler("hidden-command"));
        assert!(!ipc.has_handler("no-such-command"));
    }

    // Action-editor visibility does not affect execution or help output.
    {
        ipc.register_handler(
            "query-command",
            Box::new(|_| "state\n".to_string()),
            "",
            "Print some state",
            HandlerOptions {
                action_editor_visibility: ActionEditorVisibility::Hidden,
                ..Default::default()
            },
        );
        assert_eq!(ipc.execute("query-command"), "state\n");
        assert!(ipc.has_handler("query-command"));

        let infos = ipc.handlers();
        let query = infos
            .iter()
            .find(|i| i.command == "query-command")
            .expect("query-command present");
        assert_eq!(
            query.action_editor_visibility,
            ActionEditorVisibility::Hidden
        );
        assert!(ipc.execute("--help").contains("query-command"));

        let visible = infos
            .iter()
            .find(|i| i.command == "visible-command")
            .expect("visible-command present");
        assert_eq!(
            visible.action_editor_visibility,
            ActionEditorVisibility::Shown
        );
    }

    // A cycling command runs like any other, but declares that a scroll flick should move one
    // position rather than one per notch.
    {
        ipc.register_cycle_handler(
            "cycle-command",
            Box::new(|_| "moved\n".to_string()),
            "<next|prev>",
            "Step",
            HandlerOptions::default(),
        );
        assert_eq!(ipc.execute("cycle-command next"), "moved\n");
        assert!(ipc.handler_cycles("cycle-command"));
        assert!(!ipc.handler_cycles("visible-command"));
        assert!(!ipc.handler_cycles("no-such-command"));

        let infos = ipc.handlers();
        let cycle = infos
            .iter()
            .find(|i| i.command == "cycle-command")
            .expect("cycle-command present");
        assert!(cycle.cycles);
        // Cycling says nothing about whether an action picker should offer it.
        assert_eq!(
            cycle.action_editor_visibility,
            ActionEditorVisibility::Shown
        );
    }

    // `exec` and `none` are reserved by the bar widget action grammar and must never become IPC
    // commands, or a binding would resolve to two different things.
    assert!(!ipc.has_handler("exec"));
    assert!(!ipc.has_handler("none"));

    // The invocation context is empty unless a scope is active, and scopes nest.
    {
        assert!(ipc.invocation_context().is_none());
        let outer = InvocationScope::new(
            &ipc,
            Some(IpcInvocationContext {
                widget_name: "media".to_string(),
                bar_name: "default".to_string(),
                ..Default::default()
            }),
        );
        assert!(ipc.invocation_context().is_some());
        assert_eq!(
            ipc.invocation_context().as_ref().unwrap().widget_name,
            "media"
        );
        {
            let inner = InvocationScope::new(
                &ipc,
                Some(IpcInvocationContext {
                    widget_name: "clock".to_string(),
                    ..Default::default()
                }),
            );
            assert_eq!(
                ipc.invocation_context().as_ref().unwrap().widget_name,
                "clock"
            );
            let cleared = InvocationScope::new(&ipc, None);
            assert!(ipc.invocation_context().is_none());
            drop(cleared);
            drop(inner);
        }
        assert_eq!(
            ipc.invocation_context().as_ref().unwrap().widget_name,
            "media"
        );
        assert_eq!(
            ipc.invocation_context().as_ref().unwrap().bar_name,
            "default"
        );
        drop(outer);
    }
    assert!(ipc.invocation_context().is_none());

    let help = ipc.execute("--help");
    assert!(help.contains("visible-command <value>"));
    assert!(help.contains("Visible command"));
    assert!(!help.contains("hidden-command"));
    assert!(!help.contains("Hidden command"));

    ipc.register_handler(
        "visible-command",
        Box::new(|_| "hidden-now\n".to_string()),
        "",
        "Now hidden",
        HandlerOptions {
            help_visibility: HelpVisibility::Hidden,
            ..Default::default()
        },
    );

    assert_eq!(ipc.execute("visible-command"), "hidden-now\n");
    let updated_help = ipc.execute("--help");
    assert!(!updated_help.contains("visible-command"));
    assert!(!updated_help.contains("Now hidden"));

    assert!(ipc.start());
    let socket_path = runtime_dir.join(format!("noctalia-{wayland_display}.sock"));
    assert_eq!(
        send_raw(&ipc, &socket_path, "hidden-command line1\nline2\nline3"),
        "hidden:line1\nline2\nline3\n"
    );

    let _ = std::fs::remove_dir_all(&runtime_dir);
}
