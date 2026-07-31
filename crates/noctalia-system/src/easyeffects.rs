//! Port of `src/system/easyeffects_service.{cpp,h}` (task 5.6.4): EasyEffects preset discovery
//! and control over its own Unix-domain socket protocol (`AF_UNIX`, not D-Bus), plus
//! `ipc::service` registration for the `noctalia msg effects-profile-set` verb.
//!
//! No C++ test exists for this file (confirmed: no `easyeffects_service_test.cpp` in `tests/`) —
//! task 5.6's own done bar is fixture-driven tests for the reachable protocol-parsing/profile-list
//! logic; the live socket round-trip is a manual check (same precedent as task 1.6.5's systemd
//! check, task 5.3.2's `ddcutil` check, and task 5.6.2's `/dev/rfkill` write — this dev host has no
//! `easyeffects` process running, so `discoverEffectsProfiles`/`exchangeEasyEffectsServerCommand`'s
//! real-socket paths are not exercised live this session either).
//!
//! `discoverEffectsProfiles`/`discoverActiveEffectsProfile` are split into a pure
//! `*_from_roots` core (already-computed data/config roots and a `running` flag in, `Vec<String>`/
//! `String` out) so they're fixture-testable without mutating real process env vars — same
//! pure-core-extraction precedent as task 5.6.3's `sample_from_clients`. The env-var-reading root
//! computation (`easyeffects_config_roots`/`easyeffects_data_roots`) and the live-socket
//! `exchange_easy_effects_server_command` are ported faithfully but smoke-tested only.
//!
//! `exchangeEasyEffectsServerCommand`'s raw `socket()`/`connect()`/`send()`/`recv()`/`poll()`
//! syscalls are replaced with `std::os::unix::net::UnixStream` (`connect`/`write_all`/
//! `set_read_timeout`/`read`), same divergence precedent as `noctalia_ipc::service`'s own socket
//! handling: a 250ms `SO_RCVTIMEO` read timeout that resets on every byte received is behaviorally
//! equivalent to the C++'s `poll(..., 250)` loop that also resets on every readable wakeup, and
//! `write_all`'s automatic `EINTR` retry matches the C++'s explicit `errno == EINTR` retry loop.
//! The C++'s explicit `nativePath.size() >= sizeof(sockaddr_un::sun_path)` pre-check is dropped in
//! favor of letting `UnixStream::connect` report its own "path too long" error — both paths return
//! `None`/`nullopt`, so the outcome is identical, only the diagnostic detail differs.
//!
//! `EasyEffectsService::registerIpc` captures `this` in a `[this]`-style lambda kept alive by the
//! `IpcService`'s handler registry outliving the stack frame that registered it. Rust's
//! `noctalia_ipc::Handler` requires `'static`, so this is the first port in the migration to need
//! shared ownership across a callback boundary: `register_ipc` takes `self` by value, wraps it in
//! `Rc<RefCell<Self>>`, and hands the caller back that shared handle (used to keep driving
//! `refresh_profiles`/`refresh_active_effects_profiles` from a timer once app assembly exists) —
//! every other method on the type stays a plain `&mut self`/`&self` call, so this wrapping is
//! opt-in only for the one call site that genuinely needs `'static` shared ownership.
//!
//! The C++ signature carries an always-unused `const ConfigService&` parameter (shared with sibling
//! services' `registerIpc` signatures, e.g. `PipeWireService`/`ScreenshotService`, which *do* use
//! theirs). Since nothing here reads it and no Rust caller exists yet to constrain the signature,
//! it's dropped rather than carried as permanently-dead API surface; the app-assembly phase can add
//! it back if a real need shows up.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use std::{env, fs};

use noctalia_core::log::Logger;
use noctalia_ipc::{HandlerOptions, IpcService};

use crate::brightness::c_trim;

const LOG: Logger = Logger::new("easyeffects");

/// Port of `AudioEffectsProfileKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEffectsProfileKind {
    Output,
    Input,
}

pub type ChangeCallback = Box<dyn Fn()>;
pub type EffectsProfileFeedbackCallback = Box<dyn Fn(AudioEffectsProfileKind, &str)>;

/// Port of `EasyEffectsService`.
#[derive(Default)]
pub struct EasyEffectsService {
    output_effects_profiles: Vec<String>,
    input_effects_profiles: Vec<String>,
    active_output_effects_profile: Option<String>,
    active_input_effects_profile: Option<String>,
    change_callback: Option<ChangeCallback>,
}

