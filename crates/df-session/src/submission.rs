//! Native session submission: only the durable repository acknowledgement releases a receipt.
//! The inbox is admission, the checkpoint is a cache, and neither confers database authority.
use std::collections::BTreeMap;
use std::mem::size_of;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use df_model::checkpoint::{AcceptedDecision, Basis, Checkpoint, GameInput};
use df_model::transition::TransitionResult;
use df_observe::OperationContext;
use df_types::{OperationId, SessionId};

use crate::inbox::{ActorInput, AdmissionSequence, Reducer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryError {
    Unauthorized,
    InputBinding,
    OperationConflict,
    RetiredNamespace,
    StaleEpoch,
    StaleFence,
    ExpiredOwner,
    RevisionConflict,
    SequenceExhausted,
    InvalidCandidate,
    InvalidReceipt,
    UnresolvedCommit,
    Capacity,
    Unavailable,
}

/// Errors that refuse an operation outright, decided from the request and the
/// current state alone: no write was attempted and no commit can be in doubt.
fn definitive_refusal(error: RepositoryError) -> bool {
    matches!(
        error,
        RepositoryError::Unauthorized
            | RepositoryError::InputBinding
            | RepositoryError::InvalidCandidate
            | RepositoryError::Capacity
    )
}

/// Adapter-owned operation data from authenticated native ingress. Implementing this
/// trait supplies input binding, not authority: repository methods independently
/// verify tenant/principal/namespace/fence/access against their trusted DB boundary.
/// Lookup-only disposition must survive ambiguous submission/restart at that boundary.
pub trait OperationScope: ActorInput {
    /// Consumer-owned exact retained operation identity: native deduplication tuple
    /// (tenant, session, principal, namespace, recovery epoch, operation) AND exact
    /// fingerprint version/value. Refreshed grants/access/owner proof are excluded.
    /// Eq must compare every semantic component, not a lossy hash or OperationId.
    /// This identity is data only and never authorizes repository access.
    type UncertaintyKey: Eq + ActorInput;

    /// Fallibly own that exact identity within the total inline+heap byte bound.
    /// Preflight every allocation, use fallible bounded copying, and charge actual
    /// retained capacities. Unrepresentable/oversized/allocation failures return
    /// Capacity, never a truncated/fallback identity. No Clone contract is assumed.
    fn capture_uncertainty_key(
        &self,
        maximum_retained_bytes: usize,
    ) -> Result<Self::UncertaintyKey, RepositoryError>;

    fn session(&self) -> SessionId;
    fn operation(&self) -> OperationId;
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError>;
    fn is_lookup_only(&self) -> bool;
}

/// Exact committed domain result. Scope authorization and retention belong to the repository.
/// Construction validates binding and owned capacity, never asserts a database commit.
#[derive(Clone, Eq, PartialEq)]
pub struct DecisionReceipt {
    basis: Basis,
    decision: AcceptedDecision,
}

// Canonical decisions may contain private semantic text. Default diagnostics retain
// identity, revision and memory bounds without serializing the decision payload.
impl std::fmt::Debug for DecisionReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecisionReceipt")
            .field("operation", &self.decision.operation)
            .field("revision", &self.basis.revision)
            .field("retained_bytes", &self.retained_bytes())
            .finish_non_exhaustive()
    }
}

impl DecisionReceipt {
    pub fn new(
        basis: Basis,
        decision: AcceptedDecision,
        maximum_retained_bytes: usize,
    ) -> Result<Self, RepositoryError> {
        if decision.revision != basis.revision {
            return Err(RepositoryError::InvalidReceipt);
        }
        let receipt = Self { basis, decision };
        if maximum_retained_bytes == 0
            || receipt.retained_bytes().ok_or(RepositoryError::Capacity)? > maximum_retained_bytes
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(receipt)
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }
    pub fn decision(&self) -> &AcceptedDecision {
        &self.decision
    }

    pub fn retained_bytes(&self) -> Option<usize> {
        receipt_retained_bytes(&self.decision)
    }

    fn matches_scope<S: OperationScope>(&self, scope: &S) -> bool {
        self.basis.session == scope.session() && self.decision.operation == scope.operation()
    }
}

fn receipt_retained_bytes(decision: &AcceptedDecision) -> Option<usize> {
    size_of::<DecisionReceipt>()
        .checked_add(
            decision
                .facts
                .capacity()
                .checked_mul(size_of::<df_model::checkpoint::FactId>())?,
        )?
        .checked_add(decision.draws.capacity().checked_mul(size_of::<u32>())?)?
        .checked_add(
            decision
                .effects
                .capacity()
                .checked_mul(size_of::<df_model::checkpoint::EffectId>())?,
        )?
        .checked_add(decision.source_policy.retained_heap_bytes())?
        .checked_add(
            decision
                .semantic_output
                .as_ref()
                .map_or(0, String::capacity),
        )
}

#[derive(Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    Confirmed(DecisionReceipt),
    PreviouslyCommitted(DecisionReceipt),
    Indeterminate,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OperationLookup {
    Committed(DecisionReceipt),
    Conflict,
    InProgress,
    ExpiredOrIndeterminate,
    NotRecorded,
}

