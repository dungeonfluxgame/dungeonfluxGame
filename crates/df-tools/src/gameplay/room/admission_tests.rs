//! Controlled repository observations exercise the real DurableOwner and Join result gate.
//! These fixture scopes are identity data, not a native credential issuer or PostgreSQL proof.
use super::*;
use df_model::checkpoint::{Basis, Checkpoint};
use df_observe::OperationContext;
use df_session::inbox::ActorInput;
use df_session::submission::{
    CommitOutcome, DecisionReceipt, DeliveryError, DurableOwner, OperationLookup, OperationScope,
    PublicationOwner, RepositoryError, SessionEngine, SessionRepository,
};
use df_types::{MemberId, OperationId, RecoveryEpoch, SessionId};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::watch;

const RECEIPT_BYTES: usize = 64 * 1024;

#[derive(Eq, PartialEq)]
struct Key {
    tenant: [u8; 16],
    session: SessionId,
    principal: MemberId,
    namespace: [u8; 16],
    epoch: RecoveryEpoch,
    operation: OperationId,
    fingerprint_version: u8,
    fingerprint: [u8; 32],
}
impl ActorInput for Key {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
struct Scope {
    basis: Basis,
    operation: OperationId,
    fingerprint: [u8; 32],
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(0)
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = Key;

