//! Durable imagery ownership bridge (task IMG-HOST-IDENTITY-20261028-A1).
//!
//! What it owns: the atomic run binding from one trusted hosted allocation to
//! its gameplay session, run and owner fence; the seat binding from one
//! canonical account/member to its gameplay member, run/session and active
//! generation; and credential-scoped local guest bootstraps. What it never does:
//! no credential storage or authentication, no customer invention, no finance
//! movement, no provider or network call, and no cross-context mixing: hosted
//! rows name a real finance tuple, local rows name only the guest domain.
//!
//! The trusted host startup or runtime allocation supplies the validated
//! command; request parameters never bind accounts. Exact replay returns the
//! stored row; a different tuple under the same key is a conflict; a commit
//! whose outcome is unknown rereads the stored row instead of rewriting it.
//! Rotation replaces a seat generation and reclaim clears its activity while
//! the account stays put. Raw credentials stay with the existing scope
//! issuer; bindings carry stable member and generation proof only.

mod authority_evidence;
mod rows;
pub use authority_evidence::{
    AuthorityEvidence, HostedAuthorityCheck, lock_hosted_authority_evidence,
    lock_local_guest_authority_evidence, lock_local_run_authority_evidence,
};
/// Closure-only authority evidence installation; ordinary delivery resolution remains unchanged.
pub const IMAGERY_AUTHORITY_EVIDENCE_MIGRATION_VERSION: i32 = 52;
pub const IMAGERY_AUTHORITY_EVIDENCE_MIGRATION_SQL: &str =
    include_str!("../migrations/0052_imagery_authority_evidence.sql");

use df_commerce::snapshot::ReceiptRow;
use df_finance::ImageStageBinding;
use df_imagery::ImageOwner;
use tokio_postgres::Transaction;

use super::imagery_admission::{RunSessionAuthority, RunSessionError};

/// Migration 0041 requires canonical accounts, campaigns, O1 sessions/roles and imagery 0039.
pub const IMAGERY_IDENTITY_MIGRATION_VERSION: i32 = 41;

/// The exact SQL of migration 0041 for the composition installer.
pub const IMAGERY_IDENTITY_MIGRATION_SQL: &str =
    include_str!("../migrations/0041_imagery_identity.sql");

/// The one closed authority context a binding or principal lives in. Hosted
/// rows prove a customer finance tuple; the local guest proves only local
/// development ownership. A typed [`ImageOwner`] alone never selects one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityMode {
    Hosted,
    LocalGuest,
    LocalRun,
}

/// The durable kind of one stored binding. Hosted run and seat rows never
/// share a key with the local guest row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingKind {
    HostRun,
    HostSeat,
    LocalGuest,
}

/// Why an identity write or read failed. Carries no owner bytes: a scope
/// conflict must never leak another owner id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityError {
    /// The key already names a different tuple; reread it instead of writing.
    Conflict,
    /// The tuple names no durable binding (unknown run, revoked seat, wrong
    /// generation, or a local identity offered to hosted finance).
    Refused,
    /// The mapping store was unreachable; nothing was written.
    Unavailable {
        sqlstate: Option<String>,
        detail: String,
    },
    /// The commit outcome is unknown; reread the stored row to reconcile.
    Unknown { detail: String },
}

/// Whether a bind stored a new row or replayed the identical stored one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindOutcome {
    Accepted,
    Existing,
}

/// One durable hosted run binding: the trusted allocation's finance tuple to
/// its gameplay session, run and owner fence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRunBinding {
    pub tenant: [u8; 16],
    pub campaign: [u8; 16],
    pub hosted_session: [u8; 16],
    pub gameplay_tenant: [u8; 16],
    pub gameplay_session: [u8; 16],
    pub run: [u8; 16],
    pub fence: [u8; 16],
}

/// One durable hosted seat binding: the canonical account/member to its
/// gameplay member, run/session and active generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostSeatBinding {
    pub tenant: [u8; 16],
    pub campaign: [u8; 16],
    pub hosted_session: [u8; 16],
    pub account: [u8; 16],
    pub member: [u8; 16],
    pub gameplay_member: [u8; 16],
    pub run: [u8; 16],
    pub gameplay_session: [u8; 16],
    pub generation: [u8; 16],
    pub active: bool,
}