/// Consumer-owned transaction port, implemented by native PostgreSQL persistence.
/// Calls are bounded by the native runtime's explicit transaction/receipt deadlines.
/// Lookup authenticates and resolves the exact scoped key/fingerprint before current
/// command/run/owner/offer checks. NotRecorded never authorizes an uncertain replay.
/// commit_decision rechecks scope, fingerprint, owner fence, strictly live DB-clock
/// lease and exact expected Basis in ONE transaction with checkpoint, ordered facts,
/// AcceptedDecision and DurableIntent records. Known failures roll back everything;
/// a lost COMMIT response returns Indeterminate, never Unavailable or fabricated success.
/// Confirmed/PreviouslyCommitted mean actual durable acknowledgement, never queue admission.
pub trait SessionRepository {
    type Scope: OperationScope;
    /// Restore a usable owned connection before an EXACT retained uncertain retry.
    /// The session still owns its original checkpoint/key. Native implementations
    /// close/join the displaced driver before replacement and never replay a write.
    /// In-memory/already usable ports need no connection replacement.
    fn recover_connection(&mut self, _context: &OperationContext) -> Result<(), RepositoryError> {
        Ok(())
    }
    fn lookup_operation(
        &mut self,
        operation: &Self::Scope,
        context: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError>;
    fn commit_decision(
        &mut self,
        operation: &Self::Scope,
        checkpoint: &Checkpoint,
        expected: Basis,
        context: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError>;
    fn load_current(
        &mut self,
        operation: &Self::Scope,
        context: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError>;
}

/// Pure engine authority supplies a complete validated canonical checkpoint, including
/// its AcceptedDecision, facts/draws and DurableIntents. This caller never reduces rules.
pub trait SessionEngine<S: OperationScope> {
    fn decide(
        &mut self,
        current: &Checkpoint,
        operation: &S,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError>;
    /// Existing engines may return a detached checkpoint; new owners preserve the explicit
    /// current-operation pending disposition. This remains staging, never commit authority.
    fn decide_transition(
        &mut self,
        current: &Checkpoint,
        operation: &S,
        input: &GameInput,
    ) -> TransitionResult<RepositoryError> {
        match self.decide(current, operation, input) {
            Ok(candidate) => TransitionResult::classify(current, candidate, operation.operation())
                .unwrap_or(TransitionResult::Rejected(
                    RepositoryError::InvalidCandidate,
                )),
            Err(error) => TransitionResult::Rejected(error),
        }
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    Unavailable,
    AccessRevoked,
    StaleOwner,
}

/// Native publication revalidates current scope/access/owner before each delivery.
/// Waking intents means reading registered DURABLE jobs, never executing staged bytes.
/// Failed delivery remains pending/recoverable and cannot undo an acknowledged commit.
pub trait PublicationOwner<S: OperationScope> {
    fn publish_committed(
        &mut self,
        operation: &S,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError>;
    fn wake_committed_intents(&mut self, operation: &S) -> Result<(), DeliveryError>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum SubmissionOutcome {
    Confirmed(DecisionReceipt),
    OperationConflict,
    LookupRequired,
    ExpiredOrIndeterminate,
    Refused(RepositoryError),
}

/// A fresh, uncloneable capacity-one sender per inbox item. Dropping its receiver
/// cancels only receipt delivery: the session still owns admitted durable work.
pub struct OwnedInput<S> {
    context: OperationContext,
    operation: S,
    input: GameInput,
    receipt: SyncSender<SubmissionOutcome>,
}
impl<S> OwnedInput<S> {
    pub fn new(
        context: OperationContext,
        operation: S,
        input: GameInput,
    ) -> (Self, Receiver<SubmissionOutcome>) {
        let (receipt, wait) = sync_channel(1);
        (
            Self {
                context,
                operation,
                input,
                receipt,
            },
            wait,
        )
    }
}
impl<S: OperationScope> ActorInput for OwnedInput<S> {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.input
            .retained_heap_bytes()?
            .checked_add(self.operation.retained_heap_bytes()?)?
            .checked_add(self.context.trace_parent.capacity())?
            .checked_add(self.context.build.capacity())
    }
}

enum CacheState<K> {
    Current,
    ReloadRequired,
    Uncertain(K),
}

/// One serialization owner connects the accepted inbox to pure reduction and durable
/// submission. No task/thread is spawned; the runtime owns and joins the actor loop.
/// Repository/engine/publication ports remain separate owners with actual domain types.
/// maximum_receipt_bytes bounds each retained receipt AND each full uncertainty key
/// (inline size plus checked heap capacity). At most one unresolved key is retained;
/// capturing the next input temporarily owns a second bounded key. No key is cloned.
pub struct DurableOwner<R: SessionRepository, E, P> {
    repository: R,
    engine: E,
    publication: P,
    checkpoint: Checkpoint,
    cache: CacheState<<R::Scope as OperationScope>::UncertaintyKey>,
    maximum_receipt_bytes: usize,
}
impl<R, E, P> DurableOwner<R, E, P>
where
    R: SessionRepository,
    E: SessionEngine<R::Scope>,
    P: PublicationOwner<R::Scope>,
{
    pub fn new(
        repository: R,
        mut engine: E,
        publication: P,
        checkpoint: Checkpoint,
        maximum_receipt_bytes: usize,
    ) -> Result<Self, RepositoryError> {
        if maximum_receipt_bytes == 0 {
            return Err(RepositoryError::Capacity);
        }
        engine.validate_recovery(&checkpoint)?;
        Ok(Self {
            repository,
            engine,
            publication,
            checkpoint,
            cache: CacheState::Current,
            maximum_receipt_bytes,
        })
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    /// Whether the cached checkpoint may admit a new decision. A committed receipt
    /// alone does not establish this: uncertain recovery also requires validated reload.
    pub fn is_current(&self) -> bool {
        matches!(self.cache, CacheState::Current)
    }

    /// Distinguishes ambiguous admission from a known stale cache requiring reload.
    /// Only the former restricts recovery to the first exact retained key.
    pub fn has_uncertain_operation(&self) -> bool {
        matches!(self.cache, CacheState::Uncertain(_))
    }

    /// Native ingress can route a fenced retry only when its full retained identity
    /// and bound input match. This check confers no permission: the repository must
    /// independently authenticate the refreshed scope before lookup/reload.
    pub fn matches_uncertain_retry(
        &self,
        operation: &R::Scope,
        input: &GameInput,
    ) -> Result<bool, RepositoryError> {
        operation.validate_input(input)?;
        let key = self.capture_uncertainty_key(operation)?;
        Ok(matches!(&self.cache, CacheState::Uncertain(unresolved) if unresolved == &key))
    }

    /// Consumes this reducer and returns its original owned repository for explicit
    /// native shutdown. The runtime must first drain the inbox, then close/join this
    /// repository on the dedicated actor thread before joining that thread, keeping
    /// its async runtime alive. Handback itself does not drain, close, join, resolve
    /// uncertainty, or certify successful cleanup; the native close result must be
    /// observed while the returned repository remains owned.
    #[must_use = "The native owner must explicitly close and join its returned repository"]
    pub fn into_repository(self) -> R {
        self.repository
    }

    /// Explicit reload after a known CAS/fence change. It cannot clear uncertainty:
    /// that operation must first resolve to an authorized committed receipt by lookup.
    pub fn reload_current(
        &mut self,
        operation: &R::Scope,
        context: &OperationContext,
    ) -> Result<(), RepositoryError> {
        if matches!(&self.cache, CacheState::Uncertain(_)) {
            return Err(RepositoryError::UnresolvedCommit);
        }
        self.repository.recover_connection(context)?;
        self.reload(operation, context, None)
    }

    fn reload(
        &mut self,
        operation: &R::Scope,
        context: &OperationContext,
        receipt: Option<&DecisionReceipt>,
    ) -> Result<(), RepositoryError> {
        let mut span = df_observe::begin(context, "session.reload");
        let result = self.reload_scoped(operation, context, receipt);
        span.finish_unmeasured(if result.is_ok() {
            "reloaded"
        } else {
            "reload_pending"
        });
        result
    }

    fn reload_scoped(
        &mut self,
        operation: &R::Scope,
        context: &OperationContext,
        receipt: Option<&DecisionReceipt>,
    ) -> Result<(), RepositoryError> {
        let checkpoint = self.repository.load_current(operation, context)?;
        if checkpoint.basis().session != operation.session()
            || checkpoint.basis().session != self.checkpoint.basis().session
            || checkpoint.basis().revision < self.checkpoint.basis().revision
            || receipt.is_some_and(|receipt| {
                checkpoint.basis().revision < receipt.basis().revision
                    || !checkpoint.state().decisions.contains(receipt.decision())
            })
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        self.engine.validate_recovery(&checkpoint)?;
        self.checkpoint = checkpoint;
        self.cache = CacheState::Current;
        Ok(())
    }

    fn check_receipt(
        &self,
        receipt: &DecisionReceipt,
        operation: &R::Scope,
    ) -> Result<(), RepositoryError> {
        if !receipt.matches_scope(operation) {
            return Err(RepositoryError::InvalidReceipt);
        }
        if receipt.retained_bytes().ok_or(RepositoryError::Capacity)? > self.maximum_receipt_bytes {
            return Err(RepositoryError::Capacity);
        }
        Ok(())
    }

    fn capture_uncertainty_key(
        &self,
        operation: &R::Scope,
    ) -> Result<<R::Scope as OperationScope>::UncertaintyKey, RepositoryError> {
        let inline = size_of::<<R::Scope as OperationScope>::UncertaintyKey>();
        if inline > self.maximum_receipt_bytes {
            return Err(RepositoryError::Capacity);
        }
        let key = operation.capture_uncertainty_key(self.maximum_receipt_bytes)?;
        let retained = inline
            .checked_add(key.retained_heap_bytes().ok_or(RepositoryError::Capacity)?)
            .ok_or(RepositoryError::Capacity)?;
        if retained > self.maximum_receipt_bytes {
            return Err(RepositoryError::Capacity);
        }
        Ok(key)
    }

    /// Fences for an unknown outcome, unless `error` definitively refused the
    /// operation before any write while the cache was known current: then the
    /// operation is known not to have committed, so the owner keeps serving.
    fn fence_unless_definitive_refusal(
        &mut self,
        operation: <R::Scope as OperationScope>::UncertaintyKey,
        error: RepositoryError,
    ) {
        if matches!(&self.cache, CacheState::Current) && definitive_refusal(error) {
            return;
        }
        self.fence_uncertain(operation);
    }

    fn fence_uncertain(&mut self, operation: <R::Scope as OperationScope>::UncertaintyKey) {
        // A later lookup cannot replace the first unresolved operation. Only its
        // exact committed receipt plus validated reload can release this fence.
        if !matches!(&self.cache, CacheState::Uncertain(_)) {
            self.cache = CacheState::Uncertain(operation);
        }
    }

    /// Commits an explicitly built candidate checkpoint as the next decision.
    ///
    /// The caller (a development control surface) builds the candidate from the
    /// current checkpoint: same session, run and pins, revision exactly one
    /// sequence ahead, history chains extended by exactly one new decision for
    /// this operation. The candidate passes the same `validate_candidate` gate
    /// every player decision passes, then travels the same retained lookup,
    /// repository compare-and-swap, receipt checks and publication path as a
    /// player commit, so fencing, watches, screens and the database stay
    /// consistent. No engine rules run on the candidate: the caller owns its
    /// content and records that provenance in the new decision itself.
    ///
    /// This mirrors the tail of `submit_scoped` below on purpose; the player
    /// path is untouched. Keep the two in step.
    pub fn commit_owner_candidate(
        &mut self,
        operation: &R::Scope,
        candidate: &Checkpoint,
        context: &OperationContext,
    ) -> SubmissionOutcome {
        let mut span = df_observe::begin(context, "session.commit_owner_candidate");
        let result =
            self.commit_candidate_scoped(operation, candidate, context, CommitMode::Extend);
        span.finish_unmeasured(match &result {
            SubmissionOutcome::Confirmed(_) => "confirmed",
            SubmissionOutcome::OperationConflict => "operation_conflict",
            SubmissionOutcome::LookupRequired => "lookup_required",
            SubmissionOutcome::ExpiredOrIndeterminate => "expired_or_indeterminate",
            SubmissionOutcome::Refused(_) => "refused",
        });
        result
    }

    /// Commits a development rewind (DEVCTL-001): a saved moment's full state
    /// re-committed at the next revision. Same uncertainty, lookup, compare-and-swap,
    /// receipt and publication path as [`Self::commit_owner_candidate`]; only the
    /// chain validation differs: the restored chains must stand in a prefix
    /// relationship with the current chains (back to an ancestor, or forward along
    /// recorded history), never a mix of old content with newer chains.
    pub fn commit_owner_rewind(
        &mut self,
        operation: &R::Scope,
        candidate: &Checkpoint,
        context: &OperationContext,
    ) -> SubmissionOutcome {
        let mut span = df_observe::begin(context, "session.commit_owner_rewind");
        let result =
            self.commit_candidate_scoped(operation, candidate, context, CommitMode::Rewind);
        span.finish_unmeasured(match &result {
            SubmissionOutcome::Confirmed(_) => "confirmed",
            SubmissionOutcome::OperationConflict => "operation_conflict",
            SubmissionOutcome::LookupRequired => "lookup_required",
            SubmissionOutcome::ExpiredOrIndeterminate => "expired_or_indeterminate",
            SubmissionOutcome::Refused(_) => "refused",
        });
        result
    }

    #[allow(clippy::too_many_lines)]
    fn commit_candidate_scoped(
        &mut self,
        operation: &R::Scope,
        candidate: &Checkpoint,
        context: &OperationContext,
        mode: CommitMode,
    ) -> SubmissionOutcome {
        let key = match self.capture_uncertainty_key(operation) {
            Ok(key) => key,
            Err(error) => return SubmissionOutcome::Refused(error),
        };
        if matches!(&self.cache, CacheState::Uncertain(unresolved) if unresolved == &key)
            && let Err(error) = self.repository.recover_connection(context)
        {
            return SubmissionOutcome::Refused(error);
        }
        match self.repository.lookup_operation(operation, context) {
            Ok(OperationLookup::Committed(receipt)) => {
                if let Err(error) = self.check_receipt(&receipt, operation) {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::Refused(error);
                }
                if matches!(&self.cache, CacheState::Current)
                    && receipt.basis().revision > self.checkpoint.basis().revision
                {
                    self.cache = CacheState::ReloadRequired;
                }
                let can_reload = match &self.cache {
                    CacheState::ReloadRequired => true,
                    CacheState::Uncertain(unresolved) => unresolved == &key,
                    CacheState::Current => false,
                };
                if can_reload {
                    let _reload_outcome = self.reload(operation, context, Some(&receipt));
                }
                return SubmissionOutcome::Confirmed(receipt);
            }
            Ok(OperationLookup::Conflict) => return SubmissionOutcome::OperationConflict,
            Ok(OperationLookup::InProgress) => {
                self.fence_uncertain(key);
                return SubmissionOutcome::LookupRequired;
            }
            Ok(OperationLookup::ExpiredOrIndeterminate) => {
                self.fence_uncertain(key);
                return SubmissionOutcome::ExpiredOrIndeterminate;
            }
            Ok(OperationLookup::NotRecorded) => {}
            Err(error) => {
                self.fence_unless_definitive_refusal(key, error);
                return SubmissionOutcome::Refused(error);
            }
        }
        if operation.is_lookup_only() {
            self.fence_uncertain(key);
            return SubmissionOutcome::LookupRequired;
        }
        if !matches!(&self.cache, CacheState::Current) {
            return SubmissionOutcome::LookupRequired;
        }
        if operation.session() != self.checkpoint.basis().session {
            return SubmissionOutcome::Refused(RepositoryError::InputBinding);
        }
        if self.checkpoint.basis().revision.next_sequence().is_err() {
            return SubmissionOutcome::Refused(RepositoryError::SequenceExhausted);
        }
        let valid = match mode {
            CommitMode::Extend => {
                validate_candidate(&self.checkpoint, candidate, operation.operation()).is_ok()
            }
            CommitMode::Rewind => {
                validate_rewind(&self.checkpoint, candidate, operation.operation()).is_ok()
            }
        };
        if !valid {
            return SubmissionOutcome::Refused(RepositoryError::InvalidCandidate);
        }
        let Some(decision) = candidate
            .state()
            .decisions
            .iter()
            .find(|d| d.operation == operation.operation())
        else {
            return SubmissionOutcome::Refused(RepositoryError::InvalidCandidate);
        };
        if receipt_retained_bytes(decision).is_none_or(|bytes| bytes > self.maximum_receipt_bytes) {
            return SubmissionOutcome::Refused(RepositoryError::Capacity);
        }
        match self.repository.commit_decision(
            operation,
            candidate,
            self.checkpoint.basis(),
            context,
        ) {
            Ok(CommitOutcome::Confirmed(receipt)) => {
                if self.check_receipt(&receipt, operation).is_err()
                    || receipt.basis() != candidate.basis()
                    || candidate
                        .state()
                        .decisions
                        .iter()
                        .find(|d| d.operation == operation.operation())
                        != Some(receipt.decision())
                {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::LookupRequired;
                }
                self.checkpoint = candidate.clone();
                let mut span = df_observe::begin(context, "session.publication");
                span.finish_unmeasured(
                    if self
                        .publication
                        .publish_committed(operation, &self.checkpoint)
                        .is_ok()
                    {
                        "published"
                    } else {
                        "committed_publication_pending"
                    },
                );
                let mut span = df_observe::begin(context, "session.intent_wakeup");
                span.finish_unmeasured(
                    if self.publication.wake_committed_intents(operation).is_ok() {
                        "durable_intents_woken"
                    } else {
                        "durable_intents_pending"
                    },
                );
                SubmissionOutcome::Confirmed(receipt)
            }
            Ok(CommitOutcome::PreviouslyCommitted(receipt)) => {
                if let Err(error) = self.check_receipt(&receipt, operation) {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::Refused(error);
                }
                self.cache = CacheState::ReloadRequired;
                let _reload_outcome = self.reload(operation, context, Some(&receipt));
                SubmissionOutcome::Confirmed(receipt)
            }
            Ok(CommitOutcome::Indeterminate) => {
                self.fence_uncertain(key);
                SubmissionOutcome::LookupRequired
            }
            Err(error) => {
                if matches!(
                    error,
                    RepositoryError::StaleEpoch
                        | RepositoryError::StaleFence
                        | RepositoryError::ExpiredOwner
                        | RepositoryError::RevisionConflict
                ) {
                    self.cache = CacheState::ReloadRequired;
                }
                SubmissionOutcome::Refused(error)
            }
        }
    }

    fn submit(&mut self, item: &OwnedInput<R::Scope>) -> SubmissionOutcome {
        let mut span = df_observe::begin(&item.context, "session.submit");
        let result = self.submit_scoped(&item.operation, &item.input, &item.context);
        span.finish_unmeasured(match &result {
            SubmissionOutcome::Confirmed(_) => "confirmed",
            SubmissionOutcome::OperationConflict => "operation_conflict",
            SubmissionOutcome::LookupRequired => "lookup_required",
            SubmissionOutcome::ExpiredOrIndeterminate => "expired_or_indeterminate",
            SubmissionOutcome::Refused(_) => "refused",
        });
        result
    }

    fn submit_scoped(
        &mut self,
        operation: &R::Scope,
        input: &GameInput,
        context: &OperationContext,
    ) -> SubmissionOutcome {
        if let Err(error) = operation.validate_input(input) {
            return SubmissionOutcome::Refused(error);
        }
        // Capture before any repository call. A capacity failure is a known local
        // refusal and cannot erase an earlier unknown or run an untracked operation.
        let key = match self.capture_uncertainty_key(operation) {
            Ok(key) => key,
            Err(error) => return SubmissionOutcome::Refused(error),
        };
        // Only the exact unresolved key may recover its displaced connection.
        // Other keys still require independently authorized retained lookup; a
        // not-recorded result cannot pass the current-cache gate below.
        if matches!(&self.cache, CacheState::Uncertain(unresolved) if unresolved == &key)
            && let Err(error) = self.repository.recover_connection(context)
        {
            return SubmissionOutcome::Refused(error);
        }
        match self.repository.lookup_operation(operation, context) {
            Ok(OperationLookup::Committed(receipt)) => {
                if let Err(error) = self.check_receipt(&receipt, operation) {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::Refused(error);
                }
                if matches!(&self.cache, CacheState::Current)
                    && receipt.basis().revision > self.checkpoint.basis().revision
                {
                    // A newer durable result disproves the cached basis. Fence before
                    // reload so a failed read cannot admit another stale candidate.
                    self.cache = CacheState::ReloadRequired;
                }
                let can_reload = match &self.cache {
                    CacheState::ReloadRequired => true,
                    CacheState::Uncertain(unresolved) => unresolved == &key,
                    CacheState::Current => false,
                };
                if can_reload {
                    // Preserve uncertainty if the bounded reload fails; the durable
                    // receipt is still truthful and no staged effects are dispatched.
                    let _reload_outcome = self.reload(operation, context, Some(&receipt));
                }
                return SubmissionOutcome::Confirmed(receipt);
            }
            Ok(OperationLookup::Conflict) => return SubmissionOutcome::OperationConflict,
            Ok(OperationLookup::InProgress) => {
                self.fence_uncertain(key);
                return SubmissionOutcome::LookupRequired;
            }
            Ok(OperationLookup::ExpiredOrIndeterminate) => {
                self.fence_uncertain(key);
                return SubmissionOutcome::ExpiredOrIndeterminate;
            }
            Ok(OperationLookup::NotRecorded) => {}
            Err(error) => {
                // A lookup failure that leaves the outcome in doubt must not
                // permit another key to execute around an unknown result. A
                // definitive refusal is known not to have committed.
                self.fence_unless_definitive_refusal(key, error);
                return SubmissionOutcome::Refused(error);
            }
        }
        if operation.is_lookup_only() {
            self.fence_uncertain(key);
            return SubmissionOutcome::LookupRequired;
        }
        if !matches!(&self.cache, CacheState::Current) {
            return SubmissionOutcome::LookupRequired;
        }
        if operation.session() != self.checkpoint.basis().session {
            return SubmissionOutcome::Refused(RepositoryError::InputBinding);
        }
        if self.checkpoint.basis().revision.next_sequence().is_err() {
            return SubmissionOutcome::Refused(RepositoryError::SequenceExhausted);
        }
        let transition = self
            .engine
            .decide_transition(&self.checkpoint, operation, input);
        if transition
            .validate(&self.checkpoint, operation.operation())
            .is_err()
        {
            return SubmissionOutcome::Refused(RepositoryError::InvalidCandidate);
        }
        let candidate = match transition {
            TransitionResult::Accepted(candidate) | TransitionResult::Pending { candidate, .. } => {
                candidate
            }
            TransitionResult::Rejected(error) => return SubmissionOutcome::Refused(error),
        };
        if let Err(error) = validate_candidate(&self.checkpoint, &candidate, operation.operation())
        {
            return SubmissionOutcome::Refused(error);
        }
        let Some(decision) = candidate
            .state()
            .decisions
            .iter()
            .find(|d| d.operation == operation.operation())
        else {
            return SubmissionOutcome::Refused(RepositoryError::InvalidCandidate);
        };
        if receipt_retained_bytes(decision).is_none_or(|bytes| bytes > self.maximum_receipt_bytes) {
            return SubmissionOutcome::Refused(RepositoryError::Capacity);
        }
        match self.repository.commit_decision(
            operation,
            &candidate,
            self.checkpoint.basis(),
            context,
        ) {
            Ok(CommitOutcome::Confirmed(receipt)) => {
                if self.check_receipt(&receipt, operation).is_err()
                    || receipt.basis() != candidate.basis()
                    || candidate
                        .state()
                        .decisions
                        .iter()
                        .find(|d| d.operation == operation.operation())
                        != Some(receipt.decision())
                {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::LookupRequired;
                }
                self.checkpoint = candidate;
                let mut span = df_observe::begin(context, "session.publication");
                span.finish_unmeasured(
                    if self
                        .publication
                        .publish_committed(operation, &self.checkpoint)
                        .is_ok()
                    {
                        "published"
                    } else {
                        "committed_publication_pending"
                    },
                );
                let mut span = df_observe::begin(context, "session.intent_wakeup");
                span.finish_unmeasured(
                    if self.publication.wake_committed_intents(operation).is_ok() {
                        "durable_intents_woken"
                    } else {
                        "durable_intents_pending"
                    },
                );
                SubmissionOutcome::Confirmed(receipt)
            }
            Ok(CommitOutcome::PreviouslyCommitted(receipt)) => {
                if let Err(error) = self.check_receipt(&receipt, operation) {
                    self.fence_uncertain(key);
                    return SubmissionOutcome::Refused(error);
                }
                self.cache = CacheState::ReloadRequired;
                let _reload_outcome = self.reload(operation, context, Some(&receipt));
                SubmissionOutcome::Confirmed(receipt)
            }
            Ok(CommitOutcome::Indeterminate) => {
                self.fence_uncertain(key);
                SubmissionOutcome::LookupRequired
            }
            Err(error) => {
                if matches!(
                    error,
                    RepositoryError::StaleEpoch
                        | RepositoryError::StaleFence
                        | RepositoryError::ExpiredOwner
                        | RepositoryError::RevisionConflict
                ) {
                    self.cache = CacheState::ReloadRequired;
                }
                SubmissionOutcome::Refused(error)
            }
        }
    }
}

/// Which chain rule a commit must satisfy: player and override commits only
/// extend history, while a development rewind re-commits a saved moment's full
/// state (DEVCTL-001) and must satisfy the prefix rule in [`validate_rewind`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitMode {
    Extend,
    Rewind,
}

/// The source policy of a development rewind decision (DEVCTL-001). Every view
/// projection replays only its own admitted policies and hex-decodes their
/// semantic outputs, so the dev decision carries a policy no projection reads:
/// replays walk exactly the saved moment's game decisions and skip the rewind
/// marker itself.
pub const DEV_REWIND_POLICY: &str = "dev-rewind-1";

/// How a rewind candidate's chains relate to the current chains, per chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RewindDirection {
    /// Restored chains are a strict-or-equal prefix: stepping (or staying).
    Back,
    /// Restored chains extend the current dev-stripped chains: re-applying.
    Forward,
}

fn validate_candidate(
    current: &Checkpoint,
    candidate: &Checkpoint,
    operation: OperationId,
) -> Result<(), RepositoryError> {
    let expected = current.basis();
    let next = expected
        .revision
        .next_sequence()
        .map_err(|_| RepositoryError::SequenceExhausted)?;
    let actual = candidate.basis();
    if actual.session != expected.session
        || actual.run != expected.run
        || actual.revision != next
        || current.pins() != candidate.pins()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let history = current.state();
    let proposed = candidate.state();
    if !proposed.decisions.starts_with(&history.decisions)
        || !proposed.facts.starts_with(&history.facts)
        || !proposed.draws.starts_with(&history.draws)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let Some([decision]) = proposed.decisions.get(history.decisions.len()..) else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if decision.operation != operation || decision.revision != next {
        return Err(RepositoryError::InvalidCandidate);
    }
    validate_retained_intents(current, candidate)?;
    Ok(())
}

/// Validates a development rewind candidate: the saved moment's full state
/// re-committed at the next revision with exactly one appended dev decision.
///
/// The restored chains (minus that decision) must stand in a prefix
/// relationship with the current chains — backwards to an ancestor, or forwards
/// along recorded history after trailing `dev:*` decisions are stripped — for
/// decisions, facts and draws in the same direction. Anything else (a mix of
/// old content with newer chains, or a state from another history) is refused:
/// the view projection jointly replays chains and content, so only a
/// jointly-consistent past moment projects.
fn validate_rewind(
    current: &Checkpoint,
    candidate: &Checkpoint,
    operation: OperationId,
) -> Result<(), RepositoryError> {
    let expected = current.basis();
    let next = expected
        .revision
        .next_sequence()
        .map_err(|_| RepositoryError::SequenceExhausted)?;
    let actual = candidate.basis();
    if actual.session != expected.session
        || actual.run != expected.run
        || actual.revision != next
        || current.pins() != candidate.pins()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let history = current.state();
    let proposed = candidate.state();
    let Some(dev) = proposed.decisions.last() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if dev.operation != operation || dev.revision != next {
        return Err(RepositoryError::InvalidCandidate);
    }
    if dev.source_policy.as_str() != DEV_REWIND_POLICY || !dev_marked(&dev.semantic_output) {
        return Err(RepositoryError::InvalidCandidate);
    }
    if !dev.facts.is_empty() || !dev.draws.is_empty() || !dev.effects.is_empty() {
        return Err(RepositoryError::InvalidCandidate);
    }
    if proposed
        .facts
        .iter()
        .any(|fact| fact.revision == next || fact.operation == operation)
        || proposed
            .draws
            .iter()
            .any(|draw| draw.operation == operation)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let base_decisions = &proposed.decisions[..proposed.decisions.len() - 1];
    let core_decisions = strip_trailing_dev(&history.decisions);
    let direction =
        chain_direction(core_decisions, base_decisions).ok_or(RepositoryError::InvalidCandidate)?;
    if chain_direction(&history.facts, &proposed.facts) != Some(direction)
        || chain_direction(&history.draws, &proposed.draws) != Some(direction)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    validate_retained_intents(current, candidate)?;
    Ok(())
}

/// A dev decision carries a `dev:*` note in its semantic output, so player
/// commits can never be routed through the rewind validation.
fn dev_marked(semantic_output: &Option<String>) -> bool {
    semantic_output
        .as_deref()
        .is_some_and(|note| note.starts_with("dev:"))
}

/// Trailing dev decisions stripped: stepping back lands on the last player
/// state, and stepping forward extends it.
fn strip_trailing_dev(decisions: &[AcceptedDecision]) -> &[AcceptedDecision] {
    let mut core = decisions;
    while core
        .last()
        .is_some_and(|decision| dev_marked(&decision.semantic_output))
    {
        core = &core[..core.len() - 1];
    }
    core
}

/// The prefix relationship between current chains and restored chains, if any.
/// Equal chains count as [`RewindDirection::Back`] (a stay, not a fork).
fn chain_direction<T: PartialEq>(current: &[T], restored: &[T]) -> Option<RewindDirection> {
    if current.starts_with(restored) {
        Some(RewindDirection::Back)
    } else if restored.starts_with(current) {
        Some(RewindDirection::Forward)
    } else {
        None
    }
}

fn validate_retained_intents(
    current: &Checkpoint,
    candidate: &Checkpoint,
) -> Result<(), RepositoryError> {
    let prior = &current.state().intents;
    let staged = &candidate.state().intents;
    if prior.is_empty() {
        return Ok(());
    }
    if prior.len() > staged.len() {
        return Err(RepositoryError::InvalidCandidate);
    }
    // Checkpoint admission bounds this borrowed index. Retained intent identity
    // carries its binding; status and inventory order may change independently.
    let by_id: BTreeMap<_, _> = staged.iter().map(|intent| (intent.id, intent)).collect();
    for intent in prior {
        let Some(retained) = by_id.get(&intent.id) else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if retained.basis != intent.basis
            || retained.operation != intent.operation
            || retained.slot != intent.slot
            || retained.kind != intent.kind
            || retained.job != intent.job
            || retained.timer != intent.timer
            || retained.generation != intent.generation
            || retained.definition != intent.definition
        {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    Ok(())
}

impl<R, E, P> Reducer<OwnedInput<R::Scope>> for DurableOwner<R, E, P>
where
    R: SessionRepository,
    E: SessionEngine<R::Scope>,
    P: PublicationOwner<R::Scope>,
{
    fn reduce(&mut self, _admission: AdmissionSequence, item: OwnedInput<R::Scope>) {
        let result = self.submit(&item);
        if let Err(error) = item.receipt.try_send(result) {
            let mut span = df_observe::begin(&item.context, "session.receipt_delivery");
            span.finish_unmeasured(match error {
                std::sync::mpsc::TrySendError::Disconnected(_) => "receipt_wait_dropped",
                std::sync::mpsc::TrySendError::Full(_) => "receipt_channel_contract_failure",
            });
        }
    }
}

#[cfg(test)]
mod rewind_tests {
    use super::*;

    fn decision(note: Option<&str>) -> AcceptedDecision {
        AcceptedDecision {
            operation: df_types::OperationId::from_bytes(&[7; 16]).unwrap(),
            revision: df_types::SessionRevision::new(df_types::RecoveryEpoch::new(1).unwrap(), 1),
            facts: Vec::new(),
            draws: Vec::new(),
            effects: Vec::new(),
            source_policy: df_types::RevisionLabel::new(Some("policy")).unwrap(),
            semantic_output: note.map(str::to_owned),
        }
    }

    #[test]
    fn chain_direction_names_back_forward_and_refuses_foreign() {
        use RewindDirection::{Back, Forward};
        let current = vec![1, 2, 3, 4];
        assert_eq!(chain_direction(&current, &current[..2]), Some(Back));
        assert_eq!(chain_direction(&current, &current), Some(Back));
        assert_eq!(chain_direction(&current[..2], &current), Some(Forward));
        assert_eq!(chain_direction::<u8>(&[], &[]), Some(Back));
        assert_eq!(chain_direction(&[1, 2], &[1, 9]), None);
        assert_eq!(chain_direction(&[1, 2], &[9, 1, 2]), None);
    }

    #[test]
    fn dev_decisions_are_marked_and_stripped_from_the_tail_only() {
        assert!(dev_marked(&Some("dev:restore:back 2".to_owned())));
        assert!(!dev_marked(&Some("player note".to_owned())));
        assert!(!dev_marked(&None));
        let decisions = vec![
            decision(None),
            decision(Some("dev:restore:back 1")),
            decision(Some("dev:restore:forward 1")),
        ];
        assert_eq!(strip_trailing_dev(&decisions).len(), 1);
        let decisions = vec![decision(None), decision(Some("dev:override:hp=9"))];
        assert_eq!(strip_trailing_dev(&decisions).len(), 1);
        // A buried dev decision is history, not a tail: it stays.
        let decisions = vec![decision(Some("dev:restore:back 1")), decision(None)];
        assert_eq!(strip_trailing_dev(&decisions).len(), 2);
    }
}
