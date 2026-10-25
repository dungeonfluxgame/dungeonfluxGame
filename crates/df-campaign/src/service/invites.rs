//! Invitations, room codes and join requests: tokens and codes are made and read here, the
//! guess throttles are applied here, and the store decides everything else.

use std::{future::Future, net::IpAddr, time::Instant};

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

const INVITATION_REFUSAL_FLOOR: std::time::Duration = std::time::Duration::from_millis(5);

fn conceal_invitation_failure(failure: crate::errors::Failure) -> crate::errors::Failure {
    match failure {
        crate::errors::Failure::Refused(
            Refusal::InvitationUnknown
            | Refusal::Join(
                crate::join_invitation::JoinFailure::Expired
                | crate::join_invitation::JoinFailure::Revoked
                | crate::join_invitation::JoinFailure::Exhausted,
            ),
        ) => crate::errors::Failure::Refused(Refusal::InvitationUnknown),
        other => other,
    }
}

async fn equalize_invitation_refusal(started: Instant) {
    tokio::time::sleep(INVITATION_REFUSAL_FLOOR.saturating_sub(started.elapsed())).await;
}

async fn accept_invitation_lookup<T, Lookup, LookupFuture>(
    started: Instant,
    parsed_digest: Option<[u8; 32]>,
    lookup: Lookup,
) -> Outcome<T>
where
    Lookup: FnOnce([u8; 32]) -> LookupFuture,
    LookupFuture: Future<Output = Outcome<T>>,
{
    let result = lookup(parsed_digest.unwrap_or([0; 32])).await;
    if parsed_digest.is_none() {
        equalize_invitation_refusal(started).await;
        return match result {
            Err(crate::errors::Failure::Store(error)) => Err(error.into()),
            _ => refuse(Refusal::InvitationUnknown),
        };
    }
    match result {
        Ok(value) => Ok(value),
        Err(failure) => {
            equalize_invitation_refusal(started).await;
            Err(conceal_invitation_failure(failure))
        }
    }
}

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
        self.window_outcome(allowed, "INVITE", "invite.create_saturated")?;
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
        let parsed = Self::token_digest(token);
        let digest = parsed.unwrap_or([0; 32]);
        let result = self
            .inner
            .store
            .preview_invitation(digest, self.now_ms())
            .await;
        let facts = match result {
            Ok(facts) if parsed.is_some() => facts,
            Ok(_) => {
                AuditLine::new("INVITE", "refused")
                    .reason("unknown")
                    .emit("invite.refused");
                equalize_invitation_refusal(started).await;
                return refuse(Refusal::InvitationUnknown);
            }
            Err(failure) => {
                AuditLine::new("INVITE", "refused")
                    .reason(failure_word(failure))
                    .emit("invite.refused");
                equalize_invitation_refusal(started).await;
                return Err(match (parsed.is_none(), failure) {
                    (true, crate::errors::Failure::Refused(_)) => {
                        crate::errors::Failure::Refused(Refusal::InvitationUnknown)
                    }
                    (false, failure) => conceal_invitation_failure(failure),
                    (true, failure) => failure,
                });
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
        let started = Instant::now();
        if !self.passed(
            allowed,
            || AuditLine::new("INVITE", "refused").actor(account),
            ("invite.throttled", "invite.saturated"),
            None,
        ) {
            return refuse(Refusal::Throttled);
        }
        let parsed_digest = Self::token_digest(token);
        self.require_verified_to_join(account).await?;
        let accepted = match accept_invitation_lookup(started, parsed_digest, |digest| {
            self.inner
                .store
                .accept_invitation(account, digest, operation, self.now_ms())
        })
        .await
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_invitation::JoinFailure;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[tokio::test]
    async fn malformed_and_unknown_acceptance_tokens_do_the_same_delayed_lookup() {
        let lookups = Arc::new(AtomicUsize::new(0));
        let delayed_unknown = |digest: [u8; 32]| {
            let lookups = Arc::clone(&lookups);
            async move {
                lookups.fetch_add(1, Ordering::SeqCst);
                assert!(digest == [0; 32] || digest == [7; 32]);
                tokio::time::sleep(Duration::from_millis(25)).await;
                Err::<(), _>(crate::errors::Failure::Refused(Refusal::InvitationUnknown))
            }
        };

        let malformed_started = Instant::now();
        let malformed = accept_invitation_lookup(malformed_started, None, delayed_unknown).await;
        let malformed_elapsed = malformed_started.elapsed();
        let known_shape_started = Instant::now();
        let unknown =
            accept_invitation_lookup(known_shape_started, Some([7; 32]), delayed_unknown).await;
        let known_shape_elapsed = known_shape_started.elapsed();

        assert_eq!(
            malformed,
            Err(crate::errors::Failure::Refused(Refusal::InvitationUnknown))
        );
        assert_eq!(malformed, unknown);
        assert_eq!(lookups.load(Ordering::SeqCst), 2);
        assert!(
            malformed_elapsed.abs_diff(known_shape_elapsed) <= Duration::from_millis(4),
            "malformed={malformed_elapsed:?}, well_formed={known_shape_elapsed:?}"
        );
    }

    #[test]
    fn unknown_expired_revoked_and_exhausted_invites_share_one_refusal() {
        let failures = [
            crate::errors::Failure::Refused(Refusal::InvitationUnknown),
            crate::errors::Failure::Refused(Refusal::Join(JoinFailure::Expired)),
            crate::errors::Failure::Refused(Refusal::Join(JoinFailure::Revoked)),
            crate::errors::Failure::Refused(Refusal::Join(JoinFailure::Exhausted)),
        ];
        for failure in failures {
            assert_eq!(
                conceal_invitation_failure(failure),
                crate::errors::Failure::Refused(Refusal::InvitationUnknown)
            );
        }
        assert_eq!(
            conceal_invitation_failure(crate::errors::Failure::Refused(Refusal::Join(
                JoinFailure::Full
            ))),
            crate::errors::Failure::Refused(Refusal::Join(JoinFailure::Full))
        );
    }
}
