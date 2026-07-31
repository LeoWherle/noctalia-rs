//! Port of `src/system/distro_info.{cpp,h}` (task 5.6.2): OS-release/kernel/session/hostname/
//! uptime labels for hardware-info displays. Depends on already-ported `i18n::{tr,trp}` (task
//! 1.8, and — per that module's own doc comment — this is its first real caller).
//!
//! `StringUtils::trim`/`unquote` aren't centrally ported yet; this reuses the crate-shared
//! `crate::brightness::c_trim` and `crate::icon_resolver::unquote` rather than duplicating them a
//! third time (same reuse precedent noted in both of those modules' doc comments).
//!
//! `DistroDetector`, a C++ class with exactly one static method (`detect`), is flattened to a
//! free function `detect()` at module scope — Rust has no static-method-only-class idiom, and a
//! module-level function is the direct equivalent.
//!
//! Recorded divergence: `parseOsRelease` reads the file as raw bytes in the C++ (`std::ifstream`,
//! byte-wise `getline`/`substr`), so it still parses whatever `ID`/`NAME`/... lines it can find
//! even if some other line in the file holds invalid UTF-8. This port uses `fs::read_to_string`,
//! which fails the whole read on any invalid UTF-8 byte anywhere in the file. `os-release` files
//! are ASCII/UTF-8 by spec in practice, so this is a low-risk, deliberate divergence rather than a
//! parity gap worth hand-rolling byte-oriented parsing for.
//!
//! Same class of divergence in `sessionDisplayName`: `pw_name`/`pw_gecos` are read via
//! `CStr::to_string_lossy` (UTF-8-validating, lossy-substituting on invalid bytes) rather than the
//! C++'s raw byte-wise `std::string` construction from the `char*`. Real usernames/GECOS fields
//! are effectively always valid UTF-8/ASCII in practice.
//!
//! No C++ test exists for this file (confirmed: no `distro_info_test.cpp` in `tests/`) — task
//! 5.6's own done bar is a smoke test.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use noctalia_core::i18n;

use crate::brightness::c_trim;
use crate::icon_resolver::unquote;

/// Port of `DistroInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DistroInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub pretty_name: String,
}

/// Port of the anonymous namespace's `parseOsRelease`.
fn parse_os_release(path: &Path) -> Option<HashMap<String, String>> {
    let content = fs::read_to_string(path).ok()?;

    let mut values = HashMap::new();
    for line in content.lines() {
        let trimmed = c_trim(line);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        if eq == 0 {
            continue;
        }

        let key = trimmed[..eq].to_string();
        let value = unquote(c_trim(&trimmed[eq + 1..]));
        values.insert(key, value);
    }

    Some(values)
}

/// Port of `DistroDetector::detect` — see the module doc comment on flattening the C++'s
/// static-method-only class.
pub fn detect() -> Option<DistroInfo> {
    for candidate in ["/etc/os-release", "/usr/lib/os-release"] {
        let Some(parsed) = parse_os_release(Path::new(candidate)) else {
            continue;
        };

        let mut info = DistroInfo::default();
        if let Some(v) = parsed.get("ID") {
            info.id = v.clone();
        }
        if let Some(v) = parsed.get("NAME") {
            info.name = v.clone();
        }
        if let Some(v) = parsed.get("VERSION") {
            info.version = v.clone();
        }
        if let Some(v) = parsed.get("PRETTY_NAME") {
            info.pretty_name = v.clone();
        }

        if !info.pretty_name.is_empty() || !info.name.is_empty() || !info.id.is_empty() {
            return Some(info);
        }
    }
    None
}

/// Port of `distroLabel`.
pub fn distro_label() -> String {
    if let Some(distro) = detect() {
        if !distro.pretty_name.is_empty() {
            return distro.pretty_name;
        }
        if !distro.name.is_empty() {
            return distro.name;
        }
        if !distro.id.is_empty() {
            return distro.id;
        }
    }
    i18n::tr("system.hardware.unknown-distro", &[])
}

