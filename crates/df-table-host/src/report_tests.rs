use df_table::kind::Shape;

use crate::event_log::LoggedLine;
use crate::report::{
    MediaListing, describe_done, format_clients, format_events, format_frame,
    format_frame_unchanged, format_host_status, format_kinds, format_media_head,
    format_media_listing, format_timeline, frame_count,
};
use crate::view::{CanvasView, FieldKind, FieldView, KindView};
use crate::wire::EventsReply;

fn view_with_a_kind() -> CanvasView {
    CanvasView {
        revision: 1,
        elements: Vec::new(),
        kinds: vec![KindView {
            name: "character".into(),
            description: "A person.".into(),
            shape: Shape::Circle,
            width_units: 6.0,
            height_units: 6.0,
            fields: vec![
                FieldView {
                    name: "name".into(),
                    kind: FieldKind::Text,
                    description: "text (at most 20)".into(),
                    required: true,
                    default: None,
                },
                FieldView {
                    name: "hit_points".into(),
                    kind: FieldKind::Number,
                    description: "number (0 to 999)".into(),
                    required: false,
                    default: Some(serde_json::json!(10.0)),
                },
            ],
        }],
    }
}

// A refused `Reply` prints as its `DevError::Rejected` (dev_tests).
#[test]
fn a_done_reply_reads_as_ok_with_its_revision_and_notices() {
    assert_eq!(
        describe_done(4, &["element 'pip': it no longer fits".to_owned()]),
        "ok (revision 4)\n  element 'pip': it no longer fits"
    );
}

#[test]
fn kinds_lists_each_field_with_its_description_a_star_for_required_and_its_default() {
    let text = format_kinds(&view_with_a_kind());
    assert!(text.contains("character (Circle 6x6) A person."));
    assert!(text.contains("  name*: text (at most 20)"));
    assert!(text.contains("  hit_points: number (0 to 999) default 10.0"));
}

#[test]
fn with_no_kinds_the_text_says_how_to_add_one() {
    let empty = CanvasView {
        revision: 0,
        elements: Vec::new(),
        kinds: Vec::new(),
    };
    assert!(format_kinds(&empty).contains("add a file to the kind directory"));
}

fn events(sequences: &[u64], latest: u64) -> EventsReply {
    EventsReply {
        events: sequences
            .iter()
            .map(|sequence| LoggedLine {
                sequence: *sequence,
                text: format!("CANVAS event {sequence}"),
                at_ms: 0,
            })
            .collect(),
        latest,
        forgotten: 0,
    }
}

#[test]
fn events_print_with_their_sequence_numbers_oldest_first() {
    let text = format_events(&events(&[1, 2], 2), None);
    assert_eq!(
        text,
        "   1              0  CANVAS event 1\n   2              0  CANVAS event 2"
    );
}

#[test]
fn no_events_says_so() {
    assert_eq!(format_events(&events(&[], 0), Some(0)), "no events");
}

#[test]
fn a_reader_ahead_of_the_newest_sequence_is_told_the_host_restarted() {
    let text = format_events(&events(&[1], 1), Some(500));
    assert!(text.starts_with("(the host has restarted"));
    let normal = format_events(&events(&[2], 2), Some(1));
    assert!(!normal.contains("restarted"));
}

#[test]
fn the_playground_is_printed_one_setting_per_line_with_the_fixtures() {
    let reply = crate::wire::ControlsReply {
        revision: 3,
        boot_id: String::new(),
        controls: df_playground::controls::Controls {
            fixture: Some("a.json".into()),
            ..df_playground::controls::Controls::default()
        },
        scene: df_playground::scene::SceneOverlay::default(),
    };
    let text =
        crate::report::format_playground(&reply, &["a.json".to_owned(), "b.json".to_owned()]);
    assert_eq!(
        text,
        "fixture: a.json\ndevice: both\npace: normal\nstatus: fixture\nunavailable: false\ngrid: false\nsam: true\ntransition: fade 700 ms\nscene: as the fixture has it\nrevision: 3\nfixtures: a.json, b.json"
    );
    let empty = crate::report::format_playground(
        &crate::wire::ControlsReply {
            revision: 0,
            boot_id: String::new(),
            controls: df_playground::controls::Controls::default(),
            scene: df_playground::scene::SceneOverlay::default(),
        },
        &[],
    );
    assert!(empty.contains("fixture: (none)") && empty.contains("fixtures: (none)"));
}

#[test]
fn a_camera_only_move_is_reported_as_the_one_changed_slot() {
    let camera = df_playground::fixture::Camera {
        target: Some("door".to_owned()),
        zoom_percent: 150,
        duration_ms: 400,
        serial: 1,
        began_ms: 0,
        elapsed_ms: 0,
    };
    let reply = crate::wire::ControlsReply {
        revision: 1,
        boot_id: String::new(),
        controls: df_playground::controls::Controls::default(),
        scene: df_playground::scene::SceneOverlay {
            camera: df_playground::overlay::Edit::Set(camera),
            ..df_playground::scene::SceneOverlay::default()
        },
    };
    let text = crate::report::format_playground(&reply, &[]);
    assert!(
        text.lines().any(|line| line == "scene: changed: camera"),
        "{text}"
    );
}