/// The one retained local-guest bootstrap: a persisted local identity with
/// its exact scoped operation and fingerprint replay. Names no customer,
/// campaign, run or finance tuple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalGuestBinding {
    pub member: [u8; 16],
    pub session: [u8; 16],
    pub account: [u8; 16],
    pub operation: [u8; 16],
    pub fingerprint: [u8; 32],
    pub generation: [u8; 16],
}

fn nonzero(_field: &str, bytes: &[u8; 16]) -> Result<(), IdentityError> {
    if bytes == &[0; 16] {
        return Err(IdentityError::Conflict);
    }
    Ok(())
}

fn valid_host_run(binding: &HostRunBinding) -> Result<(), IdentityError> {
    nonzero("tenant", &binding.tenant)?;
    nonzero("campaign", &binding.campaign)?;
    nonzero("hosted_session", &binding.hosted_session)?;
    nonzero("gameplay_session", &binding.gameplay_session)?;
    nonzero("run", &binding.run)?;
    nonzero("fence", &binding.fence)?;
    Ok(())
}

fn valid_host_seat(binding: &HostSeatBinding) -> Result<(), IdentityError> {
    nonzero("tenant", &binding.tenant)?;
    nonzero("campaign", &binding.campaign)?;
    nonzero("hosted_session", &binding.hosted_session)?;
    nonzero("account", &binding.account)?;
    nonzero("member", &binding.member)?;
    nonzero("gameplay_member", &binding.gameplay_member)?;
    nonzero("run", &binding.run)?;
    nonzero("gameplay_session", &binding.gameplay_session)?;
    nonzero("generation", &binding.generation)?;
    Ok(())
}

/// Atomically binds one hosted run, replaying the identical stored tuple.
/// A different tuple under the same key is a conflict; the caller rereads it.
pub async fn bind_host_run(
    transaction: &Transaction<'_>,
    binding: &HostRunBinding,
    now_ms: i64,
) -> Result<BindOutcome, IdentityError> {
    valid_host_run(binding)?;
    verify_run(transaction, binding).await?;
    if now_ms <= 0 {
        return Err(IdentityError::Conflict);
    }
    if let Some(stored) = rows::read_host_run(transaction, binding).await? {
        return if stored == *binding {
            Ok(BindOutcome::Existing)
        } else {
            Err(IdentityError::Conflict)
        };
    }
    match rows::insert_host_run(transaction, binding, now_ms).await {
        Ok(true) => {
            transaction.execute(
                "INSERT INTO imagery.image_host_fence_history VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea)",
                &[&binding.tenant.as_slice(), &binding.campaign.as_slice(),
                  &binding.hosted_session.as_slice(), &binding.fence.as_slice()],
            ).await.map_err(rows::unavailable_of)?;
            Ok(BindOutcome::Accepted)
        }
        Ok(false) => {
            // A lost race: reread the winner instead of deciding from the error.
            match rows::read_host_run(transaction, binding).await? {
                Some(stored) if stored == *binding => Ok(BindOutcome::Existing),
                _ => Err(IdentityError::Conflict),
            }
        }
        Err(error) => Err(error),
    }
}

/// Rereads one hosted run binding for exact replay or unknown-commit
/// reconciliation. Never writes.
pub async fn read_host_run(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
) -> Result<Option<HostRunBinding>, IdentityError> {
    rows::read_host_run_by_key(transaction, tenant, campaign, hosted_session).await
}

/// Atomically binds one hosted seat, replaying the identical stored tuple.
/// Rotation and reclaim go through their own calls; a silent overwrite of a
/// live generation is a conflict.
pub async fn bind_host_seat(
    transaction: &Transaction<'_>,
    binding: &HostSeatBinding,
    now_ms: i64,
) -> Result<BindOutcome, IdentityError> {
    valid_host_seat(binding)?;
    verify_seat(transaction, binding).await?;
    if now_ms <= 0 {
        return Err(IdentityError::Conflict);
    }
    if let Some(stored) = rows::read_host_seat(transaction, binding).await? {
        return if stored == *binding {
            Ok(BindOutcome::Existing)
        } else {
            Err(IdentityError::Conflict)
        };
    }
    match rows::insert_host_seat(transaction, binding, now_ms).await {
        Ok(true) => Ok(BindOutcome::Accepted),
        Ok(false) => match rows::read_host_seat(transaction, binding).await? {
            Some(stored) if stored == *binding => Ok(BindOutcome::Existing),
            _ => Err(IdentityError::Conflict),
        },
        Err(error) => Err(error),
    }
}

