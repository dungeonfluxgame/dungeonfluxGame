//! The wire values of `rules check`: the request, the report, the typed refusal and the reply.

use axum::http::StatusCode;
use df_table::event::FailureCause;
use df_table::outcome::DeclineCode;
use df_table::ports::{PortError, PortName};
use serde::{Deserialize, Serialize};

use crate::adapters::action_book::DifficultyClass;
use crate::adapters::hero_profile::{Ability, Skill};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::engine::report::UnreadableBodyCause;

/// What `rules check` asks for. The actor is a seat, which is how the rules port names a hero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesCheckRequest {
    pub skill: Skill,
    pub actor: u64,
    /// The authored difficulty class; the SRD's Medium (15) when absent.
    pub dc: Option<DifficultyClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedFact {
    pub label: String,
    pub amount: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedDraw {
    pub sequence: u64,
    pub sides: u16,
    pub value: u16,
}

/// The answer to `rules check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RulesCheckReport {
    Resolved {
        skill: Skill,
        ability: Ability,
        actor: u64,
        difficulty_class: u8,
        succeeded: bool,
        facts: Vec<ReportedFact>,
        draws: Vec<ReportedDraw>,
    },
    Declined {
        actor: u64,
        code: DeclineCode,
        reason: String,
        missing_piece: Option<String>,
    },
}

/// Why a `rules check` call was refused. A decline of the rules is not a refusal: it is a
/// `RulesCheckReport::Declined`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum RulesCheckError {
    /// The request body is not a `RulesCheckRequest`.
    InvalidBody(BodyError),
    /// The request body could not be read at all: it is over the host's limit, or it broke off.
    UnreadableBody { cause: UnreadableBodyCause },
    /// The table runs the scripted rules backend, which has no check kit.
    ScriptedBackend,
    /// A port the check went through could not answer.
    PortFailed { port: PortName, cause: FailureCause },
    /// The rules adapter asked for more than a check needs (more draws or a pause).
    UnexpectedResolution,
    /// The rules adapter resolved the check without a `succeeded` fact.
    NoVerdict,
}

impl RulesCheckError {
    /// The serde tag of the error, for the refusal's audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::UnreadableBody { .. } => "unreadable_body",
            Self::ScriptedBackend => "scripted_backend",
            Self::PortFailed { .. } => "port_failed",
            Self::UnexpectedResolution => "unexpected_resolution",
            Self::NoVerdict => "no_verdict",
        }
    }
}

impl From<BodyError> for RulesCheckError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<PortError> for RulesCheckError {
    fn from(error: PortError) -> Self {
        Self::PortFailed {
            port: error.port,
            cause: error.cause,
        }
    }
}

impl std::fmt::Display for RulesCheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::UnreadableBody {
                cause: UnreadableBodyCause::TooLarge,
            } => formatter.write_str("the request body is over the host's limit"),
            Self::UnreadableBody {
                cause: UnreadableBodyCause::Interrupted,
            } => formatter.write_str("the request body broke off before it was read"),
            Self::ScriptedBackend => formatter.write_str(
                "a rules check is not available: this table runs the scripted rules backend",
            ),
            Self::PortFailed { port, cause } => write!(
                formatter,
                "the {} port failed during the check: {}",
                port.key(),
                cause.key()
            ),
            Self::UnexpectedResolution => {
                formatter.write_str("the rules adapter asked for more than a check needs")
            }
            Self::NoVerdict => formatter.write_str("the rules adapter reported no verdict"),
        }
    }
}

impl std::error::Error for RulesCheckError {}

impl RouteError for RulesCheckError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::UnreadableBody {
                cause: UnreadableBodyCause::Interrupted,
            } => StatusCode::BAD_REQUEST,
            Self::UnreadableBody {
                cause: UnreadableBodyCause::TooLarge,
            } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ScriptedBackend => StatusCode::CONFLICT,
            Self::PortFailed { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::UnexpectedResolution | Self::NoVerdict => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// The answer of the `rules check` route.
pub type RulesCheckReply = DevReply<RulesCheckReport, RulesCheckError>;
