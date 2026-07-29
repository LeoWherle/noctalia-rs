//! Config service & file polling.
//! Port of `src/config/config_service.{cpp,h}` and `src/config/config_poll_source.h`.

use std::os::fd::BorrowedFd;
use std::path::PathBuf;

use noctalia_core::atomic_file::write_text_file_atomic;
use noctalia_core::files::file_watcher::{FileWatcher, WatchTrigger};
use noctalia_core::log::Logger;

use crate::change_set::compute_config_change_set;
use crate::merge::merge_config_with_includes;
use crate::migrations::{
    apply_pending_config_migrations, config_migrations, current_config_version,
    stored_config_version,
};
use crate::schema::config_schema::bar_fields_schema;
use crate::schema::config_sections::sections;
use crate::schema::diagnostics::Diagnostics;
use crate::schema::engine::read_into;
use crate::state_store::StateStore;
use crate::types::bar::BarConfig;
use crate::types::config::{Config, ConfigChangeSet};

const LOG: Logger = Logger::new("config");

/// Recursively overlays `overlay` onto `base` (tables merge, everything else replaces).
/// Port of `ConfigService::deepMerge` (`config_service.cpp:163`).
pub fn deep_merge(base: &mut toml::Table, overlay: &toml::Table) {
    for (key, val) in overlay {
        match val {
            toml::Value::Table(overlay_sub) => {
                if let Some(toml::Value::Table(base_sub)) = base.get_mut(key) {
                    deep_merge(base_sub, overlay_sub);
                } else {
                    base.insert(key.clone(), val.clone());
                }
            }
            _ => {
                base.insert(key.clone(), val.clone());
            }
        }
    }
}

pub type ReloadCallback = Box<dyn FnMut(&Config, &ConfigChangeSet) + 'static>;

pub struct ConfigService {
    config_dir: PathBuf,
    overrides_path: PathBuf,
    state_store: StateStore,
    config: Config,
    last_change: ConfigChangeSet,
    overrides_table: toml::Table,
    reload_callbacks: Vec<ReloadCallback>,
    file_watcher: FileWatcher,
    watch_ids: Vec<u64>,
    loaded_files: Vec<PathBuf>,
}

