//! What became of one input: the four outcomes, the declines and their public class.
//!
//! It owns `Outcome` (applied, needs clarification, declined, pending: RES16, O.7), `Decline`
//! with its closed `DeclineCode` (C.6), `PendingKind` (RES26) and the three exhaustive tables of a
//! code: its key, its reason text and its public class (RES40). Each table is a `match` with no
//! wildcard arm, and `DeclineCode::ALL` is generated from the enum, so a code added without a key,
//! a reason text and a class does not compile. It never decides whether an input is declined (a
//! machine or the pipeline does) and never shows a code to a player (K.5: the reason text is the
//! only thing a person reads).
//!
//! A `Decline` is built only with a reason: an empty or blank text is `DeclineError::EmptyReason`,
//! and the list of alternatives has the named bound `MAXIMUM_ALTERNATIVES`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;
use crate::private_data::{LengthClass, Measured};

/// The most alternatives a decline holds. Deliberately huge so it never limits play; scope it down
/// later (owner, 2026-11-02). The phone's own choice-list rule of O.12 is applied when it draws.
pub const MAXIMUM_ALTERNATIVES: usize = 10_000;

listed_enum! {
    /// Why an input was declined. The list is C.6; an RES47 or M2-D27 code is one more variant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DeclineCode {
        NotViable,
        NotSupportedYet,
        NotYourTurn,
        NotInCombat,
        NoCombatProfile,
        MissingRequirement,
        WindowClosed,
        EconomySpent,
        ResourceSpent,
        TableBusy,
        RateLimited,
        IntentInvalid,
        PendingOpen,
        PvpOff,
        ConsentDeclined,
        TimeLocked,
        NotKnowable,
        StockChanged,
        SameUrging,
        CastingRestricted,
    }
}

/// How a decline reaches a client: one of the closed public rejections of `rpc-api.md` (RES40), a
/// resource-exhausted failure, or no public class at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicClass {
    WrongTurn,
    StaleOffer,
    InvalidSelection,
    ResourceMissing,
    Paused,
    CapabilityUnavailable,
    /// A `RESOURCE_EXHAUSTED` failure, not a rejection receipt.
    ResourceExhausted,
    /// Stays an internal reason key until the contract wave admits it (RES40).
    InternalReasonKey,
}

