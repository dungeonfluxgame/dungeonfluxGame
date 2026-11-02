//! The threads that feed the owner: the six phones, standing in for the host's socket tasks.
//!
//! A producer builds a tap envelope, stamps the time it was handed over, and sends it down the
//! mailbox channel to the owner thread, which is where the host's connection tasks would send. Two
//! kinds exist. A scheduled producer sends on a fixed timetable, whether or not the owner keeps up,
//! so queue wait grows when the owner is slow (the honest open-loop case). Saturating producers,
//! one per seat, keep a fixed number of inputs outstanding and send another when one finishes,
//! which keeps the owner busy without limit and measures its speed flat out.

use std::num::NonZeroU128;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use df_table::envelope::{InputEnvelope, PlayerInput, ViewStamp};
use df_table::identity::{OperationId, RecoveryEpoch, SeatId, SessionRevision};
use df_table::ports::ActionId;

use super::state::SEAT_COUNT;
use super::statistics::nanoseconds;

/// How early a producer wakes before its due time and spins the rest, to keep the arrival time
/// close to the schedule on a coarse timer.
const SPIN_MARGIN: Duration = Duration::from_micros(300);

/// An input on its way to the owner, with the time it was handed over.
pub struct Submission {
    pub envelope: InputEnvelope,
    pub submitted_ns: u64,
}

/// How inputs arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// One input at a time at a fixed rate, the seats taking turns.
    Steady { inputs_per_s: u32 },
    /// `inputs` at once every `period_ms`, the seats taking turns: six at once is every seat
    /// submitting together.
    Burst { inputs: usize, period_ms: u64 },
    /// Each seat keeps `window` inputs outstanding.
    Saturated { window: usize },
}

/// When a producer stops sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendLimit {
    Inputs(usize),
    Until(Duration),
}

/// Everything a producer thread shares with the others.
#[derive(Clone)]
pub struct ProducerContext {
    pub origin: Instant,
    pub limit: SendLimit,
    pub action: ActionId,
    pub operations: Arc<AtomicU64>,
}

impl ProducerContext {
    fn elapsed_ns(&self) -> u64 {
        nanoseconds(self.origin.elapsed())
    }

    fn envelope(&self, seat_index: usize) -> Option<InputEnvelope> {
        let number = self.operations.fetch_add(1, Ordering::Relaxed);
        let operation =
            OperationId::from_nonzero_u128(NonZeroU128::MIN.saturating_add(u128::from(number)));
        let view = ViewStamp::new(SessionRevision::new(RecoveryEpoch::FIRST, 1));
        let seat = SeatId::new(u64::try_from(seat_index).ok()?.saturating_add(1));
        Some(InputEnvelope::from_player(
            operation,
            seat,
            view,
            PlayerInput::Tap {
                action: self.action.clone(),
            },
        ))
    }

    fn submit(&self, mailbox: &Sender<Submission>, seat_index: usize) -> bool {
        let Some(envelope) = self.envelope(seat_index) else {
            return false;
        };
        let submission = Submission {
            envelope,
            submitted_ns: self.elapsed_ns(),
        };
        mailbox.send(submission).is_ok()
    }

    fn past_deadline(&self) -> bool {
        match self.limit {
            SendLimit::Inputs(_) => false,
            SendLimit::Until(duration) => self.origin.elapsed() >= duration,
        }
    }

    fn limit_reached(&self, sent: usize) -> bool {
        match self.limit {
            SendLimit::Inputs(count) => sent >= count,
            SendLimit::Until(duration) => self.origin.elapsed() >= duration,
        }
    }
}

fn sleep_until(due: Instant) {
    let remaining = due.saturating_duration_since(Instant::now());
    if remaining > SPIN_MARGIN {
        std::thread::sleep(remaining - SPIN_MARGIN);
    }
    while Instant::now() < due {
        std::hint::spin_loop();
    }
}

fn scheduled(
    context: &ProducerContext,
    mailbox: &Sender<Submission>,
    per_round: usize,
    period: Duration,
) {
    let mut sent = 0usize;
    let mut round = 0u32;
    loop {
        round = round.saturating_add(1);
        sleep_until(context.origin + period.saturating_mul(round));
        for _ in 0..per_round {
            if context.limit_reached(sent) || !context.submit(mailbox, sent % SEAT_COUNT) {
                return;
            }
            sent += 1;
        }
    }
}

fn saturating(
    context: &ProducerContext,
    mailbox: &Sender<Submission>,
    seat_index: usize,
    tokens: &Receiver<()>,
    seat_quota: usize,
) {
    let mut sent = 0usize;
    while sent < seat_quota && !context.past_deadline() {
        if tokens.recv().is_err() || !context.submit(mailbox, seat_index) {
            return;
        }
        sent += 1;
    }
}

/// Starts the producers for `arrival`. `tokens` holds one receiver per seat for `Saturated` and is
/// unused otherwise. The mailbox closes when the last producer finishes.
pub fn spawn(
    context: &ProducerContext,
    arrival: Arrival,
    mailbox: Sender<Submission>,
    tokens: Vec<Receiver<()>>,
) -> Vec<JoinHandle<()>> {
    match arrival {
        Arrival::Steady { inputs_per_s } => {
            let period = Duration::from_secs(1) / inputs_per_s.max(1);
            vec![spawn_scheduled(context.clone(), mailbox, 1, period)]
        }
        Arrival::Burst { inputs, period_ms } => {
            let period = Duration::from_millis(period_ms.max(1));
            vec![spawn_scheduled(
                context.clone(),
                mailbox,
                inputs.max(1),
                period,
            )]
        }
        Arrival::Saturated { .. } => {
            let seat_quota = match context.limit {
                SendLimit::Inputs(count) => count.div_ceil(SEAT_COUNT),
                SendLimit::Until(_) => usize::MAX,
            };
            tokens
                .into_iter()
                .enumerate()
                .map(|(seat_index, receiver)| {
                    let context = context.clone();
                    let mailbox = mailbox.clone();
                    std::thread::spawn(move || {
                        saturating(&context, &mailbox, seat_index, &receiver, seat_quota);
                    })
                })
                .collect()
        }
    }
}

fn spawn_scheduled(
    context: ProducerContext,
    mailbox: Sender<Submission>,
    per_round: usize,
    period: Duration,
) -> JoinHandle<()> {
    std::thread::spawn(move || scheduled(&context, &mailbox, per_round, period))
}