    fn capture_uncertainty_key(&self, maximum: usize) -> Result<Key, RepositoryError> {
        if std::mem::size_of::<Key>() > maximum {
            return Err(RepositoryError::Capacity);
        }
        Ok(Key {
            tenant: [1; 16],
            session: self.basis.session,
            principal: journey::bootstrap_member()?,
            namespace: [2; 16],
            epoch: self.basis.revision.epoch(),
            operation: self.operation,
            fingerprint_version: 1,
            fingerprint: self.fingerprint,
        })
    }
    fn session(&self) -> SessionId {
        self.basis.session
    }
    fn operation(&self) -> OperationId {
        self.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if input != &join_input(self) {
            return Err(RepositoryError::InputBinding);
        }
        Ok(())
    }
    fn is_lookup_only(&self) -> bool {
        false
    }
}
fn join_input(scope: &Scope) -> GameInput {
    GameInput::Game(CommandInput {
        basis: scope.basis,
        operation: scope.operation,
        member: journey::bootstrap_member().unwrap(),
        observed_revision: scope.basis.revision,
        command: GameCommand::ProposeAction {
            actor: journey::room_entity().unwrap(),
            action: model::content("join-room").unwrap(),
            targets: vec![],
            choices: vec![],
        },
    })
}
fn receipt(candidate: &Checkpoint, operation: OperationId) -> DecisionReceipt {
    let decision = candidate
        .state()
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .unwrap()
        .clone();
    let mut basis = candidate.basis();
    basis.revision = decision.revision;
    DecisionReceipt::new(basis, decision, RECEIPT_BYTES).unwrap()
}

#[derive(Clone, Copy)]
enum Observation {
    Fresh,
    LookupCommitted,
    CommitPreviouslyCommitted,
    CommitIndeterminate,
    CommitRefused(RepositoryError),
    LookupRefused(RepositoryError),
    Expired,
}
#[derive(Default)]
struct Calls {
    lookup: usize,
    engine: usize,
    commit: usize,
    reload: usize,
    publication: usize,
    intent_wakeup: usize,
}
struct Repository {
    observation: Observation,
    committed: Checkpoint,
    reload_fails: bool,
    calls: Rc<RefCell<Calls>>,
}
impl SessionRepository for Repository {
    type Scope = Scope;

    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        self.calls.borrow_mut().lookup += 1;
        match self.observation {
            Observation::LookupCommitted => {
                if self
                    .committed
                    .state()
                    .decisions
                    .iter()
                    .any(|decision| decision.operation == scope.operation)
                {
                    Ok(OperationLookup::Committed(receipt(
                        &self.committed,
                        scope.operation,
                    )))
                } else {
                    Ok(OperationLookup::NotRecorded)
                }
            }
            Observation::LookupRefused(error) => Err(error),
            Observation::Expired => Ok(OperationLookup::ExpiredOrIndeterminate),
            _ => Ok(OperationLookup::NotRecorded),
        }
    }
    fn commit_decision(
        &mut self,
        scope: &Scope,
        candidate: &Checkpoint,
        _: Basis,
        _: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        self.calls.borrow_mut().commit += 1;
        match self.observation {
            Observation::CommitRefused(error) => Err(error),
            Observation::CommitPreviouslyCommitted => Ok(CommitOutcome::PreviouslyCommitted(
                receipt(&self.committed, scope.operation),
            )),
            Observation::Fresh
            | Observation::LookupCommitted
            | Observation::CommitIndeterminate => {
                self.committed = candidate.clone();
                if matches!(self.observation, Observation::CommitIndeterminate) {
                    self.observation = Observation::LookupCommitted;
                    Ok(CommitOutcome::Indeterminate)
                } else {
                    Ok(CommitOutcome::Confirmed(receipt(
                        candidate,
                        scope.operation,
                    )))
                }
            }
            _ => panic!("lookup-only observation reached commit"),
        }
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        self.calls.borrow_mut().reload += 1;
        if self.reload_fails {
            return Err(RepositoryError::Unavailable);
        }
        Ok(self.committed.clone())
    }
}
struct Engine(Rc<RefCell<Calls>>);
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        self.0.borrow_mut().engine += 1;
        scope.validate_input(input)?;
        journey::stage_join(current, input)
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        if &model::checkpoint(checkpoint.basis(), checkpoint.state().clone())? != checkpoint {
            return Err(RepositoryError::InvalidCandidate);
        }
        journey::phase(checkpoint).map(|_| ())
    }
}
struct Publication(Rc<RefCell<Calls>>);
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(&mut self, _: &Scope, _: &Checkpoint) -> Result<(), DeliveryError> {
        self.0.borrow_mut().publication += 1;
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        self.0.borrow_mut().intent_wakeup += 1;
        Ok(())
    }
}
struct Submission {
    owner: DurableOwner<Repository, Engine, Publication>,
    raw: SubmissionOutcome,
    initial: Checkpoint,
    calls: Rc<RefCell<Calls>>,
}
fn submit(observation: Observation, reload_fails: bool) -> Submission {
    let initial = journey::initial().unwrap();
    let scope = Scope {
        basis: initial.basis(),
        operation: OperationId::from_bytes(&[0x31; 16]).unwrap(),
        fingerprint: [3; 32],
    };
    let input = join_input(&scope);
    let committed = journey::stage_join(&initial, &input).unwrap();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut owner = DurableOwner::new(
        Repository {
            observation,
            committed,
            reload_fails,
            calls: calls.clone(),
        },
        Engine(calls.clone()),
        Publication(calls.clone()),
        initial.clone(),
        RECEIPT_BYTES,
    )
    .unwrap();
    let (item, reply) = OwnedInput::new(
        OperationContext {
            trace_parent: String::new(),
            build: "controlled-room-admission".to_owned(),
        },
        scope,
        input,
    );
    owner.reduce(AdmissionSequence(0), item);
    Submission {
        owner,
        raw: reply.try_recv().unwrap(),
        initial,
        calls,
    }
}

fn assert_noncurrent_fenced(submission: Submission, expected: SubmissionOutcome) {
    assert!(!submission.owner.is_current());
    assert_eq!(submission.owner.checkpoint(), &submission.initial);
    let (wakeup, watch) = watch::channel(submission.initial.clone());
    let mut fenced = false;
    let outcome = admit_join_outcome(
        submission.owner.is_current(),
        Ok(submission.raw),
        submission.owner.checkpoint(),
        &mut fenced,
        &wakeup,
    )
    .unwrap();
    assert_eq!(outcome, expected);
    assert!(fenced);
    // Same-checkpoint notification wakes existing watches so their actor read is denied.
    assert!(watch.has_changed().unwrap());
    assert_eq!(&*watch.borrow(), &submission.initial);
    assert_eq!(
        actor::require_readable(fenced).unwrap_err().code(),
        tonic::Code::Unavailable,
    );
    assert!(!matches!(outcome, SubmissionOutcome::Confirmed(_)));
    assert_eq!(submission.calls.borrow().publication, 0);
    assert_eq!(submission.calls.borrow().intent_wakeup, 0);
}

