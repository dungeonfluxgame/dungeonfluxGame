//! Campaign screens over S1's JSON API (decision 29): the create wizard, a campaign's members and
//! invitations, joining by link or room code, and the lobby with display pairing (plan 9.4-9.6;
//! S1 plan 8.1-8.5). S1 enforces every rule; the app shows what S1 returns.

use super::{
    Effect, Field, FocusTarget, Form, History, Intent, Model, OperationSlot, Remote, Screen, Submit,
};
use crate::{
    api::{ApiCall, JoinDecision, Reply},
    money::parse_dollars,
    route::Route,
    wire::{ApiError, PacePreset, Refusal},
    wire_campaign::{
        ApproveRequestBody, CampaignDraft, CampaignOverview, ContentRating,
        CreateInvitationRequest, FundingModel, HeroChoice, InvitationCreated, InvitationKind,
        InvitePreview, JoinAcceptRequest, JoinMode, JoinRequested, LobbyView, MicrophonePermission,
        MicrophoneRequest, OperationRequest, PairingApproveRequest, PairingLookup,
        PairingLookupRequest, PictureDefault, ReadyRequest, RoomCodeJoinRequest,
        RoomCodeLookupRequest, SaveDraftRequest, TokenBody, WizardCatalog,
    },
};

/// The wizard's steps (plan 9.5): basics, heroes, look and pace, joining and funding, summary.
pub const WIZARD_STEPS: u8 = 5;
/// Longest campaign name the wizard accepts before S1 checks it again.
pub const MAXIMUM_CAMPAIGN_NAME_CHARS: usize = 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WizardScreen {
    pub catalog: Remote<WizardCatalog>,
    pub draft_id: Remote<String>,
    pub step: u8,
    pub draft: CampaignDraft,
    pub custom_ceiling: String,
    pub operation: Remote<String>,
    pub save: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignScreen {
    pub campaign_id: String,
    pub overview: Remote<CampaignOverview>,
    pub operation: Remote<String>,
    pub action: Submit,
    pub created_link: Option<InvitationCreated>,
    pub is_confirming_leave: bool,
}

/// A host's answer to a room-code request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDecision {
    Approve(InvitationKind),
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinScreen {
    /// The invitation link's token, held in memory only; it was removed from the address bar.
    pub token: Option<String>,
    /// What the link's preview said (present only when the page opened with a token).
    pub preview: Option<Remote<InvitePreview>>,
    pub code: String,
    /// What a typed room code resolved to.
    pub code_preview: Option<InvitePreview>,
    pub operation: Remote<String>,
    pub submit: Submit,
    /// Where to go once the server remembered the invitation (sign in or sign up).
    pub after_hold: Option<Route>,
    /// The table whose host was asked to approve this person.
    pub requested: Option<JoinRequested>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LobbyScreen {
    pub session_id: String,
    pub lobby: Remote<LobbyView>,
    pub operation: Remote<String>,
    pub action: Submit,
    pub pairing_code: String,
    pub lookup: Option<PairingLookup>,
}

pub(super) fn empty_draft() -> CampaignDraft {
    CampaignDraft {
        name: String::new(),
        adventure_id: None,
        content_rating: None,
        seat_count: 4,
        hero_choice: HeroChoice::InLobby,
        has_voice: true,
        pictures: PictureDefault::Automatic,
        pace: PacePreset::Immersive,
        custom_ceiling_cents_per_hour: None,
        co_hosts_can_invite: true,
        co_hosts_can_start: false,
        co_hosts_can_export: false,
        join_mode: JoinMode::LinkOnly,
        funding: FundingModel::Sponsor,
    }
}

fn rating_from_name(name: &str) -> Option<ContentRating> {
    [ContentRating::Teen, ContentRating::Mature]
        .into_iter()
        .find(|rating| crate::copy::rating_value(*rating) == name)
}

impl WizardScreen {
    /// What blocks leaving the current step, checked before S1 checks it again.
    fn step_refusal(&self) -> Option<Refusal> {
        let draft = &self.draft;
        let catalog = self.catalog.ready()?;
        match self.step {
            1 if draft.name.trim().is_empty() => Some(Refusal::CampaignNameRequired),
            1 if draft.name.trim().chars().count() > MAXIMUM_CAMPAIGN_NAME_CHARS => {
                Some(Refusal::CampaignNameTooLong)
            }
            1 if draft.adventure_id.is_none() => Some(Refusal::AdventureRequired),
            1 if draft.content_rating.is_none() => Some(Refusal::ContentRatingRequired),
            1 if draft.seat_count == 0 || draft.seat_count > catalog.maximum_seats => {
                Some(Refusal::SeatsOutOfRange)
            }
            3 if draft.pace == PacePreset::Custom => {
                let range =
                    catalog.custom_minimum_cents_per_hour..=catalog.custom_maximum_cents_per_hour;
                match draft.custom_ceiling_cents_per_hour {
                    Some(cents) if range.contains(&cents) => None,
                    _ => Some(Refusal::CeilingOutOfRange),
                }
            }
            _ => None,
        }
    }
}

impl Model {
    pub(super) fn enter_campaign(&mut self, route: Route) -> Vec<Effect> {
        match route {
            Route::NewCampaign => {
                let is_verified = self.account().is_some_and(|account| account.is_verified);
                self.screen = Screen::NewCampaign(WizardScreen {
                    catalog: Remote::Loading,
                    draft_id: Remote::Loading,
                    step: 1,
                    draft: empty_draft(),
                    custom_ceiling: String::new(),
                    operation: Remote::Loading,
                    save: Submit::Idle,
                });
                // An unverified account cannot start a draft (S1 decision 12); the screen says so.
                if !is_verified {
                    return Vec::new();
                }
                vec![
                    self.api(ApiCall::WizardCatalog),
                    self.api(ApiCall::StartDraft),
                    self.issue_operation(OperationSlot::Primary),
                ]
            }
            Route::Campaign { campaign_id } => {
                self.screen = Screen::Campaign(CampaignScreen {
                    campaign_id: campaign_id.clone(),
                    overview: Remote::Loading,
                    operation: Remote::Loading,
                    action: Submit::Idle,
                    created_link: None,
                    is_confirming_leave: false,
                });
                vec![
                    self.api(ApiCall::Campaign { campaign_id }),
                    self.issue_operation(OperationSlot::Primary),
                ]
            }
            Route::Lobby { session_id } => {
                self.screen = Screen::Lobby(LobbyScreen {
                    session_id: session_id.clone(),
                    lobby: Remote::Loading,
                    operation: Remote::Loading,
                    action: Submit::Idle,
                    pairing_code: String::new(),
                    lookup: None,
                });
                vec![
                    self.api(ApiCall::Lobby { session_id }),
                    self.issue_operation(OperationSlot::Primary),
                ]
            }
            Route::Join { token } => {
                let mut effects = Vec::new();
                if let Some(token) = &token {
                    effects.push(self.api(ApiCall::JoinPreview(TokenBody {
                        token: token.clone(),
                    })));
                }
                if self.account().is_some() {
                    effects.push(self.issue_operation(OperationSlot::Primary));
                }
                self.screen = Screen::Join(JoinScreen {
                    preview: token.is_some().then_some(Remote::Loading),
                    token,
                    code: String::new(),
                    code_preview: None,
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                    after_hold: None,
                    requested: None,
                });
                effects
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn campaign_intent(&mut self, intent: Intent) -> Vec<Effect> {
        match &self.screen {
            Screen::Join(_) => self.join_intent(intent),
            Screen::Lobby(_) => self.lobby_intent(intent),
            Screen::Campaign(_) => self.overview_intent(intent),
            Screen::NewCampaign(_) => self.wizard_intent(intent),
            Screen::Dashboard(_) => match intent {
                Intent::AcceptPendingInvitation => self.accept_pending_invitation(),
                Intent::CopyText(text) => vec![Effect::CopyText(text)],
                _ => Vec::new(),
            },
            _ => match intent {
                Intent::CopyText(text) => vec![Effect::CopyText(text)],
                _ => Vec::new(),
            },
        }
    }

    fn wizard_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let Screen::NewCampaign(screen) = &mut self.screen else {
            return Vec::new();
        };
        match intent {
            Intent::WizardBack => {
                screen.step = screen.step.saturating_sub(1).max(1);
                screen.save = Submit::Idle;
                vec![Effect::Focus(FocusTarget::Heading)]
            }
            _ => Vec::new(),
        }
    }

    fn accept_pending_invitation(&mut self) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Dashboard(screen) = &mut self.screen else {
            return Vec::new();
        };
        let Some(operation) = screen.join_operation.ready().cloned() else {
            return Vec::new();
        };
        if screen.join.is_sending() {
            return Vec::new();
        }
        screen.join = Submit::Sending;
        vec![Effect::Api {
            generation,
            call: ApiCall::JoinAccept(JoinAcceptRequest {
                token: None,
                operation,
            }),
        }]
    }

    fn join_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let generation = self.generation;
        let is_signed_in = self.account().is_some();
        let Screen::Join(screen) = &mut self.screen else {
            return Vec::new();
        };
        let call = match intent {
            Intent::AcceptInvitation if is_signed_in => {
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.submit)
                else {
                    return Vec::new();
                };
                ApiCall::JoinAccept(JoinAcceptRequest {
                    token: screen.token.clone(),
                    operation,
                })
            }
            Intent::HoldInvitation { then } if !is_signed_in => {
                let Some(token) = screen.token.clone() else {
                    return Vec::new();
                };
                if screen.submit.is_sending() {
                    return Vec::new();
                }
                screen.submit = Submit::Sending;
                screen.after_hold = Some(then);
                ApiCall::JoinHold(TokenBody { token })
            }
            Intent::AskToJoin if is_signed_in => {
                if screen.code_preview.is_none() {
                    return Vec::new();
                }
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.submit)
                else {
                    return Vec::new();
                };
                ApiCall::CodeRequest(RoomCodeJoinRequest {
                    code: screen.code.trim().to_owned(),
                    operation,
                })
            }
            Intent::ClearCodeLookup => {
                screen.code_preview = None;
                screen.submit = Submit::Idle;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        vec![Effect::Api { generation, call }]
    }

    fn overview_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Campaign(screen) = &mut self.screen else {
            return Vec::new();
        };
        let campaign_id = screen.campaign_id.clone();
        let call = match intent {
            Intent::CopyText(text) => return vec![Effect::CopyText(text)],
            Intent::AskLeave => {
                screen.is_confirming_leave = true;
                return Vec::new();
            }
            Intent::CancelEdit => {
                screen.is_confirming_leave = false;
                return Vec::new();
            }
            intent => {
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.action)
                else {
                    return Vec::new();
                };
                match intent {
                    Intent::CreateInvitation(kind) => ApiCall::CreateInvitation {
                        campaign_id,
                        request: CreateInvitationRequest { kind, operation },
                    },
                    Intent::RevokeInvitation { invitation_id } => ApiCall::RevokeInvitation {
                        campaign_id,
                        invitation_id,
                        request: OperationRequest { operation },
                    },
                    Intent::RotateCode => ApiCall::RotateCode {
                        campaign_id,
                        request: OperationRequest { operation },
                    },
                    Intent::DecideRequest {
                        request_id,
                        decision,
                    } => ApiCall::DecideJoinRequest {
                        campaign_id,
                        request_id,
                        decision: match decision {
                            RequestDecision::Approve(role) => {
                                JoinDecision::Approve(ApproveRequestBody { role, operation })
                            }
                            RequestDecision::Deny => {
                                JoinDecision::Deny(OperationRequest { operation })
                            }
                        },
                    },
                    Intent::LeaveCampaign => ApiCall::LeaveCampaign {
                        campaign_id,
                        request: OperationRequest { operation },
                    },
                    Intent::OpenLobby => ApiCall::OpenLobby {
                        campaign_id,
                        request: OperationRequest { operation },
                    },
                    _ => {
                        // Not an action of this screen: give the id back.
                        screen.operation = Remote::Ready(operation);
                        screen.action = Submit::Idle;
                        return Vec::new();
                    }
                }
            }
        };
        vec![Effect::Api { generation, call }]
    }

    fn lobby_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Lobby(screen) = &mut self.screen else {
            return Vec::new();
        };
        let session_id = screen.session_id.clone();
        let call = match intent {
            Intent::CopyText(text) => return vec![Effect::CopyText(text)],
            Intent::CancelPairing => {
                screen.lookup = None;
                screen.pairing_code.clear();
                return Vec::new();
            }
            Intent::EnableMicrophone => {
                if screen.action.is_sending() || screen.operation.ready().is_none() {
                    return Vec::new();
                }
                return vec![Effect::RequestMicrophone { generation }];
            }
            Intent::ApprovePairing => {
                let Some(PairingLookup::Found { pairing_id, .. }) = screen.lookup.clone() else {
                    return Vec::new();
                };
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.action)
                else {
                    return Vec::new();
                };
                ApiCall::ApproveDisplay {
                    session_id,
                    request: PairingApproveRequest {
                        pairing_id,
                        operation,
                    },
                }
            }
            intent => {
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.action)
                else {
                    return Vec::new();
                };
                match intent {
                    Intent::SetReady(is_ready) => ApiCall::SetReady {
                        session_id,
                        is_ready,
                        request: ReadyRequest { operation },
                    },
                    Intent::PreferTyping => ApiCall::ReportMicrophone {
                        session_id,
                        request: MicrophoneRequest {
                            prefers_typing: true,
                            permission: None,
                            operation,
                        },
                    },
                    Intent::StartSession => ApiCall::StartSession {
                        session_id,
                        request: OperationRequest { operation },
                    },
                    Intent::UnpairDisplay { ticket_id } => ApiCall::UnpairDisplay {
                        session_id,
                        ticket_id,
                        request: OperationRequest { operation },
                    },
                    Intent::UnpairAllDisplays => ApiCall::UnpairAllDisplays {
                        session_id,
                        request: OperationRequest { operation },
                    },
                    _ => {
                        screen.operation = Remote::Ready(operation);
                        screen.action = Submit::Idle;
                        return Vec::new();
                    }
                }
            }
        };
        vec![Effect::Api { generation, call }]
    }

    /// The browser answered the microphone request: tell S1 what it said.
    pub(super) fn microphone_answered(&mut self, permission: MicrophonePermission) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Lobby(screen) = &mut self.screen else {
            return Vec::new();
        };
        let Some(operation) = take_operation(&mut screen.operation, &mut screen.action) else {
            return Vec::new();
        };
        vec![Effect::Api {
            generation,
            call: ApiCall::ReportMicrophone {
                session_id: screen.session_id.clone(),
                request: MicrophoneRequest {
                    prefers_typing: false,
                    permission: Some(permission),
                    operation,
                },
            },
        }]
    }

    pub(super) fn edit_campaign(&mut self, field: Field, value: &str) -> bool {
        match (&mut self.screen, field) {
            (Screen::NewCampaign(screen), field) => {
                let draft = &mut screen.draft;
                match field {
                    Field::CampaignName => value.clone_into(&mut draft.name),
                    Field::Adventure => {
                        draft.adventure_id = Some(value.to_owned());
                        // A rating the new adventure does not allow must not stay in the draft.
                        let allowed = screen.catalog.ready().and_then(|catalog| {
                            catalog
                                .adventures
                                .iter()
                                .find(|adventure| adventure.adventure_id == value)
                        });
                        if let (Some(adventure), Some(rating)) = (allowed, draft.content_rating)
                            && !adventure.ratings.contains(&rating)
                        {
                            draft.content_rating = None;
                        }
                    }
                    Field::Rating => draft.content_rating = rating_from_name(value),
                    Field::SeatCount => {
                        if let Ok(seats) = value.parse() {
                            draft.seat_count = seats;
                        }
                    }
                    Field::Pictures => {
                        draft.pictures = if value == "existing" {
                            PictureDefault::ExistingArtOnly
                        } else {
                            PictureDefault::Automatic
                        };
                    }
                    Field::PacePreset => {
                        if let Some(preset) = super::customer_flow::preset_from_name(value) {
                            draft.pace = preset;
                        }
                    }
                    Field::CustomCeiling => {
                        value.clone_into(&mut screen.custom_ceiling);
                        draft.custom_ceiling_cents_per_hour = parse_dollars(value).ok();
                    }
                    Field::JoinModeChoice => {
                        draft.join_mode = if value == "code" {
                            JoinMode::LinkAndRoomCode
                        } else {
                            JoinMode::LinkOnly
                        };
                    }
                    Field::FundingChoice => {
                        draft.funding = if value == "shared" {
                            FundingModel::SharedPot
                        } else {
                            FundingModel::Sponsor
                        };
                    }
                    _ => return false,
                }
                screen.save = Submit::Idle;
            }
            (Screen::Join(screen), Field::RoomCode) => value.clone_into(&mut screen.code),
            (Screen::Lobby(screen), Field::PairingCode) => {
                value.clone_into(&mut screen.pairing_code);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn toggle_campaign(&mut self, field: Field, is_on: bool) {
        if let Screen::NewCampaign(screen) = &mut self.screen {
            match field {
                Field::HasVoice => screen.draft.has_voice = is_on,
                Field::CoHostsCanInvite => screen.draft.co_hosts_can_invite = is_on,
                Field::CoHostsCanStart => screen.draft.co_hosts_can_start = is_on,
                Field::CoHostsCanExport => screen.draft.co_hosts_can_export = is_on,
                _ => {}
            }
        }
    }

    pub(super) fn submit_campaign(&mut self, form: Form) -> Vec<Effect> {
        let generation = self.generation;
        let is_signed_in = self.account().is_some();
        let call = match (&mut self.screen, form) {
            (Screen::NewCampaign(screen), Form::WizardStep) => {
                let Some(draft_id) = screen.draft_id.ready().cloned() else {
                    return Vec::new();
                };
                if screen.save.is_sending() {
                    return Vec::new();
                }
                if let Some(refusal) = screen.step_refusal() {
                    screen.save = Submit::Refused(ApiError::refused(refusal));
                    return vec![Effect::Focus(FocusTarget::ErrorSummary)];
                }
                screen.save = Submit::Sending;
                ApiCall::SaveDraft {
                    draft_id,
                    step: screen.step,
                    request: SaveDraftRequest {
                        draft: screen.draft.clone(),
                    },
                }
            }
            (Screen::NewCampaign(screen), Form::CreateCampaign) => {
                let Some(draft_id) = screen.draft_id.ready().cloned() else {
                    return Vec::new();
                };
                let Some(operation) = take_operation(&mut screen.operation, &mut screen.save)
                else {
                    return Vec::new();
                };
                ApiCall::FinishDraft {
                    draft_id,
                    request: OperationRequest { operation },
                }
            }
            (Screen::Join(screen), Form::CodeLookup) if is_signed_in => {
                if screen.code.trim().is_empty() {
                    screen.submit = Submit::Refused(ApiError::refused(Refusal::CodeRequired));
                    return vec![Effect::Focus(FocusTarget::ErrorSummary)];
                }
                if screen.submit.is_sending() {
                    return Vec::new();
                }
                screen.submit = Submit::Sending;
                ApiCall::CodeLookup(RoomCodeLookupRequest {
                    code: screen.code.trim().to_owned(),
                })
            }
            (Screen::Lobby(screen), Form::PairingLookup) => {
                if screen.pairing_code.trim().is_empty() {
                    screen.action = Submit::Refused(ApiError::refused(Refusal::CodeRequired));
                    return vec![Effect::Focus(FocusTarget::ErrorSummary)];
                }
                if screen.action.is_sending() {
                    return Vec::new();
                }
                screen.action = Submit::Sending;
                ApiCall::LookupDisplay {
                    session_id: screen.session_id.clone(),
                    request: PairingLookupRequest {
                        code: screen.pairing_code.trim().to_owned(),
                    },
                }
            }
            (_, form) => return self.submit_admin(form),
        };
        vec![Effect::Api { generation, call }]
    }

    pub(super) fn campaign_reply(&mut self, reply: Reply) -> Vec<Effect> {
        match (reply, &self.screen) {
            (
                reply @ (Reply::WizardCatalog(_) | Reply::Draft(_) | Reply::CampaignCreated(_)),
                _,
            ) => self.wizard_reply(reply),
            (
                reply @ (Reply::Campaign(_)
                | Reply::InvitationCreated(_)
                | Reply::Left(_)
                | Reply::LobbyOpened(_)),
                _,
            ) => self.overview_reply(reply),
            (reply @ (Reply::Lobby(_) | Reply::PairingLookup(_)), _) => self.lobby_reply(reply),
            (
                reply @ (Reply::JoinPreview(_)
                | Reply::JoinHeld(_)
                | Reply::Joined(_)
                | Reply::CodeRequested(_)
                | Reply::JoinPending(_)),
                _,
            ) => self.join_reply(reply),
            _ => Vec::new(),
        }
    }

    fn wizard_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let Screen::NewCampaign(screen) = &mut self.screen else {
            return Vec::new();
        };
        match reply {
            Reply::WizardCatalog(result) => {
                screen.catalog = Remote::from_result(result);
                Vec::new()
            }
            Reply::Draft(result) => match result {
                Ok(view) if screen.draft_id.ready().is_none() => {
                    // A draft in progress resumes where it was left (plan 9.5).
                    if view.step > 1 {
                        screen.step = view.step.min(WIZARD_STEPS);
                        screen.draft = view.draft;
                        screen.custom_ceiling = screen
                            .draft
                            .custom_ceiling_cents_per_hour
                            .map(|cents| {
                                crate::money::cents(u64::from(cents))
                                    .trim_start_matches('$')
                                    .to_owned()
                            })
                            .unwrap_or_default();
                    }
                    screen.draft_id = Remote::Ready(view.draft_id);
                    Vec::new()
                }
                Ok(_) => {
                    screen.save = Submit::Idle;
                    screen.step = (screen.step + 1).min(WIZARD_STEPS);
                    vec![Effect::Focus(FocusTarget::Heading)]
                }
                Err(error) if screen.draft_id.ready().is_none() => {
                    screen.draft_id = Remote::Failed(error);
                    Vec::new()
                }
                Err(error) => refused(&mut screen.save, error),
            },
            Reply::CampaignCreated(result) => match result {
                Ok(created) => self.navigate(
                    Route::Campaign {
                        campaign_id: created.campaign_id,
                    },
                    History::Push,
                ),
                Err(error) => self.refused_and_reissue(error),
            },
            _ => Vec::new(),
        }
    }

    fn overview_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let Screen::Campaign(screen) = &mut self.screen else {
            return Vec::new();
        };
        match reply {
            Reply::Campaign(result) => {
                let was_action = screen.action.is_sending();
                match result {
                    Ok(overview) => {
                        screen.overview = Remote::Ready(overview);
                        screen.action = Submit::Idle;
                        screen.is_confirming_leave = false;
                    }
                    Err(error) if was_action => return self.refused_and_reissue(error),
                    Err(error) => screen.overview = Remote::Failed(error),
                }
                if was_action {
                    return vec![self.issue_operation(OperationSlot::Primary)];
                }
                Vec::new()
            }
            Reply::InvitationCreated(result) => match result {
                Ok(created) => {
                    screen.created_link = Some(created);
                    screen.action = Submit::Idle;
                    let campaign_id = screen.campaign_id.clone();
                    vec![
                        self.issue_operation(OperationSlot::Primary),
                        self.api(ApiCall::Campaign { campaign_id }),
                    ]
                }
                Err(error) => self.refused_and_reissue(error),
            },
            Reply::Left(result) => match result {
                Ok(_) => self.navigate(Route::Dashboard, History::Push),
                Err(error) => {
                    screen.is_confirming_leave = false;
                    self.refused_and_reissue(error)
                }
            },
            Reply::LobbyOpened(result) => match result {
                Ok(opened) => self.navigate(
                    Route::Lobby {
                        session_id: opened.session_id,
                    },
                    History::Push,
                ),
                Err(error) => self.refused_and_reissue(error),
            },
            _ => Vec::new(),
        }
    }

    fn lobby_reply(&mut self, reply: Reply) -> Vec<Effect> {
        let generation = self.generation;
        let Screen::Lobby(screen) = &mut self.screen else {
            return Vec::new();
        };
        match reply {
            Reply::Lobby(result) => {
                let was_action = screen.action.is_sending();
                let mut effects = Vec::new();
                match result {
                    Ok(lobby) => {
                        if was_action {
                            screen.lookup = None;
                            screen.pairing_code.clear();
                            screen.action = Submit::Idle;
                            effects.push(Effect::IssueOperation {
                                generation,
                                slot: OperationSlot::Primary,
                            });
                        }
                        let poll_ms = lobby.poll_seconds.saturating_mul(1_000).max(1_000);
                        let is_open = !lobby.has_started;
                        screen.lobby = Remote::Ready(lobby);
                        if is_open && !was_action {
                            effects.push(Effect::After {
                                generation,
                                delay_ms: poll_ms,
                            });
                        }
                    }
                    Err(error) if was_action => return self.refused_and_reissue(error),
                    Err(error) => screen.lobby = Remote::Failed(error),
                }
                effects
            }
            Reply::PairingLookup(result) => match result {
                Ok(lookup) => {
                    screen.action = Submit::Idle;
                    screen.lookup = Some(lookup);
                    vec![Effect::Focus(FocusTarget::Heading)]
                }
                Err(error) => refused(&mut screen.action, error),
            },
            _ => Vec::new(),
        }
    }

    fn join_reply(&mut self, reply: Reply) -> Vec<Effect> {
        match (reply, &mut self.screen) {
            (Reply::JoinPreview(result), Screen::Join(screen)) => {
                if screen.token.is_some() {
                    screen.preview = Some(Remote::from_result(result));
                    return Vec::new();
                }
                match result {
                    Ok(preview) => {
                        screen.submit = Submit::Idle;
                        screen.code_preview = Some(preview);
                        vec![Effect::Focus(FocusTarget::Heading)]
                    }
                    Err(error) => refused(&mut screen.submit, error),
                }
            }
            (Reply::JoinHeld(result), Screen::Join(screen)) => match result {
                Ok(_) => {
                    screen.submit = Submit::Idle;
                    let then = screen.after_hold.take().unwrap_or(Route::SignIn);
                    // The token has done its job: the server remembers the invitation now.
                    screen.token = None;
                    self.navigate(then, History::Push)
                }
                Err(error) => refused(&mut screen.submit, error),
            },
            (Reply::CodeRequested(result), Screen::Join(screen)) => match result {
                Ok(requested) => {
                    screen.submit = Submit::Idle;
                    screen.requested = Some(requested);
                    screen.code_preview = None;
                    vec![
                        self.issue_operation(OperationSlot::Primary),
                        Effect::Focus(FocusTarget::Heading),
                    ]
                }
                Err(error) => self.refused_and_reissue(error),
            },
            (Reply::Joined(result), Screen::Join(_)) => match result {
                Ok(accepted) => self.navigate(
                    Route::Campaign {
                        campaign_id: accepted.campaign_id,
                    },
                    History::Push,
                ),
                Err(error) => self.refused_and_reissue(error),
            },
            (Reply::Joined(result), Screen::Dashboard(screen)) => match result {
                Ok(accepted) => self.navigate(
                    Route::Campaign {
                        campaign_id: accepted.campaign_id,
                    },
                    History::Push,
                ),
                Err(error) => {
                    screen.join = Submit::Refused(error);
                    vec![
                        self.issue_operation(OperationSlot::Secondary),
                        Effect::Focus(FocusTarget::ErrorSummary),
                    ]
                }
            },
            (Reply::JoinPending(result), Screen::Dashboard(screen)) => {
                let has_invitation = result
                    .as_ref()
                    .is_ok_and(|pending| pending.preview.is_some());
                screen.pending = Remote::from_result(result);
                if has_invitation {
                    vec![self.issue_operation(OperationSlot::Secondary)]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// A refused mutation spent its operation id: mark the screen's form refused and ask for a
    /// new id so the person can send it again.
    fn refused_and_reissue(&mut self, error: ApiError) -> Vec<Effect> {
        let submit = match &mut self.screen {
            Screen::NewCampaign(screen) => &mut screen.save,
            Screen::Campaign(screen) => &mut screen.action,
            Screen::Lobby(screen) => &mut screen.action,
            Screen::Join(screen) => &mut screen.submit,
            _ => return Vec::new(),
        };
        *submit = Submit::Refused(error);
        vec![
            self.issue_operation(OperationSlot::Primary),
            Effect::Focus(FocusTarget::ErrorSummary),
        ]
    }

    pub(super) fn refresh_lobby(&mut self) -> Vec<Effect> {
        let Screen::Lobby(screen) = &self.screen else {
            return Vec::new();
        };
        vec![self.api(ApiCall::Lobby {
            session_id: screen.session_id.clone(),
        })]
    }
}

/// Takes the screen's operation id for one mutation and marks the screen sending; the next
/// mutation waits for a new id (each id is used once).
fn take_operation(operation: &mut Remote<String>, submit: &mut Submit) -> Option<String> {
    if submit.is_sending() || operation.ready().is_none() {
        return None;
    }
    let Remote::Ready(id) = std::mem::replace(operation, Remote::Loading) else {
        return None;
    };
    *submit = Submit::Sending;
    Some(id)
}

fn refused(submit: &mut Submit, error: ApiError) -> Vec<Effect> {
    *submit = Submit::Refused(error);
    vec![Effect::Focus(FocusTarget::ErrorSummary)]
}
