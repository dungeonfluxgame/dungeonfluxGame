//! The handle to the owner thread: a bounded mailbox and the typed calls over it.
//!
//! A call sends one command with a one-shot reply and awaits the reply on the caller's runtime; the
//! owner thread itself never awaits. A full mailbox is refused at once with `MailboxFull` (the
//! caller retries): the call never waits for room.
//!
//! The owner's loop runs inside `catch_unwind`. A panic is recorded once as `ENGINE stopped
//! cause=panic` (never with the panic's message, which may hold text), the table is stopped, and
//! every later call, and every call that was waiting, is answered `OwnerPanicked`. Dropping the
//! handle closes the mailbox, which ends the owner's loop by itself; the thread is not joined,
//! because a join on the thread that drops the handle (often an async runtime worker) would block
//! it for as long as the owner's current batch, up to the commit deadline.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

use tokio::sync::{oneshot, watch};

use crate::event_log::EventLog;

use super::adapters::Adapters;
use super::audit::EngineAudit;
use super::audit_fields::record_engine;
use super::commit::CommitStore;
use super::extras::Extras;
use super::limits::{COMMAND_MAILBOX_CAPACITY, LOST_ANSWER_WAIT};
use super::owner::{Command, Owner, Reply};
use super::report::{
    BrokerRequested, BrokerView, ChaosReport, ClockReport, Description, DiceReport, EngineDump,
    EngineError, ForceReport, HealthReport, QueueReport, ServedReply, SubmitReply, ViableReport,
};
use super::report_observe::{ConfigReport, HistoryReport, ReplayReport, TraceReport};
use super::request::{
    AudienceSpec, ChaosRequest, ClockControl, ConfigControl, DiceControl, ForceRequest,
    HistoryRequest, InputRequest, QueueControl, ReplayRequest,
};
use super::setup::{EngineConfig, EngineStartError};

/// Why the owner thread ended without being asked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    Panicked,
}

/// A running owner thread.
pub struct EngineHandle {
    sender: Option<SyncSender<Box<Command>>>,
    ended: watch::Receiver<Option<Ended>>,
}

impl EngineHandle {
    /// Builds the owner (a failure is a typed `EngineStartError`) and starts its thread.
    pub fn start(
        config: EngineConfig,
        adapters: Adapters,
        store: Arc<dyn CommitStore>,
        log: Arc<EventLog>,
    ) -> Result<Self, EngineStartError> {
        Self::start_with_mailbox(config, adapters, store, log, COMMAND_MAILBOX_CAPACITY)
    }

    /// `start` with a chosen mailbox size, so a test can fill it.
    pub fn start_with_mailbox(
        config: EngineConfig,
        adapters: Adapters,
        store: Arc<dyn CommitStore>,
        log: Arc<EventLog>,
        mailbox_capacity: usize,
    ) -> Result<Self, EngineStartError> {
        Self::start_with_extras(
            config,
            adapters,
            store,
            Extras::memory_only(log),
            mailbox_capacity,
        )
    }

