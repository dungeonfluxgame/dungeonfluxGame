//! Creating, archiving, unarchiving and deleting campaigns.

use std::time::Instant;

use df_types::{AccountId, CampaignId, OperationId};

use super::{CampaignService, Ports, refuse};
use crate::{
    audit::{AuditLine, failure_word},
    errors::{Outcome, Refusal},
    records::{CampaignRow, ContentRating, NewCampaign, RoomCodes},
    roles::Delegations,
    secrets::tagged_digest,
    store::{AccountDirectory, CampaignStore, SessionDirectory},
};

/// What a creator supplies.
#[derive(Clone, Debug)]
pub struct CreateCampaign {
    pub name: String,
    pub seat_count: u32,
    pub content_rating: ContentRating,
    pub room_codes: RoomCodes,
    pub default_co_host_delegations: Delegations,
    pub operation: OperationId,
}

impl<P: Ports> CampaignService<P> {
    pub async fn create_campaign(
        &self,
        account: AccountId,
        request: CreateCampaign,
    ) -> Outcome<CampaignRow> {
        let started = Instant::now();
        let tunables = &self.inner.tunables;
        let name = request.name.trim().to_owned();
        if name.is_empty()
            || name.chars().count() > 80
            || request.seat_count == 0
            || request.seat_count > tunables.maximum_seats
        {
            return refuse(Refusal::InvalidInput);
        }
        // Creating is a change like any other: the per-account limit comes before any read.
        self.change_allowed(account)?;
        let Some(profile) = self.inner.accounts.profile(account).await? else {
            return refuse(Refusal::NotFound);
        };
        if !profile.status.is_active() {
            return refuse(Refusal::AccountClosing);
        }
        // A verified creator always has a tenant; both are checked here, not by a foreign key.
        let (true, Some(tenant)) = (profile.verified, profile.tenant) else {
            return refuse(Refusal::NotAllowed);
        };
        let fingerprint = tagged_digest(
            "df-campaign create fingerprint v1",
            format!(
                "{}|{}|{:?}|{:?}|{}",
                name,
                request.seat_count,
                request.content_rating,
                request.room_codes,
                request.default_co_host_delegations.bits()
            )
            .as_bytes(),
        );
        let result = self
            .inner
            .store
            .create_campaign(
                NewCampaign {
                    tenant,
                    owner: account,
                    name,
                    seat_count: request.seat_count,
                    content_rating: request.content_rating,
                    room_codes: request.room_codes,
                    default_co_host_delegations: request.default_co_host_delegations,
                },
                request.operation,
                fingerprint,
                self.now_ms(),
            )
            .await;
        match &result {
            Ok(created) => AuditLine::new("CAMPAIGN", "created")
                .campaign(created.id)
                .actor(account)
                .reason("ok")
                .ms(started.elapsed().as_millis())
                .emit("campaign.created"),
            Err(failure) => AuditLine::new("CAMPAIGN", "refused")
                .actor(account)
                .reason(failure_word(*failure))
                .emit("campaign.refused"),
        }
        result
    }

    /// The session state is read before the campaign lock (decision 29). A session that cannot be
    /// read blocks the change: it may be past the lobby.
    async fn past_lobby(&self, campaign: CampaignId) -> Outcome<bool> {
        match self.inner.sessions.live_session_of(campaign).await? {
            Some((_, phase)) => Ok(phase.is_past_lobby()),
            None => Ok(false),
        }
    }

    fn audit_campaign(
        &self,
        verb: &'static str,
        event: &'static str,
        campaign: CampaignId,
        actor: AccountId,
        result: &Outcome<()>,
    ) {
        let line = match result {
            Ok(()) => AuditLine::new("CAMPAIGN", verb).reason("ok"),
            Err(failure) => AuditLine::new("CAMPAIGN", "refused").reason(failure_word(*failure)),
        };
        line.campaign(campaign)
            .actor(actor)
            .emit(if result.is_ok() {
                event
            } else {
                "campaign.refused"
            });
    }

    pub async fn archive(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let past = self.past_lobby(campaign).await?;
        let result = self
            .inner
            .store
            .archive_campaign(actor, campaign, typed_name, past, self.now_ms())
            .await;
        self.audit_campaign("archived", "campaign.archived", campaign, actor, &result);
        result
    }

    pub async fn unarchive(&self, actor: AccountId, campaign: CampaignId) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .unarchive_campaign(actor, campaign, self.now_ms())
            .await;
        self.audit_campaign(
            "unarchived",
            "campaign.unarchived",
            campaign,
            actor,
            &result,
        );
        result
    }

    pub async fn request_delete(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let past = self.past_lobby(campaign).await?;
        let result = self
            .inner
            .store
            .request_delete(actor, campaign, typed_name, past, self.now_ms())
            .await;
        self.audit_campaign(
            "delete_requested",
            "campaign.delete_requested",
            campaign,
            actor,
            &result,
        );
        result
    }
}
