//! Port of `src/hooks/battery_hook_state.{h,cpp}`.
//!
//! `BatteryState`/`UPowerState` are minimal pull-forwards of the same-named types declared in
//! `src/dbus/upower/upower_service.h` (task 6.2, D-Bus/UPower, not yet ported) — only the three
//! fields `BatteryHookState::update` actually reads (`state`, `percentage`, `isPresent`), not
//! UPower's whole device model (`energyRate`/`timeToEmpty`/`timeToFull`/`energy`/`onBattery`).
//! Same "minimal shared piece, delete once the real task lands" pattern as task 1.6.5's local
//! `generate_uuid_v4` and task 2.1.3's `KeyChord` POD.

use super::manager::EnvVar;
use noctalia_config::types::hooks::HookKind;

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

#[derive(Debug, Clone, Copy, Default)]
pub struct UPowerState {
    pub percentage: f64,
    pub state: BatteryState,
    pub is_present: bool,
}

/// Port of `BatteryHookState::Event`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: HookKind,
    pub env: Vec<EnvVar>,
}

#[derive(Default)]
pub struct BatteryHookState {
    initialized: bool,
    last_state_hook: Option<HookKind>,
    last_percent: Option<i32>,
}

impl BatteryHookState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self, state: &UPowerState) {
        self.initialized = true;
        self.last_state_hook = battery_state_hook(state.state);
        self.last_percent = state
            .is_present
            .then(|| normalized_battery_percent(state.percentage));
    }

    pub fn update(&mut self, state: &UPowerState) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.initialized {
            self.reset(state);
            return events;
        }

        if !state.is_present {
            self.last_state_hook = None;
            self.last_percent = None;
            return events;
        }

        if let Some(state_hook) = battery_state_hook(state.state) {
            if self.last_state_hook != Some(state_hook) {
                events.push(Event {
                    kind: state_hook,
                    env: Vec::new(),
                });
            }
            self.last_state_hook = Some(state_hook);
        }

        let percent = normalized_battery_percent(state.percentage);
        if let Some(last) = self.last_percent
            && last != percent
        {
            events.push(Event {
                kind: HookKind::BatteryPercentageChanged,
                env: vec![
                    (
                        "NOCTALIA_BATTERY_STATE",
                        battery_state_hook_value(state.state).to_string(),
                    ),
                    ("NOCTALIA_BATTERY_PERCENT", percent.to_string()),
                ],
            });
        }
        self.last_percent = Some(percent);

        events
    }
}

fn battery_state_hook(state: BatteryState) -> Option<HookKind> {
    match state {
        BatteryState::Charging => Some(HookKind::BatteryCharging),
        BatteryState::Discharging => Some(HookKind::BatteryDischarging),
        BatteryState::FullyCharged | BatteryState::PendingCharge => Some(HookKind::BatteryPlugged),
        BatteryState::Unknown | BatteryState::Empty | BatteryState::PendingDischarge => None,
    }
}

fn battery_state_hook_value(state: BatteryState) -> &'static str {
    match state {
        BatteryState::Charging => "charging",
        BatteryState::Discharging => "discharging",
        BatteryState::Empty => "empty",
        BatteryState::FullyCharged => "fully_charged",
        BatteryState::PendingCharge => "pending_charge",
        BatteryState::PendingDischarge => "pending_discharge",
        BatteryState::Unknown => "unknown",
    }
}

fn normalized_battery_percent(percentage: f64) -> i32 {
    if !percentage.is_finite() {
        return 0;
    }
    (percentage.round() as i64).clamp(0, 100) as i32
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn battery_state(state: BatteryState, percent: f64, present: bool) -> UPowerState {
        UPowerState {
            state,
            percentage: percent,
            is_present: present,
        }
    }

    fn env_value<'a>(event: &'a Event, key: &str) -> &'a str {
        event
            .env
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    /// Direct port of `tests/battery_hook_state_test.cpp`'s `main()`.
    #[test]
    fn ported_battery_hook_state_test_cpp() {
        let mut hooks = BatteryHookState::new();
        hooks.reset(&battery_state(BatteryState::Charging, 50.0, true));

        assert!(
            hooks
                .update(&battery_state(BatteryState::Unknown, 50.0, true))
                .is_empty()
        );
        assert!(
            hooks
                .update(&battery_state(BatteryState::Charging, 50.0, true))
                .is_empty()
        );

        let events = hooks.update(&battery_state(BatteryState::Discharging, 50.0, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, HookKind::BatteryDischarging);
        assert!(events[0].env.is_empty());

        assert!(
            hooks
                .update(&battery_state(BatteryState::PendingDischarge, 50.0, true))
                .is_empty()
        );
        assert!(
            hooks
                .update(&battery_state(BatteryState::Discharging, 50.0, true))
                .is_empty()
        );

        let events = hooks.update(&battery_state(BatteryState::Charging, 50.0, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, HookKind::BatteryCharging);
        assert!(events[0].env.is_empty());

        let events = hooks.update(&battery_state(BatteryState::Charging, 51.0, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, HookKind::BatteryPercentageChanged);
        assert_eq!(env_value(&events[0], "NOCTALIA_BATTERY_STATE"), "charging");
        assert_eq!(env_value(&events[0], "NOCTALIA_BATTERY_PERCENT"), "51");

        let events = hooks.update(&battery_state(BatteryState::FullyCharged, 100.0, true));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, HookKind::BatteryPlugged);
        assert!(events[0].env.is_empty());
        assert_eq!(events[1].kind, HookKind::BatteryPercentageChanged);
        assert_eq!(
            env_value(&events[1], "NOCTALIA_BATTERY_STATE"),
            "fully_charged"
        );
        assert_eq!(env_value(&events[1], "NOCTALIA_BATTERY_PERCENT"), "100");
        assert!(
            hooks
                .update(&battery_state(BatteryState::PendingCharge, 100.0, true))
                .is_empty()
        );

        let events = hooks.update(&battery_state(BatteryState::Charging, 100.0, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, HookKind::BatteryCharging);
        assert!(events[0].env.is_empty());

        assert!(
            hooks
                .update(&battery_state(BatteryState::Unknown, 0.0, false))
                .is_empty()
        );
        let events = hooks.update(&battery_state(BatteryState::Discharging, 40.0, true));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, HookKind::BatteryDischarging);
        assert!(events[0].env.is_empty());
    }

    #[test]
    fn normalized_battery_percent_clamps_and_handles_non_finite() {
        assert_eq!(normalized_battery_percent(f64::NAN), 0);
        assert_eq!(normalized_battery_percent(f64::INFINITY), 0);
        assert_eq!(normalized_battery_percent(-5.0), 0);
        assert_eq!(normalized_battery_percent(150.0), 100);
        assert_eq!(normalized_battery_percent(42.6), 43);
    }
}
