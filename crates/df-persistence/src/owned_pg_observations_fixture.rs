//! Real repository/physical-row observations for the registered PostgreSQL fixture.
//! These functions execute on the joined actor thread with its injected existing runtime.
use crate::checkpoint_codec::{CodecLimits, decode_receipt};
use crate::native_bridge::PostgresRepository;
use crate::native_connection::actor_block_on;
use crate::native_scope::NativeScope;
use crate::owned_pg_admin_fixture::{
    physical_snapshot, validate_physical_commit, validate_physical_rollback,
};
use crate::owned_pg_fault_fixture::{InsertedFamily, install_fault, remove_fault};
use crate::owned_pg_membership_fixture::FixtureMembershipAuthority;
use df_model::checkpoint::{Basis, Checkpoint, CheckpointLimits, ReferenceInventory};
use df_observe::OperationContext;
use df_session::submission::{
    CommitOutcome, DecisionReceipt, OperationLookup, RepositoryError, SessionRepository,
};
use tokio::runtime::Handle;
use tokio::time::Instant;
use tokio_postgres::Client;

pub(crate) struct PhysicalObservation<'a> {
    pub(crate) runtime: &'a Handle,
    pub(crate) administrator: &'a Client,
    pub(crate) inventory: ReferenceInventory<'a>,
    pub(crate) model_limits: CheckpointLimits,
    pub(crate) codec_limits: CodecLimits,
    pub(crate) maximum_receipt_bytes: usize,
    pub(crate) deadline: Instant,
}
impl PhysicalObservation<'_> {
    fn admitted_inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: self.inventory.rules,
            content: self.inventory.content,
            resources: self.inventory.resources,
            assets: self.inventory.assets,
        }
    }
    fn check_commit(
        &self,
        scope: &NativeScope<FixtureMembershipAuthority>,
        candidate: &Checkpoint,
        receipt: &DecisionReceipt,
    ) -> Result<(), RepositoryError> {
        let snapshot = actor_block_on(
            self.runtime,
            physical_snapshot(
                self.administrator,
                &scope.tenant,
                candidate.basis(),
                self.codec_limits.maximum_document_bytes,
                self.deadline,
            ),
        )??;
        validate_physical_commit(
            &snapshot,
            candidate,
            self.admitted_inventory(),
            self.model_limits,
            self.codec_limits,
        )?;
        let document = snapshot
            .retained_receipt
            .as_ref()
            .ok_or(RepositoryError::InvalidReceipt)?;
        let stored = decode_receipt(
            document,
            scope.session,
            scope.operation,
            self.maximum_receipt_bytes,
            self.codec_limits,
        )
        .map_err(|_| RepositoryError::InvalidReceipt)?;
        if stored != *receipt {
            return Err(RepositoryError::InvalidReceipt);
        }
        Ok(())
    }
}

pub(crate) fn observe_success_and_retained(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    candidate: &Checkpoint,
    expected: Basis,
    context: &OperationContext,
    physical: &PhysicalObservation<'_>,
) -> Result<DecisionReceipt, RepositoryError> {
    if !matches!(
        repository.lookup_operation(scope, context)?,
        OperationLookup::NotRecorded
    ) {
        return Err(RepositoryError::InvalidReceipt);
    }
    let receipt = match repository.commit_decision(scope, candidate, expected, context)? {
        CommitOutcome::Confirmed(receipt) => receipt,
        _ => return Err(RepositoryError::InvalidReceipt),
    };
    physical.check_commit(scope, candidate, &receipt)?;
    match repository.lookup_operation(scope, context)? {
        OperationLookup::Committed(retained) if retained == receipt => {}
        _ => return Err(RepositoryError::InvalidReceipt),
    }
    match repository.commit_decision(scope, candidate, expected, context)? {
        CommitOutcome::PreviouslyCommitted(retained) if retained == receipt => {}
        _ => return Err(RepositoryError::InvalidReceipt),
    }
    if repository.load_current(scope, context)? != *candidate {
        return Err(RepositoryError::InvalidCandidate);
    }
    physical.check_commit(scope, candidate, &receipt)?;
    Ok(receipt)
}