impl EasyEffectsService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_change_callback(&mut self, callback: ChangeCallback) {
        self.change_callback = Some(callback);
    }

    #[must_use]
    pub fn effects_profiles(&self, kind: AudioEffectsProfileKind) -> &[String] {
        match kind {
            AudioEffectsProfileKind::Input => &self.input_effects_profiles,
            AudioEffectsProfileKind::Output => &self.output_effects_profiles,
        }
    }

    #[must_use]
    pub fn active_effects_profile(&self, kind: AudioEffectsProfileKind) -> String {
        let active = match kind {
            AudioEffectsProfileKind::Input => &self.active_input_effects_profile,
            AudioEffectsProfileKind::Output => &self.active_output_effects_profile,
        };
        active.clone().unwrap_or_default()
    }

    fn emit_changed(&self) {
        if let Some(callback) = &self.change_callback {
            callback();
        }
    }

    pub fn refresh_profiles(&mut self) {
        let next_output = discover_effects_profiles(AudioEffectsProfileKind::Output);
        let next_input = discover_effects_profiles(AudioEffectsProfileKind::Input);
        if next_output == self.output_effects_profiles && next_input == self.input_effects_profiles
        {
            return;
        }
        self.output_effects_profiles = next_output;
        self.input_effects_profiles = next_input;
        self.emit_changed();
    }

    pub fn refresh_active_effects_profiles(&mut self) {
        let mut changed = false;
        for kind in [
            AudioEffectsProfileKind::Output,
            AudioEffectsProfileKind::Input,
        ] {
            let active = match query_easy_effects_active_preset(kind) {
                Some(active) => active,
                None => {
                    let discovered = discover_active_effects_profile(kind);
                    if discovered.is_empty() {
                        continue;
                    }
                    discovered
                }
            };

            let cached = match kind {
                AudioEffectsProfileKind::Input => &mut self.active_input_effects_profile,
                AudioEffectsProfileKind::Output => &mut self.active_output_effects_profile,
            };
            if cached.as_deref() != Some(active.as_str()) {
                *cached = Some(active);
                changed = true;
            }
        }
        if changed {
            self.emit_changed();
        }
    }

    pub fn load_effects_profile(&mut self, kind: AudioEffectsProfileKind, profile: &str) -> bool {
        let trimmed = c_trim(profile);
        if trimmed.is_empty() {
            return false;
        }
        if !is_protocol_safe_preset_name(trimmed) {
            LOG.warn(format_args!(
                "refusing to load EasyEffects {} profile with protocol-unsafe name \"{trimmed}\"",
                effects_profile_kind_name(kind)
            ));
            return false;
        }

        if !send_easy_effects_server_command(&easy_effects_load_preset_command(kind, trimmed)) {
            LOG.warn(format_args!(
                "failed to load EasyEffects {} profile \"{trimmed}\" - local server unavailable",
                effects_profile_kind_name(kind)
            ));
            return false;
        }

        let Some(active_preset) = query_easy_effects_active_preset(kind) else {
            LOG.warn(format_args!(
                "failed to confirm EasyEffects {} profile \"{trimmed}\" after load command",
                effects_profile_kind_name(kind)
            ));
            return false;
        };
        if active_preset != trimmed {
            LOG.warn(format_args!(
                "EasyEffects {} profile load mismatch requested=\"{trimmed}\" active=\"{active_preset}\"",
                effects_profile_kind_name(kind)
            ));
            return false;
        }

        let cached = match kind {
            AudioEffectsProfileKind::Input => &mut self.active_input_effects_profile,
            AudioEffectsProfileKind::Output => &mut self.active_output_effects_profile,
        };
        let changed = cached.as_deref() != Some(active_preset.as_str());
        *cached = Some(active_preset);
        if changed {
            self.emit_changed();
        }
        true
    }

    /// Port of `registerIpc`. Consumes `self`, wraps it in a shared handle so the registered
    /// handler (kept `'static` by the `IpcService` registry) can mutate it, and hands that handle
    /// back to the caller. See the module doc comment for why this is the one method that needs
    /// shared ownership.
    pub fn register_ipc(
        self,
        ipc: &mut IpcService,
        effects_profile_feedback: Option<EffectsProfileFeedbackCallback>,
    ) -> Rc<RefCell<Self>> {
        let shared = Rc::new(RefCell::new(self));
        let handler_state = Rc::clone(&shared);
        ipc.register_handler(
            "effects-profile-set",
            Box::new(move |args| {
                let Some(split) = args.find(' ') else {
                    return "error: effects-profile-set requires <output|input> <profile>\n"
                        .to_string();
                };
                let kind_arg = c_trim(&args[..split]);
                let Some(kind) = parse_effects_profile_kind(kind_arg) else {
                    return "error: effects-profile-set requires <output|input> <profile>\n"
                        .to_string();
                };
                let profile = c_trim(&args[split..]).to_string();
                if profile.is_empty() {
                    return "error: profile required\n".to_string();
                }

                let mut service = handler_state.borrow_mut();
                service.refresh_profiles();
                let profiles = service.effects_profiles(kind).to_vec();
                if profiles.is_empty() {
                    return format!(
                        "error: no EasyEffects {} profiles found\n",
                        effects_profile_kind_name(kind)
                    );
                }
                if !profiles.contains(&profile) {
                    return format!(
                        "error: unknown EasyEffects {} profile \"{}\"{}",
                        effects_profile_kind_name(kind),
                        profile,
                        available_effects_profiles_suffix(&profiles)
                    );
                }
                if !service.load_effects_profile(kind, &profile) {
                    return format!(
                        "error: failed to set EasyEffects {} profile (is EasyEffects running?)\n",
                        effects_profile_kind_name(kind)
                    );
                }
                drop(service);
                if let Some(feedback) = &effects_profile_feedback {
                    feedback(kind, &profile);
                }
                "ok\n".to_string()
            }),
            "<output|input> <profile>",
            "Set the EasyEffects output or input profile",
            HandlerOptions::default(),
        );
        shared
    }
}

