use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::config::{
    LARGE_SETTING_MAXIMUM, LoadedSettings, SettingFault, SettingId, SettingName, SettingOwner,
    SettingUnit, Settings,
};
use crate::event::TableEvent;

fn configuration(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("a test configuration is an object, not {other}"),
    }
}

fn complete_configuration() -> Value {
    json!({
        "narration_late_s": 10,
        "narration_batch_s": 5,
        "seat_pending_limit": 4,
        "seat_rate_burst": 6,
        "seat_rate_sustained_per_s": 2,
    })
}

fn load(raw: Value) -> LoadedSettings {
    Settings::load(&configuration(raw)).expect("the crate's own setting names are valid")
}

fn rejection(name: &str, cause: SettingFault) -> TableEvent {
    TableEvent::SettingRejected {
        name: SettingName::new(name).expect("valid setting name"),
        cause,
    }
}

#[test]
fn the_defaults_are_the_numbers_of_section_j4() {
    let settings = Settings::defaults();
    assert_eq!(settings.narration_late_s(), 8);
    assert_eq!(settings.narration_batch_s(), 4);
    assert_eq!(settings.seat_pending_limit(), 3);
    assert_eq!(settings.seat_rate_burst(), 3);
    assert_eq!(settings.seat_rate_sustained_per_s(), 1);
}

#[test]
fn every_default_lies_inside_its_own_range() {
    for id in SettingId::ALL {
        let spec = id.spec();
        assert!(spec.range.minimum <= spec.range.maximum, "{}", spec.name);
        assert!(spec.range.contains(spec.default), "{}", spec.name);
    }
}

#[test]
fn setting_names_are_valid_unique_and_carry_their_unit() {
    let mut seen = BTreeSet::new();
    for id in SettingId::ALL {
        let spec = id.spec();
        assert!(SettingName::new(spec.name).is_ok(), "{}", spec.name);
        assert!(seen.insert(spec.name), "{} is listed twice", spec.name);
        match spec.unit {
            SettingUnit::Seconds => assert!(spec.name.ends_with("_s"), "{}", spec.name),
            SettingUnit::PerSecond => assert!(spec.name.ends_with("_per_s"), "{}", spec.name),
            SettingUnit::Count => {
                assert!(!spec.name.ends_with("_s"), "{}", spec.name);
                assert!(!spec.name.ends_with("_ms"), "{}", spec.name);
            }
        }
    }
}

#[test]
fn every_seeded_setting_is_owned_by_the_launcher_settings() {
    for id in SettingId::ALL {
        assert_eq!(id.spec().owner, SettingOwner::LauncherSettings);
    }
}

#[test]
fn a_complete_valid_configuration_loads_every_value_and_reports_nothing() {
    let loaded = load(complete_configuration());
    assert!(loaded.events.is_empty());
    assert_eq!(loaded.settings.narration_late_s(), 10);
    assert_eq!(loaded.settings.narration_batch_s(), 5);
    assert_eq!(loaded.settings.seat_pending_limit(), 4);
    assert_eq!(loaded.settings.seat_rate_burst(), 6);
    assert_eq!(loaded.settings.seat_rate_sustained_per_s(), 2);
}

#[test]
fn an_out_of_range_value_falls_back_to_the_default_and_is_reported() {
    let mut raw = complete_configuration();
    raw["narration_late_s"] = json!(0);
    raw["seat_pending_limit"] = json!(LARGE_SETTING_MAXIMUM + 1);
    let loaded = load(raw);
    assert_eq!(loaded.settings.narration_late_s(), 8);
    assert_eq!(loaded.settings.seat_pending_limit(), 3);
    assert_eq!(loaded.settings.narration_batch_s(), 5);
    assert_eq!(
        loaded.events,
        [
            rejection("narration_late_s", SettingFault::OutOfRange),
            rejection("seat_pending_limit", SettingFault::OutOfRange),
        ]
    );
}

#[test]
fn a_value_that_is_not_a_whole_number_falls_back_and_is_reported() {
    for bad in [
        json!(2.5),
        json!(3.0),
        json!(-1),
        json!("4"),
        json!(null),
        json!(true),
    ] {
        let mut raw = complete_configuration();
        raw["narration_batch_s"] = bad.clone();
        let loaded = load(raw);
        assert_eq!(loaded.settings.narration_batch_s(), 4, "{bad}");
        assert_eq!(
            loaded.events,
            [rejection(
                "narration_batch_s",
                SettingFault::NotAWholeNumber
            )],
            "{bad}"
        );
    }
}