#[test]
fn committed_lookup_with_failed_reload_cannot_issue_join_grant_or_read_stale_view() {
    let submission = submit(Observation::LookupCommitted, true);
    assert!(matches!(&submission.raw, SubmissionOutcome::Confirmed(_)));
    assert_eq!(submission.calls.borrow().engine, 0);
    assert_eq!(submission.calls.borrow().commit, 0);
    assert_eq!(submission.calls.borrow().reload, 1);
    assert_noncurrent_fenced(submission, SubmissionOutcome::LookupRequired);
}

#[test]
fn duplicate_commit_with_failed_reload_cannot_issue_join_grant_or_read_stale_view() {
    let submission = submit(Observation::CommitPreviouslyCommitted, true);
    assert!(matches!(&submission.raw, SubmissionOutcome::Confirmed(_)));
    assert_eq!(submission.calls.borrow().engine, 1);
    assert_eq!(submission.calls.borrow().commit, 1);
    assert_eq!(submission.calls.borrow().reload, 1);
    assert_noncurrent_fenced(submission, SubmissionOutcome::LookupRequired);
}

#[test]
fn known_commit_scope_and_cas_refusals_fence_view_and_wake_existing_watches() {
    for error in [
        RepositoryError::StaleEpoch,
        RepositoryError::StaleFence,
        RepositoryError::ExpiredOwner,
        RepositoryError::RevisionConflict,
    ] {
        let submission = submit(Observation::CommitRefused(error), false);
        assert_eq!(submission.raw, SubmissionOutcome::Refused(error));
        assert_eq!(submission.calls.borrow().engine, 1);
        assert_eq!(submission.calls.borrow().commit, 1);
        assert_noncurrent_fenced(submission, SubmissionOutcome::Refused(error));
    }
}

#[test]
fn unresolved_lookup_fences_before_join_outcome_and_view() {
    let submission = submit(
        Observation::LookupRefused(RepositoryError::UnresolvedCommit),
        false,
    );
    assert!(submission.owner.has_uncertain_operation());
    assert_eq!(submission.calls.borrow().engine, 0);
    assert_eq!(submission.calls.borrow().commit, 0);
    assert_noncurrent_fenced(
        submission,
        SubmissionOutcome::Refused(RepositoryError::UnresolvedCommit),
    );
}

#[test]
fn expired_lookup_retains_expired_result_and_denies_stale_view() {
    let submission = submit(Observation::Expired, false);
    assert!(submission.owner.has_uncertain_operation());
    assert_eq!(submission.calls.borrow().engine, 0);
    assert_eq!(submission.calls.borrow().commit, 0);
    assert_noncurrent_fenced(submission, SubmissionOutcome::ExpiredOrIndeterminate);
}

#[test]
fn current_ack_and_valid_duplicate_keep_exact_join_receipt_and_readability() {
    for observation in [Observation::Fresh, Observation::LookupCommitted] {
        let submission = submit(observation, false);
        assert!(submission.owner.is_current());
        let expected = match &submission.raw {
            SubmissionOutcome::Confirmed(receipt) => receipt.clone(),
            other => panic!("expected actual confirmed receipt, got {other:?}"),
        };
        let (wakeup, watch) = watch::channel(submission.initial.clone());
        let mut fenced = false;
        let admitted = admit_join_outcome(
            submission.owner.is_current(),
            Ok(submission.raw),
            submission.owner.checkpoint(),
            &mut fenced,
            &wakeup,
        )
        .unwrap();
        assert_eq!(admitted, SubmissionOutcome::Confirmed(expected.clone()));
        assert!(journey::decode_join(expected.decision()).is_ok());
        assert!(!fenced);
        assert!(!watch.has_changed().unwrap());
        actor::require_readable(fenced).unwrap();
    }
}

#[test]
fn native_join_lookup_unresolved_error_fences_without_changing_checkpoint() {
    let initial = journey::initial().unwrap();
    let (wakeup, watch) = watch::channel(initial.clone());
    let mut fenced = false;
    let error = join_lookup_error(
        RepositoryError::UnresolvedCommit,
        &initial,
        &mut fenced,
        &wakeup,
    );
    assert_eq!(error.code(), tonic::Code::Unavailable);
    assert!(fenced);
    assert!(watch.has_changed().unwrap());
    assert_eq!(&*watch.borrow(), &initial);
    assert!(actor::require_readable(fenced).is_err());
}