/// AFTER INSERT faults require a nonempty candidate for the selected family.
/// The owned PostgreSQL error log supplies the exact fault hit observation; the
/// original full criterion still requires its registered log/remaining case proofs.
pub(crate) fn observe_family_rollback(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    candidate: &Checkpoint,
    expected: Basis,
    family: InsertedFamily,
    context: &OperationContext,
    physical: &PhysicalObservation<'_>,
) -> Result<(), RepositoryError> {
    let family_row_count = match family {
        InsertedFamily::Checkpoint
        | InsertedFamily::Operation
        | InsertedFamily::FinalRevisionCas
        | InsertedFamily::LeaseBeforeCas => 1,
        InsertedFamily::Fact | InsertedFamily::FactOrdinal => candidate
            .state()
            .facts
            .iter()
            .filter(|f| f.revision == candidate.basis().revision)
            .count(),
        InsertedFamily::Intent
        | InsertedFamily::IntentSlot
        | InsertedFamily::EffectIdentity
        | InsertedFamily::CreatedJob
        | InsertedFamily::CreatedTimer => candidate
            .state()
            .intents
            .iter()
            .filter(|i| i.basis == candidate.basis())
            .count(),
    };
    if family_row_count == 0 {
        return Err(RepositoryError::InvalidCandidate);
    }
    if repository.load_current(scope, context)?.basis() != expected
        || !matches!(
            repository.lookup_operation(scope, context)?,
            OperationLookup::NotRecorded
        )
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    actor_block_on(
        physical.runtime,
        install_fault(physical.administrator, family, physical.deadline),
    )??;
    let outcome = repository.commit_decision(scope, candidate, expected, context);
    // Remove the root-owned trigger even when the tested adapter outcome is unexpected.
    let removed = actor_block_on(
        physical.runtime,
        remove_fault(physical.administrator, family, physical.deadline),
    )?;
    removed?;
    let expected_refusal = match family {
        InsertedFamily::FinalRevisionCas | InsertedFamily::LeaseBeforeCas => {
            RepositoryError::RevisionConflict
        }
        _ => RepositoryError::Unavailable,
    };
    if !matches!(outcome,Err(error) if error==expected_refusal) {
        return Err(RepositoryError::InvalidReceipt);
    }
    let snapshot = actor_block_on(
        physical.runtime,
        physical_snapshot(
            physical.administrator,
            &scope.tenant,
            candidate.basis(),
            physical.codec_limits.maximum_document_bytes,
            physical.deadline,
        ),
    )??;
    validate_physical_rollback(&snapshot, expected)?;
    if !matches!(
        repository.lookup_operation(scope, context)?,
        OperationLookup::NotRecorded
    ) || repository.load_current(scope, context)?.basis() != expected
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    // With the fault removed the exact same canonical candidate must actually commit.
    let receipt = match repository.commit_decision(scope, candidate, expected, context)? {
        CommitOutcome::Confirmed(receipt) => receipt,
        _ => return Err(RepositoryError::InvalidReceipt),
    };
    physical.check_commit(scope, candidate, &receipt)
}

