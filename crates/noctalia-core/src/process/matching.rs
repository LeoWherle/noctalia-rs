//! Port of process listing & matching from `src/core/process/process.cpp`
//! (task 1.6.4): `/proc` command-line scanning behind a 250ms TTL cache
//! (`cachedProcessCommandLines`/`readProcessCommandLines`),
//! `commandLineMatchesAll`, `desktopPortalAvailable`, and
//! `flatpakAppInstalled` (XDG data-root enumeration).
//!
//! One divergence, matching the reasoning already recorded for
//! `RunResult::out`/`err` in `process::mod`'s doc comment: cached command
//! lines are `Vec<u8>`, not `String`. `/proc/<pid>/cmdline` is a raw,
//! NUL-separated byte buffer with no UTF-8 guarantee (a process can exec
//! with arbitrary bytes in argv), and the C++ only ever treats it as an
//! opaque byte string for substring search (`std::string::find` on raw
//! bytes) — never as text. `contains_bytes` below reimplements that
//! byte-level substring search directly, rather than lossily converting to
//! `String` first (which could corrupt a match at a boundary) or making an
//! otherwise-infallible API fallible.
//!
//! No C++ test exists for any of this (there's no fixture harness for
//! spawning/inspecting `/proc` entries in `tests/process_test.cpp`) — the
//! task's own bar is "round-trip tests for each helper" against real spawned
//! processes and a real temporary flatpak data root, which is what the test
//! module here does.
//!
//! One more, minor divergence: `readProcessCommandLines`'s C++
//! `directory_iterator` treats an enumeration-level error as a signal to
//! `break` (stop scanning, return whatever was collected so far), separate
//! from its `skip_permission_denied` option, which already turns individual
//! per-entry permission errors into silent skips rather than iterator
//! errors. `read_process_command_lines` here instead skips just the one bad
//! `std::fs::read_dir` entry and keeps scanning — never observed to trigger
//! against a real `/proc` (there's no realistic way for a single directory
//! entry to come back `Err` mid-listing), and if it ever did, scanning more
//! rather than less is strictly more complete, not less. Not fixed to match
//! exactly since matching it would mean discarding already-good data on a
//! hypothetical error that isn't reachable in practice.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PROCESS_COMMAND_LINE_CACHE_TTL: Duration = Duration::from_millis(250);

struct ProcessCommandLineCache {
    captured_at: Option<Instant>,
    command_lines: Vec<Vec<u8>>,
}

static PROCESS_COMMAND_LINE_CACHE: Mutex<ProcessCommandLineCache> =
    Mutex::new(ProcessCommandLineCache {
        captured_at: None,
        command_lines: Vec::new(),
    });

fn is_proc_pid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Port of `readProcCmdline`: NUL bytes become spaces, trailing spaces are
/// trimmed, and a non-empty result is wrapped in a leading/trailing space so
/// every argument (including the first and last) has word boundaries on both
/// sides for substring search.
fn read_proc_cmdline(pid_dir: &Path) -> Vec<u8> {
    let Ok(raw) = std::fs::read(pid_dir.join("cmdline")) else {
        return Vec::new();
    };
    if raw.is_empty() {
        return Vec::new();
    }

    let mut cmdline: Vec<u8> = raw
        .into_iter()
        .map(|b| if b == 0 { b' ' } else { b })
        .collect();
    while matches!(cmdline.last(), Some(b' ')) {
        cmdline.pop();
    }
    if cmdline.is_empty() {
        return Vec::new();
    }

    let mut wrapped = Vec::with_capacity(cmdline.len() + 2);
    wrapped.push(b' ');
    wrapped.append(&mut cmdline);
    wrapped.push(b' ');
    wrapped
}

/// Port of `readProcessCommandLines`.
fn read_process_command_lines() -> Vec<Vec<u8>> {
    let mut command_lines = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return command_lines;
    };

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_proc_pid_name(name) {
            continue;
        }
        let cmdline = read_proc_cmdline(&entry.path());
        if !cmdline.is_empty() {
            command_lines.push(cmdline);
        }
    }

    command_lines
}

