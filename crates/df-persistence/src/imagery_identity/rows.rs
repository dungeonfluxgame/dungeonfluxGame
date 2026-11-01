//! Parameterized identity statements in the caller transaction. Inserts use
//! ON CONFLICT so race reconciliation never queries an aborted transaction.

use tokio_postgres::Transaction;

use super::{HostRunBinding, HostSeatBinding, HostSeatKey, IdentityError, LocalGuestBinding};

/// Owned unique constraints of migration 0041: a violation here means the key
/// already names a row, so the caller rereads it instead of deciding from the
/// error. Any other database error stays unavailable.
const OWNED_UNIQUE_CONSTRAINTS: [&str; 3] = [
    "image_host_run_binding_pkey",
    "image_host_seat_binding_pkey",
    "image_local_guest_pkey",
];

pub(super) fn unavailable_of(error: tokio_postgres::Error) -> IdentityError {
    let (sqlstate, detail) = match error.as_db_error() {
        Some(db) => (
            Some(db.code().code().to_owned()),
            "imagery identity unavailable".to_owned(),
        ),
        None => (None, "imagery identity unavailable".to_owned()),
    };
    IdentityError::Unavailable { sqlstate, detail }
}

/// A unique violation of an owned binding constraint is a deterministic
/// conflict; anything else keeps its SQLSTATE as unavailable. No payload ever
/// enters the detail.
pub(super) fn statement_failure(error: tokio_postgres::Error) -> IdentityError {
    let db = error.as_db_error();
    let code = db.map(|db| db.code().code()).unwrap_or("");
    if code == "23505"
        && db
            .and_then(|db| db.constraint())
            .is_some_and(|name| OWNED_UNIQUE_CONSTRAINTS.contains(&name))
    {
        return IdentityError::Conflict;
    }
    unavailable_of(error)
}

/// Adapts fixed-size ids to the driver's byte-slice parameter shape.
fn bytes(value: &[u8]) -> &[u8] {
    value
}

pub(super) async fn read_host_run(
    transaction: &Transaction<'_>,
    binding: &HostRunBinding,
) -> Result<Option<HostRunBinding>, IdentityError> {
    read_host_run_by_key(
        transaction,
        &binding.tenant,
        &binding.campaign,
        &binding.hosted_session,
    )
    .await
}

pub(super) async fn read_host_run_by_key(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
) -> Result<Option<HostRunBinding>, IdentityError> {
    let row = transaction
        .query_opt(
            "SELECT tenant_id, campaign_id, hosted_session_id, \
             gameplay_tenant_id, gameplay_session_id, run_id, owner_fence \
             FROM imagery.image_host_run_binding \
             WHERE tenant_id = $1::bytea AND campaign_id = $2::bytea \
             AND hosted_session_id = $3::bytea",
            &[&bytes(tenant), &bytes(campaign), &bytes(hosted_session)],
        )
        .await
        .map_err(statement_failure)?;
    Ok(row.map(|row| {
        let tenant: Vec<u8> = row.get("tenant_id");
        let campaign: Vec<u8> = row.get("campaign_id");
        let hosted_session: Vec<u8> = row.get("hosted_session_id");
        let gameplay_tenant: Vec<u8> = row.get("gameplay_tenant_id");
        let gameplay_session: Vec<u8> = row.get("gameplay_session_id");
        let run: Vec<u8> = row.get("run_id");
        let fence: Vec<u8> = row.get("owner_fence");
        let mut binding = HostRunBinding {
            tenant: [0; 16],
            campaign: [0; 16],
            hosted_session: [0; 16],
            gameplay_tenant: [0; 16],
            gameplay_session: [0; 16],
            run: [0; 16],
            fence: [0; 16],
        };
        binding.tenant.copy_from_slice(&tenant);
        binding.campaign.copy_from_slice(&campaign);
        binding.hosted_session.copy_from_slice(&hosted_session);
        binding.gameplay_tenant.copy_from_slice(&gameplay_tenant);
        binding.gameplay_session.copy_from_slice(&gameplay_session);
        binding.run.copy_from_slice(&run);
        binding.fence.copy_from_slice(&fence);
        binding
    }))
}

