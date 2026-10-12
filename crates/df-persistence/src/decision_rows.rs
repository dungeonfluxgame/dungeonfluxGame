use df_model::checkpoint::{Basis, Checkpoint, DurableIntent, EffectKind, ExecutionMode, GameFact};
use df_session::submission::{DecisionReceipt, OperationLookup, RepositoryError};
use df_types::{RunId, SessionRevision};
use tokio_postgres::{Row, Transaction};

use crate::checkpoint_codec::{
    CodecLimits, STORAGE_CODEC_VERSION, decode_receipt, validate_storage_format,
};
use crate::native_scope::NativeScope;
use crate::sql::*;

pub(crate) struct LockedSession {
    run: RunId,
    revision: SessionRevision,
    fence: Vec<u8>,
    lease_current: bool,
}
impl LockedSession {
    pub(crate) fn basis(&self, session: df_types::SessionId) -> Basis {
        Basis {
            session,
            run: self.run,
            revision: self.revision,
        }
    }
    pub(crate) fn validate_new_decision<A: df_auth::membership::MembershipAuthority>(
        &self,
        scope: &NativeScope<A>,
        expected: Basis,
    ) -> Result<(), RepositoryError> {
        if expected.session != scope.session() {
            return Err(RepositoryError::InputBinding);
        }
        if self.run != expected.run {
            return Err(RepositoryError::RevisionConflict);
        }
        if self.revision.epoch() != expected.revision.epoch()
            || scope.epoch() != self.revision.epoch()
        {
            return Err(RepositoryError::StaleEpoch);
        }
        if self.revision != expected.revision {
            return Err(RepositoryError::RevisionConflict);
        }
        self.validate_current_owner(scope)
    }
    // Reload selects the current row's basis. The retained operation key can precede
    // a restore epoch; authority still requires this exact current owner and strict lease.
    pub(crate) fn validate_current_owner<A: df_auth::membership::MembershipAuthority>(
        &self,
        scope: &NativeScope<A>,
    ) -> Result<(), RepositoryError> {
        if self.fence.as_slice() != scope.owner_fence_bytes() {
            return Err(RepositoryError::StaleFence);
        }
        if !self.lease_current {
            return Err(RepositoryError::ExpiredOwner);
        }
        Ok(())
    }
}
fn decode_revision(
    row: &Row,
    epoch_column: &str,
    sequence_column: &str,
) -> Result<SessionRevision, RepositoryError> {
    let epoch = row
        .try_get::<_, String>(epoch_column)
        .map_err(|_| RepositoryError::Unavailable)?;
    let sequence = row
        .try_get::<_, String>(sequence_column)
        .map_err(|_| RepositoryError::Unavailable)?;
    crate::revision_codec::decode_revision(&epoch, &sequence)
        .map_err(|_| RepositoryError::Unavailable)
}

pub(crate) async fn lock_session<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
) -> Result<LockedSession, RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let row = transaction
        .query_opt(LOCK_SESSION, &[&tenant, &session.as_bytes().as_slice()])
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .ok_or(RepositoryError::Unauthorized)?;
    let run_bytes = row
        .try_get::<_, Vec<u8>>("run_id")
        .map_err(|_| RepositoryError::Unavailable)?;
    let run = RunId::from_bytes(&run_bytes).map_err(|_| RepositoryError::Unavailable)?;
    Ok(LockedSession {
        run,
        revision: decode_revision(&row, "recovery_epoch", "in_epoch_sequence")?,
        fence: row
            .try_get("owner_fence")
            .map_err(|_| RepositoryError::Unavailable)?,
        lease_current: row
            .try_get("lease_current")
            .map_err(|_| RepositoryError::Unavailable)?,
    })
}

