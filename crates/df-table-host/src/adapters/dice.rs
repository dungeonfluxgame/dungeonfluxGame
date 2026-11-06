//! The production `Dice` port: one draw point over an entropy source the host chooses, recording
//! every draw so a replay can supply the same dice again (coding-style section 8, pattern 5).
//!
//! It owns the entropy sources (a seeded one for tests and reproducible dev runs, and a system one
//! for play, both std only), the unbiased mapping of a random number to a die face, the record of
//! every draw (`DrawRecord`) and `ReplayDice`, which hands recorded draws back in order. It never
//! rolls inside the rules: the pipeline draws and passes the faces to the rules owner, which only
//! reads them. It fails with a `PortError` for the `Dice` port: `unavailable` when the source
//! cannot produce a face or the record is full (a draw that cannot be recorded cannot be replayed,
//! so it is refused), and `invalid_answer` when a replayed record does not match the question.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::{Mutex, MutexGuard, PoisonError};

use df_table::event::FailureCause;
use df_table::identity::{Correlation, OperationId};
use df_table::ports::{Dice, Draw, DrawRequest, PortError, PortName};

use super::limits::{MAXIMUM_RECORDED_DRAWS, MAXIMUM_REJECTIONS_PER_DRAW};

/// A stream of random 64-bit numbers.
pub trait EntropySource: Send + std::fmt::Debug {
    fn next_u64(&mut self) -> u64;
}

/// A reproducible source: the same seed gives the same numbers (SplitMix64). It is not a security
/// boundary.
#[derive(Debug, Clone)]
pub struct SeededEntropy {
    state: u64,
}

impl SeededEntropy {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl EntropySource for SeededEntropy {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }
}

/// The operating system's entropy through std: a `RandomState` is keyed by the system at
/// creation, and hashing a counter with it is a keyed pseudo-random function. That is enough for
/// tabletop dice and needs no dependency; a host that wants a cryptographic generator supplies its
/// own `EntropySource`.
#[derive(Debug)]
pub struct SystemEntropy {
    keyed: RandomState,
    counter: u64,
}

impl SystemEntropy {
    pub fn new() -> Self {
        Self {
            keyed: RandomState::new(),
            counter: 0,
        }
    }
}

impl Default for SystemEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl EntropySource for SystemEntropy {
    fn next_u64(&mut self) -> u64 {
        self.counter = self.counter.wrapping_add(1);
        let mut hasher = self.keyed.build_hasher();
        hasher.write_u64(self.counter);
        hasher.finish()
    }
}

/// One die that was drawn, with the input it served. Replay re-supplies these in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawRecord {
    /// The draw's place among all draws of this dice, from 1.
    pub sequence: u64,
    pub correlation: Correlation,
    pub draw: Draw,
}

#[derive(Debug)]
struct Recorded {
    source: Box<dyn EntropySource>,
    records: Vec<DrawRecord>,
    /// Draws a refused batch gave back: served again, in order, before the source is asked.
    returned: VecDeque<Draw>,
}

/// Dice over an entropy source that record every draw.
#[derive(Debug)]
pub struct RecordedDice {
    state: Mutex<Recorded>,
    maximum_records: usize,
}

fn unavailable() -> PortError {
    PortError::new(PortName::Dice, FailureCause::Unavailable)
}

impl RecordedDice {
    pub fn new(source: Box<dyn EntropySource>) -> Self {
        Self {
            state: Mutex::new(Recorded {
                source,
                records: Vec::new(),
                returned: VecDeque::new(),
            }),
            maximum_records: MAXIMUM_RECORDED_DRAWS,
        }
    }

    /// Refuses a draw once `maximum_records` are recorded, in place of the default maximum.
    pub fn recording_at_most(mut self, maximum_records: usize) -> Self {
        self.maximum_records = maximum_records;
        self
    }

    pub fn seeded(seed: u64) -> Self {
        Self::new(Box::new(SeededEntropy::new(seed)))
    }

