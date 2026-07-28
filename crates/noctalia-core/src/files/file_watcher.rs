//! Port of `src/core/files/file_watcher.{cpp,h}` — the raw inotify logic only.
//! `FileWatchPollSource` (the glue into the C++ codebase's own poll-based event
//! loop) has no port here; see the module-level doc comment in `files::mod`.

use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::log::Logger;

const LOG: Logger = Logger::new("file-watcher");

const DEBOUNCE: Duration = Duration::from_millis(100);
const WATCH_MASK: u32 =
    libc::IN_MODIFY | libc::IN_CLOSE_WRITE | libc::IN_CREATE | libc::IN_MOVED_TO;

pub type WatchId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchTrigger {
    Modified,
    WriteCompleted,
}

fn event_matches_trigger(trigger: WatchTrigger, mask: u32) -> bool {
    match trigger {
        WatchTrigger::Modified => mask & WATCH_MASK != 0,
        WatchTrigger::WriteCompleted => mask & (libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO) != 0,
    }
}

struct WatchEntry {
    filename: OsString,
    callback: Box<dyn FnMut()>,
    dir_wd: i32,
    trigger: WatchTrigger,
    last_fired: Option<Instant>,
}

pub struct FileWatcher {
    inotify_fd: i32,
    next_id: WatchId,
    watches: HashMap<WatchId, WatchEntry>,
    dir_wd_ref_count: HashMap<i32, i32>,
    dir_to_wd: HashMap<OsString, i32>,
}

impl Default for FileWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWatcher {
    pub fn new() -> Self {
        // SAFETY: inotify_init1 is a plain syscall wrapper; IN_NONBLOCK|IN_CLOEXEC
        // are valid flags for it.
        let inotify_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if inotify_fd < 0 {
            LOG.warn(format_args!("inotify_init1 failed"));
        }
        Self {
            inotify_fd,
            next_id: 1,
            watches: HashMap::new(),
            dir_wd_ref_count: HashMap::new(),
            dir_to_wd: HashMap::new(),
        }
    }

    pub fn fd(&self) -> i32 {
        self.inotify_fd
    }

    pub fn watch(
        &mut self,
        file_path: &Path,
        callback: impl FnMut() + 'static,
        trigger: WatchTrigger,
    ) -> WatchId {
        if self.inotify_fd < 0 {
            return 0;
        }

        let dir = file_path.parent().unwrap_or_else(|| Path::new(""));
        let filename = file_path
            .file_name()
            .map(OsStr::to_os_string)
            .unwrap_or_default();
        let dir_key = dir.as_os_str().to_os_string();

        let wd = if let Some(&existing) = self.dir_to_wd.get(&dir_key) {
            *self.dir_wd_ref_count.entry(existing).or_insert(0) += 1;
            existing
        } else {
            let Some(new_wd) = add_directory_watch(self.inotify_fd, dir) else {
                LOG.warn(format_args!(
                    "failed to watch directory '{}'",
                    dir.display()
                ));
                return 0;
            };
            self.dir_to_wd.insert(dir_key, new_wd);
            self.dir_wd_ref_count.insert(new_wd, 1);
            new_wd
        };

        let id = self.next_id;
        self.next_id += 1;
        LOG.debug(format_args!("watching '{}' (id {id})", file_path.display()));
        self.watches.insert(
            id,
            WatchEntry {
                filename,
                callback: Box::new(callback),
                dir_wd: wd,
                trigger,
                last_fired: None,
            },
        );
        id
    }

    pub fn unwatch(&mut self, id: WatchId) {
        let Some(entry) = self.watches.remove(&id) else {
            return;
        };
        LOG.debug(format_args!("unwatching id {id}"));

        if let Some(count) = self.dir_wd_ref_count.get_mut(&entry.dir_wd) {
            *count -= 1;
            if *count <= 0 {
                // SAFETY: `entry.dir_wd` was returned by a prior successful
                // `inotify_add_watch` on this same `inotify_fd` and hasn't been
                // removed since (this is the only place watches are torn down,
                // guarded by the same ref count).
                unsafe {
                    libc::inotify_rm_watch(self.inotify_fd, entry.dir_wd);
                }
                self.dir_wd_ref_count.remove(&entry.dir_wd);
                self.dir_to_wd.retain(|_, wd| *wd != entry.dir_wd);
            }
        }
    }

