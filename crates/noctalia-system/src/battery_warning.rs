//! Port of the pure escalation state machine in `src/system/battery_warning_monitor.{h,cpp}`
//! (task 5.4.1): `alertPointsForDevice`, `currentLevelFor`, and `BatteryWarningMonitor::evaluate`'s
//! per-device fired-level bookkeeping.
//!
//! Scoped out (task 5.4.2, blocked): `batteryWarningThresholdForDevice`/
//! `batteryWarningThresholdForSelector` and `deviceKey`/`deviceLabel`/`isSystemBattery` (need
//! `UPowerService`/`upowerDeviceMatchesSelector`, task 6.2, not yet ported), and
//! `fireLowBatteryNotification` (needs `NotificationManager`, which doesn't even have a scheduled
//! migration task yet — see MIGRATION_PLAN.md's note under task 6.11). `evaluate` here is
//! dependency-injected instead: the caller supplies each device's already-resolved `key`/
//! `is_system`/`threshold` (what 5.4.2 will compute from a real `UPowerService` + `BatteryConfig`)
//! and a `fire` callback (what 5.4.2 will wire to a real `NotificationManager`) — the escalation
//! bookkeeping itself doesn't need either dependency to be correct or to be tested.

use std::collections::{HashMap, HashSet};

/// Minimal pull-forward of `src/dbus/upower/upower_service.h`'s `BatteryState` — task 6.2
/// (UPower) hasn't landed, so this can't be shared from there yet. `noctalia_shell::hooks::
/// battery` (task 4.3) already pulls forward the identical type for the same reason, but that's a
/// different crate this one can't depend on (shell depends on system, not the reverse), so this
/// is a second, independent duplicate rather than a shared import — same "minimal shared piece,
/// delete once the real task lands" pattern as task 1.6.5's local `generate_uuid_v4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatteryState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
}

/// Port of the free function `isPluggedIn`.
pub fn is_plugged_in(state: BatteryState) -> bool {
    matches!(
        state,
        BatteryState::Charging | BatteryState::FullyCharged | BatteryState::PendingCharge
    )
}

/// Port of `BatteryWarningMonitor::kNoLevel`: the percentage of the deepest alert level already
/// notified this discharge cycle, when none has fired yet (so the next `evaluate` fires whatever
/// level the battery is in).
pub const NO_LEVEL: i32 = 101;

/// Port of the anonymous `alertPointsForDevice`: escalating alert levels (percent), descending.
/// The system battery adds fixed deeper levels at 5% and 2% below the configurable threshold;
/// peripherals warn only at their single configured level. Empty when `threshold <= 0`
/// (warnings disabled for this device).
pub fn alert_points_for_device(threshold: i32, is_system: bool) -> Vec<i32> {
    let mut points = Vec::new();
    if threshold <= 0 {
        return points;
    }
    points.push(threshold);
    if is_system {
        for deep in [5, 2] {
            if deep < threshold {
                points.push(deep);
            }
        }
    }
    points // already descending
}

/// Port of the anonymous `currentLevelFor`: the smallest alert point at or above `percent` (the
/// most severe level entered), or [`NO_LEVEL`] if `percent` is above every alert point.
pub fn current_level_for(points: &[i32], percent: i32) -> i32 {
    let mut level = NO_LEVEL;
    for &point in points {
        // points is descending -> the last qualifying assignment is the smallest.
        if point >= percent {
            level = point;
        }
    }
    level
}

/// One device's live inputs to [`BatteryWarningMonitor::evaluate`], standing in for what the C++
/// reads off a real `UPowerDeviceInfo` plus its own pre-evaluate `isSystem`/threshold resolution
/// (`deviceKey`, `isSystemBattery`, `batteryWarningThresholdForDevice` — all task 5.4.2). `key`
/// must be stable and non-empty for a real device across repeated `evaluate` calls, matching the
/// C++'s `deviceKey` contract (derived there; caller-supplied here since the derivation itself
/// needs `UPowerDeviceInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct BatteryDeviceInput {
    pub key: String,
    pub is_system: bool,
    pub threshold: i32,
    pub is_present: bool,
    pub state: BatteryState,
    pub percentage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceWarningState {
    fired_level: i32,
}