fn is_protocol_safe_preset_name(name: &str) -> bool {
    !name.contains(':') && !name.contains('\r') && !name.contains('\n')
}

fn push_unique_profile(profiles: &mut Vec<String>, seen: &mut HashSet<String>, name: String) {
    let name = c_trim(&name).to_string();
    if name.is_empty() || !is_protocol_safe_preset_name(&name) || !seen.insert(name.clone()) {
        return;
    }
    profiles.push(name);
}

fn preset_name_from_usage_entry(entry: &str) -> String {
    let entry = match entry.find(':') {
        Some(colon) => &entry[..colon],
        None => entry,
    };
    c_trim(entry).to_string()
}

fn append_comma_separated_preset_names(
    profiles: &mut Vec<String>,
    seen: &mut HashSet<String>,
    value: &str,
    strip_usage_count: bool,
) {
    let mut start = 0usize;
    while start <= value.len() {
        let end = value[start..].find(',').map(|i| start + i);
        let item = &value[start..end.unwrap_or(value.len())];
        let name = if strip_usage_count {
            preset_name_from_usage_entry(item)
        } else {
            c_trim(item).to_string()
        };
        push_unique_profile(profiles, seen, name);
        match end {
            Some(e) => start = e + 1,
            None => break,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct EasyEffectsPresetConfig {
    output_profiles: Vec<String>,
    input_profiles: Vec<String>,
    last_loaded_output_preset: String,
    last_loaded_input_preset: String,
}

/// Port of `readEasyEffectsPresetConfig`. See the module doc comment on the crate's established
/// `fs::read_to_string`-fails-whole-file-on-invalid-UTF-8 divergence from the C++'s byte-wise
/// `ifstream`/`getline` reading (same precedent as `distro_info::parse_os_release`); EasyEffects's
/// own config file is ASCII/UTF-8 in practice.
fn read_easy_effects_preset_config(file: &Path) -> EasyEffectsPresetConfig {
    let mut result = EasyEffectsPresetConfig::default();
    let Ok(content) = fs::read_to_string(file) else {
        return result;
    };

    let mut seen_outputs = HashSet::new();
    let mut seen_inputs = HashSet::new();
    let mut section = String::new();

    for line in content.lines() {
        let line = c_trim(line);
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.len() >= 2 && line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }

        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = c_trim(&line[..eq]);
        let value = c_trim(&line[eq + 1..]);

        if key == "lastLoadedOutputPreset" {
            result.last_loaded_output_preset = value.to_string();
            push_unique_profile(
                &mut result.output_profiles,
                &mut seen_outputs,
                value.to_string(),
            );
        } else if key == "lastLoadedInputPreset" {
            result.last_loaded_input_preset = value.to_string();
            push_unique_profile(
                &mut result.input_profiles,
                &mut seen_inputs,
                value.to_string(),
            );
        } else if section == "StreamOutputs" {
            if key == "usedPresets" {
                append_comma_separated_preset_names(
                    &mut result.output_profiles,
                    &mut seen_outputs,
                    value,
                    true,
                );
            } else if key == "mostUsedPresets" {
                append_comma_separated_preset_names(
                    &mut result.output_profiles,
                    &mut seen_outputs,
                    value,
                    false,
                );
            }
        } else if section == "StreamInputs" {
            if key == "usedPresets" {
                append_comma_separated_preset_names(
                    &mut result.input_profiles,
                    &mut seen_inputs,
                    value,
                    true,
                );
            } else if key == "mostUsedPresets" {
                append_comma_separated_preset_names(
                    &mut result.input_profiles,
                    &mut seen_inputs,
                    value,
                    false,
                );
            }
        }
    }

    result
}

/// Port of `appendPresetFilesFromDir`. Uses `fs::metadata` (follows symlinks) rather than
/// `DirEntry::file_type`/`metadata` (which do not) to match `directory_entry::is_regular_file`'s
/// symlink-following C++ behavior.
fn append_preset_files_from_dir(
    profiles: &mut Vec<String>,
    seen: &mut HashSet<String>,
    dir: &Path,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            push_unique_profile(profiles, seen, stem.to_string());
        }
    }
}

