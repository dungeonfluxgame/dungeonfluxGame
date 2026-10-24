use df_client::receipts::{
    LookupOutcome, OperationKey, OperationResolution, ReceiptLookup, ResolutionError,
    UncertainOperation,
};
use df_player::{
    ActionDraft, ActionDraftError, ActionDraftStatus, ActionLookupError, ActionReceiptError,
};
use df_protocol::common::{self as wire, DecisionReceipt, RejectionCode, SubmitActionRequest};
use df_types::{OperationId, RecoveryEpoch, SessionId, SessionRevision};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn epoch(value: u64) -> RecoveryEpoch {
    RecoveryEpoch::new(value).unwrap()
}
fn revision(sequence: u64) -> SessionRevision {
    SessionRevision::new(epoch(1), sequence)
}
fn wire_revision(sequence: u64) -> wire::SessionRevision {
    wire::SessionRevision {
        epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
        sequence: Some(sequence),
    }
}
fn key() -> OperationKey {
    OperationKey::new(
        SessionId::from_bytes(&[1; 16]).unwrap(),
        OperationId::from_bytes(&[3; 16]).unwrap(),
        epoch(1),
    )
}
fn request() -> SubmitActionRequest {
    SubmitActionRequest {
        session_id: Some(wire::SessionId {
            value: Some(vec![1; 16]),
        }),
        run_id: Some(wire::RunId {
            value: Some(vec![2; 16]),
        }),
        operation_id: Some(wire::OperationId {
            value: Some(vec![3; 16]),
        }),
        observed_revision: Some(wire_revision(7)),
        offer_id: "original-offer".into(),
        action_kind: 1,
        ..Default::default()
    }
}
fn draft() -> ActionDraft {
    ActionDraft::from_uncertain(UncertainOperation::new(key(), request())).unwrap()
}
fn receipt(accepted: bool) -> DecisionReceipt {
    let request = request();
    DecisionReceipt {
        session_id: request.session_id,
        run_id: request.run_id,
        operation_id: request.operation_id,
        revision: Some(wire_revision(8)),
        replayed: true,
        outcome: Some(if accepted {
            wire::decision_receipt::Outcome::Accepted(Box::default())
        } else {
            wire::decision_receipt::Outcome::Rejected(wire::Rejection {
                code: RejectionCode::StaleOffer as i32,
            })
        }),
    }
}
struct Port {
    next: Option<Result<LookupOutcome<DecisionReceipt>, &'static str>>,
    calls: Vec<OperationKey>,
}
impl ReceiptLookup for Port {
    type Receipt = DecisionReceipt;
    type Error = &'static str;
    async fn lookup(
        &mut self,
        key: OperationKey,
    ) -> Result<LookupOutcome<DecisionReceipt>, Self::Error> {
        self.calls.push(key);
        self.next.take().expect("unexpected additional lookup")
    }
}
fn port(outcome: Result<LookupOutcome<DecisionReceipt>, &'static str>) -> Port {
    Port {
        next: Some(outcome),
        calls: Vec::new(),
    }
}
fn committed(value: DecisionReceipt) -> Result<LookupOutcome<DecisionReceipt>, &'static str> {
    Ok(LookupOutcome::Committed {
        revision: revision(8),
        receipt: value,
    })
}
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("fixture expected ready borrowed lookup"),
    }
}

#[test]
fn ambiguous_submission_and_non_decisions_never_supply_success_or_replace_intent() {
    let mut draft = draft();
    for outcome in [
        LookupOutcome::InProgress,
        LookupOutcome::NotRecorded,
        LookupOutcome::ExpiredOrIndeterminate,
    ] {
        let mut port = port(Ok(outcome));
        ready(draft.resolve(&mut port)).unwrap();
        assert_eq!(port.calls, vec![key()]);
        assert_eq!(draft.key(), key());
        assert_eq!(draft.intent(), &request());
        assert!(matches!(
            draft.status(),
            ActionDraftStatus::Uncertain | ActionDraftStatus::Pending
        ));
    }
    let mut port = port(Err("safe transport error"));
    assert!(matches!(
        ready(draft.resolve(&mut port)),
        Err(ResolutionError::Lookup(ActionLookupError::Lookup(_)))
    ));
    assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
}

#[test]
fn only_validated_server_acceptance_settles_and_never_looks_up_again() {
    let mut draft = draft();
    let mut port = port(committed(receipt(true)));
    ready(draft.resolve(&mut port)).unwrap();
    assert_eq!(
        draft.status(),
        ActionDraftStatus::Accepted {
            revision: revision(8)
        }
    );
    ready(draft.resolve(&mut port)).unwrap();
    assert_eq!(port.calls, vec![key()]);
    assert_eq!(draft.intent(), &request());
}

#[test]
fn server_rejection_retains_exact_code_original_draft_and_terminal_receipt() {
    let mut draft = draft();
    let mut port = port(committed(receipt(false)));
    ready(draft.resolve(&mut port)).unwrap();
    assert_eq!(
        draft.status(),
        ActionDraftStatus::Rejected(RejectionCode::StaleOffer)
    );
    ready(draft.resolve(&mut port)).unwrap();
    assert_eq!(port.calls, vec![key()]);
    assert_eq!(draft.intent(), &request());
}

