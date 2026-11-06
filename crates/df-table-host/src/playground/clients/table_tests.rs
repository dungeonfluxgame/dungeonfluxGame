//! The client table: which reports write which lines, and the timeline it keeps.

use df_playground::controls::{Device, TransitionStyle};
use df_playground::limits::MAXIMUM_KEPT_TIMELINE_RECORDS;
use df_playground::media::{AudioState, ClientReport};
use df_playground::timeline::{
    ClientError, EndReason, ErrorSource, FixtureLoadCause, LinkState, TimelineDetail, TimelineKind,
    TimelineQuery, TimelineRecord, ViewportPx,
};

use super::ClientTable;
use crate::limits::{FORGET_CLIENT_AFTER, MAXIMUM_CLIENTS};

fn report() -> ClientReport {
    ClientReport {
        asset_revision: "00aa.00bb".to_owned(),
        device: Device::Tv,
        viewport_px: Some(ViewportPx {
            width: 1920,
            height: 1080,
        }),
        ..ClientReport::default()
    }
}

fn record(seq: u64, kind: TimelineKind, id: &str, start: u64) -> TimelineRecord {
    TimelineRecord {
        seq,
        kind,
        id: id.to_owned(),
        start_page_ms: start,
        end_page_ms: None,
        end_reason: None,
        planned_ms: None,
        detail: None,
    }
}

fn closed(mut record: TimelineRecord, end: u64, reason: EndReason) -> TimelineRecord {
    record.end_page_ms = Some(end);
    record.end_reason = Some(reason);
    record
}

/// A table that has heard `page-1` once, at host time 1 000, and the lines of that first report.
fn heard() -> (ClientTable, Vec<String>) {
    let table = ClientTable::default();
    let recorded = table.record("page-1".to_owned(), report(), 1_000);
    (table, recorded.lines)
}

#[test]
fn a_new_page_is_seen_with_its_device_and_its_revision_is_checked() {
    let table = ClientTable::default();
    let recorded = table.record("page-1".to_owned(), report(), 1_000);
    assert_eq!(
        recorded.lines,
        ["CLIENT seen client=page-1 device=tv viewport=1920x1080"]
    );
    assert_eq!(recorded.check_revision.as_deref(), Some("00aa.00bb"));
}

#[test]
fn two_identical_heartbeats_write_no_line() {
    let (table, _) = heard();
    let recorded = table.record("page-1".to_owned(), report(), 11_000);
    assert!(recorded.lines.is_empty(), "{:?}", recorded.lines);
    assert_eq!(recorded.check_revision, None);
    assert_eq!(table.views_at(11_000)[0].age_ms, 0);
}

#[test]
fn a_revision_change_writes_one_line_and_asks_for_a_check() {
    let (table, _) = heard();
    let changed = ClientReport {
        asset_revision: "00cc.00bb".to_owned(),
        ..report()
    };
    let recorded = table.record("page-1".to_owned(), changed, 2_000);
    assert_eq!(
        recorded.lines,
        ["CLIENT revision client=page-1 from=00aa.00bb to=00cc.00bb"]
    );
    assert_eq!(recorded.check_revision.as_deref(), Some("00cc.00bb"));
}

#[test]
fn the_link_the_fixture_and_the_sound_lock_each_write_a_line_when_they_change() {
    let (table, _) = heard();
    let down = ClientReport {
        link: LinkState::Down,
        failures: 3,
        fixture: Some("tavern.json".to_owned()),
        fixture_error: Some(FixtureLoadCause::Invalid),
        audio: AudioState::Unlocked,
        ..report()
    };
    let lines = table.record("page-1".to_owned(), down, 2_000).lines;
    assert_eq!(
        lines,
        [
            "CLIENT link client=page-1 state=down failures=3",
            "CLIENT fixture client=page-1 name=tavern.json result=invalid",
            "CLIENT audio client=page-1 state=unlocked at_ms=2000",
        ]
    );
}

