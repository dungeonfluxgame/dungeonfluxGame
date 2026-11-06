use crate::controls::TransitionStyle;
use crate::copy::{ReadingPace, caption_dwell_ms, word_count};
use crate::fixture::Fixture;
use crate::fixtures::assert_keys_are_serde_names;
use crate::limits::{MAXIMUM_REPORTED_ERRORS, MAXIMUM_TIMELINE_RECORDS};
use crate::timeline::{
    EndReason, ErrorSource, FixtureLoadCause, LinkState, PageLog, Showing, ShownCamera,
    TimelineDetail, TimelineKind, TimelineRecord, line_id,
};

fn showing(scene: &str, lines: &[&str], plates: &[&str], figures: &[&str]) -> Showing {
    Showing {
        scene: Some(scene.to_owned()),
        lines: lines.iter().map(|text| (line_id(text), 1_800)).collect(),
        plates: plates
            .iter()
            .map(|name| ((*name).to_owned(), None))
            .collect(),
        figures: figures.iter().map(|id| (*id).to_owned()).collect(),
        ..Showing::default()
    }
}

/// The newest state of each record not yet reported.
fn records(log: &PageLog) -> Vec<TimelineRecord> {
    log.unreported().0
}

fn of_kind(log: &PageLog, kind: TimelineKind) -> Vec<TimelineRecord> {
    records(log)
        .into_iter()
        .filter(|record| record.kind == kind)
        .collect()
}

#[test]
fn a_replaced_line_ends_when_the_next_one_starts() {
    let mut log = PageLog::default();
    log.observe(showing("tavern.json", &["Hello there."], &[], &[]), 1_000);
    log.observe(showing("tavern.json", &["Who goes?"], &[], &[]), 4_000);
    let lines = of_kind(&log, TimelineKind::Line);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].id, line_id("Hello there."));
    assert_eq!(lines[0].start_page_ms, 1_000);
    assert_eq!(lines[0].end_page_ms, Some(4_000));
    assert_eq!(lines[0].end_reason, Some(EndReason::Replaced));
    assert_eq!(lines[1].start_page_ms, 4_000);
    assert_eq!(lines[1].end_page_ms, None, "the new line is still up");
    log.observe(showing("tavern.json", &[], &[], &[]), 6_000);
    let lines = of_kind(&log, TimelineKind::Line);
    assert_eq!(lines[1].end_reason, Some(EndReason::Removed));
}

#[test]
fn a_line_still_shown_is_not_opened_again() {
    let mut log = PageLog::default();
    log.observe(
        showing("tavern.json", &["Hello."], &["tavern.png"], &["a"]),
        10,
    );
    log.observe(
        showing("tavern.json", &["Hello."], &["tavern.png"], &["a"]),
        20,
    );
    assert_eq!(
        records(&log).len(),
        4,
        "a scene, a line, a plate and a figure"
    );
    assert!(
        records(&log)
            .iter()
            .all(|record| record.end_page_ms.is_none())
    );
}

#[test]
fn a_scene_change_ends_everything_shown_and_names_the_scene_before() {
    let mut log = PageLog::default();
    log.observe(
        showing("tavern.json", &["Hello."], &["tavern.png"], &["a"]),
        100,
    );
    let mut next = showing("road.json", &["Hello."], &["road.png"], &["a"]);
    next.transition = Some(TransitionStyle::Fade);
    log.observe(next, 900);
    let ended: Vec<_> = records(&log)
        .into_iter()
        .filter(|record| record.end_page_ms.is_some())
        .collect();
    assert_eq!(ended.len(), 4, "{ended:?}");
    for record in &ended {
        let expected = if record.kind == TimelineKind::Scene {
            EndReason::Replaced
        } else {
            EndReason::SceneChange
        };
        assert_eq!(record.end_reason, Some(expected), "{record:?}");
        assert_eq!(record.end_page_ms, Some(900));
    }
    let scenes = of_kind(&log, TimelineKind::Scene);
    assert_eq!(scenes[1].id, "road.json");
    assert_eq!(
        scenes[1].detail,
        Some(TimelineDetail::Scene {
            from: Some("tavern.json".to_owned()),
            transition: Some(TransitionStyle::Fade),
        })
    );
    let lines = of_kind(&log, TimelineKind::Line);
    assert_eq!(
        lines.len(),
        2,
        "the same words in a new scene are a new line"
    );
}