/// Port of `cachedProcessCommandLines`.
fn cached_process_command_lines() -> Vec<Vec<u8>> {
    let mut cache = PROCESS_COMMAND_LINE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let stale = match cache.captured_at {
        None => true,
        Some(captured_at) => now.duration_since(captured_at) >= PROCESS_COMMAND_LINE_CACHE_TTL,
    };
    if stale {
        cache.command_lines = read_process_command_lines();
        cache.captured_at = Some(now);
    }
    cache.command_lines.clone()
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Port of `cachedProcessMatchesAny`.
fn cached_process_matches_any(needles: &[&str]) -> bool {
    let command_lines = cached_process_command_lines();
    command_lines.iter().any(|line| {
        needles
            .iter()
            .any(|needle| contains_bytes(line, needle.as_bytes()))
    })
}

/// Port of `commandLineMatchesAll`.
pub fn command_line_matches_all(needles: &[&str]) -> bool {
    if needles.is_empty() || needles.iter().any(|needle| needle.is_empty()) {
        return false;
    }

    let command_lines = cached_process_command_lines();
    command_lines.iter().any(|line| {
        needles
            .iter()
            .all(|needle| contains_bytes(line, needle.as_bytes()))
    })
}

/// Port of `desktopPortalAvailable`.
pub fn desktop_portal_available() -> bool {
    if !cached_process_matches_any(&["xdg-desktop-portal "]) {
        return false;
    }
    cached_process_matches_any(&[
        "xdg-desktop-portal-wlr ",
        "xdg-desktop-portal-hyprland ",
        "xdg-desktop-portal-gnome ",
        "xdg-desktop-portal-kde ",
        "niri-screenshare ",
    ])
}

/// Port of `isSafeFlatpakAppId`.
fn is_safe_flatpak_app_id(app_id: &str) -> bool {
    if app_id.is_empty() || app_id.contains('/') || app_id.contains('\\') {
        return false;
    }
    !app_id.contains("..")
}

/// Port of `appendFlatpakDataRoots`.
fn append_flatpak_data_roots(roots: &mut Vec<PathBuf>) {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(xdg_data_home) => roots.push(PathBuf::from(xdg_data_home)),
        None => {
            if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
                roots.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }

    let xdg_data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for dir in xdg_data_dirs.split(':') {
        if !dir.is_empty() {
            roots.push(PathBuf::from(dir));
        }
    }

    roots.push(PathBuf::from("/var/lib"));
}

/// Port of `flatpakAppInstalled`.
pub fn flatpak_app_installed(app_id: &str) -> bool {
    if !is_safe_flatpak_app_id(app_id) {
        return false;
    }

    let mut roots = Vec::new();
    append_flatpak_data_roots(&mut roots);

    roots
        .iter()
        .any(|root| root.join("flatpak/app").join(app_id).exists())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    /// Serializes every test in this module that reads or refreshes the
    /// shared `PROCESS_COMMAND_LINE_CACHE`: the TTL test below asserts on the
    /// cache being observably stale-then-fresh at specific instants, which
    /// only holds if no other thread in this test binary refreshes the same
    /// process-wide cache in between.
    static PROCESS_CACHE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Spawns `/bin/sh -c "sleep 2 & wait # <marker>"` and returns the shell
    /// process (not the backgrounded `sleep`). Deliberately not just
    /// `sh -c "sleep 2 # <marker>"`: when the last command in a `-c` script
    /// is the only thing left to run, bash (which `/bin/sh` is a symlink to
    /// on this host, and commonly elsewhere) execs directly into it instead
    /// of forking — replacing the shell's own argv with `sleep`'s, silently
    /// discarding the marker text. Backgrounding `sleep` and blocking on the
    /// builtin `wait` keeps the shell process alive with its original,
    /// marker-carrying argv intact for `/proc/<pid>/cmdline` to see.
    /// `process_group(0)` makes the shell its own process-group leader, so
    /// `kill_and_reap` can clean up the backgrounded `sleep` too, not just
    /// the shell.
    fn spawn_marker_process(marker: &str) -> Child {
        use std::os::unix::process::CommandExt as _;
        Command::new("/bin/sh")
            .args(["-c", &format!("sleep 2 & wait # {marker}")])
            .process_group(0)
            .spawn()
            .expect("failed to spawn marker process")
    }

    fn kill_and_reap(mut child: Child) {
        let pid = child.id() as i32;
        // SAFETY: `pid` is this child's own pid, which `spawn_marker_process`
        // made its own process-group leader via `process_group(0)`; `-pid` is
        // therefore a valid `kill(2)` target for the whole group (the shell
        // and its backgrounded `sleep`), and `pid` has not yet been reaped on
        // this path.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = child.wait();
    }

    #[test]
    fn command_line_matches_all_rejects_empty_needles() {
        assert!(!command_line_matches_all(&[]));
        assert!(!command_line_matches_all(&["real-needle", ""]));
    }

    #[test]
    fn is_safe_flatpak_app_id_rejects_traversal_and_separators() {
        assert!(is_safe_flatpak_app_id("org.example.App"));
        assert!(!is_safe_flatpak_app_id(""));
        assert!(!is_safe_flatpak_app_id("org/example"));
        assert!(!is_safe_flatpak_app_id("org\\example"));
        assert!(!is_safe_flatpak_app_id("../org.example.App"));
        // Isolates the standalone `contains("..")` check from the separator
        // checks above (no `/` or `\` here at all).
        assert!(!is_safe_flatpak_app_id("foo..bar"));
    }

    #[test]
    fn command_line_matches_all_finds_and_loses_a_spawned_marker_process() {
        let _guard = PROCESS_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let marker = format!("noctalia-process-match-test-{}", std::process::id());
        let child = spawn_marker_process(&marker);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            if command_line_matches_all(&[&marker]) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(found, "expected the spawned marker process to be found");

        kill_and_reap(child);

        // Give the cache one full TTL window to notice the process is gone.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut gone = false;
        while Instant::now() < deadline {
            std::thread::sleep(PROCESS_COMMAND_LINE_CACHE_TTL);
            if !command_line_matches_all(&[&marker]) {
                gone = true;
                break;
            }
        }
        assert!(
            gone,
            "expected the marker process to disappear after being killed"
        );
    }

    #[test]
    fn command_line_matches_all_requires_every_needle_on_the_same_line() {
        let _guard = PROCESS_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let suffix = std::process::id();
        let token_a = format!("noctalia-match-all-a-{suffix}");
        let token_b = format!("noctalia-match-all-b-{suffix}");
        let child = spawn_marker_process(&format!("{token_a} {token_b}"));

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut both_found = false;
        while Instant::now() < deadline {
            if command_line_matches_all(&[&token_a, &token_b]) {
                both_found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            both_found,
            "expected both needles to match the single process carrying both"
        );

        // `token_a` is real (on this process's command line) but
        // `not-present-anywhere` matches nothing anywhere — requiring ALL
        // needles on the SAME command line must reject this pairing, proving
        // this isn't secretly an "any needle, any line" check.
        assert!(!command_line_matches_all(&[
            &token_a,
            "not-present-anywhere"
        ]));

        kill_and_reap(child);
    }

    #[test]
    fn cached_process_command_lines_respects_the_ttl() {
        let _guard = PROCESS_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // Populate the cache with a pre-spawn snapshot.
        let _ = command_line_matches_all(&["priming the cache"]);

        let marker = format!("noctalia-process-ttl-test-{}", std::process::id());
        let child = spawn_marker_process(&marker);

        // Immediately after spawning, well within the TTL, the cache should
        // still hold the pre-spawn snapshot and not see the new process yet.
        assert!(
            !command_line_matches_all(&[&marker]),
            "expected the cache to still be serving its pre-spawn snapshot"
        );

        std::thread::sleep(PROCESS_COMMAND_LINE_CACHE_TTL + Duration::from_millis(100));
        assert!(
            command_line_matches_all(&[&marker]),
            "expected the cache to have refreshed and found the new process"
        );

        kill_and_reap(child);
    }

    #[test]
    fn desktop_portal_available_does_not_panic() {
        let _guard = PROCESS_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // No fixture: this just exercises the real-system code path safely.
        let _ = desktop_portal_available();
    }

    #[test]
    fn flatpak_app_installed_finds_an_app_in_a_fake_data_root() {
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let data_home =
            std::env::temp_dir().join(format!("noctalia_flatpak_test_{}", std::process::id()));
        let app_dir = data_home.join("flatpak/app/org.example.App");
        std::fs::create_dir_all(&app_dir).expect("failed to create fake flatpak app dir");

        let original = std::env::var_os("XDG_DATA_HOME");
        // SAFETY: guarded by ENV_MUTATION_LOCK above; see `detached`'s module
        // doc comment for why any `set_var`/`remove_var` in this crate's test
        // binary needs that lock (a raw `fork()` elsewhere doesn't go
        // through std's own `ENV_LOCK`).
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &data_home);
        }

        let installed = flatpak_app_installed("org.example.App");
        let not_installed = flatpak_app_installed("org.example.NotInstalled");
        let rejected = flatpak_app_installed("../org.example.App");

        // SAFETY: as above.
        unsafe {
            match &original {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&data_home);

        assert!(installed);
        assert!(!not_installed);
        assert!(!rejected);
    }
}
