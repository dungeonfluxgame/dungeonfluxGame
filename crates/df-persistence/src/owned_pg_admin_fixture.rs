//! Registered owned-database fixture administration, never a runtime authority API.
//! Call only with ROOT's registered isolated administrator connection and finite deadline.
use crate::checkpoint_codec::{
    CodecLimits, STORAGE_CODEC_VERSION, decode_checkpoint, encode_checkpoint,
};
use df_model::checkpoint::{Basis, Checkpoint, CheckpointLimits, ReferenceInventory};
use df_session::submission::RepositoryError;
use tokio::time::{Instant, timeout_at};
use tokio_postgres::{Client, Row, Transaction};

pub(crate) fn fixture_storage_header(magic: [u8; 4]) -> Result<[u8; 6], RepositoryError> {
    let version =
        u16::try_from(STORAGE_CODEC_VERSION).map_err(|_| RepositoryError::InvalidCandidate)?;
    let mut header = [0_u8; 6];
    header[..4].copy_from_slice(&magic);
    header[4..].copy_from_slice(&version.to_be_bytes());
    Ok(header)
}

pub(crate) struct FixtureGrant<'a> {
    pub(crate) service_role: &'a str,
    pub(crate) tenant: &'a [u8; 16],
    pub(crate) principal: &'a [u8; 16],
    pub(crate) campaign: &'a [u8; 16],
    pub(crate) role: &'a [u8],
    pub(crate) access_revision: &'a [u8],
    pub(crate) lifetime_seconds: i64,
}
pub(crate) struct FixtureProof<'a> {
    pub(crate) grant: FixtureGrant<'a>,
    pub(crate) basis: Basis,
    pub(crate) operation: &'a [u8; 16],
    pub(crate) namespace: &'a [u8],
    pub(crate) canonical_fingerprint: &'a [u8; 32],
    pub(crate) fence: &'a [u8; 16],
    pub(crate) mode: i16,
    pub(crate) lookup_only: bool,
}
impl FixtureGrant<'_> {
    fn validate(&self) -> Result<(), RepositoryError> {
        if !matches!(
            self.service_role,
            "df_persistence_fixture_runtime_a" | "df_persistence_fixture_runtime_b"
        ) || *self.tenant == [0; 16]
            || *self.principal == [0; 16]
            || *self.campaign == [0; 16]
            || self.role.is_empty()
            || self.role.len() > 128
            || self.access_revision.is_empty()
            || self.access_revision.len() > 128
            || !(1..=300).contains(&self.lifetime_seconds)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }
}

