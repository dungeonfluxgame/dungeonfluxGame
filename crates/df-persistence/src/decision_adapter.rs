// Canonical session port draft09 and complete model source are consumed directly.
// NativeScope is the remaining root-owned authenticated, database-verifiable binding.
use df_model::checkpoint::{Basis, Checkpoint, DurableStatus};
use df_observe::OperationContext;
use df_session::submission::{CommitOutcome, DecisionReceipt, OperationLookup, RepositoryError};
use tokio::time::{Instant, timeout_at};
use tokio_postgres::Transaction;

use crate::checkpoint_codec::{encode_checkpoint, encode_fact, encode_intent, encode_receipt};
use crate::decision_rows::{
    final_revision_cas, insert_checkpoint, insert_fact, insert_intent, insert_operation,
    lock_session, lookup_locked,
};
use crate::native_connection::{
    DiscardState, OwnedConnection, TransactionBounds, acknowledge_commit, known_rollback,
};
use crate::native_scope::{DatabaseBindingVerifier, NativeScope};

pub(crate) struct DecisionAdapter {
    connection: OwnedConnection,
    bounds: TransactionBounds,
    codec_limits: crate::checkpoint_codec::CodecLimits,
    maximum_receipt_bytes: usize,
    verifier: DatabaseBindingVerifier,
    recovery: Option<RecoverySource>,
}

enum Stage {
    PreviouslyCommitted(DecisionReceipt),
    Refused(RepositoryError),
    LookupRequired,
    Ready(DecisionReceipt),
}

impl DecisionAdapter {
    // Native bridge checks its complete options before driver spawn/preparation. Keeping the
    // actual owner here avoids losing a pending driver through a second constructor refusal.
    pub(crate) fn from_prechecked_owned(
        connection: OwnedConnection,
        bounds: TransactionBounds,
        codec_limits: crate::checkpoint_codec::CodecLimits,
        maximum_receipt_bytes: usize,
        verifier: DatabaseBindingVerifier,
        recovery: Option<RecoverySource>,
    ) -> Self {
        Self {
            connection,
            bounds,
            codec_limits,
            maximum_receipt_bytes,
            verifier,
            recovery,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture_discard_state(&self) -> DiscardState {
        self.connection.discard_state()
    }

    pub(crate) fn connection_usable(&self) -> bool {
        self.connection.ready_for_lookup()
    }

    pub(crate) async fn close(&mut self) -> Result<(), RepositoryError> {
        self.connection.close().await
    }

    pub(crate) async fn lookup_scoped<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        context: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        match self
            .read_scoped(scope, ReadRequest::Lookup, context)
            .await?
        {
            ReadValue::Lookup(value) => Ok(value),
            ReadValue::Checkpoint(_) => Err(RepositoryError::Unavailable),
        }
    }

    pub(crate) async fn load_scoped<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        context: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        match self
            .read_scoped(scope, ReadRequest::Checkpoint, context)
            .await?
        {
            ReadValue::Checkpoint(value) => Ok(*value),
            ReadValue::Lookup(_) => Err(RepositoryError::Unavailable),
        }
    }