    /// `start` with the event sink and the replayer the caller chose (`Extras`).
    pub fn start_with_extras(
        config: EngineConfig,
        adapters: Adapters,
        store: Arc<dyn CommitStore>,
        extras: Extras,
        mailbox_capacity: usize,
    ) -> Result<Self, EngineStartError> {
        let observer = Arc::clone(&extras.sink);
        let owner = Owner::new(config, adapters, store, extras)?;
        let (sender, mailbox) = sync_channel(mailbox_capacity);
        let (ended_sender, ended) = watch::channel(None);
        std::thread::Builder::new()
            .name("df-table-owner".to_owned())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(move || owner.run(mailbox)));
                if outcome.is_err() {
                    record_engine(&observer, &EngineAudit::Panicked);
                    ended_sender.send_replace(Some(Ended::Panicked));
                }
            })
            .map_err(|error| EngineStartError::Thread(error.to_string()))?;
        Ok(Self {
            sender: Some(sender),
            ended,
        })
    }

    /// What a call that lost its owner reports: `OwnerPanicked` once the owner's thread says it
    /// panicked (it says so within moments of the call failing), `fallback` if it never does.
    async fn lost_owner(&self, fallback: EngineError) -> EngineError {
        let mut ended = self.ended.clone();
        let heard = tokio::time::timeout(LOST_ANSWER_WAIT, ended.wait_for(Option::is_some)).await;
        match heard {
            Ok(Ok(_)) => EngineError::OwnerPanicked,
            Ok(Err(_)) | Err(_) => fallback,
        }
    }

    async fn call<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, EngineError> {
        let (reply, answer) = oneshot::channel();
        let sender = self.sender.as_ref().ok_or(EngineError::Stopped)?;
        if let Err(error) = sender.try_send(Box::new(command(reply))) {
            return Err(match error {
                TrySendError::Full(_) => EngineError::MailboxFull,
                TrySendError::Disconnected(_) => self.lost_owner(EngineError::Stopped).await,
            });
        }
        match answer.await {
            Ok(result) => result,
            Err(_) => Err(self.lost_owner(EngineError::NoAnswer).await),
        }
    }

    pub async fn submit(&self, request: InputRequest) -> Result<SubmitReply, EngineError> {
        self.call(|reply| Command::Submit(request, reply)).await
    }

    pub async fn dump(&self) -> Result<EngineDump, EngineError> {
        self.call(Command::Dump).await
    }

    pub async fn viable(&self, seat: u64) -> Result<ViableReport, EngineError> {
        self.call(|reply| Command::Viable(seat, reply)).await
    }

    pub async fn force(&self, request: ForceRequest) -> Result<ForceReport, EngineError> {
        self.call(|reply| Command::Force(request, reply)).await
    }

    pub async fn projection(
        &self,
        audience: AudienceSpec,
    ) -> Result<serde_json::Value, EngineError> {
        self.call(|reply| Command::Projection(audience, reply))
            .await
    }

    /// What a TV (`Tv`) or one seat's phone is served now, as the wire's `MachineView`.
    pub async fn machine_view(
        &self,
        audience: AudienceSpec,
    ) -> Result<df_protocol::common::MachineView, EngineError> {
        self.call(|reply| Command::MachineView(audience, reply))
            .await
    }

    pub async fn queue_show(&self) -> Result<QueueReport, EngineError> {
        self.call(Command::QueueShow).await
    }

    pub async fn queue_control(&self, control: QueueControl) -> Result<ServedReply, EngineError> {
        self.call(|reply| Command::QueueControl(control, reply))
            .await
    }

    pub async fn describe(&self) -> Result<Description, EngineError> {
        self.call(Command::Describe).await
    }

    pub async fn health(&self) -> Result<HealthReport, EngineError> {
        self.call(Command::Health).await
    }

    pub async fn clock(&self, control: ClockControl) -> Result<ClockReport, EngineError> {
        self.call(|reply| Command::Clock(control, reply)).await
    }

    pub async fn dice(&self, control: DiceControl) -> Result<DiceReport, EngineError> {
        self.call(|reply| Command::Dice(control, reply)).await
    }

    pub async fn chaos(&self, request: ChaosRequest) -> Result<ChaosReport, EngineError> {
        self.call(|reply| Command::Chaos(request, reply)).await
    }

    pub async fn broker_queues(&self) -> Result<BrokerView, EngineError> {
        self.call(Command::BrokerQueues).await
    }

    pub async fn broker_ledger(&self) -> Result<BrokerView, EngineError> {
        self.call(Command::BrokerLedger).await
    }

    pub async fn broker_request(&self, purpose: String) -> Result<BrokerRequested, EngineError> {
        self.call(|reply| Command::BrokerRequest(purpose, reply))
            .await
    }

    pub async fn history(&self, request: HistoryRequest) -> Result<HistoryReport, EngineError> {
        self.call(|reply| Command::History(request, reply)).await
    }

    pub async fn trace(&self, operation: String) -> Result<TraceReport, EngineError> {
        self.call(|reply| Command::Trace(operation, reply)).await
    }

    pub async fn replay(&self, request: ReplayRequest) -> Result<ReplayReport, EngineError> {
        self.call(|reply| Command::Replay(request, reply)).await
    }

    pub async fn config(&self, control: ConfigControl) -> Result<ConfigReport, EngineError> {
        self.call(|reply| Command::Config(control, reply)).await
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Closing the mailbox ends the owner's loop; see the module documentation for why the
        // thread is not joined.
        self.sender.take();
    }
}
