//! The state of one scene transition: its stages in order, one deadline per stage, a typed failure
//! that leaves the old scene up, and a record at every step (`planning/scene-transitions.md`).
//!
//! Pure: the caller supplies every "now", so nothing here reads a clock, logs or spawns work. The
//! host writes the records to the audit log through [`format_record`].

use std::fmt;

use super::types::{TransitionFailure, TransitionRecord, TransitionStage};

/// The limit of each non-terminal stage, in milliseconds. A stage that runs past its limit fails
/// with the key `deadline`. `Live` has no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines {
    pub interpreting_ms: u64,
    pub designing_ms: u64,
    pub briefing_ms: u64,
    pub generating_ms: u64,
    pub locating_ms: u64,
    pub placing_ms: u64,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            interpreting_ms: 60_000,
            designing_ms: 60_000,
            briefing_ms: 5_000,
            generating_ms: 900_000,
            locating_ms: 600_000,
            placing_ms: 10_000,
        }
    }
}

impl Deadlines {
    /// The same limits with `stage` set to `ms`. `Live` has no limit, so it changes nothing.
    pub fn with(mut self, stage: TransitionStage, ms: u64) -> Self {
        match stage {
            TransitionStage::Interpreting => self.interpreting_ms = ms,
            TransitionStage::Designing => self.designing_ms = ms,
            TransitionStage::Briefing => self.briefing_ms = ms,
            TransitionStage::Generating => self.generating_ms = ms,
            TransitionStage::Locating => self.locating_ms = ms,
            TransitionStage::Placing => self.placing_ms = ms,
            TransitionStage::Live => {}
        }
        self
    }

    pub fn of(&self, stage: TransitionStage) -> Option<u64> {
        match stage {
            TransitionStage::Interpreting => Some(self.interpreting_ms),
            TransitionStage::Designing => Some(self.designing_ms),
            TransitionStage::Briefing => Some(self.briefing_ms),
            TransitionStage::Generating => Some(self.generating_ms),
            TransitionStage::Locating => Some(self.locating_ms),
            TransitionStage::Placing => Some(self.placing_ms),
            TransitionStage::Live => None,
        }
    }
}

/// Why a step of a transition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// The transition already reached `Live` or failed, so it takes no further step.
    AlreadyFinished { stage: TransitionStage },
    /// The clock is earlier than the last record, so the step would be out of order.
    ClockWentBack { now_ms: u64, last_ms: u64 },
    /// A trace to replay has no records.
    TraceEmpty,
    /// A trace to replay is not one transition in order. `index` is the first record that breaks
    /// the order: the first record must be `Interpreting`, and each later one must follow the stage
    /// before it, in the same transition, at or after the time before it.
    TraceOutOfOrder { index: usize },
}

impl TransitionError {
    pub fn key(self) -> &'static str {
        match self {
            Self::AlreadyFinished { .. } => "already_finished",
            Self::ClockWentBack { .. } => "clock_went_back",
            Self::TraceEmpty => "trace_empty",
            Self::TraceOutOfOrder { .. } => "trace_out_of_order",
        }
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyFinished { stage } => {
                write!(
                    formatter,
                    "transition already finished at stage {}",
                    stage.key()
                )
            }
            Self::ClockWentBack { now_ms, last_ms } => {
                write!(
                    formatter,
                    "transition clock went back from {last_ms} ms to {now_ms} ms"
                )
            }
            Self::TraceEmpty => write!(formatter, "replayed trace has no records"),
            Self::TraceOutOfOrder { index } => {
                write!(
                    formatter,
                    "replayed trace is out of order at record {index}"
                )
            }
        }
    }
}

impl std::error::Error for TransitionError {}

/// One scene transition, from the player's line to the new plate on the scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    id: u64,
    deadlines: Deadlines,
    stage: TransitionStage,
    started_ms: u64,
    stage_started_ms: u64,
    last_ms: u64,
    failure: Option<TransitionFailure>,
    records: Vec<TransitionRecord>,
}

impl Transition {
    /// Starts a transition at `Interpreting` and records the line it was started with.
    pub fn start(id: u64, line: &str, now_ms: u64, deadlines: Deadlines) -> Self {
        let mut transition = Self {
            id,
            deadlines,
            stage: TransitionStage::Interpreting,
            started_ms: now_ms,
            stage_started_ms: now_ms,
            last_ms: now_ms,
            failure: None,
            records: Vec::new(),
        };
        transition.push_record(now_ms, format!("line={line}"));
        transition
    }

