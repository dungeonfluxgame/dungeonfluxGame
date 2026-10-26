//! JSON shapes shared with S1's campaign API (decision 29; S1 plan sections 7 and 8):
//! dashboard, campaign membership, invitations, joining, lobby and display pairing. S1 owns every
//! endpoint and rule behind them; these types are the proposed
//! contract, served by a fake until S1's `df-campaign-wire` crate lands (TODO(S1): replace this
//! module with that crate's types). Paths live under `/app/api/campaigns`, `/app/api/sessions`,
//! `/app/api/join` and `/app/api/display`.

use serde::{Deserialize, Serialize};

use crate::wire::{PacePreset, PresetOffer};

/// Why a campaign request was refused. It rides beside the envelope's `code` (S1 plan 7) and every
/// invitation refusal omits the preview, so a revoked link tells its holder nothing about the table.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Expired,
    Revoked,
    Exhausted,
    /// Every seat is taken.
    Full,
    ContributorsFull,
    AlreadyJoined,
    /// Unknown, malformed or no longer valid: one answer for all.
    NoLongerValid,
    NotTakingRequests,
    RequestsFull,
    /// A removed person must be allowed back before a request can be approved.
    AllowBackFirst,
    DisplayLimit,
    StaleRevision,
    AccountClosing,
    EmailUnverified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DashboardView {
    pub campaigns: Vec<CampaignCard>,
    /// Creating needs a verified email (S1 decision 12); joining does not.
    pub can_create: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CampaignCard {
    pub campaign_id: String,
    pub name: String,
    pub role: CampaignRole,
    pub host_name: String,
    pub party_size: u8,
    pub last_played_at_ms: Option<u64>,
    pub last_saved_at_ms: Option<u64>,
    /// The campaign's open lobby or running session, if any.
    pub live_session_id: Option<String>,
    pub is_archived: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignRole {
    Host,
    CoHost,
    Player,
    Contributor,
}

/// The four fields an invitation reveals before membership (plan 9.6, S1 8.2).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvitePreview {
    pub campaign_name: String,
    pub host_name: String,
    pub scheduled_at_ms: Option<u64>,
    pub content_rating: ContentRating,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentRating {
    Teen,
    Mature,
}

/// `GET join/pending`: the invitation remembered in the `__Host-df_join` cookie, if it is still usable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PendingInvitation {
    pub preview: Option<InvitePreview>,
}

/// The token travels in the body, never in a path (it came from the URL fragment).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBody {
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinAcceptRequest {
    /// The link's token, or `None` for the invitation remembered in the join cookie.
    pub token: Option<String>,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JoinAccepted {
    pub campaign_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomCodeLookupRequest {
    pub code: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomCodeJoinRequest {
    pub code: String,
    pub operation: String,
}

/// A room code always ends in approval by the host (S1 8.3).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JoinRequested {
    pub campaign_name: String,
}

/// Defaults and preset offers used by server-owned campaign creation (S1 8.1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WizardCatalog {
    pub adventures: Vec<AdventureOption>,
    pub maximum_seats: u8,
    pub presets: Vec<PresetOffer>,
    pub is_proposed_pricing: bool,
    pub custom_minimum_cents_per_hour: u32,
    pub custom_maximum_cents_per_hour: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdventureOption {
    pub adventure_id: String,
    pub name: String,
    pub summary: String,
    pub ratings: Vec<ContentRating>,
    pub rules_text: String,
}

/// Campaign defaults and business settings persisted by the campaign service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignDraft {
    pub name: String,
    pub adventure_id: Option<String>,
    pub content_rating: Option<ContentRating>,
    pub seat_count: u8,
    pub hero_choice: HeroChoice,
    pub has_voice: bool,
    pub pictures: PictureDefault,
    pub pace: PacePreset,
    pub custom_ceiling_cents_per_hour: Option<u32>,
    /// The three delegations new co-hosts start with (S1 4.1).
    pub co_hosts_can_invite: bool,
    pub co_hosts_can_start: bool,
    pub co_hosts_can_export: bool,
    pub join_mode: JoinMode,
    pub funding: FundingModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeroChoice {
    /// Choose the hero in the lobby; picking a saved template is O2's (TODO(O2)).
    InLobby,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureDefault {
    Automatic,
    ExistingArtOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinMode {
    LinkOnly,
    LinkAndRoomCode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingModel {
    Sponsor,
    SharedPot,
}

/// `PUT campaigns/drafts/:draft/steps/:n` carries the whole draft; the step is in the path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDraftRequest {
    pub draft: CampaignDraft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRequest {
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CampaignOverview {
    pub campaign_id: String,
    pub name: String,
    pub my_role: CampaignRole,
    pub can_invite: bool,
    pub can_open_lobby: bool,
    pub members: Vec<MemberRow>,
    pub invitations: Vec<InvitationRow>,
    pub join_requests: Vec<JoinRequestRow>,
    pub room_code: RoomCodeState,
    pub live_session_id: Option<String>,
    pub last_saved_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum RoomCodeState {
    /// The campaign does not take join requests by code.
    Off,
    Active {
        code: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemberRow {
    pub display_name: String,
    pub role: CampaignRole,
    pub is_me: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvitationRow {
    pub invitation_id: String,
    pub kind: InvitationKind,
    pub uses: u32,
    pub maximum_uses: u32,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationKind {
    Player,
    /// Joins without a seat, sees pot totals and funds the table (S1 8.2).
    Contributor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JoinRequestRow {
    pub request_id: String,
    pub display_name: String,
    pub requested_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitationRequest {
    pub kind: InvitationKind,
    pub operation: String,
}

/// The link is shown once; the server keeps only its digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvitationCreated {
    pub invitation_id: String,
    pub link: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveRequestBody {
    pub role: InvitationKind,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LobbyOpened {
    pub session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LobbyView {
    pub session_id: String,
    pub campaign_id: String,
    pub campaign_name: String,
    /// Host, or co-host with the start delegation: may start, pair and unpair displays.
    pub can_manage: bool,
    pub start_blockers: Vec<StartBlocker>,
    pub members: Vec<LobbyMember>,
    pub funding_note: FundingNote,
    pub has_started: bool,
    /// How often the screen reads the lobby again (S1 `lobby_poll_seconds`).
    pub poll_seconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LobbyMember {
    pub display_name: String,
    pub is_me: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingNote {
    NoChargeUntilHostStarts,
    WaitingForSponsor,
    HostCredit(HostCreditView),
}

/// Finance's customer-safe host quote; amounts are exact USD micro-dollars.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostCreditView {
    pub price_version: [u8; 16],
    pub hourly_micro_dollars: u64,
    pub available_micro_dollars: u64,
    pub cap_micro_dollars: u64,
    pub terms_version: i32,
    pub consented_at_ms: Option<i64>,
    pub is_fundable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConsentRequest {
    pub price_version: [u8; 16],
    pub cap_micro_dollars: u64,
    pub terms_version: i32,
    pub operation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartBlocker {
    NoSeatedMembers,
    FundingNotReady,
    SessionNotInLobby,
    CampaignArchived,
}

/// The account's hand-off for a session: whether it has started and, once its game is reachable,
/// the game launcher's address. The launcher (the desktop launcher's screen) offers the table
/// screen, the players' join and the room code; the account never does (planning/client-architecture.md
/// "Game start flow").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GameLaunchView {
    pub session_id: String,
    pub campaign_name: String,
    pub has_started: bool,
    pub can_manage: bool,
    pub launcher_url: Option<String>,
    pub poll_seconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GameLinks {
    pub display_url: String,
    pub player_url: String,
    pub room_code: Option<String>,
}

/// Defaults are chosen by the campaign owner; funding may still need the host's consent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlayNowView {
    pub campaign_id: String,
    pub game_unavailable: bool,
    pub lobby: LobbyView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CampaignSettingsView {
    pub draft: CampaignDraft,
    pub catalog: WizardCatalog,
    pub can_edit: bool,
}
