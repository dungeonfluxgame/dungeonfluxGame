//! Native synthetic telemetry foundation. This is not an authenticated game collector.
#![cfg(not(target_arch = "wasm32"))]
mod clock;
#[cfg(test)]
mod configured_tests;
mod limits;
pub use clock::StoreClock;
use limits::{
    BARRIER_RELEASE_TIMEOUT, DEGRADED_LAG_MS, MAXIMUM_EMERGENCY_FILE_BYTES,
    MAXIMUM_EMERGENCY_RECORDS, SINK_RETRY_BACKOFF, WARNING_LAG_MS, WORKER_IDLE_POLL,
};
pub use limits::{MAXIMUM_PAGE_ROWS, MAXIMUM_ROOT_RECORDS};
mod ownership;
mod persistence;
mod query;
mod wire;
use df_observe::{
    Durability, IngestReceipt, PendingReceipt, Signal, TelemetryError, TelemetryIngress,
    TelemetryLimits,
};
pub use ownership::fixture_root_parent;
use ownership::{OwnedRoot, open_or_create_file};
use persistence::Persistence;
pub use query::{DiagnosticReader, QueryFilter, QueryPage, RecordView, SourceWatermark};
use std::{
    fmt,
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wire::Batch;

/// Controlled synthetic barrier; never enabled in normal native capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarrierStage {
    BeforeSpool,
    Spooled,
    CommittedBeforeCheckpoint,
}
/// Pause point for crash tests: the worker reports `stage`, then waits for `release`.
pub struct FailureBarrier {
    pub stage: BarrierStage,
    pub reached: mpsc::SyncSender<BarrierStage>,
    pub release: mpsc::Receiver<()>,
}
/// Health classification from the age of the oldest uncommitted spool frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Healthy,
    Warning,
    Degraded,
}
/// Point-in-time store counters; synthetic health grants no game admission authority.
#[derive(Debug, Clone)]
pub struct IngestionHealth {
    pub admitted: u64,
    pub spooled: u64,
    pub committed: u64,
    /// Backlog commit attempts, including first attempts after spooling.
    pub retries: u64,
    pub duplicates: u64,
    pub rejected: u64,
    /// Refusals and commit failures caused by a full queue, spool or SQLite budget.
    pub capacity: u64,
    /// Places where completeness cannot be confirmed (refusals, lost receipts, overflow).
    pub unconfirmed_gaps: u64,
    pub pending_items: usize,
    pub pending_bytes: usize,
    pub backlog_files: usize,
    pub spool_bytes: u64,
    /// Last spool position durably committed to SQLite.
    pub checkpoint: u64,
    pub committed_watermark: u64,
    /// Injected clock time of the last commit, in milliseconds.
    pub last_commit_millis: Option<u64>,
    /// Milliseconds the oldest uncommitted spool frame has waited.
    pub lag_millis: u64,
    pub readiness: Readiness,
    /// Emergency lines written to the bounded fallback file.
    pub emergency_records: u64,
}
#[derive(Default, Debug)]
struct Counters {
    admitted: AtomicU64,
    spooled: AtomicU64,
    committed: AtomicU64,
    retries: AtomicU64,
    duplicates: AtomicU64,
    rejected: AtomicU64,
    capacity: AtomicU64,
    gaps: AtomicU64,
    items: AtomicUsize,
    bytes: AtomicUsize,
    backlog: AtomicUsize,
    spool_bytes: AtomicU64,
    checkpoint: AtomicU64,
    watermark: AtomicU64,
    clock: AtomicU64,
    last_commit: AtomicU64,
    first_backlog: AtomicU64,
    emergency: AtomicU64,
    closed: AtomicBool,
}
struct Admission {
    batch: Batch,
    cost: usize,
    reply: mpsc::Sender<Result<IngestReceipt, TelemetryError>>,
}
enum Command {
    Admit(Admission),
    Flush(mpsc::Sender<Result<(), TelemetryError>>),
    Shutdown(mpsc::Sender<Result<(), TelemetryError>>),
}
/// Cloneable fail-fast ingress. All I/O is confined to its Store worker.
#[derive(Clone)]
pub struct LocalIngress {
    sender: mpsc::SyncSender<Command>,
    counters: Arc<Counters>,
    limits: TelemetryLimits,
}
impl fmt::Debug for LocalIngress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIngress")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl TelemetryIngress for LocalIngress {
    fn submit(&self, signal: Signal, bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        if self.counters.closed.load(Ordering::Acquire) {
            return Err(TelemetryError::Closed);
        }
        let batch = match Batch::decode(signal, bytes, self.limits) {
            Ok(batch) => batch,
            Err(error) => {
                reject(&self.counters, &error);
                return Err(error);
            }
        };
        // Bound owned queue payload plus canonical records; decoder's fixed field/depth
        // preflight separately bounds expansion. This is not a whole-process RAM bound.
        let cost = match batch.queue_cost() {
            Ok(cost) => cost,
            Err(error) => {
                reject(&self.counters, &error);
                return Err(error);
            }
        };
        let reserve = |counter: &AtomicUsize, amount: usize, limit: usize| {
            counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(amount).filter(|next| *next <= limit)
            })
        };
        if reserve(&self.counters.items, 1, self.limits.queue_items).is_err() {
            reject(&self.counters, &TelemetryError::Capacity);
            return Err(TelemetryError::Capacity);
        }
        if reserve(&self.counters.bytes, cost, self.limits.queue_bytes).is_err() {
            self.counters.items.fetch_sub(1, Ordering::AcqRel);
            reject(&self.counters, &TelemetryError::Capacity);
            return Err(TelemetryError::Capacity);
        }
        let (reply, pending) = PendingReceipt::channel();
        match self
            .sender
            .try_send(Command::Admit(Admission { batch, cost, reply }))
        {
            Ok(()) => {
                self.counters.admitted.fetch_add(1, Ordering::Relaxed);
                Ok(pending)
            }
            Err(error) => {
                self.counters.items.fetch_sub(1, Ordering::AcqRel);
                self.counters.bytes.fetch_sub(cost, Ordering::AcqRel);
                let error = match error {
                    mpsc::TrySendError::Full(_) => TelemetryError::Capacity,
                    mpsc::TrySendError::Disconnected(_) => TelemetryError::Closed,
                };
                reject(&self.counters, &error);
                Err(error)
            }
        }
    }
}
fn reject(counters: &Counters, error: &TelemetryError) {
    counters.rejected.fetch_add(1, Ordering::Relaxed);
    counters.gaps.fetch_add(1, Ordering::Relaxed);
    if *error == TelemetryError::Capacity {
        counters.capacity.fetch_add(1, Ordering::Relaxed);
    }
}
/// Sole owner of a native durable root and writer thread. Keep this owner alive after a
/// shutdown deadline until retry succeeds or the fixture process owner terminates it.
pub struct Store {
    ingress: LocalIngress,
    worker: Option<JoinHandle<()>>,
    shutdown_reply: Option<mpsc::Receiver<Result<(), TelemetryError>>>,
}
impl Store {
    /// Open a fixture store. The root must be a fresh or owned `ownedfixture-*` directory
    /// directly under [`fixture_root_parent`]; `barrier` pauses the worker for crash tests.
    pub fn open(
        path: &Path,
        limits: TelemetryLimits,
        barrier: Option<FailureBarrier>,
    ) -> Result<Self, TelemetryError> {
        let limits = limits.validate()?;
        Self::start(OwnedRoot::open(path)?, limits, barrier)
    }
    /// Open a store under a root the deployment composition configured. The
    /// fixture-only `open` path restriction does not apply; marker, allowlist,
    /// spool and SQLite bounds are unchanged.
    pub fn open_configured(path: &Path, limits: TelemetryLimits) -> Result<Self, TelemetryError> {
        let limits = limits.validate()?;
        Self::start(OwnedRoot::open_configured(path)?, limits, None)
    }
    fn start(
        root: OwnedRoot,
        limits: TelemetryLimits,
        barrier: Option<FailureBarrier>,
    ) -> Result<Self, TelemetryError> {
        let persistent = Persistence::open(root, limits)?;
        let counters = Arc::new(Counters::default());
        // Never assert completeness of a producer lifetime from local accepted records.
        counters.gaps.store(1, Ordering::Relaxed);
        update(&persistent, &counters);
        let (sender, receiver) = mpsc::sync_channel(limits.queue_items);
        let ingress = LocalIngress {
            sender,
            counters: Arc::clone(&counters),
            limits,
        };
        let worker = thread::Builder::new()
            .name("df-telemetry-spool".into())
            .spawn(move || worker(persistent, receiver, counters, barrier))
            .map_err(|_| TelemetryError::Internal)?;
        Ok(Self {
            ingress,
            worker: Some(worker),
            shutdown_reply: None,
        })
    }
    /// A fail-fast ingress handle; all disk work stays on the store worker.
    pub fn ingress(&self) -> Arc<dyn TelemetryIngress> {
        Arc::new(self.ingress.clone())
    }
    /// Inject monotonic elapsed time; synthetic health has no game admission authority.
    pub fn set_time(&self, elapsed: Duration) -> Result<(), TelemetryError> {
        self.clock().set_time(elapsed)
    }
    /// Clone clock injection without sharing ownership of the disk worker or Store lock.
    pub fn clock(&self) -> StoreClock {
        StoreClock::new(Arc::clone(&self.ingress.counters))
    }
    pub fn health(&self) -> IngestionHealth {
        let c = &self.ingress.counters;
        let backlog = c.backlog.load(Ordering::Acquire);
        let now = c.clock.load(Ordering::Acquire);
        let lag = if backlog == 0 {
            0
        } else {
            now.saturating_sub(c.first_backlog.load(Ordering::Acquire))
        };
        let last = c.last_commit.load(Ordering::Acquire);
        IngestionHealth {
            admitted: c.admitted.load(Ordering::Acquire),
            spooled: c.spooled.load(Ordering::Acquire),
            committed: c.committed.load(Ordering::Acquire),
            retries: c.retries.load(Ordering::Acquire),
            duplicates: c.duplicates.load(Ordering::Acquire),
            rejected: c.rejected.load(Ordering::Acquire),
            capacity: c.capacity.load(Ordering::Acquire),
            unconfirmed_gaps: c.gaps.load(Ordering::Acquire),
            pending_items: c.items.load(Ordering::Acquire),
            pending_bytes: c.bytes.load(Ordering::Acquire),
            backlog_files: backlog,
            spool_bytes: c.spool_bytes.load(Ordering::Acquire),
            checkpoint: c.checkpoint.load(Ordering::Acquire),
            committed_watermark: c.watermark.load(Ordering::Acquire),
            last_commit_millis: if last == 0 { None } else { Some(last - 1) },
            lag_millis: lag,
            readiness: if lag > DEGRADED_LAG_MS {
                Readiness::Degraded
            } else if lag > WARNING_LAG_MS {
                Readiness::Warning
            } else {
                Readiness::Healthy
            },
            emergency_records: c.emergency.load(Ordering::Acquire),
        }
    }
    /// FIFO barrier after earlier admissions; sink failure returns explicitly with durable backlog.
    pub fn flush(&self, timeout: Duration) -> Result<(), TelemetryError> {
        let (reply, receiver) = mpsc::channel();
        self.ingress
            .sender
            .try_send(Command::Flush(reply))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => TelemetryError::Capacity,
                mpsc::TrySendError::Disconnected(_) => TelemetryError::Closed,
            })?;
        receive(receiver, timeout)
    }
    /// Drain the backlog and stop the worker; retry after `Deadline` with the store kept alive.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), TelemetryError> {
        if self.worker.is_none() {
            return Ok(());
        }
        self.ingress.counters.closed.store(true, Ordering::Release);
        if self.shutdown_reply.is_none() {
            let (reply, receiver) = mpsc::channel();
            self.ingress
                .sender
                .try_send(Command::Shutdown(reply))
                .map_err(|_| TelemetryError::Capacity)?;
            self.shutdown_reply = Some(receiver);
        }
        // A timeout too large to add to now (for example `Duration::MAX`) means no deadline.
        let deadline = Instant::now().checked_add(timeout);
        let shutdown_outcome = self
            .shutdown_reply
            .as_ref()
            .ok_or(TelemetryError::Internal)?
            .recv_timeout(timeout)
            .map_err(|_| TelemetryError::Deadline)?;
        while self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(TelemetryError::Deadline);
            }
            thread::yield_now();
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| TelemetryError::Internal)?;
        }
        shutdown_outcome
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        // Signal cancellation even if a caller violates the explicit shutdown contract.
        // Disk syscalls cannot be forcibly canceled safely; process deadlines own that limit.
        self.ingress.counters.closed.store(true, Ordering::Release);
        if self.worker.is_some() && self.shutdown_reply.is_none() {
            let (reply, _) = mpsc::channel();
            if self
                .ingress
                .sender
                .try_send(Command::Shutdown(reply))
                .is_err()
            {
                self.ingress.counters.gaps.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
fn receive(
    receiver: mpsc::Receiver<Result<(), TelemetryError>>,
    timeout: Duration,
) -> Result<(), TelemetryError> {
    receiver
        .recv_timeout(timeout)
        .map_err(|_| TelemetryError::Deadline)?
}
fn update(persistent: &Persistence, counters: &Counters) {
    let old = counters
        .backlog
        .swap(persistent.backlog.len(), Ordering::AcqRel);
    if old == 0 && !persistent.backlog.is_empty() {
        counters
            .first_backlog
            .store(counters.clock.load(Ordering::Acquire), Ordering::Release);
    }
    counters
        .spool_bytes
        .store(persistent.spool_bytes, Ordering::Release);
    counters
        .checkpoint
        .store(persistent.checkpoint, Ordering::Release);
    counters
        .watermark
        .store(persistent.watermark, Ordering::Release);
}
fn barrier_wait(
    barrier: &mut Option<FailureBarrier>,
    stage: BarrierStage,
) -> Result<(), TelemetryError> {
    if barrier
        .as_ref()
        .is_some_and(|barrier| barrier.stage == stage)
    {
        let barrier = barrier.take().ok_or(TelemetryError::Internal)?;
        barrier
            .reached
            .try_send(stage)
            .map_err(|_| TelemetryError::Internal)?;
        barrier
            .release
            .recv_timeout(BARRIER_RELEASE_TIMEOUT)
            .map_err(|_| TelemetryError::Deadline)?;
    }
    Ok(())
}
fn emergency(persistent: &Persistence, counters: &Counters, error: &TelemetryError) {
    // Independent bounded fallback, no OTEL recursion. Recovery query links this file.
    if counters
        .emergency
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAXIMUM_EMERGENCY_RECORDS).then_some(count + 1)
        })
        .is_err()
    {
        counters.gaps.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let line = format!(
        concat!(
            "df-telemetry emergency source=local-spool error={} checkpoint={} backlog={} ",
            "unconfirmed=true\n",
        ),
        error,
        persistent.checkpoint,
        persistent.backlog.len()
    );
    let path = persistent.root.path.join("emergency");
    let write_result = (|| {
        let mut file = open_or_create_file(&path)?;
        let existing_file_bytes = file.metadata().map_err(|_| TelemetryError::Io)?.len();
        let line_bytes = u64::try_from(line.len()).map_err(|_| TelemetryError::Capacity)?;
        if existing_file_bytes.saturating_add(line_bytes) > MAXIMUM_EMERGENCY_FILE_BYTES {
            return Err(TelemetryError::Capacity);
        }
        use std::io::Seek;
        file.seek(std::io::SeekFrom::End(0))
            .map_err(|_| TelemetryError::Io)?;
        file.write_all(line.as_bytes())
            .map_err(|_| TelemetryError::Io)?;
        file.sync_all().map_err(|_| TelemetryError::Io)?;
        ownership::sync_directory(&persistent.root.path)
    })();
    if write_result.is_err() {
        let mut stderr = std::io::stderr().lock();
        if stderr.write_all(line.as_bytes()).is_err() {
            counters.gaps.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn drain_one(
    persistent: &mut Persistence,
    counters: &Counters,
    barrier: &mut Option<FailureBarrier>,
) -> Result<(), TelemetryError> {
    if !persistent.backlog.is_empty() {
        counters.retries.fetch_add(1, Ordering::Relaxed);
        let (position, inserted, duplicates) = persistent.commit_front()?;
        counters
            .committed
            .fetch_add(inserted as u64, Ordering::Relaxed);
        counters
            .duplicates
            .fetch_add(duplicates as u64, Ordering::Relaxed);
        counters.last_commit.store(
            counters.clock.load(Ordering::Acquire).saturating_add(1),
            Ordering::Release,
        );
        barrier_wait(barrier, BarrierStage::CommittedBeforeCheckpoint)?;
        persistent.checkpoint(position)?;
        update(persistent, counters);
    }
    Ok(())
}
fn drain(
    persistent: &mut Persistence,
    counters: &Counters,
    barrier: &mut Option<FailureBarrier>,
) -> Result<(), TelemetryError> {
    while !persistent.backlog.is_empty() {
        drain_one(persistent, counters, barrier)?;
    }
    Ok(())
}
/// Count and record one failed backlog commit, and return when the next retry may start.
fn record_commit_failure(
    persistent: &Persistence,
    counters: &Counters,
    error: &TelemetryError,
) -> Instant {
    if *error == TelemetryError::Capacity {
        counters.capacity.fetch_add(1, Ordering::Relaxed);
    }
    emergency(persistent, counters, error);
    Instant::now() + SINK_RETRY_BACKOFF
}
fn worker(
    mut persistent: Persistence,
    receiver: mpsc::Receiver<Command>,
    counters: Arc<Counters>,
    mut barrier: Option<FailureBarrier>,
) {
    let mut deferred = None;
    let mut retry_not_before = Instant::now();
    loop {
        let command = if let Some(command) = deferred.take() {
            command
        } else {
            match receiver.recv_timeout(WORKER_IDLE_POLL) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if counters.closed.load(Ordering::Acquire) {
                        break;
                    }
                    if !persistent.backlog.is_empty()
                        && Instant::now() >= retry_not_before
                        && let Err(error) = drain_one(&mut persistent, &counters, &mut barrier)
                    {
                        retry_not_before = record_commit_failure(&persistent, &counters, &error);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        match command {
            Command::Admit(first) => {
                let mut admitted = AdmittedBatch::new(first);
                deferred = admitted.combine_queued(&receiver, persistent.limits);
                let outcome = admitted.spool_and_commit(
                    &mut persistent,
                    &counters,
                    &mut barrier,
                    &mut retry_not_before,
                );
                if let Err(error) = &outcome {
                    reject(&counters, error);
                    emergency(&persistent, &counters, error);
                }
                admitted.reply(&counters, &outcome);
            }
            Command::Flush(reply) => {
                drain_and_reply(&mut persistent, &counters, &mut barrier, reply);
            }
            Command::Shutdown(reply) => {
                drain_and_reply(&mut persistent, &counters, &mut barrier, reply);
                break;
            }
        }
    }
    counters.closed.store(true, Ordering::Release);
}
/// One producer's receipt channel and what it reserved in the admission queue.
struct PendingReply {
    reply: mpsc::Sender<Result<IngestReceipt, TelemetryError>>,
    queue_cost_bytes: usize,
    records: usize,
}
impl Admission {
    fn split(self) -> (Batch, PendingReply) {
        let reply = PendingReply {
            reply: self.reply,
            queue_cost_bytes: self.cost,
            records: self.batch.records.len(),
        };
        (self.batch, reply)
    }
}
/// One spool frame built from an admission plus the queued admissions combined into it.
struct AdmittedBatch {
    batch: Batch,
    replies: Vec<PendingReply>,
}
impl AdmittedBatch {
    fn new(first: Admission) -> Self {
        let (batch, reply) = first.split();
        Self {
            batch,
            replies: vec![reply],
        }
    }
    /// Opportunistically combine already queued standard protobuf requests. Repeated root
    /// messages concatenate legally in protobuf; the signal stays fixed. Returns the first
    /// queued command that could not join, for the worker to handle next.
    fn combine_queued(
        &mut self,
        receiver: &mpsc::Receiver<Command>,
        limits: TelemetryLimits,
    ) -> Option<Command> {
        while self.batch.records.len() < limits.batch_records {
            match receiver.try_recv() {
                Ok(Command::Admit(next))
                    if next.batch.signal == self.batch.signal
                        && self.batch.records.len() + next.batch.records.len()
                            <= limits.batch_records
                        && self.batch.bytes.len() + next.batch.bytes.len()
                            <= limits.batch_bytes =>
                {
                    let (batch, reply) = next.split();
                    self.replies.push(reply);
                    self.batch.bytes.extend(batch.bytes);
                    self.batch.records.extend(batch.records);
                }
                Ok(command) => return Some(command),
                Err(_) => return None,
            }
        }
        None
    }
    fn spool_and_commit(
        &self,
        persistent: &mut Persistence,
        counters: &Counters,
        barrier: &mut Option<FailureBarrier>,
        retry_not_before: &mut Instant,
    ) -> Result<IngestReceipt, TelemetryError> {
        let batch = &self.batch;
        barrier_wait(barrier, BarrierStage::BeforeSpool)?;
        let position = persistent.spool(batch)?;
        counters
            .spooled
            .fetch_add(batch.records.len() as u64, Ordering::Relaxed);
        update(persistent, counters);
        barrier_wait(barrier, BarrierStage::Spooled)?;
        // A blocked sink does not impose its busy wait on every spool admission. One owned
        // retry clock backs off; spooling continues and FIFO flush explicitly forces retry.
        let durability = if Instant::now() < *retry_not_before {
            Durability::Spooled
        } else {
            // One old frame between new spool admissions keeps recovery fair.
            // Only a checkpoint covering this frame earns Committed.
            match drain_one(persistent, counters, barrier) {
                Ok(()) if persistent.checkpoint >= position => Durability::Committed,
                Ok(()) => Durability::Spooled,
                Err(error) => {
                    *retry_not_before = record_commit_failure(persistent, counters, &error);
                    Durability::Spooled
                }
            }
        };
        Ok(IngestReceipt {
            durability,
            spool_position: position,
            committed_watermark: persistent.watermark,
            records: batch.records.len(),
        })
    }
    /// Release queue reservations and send each producer its own record count.
    fn reply(self, counters: &Counters, outcome: &Result<IngestReceipt, TelemetryError>) {
        for pending in self.replies {
            counters.items.fetch_sub(1, Ordering::AcqRel);
            counters
                .bytes
                .fetch_sub(pending.queue_cost_bytes, Ordering::AcqRel);
            let outcome = outcome.clone().map(|mut receipt| {
                receipt.records = pending.records;
                receipt
            });
            // A lost receipt never cancels or reallocates accepted identities.
            if pending.reply.send(outcome).is_err() {
                counters.gaps.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
fn drain_and_reply(
    persistent: &mut Persistence,
    counters: &Counters,
    barrier: &mut Option<FailureBarrier>,
    reply: mpsc::Sender<Result<(), TelemetryError>>,
) {
    let drain_result = drain(persistent, counters, barrier);
    if let Err(error) = &drain_result {
        emergency(persistent, counters, error);
    }
    if reply.send(drain_result).is_err() {
        counters.gaps.fetch_add(1, Ordering::Relaxed);
    }
}