impl DeclineCode {
    /// The closed key written into an audit line, a record and the wire's internal reason.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotViable => "not_viable",
            Self::NotSupportedYet => "not_supported_yet",
            Self::NotYourTurn => "not_your_turn",
            Self::NotInCombat => "not_in_combat",
            Self::NoCombatProfile => "no_combat_profile",
            Self::MissingRequirement => "missing_requirement",
            Self::WindowClosed => "window_closed",
            Self::EconomySpent => "economy_spent",
            Self::ResourceSpent => "resource_spent",
            Self::TableBusy => "table_busy",
            Self::RateLimited => "rate_limited",
            Self::IntentInvalid => "intent_invalid",
            Self::PendingOpen => "pending_open",
            Self::PvpOff => "pvp_off",
            Self::ConsentDeclined => "consent_declined",
            Self::TimeLocked => "time_locked",
            Self::NotKnowable => "not_knowable",
            Self::StockChanged => "stock_changed",
            Self::SameUrging => "same_urging",
            Self::CastingRestricted => "casting_restricted",
        }
    }

    /// The plain sentence a player reads when nothing more specific is known (K.5).
    pub fn default_reason(self) -> &'static str {
        match self {
            Self::NotViable => "That isn't possible right now.",
            Self::NotSupportedYet => "I can't run that yet.",
            Self::NotYourTurn => "It isn't your turn.",
            Self::NotInCombat => "That only works in combat.",
            Self::NoCombatProfile => "I have no combat details for that yet.",
            Self::MissingRequirement => "You don't have what that needs.",
            Self::WindowClosed => "That chance has passed.",
            Self::EconomySpent => "You have already used that this turn.",
            Self::ResourceSpent => "That is used up.",
            Self::TableBusy => "You have too many moves waiting. Give them a moment.",
            Self::RateLimited => "That is a little too fast. Try again in a moment.",
            Self::IntentInvalid => "I could not use that choice.",
            Self::PendingOpen => "Something is waiting for an answer first.",
            Self::PvpOff => "This table has fighting between players switched off.",
            Self::ConsentDeclined => "Someone at the table has not agreed to that.",
            Self::TimeLocked => "Time can't move on while that is happening.",
            Self::NotKnowable => "You can't know that.",
            Self::StockChanged => "The stock changed before you chose.",
            Self::SameUrging => "They have already heard that. Try something new.",
            Self::CastingRestricted => "You can't cast that right now.",
        }
    }

    /// The public class of this decline (RES40). Codes the plan leaves internal map to
    /// `InternalReasonKey`; `time_locked` and `pending_open` pass once the table moves on, so they
    /// are `Paused`, while `pvp_off` and `consent_declined` stay true until a person changes them,
    /// so they are `CapabilityUnavailable`.
    pub fn public_class(self) -> PublicClass {
        match self {
            Self::NotYourTurn => PublicClass::WrongTurn,
            Self::WindowClosed | Self::StockChanged => PublicClass::StaleOffer,
            Self::IntentInvalid => PublicClass::InvalidSelection,
            Self::ResourceSpent | Self::EconomySpent => PublicClass::ResourceMissing,
            Self::TimeLocked | Self::PendingOpen => PublicClass::Paused,
            Self::PvpOff | Self::ConsentDeclined => PublicClass::CapabilityUnavailable,
            Self::TableBusy | Self::RateLimited => PublicClass::ResourceExhausted,
            Self::NotViable
            | Self::NotSupportedYet
            | Self::NotInCombat
            | Self::NoCombatProfile
            | Self::MissingRequirement
            | Self::NotKnowable
            | Self::SameUrging
            | Self::CastingRestricted => PublicClass::InternalReasonKey,
        }
    }
}

/// Why a decline could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineError {
    EmptyReason,
    EmptyMissingPiece,
    EmptyAlternative,
    TooManyAlternatives,
}

impl fmt::Display for DeclineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyReason => formatter.write_str("a decline needs a reason"),
            Self::EmptyMissingPiece => formatter.write_str("a missing piece cannot be blank"),
            Self::EmptyAlternative => formatter.write_str("an alternative cannot be blank"),
            Self::TooManyAlternatives => write!(
                formatter,
                "a decline offers at most {MAXIMUM_ALTERNATIVES} alternatives"
            ),
        }
    }
}

impl std::error::Error for DeclineError {}

fn non_blank(text: &str, error: DeclineError) -> Result<String, DeclineError> {
    if text.trim().is_empty() {
        return Err(error);
    }
    Ok(text.to_owned())
}

/// A refusal with a code, a reason a person can read, what was missing and what would work. Its
/// `Debug` shows the code and only the length class of each text, because a missing piece can name
/// something a seat has not earned the right to know.
#[derive(Clone, PartialEq, Eq)]
pub struct Decline {
    code: DeclineCode,
    reason: String,
    missing_piece: Option<String>,
    alternatives: Vec<String>,
}

impl Decline {
    /// A decline with the reason the machine wrote for this case.
    pub fn new(code: DeclineCode, reason: &str) -> Result<Self, DeclineError> {
        Ok(Self {
            code,
            reason: non_blank(reason, DeclineError::EmptyReason)?,
            missing_piece: None,
            alternatives: Vec::new(),
        })
    }

    /// A decline with the default reason text of the code.
    pub fn with_default_reason(code: DeclineCode) -> Self {
        Self {
            code,
            reason: code.default_reason().to_owned(),
            missing_piece: None,
            alternatives: Vec::new(),
        }
    }

    pub fn with_missing_piece(mut self, missing_piece: &str) -> Result<Self, DeclineError> {
        self.missing_piece = Some(non_blank(missing_piece, DeclineError::EmptyMissingPiece)?);
        Ok(self)
    }

