//! Display pairing and tickets: challenges, codes, approval, redemption, renewal, verification and
//! revocation. Authority is checked before anything about a session is read, and every public
//! entry has its own throttle layer.

use std::net::IpAddr;

use df_campaign_wire::DisplayLookupResponse;
use df_types::{AccountId, CampaignId, SessionId};

use super::{CampaignService, Ports, refuse};
use crate::{
    audit::{AuditLine, failure_word},
    display_pairing::TicketRefusal,
    errors::{Failure, Outcome, Refusal, StoreError},
    ids::{PairingId, TicketId},
    records::{DeviceLabel, PairingRow, RevokeReason, TicketRow},
    roles::Capability,
    secrets::{DEVICE_SECRET_TAG, DISPLAY_TICKET_TAG, SecretToken, random_bytes},
    short_code::ShortCode,
    store::{
        CampaignStore, NewPairing, OwnerFence, Redeemed, RevocationCaller, SessionDirectory,
        SessionPhase, VerifiedTicket,
    },
};

/// A device secret and its pairing, as the display receives them.
pub struct Challenge {
    pub pairing: PairingRow,
    pub device_secret: String,
}

pub struct RedeemedTicket {
    pub redeemed: Redeemed,
    pub ticket_secret: String,
}

impl<P: Ports> CampaignService<P> {
    pub async fn begin_display_pairing(
        &self,
        ip: IpAddr,
        label: DeviceLabel,
    ) -> Outcome<Challenge> {
        let now = self.now_ms();
        let keys = self.ip_keys(ip);
        let gate = self.inner.throttles.challenge_gate(&keys, now);
        if !self.passed(
            gate,
            || AuditLine::new("DISPLAY", "challenge_refused"),
            ("display.challenge_refused", "display.challenge_refused"),
            Some(&keys),
        ) {
            return refuse(Refusal::Throttled);
        }
        // A code that collides with a live pending one is drawn again.
        let mut last = Failure::Store(StoreError::Unavailable);
        for _ in 0..4 {
            let secret = SecretToken::generate().map_err(|_| StoreError::RandomUnavailable)?;
            let entropy = random_bytes::<5>().map_err(|_| StoreError::RandomUnavailable)?;
            let code = ShortCode::from_entropy(entropy);
            let new = NewPairing {
                device_digest: secret.digest(DEVICE_SECRET_TAG),
                device_label: label,
                ip_key: keys.tight,
                ip_key_loose: keys.loose,
                user_code: code.canonical_text(),
            };
            match self
                .inner
                .store
                .begin_pairing(new, self.inner.tunables.display_challenge_ms(), now)
                .await
            {
                Ok(pairing) => {
                    AuditLine::new("DISPLAY", "challenge_created")
                        .reason("ok")
                        .emit("display.challenge_created");
                    return Ok(Challenge {
                        pairing,
                        device_secret: secret.text(),
                    });
                }
                Err(Failure::Store(StoreError::Retryable)) => {
                    last = Failure::Store(StoreError::Retryable);
                }
                Err(other) => {
                    AuditLine::new("DISPLAY", "challenge_refused")
                        .reason(failure_word(other))
                        .emit("display.challenge_refused");
                    return Err(other);
                }
            }
        }
        Err(last)
    }

    fn device_digest(secret: &str) -> Option<[u8; 32]> {
        SecretToken::parse(secret).map(|token| token.digest(DEVICE_SECRET_TAG))
    }

    fn ticket_digest(secret: &str) -> Option<[u8; 32]> {
        SecretToken::parse(secret).map(|token| token.digest(DISPLAY_TICKET_TAG))
    }

    /// Side-effect free, and limited per address because each call reads the database. A missing,
    /// malformed or unknown secret is the one neutral refusal.
    pub async fn display_status(&self, ip: IpAddr, device_secret: &str) -> Outcome<PairingRow> {
        let keys = self.ip_keys(ip);
        let gate = self.inner.throttles.status_gate(&keys, self.now_ms());
        if !self.passed(
            gate,
            || AuditLine::new("DISPLAY", "redeem_refused"),
            ("display.status_throttled", "display.status_saturated"),
            Some(&keys),
        ) {
            return refuse(Refusal::Throttled);
        }
        let Some(device) = Self::device_digest(device_secret) else {
            return refuse(Refusal::DisplayNeutral);
        };
        self.inner.store.pairing_status(device, self.now_ms()).await
    }