pub(super) async fn insert_host_run(
    transaction: &Transaction<'_>,
    binding: &HostRunBinding,
    now_ms: i64,
) -> Result<bool, IdentityError> {
    let inserted = transaction
        .execute(
            "INSERT INTO imagery.image_host_run_binding \
             (tenant_id, campaign_id, hosted_session_id, gameplay_session_id, \
             run_id, owner_fence, created_at_ms, gameplay_tenant_id) \
             VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, $5::bytea, \
             $6::bytea, $7, $8::bytea) ON CONFLICT DO NOTHING",
            &[
                &bytes(&binding.tenant),
                &bytes(&binding.campaign),
                &bytes(&binding.hosted_session),
                &bytes(&binding.gameplay_session),
                &bytes(&binding.run),
                &bytes(&binding.fence),
                &now_ms,
                &bytes(&binding.gameplay_tenant),
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(inserted == 1)
}

pub(super) async fn read_host_seat(
    transaction: &Transaction<'_>,
    binding: &HostSeatBinding,
) -> Result<Option<HostSeatBinding>, IdentityError> {
    read_host_seat_by_key(
        transaction,
        &binding.tenant,
        &binding.campaign,
        &binding.hosted_session,
        &binding.account,
        &binding.member,
    )
    .await
}

pub(super) async fn read_host_seat_by_member(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
    member: &[u8; 16],
) -> Result<Option<HostSeatBinding>, IdentityError> {
    let row = transaction
        .query_opt(
            "SELECT tenant_id, campaign_id, hosted_session_id, account_id, \
             member_id, gameplay_member_id, run_id, gameplay_session_id, \
             generation, active \
             FROM imagery.image_host_seat_binding \
             WHERE tenant_id = $1::bytea AND campaign_id = $2::bytea \
             AND hosted_session_id = $3::bytea AND member_id = $4::bytea",
            &[
                &bytes(tenant),
                &bytes(campaign),
                &bytes(hosted_session),
                &bytes(member),
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(row.map(decode_host_seat))
}

pub(super) async fn read_host_seat_by_key(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
    account: &[u8; 16],
    member: &[u8; 16],
) -> Result<Option<HostSeatBinding>, IdentityError> {
    let row = transaction
        .query_opt(
            "SELECT tenant_id, campaign_id, hosted_session_id, account_id, \
             member_id, gameplay_member_id, run_id, gameplay_session_id, \
             generation, active \
             FROM imagery.image_host_seat_binding \
             WHERE tenant_id = $1::bytea AND campaign_id = $2::bytea \
             AND hosted_session_id = $3::bytea AND account_id = $4::bytea \
             AND member_id = $5::bytea FOR SHARE",
            &[
                &bytes(tenant),
                &bytes(campaign),
                &bytes(hosted_session),
                &bytes(account),
                &bytes(member),
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(row.map(decode_host_seat))
}

fn decode_host_seat(row: tokio_postgres::Row) -> HostSeatBinding {
    let tenant: Vec<u8> = row.get("tenant_id");
    let campaign: Vec<u8> = row.get("campaign_id");
    let hosted_session: Vec<u8> = row.get("hosted_session_id");
    let account: Vec<u8> = row.get("account_id");
    let member: Vec<u8> = row.get("member_id");
    let gameplay_member: Vec<u8> = row.get("gameplay_member_id");
    let run: Vec<u8> = row.get("run_id");
    let gameplay_session: Vec<u8> = row.get("gameplay_session_id");
    let generation: Vec<u8> = row.get("generation");
    let mut binding = HostSeatBinding {
        tenant: [0; 16],
        campaign: [0; 16],
        hosted_session: [0; 16],
        account: [0; 16],
        member: [0; 16],
        gameplay_member: [0; 16],
        run: [0; 16],
        gameplay_session: [0; 16],
        generation: [0; 16],
        active: row.get("active"),
    };
    binding.tenant.copy_from_slice(&tenant);
    binding.campaign.copy_from_slice(&campaign);
    binding.hosted_session.copy_from_slice(&hosted_session);
    binding.account.copy_from_slice(&account);
    binding.member.copy_from_slice(&member);
    binding.gameplay_member.copy_from_slice(&gameplay_member);
    binding.run.copy_from_slice(&run);
    binding.gameplay_session.copy_from_slice(&gameplay_session);
    binding.generation.copy_from_slice(&generation);
    binding
}

pub(super) async fn insert_host_seat(
    transaction: &Transaction<'_>,
    binding: &HostSeatBinding,
    now_ms: i64,
) -> Result<bool, IdentityError> {
    let inserted = transaction
        .execute(
            "INSERT INTO imagery.image_host_seat_binding \
             (tenant_id, campaign_id, hosted_session_id, account_id, member_id, \
             gameplay_member_id, run_id, gameplay_session_id, generation, \
             active, created_at_ms, updated_at_ms) \
             VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, $5::bytea, \
             $6::bytea, $7::bytea, $8::bytea, $9::bytea, $10, $11, $11) ON CONFLICT DO NOTHING",
            &[
                &bytes(&binding.tenant),
                &bytes(&binding.campaign),
                &bytes(&binding.hosted_session),
                &bytes(&binding.account),
                &bytes(&binding.member),
                &bytes(&binding.gameplay_member),
                &bytes(&binding.run),
                &bytes(&binding.gameplay_session),
                &bytes(&binding.generation),
                &binding.active,
                &now_ms,
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(inserted == 1)
}

pub(super) async fn rotate_host_seat(
    transaction: &Transaction<'_>,
    key: &HostSeatKey,
    old_generation: &[u8; 16],
    new_generation: &[u8; 16],
    now_ms: i64,
) -> Result<(), IdentityError> {
    let HostSeatKey {
        tenant,
        campaign,
        hosted_session,
        account,
        member,
    } = key;
    let seat=transaction.query_opt(
        "SELECT generation,active FROM imagery.image_host_seat_binding WHERE tenant_id=$1::bytea \
         AND campaign_id=$2::bytea AND hosted_session_id=$3::bytea \
         AND account_id=$4::bytea AND member_id=$5::bytea FOR UPDATE",
        &[&bytes(tenant),&bytes(campaign),&bytes(hosted_session),&bytes(account),&bytes(member)],
    ).await.map_err(statement_failure)?.ok_or(IdentityError::Conflict)?;
    if !seat.get::<_, bool>("active") {
        return Err(IdentityError::Conflict);
    }
    let prior = transaction
        .query_opt(
            "SELECT new_generation FROM imagery.image_host_seat_rotation WHERE tenant_id=$1::bytea \
         AND campaign_id=$2::bytea AND hosted_session_id=$3::bytea \
         AND account_id=$4::bytea AND member_id=$5::bytea AND old_generation=$6::bytea",
            &[
                &bytes(tenant),
                &bytes(campaign),
                &bytes(hosted_session),
                &bytes(account),
                &bytes(member),
                &bytes(old_generation),
            ],
        )
        .await
        .map_err(statement_failure)?;
    if let Some(prior) = prior {
        return if prior.get::<_, Vec<u8>>(0).as_slice() == new_generation.as_slice() {
            Ok(())
        } else {
            Err(IdentityError::Conflict)
        };
    }
    if seat.get::<_, Vec<u8>>("generation").as_slice() != old_generation.as_slice() {
        return Err(IdentityError::Conflict);
    }
    transaction.execute(
        "INSERT INTO imagery.image_host_seat_rotation VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea)",
        &[&bytes(tenant),&bytes(campaign),&bytes(hosted_session),&bytes(account),&bytes(member),&bytes(old_generation),&bytes(new_generation)],
    ).await.map_err(statement_failure)?;
    transaction
        .execute(
            "UPDATE imagery.image_host_seat_binding SET generation=$6::bytea,updated_at_ms=$7 \
         WHERE tenant_id=$1::bytea AND campaign_id=$2::bytea AND hosted_session_id=$3::bytea \
         AND account_id=$4::bytea AND member_id=$5::bytea",
            &[
                &bytes(tenant),
                &bytes(campaign),
                &bytes(hosted_session),
                &bytes(account),
                &bytes(member),
                &bytes(new_generation),
                &now_ms,
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(())
}

pub(super) async fn revoke_host_seat(
    transaction: &Transaction<'_>,
    key: &HostSeatKey,
    now_ms: i64,
) -> Result<(), IdentityError> {
    let HostSeatKey {
        tenant,
        campaign,
        hosted_session,
        account,
        member,
    } = key;
    let changed = transaction
        .execute(
            "UPDATE imagery.image_host_seat_binding SET active=false,updated_at_ms=$6 \
         WHERE tenant_id=$1::bytea AND campaign_id=$2::bytea AND hosted_session_id=$3::bytea \
         AND account_id=$4::bytea AND member_id=$5::bytea",
            &[
                &bytes(tenant),
                &bytes(campaign),
                &bytes(hosted_session),
                &bytes(account),
                &bytes(member),
                &now_ms,
            ],
        )
        .await
        .map_err(statement_failure)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(IdentityError::Conflict)
    }
}

pub(super) async fn count_active_host_seats(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
) -> Result<u64, IdentityError> {
    let row = transaction
        .query_one(
            "SELECT COUNT(*)::bigint AS seats \
             FROM imagery.image_host_seat_binding \
             WHERE tenant_id = $1::bytea AND campaign_id = $2::bytea \
             AND hosted_session_id = $3::bytea AND active",
            &[&bytes(tenant), &bytes(campaign), &bytes(hosted_session)],
        )
        .await
        .map_err(statement_failure)?;
    let seats: i64 = row.get("seats");
    u64::try_from(seats).map_err(|_| IdentityError::Unavailable {
        sqlstate: None,
        detail: "imagery identity seat count out of range".to_owned(),
    })
}

pub(super) async fn read_local_guest(
    transaction: &Transaction<'_>,
    member: &[u8; 16],
    session: &[u8; 16],
) -> Result<Option<LocalGuestBinding>, IdentityError> {
    let row = transaction
        .query_opt(
            "SELECT guest_account_id, operation_id, fingerprint, generation \
             FROM imagery.image_local_guest WHERE member_id=$1::bytea AND session_id=$2::bytea",
            &[&bytes(member), &bytes(session)],
        )
        .await
        .map_err(statement_failure)?;
    Ok(row.map(|row| {
        let account: Vec<u8> = row.get("guest_account_id");
        let operation: Vec<u8> = row.get("operation_id");
        let fingerprint: Vec<u8> = row.get("fingerprint");
        let generation: Vec<u8> = row.get("generation");
        let mut binding = LocalGuestBinding {
            member: *member,
            session: *session,
            account: [0; 16],
            operation: [0; 16],
            fingerprint: [0; 32],
            generation: [0; 16],
        };
        binding.account.copy_from_slice(&account);
        binding.operation.copy_from_slice(&operation);
        binding.fingerprint.copy_from_slice(&fingerprint);
        binding.generation.copy_from_slice(&generation);
        binding
    }))
}

pub(super) async fn insert_local_guest(
    transaction: &Transaction<'_>,
    binding: &LocalGuestBinding,
    now_ms: i64,
) -> Result<bool, IdentityError> {
    let inserted = transaction
        .execute(
            "INSERT INTO imagery.image_local_guest \
             (member_id, session_id, guest_account_id, operation_id, fingerprint, \
             generation, created_at_ms, updated_at_ms) \
             VALUES ($6::bytea, $7::bytea, $1::bytea, $2::bytea, $3::bytea, $4::bytea, $5, $5) ON CONFLICT DO NOTHING",
            &[
                &bytes(&binding.account),
                &bytes(&binding.operation),
                &bytes(&binding.fingerprint),
                &bytes(&binding.generation),
                &now_ms,
                &bytes(&binding.member),
                &bytes(&binding.session),
            ],
        )
        .await
        .map_err(statement_failure)?;
    Ok(inserted == 1)
}
