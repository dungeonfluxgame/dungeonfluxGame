//! Invitations, room codes and join requests (8.2, 8.3). A code always ends in approval; a link
//! admits directly. Both paths that create a member take the account share lock first, then the
//! campaign lock, then re-verify under the locks.

use df_campaign::{
    config::Tunables,
    errors::{Failure, Outcome, Refusal, StoreError},
    ids::{InvitationId, OPERATION_RETENTION_MS, RequestId, operation_issued_at_ms},
    join_invitation::{JoinContext, JoinFailure, JoinInvitation, MemberState, admit_approved},
    records::{
        CampaignStatus, CodeRow, InvitationRow, MemberRow, MemberStatus, RequestRow, RequestState,
        RoomCodes,
    },
    roles::Capability,
    secrets::random_identity,
    store::{Accepted, Decision, NewInvitation, PreviewFacts},
};
use df_types::{AccountId, CampaignId, MemberId, OperationId};

use super::{
    conn::Tx,
    queries as q,
    shared::{
        active_actor, admit, join_context, locked_campaign, member_state, refuse, require_active,
        require_capability,
    },
};

const ACCEPT_NAMESPACE: &str = "accept_invitation";

fn new_id<T>(make: impl FnOnce(&[u8]) -> Option<T>) -> Outcome<T> {
    let bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    make(&bytes).ok_or(Failure::Store(StoreError::InvalidRecord))
}

pub(super) async fn create_invitation(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    new: NewInvitation,
    now_ms: u64,
) -> Outcome<InvitationRow> {
    let NewInvitation {
        joins_as,
        lifetime_ms,
        max_uses,
        token_digest,
    } = new;
    if max_uses == 0 || lifetime_ms == 0 {
        return refuse(Refusal::InvalidInput);
    }
    let row = locked_campaign(tx, campaign).await?;
    let member = active_actor(tx, campaign, actor).await?;
    require_capability(&member, Capability::InviteOrRemovePlayers)?;
    require_active(&row)?;
    let invitation = InvitationRow {
        id: new_id(InvitationId::from_bytes)?,
        campaign,
        token_digest,
        created_by: member.member,
        joins_as,
        created_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(lifetime_ms),
        max_uses,
        use_count: 0,
        revoked_at_ms: None,
    };
    q::insert_invitation(tx, &invitation).await?;
    Ok(invitation)
}

pub(super) async fn revoke_invitation(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    invitation: InvitationId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let member = active_actor(tx, campaign, actor).await?;
    require_capability(&member, Capability::InviteOrRemovePlayers)?;
    require_active(&row)?;
    if q::invitation_by_id(tx, campaign, invitation)
        .await?
        .is_none()
    {
        return refuse(Refusal::NotFound);
    }
    q::revoke_invitation_row(tx, invitation, now_ms).await?;
    Ok(())
}

/// The four-field preview source for a usable invitation. Every failure carries only its reason.
pub(super) async fn preview_invitation(
    tx: &Tx<'_>,
    tunables: &Tunables,
    token_digest: [u8; 32],
    now_ms: u64,
) -> Outcome<PreviewFacts> {
    let Some(invitation) = q::invitation_by_digest(tx, token_digest).await? else {
        return refuse(Refusal::InvitationUnknown);
    };
    let Some(campaign) = q::read_campaign(tx, invitation.campaign).await? else {
        return refuse(Refusal::InvitationUnknown);
    };
    let context = join_context(
        tx,
        &campaign,
        None,
        df_campaign::records::AccountStatus::Active,
        tunables,
        now_ms,
    )
    .await?;
    consume(&invitation, &context)?;
    let Some(host_account) = q::host_account(tx, campaign.id).await? else {
        return refuse(Refusal::CampaignUnavailable);
    };
    Ok(PreviewFacts {
        campaign,
        host_account,
    })
}

fn consume(
    invitation: &InvitationRow,
    context: &JoinContext,
) -> Result<df_campaign::join_invitation::JoinUse, Failure> {
    JoinInvitation {
        expires_at_ms: invitation.expires_at_ms,
        max_uses: invitation.max_uses,
        use_count: invitation.use_count,
        revoked: invitation.revoked_at_ms.is_some(),
        joins_as: invitation.joins_as,
    }
    .consume(context)
    .map_err(|failure| Failure::Refused(Refusal::Join(failure)))
}

fn accepted_bytes(accepted: &Accepted) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(33);
    bytes.extend_from_slice(accepted.campaign.as_bytes());
    bytes.extend_from_slice(accepted.member.as_bytes());
    bytes.push(u8::from(accepted.rejoined));
    bytes
}