    pub fn from_system_entropy() -> Self {
        Self::new(Box::new(SystemEntropy::new()))
    }

    fn locked(&self) -> MutexGuard<'_, Recorded> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every draw so far, oldest first.
    pub fn records(&self) -> Vec<DrawRecord> {
        self.locked().records.clone()
    }

    /// The draws that served one input, oldest first.
    pub fn records_of(&self, operation: OperationId) -> Vec<DrawRecord> {
        self.locked()
            .records
            .iter()
            .filter(|record| record.correlation.operation_id == operation)
            .copied()
            .collect()
    }
}

/// A face from 1 to `sides`, without modulo bias: a number from the unevenly sized last block of
/// the range is thrown away and another is taken.
fn face(source: &mut dyn EntropySource, sides: u16) -> Option<u16> {
    let span = u64::from(sides);
    let accepted_below = (u64::MAX / span) * span;
    (0..MAXIMUM_REJECTIONS_PER_DRAW)
        .map(|_| source.next_u64())
        .find(|number| *number < accepted_below)
        .and_then(|number| u16::try_from(number % span + 1).ok())
}

impl Dice for RecordedDice {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        let mut state = self.locked();
        if state.records.len() >= self.maximum_records {
            return Err(unavailable());
        }
        // A draw a refused batch gave back is served again if it is the die that is asked for; a
        // question that differs means the retried input is not the one that drew it, so the rest
        // of what was given back is dropped and the source speaks.
        let again = match state.returned.pop_front() {
            Some(back) if back.sides() == request.sides => Some(back),
            Some(_) => {
                state.returned.clear();
                None
            }
            None => None,
        };
        let draw = match again {
            Some(back) => back,
            None => {
                let value =
                    face(state.source.as_mut(), request.sides.get()).ok_or_else(unavailable)?;
                Draw::new(request.sides, value).ok_or_else(unavailable)?
            }
        };
        let sequence = u64::try_from(state.records.len())
            .ok()
            .and_then(|recorded| recorded.checked_add(1))
            .ok_or_else(unavailable)?;
        state.records.push(DrawRecord {
            sequence,
            correlation,
            draw,
        });
        Ok(draw)
    }

    fn give_back(&self, correlation: Correlation, draws: &[Draw]) {
        let mut state = self.locked();
        // The records of the refused input go with its draws: the retried input records its own.
        for _ in draws {
            if state
                .records
                .last()
                .is_some_and(|last| last.correlation.operation_id == correlation.operation_id)
            {
                state.records.pop();
            }
        }
        for draw in draws.iter().rev() {
            state.returned.push_front(*draw);
        }
    }
}

/// Dice that hand back recorded draws in order and never roll. A question that does not match the
/// next record (different sides, or a different input) is `invalid_answer`, and running out of
/// records is `unavailable`: recovery never draws again.
#[derive(Debug)]
pub struct ReplayDice {
    remaining: Mutex<VecDeque<DrawRecord>>,
}

impl ReplayDice {
    pub fn new(records: impl IntoIterator<Item = DrawRecord>) -> Self {
        Self {
            remaining: Mutex::new(records.into_iter().collect()),
        }
    }

    pub fn remaining(&self) -> usize {
        self.remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Dice for ReplayDice {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let next = remaining.front().ok_or_else(unavailable)?;
        let matches = next.draw.sides() == request.sides
            && next.correlation.operation_id == correlation.operation_id;
        if !matches {
            return Err(PortError::new(PortName::Dice, FailureCause::InvalidAnswer));
        }
        Ok(remaining.pop_front().ok_or_else(unavailable)?.draw)
    }

    fn give_back(&self, correlation: Correlation, draws: &[Draw]) {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for draw in draws.iter().rev() {
            remaining.push_front(DrawRecord {
                sequence: 0,
                correlation,
                draw: *draw,
            });
        }
    }
}