    async fn read_scoped<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        request: ReadRequest,
        context: &OperationContext,
    ) -> Result<ReadValue, RepositoryError> {
        let mut span = df_observe::begin(
            context,
            match request {
                ReadRequest::Lookup => "persistence.lookup_operation",
                ReadRequest::Checkpoint => "persistence.load_current",
            },
        );
        let started = Instant::now();
        let result = self.read_bounded(scope, request, context).await;
        df_observe::record_audit_line(
            "persistence.read",
            format!(
                "SAVE read kind={} outcome={} ms={} op={}",
                match request {
                    ReadRequest::Lookup => "lookup",
                    ReadRequest::Checkpoint => "checkpoint",
                },
                if result.is_ok() { "ready" } else { "refused" },
                started.elapsed().as_millis(),
                df_observe::operation_hash(scope.operation().as_bytes()),
            ),
        );
        span.finish_unmeasured(match &result {
            Ok(ReadValue::Lookup(OperationLookup::Committed(_))) => "operation_retained",
            Ok(ReadValue::Lookup(_)) => "operation_lookup",
            Ok(ReadValue::Checkpoint(_)) => "checkpoint_loaded",
            Err(RepositoryError::Unauthorized) => "scope_refused",
            Err(_) => "read_refused",
        });
        result
    }

    async fn read_bounded<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        request: ReadRequest,
        context: &OperationContext,
    ) -> Result<ReadValue, RepositoryError> {
        if self.verifier.statement.is_none() {
            return Err(RepositoryError::Unauthorized);
        }
        if matches!(request, ReadRequest::Checkpoint) && self.recovery.is_none() {
            return Err(RepositoryError::InvalidCandidate);
        }
        let deadline = Instant::now()
            .checked_add(self.bounds.transaction)
            .ok_or(RepositoryError::Capacity)?;
        let mut client = self.connection.take_client()?;
        let result = self
            .read_transaction(&mut client, scope, request, deadline, context)
            .await;
        if self.connection.usable() && self.connection.return_client(client).is_err() {
            self.discard_observed(context).await?;
            return Err(RepositoryError::Unavailable);
        }
        result
    }

    async fn read_transaction<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        client: &mut tokio_postgres::Client,
        scope: &NativeScope<A>,
        request: ReadRequest,
        deadline: Instant,
        context: &OperationContext,
    ) -> Result<ReadValue, RepositoryError> {
        let transaction = match timeout_at(deadline, client.transaction()).await {
            Ok(Ok(transaction)) => transaction,
            Ok(Err(_)) | Err(_) => {
                self.discard_observed(context).await?;
                return Err(RepositoryError::Unavailable);
            }
        };
        let value = timeout_at(deadline, async {
            configure_deadline(&transaction, deadline).await?;
            self.verifier.verify(&transaction, scope).await?;
            lock_session(&transaction, scope).await?;
            // Waiting for the common lock must not carry an expired current grant into a read.
            self.verifier.verify(&transaction, scope).await?;
            match request {
                // Retained lookup must not reject a changed current run, fence, epoch or pins.
                ReadRequest::Lookup => lookup_locked(
                    &transaction,
                    scope,
                    self.codec_limits,
                    self.maximum_receipt_bytes,
                )
                .await
                .map(ReadValue::Lookup),
                ReadRequest::Checkpoint => {
                    // The owner lease can expire while the current grant verifier waits.
                    let current = lock_session(&transaction, scope).await?;
                    current.validate_current_owner(scope)?;
                    let recovery = self
                        .recovery
                        .as_ref()
                        .ok_or(RepositoryError::InvalidCandidate)?;
                    recovery
                        .load_current(
                            &transaction,
                            scope,
                            current.basis(scope.session()),
                            self.codec_limits,
                        )
                        .await
                        .map(|value| ReadValue::Checkpoint(Box::new(value)))
                }
            }
        })
        .await;
        let result = match value {
            Ok(result) => result,
            Err(_) => Err(RepositoryError::Unavailable),
        };
        if known_rollback(transaction, self.bounds).await.is_err() {
            self.discard_observed(context).await?;
            return Err(RepositoryError::Unavailable);
        }
        result
    }

    pub(crate) async fn commit_scoped<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        checkpoint: &Checkpoint,
        expected: Basis,
        context: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let started = Instant::now();
        let deadline = started
            .checked_add(self.bounds.transaction)
            .ok_or(RepositoryError::Capacity)?;
        let mut span = df_observe::begin(context, "persistence.commit_decision");
        let result = self
            .commit_bounded(scope, checkpoint, expected, deadline, context)
            .await;
        df_observe::record_audit_line(
            "persistence.commit",
            format!(
                "SAVE commit outcome={} ms={} rev={}:{} op={}",
                match &result {
                    Ok(CommitOutcome::Confirmed(_)) => "confirmed",
                    Ok(CommitOutcome::PreviouslyCommitted(_)) => "retained",
                    Ok(CommitOutcome::Indeterminate) => "indeterminate",
                    Err(_) => "refused",
                },
                started.elapsed().as_millis(),
                expected.revision.epoch().get(),
                expected.revision.sequence(),
                df_observe::operation_hash(scope.operation().as_bytes()),
            ),
        );
        span.finish_unmeasured(match &result {
            Ok(CommitOutcome::Confirmed(_)) => "commit_confirmed",
            Ok(CommitOutcome::PreviouslyCommitted(_)) => "operation_retained",
            Ok(CommitOutcome::Indeterminate) => "commit_indeterminate",
            Err(RepositoryError::Unauthorized) => "scope_refused",
            Err(RepositoryError::Unavailable) => "storage_unavailable",
            Err(_) => "decision_refused",
        });
        result
    }

    async fn commit_bounded<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        scope: &NativeScope<A>,
        checkpoint: &Checkpoint,
        expected: Basis,
        deadline: Instant,
        context: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        if self.verifier.statement.is_none() {
            return Err(RepositoryError::Unauthorized);
        }
        let mut client = self.connection.take_client()?;
        let result = self
            .run_transaction(&mut client, scope, checkpoint, expected, deadline, context)
            .await;
        if self.connection.usable() {
            let returned = self.connection.return_client(client);
            if returned.is_err() {
                match self.discard_observed(context).await {
                    Ok(()) => {}
                    Err(_) => { /* Retained poisoned driver is closed by native shutdown. */ }
                }
                // A known durable acknowledgement survives a connection-management failure.
                if !matches!(
                    &result,
                    Ok(CommitOutcome::Confirmed(_))
                        | Ok(CommitOutcome::PreviouslyCommitted(_))
                        | Ok(CommitOutcome::Indeterminate)
                ) {
                    return Err(RepositoryError::Unavailable);
                }
            }
        }
        result
    }

    async fn run_transaction<A: df_auth::membership::MembershipAuthority>(
        &mut self,
        client: &mut tokio_postgres::Client,
        scope: &NativeScope<A>,
        checkpoint: &Checkpoint,
        expected: Basis,
        deadline: Instant,
        context: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let transaction = match timeout_at(deadline, client.transaction()).await {
            Ok(Ok(transaction)) => transaction,
            Ok(Err(_)) | Err(_) => {
                self.discard_observed(context).await?;
                return Err(RepositoryError::Unavailable);
            }
        };
        let stage = timeout_at(
            deadline,
            self.stage(&transaction, scope, checkpoint, expected, deadline),
        )
        .await;
        match stage {
            Ok(Ok(Stage::Ready(receipt))) => {
                let outcome = acknowledge_commit(transaction, receipt, deadline).await;
                if matches!(outcome, CommitOutcome::Indeterminate) {
                    // Cleanup cannot change this operation's uncertainty classification.
                    // A failed join leaves the driver handle owned and the connection poisoned.
                    match self.discard_observed(context).await {
                        Ok(()) => {}
                        Err(_) => { /* The poisoned driver remains owned for native shutdown join. */
                        }
                    }
                }
                Ok(outcome)
            }
            Ok(Ok(Stage::PreviouslyCommitted(receipt))) => {
                let cleanup = known_rollback(transaction, self.bounds).await;
                if cleanup.is_err() {
                    // Reading the retained receipt already proved its prior commit.
                    match self.discard_observed(context).await {
                        Ok(()) => {}
                        Err(_) => { /* Its classified span records the owned pending join; native shutdown retries close. */
                        }
                    }
                }
                Ok(CommitOutcome::PreviouslyCommitted(receipt))
            }
            Ok(Ok(Stage::LookupRequired)) => {
                let cleanup = known_rollback(transaction, self.bounds).await;
                if cleanup.is_err() {
                    // Lookup-only submission remains unresolved regardless of cleanup.
                    match self.discard_observed(context).await {
                        Ok(()) => {}
                        Err(_) => { /* Its classified span records the owned pending join; native shutdown retries close. */
                        }
                    }
                }
                Ok(CommitOutcome::Indeterminate)
            }
            Ok(Ok(Stage::Refused(error))) | Ok(Err(error)) => {
                // Known pre-COMMIT refusal, with confirmed cleanup before connection reuse.
                let cleanup = known_rollback(transaction, self.bounds).await;
                if cleanup.is_err() {
                    self.discard_observed(context).await?;
                }
                cleanup?;
                Err(error)
            }
            Err(_) => {
                let cleanup = known_rollback(transaction, self.bounds).await;
                if cleanup.is_err() {
                    self.discard_observed(context).await?;
                }
                cleanup?;
                Err(RepositoryError::Unavailable)
            }
        }
    }

    async fn discard_observed(
        &mut self,
        context: &OperationContext,
    ) -> Result<(), RepositoryError> {
        let started = Instant::now();
        let mut span = df_observe::begin(context, "persistence.connection_discard");
        let result = self.connection.discard().await;
        let state = match self.connection.discard_state() {
            DiscardState::NotStarted => "driver_absent",
            DiscardState::Joined => "driver_joined",
            DiscardState::JoinedWithDriverError => "driver_joined_io_error",
            DiscardState::JoinedCancelled => "driver_joined_cancelled",
            DiscardState::JoinedFailed => "driver_join_failed",
            DiscardState::JoinPending => "driver_poisoned_join_pending",
        };
        df_observe::record_audit_line(
            "persistence.connection",
            format!(
                "DATABASE connection event=discard outcome={} state={state} ms={}",
                if result.is_ok() { "closed" } else { "pending" },
                started.elapsed().as_millis()
            ),
        );
        span.finish_unmeasured(state);
        result
    }

    async fn stage<A: df_auth::membership::MembershipAuthority>(
        &self,
        transaction: &Transaction<'_>,
        scope: &NativeScope<A>,
        checkpoint: &Checkpoint,
        expected: Basis,
        deadline: Instant,
    ) -> Result<Stage, RepositoryError> {
        let codec_limits = self.codec_limits;
        let maximum_receipt_bytes = self.maximum_receipt_bytes;
        let verifier = &self.verifier;
        configure_deadline(transaction, deadline).await?;
        // Database role/capability verification and revocation are independent of data-trait assertions.
        verifier.verify(transaction, scope).await?;
        let current = lock_session(transaction, scope).await?;
        // Access/issuer verification is independent of current owner/run/epoch checks.
        verifier.verify(transaction, scope).await?;
        match lookup_locked(transaction, scope, codec_limits, maximum_receipt_bytes).await? {
            OperationLookup::Committed(receipt) => return Ok(Stage::PreviouslyCommitted(receipt)),
            OperationLookup::Conflict => {
                return Ok(Stage::Refused(RepositoryError::OperationConflict));
            }
            OperationLookup::InProgress => return Ok(Stage::LookupRequired),
            OperationLookup::ExpiredOrIndeterminate => {
                return Ok(Stage::Refused(RepositoryError::RetiredNamespace));
            }
            OperationLookup::NotRecorded => {}
        }
        if scope.is_lookup_only() {
            return Ok(Stage::LookupRequired);
        }
        current.validate_new_decision(scope, expected)?;
        let next = expected
            .revision
            .next_sequence()
            .map_err(|_| RepositoryError::SequenceExhausted)?;
        let basis = checkpoint.basis();
        if basis.session != expected.session || basis.run != expected.run || basis.revision != next
        {
            return Ok(Stage::Refused(RepositoryError::InvalidCandidate));
        }
        if checkpoint.state().mode != scope.admitted_mode() {
            return Err(RepositoryError::InvalidCandidate);
        }
        checkpoint
            .validate_resume(basis, scope.admitted_pins())
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        // Every state field is retained in the complete codec; new row projections are exact references.
        let decision = checkpoint
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == scope.operation() && decision.revision == next)
            .ok_or(RepositoryError::InvalidCandidate)?;
        validate_decision_projection(checkpoint, decision)?;
        check_receipt_capacity(decision, maximum_receipt_bytes)?;
        let receipt = DecisionReceipt::new(basis, decision.clone(), maximum_receipt_bytes)?;
        // A caller's Checkpoint constructor is not source admission. Revalidate the
        // actual complete encoded candidate with the root's admitted reference inventory.
        let recovery = self
            .recovery
            .as_ref()
            .ok_or(RepositoryError::InvalidCandidate)?;
        let document =
            recovery.encode_candidate(checkpoint, scope.admitted_pins(), codec_limits)?;
        let receipt_document =
            encode_receipt(&receipt, codec_limits).map_err(|_| RepositoryError::InvalidReceipt)?;
        insert_checkpoint(transaction, scope, checkpoint, &document).await?;
        for fact in checkpoint
            .state()
            .facts
            .iter()
            .filter(|fact| fact.operation == scope.operation() && fact.revision == next)
        {
            let encoded =
                encode_fact(fact, codec_limits).map_err(|_| RepositoryError::InvalidCandidate)?;
            insert_fact(transaction, scope, fact, &encoded).await?;
        }
        insert_operation(transaction, scope, &receipt, &receipt_document).await?;
        for intent in checkpoint
            .state()
            .intents
            .iter()
            .filter(|intent| intent.operation == scope.operation() && intent.basis == basis)
        {
            if intent.status != DurableStatus::Pending {
                return Err(RepositoryError::InvalidCandidate);
            }
            // This intent projection has no admitted commerce transaction hook. Until its
            // owner supplies the exact atomic reservation linkage, refuse live paid work.
            scope.validate_unpaid_intent(intent)?;
            let encoded = encode_intent(intent, codec_limits)
                .map_err(|_| RepositoryError::InvalidCandidate)?;
            insert_intent(
                transaction,
                scope,
                checkpoint.state().mode,
                intent,
                &encoded,
            )
            .await?;
        }
        // Recheck the actual producer's current grant/proof lifetime before the last write.
        // Its SQL must hold the authoritative revocation lock through transaction completion.
        verifier.verify(transaction, scope).await?;
        // Recheck strict lease expiry using clock_timestamp(), not a cached actor timestamp.
        final_revision_cas(transaction, scope, expected, basis).await?;
        Ok(Stage::Ready(receipt))
    }
}