/// Rereads one hosted seat binding by its canonical member for live-grant
/// resolution. The stored account is the answer, never a grant parameter.
pub async fn read_host_seat_by_member(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
    member: &[u8; 16],
) -> Result<Option<HostSeatBinding>, IdentityError> {
    rows::read_host_seat_by_member(transaction, tenant, campaign, hosted_session, member).await
}

/// Rereads one hosted seat binding for exact replay or rotation checks.
pub async fn read_host_seat(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
    account: &[u8; 16],
    member: &[u8; 16],
) -> Result<Option<HostSeatBinding>, IdentityError> {
    rows::read_host_seat_by_key(
        transaction,
        tenant,
        campaign,
        hosted_session,
        account,
        member,
    )
    .await
}

/// Stable canonical key for a hosted credential mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostSeatKey {
    pub tenant: [u8; 16],
    pub campaign: [u8; 16],
    pub hosted_session: [u8; 16],
    pub account: [u8; 16],
    pub member: [u8; 16],
}

/// Rotates one seat to a new generation after proving the old one. The
/// account, member and run mapping stay put; the old generation stops working.
pub async fn rotate_host_seat(
    transaction: &Transaction<'_>,
    key: &HostSeatKey,
    old_generation: &[u8; 16],
    new_generation: &[u8; 16],
    now_ms: i64,
) -> Result<(), IdentityError> {
    nonzero("generation", new_generation)?;
    if now_ms <= 0 || old_generation == new_generation {
        return Err(IdentityError::Conflict);
    }
    rows::rotate_host_seat(transaction, key, old_generation, new_generation, now_ms).await
}

/// Reclaims a seat idempotently. Ordinary rotation cannot reactivate a revoked seat.
pub async fn revoke_host_seat(
    transaction: &Transaction<'_>,
    key: &HostSeatKey,
    now_ms: i64,
) -> Result<(), IdentityError> {
    if now_ms <= 0 {
        return Err(IdentityError::Conflict);
    }
    rows::revoke_host_seat(transaction, key, now_ms).await
}

/// Counts the active seats of one hosted session for the current-membership
/// check. Zero means the membership was revoked.
pub async fn active_host_seats(
    transaction: &Transaction<'_>,
    tenant: &[u8; 16],
    campaign: &[u8; 16],
    hosted_session: &[u8; 16],
) -> Result<u64, IdentityError> {
    rows::count_active_host_seats(transaction, tenant, campaign, hosted_session).await
}

/// Authenticated local bootstrap intent; raw credentials are never retained by the bridge.
pub struct LocalGuestBootstrap {
    pub member: [u8; 16],
    pub session: [u8; 16],
    pub credential: [u8; 32],
    pub candidate_account: [u8; 16],
    pub operation: [u8; 16],
    pub fingerprint: [u8; 32],
    pub generation: [u8; 16],
}

