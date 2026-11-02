//! The owner actor: one dedicated thread that owns the table's queue, machines and committed state.
//!
//! All mutable state of the table lives in `Owner` and is touched only by its thread; routes and
//! the command line reach it through a bounded mailbox and wait for a one-shot answer, so the
//! owner thread never holds a lock a request thread can block on and never awaits (the host owns
//! every `.await`; the owner thread owns none). Its loop takes a command or, when none comes for
//! `IDLE_TICK`, ticks: it reaps inputs whose lease ran out, pumps the broker's fake worker, turns
//! settled resource requests into engine inputs, and serves what waits.
//!
//! Serving is group commit (S1 decision, see `commit`), and the batch is formed from the mailbox:
//! when a submit arrives and the owner is serving, it takes every submit already waiting in the
//! mailbox (up to the policy's time slice and count), admits them all, serves them in the queue's
//! order against a copy of the committed world, writes ONE checkpoint, and only then answers each
//! submit with its own turn reports, which carry that commit's id. A commit that fails or misses
//! its deadline fails the whole batch, drops the copy, releases the operation ids so the same ids
//! may be sent again, and makes the owner refuse new input `table_busy` for a backoff.
//!
//! What it never does: block on a store (the committer bounds the wait), spawn work, read the
//! system clock (the `Clock` port does), or draw a die (the `Dice` port does). Every dev call is
//! audited where it is handled, as one `EngineAudit` line.

mod batch;
mod force;
mod reads;
mod save;
mod submit;

use std::collections::VecDeque;
use std::num::NonZeroU128;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};

use df_table::identity::{Correlation, OperationId, RecoveryEpoch, SeatId, SessionRevision};
use df_table::owner_queue::{Lane, OwnerQueue};
use df_table::pipeline::PipelineTable;
use df_table::ports::EventSink;
use df_table::registry::Registry;
use tokio::sync::oneshot;

use crate::event_log::EventLog;

use super::adapters::Adapters;
use super::audit::{AuditSink, EngineAudit};
use super::commit::{CommitId, CommitStore};
use super::committer::Committer;
use super::limits::IDLE_TICK;
use super::report::{
    BrokerRequested, BrokerView, ChaosReport, ClockReport, Description, DiceReport, EngineDump,
    EngineError, ForceReport, HealthReport, QueueReport, ServedReply, SubmissionReport,
    SubmitReply, TurnReport, ViableReport,
};
use super::request::{
    AudienceSpec, ChaosRequest, ClockControl, DiceControl, ForceRequest, InputRequest, QueueControl,
};
use super::setup::{ActionCatalog, EngineConfig, EngineStartError, dev_machines, starting_world};

/// The one-shot answer to a command.
pub(super) type Reply<T> = oneshot::Sender<Result<T, EngineError>>;

/// What the routes and the command line ask the owner to do.
pub(super) enum Command {
    Submit(InputRequest, Reply<SubmitReply>),
    Dump(Reply<EngineDump>),
    Viable(u64, Reply<ViableReport>),
    Force(ForceRequest, Reply<ForceReport>),
    Projection(AudienceSpec, Reply<serde_json::Value>),
    QueueShow(Reply<QueueReport>),
    QueueControl(QueueControl, Reply<ServedReply>),
    Describe(Reply<Description>),
    Health(Reply<HealthReport>),
    Clock(ClockControl, Reply<ClockReport>),
    Dice(DiceControl, Reply<DiceReport>),
    Chaos(ChaosRequest, Reply<ChaosReport>),
    BrokerQueues(Reply<BrokerView>),
    BrokerLedger(Reply<BrokerView>),
    BrokerRequest(String, Reply<BrokerRequested>),
}

/// How commits have gone, for `queue show`.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct CommitStats {
    pub(super) commits: u64,
    pub(super) failures: u64,
    pub(super) last_bytes: Option<usize>,
}

/// The table's owner.
pub(super) struct Owner {
    pub(super) config: EngineConfig,
    pub(super) registry: Registry,
    pub(super) catalog: ActionCatalog,
    pub(super) adapters: Adapters,
    pub(super) sink: Arc<dyn EventSink>,
    pub(super) log: Arc<EventLog>,
    pub(super) committer: Committer,
    pub(super) queue: OwnerQueue,
    pub(super) committed: PipelineTable,
    /// The newest commit that succeeded; its number is the table's revision.
    pub(super) commit_id: CommitId,
    /// The newest commit attempt, successful or not. An attempt's id is never used twice.
    pub(super) last_attempt: CommitId,
    pub(super) held: bool,
    pub(super) backoff_until_ms: Option<u64>,
    pub(super) next_operation: u64,
    pub(super) next_engine_operation: u64,
    pub(super) next_utterance: u64,
    pub(super) recent_turns: VecDeque<TurnReport>,
    pub(super) recent_submissions: VecDeque<SubmissionReport>,
    pub(super) stats: CommitStats,
}

/// The operation ids the owner mints start here for seats and here for the engine, far above any
/// number a person types with `--op`, so the two never meet by accident.
const MINTED_SEAT_OPERATIONS: u128 = 1 << 64;
const MINTED_ENGINE_OPERATIONS: u128 = 2 << 64;