fn accepted_from(bytes: &[u8]) -> Outcome<Accepted> {
    if bytes.len() != 33 {
        return Err(Failure::Store(StoreError::InvalidRecord));
    }
    let campaign = CampaignId::from_bytes(&bytes[..16]).map_err(|_| StoreError::InvalidRecord)?;
    let member = MemberId::from_bytes(&bytes[16..32]).map_err(|_| StoreError::InvalidRecord)?;
    Ok(Accepted {
        campaign,
        member,
        rejoined: bytes[32] == 1,
    })
}

/// A repeated operation returns its first result before any invitation check; a changed payload is
/// a conflict; an id issued ninety days ago or more never executes, whether or not its receipt
/// was purged (the age is read from the id).
pub(super) async fn replay(
    tx: &Tx<'_>,
    namespace: &str,
    account: AccountId,
    operation: OperationId,
    fingerprint: [u8; 32],
    now_ms: u64,
) -> Outcome<Option<Vec<u8>>> {
    if now_ms.saturating_sub(operation_issued_at_ms(&operation)) >= OPERATION_RETENTION_MS {
        return refuse(Refusal::OperationExpired);
    }
    match q::operation_result(tx, namespace, account, operation.as_bytes()).await? {
        None => Ok(None),
        Some((stored, _)) if stored != fingerprint => refuse(Refusal::OperationConflict),
        Some((_, result)) => Ok(Some(result)),
    }
}

pub(super) async fn accept_invitation(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
    token_digest: [u8; 32],
    operation: OperationId,
    now_ms: u64,
) -> Outcome<Accepted> {
    if let Some(result) = replay(
        tx,
        ACCEPT_NAMESPACE,
        account,
        operation,
        token_digest,
        now_ms,
    )
    .await?
    {
        return accepted_from(&result);
    }
    // The campaign id comes from an unlocked read; everything is re-read under the locks.
    let Some(first) = q::invitation_by_digest(tx, token_digest).await? else {
        return refuse(Refusal::InvitationUnknown);
    };
    let status = q::lock_account(tx, account).await?;
    let campaign = locked_campaign(tx, first.campaign).await?;
    if let Some(result) = replay(
        tx,
        ACCEPT_NAMESPACE,
        account,
        operation,
        token_digest,
        now_ms,
    )
    .await?
    {
        return accepted_from(&result);
    }
    let Some(invitation) = q::invitation_by_digest(tx, token_digest).await? else {
        return refuse(Refusal::InvitationUnknown);
    };
    let existing = q::member_by_account(tx, campaign.id, account).await?;
    let context = join_context(tx, &campaign, existing.as_ref(), status, tunables, now_ms).await?;
    let admission = consume(&invitation, &context)?;
    let admitted = admit(
        tx,
        &campaign,
        account,
        existing,
        admission,
        Some(invitation.id),
        now_ms,
    )
    .await?;
    q::bump_invitation_use(tx, invitation.id).await?;
    let accepted = Accepted {
        campaign: campaign.id,
        member: admitted.member.member,
        rejoined: admitted.rejoined,
    };
    if !q::insert_operation(
        tx,
        ACCEPT_NAMESPACE,
        account,
        operation.as_bytes(),
        token_digest,
        &accepted_bytes(&accepted),
        now_ms,
    )
    .await?
    {
        return Err(Failure::Store(StoreError::Retryable));
    }
    Ok(accepted)
}

pub(super) async fn rotate_code(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    code_digest: [u8; 32],
    lifetime_ms: u64,
    now_ms: u64,
) -> Outcome<CodeRow> {
    let row = locked_campaign(tx, campaign).await?;
    let member = active_actor(tx, campaign, actor).await?;
    require_capability(&member, Capability::InviteOrRemovePlayers)?;
    require_active(&row)?;
    if row.room_codes == RoomCodes::Off {
        return refuse(Refusal::InvalidInput);
    }
    q::revoke_live_code(tx, campaign, now_ms).await?;
    let code = CodeRow {
        code_digest,
        campaign,
        created_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(lifetime_ms),
        revoked_at_ms: None,
    };
    q::insert_code(tx, &code).await?;
    Ok(code)
}

/// A live code of an active campaign that takes codes, or the one unknown-code answer.
async fn usable_code(
    tx: &Tx<'_>,
    code_digest: [u8; 32],
    now_ms: u64,
) -> Outcome<(CodeRow, df_campaign::records::CampaignRow)> {
    let Some(code) = q::code_by_digest(tx, code_digest).await? else {
        return refuse(Refusal::CodeUnknown);
    };
    if code.revoked_at_ms.is_some() || now_ms >= code.expires_at_ms {
        return refuse(Refusal::CodeUnknown);
    }
    let Some(campaign) = q::read_campaign(tx, code.campaign).await? else {
        return refuse(Refusal::CodeUnknown);
    };
    if campaign.status != CampaignStatus::Active || campaign.room_codes == RoomCodes::Off {
        return refuse(Refusal::CodeUnknown);
    }
    Ok((code, campaign))
}