    /// Moves to the next stage and records `detail`. A clock earlier than the last record is
    /// refused, and so is any step once the transition is `Live` or failed.
    pub fn advance(
        &mut self,
        now_ms: u64,
        detail: &str,
    ) -> Result<TransitionStage, TransitionError> {
        if self.is_finished() {
            return Err(TransitionError::AlreadyFinished { stage: self.stage });
        }
        self.check_clock(now_ms)?;
        let next = self
            .stage
            .next()
            .ok_or(TransitionError::AlreadyFinished { stage: self.stage })?;
        self.stage = next;
        self.stage_started_ms = now_ms;
        self.push_record(now_ms, detail.to_owned());
        Ok(next)
    }

    /// Fails the current stage, keeping it, and stores the failure. A finished transition keeps
    /// its first outcome, so a late failure is ignored. A clock earlier than the last record is
    /// recorded at the last record's time, so the failure is never lost.
    pub fn fail(&mut self, now_ms: u64, key: &str, message: &str) {
        if self.is_finished() {
            return;
        }
        let at_ms = now_ms.max(self.last_ms);
        self.failure = Some(TransitionFailure {
            stage: self.stage,
            key: key.to_owned(),
            message: message.to_owned(),
        });
        self.push_record(at_ms, format!("failed key={key} message={message}"));
    }

    /// Fails the transition with the key `deadline` when the current stage has run past its limit.
    /// The limit is inclusive: exactly the limit is in time. Returns the stored failure, which a
    /// second call returns again without a second record.
    pub fn check_deadline(&mut self, now_ms: u64) -> Option<&TransitionFailure> {
        if self.failure.is_none() {
            let limit = self.deadlines.of(self.stage)?;
            let in_stage_ms = now_ms.saturating_sub(self.stage_started_ms);
            if in_stage_ms > limit {
                let message = format!(
                    "stage {} ran past its {limit} ms deadline",
                    self.stage.key()
                );
                self.fail(now_ms, "deadline", &message);
            }
        }
        self.failure.as_ref()
    }

    pub fn stage(&self) -> TransitionStage {
        self.stage
    }

    pub fn failure(&self) -> Option<&TransitionFailure> {
        self.failure.as_ref()
    }

    /// `true` once the transition is `Live` or has failed.
    pub fn is_finished(&self) -> bool {
        self.stage == TransitionStage::Live || self.failure.is_some()
    }

    pub fn records(&self) -> &[TransitionRecord] {
        &self.records
    }

    /// Rebuilds a transition from its records, to read its stage and trace. Records carry neither
    /// deadlines nor a failure, so the result has the default deadlines and is never failed. A
    /// failure record repeats its stage, so a trace that ends in one is refused as out of order.
    pub fn replay(records: &[TransitionRecord]) -> Result<Self, TransitionError> {
        let first = records.first().ok_or(TransitionError::TraceEmpty)?;
        let last = records.last().ok_or(TransitionError::TraceEmpty)?;
        if first.stage != TransitionStage::Interpreting {
            return Err(TransitionError::TraceOutOfOrder { index: 0 });
        }
        for (index, pair) in records.windows(2).enumerate() {
            if let [previous, current] = pair {
                let in_order = current.transition == first.transition
                    && current.at_ms >= previous.at_ms
                    && previous.stage.next() == Some(current.stage);
                if !in_order {
                    return Err(TransitionError::TraceOutOfOrder { index: index + 1 });
                }
            }
        }
        Ok(Self {
            id: first.transition,
            deadlines: Deadlines::default(),
            stage: last.stage,
            started_ms: first.at_ms,
            stage_started_ms: last.at_ms,
            last_ms: last.at_ms,
            failure: None,
            records: records.to_vec(),
        })
    }

    /// Milliseconds since the transition started; zero when the clock is earlier than the start.
    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.started_ms)
    }

    fn check_clock(&self, now_ms: u64) -> Result<(), TransitionError> {
        if now_ms < self.last_ms {
            return Err(TransitionError::ClockWentBack {
                now_ms,
                last_ms: self.last_ms,
            });
        }
        Ok(())
    }

    fn push_record(&mut self, at_ms: u64, detail: String) {
        self.records.push(TransitionRecord {
            transition: self.id,
            stage: self.stage,
            at_ms,
            detail,
        });
        self.last_ms = at_ms;
    }
}

/// The audit line of one record: `TRANSITION id=<n> stage=<key> at_ms=<ms> detail=<detail>`, with
/// the detail on one line.
pub fn format_record(record: &TransitionRecord) -> String {
    let detail = record.detail.lines().collect::<Vec<_>>().join(" ");
    format!(
        "TRANSITION id={} stage={} at_ms={} detail={detail}",
        record.transition,
        record.stage.key(),
        record.at_ms
    )
}

/// The audit lines of a transition, one per record, in order.
pub fn format_trace(records: &[TransitionRecord]) -> String {
    records
        .iter()
        .map(format_record)
        .collect::<Vec<_>>()
        .join("\n")
}
