//! Port of `src/hooks/hook_manager.{h,cpp}`.
//!
//! `HookKind::Count` (the C++ enum's array-sizing sentinel) has no Rust counterpart — the
//! ported `HookKind` (`noctalia_config::types::hooks::HookKind`) only has the 18 real variants,
//! so `fireWithRunner`'s `kind == HookKind::Count` guard is unreachable by construction here and
//! was dropped rather than translated.

use noctalia_config::types::hooks::{HookKind, HooksConfig};

static LOG: noctalia_core::log::Logger = noctalia_core::log::Logger::new("hooks");

/// Port of `HookManager::EnvVar` (`std::pair<const char*, std::string>`).
pub type EnvVar = (&'static str, String);

/// Port of `HookManager::CommandRunner` (`std::function<bool(const std::string&)>`).
pub type CommandRunner = Box<dyn Fn(&str) -> bool>;

#[derive(Default)]
pub struct HookManager {
    config: HooksConfig,
    runner: Option<CommandRunner>,
    blocking_runner: Option<CommandRunner>,
}

impl HookManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_command_runner(&mut self, runner: CommandRunner) {
        self.runner = Some(runner);
    }

    pub fn set_blocking_command_runner(&mut self, runner: CommandRunner) {
        self.blocking_runner = Some(runner);
    }

    pub fn reload(&mut self, config: HooksConfig) {
        self.config = config;
    }

    pub fn config(&self) -> &HooksConfig {
        &self.config
    }

    pub fn fire(&self, kind: HookKind) {
        if let Some(runner) = &self.runner {
            self.fire_with_runner(kind, runner.as_ref());
        }
    }

    pub fn fire_blocking(&self, kind: HookKind) -> bool {
        match self.blocking_runner.as_deref().or(self.runner.as_deref()) {
            Some(runner) => self.fire_with_runner(kind, runner),
            None => false,
        }
    }

    /// Port of `HookManager::fireWithEnv`: temporarily sets `env` in the process environment
    /// (matching the C++'s raw `setenv`/`unsetenv`, since `CommandRunner` takes only a command
    /// string — the runner reads the target command's environment ambiently, not via an
    /// explicit map), fires `kind`, then unsets each key again.
    ///
    /// SAFETY (of the `unsafe` env calls below): matches the C++'s own hazard exactly — mutating
    /// the process environment races with any other thread reading it concurrently (e.g. a
    /// concurrent `std::process::Command::spawn`). Callers must only invoke this from the single
    /// main-loop thread hooks are architecturally fired from (architecture decision 1); it is
    /// not safe to call from multiple threads concurrently, same as the C++.
    pub fn fire_with_env(&self, kind: HookKind, env: &[EnvVar]) {
        for (key, value) in env {
            // SAFETY: see doc comment above — single-threaded main-loop caller assumed.
            unsafe { std::env::set_var(key, value) };
        }
        self.fire(kind);
        for (key, _) in env {
            // SAFETY: see doc comment above.
            unsafe { std::env::remove_var(key) };
        }
    }

    fn fire_with_runner(&self, kind: HookKind, runner: &dyn Fn(&str) -> bool) -> bool {
        let cmds = self.config.commands(kind);
        if cmds.is_empty() {
            return true;
        }
        let name = kind.key();
        LOG.debug(format_args!(
            "hook '{name}' running {} command(s)",
            cmds.len()
        ));
        let mut ok = true;
        for cmd in cmds {
            if !runner(cmd) {
                LOG.warn(format_args!("hook '{name}' command failed: {cmd}"));
                ok = false;
            }
        }
        ok
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;

    // This crate's only test that mutates process env; no sibling test in this binary races it,
    // but a shared lock is cheap insurance against that changing later (same pattern as
    // noctalia-core's `process::test_support::ENV_MUTATION_LOCK`).
    static ENV_MUTATION_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn fire_with_env_sets_and_unsets_around_the_call_matching_cpp_test() {
        let _guard = ENV_MUTATION_LOCK.lock().expect("lock poisoned");
        const PATH_NAME: &str = "NOCTALIA_WALLPAPER_PATH";
        const CONNECTOR_NAME: &str = "NOCTALIA_WALLPAPER_CONNECTOR";
        // SAFETY: guarded by ENV_MUTATION_LOCK above; single-threaded test.
        unsafe {
            std::env::remove_var(PATH_NAME);
            std::env::remove_var(CONNECTOR_NAME);
        }

        let mut hooks = HookManager::new();
        let config = HooksConfig {
            wallpaper_changed: vec!["record-wallpaper-hook".to_string()],
            ..Default::default()
        };
        hooks.reload(config);

        let commands = Rc::new(RefCell::new(Vec::new()));
        let path_seen = Rc::new(RefCell::new(String::new()));
        let connector_seen = Rc::new(RefCell::new(String::new()));
        let (c, p, conn) = (commands.clone(), path_seen.clone(), connector_seen.clone());
        hooks.set_command_runner(Box::new(move |command| {
            c.borrow_mut().push(command.to_string());
            *p.borrow_mut() = std::env::var(PATH_NAME).unwrap_or_default();
            *conn.borrow_mut() = std::env::var(CONNECTOR_NAME).unwrap_or_default();
            true
        }));

        hooks.fire_with_env(
            HookKind::WallpaperChanged,
            &[
                (PATH_NAME, "/tmp/noctalia test/wallpaper.png".to_string()),
                (CONNECTOR_NAME, "DP-1".to_string()),
            ],
        );

        assert_eq!(
            *commands.borrow(),
            vec!["record-wallpaper-hook".to_string()]
        );
        assert_eq!(*path_seen.borrow(), "/tmp/noctalia test/wallpaper.png");
        assert_eq!(*connector_seen.borrow(), "DP-1");
        assert!(std::env::var(PATH_NAME).is_err());
        assert!(std::env::var(CONNECTOR_NAME).is_err());
    }

    #[test]
    fn fire_with_no_commands_configured_is_a_silent_no_op() {
        let mut hooks = HookManager::new();
        hooks.reload(HooksConfig::default());
        let called = Rc::new(RefCell::new(false));
        let called2 = called.clone();
        hooks.set_command_runner(Box::new(move |_| {
            *called2.borrow_mut() = true;
            true
        }));
        hooks.fire(HookKind::Started);
        assert!(!*called.borrow());
    }

    #[test]
    fn fire_blocking_prefers_blocking_runner_and_reports_failure() {
        let mut hooks = HookManager::new();
        let config = HooksConfig {
            started: vec!["a".to_string(), "b".to_string()],
            ..Default::default()
        };
        hooks.reload(config);
        hooks.set_command_runner(Box::new(|_| true));
        hooks.set_blocking_command_runner(Box::new(|cmd| cmd != "b"));

        assert!(!hooks.fire_blocking(HookKind::Started));
    }

    #[test]
    fn fire_blocking_without_a_blocking_runner_falls_back_to_the_async_one() {
        let mut hooks = HookManager::new();
        let config = HooksConfig {
            started: vec!["a".to_string()],
            ..Default::default()
        };
        hooks.reload(config);
        hooks.set_command_runner(Box::new(|_| true));

        assert!(hooks.fire_blocking(HookKind::Started));
    }

    #[test]
    fn fire_blocking_with_no_runner_at_all_returns_false() {
        let mut hooks = HookManager::new();
        let config = HooksConfig {
            started: vec!["a".to_string()],
            ..Default::default()
        };
        hooks.reload(config);

        assert!(!hooks.fire_blocking(HookKind::Started));
    }
}
