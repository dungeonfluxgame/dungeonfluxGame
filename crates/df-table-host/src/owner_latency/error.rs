//! Why a latency run could not finish, as one typed enum.
//!
//! A run either produces every number it promised or stops with one of these; it never reports a
//! partial result as a whole one.

use std::fmt;

use df_table::config::ConfigError;
use df_table::element::NameError;
use df_table::owner_queue::{FinishError, QueueConfigError, SeatError, ServeError};

use super::statistics::StatisticsError;

/// A failure of the harness itself, not a measurement.
#[derive(Debug)]
pub enum HarnessError {
    /// The queue settings could not be loaded.
    Settings(ConfigError),
    /// The action name of the stand-in input is not a valid name.
    Action(NameError),
    /// The queue could not be built from the settings.
    Queue(QueueConfigError),
    /// A seat could not join the queue.
    Seat(SeatError),
    /// The queue could not serve the next input.
    Serve(ServeError),
    /// The queue refused to finish a served input.
    Finish(FinishError),
    /// A view or checkpoint could not be written as JSON.
    Serialise(serde_json::Error),
    /// A checkpoint file operation failed; `step` says which.
    Io {
        step: &'static str,
        source: std::io::Error,
    },
    /// A statistic could not be computed over the records.
    Statistics(StatisticsError),
    /// A harness thread panicked; `thread` names it.
    ThreadPanicked { thread: &'static str },
    /// The run served nothing after the warm-up, so there is nothing to report.
    NothingMeasured,
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settings(error) => write!(formatter, "the queue settings failed: {error}"),
            Self::Action(error) => write!(formatter, "the stand-in action is invalid: {error}"),
            Self::Queue(error) => write!(formatter, "the queue could not be built: {error}"),
            Self::Seat(error) => write!(formatter, "a seat could not join: {error}"),
            Self::Serve(error) => write!(formatter, "the queue could not serve: {error}"),
            Self::Finish(error) => write!(formatter, "the queue could not finish: {error}"),
            Self::Serialise(error) => write!(formatter, "serialising failed: {error}"),
            Self::Io { step, source } => write!(formatter, "{step} failed: {source}"),
            Self::Statistics(error) => write!(formatter, "a statistic failed: {error}"),
            Self::ThreadPanicked { thread } => write!(formatter, "the {thread} thread panicked"),
            Self::NothingMeasured => formatter.write_str("no input was served after the warm-up"),
        }
    }
}

impl std::error::Error for HarnessError {}

impl From<StatisticsError> for HarnessError {
    fn from(error: StatisticsError) -> Self {
        Self::Statistics(error)
    }
}

impl From<serde_json::Error> for HarnessError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialise(error)
    }
}

impl From<SeatError> for HarnessError {
    fn from(error: SeatError) -> Self {
        Self::Seat(error)
    }
}

impl From<ServeError> for HarnessError {
    fn from(error: ServeError) -> Self {
        Self::Serve(error)
    }
}

impl From<FinishError> for HarnessError {
    fn from(error: FinishError) -> Self {
        Self::Finish(error)
    }
}