fn home_path() -> Option<PathBuf> {
    env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn append_colon_separated_roots(roots: &mut Vec<PathBuf>, dirs: &str, app_dir: &str) {
    let mut start = 0usize;
    while start <= dirs.len() {
        let end = dirs[start..].find(':').map(|i| start + i);
        let item = &dirs[start..end.unwrap_or(dirs.len())];
        if !item.is_empty() {
            roots.push(Path::new(item).join(app_dir));
        }
        match end {
            Some(e) => start = e + 1,
            None => break,
        }
    }
}

fn easyeffects_config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    match env::var("XDG_CONFIG_HOME").ok().filter(|v| !v.is_empty()) {
        Some(xdg_config_home) => roots.push(Path::new(&xdg_config_home).join("easyeffects")),
        None => {
            if let Some(home) = home_path() {
                roots.push(home.join(".config/easyeffects"));
            }
        }
    }

    let dirs = env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/etc/xdg".to_string());
    append_colon_separated_roots(&mut roots, &dirs, "easyeffects");

    if let Some(home) = home_path() {
        roots.push(home.join(".var/app/com.github.wwmm.easyeffects/config/easyeffects"));
    }

    roots
}

fn easyeffects_data_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    match env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()) {
        Some(xdg_data_home) => roots.push(Path::new(&xdg_data_home).join("easyeffects")),
        None => {
            if let Some(home) = home_path() {
                roots.push(home.join(".local/share/easyeffects"));
            }
        }
    }

    let dirs = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    append_colon_separated_roots(&mut roots, &dirs, "easyeffects");

    if let Some(home) = home_path() {
        roots.push(home.join(".var/app/com.github.wwmm.easyeffects/data/easyeffects"));
    }

    roots
}

fn parse_effects_profile_kind(value: &str) -> Option<AudioEffectsProfileKind> {
    match c_trim(value) {
        "output" | "out" | "sink" => Some(AudioEffectsProfileKind::Output),
        "input" | "in" | "source" | "mic" => Some(AudioEffectsProfileKind::Input),
        _ => None,
    }
}

fn effects_profile_kind_name(kind: AudioEffectsProfileKind) -> &'static str {
    match kind {
        AudioEffectsProfileKind::Input => "input",
        AudioEffectsProfileKind::Output => "output",
    }
}

fn available_effects_profiles_suffix(profiles: &[String]) -> String {
    if profiles.is_empty() {
        return "\n".to_string();
    }
    let mut suffix = String::from("; available:");
    for profile in profiles {
        suffix.push(' ');
        suffix.push_str(profile);
    }
    suffix.push('\n');
    suffix
}

fn easy_effects_server_path() -> PathBuf {
    if let Some(runtime_dir) = env::var("XDG_RUNTIME_DIR").ok().filter(|v| !v.is_empty()) {
        return Path::new(&runtime_dir).join("EasyEffectsServer");
    }
    // SAFETY: getuid() takes no arguments and cannot fail.
    let uid = unsafe { libc::getuid() };
    Path::new("/run/user")
        .join(uid.to_string())
        .join("EasyEffectsServer")
}

fn easyeffects_running() -> bool {
    easy_effects_server_path().exists()
}

