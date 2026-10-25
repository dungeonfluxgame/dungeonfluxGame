//! Leaving, removal, readmission, co-hosts, seats, roles and transfers. Each change is limited per
//! account, goes to the store (which authorizes under the campaign lock) and is audited.

use df_types::{AccountId, CampaignId, MemberId};

use super::{CampaignService, Ports};
use crate::{
    audit::{AuditLine, failure_word},
    errors::Outcome,
    ids::ProposalId,
    join_invitation::JoinRole,
    records::TransferKind,
    roles::Delegations,
    store::{CampaignStore, NewProposal},
};

impl<P: Ports> CampaignService<P> {
    fn audit_member(
        &self,
        verb: &'static str,
        event: &'static str,
        campaign: CampaignId,
        member: Option<MemberId>,
        result: &Outcome<()>,
    ) {
        let mut line = match result {
            Ok(()) => AuditLine::new("MEMBER", verb).reason("ok"),
            Err(failure) => AuditLine::new("MEMBER", "refused").reason(failure_word(*failure)),
        }
        .campaign(campaign);
        if let Some(member) = member {
            line = line.member(member);
        }
        line.emit(if result.is_ok() {
            event
        } else {
            "member.refused"
        });
    }

    pub async fn remove_member(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .remove_member(actor, campaign, member, self.now_ms())
            .await;
        self.audit_member("removed", "member.removed", campaign, Some(member), &result);
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    pub async fn leave(&self, actor: AccountId, campaign: CampaignId) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .leave_campaign(actor, campaign, self.now_ms())
            .await;
        self.audit_member("left", "member.left", campaign, None, &result);
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    pub async fn allow_rejoin(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .allow_rejoin(actor, campaign, member, self.now_ms())
            .await;
        self.audit_member(
            "rejoin_allowed",
            "member.rejoin_allowed",
            campaign,
            Some(member),
            &result,
        );
        result
    }

    pub async fn set_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        delegations: Delegations,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .set_co_host(actor, campaign, member, delegations, self.now_ms())
            .await;
        self.audit_member(
            "delegations_changed",
            "member.delegations_changed",
            campaign,
            Some(member),
            &result,
        );
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    pub async fn demote_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .demote_co_host(actor, campaign, member, self.now_ms())
            .await;
        self.audit_member(
            "role_changed",
            "member.role_changed",
            campaign,
            Some(member),
            &result,
        );
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    pub async fn set_seat(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        seated: bool,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .set_seat(actor, campaign, seated, self.now_ms())
            .await;
        self.audit_member(
            "seat_changed",
            "member.seat_changed",
            campaign,
            None,
            &result,
        );
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    /// Host only; the store enforces it under the campaign lock.
    pub async fn set_player_role(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        role: JoinRole,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .set_player_role(actor, campaign, member, role, self.now_ms())
            .await;
        self.audit_member(
            "role_changed",
            "member.role_changed",
            campaign,
            Some(member),
            &result,
        );
        if result.is_ok() {
            self.notify(campaign).await;
        }
        result
    }

    pub async fn propose_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: NewProposal,
    ) -> Outcome<ProposalId> {
        self.change_allowed(actor)?;
        let kind = proposal.kind;
        let result = self
            .inner
            .store
            .propose_transfer(actor, campaign, proposal, self.now_ms())
            .await;
        match (&result, kind) {
            (Ok(_), TransferKind::Host) => AuditLine::new("MEMBER", "host_transfer_proposed")
                .campaign(campaign)
                .member(proposal.to_member)
                .reason("ok")
                .emit("member.host_transfer_proposed"),
            (Ok(_), TransferKind::Ownership) => AuditLine::new("CAMPAIGN", "ownership_proposed")
                .campaign(campaign)
                .actor(actor)
                .reason("ok")
                .emit("campaign.ownership_proposed"),
            (Err(failure), _) => AuditLine::new("CAMPAIGN", "refused")
                .campaign(campaign)
                .actor(actor)
                .reason(failure_word(*failure))
                .emit("campaign.refused"),
        }
        result
    }

    pub async fn accept_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .accept_transfer(actor, campaign, proposal, self.now_ms())
            .await;
        match result {
            Ok(kind) => {
                let (family, verb, event) = match kind {
                    TransferKind::Host => ("MEMBER", "host_transferred", "member.host_transferred"),
                    TransferKind::Ownership => (
                        "CAMPAIGN",
                        "ownership_transferred",
                        "campaign.ownership_transferred",
                    ),
                };
                AuditLine::new(family, verb)
                    .campaign(campaign)
                    .actor(actor)
                    .reason("ok")
                    .emit(event);
                self.notify(campaign).await;
                Ok(())
            }
            Err(failure) => {
                self.audit_member("refused", "member.refused", campaign, None, &Err(failure));
                Err(failure)
            }
        }
    }

    pub async fn cancel_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
    ) -> Outcome<()> {
        self.change_allowed(actor)?;
        let result = self
            .inner
            .store
            .cancel_transfer(actor, campaign, proposal, self.now_ms())
            .await;
        self.audit_member(
            "transfer_cancelled",
            "member.transfer_cancelled",
            campaign,
            None,
            &result,
        );
        result
    }
}
