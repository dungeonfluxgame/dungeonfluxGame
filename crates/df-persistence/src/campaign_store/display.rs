//! Display pairing and read-only tickets (8.5). A pairing is a device-authorization-style state
//! machine; a ticket is bound to the device secret's digest and can only view the public
//! projection. The cap on displays is enforced at redemption under the campaign lock.

use df_campaign::{
    config::Tunables,
    display_pairing::{
        PairingFacts, PairingRefusal, PairingState, RedeemAction, RenewalPolicy, TicketFacts,
        TicketRefusal, approve, extend, redeem, ticket_valid,
    },
    errors::{Failure, Outcome, Refusal, StoreError},
    ids::{PairingId, TicketId},
    records::{CampaignStatus, MemberStatus, PairingRow, RevokeReason, TicketRow},
    roles::Capability,
    secrets::random_identity,
    store::{NewPairing, OwnerFence, Redeemed, VerifiedTicket},
};
use df_types::{AccountId, CampaignId, MemberId, SessionId};

use super::{
    conn::Tx,
    queries as q,
    shared::{
        Standing, active_actor, locked_campaign, refuse, require_active, require_capability,
        revoke_tickets,
    },
};

fn pairing_refusal<T>(refusal: PairingRefusal) -> Outcome<T> {
    refuse(Refusal::Pairing(refusal))
}

/// A pending or approved pairing past its expiry reads as expired; retention writes the state.
fn visible(mut row: PairingRow, now_ms: u64) -> PairingRow {
    if matches!(row.state, PairingState::Pending | PairingState::Approved)
        && now_ms >= row.expires_at_ms
    {
        row.state = PairingState::Expired;
    }
    row
}

pub(super) async fn begin_pairing(
    tx: &Tx<'_>,
    tunables: &Tunables,
    new: NewPairing,
    lifetime_ms: u64,
    now_ms: u64,
) -> Outcome<PairingRow> {
    // The caps are checked with bounded counts, so a flood of challenges cannot make this scan grow.
    let tight_cap = tunables.display_pending_per_ip_maximum;
    let loose_cap = tight_cap.saturating_mul(tunables.throttle_loose_multiplier);
    let hard_cap = tunables.display_pending_hard_limit;
    let tight =
        q::pending_pairings_for_key(tx, q::AddressKey::Tight, new.ip_key, now_ms, tight_cap);
    let loose = q::pending_pairings_for_key(
        tx,
        q::AddressKey::Loose,
        new.ip_key_loose,
        now_ms,
        loose_cap,
    );
    if tight.await? >= tight_cap
        || loose.await? >= loose_cap
        || q::pending_pairings_total(tx, now_ms, hard_cap).await? >= hard_cap
    {
        return refuse(Refusal::Throttled);
    }
    if let Some(taken) = q::pending_pairing_by_code(tx, &new.user_code).await? {
        // A pending row past its expiry frees its code; a live one means a collision.
        if now_ms >= taken.expires_at_ms {
            let mut expired = taken;
            expired.state = PairingState::Expired;
            expired.user_code = None;
            q::update_pairing(tx, &expired).await?;
        } else {
            return Err(Failure::Store(StoreError::Retryable));
        }
    }
    let id_bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    let row = PairingRow {
        id: PairingId::from_bytes(&id_bytes).ok_or(StoreError::InvalidRecord)?,
        campaign: None,
        user_code: Some(new.user_code),
        device_digest: new.device_digest,
        device_label: new.device_label,
        ip_key: new.ip_key,
        ip_key_loose: new.ip_key_loose,
        created_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(lifetime_ms),
        state: PairingState::Pending,
        session: None,
        approved_by: None,
        approved_at_ms: None,
        redeemed_at_ms: None,
        reissue_count: 0,
        renewable_until_ms: None,
    };
    q::insert_pairing(tx, &row).await?;
    Ok(row)
}

pub(super) async fn pairing_status(
    tx: &Tx<'_>,
    device_digest: [u8; 32],
    now_ms: u64,
) -> Outcome<PairingRow> {
    match q::pairing_by_device(tx, device_digest).await? {
        Some(row) => Ok(visible(row, now_ms)),
        None => refuse(Refusal::DisplayNeutral),
    }
}