/// Reads a fixed-size NUL-terminated `libc::utsname` field as a `String`, stopping at the first
/// NUL byte (or the field's end, if unterminated).
fn utsname_field(field: &[libc::c_char]) -> String {
    let len = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    // SAFETY: `field[..len]` holds no NUL byte by construction; `uname(2)` fills these fields
    // with the host's C-locale-safe sysname/nodename/release text.
    let bytes: Vec<u8> = field[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn uname() -> Option<libc::utsname> {
    let mut un: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `un` is a valid, live `libc::utsname` buffer of the size `uname(2)` expects;
    // `uname` only writes into it.
    let rc = unsafe { libc::uname(&mut un) };
    (rc == 0).then_some(un)
}

/// Port of `kernelLabel`.
pub fn kernel_label() -> String {
    if let Some(un) = uname() {
        let release = utsname_field(&un.release);
        if !release.is_empty() {
            let sysname = utsname_field(&un.sysname);
            if !sysname.is_empty() {
                return format!("{sysname} {release}");
            }
            return release;
        }
    }
    i18n::tr("control-center.system.unknown", &[])
}

/// Port of `hostName`.
pub fn host_name() -> String {
    if let Some(un) = uname() {
        let nodename = utsname_field(&un.nodename);
        if !nodename.is_empty() {
            return nodename;
        }
    }
    i18n::tr("control-center.system.unknown", &[])
}

/// `statx`'s `STATX_BTIME` (creation/birth time) for `path`, or `None` if unsupported/unavailable
/// (matches the C++'s `ec || !(stx_mask & STATX_BTIME) || stx_btime.tv_sec <= 0` skip condition).
fn statx_birth_time(path: &str) -> Option<u64> {
    let c_path = CString::new(path).ok()?;
    let mut stx: libc::statx = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid, NUL-terminated C string live for the call; `stx` is a valid,
    // live output buffer of the size `statx(2)` expects. `statx` only reads the path/dirfd/flags/
    // mask arguments and writes into `stx`.
    let rc = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            c_path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_BTIME,
            &mut stx,
        )
    };
    if rc != 0 || (stx.stx_mask & libc::STATX_BTIME) == 0 {
        return None;
    }
    let btime_sec = stx.stx_btime.tv_sec;
    (btime_sec > 0).then_some(btime_sec as u64)
}

fn now_unix_seconds() -> i64 {
    // SAFETY: passing a null out-param to `time(2)` is explicitly valid POSIX usage — it just
    // returns the value instead of also writing it through the pointer.
    unsafe { libc::time(std::ptr::null_mut()) }
}

/// Port of `osAgeLabel`.
pub fn os_age_label() -> String {
    let mut oldest: u64 = 0;
    for path in ["/", "/etc", "/var", "/home"] {
        if let Some(btime) = statx_birth_time(path)
            && (oldest == 0 || btime < oldest)
        {
            oldest = btime;
        }
    }

    if oldest == 0 {
        oldest = fs::metadata("/etc/machine-id")
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|since_epoch| since_epoch.as_secs())
            .filter(|&mtime| mtime > 0)
            .unwrap_or(0);
    }

    if oldest == 0 {
        return i18n::tr("control-center.system.unknown", &[]);
    }

    let now = now_unix_seconds();
    if now <= 0 || (now as u64) <= oldest {
        return i18n::tr("time.duration.less-than-day", &[]);
    }

    let seconds = now as u64 - oldest;
    let days = seconds / 86400;
    let years = days / 365;
    let months = (days % 365) / 30;
    if years > 0 {
        let year_text = i18n::trp("time.units.year", years as i64, &[]);
        if months > 0 {
            let month_text = i18n::trp("time.units.month", months as i64, &[]);
            return i18n::tr(
                "time.duration.two-parts",
                &[("first", year_text), ("second", month_text)],
            );
        }
        return year_text;
    }
    i18n::trp("time.units.day", days as i64, &[])
}

/// Port of `sessionDisplayName`.
pub fn session_display_name() -> String {
    // SAFETY: `getuid` takes no arguments and cannot fail.
    let uid = unsafe { libc::getuid() };
    // SAFETY: `getpwuid` returns either null or a pointer into a buffer libc owns and reuses on
    // the next passwd-db call on this thread; the fields read out below are copied before any
    // other passwd-db call happens.
    let pw = unsafe { libc::getpwuid(uid) };

    let mut login = "user".to_string();
    if !pw.is_null() {
        // SAFETY: `pw` was just checked non-null; `pw_name` is a valid NUL-terminated C string
        // for as long as `pw` itself is (no other passwd-db call happens before it's read).
        let pw_name = unsafe { CStr::from_ptr((*pw).pw_name) };
        login = pw_name.to_string_lossy().into_owned();
    } else if let Ok(env_login) = std::env::var("USER") {
        login = env_login;
    }

    if !pw.is_null() {
        // SAFETY: same as the `pw_name` read above.
        let gecos_ptr = unsafe { (*pw).pw_gecos };
        if !gecos_ptr.is_null() {
            // SAFETY: `gecos_ptr` was just checked non-null; valid NUL-terminated C string under
            // the same lifetime as `pw_name` above.
            let gecos = unsafe { CStr::from_ptr(gecos_ptr) }
                .to_string_lossy()
                .into_owned();
            if !gecos.is_empty() {
                return match gecos.find(',') {
                    Some(comma) => gecos[..comma].to_string(),
                    None => gecos,
                };
            }
        }
    }
    login
}