#[test]
fn ordinary_lookup_refusal_does_not_invent_owner_uncertainty() {
    let initial = journey::initial().unwrap();
    let (wakeup, watch) = watch::channel(initial.clone());
    let mut fenced = false;
    let error = join_lookup_error(
        RepositoryError::Unauthorized,
        &initial,
        &mut fenced,
        &wakeup,
    );
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert!(!fenced);
    assert!(!watch.has_changed().unwrap());
}

#[test]
fn missing_owner_reply_still_fences_noncurrent_view_before_reporting_error() {
    let initial = journey::initial().unwrap();
    let (wakeup, watch) = watch::channel(initial.clone());
    let mut fenced = false;
    let error = admit_join_outcome(
        false,
        Err(std::sync::mpsc::TryRecvError::Disconnected),
        &initial,
        &mut fenced,
        &wakeup,
    )
    .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Internal);
    assert!(fenced);
    assert!(watch.has_changed().unwrap());
    assert!(actor::require_readable(fenced).is_err());
}

fn retry_scope(initial: &Checkpoint, operation: u8, fingerprint: u8) -> Scope {
    Scope {
        basis: initial.basis(),
        operation: OperationId::from_bytes(&[operation; 16]).unwrap(),
        fingerprint: [fingerprint; 32],
    }
}

fn recovery_context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: "controlled-room-recovery".to_owned(),
    }
}

fn submit_again(
    owner: &mut DurableOwner<Repository, Engine, Publication>,
    scope: Scope,
) -> SubmissionOutcome {
    let input = join_input(&scope);
    let (item, reply) = OwnedInput::new(recovery_context(), scope, input);
    owner.reduce(AdmissionSequence(1), item);
    reply.try_recv().unwrap()
}