/// Lookup shows what is waiting; it never approves.
pub(super) async fn lookup_pairing(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    user_code: &str,
    now_ms: u64,
) -> Outcome<PairingRow> {
    let acting = active_actor(tx, campaign, actor).await?;
    require_capability(&acting, Capability::StartOrPauseSession)?;
    match q::read_campaign(tx, campaign).await? {
        Some(row) => require_active(&row)?,
        None => return refuse(Refusal::NotFound),
    }
    match q::pending_pairing_by_code(tx, user_code).await? {
        Some(row) if now_ms < row.expires_at_ms => Ok(row),
        _ => refuse(Refusal::DisplayNeutral),
    }
}

pub(super) async fn approve_pairing(
    tx: &Tx<'_>,
    tunables: &Tunables,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
    pairing: PairingId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    require_capability(&acting, Capability::StartOrPauseSession)?;
    require_active(&row)?;
    // The pairing row is locked for the rest of the transaction, so two campaigns approving one
    // code at once serialize here, and the update below only moves a pending pairing.
    let Some(mut found) = q::pairing_by_id_for_update(tx, pairing).await? else {
        return refuse(Refusal::DisplayNeutral);
    };
    let facts = PairingFacts {
        state: found.state,
        expires_at_ms: found.expires_at_ms,
        device_digest: found.device_digest,
        reissue_count: found.reissue_count,
        redeemed_at_ms: found.redeemed_at_ms,
    };
    if let Err(refusal) = approve(&facts, true, now_ms) {
        return pairing_refusal(refusal);
    }
    // Approved-but-unredeemed pairings count toward the cap, pending ones do not.
    let live = q::live_tickets_of_session(tx, campaign, session, now_ms)
        .await?
        .len();
    let waiting = q::approved_pairings_of_session(tx, campaign, session).await?;
    if live as u64 + u64::from(waiting) >= u64::from(tunables.maximum_displays_per_session) {
        return refuse(Refusal::DisplayLimit);
    }
    found.campaign = Some(campaign);
    found.session = Some(session);
    found.state = PairingState::Approved;
    found.approved_by = Some(acting.member);
    found.approved_at_ms = Some(now_ms);
    found.renewable_until_ms = Some(now_ms.saturating_add(tunables.display_pairing_maximum_ms()));
    found.user_code = None;
    if !q::approve_pairing_row(tx, &found).await? {
        return pairing_refusal(PairingRefusal::NotPending);
    }
    Ok(())
}

fn new_ticket(
    pairing: &PairingRow,
    session: SessionId,
    campaign: CampaignId,
    approved_by: df_types::MemberId,
    digest: [u8; 32],
    expires_at_ms: u64,
    now_ms: u64,
) -> Outcome<TicketRow> {
    let id_bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    Ok(TicketRow {
        id: TicketId::from_bytes(&id_bytes).ok_or(StoreError::InvalidRecord)?,
        token_digest: digest,
        pairing: pairing.id,
        campaign,
        session,
        device_digest: pairing.device_digest,
        device_label: pairing.device_label,
        approved_by,
        issued_at_ms: now_ms,
        expires_at_ms,
        revoked_at_ms: None,
        revoke_reason: None,
    })
}

