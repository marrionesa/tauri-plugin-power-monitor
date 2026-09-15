//! Event names, the internal native event protocol and the state diffing rules.

use crate::models::PowerState;

/// Emitted when the system switches between AC and battery power.
///
/// Payload: the full [`PowerState`] snapshot after the change.
pub(crate) const POWER_SOURCE_CHANGED_EVENT: &str = "power-monitor://power-source-changed";

/// Emitted when the battery percentage, charging state or presence changes.
///
/// Payload: the full [`PowerState`] snapshot after the change.
pub(crate) const BATTERY_CHANGED_EVENT: &str = "power-monitor://battery-changed";

/// Emitted when the system is about to suspend.
pub(crate) const SUSPEND_EVENT: &str = "power-monitor://suspend";

/// Emitted when the system resumed from sleep.
pub(crate) const RESUME_EVENT: &str = "power-monitor://resume";

/// Signals sent by platform listeners to the dispatcher thread.
///
/// Listeners never query the system themselves: they only signal *what kind* of
/// native activity happened. The single dispatcher thread re-queries the
/// backend, diffs the result against the last known state and emits the
/// granular Tauri events. This keeps all query logic in one place and
/// guarantees that native listeners stay trivial and never block on IPC.
pub(crate) enum NativeEvent {
    /// A power-related native signal fired (AC change, battery change, device
    /// hotplug, power settings change, …). The dispatcher re-queries and diffs.
    Requery,
    /// The system is about to suspend.
    Suspend,
    /// The system resumed.
    Resume,
}

/// Sender type for [`NativeEvent`].
pub(crate) type NativeEventSender = std::sync::mpsc::Sender<NativeEvent>;

/// Outcome of comparing a fresh [`PowerState`] against the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateDiff {
    pub power_source_changed: bool,
    pub battery_changed: bool,
}

/// Compares two consecutive power states and decides which events to emit.
///
/// The comparison only takes *notification-relevant* battery fields into
/// account (see [`BatteryInfo::has_notification_relevant_change`] in
/// [`crate::models`]).
pub(crate) fn diff_states(previous: &PowerState, next: &PowerState) -> StateDiff {
    let power_source_changed = previous.power_source != next.power_source;
    let battery_changed = match (&previous.battery, &next.battery) {
        (Some(prev), Some(next)) => prev.has_notification_relevant_change(next),
        (None, None) => false,
        // A battery appeared or disappeared.
        _ => true,
    };
    StateDiff {
        power_source_changed,
        battery_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BatteryInfo;

    fn state(source: PowerSourceForTest, battery: Option<BatteryInfo>) -> PowerState {
        PowerState {
            power_source: source,
            battery,
        }
    }
    use crate::models::PowerSource as PowerSourceForTest;

    fn battery(percentage: Option<u8>, charging: Option<bool>) -> BatteryInfo {
        BatteryInfo {
            is_present: true,
            percentage,
            is_charging: charging,
            time_to_empty: None,
            time_to_full: None,
        }
    }

    #[test]
    fn identical_states_produce_no_events() {
        let prev = state(
            PowerSourceForTest::Battery,
            Some(battery(Some(50), Some(false))),
        );
        let next = prev.clone();
        let diff = diff_states(&prev, &next);
        assert!(!diff.power_source_changed);
        assert!(!diff.battery_changed);
    }

    #[test]
    fn power_source_change_is_detected() {
        let prev = state(PowerSourceForTest::Ac, Some(battery(Some(50), Some(false))));
        let next = state(
            PowerSourceForTest::Battery,
            Some(battery(Some(50), Some(false))),
        );
        let diff = diff_states(&prev, &next);
        assert!(diff.power_source_changed);
        assert!(!diff.battery_changed);
    }

    #[test]
    fn percentage_change_only_emits_battery_event() {
        let prev = state(
            PowerSourceForTest::Battery,
            Some(battery(Some(50), Some(false))),
        );
        let next = state(
            PowerSourceForTest::Battery,
            Some(battery(Some(49), Some(false))),
        );
        let diff = diff_states(&prev, &next);
        assert!(!diff.power_source_changed);
        assert!(diff.battery_changed);
    }

    #[test]
    fn charging_state_change_only_emits_battery_event() {
        let prev = state(PowerSourceForTest::Ac, Some(battery(Some(50), Some(false))));
        let next = state(PowerSourceForTest::Ac, Some(battery(Some(50), Some(true))));
        let diff = diff_states(&prev, &next);
        assert!(!diff.power_source_changed);
        assert!(diff.battery_changed);
    }

    #[test]
    fn time_estimate_changes_are_ignored() {
        let mut prev_bat = battery(Some(50), Some(false));
        prev_bat.time_to_empty = Some(3600);
        let mut next_bat = battery(Some(50), Some(false));
        next_bat.time_to_empty = Some(3540);
        let prev = state(PowerSourceForTest::Battery, Some(prev_bat));
        let next = state(PowerSourceForTest::Battery, Some(next_bat));
        let diff = diff_states(&prev, &next);
        assert!(!diff.power_source_changed);
        assert!(!diff.battery_changed);
    }

    #[test]
    fn battery_appearing_and_disappearing_is_detected() {
        let prev = state(PowerSourceForTest::Ac, None);
        let next = state(PowerSourceForTest::Ac, Some(battery(Some(60), None)));
        assert!(diff_states(&prev, &next).battery_changed);
        assert!(diff_states(&next, &prev).battery_changed);
    }

    #[test]
    fn battery_without_info_changes_is_silent_across_source_changes() {
        let prev = state(PowerSourceForTest::Ac, None);
        let next = state(PowerSourceForTest::Battery, None);
        let diff = diff_states(&prev, &next);
        assert!(diff.power_source_changed);
        assert!(!diff.battery_changed);
    }
}
