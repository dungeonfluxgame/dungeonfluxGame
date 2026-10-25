//! Invitations, join requests, room codes and joining in the fake campaign directory (TODO(S1)).

use df_account_app::wire_campaign::{
    CampaignOverview, CampaignRole, InvitationCreated, InvitationKind, InvitePreview,
    JoinRequested, Reason,
};
use df_types::{CampaignId, OperationId};

use super::{
    campaigns::{Campaign, Invitation, JoinRequest, Member, Receipt, State, fresh_code},
    campaigns_rules::{because, host_name, seated},
};
use crate::app_api::ports::{Actor, PortError, PortResult, RequestDecision};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const INVITATION_LIFETIME_MS: u64 = 7 * DAY_MS;
const MAXIMUM_CONTRIBUTORS: usize = 12;
const MAXIMUM_REQUESTS: usize = 10;

impl State {
    pub(super) fn add_invitation(
        &mut self,
        campaign: CampaignId,
        kind: InvitationKind,
        now_ms: u64,
        origin: &str,
    ) -> Option<String> {
        let found = self.campaigns.get_mut(&campaign)?;
        let token = df_customer::SecretToken::generate().ok()?.text();
        let open_seats = u32::from(found.seats).saturating_sub(u32::from(seated(found)));
        found.invitations.push(Invitation {
            id: format!("inv-{}", found.invitations.len() + 1),
            kind,
            token: token.clone(),
            uses: 0,
            maximum_uses: open_seats.max(1),
            expires_at_ms: now_ms + INVITATION_LIFETIME_MS,
            is_revoked: false,
        });
        Some(format!("{origin}/app/join#token={token}"))
    }

    pub(super) fn create_invitation(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
        kind: InvitationKind,
        operation: OperationId,
        now_ms: u64,
        origin: &str,
    ) -> PortResult<InvitationCreated> {
        if let Some(Receipt::Invitation(first)) = self.receipts.replay(who.account, operation) {
            return Ok(first);
        }
        self.managed(who, campaign)?;
        let link = self
            .add_invitation(campaign, kind, now_ms, origin)
            .ok_or(PortError::Unavailable)?;
        let invitation_id = self
            .campaigns
            .get(&campaign)
            .and_then(|found| found.invitations.last())
            .map(|invitation| invitation.id.clone())
            .ok_or(PortError::Unavailable)?;
        let created = InvitationCreated {
            invitation_id,
            link,
        };
        self.receipts
            .record(who.account, operation, Receipt::Invitation(created.clone()));
        Ok(created)
    }

    pub(super) fn revoke_invitation(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
        invitation_id: &str,
    ) -> PortResult<CampaignOverview> {
        let found = self.managed(who, campaign)?;
        found
            .invitations
            .iter_mut()
            .find(|invitation| invitation.id == invitation_id)
            .ok_or(PortError::NotFound)?
            .is_revoked = true;
        self.overview(who, campaign)
    }

    pub(super) fn rotate_code(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
    ) -> PortResult<CampaignOverview> {
        self.managed(who, campaign)?.room_code = Some(fresh_code());
        self.overview(who, campaign)
    }

    pub(super) fn add_request(&mut self, campaign: CampaignId, requester: &Actor, now_ms: u64) {
        if let Some(found) = self.campaigns.get_mut(&campaign) {
            found.requests.push(JoinRequest {
                id: format!("req-{}", found.requests.len() + 1),
                account: requester.account,
                name: requester.display_name.clone(),
                requested_at_ms: now_ms,
            });
        }
    }

    pub(super) fn decide_request(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
        request_id: &str,
        decision: RequestDecision,
    ) -> PortResult<CampaignOverview> {
        let found = self.managed(who, campaign)?;
        let position = found
            .requests
            .iter()
            .position(|request| request.id == request_id)
            .ok_or(PortError::NotFound)?;
        if let RequestDecision::Approve(kind) = decision {
            let is_player = kind == InvitationKind::Player;
            if is_player && seated(found) >= found.seats {
                return Err(because(Reason::Full));
            }
            let request = found.requests.remove(position);
            found.members.push(Member {
                account: request.account,
                name: request.name,
                role: if is_player {
                    CampaignRole::Player
                } else {
                    CampaignRole::Contributor
                },
            });
        } else {
            found.requests.remove(position);
        }
        self.overview(who, campaign)
    }