    pub fn with_alternative(mut self, alternative: &str) -> Result<Self, DeclineError> {
        if self.alternatives.len() >= MAXIMUM_ALTERNATIVES {
            return Err(DeclineError::TooManyAlternatives);
        }
        self.alternatives
            .push(non_blank(alternative, DeclineError::EmptyAlternative)?);
        Ok(self)
    }

    pub fn code(&self) -> DeclineCode {
        self.code
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn missing_piece(&self) -> Option<&str> {
        self.missing_piece.as_deref()
    }

    pub fn alternatives(&self) -> &[String] {
        &self.alternatives
    }

    /// The same decline keeping only the alternatives `keep` accepts, so the text a seat reads
    /// offers nothing the rules owner's legal choices do not (C.2).
    pub fn keeping_alternatives(&self, keep: impl Fn(&str) -> bool) -> Self {
        Self {
            code: self.code,
            reason: self.reason.clone(),
            missing_piece: self.missing_piece.clone(),
            alternatives: self
                .alternatives
                .iter()
                .filter(|alternative| keep(alternative))
                .cloned()
                .collect(),
        }
    }

    /// The saved form of a decline: the words a seat was shown, for a checkpoint or a result book.
    pub fn to_record(&self) -> DeclineRecord {
        DeclineRecord {
            code: self.code,
            reason: self.reason.clone(),
            missing_piece: self.missing_piece.clone(),
            alternatives: self.alternatives.clone(),
        }
    }

    /// A decline read back from its saved form, refused if a text is blank.
    pub fn from_record(record: DeclineRecord) -> Result<Self, DeclineError> {
        let mut decline = Self::new(record.code, &record.reason)?;
        if let Some(missing) = &record.missing_piece {
            decline = decline.with_missing_piece(missing)?;
        }
        for alternative in &record.alternatives {
            decline = decline.with_alternative(alternative)?;
        }
        Ok(decline)
    }
}

/// A decline as it is saved. It holds the texts a seat was shown, so it belongs in the table's own
/// checkpoint and never in a log.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclineRecord {
    code: DeclineCode,
    reason: String,
    missing_piece: Option<String>,
    alternatives: Vec<String>,
}

impl DeclineRecord {
    pub fn code(&self) -> DeclineCode {
        self.code
    }
}

impl fmt::Debug for DeclineRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclineRecord")
            .field("code", &self.code)
            .field("alternatives", &self.alternatives.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Decline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class_of = |text: &String| LengthClass::of_length(text.length_in_chars());
        formatter
            .debug_struct("Decline")
            .field("code", &self.code)
            .field("reason", &class_of(&self.reason))
            .field("missing_piece", &self.missing_piece.as_ref().map(class_of))
            .field("alternatives", &self.alternatives.len())
            .finish()
    }
}

listed_enum! {
    /// What a pending resolution waits for (RES26).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PendingKind {
        Choice,
        Roll,
        Reaction,
        Ruling,
    }
}

impl PendingKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Roll => "roll",
            Self::Reaction => "reaction",
            Self::Ruling => "ruling",
        }
    }
}

listed_enum! {
    /// The four shapes of an outcome without their payload, for events and counters.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum OutcomeKind {
        Applied,
        NeedsClarification,
        Declined,
        Pending,
    }
}

impl OutcomeKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NeedsClarification => "needs_clarification",
            Self::Declined => "declined",
            Self::Pending => "pending",
        }
    }
}

/// How one input ended. `Waiting{pending_id}` is a machine's step result, not an outcome (O.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    NeedsClarification,
    Declined(Decline),
    Pending(PendingKind),
}

impl Outcome {
    pub fn kind(&self) -> OutcomeKind {
        match self {
            Self::Applied => OutcomeKind::Applied,
            Self::NeedsClarification => OutcomeKind::NeedsClarification,
            Self::Declined(_) => OutcomeKind::Declined,
            Self::Pending(_) => OutcomeKind::Pending,
        }
    }
}
