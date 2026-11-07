//! The S1 rerun on the real engine (O.16 H3): the owner actor of `crate::engine` with the real
//! queue, the real pipeline, the production `df-rules` adapters, the real projection of seven
//! audiences and the real group commit, measured from outside as a caller sees it.
//!
//! The first S1 harness (`run`) measured stand-ins. This one measures the thing: each input is a
//! seat's tap submitted to `EngineHandle`, and its latency is the time until the answer comes
//! back, which is after the batch's checkpoint was written and the sender's view was built. The
//! store behind the commit is one of the harness's three (`CommitMode`): nowhere, a file that is
//! written and `fsync`ed, or a fixed sleep that models Postgres. The checkpoint is the real
//! document (the table's dump and everything the pipeline holds), hashed here with the
//! harness's software SHA-256 as the first S1 did, so the figures stay comparable.
//!
//! Workloads: six seats at the lobby alternate `unready` and `ready` (each input changes the
//! lobby's state, so each batch has something to commit) one at a time, six at once, and
//! eighteen at once (three each, the pending bound); the same six at once with a view read in
//! front of every input, as the live path has (a phone polls its view and the wire route reads one
//! before each intent), so the figures say what reads cost the group commit; and the rules path,
//! the conversation's urging with its two d20s through the production influence flow, on a fresh
//! table for every sample. Projection cost is read as the difference between asking for a view
//! and asking for the owner's health (a call that does no work), over many calls.
//!
//! It reads `std::time::Instant`, which the pure crate never does, and is a measurement tool.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use df_table::config::{ConfigError, Settings};
use df_table::ports::Clock;
use df_types::saturating_millis;

use crate::adapters::{RulesBackend, story_ports};
use crate::engine::EngineHandle as Handle;
use crate::engine::adapters::{
    Adapters, BrokerSide, DevClock, DevControls, DevKnowledge, DevNarrator,
};
use crate::engine::audit::LogBrokerAudit;
use crate::engine::commit::{
    Checkpoint, CommitError, CommitReceipt, CommitStore, MemoryCommitStore, Restored,
};
use crate::engine::jev::JevInterpreter;
use crate::engine::report::EngineError;
use crate::engine::request::{AudienceSpec, InputBody, InputRequest};
use crate::engine::setup::{EngineConfig, EngineStartError, machines_for};
use crate::event_log::EventLog;
use crate::text::TextRuntime;

use super::commit::{CommitMode, MODELLED_POSTGRES_MS};
use super::limits::COMMIT_DEADLINE_MS;
use super::sha256::sha256;
use super::statistics::{Summary, nanoseconds, summarise};

/// The seats of the table: six, as S1 and the dev table have.
pub const SEATS: u64 = 6;
/// The seed of the recorded dice, so a run can be repeated.
const SEED: u64 = 20_261_103;

/// Why a measurement could not run.
#[derive(Debug)]
pub enum RealError {
    /// The raised queue limits the harness runs under do not load.
    Settings(ConfigError),
    /// The engine did not start.
    Start(EngineStartError),
    /// A call to the running engine was refused.
    Call(EngineError),
    /// A measuring task stopped without an answer (it panicked or was cancelled).
    TaskStopped,
    /// A result file could not be written.
    Io(std::io::ErrorKind),
    Nothing,
}

impl std::fmt::Display for RealError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settings(error) => write!(formatter, "the harness settings do not load: {error}"),
            Self::Start(error) => write!(formatter, "the engine did not start: {error}"),
            Self::Call(error) => write!(formatter, "a call failed: {error}"),
            Self::TaskStopped => formatter.write_str("a measuring task stopped without an answer"),
            Self::Io(kind) => write!(formatter, "a file could not be written ({kind})"),
            Self::Nothing => formatter.write_str("nothing was measured"),
        }
    }
}

impl std::error::Error for RealError {}

impl From<std::io::Error> for RealError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<EngineStartError> for RealError {
    fn from(error: EngineStartError) -> Self {
        Self::Start(error)
    }
}

impl From<EngineError> for RealError {
    fn from(error: EngineError) -> Self {
        Self::Call(error)
    }
}

