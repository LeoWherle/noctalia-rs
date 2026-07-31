//! Port of `src/system/format_units.{cpp,h}` (task 5.6.1): human-readable byte/byte-rate
//! formatting helpers. Fully self-contained (no dependencies beyond the standard library) — no
//! forward-phase blocker.
//!
//! No C++ test exists for this file (confirmed: no `format_units_test.cpp` in `tests/`) — task
//! 5.6's own done bar is a smoke test.

/// Port of `FormatUnits::DecimalByteRateUnit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecimalByteRateUnit {
    #[default]
    Auto,
    Kilobytes,
    Megabytes,
}

/// Port of `FormatUnits::ByteRateLabelStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ByteRateLabelStyle {
    #[default]
    Full,
    Compact,
}

const MIB_PER_GIB: f64 = 1024.0;
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const BYTES_PER_KB: f64 = 1000.0;
const BYTES_PER_MB: f64 = 1000.0 * 1000.0;
const BYTES_PER_GB: f64 = 1000.0 * 1000.0 * 1000.0;
const BYTES_PER_TB: f64 = 1000.0 * 1000.0 * 1000.0 * 1000.0;

/// Port of `formatBinaryMib`.
pub fn format_binary_mib(mib: u64) -> String {
    if mib >= MIB_PER_GIB as u64 {
        return format_binary_mib_as_gib(mib);
    }
    format!("{mib} MiB")
}

/// Port of `formatBinaryMibAsGib`.
pub fn format_binary_mib_as_gib(mib: u64) -> String {
    format!("{:.1} GiB", mib as f64 / MIB_PER_GIB)
}

/// Port of `formatBinaryMibUsageAsGib`.
pub fn format_binary_mib_usage_as_gib(used_mib: u64, total_mib: u64) -> String {
    format!(
        "{:.1} / {:.1} GiB",
        used_mib as f64 / MIB_PER_GIB,
        total_mib as f64 / MIB_PER_GIB
    )
}

/// Port of `formatBinaryBytesAsGib`.
pub fn format_binary_bytes_as_gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / BYTES_PER_GIB)
}

/// Port of `formatDecimalBytesUsage`. Picks the unit from the total so both numbers share it; TB
/// once disks pass 1000 GB keeps the column narrow and readable.
pub fn format_decimal_bytes_usage(used_bytes: f64, total_bytes: f64) -> String {
    if total_bytes >= BYTES_PER_TB {
        return format!(
            "{:.1} / {:.1} TB",
            used_bytes / BYTES_PER_TB,
            total_bytes / BYTES_PER_TB
        );
    }
    format!(
        "{:.1} / {:.1} GB",
        used_bytes / BYTES_PER_GB,
        total_bytes / BYTES_PER_GB
    )
}

/// Port of `formatDecimalBytesAsGb`.
pub fn format_decimal_bytes_as_gb(bytes: f64) -> String {
    format!("{:.1} GB", bytes / BYTES_PER_GB)
}

/// Port of `decimalByteRateUnitFromString`.
pub fn decimal_byte_rate_unit_from_string(value: &str) -> DecimalByteRateUnit {
    match value {
        "kb" => DecimalByteRateUnit::Kilobytes,
        "mb" => DecimalByteRateUnit::Megabytes,
        _ => DecimalByteRateUnit::Auto,
    }
}

fn format_byte_rate_value(
    value: f64,
    full_suffix: &str,
    compact_suffix: &str,
    label_style: ByteRateLabelStyle,
) -> String {
    match label_style {
        ByteRateLabelStyle::Compact => format!("{value:.1}{compact_suffix}"),
        ByteRateLabelStyle::Full => format!("{value:.1} {full_suffix}"),
    }
}

fn format_byte_rate_bytes(bytes_per_sec: f64, label_style: ByteRateLabelStyle) -> String {
    match label_style {
        ByteRateLabelStyle::Compact => format!("{bytes_per_sec:.0}B"),
        ByteRateLabelStyle::Full => format!("{bytes_per_sec:.0} B/s"),
    }
}