/// Port of `systemUptime`. The C++'s `in >> up >> idleDummy` requires both whitespace-separated
/// fields to parse as `double`s (the second is read and discarded) or the whole extraction fails;
/// ported here as two sequential `f64` parses that must both succeed.
pub fn system_uptime() -> Option<Duration> {
    let content = fs::read_to_string("/proc/uptime").ok()?;
    let mut fields = content.split_whitespace();
    let uptime_secs: f64 = fields.next()?.parse().ok()?;
    let _idle_secs: f64 = fields.next()?.parse().ok()?;
    Some(Duration::from_secs(uptime_secs as u64))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_os_release_reads_known_keys_and_skips_comments_and_blank_lines() {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-distro-info-test-{}-{}",
            std::process::id(),
            "known-keys"
        ));
        fs::create_dir_all(&dir).expect("create fixture dir");
        let path = dir.join("os-release");
        fs::write(
            &path,
            "# a comment\n\nID=nixos\nNAME=\"NixOS\"\nVERSION=\"26.11 (Cascade)\"\nPRETTY_NAME='NixOS 26.11'\n",
        )
        .expect("write fixture");

        let parsed = parse_os_release(&path).expect("file exists and parses");
        assert_eq!(parsed.get("ID").map(String::as_str), Some("nixos"));
        assert_eq!(parsed.get("NAME").map(String::as_str), Some("NixOS"));
        assert_eq!(
            parsed.get("VERSION").map(String::as_str),
            Some("26.11 (Cascade)")
        );
        assert_eq!(
            parsed.get("PRETTY_NAME").map(String::as_str),
            Some("NixOS 26.11")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_os_release_skips_lines_with_no_equals_or_a_leading_equals() {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-distro-info-test-{}-{}",
            std::process::id(),
            "malformed-lines"
        ));
        fs::create_dir_all(&dir).expect("create fixture dir");
        let path = dir.join("os-release");
        fs::write(&path, "NOTAKEYVALUELINE\n=leading-equals\nID=arch\n").expect("write fixture");

        let parsed = parse_os_release(&path).expect("file exists and parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("ID").map(String::as_str), Some("arch"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_os_release_returns_none_for_a_missing_file() {
        assert!(parse_os_release(Path::new("/definitely/not/a/real/os-release/path")).is_none());
    }

    #[test]
    fn detect_finds_the_real_hosts_distro_info() {
        // `/etc/os-release` is present on every real Linux host this shell targets (including
        // this dev shell) — a smoke test against the live system, same precedent as other
        // sysfs/procfs-reading modules in this crate.
        let distro =
            detect().expect("a real Linux host has /etc/os-release or /usr/lib/os-release");
        assert!(!distro.id.is_empty() || !distro.name.is_empty() || !distro.pretty_name.is_empty());
    }

    #[test]
    fn distro_label_is_never_empty() {
        assert!(!distro_label().is_empty());
    }

    #[test]
    fn kernel_label_reports_the_real_kernel() {
        let label = kernel_label();
        assert!(!label.is_empty());
        // `uname(2)` cannot fail with the arguments used here, so this is always the real
        // "sysname release" branch on Linux, never the i18n-unknown fallback.
        assert!(label.starts_with("Linux "), "got: {label}");
    }

    #[test]
    fn host_name_is_never_empty() {
        assert!(!host_name().is_empty());
    }

    #[test]
    fn os_age_label_is_never_empty() {
        assert!(!os_age_label().is_empty());
    }

    #[test]
    fn session_display_name_is_never_empty() {
        assert!(!session_display_name().is_empty());
    }

    #[test]
    fn system_uptime_reports_a_positive_duration_on_a_running_host() {
        let uptime = system_uptime().expect("/proc/uptime is readable on Linux");
        assert!(uptime.as_secs() > 0);
    }
}
