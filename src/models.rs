//! Data model shared by all platform backends and exposed over IPC.

use serde::{Deserialize, Serialize};

/// The source the system is currently drawing power from.
///
/// Serialized as `"ac"`, `"battery"` or `"unknown"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    /// External power (AC adapter / USB-C power).
    Ac,
    /// Internal battery.
    Battery,
    /// The system could not determine the power source.
    Unknown,
}

impl PowerSource {
    /// Stable string representation used in the serialized JSON model.
    pub const fn as_str(self) -> &'static str {
        match self {
            PowerSource::Ac => "ac",
            PowerSource::Battery => "battery",
            PowerSource::Unknown => "unknown",
        }
    }
}

/// Battery information.
///
/// Fields are [`Option`]s because platforms differ in what they can report.
/// Absence of data is never silently converted into `false` or `0`; see the
/// platform notes in the README for the exact guarantees per OS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryInfo {
    /// Whether a battery is present in the system.
    pub is_present: bool,
    /// Battery charge percentage (`0..=100`), or `None` when unknown.
    pub percentage: Option<u8>,
    /// Whether the battery is currently charging, or `None` when unknown.
    pub is_charging: Option<bool>,
    /// Estimated seconds until the battery is empty, or `None` when unknown.
    pub time_to_empty: Option<u64>,
    /// Estimated seconds until the battery is fully charged, or `None` when unknown.
    pub time_to_full: Option<u64>,
}

impl BatteryInfo {
    /// Whether a state transition should trigger a `battery-changed` event.
    ///
    /// Only *notification-relevant* fields count: presence, percentage and
    /// charging state. Time estimates (`time_to_empty` / `time_to_full`) change
    /// frequently while discharging (roughly once per minute on some platforms)
    /// and would flood the event channel; they are part of the state payload,
    /// but not of the notification surface.
    pub(crate) fn has_notification_relevant_change(&self, other: &Self) -> bool {
        self.is_present != other.is_present
            || self.percentage != other.percentage
            || self.is_charging != other.is_charging
    }
}

/// A full snapshot of the system's power state.
///
/// This is the payload of the [`get_power_state`](crate::init) IPC command and
/// of the `power-source-changed` / `battery-changed` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerState {
    /// The power source the system is currently running on.
    pub power_source: PowerSource,
    /// Battery details, or `None` when no battery is present.
    pub battery: Option<BatteryInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_source_serializes_to_lowercase_strings() {
        assert_eq!(serde_json::to_string(&PowerSource::Ac).unwrap(), "\"ac\"");
        assert_eq!(
            serde_json::to_string(&PowerSource::Battery).unwrap(),
            "\"battery\""
        );
        assert_eq!(
            serde_json::to_string(&PowerSource::Unknown).unwrap(),
            "\"unknown\""
        );
    }

    #[test]
    fn power_source_as_str_round_trips() {
        for source in [PowerSource::Ac, PowerSource::Battery, PowerSource::Unknown] {
            let parsed = serde_json::from_str::<PowerSource>(&format!("\"{}\"", source.as_str()));
            assert_eq!(parsed.unwrap(), source);
        }
    }

    #[test]
    fn battery_info_serializes_with_camel_case_and_nulls() {
        let info = BatteryInfo {
            is_present: true,
            percentage: Some(87),
            is_charging: Some(true),
            time_to_empty: None,
            time_to_full: Some(2340),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "isPresent": true,
                "percentage": 87,
                "isCharging": true,
                "timeToEmpty": null,
                "timeToFull": 2340
            })
        );
    }

    #[test]
    fn power_state_serializes_with_camel_case() {
        let state = PowerState {
            power_source: PowerSource::Ac,
            battery: None,
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "powerSource": "ac", "battery": null })
        );
    }

    #[test]
    fn battery_info_round_trips_through_json() {
        let state = PowerState {
            power_source: PowerSource::Battery,
            battery: Some(BatteryInfo {
                is_present: true,
                percentage: Some(42),
                is_charging: None,
                time_to_empty: Some(3600),
                time_to_full: None,
            }),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: PowerState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn notification_relevant_change_ignores_time_estimates() {
        let a = BatteryInfo {
            is_present: true,
            percentage: Some(80),
            is_charging: Some(false),
            time_to_empty: Some(3600),
            time_to_full: None,
        };
        let mut b = a.clone();
        b.time_to_empty = Some(3540);
        assert!(!a.has_notification_relevant_change(&b));

        b.percentage = Some(79);
        assert!(a.has_notification_relevant_change(&b));

        let mut c = a.clone();
        c.is_charging = Some(true);
        assert!(a.has_notification_relevant_change(&c));

        let mut d = a.clone();
        d.is_present = false;
        assert!(a.has_notification_relevant_change(&d));
    }
}
