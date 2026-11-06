use std::sync::Arc;

use df_table::event::FailureCause;
use df_table::ports::{Broker, Clock, PortName, Readiness};
use df_table::testkit::{ManualClock, correlation};

use super::fixtures::{RecordingAudit, name, test_table};
use super::hub::BrokerHub;
use super::providers::{FakeProvider, FakeWorker, ScriptedCall};
use super::purpose::{QueueClass, ResourceKind};
use super::queues::QueueBounds;
use super::request::{Epoch, ProviderCause, RequestStatus, ResourceResult, WorkerId};
use super::scheduler::BrokerLimits;

struct Rig {
    hub: Arc<BrokerHub>,
    clock: Arc<ManualClock>,
    audit: Arc<RecordingAudit>,
}

fn rig(limits: BrokerLimits) -> Rig {
    let clock = Arc::new(ManualClock::default());
    let audit = Arc::new(RecordingAudit::default());
    let hub = Arc::new(BrokerHub::new(
        test_table(),
        limits,
        audit.clone(),
        clock.clone(),
    ));
    Rig { hub, clock, audit }
}

fn worker(rig: &Rig, text: FakeProvider, image: FakeProvider) -> FakeWorker {
    FakeWorker::new(rig.hub.clone(), WorkerId::new(1), text, image)
}

fn plain_providers() -> (FakeProvider, FakeProvider) {
    (
        FakeProvider::new(ResourceKind::Text, 300),
        FakeProvider::new(ResourceKind::Image, 1_000),
    )
}

#[test]
fn no_request_waits_on_the_owner_thread_even_while_every_provider_call_hangs() {
    let rig = rig(BrokerLimits::new(8, 1_024, 1, QueueBounds::default()).unwrap());
    let hanging =
        FakeProvider::new(ResourceKind::Text, 0).scripted(vec![ScriptedCall::hangs(); 64]);
    let mut worker = worker(&rig, hanging, FakeProvider::new(ResourceKind::Image, 0));
    for turn in 0..64 {
        rig.hub
            .request(correlation(turn), &name("text_live"))
            .expect("accepted");
    }
    worker.pump(rig.clock.now_ms());
    assert_eq!(worker.text().calls_started(), 8);

    for turn in 100..1_100 {
        let handle = rig
            .hub
            .request(correlation(turn), &name("text_live"))
            .expect("accepted at once");
        assert_eq!(
            rig.hub.readiness(correlation(turn), handle),
            Ok(Readiness::Pending)
        );
    }
    assert_eq!(
        worker.text().calls_started(),
        8,
        "a request never starts a provider call"
    );
}

#[test]
fn the_port_reads_a_handle_it_does_not_know_as_stale() {
    let rig = rig(BrokerLimits::default());
    let error = rig
        .hub
        .readiness(correlation(1), df_table::ports::ResourceHandle::new(404))
        .expect_err("unknown");
    assert_eq!(
        (error.port, error.cause),
        (PortName::Broker, FailureCause::Stale)
    );
}

#[test]
fn the_port_refuses_an_unknown_purpose_as_unavailable() {
    let rig = rig(BrokerLimits::default());
    let error = rig
        .hub
        .request(correlation(1), &name("no_such_purpose"))
        .expect_err("refused");
    assert_eq!(error.cause, FailureCause::Unavailable);
}

#[test]
fn a_request_made_through_the_port_is_served_by_the_worker_and_its_result_taken() {
    let rig = rig(BrokerLimits::default());
    let (text, image) = plain_providers();
    let mut worker = worker(&rig, text, image);
    let handle = rig
        .hub
        .request(correlation(1), &name("text_live"))
        .expect("accepted");

    assert_eq!(worker.pump(rig.clock.now_ms()).started, 1);
    rig.clock.advance_ms(299);
    assert_eq!(worker.pump(rig.clock.now_ms()).delivered, 0);
    assert_eq!(
        rig.hub.readiness(correlation(1), handle),
        Ok(Readiness::Pending)
    );
    rig.clock.advance_ms(1);
    assert_eq!(worker.pump(rig.clock.now_ms()).delivered, 1);

    assert_eq!(
        rig.hub.readiness(correlation(1), handle),
        Ok(Readiness::Ready)
    );
    assert!(matches!(
        rig.hub.take_result(handle),
        Ok(ResourceResult::Text(_))
    ));
    let completions = rig.hub.take_completions();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].handle, handle);
    assert_eq!(rig.hub.ledger_report(10).totals[0].totals.calls_sent, 1);
}

#[test]
fn a_scripted_failure_is_retried_after_its_backoff_and_both_calls_are_recorded() {
    let rig = rig(BrokerLimits::default());
    let text = FakeProvider::new(ResourceKind::Text, 10).scripted([
        ScriptedCall::fails_after(10, ProviderCause::Unavailable),
        ScriptedCall::succeeds_after(10),
    ]);
    let mut worker = worker(&rig, text, FakeProvider::new(ResourceKind::Image, 10));
    let handle = rig
        .hub
        .request(correlation(1), &name("text_live"))
        .expect("accepted");

    let mut now = 0;
    while now < 1_000 && rig.hub.status(handle) != Some(RequestStatus::Ready) {
        worker.pump(now);
        now += 10;
        rig.clock.advance_ms(10);
    }

    assert_eq!(rig.hub.status(handle), Some(RequestStatus::Ready));
    assert_eq!(worker.text().calls_started(), 2);
    assert_eq!(rig.hub.ledger_report(10).entries[0].calls_sent, 2);
}

