//! Minimal `dlopen`/`dlsym`/`dlclose` wrapper shared by the NVML (`system::gpu_nvml`) and ROCm SMI
//! (`system::gpu_rsmi`) readers (task 5.6.5.3). Neither vendor library is a build-time link
//! dependency — both are optional runtime `dlopen`s exactly as the C++ does it, so there's no
//! crate to choose between (MIGRATION_PLAN.md 5.6.5.3).
//!
//! The `SymbolSource` trait separates *which name resolves* (a name-cascade decision, pure and
//! testable via a fake) from *fetching the pointer* (real `dlsym`, only meaningful against a real
//! loaded library) — this is the "dlsym-loading logic structured to be testable without the real
//! `.so`" seam the task's done-bar calls for.

use std::ffi::{CString, c_void};

/// A source of named symbols, abstracted so the preferred/fallback name-resolution cascade in
/// [`crate::gpu_nvml`]/[`crate::gpu_rsmi`] is testable against a fake without a real library.
pub(crate) trait SymbolSource {
    fn has_symbol(&self, name: &str) -> bool;
}

/// A loaded shared library, holding the `dlopen` handle for its lifetime.
pub(crate) struct DlLibrary {
    handle: *mut c_void,
}

// SAFETY: `handle` is an opaque `dlopen` token; the POSIX dl* API is documented safe to call from
// any thread (glibc's implementation serializes access internally), and this wrapper exposes no
// interior mutability that could race across threads holding a `&DlLibrary`.
unsafe impl Send for DlLibrary {}
unsafe impl Sync for DlLibrary {}

impl DlLibrary {
    /// Tries each candidate name in order via `dlopen`, returning the first that succeeds — port
    /// of the `for (const char* library : kLibraries) { dlopen(...); if (...) break; }` scan used
    /// by `AmdRsmiReader::ensureReady` (NVML's `NvidiaNvmlReader::ensureReady` calls this with a
    /// single-element slice).
    pub(crate) fn open_first(candidates: &[&str], flags: i32) -> Option<Self> {
        for candidate in candidates {
            let cname = CString::new(*candidate).ok()?;
            // SAFETY: `cname` is a valid NUL-terminated C string live for the duration of the
            // call; `dlopen` accepts any well-formed path/flags and reports failure via a null
            // return rather than trapping.
            let handle = unsafe { libc::dlopen(cname.as_ptr(), flags) };
            if !handle.is_null() {
                return Some(Self { handle });
            }
        }
        None
    }

    /// Resolves a symbol by name, returning its raw address or `None` if it isn't present.
    pub(crate) fn symbol(&self, name: &str) -> Option<*mut c_void> {
        let cname = CString::new(name).ok()?;
        // SAFETY: `self.handle` is a live handle from a successful `dlopen` not yet closed
        // (`DlLibrary` isn't `Clone`, and `Drop` is the only closer); `cname` is a valid
        // NUL-terminated C string live for the call.
        let symbol = unsafe { libc::dlsym(self.handle, cname.as_ptr()) };
        if symbol.is_null() { None } else { Some(symbol) }
    }
}

impl SymbolSource for DlLibrary {
    fn has_symbol(&self, name: &str) -> bool {
        self.symbol(name).is_some()
    }
}

impl Drop for DlLibrary {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a live handle from a successful `dlopen`, closed exactly once
        // here (no other code path calls `dlclose` on it).
        unsafe {
            libc::dlclose(self.handle);
        }
    }
}

/// Port of `loadDlsymFunction`'s name-resolution cascade: try `preferred` first, then `fallback`
/// if given. Kept separate from the actual pointer fetch/cast so it's testable against a fake
/// [`SymbolSource`] without a real library or any `unsafe` function-pointer transmute.
pub(crate) fn resolve_symbol_name<'a>(
    source: &impl SymbolSource,
    preferred: &'a str,
    fallback: Option<&'a str>,
) -> Option<&'a str> {
    if source.has_symbol(preferred) {
        Some(preferred)
    } else {
        fallback.filter(|fb| source.has_symbol(fb))
    }
}

/// Resolves and casts a symbol to the requested function-pointer type `T`. `T` must be an
/// `unsafe extern "C" fn(...)` pointer type matching the dlsym'd symbol's real signature — the
/// caller is responsible for that match, same trust boundary the C++ has via its `T&` template
/// parameter and `std::memcpy`.
pub(crate) fn load_fn<T: Copy>(
    lib: &DlLibrary,
    preferred: &str,
    fallback: Option<&str>,
) -> Option<T> {
    let name = resolve_symbol_name(lib, preferred, fallback)?;
    let ptr = lib.symbol(name)?;
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    // SAFETY: `T` is a `#[repr(C)]`-ABI function-pointer type sized identically to `*mut c_void`
    // (asserted above); `ptr` is a non-null symbol address just resolved from a live library under
    // that name, and the caller guarantees `T`'s signature matches what the symbol actually is.
    Some(unsafe { std::mem::transmute_copy(&ptr) })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct FakeSymbols(HashSet<&'static str>);

    impl SymbolSource for FakeSymbols {
        fn has_symbol(&self, name: &str) -> bool {
            self.0.contains(name)
        }
    }

    #[test]
    fn resolve_symbol_name_prefers_the_preferred_name() {
        let source = FakeSymbols(HashSet::from(["nvmlInit_v2", "nvmlInit"]));
        assert_eq!(
            resolve_symbol_name(&source, "nvmlInit_v2", Some("nvmlInit")),
            Some("nvmlInit_v2")
        );
    }

    #[test]
    fn resolve_symbol_name_falls_back_when_preferred_is_missing() {
        let source = FakeSymbols(HashSet::from(["nvmlInit"]));
        assert_eq!(
            resolve_symbol_name(&source, "nvmlInit_v2", Some("nvmlInit")),
            Some("nvmlInit")
        );
    }

    #[test]
    fn resolve_symbol_name_none_when_neither_is_present() {
        let source = FakeSymbols(HashSet::new());
        assert_eq!(
            resolve_symbol_name(&source, "nvmlInit_v2", Some("nvmlInit")),
            None
        );
    }

    #[test]
    fn resolve_symbol_name_none_when_preferred_missing_and_no_fallback_given() {
        let source = FakeSymbols(HashSet::from(["somethingElse"]));
        assert_eq!(resolve_symbol_name(&source, "nvmlShutdown", None), None);
    }

    #[test]
    fn open_first_returns_none_when_no_candidate_exists() {
        assert!(
            DlLibrary::open_first(
                &["libdefinitely-not-a-real-library.so.999"],
                libc::RTLD_LAZY
            )
            .is_none()
        );
    }
}
