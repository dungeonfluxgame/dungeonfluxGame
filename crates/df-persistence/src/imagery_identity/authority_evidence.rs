//! Closure-only authority evidence; ordinary request and delivery resolution is unchanged.
use super::{IdentityError, LocalRunGrant, rows};
use tokio_postgres::{Row, Transaction};

/// Only Refused proves an existing denied row remains locked through the caller's transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityEvidence {
    Current,
    Refused,
    Unproven,
}

/// The host's retained proof; fields are checked against canonical rows, never treated as authority.
pub struct HostedAuthorityCheck {
    pub tenant: [u8; 16],
    pub campaign: [u8; 16],
    pub hosted_session: [u8; 16],
    pub member: [u8; 16],
    pub generation: [u8; 16],
    pub web_session: [u8; 16],
    pub gameplay_credential: [u8; 32],
}
fn fixed(row: &Row, name: &str) -> Result<[u8; 16], IdentityError> {
    row.try_get::<_, Vec<u8>>(name)
        .map_err(rows::unavailable_of)?
        .try_into()
        .map_err(|_| IdentityError::Unavailable {
            sqlstate: None,
            detail: "invalid retained authority evidence".to_owned(),
        })
}
async fn credential(
    transaction: &Transaction<'_>,
    value: &[u8; 32],
) -> Result<Option<Row>, IdentityError> {
    transaction.query_opt("SELECT tenant_id,principal_id,campaign_id,effective_role,session_id,active,expires_at>clock_timestamp() AS live FROM df_local_demo.grants WHERE credential=$1::bytea FOR SHARE",&[&value.as_slice()]).await.map_err(rows::unavailable_of)
}
fn credential_current(row: &Row, member: &[u8; 16], session: &[u8; 16]) -> bool {
    row.get::<_, Vec<u8>>("principal_id").as_slice() == member.as_slice()
        && row.get::<_, Vec<u8>>("session_id").as_slice() == session.as_slice()
        && row.get::<_, Vec<u8>>("effective_role").as_slice() == b"player"
        && row.get::<_, bool>("active")
        && row.get::<_, bool>("live")
}
async fn session(
    transaction: &Transaction<'_>,
    tenant: &[u8],
    session: &[u8; 16],
) -> Result<Option<Row>, IdentityError> {
    transaction.query_opt("SELECT run_id,owner_fence,lease_until>clock_timestamp() AS live FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea FOR SHARE",&[&tenant,&session.as_slice()]).await.map_err(rows::unavailable_of)
}

/// Reads the append-only guest mapping and locks the canonical session/credential before refusal.
pub async fn lock_local_guest_authority_evidence(
    transaction: &Transaction<'_>,
    member: &[u8; 16],
    session_id: &[u8; 16],
    value: &[u8; 32],
) -> Result<AuthorityEvidence, IdentityError> {
    if transaction.query_opt("SELECT 1 FROM imagery.image_local_guest WHERE member_id=$1::bytea AND session_id=$2::bytea",&[&member.as_slice(),&session_id.as_slice()]).await.map_err(rows::unavailable_of)?.is_none() {
        return Ok(AuthorityEvidence::Unproven);
    }
    let Some(hint) = transaction
        .query_opt(
            "SELECT tenant_id FROM df_local_demo.grants WHERE credential=$1::bytea",
            &[&value.as_slice()],
        )
        .await
        .map_err(rows::unavailable_of)?
    else {
        return Ok(AuthorityEvidence::Unproven);
    };
    let tenant: Vec<u8> = hint.get(0);
    if session(transaction, &tenant, session_id).await?.is_none() {
        return Ok(AuthorityEvidence::Unproven);
    }
    let Some(row) = credential(transaction, value).await? else {
        return Ok(AuthorityEvidence::Unproven);
    };
    if row.get::<_, Vec<u8>>("tenant_id") != tenant {
        return Ok(AuthorityEvidence::Unproven);
    }
    Ok(if credential_current(&row, member, session_id) {
        AuthorityEvidence::Current
    } else {
        AuthorityEvidence::Refused
    })
}

/// Locks the real reserved bootstrap grant and current local session; guests never gain host authority.
pub async fn lock_local_run_authority_evidence(
    transaction: &Transaction<'_>,
    grant: &LocalRunGrant,
) -> Result<AuthorityEvidence, IdentityError> {
    use crate::local_demo_scope::{CAMPAIGN, PLAYER, TENANT};
    let Some(current_session) = session(transaction, &TENANT, &grant.session).await? else {
        return Ok(AuthorityEvidence::Unproven);
    };
    let Some(row) = credential(transaction, &grant.bootstrap_credential).await? else {
        return Ok(AuthorityEvidence::Unproven);
    };
    let room = transaction
        .query_opt(
            "SELECT 1 FROM df_local_demo.room_grants WHERE credential=$1::bytea FOR SHARE",
            &[&grant.bootstrap_credential.as_slice()],
        )
        .await
        .map_err(rows::unavailable_of)?;
    let current = credential_current(&row, &PLAYER, &grant.session)
        && row.get::<_, Vec<u8>>("tenant_id").as_slice() == TENANT.as_slice()
        && row.get::<_, Vec<u8>>("campaign_id").as_slice() == CAMPAIGN.as_slice()
        && room.is_none()
        && fixed(&current_session, "run_id")? == grant.run
        && fixed(&current_session, "owner_fence")? == grant.fence
        && current_session.get::<_, bool>("live");
    Ok(if current {
        AuthorityEvidence::Current
    } else {
        AuthorityEvidence::Refused
    })
}

/// Locks every required existing hosted row; unknown durable rows never justify terminal denial.
pub async fn lock_hosted_authority_evidence(
    transaction: &Transaction<'_>,
    check: &HostedAuthorityCheck,
) -> Result<AuthorityEvidence, IdentityError> {
    let value: i16=transaction.query_one(
        "SELECT imagery.lock_hosted_authority_evidence($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea)",
        &[&check.tenant.as_slice(),&check.campaign.as_slice(),&check.hosted_session.as_slice(),
          &check.member.as_slice(),&check.generation.as_slice(),&check.web_session.as_slice(),
          &check.gameplay_credential.as_slice()],
    ).await.map_err(rows::unavailable_of)?.get(0);
    match value {
        0 => Ok(AuthorityEvidence::Unproven),
        1 => Ok(AuthorityEvidence::Current),
        2 => Ok(AuthorityEvidence::Refused),
        _ => Err(IdentityError::Unavailable {
            sqlstate: None,
            detail: "invalid canonical authority evidence".to_owned(),
        }),
    }
}