pub(super) async fn redeem_pairing(
    tx: &Tx<'_>,
    tunables: &Tunables,
    device_digest: [u8; 32],
    presented_ticket: Option<[u8; 32]>,
    new_ticket_digest: [u8; 32],
    session_usable: bool,
    now_ms: u64,
) -> Outcome<Redeemed> {
    let Some(first) = q::pairing_by_device(tx, device_digest).await? else {
        return refuse(Refusal::DisplayNeutral);
    };
    let Some(campaign) = first.campaign else {
        return pairing_refusal(PairingRefusal::NotApproved);
    };
    let row = locked_campaign(tx, campaign).await?;
    let Some(mut pairing) = q::pairing_by_device(tx, device_digest).await? else {
        return refuse(Refusal::DisplayNeutral);
    };
    let (Some(session), Some(approver)) = (pairing.session, pairing.approved_by) else {
        return pairing_refusal(PairingRefusal::NotApproved);
    };
    if row.status != CampaignStatus::Active {
        return refuse(Refusal::CampaignUnavailable);
    }
    if !session_usable {
        return refuse(Refusal::SessionEnded);
    }
    let live = q::live_tickets_of_session(tx, campaign, session, now_ms).await?;
    let cookie_valid = match presented_ticket {
        Some(digest) => match q::ticket_by_digest(tx, digest).await? {
            Some(ticket) => {
                ticket.pairing == pairing.id
                    && ticket.revoked_at_ms.is_none()
                    && now_ms < ticket.expires_at_ms
            }
            None => false,
        },
        None => false,
    };
    let facts = PairingFacts {
        state: pairing.state,
        expires_at_ms: pairing.expires_at_ms,
        device_digest: pairing.device_digest,
        reissue_count: pairing.reissue_count,
        redeemed_at_ms: pairing.redeemed_at_ms,
    };
    let action = match redeem(
        &facts,
        &device_digest,
        cookie_valid,
        u32::try_from(live.len()).unwrap_or(u32::MAX),
        tunables.maximum_displays_per_session,
        tunables.display_reissue_window_ms(),
        now_ms,
    ) {
        Ok(action) => action,
        Err(refusal) => return pairing_refusal(refusal),
    };
    let cap = pairing.renewable_until_ms.unwrap_or(u64::MAX);
    // The approver's authority is checked at the first redemption and again at a reissue: a ticket is
    // never issued on authority the approver no longer has.
    let may_pair = member_may_pair(tx, campaign, approver).await?;
    let (expires_at_ms, reissued) = match action {
        RedeemAction::IssueFirst => {
            if !may_pair {
                return pairing_refusal(PairingRefusal::ApproverNotAllowed);
            }
            (
                now_ms.saturating_add(tunables.display_ticket_ms()).min(cap),
                false,
            )
        }
        RedeemAction::Reissue => {
            // A reissue replaces exactly one live ticket with one live ticket. Zero (unpaired,
            // revoked, ended) or several means there is nothing to replace, and the display must
            // pair again; it never turns a revocation back into a ticket.
            let mine: Vec<TicketRow> = live
                .iter()
                .filter(|ticket| ticket.pairing == pairing.id)
                .cloned()
                .collect();
            let [lost] = mine.as_slice() else {
                return refuse(Refusal::Ticket(TicketRefusal::Revoked));
            };
            if !may_pair {
                return refuse(Refusal::Ticket(TicketRefusal::ApproverLostAuthority));
            }
            // The replacement keeps the life the lost ticket had: a reissue extends nothing.
            let kept = lost.expires_at_ms;
            revoke_tickets(
                tx,
                campaign,
                &mine,
                RevokeReason::Reissued,
                None,
                None,
                now_ms,
            )
            .await?;
            (kept, true)
        }
    };
    let ticket = new_ticket(
        &pairing,
        session,
        campaign,
        approver,
        new_ticket_digest,
        expires_at_ms,
        now_ms,
    )?;
    q::insert_ticket(tx, &ticket).await?;
    if reissued {
        pairing.reissue_count = 1;
    } else {
        pairing.state = PairingState::Redeemed;
        pairing.redeemed_at_ms = Some(now_ms);
    }
    q::update_pairing(tx, &pairing).await?;
    Ok(Redeemed { ticket, reissued })
}

/// Whether the approving member still has the authority a ticket was issued under.
async fn approver_authorized(tx: &Tx<'_>, ticket: &TicketRow) -> Outcome<bool> {
    member_may_pair(tx, ticket.campaign, ticket.approved_by).await
}

/// Whether a member is still active and may pair displays (host, or co-host with the bit), in a
/// campaign that is still active.
async fn member_may_pair(tx: &Tx<'_>, campaign: CampaignId, member: MemberId) -> Outcome<bool> {
    let Some(campaign) = q::read_campaign(tx, campaign).await? else {
        return Ok(false);
    };
    if campaign.status != CampaignStatus::Active {
        return Ok(false);
    }
    let Some(member) = q::member_by_id(tx, campaign.id, member).await? else {
        return Ok(false);
    };
    if member.status != MemberStatus::Active {
        return Ok(false);
    }
    Ok(Standing::of(member.role, member.delegations).pairs)
}

