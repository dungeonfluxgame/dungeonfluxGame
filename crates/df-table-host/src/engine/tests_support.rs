//! Shared fixtures for the owner actor's tests: a rig that starts the real owner thread over fake
//! adapters, a manual clock, and request builders.
//!
//! The owner thread is real; time is not: every adapter reads a manual clock the test moves, and
//! nothing sleeps. A test asks the owner a question and awaits the answer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_table::config::Settings;
use df_table::ports::Clock;
use df_table::testkit::ManualClock;

use crate::event_log::EventLog;

use super::adapters::{Adapters, BrokerSide, DevDice, DevInterpreter, DevNarrator, DevRulesOwner};
use super::audit::LogBrokerAudit;
use super::commit::{CommitPolicy, ScriptedCommit, ScriptedCommitStore};
use super::handle::EngineHandle;
use super::request::{InputBody, InputRequest};
use super::setup::{EngineConfig, dev_machines};

/// A clock that moves a fixed number of milliseconds every time it is read, so a time slice can
/// run out without a sleep.
pub struct TickingClock {
    now_ms: AtomicU64,
    step_ms: u64,
}

impl TickingClock {
    pub fn stepping(step_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(0),
            step_ms,
        }
    }
}

impl Clock for TickingClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.fetch_add(self.step_ms, Ordering::Relaxed)
    }
}

/// What a test varies about the owner.
pub struct RigOptions {
    pub held: bool,
    pub policy: CommitPolicy,
    pub commit_deadline_ms: u64,
    pub commit_backoff_ms: u64,
    pub script: Vec<ScriptedCommit>,
    /// The seat limits of J.4 (3 pending, 3 burst, 1 a second) when true; far above any test when
    /// false.
    pub default_seat_limits: bool,
}

impl Default for RigOptions {
    fn default() -> Self {
        Self {
            held: false,
            policy: CommitPolicy::per_batch(),
            commit_deadline_ms: 1_000,
            commit_backoff_ms: 5_000,
            script: Vec::new(),
            default_seat_limits: false,
        }
    }
}

/// A running owner thread and what a test reads about it.
pub struct Rig {
    pub handle: EngineHandle,
    pub log: Arc<EventLog>,
    pub store: Arc<ScriptedCommitStore>,
    pub clock: Arc<ManualClock>,
}

fn settings(default_seat_limits: bool) -> Settings {
    if default_seat_limits {
        return Settings::defaults();
    }
    let large = serde_json::json!({
        "seat_pending_limit": 1_000,
        "seat_rate_burst": 1_000_000,
        "seat_rate_sustained_per_s": 1_000_000,
    });
    let configuration = large.as_object().cloned().unwrap_or_default();
    Settings::load(&configuration)
        .expect("the settings load")
        .settings
}

pub fn config(options: &RigOptions) -> EngineConfig {
    EngineConfig {
        settings: settings(options.default_seat_limits),
        policy: options.policy,
        commit_deadline_ms: options.commit_deadline_ms,
        commit_backoff_ms: options.commit_backoff_ms,
        start_held: options.held,
        ..EngineConfig::dev()
    }
}

pub fn adapters(clock: Arc<dyn Clock>, log: &Arc<EventLog>) -> Adapters {
    let machines = dev_machines(EngineConfig::dev().first_scene).expect("the dev machines build");
    Adapters {
        broker: BrokerSide::dev(
            Arc::clone(&clock),
            Arc::new(LogBrokerAudit::new(Arc::clone(log))),
        )
        .expect("the broker builds"),
        clock,
        rules: Arc::new(DevRulesOwner::new(machines.check)),
        interpreter: Arc::new(DevInterpreter),
        narrator: Arc::new(DevNarrator),
        dice: Arc::new(DevDice::seeded(7)),
        knowledge: Arc::new(super::adapters::DevKnowledge),
        controls: super::adapters::DevControls::default(),
    }
}

pub fn rig() -> Rig {
    rig_with(RigOptions::default())
}

pub fn rig_with(options: RigOptions) -> Rig {
    let clock = Arc::new(ManualClock::default());
    let log = Arc::new(EventLog::default());
    let store = Arc::new(ScriptedCommitStore::scripted(options.script.clone()));
    let handle = EngineHandle::start(
        config(&options),
        adapters(clock.clone(), &log),
        store.clone(),
        Arc::clone(&log),
    )
    .expect("the owner starts");
    Rig {
        handle,
        log,
        store,
        clock,
    }
}

