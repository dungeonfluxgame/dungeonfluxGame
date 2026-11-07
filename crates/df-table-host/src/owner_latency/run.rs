//! One run of the owner-thread harness: producers, the real owner queue, and a measured reduce.
//!
//! The owner thread here is a plain `std::thread` that owns an `OwnerQueue` from `df-table` (the
//! real C1 code, over a sink that discards events), the stand-in table state, and the stand-in
//! commit. It admits what the producers send, serves the next input in the queue's total order, and
//! for each served input does what the design says one input costs: `step` the state, project
//! every audience from the new state, commit the checkpoint, and finish the input. Each phase is
//! timed with `std::time::Instant`, which is allowed in this measurement tool and nowhere in the
//! pure crate.
//!
//! What the numbers leave out: a real `step` (CHECK, DECIDE and APPLY of the rules), the sockets
//! that carry views away, the broker, and contention from the rest of the host. They are floors for
//! the owner thread, not predictions of a whole turn.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use df_table::config::Settings;
use df_table::event::TableEvent;
use df_table::identity::{OperationId, SeatId, TableId};
use df_table::outcome::OutcomeKind;
use df_table::owner_queue::{Lane, OwnerQueue};
use df_table::ports::{ActionId, EventSink};
use df_types::saturating_millis;
use serde::{Deserialize, Serialize};

use super::commit::{CommitMode, Committer};
use super::error::HarnessError;
use super::limits::{SEAT_PENDING_LIMIT, UNLIMITED_RATE};
use super::producers::{self, Arrival, ProducerContext, SendLimit, Submission};
use super::projection::project_all;
use super::state::{AUDIENCE_COUNT, SEAT_COUNT, TableState, WorkloadProfile};
use super::statistics::nanoseconds;

/// Whether a reader still holds the previous snapshot when `step` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    /// No reader holds it, so `step` mutates the state in place.
    InPlace,
    /// A published snapshot is still held, so `step` deep-copies the state first (copy on write).
    CopyOnWrite,
}

impl SnapshotMode {
    pub const ALL: [Self; 2] = [Self::InPlace, Self::CopyOnWrite];

    /// The serde name of the mode.
    pub fn key(self) -> &'static str {
        match self {
            Self::InPlace => "in_place",
            Self::CopyOnWrite => "copy_on_write",
        }
    }

    /// The mode whose `key()` is `text`, if any.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == text)
    }
}

/// When a run stops sending, counted in inputs that are measured (after the warm-up) or in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    AfterInputs(usize),
    AfterSeconds(u64),
}

/// Everything that defines one run.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub profile: WorkloadProfile,
    pub commit: CommitMode,
    pub checkpoint_bytes: usize,
    pub snapshot: SnapshotMode,
    pub arrival: Arrival,
    pub stop: Stop,
    pub warmup_inputs: usize,
    pub directory: PathBuf,
}

/// What the owner thread measured for one served input; times are nanoseconds since the run began
/// or durations as named.
#[derive(Debug, Clone, Copy)]
pub struct InputRecord {
    pub turn_seq: u64,
    pub submitted_ns: u64,
    pub serve_started_ns: u64,
    pub finished_ns: u64,
    pub serve_ns: u64,
    pub step_ns: u64,
    pub projection_ns: u64,
    pub projection_per_audience_ns: [u64; AUDIENCE_COUNT],
    pub commit_ns: u64,
    pub serialise_ns: u64,
    pub hash_ns: u64,
    pub store_ns: u64,
    pub finish_ns: u64,
}

impl InputRecord {
    /// From handing the input over to the start of serving it.
    pub fn queue_wait_ns(&self) -> u64 {
        self.serve_started_ns.saturating_sub(self.submitted_ns)
    }

    /// The owner's time for the input: serve, step, project, commit and finish.
    pub fn reduce_ns(&self) -> u64 {
        self.finished_ns.saturating_sub(self.serve_started_ns)
    }

    /// From handing the input over to the end of the reduce, before any view reaches a socket.
    pub fn end_to_end_ns(&self) -> u64 {
        self.finished_ns.saturating_sub(self.submitted_ns)
    }
}

