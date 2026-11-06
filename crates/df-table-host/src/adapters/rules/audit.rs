//! The lines the production rules owner writes: `RULES resolved` for every check or urging it
//! resolved through the rules port, and `RULES refused` with a closed cause for every call it
//! could not answer.
//!
//! They hold the action's checked name, the operation id, closed keys and numbers, never a hero's
//! name or a player's words. They go through df-observe's shared audit path, the one the host
//! makes its event log at start (`event_log::install_as_audit_sink`), because the adapter is
//! built before any route and has no log of its own.

use std::fmt::Write;

use df_rules::influence::RollMode;
use df_table::event::FailureCause;
use df_table::identity::OperationId;
use df_table::ports::{ActionId, PortError, PortName};

/// The audit area of the lines.
const AREA: &str = "RULES";

/// Why the adapter could not answer a call: the closed half of its `invalid_answer` refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesRefusal {
    /// More draws than the question needs, or a die that is not a d20.
    DrawsUnfit,
    /// A drawn value that is no d20 face.
    FaceOutOfRange,
    /// `df-rules` refused the inputs of an ability check.
    CheckRefused,
    /// `df-rules` refused the inputs of an influence urging.
    InfluenceRefused,
    /// A rolled urging came back without its counted die or its total.
    InfluenceIncomplete,
    /// The table refused the name of a fact.
    FactName,
    /// The table refused the text of a decline.
    DeclineText,
}

impl RulesRefusal {
    /// Every cause, for the test that their keys are distinct.
    #[cfg(test)]
    pub const ALL: [Self; 7] = [
        Self::DrawsUnfit,
        Self::FaceOutOfRange,
        Self::CheckRefused,
        Self::InfluenceRefused,
        Self::InfluenceIncomplete,
        Self::FactName,
        Self::DeclineText,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::DrawsUnfit => "draws_unfit",
            Self::FaceOutOfRange => "face_out_of_range",
            Self::CheckRefused => "check_refused",
            Self::InfluenceRefused => "influence_refused",
            Self::InfluenceIncomplete => "influence_incomplete",
            Self::FactName => "fact_name",
            Self::DeclineText => "decline_text",
        }
    }

    /// The port's refusal: every cause is an answer the port cannot give.
    pub fn port_error(self) -> PortError {
        PortError::new(PortName::RulesOwner, FailureCause::InvalidAnswer)
    }
}

/// What was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedKind {
    SkillCheck,
    Influence,
}

impl ResolvedKind {
    fn key(self) -> &'static str {
        match self {
            Self::SkillCheck => "skill_check",
            Self::Influence => "influence",
        }
    }
}

/// How a resolution came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedResult {
    Success,
    Failure,
    /// An urging a willing or an unwilling creature answered without a roll.
    NoRoll,
}

impl ResolvedResult {
    fn key(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::NoRoll => "no_roll",
        }
    }
}

/// The numbers of one resolution, as its `RULES resolved` line shows them. A die and a total are
/// absent when no roll was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedLine {
    pub kind: ResolvedKind,
    pub natural: Option<u8>,
    pub total: Option<i16>,
    pub difficulty_class: u8,
    pub result: ResolvedResult,
    pub mode: RollMode,
}

fn mode_key(mode: RollMode) -> &'static str {
    match mode {
        RollMode::Normal => "normal",
        RollMode::Advantage => "advantage",
        RollMode::Disadvantage => "disadvantage",
    }
}

fn dash_or(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

/// `RULES resolved action=<id> operation=<id> kind=.. natural=.. total=.. dc=.. result=.. mode=..`.
pub fn resolved_line(action: &ActionId, operation: OperationId, line: &ResolvedLine) -> String {
    let mut text = String::with_capacity(160);
    // Writing into a String cannot fail, so the result carries no information.
    let _ = write!(
        text,
        "{AREA} resolved action={} operation={operation} kind={} natural={} total={} dc={} \
         result={} mode={}",
        action.as_str(),
        line.kind.key(),
        dash_or(line.natural),
        dash_or(line.total),
        line.difficulty_class,
        line.result.key(),
        mode_key(line.mode)
    );
    text
}

/// `RULES refused action=<id> operation=<id> cause=<key>`.
pub fn refused_line(action: &ActionId, operation: OperationId, cause: RulesRefusal) -> String {
    format!(
        "{AREA} refused action={} operation={operation} cause={}",
        action.as_str(),
        cause.key()
    )
}

/// The audit correlation of a line about `operation`: the shared `op=` key is the
/// `df_observe::operation_hash` of the operation's bytes, so it names the same operation as the
/// line's own `operation=` field instead of the owner thread's empty context (`op=none`).
pub fn correlation(operation: OperationId) -> df_observe::AuditContext {
    df_observe::AuditContext {
        operation: Some(df_observe::operation_hash(operation.as_bytes())),
        ..df_observe::current_audit_context()
    }
}

/// Records one line about `operation` through df-observe's shared audit path.
pub fn record(operation: OperationId, line: String) {
    let _scope = df_observe::audit_scope(correlation(operation));
    df_observe::record_audit_line(AREA, line);
}