#[test]
fn a_hanging_provider_call_ends_at_the_deadline_with_a_typed_timeout() {
    let rig = rig(BrokerLimits::default());
    let text = FakeProvider::new(ResourceKind::Text, 10).scripted([ScriptedCall::hangs()]);
    let mut worker = worker(&rig, text, FakeProvider::new(ResourceKind::Image, 10));
    let handle = rig
        .hub
        .request(correlation(1), &name("text_live"))
        .expect("accepted");

    worker.pump(rig.clock.now_ms());
    rig.clock.advance_ms(9_999);
    worker.pump(rig.clock.now_ms());
    assert!(matches!(
        rig.hub.status(handle),
        Some(RequestStatus::InFlight { .. })
    ));
    rig.clock.advance_ms(1);
    worker.pump(rig.clock.now_ms());

    assert_eq!(
        rig.hub.readiness(correlation(1), handle),
        Ok(Readiness::Failed(FailureCause::TimedOut))
    );
    assert!(rig.audit.lines().iter().any(|line| {
        line.starts_with("RESOURCE failed purpose=text_live handle=1 cause=timed_out ms=10000")
    }));
}

#[test]
fn a_plate_preempts_a_portrait_and_the_worker_cancels_its_provider_call() {
    let rig = rig(BrokerLimits::default().with_image_slots(1).unwrap());
    let image = FakeProvider::new(ResourceKind::Image, 50).scripted([
        ScriptedCall::succeeds_after(1_000),
        ScriptedCall::succeeds_after(50),
        ScriptedCall::succeeds_after(50),
    ]);
    let mut worker = worker(&rig, FakeProvider::new(ResourceKind::Text, 10), image);
    let portrait = rig
        .hub
        .request(correlation(1), &name("image_background"))
        .expect("accepted");
    worker.pump(0);

    rig.clock.advance_ms(100);
    let plate = rig
        .hub
        .request(correlation(2), &name("image_plate"))
        .expect("accepted");
    worker.pump(100);
    rig.clock.advance_ms(50);
    worker.pump(150);

    assert_eq!(rig.hub.status(plate), Some(RequestStatus::Ready));
    rig.clock.advance_ms(50);
    assert_eq!(worker.pump(200).started, 1);
    rig.clock.advance_ms(50);
    worker.pump(250);
    assert_eq!(rig.hub.status(portrait), Some(RequestStatus::Ready));
    rig.clock.advance_ms(750);
    assert_eq!(
        worker.pump(1_000).delivered,
        0,
        "the cancelled call never answers"
    );
    assert_eq!(worker.image().calls_started(), 3);
    assert_eq!(rig.hub.ledger_report(10).entries[0].calls_sent, 2);
}

#[test]
fn advancing_the_epoch_through_the_hub_supersedes_a_running_call_and_rejects_its_answer() {
    let rig = rig(BrokerLimits::default());
    let (text, image) = plain_providers();
    let mut worker = worker(&rig, text, image);
    let handle = rig
        .hub
        .request(correlation(1), &name("text_live"))
        .expect("accepted");
    worker.pump(0);

    assert_eq!(rig.hub.advance_epoch(Epoch::new(2)), Ok(1));
    rig.clock.advance_ms(300);
    worker.pump(300);

    assert_eq!(rig.hub.status(handle), Some(RequestStatus::Superseded));
    assert_eq!(
        rig.hub.readiness(correlation(1), handle),
        Ok(Readiness::Failed(FailureCause::Stale))
    );
}

#[test]
fn a_request_through_the_port_after_the_epoch_moved_is_made_for_the_new_epoch() {
    let rig = rig(BrokerLimits::default());
    rig.hub.advance_epoch(Epoch::new(5)).expect("forward");

    let handle = rig
        .hub
        .request(correlation(1), &name("text_live"))
        .expect("accepted");

    assert!(matches!(
        rig.hub.status(handle),
        Some(RequestStatus::Queued {
            class: QueueClass::Live
        })
    ));
    assert!(rig.audit.lines()[1].contains("epoch=5"));
}

#[test]
fn the_queue_report_shows_depth_oldest_age_and_what_is_running() {
    let rig = rig(BrokerLimits::default().with_image_slots(1).unwrap());
    let (text, image) = plain_providers();
    let mut worker = worker(&rig, text, image);
    rig.hub
        .request(correlation(1), &name("image_plate"))
        .expect("accepted");
    rig.hub
        .request(correlation(2), &name("image_plate"))
        .expect("accepted");
    worker.pump(0);
    rig.clock.advance_ms(40);

    let report = rig.hub.queues_report();

    let plate = &report.classes[QueueClass::Plate.rank()];
    assert_eq!((plate.depth, plate.oldest_age_ms), (1, Some(40)));
    assert_eq!(report.running.len(), 1);
    assert!(report.running[0].sent);
    assert_eq!(report.epoch, 1);
}