impl Owner {
    pub(super) fn new(
        config: EngineConfig,
        adapters: Adapters,
        store: Arc<dyn CommitStore>,
        log: Arc<EventLog>,
    ) -> Result<Self, EngineStartError> {
        let machines = dev_machines(config.first_scene)?;
        let committed = starting_world(&config, &machines)?;
        let sink: Arc<dyn EventSink> = Arc::new(AuditSink::new(Arc::clone(&log)));
        let mut queue = OwnerQueue::new(config.table, &config.settings, Arc::clone(&sink))?;
        let now_ms = adapters.clock.now_ms();
        for seat in 1..=config.seats {
            queue.join_seat(SeatId::new(seat), now_ms)?;
        }
        let last_attempt = store
            .restore()
            .map_or(CommitId::new(0), |restored| restored.last_attempt);
        Ok(Self {
            held: config.start_held,
            registry: machines.registry,
            catalog: machines.catalog,
            committed,
            config,
            adapters,
            sink,
            log,
            committer: Committer::start(store)?,
            queue,
            commit_id: CommitId::new(0),
            last_attempt,
            backoff_until_ms: None,
            next_operation: 0,
            next_engine_operation: 0,
            next_utterance: 0,
            recent_turns: VecDeque::new(),
            recent_submissions: VecDeque::new(),
            stats: CommitStats::default(),
        })
    }

    /// The thread's loop: handle a command, or tick when none comes. It ends when every sender of
    /// the mailbox is gone.
    pub(super) fn run(mut self, mailbox: Receiver<Box<Command>>) {
        let mut carried: Option<Command> = None;
        loop {
            let command = match carried.take() {
                Some(command) => command,
                None => match mailbox.recv_timeout(IDLE_TICK) {
                    Ok(command) => *command,
                    Err(RecvTimeoutError::Timeout) => {
                        self.tick();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
            };
            carried = self.dispatch(command, &mailbox);
        }
        self.shut_down();
    }

    /// Handles one command. A submit to an owner that is serving collects every submit waiting
    /// behind it into one batch; the first command that is not a submit ends the collection and
    /// is returned to be handled next.
    fn dispatch(&mut self, command: Command, mailbox: &Receiver<Box<Command>>) -> Option<Command> {
        self.keep_house();
        match command {
            Command::Submit(request, reply) if !self.held => {
                self.serve_submits((request, reply), mailbox)
            }
            other => {
                self.handle(other);
                None
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Submit(request, reply) => {
                let result = self.submit_held(request);
                answer(reply, result);
            }
            Command::Dump(reply) => answer(reply, self.dump()),
            Command::Viable(seat, reply) => answer(reply, self.viable(seat)),
            Command::Force(request, reply) => answer(reply, self.force(request)),
            Command::Projection(audience, reply) => answer(reply, self.projection(audience)),
            Command::QueueShow(reply) => answer(reply, Ok(self.queue_show())),
            Command::QueueControl(control, reply) => answer(reply, self.control_queue(control)),
            Command::Describe(reply) => answer(reply, Ok(self.describe())),
            Command::Health(reply) => answer(reply, Ok(self.health())),
            Command::Clock(control, reply) => answer(reply, self.clock(control)),
            Command::Dice(control, reply) => answer(reply, self.dice(control)),
            Command::Chaos(request, reply) => answer(reply, self.chaos(request)),
            Command::BrokerQueues(reply) => answer(reply, Ok(self.broker_queues())),
            Command::BrokerLedger(reply) => answer(reply, Ok(self.broker_ledger())),
            Command::BrokerRequest(purpose, reply) => answer(reply, self.broker_request(&purpose)),
        }
    }

    /// The bounded work the owner does around every command and tick: abandon inputs whose lease
    /// ran out, let the broker's worker run, and queue the resource requests that settled.
    fn keep_house(&mut self) {
        let now_ms = self.adapters.clock.now_ms();
        self.queue.reap_expired(now_ms);
        self.adapters.broker.worker.pump(now_ms);
        self.admit_resource_results(now_ms);
    }

    /// Runs when nothing has asked for a while: the housekeeping, then whatever waits.
    fn tick(&mut self) {
        self.keep_house();
        if !self.held {
            self.drain();
        }
    }

    /// The loop ended because the host closed the mailbox: close the broker and say what was left.
    fn shut_down(&mut self) {
        self.audit(EngineAudit::Stopped {
            dropped_inputs: self.waiting_inputs(),
        });
        self.adapters.broker.hub.close();
    }

    pub(super) fn waiting_inputs(&self) -> usize {
        Lane::ALL.iter().map(|lane| self.queue.depth(*lane)).sum()
    }

    pub(super) fn audit(&self, audit: EngineAudit) {
        self.log.push(audit.render_audit_line());
    }

    pub(super) fn now_ms(&self) -> u64 {
        self.adapters.clock.now_ms()
    }

    /// The revision of the table: the number of the newest commit that succeeded.
    pub(super) fn revision(&self) -> u64 {
        self.commit_id.get()
    }

    pub(super) fn session_revision(&self) -> SessionRevision {
        SessionRevision::new(RecoveryEpoch::FIRST, self.revision())
    }

    /// The correlation of work the owner does for no served input, such as a forced change.
    pub(super) fn correlation_for(&self, operation_id: OperationId) -> Correlation {
        Correlation {
            operation_id,
            turn_seq: self.queue.last_turn_seq(),
            revision: self.session_revision(),
        }
    }

    pub(super) fn mint_seat_operation(&mut self) -> OperationId {
        self.next_operation = self.next_operation.saturating_add(1);
        minted(MINTED_SEAT_OPERATIONS, self.next_operation)
    }

    pub(super) fn mint_engine_operation(&mut self) -> OperationId {
        self.next_engine_operation = self.next_engine_operation.saturating_add(1);
        minted(MINTED_ENGINE_OPERATIONS, self.next_engine_operation)
    }
}

fn minted(base: u128, counter: u64) -> OperationId {
    OperationId::from_nonzero_u128(NonZeroU128::MIN.saturating_add(base + u128::from(counter)))
}

/// Sends the answer. A caller that stopped waiting has nobody left to tell, so a failed send is
/// not an error.
pub(super) fn answer<T>(reply: Reply<T>, value: Result<T, EngineError>) {
    let _ = reply.send(value);
}