#[test]
fn a_removed_figure_ends_and_the_others_stay() {
    let mut log = PageLog::default();
    log.observe(showing("s.json", &[], &[], &["a", "b"]), 0);
    log.observe(showing("s.json", &[], &[], &["a"]), 50);
    let figures = of_kind(&log, TimelineKind::Figure);
    let b = figures.iter().find(|record| record.id == "b").expect("b");
    assert_eq!(b.end_reason, Some(EndReason::Removed));
    let a = figures.iter().find(|record| record.id == "a").expect("a");
    assert_eq!(a.end_page_ms, None);
}

#[test]
fn a_new_camera_serial_retargets_and_a_clamp_is_a_record_not_a_console_warning() {
    let mut log = PageLog::default();
    let mut first = showing("s.json", &[], &[], &[]);
    first.camera = Some(ShownCamera {
        serial: 1,
        zoom_percent: 300,
        duration_ms: 1_200,
    });
    log.observe(first.clone(), 0);
    log.note_camera_clamp(1, 300, 180, 5);
    let mut second = first;
    second.camera = Some(ShownCamera {
        serial: 2,
        zoom_percent: 150,
        duration_ms: 800,
    });
    log.observe(second, 2_000);
    let cameras = of_kind(&log, TimelineKind::Camera);
    assert_eq!(cameras.len(), 2);
    assert_eq!(cameras[0].id, "1");
    assert_eq!(cameras[0].planned_ms, Some(1_200));
    assert_eq!(
        cameras[0].detail,
        Some(TimelineDetail::Camera {
            zoom_percent: 300,
            allowed_percent: Some(180),
        })
    );
    assert_eq!(cameras[0].end_reason, Some(EndReason::Retargeted));
    assert_eq!(cameras[1].id, "2");
}

#[test]
fn a_clamp_with_no_camera_record_opens_one() {
    let mut log = PageLog::default();
    log.note_camera_clamp(9, 500, 200, 40);
    let cameras = of_kind(&log, TimelineKind::Camera);
    assert_eq!(cameras.len(), 1);
    assert_eq!(cameras[0].start_page_ms, 40);
}

#[test]
fn the_unreported_ring_is_bounded_and_keeps_the_newest() {
    let mut log = PageLog::default();
    for index in 0..MAXIMUM_TIMELINE_RECORDS + 10 {
        log.open(TimelineKind::Sound, "bell.wav", index as u64, None, None);
    }
    assert_eq!(log.unreported_len(), MAXIMUM_TIMELINE_RECORDS);
    let kept = records(&log);
    assert_eq!(kept[0].start_page_ms, 10, "the oldest went first");
}

#[test]
fn a_delivered_report_forgets_what_it_carried_and_keeps_what_came_after() {
    let mut log = PageLog::default();
    let first = log.open(TimelineKind::Sound, "bell.wav", 0, None, None);
    let (carried, through) = log.unreported();
    assert_eq!(carried.len(), 1);
    log.close(first, 300, EndReason::Ended);
    log.acknowledge(through);
    let left = records(&log);
    assert_eq!(left.len(), 1, "the close came after the report was made");
    assert_eq!(left[0].end_reason, Some(EndReason::Ended));
    let (_, through) = log.unreported();
    log.acknowledge(through);
    assert_eq!(log.unreported_len(), 0);
}

#[test]
fn a_record_opened_and_closed_between_reports_is_sent_once_closed() {
    let mut log = PageLog::default();
    let seq = log.open(TimelineKind::Music, "song.ogg", 10, None, None);
    log.close(seq, 70, EndReason::Replaced);
    log.close(seq, 90, EndReason::Removed);
    let sent = records(&log);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].end_page_ms,
        Some(70),
        "a second close changes nothing"
    );
    assert_eq!(sent[0].end_reason, Some(EndReason::Replaced));
}