#[test]
fn an_error_is_written_once_with_its_length_and_never_its_words() {
    let (table, _) = heard();
    let with_error = ClientReport {
        errors: vec![ClientError {
            seq: 4,
            source: ErrorSource::Draw,
            page_ms: 500,
            message: "could not draw the playground: secret words".to_owned(),
        }],
        page_clock_ms: 1_000,
        ..report()
    };
    let lines = table
        .record("page-1".to_owned(), with_error.clone(), 5_000)
        .lines;
    assert_eq!(
        lines,
        ["CLIENT error client=page-1 seq=4 source=draw at_ms=4500 chars=43"]
    );
    assert!(
        table
            .record("page-1".to_owned(), with_error, 6_000)
            .lines
            .is_empty()
    );
    let timeline = table
        .timeline("page-1", TimelineQuery::default())
        .expect("open");
    assert_eq!(timeline.entries[0].kind, TimelineKind::Error);
    assert_eq!(
        timeline.entries[0].message.as_deref(),
        Some("could not draw the playground: secret words"),
        "the words stay in the table"
    );
}

#[test]
fn a_line_closed_before_its_dwell_is_timed_on_the_host_clock_and_marked_short() {
    let (table, _) = heard();
    let mut line = record(7, TimelineKind::Line, "line-0000beef", 200);
    line.planned_ms = Some(1_800);
    let opened = ClientReport {
        timeline: vec![line.clone()],
        page_clock_ms: 1_000,
        ..report()
    };
    // The page's clock read 1 000 when the host's read 10 000: page time + 9 000 is host time.
    assert!(
        table
            .record("page-1".to_owned(), opened, 10_000)
            .lines
            .is_empty(),
        "a line writes when it ends"
    );
    let ended = ClientReport {
        timeline: vec![closed(line, 1_200, EndReason::Replaced)],
        page_clock_ms: 2_000,
        ..report()
    };
    let lines = table.record("page-1".to_owned(), ended, 11_000).lines;
    assert_eq!(
        lines,
        [concat!(
            "TIMING line client=page-1 id=line-0000beef start_ms=9200 end_ms=10200 ",
            "shown_ms=1000 planned_ms=1800 reason=replaced short=true"
        )]
    );
    let entry = &table
        .timeline("page-1", TimelineQuery::default())
        .expect("open")
        .entries[0];
    assert_eq!(
        (entry.start_ms, entry.end_ms, entry.short),
        (9_200, Some(10_200), true)
    );
}

#[test]
fn a_record_opened_and_closed_between_reports_writes_its_start_and_its_end() {
    let (table, _) = heard();
    let sound = closed(
        record(3, TimelineKind::Sound, "bell.wav", 100),
        900,
        EndReason::Ended,
    );
    let report = ClientReport {
        timeline: vec![sound],
        page_clock_ms: 1_000,
        ..report()
    };
    let lines = table.record("page-1".to_owned(), report, 2_000).lines;
    assert_eq!(
        lines,
        [
            "CLIENT sound client=page-1 media=bell.wav action=started at_ms=1100",
            "CLIENT sound client=page-1 media=bell.wav action=ended at_ms=1900",
        ]
    );
}

#[test]
fn a_scene_a_camera_and_a_draw_write_their_lines() {
    let (table, _) = heard();
    let mut scene = record(1, TimelineKind::Scene, "road.json", 0);
    scene.detail = Some(TimelineDetail::Scene {
        from: Some("tavern.json".to_owned()),
        transition: Some(TransitionStyle::Fade),
    });
    let mut camera = record(2, TimelineKind::Camera, "5", 0);
    camera.detail = Some(TimelineDetail::Camera {
        zoom_percent: 300,
        allowed_percent: Some(180),
    });
    let mut draw = closed(
        record(3, TimelineKind::Draw, "devices", 10),
        10,
        EndReason::Ended,
    );
    draw.end_reason = None;
    draw.detail = Some(TimelineDetail::Draw {
        frame: Some(12),
        devices: true,
        parts: Vec::new(),
        transition: Some(TransitionStyle::Fade),
        moved: 4,
    });
    let report = ClientReport {
        timeline: vec![
            closed(scene, 5_000, EndReason::Replaced),
            closed(camera, 700, EndReason::Retargeted),
            draw,
        ],
        page_clock_ms: 0,
        ..report()
    };
    let lines = table.record("page-1".to_owned(), report, 0).lines;
    assert_eq!(
        lines,
        [
            concat!(
                "TIMING scene client=page-1 from=tavern.json to=road.json transition=fade ",
                "start_ms=0 end_ms=5000 reason=replaced"
            ),
            concat!(
                "CLIENT camera client=page-1 serial=5 zoom=300 allowed=180 start_ms=0 ",
                "end_ms=700 reason=retargeted"
            ),
            concat!(
                "CLIENT draw client=page-1 seq=3 frame=12 at_ms=10 devices=true parts=- ",
                "transition=fade moved=4"
            ),
        ]
    );
}

