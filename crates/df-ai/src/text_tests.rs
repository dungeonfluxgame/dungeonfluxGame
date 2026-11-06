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
        images: Vec::new(),
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

#[test]
fn pictures_are_checked_and_change_the_fingerprint_only_when_present() {
    use crate::text::{MAXIMUM_IMAGES, TextImage, TextImageType};
    use crate::text_attempts::fingerprint;
    let plain = request();
    let mut with_picture = plain.clone();
    with_picture.images.push(TextImage {
        label: "Candidate 1".to_owned(),
        media_type: TextImageType::Jpeg,
        bytes: vec![1, 2, 3],
    });
    assert_ne!(fingerprint(&plain), fingerprint(&with_picture));
    let mut other_picture = with_picture.clone();
    other_picture.images[0].bytes = vec![9];
    assert_ne!(fingerprint(&with_picture), fingerprint(&other_picture));
    assert!(!format!("{:?}", with_picture.images[0]).contains("[1, 2, 3]"));
    let service = TextService::new(Fixed(Mutex::new(Vec::new())));
    assert!(service.check(&with_picture).is_ok());
    let mut empty = with_picture.clone();
    empty.images[0].bytes.clear();
    assert_eq!(service.check(&empty), Err(TextRequestError::InvalidImages));
    let mut many = with_picture;
    many.images = vec![many.images[0].clone(); MAXIMUM_IMAGES + 1];
    assert_eq!(service.check(&many), Err(TextRequestError::InvalidImages));
}

#[test]
fn an_output_name_the_attempt_store_cannot_hold_is_refused_before_admission() {
    let service = service(TextOutcome::Invalid);
    let mut too_long = request();
    too_long.output_name = "n".repeat(MAXIMUM_LABEL_BYTES + 1);
    assert_eq!(
        service.check(&too_long),
        Err(TextRequestError::InvalidSchema)
    );
    let mut control = request();
    control.output_name = "narration\n".to_owned();
    assert_eq!(
        service.check(&control),
        Err(TextRequestError::InvalidSchema)
    );
    let mut longest = request();
    longest.output_name = "n".repeat(MAXIMUM_LABEL_BYTES);
    assert!(service.check(&longest).is_ok());
}

#[test]
fn facts_at_the_budget_pass_and_one_byte_over_is_refused() {
    let facts = json!({"room": "harbor"});
    let facts_bytes = serde_json::to_vec(&facts).unwrap().len();
    let at_budget = TextLimits::new(1024, facts_bytes, DEFAULT_MAXIMUM_DEADLINE).unwrap();
    let mut fitting = request();
    fitting.facts = facts.clone();
    assert!(
        service(TextOutcome::Invalid)
            .with_limits(at_budget)
            .check(&fitting)
            .is_ok()
    );
    let below = TextLimits::new(1024, facts_bytes - 1, DEFAULT_MAXIMUM_DEADLINE).unwrap();
    assert_eq!(
        service(TextOutcome::Invalid)
            .with_limits(below)
            .check(&fitting),
        Err(TextRequestError::FactsTooLarge)
    );
}
