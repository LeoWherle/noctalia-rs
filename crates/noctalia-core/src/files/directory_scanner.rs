//! Port of `src/core/files/directory_scanner.{cpp,h}`.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogSortField {
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogSortOrder {
    Ascending,
    Descending,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub abs_path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

// Canonical raster image extensions: lowercase, dot-prefixed.
const IMAGE_EXTENSIONS: [&str; 7] = [".jpg", ".jpeg", ".png", ".webp", ".jxl", ".bmp", ".gif"];

pub fn scan(
    dir: &Path,
    extensions: &[String],
    show_hidden_files: bool,
    sort_field: FileDialogSortField,
    sort_order: FileDialogSortOrder,
) -> Vec<FileEntry> {
    if dir.as_os_str().is_empty() {
        return Vec::new();
    }
    match fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        _ => return Vec::new(),
    }

    let normalized_extensions: Vec<String> = extensions
        .iter()
        .map(|e| normalize_extension(e))
        .filter(|e| !e.is_empty())
        .collect();

    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<FileEntry> = Vec::new();
    for item in read_dir {
        // Mirrors `directory_options::skip_permission_denied` for permission
        // errors, but diverges for any other iteration error: the C++ sets
        // `ec` and `break`s the whole scan (directory_scanner.cpp:44-45),
        // where this `continue`s past the bad entry and keeps scanning.
        // Deliberate, not "fixed": low practical impact (Rust ends up
        // scanning more on the hypothetical error path, not less — same
        // shape as `process::matching`'s break-vs-skip divergence) and a
        // `break` here would require distinguishing error kinds that
        // `std::fs::ReadDir` doesn't expose as cleanly as `std::error_code`.
        let Ok(item) = item else { continue };

        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden_files && is_hidden_name(&name) {
            continue;
        }

        // `std::filesystem::directory_entry::is_directory()` follows symlinks, so
        // stat (not `DirEntry::file_type`, which is an `lstat`-equivalent) to match.
        let Ok(meta) = fs::metadata(item.path()) else {
            continue;
        };
        let is_dir = meta.is_dir();
        if !is_dir && (!meta.is_file() || !matches_extension(&item.path(), &normalized_extensions))
        {
            continue;
        }

        let abs_path = make_absolute(&item.path());
        let size = if is_dir { 0 } else { meta.len() };
        let mtime = meta.modified().ok();

        entries.push(FileEntry {
            name,
            abs_path,
            is_dir,
            size,
            mtime,
        });
    }

    entries.sort_by(|a, b| compare_entries(a, b, sort_field, sort_order));
    entries
}