/// Retains one independently random account for an authenticated local session member.
pub async fn bootstrap_local_guest(
    transaction: &Transaction<'_>,
    command: &LocalGuestBootstrap,
    now_ms: i64,
) -> Result<(LocalGuestBinding, BindOutcome), IdentityError> {
    let LocalGuestBootstrap {
        member,
        session,
        credential,
        candidate_account,
        operation,
        fingerprint,
        generation,
    } = command;
    verify_local_credential(transaction, member, session, credential).await?;
    nonzero("account", candidate_account)?;
    nonzero("operation", operation)?;
    nonzero("generation", generation)?;
    if now_ms <= 0 {
        return Err(IdentityError::Conflict);
    }
    if let Some(stored) = rows::read_local_guest(transaction, member, session).await? {
        return if stored.operation == *operation && stored.fingerprint == *fingerprint {
            Ok((stored, BindOutcome::Existing))
        } else {
            Err(IdentityError::Conflict)
        };
    }
    let candidate = LocalGuestBinding {
        member: *member,
        session: *session,
        account: *candidate_account,
        operation: *operation,
        fingerprint: *fingerprint,
        generation: *generation,
    };
    match rows::insert_local_guest(transaction, &candidate, now_ms).await {
        Ok(true) => Ok((candidate, BindOutcome::Accepted)),
        Ok(false) => match rows::read_local_guest(transaction, member, session).await? {
            Some(stored)
                if stored.operation == *operation && stored.fingerprint == *fingerprint =>
            {
                Ok((stored, BindOutcome::Existing))
            }
            _ => Err(IdentityError::Conflict),
        },
        Err(error) => Err(error),
    }
}

/// Rereads the retained local guest for restart recovery. Never writes.
pub async fn read_local_guest(
    transaction: &Transaction<'_>,
    member: &[u8; 16],
    session: &[u8; 16],
) -> Result<Option<LocalGuestBinding>, IdentityError> {
    rows::read_local_guest(transaction, member, session).await
}

/// The production [`RunSessionAuthority`]: verifies the finance binding
/// against the stored durable run and seat bindings, the current owner fence
/// and canonical session lease. Finance MemberPort retains admission-member authority.
/// A draft owner, an unknown run, a forged
/// finance tuple, a moved fence or a revoked membership is refused. The
/// local guest context never enters hosted finance.
pub struct PgRunSessionAuthority;

impl RunSessionAuthority for PgRunSessionAuthority {
    async fn verify_canonical_run(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        binding: &ImageStageBinding,
        receipt: &ReceiptRow,
    ) -> Result<(), RunSessionError> {
        verify_hosted(transaction, owner, binding, receipt).await
    }
}

async fn verify_hosted(
    transaction: &Transaction<'_>,
    owner: &ImageOwner,
    binding: &ImageStageBinding,
    receipt: &ReceiptRow,
) -> Result<(), RunSessionError> {
    let ImageOwner::Run { campaign, run } = owner else {
        // Draft owners never authorize hosted finance, including the local
        // guest domain: it grants local development ownership only.
        return Err(RunSessionError::Refused);
    };
    if receipt.attempt.as_bytes() != binding.attempt.as_bytes() {
        return Err(RunSessionError::Refused);
    }
    let stored = rows::read_host_run_by_key(
        transaction,
        binding.tenant.as_bytes(),
        binding.campaign.as_bytes(),
        binding.session.as_bytes(),
    )
    .await
    .map_err(|error| match error {
        IdentityError::Conflict | IdentityError::Refused => RunSessionError::Refused,
        IdentityError::Unknown { detail } => RunSessionError::Unavailable {
            sqlstate: None,
            detail,
        },
        IdentityError::Unavailable { sqlstate, detail } => {
            RunSessionError::Unavailable { sqlstate, detail }
        }
    })?
    .ok_or(RunSessionError::Refused)?;
    if stored.campaign != *campaign.as_bytes()
        || stored.run != *run.as_bytes()
        || stored.hosted_session != *binding.session.as_bytes()
        || stored.campaign != *binding.campaign.as_bytes()
        || stored.tenant != *binding.tenant.as_bytes()
    {
        return Err(RunSessionError::Refused);
    }
    verify_run(transaction, &stored)
        .await
        .map_err(|error| match error {
            IdentityError::Refused | IdentityError::Conflict => RunSessionError::Refused,
            IdentityError::Unavailable { sqlstate, detail } => {
                RunSessionError::Unavailable { sqlstate, detail }
            }
            IdentityError::Unknown { detail } => RunSessionError::Unavailable {
                sqlstate: None,
                detail,
            },
        })
}