/// A commit store that hashes the checkpoint as S1 did and then stores it by `mode`.
struct HarnessStore {
    mode: CommitMode,
    file: Option<PathBuf>,
    memory: MemoryCommitStore,
}

impl CommitStore for HarnessStore {
    fn commit(
        &self,
        checkpoint: &Checkpoint,
        deadline_ms: u64,
    ) -> Result<CommitReceipt, CommitError> {
        let started = Instant::now();
        std::hint::black_box(sha256(&checkpoint.bytes));
        match (self.mode, &self.file) {
            (CommitMode::Fsync, Some(path)) => {
                let written = File::create(path).and_then(|mut file| {
                    file.write_all(&checkpoint.bytes)?;
                    file.sync_all()
                });
                written.map_err(|_| CommitError::Unavailable)?;
            }
            (CommitMode::ModelledPostgres, _) => {
                std::thread::sleep(Duration::from_millis(MODELLED_POSTGRES_MS));
            }
            _ => {}
        }
        let took_ms = saturating_millis(started.elapsed());
        if took_ms > deadline_ms {
            return Err(CommitError::DeadlineExceeded { deadline_ms });
        }
        self.memory.commit(checkpoint, deadline_ms)?;
        Ok(CommitReceipt { took_ms })
    }

    fn restore(&self) -> Option<Restored> {
        self.memory.restore()
    }
}

fn settings() -> Result<Settings, RealError> {
    // The queue's own limits are not what is measured here: they are raised far above any load.
    let large = serde_json::json!({
        "seat_pending_limit": 1_000,
        "seat_rate_burst": 1_000_000,
        "seat_rate_sustained_per_s": 1_000_000,
    });
    let object = large.as_object().cloned().unwrap_or_default();
    Settings::load(&object)
        .map(|loaded| loaded.settings)
        .map_err(RealError::Settings)
}

/// Starts the first playable path over the production rules adapters and a harness store.
fn start(mode: CommitMode, directory: &std::path::Path) -> Result<Handle, RealError> {
    let config = EngineConfig {
        settings: settings()?,
        commit_deadline_ms: COMMIT_DEADLINE_MS,
        ..EngineConfig::first_playable()
    };
    let machines = machines_for(&config)?;
    let ports = story_ports(
        RulesBackend::DfRules,
        &machines.check,
        machines.urge.as_ref(),
        SEED,
    )
    .map_err(EngineStartError::from)?;
    let clock = Arc::new(DevClock::starting_now());
    let log = Arc::new(EventLog::default());
    let adapters = Adapters {
        broker: BrokerSide::dev(
            clock.clone() as Arc<dyn Clock>,
            Arc::new(LogBrokerAudit::new(Arc::clone(&log))),
        )
        .map_err(EngineStartError::from)?,
        rules: ports.rules,
        // The harness only taps, so the interpreter is never asked.
        interpreter: Arc::new(JevInterpreter::new(Arc::new(TextRuntime::from_environment(
            Arc::new(|_| {}),
        )))),
        narrator: Arc::new(DevNarrator),
        dice: ports.dice,
        knowledge: Arc::new(DevKnowledge),
        controls: DevControls::default(),
        clock,
    };
    let store = Arc::new(HarnessStore {
        mode,
        file: Some(directory.join("checkpoint.json")),
        memory: MemoryCommitStore::default(),
    });
    Ok(Handle::start(config, adapters, store, log)?)
}

fn tap(seat: u64, action: &str) -> InputRequest {
    InputRequest {
        input: InputBody::Tap {
            seat,
            action: action.to_owned(),
        },
        operation: None,
        operation_id: None,
        view: None,
        machine: None,
    }
}

async fn call(handle: &Handle, request: InputRequest) -> Result<(), RealError> {
    handle
        .submit(request)
        .await
        .map(|_| ())
        .map_err(RealError::Call)
}

async fn seat_down(handle: &Handle) -> Result<(), RealError> {
    for seat in 1..=SEATS {
        call(handle, tap(seat, "join")).await?;
    }
    Ok(())
}

