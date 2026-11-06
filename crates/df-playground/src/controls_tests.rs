use crate::controls::{
    Controls, ControlsChange, ControlsChangeReply, ControlsError, ControlsReply, Device,
    FIXTURE_STATUS_KEY, Transition, TransitionStyle, ZoomLimit,
};
use crate::copy::ReadingPace;
use crate::fixture::Status;
use crate::fixtures::assert_keys_are_serde_names;
use crate::limits::MAXIMUM_TRANSITION_MS;

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
            show_sam: true,
            transition: Transition::default(),
            zoom_limit: ZoomLimit::default(),
        }
    );
}

#[test]
fn the_sam_layer_is_shown_by_default_and_toggles() {
    let mut controls = Controls::default();
    assert!(controls.show_sam);
    controls
        .apply(ControlsChange::ShowSam(false))
        .expect("applies");
    assert!(!controls.show_sam);
    // A controls record written before the layer existed reads as shown.
    let old: Controls = serde_json::from_str(
        r#"{"fixture":null,"device":"both","pace":"normal","status_key":"fixture","show_unavailable":false,"show_grid":false}"#,
    )
    .expect("reads");
    assert!(old.show_sam);
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

#[test]
fn a_transition_defaults_to_a_short_fade_and_each_part_is_set_on_its_own() {
    let mut controls = Controls::default();
    assert_eq!(
        controls.transition,
        Transition {
            style: TransitionStyle::Fade,
            duration_ms: 700
        }
    );
    controls
        .apply(ControlsChange::TransitionStyle(TransitionStyle::Wipe))
        .expect("applies");
    controls
        .apply(ControlsChange::TransitionMs(1200))
        .expect("applies");
    assert_eq!(
        controls.transition,
        Transition {
            style: TransitionStyle::Wipe,
            duration_ms: 1200
        }
    );
}

#[test]
fn a_transition_may_be_instant_or_as_long_as_the_limit_and_no_longer() {
    let mut controls = Controls::default();
    assert!(controls.apply(ControlsChange::TransitionMs(0)).is_ok());
    assert!(
        controls
            .apply(ControlsChange::TransitionMs(MAXIMUM_TRANSITION_MS))
            .is_ok()
    );
    let before = controls.clone();
    let error = controls
        .apply(ControlsChange::TransitionMs(MAXIMUM_TRANSITION_MS + 1))
        .expect_err("refused");
    assert_eq!(
        error,
        ControlsError::TransitionTooLong(MAXIMUM_TRANSITION_MS + 1)
    );
    assert!(
        error
            .to_string()
            .contains(&MAXIMUM_TRANSITION_MS.to_string())
    );
    assert_eq!(controls, before);
}

#[test]
fn transition_styles_round_trip_by_key_and_by_json() {
    for style in [
        TransitionStyle::Cut,
        TransitionStyle::Fade,
        TransitionStyle::Wipe,
    ] {
        assert_eq!(TransitionStyle::from_key(style.key()), Some(style));
    }
    assert_eq!(TransitionStyle::from_key("spin"), None);
    let text = serde_json::to_string(&ControlsChange::TransitionStyle(TransitionStyle::Wipe))
        .expect("writes");
    assert_eq!(text, r#"{"change":"transition_style","value":"wipe"}"#);
    let old = concat!(
        r#"{"fixture":null,"device":"both","pace":"normal","status_key":"fixture","#,
        r#""show_unavailable":false,"show_grid":false}"#
    );
    assert_eq!(
        serde_json::from_str::<Controls>(old)
            .expect("an older controls text still reads")
            .transition,
        Transition::default()
    );
}

#[test]
fn the_keys_of_a_device_and_a_transition_style_are_their_serde_names() {
    assert_keys_are_serde_names(&Device::ALL, Device::key, Device::from_key);
    assert_keys_are_serde_names(
        &TransitionStyle::ALL,
        TransitionStyle::key,
        TransitionStyle::from_key,
    );
    assert_keys_are_serde_names(&ZoomLimit::ALL, ZoomLimit::key, ZoomLimit::from_key);
}

#[test]
fn the_zoom_limit_is_the_model_by_default_is_set_on_its_own_and_reads_from_older_records() {
    let mut controls = Controls::default();
    assert_eq!(controls.zoom_limit, ZoomLimit::Model);
    controls
        .apply(ControlsChange::ZoomLimit(ZoomLimit::Pixels))
        .expect("applies");
    assert_eq!(controls.zoom_limit, ZoomLimit::Pixels);
    assert_eq!(ZoomLimit::from_key("sharp"), None);
    let text =
        serde_json::to_string(&ControlsChange::ZoomLimit(ZoomLimit::Pixels)).expect("writes");
    assert_eq!(text, r#"{"change":"zoom_limit","value":"pixels"}"#);
    let old = concat!(
        r#"{"fixture":null,"device":"both","pace":"normal","status_key":"fixture","#,
        r#""show_unavailable":false,"show_grid":false}"#
    );
    assert_eq!(
        serde_json::from_str::<Controls>(old)
            .expect("reads")
            .zoom_limit,
        ZoomLimit::Model
    );
}

#[test]
fn a_change_reply_reads_the_controls_when_done_and_the_hosts_sentence_when_rejected() {
    let controls = ControlsReply {
        revision: 7,
        boot_id: "run".to_owned(),
        controls: Controls::default(),
        scene: Default::default(),
    };
    let done = serde_json::json!({"outcome": "done", "value": controls});
    let read: ControlsChangeReply = serde_json::from_value(done).expect("a done reply reads");
    assert_eq!(read.into_result(), Ok(controls));
    let rejected =
        r#"{"outcome":"rejected","error":{"error":"invalid_change","detail":"no"},"message":"no"}"#;
    let read: ControlsChangeReply = serde_json::from_str(rejected).expect("a refusal reads");
    assert_eq!(read.into_result(), Err("no".to_owned()));
    // The guard in front of every route refuses with the host's general reply.
    let guarded = r#"{"outcome":"rejected","cause":null,"message":"not here","problems":[]}"#;
    let read: ControlsChangeReply = serde_json::from_str(guarded).expect("a guard refusal reads");
    assert_eq!(read.into_result(), Err("not here".to_owned()));
}

#[test]
fn a_bare_controls_reply_is_not_a_change_reply() {
    let bare = serde_json::to_string(&ControlsReply {
        revision: 1,
        boot_id: String::new(),
        controls: Controls::default(),
        scene: Default::default(),
    })
    .expect("writes");
    assert!(serde_json::from_str::<ControlsChangeReply>(&bare).is_err());
}
