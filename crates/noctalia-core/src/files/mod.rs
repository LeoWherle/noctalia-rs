//! Port of `src/core/files/*`.
//!
//! `file_watcher` ports only `FileWatcher`'s raw inotify logic, not
//! `FileWatchPollSource` — that glue binds it to the C++ codebase's own
//! `PollSource` abstraction, which the Rust port replaces with calloop
//! (architecture decision 1, task 1.5). A calloop event source wrapping
//! `FileWatcher::fd()`/`dispatch()` lands with whichever task first needs it wired
//! into the event loop (config's file-watch service, task 2.9, is the likely one).

pub mod directory_scanner;
pub mod file_watcher;
pub mod paths;