#[test]
fn events_print_the_time_each_line_was_recorded() {
    let reply = EventsReply {
        events: vec![LoggedLine {
            sequence: 7,
            text: "HOST sink_level from=always to=verbose driven=true".to_owned(),
            at_ms: 1_700_000_123_456,
        }],
        latest: 7,
        forgotten: 0,
    };
    assert_eq!(
        format_events(&reply, None),
        "   7  1700000123456  HOST sink_level from=always to=verbose driven=true"
    );
}

#[test]
fn events_say_how_many_lines_the_log_forgot_only_when_it_forgot_some() {
    let mut reply = events(&[3], 3);
    assert!(!format_events(&reply, None).contains("forgotten"));
    reply.forgotten = 12;
    let text = format_events(&reply, None);
    assert_eq!(text.lines().next(), Some("forgotten=12"));
    let mut none = events(&[], 0);
    none.forgotten = 4;
    assert_eq!(format_events(&none, None), "forgotten=4\nno events");
}

fn sink(level: &str) -> crate::observe::SinkHealth {
    crate::observe::SinkHealth {
        level: level.to_owned(),
        seen: 10,
        always: 6,
        verbose: 4,
        filtered_verbose: 2,
        unrenderable: 0,
        retained: 8,
        evicted: 1,
        lag_ms: 250,
        lost_always: 0,
        lost_verbose: 3,
        degraded: false,
        outlets: vec![crate::observe::OutletStatus {
            name: "spool".to_owned(),
            accepted: 6,
            confirmed: 5,
            pending: 1,
            lag_ms: 250,
            lost_always: 0,
            lost_verbose: 3,
            evicted: 0,
            unconfirmed: 0,
        }],
        shared: Some(crate::observe::SharedPathHealth {
            spooled: 6,
            exported: 5,
            dropped: 1,
            lag_ms: 250,
        }),
    }
}

fn host_status() -> crate::server::request::HostStatus {
    use crate::server::request::{ClockStatus, HostStatus, LogStatus};
    HostStatus {
        log: LogStatus {
            latest: 90,
            kept: 80,
            bytes: 4096,
            forgotten: 10,
            echo_pending: 0,
            echo_dropped: 2,
            echo_writer_gone: false,
            undelivered_audit_lines: 5,
        },
        sink: sink("always"),
        clock: ClockStatus {
            ticks: 33,
            moving: 1,
            tick_ms: 50,
        },
        engine_sink: None,
    }
}

#[test]
fn host_status_shows_the_log_the_clock_and_the_loss_and_lag_of_each_sink() {
    let text = format_host_status(&host_status());
    assert!(
        text.contains("log: latest=90 kept=80 bytes=4096 forgotten=10 echo_pending=0"),
        "{text}"
    );
    assert!(text.contains("undelivered_audit_lines=5"), "{text}");
    assert!(
        text.contains("clock: ticks=33 moving=1 tick_ms=50"),
        "{text}"
    );
    assert!(text.contains("sink: level=always seen=10"), "{text}");
    assert!(
        text.contains("lost_always=0 lost_verbose=3 degraded=false"),
        "{text}"
    );
    assert!(
        text.contains("  shared path: spooled=6 exported=5 dropped=1 lag_ms=250"),
        "{text}"
    );
    assert!(
        text.contains("  outlet spool: accepted=6 confirmed=5 pending=1"),
        "{text}"
    );
    assert!(text.contains("engine sink: not running"), "{text}");
}

#[test]
fn host_status_names_a_shared_path_that_is_not_composed_and_shows_the_engine_sink() {
    let mut status = host_status();
    status.sink.shared = None;
    status.engine_sink = Some(sink("verbose"));
    let text = format_host_status(&status);
    assert!(text.contains("  shared path: not composed"), "{text}");
    assert!(text.contains("engine sink: level=verbose"), "{text}");
}

#[test]
fn a_frame_is_summarised_by_count_run_size_and_keys() {
    let body =
        r#"{"frame":12,"canvas":{"revision":3,"elements":[]},"controls":{"boot_id":"run-9"}}"#;
    let text = format_frame(body).expect("a frame");
    assert_eq!(
        text,
        format!(
            "frame: 12\nboot: run-9\nbytes: {}\nkeys: canvas, controls, frame",
            body.len()
        )
    );
    assert_eq!(frame_count(body).expect("a count"), 12);
    assert_eq!(
        format_frame_unchanged(12),
        "frame: unchanged after 12 (no change before the wait ran out)"
    );
}

