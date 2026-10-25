//! Native account API adapters; missing owner contracts refuse without changing state.

use df_account_app::{
    wire::PlaySessionsView,
    wire_campaign::{
        CampaignCard, CampaignDraft, CampaignOverview, CampaignRole, CharacterRequest,
        DashboardView, DisplayState, InvitationCreated, InvitationKind, InvitePreview,
        JoinRequested, LobbyView, MicrophoneRequest, PairingLookup, WizardCatalog,
    },
};
use df_campaign::{
    service::{CampaignService, Ports as ServicePorts},
    store::*,
};
use df_customer::DeviceLabel;
use df_portal::app_api::ports::*;
use df_types::{AccountId, CampaignId, OperationId, SessionId};

pub(crate) struct Campaigns {
    service: CampaignService<Owners>,
    sessions: crate::campaign_sessions::Sessions,
}
struct Owners;

impl ServicePorts for Owners {
    type Store = crate::campaign_store::PgCampaignStore;
    type Accounts = crate::campaign_accounts::Accounts;
    type Sessions = crate::campaign_sessions::Sessions;
    type Funding = crate::campaign_funding::Funding;
    type Time = df_campaign::clock::SystemClock;
}

impl Campaigns {
    async fn campaign_of(&self, session: SessionId) -> PortResult<CampaignId> {
        self.sessions
            .session(session)
            .await
            .map_err(|_| PortError::Unavailable)?
            .map(|facts| facts.campaign)
            .ok_or(PortError::NotFound)
    }

    pub(crate) fn new(
        store: crate::campaign_store::PgCampaignStore,
        accounts: crate::campaign_accounts::Accounts,
        keys: df_campaign::service::ServiceKeys,
        sessions: crate::campaign_sessions::Sessions,
        funding: crate::campaign_funding::Funding,
    ) -> Self {
        Self {
            service: CampaignService::new(
                store,
                accounts,
                sessions.clone(),
                funding,
                df_campaign::clock::SystemClock,
                df_campaign::config::Tunables::default(),
                keys,
            ),
            sessions,
        }
    }
}

fn failure(error: df_campaign::errors::Failure) -> PortError {
    match error {
        df_campaign::errors::Failure::Store(df_campaign::errors::StoreError::Unknown) => {
            PortError::UnknownOutcome
        }
        df_campaign::errors::Failure::Store(_) => PortError::Unavailable,
        df_campaign::errors::Failure::Refused(df_campaign::errors::Refusal::NotFound) => {
            PortError::NotFound
        }
        df_campaign::errors::Failure::Refused(_) => PortError::Forbidden,
    }
}

impl CampaignDirectory for Campaigns {
    async fn dashboard(&self, who: &Actor) -> PortResult<DashboardView> {
        let dashboard = self.service.dashboard(who.account).await.map_err(failure)?;
        let mut campaigns = Vec::with_capacity(dashboard.campaigns.len());
        for entry in dashboard.campaigns {
            // The app's view has no Unknown session variant; never report an unreadable session as absent.
            let live_session_id = match entry.session {
                df_campaign::service::SessionReading::Live(live) => {
                    Some(hex(live.session.as_bytes()))
                }
                df_campaign::service::SessionReading::None => None,
                _ => return Err(PortError::Unavailable),
            };
            campaigns.push(CampaignCard {
                campaign_id: hex(entry.row.id.as_bytes()),
                name: entry.row.name,
                role: match entry.row.role {
                    df_campaign::roles::CampaignRole::Host => CampaignRole::Host,
                    df_campaign::roles::CampaignRole::CoHost => CampaignRole::CoHost,
                    df_campaign::roles::CampaignRole::Player => CampaignRole::Player,
                    df_campaign::roles::CampaignRole::Contributor => CampaignRole::Contributor,
                    df_campaign::roles::CampaignRole::PairedDisplay => {
                        return Err(PortError::Unavailable);
                    }
                },
                host_name: self
                    .service
                    .overview(who.account, entry.row.id)
                    .await
                    .map_err(failure)?
                    .roster
                    .into_iter()
                    .find(|member| member.member.role == df_campaign::roles::CampaignRole::Host)
                    .and_then(|member| member.display_name)
                    .ok_or(PortError::Unavailable)?,
                party_size: u8::try_from(entry.row.seats_in_use)
                    .map_err(|_| PortError::Unavailable)?,
                last_played_at_ms: None,
                last_saved_at_ms: None,
                live_session_id,
                is_archived: entry.row.status == df_campaign::records::CampaignStatus::Archived,
            });
        }
        Ok(DashboardView {
            campaigns,
            can_create: who.is_verified,
        })
    }

