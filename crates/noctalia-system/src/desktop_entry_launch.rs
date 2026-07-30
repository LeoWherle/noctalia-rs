//! Port of `src/system/desktop_entry_launch.{cpp,h}` (task 5.5.3): desktop-file `Exec=` value
//! decoding/field-code stripping, argv tokenization, and the `launchEntry`/`launchAction` entry
//! points that hand off to [`noctalia_core::process::launch_detached`]/
//! [`noctalia_core::process::run_async_as_systemd_service`] (or, for terminal launches,
//! [`crate::terminal_launch`]).
//!
//! `tokenize` here is a private near-duplicate of [`crate::terminal_launch`]'s own `tokenize` —
//! not shared, because the C++ originals aren't either (independent anonymous-namespace
//! functions in independent translation units): this one additionally understands
//! backslash-escaping inside double quotes (`\"`, `` \` ``, `\$`, `\\`), which
//! `terminal_launch::tokenize` does not.

use std::sync::Once;

use noctalia_core::files::paths::expand_user_path;
use noctalia_core::log::Logger;
use noctalia_core::process;

use crate::desktop_entry::{DesktopAction, DesktopEntry};
use crate::terminal_launch;

const LOG: Logger = Logger::new("desktop_entry_launch");

/// Port of `desktop_entry_launch::LaunchOptions`.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub activation_token: String,
    pub run_as_systemd_service: bool,
    pub custom_command: String,
}

/// Port of `desktop_entry_launch::PrepareOptions`.
#[derive(Debug, Clone)]
pub struct PrepareOptions {
    pub terminal_candidates: Vec<String>,
    pub use_system_terminal_discovery: bool,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            terminal_candidates: Vec::new(),
            use_system_terminal_discovery: true,
        }
    }
}

/// Port of `desktop_entry_launch::PreparedCommand`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    pub args: Vec<String>,
}

