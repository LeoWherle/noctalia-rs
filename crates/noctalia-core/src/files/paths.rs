//! Port of `src/core/files/resource_paths.{cpp,h}`.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::log::Logger;

const LOG: Logger = Logger::new("paths");

// Provisional: meson injects the real install prefix/datadir
// (`get_option('prefix')`/`get_option('datadir')`) at compile time. The Rust build
// has no install-layout story yet — that lands with the Phase 16.5 cutover task
// (`nix/package.nix` gains a Rust build) — so these are placeholders until then.
const INSTALL_PREFIX: &str = "/usr";
const INSTALL_DATADIR: &str = "share";

// Set by build.rs to `<repo root>/assets`, mirroring meson's
// `-DNOCTALIA_SOURCE_ASSETS_DIR="<source root>/assets"`.
const SOURCE_ASSETS_DIR: &str = env!("NOCTALIA_SOURCE_ASSETS_DIR");

fn installed_assets_root() -> PathBuf {
    let datadir = Path::new(INSTALL_DATADIR);
    if datadir.is_absolute() {
        datadir.join("noctalia").join("assets")
    } else {
        Path::new(INSTALL_PREFIX)
            .join(datadir)
            .join("noctalia")
            .join("assets")
    }
}

fn source_assets_root() -> PathBuf {
    PathBuf::from(SOURCE_ASSETS_DIR)
}

fn is_asset_root(root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }

    root.join("emoji.json").exists()
        && root.join("fonts").join("tabler.ttf").exists()
        && root.join("templates").join("builtin.toml").exists()
        && root.join("translations").join("en.json").exists()
}

fn executable_path() -> Option<PathBuf> {
    std::fs::read_link("/proc/self/exe").ok()
}

fn append_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn asset_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(env_override) = env::var("NOCTALIA_ASSETS_DIR")
        && !env_override.is_empty()
    {
        let override_path = PathBuf::from(&env_override);
        if is_asset_root(&override_path) {
            candidates.push(override_path);
            return candidates;
        }
        LOG.warn(format_args!(
            "NOCTALIA_ASSETS_DIR is not a valid asset bundle: {}",
            override_path.display()
        ));
    }

    if let Some(exe_dir) = executable_path().as_deref().and_then(Path::parent) {
        append_unique(&mut candidates, exe_dir.join("assets"));
        if let Some(exe_parent) = exe_dir.parent() {
            append_unique(&mut candidates, exe_parent.join("assets"));

            let datadir = Path::new(INSTALL_DATADIR);
            if !datadir.as_os_str().is_empty() && !datadir.is_absolute() {
                append_unique(
                    &mut candidates,
                    exe_parent.join(datadir).join("noctalia").join("assets"),
                );
            }
            append_unique(
                &mut candidates,
                exe_parent.join("share").join("noctalia").join("assets"),
            );
        }
    }

    append_unique(&mut candidates, installed_assets_root());
    append_unique(&mut candidates, source_assets_root());
    candidates
}

fn resolve_assets_root() -> PathBuf {
    for candidate in asset_candidates() {
        if is_asset_root(&candidate) {
            LOG.debug(format_args!("using assets from {}", candidate.display()));
            return candidate;
        }
    }

    let fallback = installed_assets_root();
    LOG.warn(format_args!(
        "could not locate a valid asset bundle; defaulting to {}",
        fallback.display()
    ));
    fallback
}

static ASSETS_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn assets_root() -> &'static Path {
    ASSETS_ROOT.get_or_init(resolve_assets_root)
}

pub fn asset_path(relative_path: &str) -> PathBuf {
    assets_root().join(relative_path)
}

/// Expands $NAME and ${NAME} environment-variable references.
/// NAME matches [A-Za-z_][A-Za-z0-9_]*; undefined variables expand to empty string.
/// A lone '$' or '$' followed by a non-name character is left verbatim.
pub fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let is_name_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_name_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }

        // Handle ${NAME}
        if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(close_rel) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                let close = i + 2 + close_rel;
                let name = &input[i + 2..close];
                if let Ok(val) = env::var(name) {
                    out.push_str(&val);
                }
                i = close + 1;
                continue;
            }
            // No closing brace
            out.push('$');
            i += 1;
            continue;
        }

        // Handle $NAME
        if i + 1 < bytes.len() && is_name_start(bytes[i + 1]) {
            let mut j = i + 1;
            while j < bytes.len() && is_name_char(bytes[j]) {
                j += 1;
            }
            let name = &input[i + 1..j];
            if let Ok(val) = env::var(name) {
                out.push_str(&val);
            }
            i = j;
            continue;
        }

        // Lone '$'
        out.push('$');
        i += 1;
    }
    out
}

