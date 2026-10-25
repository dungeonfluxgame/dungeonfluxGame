//! Fixture-only composition from registered real connections and real capability producer.
//! No runtime/process/database is created, and no production scope issuer is claimed.
use crate::checkpoint_codec::CodecLimits;
use crate::decision_adapter::{DecisionAdapter, RecoverySource};
use crate::native_bridge::PostgresRepository;
use crate::native_connection::{TransactionBounds, actor_block_on};
use crate::native_scope::{DatabaseBindingVerifier, NativeScope};
use crate::owned_pg_membership_fixture::{
    FixtureCampaign, FixtureMembershipAuthority, FixturePrincipal, FixtureRole, FixtureTenant,
};
use crate::owned_pg_scope_fixture::{FixtureBoundInput, fixture_scope, fixture_verifier};
use df_auth::membership::{MembershipError, MembershipRequest, authorize_membership};
use df_session::submission::RepositoryError;
use tokio::time::Instant;
use tokio_postgres::Row;

pub(crate) fn bind_registered_scope(
    authority: &mut FixtureMembershipAuthority,
    proof: &Row,
    bound: FixtureBoundInput,
    current_database_unix_seconds: u64,
) -> Result<
    (
        NativeScope<FixtureMembershipAuthority>,
        DatabaseBindingVerifier,
    ),
    RepositoryError,
> {
    fn decode_identity(row: &Row, column: &str) -> Result<[u8; 16], RepositoryError> {
        let identity_bytes: Vec<u8> = row
            .try_get(column)
            .map_err(|_| RepositoryError::Unauthorized)?;
        let identity: [u8; 16] = identity_bytes
            .try_into()
            .map_err(|_| RepositoryError::Unauthorized)?;
        if identity == [0; 16] {
            return Err(RepositoryError::Unauthorized);
        }
        Ok(identity)
    }
    let principal = FixturePrincipal(decode_identity(proof, "principal_id")?);
    let tenant = FixtureTenant(decode_identity(proof, "tenant_id")?);
    let campaign = FixtureCampaign(decode_identity(proof, "campaign_id")?);
    let role = FixtureRole(
        proof
            .try_get("effective_role")
            .map_err(|_| RepositoryError::Unauthorized)?,
    );
    if role.0.is_empty() || role.0.len() > 128 {
        return Err(RepositoryError::Unauthorized);
    }
    let capability = authorize_membership(
        authority,
        MembershipRequest {
            principal: &principal,
            tenant: &tenant,
            campaign: &campaign,
            role: &role,
        },
        current_database_unix_seconds,
    )
    .map_err(|error| match error {
        MembershipError::Source(error) => error,
        _ => RepositoryError::Unauthorized,
    })?;
    let scope = fixture_scope(capability, proof, bound)?;
    let runtime = authority.runtime.clone();
    let verifier = actor_block_on(&runtime, async {
        let deadline = Instant::now()
            .checked_add(authority.query_bound)
            .ok_or(RepositoryError::Capacity)?;
        let client = authority.connection.take_client()?;
        let result = fixture_verifier(&client, deadline).await;
        if result.is_err() {
            drop(client);
            authority.connection.discard().await?;
            return result;
        }
        if authority.connection.return_client(client).is_err() {
            authority.connection.discard().await?;
            return Err(RepositoryError::Unavailable);
        }
        result
    })??;
    Ok((scope, verifier))
}

/// Bounds are validated while the caller still owns and can explicitly close the authority.
/// The prepared Statement belongs to this exact connection; no pool substitution is used.
pub(crate) fn validate_fixture_composition(
    codec_limits: CodecLimits,
    maximum_receipt_bytes: usize,
    bounds: TransactionBounds,
) -> Result<(), RepositoryError> {
    bounds.validate()?;
    codec_limits
        .validate()
        .map_err(|_| RepositoryError::Capacity)?;
    if maximum_receipt_bytes == 0 {
        return Err(RepositoryError::Capacity);
    }
    Ok(())
}

pub(crate) fn compose_registered_repository(
    authority: FixtureMembershipAuthority,
    codec_limits: CodecLimits,
    maximum_receipt_bytes: usize,
    verifier: DatabaseBindingVerifier,
    recovery: RecoverySource,
    bounds: TransactionBounds,
) -> Result<PostgresRepository<FixtureMembershipAuthority>, RepositoryError> {
    validate_fixture_composition(codec_limits, maximum_receipt_bytes, bounds)?;
    let runtime = authority.runtime.clone();
    let connection = authority.into_owned_connection();
    let adapter = DecisionAdapter::from_prechecked_owned(
        connection,
        bounds,
        codec_limits,
        maximum_receipt_bytes,
        verifier,
        Some(recovery),
    );
    Ok(PostgresRepository::from_owned(runtime, adapter))
}
