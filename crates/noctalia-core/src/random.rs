//! Port of `src/core/random.h`.
//!
//! The C++ uses `std::mt19937` seeded from `std::random_device` — a C++-stdlib-only
//! facility with no C ABI to bind against, so Phase A FFI doesn't apply here (see
//! MIGRATION_PLAN.md's "Dependency strategy"). `rand` is the canonical Rust
//! equivalent, used the same way `toml`/`serde_json`/`zbus` stand in for their
//! C++-only counterparts elsewhere in this migration.
//!
//! Every call site (`src/shell/wallpaper/wallpaper.cpp`, ported later) only needs a
//! uniformly-distributed float in a range for transition jitter/selection, never a
//! specific PRNG algorithm or a reproducible sequence, so there's no bit-for-bit
//! output to preserve — unlike `process::systemd`'s hand-rolled `/dev/urandom` UUID
//! generator, which matches the C++'s own hand-rolled algorithm byte-for-byte
//! because *those* UUIDs are embedded in systemd unit names other tooling parses.
//!
//! One narrow, currently-unreachable divergence, recorded per this project's
//! convention: C++'s `uniform_real_distribution<float>` tolerates `min == max`
//! (it just always returns `min`), while `rand`'s `random_range` panics on an
//! empty range. None of the real C++ call sites ever pass `min == max` (each
//! guards the degenerate case — e.g. a single-candidate list — before calling
//! `randomFloat` at all), so this never fires in practice.

use rand::Rng as _;

/// Uniform random float in `[min, max)`, matching `Random::randomFloat`.
pub fn random_float(min: f32, max: f32) -> f32 {
    rand::rng().random_range(min..max)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn random_float_stays_in_range() {
        for _ in 0..1000 {
            let value = random_float(0.2, 0.8);
            assert!((0.2..0.8).contains(&value));
        }
    }

    #[test]
    fn random_float_varies_across_calls() {
        let values: std::collections::HashSet<_> =
            (0..50).map(|_| random_float(0.0, 1.0).to_bits()).collect();
        assert!(values.len() > 1);
    }
}