/// Locks canonical campaign/session authority and the live gameplay fence through commit.
async fn verify_run(
    transaction: &Transaction<'_>,
    run: &HostRunBinding,
) -> Result<(), IdentityError> {
    let row = transaction.query_one(
        "SELECT imagery.current_run($1::bytea,$2::bytea,$3::bytea,$4::bytea,$5::bytea,$6::bytea,$7::bytea)",
        &[&run.tenant.as_slice(), &run.campaign.as_slice(), &run.hosted_session.as_slice(),
          &run.gameplay_tenant.as_slice(), &run.gameplay_session.as_slice(), &run.run.as_slice(),
          &run.fence.as_slice()],
    ).await.map_err(rows::unavailable_of)?;
    if row.get::<_, bool>(0) {
        Ok(())
    } else {
        Err(IdentityError::Refused)
    }
}

async fn verify_seat(
    transaction: &Transaction<'_>,
    seat: &HostSeatBinding,
) -> Result<bool, IdentityError> {
    let account = transaction.query_opt(
        "SELECT 1 FROM accounts.account WHERE account_id=$1::bytea AND status='active' FOR SHARE",
        &[&seat.account.as_slice()],
    ).await.map_err(rows::unavailable_of)?;
    if account.is_none() {
        return Err(IdentityError::Refused);
    }
    let run = read_host_run(
        transaction,
        &seat.tenant,
        &seat.campaign,
        &seat.hosted_session,
    )
    .await?
    .ok_or(IdentityError::Refused)?;
    verify_run(transaction, &run).await?;
    if seat.run != run.run || seat.gameplay_session != run.gameplay_session || !seat.active {
        return Err(IdentityError::Refused);
    }
    let member = transaction
        .query_opt(
            "SELECT role FROM campaigns.campaign_member WHERE campaign_id=$1::bytea \
         AND member_id=$2::bytea AND account_id=$3::bytea AND status='active' \
         AND holds_seat FOR SHARE",
            &[
                &seat.campaign.as_slice(),
                &seat.member.as_slice(),
                &seat.account.as_slice(),
            ],
        )
        .await
        .map_err(rows::unavailable_of)?
        .ok_or(IdentityError::Refused)?;
    Ok(member.get::<_, String>(0) == "host")
}

/// Resolves host entitlement only after locking the current account session and membership.
pub async fn verify_host_principal(
    transaction: &Transaction<'_>,
    run: &HostRunBinding,
    seat: &HostSeatBinding,
    web_session: &[u8; 16],
    credential: &[u8; 32],
) -> Result<bool, IdentityError> {
    let session = transaction
        .query_opt(
            "SELECT 1 FROM accounts.account a JOIN accounts.web_session s USING(account_id) \
         WHERE a.account_id=$1::bytea AND a.status='active' AND s.session_id=$2::bytea \
         AND s.revoked_at_ms IS NULL AND s.credential_generation=a.credential_generation \
         AND s.absolute_expires_at_ms>extract(epoch from clock_timestamp())*1000 \
         FOR SHARE OF a,s",
            &[&seat.account.as_slice(), &web_session.as_slice()],
        )
        .await
        .map_err(rows::unavailable_of)?;
    if session.is_none() {
        return Err(IdentityError::Refused);
    }
    let host = verify_seat(transaction, seat).await?;
    verify_run(transaction, run).await?;
    verify_local_credential(
        transaction,
        &seat.gameplay_member,
        &seat.gameplay_session,
        credential,
    )
    .await?;
    let current = rows::read_host_seat_by_key(
        transaction,
        &seat.tenant,
        &seat.campaign,
        &seat.hosted_session,
        &seat.account,
        &seat.member,
    )
    .await?
    .ok_or(IdentityError::Refused)?;
    if current != *seat || !current.active {
        return Err(IdentityError::Refused);
    }
    Ok(host)
}

/// Checks the existing scope issuer's current credential, never a copied imagery generation.
pub async fn verify_local_credential(
    transaction: &Transaction<'_>,
    member: &[u8; 16],
    session: &[u8; 16],
    credential: &[u8; 32],
) -> Result<(), IdentityError> {
    let row = transaction
        .query_opt(
            "SELECT 1 FROM df_local_demo.grants WHERE credential=$1::bytea \
         AND principal_id=$2::bytea AND session_id=$3::bytea AND active \
         AND effective_role=$4::bytea AND expires_at>clock_timestamp() FOR SHARE",
            &[
                &credential.as_slice(),
                &member.as_slice(),
                &session.as_slice(),
                &b"player".as_slice(),
            ],
        )
        .await
        .map_err(rows::unavailable_of)?;
    row.map(|_| ()).ok_or(IdentityError::Refused)
}

