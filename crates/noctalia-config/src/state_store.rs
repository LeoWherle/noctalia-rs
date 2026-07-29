//! State store — owner-scoped persistent key-value table.
//! Port of `src/config/state_store.{cpp,h}`.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use noctalia_core::atomic_file::write_text_file_atomic;
use noctalia_core::log::Logger;

const LOG: Logger = Logger::new("state");

pub fn valid_state_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

pub fn secure_state_file(path: &Path) {
    if path.as_os_str().is_empty() || !path.exists() {
        return;
    }
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        if let Err(err) = fs::set_permissions(path, perms) {
            LOG.warn(format_args!("failed to secure {}: {}", path.display(), err));
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateStore {
    path: PathBuf,
    state: toml::Table,
    parse_error: String,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: toml::Table::new(),
            parse_error: String::new(),
        }
    }

    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        self.path = path.into();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn parse_error(&self) -> &str {
        &self.parse_error
    }

    pub fn load(&mut self) {
        self.state = toml::Table::new();
        self.parse_error.clear();

        if self.path.as_os_str().is_empty() || !self.path.exists() {
            return;
        }

        secure_state_file(&self.path);

        match fs::read_to_string(&self.path) {
            Ok(content) => match content.parse::<toml::Table>() {
                Ok(table) => {
                    self.state = table;
                }
                Err(err) => {
                    let filename = self
                        .path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.parse_error = format!("{filename}: {err}");
                    LOG.warn(format_args!(
                        "parse error in {}: {}",
                        self.path.display(),
                        err
                    ));
                    self.state = toml::Table::new();
                }
            },
            Err(err) => {
                LOG.warn(format_args!(
                    "failed to read {}: {}",
                    self.path.display(),
                    err
                ));
            }
        }
    }

    pub fn bool_value(&self, owner: &str, key: &str) -> Option<bool> {
        if !valid_state_identifier(owner) || !valid_state_identifier(key) {
            LOG.warn(format_args!("invalid state key {owner}.{key}"));
            return None;
        }

        let node = match self.state.get(owner) {
            Some(toml::Value::Table(t)) => t.get(key),
            Some(_) => {
                LOG.warn(format_args!("state owner {owner} is not a table"));
                return None;
            }
            None => return None,
        };

        let node = node?;
        let val = node.as_bool();
        if val.is_none() {
            LOG.warn(format_args!("state value {owner}.{key} is not a bool"));
        }
        val
    }

    pub fn string_value(&self, owner: &str, key: &str) -> Option<String> {
        if !valid_state_identifier(owner) || !valid_state_identifier(key) {
            LOG.warn(format_args!("invalid state key {owner}.{key}"));
            return None;
        }

        let node = match self.state.get(owner) {
            Some(toml::Value::Table(t)) => t.get(key),
            Some(_) => {
                LOG.warn(format_args!("state owner {owner} is not a table"));
                return None;
            }
            None => return None,
        };

        let node = node?;
        let val = node.as_str().map(|s| s.to_string());
        if val.is_none() {
            LOG.warn(format_args!("state value {owner}.{key} is not a string"));
        }
        val
    }

    pub fn set_bool(&mut self, owner: &str, key: &str, value: bool) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        if !valid_state_identifier(owner) || !valid_state_identifier(key) {
            LOG.warn(format_args!("invalid state key {owner}.{key}"));
            return false;
        }

        let table = match self.state.entry(owner.to_string()) {
            toml::map::Entry::Occupied(mut entry) => {
                if !entry.get().is_table() {
                    LOG.warn(format_args!(
                        "state owner {owner} is not a table; replacing it"
                    ));
                    entry.insert(toml::Value::Table(toml::Table::new()));
                }
                let Some(t) = entry.into_mut().as_table_mut() else {
                    return false;
                };
                t
            }
            toml::map::Entry::Vacant(entry) => {
                let node = entry.insert(toml::Value::Table(toml::Table::new()));
                let Some(t) = node.as_table_mut() else {
                    return false;
                };
                t
            }
        };

        if let Some(existing) = table.get(key).and_then(|v| v.as_bool())
            && existing == value
        {
            return true;
        }

        table.insert(key.to_string(), toml::Value::Boolean(value));
        if !self.write() {
            LOG.warn(format_args!("failed to write {}", self.path.display()));
            return false;
        }

        self.parse_error.clear();
        true
    }

    pub fn set_string(&mut self, owner: &str, key: &str, value: &str) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        if !valid_state_identifier(owner) || !valid_state_identifier(key) {
            LOG.warn(format_args!("invalid state key {owner}.{key}"));
            return false;
        }

        let table = match self.state.entry(owner.to_string()) {
            toml::map::Entry::Occupied(mut entry) => {
                if !entry.get().is_table() {
                    LOG.warn(format_args!(
                        "state owner {owner} is not a table; replacing it"
                    ));
                    entry.insert(toml::Value::Table(toml::Table::new()));
                }
                let Some(t) = entry.into_mut().as_table_mut() else {
                    return false;
                };
                t
            }
            toml::map::Entry::Vacant(entry) => {
                let node = entry.insert(toml::Value::Table(toml::Table::new()));
                let Some(t) = node.as_table_mut() else {
                    return false;
                };
                t
            }
        };

        if let Some(existing) = table.get(key).and_then(|v| v.as_str())
            && existing == value
        {
            return true;
        }

        table.insert(key.to_string(), toml::Value::String(value.to_string()));
        if !self.write() {
            LOG.warn(format_args!("failed to write {}", self.path.display()));
            return false;
        }

        self.parse_error.clear();
        true
    }

    pub fn clear_owner(&mut self, owner: &str) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        if !valid_state_identifier(owner) {
            LOG.warn(format_args!("invalid state owner {owner}"));
            return false;
        }
        if !self.state.contains_key(owner) {
            return true;
        }

        self.state.remove(owner);
        if !self.write() {
            LOG.warn(format_args!("failed to write {}", self.path.display()));
            return false;
        }

        self.parse_error.clear();
        true
    }

    fn write(&self) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }

        let content = toml::to_string(&self.state).unwrap_or_default();
        write_text_file_atomic(&self.path, &content, Some(0o600)).is_ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "noctalia-state-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn file_mode(path: &Path) -> u32 {
        fs::metadata(path).expect("stat file").permissions().mode() & 0o777
    }

    #[test]
    fn bool_state_round_trips() {
        let dir = temp_dir("bool-rt");
        let path = dir.join("state.toml");

        let mut store = StateStore::new(&path);
        store.load();

        assert_eq!(store.bool_value("wallpaper_panel", "flatten"), None);
        assert!(store.set_bool("wallpaper_panel", "flatten", true));

        let content = fs::read_to_string(&path).expect("read state file");
        assert!(content.contains("[wallpaper_panel]"));
        assert!(content.contains("flatten = true"));

        let mut loaded = StateStore::new(&path);
        loaded.load();
        assert_eq!(loaded.bool_value("wallpaper_panel", "flatten"), Some(true));
        assert!(loaded.set_bool("wallpaper_panel", "flatten", false));

        let mut reloaded = StateStore::new(&path);
        reloaded.load();
        assert_eq!(
            reloaded.bool_value("wallpaper_panel", "flatten"),
            Some(false)
        );
        assert!(!reloaded.set_bool("wallpaper.panel", "flatten", true));
        assert!(reloaded.clear_owner("wallpaper_panel"));
        assert_eq!(reloaded.bool_value("wallpaper_panel", "flatten"), None);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn state_file_is_owner_only_on_create() {
        let dir = temp_dir("create-mode");
        let path = dir.join("state.toml");

        let mut store = StateStore::new(&path);
        store.load();
        assert!(store.set_string("calendar_credentials", "personal_password", "secret"));
        assert_eq!(file_mode(&path), 0o600);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn state_load_tightens_existing_file() {
        let dir = temp_dir("tighten");
        let path = dir.join("state.toml");
        fs::write(
            &path,
            "[calendar_credentials]\npersonal_password = \"secret\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).unwrap();

        let mut store = StateStore::new(&path);
        store.load();

        assert_eq!(file_mode(&path), 0o600);
        assert_eq!(
            store.string_value("calendar_credentials", "personal_password"),
            Some("secret".to_string())
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wrong_type_is_not_read_as_bool() {
        let dir = temp_dir("wrong-type");
        let path = dir.join("state.toml");
        fs::write(&path, "[wallpaper_panel]\nflatten = \"yes\"\n").unwrap();

        let mut store = StateStore::new(&path);
        store.load();

        assert_eq!(store.bool_value("wallpaper_panel", "flatten"), None);
        assert!(store.set_bool("wallpaper_panel", "flatten", true));
        assert_eq!(store.bool_value("wallpaper_panel", "flatten"), Some(true));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn state_store_preserves_symlink() {
        let dir = temp_dir("symlink");
        let target_dir = dir.join("dotfiles");
        fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("state.toml");
        let link = dir.join("state.toml");

        fs::write(&target, "[wallpaper_panel]\nflatten = false\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut store = StateStore::new(&link);
        store.load();

        assert!(store.set_bool("wallpaper_panel", "flatten", true));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("flatten = true"));
        assert_eq!(file_mode(&target), 0o600);

        let _ = fs::remove_dir_all(dir);
    }
}