/// Seed an immutable canonical baseline before runtime roles receive a bound scope.
/// Stored bytes must roundtrip under the fixture's independently admitted inventory.
pub(crate) async fn seed_session(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    fence: &[u8; 16],
    checkpoint: &Checkpoint,
    inventory: ReferenceInventory<'_>,
    validation_limits: (CheckpointLimits, CodecLimits),
    lease_seconds: i64,
) -> Result<(), RepositoryError> {
    let (model_limits, codec_limits) = validation_limits;
    if *tenant == [0; 16] || *fence == [0; 16] || !(1..=300).contains(&lease_seconds) {
        return Err(RepositoryError::InvalidCandidate);
    }
    let checkpoint_document = encode_checkpoint(checkpoint, codec_limits)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let decoded = decode_checkpoint(
        &checkpoint_document,
        checkpoint.basis(),
        checkpoint.pins(),
        inventory,
        model_limits,
        codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    if decoded != *checkpoint {
        return Err(RepositoryError::InvalidCandidate);
    }
    let basis = checkpoint.basis();
    let epoch = basis.revision.epoch().get().to_string();
    let sequence = basis.revision.sequence().to_string();
    let inserted_session_count = transaction
        .execute(
            "INSERT INTO df_game.sessions (tenant_id, session_id, run_id,
        recovery_epoch, in_epoch_sequence, owner_fence, lease_until)
        VALUES ($1::bytea, $2::bytea, $3::bytea, $4::text::numeric, $5::text::numeric,
            $6::bytea, clock_timestamp() + $7::bigint * interval '1 second')",
            &[
                &tenant.as_slice(),
                &basis.session.as_bytes().as_slice(),
                &basis.run.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &fence.as_slice(),
                &lease_seconds,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if inserted_session_count != 1 {
        return Err(RepositoryError::Unavailable);
    }
    let inserted_checkpoint_count = transaction
        .execute(
            crate::sql::INSERT_CHECKPOINT,
            &[
                &tenant.as_slice(),
                &basis.session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &basis.run.as_bytes().as_slice(),
                &i32::from(checkpoint.schema()),
                &STORAGE_CODEC_VERSION,
                &checkpoint_document,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if inserted_checkpoint_count != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}

pub(crate) async fn seed_grant(
    transaction: &Transaction<'_>,
    grant: &FixtureGrant<'_>,
) -> Result<(), RepositoryError> {
    grant.validate()?;
    let inserted_grant_count = transaction.execute("INSERT INTO df_fixture_authority.grants
        (service_role, tenant_id, principal_id, campaign_id, effective_role, access_revision, active, expires_at)
        VALUES ($1::text::name, $2::bytea, $3::bytea, $4::bytea, $5::bytea, $6::bytea, true,
            clock_timestamp() + $7::bigint * interval '1 second')",
        &[&grant.service_role, &grant.tenant.as_slice(), &grant.principal.as_slice(),
            &grant.campaign.as_slice(), &grant.role, &grant.access_revision, &grant.lifetime_seconds])
        .await.map_err(|_| RepositoryError::Unavailable)?;
    if inserted_grant_count != 1 {
        return Err(RepositoryError::Unavailable);
    }
    Ok(())
}

/// Return the actual administrator-owned registered proof row with base driver types.
/// Its fingerprint/input association is the explicitly registered fixture case, not a
/// production fingerprint algorithm or a client-issued continuing grant.
pub(crate) async fn seed_proof(
    transaction: &Transaction<'_>,
    proof: &FixtureProof<'_>,
) -> Result<Row, RepositoryError> {
    proof.grant.validate()?;
    if *proof.operation == [0; 16]
        || *proof.fence == [0; 16]
        || proof.namespace.is_empty()
        || proof.namespace.len() > 128
        || !(1..=3).contains(&proof.mode)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    transaction
        .query_one(
            "INSERT INTO df_fixture_authority.scope_proofs
        (service_role, tenant_id, principal_id, campaign_id, effective_role, access_revision,
         session_id, operation_id, command_namespace, recovery_epoch, fingerprint_version,
         canonical_fingerprint, owner_fence, execution_mode, lookup_only, expires_at)
        VALUES ($1::text::name, $2::bytea, $3::bytea, $4::bytea, $5::bytea, $6::bytea,
            $7::bytea, $8::bytea, $9::bytea, $10::text::numeric, 1, $11::bytea,
            $12::bytea, $13::smallint, $14::boolean,
            clock_timestamp() + $15::bigint * interval '1 second')
        RETURNING binding, tenant_id, principal_id, campaign_id, effective_role, access_revision,
            session_id, operation_id, command_namespace, recovery_epoch::text,
            fingerprint_version, canonical_fingerprint, owner_fence, execution_mode, lookup_only",
            &[
                &proof.grant.service_role,
                &proof.grant.tenant.as_slice(),
                &proof.grant.principal.as_slice(),
                &proof.grant.campaign.as_slice(),
                &proof.grant.role,
                &proof.grant.access_revision,
                &proof.basis.session.as_bytes().as_slice(),
                &proof.operation.as_slice(),
                &proof.namespace,
                &proof.basis.revision.epoch().get().to_string(),
                &proof.canonical_fingerprint.as_slice(),
                &proof.fence.as_slice(),
                &proof.mode,
                &proof.lookup_only,
                &proof.grant.lifetime_seconds,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)
}

pub(crate) struct PhysicalDecisionSnapshot {
    pub(crate) family_counts: [u64; 4],
    pub(crate) envelope: Option<Vec<u8>>,
    pub(crate) retained_receipt: Option<Vec<u8>>,
    pub(crate) session_sequence: u64,
    pub(crate) format_metadata_matches: bool,
}
/// Inspect one exact committed revision using the independently owned admin connection.
/// Bounds are applied by SQL before any stored byte document reaches the driver allocator.
pub(crate) async fn physical_snapshot(
    client: &Client,
    tenant: &[u8; 16],
    basis: Basis,
    maximum_document_bytes: usize,
    deadline: Instant,
) -> Result<PhysicalDecisionSnapshot, RepositoryError> {
    let maximum = i64::try_from(maximum_document_bytes).map_err(|_| RepositoryError::Capacity)?;
    if maximum == 0 {
        return Err(RepositoryError::Capacity);
    }
    let epoch = basis.revision.epoch().get().to_string();
    let sequence = basis.revision.sequence().to_string();
    let checkpoint_header = fixture_storage_header(*b"DFCP")?;
    let fact_header = fixture_storage_header(*b"DFFA")?;
    let receipt_header = fixture_storage_header(*b"DFRC")?;
    let intent_header = fixture_storage_header(*b"DFIT")?;
    let row = timeout_at(deadline, client.query_one("SELECT
        (SELECT count(*) FROM df_game.checkpoints WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND recovery_epoch=$3::text::numeric AND in_epoch_sequence=$4::text::numeric) AS checkpoints,
        (SELECT count(*) FROM df_game.facts WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric) AS facts,
        (SELECT count(*) FROM df_game.operations WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric) AS operations,
        (SELECT count(*) FROM df_game.intents WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric) AS intents,
        (SELECT complete_envelope FROM df_game.checkpoints WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND recovery_epoch=$3::text::numeric AND in_epoch_sequence=$4::text::numeric
            AND octet_length(complete_envelope) <= $5::bigint) AS envelope,
        (SELECT receipt FROM df_game.operations WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric
            AND octet_length(receipt) <= $5::bigint) AS receipt,
        NOT EXISTS (SELECT 1 FROM df_game.checkpoints WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND recovery_epoch=$3::text::numeric AND in_epoch_sequence=$4::text::numeric
            AND (schema_version <> $6::integer OR codec_version <> $7::integer
                OR substring(complete_envelope FROM 1 FOR 6) <> $8::bytea))
        AND NOT EXISTS (SELECT 1 FROM df_game.facts WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric
            AND (fact_version <> $7::integer OR substring(fact FROM 1 FOR 6) <> $9::bytea))
        AND NOT EXISTS (SELECT 1 FROM df_game.operations WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric
            AND (receipt_version <> $7::integer OR receipt IS NULL
                OR substring(receipt FROM 1 FOR 6) <> $10::bytea))
        AND NOT EXISTS (SELECT 1 FROM df_game.intents WHERE tenant_id=$1::bytea AND session_id=$2::bytea
            AND committed_epoch=$3::text::numeric AND committed_sequence=$4::text::numeric
            AND (intent_version <> $7::integer OR substring(intent FROM 1 FOR 6) <> $11::bytea))
        AS format_metadata_matches,
        (SELECT in_epoch_sequence::text FROM df_game.sessions
            WHERE tenant_id=$1::bytea AND session_id=$2::bytea) AS session_sequence",
        &[&tenant.as_slice(), &basis.session.as_bytes().as_slice(), &epoch, &sequence, &maximum,
          &i32::from(df_model::checkpoint::CHECKPOINT_SCHEMA), &STORAGE_CODEC_VERSION,
          &checkpoint_header.as_slice(), &fact_header.as_slice(),
          &receipt_header.as_slice(), &intent_header.as_slice()]))
        .await.map_err(|_| RepositoryError::Unavailable)?.map_err(|_| RepositoryError::Unavailable)?;
    let mut counts = [0; 4];
    for (slot, column) in ["checkpoints", "facts", "operations", "intents"]
        .iter()
        .enumerate()
    {
        let family_row_count: i64 = row
            .try_get(*column)
            .map_err(|_| RepositoryError::Unavailable)?;
        counts[slot] = u64::try_from(family_row_count).map_err(|_| RepositoryError::Unavailable)?;
    }
    let sequence: String = row
        .try_get("session_sequence")
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(PhysicalDecisionSnapshot {
        family_counts: counts,
        envelope: row
            .try_get("envelope")
            .map_err(|_| RepositoryError::Unavailable)?,
        retained_receipt: row
            .try_get("receipt")
            .map_err(|_| RepositoryError::Unavailable)?,
        session_sequence: crate::revision_codec::decode_unsigned_number(&sequence)
            .map_err(|_| RepositoryError::Unavailable)?,
        format_metadata_matches: row
            .try_get("format_metadata_matches")
            .map_err(|_| RepositoryError::Unavailable)?,
    })
}

pub(crate) fn validate_physical_commit(
    snapshot: &PhysicalDecisionSnapshot,
    checkpoint: &Checkpoint,
    inventory: ReferenceInventory<'_>,
    model_limits: CheckpointLimits,
    codec_limits: CodecLimits,
) -> Result<(), RepositoryError> {
    let basis = checkpoint.basis();
    let expected = [
        1,
        u64::try_from(
            checkpoint
                .state()
                .facts
                .iter()
                .filter(|fact| fact.revision == basis.revision)
                .count(),
        )
        .map_err(|_| RepositoryError::Capacity)?,
        1,
        u64::try_from(
            checkpoint
                .state()
                .intents
                .iter()
                .filter(|intent| intent.basis == basis)
                .count(),
        )
        .map_err(|_| RepositoryError::Capacity)?,
    ];
    if snapshot.family_counts != expected
        || !snapshot.format_metadata_matches
        || snapshot.session_sequence != basis.revision.sequence()
        || snapshot.retained_receipt.is_none()
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    let envelope = snapshot
        .envelope
        .as_ref()
        .ok_or(RepositoryError::InvalidCandidate)?;
    let decoded = decode_checkpoint(
        envelope,
        basis,
        checkpoint.pins(),
        inventory,
        model_limits,
        codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    if decoded != *checkpoint {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

pub(crate) fn validate_physical_rollback(
    snapshot: &PhysicalDecisionSnapshot,
    expected: Basis,
) -> Result<(), RepositoryError> {
    if snapshot.family_counts != [0; 4]
        || snapshot.envelope.is_some()
        || snapshot.retained_receipt.is_some()
        || snapshot.session_sequence != expected.revision.sequence()
    {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

/// Install a ROOT-admitted fixture checkpoint in a later epoch using one real admin
/// transaction. This is a read-boundary fixture, not a production restore algorithm.
/// Old immutable checkpoint/receipt/fact/intent rows remain byte-for-byte retained.
pub(crate) async fn install_fixture_current_epoch(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    previous: Basis,
    fence: &[u8; 16],
    current: &Checkpoint,
    inventory: ReferenceInventory<'_>,
    validation_limits: (CheckpointLimits, CodecLimits),
) -> Result<(), RepositoryError> {
    let (model_limits, codec_limits) = validation_limits;
    let next = current.basis();
    if next.session != previous.session
        || next.run != previous.run
        || next.revision.epoch() <= previous.revision.epoch()
        || *fence == [0; 16]
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let bytes =
        encode_checkpoint(current, codec_limits).map_err(|_| RepositoryError::InvalidCandidate)?;
    let decoded = decode_checkpoint(
        &bytes,
        next,
        current.pins(),
        inventory,
        model_limits,
        codec_limits,
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    if decoded != *current {
        return Err(RepositoryError::InvalidCandidate);
    }
    let (epoch, sequence) = crate::revision_codec::encode_revision(next.revision);
    let inserted = transaction
        .execute(
            crate::sql::INSERT_CHECKPOINT,
            &[
                &tenant.as_slice(),
                &next.session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &next.run.as_bytes().as_slice(),
                &i32::from(current.schema()),
                &STORAGE_CODEC_VERSION,
                &bytes,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if inserted != 1 {
        return Err(RepositoryError::Unavailable);
    }
    let (old_epoch, old_sequence) = crate::revision_codec::encode_revision(previous.revision);
    let updated = transaction
        .execute(
            "UPDATE df_game.sessions SET recovery_epoch=$3::text::numeric,
        in_epoch_sequence=$4::text::numeric, owner_fence=$5::bytea,
        lease_until=clock_timestamp()+interval '120 seconds'
        WHERE tenant_id=$1::bytea AND session_id=$2::bytea AND run_id=$6::bytea
        AND recovery_epoch=$7::text::numeric AND in_epoch_sequence=$8::text::numeric",
            &[
                &tenant.as_slice(),
                &next.session.as_bytes().as_slice(),
                &epoch,
                &sequence,
                &fence.as_slice(),
                &previous.run.as_bytes().as_slice(),
                &old_epoch,
                &old_sequence,
            ],
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?;
    if updated != 1 {
        return Err(RepositoryError::RevisionConflict);
    }
    Ok(())
}
