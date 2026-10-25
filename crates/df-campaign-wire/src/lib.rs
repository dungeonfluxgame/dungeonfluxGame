//! The JSON contract of the campaign API (`planning/campaigns-and-table-research.md` section 7).
//! Pure serde types, shared by the server and by P's wasm app; no I/O, no server types. Nothing
//! here grants anything: the server decides every permission.
//!
//! Customer-visible types carry no fee, multiplier or provider price (launch decision 26). Ids
//! are 32 lowercase hexadecimal digits; times are milliseconds since the Unix epoch.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// The content rating a campaign was created with.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentRating {
    Gentle,
    Standard,
    Mature,
}

/// The coarse device class of a display, from the same classification as web sessions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLabel {
    Phone,
    Tablet,
    Desktop,
    Unknown,
}

impl DeviceLabel {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Desktop => "desktop",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "phone" => Some(Self::Phone),
            "tablet" => Some(Self::Tablet),
            "desktop" => Some(Self::Desktop),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// The role an invitation or an approval admits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinsAs {
    Player,
    Contributor,
}

/// The closed set of reasons an endpoint can give beside P's envelope `code`. Every failure page
/// of the join flow is one of these and shows nothing else about the campaign.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    InvitationExpired,
    InvitationRevoked,
    InvitationExhausted,
    /// An unknown token and a malformed one are the same response.
    InvitationUnknown,
    Full,
    ContributorsFull,
    AlreadyJoined,
    NoLongerValid,
    CampaignUnavailable,
    AccountClosing,
    AllowBackFirst,
    RequestsFull,
    CodeUnknown,
    StaleRevision,
    SessionInProgress,
    SessionEnded,
    HostAccountNotActive,
    HostMustTransferFirst,
    OwnerMustTransferFirst,
    CannotRemoveHost,
    CannotRemoveOwner,
    TargetOutranks,
    NotAllowed,
    ConfirmationRequired,
    DisplayLimit,
    DisplayNeutral,
    DeletionScheduled,
    AlreadyDeleting,
    ServiceHalting,
    OperationConflict,
    Throttled,
    NotFound,
    CsrfRefused,
    Unauthenticated,
    Unavailable,
    OutcomeUnknown,
    InvalidInput,
    OperationExpired,
    ProposalExpired,
    OwnerAccountNotActive,
    TooManyCampaigns,
    NotSeated,
    ContributorCannotBeCoHost,
    TransferTargetNotAllowed,
    SeatsFull,
}

/// What an invitation preview shows: exactly these four fields, and only for a usable invitation.
/// A fifth field must not be added without changing the privacy rule of plan 9.6.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InvitePreview {
    pub campaign_name: String,
    pub host_display_name: String,
    pub scheduled_at_ms: Option<u64>,
    pub content_rating: ContentRating,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReasonBody {
    pub reason: Reason,
}