pub(crate) fn observe_scope_refusal(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    candidate: &Checkpoint,
    expected: Basis,
    context: &OperationContext,
) -> Result<(), RepositoryError> {
    if !matches!(
        repository.lookup_operation(scope, context),
        Err(RepositoryError::Unauthorized)
    ) || !matches!(
        repository.commit_decision(scope, candidate, expected, context),
        Err(RepositoryError::Unauthorized)
    ) || !matches!(
        repository.load_current(scope, context),
        Err(RepositoryError::Unauthorized)
    ) {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

pub(crate) fn observe_configured_read_refusal(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    context: &OperationContext,
    document_too_small: bool,
) -> Result<(), RepositoryError> {
    if !matches!(
        repository.lookup_operation(scope, context),
        Err(RepositoryError::InvalidReceipt)
    ) {
        return Err(RepositoryError::InvalidReceipt);
    }
    if document_too_small
        && !matches!(
            repository.load_current(scope, context),
            Err(RepositoryError::Unavailable)
        )
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

pub(crate) fn observe_historical_key_reload(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    retained: &DecisionReceipt,
    obsolete: (&Checkpoint, Basis),
    current: &Checkpoint,
    context: &OperationContext,
    physical: &PhysicalObservation<'_>,
) -> Result<(), RepositoryError> {
    let (obsolete_candidate, old_expected) = obsolete;
    if current.basis().revision.epoch() <= retained.basis().revision.epoch() {
        return Err(RepositoryError::InvalidCandidate);
    }
    match repository.lookup_operation(scope, context)? {
        OperationLookup::Committed(receipt) if receipt == *retained => {}
        _ => return Err(RepositoryError::InvalidReceipt),
    }
    // Retained lookup precedes obsolete candidate/old expected epoch checks.
    match repository.commit_decision(scope, obsolete_candidate, old_expected, context)? {
        CommitOutcome::PreviouslyCommitted(receipt) if receipt == *retained => {}
        _ => return Err(RepositoryError::InvalidReceipt),
    }
    if repository.load_current(scope, context)? != *current {
        return Err(RepositoryError::InvalidCandidate);
    }
    let old = actor_block_on(
        physical.runtime,
        physical_snapshot(
            physical.administrator,
            &scope.tenant,
            obsolete_candidate.basis(),
            physical.codec_limits.maximum_document_bytes,
            physical.deadline,
        ),
    )??;
    if old.family_counts != [1, 1, 1, 1]
        || old.session_sequence != current.basis().revision.sequence()
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    let document = old
        .envelope
        .as_ref()
        .ok_or(RepositoryError::InvalidCandidate)?;
    let decoded = crate::checkpoint_codec::decode_checkpoint(
        document,
        obsolete_candidate.basis(),
        obsolete_candidate.pins(),
        physical.admitted_inventory(),
        physical.model_limits,
        physical.codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    if decoded != *obsolete_candidate {
        return Err(RepositoryError::InvalidCandidate);
    }
    let receipt = decode_receipt(
        old.retained_receipt
            .as_ref()
            .ok_or(RepositoryError::InvalidReceipt)?,
        scope.session,
        scope.operation,
        physical.maximum_receipt_bytes,
        physical.codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidReceipt)?;
    if receipt != *retained {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

/// A full-u64 maximum current sequence remains readable but cannot execute a fresh decision.
/// The complete independent baseline document and all family counts must remain unchanged.
pub(crate) fn observe_sequence_exhaustion(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    baseline: &Checkpoint,
    candidate: &Checkpoint,
    context: &OperationContext,
    physical: &PhysicalObservation<'_>,
) -> Result<(), RepositoryError> {
    if baseline.basis().revision.sequence() != u64::MAX
        || repository.load_current(scope, context)? != *baseline
        || !matches!(
            repository.lookup_operation(scope, context)?,
            OperationLookup::NotRecorded
        )
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let before = actor_block_on(
        physical.runtime,
        physical_snapshot(
            physical.administrator,
            &scope.tenant,
            baseline.basis(),
            physical.codec_limits.maximum_document_bytes,
            physical.deadline,
        ),
    )??;
    if before.family_counts != [1, 0, 0, 0]
        || before.retained_receipt.is_some()
        || before.session_sequence != u64::MAX
        || before.envelope.is_none()
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    for _ in 0..2 {
        if !matches!(
            repository.commit_decision(scope, candidate, baseline.basis(), context),
            Err(RepositoryError::SequenceExhausted)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        if !matches!(
            repository.lookup_operation(scope, context)?,
            OperationLookup::NotRecorded
        ) || repository.load_current(scope, context)? != *baseline
        {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    let after = actor_block_on(
        physical.runtime,
        physical_snapshot(
            physical.administrator,
            &scope.tenant,
            baseline.basis(),
            physical.codec_limits.maximum_document_bytes,
            physical.deadline,
        ),
    )??;
    if after.family_counts != before.family_counts
        || after.envelope != before.envelope
        || after.retained_receipt != before.retained_receipt
        || after.session_sequence != before.session_sequence
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

/// Mutate only the registered owned fixture's current row, never the supplied scope.
/// Every exact refusal must leave the candidate absent; restore the original row
/// before inspecting bytes and reusing the very same runtime connection.
pub(crate) fn observe_current_owner_refusals(
    repository: &mut PostgresRepository<FixtureMembershipAuthority>,
    scope: &NativeScope<FixtureMembershipAuthority>,
    baseline: &Checkpoint,
    candidate: &Checkpoint,
    context: &OperationContext,
    physical: &PhysicalObservation<'_>,
) -> Result<(), RepositoryError> {
    let expected = baseline.basis();
    let original = actor_block_on(physical.runtime, async {
        tokio::time::timeout_at(
            physical.deadline,
            physical.administrator.query_one(
                "SELECT run_id, recovery_epoch::text, in_epoch_sequence::text, owner_fence,
                lease_until::text FROM df_game.sessions
                WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
                &[
                    &scope.tenant.as_slice(),
                    &expected.session.as_bytes().as_slice(),
                ],
            ),
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)
    })??;
    let run: Vec<u8> = original
        .try_get(0)
        .map_err(|_| RepositoryError::Unavailable)?;
    let epoch: String = original
        .try_get(1)
        .map_err(|_| RepositoryError::Unavailable)?;
    let sequence: String = original
        .try_get(2)
        .map_err(|_| RepositoryError::Unavailable)?;
    let fence: Vec<u8> = original
        .try_get(3)
        .map_err(|_| RepositoryError::Unavailable)?;
    let lease: String = original
        .try_get(4)
        .map_err(|_| RepositoryError::Unavailable)?;
    // Static statements and expectations are a closed test-only set.
    let cases = [
        (
            "UPDATE df_game.sessions SET run_id=decode(repeat('54',16),'hex') WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            RepositoryError::RevisionConflict,
        ),
        (
            "UPDATE df_game.sessions SET recovery_epoch=recovery_epoch+1 WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            RepositoryError::StaleEpoch,
        ),
        (
            "UPDATE df_game.sessions SET in_epoch_sequence=in_epoch_sequence+1 WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            RepositoryError::RevisionConflict,
        ),
        (
            "UPDATE df_game.sessions SET owner_fence=decode(repeat('55',16),'hex') WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            RepositoryError::StaleFence,
        ),
        (
            "UPDATE df_game.sessions SET lease_until=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            RepositoryError::ExpiredOwner,
        ),
    ];
    for (change, expected_error) in cases {
        let changed = actor_block_on(physical.runtime, async {
            tokio::time::timeout_at(
                physical.deadline,
                physical.administrator.execute(
                    change,
                    &[
                        &scope.tenant.as_slice(),
                        &expected.session.as_bytes().as_slice(),
                    ],
                ),
            )
            .await
            .map_err(|_| RepositoryError::Unavailable)?
            .map_err(|_| RepositoryError::Unavailable)
        })??;
        if changed != 1 {
            return Err(RepositoryError::InvalidReceipt);
        }
        let refused = repository.commit_decision(scope, candidate, expected, context);
        // Restore even an unexpected adapter result, without silently replacing that result.
        let restored = actor_block_on(physical.runtime, async {
            tokio::time::timeout_at(physical.deadline, physical.administrator.execute(
                "UPDATE df_game.sessions SET run_id=$3::bytea, recovery_epoch=$4::text::numeric,
                    in_epoch_sequence=$5::text::numeric, owner_fence=$6::bytea, lease_until=$7::text::timestamptz
                    WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
                &[&scope.tenant.as_slice(), &expected.session.as_bytes().as_slice(), &run, &epoch,
                    &sequence, &fence, &lease])).await.map_err(|_| RepositoryError::Unavailable)?
                    .map_err(|_| RepositoryError::Unavailable)
        })??;
        if restored != 1 || !matches!(refused, Err(error) if error == expected_error) {
            return Err(RepositoryError::InvalidReceipt);
        }
        let snapshot = actor_block_on(
            physical.runtime,
            physical_snapshot(
                physical.administrator,
                &scope.tenant,
                candidate.basis(),
                physical.codec_limits.maximum_document_bytes,
                physical.deadline,
            ),
        )??;
        validate_physical_rollback(&snapshot, expected)?;
        if !matches!(
            repository.lookup_operation(scope, context)?,
            OperationLookup::NotRecorded
        ) || repository.load_current(scope, context)? != *baseline
        {
            return Err(RepositoryError::InvalidReceipt);
        }
    }
    let receipt = match repository.commit_decision(scope, candidate, expected, context)? {
        CommitOutcome::Confirmed(receipt) => receipt,
        _ => return Err(RepositoryError::InvalidReceipt),
    };
    physical.check_commit(scope, candidate, &receipt)?;
    if repository.load_current(scope, context)? != *candidate {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}