#[test]
fn a_draw_record_carries_the_stage_moves_since_the_last_one() {
    let mut log = PageLog::default();
    log.note_stage_move();
    log.note_stage_move();
    log.note_draw(Some(4), false, vec!["tv-caption".to_owned()], None, 500);
    log.note_draw(Some(5), true, Vec::new(), Some(TransitionStyle::Cut), 600);
    let draws = of_kind(&log, TimelineKind::Draw);
    assert_eq!(draws.len(), 2);
    assert_eq!(draws[0].id, "parts");
    assert_eq!(draws[0].end_page_ms, Some(500));
    assert_eq!(
        draws[0].detail,
        Some(TimelineDetail::Draw {
            frame: Some(4),
            devices: false,
            parts: vec!["tv-caption".to_owned()],
            transition: None,
            moved: 2,
        })
    );
    assert!(matches!(
        draws[1].detail,
        Some(TimelineDetail::Draw {
            moved: 0,
            devices: true,
            ..
        })
    ));
}

#[test]
fn a_panic_is_reported_at_once_and_other_errors_wait() {
    let mut log = PageLog::default();
    assert!(!log.note_error(ErrorSource::Audio, "blocked", 1));
    assert!(log.note_error(ErrorSource::Panic, "playground panic", 2));
    assert!(ErrorSource::Panic.forces_report());
    let errors = log.errors();
    assert_eq!(errors.len(), 2);
    assert!(errors[0].seq < errors[1].seq);
    assert_eq!(errors[1].source, ErrorSource::Panic);
    assert_eq!(errors[1].page_ms, 2);
}

#[test]
fn a_repeated_error_is_kept_once_and_the_errors_are_bounded() {
    let mut log = PageLog::default();
    log.note_error(ErrorSource::Net, "down", 1);
    log.note_error(ErrorSource::Net, "down", 2);
    assert_eq!(log.errors().len(), 1);
    for index in 0..MAXIMUM_REPORTED_ERRORS + 3 {
        log.note_error(ErrorSource::Draw, &format!("error {index}"), 3);
    }
    let errors = log.errors();
    assert_eq!(errors.len(), MAXIMUM_REPORTED_ERRORS);
    assert_eq!(
        errors.last().map(|error| error.message.as_str()),
        Some(format!("error {}", MAXIMUM_REPORTED_ERRORS + 2).as_str())
    );
}

#[test]
fn what_the_shared_screen_shows_is_named_without_its_words() {
    let fixture = Fixture::parse(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},
            "caption":{"text":"The door creaks open slowly"},
            "backdrop":{"media":"hall.mp4"},"card":{"media":"card.png"},
            "elements":[]},"phone":{"hero":"H","status":{"kind":"ok"}}}"#,
    )
    .expect("reads");
    let shown = Showing::of(
        Some("hall.json"),
        Some(&fixture),
        None,
        ReadingPace::Slow,
        true,
    );
    let text = "The door creaks open slowly";
    assert_eq!(
        shown.lines.get(&line_id(text)).copied(),
        Some(caption_dwell_ms(word_count(text), ReadingPace::Slow))
    );
    assert!(line_id(text).starts_with("line-") && !line_id(text).contains("door"));
    assert!(
        shown.clips.contains_key("hall.mp4"),
        "a moving backdrop is a clip"
    );
    assert!(shown.plates.contains_key("card.png"));
    let phone_only = Showing::of(
        Some("hall.json"),
        Some(&fixture),
        None,
        ReadingPace::Slow,
        false,
    );
    assert!(phone_only.lines.is_empty() && phone_only.plates.is_empty());
    assert_eq!(phone_only.scene.as_deref(), Some("hall.json"));
}

#[test]
fn the_keys_of_the_timeline_enums_are_their_serde_names() {
    assert_keys_are_serde_names(
        &TimelineKind::ALL,
        TimelineKind::key,
        TimelineKind::from_key,
    );
    assert_keys_are_serde_names(&EndReason::ALL, EndReason::key, EndReason::from_key);
    assert_keys_are_serde_names(&ErrorSource::ALL, ErrorSource::key, ErrorSource::from_key);
    assert_keys_are_serde_names(&LinkState::ALL, LinkState::key, LinkState::from_key);
    assert_keys_are_serde_names(
        &FixtureLoadCause::ALL,
        FixtureLoadCause::key,
        FixtureLoadCause::from_key,
    );
}