/// The records of a run after its warm-up, and what the run learned about itself.
#[derive(Debug)]
pub struct RunResult {
    pub config: RunConfig,
    pub records: Vec<InputRecord>,
    pub warmup_served: usize,
    pub refused: u64,
    pub duration_ns: u64,
    pub phone_view_bytes: usize,
    pub tv_view_bytes: usize,
    pub checkpoint_bytes: usize,
}

struct DiscardingSink;

impl EventSink for DiscardingSink {
    fn record(&self, _event: &TableEvent) {}
}

fn queue_settings() -> Result<Settings, HarnessError> {
    let configuration = serde_json::json!({
        "seat_pending_limit": SEAT_PENDING_LIMIT,
        "seat_rate_burst": UNLIMITED_RATE,
        "seat_rate_sustained_per_s": UNLIMITED_RATE,
    });
    let object = configuration.as_object().cloned().unwrap_or_default();
    Settings::load(&object)
        .map(|loaded| loaded.settings)
        .map_err(HarnessError::Settings)
}

struct Owner {
    config: RunConfig,
    origin: Instant,
    queue: OwnerQueue,
    state: Arc<TableState>,
    published: Option<Arc<TableState>>,
    committer: Committer,
    submitted: HashMap<OperationId, u64>,
    tokens: Vec<Sender<()>>,
    records: Vec<InputRecord>,
    refused: u64,
    phone_view_bytes: usize,
    tv_view_bytes: usize,
}

impl Owner {
    fn new(
        config: &RunConfig,
        origin: Instant,
        tokens: Vec<Sender<()>>,
    ) -> Result<Self, HarnessError> {
        let mut queue = OwnerQueue::new(
            TableId::new(1),
            &queue_settings()?,
            Arc::new(DiscardingSink),
        )
        .map_err(HarnessError::Queue)?;
        for seat in 0..SEAT_COUNT {
            queue.join_seat(SeatId::new(seat as u64 + 1), 0)?;
        }
        let state = TableState::generate(&config.profile);
        let committer = Committer::new(
            config.commit,
            &config.directory,
            config.checkpoint_bytes,
            &state,
        )?;
        Ok(Self {
            config: config.clone(),
            origin,
            queue,
            state: Arc::new(state),
            published: None,
            committer,
            submitted: HashMap::new(),
            tokens,
            records: Vec::new(),
            refused: 0,
            phone_view_bytes: 0,
            tv_view_bytes: 0,
        })
    }

    fn now_ms(&self) -> u64 {
        saturating_millis(self.origin.elapsed())
    }

    fn admit(&mut self, submission: Submission) {
        let operation = submission.envelope.operation_id();
        let seat = submission.envelope.seat();
        match self.queue.admit(submission.envelope, self.now_ms()) {
            Ok(_) => {
                self.submitted.insert(operation, submission.submitted_ns);
            }
            Err(_) => {
                self.refused += 1;
                self.return_token(seat);
            }
        }
    }

    fn return_token(&self, seat: Option<SeatId>) {
        let index = seat.and_then(|seat| usize::try_from(seat.get().checked_sub(1)?).ok());
        if let Some(sender) = index.and_then(|index| self.tokens.get(index)) {
            sender.send(()).ok();
        }
    }

    fn waiting(&self) -> usize {
        Lane::ALL.iter().map(|&lane| self.queue.depth(lane)).sum()
    }

