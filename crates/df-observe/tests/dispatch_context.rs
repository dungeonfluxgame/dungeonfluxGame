#![cfg(not(target_arch = "wasm32"))]

use df_observe::{FixtureTelemetry, OperationContext};
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

async fn wait_for_release(gate: &AtomicBool) {
    poll_fn(|_| {
        if gate.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

fn operation_context(trace: &str, build: &str) -> OperationContext {
    OperationContext {
        trace_parent: format!("00-{trace}-2222222222222222-01"),
        build: build.to_owned(),
    }
}

#[test]
fn interleaved_futures_create_children_from_dispatch_snapshots_after_requests_end() {
    let telemetry = FixtureTelemetry::default();
    let mut current_context = operation_context("11111111111111111111111111111111", "build.first");
    let mut first = telemetry.begin(&current_context, "request.first");
    let first_dispatch = first.capture_context();
    let first_parent = first_dispatch
        .trace_parent
        .split('-')
        .nth(2)
        .unwrap()
        .to_owned();
    let first_gate = AtomicBool::new(false);
    let exporter = &telemetry;
    let first_release = &first_gate;
    let mut first_work = Box::pin(async move {
        wait_for_release(first_release).await;
        exporter
            .begin_dispatched(&first_dispatch, "child.first")
            .finish_unmeasured("complete");
    });
    assert!(poll_once(first_work.as_mut()).is_pending());
    first.finish_unmeasured("complete");
    drop(first);

    current_context = operation_context("33333333333333333333333333333333", "build.second");
    let mut second = telemetry.begin(&current_context, "request.second");
    let second_dispatch = second.capture_context();
    let second_parent = second_dispatch
        .trace_parent
        .split('-')
        .nth(2)
        .unwrap()
        .to_owned();
    let second_gate = AtomicBool::new(false);
    let second_release = &second_gate;
    let mut second_work = Box::pin(async move {
        wait_for_release(second_release).await;
        exporter
            .begin_dispatched(&second_dispatch, "child.second")
            .finish_unmeasured("complete");
    });
    assert!(poll_once(second_work.as_mut()).is_pending());
    second.finish_unmeasured("cancelled");
    drop(second);
    current_context.trace_parent.clear();
    current_context.build = "build.replaced".to_owned();

    second_gate.store(true, Ordering::Release);
    assert!(poll_once(second_work.as_mut()).is_ready());
    assert!(poll_once(first_work.as_mut()).is_pending());
    first_gate.store(true, Ordering::Release);
    assert!(poll_once(first_work.as_mut()).is_ready());
    let snapshot = telemetry.snapshot().unwrap();
    let first_child = snapshot
        .lines()
        .find(|line| line.starts_with("child.first "))
        .unwrap();
    let second_child = snapshot
        .lines()
        .find(|line| line.starts_with("child.second "))
        .unwrap();
    assert!(first_child.contains("trace=11111111111111111111111111111111"));
    assert!(first_child.contains(&format!("parent={first_parent}")));
    assert!(first_child.contains("build.first"));
    assert!(!first_child.contains("build.second"));
    assert!(second_child.contains("trace=33333333333333333333333333333333"));
    assert!(second_child.contains(&format!("parent={second_parent}")));
    assert!(second_child.contains("build.second"));
    assert!(!second_child.contains("cancelled"));
    assert!(!snapshot.contains("build.replaced"));
    assert_eq!(telemetry.count().unwrap(), 4);
    telemetry.shutdown().unwrap();
}

#[test]
fn dropping_owned_future_ends_only_its_child_and_keeps_accepted_sibling_live() {
    let telemetry = FixtureTelemetry::default();
    let mut request = telemetry.begin(
        &operation_context("11111111111111111111111111111111", "build.dispatch"),
        "request",
    );
    let dispatch = request.capture_context();
    let gate = AtomicBool::new(false);
    let cancelled_dispatch = dispatch.clone();
    let exporter = &telemetry;
    let release = &gate;
    let mut cancelled_work = Box::pin(async move {
        let mut child = exporter.begin_dispatched(&cancelled_dispatch, "child.cancelled");
        wait_for_release(release).await;
        child.finish_unmeasured("complete");
    });
    let mut accepted_work = Box::pin(async move {
        let mut child = exporter.begin_dispatched(&dispatch, "child.accepted");
        wait_for_release(release).await;
        child.finish_unmeasured("complete");
    });
    assert!(poll_once(cancelled_work.as_mut()).is_pending());
    assert!(poll_once(accepted_work.as_mut()).is_pending());
    request.finish_unmeasured("cancelled");
    drop(request);
    assert_eq!(telemetry.count().unwrap(), 1);
    drop(cancelled_work);
    assert_eq!(telemetry.count().unwrap(), 2);
    assert!(poll_once(accepted_work.as_mut()).is_pending());
    gate.store(true, Ordering::Release);
    assert!(poll_once(accepted_work.as_mut()).is_ready());
    let snapshot = telemetry.snapshot().unwrap();
    let cancelled = snapshot
        .lines()
        .find(|line| line.starts_with("child.cancelled "))
        .unwrap();
    let accepted = snapshot
        .lines()
        .find(|line| line.starts_with("child.accepted "))
        .unwrap();
    assert!(cancelled.contains("abandoned"));
    assert!(!cancelled.contains("complete"));
    assert!(accepted.contains("complete"));
    assert!(!accepted.contains("abandoned"));
    assert_eq!(telemetry.count().unwrap(), 3);
    telemetry.shutdown().unwrap();
}
