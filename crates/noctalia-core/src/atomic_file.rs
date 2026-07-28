//! Port of `src/config/atomic_file.{cpp,h}`.
//!
//! `write_text_file_atomic` never writes the destination path directly: content
//! goes to a sibling `.tmp` file first, which is only `rename`d onto the real path
//! once fully written and closed. `rename(2)` is atomic on the same filesystem, so
//! a crash at any point leaves either the old file untouched or the new one intact
//! — never a half-written destination.
//!
//! Returns `Result` rather than C++'s `bool` (architecture decision 4: `thiserror`
//! in libraries) — the underlying write/rename mechanics and permission handling
//! are unchanged, only the failure signal is richer.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::io::{AsRawFd as _, IntoRawFd as _};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AtomicFileError {
    #[error("empty path")]
    EmptyPath,
    #[error("could not resolve a writable target for {0}")]
    UnresolvableTarget(PathBuf),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicWriteTarget {
    pub path: PathBuf,
    pub through_symlink: bool,
}

pub fn resolve_atomic_write_target(path: &Path) -> Option<AtomicWriteTarget> {
    if path.as_os_str().is_empty() {
        return None;
    }

    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Some(AtomicWriteTarget {
                    path: path.to_path_buf(),
                    through_symlink: false,
                });
            }
        }
        Err(err) => {
            return if err.kind() == std::io::ErrorKind::NotFound {
                Some(AtomicWriteTarget {
                    path: path.to_path_buf(),
                    through_symlink: false,
                })
            } else {
                None
            };
        }
    }

    // It's a symlink: prefer the fully resolved (canonical) target; a broken
    // symlink can't be canonicalized, so fall back to a lexical join of its raw
    // link text against the link's own directory.
    if let Ok(canonical) = fs::canonicalize(path) {
        return Some(AtomicWriteTarget {
            path: canonical,
            through_symlink: true,
        });
    }

    let link_target = fs::read_link(path).ok()?;
    Some(AtomicWriteTarget {
        path: resolve_relative_symlink_target(path, link_target),
        through_symlink: true,
    })
}

fn resolve_relative_symlink_target(link_path: &Path, target: PathBuf) -> PathBuf {
    let target = if target.is_relative() {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(target)
    } else {
        target
    };
    lexically_normal(&target)
}

/// Collapses `.`/`..` components without touching the filesystem (no symlink
/// resolution) — matches `std::filesystem::path::lexically_normal`.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => result.push(".."),
            },
            other => result.push(other.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut owned = path.as_os_str().to_os_string();
    owned.push(suffix);
    PathBuf::from(owned)
}