/// Expands ~ and ~/ paths using $HOME.
pub fn expand_user_path(path: &str) -> PathBuf {
    if path.is_empty() || !path.starts_with('~') {
        return PathBuf::from(path);
    }
    let home = match env::var("HOME") {
        Ok(val) if !val.is_empty() => val,
        _ => return PathBuf::from(path),
    };
    if path == "~" {
        return PathBuf::from(home);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

/// Collapses `.`/`..` components without touching the filesystem (no symlink
/// resolution) — matches `std::filesystem::path::lexically_normal`.
pub fn lexically_normal(path: &Path) -> PathBuf {
    use std::path::Component;
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

/// Expands ~ and resolves relative paths against `base_dir` when provided;
/// otherwise uses current working directory for relative paths.
pub fn resolve_path(path: &str, base_dir: Option<&Path>) -> PathBuf {
    if path.is_empty() || path.starts_with("color:") {
        return PathBuf::from(path);
    }

    let mut resolved = expand_user_path(path);
    if !resolved.is_absolute() {
        if let Some(base) = base_dir
            && !base.as_os_str().is_empty()
        {
            resolved = base.join(resolved);
        } else if let Ok(cwd) = env::current_dir() {
            resolved = cwd.join(resolved);
        }
    }

    lexically_normal(&resolved)
}

/// Shared shape of `FileUtils::configDir`/`stateDir` (`src/util/file_utils.h:254-284`):
/// `$NOCTALIA_<KIND>_HOME/noctalia`, else `$XDG_<KIND>_HOME/noctalia`, else
/// `$HOME/<home_suffix>/noctalia`, else empty. The C++ has three independent copies of this
/// shape (config/state/data); only the two this task needs (config, state) are ported, as one
/// shared helper rather than two more copies — a source-organization simplification only, same
/// precedent as `noctalia-ipc`'s deduplicated `resolve_socket_path`.
fn xdg_style_dir(noctalia_var: &str, xdg_var: &str, home_suffix: &str) -> String {
    if let Ok(v) = env::var(noctalia_var)
        && !v.is_empty()
    {
        return format!("{v}/noctalia");
    }
    if let Ok(v) = env::var(xdg_var)
        && !v.is_empty()
    {
        return format!("{v}/noctalia");
    }
    if let Ok(home) = env::var("HOME")
        && !home.is_empty()
    {
        return format!("{home}/{home_suffix}/noctalia");
    }
    String::new()
}

/// Port of `FileUtils::configDir` (`src/util/file_utils.h:254-268`).
pub fn config_dir() -> String {
    xdg_style_dir("NOCTALIA_CONFIG_HOME", "XDG_CONFIG_HOME", ".config")
}

/// Port of `FileUtils::stateDir` (`src/util/file_utils.h:270-284`).
pub fn state_dir() -> String {
    xdg_style_dir("NOCTALIA_STATE_HOME", "XDG_STATE_HOME", ".local/state")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn make_temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    #[test]
    fn empty_root_is_not_an_asset_root() {
        assert!(!is_asset_root(Path::new("")));
    }

    #[test]
    fn incomplete_bundle_is_not_an_asset_root() {
        let root = make_temp_dir("noctalia-asset-root-incomplete");
        assert!(!is_asset_root(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn complete_bundle_is_an_asset_root() {
        let root = make_temp_dir("noctalia-asset-root-complete");
        fs::create_dir_all(root.join("fonts")).expect("mkdir fonts");
        fs::create_dir_all(root.join("templates")).expect("mkdir templates");
        fs::create_dir_all(root.join("translations")).expect("mkdir translations");
        fs::write(root.join("emoji.json"), "{}").expect("write emoji.json");
        fs::write(root.join("fonts").join("tabler.ttf"), b"").expect("write tabler.ttf");
        fs::write(root.join("templates").join("builtin.toml"), "").expect("write builtin.toml");
        fs::write(root.join("translations").join("en.json"), "{}").expect("write en.json");

        assert!(is_asset_root(&root));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn source_assets_root_points_at_the_real_repo_assets() {
        // This is the repo's actual `assets/` dir (build.rs resolves it relative to
        // the crate's manifest dir), so it should always satisfy `is_asset_root`.
        assert!(is_asset_root(&source_assets_root()));
    }

    #[test]
    fn expand_env_vars_expands_braced_and_unbraced_vars() {
        let _guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var("TEST_VAR_FOO", "hello");
            env::set_var("TEST_VAR_BAR", "world");
        }
        assert_eq!(expand_env_vars("echo $TEST_VAR_FOO"), "echo hello");
        assert_eq!(expand_env_vars("echo ${TEST_VAR_BAR}!"), "echo world!");
        assert_eq!(expand_env_vars("echo $NONEXISTENT_VAR"), "echo ");
        assert_eq!(
            expand_env_vars("echo ${UNCLOSED_VAR"),
            "echo ${UNCLOSED_VAR"
        );
        assert_eq!(expand_env_vars("price is $5"), "price is $5");
        unsafe {
            env::remove_var("TEST_VAR_FOO");
            env::remove_var("TEST_VAR_BAR");
        }
    }

    #[test]
    fn expand_user_path_expands_tilde() {
        let home = env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            assert_eq!(expand_user_path("~"), PathBuf::from(&home));
            assert_eq!(
                expand_user_path("~/sub/dir"),
                PathBuf::from(&home).join("sub/dir")
            );
        }
        assert_eq!(expand_user_path("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_user_path("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn lexically_normal_collapses_curdir_and_parentdir() {
        assert_eq!(
            lexically_normal(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            lexically_normal(Path::new("a/b/../../c")),
            PathBuf::from("c")
        );
        assert_eq!(lexically_normal(Path::new(".")), PathBuf::from("."));
    }

    #[test]
    fn resolve_path_handles_base_dir_and_special_schemes() {
        assert_eq!(
            resolve_path("color:#123456", None),
            PathBuf::from("color:#123456")
        );
        assert_eq!(
            resolve_path("foo/bar.toml", Some(Path::new("/base"))),
            PathBuf::from("/base/foo/bar.toml")
        );
        assert_eq!(
            resolve_path("/abs/foo.toml", Some(Path::new("/base"))),
            PathBuf::from("/abs/foo.toml")
        );
    }

    /// Saves and restores a real env var around a test that needs to control it, unlike the
    /// synthetic `TEST_VAR_FOO`/`BAR` names above which are always safe to just remove.
    struct EnvVarRestore {
        name: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarRestore {
        fn unset(name: &'static str) -> Self {
            let previous = env::var_os(name);
            // SAFETY: guarded by ENV_MUTATION_LOCK for the caller's whole critical section.
            unsafe { env::remove_var(name) };
            Self { name, previous }
        }
    }

    impl Drop for EnvVarRestore {
        fn drop(&mut self) {
            // SAFETY: guarded by ENV_MUTATION_LOCK for the caller's whole critical section.
            unsafe {
                match &self.previous {
                    Some(value) => env::set_var(self.name, value),
                    None => env::remove_var(self.name),
                }
            }
        }
    }

    #[test]
    fn config_dir_and_state_dir_follow_the_documented_precedence() {
        let _guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _r1 = EnvVarRestore::unset("NOCTALIA_CONFIG_HOME");
        let _r2 = EnvVarRestore::unset("XDG_CONFIG_HOME");
        let _r3 = EnvVarRestore::unset("NOCTALIA_STATE_HOME");
        let _r4 = EnvVarRestore::unset("XDG_STATE_HOME");
        let _r5 = EnvVarRestore::unset("HOME");

        // Nothing set at all: empty.
        assert_eq!(config_dir(), "");
        assert_eq!(state_dir(), "");

        // HOME-relative fallback.
        // SAFETY: guarded by ENV_MUTATION_LOCK.
        unsafe { env::set_var("HOME", "/home/tester") };
        assert_eq!(config_dir(), "/home/tester/.config/noctalia");
        assert_eq!(state_dir(), "/home/tester/.local/state/noctalia");

        // XDG_*_HOME takes priority over HOME.
        // SAFETY: guarded by ENV_MUTATION_LOCK.
        unsafe {
            env::set_var("XDG_CONFIG_HOME", "/xdg/config");
            env::set_var("XDG_STATE_HOME", "/xdg/state");
        }
        assert_eq!(config_dir(), "/xdg/config/noctalia");
        assert_eq!(state_dir(), "/xdg/state/noctalia");

        // NOCTALIA_*_HOME takes priority over XDG_*_HOME.
        // SAFETY: guarded by ENV_MUTATION_LOCK.
        unsafe {
            env::set_var("NOCTALIA_CONFIG_HOME", "/noctalia/config");
            env::set_var("NOCTALIA_STATE_HOME", "/noctalia/state");
        }
        assert_eq!(config_dir(), "/noctalia/config/noctalia");
        assert_eq!(state_dir(), "/noctalia/state/noctalia");
    }
}