    pub fn dispatch(&mut self) {
        // `inotify_event` (4 `u32`/`c_int` fields) needs 4-byte alignment; a plain
        // `[u8; N]` only guarantees alignment 1. C++ pins this down with
        // `alignas(inotify_event) char buf[4096]` — this wrapper is the same fix,
        // otherwise casting a slice of the buffer to `*const inotify_event` below
        // is UB regardless of what the target's load instructions tolerate.
        #[repr(C, align(4))]
        struct AlignedBuf([u8; 4096]);

        let mut buf = AlignedBuf([0u8; 4096]);
        let mut triggered: Vec<WatchId> = Vec::new();
        let header_size = std::mem::size_of::<libc::inotify_event>();

        loop {
            // SAFETY: `buf` is a valid, appropriately sized, 4-byte-aligned
            // buffer; `inotify_fd` is a valid fd owned by this `FileWatcher` (or
            // read is skipped above).
            let n = unsafe { libc::read(self.inotify_fd, buf.0.as_mut_ptr().cast(), buf.0.len()) };
            if n <= 0 {
                break;
            }
            let n = n as usize;

            let mut offset = 0usize;
            while offset + header_size <= n {
                // SAFETY: the kernel writes complete `inotify_event` records into
                // this buffer, back to back, each padded so the next one starts
                // 4-byte aligned (`header_size` and `event.len` are both multiples
                // of 4); `buf` itself is 4-byte aligned per `AlignedBuf` above, so
                // every `offset` here is too. `offset + header_size <= n`
                // guarantees the fixed-size header is fully present.
                let event = unsafe { &*(buf.0.as_ptr().add(offset).cast::<libc::inotify_event>()) };
                let len = event.len as usize;
                if event.len > 0 && offset + header_size + len <= n {
                    let name_start = offset + header_size;
                    let name_bytes = &buf.0[name_start..name_start + len];
                    let nul = name_bytes.iter().position(|&b| b == 0).unwrap_or(len);
                    let name = OsStr::from_bytes(&name_bytes[..nul]);

                    for (&id, entry) in &self.watches {
                        if entry.dir_wd == event.wd
                            && entry.filename == name
                            && event_matches_trigger(entry.trigger, event.mask)
                            && !triggered.contains(&id)
                        {
                            triggered.push(id);
                        }
                    }
                }
                offset += header_size + len;
            }
        }

        let now = Instant::now();
        for id in triggered {
            let Some(entry) = self.watches.get_mut(&id) else {
                continue;
            };
            if let Some(last) = entry.last_fired
                && now.duration_since(last) < DEBOUNCE
            {
                continue;
            }
            entry.last_fired = Some(now);
            (entry.callback)();
        }
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        if self.inotify_fd < 0 {
            return;
        }
        for &wd in self.dir_wd_ref_count.keys() {
            // SAFETY: each `wd` was returned by a prior successful
            // `inotify_add_watch` on this fd and is only removed once, here.
            unsafe {
                libc::inotify_rm_watch(self.inotify_fd, wd);
            }
        }
        // SAFETY: `inotify_fd` is owned by this `FileWatcher` and not touched again.
        unsafe {
            libc::close(self.inotify_fd);
        }
    }
}