/// Pure core of `discoverEffectsProfiles`: takes already-computed roots and a `running` flag so it
/// can be fixture-tested without mutating real process env vars.
fn discover_effects_profiles_from_roots(
    kind: AudioEffectsProfileKind,
    running: bool,
    data_roots: &[PathBuf],
    config_roots: &[PathBuf],
) -> Vec<String> {
    let mut profiles = Vec::new();
    let mut seen = HashSet::new();

    if !running {
        return profiles;
    }

    let profile_dir = match kind {
        AudioEffectsProfileKind::Input => "input",
        AudioEffectsProfileKind::Output => "output",
    };
    for root in data_roots {
        append_preset_files_from_dir(&mut profiles, &mut seen, &root.join(profile_dir));
    }

    let found_data_profiles = !profiles.is_empty();
    for root in config_roots {
        if !found_data_profiles {
            append_preset_files_from_dir(&mut profiles, &mut seen, &root.join(profile_dir));
            let config = read_easy_effects_preset_config(&root.join("db/easyeffectsrc"));
            let discovered = match kind {
                AudioEffectsProfileKind::Input => &config.input_profiles,
                AudioEffectsProfileKind::Output => &config.output_profiles,
            };
            for profile in discovered {
                push_unique_profile(&mut profiles, &mut seen, profile.clone());
            }
        }
    }

    profiles.sort_by_key(|a| a.to_ascii_lowercase());
    profiles
}

fn discover_effects_profiles(kind: AudioEffectsProfileKind) -> Vec<String> {
    discover_effects_profiles_from_roots(
        kind,
        easyeffects_running(),
        &easyeffects_data_roots(),
        &easyeffects_config_roots(),
    )
}

/// Pure core of `discoverActiveEffectsProfile`, same fixture-testability rationale as
/// `discover_effects_profiles_from_roots`.
fn discover_active_effects_profile_from_roots(
    kind: AudioEffectsProfileKind,
    running: bool,
    config_roots: &[PathBuf],
) -> String {
    if !running {
        return String::new();
    }
    for root in config_roots {
        let config = read_easy_effects_preset_config(&root.join("db/easyeffectsrc"));
        match kind {
            AudioEffectsProfileKind::Output if !config.last_loaded_output_preset.is_empty() => {
                return config.last_loaded_output_preset;
            }
            AudioEffectsProfileKind::Input if !config.last_loaded_input_preset.is_empty() => {
                return config.last_loaded_input_preset;
            }
            _ => {}
        }
    }
    String::new()
}

fn discover_active_effects_profile(kind: AudioEffectsProfileKind) -> String {
    discover_active_effects_profile_from_roots(
        kind,
        easyeffects_running(),
        &easyeffects_config_roots(),
    )
}

/// Port of `exchangeEasyEffectsServerCommand`. See the module doc comment for the syscall-level
/// divergences from the C++'s raw `socket`/`connect`/`send`/`recv`/`poll`.
fn exchange_easy_effects_server_command(command: &str, read_response: bool) -> Option<String> {
    let path = easy_effects_server_path();
    let mut stream = UnixStream::connect(&path).ok()?;

    let mut payload = command.to_string();
    payload.push('\n');
    stream.write_all(payload.as_bytes()).ok()?;

    if !read_response {
        return Some(String::new());
    }

    let _ = stream.shutdown(std::net::Shutdown::Write);
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .ok()?;

    let mut response = Vec::new();
    let mut received = false;
    let mut buf = [0u8; 512];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                received = true;
                response.extend_from_slice(&buf[..n]);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    if !received {
        return None;
    }
    Some(String::from_utf8_lossy(&response).into_owned())
}

fn send_easy_effects_server_command(command: &str) -> bool {
    exchange_easy_effects_server_command(command, false).is_some()
}

fn easy_effects_load_preset_command(kind: AudioEffectsProfileKind, profile: &str) -> String {
    format!("load_preset:{}:{profile}", effects_profile_kind_name(kind))
}

fn easy_effects_get_last_loaded_preset_command(kind: AudioEffectsProfileKind) -> String {
    format!("get_last_loaded_preset:{}", effects_profile_kind_name(kind))
}

