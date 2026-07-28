//! Port of systemd user-manager integration from `src/core/process/process.cpp`
//! (task 1.6.5): `cgroupIndicatesSystemdUserManager`/
//! `runningUnderSystemdUserManager` (probing `/proc/self/cgroup` once, cached
//! for the process's lifetime — a process cannot migrate between the login
//! session scope and the user manager), `escapeSystemdUnitName`,
//! `startSystemdService`, and `runAsyncAsSystemdService`.
//!
//! Divergences, all recorded here per rule 6 (must record even minor ones):
//!
//! - `StringUtils::generateUuid` (`src/util/string_utils.h`) hasn't been
//!   ported yet — that header is a much larger, separate future task, not
//!   scoped to process spawning. `generate_uuid_v4` below is a minimal,
//!   private, byte-for-byte port of just that one function (same
//!   `/dev/urandom` source, same RFC 4122 version/variant bit-forcing, same
//!   lowercase `8-4-4-4-12` hex format, same "return empty string on any I/O
//!   failure" behavior), scoped to this module. When `string_utils.h` is
//!   ported, this local copy should be deleted in favor of the shared one —
//!   left as-is for now to avoid pulling in a whole unrelated module (or a
//!   new `uuid` crate dependency, which would violate the "match C++
//!   behavior" bar for something the C++ hand-rolls itself with no C library
//!   backing it) for one anti-collision suffix on a systemd unit name.
//! - `startSystemdService`'s "app should inherit our environment" loop
//!   matches the C++ exactly, including what reads like a C++ quirk worth
//!   flagging: `char c = **s` dereferences the env entry's pointer-to-pointer
//!   twice, i.e. it inspects only the *first* character of each `NAME=VALUE`
//!   entry (checking it's alnum-or-`_`) before deciding whether to forward
//!   that variable via `-E NAME`; it does not validate every character of
//!   the name. This looks unusual but is what the C++ does, so
//!   `env_looks_forwardable` below reproduces exactly that (first-byte-only)
//!   check rather than "fixing" it into a full identifier validation.
//! - That same loop reads each entry's name via `to_str()` (rejecting
//!   non-UTF-8 names) rather than raw bytes; real process environments are
//!   overwhelmingly ASCII `NAME=VALUE` pairs, and a non-UTF-8 environment
//!   variable *name* (not value) is not a case the C++ itself handles any
//!   more meaningfully — it would just forward a `-E` flag carrying whatever
//!   bytes were in `argv`/`environ`. Skipping the rare non-UTF-8 name here
//!   instead of forwarding it is not reachable in practice.
//! - `running_under_systemd_user_manager` reads `/proc/self/cgroup` with
//!   `std::fs::read_to_string`, which requires valid UTF-8, whereas the C++'s
//!   `std::ifstream` reads it as a raw byte buffer with no such constraint.
//!   `/proc/self/cgroup` is kernel-generated and always ASCII in practice (a
//!   colon-separated hierarchy id, comma-separated controller names, and a
//!   cgroup path built from unit/slice names, all of which reject non-ASCII
//!   bytes at creation time), so this is not reachable in practice; a decode
//!   failure here just falls to `false` (not managed) via the same `Ok`/`Err`
//!   short-circuit already used for a missing file, rather than panicking.

use std::io::Read;
use std::sync::OnceLock;

use super::async_exec::{RunCallbacks, run_async_with_options};
use super::core_exec::{EnvOverride, RunOptions};
use super::detached::launch_detached;
use crate::log::Logger;

static LOG: Logger = Logger::new("process");

/// Port of `cgroupIndicatesSystemdUserManager`. Matches both the cgroup v2
/// `"0::<path>"` line and legacy v1 `"<id>:<controller>:<path>"` lines.
pub fn cgroup_indicates_systemd_user_manager(cgroup_file_contents: &str, uid: u32) -> bool {
    cgroup_file_contents.contains(&format!("/user@{uid}.service/"))
}

/// Port of `runningUnderSystemdUserManager`: whether this process itself is
/// managed by the systemd user manager (started as a user unit — uwsm, a
/// home-manager/NixOS service) rather than as a child of a login session
/// scope. Probed once via `/proc/self/cgroup` and cached, since a process
/// cannot migrate between the two after it starts.
#[cfg(target_os = "linux")]
pub fn running_under_systemd_user_manager() -> bool {
    static MANAGED: OnceLock<bool> = OnceLock::new();
    *MANAGED.get_or_init(|| {
        let Ok(contents) = std::fs::read_to_string("/proc/self/cgroup") else {
            return false;
        };
        // SAFETY: `getuid()` takes no arguments and cannot fail.
        let uid = unsafe { libc::getuid() };
        cgroup_indicates_systemd_user_manager(&contents, uid)
    })
}