fn make_absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn compare_entries(
    a: &FileEntry,
    b: &FileEntry,
    sort_field: FileDialogSortField,
    sort_order: FileDialogSortOrder,
) -> Ordering {
    if a.is_dir != b.is_dir {
        return if a.is_dir {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    let ascending = sort_order == FileDialogSortOrder::Ascending;

    match sort_field {
        FileDialogSortField::Size => {
            if !a.is_dir && !b.is_dir && a.size != b.size {
                return if ascending {
                    a.size.cmp(&b.size)
                } else {
                    b.size.cmp(&a.size)
                };
            }
        }
        FileDialogSortField::Modified => {
            if a.mtime != b.mtime {
                return if ascending {
                    a.mtime.cmp(&b.mtime)
                } else {
                    b.mtime.cmp(&a.mtime)
                };
            }
        }
        FileDialogSortField::Name => {}
    }

    let lower_a = a.name.to_ascii_lowercase();
    let lower_b = b.name.to_ascii_lowercase();
    if lower_a != lower_b {
        return if ascending {
            lower_a.cmp(&lower_b)
        } else {
            lower_b.cmp(&lower_a)
        };
    }
    if a.name != b.name {
        return if ascending {
            a.name.cmp(&b.name)
        } else {
            b.name.cmp(&a.name)
        };
    }
    a.abs_path.as_os_str().cmp(b.abs_path.as_os_str())
}

pub fn is_image_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(normalize_extension)
        .unwrap_or_default();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

pub fn image_extension_filter(include_svg: bool) -> Vec<String> {
    let mut out: Vec<String> = IMAGE_EXTENSIONS
        .iter()
        .map(|ext| (*ext).to_string())
        .collect();
    if include_svg {
        out.push(".svg".to_string());
    }
    out
}

fn matches_extension(path: &Path, extensions: &[String]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(normalize_extension)
        .unwrap_or_default();
    extensions.contains(&ext)
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

fn normalize_extension(extension: &str) -> String {
    if extension.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(extension.len() + 1);
    if !extension.starts_with('.') {
        out.push('.');
    }
    out.push_str(&extension.to_ascii_lowercase());
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn make_temp_dir(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn touch(path: &Path, contents: &[u8]) {
        let mut file = fs::File::create(path).expect("failed to create fixture file");
        file.write_all(contents)
            .expect("failed to write fixture file");
    }

    #[test]
    fn scan_of_missing_directory_is_empty() {
        let dir = std::env::temp_dir().join("noctalia-directory-scanner-does-not-exist");
        let _ = fs::remove_dir_all(&dir);
        assert!(
            scan(
                &dir,
                &[],
                false,
                FileDialogSortField::Name,
                FileDialogSortOrder::Ascending
            )
            .is_empty()
        );
    }

    #[test]
    fn scan_hides_dotfiles_unless_asked() {
        let dir = make_temp_dir("noctalia-directory-scanner-hidden");
        touch(&dir.join("visible.txt"), b"a");
        touch(&dir.join(".hidden.txt"), b"b");

        let hidden_excluded = scan(
            &dir,
            &[],
            false,
            FileDialogSortField::Name,
            FileDialogSortOrder::Ascending,
        );
        assert_eq!(hidden_excluded.len(), 1);
        assert_eq!(hidden_excluded[0].name, "visible.txt");

        let hidden_included = scan(
            &dir,
            &[],
            true,
            FileDialogSortField::Name,
            FileDialogSortOrder::Ascending,
        );
        assert_eq!(hidden_included.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_filters_by_extension_case_and_dot_insensitively() {
        let dir = make_temp_dir("noctalia-directory-scanner-ext");
        touch(&dir.join("a.PNG"), b"a");
        touch(&dir.join("b.jpg"), b"b");
        touch(&dir.join("c.txt"), b"c");

        let entries = scan(
            &dir,
            &["png".to_string()],
            false,
            FileDialogSortField::Name,
            FileDialogSortOrder::Ascending,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.PNG");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_sorts_directories_before_files_regardless_of_name() {
        let dir = make_temp_dir("noctalia-directory-scanner-dirs-first");
        touch(&dir.join("aaa.txt"), b"a");
        fs::create_dir(dir.join("zzz-subdir")).expect("failed to create subdir");

        let entries = scan(
            &dir,
            &[],
            false,
            FileDialogSortField::Name,
            FileDialogSortOrder::Ascending,
        );
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "zzz-subdir");
        assert!(!entries[1].is_dir);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_sorts_by_name_case_insensitively_and_respects_order() {
        let dir = make_temp_dir("noctalia-directory-scanner-name-order");
        touch(&dir.join("Banana.txt"), b"a");
        touch(&dir.join("apple.txt"), b"b");
        touch(&dir.join("cherry.txt"), b"c");

        let ascending = scan(
            &dir,
            &[],
            false,
            FileDialogSortField::Name,
            FileDialogSortOrder::Ascending,
        );
        let names: Vec<_> = ascending.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["apple.txt", "Banana.txt", "cherry.txt"]);

        let descending = scan(
            &dir,
            &[],
            false,
            FileDialogSortField::Name,
            FileDialogSortOrder::Descending,
        );
        let names: Vec<_> = descending.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["cherry.txt", "Banana.txt", "apple.txt"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_sorts_by_size_within_files_only() {
        let dir = make_temp_dir("noctalia-directory-scanner-size-order");
        touch(&dir.join("big.txt"), b"aaaaaaaaaa");
        touch(&dir.join("small.txt"), b"a");

        let entries = scan(
            &dir,
            &[],
            false,
            FileDialogSortField::Size,
            FileDialogSortOrder::Ascending,
        );
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["small.txt", "big.txt"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn modified_field_compares_directory_mtimes_unlike_size() {
        // Unlike `Size` (guarded to `!a.is_dir && !b.is_dir`), the C++ comparator's
        // `Modified` arm compares `a.mtime != b.mtime` unconditionally — so two
        // directories with different mtimes sort by time, not by falling through
        // to name order the way they would under `Size`.
        let older = FileEntry {
            name: "zzz-newer-name-older-mtime".to_string(),
            abs_path: PathBuf::from("/tmp/zzz-newer-name-older-mtime"),
            is_dir: true,
            size: 0,
            mtime: Some(SystemTime::UNIX_EPOCH),
        };
        let newer = FileEntry {
            name: "aaa-older-name-newer-mtime".to_string(),
            abs_path: PathBuf::from("/tmp/aaa-older-name-newer-mtime"),
            is_dir: true,
            size: 0,
            mtime: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)),
        };

        assert_eq!(
            compare_entries(
                &older,
                &newer,
                FileDialogSortField::Modified,
                FileDialogSortOrder::Ascending
            ),
            Ordering::Less,
            "older directory should sort first by mtime despite its alphabetically later name"
        );
        assert_eq!(
            compare_entries(
                &older,
                &newer,
                FileDialogSortField::Modified,
                FileDialogSortOrder::Descending
            ),
            Ordering::Greater
        );

        // Under `Size`, both being directories means the size guard never applies,
        // so two dirs fall straight through to name order regardless of field.
        assert_eq!(
            compare_entries(
                &older,
                &newer,
                FileDialogSortField::Size,
                FileDialogSortOrder::Ascending
            ),
            older
                .name
                .to_ascii_lowercase()
                .cmp(&newer.name.to_ascii_lowercase()),
            "two dirs should sort by name under Size, not by (irrelevant) size"
        );
    }

    #[test]
    fn is_image_path_matches_known_raster_extensions_case_insensitively() {
        assert!(is_image_path(Path::new("photo.PNG")));
        assert!(is_image_path(Path::new("photo.jpg")));
        assert!(!is_image_path(Path::new("photo.svg")));
        assert!(!is_image_path(Path::new("photo")));
    }

    #[test]
    fn image_extension_filter_optionally_includes_svg() {
        assert!(!image_extension_filter(false).contains(&".svg".to_string()));
        assert!(image_extension_filter(true).contains(&".svg".to_string()));
        assert!(image_extension_filter(false).contains(&".png".to_string()));
    }
}