#[test]
fn a_number_too_large_for_a_setting_is_out_of_range_not_a_wrap() {
    let mut raw = complete_configuration();
    raw["seat_rate_burst"] = json!(u64::from(u32::MAX) + 4);
    let loaded = load(raw);
    assert_eq!(loaded.settings.seat_rate_burst(), 3);
    assert_eq!(
        loaded.events,
        [rejection("seat_rate_burst", SettingFault::OutOfRange)]
    );
}

#[test]
fn a_missing_value_takes_the_default_and_is_reported() {
    let loaded = load(json!({}));
    assert_eq!(loaded.settings, Settings::defaults());
    assert_eq!(loaded.events.len(), SettingId::ALL.len());
    assert_eq!(
        loaded.events[0],
        rejection("narration_late_s", SettingFault::Missing)
    );
}

#[test]
fn unknown_names_are_reported_in_key_order_and_change_nothing() {
    let mut raw = complete_configuration();
    raw["narration_late"] = json!(5);
    raw["narration_early"] = json!(5);
    let loaded = load(raw);
    assert_eq!(loaded.settings.narration_late_s(), 10);
    assert_eq!(
        loaded.events,
        [
            rejection("narration_early", SettingFault::UnknownSetting),
            rejection("narration_late", SettingFault::UnknownSetting),
        ]
    );
}

#[test]
fn keys_that_are_not_valid_names_are_counted_not_named() {
    let mut raw = complete_configuration();
    raw["Bad Name"] = json!(1);
    raw["ANOTHER"] = json!(2);
    let loaded = load(raw);
    assert_eq!(loaded.events, [TableEvent::SettingKeysIgnored { count: 2 }]);
    assert_eq!(
        loaded.events[0].render_audit_line(),
        "SETTING keys_ignored count=2"
    );
}

#[test]
fn a_late_limit_below_the_batch_window_puts_both_back_to_their_defaults() {
    let mut raw = complete_configuration();
    raw["narration_late_s"] = json!(3);
    raw["narration_batch_s"] = json!(20);
    let loaded = load(raw);
    assert_eq!(loaded.settings.narration_late_s(), 8);
    assert_eq!(loaded.settings.narration_batch_s(), 4);
    assert_eq!(
        loaded.events,
        [
            rejection("narration_late_s", SettingFault::InconsistentWith),
            rejection("narration_batch_s", SettingFault::InconsistentWith),
        ]
    );
}

#[test]
fn a_late_limit_equal_to_the_batch_window_is_consistent() {
    let mut raw = complete_configuration();
    raw["narration_late_s"] = json!(5);
    raw["narration_batch_s"] = json!(5);
    let loaded = load(raw);
    assert!(loaded.events.is_empty());
    assert_eq!(loaded.settings.narration_late_s(), 5);
}

#[test]
fn a_batch_window_above_the_default_late_limit_is_judged_after_the_fallback() {
    let mut raw = complete_configuration();
    raw["narration_late_s"] = json!(0);
    raw["narration_batch_s"] = json!(30);
    let loaded = load(raw);
    assert_eq!(loaded.settings.narration_late_s(), 8);
    assert_eq!(loaded.settings.narration_batch_s(), 4);
    assert_eq!(
        loaded.events,
        [
            rejection("narration_late_s", SettingFault::OutOfRange),
            rejection("narration_late_s", SettingFault::InconsistentWith),
            rejection("narration_batch_s", SettingFault::InconsistentWith),
        ]
    );
}

#[test]
fn the_rate_settings_are_independent_because_j4_states_no_relation_between_them() {
    let mut raw = complete_configuration();
    raw["seat_rate_burst"] = json!(1);
    raw["seat_rate_sustained_per_s"] = json!(5);
    let loaded = load(raw);
    assert!(loaded.events.is_empty());
    assert_eq!(loaded.settings.seat_rate_burst(), 1);
    assert_eq!(loaded.settings.seat_rate_sustained_per_s(), 5);
}

#[test]
fn a_rejected_setting_is_one_audit_line_with_its_name_and_cause() {
    let event = rejection("narration_late_s", SettingFault::OutOfRange);
    assert_eq!(
        event.render_audit_line(),
        "SETTING rejected name=narration_late_s cause=out_of_range"
    );
}

#[test]
fn the_fault_keys_are_distinct_and_are_their_serialized_names() {
    let faults = [
        SettingFault::Missing,
        SettingFault::NotAWholeNumber,
        SettingFault::OutOfRange,
        SettingFault::UnknownSetting,
        SettingFault::InconsistentWith,
    ];
    let keys: BTreeSet<_> = faults.iter().map(|fault| fault.key()).collect();
    assert_eq!(keys.len(), faults.len());
    for fault in faults {
        assert_eq!(
            serde_json::to_string(&fault).expect("a fault serializes"),
            format!("\"{}\"", fault.key())
        );
    }
}