/// Port of the anonymous namespace's `decodeDesktopStringValue`: desktop-entry value-escaping
/// (`\\`, `\s`, `\n`, `\t`, `\r`; any other `\X` drops the backslash and keeps `X`).
fn decode_desktop_string_value(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            match next {
                '\\' => result.push('\\'),
                's' => result.push(' '),
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                other => {
                    result.push(other);
                    i += 2;
                    continue;
                }
            }
            i += 2;
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

const FIELD_CODES: [char; 11] = ['f', 'F', 'u', 'U', 'd', 'D', 'n', 'N', 'i', 'c', 'k'];

/// Port of the anonymous namespace's `stripFieldCodes`: removes `%f`/`%F`/`%u`/`%U`/`%d`/`%D`/
/// `%n`/`%N`/`%i`/`%c`/`%k` field codes (and one following space, if any), unescapes `%%` to a
/// literal `%`, trims trailing spaces, then strips orphaned Flatpak `@@u ... @@` file-forwarding
/// markers whose interior is whitespace-only.
fn strip_field_codes(exec: &str) -> String {
    let chars: Vec<char> = exec.chars().collect();
    let mut result: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if FIELD_CODES.contains(&next) {
                i += 2;
                if i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                continue;
            }
            if next == '%' {
                result.push('%');
                i += 2;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    while result.last() == Some(&' ') {
        result.pop();
    }

    let mut result: String = result.into_iter().collect();
    let mut search_from = 0usize;
    while let Some(pos) = result[search_from..].find("@@u").map(|p| p + search_from) {
        let Some(end) = result[pos + 3..].find("@@").map(|p| p + pos + 3) else {
            break;
        };
        let only_whitespace = result[pos + 3..end].bytes().all(|b| b == b' ');
        if only_whitespace {
            result.replace_range(pos..end + 2, "");
            search_from = pos;
        } else {
            search_from = end + 2;
        }
    }

    result
}

/// Port of the anonymous namespace's `tokenize` — see the module doc comment for how this
/// differs from [`terminal_launch`]'s own, separate `tokenize`.
fn tokenize(cmd: &str) -> Vec<String> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && in_double && i + 1 < chars.len() {
            let next = chars[i + 1];
            if matches!(next, '"' | '`' | '$' | '\\') {
                current.push(next);
                i += 2;
                continue;
            }
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if c == ' ' && !in_single && !in_double {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
            i += 1;
            continue;
        }
        current.push(c);
        i += 1;
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn expand_executable_path(binary: &str) -> String {
    if binary.is_empty() || !binary.starts_with('~') {
        return binary.to_string();
    }
    expand_user_path(binary).to_string_lossy().into_owned()
}

fn app_name_or_default(app_name: &str) -> String {
    if app_name.is_empty() {
        "desktop-entry".to_string()
    } else {
        app_name.to_string()
    }
}

/// Port of the anonymous namespace's `parseCustomCommand`. `str::replace` matches the C++'s
/// manual `find`/`replace` loop here: both scan the original string left-to-right for the
/// placeholder and never rescan text just inserted by a previous replacement (the C++ advances
/// `pos` past the inserted `exec` text after each match; `str::replace` never revisits replaced
/// spans either), so a `$CMD` occurring inside `exec` itself doesn't get replaced again.
fn parse_custom_command(exec: &str, custom_command: &str) -> String {
    if custom_command.is_empty() {
        return exec.to_string();
    }
    const PLACEHOLDER: &str = "$CMD";
    if !custom_command.contains(PLACEHOLDER) {
        LOG.warn(format_args!(
            "Custom command does not contain '$CMD': '{custom_command}'"
        ));
    }
    custom_command.replace(PLACEHOLDER, exec)
}

/// Port of `effectiveRunAsSystemdService`: resolves whether this launch really goes through
/// systemd, warning once (per process lifetime, matching the C++'s function-local
/// `static std::once_flag`) when the option is set but the shell is not itself a systemd user
/// unit.
fn effective_run_as_systemd_service(options: &LaunchOptions) -> bool {
    if !options.run_as_systemd_service {
        return false;
    }
    if process::running_under_systemd_user_manager() {
        return true;
    }
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        LOG.warn(format_args!(
            "launch_apps_as_systemd_services is enabled but Noctalia is not running under the systemd user \
             manager (no uwsm or systemd user service); launching apps directly"
        ));
    });
    false
}

fn warn_if_custom_command_conflicts_with_systemd(
    run_as_systemd_service: bool,
    options: &LaunchOptions,
) {
    if run_as_systemd_service && !options.custom_command.is_empty() {
        LOG.warn(format_args!(
            "launch_apps_as_systemd_services and launch_apps_custom_command are mutually exclusive; ignoring \
             custom command"
        ));
    }
}

/// Port of `desktop_entry_launch::prepareCommand`.
pub fn prepare_command(
    exec: &str,
    terminal: bool,
    options: &PrepareOptions,
) -> Option<PreparedCommand> {
    let decoded_exec = decode_desktop_string_value(exec);
    let clean_exec = strip_field_codes(&decoded_exec);
    let mut args = if terminal {
        terminal_launch::prepare_command(
            &clean_exec,
            &terminal_launch::Options {
                terminal_candidates: options.terminal_candidates.clone(),
                use_system_terminal_discovery: options.use_system_terminal_discovery,
            },
        )?
    } else {
        tokenize(&clean_exec)
    };

    if let Some(first) = args.first()
        && first.contains('/')
    {
        args[0] = expand_executable_path(&args[0]);
    }

    if args.is_empty() {
        return None;
    }
    Some(PreparedCommand { args })
}

/// Port of `desktop_entry_launch::launchEntry`.
pub fn launch_entry(entry: &DesktopEntry, options: &LaunchOptions) -> bool {
    let run_as_systemd_service = effective_run_as_systemd_service(options);
    warn_if_custom_command_conflicts_with_systemd(run_as_systemd_service, options);
    let custom_command = if run_as_systemd_service {
        ""
    } else {
        &options.custom_command
    };
    let command = parse_custom_command(&entry.exec, custom_command);
    let Some(prepared) = prepare_command(&command, entry.terminal, &PrepareOptions::default())
    else {
        LOG.warn(format_args!(
            "Failed to prepare launch command for desktop entry '{}'",
            if entry.id.is_empty() {
                &entry.name
            } else {
                &entry.id
            }
        ));
        return false;
    };

    let app_name = if !entry.id.is_empty() {
        entry.id.clone()
    } else {
        app_name_or_default(&entry.name)
    };
    if run_as_systemd_service {
        return process::run_async_as_systemd_service(
            &prepared.args,
            &app_name,
            &options.activation_token,
            &entry.working_dir,
        );
    }
    process::launch_detached(
        &prepared.args,
        &options.activation_token,
        &entry.working_dir,
    )
}

/// Port of `desktop_entry_launch::launchAction`.
pub fn launch_action(
    action: &DesktopAction,
    app_name: &str,
    working_dir: &str,
    terminal: bool,
    options: &LaunchOptions,
) -> bool {
    let run_as_systemd_service = effective_run_as_systemd_service(options);
    warn_if_custom_command_conflicts_with_systemd(run_as_systemd_service, options);
    let custom_command = if run_as_systemd_service {
        ""
    } else {
        &options.custom_command
    };
    let command = parse_custom_command(&action.exec, custom_command);
    let Some(prepared) = prepare_command(&command, terminal, &PrepareOptions::default()) else {
        LOG.warn(format_args!(
            "Failed to prepare launch command for desktop action '{}'",
            if action.id.is_empty() {
                &action.name
            } else {
                &action.id
            }
        ));
        return false;
    };

    if run_as_systemd_service {
        return process::run_async_as_systemd_service(
            &prepared.args,
            &app_name_or_default(app_name),
            &options.activation_token,
            working_dir,
        );
    }
    process::launch_detached(&prepared.args, &options.activation_token, working_dir)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn make_executable_fixture() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "noctalia-terminal-fixture-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("chmod fixture");
        path
    }

    fn expect_args(exec: &str, terminal: bool, options: &PrepareOptions, expected: &[&str]) {
        let prepared =
            prepare_command(exec, terminal, options).expect("expected a prepared command");
        assert_eq!(prepared.args, expected);
    }

    #[test]
    fn field_codes_should_be_removed() {
        expect_args(
            "sample --name %% --file %f --url %U --keep",
            false,
            &PrepareOptions::default(),
            &["sample", "--name", "%", "--file", "--url", "--keep"],
        );
    }

    #[test]
    fn quoted_arguments_should_stay_together() {
        expect_args(
            "sample --title \"Hello World\" --single 'Two Words'",
            false,
            &PrepareOptions::default(),
            &["sample", "--title", "Hello World", "--single", "Two Words"],
        );
    }

    #[test]
    fn desktop_entry_escaping_should_preserve_a_shell_variable_in_a_quoted_argument() {
        expect_args(
            r#"/bin/sh -c "\\$SHELL -i -c scrcpy""#,
            false,
            &PrepareOptions::default(),
            &["/bin/sh", "-c", "$SHELL -i -c scrcpy"],
        );
    }

    #[test]
    fn terminal_candidates_should_use_the_first_executable_candidate() {
        let fake_terminal = make_executable_fixture();
        let options = PrepareOptions {
            terminal_candidates: vec![
                "missing-terminal-candidate".to_string(),
                fake_terminal.to_string_lossy().into_owned(),
            ],
            use_system_terminal_discovery: true,
        };
        expect_args(
            "sample --flag",
            true,
            &options,
            &[
                &fake_terminal.to_string_lossy(),
                "-e",
                "sh",
                "-lc",
                "sample --flag",
            ],
        );
        let _ = std::fs::remove_file(&fake_terminal);
    }

    #[test]
    fn gnome_style_terminals_should_use_double_dash_before_the_shell_command() {
        // Mutates `PATH`, so this shares `terminal_launch`'s `ENV_LOCK` rather than declaring a
        // second, uncoordinated lock over the same process-global `PATH` (see that module's
        // `test_support` doc comment).
        let _guard = crate::terminal_launch::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let dir = std::env::temp_dir().join(format!(
            "noctalia_desktop_entry_launch_gnome_test_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let fake_gnome_terminal = dir.join("gnome-terminal");
        std::fs::write(&fake_gnome_terminal, "#!/bin/sh\nexit 0\n").expect("write fixture");
        std::fs::set_permissions(&fake_gnome_terminal, std::fs::Permissions::from_mode(0o700))
            .expect("chmod fixture");

        let old_path = std::env::var_os("PATH");
        // SAFETY: guarded by `ENV_LOCK` for the duration of this test.
        unsafe {
            std::env::set_var(
                "PATH",
                format!(
                    "{}:{}",
                    dir.display(),
                    old_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
            );
        }

        let options = PrepareOptions {
            terminal_candidates: vec!["gnome-terminal".to_string()],
            use_system_terminal_discovery: true,
        };
        let result = prepare_command("sample --flag", true, &options);

        // SAFETY: guarded by `ENV_LOCK`.
        unsafe {
            match &old_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            result.expect("expected a prepared command").args,
            vec!["gnome-terminal", "--", "sh", "-lc", "sample --flag"]
        );
    }

    #[test]
    fn terminal_preparation_should_fail_when_discovery_is_disabled_and_no_candidates_are_provided()
    {
        let options = PrepareOptions {
            terminal_candidates: Vec::new(),
            use_system_terminal_discovery: false,
        };
        assert!(prepare_command("sample --flag", true, &options).is_none());
    }

    #[test]
    fn field_code_only_command_should_not_prepare_an_empty_argv() {
        assert!(prepare_command(" %f %U ", false, &PrepareOptions::default()).is_none());
    }

    #[test]
    fn decode_desktop_string_value_unescapes_known_sequences() {
        assert_eq!(
            decode_desktop_string_value(r"a\sb\nc\td\re\\f"),
            "a b\nc\td\re\\f"
        );
        assert_eq!(decode_desktop_string_value(r"\X"), "X");
    }

    #[test]
    fn strip_field_codes_removes_orphaned_flatpak_markers() {
        // The space before `@@u` and the space after the closing `@@` both fall outside the
        // matched span, so they both survive — matching the C++'s `erase(pos, end + 2 - pos)`
        // exactly (only `@@u   @@` itself is removed, not its neighboring whitespace).
        assert_eq!(
            strip_field_codes("flatpak run app @@u   @@ --flag"),
            "flatpak run app  --flag"
        );
        // Non-whitespace interior is left alone.
        assert_eq!(strip_field_codes("app @@u foo @@"), "app @@u foo @@");
    }

    #[test]
    fn parse_custom_command_substitutes_cmd_placeholder() {
        assert_eq!(
            parse_custom_command("firefox", "flatpak-spawn --host $CMD"),
            "flatpak-spawn --host firefox"
        );
        assert_eq!(parse_custom_command("firefox", ""), "firefox");
    }

    #[test]
    fn launch_entry_fails_when_command_cannot_be_prepared() {
        let entry = DesktopEntry {
            exec: " %f ".to_string(),
            id: "org.example.App".to_string(),
            ..Default::default()
        };
        assert!(!launch_entry(&entry, &LaunchOptions::default()));
    }

    #[test]
    fn launch_action_fails_when_command_cannot_be_prepared() {
        let action = DesktopAction {
            id: "new-window".to_string(),
            exec: " %f ".to_string(),
            ..Default::default()
        };
        assert!(!launch_action(
            &action,
            "app",
            "",
            false,
            &LaunchOptions::default()
        ));
    }
}
