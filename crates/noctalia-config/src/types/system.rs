//! Task 2.1.7 — System & System Monitor configuration types.
//! Port of `SystemConfig::MonitorConfig` and `SystemConfig` from `src/config/config_types.h`
//! (lines 1096-1166) and `config_schema.cpp:106-151`.

use serde::{Deserialize, Serialize};

/// Port of `SystemConfig::MonitorConfig` (config_types.h:1097-1161).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemMonitorConfig {
    pub enabled: bool,
    pub cpu_temp_sensor_path: String,
    pub cpu_poll_seconds: f32,
    pub gpu_poll_seconds: f32,
    pub memory_poll_seconds: f32,
    pub network_poll_seconds: f32,
    pub disk_poll_seconds: f32,
    pub cpu_usage_activity_threshold: f64,
    pub cpu_usage_critical_threshold: f64,
    pub cpu_temp_activity_threshold: f64,
    pub cpu_temp_critical_threshold: f64,
    pub gpu_temp_activity_threshold: f64,
    pub gpu_temp_critical_threshold: f64,
    pub gpu_usage_activity_threshold: f64,
    pub gpu_usage_critical_threshold: f64,
    pub gpu_vram_activity_threshold: f64,
    pub gpu_vram_critical_threshold: f64,
    pub ram_pct_activity_threshold: f64,
    pub ram_pct_critical_threshold: f64,
    pub swap_pct_activity_threshold: f64,
    pub swap_pct_critical_threshold: f64,
    pub disk_used_pct_activity_threshold: f64,
    pub disk_used_pct_critical_threshold: f64,
    pub disk_used_activity_threshold: f64,
    pub disk_used_critical_threshold: f64,
    pub disk_free_pct_activity_threshold: f64,
    pub disk_free_pct_critical_threshold: f64,
    pub disk_free_activity_threshold: f64,
    pub disk_free_critical_threshold: f64,
    pub net_rx_activity_threshold: f64,
    pub net_rx_critical_threshold: f64,
    pub net_tx_activity_threshold: f64,
    pub net_tx_critical_threshold: f64,
}

impl Default for SystemMonitorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_temp_sensor_path: String::new(),
            cpu_poll_seconds: 2.0,
            gpu_poll_seconds: 5.0,
            memory_poll_seconds: 2.0,
            network_poll_seconds: 3.0,
            disk_poll_seconds: 10.0,
            cpu_usage_activity_threshold: 50.0,
            cpu_usage_critical_threshold: 90.0,
            cpu_temp_activity_threshold: 60.0,
            cpu_temp_critical_threshold: 85.0,
            gpu_temp_activity_threshold: 60.0,
            gpu_temp_critical_threshold: 85.0,
            gpu_usage_activity_threshold: 50.0,
            gpu_usage_critical_threshold: 95.0,
            gpu_vram_activity_threshold: 50.0,
            gpu_vram_critical_threshold: 90.0,
            ram_pct_activity_threshold: 60.0,
            ram_pct_critical_threshold: 90.0,
            swap_pct_activity_threshold: 20.0,
            swap_pct_critical_threshold: 80.0,
            disk_used_pct_activity_threshold: 80.0,
            disk_used_pct_critical_threshold: 95.0,
            disk_used_activity_threshold: 80.0,
            disk_used_critical_threshold: 95.0,
            disk_free_pct_activity_threshold: 80.0,
            disk_free_pct_critical_threshold: 95.0,
            disk_free_activity_threshold: 80.0,
            disk_free_critical_threshold: 95.0,
            net_rx_activity_threshold: 1.0,
            net_rx_critical_threshold: 50.0,
            net_tx_activity_threshold: 1.0,
            net_tx_critical_threshold: 50.0,
        }
    }
}

/// Port of `SystemConfig` (config_types.h:1096-1166).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    pub monitor: SystemMonitorConfig,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn system_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let sys_tbl = root
            .get("system")
            .expect("[system] table present in example.toml");
        let config: SystemConfig = sys_tbl
            .clone()
            .try_into()
            .expect("[system] parses into SystemConfig");

        assert!(config.monitor.enabled);
        assert_eq!(config.monitor.cpu_poll_seconds, 2.0);
        assert_eq!(config.monitor.memory_poll_seconds, 2.0);
    }

    #[test]
    fn system_monitor_config_defaults_match_cpp() {
        let config = SystemMonitorConfig::default();
        assert!(config.enabled);
        assert_eq!(config.cpu_poll_seconds, 2.0);
        assert_eq!(config.gpu_poll_seconds, 5.0);
        assert_eq!(config.memory_poll_seconds, 2.0);
        assert_eq!(config.network_poll_seconds, 3.0);
        assert_eq!(config.disk_poll_seconds, 10.0);
        assert_eq!(config.cpu_usage_activity_threshold, 50.0);
        assert_eq!(config.cpu_usage_critical_threshold, 90.0);
    }
}
