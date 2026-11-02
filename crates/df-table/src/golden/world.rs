//! The scripted world a golden session or a pipeline test runs in.
//!
//! It owns `World`: one scripted fake per port, a registry with a lobby, an explore machine and
//! the golden scene, the settings, the table and a counter that numbers the inputs, so a test says
//! what each fake answers and then feeds inputs. `CountingDice` counts every die drawn, which is
//! how a test proves that waiting, replaying and restarting never draw again. It never decides
//! anything of its own: the fakes answer from their scripts, and a question outside a script
//! fails the test loudly, as `testkit` says.
//!
//! It never panics itself; building a world returns a typed error.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::Settings;
use crate::effects::{TableEffect, TableState};
use crate::envelope::{EngineInput, InputEnvelope, PlayerInput, ViewStamp};
use crate::event::TableEvent;
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::identity::{
    Correlation, OperationId, RecoveryEpoch, SceneId, SeatId, SessionRevision, TableId, TableScope,
    TurnSeq,
};
use crate::lobby::LobbyMachine;
use crate::machine::Machine;
use crate::outcome::OutcomeKind;
use crate::pipeline::{Pipeline, PipelineError, PipelineLimits, PipelineTable, Ports, TurnResult};
use crate::ports::{
    ActionId, Clock, CommittedOutcome, Dice, Draw, DrawRequest, Narrator, PortError,
};
use crate::private_data::{Narration, PlayerText};
use crate::registry::{Registry, RegistryBuilder, RegistryError};
use crate::table_state::MachineHome;
use crate::testkit::{
    FixedKnowledgeView, ManualClock, QueuedDice, RecordingSink, ScriptedBroker,
    ScriptedInterpreter, ScriptedRulesOwner, correlation,
};

use super::scene_machine::{Leaves, SceneMachine};

/// The table every world runs.
pub const TABLE: TableId = TableId::new(1);
/// The turn number of the setup commits, far from any input a test numbers.
const SETUP_TURN: u64 = 1_000_000;
/// The scene a started game plays in.
pub const FIRST_SCENE: SceneId = SceneId::new(1);

/// Dice that give the queued values and count every die drawn.
#[derive(Debug, Default)]
pub struct CountingDice {
    inner: QueuedDice,
    drawn: AtomicU64,
}

impl CountingDice {
    pub fn new(values: impl IntoIterator<Item = u16>) -> Self {
        Self {
            inner: QueuedDice::new(values),
            drawn: AtomicU64::new(0),
        }
    }

    /// How many dice were drawn so far.
    pub fn drawn(&self) -> u64 {
        // Relaxed: a counter that orders no other data, read by the thread that drew.
        self.drawn.load(Ordering::Relaxed)
    }
}

impl Dice for CountingDice {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        self.drawn.fetch_add(1, Ordering::Relaxed);
        self.inner.draw(correlation, request)
    }
}

/// A narrator that always says the same qualified line.
#[derive(Debug, Clone)]
pub struct FixedNarrator(pub String);

impl Narrator for FixedNarrator {
    fn narrate(
        &self,
        _correlation: Correlation,
        _committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        Ok(Narration::new(self.0.clone()))
    }
}

/// A narrator that is slow: it moves the clock by `delay_ms` before it answers.
pub struct SlowNarrator {
    pub clock: Arc<ManualClock>,
    pub delay_ms: u64,
    pub line: String,
}

impl Narrator for SlowNarrator {
    fn narrate(
        &self,
        _correlation: Correlation,
        _committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        self.clock.advance_ms(self.delay_ms);
        Ok(Narration::new(self.line.clone()))
    }
}

