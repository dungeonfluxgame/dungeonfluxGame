use super::*;
use serde_json::json;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Waker};

/// Every test future here completes on its first poll, so one poll with a no-op waker suffices.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("test future did not complete on its first poll"),
    }
}

struct Fixed(Mutex<Vec<TextOutcome>>);

impl TextBackend for Fixed {
    fn complete(&self, _: &TextRequest) -> impl Future<Output = TextReceipt> + Send {
        let outcome = self.0.lock().unwrap().remove(0);
        async move { TextReceipt::from(outcome) }
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<TextEvent>>);

impl TextObserver for Recorder {
    fn observe(&self, event: &TextEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

fn request() -> TextRequest {
    TextRequest {
        purpose: TextPurpose::Storyteller,
        profile: TextProfile::new("profile-a").unwrap(),
        instructions: "Narrate one line.".to_owned(),
        facts: json!({"room": "harbor"}),
        output_name: "narration".to_owned(),
        output_schema: json!({"type": "object", "properties": {"line": {"type": "string"}}}),
        deadline: Duration::from_secs(30),
    }
}

fn service(outcome: TextOutcome) -> TextService<Fixed> {
    TextService::new(Fixed(Mutex::new(vec![outcome])))
}

#[test]
fn limits_refuse_zero_and_above_ceiling_values() {
    assert_eq!(
        TextLimits::new(0, 1, Duration::from_secs(1)),
        Err(TextLimitsError::ZeroBudget)
    );
    assert_eq!(
        TextLimits::new(CEILING_INSTRUCTION_BYTES + 1, 1, Duration::from_secs(1)),
        Err(TextLimitsError::AboveCeiling)
    );
    assert!(TextLimits::new(1024, 1024, Duration::from_secs(60)).is_ok());
}

#[test]
fn a_configured_limit_applies_to_every_request() {
    let limits = TextLimits::new(8, DEFAULT_FACTS_BYTES, DEFAULT_MAXIMUM_DEADLINE).unwrap();
    let service = service(TextOutcome::Invalid).with_limits(limits);
    assert_eq!(
        block_on(service.submit(&request())),
        Err(TextRequestError::InstructionsTooLarge)
    );
}

#[test]
fn an_object_answer_passes_through() {
    let answer = json!({"line": "The tide turns."});
    let outcome = block_on(service(TextOutcome::Answered(answer.clone())).submit(&request()));
    assert_eq!(outcome, Ok(TextOutcome::Answered(answer)));
}

#[test]
fn a_non_object_answer_is_invalid_not_trusted() {
    let outcome = block_on(service(TextOutcome::Answered(json!("bare string"))).submit(&request()));
    assert_eq!(outcome, Ok(TextOutcome::Invalid));
}

#[test]
fn backend_refusals_pass_through_unchanged() {
    assert_eq!(
        block_on(service(TextOutcome::Deadline).submit(&request())),
        Ok(TextOutcome::Deadline)
    );
    assert_eq!(
        block_on(
            service(TextOutcome::Unavailable(UnavailableReason::Interrupted)).submit(&request())
        ),
        Ok(TextOutcome::Unavailable(UnavailableReason::Interrupted))
    );
}

#[test]
fn bad_requests_never_reach_the_backend_and_are_each_reported_once() {
    let recorder = std::sync::Arc::new(Recorder::default());
    let service = service(TextOutcome::Invalid).with_observer(recorder.clone());
    let mut bad = request();
    bad.instructions = "   ".to_owned();
    assert_eq!(
        block_on(service.submit(&bad)),
        Err(TextRequestError::EmptyInstructions)
    );
    let mut zero = request();
    zero.deadline = Duration::ZERO;
    assert_eq!(
        block_on(service.submit(&zero)),
        Err(TextRequestError::InvalidDeadline)
    );
    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].kind,
        TextEventKind::Refused(TextRequestError::EmptyInstructions)
    );
    assert_eq!(events[1].kind.label(), "invalid_deadline");
}

#[test]
fn every_answered_request_reports_one_event_with_purpose_and_outcome() {
    let recorder = std::sync::Arc::new(Recorder::default());
    let service =
        service(TextOutcome::Answered(json!({"line": "ok"}))).with_observer(recorder.clone());
    block_on(service.submit(&request())).unwrap();
    let events = recorder.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].purpose.label(), "storyteller");
    assert_eq!(events[0].output_name, "narration");
    assert_eq!(events[0].kind, TextEventKind::Answered);
}
