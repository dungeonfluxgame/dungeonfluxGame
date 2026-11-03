use std::sync::Arc;

use df_table::ports::{Clock, EventSink};
use df_table::testkit::ManualClock;

use crate::event_log::EventLog;

use super::fixtures::{
    boundary_failed, dice_drawn, operation_of, received, spawned, step_finished,
};
use super::{
    EventLevel, HistoryFilter, HostEventSink, Losses, MemoryOutlet, ObservedRecord, OutletRefusal,
    OutletStatus, RecordClass, RecordOutlet,
};

fn sink() -> (HostEventSink, Arc<EventLog>, Arc<ManualClock>) {
    let log = Arc::new(EventLog::default());
    let clock = Arc::new(ManualClock::starting_at(5_000));
    (
        HostEventSink::new(Arc::clone(&log), clock.clone()),
        log,
        clock,
    )
}

/// An outlet that takes nothing, as a full disk would.
#[derive(Default)]
struct FullOutlet {
    losses: Losses,
}

impl RecordOutlet for FullOutlet {
    fn name(&self) -> &'static str {
        "full"
    }

    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletRefusal> {
        self.losses.record(record.class);
        Err(OutletRefusal::Full)
    }

    fn status(&self, _now_ms: u64) -> OutletStatus {
        OutletStatus {
            name: "full".to_owned(),
            accepted: 0,
            confirmed: 0,
            pending: 0,
            lag_ms: 0,
            lost_always: self.losses.always(),
            lost_verbose: self.losses.verbose(),
            evicted: 0,
            unconfirmed: 0,
        }
    }
}

#[test]
fn one_event_is_one_line_and_one_record_made_from_the_same_value() {
    let (sink, log, clock) = sink();
    let event = received(1);

    sink.record(&event);

    let lines = log.newest(10);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, event.render_audit_line());
    let page = sink.query(&HistoryFilter::default(), 10);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].audit_line, lines[0].text);
    assert_eq!(page.records[0].audit_id, 1);
    assert_eq!(page.records[0].observed_ms, clock.now_ms());
}

#[test]
fn class_a_is_never_filtered_and_class_b_is_counted_when_the_level_leaves_it_out() {
    let (sink, log, _clock) = sink();
    let sink = sink.with_level(EventLevel::Always);

    sink.record(&received(1));
    sink.record(&step_finished(1));
    sink.record(&boundary_failed(1));
    sink.record(&step_finished(2));

    let health = sink.health();
    assert_eq!(health.level, "always");
    assert_eq!((health.seen, health.always, health.verbose), (4, 2, 2));
    assert_eq!(health.filtered_verbose, 2);
    assert_eq!(health.retained, 2);
    assert_eq!(log.newest(10).len(), 2);
    let kept = sink.query(&HistoryFilter::default(), 10);
    assert!(
        kept.records
            .iter()
            .all(|record| record.class == RecordClass::Always)
    );

    sink.set_level(EventLevel::Verbose);
    sink.record(&step_finished(3));
    assert_eq!(sink.health().retained, 3);
    assert_eq!(sink.health().filtered_verbose, 2);
}

#[test]
fn audit_ids_run_in_order_with_no_gap_across_every_event_kind() {
    let (sink, _log, _clock) = sink();
    for event in [received(1), spawned(), dice_drawn(1), step_finished(1)] {
        sink.record(&event);
    }

    let ids: Vec<u64> = sink
        .query(&HistoryFilter::default(), 10)
        .records
        .iter()
        .map(|record| record.audit_id)
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[test]
fn the_history_filters_by_area_class_and_operation() {
    let (sink, _log, _clock) = sink();
    for event in [
        received(1),
        step_finished(1),
        received(2),
        spawned(),
        dice_drawn(2),
    ] {
        sink.record(&event);
    }

    let area = HistoryFilter {
        area: Some("input".to_owned()),
        ..HistoryFilter::default()
    };
    assert_eq!(sink.query(&area, 10).matched, 2);
    let verbose = HistoryFilter {
        class: Some(RecordClass::Verbose),
        ..HistoryFilter::default()
    };
    assert_eq!(sink.query(&verbose, 10).records.len(), 1);
    let operation = HistoryFilter {
        operation: Some(operation_of(2)),
        ..HistoryFilter::default()
    };
    let page = sink.query(&operation, 10);
    assert_eq!(page.records.len(), 2);
    assert!(
        page.records
            .iter()
            .all(|record| record.is_about(&operation_of(2)))
    );
}

#[test]
fn a_limit_keeps_the_newest_matches_and_says_how_many_there_were() {
    let (sink, _log, _clock) = sink();
    for turn in 1..=5 {
        sink.record(&received(turn));
    }

    let page = sink.query(&HistoryFilter::default(), 2);

    assert_eq!(page.matched, 5);
    let ids: Vec<u64> = page.records.iter().map(|record| record.audit_id).collect();
    assert_eq!(ids, vec![4, 5]);
}

#[test]
fn the_history_forgets_the_oldest_when_it_is_full_and_counts_what_it_forgot() {
    let log = Arc::new(EventLog::default());
    let sink = HostEventSink::new(log, Arc::new(ManualClock::default()))
        .with_memory(Arc::new(MemoryOutlet::with_bounds(3, usize::MAX)));

    for turn in 1..=5 {
        sink.record(&received(turn));
    }

    let page = sink.query(&HistoryFilter::default(), 10);
    let ids: Vec<u64> = page.records.iter().map(|record| record.audit_id).collect();
    assert_eq!(ids, vec![3, 4, 5]);
    assert_eq!(page.retention.evicted, 2);
    assert_eq!(page.retention.oldest_audit_id, Some(3));
    let health = sink.health();
    assert_eq!((health.retained, health.evicted), (3, 2));
    assert!(
        !health.degraded,
        "keeping within the memory bound is not a loss"
    );
}

#[test]
fn the_history_stays_within_its_byte_bound_but_always_keeps_the_newest_record() {
    let log = Arc::new(EventLog::default());
    let sink = HostEventSink::new(log, Arc::new(ManualClock::default()))
        .with_memory(Arc::new(MemoryOutlet::with_bounds(usize::MAX, 1)));

    sink.record(&received(1));
    sink.record(&received(2));

    let page = sink.query(&HistoryFilter::default(), 10);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].audit_id, 2);
}

#[test]
fn an_outlet_that_refuses_is_counted_by_class_and_the_stream_is_degraded_but_the_rest_is_kept() {
    let log = Arc::new(EventLog::default());
    let sink = HostEventSink::new(Arc::clone(&log), Arc::new(ManualClock::default()))
        .with_outlet(Arc::new(FullOutlet::default()));

    sink.record(&received(1));
    sink.record(&step_finished(1));

    let health = sink.health();
    assert_eq!((health.lost_always, health.lost_verbose), (1, 1));
    assert!(health.degraded);
    assert_eq!(health.outlets.len(), 2);
    assert_eq!(health.outlets[1].name, "full");
    assert_eq!(health.retained, 2, "the history still has both");
    assert_eq!(log.newest(10).len(), 2, "so does the line log");
}

#[test]
fn the_health_of_an_untroubled_sink_has_no_lag_and_is_not_degraded() {
    let (sink, _log, clock) = sink();
    let health = sink.health();
    assert_eq!(health.lag_ms, 0);
    assert!(!health.degraded);
    assert_eq!(health.outlets[0].name, "memory");
    clock.advance_ms(40);
    assert_eq!(sink.health().lag_ms, 0, "memory confirms at once");
}