fn add_directory_watch(inotify_fd: i32, dir: &Path) -> Option<i32> {
    let dir_c = CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: `dir_c` is a valid NUL-terminated C string alive for the call;
    // `inotify_fd` is a valid, open inotify file descriptor.
    let wd = unsafe { libc::inotify_add_watch(inotify_fd, dir_c.as_ptr(), WATCH_MASK) };
    if wd < 0 { None } else { Some(wd) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;
    use std::time::Duration;

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    // Polls `watcher.dispatch()` until `count` reaches `expected` or `timeout`
    // elapses; inotify delivery is async even on a local tmpfs.
    fn wait_for(
        watcher: &mut FileWatcher,
        count: &Rc<RefCell<u32>>,
        expected: u32,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            watcher.dispatch();
            if *count.borrow() >= expected {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn fires_callback_on_modification() {
        let dir = make_temp_dir("noctalia-file-watcher-modify");
        let file_path = dir.join("watched.txt");
        fs::write(&file_path, "initial").expect("failed to seed file");

        let mut watcher = FileWatcher::new();
        assert!(watcher.fd() >= 0);

        let count = Rc::new(RefCell::new(0u32));
        let count_clone = Rc::clone(&count);
        let id = watcher.watch(
            &file_path,
            move || *count_clone.borrow_mut() += 1,
            WatchTrigger::Modified,
        );
        assert_ne!(id, 0);

        fs::write(&file_path, "changed").expect("failed to modify file");
        assert!(
            wait_for(&mut watcher, &count, 1, Duration::from_secs(2)),
            "callback did not fire"
        );

        watcher.unwatch(id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_completed_trigger_ignores_plain_modify() {
        let dir = make_temp_dir("noctalia-file-watcher-write-completed");
        let file_path = dir.join("watched.txt");
        fs::write(&file_path, "initial").expect("failed to seed file");

        let mut watcher = FileWatcher::new();
        let count = Rc::new(RefCell::new(0u32));
        let count_clone = Rc::clone(&count);
        let id = watcher.watch(
            &file_path,
            move || *count_clone.borrow_mut() += 1,
            WatchTrigger::WriteCompleted,
        );

        // Open + write without closing: IN_MODIFY fires, IN_CLOSE_WRITE does not yet.
        {
            use std::io::Write as _;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .open(&file_path)
                .expect("failed to open file");
            f.write_all(b"partial").expect("failed to write");
            watcher.dispatch();
        }
        assert_eq!(*count.borrow(), 0, "WriteCompleted fired on a bare modify");

        fs::write(&file_path, "closed-and-written").expect("failed to rewrite file");
        assert!(
            wait_for(&mut watcher, &count, 1, Duration::from_secs(2)),
            "callback did not fire on close-write"
        );

        watcher.unwatch(id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwatch_stops_further_callbacks() {
        let dir = make_temp_dir("noctalia-file-watcher-unwatch");
        let file_path = dir.join("watched.txt");
        fs::write(&file_path, "initial").expect("failed to seed file");

        let mut watcher = FileWatcher::new();
        let count = Rc::new(RefCell::new(0u32));
        let count_clone = Rc::clone(&count);
        let id = watcher.watch(
            &file_path,
            move || *count_clone.borrow_mut() += 1,
            WatchTrigger::Modified,
        );

        watcher.unwatch(id);
        fs::write(&file_path, "changed-after-unwatch").expect("failed to modify file");
        assert!(
            !wait_for(&mut watcher, &count, 1, Duration::from_millis(300)),
            "callback fired after unwatch"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_watches_in_the_same_directory_share_one_inotify_watch() {
        let dir = make_temp_dir("noctalia-file-watcher-shared-dir");
        let file_a = dir.join("a.txt");
        let file_b = dir.join("b.txt");
        fs::write(&file_a, "a").expect("failed to seed a");
        fs::write(&file_b, "b").expect("failed to seed b");

        let mut watcher = FileWatcher::new();
        let count_a = Rc::new(RefCell::new(0u32));
        let count_b = Rc::new(RefCell::new(0u32));
        let (ca, cb) = (Rc::clone(&count_a), Rc::clone(&count_b));
        let id_a = watcher.watch(
            &file_a,
            move || *ca.borrow_mut() += 1,
            WatchTrigger::Modified,
        );
        let id_b = watcher.watch(
            &file_b,
            move || *cb.borrow_mut() += 1,
            WatchTrigger::Modified,
        );
        assert_eq!(
            watcher.dir_wd_ref_count.len(),
            1,
            "watches on the same dir should share one inotify watch"
        );

        fs::write(&file_a, "changed").expect("failed to modify a");
        assert!(wait_for(&mut watcher, &count_a, 1, Duration::from_secs(2)));
        assert_eq!(
            *count_b.borrow(),
            0,
            "b's callback should not fire for a's change"
        );

        watcher.unwatch(id_a);
        watcher.unwatch(id_b);
        let _ = fs::remove_dir_all(&dir);
    }
}