fn query_easy_effects_active_preset(kind: AudioEffectsProfileKind) -> Option<String> {
    let response = exchange_easy_effects_server_command(
        &easy_effects_get_last_loaded_preset_command(kind),
        true,
    )?;
    Some(c_trim(&response).to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-easyeffects-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn is_protocol_safe_preset_name_rejects_colon_and_line_breaks() {
        assert!(is_protocol_safe_preset_name("My Preset"));
        assert!(!is_protocol_safe_preset_name("evil:payload"));
        assert!(!is_protocol_safe_preset_name("line\rbreak"));
        assert!(!is_protocol_safe_preset_name("line\nbreak"));
    }

    #[test]
    fn push_unique_profile_trims_dedupes_and_rejects_unsafe_names() {
        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        push_unique_profile(&mut profiles, &mut seen, "  Movie  ".to_string());
        push_unique_profile(&mut profiles, &mut seen, "Movie".to_string());
        push_unique_profile(&mut profiles, &mut seen, "unsafe:name".to_string());
        push_unique_profile(&mut profiles, &mut seen, "   ".to_string());
        push_unique_profile(&mut profiles, &mut seen, "Music".to_string());
        assert_eq!(profiles, vec!["Movie".to_string(), "Music".to_string()]);
    }

    #[test]
    fn preset_name_from_usage_entry_strips_usage_count_suffix() {
        assert_eq!(preset_name_from_usage_entry("Movie:12"), "Movie");
        assert_eq!(preset_name_from_usage_entry(" Music : 3 "), "Music");
        assert_eq!(preset_name_from_usage_entry("NoCount"), "NoCount");
    }

    #[test]
    fn append_comma_separated_preset_names_handles_usage_counts_and_empty_items() {
        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        append_comma_separated_preset_names(
            &mut profiles,
            &mut seen,
            "Movie:5,Music:1,,Movie:5",
            true,
        );
        assert_eq!(profiles, vec!["Movie".to_string(), "Music".to_string()]);

        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        append_comma_separated_preset_names(&mut profiles, &mut seen, "", true);
        assert!(profiles.is_empty());

        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        append_comma_separated_preset_names(&mut profiles, &mut seen, "Movie, Music", false);
        assert_eq!(profiles, vec!["Movie".to_string(), "Music".to_string()]);
    }

    #[test]
    fn append_colon_separated_roots_joins_app_dir_and_skips_empty_items() {
        let mut roots = Vec::new();
        append_colon_separated_roots(&mut roots, "/etc/xdg::/usr/local/etc/xdg", "easyeffects");
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/etc/xdg/easyeffects"),
                PathBuf::from("/usr/local/etc/xdg/easyeffects"),
            ]
        );
    }

    #[test]
    fn parse_effects_profile_kind_accepts_all_documented_aliases() {
        for alias in ["output", "out", "sink", "  output  "] {
            assert_eq!(
                parse_effects_profile_kind(alias),
                Some(AudioEffectsProfileKind::Output)
            );
        }
        for alias in ["input", "in", "source", "mic"] {
            assert_eq!(
                parse_effects_profile_kind(alias),
                Some(AudioEffectsProfileKind::Input)
            );
        }
        assert_eq!(parse_effects_profile_kind("nonsense"), None);
    }

    #[test]
    fn available_effects_profiles_suffix_lists_every_profile_or_just_a_newline() {
        assert_eq!(available_effects_profiles_suffix(&[]), "\n");
        assert_eq!(
            available_effects_profiles_suffix(&["Movie".to_string(), "Music".to_string()]),
            "; available: Movie Music\n"
        );
    }

    #[test]
    fn easy_effects_command_strings_match_the_wire_protocol() {
        assert_eq!(
            easy_effects_load_preset_command(AudioEffectsProfileKind::Output, "Movie"),
            "load_preset:output:Movie"
        );
        assert_eq!(
            easy_effects_get_last_loaded_preset_command(AudioEffectsProfileKind::Input),
            "get_last_loaded_preset:input"
        );
    }

    #[test]
    fn read_easy_effects_preset_config_parses_sections_and_last_loaded_keys() {
        let dir = tempfile_dir();
        let file = dir.join("easyeffectsrc");
        let mut f = fs::File::create(&file).expect("create config file");
        writeln!(
            f,
            "lastLoadedOutputPreset=Movie\n\
             lastLoadedInputPreset=Podcast\n\
             \n\
             [StreamOutputs]\n\
             usedPresets=Movie:5,Music:2\n\
             mostUsedPresets=Movie,Game\n\
             \n\
             [StreamInputs]\n\
             usedPresets=Podcast:9\n\
             # a comment line\n\
             ; another comment style"
        )
        .expect("write config file");

        let config = read_easy_effects_preset_config(&file);
        assert_eq!(config.last_loaded_output_preset, "Movie");
        assert_eq!(config.last_loaded_input_preset, "Podcast");
        assert_eq!(
            config.output_profiles,
            vec!["Movie".to_string(), "Music".to_string(), "Game".to_string(),]
        );
        assert_eq!(config.input_profiles, vec!["Podcast".to_string()]);
    }

    #[test]
    fn read_easy_effects_preset_config_on_missing_file_is_empty_default() {
        let dir = tempfile_dir();
        let config = read_easy_effects_preset_config(&dir.join("does-not-exist"));
        assert_eq!(config.output_profiles, Vec::<String>::new());
        assert_eq!(config.input_profiles, Vec::<String>::new());
        assert!(config.last_loaded_output_preset.is_empty());
        assert!(config.last_loaded_input_preset.is_empty());
    }

    #[test]
    fn append_preset_files_from_dir_finds_only_json_regular_files() {
        let dir = tempfile_dir();
        fs::write(dir.join("Movie.json"), "{}").expect("write preset");
        fs::write(dir.join("Music.json"), "{}").expect("write preset");
        fs::write(dir.join("notes.txt"), "ignored").expect("write non-json file");
        fs::create_dir_all(dir.join("Nested.json")).expect("create dir with json-like name");

        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        append_preset_files_from_dir(&mut profiles, &mut seen, &dir);
        profiles.sort();
        assert_eq!(profiles, vec!["Movie".to_string(), "Music".to_string()]);
    }

    #[test]
    fn append_preset_files_from_dir_on_missing_dir_is_a_no_op() {
        let dir = tempfile_dir();
        let mut profiles = Vec::new();
        let mut seen = HashSet::new();
        append_preset_files_from_dir(&mut profiles, &mut seen, &dir.join("does-not-exist"));
        assert!(profiles.is_empty());
    }

    #[test]
    fn discover_effects_profiles_from_roots_returns_empty_when_not_running() {
        let profiles = discover_effects_profiles_from_roots(
            AudioEffectsProfileKind::Output,
            false,
            &[PathBuf::from("/nonexistent")],
            &[PathBuf::from("/nonexistent")],
        );
        assert!(profiles.is_empty());
    }

    #[test]
    fn discover_effects_profiles_from_roots_prefers_data_roots_over_config_roots() {
        let dir = tempfile_dir();
        let data_root = dir.join("data");
        let config_root = dir.join("config");
        fs::create_dir_all(data_root.join("output")).expect("create data output dir");
        fs::create_dir_all(config_root.join("output")).expect("create config output dir");
        fs::write(data_root.join("output/FromData.json"), "{}").expect("write data preset");
        fs::write(config_root.join("output/FromConfig.json"), "{}").expect("write config preset");

        let profiles = discover_effects_profiles_from_roots(
            AudioEffectsProfileKind::Output,
            true,
            &[data_root],
            &[config_root],
        );
        assert_eq!(profiles, vec!["FromData".to_string()]);
    }

    #[test]
    fn discover_effects_profiles_from_roots_falls_back_to_config_root_when_data_root_is_empty() {
        let dir = tempfile_dir();
        let data_root = dir.join("data");
        let config_root = dir.join("config");
        fs::create_dir_all(data_root.join("output")).expect("create empty data output dir");
        fs::create_dir_all(config_root.join("output")).expect("create config output dir");
        fs::write(config_root.join("output/FromConfig.json"), "{}").expect("write config preset");
        fs::create_dir_all(config_root.join("db")).expect("create db dir");
        fs::write(
            config_root.join("db/easyeffectsrc"),
            "[StreamOutputs]\nmostUsedPresets=FromRc\n",
        )
        .expect("write easyeffectsrc");

        let mut profiles = discover_effects_profiles_from_roots(
            AudioEffectsProfileKind::Output,
            true,
            &[data_root],
            &[config_root],
        );
        profiles.sort();
        assert_eq!(
            profiles,
            vec!["FromConfig".to_string(), "FromRc".to_string()]
        );
    }

    #[test]
    fn discover_effects_profiles_from_roots_sorts_case_insensitively() {
        let dir = tempfile_dir();
        let data_root = dir.join("data");
        fs::create_dir_all(data_root.join("output")).expect("create data output dir");
        fs::write(data_root.join("output/banana.json"), "{}").expect("write preset");
        fs::write(data_root.join("output/Apple.json"), "{}").expect("write preset");
        fs::write(data_root.join("output/cherry.json"), "{}").expect("write preset");

        let profiles = discover_effects_profiles_from_roots(
            AudioEffectsProfileKind::Output,
            true,
            &[data_root],
            &[],
        );
        assert_eq!(
            profiles,
            vec![
                "Apple".to_string(),
                "banana".to_string(),
                "cherry".to_string()
            ]
        );
    }

    #[test]
    fn discover_active_effects_profile_from_roots_returns_empty_when_not_running() {
        let active = discover_active_effects_profile_from_roots(
            AudioEffectsProfileKind::Output,
            false,
            &[PathBuf::from("/nonexistent")],
        );
        assert!(active.is_empty());
    }

    #[test]
    fn discover_active_effects_profile_from_roots_reads_the_matching_kind_only() {
        let dir = tempfile_dir();
        let config_root = dir.join("config");
        fs::create_dir_all(config_root.join("db")).expect("create db dir");
        fs::write(
            config_root.join("db/easyeffectsrc"),
            "lastLoadedOutputPreset=Movie\nlastLoadedInputPreset=Podcast\n",
        )
        .expect("write easyeffectsrc");

        assert_eq!(
            discover_active_effects_profile_from_roots(
                AudioEffectsProfileKind::Output,
                true,
                std::slice::from_ref(&config_root),
            ),
            "Movie"
        );
        assert_eq!(
            discover_active_effects_profile_from_roots(
                AudioEffectsProfileKind::Input,
                true,
                &[config_root],
            ),
            "Podcast"
        );
    }

    #[test]
    fn discover_active_effects_profile_from_roots_falls_through_roots_with_no_match() {
        let dir = tempfile_dir();
        let empty_root = dir.join("empty");
        let real_root = dir.join("real");
        fs::create_dir_all(empty_root.join("db")).expect("create empty db dir");
        fs::create_dir_all(real_root.join("db")).expect("create real db dir");
        fs::write(
            real_root.join("db/easyeffectsrc"),
            "lastLoadedOutputPreset=Movie\n",
        )
        .expect("write easyeffectsrc");

        assert_eq!(
            discover_active_effects_profile_from_roots(
                AudioEffectsProfileKind::Output,
                true,
                &[empty_root, real_root],
            ),
            "Movie"
        );
    }

    #[test]
    fn easyeffects_service_refresh_and_load_are_no_ops_without_a_running_server() {
        // With no `easyeffects` server socket present, discovery finds nothing and load fails
        // cleanly rather than panicking or blocking.
        let mut service = EasyEffectsService::new();
        service.refresh_profiles();
        assert!(
            service
                .effects_profiles(AudioEffectsProfileKind::Output)
                .is_empty()
        );
        assert!(!service.load_effects_profile(AudioEffectsProfileKind::Output, "Movie"));
        assert_eq!(
            service.active_effects_profile(AudioEffectsProfileKind::Output),
            ""
        );
    }

    #[test]
    fn load_effects_profile_rejects_empty_and_protocol_unsafe_names() {
        let mut service = EasyEffectsService::new();
        assert!(!service.load_effects_profile(AudioEffectsProfileKind::Output, "   "));
        assert!(!service.load_effects_profile(AudioEffectsProfileKind::Output, "evil:name"));
    }

    #[test]
    fn register_ipc_effects_profile_set_rejects_malformed_args_without_touching_the_network() {
        let mut ipc = IpcService::new();
        let service = EasyEffectsService::new();
        service.register_ipc(&mut ipc, None);

        assert_eq!(
            ipc.execute("effects-profile-set"),
            "error: effects-profile-set requires <output|input> <profile>\n"
        );
        assert_eq!(
            ipc.execute("effects-profile-set bogus-kind Movie"),
            "error: effects-profile-set requires <output|input> <profile>\n"
        );
        assert_eq!(
            ipc.execute("effects-profile-set output   "),
            "error: profile required\n"
        );

        // Whether the handler reaches "no profiles found" vs. "unknown profile" from here on
        // depends on whether this host happens to have a real EasyEffects server socket present
        // (see the module doc comment's precedent for host-dependent smoke checks); either way it
        // must not panic, block, or fall through to `Ok`, since "Movie" is never a real preset.
        let response = ipc.execute("effects-profile-set output Movie");
        assert!(
            response == "error: no EasyEffects output profiles found\n"
                || response.starts_with("error: unknown EasyEffects output profile"),
            "unexpected response: {response:?}"
        );
    }
}