/// Rebinds the same durable identity to a current owner fence with a monotonic retained epoch.
pub async fn rebind_host_run(
    transaction: &Transaction<'_>,
    binding: &HostRunBinding,
    expected_fence: &[u8; 16],
) -> Result<BindOutcome, IdentityError> {
    verify_run(transaction, binding).await?;
    let stored = rows::read_host_run(transaction, binding)
        .await?
        .ok_or(IdentityError::Refused)?;
    if stored == *binding {
        return Ok(BindOutcome::Existing);
    }
    let mut identity = binding.clone();
    identity.fence = stored.fence;
    if identity != stored || stored.fence != *expected_fence {
        return Err(IdentityError::Conflict);
    }
    let inserted = transaction.execute(
        "INSERT INTO imagery.image_host_fence_history VALUES ($1::bytea,$2::bytea,$3::bytea,$4::bytea) \
         ON CONFLICT DO NOTHING",
        &[&binding.tenant.as_slice(), &binding.campaign.as_slice(), &binding.hosted_session.as_slice(),
          &binding.fence.as_slice()],
    ).await.map_err(rows::unavailable_of)?;
    if inserted != 1 {
        return match rows::read_host_run(transaction, binding).await? {
            Some(current) if current == *binding => Ok(BindOutcome::Existing),
            _ => Err(IdentityError::Conflict),
        };
    }
    let changed = transaction.execute(
        "UPDATE imagery.image_host_run_binding SET owner_fence=$4::bytea,owner_epoch=owner_epoch+1 \
         WHERE tenant_id=$1::bytea AND campaign_id=$2::bytea AND hosted_session_id=$3::bytea \
         AND owner_fence=$5::bytea",
        &[&binding.tenant.as_slice(), &binding.campaign.as_slice(), &binding.hosted_session.as_slice(),
          &binding.fence.as_slice(), &expected_fence.as_slice()],
    ).await.map_err(rows::unavailable_of)?;
    if changed == 1 {
        Ok(BindOutcome::Accepted)
    } else {
        Err(IdentityError::Conflict)
    }
}

/// Trusted startup proof for the local server owner, never accepted from an image request.
#[derive(Clone)]
pub struct LocalRunGrant {
    pub session: [u8; 16],
    pub run: [u8; 16],
    pub fence: [u8; 16],
    pub bootstrap_credential: [u8; 32],
}

/// Current stored local run authority. It carries no customer account or hosted finance tuple.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalRunAuthority {
    pub tenant: [u8; 16],
    pub session: [u8; 16],
    pub campaign: [u8; 16],
    pub run: [u8; 16],
    pub fence: [u8; 16],
}

impl std::fmt::Debug for LocalRunAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalRunAuthority")
            .field("tenant", &self.tenant)
            .field("session", &self.session)
            .field("campaign", &self.campaign)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

fn local_identity(row: &tokio_postgres::Row, column: &str) -> Result<[u8; 16], IdentityError> {
    row.try_get::<_, Vec<u8>>(column)
        .map_err(rows::unavailable_of)?
        .try_into()
        .map_err(|_| IdentityError::Unavailable {
            sqlstate: None,
            detail: "invalid stored local run identity".to_owned(),
        })
}

