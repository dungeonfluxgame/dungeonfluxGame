//! What the owner is given beside its adapters: the event sink and the replay behind its seam.
//!
//! The sink is where every typed event and every engine audit line goes (`crate::observe`); the
//! replayer is what `replay` calls (`replay`). `Extras::memory_only` is the standard owner: the
//! history in memory, no files. `Extras::from_environment` is the dev server's: the same, plus a
//! file spool when `DF_TABLE_EVENT_SPOOL` names a directory. A spool that cannot be opened stops
//! the owner from starting with a typed `EngineStartError::Spool`, so a missing durable copy is
//! never a quiet default.

use std::path::PathBuf;
use std::sync::Arc;

use crate::event_log::EventLog;
use crate::observe::{EVENT_SPOOL_VARIABLE, FileSpool, HostEventSink, SPOOL_FILE_NAME, WallClock};

use super::replay::{PendingReplay, Replayer};
use super::setup::EngineStartError;

/// The sink and the replayer an owner is started with.
pub struct Extras {
    pub sink: Arc<HostEventSink>,
    pub replayer: Arc<dyn Replayer>,
}

impl Extras {
    pub fn new(sink: Arc<HostEventSink>, replayer: Arc<dyn Replayer>) -> Self {
        Self { sink, replayer }
    }

    /// The history in memory and nothing on disk: every event is a line in `log` and a record in
    /// the history, timed by the system clock.
    pub fn memory_only(log: Arc<EventLog>) -> Self {
        Self::new(
            Arc::new(HostEventSink::new(log, Arc::new(WallClock))),
            Arc::new(PendingReplay),
        )
    }

    /// `memory_only` plus the spool in the directory the environment names, if it names one.
    pub fn from_environment(log: Arc<EventLog>) -> Result<Self, EngineStartError> {
        Self::with_spool_directory(log, spool_directory_from_environment())
    }

    /// `memory_only`, plus a spool in `directory` when one is given (created if missing).
    pub fn with_spool_directory(
        log: Arc<EventLog>,
        directory: Option<PathBuf>,
    ) -> Result<Self, EngineStartError> {
        let sink = HostEventSink::new(log, Arc::new(WallClock));
        let sink = match directory {
            None => sink,
            Some(directory) => {
                std::fs::create_dir_all(&directory).map_err(|error| {
                    EngineStartError::SpoolDirectory {
                        kind: error.kind(),
                        directory: directory.clone(),
                    }
                })?;
                let spool = FileSpool::open(&directory.join(SPOOL_FILE_NAME))?;
                sink.with_outlet(Arc::new(spool))
            }
        };
        Ok(Self::new(Arc::new(sink), Arc::new(PendingReplay)))
    }
}

/// The directory `DF_TABLE_EVENT_SPOOL` names, if it is set to something.
fn spool_directory_from_environment() -> Option<PathBuf> {
    crate::environment::path(EVENT_SPOOL_VARIABLE)
}