#[test]
fn ambiguous_join_exact_retry_preserves_member_and_receipt_before_second_join() {
    let mut submission = submit(Observation::CommitIndeterminate, false);
    assert_eq!(submission.raw, SubmissionOutcome::LookupRequired);
    assert!(submission.owner.has_uncertain_operation());
    let scope = retry_scope(&submission.initial, 0x31, 3);
    let input = join_input(&scope);
    let expected = journey::stage_join(&submission.initial, &input).unwrap();
    let original = receipt(&expected, scope.operation);
    let original_member = journey::decode_join(original.decision()).unwrap();
    prepare_join_recovery(&mut submission.owner, &scope, &input, &recovery_context()).unwrap();
    // Preparation admits only exact lookup. It has not fabricated a current cache.
    assert!(!submission.owner.is_current());
    let SubmissionOutcome::Confirmed(recovered) = submit_again(&mut submission.owner, scope) else {
        panic!("exact uncertain Join did not recover its receipt");
    };
    assert_eq!(recovered, original);
    assert!(submission.owner.is_current());
    assert_eq!(submission.owner.checkpoint(), &expected);
    assert_eq!(
        journey::decode_join(recovered.decision()).unwrap(),
        original_member
    );
    assert!(
        submission
            .owner
            .checkpoint()
            .state()
            .members
            .iter()
            .any(|link| {
                link.member.as_bytes() == &original_member.0
                    && link
                        .character
                        .is_some_and(|entity| entity.as_bytes() == &original_member.1)
            })
    );
    let calls = submission.calls.borrow();
    assert_eq!(
        (calls.lookup, calls.engine, calls.commit, calls.reload),
        (2, 1, 1, 1)
    );
    assert_eq!((calls.publication, calls.intent_wakeup), (0, 0));
    drop(calls);

    let second = retry_scope(&submission.initial, 0x32, 4);
    assert!(matches!(
        submit_again(&mut submission.owner, second),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(
        journey::participants(submission.owner.checkpoint()).len(),
        2
    );
    let stable = submission.owner.checkpoint().clone();
    // The original receipt remains exact even after the next independent commit.
    assert_eq!(
        submit_again(
            &mut submission.owner,
            retry_scope(&submission.initial, 0x31, 3)
        ),
        SubmissionOutcome::Confirmed(original)
    );
    assert_eq!(submission.owner.checkpoint(), &stable);
    let calls = submission.calls.borrow();
    assert_eq!(
        (
            calls.engine,
            calls.commit,
            calls.publication,
            calls.intent_wakeup
        ),
        (2, 2, 1, 1)
    );
    drop(calls);
    for index in 2..df_rules::local_journey::MAXIMUM_PARTY {
        let next = retry_scope(&submission.initial, 0x31 + index as u8, 3 + index as u8);
        assert!(matches!(
            submit_again(&mut submission.owner, next),
            SubmissionOutcome::Confirmed(_)
        ));
    }
    let stable = submission.owner.checkpoint().clone();
    assert_eq!(
        journey::participants(&stable).len(),
        df_rules::local_journey::MAXIMUM_PARTY
    );
    let seventh = retry_scope(&submission.initial, 0x37, 9);
    assert_eq!(
        submit_again(&mut submission.owner, seventh),
        SubmissionOutcome::Refused(RepositoryError::Capacity)
    );
    assert_eq!(submission.owner.checkpoint(), &stable);
    let calls = submission.calls.borrow();
    assert_eq!(
        (calls.commit, calls.publication, calls.intent_wakeup),
        (6, 5, 5)
    );
}

#[test]
fn uncertain_join_recovery_rejects_changed_operation_or_fingerprint_before_lookup() {
    let mut submission = submit(Observation::CommitIndeterminate, false);
    for (operation, fingerprint) in [(0x32, 3), (0x31, 4)] {
        let scope = retry_scope(&submission.initial, operation, fingerprint);
        assert_eq!(
            prepare_join_recovery(
                &mut submission.owner,
                &scope,
                &join_input(&scope),
                &recovery_context()
            ),
            Err(RepositoryError::UnresolvedCommit)
        );
        assert!(submission.owner.has_uncertain_operation());
        assert_eq!(submission.owner.checkpoint(), &submission.initial);
        let calls = submission.calls.borrow();
        assert_eq!(
            (calls.lookup, calls.engine, calls.commit, calls.reload),
            (1, 1, 1, 0)
        );
        assert_eq!((calls.publication, calls.intent_wakeup), (0, 0));
    }
}

#[test]
fn exact_join_receipt_with_failed_reload_keeps_recovery_and_watches_fenced() {
    let mut submission = submit(Observation::CommitIndeterminate, true);
    let scope = retry_scope(&submission.initial, 0x31, 3);
    prepare_join_recovery(
        &mut submission.owner,
        &scope,
        &join_input(&scope),
        &recovery_context(),
    )
    .unwrap();
    submission.raw = submit_again(&mut submission.owner, scope);
    assert!(matches!(submission.raw, SubmissionOutcome::Confirmed(_)));
    assert!(submission.owner.has_uncertain_operation());
    assert_eq!(submission.calls.borrow().reload, 1);
    assert_noncurrent_fenced(submission, SubmissionOutcome::LookupRequired);
}

#[test]
fn known_stale_join_requires_valid_reload_before_new_decisions() {
    for reload_fails in [false, true] {
        let mut submission = submit(
            Observation::CommitRefused(RepositoryError::RevisionConflict),
            reload_fails,
        );
        assert!(!submission.owner.has_uncertain_operation());
        assert!(!submission.owner.is_current());
        let scope = retry_scope(&submission.initial, 0x31, 3);
        let result = prepare_join_recovery(
            &mut submission.owner,
            &scope,
            &join_input(&scope),
            &recovery_context(),
        );
        assert_eq!(
            result,
            if reload_fails {
                Err(RepositoryError::Unavailable)
            } else {
                Ok(())
            }
        );
        assert_eq!(submission.owner.is_current(), !reload_fails);
        let calls = submission.calls.borrow();
        assert_eq!((calls.engine, calls.commit, calls.reload), (1, 1, 1));
        assert_eq!((calls.publication, calls.intent_wakeup), (0, 0));
        if reload_fails {
            assert_eq!(submission.owner.checkpoint(), &submission.initial);
        } else {
            assert_eq!(
                journey::participants(submission.owner.checkpoint()).len(),
                1
            );
        }
    }
}
