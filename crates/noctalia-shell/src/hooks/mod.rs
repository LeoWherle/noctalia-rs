//! Shell event hooks — src: `src/hooks/*` (task 4.3).

mod battery;
mod manager;

pub use battery::{BatteryHookState, BatteryState, Event, UPowerState};
pub use manager::{CommandRunner, EnvVar, HookManager};
