//! The one error of building a production adapter.
//!
//! Every variant names a value that was refused when an adapter, a roster or a book was built, so
//! the host fails at start with the thing to fix and never plays with a half-valid table. A port
//! call that fails at run time is a `df_table::ports::PortError`, not this.

use std::fmt;

use df_table::element::NameError;
use df_table::identity::SeatId;
use df_table::ports::ActionId;

use super::backend::RULES_BACKEND_VARIABLE;
use super::hero_profile::{
    Ability, MAXIMUM_ABILITY_SCORE, MAXIMUM_PROFICIENCY_BONUS, MINIMUM_ABILITY_SCORE,
    MINIMUM_PROFICIENCY_BONUS,
};

/// Why an adapter, a roster or an action book could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    AbilityScoreOutOfRange {
        ability: Ability,
        score: u8,
    },
    ProficiencyBonusOutOfRange {
        bonus: u8,
    },
    SeatAlreadyHasHero {
        seat: SeatId,
    },
    ActionAlreadyRegistered {
        action: ActionId,
    },
    DifficultyClassIsZero,
    /// An influence entry needs the short reason the table shows for the words' tilt, which
    /// `df-rules` refuses to be blank.
    InfluenceReasonIsBlank,
    UnknownRulesBackend {
        text: String,
    },
    /// The setting is set but is not text.
    RulesBackendNotText,
    Name(NameError),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbilityScoreOutOfRange { ability, score } => write!(
                formatter,
                "the {} score {score} is outside {MINIMUM_ABILITY_SCORE} to {MAXIMUM_ABILITY_SCORE}",
                ability.key()
            ),
            Self::ProficiencyBonusOutOfRange { bonus } => write!(
                formatter,
                "the proficiency bonus {bonus} is outside \
                 {MINIMUM_PROFICIENCY_BONUS} to {MAXIMUM_PROFICIENCY_BONUS}"
            ),
            Self::SeatAlreadyHasHero { seat } => {
                write!(formatter, "seat {seat} already has a hero")
            }
            Self::ActionAlreadyRegistered { action } => {
                write!(formatter, "the action {action} is already in the book")
            }
            Self::DifficultyClassIsZero => formatter.write_str("a difficulty class is at least 1"),
            Self::InfluenceReasonIsBlank => {
                formatter.write_str("an influence entry needs a reason for the words' tilt")
            }
            Self::UnknownRulesBackend { text } => write!(
                formatter,
                "'{text}' is not a rules backend; use scripted or df-rules ({RULES_BACKEND_VARIABLE})"
            ),
            Self::RulesBackendNotText => {
                write!(formatter, "{RULES_BACKEND_VARIABLE} is not valid text")
            }
            Self::Name(error) => write!(formatter, "a name is invalid: {error}"),
        }
    }
}

impl std::error::Error for AdapterError {}

impl From<NameError> for AdapterError {
    fn from(error: NameError) -> Self {
        Self::Name(error)
    }
}
