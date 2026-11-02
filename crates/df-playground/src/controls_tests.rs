use crate::controls::{Controls, ControlsChange, ControlsError, Device, FIXTURE_STATUS_KEY};
use crate::copy::ReadingPace;
use crate::fixture::Status;

#[test]
fn by_default_both_devices_show_and_the_fixtures_own_status_is_used() {
    let controls = Controls::default();
    assert!(controls.device.shows_tv() && controls.device.shows_phone());
    assert_eq!(controls.status_key, FIXTURE_STATUS_KEY);
    assert_eq!(controls.status_for(&Status::Telling), Status::Telling);
}

#[test]
fn a_chosen_status_replaces_the_fixtures() {
    let controls = Controls {
        status_key: "reconnecting".to_owned(),
        ..Controls::default()
    };
    assert_eq!(controls.status_for(&Status::Ok), Status::Reconnecting);
}

#[test]
fn a_device_key_round_trips_and_an_unknown_key_is_refused() {
    for device in [Device::Both, Device::Tv, Device::Phone] {
        assert_eq!(Device::from_key(device.key()), Some(device));
    }
    assert_eq!(Device::from_key("watch"), None);
}

#[test]
fn each_device_shows_only_its_own_screen() {
    assert!(Device::Tv.shows_tv() && !Device::Tv.shows_phone());
    assert!(Device::Phone.shows_phone() && !Device::Phone.shows_tv());
}

#[test]
fn a_change_is_written_and_read_as_a_change_and_a_value() {
    for (change, text) in [
        (
            ControlsChange::Device(Device::Tv),
            r#"{"change":"device","value":"tv"}"#,
        ),
        (
            ControlsChange::Pace(ReadingPace::Slow),
            r#"{"change":"pace","value":"slow"}"#,
        ),
        (
            ControlsChange::ShowGrid(true),
            r#"{"change":"show_grid","value":true}"#,
        ),
        (
            ControlsChange::Fixture("a.json".into()),
            r#"{"change":"fixture","value":"a.json"}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&change).expect("writes"), text);
        assert_eq!(
            serde_json::from_str::<ControlsChange>(text).expect("reads"),
            change
        );
    }
}

#[test]
fn each_change_sets_its_own_control() {
    let mut controls = Controls::default();
    controls
        .apply(ControlsChange::Fixture("a.json".into()))
        .expect("applies");
    controls
        .apply(ControlsChange::Device(Device::Phone))
        .expect("applies");
    controls
        .apply(ControlsChange::Pace(ReadingPace::Fast))
        .expect("applies");
    controls
        .apply(ControlsChange::Status("rolling".into()))
        .expect("applies");
    controls
        .apply(ControlsChange::ShowUnavailable(true))
        .expect("applies");
    controls
        .apply(ControlsChange::ShowGrid(true))
        .expect("applies");
    assert_eq!(
        controls,
        Controls {
            fixture: Some("a.json".into()),
            device: Device::Phone,
            pace: ReadingPace::Fast,
            status_key: "rolling".into(),
            show_unavailable: true,
            show_grid: true,
        }
    );
}

#[test]
fn a_refused_change_leaves_the_controls_alone_and_says_what_is_allowed() {
    let mut controls = Controls::default();
    let error = controls
        .apply(ControlsChange::Status("sleepy".into()))
        .expect_err("refused");
    assert_eq!(error, ControlsError::UnknownStatus("sleepy".into()));
    assert!(error.to_string().contains("waiting_for"));
    assert_eq!(
        controls.apply(ControlsChange::Fixture(String::new())),
        Err(ControlsError::EmptyFixtureName)
    );
    assert_eq!(controls, Controls::default());
}

#[test]
fn the_fixtures_own_status_can_be_chosen_again() {
    let mut controls = Controls::default();
    controls
        .apply(ControlsChange::Status("rolling".into()))
        .expect("applies");
    controls
        .apply(ControlsChange::Status(FIXTURE_STATUS_KEY.into()))
        .expect("applies");
    assert_eq!(controls.status_for(&Status::Telling), Status::Telling);
}

#[test]
fn controls_with_an_unknown_key_are_refused_when_read() {
    assert!(serde_json::from_str::<Controls>(r#"{"fixture":null,"extra":1}"#).is_err());
}
