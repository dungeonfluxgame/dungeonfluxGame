#![cfg(not(target_arch = "wasm32"))]

use df_observe::{FixtureTelemetry, OperationContext};

fn context(build: &str) -> OperationContext {
    OperationContext {
        trace_parent: "00-11111111111111111111111111111111-2222222222222222-01".to_owned(),
        build: build.to_owned(),
    }
}

#[test]
fn instances_keep_spans_on_their_own_exporters() {
    let first_telemetry = FixtureTelemetry::default();
    let mut pending_a = first_telemetry.begin(&context("build-a"), "a.pending");
    first_telemetry.record(&context("build-a"), "a.before", "complete", 3);
    let mut finished_a = first_telemetry.begin(&context("build-a"), "a.finished-once");
    finished_a.finish("complete", 5);
    finished_a.finish("duplicate-finish", 9);
    drop(first_telemetry.begin(&context("build-a"), "a.abandoned"));
    let second_telemetry = FixtureTelemetry::default();
    second_telemetry.record(&context("build-b"), "b.only", "complete", 7);
    pending_a.finish_unmeasured("cancelled");
    drop(pending_a);

    assert_eq!(first_telemetry.count().unwrap(), 4);
    assert_eq!(second_telemetry.count().unwrap(), 1);
    let a_snapshot = first_telemetry.snapshot().unwrap();
    assert!(a_snapshot.contains("a.before"));
    assert!(a_snapshot.contains("a.pending"));
    assert!(a_snapshot.contains("build-a"));
    assert!(a_snapshot.contains("parent=2222222222222222"));
    assert!(a_snapshot.contains("abandoned"));
    assert!(a_snapshot.contains("cancelled"));
    assert!(!a_snapshot.contains("I64(9)"));
    assert!(!a_snapshot.contains("b.only"));
    let b_snapshot = second_telemetry.snapshot().unwrap();
    assert!(b_snapshot.contains("b.only"));
    assert!(b_snapshot.contains("build-b"));
    assert!(b_snapshot.contains("parent=2222222222222222"));
    assert!(!b_snapshot.contains("a."));

    first_telemetry.shutdown().unwrap();
    drop(first_telemetry);
    second_telemetry.record(&context("build-b"), "b.after-a-drop", "complete", 1);
    assert_eq!(second_telemetry.count().unwrap(), 2);
    assert!(
        second_telemetry
            .snapshot()
            .unwrap()
            .contains("b.after-a-drop")
    );
}

#[test]
fn fixture_exporter_retains_latest_512_and_counts_evictions() {
    let telemetry = FixtureTelemetry::default();
    for index in 0..520 {
        telemetry.record(&context("bounded"), "bounded.span", "complete", index);
    }
    assert_eq!(telemetry.count().unwrap(), 512);
    assert_eq!(telemetry.dropped(), 8);
    let snapshot = telemetry.snapshot().unwrap();
    assert!(snapshot.contains("519"));
    assert!(!snapshot.contains("I64(7)"));
}