/// A token or code travels in a request body, never in a URL.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JoinTokenBody {
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JoinPreviewResponse {
    pub preview: InvitePreview,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JoinAcceptBody {
    /// Absent means "the token held in the pending-join cookie".
    pub token: Option<String>,
    pub operation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JoinAcceptResponse {
    pub campaign_id: String,
    pub member_id: String,
    pub rejoined: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeBody {
    pub code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeLookupResponse {
    pub preview: InvitePreview,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeRequestResponse {
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InvitationCreateBody {
    pub joins_as: JoinsAs,
    pub lifetime_hours: Option<u32>,
    pub max_uses: Option<u32>,
}

/// The token is shown once; only its digest is stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InvitationCreated {
    pub invitation_id: String,
    pub token: String,
    /// The path and fragment to append to the origin: `/app/join#token=<token>`.
    pub link: String,
    pub expires_at_ms: u64,
    pub max_uses: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoomCodeCreated {
    /// The display form `ABCD-EFGH`.
    pub code: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecideRequestBody {
    pub role: JoinsAs,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoHostBody {
    pub invite_players: bool,
    pub start_or_pause_session: bool,
    pub export_campaign: bool,
}

/// The state of a pairing as the display screen sees it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStateName {
    Pending,
    Approved,
    Redeemed,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayChallengeResponse {
    /// The display form `ABCD-EFGH`; the device secret travels in a cookie, never here.
    pub user_code: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayStatusResponse {
    pub state: PairingStateName,
    pub user_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayRedeemResponse {
    pub session_id: String,
    pub ticket_expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayRenewResponse {
    pub ticket_expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayLookupBody {
    pub code: String,
}

/// What the host sees before confirming: creation time, device class and how long it has waited.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayLookupResponse {
    pub pairing_id: String,
    pub created_at_ms: u64,
    pub device_label: DeviceLabel,
    pub waiting_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayApproveBody {
    pub pairing_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DisplayCard {
    pub ticket_id: String,
    pub device_label: DeviceLabel,
    pub paired_at_ms: u64,
    pub expires_at_ms: u64,
}

/// An empty success body.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Done {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CampaignCreateBody {
    pub name: String,
    pub seat_count: u32,
    pub content_rating: ContentRating,
    /// Room codes with approval (true) or none (false).
    pub room_codes: bool,
    pub default_co_host: CoHostBody,
    pub operation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CampaignCreated {
    pub campaign_id: String,
    pub member_id: String,
}

/// The owner's typed campaign name, for archive and delete.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TypedNameBody {
    pub typed_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeatBody {
    pub seated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleBody {
    pub role: JoinsAs,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferKindName {
    Host,
    Ownership,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OldHostBecomesName {
    Player,
    CoHost,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransferProposeBody {
    pub kind: TransferKindName,
    pub to_member: String,
    pub old_host_becomes: Option<OldHostBecomesName>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransferProposed {
    pub proposal_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UnpairBody {
    /// One ticket, or every ticket of the session when absent.
    pub ticket_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UnpairResponse {
    pub revoked: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleName {
    Host,
    CoHost,
    Player,
    Contributor,
}

/// The caller's own membership in a campaign (the member snapshot, without internal ids of other
/// owners).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MeResponse {
    pub member_id: String,
    pub role: RoleName,
    pub holds_seat: bool,
    pub delegations: CoHostBody,
    pub authorized: bool,
    pub member_revision: u64,
    pub seat_count: u32,
    pub event_seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_invite_preview_has_exactly_four_fields() {
        // An exhaustive destructure: adding a fifth field stops this compiling.
        let InvitePreview {
            campaign_name,
            host_display_name,
            scheduled_at_ms,
            content_rating,
        } = InvitePreview {
            campaign_name: "The Salt Road".into(),
            host_display_name: "Ana".into(),
            scheduled_at_ms: Some(5),
            content_rating: ContentRating::Standard,
        };
        let json = serde_json::to_value(InvitePreview {
            campaign_name,
            host_display_name,
            scheduled_at_ms,
            content_rating,
        })
        .unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "campaign_name",
                "content_rating",
                "host_display_name",
                "scheduled_at_ms"
            ]
        );
    }

    #[test]
    fn reasons_serialize_as_snake_case_words() {
        let json = serde_json::to_string(&ReasonBody {
            reason: Reason::InvitationExpired,
        })
        .unwrap();
        assert_eq!(json, r#"{"reason":"invitation_expired"}"#);
        let back: ReasonBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back.reason, Reason::InvitationExpired);
    }

    #[test]
    fn device_labels_round_trip_through_their_text() {
        for label in [
            DeviceLabel::Phone,
            DeviceLabel::Tablet,
            DeviceLabel::Desktop,
            DeviceLabel::Unknown,
        ] {
            assert_eq!(DeviceLabel::parse(label.text()), Some(label));
        }
        assert_eq!(DeviceLabel::parse("watch"), None);
    }

    #[test]
    fn unknown_fields_in_a_request_body_are_ignored_but_missing_ones_are_refused() {
        let body: JoinTokenBody = serde_json::from_str(r#"{"token":"abc","extra":1}"#).unwrap();
        assert_eq!(body.token, "abc");
        assert!(serde_json::from_str::<JoinTokenBody>("{}").is_err());
    }
}