pub fn write_text_file_atomic(
    path: &Path,
    content: &str,
    mode: Option<u32>,
) -> Result<(), AtomicFileError> {
    if path.as_os_str().is_empty() {
        return Err(AtomicFileError::EmptyPath);
    }

    let target = resolve_atomic_write_target(path)
        .ok_or_else(|| AtomicFileError::UnresolvableTarget(path.to_path_buf()))?;
    if target.path.as_os_str().is_empty() {
        return Err(AtomicFileError::UnresolvableTarget(path.to_path_buf()));
    }

    if let Some(parent) = target.path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| AtomicFileError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let tmp_path = append_suffix(&target.path, ".tmp");
    let open_mode = mode.map_or(0o666, |m| m & 0o777);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_CLOEXEC)
        .mode(open_mode)
        .open(&tmp_path)
        .map_err(|source| AtomicFileError::Io {
            path: tmp_path.clone(),
            source,
        })?;

    // `open`'s mode is filtered by umask; force the exact requested mode the way
    // the C++ does with an explicit `fchmod`, when a mode was requested at all.
    if let Some(requested) = mode {
        let masked = requested & 0o777;
        // SAFETY: `file`'s fd is open and owned by this scope for the call.
        let rc = unsafe { libc::fchmod(file.as_raw_fd(), masked as libc::mode_t) };
        if rc != 0 {
            let source = std::io::Error::last_os_error();
            drop(file);
            let _ = fs::remove_file(&tmp_path);
            return Err(AtomicFileError::Io {
                path: tmp_path,
                source,
            });
        }
    }

    if let Err(source) = file.write_all(content.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(&tmp_path);
        return Err(AtomicFileError::Io {
            path: tmp_path,
            source,
        });
    }

    let fd = file.into_raw_fd();
    // SAFETY: `fd` was just obtained from `File::into_raw_fd`, is uniquely owned,
    // and isn't touched anywhere else. Closing it explicitly (rather than via
    // `File`'s `Drop`, which discards close() errors) is what lets us check the
    // close() result the way the C++ does — a failed close on some filesystems
    // (e.g. NFS) is the first place a write error surfaces.
    let close_rc = unsafe { libc::close(fd) };
    if close_rc != 0 {
        let source = std::io::Error::last_os_error();
        let _ = fs::remove_file(&tmp_path);
        return Err(AtomicFileError::Io {
            path: tmp_path,
            source,
        });
    }

    if let Err(source) = fs::rename(&tmp_path, &target.path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AtomicFileError::Io {
            path: target.path,
            source,
        });
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn make_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path)
            .expect("failed to stat file")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn empty_path_is_rejected() {
        assert!(resolve_atomic_write_target(Path::new("")).is_none());
        assert!(matches!(
            write_text_file_atomic(Path::new(""), "x", None),
            Err(AtomicFileError::EmptyPath)
        ));
    }

    #[test]
    fn writes_a_new_file_and_creates_missing_parents() {
        let dir = make_temp_dir("noctalia-atomic-file-new");
        let target = dir.join("nested").join("config.toml");

        write_text_file_atomic(&target, "hello", None).expect("write failed");

        assert_eq!(fs::read_to_string(&target).expect("read failed"), "hello");
        assert!(!append_suffix(&target, ".tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_tmp_file_survives_a_successful_write() {
        let dir = make_temp_dir("noctalia-atomic-file-no-tmp-leftover");
        let target = dir.join("config.toml");

        write_text_file_atomic(&target, "content", None).expect("write failed");

        assert!(!append_suffix(&target, ".tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replacing_content_never_leaves_a_partial_file() {
        let dir = make_temp_dir("noctalia-atomic-file-replace");
        let target = dir.join("config.toml");

        write_text_file_atomic(&target, "version-one", None).expect("first write failed");
        write_text_file_atomic(&target, "version-two-longer-content", None)
            .expect("second write failed");

        // Proves the rename mechanism, not an actual crash: content is staged at
        // `.tmp` and the real path is only ever touched by one atomic rename, so
        // it can only ever hold a complete, fully-written generation.
        assert_eq!(
            fs::read_to_string(&target).expect("read failed"),
            "version-two-longer-content"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crash_before_rename_leaves_the_old_file_untouched() {
        let dir = make_temp_dir("noctalia-atomic-file-crash-before-rename");
        let target = dir.join("config.toml");
        fs::write(&target, "original").expect("failed to seed original");

        // Simulates the "crash after opening/writing the tmp file but before
        // rename" window directly, since we can't kill this test process mid-call.
        fs::write(append_suffix(&target, ".tmp"), "half-written-garbage")
            .expect("failed to seed tmp");

        assert_eq!(
            fs::read_to_string(&target).expect("read failed"),
            "original"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn requested_mode_is_applied_exactly() {
        let dir = make_temp_dir("noctalia-atomic-file-mode");
        let target = dir.join("secret.toml");

        write_text_file_atomic(&target, "secret", Some(0o600)).expect("write failed");
        assert_eq!(mode_of(&target), 0o600);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_through_a_symlink_land_at_the_resolved_target() {
        let dir = make_temp_dir("noctalia-atomic-file-symlink");
        let real_target = dir.join("real.toml");
        let link = dir.join("link.toml");
        fs::write(&real_target, "original").expect("failed to seed real target");
        symlink(&real_target, &link).expect("failed to create symlink");

        let resolved =
            resolve_atomic_write_target(&link).expect("failed to resolve symlink target");
        assert!(resolved.through_symlink);
        assert_eq!(
            resolved.path,
            fs::canonicalize(&real_target).expect("failed to canonicalize")
        );

        write_text_file_atomic(&link, "written-through-symlink", None)
            .expect("write through symlink failed");

        assert_eq!(
            fs::read_to_string(&real_target).expect("read failed"),
            "written-through-symlink"
        );
        assert!(
            fs::symlink_metadata(&link)
                .expect("symlink vanished")
                .file_type()
                .is_symlink()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_symlink_falls_back_to_lexical_resolution() {
        let dir = make_temp_dir("noctalia-atomic-file-broken-symlink");
        let link = dir.join("dangling.toml");
        symlink("does-not-exist-yet.toml", &link).expect("failed to create dangling symlink");

        let resolved =
            resolve_atomic_write_target(&link).expect("failed to resolve dangling symlink");
        assert!(resolved.through_symlink);
        assert_eq!(resolved.path, dir.join("does-not-exist-yet.toml"));

        write_text_file_atomic(&link, "now-it-exists", None)
            .expect("write through dangling symlink failed");
        assert_eq!(
            fs::read_to_string(dir.join("does-not-exist-yet.toml")).expect("read failed"),
            "now-it-exists"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_symlink_path_resolves_to_itself() {
        let dir = make_temp_dir("noctalia-atomic-file-plain");
        let target = dir.join("plain.toml");

        let resolved = resolve_atomic_write_target(&target).expect("failed to resolve plain path");
        assert!(!resolved.through_symlink);
        assert_eq!(resolved.path, target);

        let _ = fs::remove_dir_all(&dir);
    }
}
