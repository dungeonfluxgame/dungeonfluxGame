use std::sync::Arc;
use std::time::Duration;

use df_table::ports::EventSink;
use df_table::testkit::ManualClock;
use tempfile::TempDir;

use crate::event_log::EventLog;

use super::fixtures::{operation_of, received, step_finished};
use super::{FileSpool, HostEventSink, ObservedRecord, OutletRefusal, RecordDraft, RecordOutlet};

const SETTLE: Duration = Duration::from_secs(10);

fn record(event: &df_table::event::TableEvent, audit_id: u64, observed_ms: u64) -> ObservedRecord {
    ObservedRecord::stamp(
        RecordDraft::from_event(event).expect("a record"),
        audit_id,
        observed_ms,
    )
}

fn sink_with(spool: &Arc<FileSpool>) -> HostEventSink {
    HostEventSink::new(
        Arc::new(EventLog::default()),
        Arc::new(ManualClock::starting_at(2_000)),
    )
    .with_outlet(Arc::clone(spool) as Arc<dyn RecordOutlet>)
}

#[test]
fn records_are_written_as_json_lines_in_order_with_both_renderings_and_confirmed() {
    let directory = TempDir::new().expect("a directory");
    let path = directory.path().join("events.jsonl");
    let spool = Arc::new(FileSpool::open(&path).expect("the spool opens"));
    let sink = sink_with(&spool);
    let events = [received(1), step_finished(1), received(2)];

    for event in &events {
        sink.record(event);
    }
    assert!(spool.settle(SETTLE), "the writer finishes");

    let text = std::fs::read_to_string(&path).expect("the spool is a file");
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    assert_eq!(lines.len(), 3);
    let ids: Vec<u64> = lines
        .iter()
        .filter_map(|line| line["audit_id"].as_u64())
        .collect();
    assert_eq!(ids, vec![1, 2, 3]);
    for (line, event) in lines.iter().zip(&events) {
        assert_eq!(line["audit_line"], event.render_audit_line());
    }
    assert_eq!(lines[0]["correlation"]["operation"], operation_of(1));
    assert_eq!(lines[1]["class"], "verbose");
    let status = spool.status(2_000);
    assert_eq!(
        (status.accepted, status.confirmed, status.pending),
        (3, 3, 0)
    );
    assert_eq!(
        (status.lost_always, status.lost_verbose, status.lag_ms),
        (0, 0, 0)
    );
    assert!(!sink.health().degraded);
}

#[test]
fn a_record_that_does_not_fit_the_spool_is_lost_and_counted_by_class_and_nothing_is_deleted() {
    let directory = TempDir::new().expect("a directory");
    let path = directory.path().join("events.jsonl");
    let spool = Arc::new(FileSpool::open_with_bounds(&path, 1, usize::MAX).expect("opens"));
    let sink = sink_with(&spool);

    sink.record(&received(1));
    sink.record(&step_finished(1));
    assert!(spool.settle(SETTLE));

    let status = spool.status(2_000);
    assert_eq!((status.lost_always, status.lost_verbose), (1, 1));
    assert_eq!(status.confirmed, 0);
    assert_eq!(std::fs::metadata(&path).expect("the file exists").len(), 0);
    let health = sink.health();
    assert_eq!((health.lost_always, health.lost_verbose), (1, 1));
    assert!(health.degraded, "a class A loss is never a complete corpus");
}

#[test]
fn what_was_already_in_the_file_counts_toward_the_size_bound() {
    let directory = TempDir::new().expect("a directory");
    let path = directory.path().join("events.jsonl");
    std::fs::write(&path, vec![b'x'; 100]).expect("a file");
    let spool = FileSpool::open_with_bounds(&path, 100, usize::MAX).expect("opens");

    spool.accept(&record(&received(1), 1, 10)).expect("queued");
    assert!(spool.settle(SETTLE));

    assert_eq!(spool.status(10).lost_always, 1);
    assert_eq!(
        std::fs::metadata(&path).expect("the file exists").len(),
        100
    );
}

#[test]
fn a_full_queue_refuses_at_once_counts_the_loss_and_never_blocks() {
    let spool = FileSpool::without_writer(1);

    let refusal = spool.accept(&record(&received(1), 1, 10));

    assert_eq!(refusal, Err(OutletRefusal::Full));
    let status = spool.status(10);
    assert_eq!(
        (status.accepted, status.pending, status.lost_always),
        (0, 0, 1)
    );
}