    pub async fn lookup_display_code(
        &self,
        ip: IpAddr,
        actor: AccountId,
        campaign: CampaignId,
        code: &str,
    ) -> Outcome<DisplayLookupResponse> {
        self.require_full_capability(actor, campaign, Capability::StartOrPauseSession)
            .await?;
        if !self.code_gate(ip, Some(actor)) {
            return refuse(Refusal::DisplayNeutral);
        }
        let Ok(code) = ShortCode::parse(code) else {
            return refuse(Refusal::DisplayNeutral);
        };
        let now = self.now_ms();
        let pairing = self
            .inner
            .store
            .lookup_pairing(actor, campaign, code.canonical_text(), now)
            .await?;
        AuditLine::new("DISPLAY", "looked_up")
            .campaign(campaign)
            .actor(actor)
            .reason("ok")
            .emit("display.looked_up");
        Ok(DisplayLookupResponse {
            pairing_id: pairing.id.text(),
            created_at_ms: pairing.created_at_ms,
            device_label: pairing.device_label,
            waiting_ms: now.saturating_sub(pairing.created_at_ms),
        })
    }

    /// The session must belong to this campaign and still be usable; any other session answers
    /// like a missing one. Only called after the caller has been authorized for the campaign.
    async fn session_of(&self, campaign: CampaignId, session: SessionId) -> Outcome<SessionPhase> {
        match self.inner.sessions.session(session).await? {
            Some(facts) if facts.campaign == campaign => Ok(facts.phase),
            _ => refuse(Refusal::NotFound),
        }
    }

    pub async fn approve_display_pairing(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        pairing: PairingId,
    ) -> Outcome<()> {
        // Authority first: an outsider learns nothing about a session's state.
        self.require_full_capability(actor, campaign, Capability::StartOrPauseSession)
            .await?;
        let allowed = self
            .inner
            .throttles
            .display_approval
            .attempt(actor, self.now_ms());
        if allowed.is_err() {
            return refuse(Refusal::Throttled);
        }
        if !self.session_of(campaign, session).await?.is_usable() {
            return refuse(Refusal::SessionEnded);
        }
        let result = self
            .inner
            .store
            .approve_pairing(actor, campaign, session, pairing, self.now_ms())
            .await;
        match &result {
            Ok(()) => AuditLine::new("DISPLAY", "approved")
                .campaign(campaign)
                .session(session)
                .actor(actor)
                .reason("ok")
                .emit("display.approved"),
            Err(failure) => AuditLine::new("DISPLAY", "redeem_refused")
                .session(session)
                .reason(failure_word(*failure))
                .emit("display.redeem_refused"),
        }
        result
    }

    /// Fails closed: a session that cannot be read counts as ended.
    async fn session_usable(&self, session: SessionId) -> bool {
        matches!(
            self.inner.sessions.session(session).await,
            Ok(Some(facts)) if facts.phase.is_usable()
        )
    }

    /// The address layer in front of redeem and renew. Their per-pairing and per-ticket windows
    /// are keyed by cookies the caller chooses; nothing reaches them before this has passed.
    fn ticket_gate(&self, ip: IpAddr) -> bool {
        let keys = self.ip_keys(ip);
        let gate = self.inner.throttles.ticket_gate(&keys, self.now_ms());
        self.passed(
            gate,
            || AuditLine::new("DISPLAY", "redeem_refused"),
            ("display.ticket_throttled", "display.ticket_saturated"),
            Some(&keys),
        )
    }

    pub async fn redeem_display_ticket(
        &self,
        ip: IpAddr,
        device_secret: &str,
        presented_ticket: Option<&str>,
    ) -> Outcome<RedeemedTicket> {
        if !self.ticket_gate(ip) {
            return refuse(Refusal::Throttled);
        }
        let Some(device) = Self::device_digest(device_secret) else {
            return refuse(Refusal::DisplayNeutral);
        };
        let now = self.now_ms();
        // An unknown device secret stops here and counts against the address only.
        let pairing = self.inner.store.pairing_status(device, now).await?;
        // At most a few redemptions per pairing per hour: each call opens a transaction. The window
        // exists only for a pairing the store has just confirmed, so random cookies cannot fill it.
        if self
            .inner
            .throttles
            .redeem_pairing
            .attempt(device, now)
            .is_err()
        {
            return refuse(Refusal::Throttled);
        }
        let usable = match pairing.session {
            Some(session) => self.session_usable(session).await,
            None => true,
        };
        let presented = presented_ticket.and_then(Self::ticket_digest);
        let secret = SecretToken::generate().map_err(|_| StoreError::RandomUnavailable)?;
        let result = self
            .inner
            .store
            .redeem_pairing(
                device,
                presented,
                secret.digest(DISPLAY_TICKET_TAG),
                usable,
                now,
            )
            .await;
        let redeemed = match result {
            Ok(redeemed) => redeemed,
            Err(failure) => {
                AuditLine::new("DISPLAY", "redeem_refused")
                    .reason(failure_word(failure))
                    .emit("display.redeem_refused");
                return Err(failure);
            }
        };
        let verb = if redeemed.reissued {
            "reissued"
        } else {
            "redeemed"
        };
        AuditLine::new("DISPLAY", verb)
            .campaign(redeemed.ticket.campaign)
            .session(redeemed.ticket.session)
            .reason("ok")
            .emit(if redeemed.reissued {
                "display.reissued"
            } else {
                "display.redeemed"
            });
        Ok(RedeemedTicket {
            redeemed,
            ticket_secret: secret.text(),
        })
    }

