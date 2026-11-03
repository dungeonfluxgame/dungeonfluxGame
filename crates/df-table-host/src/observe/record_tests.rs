use df_table::event::TableEvent;

use super::fixtures::{
    boundary_failed, dice_drawn, operation_of, received, spawned, step_finished,
};
use super::record::{AttributeValue, ObservedRecord, RecordClass, RecordDraft, RecordSeverity};

fn stamped(event: &TableEvent) -> ObservedRecord {
    ObservedRecord::stamp(RecordDraft::from_event(event).expect("a record"), 7, 1_000)
}

fn attribute<'a>(record: &'a ObservedRecord, name: &str) -> Option<&'a AttributeValue> {
    record
        .attributes
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value)
}

fn text(record: &ObservedRecord, name: &str) -> String {
    match attribute(record, name) {
        Some(AttributeValue::Text(text)) => text.clone(),
        other => panic!("{name} should be text, found {other:?}"),
    }
}

fn whole(record: &ObservedRecord, name: &str) -> i64 {
    match attribute(record, name) {
        Some(AttributeValue::Whole(whole)) => *whole,
        other => panic!("{name} should be a whole number, found {other:?}"),
    }
}

#[test]
fn a_record_carries_the_events_own_line_and_its_typed_fields_from_one_value() {
    let event = received(3);
    let record = stamped(&event);

    assert_eq!(record.audit_line, event.render_audit_line());
    assert_eq!(record.area, "INPUT");
    assert_eq!(record.event_name, "table.input.received");
    assert_eq!(record.kind, "input_received");
    assert_eq!(text(&record, "length"), "short");
    assert_eq!(text(&record, "df.area"), "INPUT");
    assert_eq!(text(&record, "df.event"), "input_received");
    assert_eq!(whole(&record, "df.audit_id"), 7);
}

#[test]
fn the_correlation_and_the_table_scope_are_lifted_into_the_attributes_the_store_indexes() {
    let record = stamped(&received(3));

    assert_eq!(text(&record, "df.operation"), operation_of(3));
    assert_eq!(whole(&record, "df.turn_seq"), 3);
    assert_eq!(whole(&record, "df.revision.epoch"), 1);
    assert_eq!(whole(&record, "df.revision.sequence"), 3);
    assert_eq!(whole(&record, "df.table"), 1);
    assert_eq!(whole(&record, "df.scene"), 2);
    assert_eq!(text(&record, "df.machine"), "explore");
    assert_eq!(
        attribute(&record, "df.correlated"),
        Some(&AttributeValue::Flag(true))
    );
    assert!(record.is_about(&operation_of(3)));
    assert!(!record.is_about(&operation_of(4)));
    // Lifted, not repeated as nested fields of the event.
    assert!(attribute(&record, "correlation.operation_id").is_none());
    assert!(attribute(&record, "scope.table").is_none());
}

#[test]
fn an_event_with_no_correlation_says_so_and_borrows_none() {
    let record = stamped(&spawned());

    assert_eq!(
        attribute(&record, "df.correlated"),
        Some(&AttributeValue::Flag(false))
    );
    assert!(attribute(&record, "df.operation").is_none());
    assert!(attribute(&record, "df.turn_seq").is_none());
    assert!(record.correlation.is_none());
    assert_eq!(text(&record, "element"), "pip");
    assert_eq!(whole(&record, "revision"), 4);
}

#[test]
fn a_nested_family_keeps_its_variant_and_its_correlation() {
    let record = stamped(&dice_drawn(5));

    assert_eq!(record.event_name, "table.pipeline");
    assert_eq!(record.kind, "pipeline.dice_drawn");
    assert_eq!(text(&record, "df.event"), "pipeline.dice_drawn");
    assert_eq!(whole(&record, "value"), 12);
    assert_eq!(text(&record, "df.operation"), operation_of(5));
}

#[test]
fn class_and_severity_follow_the_event() {
    assert_eq!(stamped(&received(1)).class, RecordClass::Always);
    assert_eq!(stamped(&received(1)).severity, RecordSeverity::Info);
    assert_eq!(stamped(&step_finished(1)).class, RecordClass::Verbose);
    assert_eq!(stamped(&step_finished(1)).severity, RecordSeverity::Debug);
    assert_eq!(stamped(&boundary_failed(1)).severity, RecordSeverity::Error);
    assert_eq!(text(&stamped(&boundary_failed(1)), "cause"), "unavailable");
}

#[test]
fn a_record_costs_more_than_its_line_so_the_byte_bound_is_not_undercounted() {
    let record = stamped(&received(1));
    assert!(record.approximate_bytes() > record.audit_line.len());
}