#[test]
fn records_waiting_for_the_writer_show_their_age_as_lag_until_it_catches_up() {
    let spool = FileSpool::without_writer(usize::MAX);
    spool
        .accept(&record(&received(1), 1, 1_000))
        .expect("queued");
    spool
        .accept(&record(&step_finished(1), 2, 1_200))
        .expect("queued");

    let status = spool.status(1_500);

    assert_eq!(
        (status.accepted, status.confirmed, status.pending),
        (2, 0, 2)
    );
    assert_eq!(
        status.lag_ms, 500,
        "the oldest waiting record is 500 ms old"
    );
    assert_eq!(
        spool.status(1_000).lag_ms,
        0,
        "a clock behind the record reads no lag"
    );
}

#[test]
fn a_spool_that_cannot_be_opened_is_a_typed_error() {
    let directory = TempDir::new().expect("a directory");
    let path = directory.path().join("missing").join("events.jsonl");

    let error = FileSpool::open(&path).err().expect("no such directory");

    assert!(matches!(error, super::SpoolError::Open { .. }), "{error}");
    assert!(error.to_string().contains("cannot be opened"), "{error}");
}

#[test]
fn a_status_read_while_records_are_arriving_never_counts_a_record_twice_or_not_at_all() {
    let directory = TempDir::new().expect("a directory");
    let spool = Arc::new(FileSpool::open(&directory.path().join("events.jsonl")).expect("opens"));
    let feeder = {
        let spool = Arc::clone(&spool);
        std::thread::spawn(move || {
            for number in 0..2_000_u64 {
                spool
                    .accept(&record(&received(number), number + 1, 1_000))
                    .expect("accepted");
            }
        })
    };
    while !feeder.is_finished() {
        let status = spool.status(1_000);
        assert_eq!(
            status.accepted,
            status.confirmed + status.pending + status.lost_always + status.lost_verbose,
            "accepted is every record, each one confirmed, waiting or lost: {status:?}"
        );
    }
    feeder.join().expect("the feeder ends");
    assert!(spool.settle(SETTLE), "the writer finishes");
    let status = spool.status(1_000);
    assert_eq!(
        (status.accepted, status.confirmed, status.pending),
        (2_000, 2_000, 0)
    );
}

#[test]
fn the_identity_holds_on_every_read_while_records_are_refused_and_lost_by_the_writer() {
    let directory = TempDir::new().expect("a directory");
    let line = serde_json::to_vec(&record(&received(0), 1, 1_000))
        .expect("a line")
        .len()
        + 1;
    // Room for three lines on disk and for two waiting: later records are lost by the writer, and
    // while it is busy more are refused at the door.
    let spool = Arc::new(
        FileSpool::open_with_bounds(
            &directory.path().join("events.jsonl"),
            3 * line as u64,
            2 * line,
        )
        .expect("opens"),
    );
    let feeders: Vec<_> = (0..2_u64)
        .map(|feeder| {
            let spool = Arc::clone(&spool);
            std::thread::spawn(move || {
                for number in 0..1_000_u64 {
                    // A refusal is the point of the test: it is counted, not handled.
                    let _ = spool.accept(&record(&received(number), number + 1, 1_000 + feeder));
                }
            })
        })
        .collect();
    while feeders.iter().any(|feeder| !feeder.is_finished()) {
        let (status, refused) = spool.status_with_refusals(1_000);
        assert_eq!(
            status.accepted + refused,
            status.confirmed + status.pending + status.lost_always + status.lost_verbose,
            "every offered record is confirmed, waiting, lost by the writer or refused: {status:?} refused={refused}"
        );
    }
    for feeder in feeders {
        feeder.join().expect("the feeder ends");
    }
    assert!(spool.settle(SETTLE), "the writer finishes");
    let (status, refused) = spool.status_with_refusals(1_000);
    assert_eq!(status.pending, 0);
    assert_eq!(
        status.accepted + refused,
        2_000,
        "every record was offered once"
    );
    assert_eq!(
        status.accepted + refused,
        status.confirmed + status.lost_always + status.lost_verbose
    );
    assert!(
        status.confirmed <= 3,
        "the file holds three lines: {status:?}"
    );
    assert!(
        status.lost_always + status.lost_verbose > refused,
        "the writer lost records of its own: {status:?} refused={refused}"
    );
}