#[test]
fn malformed_identity_is_rejected_before_settlement_and_same_key_can_be_resolved() {
    let mut bad = Vec::new();
    let mut value = receipt(true);
    value.session_id = None;
    bad.push(value);
    let mut value = receipt(true);
    value.run_id.as_mut().unwrap().value = Some(vec![9; 16]);
    bad.push(value);
    let mut value = receipt(true);
    value.operation_id.as_mut().unwrap().value = Some(vec![4; 16]);
    bad.push(value);
    let mut value = receipt(true);
    value.operation_id.as_mut().unwrap().value = Some(vec![0; 3]);
    bad.push(value);
    for value in bad {
        let mut draft = draft();
        let mut port = port(committed(value));
        assert!(matches!(
            ready(draft.resolve(&mut port)),
            Err(ResolutionError::Lookup(ActionLookupError::InvalidReceipt(
                ActionReceiptError::Identity
            )))
        ));
        assert!(matches!(draft.resolution(), OperationResolution::Uncertain));
        port.next = Some(committed(receipt(true)));
        ready(draft.resolve(&mut port)).unwrap();
        assert_eq!(port.calls, vec![key(), key()]);
        assert!(matches!(draft.status(), ActionDraftStatus::Accepted { .. }));
    }
}

#[test]
fn receipt_revision_must_be_present_valid_same_epoch_nonregressing_and_exact_lookup_revision() {
    let variants = [
        None,
        Some(wire_revision(6)),
        Some(wire_revision(9)),
        Some(wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(2) }),
            sequence: Some(8),
        }),
        Some(wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(0) }),
            sequence: Some(8),
        }),
        Some(wire::SessionRevision {
            epoch: None,
            sequence: Some(8),
        }),
        Some(wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
            sequence: None,
        }),
    ];
    for revision in variants {
        let mut draft = draft();
        let mut value = receipt(true);
        value.revision = revision;
        let mut port = port(committed(value));
        assert!(matches!(
            ready(draft.resolve(&mut port)),
            Err(ResolutionError::Lookup(ActionLookupError::InvalidReceipt(
                ActionReceiptError::Revision
            )))
        ));
        assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
    }
    // Matching lookup and receipt revisions still cannot regress the submitted watermark.
    let mut draft = draft();
    let mut value = receipt(true);
    value.revision = Some(wire_revision(6));
    let mut port = port(Ok(LookupOutcome::Committed {
        revision: revision(6),
        receipt: value,
    }));
    assert!(ready(draft.resolve(&mut port)).is_err());
    assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
}

#[test]
fn missing_unknown_or_unspecified_outcome_cannot_become_a_decision() {
    for code in [None, Some(0), Some(999)] {
        let mut draft = draft();
        let mut value = receipt(false);
        value.outcome =
            code.map(|code| wire::decision_receipt::Outcome::Rejected(wire::Rejection { code }));
        let mut port = port(committed(value));
        assert!(matches!(
            ready(draft.resolve(&mut port)),
            Err(ResolutionError::Lookup(ActionLookupError::InvalidReceipt(
                ActionReceiptError::Outcome
            )))
        ));
        assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
    }
}

struct PendingPort {
    calls: usize,
}
impl ReceiptLookup for PendingPort {
    type Receipt = DecisionReceipt;
    type Error = ();
    async fn lookup(
        &mut self,
        supplied: OperationKey,
    ) -> Result<LookupOutcome<DecisionReceipt>, ()> {
        assert_eq!(supplied, key());
        self.calls += 1;
        std::future::pending().await
    }
}
#[test]
fn canceled_wait_and_reconnect_retain_original_key_without_retrying_submission() {
    let mut draft = draft();
    let mut pending = PendingPort { calls: 0 };
    {
        let mut wait = pin!(draft.resolve(&mut pending));
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(pending.calls, 1);
    assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
    assert_eq!(draft.intent(), &request());
    let mut port = port(Ok(LookupOutcome::NotRecorded));
    ready(draft.resolve(&mut port)).unwrap();
    assert_eq!(port.calls, vec![key()]);
    assert_eq!(draft.status(), ActionDraftStatus::Uncertain);
}

#[test]
fn namespace_retirement_never_recreates_an_operation_or_queries_a_new_epoch() {
    let mut draft = draft();
    draft.retire_namespace(epoch(2)).unwrap();
    let mut port = port(committed(receipt(true)));
    ready(draft.resolve(&mut port)).unwrap();
    assert!(port.calls.is_empty());
    assert_eq!(
        draft.status(),
        ActionDraftStatus::Retired { epoch: epoch(2) }
    );
    assert_eq!(draft.key(), key());
    assert_eq!(draft.intent(), &request());
}

#[test]
fn malformed_original_correlation_and_hidden_spare_capacity_are_bounded() {
    let mut value = request();
    value.observed_revision = None;
    assert!(matches!(
        ActionDraft::from_uncertain(UncertainOperation::new(key(), value)),
        Err(ActionDraftError::InvalidRequest)
    ));
    let mut value = request();
    value.offer_id = String::with_capacity(32_769);
    value.offer_id.push_str("tiny");
    assert!(matches!(
        ActionDraft::from_uncertain(UncertainOperation::new(key(), value)),
        Err(ActionDraftError::RequestTooLarge)
    ));
    let mut value = request();
    value.character = Some(wire::CharacterSelection {
        portrait_scenery: String::new(),
        name: "Mira".into(),
        choices: Vec::with_capacity(1000),
        identity: None,
    });
    assert!(matches!(
        ActionDraft::from_uncertain(UncertainOperation::new(key(), value)),
        Err(ActionDraftError::RequestTooLarge)
    ));
}

#[test]
fn a_previously_settled_unvalidated_owner_cannot_bypass_the_validation_boundary() {
    let mut operation = UncertainOperation::new(key(), request());
    let mut port = port(committed(receipt(true)));
    ready(operation.resolve(&mut port)).unwrap();
    assert!(matches!(
        ActionDraft::from_uncertain(operation),
        Err(ActionDraftError::InvalidOriginalState)
    ));
}