#[cfg(not(target_os = "linux"))]
pub fn running_under_systemd_user_manager() -> bool {
    false
}

/// Port of `escapeSystemdUnitName`: only alphanumerics, `:`, `_`, and `.` are
/// allowed unescaped in a systemd unit name; every other byte becomes a
/// `\xHH` escape (lowercase hex).
pub fn escape_systemd_unit_name(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'.' {
            result.push(b as char);
        } else {
            result.push_str(&format!("\\x{b:02x}"));
        }
    }
    result
}

/// Minimal local port of `StringUtils::generateUuid` — see the module doc
/// comment for why this isn't shared with a not-yet-ported `string_utils`.
fn generate_uuid_v4() -> String {
    let Ok(mut urandom) = std::fs::File::open("/dev/urandom") else {
        return String::new();
    };
    let mut bytes = [0u8; 16];
    if urandom.read_exact(&mut bytes).is_err() {
        return String::new();
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// Port of the C++ `char c = **s` first-byte-only forwardability check — see
/// the module doc comment for why this deliberately doesn't validate the
/// whole name.
fn env_looks_forwardable(name: &str) -> bool {
    matches!(name.as_bytes().first(), Some(b) if b.is_ascii_alphanumeric() || *b == b'_')
}

/// Port of `startSystemdService`.
fn start_systemd_service(
    args: &[String],
    activation_token: &str,
    working_dir: &str,
    app_name: &str,
) -> bool {
    let mut systemd_args: Vec<String> = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--slice=app.slice".to_string(),
        // Only end the service when all subprocesses have exited. Otherwise, apps
        // using a launcher script (e.g. vscode) would end prematurely when the
        // script exits while the actual app process is still running.
        "--property=ExitType=cgroup".to_string(),
    ];

    // We launch the app as a systemd service instead of a scope so the user can:
    // 1. Place drop-in files in ~/.config/systemd/user/app-<desktop-id>@.service.d/
    //    to set properties like resource limits or env vars.
    // 2. See the app's output and exit code (if it fails) in `systemctl status`.
    if !app_name.is_empty() {
        let uuid = generate_uuid_v4();
        if !uuid.is_empty() {
            systemd_args.push(format!(
                "--unit=app-{}@{uuid}.service",
                escape_systemd_unit_name(app_name)
            ));
        }
    }
    if !working_dir.is_empty() {
        systemd_args.push(format!("--working-directory={working_dir}"));
    }

    let mut run_options = RunOptions::default();

    if !activation_token.is_empty() {
        systemd_args.push("-E".to_string());
        systemd_args.push("XDG_ACTIVATION_TOKEN".to_string());
        systemd_args.push("-E".to_string());
        systemd_args.push("DESKTOP_STARTUP_ID".to_string());
        run_options.env.push(EnvOverride {
            name: "XDG_ACTIVATION_TOKEN".to_string(),
            value: Some(activation_token.to_string()),
        });
        run_options.env.push(EnvOverride {
            name: "DESKTOP_STARTUP_ID".to_string(),
            value: Some(activation_token.to_string()),
        });
    }

    // App should inherit our environment: systemd units don't automatically
    // inherit the invoking process's environment the way a forked child would,
    // so every forwardable variable is named explicitly via `-E`, which tells
    // systemd-run to import it from systemd-run's *own* environment (already
    // our environment, since it's a direct child) into the unit.
    for (key, _) in std::env::vars_os() {
        let Some(name) = key.to_str() else { continue };
        if !env_looks_forwardable(name) {
            continue;
        }
        systemd_args.push("-E".to_string());
        systemd_args.push(name.to_string());
    }

    systemd_args.push("--".to_string());
    systemd_args.extend(args.iter().cloned());

    run_async_with_options(
        &systemd_args,
        RunCallbacks {
            on_exit: Some(Box::new(|result| {
                if result.exit_code != 0 {
                    // We'd like to show a toast or notification here, but the
                    // callback is not on the UI thread, so unclear how to.
                    LOG.error(format_args!(
                        "startSystemdService: systemd-run failed with exit code {}: {}",
                        result.exit_code,
                        String::from_utf8_lossy(&result.err)
                    ));
                }
            })),
            ..RunCallbacks::default()
        },
        run_options,
    )
}

/// Port of `runAsyncAsSystemdService`: prefers a systemd unit under
/// `--slice=app.slice` when this process is itself managed by the systemd
/// user manager; otherwise falls back to a plain double-fork detached launch
/// (`launch_detached`, task 1.6.3), matching the C++'s non-Linux and
/// not-user-managed paths, which are the same fallback call.
#[cfg(target_os = "linux")]
pub fn run_async_as_systemd_service(
    args: &[String],
    app_name: &str,
    activation_token: &str,
    working_dir: &str,
) -> bool {
    if args.is_empty() || args[0].is_empty() {
        return false;
    }
    if running_under_systemd_user_manager() {
        return start_systemd_service(args, activation_token, working_dir, app_name);
    }
    launch_detached(args, activation_token, working_dir)
}

#[cfg(not(target_os = "linux"))]
pub fn run_async_as_systemd_service(
    args: &[String],
    _app_name: &str,
    activation_token: &str,
    working_dir: &str,
) -> bool {
    launch_detached(args, activation_token, working_dir)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // Ported verbatim from `tests/process_test.cpp`'s
    // `cgroupDetectsSystemdUserManager`.
    #[test]
    fn cgroup_indicates_systemd_user_manager_matches_cpp_cases() {
        assert!(
            cgroup_indicates_systemd_user_manager(
                "0::/user.slice/user-1000.slice/user@1000.service/session.slice/wayland-wm@niri.service\n",
                1000
            ),
            "uwsm compositor unit should be detected as user-manager managed"
        );
        assert!(
            cgroup_indicates_systemd_user_manager(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/noctalia.service\n",
                1000
            ),
            "noctalia user service should be detected as user-manager managed"
        );
        assert!(
            !cgroup_indicates_systemd_user_manager(
                "0::/user.slice/user-1000.slice/session-2.scope/noctalia\n",
                1000
            ),
            "login session scope should not be detected as user-manager managed"
        );
        assert!(
            !cgroup_indicates_systemd_user_manager(
                "0::/user.slice/user-1001.slice/user@1001.service/app.slice/noctalia.service\n",
                1000
            ),
            "another user's manager should not be detected as ours"
        );
        assert!(
            cgroup_indicates_systemd_user_manager(
                "1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/noctalia.service\n",
                1000
            ),
            "legacy cgroup v1 dump should be detected as user-manager managed"
        );
        assert!(
            !cgroup_indicates_systemd_user_manager("", 1000),
            "empty cgroup dump should not be managed"
        );
    }

    #[test]
    fn escape_systemd_unit_name_passes_through_safe_characters() {
        assert_eq!(
            escape_systemd_unit_name("org.example.App_1:2"),
            "org.example.App_1:2"
        );
    }

    #[test]
    fn escape_systemd_unit_name_escapes_everything_else() {
        // Space (0x20) and '@' (0x40) are not in the allowed set.
        assert_eq!(
            escape_systemd_unit_name("my app@home"),
            "my\\x20app\\x40home"
        );
    }

    #[test]
    fn escape_systemd_unit_name_handles_empty_input() {
        assert_eq!(escape_systemd_unit_name(""), "");
    }

    #[test]
    fn generate_uuid_v4_has_rfc4122_shape_and_is_not_constant() {
        let a = generate_uuid_v4();
        let b = generate_uuid_v4();
        assert_eq!(a.len(), 36);
        assert_eq!(
            a.as_bytes()[14],
            b'4',
            "version nibble should be forced to 4"
        );
        assert!(
            matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant nibble should be forced to RFC 4122 (8/9/a/b), got {}",
            a
        );
        assert_ne!(a, b, "two consecutive UUIDs should not collide");
    }

    #[test]
    fn env_looks_forwardable_checks_only_the_first_byte() {
        assert!(env_looks_forwardable("PATH"));
        assert!(env_looks_forwardable("_privateVar"));
        // Matches the C++'s `char c = **s` behavior exactly: only the first
        // byte is checked, so a name with a disallowed character anywhere
        // after the first byte is still forwarded.
        assert!(env_looks_forwardable("PATH!NOT-AN-IDENTIFIER"));
        assert!(!env_looks_forwardable("!PATH"));
        assert!(!env_looks_forwardable(""));
    }

    #[test]
    fn running_under_systemd_user_manager_does_not_panic() {
        // No fixture: this just exercises the real-system probe safely, and
        // (being a `OnceLock`) is safe to call repeatedly/concurrently.
        let _ = running_under_systemd_user_manager();
        let _ = running_under_systemd_user_manager();
    }

    #[test]
    fn run_async_as_systemd_service_rejects_empty_args() {
        assert!(!run_async_as_systemd_service(&[], "", "", ""));
        assert!(!run_async_as_systemd_service(&[String::new()], "", "", ""));
    }
}