/// Port of `formatDecimalBytesPerSecond`. The C++ defaults (`unit = Auto`,
/// `labelStyle = Full`) become explicit arguments here — no Rust default-argument equivalent;
/// both enums implement `Default` for callers that want them.
pub fn format_decimal_bytes_per_second(
    bytes_per_sec: f64,
    unit: DecimalByteRateUnit,
    label_style: ByteRateLabelStyle,
) -> String {
    match unit {
        DecimalByteRateUnit::Kilobytes => {
            return format_byte_rate_value(bytes_per_sec / BYTES_PER_KB, "kB/s", "k", label_style);
        }
        DecimalByteRateUnit::Megabytes => {
            return format_byte_rate_value(bytes_per_sec / BYTES_PER_MB, "MB/s", "M", label_style);
        }
        DecimalByteRateUnit::Auto => {}
    }

    if bytes_per_sec >= BYTES_PER_GB {
        format_byte_rate_value(bytes_per_sec / BYTES_PER_GB, "GB/s", "G", label_style)
    } else if bytes_per_sec >= BYTES_PER_MB {
        format_byte_rate_value(bytes_per_sec / BYTES_PER_MB, "MB/s", "M", label_style)
    } else if bytes_per_sec >= BYTES_PER_KB {
        format_byte_rate_value(bytes_per_sec / BYTES_PER_KB, "kB/s", "k", label_style)
    } else {
        format_byte_rate_bytes(bytes_per_sec, label_style)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn format_binary_mib_switches_to_gib_at_1024() {
        assert_eq!(format_binary_mib(512), "512 MiB");
        assert_eq!(format_binary_mib(1024), "1.0 GiB");
        assert_eq!(format_binary_mib(2048), "2.0 GiB");
    }

    #[test]
    fn format_binary_mib_usage_as_gib_formats_both_sides() {
        assert_eq!(format_binary_mib_usage_as_gib(1024, 8192), "1.0 / 8.0 GiB");
    }

    #[test]
    fn format_binary_bytes_as_gib_divides_by_1024_cubed() {
        assert_eq!(format_binary_bytes_as_gib(1024 * 1024 * 1024), "1.0 GiB");
    }

    #[test]
    fn format_decimal_bytes_usage_switches_to_tb_from_total() {
        assert_eq!(
            format_decimal_bytes_usage(1_300_000_000_000.0, 1_967_900_000_000.0),
            "1.3 / 2.0 TB"
        );
        assert_eq!(
            format_decimal_bytes_usage(50_000_000_000.0, 100_000_000_000.0),
            "50.0 / 100.0 GB"
        );
    }

    #[test]
    fn format_decimal_bytes_as_gb_divides_by_1e9() {
        assert_eq!(format_decimal_bytes_as_gb(1_500_000_000.0), "1.5 GB");
    }

    #[test]
    fn decimal_byte_rate_unit_from_string_matches_known_tags() {
        assert_eq!(
            decimal_byte_rate_unit_from_string("kb"),
            DecimalByteRateUnit::Kilobytes
        );
        assert_eq!(
            decimal_byte_rate_unit_from_string("mb"),
            DecimalByteRateUnit::Megabytes
        );
        assert_eq!(
            decimal_byte_rate_unit_from_string("gb"),
            DecimalByteRateUnit::Auto
        );
        assert_eq!(
            decimal_byte_rate_unit_from_string(""),
            DecimalByteRateUnit::Auto
        );
    }

    #[test]
    fn format_decimal_bytes_per_second_auto_picks_scale() {
        assert_eq!(
            format_decimal_bytes_per_second(
                500.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Full
            ),
            "500 B/s"
        );
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Full
            ),
            "1.5 kB/s"
        );
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500_000.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Full
            ),
            "1.5 MB/s"
        );
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500_000_000.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Full
            ),
            "1.5 GB/s"
        );
    }

    #[test]
    fn format_decimal_bytes_per_second_forces_explicit_unit() {
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500_000.0,
                DecimalByteRateUnit::Kilobytes,
                ByteRateLabelStyle::Full
            ),
            "1500.0 kB/s"
        );
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500.0,
                DecimalByteRateUnit::Megabytes,
                ByteRateLabelStyle::Full
            ),
            "0.0 MB/s"
        );
    }

    #[test]
    fn format_decimal_bytes_per_second_compact_style_drops_the_space_and_per_second() {
        assert_eq!(
            format_decimal_bytes_per_second(
                1_500_000.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Compact
            ),
            "1.5M"
        );
        assert_eq!(
            format_decimal_bytes_per_second(
                500.0,
                DecimalByteRateUnit::Auto,
                ByteRateLabelStyle::Compact
            ),
            "500B"
        );
    }
}