    pub async fn extend_display_ticket(
        &self,
        ip: IpAddr,
        device_secret: &str,
        ticket_secret: &str,
    ) -> Outcome<TicketRow> {
        if !self.ticket_gate(ip) {
            return refuse(Refusal::Throttled);
        }
        let (Some(device), Some(ticket)) = (
            Self::device_digest(device_secret),
            Self::ticket_digest(ticket_secret),
        ) else {
            return refuse(Refusal::DisplayNeutral);
        };
        let now = self.now_ms();
        // The ticket's session decides whether it is still usable.
        let verified = self
            .inner
            .store
            .verify_ticket(ticket, device, true, now)
            .await;
        let usable = match &verified {
            Ok(found) => self.session_usable(found.ticket.session).await,
            // An unknown ticket answers like a revoked one: there is nothing to renew and no
            // window to count against, so a random cookie costs the address layer and nothing else.
            Err(failure @ Failure::Refused(Refusal::Ticket(TicketRefusal::Revoked)))
            | Err(failure @ Failure::Store(_)) => return Err(*failure),
            Err(_) => true,
        };
        // Counted only now that the ticket exists: the window is keyed by a cookie the caller picks.
        if self
            .inner
            .throttles
            .renew_ticket
            .attempt(ticket, now)
            .is_err()
        {
            return refuse(Refusal::Throttled);
        }
        let result = self
            .inner
            .store
            .extend_ticket(ticket, device, usable, now)
            .await;
        if let Ok(extended) = &result {
            AuditLine::new("DISPLAY", "extended")
                .campaign(extended.campaign)
                .session(extended.session)
                .reason("ok")
                .emit("display.extended");
        }
        result
    }

    /// O1's edge calls this before minting a `Display` connection ticket: both secrets and the
    /// session the ticket was issued for.
    pub async fn verify_display_ticket(
        &self,
        ticket_secret: &str,
        device_secret: &str,
        session: SessionId,
    ) -> Outcome<VerifiedTicket> {
        let (Some(device), Some(ticket)) = (
            Self::device_digest(device_secret),
            Self::ticket_digest(ticket_secret),
        ) else {
            return refuse(Refusal::DisplayNeutral);
        };
        let usable = self.session_usable(session).await;
        let verified = self
            .inner
            .store
            .verify_ticket(ticket, device, usable, self.now_ms())
            .await?;
        if verified.ticket.session != session {
            return refuse(Refusal::Ticket(TicketRefusal::Revoked));
        }
        Ok(verified)
    }

    pub async fn unpair_display(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        ticket: Option<TicketId>,
    ) -> Outcome<u32> {
        self.require_full_capability(actor, campaign, Capability::StartOrPauseSession)
            .await?;
        self.change_allowed(actor)?;
        self.session_of(campaign, session).await?;
        let revoked = self
            .inner
            .store
            .unpair(actor, campaign, session, ticket, self.now_ms())
            .await?;
        AuditLine::new("DISPLAY", "unpaired")
            .campaign(campaign)
            .session(session)
            .actor(actor)
            .reason("ok")
            .emit("display.unpaired");
        Ok(revoked)
    }

    /// Session end and placement release: O1's runtime may reach only the sessions its own
    /// placement leases; the control plane may reach any.
    pub async fn revoke_display_tickets(
        &self,
        caller: &RevocationCaller,
        session: SessionId,
        campaign: CampaignId,
        reason: RevokeReason,
        fence: Option<OwnerFence>,
    ) -> Outcome<u32> {
        if !caller.may_revoke(session) {
            return refuse(Refusal::NotAllowed);
        }
        let revoked = self
            .inner
            .store
            .revoke_session_tickets(session, campaign, reason, fence, self.now_ms())
            .await?;
        if revoked > 0 {
            AuditLine::new("DISPLAY", "ticket_revoked")
                .campaign(campaign)
                .session(session)
                .reason("session")
                .emit("display.ticket_revoked");
        }
        Ok(revoked)
    }

    pub async fn session_started(&self, campaign: CampaignId, fence: OwnerFence) -> Outcome<()> {
        let result = self
            .inner
            .store
            .session_started(campaign, fence, self.now_ms())
            .await;
        if let Err(failure) = &result {
            AuditLine::new("LOBBY", "fence_refused")
                .campaign(campaign)
                .reason(failure_word(*failure))
                .emit("lobby.fence_refused");
        }
        result
    }
}