pub(crate) async fn lookup_locked<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    codec_limits: CodecLimits,
    maximum_receipt_bytes: usize,
) -> Result<OperationLookup, RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let principal = scope.principal_bytes();
    let namespace = scope.namespace_bytes();
    let epoch = scope.epoch().get().to_string();
    let operation = scope.operation();
    let maximum_document_bytes = i64::try_from(codec_limits.maximum_document_bytes)
        .map_err(|_| RepositoryError::Capacity)?;
    let parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
        &tenant,
        &session.as_bytes().as_slice(),
        &principal,
        &namespace,
        &epoch,
        &operation.as_bytes().as_slice(),
        &maximum_document_bytes,
    ];
    if let Some(row) = transaction
        .query_opt(LOOKUP_OPERATION, parameters)
        .await
        .map_err(|_| RepositoryError::Unavailable)?
    {
        let fingerprint_version: i32 = row
            .try_get("fingerprint_version")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        let fingerprint: Vec<u8> = row
            .try_get("canonical_fingerprint")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        if fingerprint_version != scope.fingerprint_version()
            || fingerprint.as_slice() != scope.fingerprint_bytes()
        {
            return Ok(OperationLookup::Conflict);
        }
        let receipt_bytes: Option<i64> = row
            .try_get("receipt_bytes")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        if receipt_bytes.is_some_and(|length| length < 0 || length > maximum_document_bytes) {
            return Err(RepositoryError::InvalidReceipt);
        }
        let receipt: Option<Vec<u8>> = row
            .try_get("receipt")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        let Some(receipt) = receipt else {
            return Ok(OperationLookup::ExpiredOrIndeterminate);
        };
        let version: i32 = row
            .try_get("receipt_version")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        validate_storage_format(version, b"DFRC", &receipt)
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        let decoded = decode_receipt(
            &receipt,
            session,
            operation,
            maximum_receipt_bytes,
            codec_limits,
        )
        .map_err(|_| RepositoryError::InvalidReceipt)?;
        let committed = decode_revision(&row, "committed_epoch", "committed_sequence")?;
        let committed_run: Vec<u8> = row
            .try_get("committed_run_id")
            .map_err(|_| RepositoryError::InvalidReceipt)?;
        let committed_run =
            RunId::from_bytes(&committed_run).map_err(|_| RepositoryError::InvalidReceipt)?;
        if decoded.basis().revision != committed || decoded.basis().run != committed_run {
            return Err(RepositoryError::InvalidReceipt);
        }
        return Ok(OperationLookup::Committed(decoded));
    }
    let parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
        &tenant,
        &session.as_bytes().as_slice(),
        &principal,
        &namespace,
        &epoch,
    ];
    let retired: bool = transaction
        .query_one(NAMESPACE_RETIRED, parameters)
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .try_get(0)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(if retired {
        OperationLookup::ExpiredOrIndeterminate
    } else {
        OperationLookup::NotRecorded
    })
}