/// What one scenario measured.
#[derive(Debug, Clone)]
pub struct RealReport {
    pub label: String,
    pub latency: Summary,
    pub commits: u64,
    pub inputs: usize,
    pub checkpoint_bytes: Option<usize>,
}

fn millis(nanos: u64) -> f64 {
    nanos as f64 / 1e6
}

impl RealReport {
    pub fn render(&self) -> String {
        let latency = self.latency;
        format!(
            "{}: {} inputs, {} commits; submit to answer ms p50 {:.2} p95 {:.2} p99 {:.2} \
             max {:.2} mean {:.2}; checkpoint {} bytes",
            self.label,
            self.inputs,
            self.commits,
            millis(latency.p50_ns),
            millis(latency.p95_ns),
            millis(latency.p99_ns),
            millis(latency.max_ns),
            millis(latency.mean_ns),
            self.checkpoint_bytes
                .map_or_else(|| "unknown".to_owned(), |bytes| bytes.to_string()),
        )
    }
}

async fn finish(
    label: &str,
    handle: &Handle,
    mut samples: Vec<u64>,
) -> Result<RealReport, RealError> {
    let inputs = samples.len();
    let latency = summarise(&mut samples).map_err(|_| RealError::Nothing)?;
    let queue = handle.queue_show().await?;
    Ok(RealReport {
        label: label.to_owned(),
        latency,
        commits: queue.commit.commits,
        inputs,
        checkpoint_bytes: queue.commit.last_checkpoint_bytes,
    })
}

/// `rounds` rounds of `per_round` seats each sending one input at once, with `gap_ms` between
/// rounds; every seat alternates `unready` and `ready`.
pub async fn lobby_burst(
    label: &str,
    mode: CommitMode,
    per_round: u64,
    rounds: usize,
    gap_ms: u64,
    directory: &std::path::Path,
) -> Result<RealReport, RealError> {
    burst(label, mode, per_round, rounds, gap_ms, directory, false).await
}

/// `lobby_burst` with one read of the seat's own view in front of every input, the way a phone
/// and the wire route work. The reads are not in the latency samples; they only share the
/// owner's mailbox with the inputs, so the commits per input tell what they cost the batch.
pub async fn lobby_burst_with_reads(
    label: &str,
    mode: CommitMode,
    per_round: u64,
    rounds: usize,
    gap_ms: u64,
    directory: &std::path::Path,
) -> Result<RealReport, RealError> {
    burst(label, mode, per_round, rounds, gap_ms, directory, true).await
}

async fn burst(
    label: &str,
    mode: CommitMode,
    per_round: u64,
    rounds: usize,
    gap_ms: u64,
    directory: &std::path::Path,
    read_first: bool,
) -> Result<RealReport, RealError> {
    let handle = Arc::new(start(mode, directory)?);
    seat_down(&handle).await?;
    let mut samples = Vec::new();
    for round in 0..rounds {
        let action = if round % 2 == 0 { "ready" } else { "unready" };
        let mut tasks = Vec::new();
        for index in 0..per_round {
            let seat = index % SEATS + 1;
            let handle = Arc::clone(&handle);
            // Several inputs of one seat in a round (the 18 at once) are sent as separate calls.
            tasks.push(tokio::spawn(async move {
                if read_first {
                    handle.machine_view(AudienceSpec::Seat { seat }).await?;
                }
                let started = Instant::now();
                let outcome = call(&handle, tap(seat, action)).await;
                Ok::<_, RealError>((nanoseconds(started.elapsed()), outcome))
            }));
        }
        for task in tasks {
            let (nanos, outcome) = task.await.map_err(|_| RealError::TaskStopped)??;
            outcome?;
            samples.push(nanos);
        }
        tokio::time::sleep(Duration::from_millis(gap_ms)).await;
    }
    finish(label, &handle, samples).await
}

/// One input at a time, `inputs` of them, `gap_ms` apart: a seat alternating `unready` and `ready`.
pub async fn lobby_single(
    label: &str,
    mode: CommitMode,
    inputs: usize,
    gap_ms: u64,
    directory: &std::path::Path,
) -> Result<RealReport, RealError> {
    lobby_burst(label, mode, 1, inputs, gap_ms, directory).await
}