    /// The invitation behind a token if it is currently usable for a new member.
    fn usable_invitation(&self, token: &str, now_ms: u64) -> PortResult<(&Campaign, &Invitation)> {
        let (campaign, invitation) = self
            .campaigns
            .values()
            .find_map(|campaign| {
                campaign
                    .invitations
                    .iter()
                    .find(|invitation| invitation.token == token)
                    .map(|invitation| (campaign, invitation))
            })
            .ok_or(because(Reason::NoLongerValid))?;
        if invitation.is_revoked {
            return Err(because(Reason::Revoked));
        }
        if invitation.expires_at_ms <= now_ms {
            return Err(because(Reason::Expired));
        }
        if invitation.uses >= invitation.maximum_uses {
            return Err(because(Reason::Exhausted));
        }
        Ok((campaign, invitation))
    }

    fn has_room(campaign: &Campaign, kind: InvitationKind) -> Result<(), PortError> {
        match kind {
            InvitationKind::Player if seated(campaign) >= campaign.seats => {
                Err(because(Reason::Full))
            }
            InvitationKind::Contributor
                if campaign
                    .members
                    .iter()
                    .filter(|member| !member.has_seat())
                    .count()
                    >= MAXIMUM_CONTRIBUTORS =>
            {
                Err(because(Reason::ContributorsFull))
            }
            _ => Ok(()),
        }
    }

    fn preview_of(campaign: &Campaign) -> InvitePreview {
        InvitePreview {
            campaign_name: campaign.name.clone(),
            host_name: host_name(campaign),
            scheduled_at_ms: None,
            content_rating: campaign.rating,
        }
    }

    pub(super) fn preview_invitation(
        &self,
        token: &str,
        now_ms: u64,
    ) -> PortResult<(CampaignId, InvitePreview)> {
        let (campaign, invitation) = self.usable_invitation(token, now_ms)?;
        Self::has_room(campaign, invitation.kind)?;
        Ok((campaign.id, Self::preview_of(campaign)))
    }

    pub(super) fn accept_invitation(
        &mut self,
        who: &Actor,
        token: &str,
        operation: OperationId,
        now_ms: u64,
    ) -> PortResult<CampaignId> {
        if let Some(Receipt::Joined(first)) = self.receipts.replay(who.account, operation) {
            return Ok(first);
        }
        let (campaign, invitation) = self.usable_invitation(token, now_ms)?;
        if campaign
            .members
            .iter()
            .any(|member| member.account == who.account)
        {
            return Err(because(Reason::AlreadyJoined));
        }
        Self::has_room(campaign, invitation.kind)?;
        let (id, kind) = (campaign.id, invitation.kind);
        let found = self.campaigns.get_mut(&id).ok_or(PortError::NotFound)?;
        if let Some(invitation) = found
            .invitations
            .iter_mut()
            .find(|invitation| invitation.token == token)
        {
            invitation.uses += 1;
        }
        found.members.push(Member {
            account: who.account,
            name: who.display_name.clone(),
            role: match kind {
                InvitationKind::Player => CampaignRole::Player,
                InvitationKind::Contributor => CampaignRole::Contributor,
            },
        });
        self.receipts
            .record(who.account, operation, Receipt::Joined(id));
        Ok(id)
    }

    pub(super) fn lookup_code(
        &self,
        _who: &Actor,
        code: &str,
    ) -> PortResult<(CampaignId, InvitePreview)> {
        let wanted = code.trim().to_ascii_uppercase();
        let campaign = self
            .campaigns
            .values()
            .find(|campaign| campaign.room_code.as_deref() == Some(wanted.as_str()))
            .ok_or(because(Reason::NoLongerValid))?;
        Ok((campaign.id, Self::preview_of(campaign)))
    }

    pub(super) fn request_by_code(
        &mut self,
        who: &Actor,
        code: &str,
        operation: OperationId,
        now_ms: u64,
    ) -> PortResult<JoinRequested> {
        if let Some(Receipt::Requested(first)) = self.receipts.replay(who.account, operation) {
            return Ok(first);
        }
        let (id, _) = self.lookup_code(who, code)?;
        let found = self.campaigns.get(&id).ok_or(PortError::NotFound)?;
        if found
            .members
            .iter()
            .any(|member| member.account == who.account)
        {
            return Err(because(Reason::AlreadyJoined));
        }
        let requested = JoinRequested {
            campaign_name: found.name.clone(),
        };
        let is_pending = found
            .requests
            .iter()
            .any(|request| request.account == who.account);
        if !is_pending {
            if found.requests.len() >= MAXIMUM_REQUESTS {
                return Err(because(Reason::RequestsFull));
            }
            self.add_request(id, who, now_ms);
        }
        self.receipts.record(
            who.account,
            operation,
            Receipt::Requested(requested.clone()),
        );
        Ok(requested)
    }
}