#[test]
fn an_id_that_is_not_a_plain_name_is_written_as_a_dash() {
    let (table, _) = heard();
    let figure = closed(
        record(9, TimelineKind::Figure, "the goblin/king", 0),
        10,
        EndReason::Removed,
    );
    let report = ClientReport {
        timeline: vec![figure],
        ..report()
    };
    let lines = table.record("page-1".to_owned(), report, 0).lines;
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains(" id=- "), "{}", lines[0]);
    assert!(!lines[0].contains("goblin"));
}

#[test]
fn a_silent_page_is_gone_and_a_page_past_the_maximum_evicts_the_oldest() {
    let (table, _) = heard();
    let forget_after_ms = u64::try_from(FORGET_CLIENT_AFTER.as_millis()).expect("fits");
    assert!(table.sweep(1_000 + forget_after_ms - 1).is_empty());
    assert_eq!(
        table.sweep(1_000 + forget_after_ms),
        [format!(
            "CLIENT gone client=page-1 silent_ms={forget_after_ms}"
        )]
    );
    assert!(table.views_at(1_000 + forget_after_ms).is_empty());
    // A page a read forgets keeps its line for the next sweep.
    let (table, _) = heard();
    assert!(table.views_at(1_000 + forget_after_ms).is_empty());
    assert_eq!(table.sweep(1_000 + forget_after_ms).len(), 1);
    assert!(table.sweep(1_000 + forget_after_ms).is_empty(), "once");
    let table = ClientTable::default();
    for index in 0..MAXIMUM_CLIENTS as u64 {
        table.record(format!("page-{index}"), report(), index);
    }
    let lines = table
        .record("page-new".to_owned(), report(), MAXIMUM_CLIENTS as u64)
        .lines;
    assert_eq!(lines[0], "CLIENT evicted client=page-0");
    assert_eq!(
        table.views_at(MAXIMUM_CLIENTS as u64).len(),
        MAXIMUM_CLIENTS
    );
}

#[test]
fn the_timeline_reads_by_kind_and_after_a_sequence_and_is_bounded() {
    let (table, _) = heard();
    let records = (1..=MAXIMUM_KEPT_TIMELINE_RECORDS as u64 + 3)
        .map(|seq| {
            let kind = if seq % 2 == 0 {
                TimelineKind::Figure
            } else {
                TimelineKind::Plate
            };
            record(seq, kind, "a.png", seq)
        })
        .collect::<Vec<_>>();
    for chunk in records.chunks(df_playground::limits::MAXIMUM_TIMELINE_RECORDS) {
        let report = ClientReport {
            timeline: chunk.to_vec(),
            ..report()
        };
        table.record("page-1".to_owned(), report, 2_000);
    }
    let all = table
        .timeline("page-1", TimelineQuery::default())
        .expect("open");
    assert_eq!(all.entries.len(), MAXIMUM_KEPT_TIMELINE_RECORDS);
    assert_eq!(all.entries[0].seq, 4, "the oldest go first");
    let plates = table
        .timeline(
            "page-1",
            TimelineQuery {
                kind: Some(TimelineKind::Plate),
                after: Some(MAXIMUM_KEPT_TIMELINE_RECORDS as u64),
            },
        )
        .expect("open");
    assert!(
        plates
            .entries
            .iter()
            .all(|entry| entry.kind == TimelineKind::Plate)
    );
    assert_eq!(plates.entries.len(), 2);
    assert!(table.timeline("page-2", TimelineQuery::default()).is_none());
}
