//! The seam for replay (H.16 `replay <log> [--until seq] [--diff]`): the host asks a `Replayer`,
//! and until C7's replay exists the answer is a typed "not supported yet".
//!
//! C7 builds `Inspect` and replay in `df-table` (`inspect*`, `replay*`); `golden` has the runner
//! over scripted ports today, and a turn log in a form the host can load does not exist yet. So
//! the verb, its route, its audit line and its typed refusal are built and tested, and what
//! stands behind them is `PendingReplay`. When C7 lands, one implementation of `Replayer` that
//! calls it replaces `PendingReplay` in `Extras::standard`; nothing else of the verb changes. The
//! host never reads the log: what `log` names is the replay's to open.

use std::fmt;

use super::report::EngineError;
use super::report_observe::ReplayReport;
use super::request::ReplayRequest;

/// What C7's replay is called through.
pub trait Replayer: Send + Sync {
    fn replay(&self, request: &ReplayRequest) -> Result<ReplayReport, ReplayRefusal>;
}

/// Why a replay did not run, with what the caller does next in each variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayRefusal {
    /// There is no replay behind the seam yet; `needs` names what it waits for.
    NotSupportedYet { needs: &'static str },
    /// No log of that name.
    UnknownLog { log: String },
    /// The replay ran and failed.
    Failed { reason: String },
}

impl fmt::Display for ReplayRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSupportedYet { needs } => {
                write!(formatter, "replay is not built yet: {needs}")
            }
            Self::UnknownLog { log } => write!(formatter, "there is no turn log '{log}'"),
            Self::Failed { reason } => write!(formatter, "the replay failed: {reason}"),
        }
    }
}

impl std::error::Error for ReplayRefusal {}

impl ReplayRefusal {
    /// The closed key the audit line carries.
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotSupportedYet { .. } => "not_supported_yet",
            Self::UnknownLog { .. } => "unknown_log",
            Self::Failed { .. } => "failed",
        }
    }
}

impl From<ReplayRefusal> for EngineError {
    fn from(refusal: ReplayRefusal) -> Self {
        match refusal {
            ReplayRefusal::NotSupportedYet { needs } => Self::NotSupportedYet {
                what: "replay".to_owned(),
                needs: needs.to_owned(),
            },
            ReplayRefusal::UnknownLog { log } => Self::UnknownLog { log },
            ReplayRefusal::Failed { reason } => Self::ReplayFailed { reason },
        }
    }
}

/// What C7's replay waits for, said once.
pub const REPLAY_NEEDS: &str =
    "C7's replay in df-table (inspect and replay) and a turn log the host can load";

/// The stand-in behind the seam until C7's replay exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct PendingReplay;

impl Replayer for PendingReplay {
    fn replay(&self, _request: &ReplayRequest) -> Result<ReplayReport, ReplayRefusal> {
        Err(ReplayRefusal::NotSupportedYet {
            needs: REPLAY_NEEDS,
        })
    }
}
