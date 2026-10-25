//! Invitations, room codes and join requests: tokens and codes are made and read here, the
//! guess throttles are applied here, and the store decides everything else.

use std::{net::IpAddr, time::Instant};

use df_campaign_wire::{InvitationCreated, InvitePreview, RoomCodeCreated};
use df_types::{AccountId, CampaignId, OperationId};

use super::{CampaignService, Ports, refuse};
use crate::{
    audit::{AuditLine, failure_word},
    errors::{Outcome, Refusal, StoreError},
    ids::{InvitationId, RequestId},
    join_invitation::JoinRole,
    records::{CodeRow, MemberRow, RequestRow},
    roles::Capability,
    secrets::{INVITATION_TOKEN_TAG, ROOM_CODE_TAG, SecretToken, keyed_digest, random_bytes},
    short_code::ShortCode,
    store::{Accepted, AccountDirectory, CampaignStore, Decision, NewInvitation, PreviewFacts},
};

impl<P: Ports> CampaignService<P> {
    pub async fn create_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        joins_as: JoinRole,
        lifetime_hours: Option<u32>,
        max_uses: Option<u32>,
    ) -> Outcome<(InvitationCreated, InvitationId)> {
        let tunables = &self.inner.tunables;
        let snapshot = self
            .require_full_capability(actor, campaign, Capability::InviteOrRemovePlayers)
            .await?;
        let allowed = self
            .inner
            .throttles
            .invitation_create
            .attempt(actor, self.now_ms());
        if allowed.is_err() {
            return refuse(Refusal::Throttled);
        }
        let lifetime_ms = match lifetime_hours {
            Some(hours) if (1..=tunables.invitation_lifetime_hours).contains(&hours) => {
                u64::from(hours) * 3_600_000
            }
            Some(_) => return refuse(Refusal::InvalidInput),
            None => tunables.invitation_lifetime_ms(),
        };
        // The default is the knob when set, else the seats (players) or the contributor limit.
        let default_uses = match (tunables.invitation_default_max_uses, joins_as) {
            (0, JoinRole::Player) => snapshot.seat_count,
            (0, JoinRole::Contributor) => tunables.maximum_contributors,
            (uses, _) => uses,
        };
        let uses = match max_uses {
            Some(uses) if (1..=1_000).contains(&uses) => uses,
            Some(_) => return refuse(Refusal::InvalidInput),
            None => default_uses.max(1),
        };
        let token = SecretToken::generate().map_err(|_| StoreError::RandomUnavailable)?;
        let row = self
            .inner
            .store
            .create_invitation(
                actor,
                campaign,
                NewInvitation {
                    joins_as,
                    lifetime_ms,
                    max_uses: uses,
                    token_digest: token.digest(INVITATION_TOKEN_TAG),
                },
                self.now_ms(),
            )
            .await?;
        AuditLine::new("INVITE", "created")
            .campaign(campaign)
            .actor(actor)
            .invitation(row.id)
            .reason("ok")
            .emit("invite.created");
        let text = token.text();
        Ok((
            InvitationCreated {
                invitation_id: row.id.text(),
                link: format!("/app/join#token={text}"),
                token: text,
                expires_at_ms: row.expires_at_ms,
                max_uses: row.max_uses,
            },
            row.id,
        ))
    }

    pub async fn revoke_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        invitation: InvitationId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .revoke_invitation(actor, campaign, invitation, self.now_ms())
            .await;
        if result.is_ok() {
            AuditLine::new("INVITE", "revoked")
                .campaign(campaign)
                .actor(actor)
                .invitation(invitation)
                .reason("ok")
                .emit("invite.revoked");
        }
        result
    }

    fn token_digest(text: &str) -> Option<[u8; 32]> {
        SecretToken::parse(text).map(|token| token.digest(INVITATION_TOKEN_TAG))
    }

    async fn preview_of(&self, facts: PreviewFacts) -> Outcome<InvitePreview> {
        let host = self.inner.accounts.profile(facts.host_account).await?;
        Ok(InvitePreview {
            campaign_name: facts.campaign.name,
            host_display_name: host
                .and_then(|profile| profile.display_name)
                .unwrap_or_else(|| "The host".to_owned()),
            scheduled_at_ms: facts.campaign.planned_session_at_ms,
            content_rating: facts.campaign.content_rating,
        })
    }

    /// `require_verified_email_to_join` applies to every way into a table: a link, a code lookup
    /// and a code request.
    async fn require_verified_to_join(&self, account: AccountId) -> Outcome<()> {
        if !self.inner.tunables.require_verified_email_to_join {
            return Ok(());
        }
        let verified = self
            .inner
            .accounts
            .profile(account)
            .await?
            .is_some_and(|profile| profile.verified);
        if verified {
            Ok(())
        } else {
            refuse(Refusal::NotAllowed)
        }
    }

    /// An unknown token and a malformed one are the same refusal. The preview has its own address
    /// layer (not the code gate and not the process ceiling), and a throttled preview says so:
    /// the throttle does not depend on the token, so it tells a guesser nothing.
    pub async fn preview_token(&self, ip: IpAddr, token: &str) -> Outcome<InvitePreview> {
        let started = Instant::now();
        let keys = self.ip_keys(ip);
        let gate = self.inner.throttles.preview_gate(&keys, self.now_ms());
        if !self.passed(
            gate,
            || AuditLine::new("INVITE", "refused"),
            ("invite.throttled", "invite.saturated"),
            Some(&keys),
        ) {
            return refuse(Refusal::Throttled);
        }
        let Some(digest) = Self::token_digest(token) else {
            AuditLine::new("INVITE", "refused")
                .reason("unknown")
                .emit("invite.refused");
            return refuse(Refusal::InvitationUnknown);
        };
        let result = self
            .inner
            .store
            .preview_invitation(digest, self.now_ms())
            .await;
        let facts = match result {
            Ok(facts) => facts,
            Err(failure) => {
                AuditLine::new("INVITE", "refused")
                    .reason(failure_word(failure))
                    .emit("invite.refused");
                return Err(failure);
            }
        };
        AuditLine::new("INVITE", "previewed")
            .campaign(facts.campaign.id)
            .reason("ok")
            .ms(started.elapsed().as_millis())
            .emit("invite.previewed");
        self.preview_of(facts).await
    }

    pub async fn accept_token(
        &self,
        account: AccountId,
        token: &str,
        operation: OperationId,
    ) -> Outcome<Accepted> {
        let allowed = self.inner.throttles.accept_gate(account, self.now_ms());
        if !self.passed(
            allowed,
            || AuditLine::new("INVITE", "refused").actor(account),
            ("invite.throttled", "invite.saturated"),
            None,
        ) {
            return refuse(Refusal::Throttled);
        }
        let Some(digest) = Self::token_digest(token) else {
            return refuse(Refusal::InvitationUnknown);
        };
        self.require_verified_to_join(account).await?;
        let result = self
            .inner
            .store
            .accept_invitation(account, digest, operation, self.now_ms())
            .await;
        let accepted = match result {
            Ok(accepted) => accepted,
            Err(failure) => {
                AuditLine::new("INVITE", "refused")
                    .actor(account)
                    .reason(failure_word(failure))
                    .emit("invite.refused");
                return Err(failure);
            }
        };
        AuditLine::new("INVITE", "accepted")
            .campaign(accepted.campaign)
            .actor(account)
            .member(accepted.member)
            .reason("ok")
            .emit("invite.accepted");
        AuditLine::new(
            "MEMBER",
            if accepted.rejoined {
                "rejoined"
            } else {
                "joined"
            },
        )
        .campaign(accepted.campaign)
        .member(accepted.member)
        .emit(if accepted.rejoined {
            "member.rejoined"
        } else {
            "member.joined"
        });
        self.notify(accepted.campaign).await;
        Ok(accepted)
    }

    fn code_digest(&self, code: &ShortCode) -> [u8; 32] {
        keyed_digest(
            self.inner.keys.code_key(),
            ROOM_CODE_TAG,
            code.canonical_text().as_bytes(),
        )
    }

    pub async fn rotate_code(
        &self,
        actor: AccountId,
        campaign: CampaignId,
    ) -> Outcome<(RoomCodeCreated, CodeRow)> {
        self.change_allowed(actor)?;
        let entropy = random_bytes::<5>().map_err(|_| StoreError::RandomUnavailable)?;
        let code = ShortCode::from_entropy(entropy);
        let row = self
            .inner
            .store
            .rotate_code(
                actor,
                campaign,
                self.code_digest(&code),
                self.inner.tunables.join_code_lifetime_ms(),
                self.now_ms(),
            )
            .await?;
        AuditLine::new("ROOMCODE", "rotated")
            .campaign(campaign)
            .actor(actor)
            .reason("ok")
            .emit("roomcode.rotated");
        Ok((
            RoomCodeCreated {
                code: code.display_form(),
                expires_at_ms: row.expires_at_ms,
            },
            row,
        ))
    }

    pub async fn lookup_code(
        &self,
        ip: IpAddr,
        account: AccountId,
        code: &str,
    ) -> Outcome<InvitePreview> {
        if !self.code_gate(ip, Some(account)) {
            return refuse(Refusal::CodeUnknown);
        }
        self.require_verified_to_join(account).await?;
        let Ok(code) = ShortCode::parse(code) else {
            AuditLine::new("ROOMCODE", "refused")
                .reason("unknown")
                .emit("roomcode.refused");
            return refuse(Refusal::CodeUnknown);
        };
        let facts = match self
            .inner
            .store
            .lookup_code(account, self.code_digest(&code), self.now_ms())
            .await
        {
            Ok(facts) => facts,
            Err(failure) => {
                AuditLine::new("ROOMCODE", "refused")
                    .actor(account)
                    .reason(failure_word(failure))
                    .emit("roomcode.refused");
                return Err(failure);
            }
        };
        AuditLine::new("ROOMCODE", "looked_up")
            .campaign(facts.campaign.id)
            .actor(account)
            .reason("ok")
            .emit("roomcode.looked_up");
        self.preview_of(facts).await
    }

    pub async fn request_join(
        &self,
        ip: IpAddr,
        account: AccountId,
        code: &str,
    ) -> Outcome<RequestRow> {
        if !self.code_gate(ip, Some(account)) {
            return refuse(Refusal::CodeUnknown);
        }
        self.require_verified_to_join(account).await?;
        let Ok(code) = ShortCode::parse(code) else {
            return refuse(Refusal::CodeUnknown);
        };
        let result = self
            .inner
            .store
            .request_join(account, self.code_digest(&code), self.now_ms())
            .await;
        match &result {
            Ok(request) => AuditLine::new("ROOMCODE", "request_created")
                .campaign(request.campaign)
                .actor(account)
                .reason("ok")
                .emit("roomcode.request_created"),
            Err(failure) => {
                let word = failure_word(*failure);
                let full = word == "requests_full";
                AuditLine::new("ROOMCODE", if full { "request_cap" } else { "refused" })
                    .actor(account)
                    .reason(word)
                    .emit(if full {
                        "roomcode.request_cap"
                    } else {
                        "roomcode.refused"
                    });
            }
        }
        result
    }

    pub async fn decide_request(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        request: RequestId,
        role: Option<JoinRole>,
    ) -> Outcome<Option<MemberRow>> {
        self.change_allowed(actor)?;
        let decision = role.map_or(Decision::Deny, Decision::Approve);
        let member = self
            .inner
            .store
            .decide_request(actor, campaign, request, decision, self.now_ms())
            .await?;
        AuditLine::new("ROOMCODE", "request_decided")
            .campaign(campaign)
            .actor(actor)
            .reason(if member.is_some() {
                "approved"
            } else {
                "denied"
            })
            .emit("roomcode.request_decided");
        if let Some(member) = &member {
            AuditLine::new(
                "MEMBER",
                if member.revision > 1 {
                    "rejoined"
                } else {
                    "joined"
                },
            )
            .campaign(campaign)
            .member(member.member)
            .emit(if member.revision > 1 {
                "member.rejoined"
            } else {
                "member.joined"
            });
            self.notify(campaign).await;
        }
        Ok(member)
    }
}