/// A trusted native resolver must return the complete, bounded canonical inventory.
/// It must not use stored resource minima/maxima as source authority.
pub type ResourceResolver =
    fn(
        &df_model::checkpoint::GameState,
    ) -> Result<Vec<df_model::checkpoint::ResourceConstraint>, RepositoryError>;

/// A trusted resolver verifies accepted asset metadata against its owning state.
/// It must bound its output, refuse foreign/mismatched records and perform no I/O or paid calls.
pub type AssetResolver = fn(
    &df_model::checkpoint::GameState,
) -> Result<Vec<df_model::checkpoint::AssetReference>, RepositoryError>;

/// The native root supplies its admitted canonical inventories; stored bytes cannot admit them.
#[derive(Clone)]
pub struct RecoverySource {
    pub rules: Vec<df_model::checkpoint::RuleReference>,
    pub content: Vec<df_model::checkpoint::ContentReference>,
    pub resources: Vec<df_model::checkpoint::ResourceConstraint>,
    /// Some replaces `resources` after bounded decoding; resolver errors refuse recovery/commit.
    /// None retains the fixed admitted inventory.
    pub resource_resolver: Option<ResourceResolver>,
    pub assets: Vec<df_model::checkpoint::AssetReference>,
    /// Some replaces `assets` after complete bounded decoding, before model validation.
    pub asset_resolver: Option<AssetResolver>,
    pub limits: df_model::checkpoint::CheckpointLimits,
}
impl RecoverySource {
    /// Encode a candidate only after round-trip validation against this source's inventories.
    /// The returned bytes are the exact complete envelope validated for native commit.
    pub fn encode_candidate(
        &self,
        checkpoint: &Checkpoint,
        pins: &df_model::checkpoint::CheckpointPins,
        codec_limits: crate::checkpoint_codec::CodecLimits,
    ) -> Result<Vec<u8>, RepositoryError> {
        let document = encode_checkpoint(checkpoint, codec_limits).map_err(|error| {
            df_observe::record_audit_line("persistence.checkpoint_encode", format!("CHECKPOINT encode_refused cause={error:?} limit_bytes={} limit_items={} limit_text_bytes={}", codec_limits.maximum_document_bytes, codec_limits.maximum_collection_items, codec_limits.maximum_text_bytes));
            df_session::submission::note_refusal_diagnostic(
                "candidate-encode",
                format!("{error:?}"),
            );
            RepositoryError::InvalidCandidate
        })?;
        let validated =
            self.decode_checkpoint(&document, checkpoint.basis(), pins, codec_limits)?;
        if &validated != checkpoint {
            df_session::submission::note_refusal_diagnostic(
                "candidate-equality",
                "RoundTripMismatch".to_owned(),
            );
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(document)
    }

    /// Decode and validate the original canonical state before admitting a checkpoint.
    /// This performs no I/O and grants no database or session authority.
    pub fn decode_checkpoint(
        &self,
        bytes: &[u8],
        expected: Basis,
        pins: &df_model::checkpoint::CheckpointPins,
        codec_limits: crate::checkpoint_codec::CodecLimits,
    ) -> Result<Checkpoint, RepositoryError> {
        crate::checkpoint_codec::decode_checkpoint_with_recovery(
            bytes,
            expected,
            pins,
            self.inventory(),
            self.limits,
            codec_limits,
            Some(self),
        )
        .map_err(|error| {
            df_session::submission::note_refusal_diagnostic(
                "candidate-decode",
                format!("{error:?}"),
            );
            match error {
                crate::checkpoint_codec::CodecError::ResourceResolution(error)
                | crate::checkpoint_codec::CodecError::AssetResolution(error) => error,
                _ => RepositoryError::InvalidCandidate,
            }
        })
    }

    async fn load_current<A: df_auth::membership::MembershipAuthority>(
        &self,
        transaction: &Transaction<'_>,
        scope: &NativeScope<A>,
        current_basis: Basis,
        codec_limits: crate::checkpoint_codec::CodecLimits,
    ) -> Result<Checkpoint, RepositoryError> {
        if self.resource_resolver.is_none() && self.asset_resolver.is_none() {
            return crate::decision_rows::load_current(
                transaction,
                scope,
                self.inventory(),
                self.limits,
                codec_limits,
                current_basis,
            )
            .await;
        }
        let maximum = i64::try_from(codec_limits.maximum_document_bytes)
            .map_err(|_| RepositoryError::Capacity)?;
        let row = transaction
            .query_opt(
                crate::sql::LOAD_CURRENT,
                &[
                    &scope.tenant_bytes(),
                    &scope.session().as_bytes().as_slice(),
                    &maximum,
                ],
            )
            .await
            .map_err(|_| RepositoryError::Unavailable)?
            .ok_or(RepositoryError::Unavailable)?;
        self.decode_current_row(&row, current_basis, scope.admitted_pins(), codec_limits)
    }

    // The fixed-inventory row decoder stays unchanged for existing consumers. The
    // resolved path applies the same row provenance checks before the shared codec.
    pub(crate) fn decode_current_row(
        &self,
        row: &tokio_postgres::Row,
        current_basis: Basis,
        pins: &df_model::checkpoint::CheckpointPins,
        codec_limits: crate::checkpoint_codec::CodecLimits,
    ) -> Result<Checkpoint, RepositoryError> {
        if self.resource_resolver.is_none() && self.asset_resolver.is_none() {
            return crate::decision_rows::decode_current_row(
                row,
                current_basis.session,
                current_basis,
                pins,
                self.inventory(),
                self.limits,
                codec_limits,
            );
        }
        let expected = crate::decision_rows::current_row_basis(row, current_basis.session)?;
        let schema: i32 = row
            .try_get("schema_version")
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        if expected != current_basis || schema != i32::from(df_model::checkpoint::CHECKPOINT_SCHEMA)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let codec: i32 = row
            .try_get("codec_version")
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let document: Vec<u8> = row
            .try_get("complete_envelope")
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        crate::checkpoint_codec::validate_storage_format(codec, b"DFCP", &document)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        self.decode_checkpoint(&document, expected, pins, codec_limits)
    }

    fn inventory(&self) -> df_model::checkpoint::ReferenceInventory<'_> {
        df_model::checkpoint::ReferenceInventory {
            rules: &self.rules,
            content: &self.content,
            resources: &self.resources,
            assets: &self.assets,
        }
    }
}

