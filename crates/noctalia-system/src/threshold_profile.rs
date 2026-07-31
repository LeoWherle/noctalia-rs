//! Port of `src/system/sysmon_threshold_profile.h` (task 5.6.5.1): per-stat default
//! activity/critical thresholds and slider ranges for the system monitor. Fully self-contained
//! (no dependencies).
//!
//! The *values* returned here are already baked into `noctalia-config`'s
//! `SystemMonitorConfig::default()` (task 2.1.7) as the `*_activity_threshold`/
//! `*_critical_threshold` defaults, but the `Stat` enum and `ThresholdProfile` struct (min/max/
//! step, needed for Phase 14's settings-window threshold sliders, `settings_registry.cpp`) aren't
//! ported standalone until this module.
//!
//! No C++ test exists for this file (confirmed: no `sysmon_threshold_profile_test.cpp` in
//! `tests/`) — task 5.6's own done bar is a smoke test.

/// Port of `noctalia::sysmon::Stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stat {
    CpuUsage,
    CpuTemp,
    GpuTemp,
    GpuUsage,
    GpuVram,
    RamUsed,
    RamPct,
    SwapPct,
    DiskUsedPct,
    DiskUsed,
    DiskFreePct,
    DiskFree,
    NetRx,
    NetTx,
}

/// Port of `noctalia::sysmon::ThresholdProfile`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdProfile {
    pub activity_default: f64,
    pub critical_default: f64,
    pub min_value: f64,
    pub max_value: f64,
    pub step: f64,
}

impl Default for ThresholdProfile {
    fn default() -> Self {
        Self {
            activity_default: 50.0,
            critical_default: 100.0,
            min_value: 0.0,
            max_value: 100.0,
            step: 1.0,
        }
    }
}

/// Port of `noctalia::sysmon::thresholdProfile`.
#[must_use]
pub fn threshold_profile(stat: Stat) -> ThresholdProfile {
    match stat {
        Stat::CpuUsage => ThresholdProfile {
            activity_default: 50.0,
            critical_default: 90.0,
            ..Default::default()
        },
        Stat::CpuTemp | Stat::GpuTemp => ThresholdProfile {
            activity_default: 60.0,
            critical_default: 85.0,
            ..Default::default()
        },
        Stat::GpuUsage => ThresholdProfile {
            activity_default: 50.0,
            critical_default: 95.0,
            ..Default::default()
        },
        Stat::GpuVram => ThresholdProfile {
            activity_default: 50.0,
            critical_default: 90.0,
            ..Default::default()
        },
        Stat::RamUsed | Stat::RamPct => ThresholdProfile {
            activity_default: 60.0,
            critical_default: 90.0,
            ..Default::default()
        },
        Stat::SwapPct => ThresholdProfile {
            activity_default: 20.0,
            critical_default: 80.0,
            ..Default::default()
        },
        Stat::DiskUsedPct | Stat::DiskUsed | Stat::DiskFreePct | Stat::DiskFree => {
            ThresholdProfile {
                activity_default: 80.0,
                critical_default: 95.0,
                ..Default::default()
            }
        }
        Stat::NetRx | Stat::NetTx => ThresholdProfile {
            activity_default: 1.0,
            critical_default: 50.0,
            min_value: 0.0,
            max_value: 100.0,
            step: 0.1,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_matches_cpp_defaults() {
        let profile = threshold_profile(Stat::CpuUsage);
        assert_eq!(profile.activity_default, 50.0);
        assert_eq!(profile.critical_default, 90.0);
        assert_eq!(profile.min_value, 0.0);
        assert_eq!(profile.max_value, 100.0);
        assert_eq!(profile.step, 1.0);
    }

    #[test]
    fn cpu_temp_and_gpu_temp_share_a_profile() {
        assert_eq!(
            threshold_profile(Stat::CpuTemp),
            threshold_profile(Stat::GpuTemp)
        );
        let profile = threshold_profile(Stat::CpuTemp);
        assert_eq!(profile.activity_default, 60.0);
        assert_eq!(profile.critical_default, 85.0);
    }

    #[test]
    fn ram_used_and_ram_pct_share_a_profile() {
        assert_eq!(
            threshold_profile(Stat::RamUsed),
            threshold_profile(Stat::RamPct)
        );
    }

    #[test]
    fn disk_variants_share_a_profile() {
        let disk_used_pct = threshold_profile(Stat::DiskUsedPct);
        assert_eq!(disk_used_pct, threshold_profile(Stat::DiskUsed));
        assert_eq!(disk_used_pct, threshold_profile(Stat::DiskFreePct));
        assert_eq!(disk_used_pct, threshold_profile(Stat::DiskFree));
        assert_eq!(disk_used_pct.activity_default, 80.0);
        assert_eq!(disk_used_pct.critical_default, 95.0);
    }

    #[test]
    fn net_rx_and_tx_use_a_finer_step_and_share_a_profile() {
        let net_rx = threshold_profile(Stat::NetRx);
        assert_eq!(net_rx, threshold_profile(Stat::NetTx));
        assert_eq!(net_rx.activity_default, 1.0);
        assert_eq!(net_rx.critical_default, 50.0);
        assert_eq!(net_rx.min_value, 0.0);
        assert_eq!(net_rx.max_value, 100.0);
        assert_eq!(net_rx.step, 0.1);
    }

    #[test]
    fn swap_pct_and_gpu_vram_have_distinct_profiles() {
        let swap = threshold_profile(Stat::SwapPct);
        assert_eq!(swap.activity_default, 20.0);
        assert_eq!(swap.critical_default, 80.0);

        let gpu_vram = threshold_profile(Stat::GpuVram);
        assert_eq!(gpu_vram.activity_default, 50.0);
        assert_eq!(gpu_vram.critical_default, 90.0);
    }

    #[test]
    fn gpu_usage_has_its_own_profile() {
        let profile = threshold_profile(Stat::GpuUsage);
        assert_eq!(profile.activity_default, 50.0);
        assert_eq!(profile.critical_default, 95.0);
    }
}
