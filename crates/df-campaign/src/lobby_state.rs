//! The values the lobby keeps and reads (`lobby_member`, 6) and the ports' answers it needs. Plain
//! values: holding one grants nothing.

use df_types::{CampaignId, MemberId, SessionId};

use crate::{
    ids::TicketId,
    lobby::CharacterChoice,
    records::{CampaignRow, DeviceLabel, MemberRow},
};

/// How the member wants to talk: the browser asks for the microphone, or they type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicrophonePreference {
    Ask,
    TypedOnly,
}

impl MicrophonePreference {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::TypedOnly => "typed_only",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ask" => Some(Self::Ask),
            "typed_only" => Some(Self::TypedOnly),
            _ => None,
        }
    }
}

/// What the browser said when asked for the microphone. `Declined`, `Unsupported` or a typed
/// preference leaves typed play.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicrophoneReport {
    Unknown,
    Granted,
    Declined,
    Unsupported,
}

impl MicrophoneReport {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Granted => "granted",
            Self::Declined => "declined",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "unknown" => Some(Self::Unknown),
            "granted" => Some(Self::Granted),
            "declined" => Some(Self::Declined),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// A seated member's lobby state for one session. A member with no row has the defaults:
/// undecided, not ready, `ask`, `unknown`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LobbyMemberRow {
    pub session: SessionId,
    pub member: MemberId,
    pub campaign: CampaignId,
    pub character: CharacterChoice,
    /// The characters owner's id for the chosen template or character; opaque here.
    pub character_ref: Option<[u8; 16]>,
    pub ready: bool,
    pub microphone_preference: MicrophonePreference,
    pub microphone_report: MicrophoneReport,
    pub updated_at_ms: u64,
    pub revision: u64,
    /// Set once the session owner reported the session started; nothing changes after.
    pub frozen: bool,
}

/// A live display ticket of the session, as the lobby lists displays (no secret of any kind).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LobbyDisplay {
    pub ticket: TicketId,
    pub label: DeviceLabel,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Everything the lobby view and `decide_start` are built from.
#[derive(Clone, Debug)]
pub struct LobbyRows {
    pub campaign: CampaignRow,
    /// Active members that hold a seat, host first, then by join time, with their lobby row.
    pub seats: Vec<(MemberRow, Option<LobbyMemberRow>)>,
    pub displays: Vec<LobbyDisplay>,
}

/// F's typed answer to "may this session start", never computed here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartReadiness {
    pub ready: bool,
    /// A named sponsor has not yet approved: the lobby says it is waiting for them.
    pub waiting_for_sponsor: bool,
    /// The lobby may say "No charge until the host starts, except explicitly approved setup jobs".
    pub no_charge_until_start: bool,
}

/// One seated, authorized member handed to the session owner at Start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeatedMember {
    pub member: MemberId,
    pub account: df_types::AccountId,
}