    async fn play_sessions(
        &self,
        _account: AccountId,
        _cursor: Option<&str>,
    ) -> PortResult<PlaySessionsView> {
        crate::ports::unavailable("S1")
    }

    async fn funding_targets(&self, _account: AccountId) -> PortResult<Vec<FundingTarget>> {
        crate::ports::unavailable("S1")
    }

    async fn closure_blockers(&self, _account: AccountId) -> PortResult<Vec<String>> {
        crate::ports::unavailable("S1")
    }

    async fn wizard_catalog(&self) -> PortResult<WizardCatalog> {
        crate::ports::unavailable("S1")
    }

    async fn start_draft(&self, who: &Actor) -> PortResult<(String, CampaignDraft)> {
        let draft = self
            .service
            .start_draft(who.account)
            .await
            .map_err(failure)?;
        Ok((
            hex(draft.id.as_bytes()),
            super::campaign_views::draft(draft)?,
        ))
    }

    async fn save_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        step: u8,
        draft: CampaignDraft,
    ) -> PortResult<CampaignDraft> {
        let id = CampaignId::from_hex(draft_id).map_err(|_| PortError::NotFound)?;
        let data = super::campaign_views::step(step, draft)?;
        let saved = self
            .service
            .save_draft_step(who.account, id, data)
            .await
            .map_err(failure)?;
        super::campaign_views::draft(saved)
    }

    async fn finish_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        operation: OperationId,
    ) -> PortResult<CampaignId> {
        let id = CampaignId::from_hex(draft_id).map_err(|_| PortError::NotFound)?;
        self.service
            .finish_draft(who.account, id, operation)
            .await
            .map(|row| row.id)
            .map_err(failure)
    }

    async fn campaign(&self, _who: &Actor, _campaign: CampaignId) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn create_invitation(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _kind: InvitationKind,
        _operation: OperationId,
    ) -> PortResult<InvitationCreated> {
        crate::ports::unavailable("S1")
    }

    async fn revoke_invitation(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _invitation_id: &str,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn rotate_code(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn decide_join_request(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _request_id: &str,
        _decision: RequestDecision,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn leave(&self, _who: &Actor, _campaign: CampaignId) -> PortResult<()> {
        crate::ports::unavailable("S1")
    }

    async fn open_lobby(&self, who: &Actor, campaign: CampaignId) -> PortResult<SessionId> {
        self.service
            .open_lobby(who.account, campaign)
            .await
            .map(|(id, _)| id)
            .map_err(failure)
    }

    async fn lobby(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        let campaign = self.campaign_of(session).await?;
        super::campaign_views::lobby(
            self.service
                .lobby_view(who.account, campaign, session)
                .await
                .map_err(failure)?,
        )
    }

    async fn choose_character(
        &self,
        who: &Actor,
        session: SessionId,
        request: CharacterRequest,
    ) -> PortResult<LobbyView> {
        let campaign = self.campaign_of(session).await?;
        let choice = match request.choice {
            df_account_app::wire_campaign::CharacterChoice::Undecided => {
                df_campaign::lobby::CharacterChoice::Undecided
            }
            df_account_app::wire_campaign::CharacterChoice::Template => {
                df_campaign::lobby::CharacterChoice::Template
            }
            df_account_app::wire_campaign::CharacterChoice::Existing => {
                df_campaign::lobby::CharacterChoice::Existing
            }
        };
        let reference = request
            .reference
            .map(|id| {
                CampaignId::from_hex(&id)
                    .map(|id| *id.as_bytes())
                    .map_err(|_| PortError::NotFound)
            })
            .transpose()?;
        self.service
            .choose_character(who.account, campaign, session, choice, reference)
            .await
            .map_err(failure)?;
        self.lobby(who, session).await
    }

    async fn set_ready(
        &self,
        who: &Actor,
        session: SessionId,
        is_ready: bool,
    ) -> PortResult<LobbyView> {
        let campaign = self.campaign_of(session).await?;
        self.service
            .set_ready(who.account, campaign, session, is_ready)
            .await
            .map_err(failure)?;
        self.lobby(who, session).await
    }

    async fn report_microphone(
        &self,
        who: &Actor,
        session: SessionId,
        report: MicrophoneRequest,
    ) -> PortResult<LobbyView> {
        use df_account_app::wire_campaign::MicrophonePermission;
        use df_campaign::lobby_state::{MicrophonePreference, MicrophoneReport};
        let campaign = self.campaign_of(session).await?;
        self.service
            .set_microphone(
                who.account,
                campaign,
                session,
                if report.prefers_typing {
                    MicrophonePreference::TypedOnly
                } else {
                    MicrophonePreference::Ask
                },
                match report.permission {
                    None => MicrophoneReport::Unknown,
                    Some(MicrophonePermission::Granted) => MicrophoneReport::Granted,
                    Some(MicrophonePermission::Declined) => MicrophoneReport::Declined,
                    Some(MicrophonePermission::Unsupported) => MicrophoneReport::Unsupported,
                },
            )
            .await
            .map_err(failure)?;
        self.lobby(who, session).await
    }

    async fn start_session(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        let campaign = self.campaign_of(session).await?;
        self.service
            .start_session(who.account, campaign, session)
            .await
            .map_err(failure)?;
        self.lobby(who, session).await
    }

    async fn lookup_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _code: &str,
    ) -> PortResult<PairingLookup> {
        crate::ports::unavailable("S1")
    }

    async fn approve_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _pairing_id: &str,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn unpair_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _ticket_id: &str,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn unpair_all_displays(
        &self,
        _who: &Actor,
        _session: SessionId,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn join_preview(&self, _token: &str) -> PortResult<InvitePreview> {
        crate::ports::unavailable("S1")
    }

    async fn join_accept(
        &self,
        _who: &Actor,
        _token: &str,
        _operation: OperationId,
    ) -> PortResult<CampaignId> {
        crate::ports::unavailable("S1")
    }

    async fn code_lookup(&self, _who: &Actor, _code: &str) -> PortResult<InvitePreview> {
        crate::ports::unavailable("S1")
    }

    async fn code_request(
        &self,
        _who: &Actor,
        _code: &str,
        _operation: OperationId,
    ) -> PortResult<JoinRequested> {
        crate::ports::unavailable("S1")
    }

    async fn display_challenge(&self, _device: DeviceLabel) -> PortResult<IssuedChallenge> {
        crate::ports::unavailable("S1")
    }

    async fn display_status(&self, _device_secret: &str) -> PortResult<DisplayState> {
        crate::ports::unavailable("S1")
    }

    async fn display_redeem(&self, _device_secret: &str) -> PortResult<RedeemedTicket> {
        crate::ports::unavailable("S1")
    }

    async fn display_renew(
        &self,
        _device_secret: &str,
        _ticket: &str,
    ) -> PortResult<RenewedTicket> {
        crate::ports::unavailable("S1")
    }
}

fn hex(bytes: &[u8; 16]) -> String {
    df_commerce::finance_ids::hex(bytes)
}