pub(super) async fn lookup_code(
    tx: &Tx<'_>,
    account: AccountId,
    code_digest: [u8; 32],
    now_ms: u64,
) -> Outcome<PreviewFacts> {
    let (_, campaign) = usable_code(tx, code_digest, now_ms).await?;
    // A code never resolves to an existing member.
    if let Some(member) = q::member_by_account(tx, campaign.id, account).await?
        && member.status == MemberStatus::Active
    {
        return refuse(Refusal::Join(JoinFailure::AlreadyJoined));
    }
    let Some(host_account) = q::host_account(tx, campaign.id).await? else {
        return refuse(Refusal::CodeUnknown);
    };
    Ok(PreviewFacts {
        campaign,
        host_account,
    })
}

pub(super) async fn request_join(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
    code_digest: [u8; 32],
    now_ms: u64,
) -> Outcome<RequestRow> {
    let Some(code) = q::code_by_digest(tx, code_digest).await? else {
        return refuse(Refusal::CodeUnknown);
    };
    // The campaign lock serializes the pending count; the code is re-verified under it.
    locked_campaign(tx, code.campaign).await?;
    let (_, campaign) = usable_code(tx, code_digest, now_ms).await?;
    let existing = q::member_by_account(tx, campaign.id, account).await?;
    if existing
        .as_ref()
        .is_some_and(|member| member.status == MemberStatus::Active)
    {
        return refuse(Refusal::Join(JoinFailure::AlreadyJoined));
    }
    if let Some(pending) = q::pending_request_for(tx, campaign.id, account).await? {
        return Ok(pending);
    }
    if q::pending_request_count(tx, campaign.id).await? >= tunables.pending_join_request_cap {
        return refuse(Refusal::RequestsFull);
    }
    let request = RequestRow {
        id: new_id(RequestId::from_bytes)?,
        campaign: campaign.id,
        account,
        state: RequestState::Pending,
        approved_as: None,
        created_at_ms: now_ms,
        decided_by: None,
        decided_at_ms: None,
    };
    q::insert_request(tx, &request).await?;
    Ok(request)
}

pub(super) async fn decide_request(
    tx: &Tx<'_>,
    tunables: &Tunables,
    actor: AccountId,
    campaign: CampaignId,
    request: RequestId,
    decision: Decision,
    now_ms: u64,
) -> Outcome<Option<MemberRow>> {
    // The actor is checked before any lock is taken for someone else's account.
    let acting = active_actor(tx, campaign, actor).await?;
    require_capability(&acting, Capability::InviteOrRemovePlayers)?;
    let Some(first) = q::request_by_id(tx, campaign, request).await? else {
        return refuse(Refusal::NotFound);
    };
    let role = match decision {
        Decision::Approve(role) => Some(role),
        Decision::Deny => None,
    };
    // Approval creates a member from a stored account id: the account lock comes first.
    let status = match role {
        Some(_) => Some(q::lock_account(tx, first.account).await?),
        None => None,
    };
    let row = locked_campaign(tx, campaign).await?;
    let member = active_actor(tx, campaign, actor).await?;
    require_capability(&member, Capability::InviteOrRemovePlayers)?;
    let Some(pending) = q::request_by_id(tx, campaign, request).await? else {
        return refuse(Refusal::NotFound);
    };
    if pending.state != RequestState::Pending {
        return refuse(Refusal::StaleRevision);
    }
    let (Some(role), Some(status)) = (role, status) else {
        q::decide_request_row(
            tx,
            request,
            RequestState::Denied,
            None,
            member.member,
            now_ms,
        )
        .await?;
        return Ok(None);
    };
    require_active(&row)?;
    let existing = q::member_by_account(tx, campaign, pending.account).await?;
    if member_state(existing.as_ref()) == MemberState::RemovedNotAllowed {
        return refuse(Refusal::AllowBackFirst);
    }
    let context = join_context(tx, &row, existing.as_ref(), status, tunables, now_ms).await?;
    let admission = admit_approved(&context, role)
        .map_err(|failure| Failure::Refused(Refusal::Join(failure)))?;
    let admitted = admit(tx, &row, pending.account, existing, admission, None, now_ms).await?;
    q::decide_request_row(
        tx,
        request,
        RequestState::Approved,
        Some(role),
        member.member,
        now_ms,
    )
    .await?;
    Ok(Some(admitted.member))
}
