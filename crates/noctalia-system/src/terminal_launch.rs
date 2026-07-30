//! Port of `src/system/terminal_launch.{cpp,h}` (task 5.5.3): terminal-emulator discovery and
//! `sh -lc <command>` argv preparation for launching a command inside a terminal window.
//!
//! `is_executable_on_path` is a private near-duplicate of the already-ported
//! `noctalia_core::process::command_exists`'s PATH-search core — not reused, because the C++
//! original (`isExecutableOnPath` in this same translation unit) doesn't call `commandExists`
//! either: it checks `X_OK` only (no `is_regular_file` requirement), a real behavioral difference
//! from `commandExists`, not just an incidental duplication.

use std::os::unix::ffi::OsStrExt;

use noctalia_core::files::paths::expand_user_path;

/// Port of `terminal_launch::Options`.
#[derive(Debug, Clone)]
pub struct Options {
    pub terminal_candidates: Vec<String>,
    pub use_system_terminal_discovery: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            terminal_candidates: Vec::new(),
            use_system_terminal_discovery: true,
        }
    }
}

/// Port of the anonymous namespace's `tokenize`: splits on unquoted spaces, honoring single and
/// double quotes (no backslash-escaping support — unlike `desktop_entry_launch`'s own, separate
/// `tokenize`, which does; the two are independent private functions in independent C++
/// translation units, not shared there either).
fn tokenize(cmd: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for c in cmd.chars() {
        if c == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if c == ' ' && !in_single && !in_double {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(c);
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

/// Port of `isExecutableOnPath`: absolute/relative paths (containing `/`) are checked directly;
/// bare names are searched over `$PATH`. Unlike `process::command_exists`, this only requires
/// `X_OK` — a directory on `$PATH` with the right name would pass here, matching the C++ exactly.
fn is_executable_on_path(binary: &str) -> bool {
    if binary.is_empty() {
        return false;
    }
    if binary.contains('/') {
        let expanded = expand_executable_path(binary);
        return access_x_ok(&expanded);
    }

    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    if path_env.is_empty() {
        return false;
    }
    let path_bytes = path_env.as_bytes();

    let mut start = 0usize;
    while start <= path_bytes.len() {
        let end = path_bytes[start..]
            .iter()
            .position(|&b| b == b':')
            .map(|i| start + i);
        let segment = match end {
            Some(e) => &path_bytes[start..e],
            None => &path_bytes[start..],
        };
        if !segment.is_empty() {
            let mut candidate = Vec::with_capacity(segment.len() + 1 + binary.len());
            candidate.extend_from_slice(segment);
            candidate.push(b'/');
            candidate.extend_from_slice(binary.as_bytes());
            let candidate_path = std::ffi::OsStr::from_bytes(&candidate);
            if access_x_ok(candidate_path.to_string_lossy().as_ref()) {
                return true;
            }
        }
        match end {
            Some(e) => start = e + 1,
            None => break,
        }
    }
    false
}

fn access_x_ok(path: &str) -> bool {
    let Ok(cpath) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: `cpath` is a valid, NUL-terminated C string live for this call.
    unsafe { libc::access(cpath.as_ptr(), libc::X_OK) == 0 }
}

fn discover_terminal(options: &Options) -> Vec<String> {
    if !options.terminal_candidates.is_empty() {
        for candidate in &options.terminal_candidates {
            let terminal = tokenize(candidate);
            if !terminal.is_empty() && is_executable_on_path(&terminal[0]) {
                return terminal;
            }
        }
        return Vec::new();
    }
    if !options.use_system_terminal_discovery {
        return Vec::new();
    }

    if let Ok(env_terminal) = std::env::var("TERMINAL")
        && !env_terminal.is_empty()
    {
        let terminal = tokenize(&env_terminal);
        if !terminal.is_empty() && is_executable_on_path(&terminal[0]) {
            return terminal;
        }
    }

    const TERMINAL_CANDIDATES: [&str; 11] = [
        "x-terminal-emulator",
        "ghostty",
        "kitty",
        "alacritty",
        "wezterm",
        "foot",
        "konsole",
        "gnome-terminal",
        "kgx",
        "ptyxis",
        "xterm",
    ];
    for candidate in TERMINAL_CANDIDATES {
        if is_executable_on_path(candidate) {
            return vec![candidate.to_string()];
        }
    }
    Vec::new()
}

fn uses_command_separator(terminal: &str) -> bool {
    terminal == "gnome-terminal" || terminal == "kgx" || terminal == "ptyxis"
}

/// Port of `terminal_launch::prepareCommand`.
pub fn prepare_command(command: &str, options: &Options) -> Option<Vec<String>> {
    if command.is_empty() {
        return None;
    }

    let mut terminal = discover_terminal(options);
    if terminal.is_empty() {
        return None;
    }

    if terminal[0].contains('/') {
        terminal[0] = expand_executable_path(&terminal[0]);
    }

    let term_bin = terminal[0].clone();
    if uses_command_separator(&term_bin) {
        terminal.push("--".to_string());
    } else {
        terminal.push("-e".to_string());
    }
    terminal.push("sh".to_string());
    terminal.push("-lc".to_string());
    terminal.push(command.to_string());
    Some(terminal)
}

/// Port of `terminal_launch::launch`.
pub fn launch(command: &str, options: &Options) -> bool {
    match prepare_command(command, options) {
        Some(prepared) => noctalia_core::process::launch_detached(&prepared, "", ""),
        None => false,
    }
}

/// Serializes every test in this crate that mutates `PATH`/`TERMINAL`. `desktop_entry_launch`'s
/// own test module reuses this same lock for its one `PATH`-mutating test, rather than declaring
/// a second, uncoordinated lock over the same process-global `PATH` — see that module's test for
/// why.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::test_support::ENV_LOCK;
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn make_executable_fixture(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("chmod fixture");
        path
    }

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia_terminal_launch_test_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    #[test]
    fn prepare_command_rejects_empty_command() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        assert!(prepare_command("", &Options::default()).is_none());
    }

    #[test]
    fn prepare_command_fails_when_discovery_disabled_and_no_candidates() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let options = Options {
            terminal_candidates: Vec::new(),
            use_system_terminal_discovery: false,
        };
        assert!(prepare_command("sample --flag", &options).is_none());
    }

    #[test]
    fn prepare_command_uses_first_executable_candidate() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempdir("candidate");
        let fake_terminal = make_executable_fixture(&dir, "fake-terminal");
        let options = Options {
            terminal_candidates: vec![
                "missing-terminal-candidate".to_string(),
                fake_terminal.to_string_lossy().into_owned(),
            ],
            use_system_terminal_discovery: true,
        };
        let prepared = prepare_command("sample --flag", &options).expect("expected a command");
        assert_eq!(
            prepared,
            vec![
                fake_terminal.to_string_lossy().into_owned(),
                "-e".to_string(),
                "sh".to_string(),
                "-lc".to_string(),
                "sample --flag".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_command_uses_command_separator_for_gnome_style_terminals() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempdir("gnome");
        let _fake_gnome_terminal = make_executable_fixture(&dir, "gnome-terminal");
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
        let options = Options {
            terminal_candidates: vec!["gnome-terminal".to_string()],
            use_system_terminal_discovery: true,
        };
        let prepared = prepare_command("sample --flag", &options).expect("expected a command");
        // SAFETY: guarded by `ENV_LOCK`.
        unsafe {
            match &old_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            prepared,
            vec![
                "gnome-terminal".to_string(),
                "--".to_string(),
                "sh".to_string(),
                "-lc".to_string(),
                "sample --flag".to_string(),
            ]
        );
    }

    #[test]
    fn prepare_command_returns_none_when_no_candidate_is_executable() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let options = Options {
            terminal_candidates: vec!["definitely-not-a-real-terminal-xyz".to_string()],
            use_system_terminal_discovery: true,
        };
        assert!(prepare_command("sample --flag", &options).is_none());
    }

    #[test]
    fn tokenize_honors_quotes() {
        assert_eq!(
            tokenize(r#"sample --title "Hello World" --single 'Two Words'"#),
            vec!["sample", "--title", "Hello World", "--single", "Two Words"]
        );
    }
}