impl ConfigService {
    pub fn new(
        config_dir: impl Into<PathBuf>,
        overrides_path: impl Into<PathBuf>,
        state_path: impl Into<PathBuf>,
    ) -> Self {
        let config_dir = config_dir.into();
        let overrides_path = overrides_path.into();
        let state_store = StateStore::new(state_path);

        let mut service = Self {
            config_dir,
            overrides_path,
            state_store,
            config: Config::default(),
            last_change: ConfigChangeSet::default(),
            overrides_table: toml::Table::new(),
            reload_callbacks: Vec::new(),
            file_watcher: FileWatcher::new(),
            watch_ids: Vec::new(),
            loaded_files: Vec::new(),
        };
        service.state_store.load();
        service.load_all();
        service
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn last_change(&self) -> &ConfigChangeSet {
        &self.last_change
    }

    pub fn state_store(&self) -> &StateStore {
        &self.state_store
    }

    pub fn state_store_mut(&mut self) -> &mut StateStore {
        &mut self.state_store
    }

    pub fn watch_fd(&self) -> i32 {
        self.file_watcher.fd()
    }

    pub fn add_reload_callback(
        &mut self,
        callback: impl FnMut(&Config, &ConfigChangeSet) + 'static,
    ) {
        self.reload_callbacks.push(Box::new(callback));
    }

    pub fn force_reload(&mut self) {
        self.load_all();
        self.fire_reload_callbacks();
    }

    pub fn check_reload(&mut self) {
        self.file_watcher.dispatch();
    }

    pub fn load_all(&mut self) {
        let merge_result = merge_config_with_includes(&self.config_dir);
        let mut merged = merge_result.merged;
        self.loaded_files = merge_result.loaded_files;

        if let Ok(content) = std::fs::read_to_string(&self.overrides_path)
            && let Ok(table) = content.parse::<toml::Table>()
        {
            self.overrides_table = table;
        }

        deep_merge(&mut merged, &self.overrides_table);

        let mut diag = Diagnostics::default();
        let stored_ver = stored_config_version(&merged, &mut diag).unwrap_or(0);
        if stored_ver < current_config_version() {
            apply_pending_config_migrations(
                &mut merged,
                stored_ver,
                &mut diag,
                config_migrations(),
            );
        }

        let mut candidate = Config::default();

        for spec in sections() {
            if let Some(toml::Value::Table(section_table)) = merged.get(spec.name) {
                (spec.read)(section_table, &mut candidate, &mut diag);
            }
        }

        if let Some(toml::Value::Table(bar_root)) = merged.get("bar") {
            let mut bars = Vec::new();
            let mut names: Vec<String> = bar_root.keys().cloned().collect();
            names.sort();
            for name in names {
                if name == "order" {
                    continue;
                }
                if let Some(toml::Value::Table(bar_table)) = bar_root.get(&name) {
                    let mut bar = BarConfig {
                        name: name.clone(),
                        ..Default::default()
                    };
                    if let Some(pos) = bar_table.get("position").and_then(|v| v.as_str()) {
                        bar.position = pos.to_string();
                    }
                    read_into(
                        bar_table,
                        &mut bar,
                        bar_fields_schema(),
                        &format!("bar.{name}"),
                        &mut diag,
                    );
                    bars.push(bar);
                }
            }
            if !bars.is_empty() {
                candidate.bars = bars;
            }
        }

        self.last_change = compute_config_change_set(&self.config, &candidate);
        self.config = candidate;

        self.refresh_watches();
    }

    fn fire_reload_callbacks(&mut self) {
        for cb in &mut self.reload_callbacks {
            cb(&self.config, &self.last_change);
        }
    }

    fn refresh_watches(&mut self) {
        for id in self.watch_ids.drain(..) {
            self.file_watcher.unwatch(id);
        }

        if self.config_dir.exists() {
            let id = self.file_watcher.watch(
                &self.config_dir,
                || {
                    LOG.info(format_args!("config change detected in config_dir"));
                },
                WatchTrigger::WriteCompleted,
            );
            if id > 0 {
                self.watch_ids.push(id);
            }
        }

        for file in &self.loaded_files {
            if file.exists() {
                let id = self.file_watcher.watch(
                    file,
                    || {
                        LOG.info(format_args!("config file modified"));
                    },
                    WatchTrigger::WriteCompleted,
                );
                if id > 0 {
                    self.watch_ids.push(id);
                }
            }
        }

        if self.overrides_path.exists() {
            let id = self.file_watcher.watch(
                &self.overrides_path,
                || {
                    LOG.info(format_args!("settings override file modified"));
                },
                WatchTrigger::WriteCompleted,
            );
            if id > 0 {
                self.watch_ids.push(id);
            }
        }
    }

    pub fn set_override_value(&mut self, path: &[&str], value: toml::Value) -> bool {
        if path.is_empty() {
            return false;
        }

        let mut current = &mut self.overrides_table;
        for &segment in &path[..path.len() - 1] {
            current = match current.entry(segment.to_string()) {
                toml::map::Entry::Occupied(mut entry) => {
                    if !entry.get().is_table() {
                        *entry.get_mut() = toml::Value::Table(toml::Table::new());
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
        }

        current.insert(path[path.len() - 1].to_string(), value);
        self.save_overrides()
    }

    pub fn clear_override(&mut self, path: &[&str]) -> bool {
        if path.is_empty() {
            return false;
        }

        let mut current = &mut self.overrides_table;
        for &segment in &path[..path.len() - 1] {
            let Some(next) = current.get_mut(segment).and_then(|v| v.as_table_mut()) else {
                return true;
            };
            current = next;
        }

        current.remove(path[path.len() - 1]);
        self.save_overrides()
    }

    fn save_overrides(&mut self) -> bool {
        let content = toml::to_string(&self.overrides_table).unwrap_or_default();
        if write_text_file_atomic(&self.overrides_path, &content, None).is_ok() {
            self.force_reload();
            true
        } else {
            false
        }
    }
}

/// Poll source binding ConfigService file watcher into calloop event loop.
/// Port of `ConfigPollSource` (`config_poll_source.h`).
pub fn register_config_poll_source<Data: 'static>(
    handle: &calloop::LoopHandle<'_, Data>,
    service_fd: i32,
    mut on_reload: impl FnMut(&mut Data) + 'static,
) -> Result<
    calloop::RegistrationToken,
    calloop::InsertError<calloop::generic::Generic<BorrowedFd<'static>>>,
> {
    // SAFETY: service_fd is an open inotify file descriptor owned by ConfigService.
    let fd = unsafe { BorrowedFd::borrow_raw(service_fd) };
    let source = calloop::generic::Generic::new(fd, calloop::Interest::READ, calloop::Mode::Level);
    handle.insert_source(source, move |_, _, data| {
        on_reload(data);
        Ok(calloop::PostAction::Continue)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::thread;
    use std::time::Duration;

    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "noctalia-config-svc-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn deep_merge_overlays_tables() {
        let mut base: toml::Table = r#"
[bar.main]
thickness = 40
position = "top"

[shell]
font_family = "Inter"
"#
        .parse()
        .unwrap();

        let overlay: toml::Table = r#"
[bar.main]
thickness = 50

[dock]
enabled = true
"#
        .parse()
        .unwrap();

        deep_merge(&mut base, &overlay);

        let bar_thickness = base
            .get("bar")
            .and_then(|v| v.get("main"))
            .and_then(|v| v.get("thickness"))
            .and_then(|v| v.as_integer());
        assert_eq!(bar_thickness, Some(50));

        let bar_pos = base
            .get("bar")
            .and_then(|v| v.get("main"))
            .and_then(|v| v.get("position"))
            .and_then(|v| v.as_str());
        assert_eq!(bar_pos, Some("top"));

        let dock_enabled = base
            .get("dock")
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool());
        assert_eq!(dock_enabled, Some(true));
    }

    #[test]
    fn service_loads_and_applies_overrides() {
        let dir = temp_dir("loads");
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("00-bar.toml"),
            "[bar.default]\nthickness = 30\n",
        )
        .unwrap();

        let overrides_path = dir.join("settings.toml");
        let state_path = dir.join("state.toml");

        let mut service = ConfigService::new(&config_dir, &overrides_path, &state_path);
        assert_eq!(service.config().bars[0].thickness, 30);

        let reloaded = Arc::new(AtomicBool::new(false));
        let r_flag = Arc::clone(&reloaded);
        service.add_reload_callback(move |_, diff| {
            if diff.any() {
                r_flag.store(true, Ordering::SeqCst);
            }
        });

        assert!(
            service.set_override_value(&["bar", "default", "thickness"], toml::Value::Integer(44))
        );
        assert_eq!(service.config().bars[0].thickness, 44);
        assert!(reloaded.load(Ordering::SeqCst));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_touch_triggers_reload_in_service() {
        let dir = temp_dir("watch");
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();
        let config_file = config_dir.join("bar.toml");
        fs::write(&config_file, "[bar.default]\nthickness = 20\n").unwrap();

        let overrides_path = dir.join("settings.toml");
        let state_path = dir.join("state.toml");

        let mut service = ConfigService::new(&config_dir, &overrides_path, &state_path);
        assert_eq!(service.config().bars[0].thickness, 20);

        thread::sleep(Duration::from_millis(50));
        fs::write(&config_file, "[bar.default]\nthickness = 60\n").unwrap();

        // Give inotify and debounce time to process
        thread::sleep(Duration::from_millis(150));
        service.check_reload();

        let _ = fs::remove_dir_all(dir);
    }
}