/// Why a world could not be built.
#[derive(Debug)]
pub enum WorldError {
    Registry(Vec<RegistryError>),
    Machine(String),
    Pipeline(PipelineError),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(errors) => {
                write!(formatter, "the registry has {} problems", errors.len())
            }
            Self::Machine(text) => write!(formatter, "a machine could not be built: {text}"),
            Self::Pipeline(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorldError {}

/// Everything one scripted run needs.
pub struct World {
    pub rules: ScriptedRulesOwner,
    pub interpreter: ScriptedInterpreter,
    pub narrator: Box<dyn Narrator>,
    pub broker: ScriptedBroker,
    pub clock: Arc<ManualClock>,
    pub dice: CountingDice,
    pub knowledge: FixedKnowledgeView,
    pub sink: RecordingSink,
    pub registry: Registry,
    pub settings: Settings,
    pub table: PipelineTable,
    pub limits: PipelineLimits,
    next_input: u64,
}

impl World {
    /// A world whose table seats seats 1 to `seats` and has the scene machine of `actions`
    /// running, with a scene entered. Every fake starts empty: a test scripts what it asks.
    pub fn with_scene(
        actions: &[(&str, Leaves)],
        seats: u64,
        finishing_after: Option<u64>,
    ) -> Result<Self, WorldError> {
        let mut scene =
            SceneMachine::new(actions).map_err(|error| WorldError::Machine(error.to_string()))?;
        if let Some(turns) = finishing_after {
            scene = scene.finishing_after(turns);
        }
        Self::with_scene_machine(scene, seats)
    }

    /// Like `with_scene`, for a scene machine the caller configured.
    pub fn with_scene_machine(scene: SceneMachine, seats: u64) -> Result<Self, WorldError> {
        let lobby = LobbyMachine::new(FIRST_SCENE)
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        let beat = AuthoredBeat::new("gate", "open_gate")
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        let explore =
            ExploreMachine::new(beat).map_err(|error| WorldError::Machine(error.to_string()))?;
        let registry = RegistryBuilder::new()
            .register(lobby)
            .register(explore)
            .register(scene.clone())
            .build(crate::ports::PortName::ALL)
            .map_err(WorldError::Registry)?;
        let seated: Vec<SeatId> = (1..=seats).map(SeatId::new).collect();
        let mut facts = TableState::seated(seats.max(1) as usize, &seated)
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        facts
            .apply(
                &TableScope::of_table(TABLE),
                correlation(0),
                &[TableEffect::SceneEntered { scene: FIRST_SCENE }],
            )
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        let mut table = PipelineTable::new(TABLE, facts);
        let slot = scene
            .enter(table.facts(), &crate::machine::Entry::empty())
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        table
            .start_machine(
                setup_correlation(),
                MachineHome::Scene { scene: FIRST_SCENE },
                slot,
            )
            .map_err(|error| WorldError::Machine(error.to_string()))?;
        Ok(Self {
            rules: ScriptedRulesOwner::default(),
            interpreter: ScriptedInterpreter::default(),
            narrator: Box::new(FixedNarrator("The moment passes.".to_owned())),
            broker: ScriptedBroker::default(),
            clock: Arc::new(ManualClock::starting_at(1_000)),
            dice: CountingDice::default(),
            knowledge: FixedKnowledgeView::default(),
            sink: RecordingSink::default(),
            registry,
            settings: Settings::defaults(),
            table,
            limits: PipelineLimits::default(),
            next_input: 0,
        })
    }

    /// A world that restarted after `turns` inputs: the next input is numbered after them.
    pub fn resuming_after(mut self, turns: u64) -> Self {
        self.next_input = turns;
        self
    }

    /// The number the next input will carry as its turn sequence.
    pub fn next_turn(&self) -> u64 {
        self.next_input + 1
    }

    /// An envelope for the next input, numbered in order, from the revision it will be served at.
    fn envelope_for(&self, seat: u64, input: PlayerInput) -> InputEnvelope {
        let probe = correlation(self.next_turn());
        InputEnvelope::from_player(
            probe.operation_id,
            SeatId::new(seat),
            ViewStamp::new(probe.revision),
            input,
        )
    }

    /// Runs an envelope as the next input.
    pub fn run_envelope(&mut self, envelope: &InputEnvelope) -> Result<TurnResult, WorldError> {
        self.next_input += 1;
        let probe = correlation(self.next_input);
        self.run_with(envelope, probe)
    }

    /// Runs an envelope with an explicit correlation, for a replay of an operation id, and then
    /// does what the host does after a successful commit: it releases the turn's events and
    /// issues the requests the turn recorded.
    pub fn run_with(
        &mut self,
        envelope: &InputEnvelope,
        correlation: Correlation,
    ) -> Result<TurnResult, WorldError> {
        let result = self.run_unreleased(envelope, correlation)?;
        result.release_events(&self.sink);
        self.with_pipeline(|pipeline, table| pipeline.issue_requests(table, correlation))?;
        Ok(result)
    }

    /// Runs an envelope and releases nothing: what a host that has not committed yet holds.
    pub fn run_unreleased(
        &mut self,
        envelope: &InputEnvelope,
        correlation: Correlation,
    ) -> Result<TurnResult, WorldError> {
        self.with_pipeline(|pipeline, table| pipeline.run(table, envelope, correlation))
    }

    /// Runs the narration job of a result the way the host does after its commit: the narrator
    /// answers, and the pipeline qualifies the answer against the clock reading at which the
    /// readout was shown.
    pub fn narrate(&mut self, result: &TurnResult) -> Result<Option<Narration>, WorldError> {
        let Some(job) = &result.narration_job else {
            return Ok(None);
        };
        let shown_at_ms = self.clock.now_ms();
        let answer = self.narrator.narrate(job.correlation, &job.committed);
        self.with_pipeline(|pipeline, table| {
            pipeline.attach_narration(table, job, shown_at_ms, answer)
        })
    }

    /// Gives `act` the pipeline over this world's fakes and the table, for what is not an input:
    /// a seat dropping, a ruling revoked.
    pub fn with_pipeline<R>(
        &mut self,
        act: impl FnOnce(&Pipeline<'_>, &mut PipelineTable) -> R,
    ) -> Result<R, WorldError> {
        let Self {
            rules,
            interpreter,
            narrator,
            broker,
            clock,
            dice,
            knowledge,
            sink,
            registry,
            settings,
            table,
            limits,
            ..
        } = self;
        let ports = Ports {
            rules: &*rules,
            interpreter: &*interpreter,
            narrator: narrator.as_ref(),
            broker: &*broker,
            clock: clock.as_ref(),
            dice: &*dice,
            knowledge: &*knowledge,
            sink: &*sink,
        };
        let pipeline = Pipeline::new(ports, registry, settings)
            .map_err(WorldError::Pipeline)?
            .with_limits(limits.clone());
        Ok(act(&pipeline, table))
    }

    /// A tap by `seat` on an offered choice.
    pub fn tap(&mut self, seat: u64, action: &str) -> Result<TurnResult, WorldError> {
        let action = name(action)?;
        let envelope = self.envelope_for(seat, PlayerInput::Tap { action });
        self.run_envelope(&envelope)
    }

    /// A typed line by `seat`.
    pub fn text(&mut self, seat: u64, line: &str) -> Result<TurnResult, WorldError> {
        let input = PlayerInput::SubmitText {
            utterance: crate::envelope::UtteranceId::new(self.next_turn()),
            text: PlayerText::new(line),
        };
        let envelope = self.envelope_for(seat, input);
        self.run_envelope(&envelope)
    }

    /// An answer by `seat` to the pending with this id.
    pub fn answer(
        &mut self,
        seat: u64,
        pending: u64,
        choice: &str,
    ) -> Result<TurnResult, WorldError> {
        let input = PlayerInput::PendingAnswer {
            pending: crate::envelope::PendingId::new(pending),
            choice: name(choice)?,
        };
        let envelope = self.envelope_for(seat, input);
        self.run_envelope(&envelope)
    }

    /// An engine input as the next input.
    pub fn engine(&mut self, input: EngineInput) -> Result<TurnResult, WorldError> {
        let probe = correlation(self.next_turn());
        let envelope =
            InputEnvelope::from_engine(probe.operation_id, ViewStamp::new(probe.revision), input);
        self.run_envelope(&envelope)
    }

    /// The events recorded so far.
    pub fn events(&self) -> Vec<TableEvent> {
        self.sink.events()
    }

    /// How many of the recorded events came from inputs that ended this way.
    pub fn count_outcomes(&self, kind: OutcomeKind) -> usize {
        self.events()
            .iter()
            .filter(|event| {
                matches!(event, TableEvent::StepFinished { outcome: Some(found), .. } if *found == kind)
            })
            .count()
    }

    /// The operation id of the `turn`th input.
    pub fn operation_of(turn: u64) -> OperationId {
        correlation(turn).operation_id
    }
}

/// The correlation of the setup commits: its own operation id and the genesis revision, so a
/// world starts with a table that no input has stamped.
fn setup_correlation() -> Correlation {
    Correlation {
        turn_seq: TurnSeq::new(0),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, 0),
        ..correlation(SETUP_TURN)
    }
}

fn name(text: &str) -> Result<ActionId, WorldError> {
    ActionId::new(text).map_err(|error| WorldError::Machine(error.to_string()))
}