#[test]
fn a_body_that_is_not_a_frame_is_an_error_not_a_summary() {
    assert!(format_frame("not json").is_err());
    assert!(format_frame("[1]").is_err());
    assert!(format_frame(r#"{"canvas":{}}"#).is_err());
}

#[test]
fn a_media_head_prints_status_type_length_and_tag_or_a_dash() {
    let answer = crate::client::HeadAnswer {
        status: axum::http::StatusCode::OK,
        content_type: Some("image/png".to_owned()),
        content_length: Some(2048),
        entity_tag: Some("\"abc\"".to_owned()),
    };
    assert_eq!(
        format_media_head("x.png", &answer),
        "x.png: status 200 type image/png length 2048 etag \"abc\""
    );
    let missing = crate::client::HeadAnswer {
        status: axum::http::StatusCode::NOT_FOUND,
        content_type: None,
        content_length: None,
        entity_tag: None,
    };
    assert_eq!(
        format_media_head("gone.png", &missing),
        "gone.png: status 404 type - length - etag -"
    );
}

#[test]
fn the_media_listing_prints_the_limits_when_the_host_reports_them() {
    let plain: MediaListing =
        serde_json::from_str(r#"[{"name":"a.png","kind":"image","bytes":5}]"#).expect("a list");
    assert!(format_media_listing(&plain).starts_with("a.png"));
    assert!(!format_media_listing(&plain).contains("limits"));
    let limited: MediaListing = serde_json::from_str(
        r#"{"entries":[{"name":"a.png","kind":"image","bytes":5}],"limits":
        {"used_bytes":5,"files":1,"maximum_file_bytes":100,"maximum_upload_body_bytes":140}}"#,
    )
    .expect("a listing with limits");
    let text = format_media_listing(&limited);
    assert!(
        text.ends_with(
            "limits: used_bytes=5 files=1 maximum_file_bytes=100 maximum_upload_body_bytes=140"
        ),
        "{text}"
    );
}

fn view(client: &str, report: df_playground::media::ClientReport) -> crate::wire::ClientView {
    crate::wire::ClientView {
        client: client.to_owned(),
        age_ms: 1500,
        report,
    }
}

#[test]
fn a_page_prints_its_frame_against_the_hosts_link_revision_and_fixture() {
    use df_playground::media::ClientReport;
    use df_playground::timeline::{FixtureLoadCause, LinkState};
    let report = ClientReport {
        frame: Some(4),
        link: LinkState::Down,
        failures: 3,
        asset_revision: "r1.r2".to_owned(),
        fixture: Some("tavern.json".to_owned()),
        fixture_error: Some(FixtureLoadCause::Invalid),
        ..ClientReport::default()
    };
    let text = format_clients(&[view("page-1", report)], Some(9));
    assert!(
        text.contains("client page-1 (1.5 s ago): audio LOCKED"),
        "{text}"
    );
    assert!(
        text.contains("  frame 4/9 link=down failures=3 rev=r1.r2 fixture=tavern.json (invalid)"),
        "{text}"
    );
    let unknown = format_clients(&[view("page-2", ClientReport::default())], None);
    assert!(unknown.contains("  frame -/? link=up"), "{unknown}");
    assert!(unknown.contains("rev=- fixture=-"), "{unknown}");
}

#[test]
fn a_pages_errors_print_their_message_source_sequence_and_time() {
    use df_playground::media::ClientReport;
    use df_playground::timeline::{ClientError, ErrorSource};
    let report = ClientReport {
        errors: vec![ClientError {
            seq: 6,
            source: ErrorSource::Asset,
            page_ms: 1234,
            message: "could not load ghost.png".to_owned(),
        }],
        ..ClientReport::default()
    };
    let text = format_clients(&[view("page-1", report)], Some(1));
    assert!(
        text.contains("  error: could not load ghost.png (asset #6 at 1234 ms)"),
        "{text}"
    );
}

#[test]
fn a_timeline_prints_start_end_reason_and_planned_time_per_entry() {
    use df_playground::timeline::{
        ClientTimeline, EndReason, TimelineDetail, TimelineEntry, TimelineKind,
    };
    let timeline = ClientTimeline {
        client: "page-1".to_owned(),
        entries: vec![
            TimelineEntry {
                seq: 1,
                kind: TimelineKind::Line,
                id: "line-0badcafe".to_owned(),
                start_ms: 1000,
                end_ms: Some(2500),
                shown_ms: Some(1500),
                planned_ms: Some(4000),
                end_reason: Some(EndReason::Replaced),
                short: true,
                detail: None,
                message: None,
            },
            TimelineEntry {
                seq: 2,
                kind: TimelineKind::Camera,
                id: "3".to_owned(),
                start_ms: 2600,
                end_ms: None,
                shown_ms: None,
                planned_ms: None,
                end_reason: None,
                short: false,
                detail: Some(TimelineDetail::Camera {
                    zoom_percent: 200,
                    allowed_percent: Some(150),
                }),
                message: None,
            },
        ],
    };
    let text = format_timeline(&timeline);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "client page-1: 2 entries (start..end in host ms)");
    assert_eq!(
        lines[1],
        "    1  line   line-0badcafe  1000..2500 shown=1500 planned=4000 reason=replaced short=true"
    );
    assert_eq!(
        lines[2],
        "    2  camera 3  2600.. open planned=- reason=-  zoom=200 allowed=150"
    );
    let empty = ClientTimeline {
        client: "page-1".to_owned(),
        entries: Vec::new(),
    };
    assert_eq!(
        format_timeline(&empty),
        "client page-1: no timeline entries"
    );
}