pub(super) async fn verify_ticket(
    tx: &Tx<'_>,
    ticket_digest: [u8; 32],
    device_digest: [u8; 32],
    session_usable: bool,
    now_ms: u64,
) -> Outcome<VerifiedTicket> {
    let Some(ticket) = q::ticket_by_digest(tx, ticket_digest).await? else {
        return refuse(Refusal::Ticket(
            df_campaign::display_pairing::TicketRefusal::Revoked,
        ));
    };
    let authorized = approver_authorized(tx, &ticket).await?;
    let facts = TicketFacts {
        device_digest: ticket.device_digest,
        expires_at_ms: ticket.expires_at_ms,
        revoked: ticket.revoked_at_ms.is_some(),
    };
    match ticket_valid(&facts, &device_digest, authorized, session_usable, now_ms) {
        Ok(()) => Ok(VerifiedTicket { ticket }),
        Err(refusal) => refuse(Refusal::Ticket(refusal)),
    }
}

pub(super) async fn extend_ticket(
    tx: &Tx<'_>,
    tunables: &Tunables,
    ticket_digest: [u8; 32],
    device_digest: [u8; 32],
    session_usable: bool,
    now_ms: u64,
) -> Outcome<TicketRow> {
    let Some(first) = q::ticket_by_digest(tx, ticket_digest).await? else {
        return refuse(Refusal::Ticket(
            df_campaign::display_pairing::TicketRefusal::Revoked,
        ));
    };
    locked_campaign(tx, first.campaign).await?;
    let Some(mut ticket) = q::ticket_by_digest(tx, ticket_digest).await? else {
        return refuse(Refusal::Ticket(
            df_campaign::display_pairing::TicketRefusal::Revoked,
        ));
    };
    let authorized = approver_authorized(tx, &ticket).await?;
    let pairing = q::pairing_by_id(tx, ticket.pairing).await?;
    let facts = TicketFacts {
        device_digest: ticket.device_digest,
        expires_at_ms: ticket.expires_at_ms,
        revoked: ticket.revoked_at_ms.is_some(),
    };
    let policy = RenewalPolicy {
        ticket_lifetime_ms: tunables.display_ticket_ms(),
        window_ms: tunables.display_renewal_window_ms(),
        renewable_until_ms: pairing
            .and_then(|row| row.renewable_until_ms)
            .unwrap_or(ticket.expires_at_ms),
    };
    match extend(
        &facts,
        &device_digest,
        authorized,
        session_usable,
        &policy,
        now_ms,
    ) {
        Ok(expires_at_ms) => {
            q::set_ticket_expiry(tx, ticket.id, expires_at_ms).await?;
            ticket.expires_at_ms = expires_at_ms;
            Ok(ticket)
        }
        Err(refusal) => refuse(Refusal::Ticket(refusal)),
    }
}

/// Unpairs one ticket, or every ticket of the session. Authorized against the campaign in the
/// path: a ticket of another session or campaign answers as missing.
pub(super) async fn unpair(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
    ticket: Option<TicketId>,
    now_ms: u64,
) -> Outcome<u32> {
    locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    require_capability(&acting, Capability::StartOrPauseSession)?;
    let targets = match ticket {
        Some(id) => match q::ticket_by_id(tx, campaign, session, id).await? {
            Some(row) => vec![row],
            None => return refuse(Refusal::NotFound),
        },
        None => q::live_tickets_of_session(tx, campaign, session, now_ms).await?,
    };
    let revoked = revoke_tickets(
        tx,
        campaign,
        &targets,
        RevokeReason::Unpaired,
        Some(acting.member),
        None,
        now_ms,
    )
    .await?;
    if ticket.is_none() {
        q::cancel_approved_pairings(tx, campaign, Some(session)).await?;
    }
    Ok(revoked)
}

/// Revokes every live ticket of a session for a reason with no actor (session end, placement
/// release). The caller's scope has been checked; the fence is recorded for audit only.
pub(super) async fn revoke_session_tickets(
    tx: &Tx<'_>,
    session: SessionId,
    campaign: CampaignId,
    reason: RevokeReason,
    writer_fence: Option<OwnerFence>,
    now_ms: u64,
) -> Outcome<u32> {
    locked_campaign(tx, campaign).await?;
    let tickets = q::live_tickets_of_session(tx, campaign, session, now_ms).await?;
    let fence = writer_fence.map(|fence| fence.audit_bytes());
    let revoked = revoke_tickets(tx, campaign, &tickets, reason, None, fence, now_ms).await?;
    q::cancel_approved_pairings(tx, campaign, Some(session)).await?;
    Ok(revoked)
}