impl Default for DeviceWarningState {
    fn default() -> Self {
        Self {
            fired_level: NO_LEVEL,
        }
    }
}

/// Port of `BatteryWarningMonitor`: per-device fired-level bookkeeping across calls to
/// `evaluate`, so each escalating alert level fires at most once per discharge cycle.
#[derive(Debug, Clone, Default)]
pub struct BatteryWarningMonitor {
    devices: HashMap<String, DeviceWarningState>,
}

impl BatteryWarningMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `BatteryWarningMonitor::evaluate`. Level-triggered: re-evaluates each device's
    /// live inputs and calls `fire(device, level)` at most once per device per call, exactly
    /// where the C++ calls `fireLowBatteryNotification` — the caller supplies the real
    /// notification text/dispatch (task 5.4.2). Safe to call at startup, on UPower changes, and
    /// on config reload; devices no longer present in `devices` are dropped from the internal
    /// bookkeeping, matching the C++'s `m_devices`-pruning loop.
    pub fn evaluate(
        &mut self,
        devices: &[BatteryDeviceInput],
        mut fire: impl FnMut(&BatteryDeviceInput, i32),
    ) {
        let mut seen = HashSet::new();

        for device in devices {
            if device.key.is_empty() {
                continue;
            }
            seen.insert(device.key.clone());

            let state = self.devices.entry(device.key.clone()).or_default();

            // Re-arm when charging, absent, or warnings disabled. fired_level is percentage-
            // based, so a threshold change across config reloads needs no special handling — the
            // next level decides.
            if !device.is_present || is_plugged_in(device.state) || device.threshold <= 0 {
                state.fired_level = NO_LEVEL;
                continue;
            }

            let points = alert_points_for_device(device.threshold, device.is_system);
            let percent = device.percentage.round() as i32;
            let current_level = current_level_for(&points, percent);

            if current_level == NO_LEVEL {
                state.fired_level = NO_LEVEL; // above every alert point
                continue;
            }

            // Level-triggered: fire once per level as the battery drains into it. Empty state on
            // the first evaluate (startup) fires whatever level the battery already sits in —
            // this is a deliberate safety notification at boot, not a baseline state transition.
            if current_level < state.fired_level {
                fire(device, current_level);
                state.fired_level = current_level;
            } else if current_level > state.fired_level {
                state.fired_level = current_level; // partial recharge -> arm so re-dropping re-alerts
            }
        }

        self.devices.retain(|key, _| seen.contains(key));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn device(
        key: &str,
        is_system: bool,
        threshold: i32,
        is_present: bool,
        state: BatteryState,
        percentage: f64,
    ) -> BatteryDeviceInput {
        BatteryDeviceInput {
            key: key.to_string(),
            is_system,
            threshold,
            is_present,
            state,
            percentage,
        }
    }

    #[test]
    fn is_plugged_in_covers_the_three_charging_adjacent_states() {
        assert!(is_plugged_in(BatteryState::Charging));
        assert!(is_plugged_in(BatteryState::FullyCharged));
        assert!(is_plugged_in(BatteryState::PendingCharge));
        assert!(!is_plugged_in(BatteryState::Discharging));
        assert!(!is_plugged_in(BatteryState::PendingDischarge));
        assert!(!is_plugged_in(BatteryState::Empty));
        assert!(!is_plugged_in(BatteryState::Unknown));
    }

    #[test]
    fn alert_points_for_device_disabled_threshold_is_empty() {
        assert_eq!(alert_points_for_device(0, true), Vec::<i32>::new());
        assert_eq!(alert_points_for_device(-5, false), Vec::<i32>::new());
    }

    #[test]
    fn alert_points_for_device_peripheral_gets_only_its_own_threshold() {
        assert_eq!(alert_points_for_device(15, false), vec![15]);
    }

    #[test]
    fn alert_points_for_device_system_battery_adds_deep_levels_below_threshold() {
        assert_eq!(alert_points_for_device(15, true), vec![15, 5, 2]);
    }

    #[test]
    fn alert_points_for_device_deep_levels_only_added_if_strictly_below_threshold() {
        // threshold 5: the "5" deep level is not < 5, so only "2" qualifies.
        assert_eq!(alert_points_for_device(5, true), vec![5, 2]);
        // threshold 2: neither deep level is < 2.
        assert_eq!(alert_points_for_device(2, true), vec![2]);
        // threshold 1: same.
        assert_eq!(alert_points_for_device(1, true), vec![1]);
    }

    #[test]
    fn current_level_for_picks_the_smallest_qualifying_point() {
        let points = vec![15, 5, 2];
        assert_eq!(
            current_level_for(&points, 20),
            NO_LEVEL,
            "above every point"
        );
        assert_eq!(
            current_level_for(&points, 15),
            15,
            "exactly at the shallowest point"
        );
        assert_eq!(
            current_level_for(&points, 10),
            15,
            "between 15 and 5 -> still in the 15 band"
        );
        assert_eq!(current_level_for(&points, 5), 5);
        assert_eq!(
            current_level_for(&points, 3),
            5,
            "between 5 and 2 -> still in the 5 band"
        );
        assert_eq!(current_level_for(&points, 2), 2);
        assert_eq!(
            current_level_for(&points, 0),
            2,
            "at or below the deepest point stays at the deepest"
        );
    }

    #[test]
    fn current_level_for_empty_points_is_always_no_level() {
        assert_eq!(current_level_for(&[], 0), NO_LEVEL);
    }

    #[test]
    fn evaluate_fires_the_level_the_battery_already_sits_in_at_startup() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        let devices = vec![device(
            "bat0",
            true,
            15,
            true,
            BatteryState::Discharging,
            8.0,
        )];

        monitor.evaluate(&devices, |d, level| fired.push((d.key.clone(), level)));

        assert_eq!(
            fired,
            vec![("bat0".to_string(), 15)],
            "a fresh monitor (no prior fired_level) should fire immediately for whatever level \
             the device is already in (8% has dropped to the 15-and-below band), as a boot-time \
             safety notification"
        );
    }

    #[test]
    fn evaluate_fires_each_level_at_most_once_per_discharge() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();

        // Drain from above-threshold down through 15 -> 5 -> 2, one evaluate per percentage.
        for percent in [50.0, 15.0, 15.0, 10.0, 5.0, 5.0, 3.0, 2.0, 2.0, 1.0] {
            let devices = vec![device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                percent,
            )];
            monitor.evaluate(&devices, |d, level| fired.push((d.key.clone(), level)));
        }

        assert_eq!(
            fired,
            vec![
                ("bat0".to_string(), 15),
                ("bat0".to_string(), 5),
                ("bat0".to_string(), 2)
            ],
            "each level should fire exactly once, not once per evaluate call at that level"
        );
    }

    #[test]
    fn evaluate_rearms_on_partial_recharge_so_redropping_refires() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        let mut run = |percent: f64| {
            let devices = vec![device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                percent,
            )];
            monitor.evaluate(&devices, |d, level| fired.push((d.key.clone(), level)));
        };

        run(10.0); // enters the 15 band -> fires 15
        run(20.0); // recharges above every point -> re-arms (fired_level back to NO_LEVEL)
        run(10.0); // drops into the 15 band again -> should fire 15 again

        assert_eq!(
            fired,
            vec![("bat0".to_string(), 15), ("bat0".to_string(), 15)]
        );
    }

    #[test]
    fn evaluate_partial_recharge_within_bands_rearms_only_the_recovered_level() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        let mut run = |percent: f64| {
            let devices = vec![device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                percent,
            )];
            monitor.evaluate(&devices, |d, level| fired.push((d.key.clone(), level)));
        };

        run(3.0); // enters the 5 band -> fires 5 (startup)
        run(10.0); // recovers to the 15 band (level 15 > fired 5) -> arms to 15, no fire
        run(3.0); // drops back into the 5 band (5 < 15) -> fires 5 again

        assert_eq!(
            fired,
            vec![("bat0".to_string(), 5), ("bat0".to_string(), 5)]
        );
    }

    #[test]
    fn evaluate_charging_resets_fired_level_and_never_fires() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();

        monitor.evaluate(
            &[device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                5.0,
            )],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert_eq!(fired.len(), 1, "should fire once while discharging low");

        fired.clear();
        monitor.evaluate(
            &[device("bat0", true, 15, true, BatteryState::Charging, 5.0)],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert!(
            fired.is_empty(),
            "charging should re-arm without firing, even at a low percentage"
        );

        fired.clear();
        monitor.evaluate(
            &[device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                5.0,
            )],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert_eq!(
            fired.len(),
            1,
            "discharging again at the same level after a charge cycle should re-fire"
        );
    }

    #[test]
    fn evaluate_absent_device_resets_fired_level_without_firing() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();

        monitor.evaluate(
            &[device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                5.0,
            )],
            |d, level| fired.push((d.key.clone(), level)),
        );
        fired.clear();

        monitor.evaluate(
            &[device(
                "bat0",
                true,
                15,
                false,
                BatteryState::Discharging,
                5.0,
            )],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert!(fired.is_empty(), "an absent device should never fire");
    }

    #[test]
    fn evaluate_disabled_threshold_never_fires() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        monitor.evaluate(
            &[device(
                "bat0",
                true,
                0,
                true,
                BatteryState::Discharging,
                1.0,
            )],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert!(fired.is_empty());
    }

    #[test]
    fn evaluate_empty_key_is_skipped_and_not_tracked() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        monitor.evaluate(
            &[device("", true, 15, true, BatteryState::Discharging, 1.0)],
            |d, level| fired.push((d.key.clone(), level)),
        );
        assert!(fired.is_empty());
        assert!(monitor.devices.is_empty());
    }

    #[test]
    fn evaluate_prunes_devices_no_longer_seen() {
        let mut monitor = BatteryWarningMonitor::new();
        monitor.evaluate(
            &[device(
                "bat0",
                true,
                15,
                true,
                BatteryState::Discharging,
                5.0,
            )],
            |_, _| {},
        );
        assert!(monitor.devices.contains_key("bat0"));

        // bat0 is absent from this call entirely (e.g. unplugged/removed).
        monitor.evaluate(
            &[device(
                "bat1",
                false,
                20,
                true,
                BatteryState::Discharging,
                5.0,
            )],
            |_, _| {},
        );
        assert!(
            !monitor.devices.contains_key("bat0"),
            "bat0 should be pruned once it's no longer reported"
        );
        assert!(monitor.devices.contains_key("bat1"));
    }

    #[test]
    fn evaluate_tracks_multiple_devices_independently() {
        let mut monitor = BatteryWarningMonitor::new();
        let mut fired = Vec::new();
        let devices = vec![
            device("system", true, 15, true, BatteryState::Discharging, 1.0), // system, deep band
            device("mouse", false, 20, true, BatteryState::Discharging, 15.0), // peripheral, single level
        ];

        monitor.evaluate(&devices, |d, level| fired.push((d.key.clone(), level)));

        fired.sort();
        assert_eq!(
            fired,
            vec![("mouse".to_string(), 20), ("system".to_string(), 2)],
            "each device's own alert points should apply independently"
        );
    }
}