/// Locks the actual local session lease and reserved server bootstrap grant through commit.
/// Host authority requires the captured current owner fence as well as the bootstrap credential.
pub async fn resolve_local_run(
    transaction: &Transaction<'_>,
    grant: &LocalRunGrant,
) -> Result<LocalRunAuthority, IdentityError> {
    use crate::local_demo_scope::{CAMPAIGN, PLAYER, TENANT};
    if grant.session == [0; 16]
        || grant.run == [0; 16]
        || grant.fence == [0; 16]
        || grant.bootstrap_credential == [0; 32]
    {
        return Err(IdentityError::Refused);
    }
    let session = transaction
        .query_opt(
            "SELECT tenant_id::bytea,session_id::bytea,run_id::bytea,owner_fence::bytea \
         FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea \
         AND run_id=$3::bytea AND owner_fence=$4::bytea AND lease_until>clock_timestamp() \
         FOR SHARE",
            &[
                &TENANT.as_slice(),
                &grant.session.as_slice(),
                &grant.run.as_slice(),
                &grant.fence.as_slice(),
            ],
        )
        .await
        .map_err(rows::unavailable_of)?
        .ok_or(IdentityError::Refused)?;
    let bootstrap = transaction
        .query_opt(
            "SELECT g.campaign_id FROM df_local_demo.grants g WHERE g.credential=$1::bytea \
         AND g.tenant_id=$2::bytea AND g.session_id=$3::bytea AND g.principal_id=$4::bytea \
         AND g.campaign_id=$5::bytea AND g.effective_role=$6::bytea AND g.active \
         AND g.expires_at>clock_timestamp() AND NOT EXISTS \
         (SELECT 1 FROM df_local_demo.room_grants j WHERE j.credential=g.credential) \
         FOR SHARE OF g",
            &[
                &grant.bootstrap_credential.as_slice(),
                &TENANT.as_slice(),
                &grant.session.as_slice(),
                &PLAYER.as_slice(),
                &CAMPAIGN.as_slice(),
                &b"player".as_slice(),
            ],
        )
        .await
        .map_err(rows::unavailable_of)?
        .ok_or(IdentityError::Refused)?;
    Ok(LocalRunAuthority {
        tenant: local_identity(&session, "tenant_id")?,
        session: local_identity(&session, "session_id")?,
        campaign: local_identity(&bootstrap, "campaign_id")?,
        run: local_identity(&session, "run_id")?,
        fence: local_identity(&session, "owner_fence")?,
    })
}

/// Hosted composition must call this before enabling Hosted accounting; local installation is insufficient.
pub async fn require_hosted_identity_schema(
    transaction: &Transaction<'_>,
) -> Result<(), IdentityError> {
    let row=transaction.query_one(
        "SELECT to_regclass('accounts.account') IS NOT NULL \
         AND to_regclass('accounts.web_session') IS NOT NULL \
         AND to_regclass('campaigns.campaign') IS NOT NULL \
         AND to_regclass('campaigns.campaign_member') IS NOT NULL \
         AND to_regclass('df_runtime.table_sessions') IS NOT NULL \
         AND to_regclass('df_game.sessions') IS NOT NULL \
         AND to_regrole('df_runtime_definer') IS NOT NULL \
         AND to_regrole('df_imagery_admission') IS NOT NULL \
         AND EXISTS (SELECT 1 FROM pg_proc p \
             WHERE p.oid=to_regprocedure('imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea)') \
             AND p.proowner=to_regrole('df_runtime_definer') AND p.prosecdef) \
         AND has_schema_privilege(to_regrole('df_imagery_admission'),to_regnamespace('imagery'),'USAGE') \
         AND has_function_privilege(to_regrole('df_imagery_admission'), \
             to_regprocedure('imagery.current_run(bytea,bytea,bytea,bytea,bytea,bytea,bytea)'),'EXECUTE') \
         AND EXISTS (SELECT 1 FROM pg_proc p \
             WHERE p.oid=to_regprocedure('imagery.lock_hosted_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)') \
             AND p.proowner=to_regrole('df_runtime_definer') AND p.prosecdef) \
         AND has_function_privilege(to_regrole('df_imagery_admission'), \
             to_regprocedure('imagery.lock_hosted_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)'),'EXECUTE') \
         AND has_table_privilege(to_regrole('df_imagery_admission'), \
             to_regclass('imagery.image_host_run_binding'),'SELECT') \
         AND has_table_privilege(to_regrole('df_imagery_admission'), \
             to_regclass('imagery.image_host_seat_binding'),'SELECT')",
        &[],
    ).await.map_err(rows::unavailable_of)?;
    if row.get::<_, bool>(0) {
        Ok(())
    } else {
        Err(IdentityError::Refused)
    }
}