pub(crate) async fn insert_checkpoint<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    checkpoint: &Checkpoint,
    document: &[u8],
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let basis = checkpoint.basis();
    let (epoch, sequence) = crate::revision_codec::encode_revision(basis.revision);
    let schema = i32::from(checkpoint.schema());
    let codec = crate::checkpoint_codec::checkpoint_storage_version(document)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    validate_storage_format(codec, b"DFCP", document)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let changed = transaction
        .execute(
            INSERT_CHECKPOINT,
            &[
                &tenant,
                &basis.session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &basis.run.as_bytes().as_slice(),
                &schema,
                &codec,
                &document,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}
pub(crate) async fn insert_fact<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    fact: &GameFact,
    document: &[u8],
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let (epoch, sequence) = crate::revision_codec::encode_revision(fact.revision);
    let ordinal = i64::from(fact.ordinal);
    let version = STORAGE_CODEC_VERSION;
    validate_storage_format(version, b"DFFA", document)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let changed = transaction
        .execute(
            INSERT_FACT,
            &[
                &tenant,
                &session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &ordinal,
                &version,
                &document,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}
pub(crate) async fn insert_operation<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    receipt: &DecisionReceipt,
    document: &[u8],
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let principal = scope.principal_bytes();
    let namespace = scope.namespace_bytes();
    let epoch = scope.epoch().get().to_string();
    let operation = scope.operation();
    let fingerprint_version = scope.fingerprint_version();
    let fingerprint = scope.fingerprint_bytes();
    let committed_epoch = receipt.basis().revision.epoch().get().to_string();
    let sequence = receipt.basis().revision.sequence().to_string();
    let version = STORAGE_CODEC_VERSION;
    validate_storage_format(version, b"DFRC", document)
        .map_err(|_| RepositoryError::InvalidReceipt)?;
    let changed = transaction
        .execute(
            INSERT_OPERATION,
            &[
                &tenant,
                &session.as_bytes().as_slice(),
                &principal,
                &namespace,
                &epoch,
                &operation.as_bytes().as_slice(),
                &fingerprint_version,
                &fingerprint,
                &committed_epoch,
                &sequence,
                &version,
                &document,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}
pub(crate) async fn insert_intent<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    mode: ExecutionMode,
    intent: &DurableIntent,
    document: &[u8],
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let principal = scope.principal_bytes();
    let namespace = scope.namespace_bytes();
    let epoch = scope.epoch().get().to_string();
    let operation = scope.operation();
    let slot = i64::from(intent.slot);
    let effect = intent.id.as_bytes().as_slice();
    let job = intent.job.as_ref().map(|job| job.as_bytes().as_slice());
    let timer = intent
        .timer
        .as_ref()
        .map(|timer| timer.as_bytes().as_slice());
    let run = intent.basis.run.as_bytes().as_slice();
    let committed_epoch = intent.basis.revision.epoch().get().to_string();
    let sequence = intent.basis.revision.sequence().to_string();
    let generation = intent.generation.to_string();
    let mode = match mode {
        ExecutionMode::Live => 1_i16,
        ExecutionMode::PreparedOnly => 2,
        ExecutionMode::Replay => 3,
    };
    let kind = match intent.kind {
        EffectKind::RunAi => 1_i16,
        EffectKind::RunMedia => 2,
        EffectKind::LoadMemoryCandidates => 3,
        EffectKind::ArmTimer => 4,
        EffectKind::CancelJob => 5,
        EffectKind::CancelTimer => 6,
        EffectKind::PublishPresentation => 7,
    };
    let version = STORAGE_CODEC_VERSION;
    validate_storage_format(version, b"DFIT", document)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let changed = transaction
        .execute(
            INSERT_INTENT,
            &[
                &tenant,
                &session.as_bytes().as_slice(),
                &principal,
                &namespace,
                &epoch,
                &operation.as_bytes().as_slice(),
                &slot,
                &effect,
                &job,
                &timer,
                &run,
                &committed_epoch,
                &sequence,
                &generation,
                &mode,
                &kind,
                &version,
                &document,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}
pub(crate) async fn final_revision_cas<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    expected: Basis,
    committed: Basis,
) -> Result<(), RepositoryError> {
    let tenant = scope.tenant_bytes();
    let fence = scope.owner_fence_bytes();
    let (epoch, sequence) = crate::revision_codec::encode_revision(expected.revision);
    let row = transaction
        .query_opt(
            FINAL_REVISION_CAS,
            &[
                &tenant,
                &expected.session.as_bytes().as_slice(),
                &expected.run.as_bytes().as_slice(),
                &fence,
                &epoch,
                &sequence,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .ok_or(RepositoryError::RevisionConflict)?;
    if decode_revision(&row, "recovery_epoch", "in_epoch_sequence")? != committed.revision {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

pub(crate) async fn load_current<A: df_auth::membership::MembershipAuthority>(
    transaction: &Transaction<'_>,
    scope: &NativeScope<A>,
    inventory: df_model::checkpoint::ReferenceInventory<'_>,
    checkpoint_limits: df_model::checkpoint::CheckpointLimits,
    codec_limits: CodecLimits,
    current_basis: Basis,
) -> Result<Checkpoint, RepositoryError> {
    let tenant = scope.tenant_bytes();
    let session = scope.session();
    let maximum_document_bytes = i64::try_from(codec_limits.maximum_document_bytes)
        .map_err(|_| RepositoryError::Capacity)?;
    let row = transaction
        .query_opt(
            LOAD_CURRENT,
            &[
                &tenant,
                &session.as_bytes().as_slice(),
                &maximum_document_bytes,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .ok_or(RepositoryError::Unavailable)?;
    decode_current_row(
        &row,
        scope.session(),
        current_basis,
        scope.admitted_pins(),
        inventory,
        checkpoint_limits,
        codec_limits,
    )
}

pub(crate) fn current_row_basis(
    row: &Row,
    session: df_types::SessionId,
) -> Result<Basis, RepositoryError> {
    let run_bytes: Vec<u8> = row
        .try_get("run_id")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    Ok(Basis {
        session,
        run: RunId::from_bytes(&run_bytes).map_err(|_| RepositoryError::InvalidCandidate)?,
        revision: decode_revision(row, "recovery_epoch", "in_epoch_sequence")?,
    })
}

pub(crate) fn decode_current_row(
    row: &Row,
    session: df_types::SessionId,
    current_basis: Basis,
    pins: &df_model::checkpoint::CheckpointPins,
    inventory: df_model::checkpoint::ReferenceInventory<'_>,
    checkpoint_limits: df_model::checkpoint::CheckpointLimits,
    codec_limits: CodecLimits,
) -> Result<Checkpoint, RepositoryError> {
    let expected = current_row_basis(row, session)?;
    if expected != current_basis {
        return Err(RepositoryError::InvalidCandidate);
    }
    let schema: i32 = row
        .try_get("schema_version")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let codec: i32 = row
        .try_get("codec_version")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    if schema != i32::from(df_model::checkpoint::CHECKPOINT_SCHEMA) {
        return Err(RepositoryError::InvalidCandidate);
    }
    let document: Vec<u8> = row
        .try_get("complete_envelope")
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    validate_storage_format(codec, b"DFCP", &document)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    crate::checkpoint_codec::decode_checkpoint(
        &document,
        expected,
        pins,
        inventory,
        checkpoint_limits,
        codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)
}