fn check_receipt_capacity(
    decision: &df_model::checkpoint::AcceptedDecision,
    maximum: usize,
) -> Result<(), RepositoryError> {
    let retained = std::mem::size_of::<DecisionReceipt>()
        .checked_add(
            decision
                .facts
                .capacity()
                .checked_mul(std::mem::size_of::<df_model::checkpoint::FactId>())
                .ok_or(RepositoryError::Capacity)?,
        )
        .ok_or(RepositoryError::Capacity)?
        .checked_add(
            decision
                .draws
                .capacity()
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(RepositoryError::Capacity)?,
        )
        .ok_or(RepositoryError::Capacity)?
        .checked_add(
            decision
                .effects
                .capacity()
                .checked_mul(std::mem::size_of::<df_model::checkpoint::EffectId>())
                .ok_or(RepositoryError::Capacity)?,
        )
        .ok_or(RepositoryError::Capacity)?
        .checked_add(decision.source_policy.retained_heap_bytes())
        .ok_or(RepositoryError::Capacity)?
        .checked_add(
            decision
                .semantic_output
                .as_ref()
                .map_or(0, String::capacity),
        )
        .ok_or(RepositoryError::Capacity)?;
    if maximum == 0 || retained > maximum {
        return Err(RepositoryError::Capacity);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ReadRequest {
    Lookup,
    Checkpoint,
}
enum ReadValue {
    Lookup(OperationLookup),
    Checkpoint(Box<Checkpoint>),
}

async fn configure_deadline(
    transaction: &Transaction<'_>,
    deadline: Instant,
) -> Result<(), RepositoryError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(RepositoryError::Unavailable);
    }
    let milliseconds = remaining
        .as_millis()
        .min(i32::MAX as u128)
        .max(1)
        .to_string();
    // These GUCs only limit work. They never supply tenant or principal authority.
    transaction.query_one("SELECT set_config('statement_timeout', $1, true), set_config('lock_timeout', $1, true), set_config('synchronous_commit', 'on', true)",
        &[&milliseconds]).await.map_err(|_| RepositoryError::Unavailable)?;
    let durable: bool = transaction.query_one(
        "SELECT current_setting('fsync') = 'on' AND current_setting('full_page_writes') = 'on' AND current_setting('synchronous_commit') = 'on'",
        &[]).await.map_err(|_| RepositoryError::Unavailable)?.try_get(0).map_err(|_| RepositoryError::Unavailable)?;
    if !durable {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}

/// A complete canonical checkpoint may validate multiple past operations. This repository
/// commits exactly one new operation: every current-revision row and receipt reference
/// must describe that same decision, rather than silently dropping another staged event.
fn validate_decision_projection(
    checkpoint: &Checkpoint,
    decision: &df_model::checkpoint::AcceptedDecision,
) -> Result<(), RepositoryError> {
    let basis = checkpoint.basis();
    let state = checkpoint.state();
    if state
        .decisions
        .iter()
        .filter(|record| record.revision == basis.revision)
        .count()
        != 1
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut facts = state
        .facts
        .iter()
        .filter(|record| record.revision == basis.revision);
    for expected in &decision.facts {
        let record = facts.next().ok_or(RepositoryError::InvalidCandidate)?;
        if record.id != *expected || record.operation != decision.operation {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    if facts.next().is_some() {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut draws = state
        .draws
        .iter()
        .filter(|record| record.operation == decision.operation);
    for expected in &decision.draws {
        if draws
            .next()
            .is_none_or(|record| record.ordinal != *expected)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    if draws.next().is_some() {
        return Err(RepositoryError::InvalidCandidate);
    }
    let intents = state.intents.iter().filter(|record| record.basis == basis);
    if intents.clone().count() != decision.effects.len()
        || intents.clone().any(|record| {
            record.operation != decision.operation || !decision.effects.contains(&record.id)
        })
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use df_model::checkpoint::*;
    use df_types::{
        BuildIdentity, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
        SessionRevision,
    };

    fn label(value: &str) -> RevisionLabel {
        RevisionLabel::new(Some(value)).unwrap()
    }

    fn entity(value: u8) -> EntityId {
        EntityId::from_bytes(&[value; 16]).unwrap()
    }

    fn member(value: u8) -> MemberId {
        MemberId::from_bytes(&[value; 16]).unwrap()
    }

    fn revision(epoch: u64, sequence: u64) -> SessionRevision {
        SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
    }

    fn basis() -> Basis {
        Basis {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            revision: revision(2, 8),
        }
    }

    fn content() -> ContentReference {
        ContentReference {
            package: label("fixture-package-1"),
            entry: label("fixture-entry-1"),
        }
    }

    fn rule() -> RuleReference {
        RuleReference {
            catalog: label("fixture-catalog-1"),
            source: label("fixture-source-1"),
            entry: label("fixture-entry-1"),
            clause: label("fixture-clause-1"),
        }
    }

    fn pins() -> CheckpointPins {
        CheckpointPins {
            rules: RulesPins {
                mode: RulesMode::Standard2024,
                ruleset: label("fixture-rules-1"),
                catalog: label("fixture-catalog-1"),
                catalog_digest: ContentDigest([1; 32]),
                source_manifest: label("fixture-sources-1"),
                source_manifest_digest: ContentDigest([2; 32]),
                handler: label("fixture-handler-1"),
                handler_digest: ContentDigest([3; 32]),
            },
            content: ContentPins {
                content: label("fixture-content-1"),
                content_digest: ContentDigest([4; 32]),
                package: label("fixture-package-1"),
                package_digest: ContentDigest([5; 32]),
            },
            build: BuildIdentity::new(
                Some("fixture-source-1"),
                Some("fixture-native-1"),
                Some("fixture-wasm-1"),
                Some("fixture-config-1"),
                Some("fixture-content-1"),
            )
            .unwrap(),
        }
    }

    fn resource_constraints() -> Vec<ResourceConstraint> {
        vec![ResourceConstraint {
            owner: entity(4),
            resource: label("fixture-resource-1"),
            minimum: 0,
            maximum: 8,
            source: rule(),
        }]
    }

    fn limits() -> CheckpointLimits {
        CheckpointLimits {
            maximum_records: 100,
            maximum_text_bytes: 256,
            maximum_total_text_bytes: 1024,
            maximum_retained_bytes: 1024 * 1024,
        }
    }

    fn state() -> GameState {
        GameState {
            mode: ExecutionMode::Replay,
            logical_time: LogicalTime {
                ticks: 120,
                ticks_per_second: 10,
            },
            members: vec![MembershipLink {
                member: member(3),
                character: Some(entity(4)),
            }],
            entities: vec![WorldEntity {
                id: entity(4),
                definition: content(),
                location: None,
                position: Some(Position { x: 0, y: 0, z: 0 }),
                identity_revision: label("fixture-entity-1"),
            }],
            characters: vec![CharacterState {
                entity: entity(4),
                build: content(),
                owner: member(3),
                choices: vec![],
            }],
            resources: vec![ResourceState {
                owner: entity(4),
                resource: label("fixture-resource-1"),
                value: 4,
                minimum: 0,
                maximum: 8,
                source: rule(),
            }],
            inventory: vec![],
            facts: vec![],
            draws: vec![],
            decisions: vec![],
            pending: vec![],
            intents: vec![],
            timers: vec![],
            active_effects: vec![],
            knowledge: vec![],
            beliefs: vec![],
            memories: vec![],
            schedules: vec![],
            threats: vec![],
            relationships: vec![],
            conversations: vec![],
            obligations: vec![],
            narrative: NarrativeState {
                definition: content(),
                active_beats: vec![],
                completed_beats: vec![],
                open_threads: vec![],
                accepted_facts: vec![],
                remaining_budget: 0,
            },
            encounters: vec![],
            activity: vec![],
            tempo: TempoState {
                policy: content(),
                presentation_ticks: 0,
                intensity: 0,
                inertia: 0,
                fatigue: vec![],
            },
            presentation: vec![],
            continuity: continuity(),
        }
    }

    fn checkpoint(state: GameState) -> Result<Checkpoint, CheckpointError> {
        let rules = vec![rule()];
        let content_entries = vec![content()];
        let resource_constraints = resource_constraints();
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis(),
            pins(),
            state,
            ReferenceInventory {
                rules: &rules,
                content: &content_entries,
                resources: &resource_constraints,
                assets: &[],
            },
            limits(),
        )
    }

    fn fact(value: u8, ordinal: u32) -> GameFact {
        GameFact {
            id: FactId::from_bytes(&[value; 16]).unwrap(),
            revision: basis().revision,
            operation: OperationId::from_bytes(&[6; 16]).unwrap(),
            ordinal,
            cause: None,
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: content(),
                subjects: vec![entity(4)],
            },
        }
    }

    fn continuity() -> ContinuityState {
        ContinuityState {
            creation: vec![],
            simulation: vec![],
            catch_up: None,
            environment: vec![],
            travel: vec![],
            witnesses: vec![],
            rumors: vec![],
            journal: vec![],
            summaries: vec![],
            retrieval: vec![],
            retrieved: vec![],
            consolidation: vec![],
            npcs: vec![],
            hooks: vec![],
            arcs: vec![],
            remote: None,
            presence: vec![],
            audio: None,
            private_offers: vec![],
            knowledge_cues: vec![],
            moments: vec![],
            demands: vec![],
            asset_jobs: vec![],
            asset_dependencies: vec![],
            canonical_packs: vec![],
            shots: vec![],
            prefetch: None,
            scenes: vec![],
            item_origins: vec![],
            bookends: vec![],
            exports: vec![],
            critical_cues: vec![],
            content_candidates: vec![],
            content_admissions: vec![],
            recovery: RecoveryState {
                origin: None,
                retired_epochs: vec![],
                lost_ranges: vec![],
                suppression_generation: 0,
                redacted_records: vec![],
                unavailable_sources: vec![],
            },
        }
    }

    fn decision() -> AcceptedDecision {
        AcceptedDecision {
            operation: OperationId::from_bytes(&[6; 16]).unwrap(),
            revision: basis().revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-policy"),
            semantic_output: Some("accepted semantic output".to_owned()),
        }
    }
    fn supplied_decision(state: GameState) -> Checkpoint {
        checkpoint(state).unwrap()
    }

    #[test]
    fn candidate_codec_and_decode_refusals_preserve_the_typed_deepest_cause() {
        let mut proposed = state();
        proposed.decisions.push(decision());
        let candidate = checkpoint(proposed).unwrap();
        let source = RecoverySource {
            rules: vec![rule()],
            content: vec![content()],
            resources: resource_constraints(),
            resource_resolver: None,
            assets: vec![],
            asset_resolver: None,
            limits: limits(),
        };
        let codec = crate::checkpoint_codec::CodecLimits {
            maximum_document_bytes: 1024 * 1024,
            maximum_allocated_bytes: 1024 * 1024,
            maximum_collection_items: 1024,
            maximum_text_bytes: 256,
        };
        let _ = df_session::submission::take_refusal_diagnostic();
        assert_eq!(
            source.encode_candidate(
                &candidate,
                &pins(),
                crate::checkpoint_codec::CodecLimits {
                    maximum_text_bytes: 1,
                    ..codec
                }
            ),
            Err(RepositoryError::InvalidCandidate)
        );
        let detail = df_session::submission::take_refusal_diagnostic().unwrap();
        assert_eq!(detail.step, "checkpoint-codec-budget");
        assert!(detail.cause.contains("budget=text_bytes"));
        assert!(matches!(
            source.decode_checkpoint(b"bad", basis(), &pins(), codec),
            Err(RepositoryError::InvalidCandidate)
        ));
        let detail = df_session::submission::take_refusal_diagnostic().unwrap();
        assert_eq!(detail.step, "candidate-decode");
        assert!(detail.cause.contains("Truncated"));
    }

    #[test]
    fn exact_complete_new_decision_projection_is_accepted() {
        let mut state = state();
        let fact = fact(7, 0);
        let mut decision = decision();
        decision.facts.push(fact.id);
        state.facts.push(fact);
        state.decisions.push(decision);
        let candidate = supplied_decision(state);
        assert_eq!(
            validate_decision_projection(&candidate, &candidate.state().decisions[0]),
            Ok(())
        );
    }

    #[test]
    fn canonical_but_unassociated_current_fact_cannot_be_silently_omitted() {
        let mut state = state();
        state.facts.push(fact(7, 0));
        state.decisions.push(decision());
        let candidate = supplied_decision(state);
        assert_eq!(
            validate_decision_projection(&candidate, &candidate.state().decisions[0]),
            Err(RepositoryError::InvalidCandidate)
        );
    }

    #[test]
    fn extra_current_operation_draw_cannot_be_silently_omitted() {
        let mut state = state();
        state.decisions.push(decision());
        state.draws.push(ActualDraw {
            operation: decision().operation,
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[7; 16]).unwrap(),
            window: WindowId::from_bytes(&[8; 16]).unwrap(),
            sides: 20,
            value: 13,
            source: rule(),
        });
        let candidate = supplied_decision(state);
        assert_eq!(
            validate_decision_projection(&candidate, &candidate.state().decisions[0]),
            Err(RepositoryError::InvalidCandidate)
        );
    }

    #[test]
    fn extra_current_basis_intent_cannot_be_silently_omitted() {
        let mut state = state();
        state.decisions.push(decision());
        state.intents.push(DurableIntent {
            id: EffectId::from_bytes(&[11; 16]).unwrap(),
            basis: basis(),
            operation: decision().operation,
            slot: 0,
            kind: EffectKind::PublishPresentation,
            job: None,
            timer: None,
            generation: 1,
            status: DurableStatus::Pending,
            definition: content(),
        });
        let candidate = supplied_decision(state);
        assert_eq!(
            validate_decision_projection(&candidate, &candidate.state().decisions[0]),
            Err(RepositoryError::InvalidCandidate)
        );
    }

    #[test]
    fn two_canonical_decisions_in_one_new_revision_are_not_one_atomic_operation() {
        let mut state = state();
        state.decisions.push(decision());
        let mut other = decision();
        other.operation = OperationId::from_bytes(&[99; 16]).unwrap();
        state.decisions.push(other);
        let candidate = supplied_decision(state);
        assert_eq!(
            validate_decision_projection(&candidate, &candidate.state().decisions[0]),
            Err(RepositoryError::InvalidCandidate)
        );
    }
}