    fn drain(&mut self, mailbox: &Receiver<Submission>) -> bool {
        loop {
            match mailbox.try_recv() {
                Ok(submission) => self.admit(submission),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn run(mut self, mailbox: &Receiver<Submission>) -> Result<Self, HarnessError> {
        let mut open = true;
        while open || self.waiting() > 0 {
            if open && self.waiting() == 0 {
                match mailbox.recv() {
                    Ok(submission) => self.admit(submission),
                    Err(_) => open = false,
                }
            }
            if open {
                open = self.drain(mailbox);
            }
            self.reduce_next()?;
        }
        Ok(self)
    }

    fn reduce_next(&mut self) -> Result<(), HarnessError> {
        let serve_started_ns = nanoseconds(self.origin.elapsed());
        let started = Instant::now();
        let Some(served) = self.queue.serve_next(self.now_ms())? else {
            return Ok(());
        };
        let serve_ns = nanoseconds(started.elapsed());
        let turn = served.turn_seq.get();

        let started = Instant::now();
        self.step(turn);
        let step_ns = nanoseconds(started.elapsed());

        let started = Instant::now();
        let projected = project_all(&self.state, &self.config.profile)?;
        let projection_ns = nanoseconds(started.elapsed());
        self.phone_view_bytes = projected.views.first().map_or(0, Vec::len);
        self.tv_view_bytes = projected.views.last().map_or(0, Vec::len);
        std::hint::black_box(&projected.views);

        let started = Instant::now();
        let committed = self.committer.commit(&self.state, turn)?;
        let commit_ns = nanoseconds(started.elapsed());

        let started = Instant::now();
        self.queue.finish(
            served.envelope.sender(),
            served.envelope.operation_id(),
            OutcomeKind::Applied,
        )?;
        let finish_ns = nanoseconds(started.elapsed());
        let finished_ns = nanoseconds(self.origin.elapsed());

        let submitted_ns = self
            .submitted
            .remove(&served.envelope.operation_id())
            .unwrap_or(serve_started_ns);
        self.records.push(InputRecord {
            turn_seq: turn,
            submitted_ns,
            serve_started_ns,
            finished_ns,
            serve_ns,
            step_ns,
            projection_ns,
            projection_per_audience_ns: projected.per_audience_ns,
            commit_ns,
            serialise_ns: committed.serialise_ns,
            hash_ns: committed.hash_ns,
            store_ns: committed.store_ns,
            finish_ns,
        });
        self.return_token(served.envelope.seat());
        Ok(())
    }

    fn step(&mut self, turn: u64) {
        match self.config.snapshot {
            SnapshotMode::InPlace => self.published = None,
            SnapshotMode::CopyOnWrite => {}
        }
        Arc::make_mut(&mut self.state).step(&self.config.profile, turn);
        if self.config.snapshot == SnapshotMode::CopyOnWrite {
            self.published = Some(Arc::clone(&self.state));
        }
    }
}

fn send_limit(config: &RunConfig) -> SendLimit {
    match config.stop {
        Stop::AfterInputs(measured) => {
            SendLimit::Inputs(measured.saturating_add(config.warmup_inputs))
        }
        Stop::AfterSeconds(seconds) => SendLimit::Until(Duration::from_secs(seconds)),
    }
}

/// Runs one scenario to completion: producers on their own threads, the owner on its own, and the
/// calling thread waiting for both. Every input is served before it returns.
pub fn run(config: &RunConfig) -> Result<RunResult, HarnessError> {
    let action = ActionId::new("look").map_err(HarnessError::Action)?;
    let (mailbox_sender, mailbox) = mpsc::channel::<Submission>();
    let (token_senders, token_receivers) = token_channels(config.arrival);
    let origin = Instant::now();
    let owner = Owner::new(config, origin, token_senders)?;
    let context = ProducerContext {
        origin,
        limit: send_limit(config),
        action,
        operations: Arc::new(AtomicU64::new(0)),
    };
    let owner_thread = std::thread::spawn(move || owner.run(&mailbox));
    let producers = producers::spawn(&context, config.arrival, mailbox_sender, token_receivers);
    for producer in producers {
        producer
            .join()
            .map_err(|_| HarnessError::ThreadPanicked { thread: "producer" })?;
    }
    let owner = owner_thread
        .join()
        .map_err(|_| HarnessError::ThreadPanicked { thread: "owner" })??;
    let duration_ns = nanoseconds(origin.elapsed());
    let warmup_served = config.warmup_inputs.min(owner.records.len());
    let records = owner
        .records
        .get(warmup_served..)
        .unwrap_or_default()
        .to_vec();
    if records.is_empty() {
        return Err(HarnessError::NothingMeasured);
    }
    Ok(RunResult {
        config: config.clone(),
        records,
        warmup_served,
        refused: owner.refused,
        duration_ns,
        phone_view_bytes: owner.phone_view_bytes,
        tv_view_bytes: owner.tv_view_bytes,
        checkpoint_bytes: owner.committer.document_bytes(),
    })
}

fn token_channels(arrival: Arrival) -> (Vec<Sender<()>>, Vec<Receiver<()>>) {
    let Arrival::Saturated { window } = arrival else {
        return (Vec::new(), Vec::new());
    };
    (0..SEAT_COUNT)
        .map(|_| {
            let (sender, receiver) = mpsc::channel();
            for _ in 0..window {
                sender.send(()).ok();
            }
            (sender, receiver)
        })
        .unzip()
}