pub fn request(input: InputBody) -> InputRequest {
    InputRequest {
        input,
        operation: None,
        operation_id: None,
        view: None,
        machine: None,
    }
}

pub fn tap(seat: u64, action: &str) -> InputRequest {
    request(InputBody::Tap {
        seat,
        action: action.to_owned(),
    })
}

pub fn text(seat: u64, line: &str) -> InputRequest {
    request(InputBody::Text {
        seat,
        text: line.to_owned(),
    })
}

pub fn timer(name: &str) -> InputRequest {
    request(InputBody::Timer {
        name: name.to_owned(),
    })
}

pub fn proposal(origin: &str, proposer: &str) -> InputRequest {
    request(InputBody::Proposal {
        origin: origin.to_owned(),
        proposer: proposer.to_owned(),
    })
}

pub fn with_operation(mut request: InputRequest, operation: u64) -> InputRequest {
    request.operation = Some(operation);
    request
}

/// Every line of the host's log, oldest first.
pub fn lines(log: &EventLog) -> Vec<String> {
    log.after_sequence(0, 1_000_000)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

/// How many log lines contain `needle`.
pub fn count_lines(log: &EventLog, needle: &str) -> usize {
    lines(log)
        .iter()
        .filter(|line| line.contains(needle))
        .count()
}

/// An interpreter that is down, to show the fallback and the one recorded failure.
pub struct FailingInterpreter;

impl df_table::ports::Interpreter for FailingInterpreter {
    fn interpret(
        &self,
        _correlation: df_table::identity::Correlation,
        _seat: df_table::identity::SeatId,
        _text: &df_table::private_data::PlayerText,
        _legal_choices: &[df_table::ports::ActionId],
    ) -> Result<df_table::ports::Interpretation, df_table::ports::PortError> {
        Err(df_table::ports::PortError::new(
            df_table::ports::PortName::Interpreter,
            df_table::event::FailureCause::Unavailable,
        ))
    }
}

/// A store whose commit signals that it has begun and then waits to be released, so a test can
/// hold the owner thread inside a commit.
pub struct BlockingStore {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    /// The id and input count of every checkpoint the store was handed, in order.
    attempts: std::sync::Mutex<Vec<(u64, usize)>>,
}

impl BlockingStore {
    /// The store, the receiver that hears each commit begin, and the sender that releases one.
    pub fn new() -> (
        Arc<Self>,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered, heard) = std::sync::mpsc::channel();
        let (release_sender, release) = std::sync::mpsc::channel();
        let store = Arc::new(Self {
            entered,
            release: std::sync::Mutex::new(release),
            attempts: std::sync::Mutex::new(Vec::new()),
        });
        (store, heard, release_sender)
    }

    /// The attempt id and the number of inputs of every write begun, oldest first.
    pub fn attempts(&self) -> Vec<(u64, usize)> {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl super::commit::CommitStore for BlockingStore {
    fn commit(
        &self,
        checkpoint: &super::commit::Checkpoint,
        _deadline_ms: u64,
    ) -> Result<super::commit::CommitReceipt, super::commit::CommitError> {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((checkpoint.commit.get(), checkpoint.inputs));
        // The test is listening; a closed channel means it has finished and nothing matters.
        let _ = self.entered.send(());
        let _ = self
            .release
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv();
        Ok(super::commit::CommitReceipt { took_ms: 0 })
    }

    fn restore(&self) -> Option<super::commit::Restored> {
        None
    }
}

/// An owner whose store blocks every write until the test releases it.
pub struct BlockingRig {
    pub handle: Arc<EngineHandle>,
    pub store: Arc<BlockingStore>,
    pub entered: std::sync::mpsc::Receiver<()>,
    pub release: std::sync::mpsc::Sender<()>,
    pub clock: Arc<ManualClock>,
}

pub fn blocking_rig(options: &RigOptions) -> BlockingRig {
    let (store, entered, release) = BlockingStore::new();
    let clock = Arc::new(ManualClock::default());
    let log = Arc::new(EventLog::default());
    let handle = Arc::new(
        EngineHandle::start(
            config(options),
            adapters(clock.clone(), &log),
            store.clone(),
            Arc::clone(&log),
        )
        .expect("the owner starts"),
    );
    BlockingRig {
        handle,
        store,
        entered,
        release,
        clock,
    }
}

/// An interpreter that panics, to prove the owner's thread does not take the host down silently.
pub struct PanickingInterpreter;

impl df_table::ports::Interpreter for PanickingInterpreter {
    fn interpret(
        &self,
        _correlation: df_table::identity::Correlation,
        _seat: df_table::identity::SeatId,
        _text: &df_table::private_data::PlayerText,
        _legal_choices: &[df_table::ports::ActionId],
    ) -> Result<df_table::ports::Interpretation, df_table::ports::PortError> {
        panic!("the interpreter broke")
    }
}

/// The story table (H3): the lobby, the story scene and the conversation, over the scripted rules
/// owner and dice that give `dice` first. Standing buttons are on.
pub fn story_rig(dice: &[u16]) -> Rig {
    story_rig_over(|machines| {
        let urging = machines
            .urge
            .clone()
            .expect("the story table has an urging");
        (
            Arc::new(DevRulesOwner::new(machines.check.clone()).with_urging(urging)),
            Arc::new(DevDice::queued_then_seeded(dice.iter().copied(), 7)),
        )
    })
}

/// The story table over the production `df-rules` adapters with recorded dice from `seed`.
pub fn story_rig_df_rules(seed: u64) -> Rig {
    story_rig_over(|machines| {
        let ports = crate::adapters::story_ports(
            crate::adapters::RulesBackend::DfRules,
            &machines.check,
            machines.urge.as_ref(),
            seed,
        )
        .expect("the production ports build");
        (ports.rules, ports.dice)
    })
}

type StoryPorts = (
    Arc<dyn df_table::ports::RulesOwner>,
    Arc<dyn df_table::ports::Dice>,
);

/// The story table over the dev rules owner and dice, started with the options a test chooses
/// (held, a commit script, the J.4 seat limits).
pub fn story_rig_options(options: RigOptions) -> Rig {
    let store = Arc::new(ScriptedCommitStore::scripted(options.script.clone()));
    story_rig_started(options, store, no_dice)
}

/// The story table over a store the test keeps, so a second engine can be started over the same
/// store, as a restart of the host is.
pub fn story_rig_on(store: Arc<ScriptedCommitStore>) -> Rig {
    story_rig_started(RigOptions::default(), store, no_dice)
}

fn no_dice(machines: &super::setup::Machines) -> StoryPorts {
    let urging = machines
        .urge
        .clone()
        .expect("the story table has an urging");
    (
        Arc::new(DevRulesOwner::new(machines.check.clone()).with_urging(urging)),
        Arc::new(DevDice::queued_then_seeded(std::iter::empty(), 7)),
    )
}

fn story_rig_over(ports: impl FnOnce(&super::setup::Machines) -> StoryPorts) -> Rig {
    let options = RigOptions::default();
    let store = Arc::new(ScriptedCommitStore::scripted(Vec::new()));
    story_rig_started(options, store, ports)
}

fn story_rig_started(
    options: RigOptions,
    store: Arc<ScriptedCommitStore>,
    ports: impl FnOnce(&super::setup::Machines) -> StoryPorts,
) -> Rig {
    let clock = Arc::new(ManualClock::default());
    let log = Arc::new(EventLog::default());
    let story = EngineConfig {
        story: true,
        ..config(&options)
    };
    let machines = super::setup::machines_for(&story).expect("the story machines build");
    let (rules, dice) = ports(&machines);
    let mut adapters = adapters(clock.clone(), &log);
    adapters.rules = rules;
    adapters.dice = dice;
    let handle = EngineHandle::start(story, adapters, store.clone(), Arc::clone(&log))
        .expect("the owner starts");
    Rig {
        handle,
        log,
        store,
        clock,
    }
}

/// Starts an owner over adapters the test changes first.
pub fn rig_customised(options: RigOptions, customise: impl FnOnce(&mut Adapters)) -> Rig {
    let clock = Arc::new(ManualClock::default());
    let log = Arc::new(EventLog::default());
    let store = Arc::new(ScriptedCommitStore::scripted(options.script.clone()));
    let mut adapters = adapters(clock.clone(), &log);
    customise(&mut adapters);
    let handle = EngineHandle::start(config(&options), adapters, store.clone(), Arc::clone(&log))
        .expect("the owner starts");
    Rig {
        handle,
        log,
        store,
        clock,
    }
}