/// The rules path: the conversation's urging on a fresh table for each of `samples` samples.
pub async fn rules_path(
    label: &str,
    mode: CommitMode,
    samples: usize,
    directory: &std::path::Path,
) -> Result<RealReport, RealError> {
    let mut latencies = Vec::new();
    let mut last: Option<RealReport> = None;
    for _ in 0..samples {
        let handle = start(mode, directory)?;
        seat_down(&handle).await?;
        for seat in 1..=SEATS {
            call(&handle, tap(seat, "ready")).await?;
        }
        call(&handle, tap(1, "start")).await?;
        call(&handle, tap(1, "pick_lock")).await?;
        let started = Instant::now();
        call(&handle, tap(1, "persuade_guard")).await?;
        latencies.push(nanoseconds(started.elapsed()));
        if latencies.len() == samples {
            last = Some(finish(label, &handle, latencies.clone()).await?);
        }
    }
    last.ok_or(RealError::Nothing)
}

/// What asking for a view costs the owner beyond the call itself, from `calls` calls of each.
#[derive(Debug, Clone, Copy)]
pub struct ProjectionCost {
    pub baseline: Summary,
    pub phone_json: Summary,
    pub tv_json: Summary,
    pub phone_wire: Summary,
    pub phone_wire_bytes: usize,
    pub tv_wire_bytes: usize,
}

impl ProjectionCost {
    pub fn render(&self) -> String {
        let extra = |summary: Summary| millis(summary.p50_ns.saturating_sub(self.baseline.p50_ns));
        format!(
            "projection (one view, over the health call's p50 {:.3} ms): phone JSON +{:.3} ms p50 \
             (p95 {:.3} ms), tv JSON +{:.3} ms, phone MachineView +{:.3} ms; \
             wire bytes phone {} tv {}",
            millis(self.baseline.p50_ns),
            extra(self.phone_json),
            millis(self.phone_json.p95_ns),
            extra(self.tv_json),
            extra(self.phone_wire),
            self.phone_wire_bytes,
            self.tv_wire_bytes,
        )
    }
}

/// The projection cost on a table in the conversation with six seats and some story behind it.
pub async fn projection_cost(
    calls: usize,
    directory: &std::path::Path,
) -> Result<ProjectionCost, RealError> {
    let handle = start(CommitMode::Memory, directory)?;
    seat_down(&handle).await?;
    for seat in 1..=SEATS {
        call(&handle, tap(seat, "ready")).await?;
    }
    call(&handle, tap(1, "start")).await?;
    call(&handle, tap(1, "pick_lock")).await?;
    let mut baseline = Vec::new();
    let mut phone_json = Vec::new();
    let mut tv_json = Vec::new();
    let mut phone_wire = Vec::new();
    for _ in 0..calls {
        let started = Instant::now();
        handle.health().await?;
        baseline.push(nanoseconds(started.elapsed()));
        let started = Instant::now();
        handle.projection(AudienceSpec::Seat { seat: 1 }).await?;
        phone_json.push(nanoseconds(started.elapsed()));
        let started = Instant::now();
        handle.projection(AudienceSpec::Tv).await?;
        tv_json.push(nanoseconds(started.elapsed()));
        let started = Instant::now();
        handle.machine_view(AudienceSpec::Seat { seat: 1 }).await?;
        phone_wire.push(nanoseconds(started.elapsed()));
    }
    let size = |view: Result<df_protocol::common::MachineView, _>| {
        view.map(|view| prost::Message::encoded_len(&view))
            .map_err(RealError::Call)
    };
    let phone_wire_bytes = size(handle.machine_view(AudienceSpec::Seat { seat: 1 }).await)?;
    let tv_wire_bytes = size(handle.machine_view(AudienceSpec::Tv).await)?;
    let summary = |mut values: Vec<u64>| summarise(&mut values).map_err(|_| RealError::Nothing);
    Ok(ProjectionCost {
        baseline: summary(baseline)?,
        phone_json: summary(phone_json)?,
        tv_json: summary(tv_json)?,
        phone_wire: summary(phone_wire)?,
        phone_wire_bytes,
        tv_wire_bytes,
    })
}
