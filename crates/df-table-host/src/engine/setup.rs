//! What the owner is built from: its configuration, the dev table's machines and content, and the
//! errors of putting them together.
//!
//! The dev table registers the two real machines of C2, the lobby and a minimal explore, over one
//! authored scene and one authored beat. The action catalog is the stand-in for the action
//! registry of O.5: for each machine, the actions a seat can send it, which `viable` and the
//! interpreter read. Starting fails with a typed `EngineStartError` that names what to fix; a
//! machine that is not startable is never left out silently.

use std::collections::BTreeMap;
use std::fmt;

use df_table::config::Settings;
use df_table::effects::{EffectError, TableState, TableStateError};
use df_table::element::NameError;
use df_table::explore::{AuthoredBeat, ExploreMachine};
use df_table::identity::{
    Correlation, MachineId, OperationId, RecoveryEpoch, SceneId, SeatId, SessionRevision, TableId,
    TurnSeq,
};
use df_table::lobby::{LobbyAction, LobbyMachine};
use df_table::machine::{Entry, Machine, MachineError, SpecError};
use df_table::owner_queue::{QueueConfigError, SeatError};
use df_table::pipeline::{PipelineError, PipelineLimits, PipelineTable};
use df_table::ports::{ActionId, PortName};
use df_table::registry::{Registry, RegistryBuilder, RegistryError};
use df_table::table_state::MachineHome;

use crate::adapters::AdapterError;
use crate::broker::purpose::PurposeError;

use super::commit::CommitPolicy;
use super::limits::{DEFAULT_COMMIT_BACKOFF_MS, DEFAULT_COMMIT_DEADLINE_MS, DEV_SEAT_COUNT};

/// The id of the dev table's first and only scene.
const DEV_FIRST_SCENE: u64 = 1;
/// The authored beat of the dev scene and the check it offers.
const DEV_BEAT: &str = "locked_gate";
const DEV_CHECK: &str = "pick_lock";
/// The id of the one table the dev engine runs.
const DEV_TABLE: u64 = 1;

/// The ports the dev engine wires, which every enabled machine's required ports must be among.
pub const WIRED_PORTS: [PortName; 6] = [
    PortName::RulesOwner,
    PortName::Interpreter,
    PortName::Narrator,
    PortName::Broker,
    PortName::Dice,
    PortName::KnowledgeView,
];

/// The seat whose phone adjudicates rulings at the dev table (K.4): the first.
const DEV_HOST_SEAT: u64 = 1;

/// How the owner is configured.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub table: TableId,
    /// Members of the session, joined to the queue as seats 1 to this number.
    pub seats: u64,
    pub settings: Settings,
    pub limits: PipelineLimits,
    pub policy: CommitPolicy,
    pub commit_deadline_ms: u64,
    pub commit_backoff_ms: u64,
    pub first_scene: SceneId,
    /// Start with serving held, so inputs wait until `queue step` or `queue release`.
    pub start_held: bool,
}

impl EngineConfig {
    /// The dev table: six seats, the J.4 defaults, commits per batch.
    pub fn dev() -> Self {
        Self {
            table: TableId::new(DEV_TABLE),
            seats: DEV_SEAT_COUNT,
            settings: Settings::defaults(),
            limits: PipelineLimits::default(),
            policy: CommitPolicy::per_batch(),
            commit_deadline_ms: DEFAULT_COMMIT_DEADLINE_MS,
            commit_backoff_ms: DEFAULT_COMMIT_BACKOFF_MS,
            first_scene: SceneId::new(DEV_FIRST_SCENE),
            start_held: false,
        }
    }
}

/// Why the owner could not start.
#[derive(Debug)]
pub enum EngineStartError {
    Name(NameError),
    Spec(SpecError),
    Registry(Vec<RegistryError>),
    Machine(MachineError),
    Table(TableStateError),
    /// Starting the first machine was refused by the table.
    Setup(EffectError),
    Pipeline(PipelineError),
    Queue(QueueConfigError),
    Seat(SeatError),
    Purpose(PurposeError),
    /// The production rules adapters could not be built or the `rules_backend` setting is invalid.
    Adapter(AdapterError),
    /// The machine the table starts in is not registered and enabled.
    FirstMachineMissing(MachineId),
    /// The owner thread could not be started.
    Thread(String),
    /// The event spool that the environment asks for could not be opened.
    Spool {
        reason: String,
    },
}

impl fmt::Display for EngineStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(error) => write!(formatter, "a name is invalid: {error}"),
            Self::Spec(error) => write!(formatter, "a machine spec is invalid: {error}"),
            Self::Registry(errors) => {
                formatter.write_str("the machines cannot start:")?;
                errors
                    .iter()
                    .try_for_each(|error| write!(formatter, " {error};"))
            }
            Self::Machine(error) => write!(formatter, "a machine refused to start: {error}"),
            Self::Table(error) => write!(formatter, "the table cannot be built: {error}"),
            Self::Setup(error) => write!(formatter, "the first machine cannot start: {error}"),
            Self::Pipeline(error) => write!(formatter, "the pipeline cannot be built: {error}"),
            Self::Queue(error) => write!(formatter, "the owner queue cannot be built: {error}"),
            Self::Seat(error) => write!(formatter, "a seat cannot join the queue: {error}"),
            Self::Purpose(error) => write!(formatter, "the broker's purposes are invalid: {error}"),
            Self::Adapter(error) => write!(formatter, "the rules adapters cannot start: {error}"),
            Self::FirstMachineMissing(machine) => {
                write!(formatter, "the starting machine {machine} is not enabled")
            }
            Self::Thread(message) => write!(formatter, "the owner thread did not start: {message}"),
            Self::Spool { reason } => {
                write!(formatter, "the event spool cannot be opened: {reason}")
            }
        }
    }
}

impl std::error::Error for EngineStartError {}

macro_rules! start_error_from {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(
            impl From<$source> for EngineStartError {
                fn from(source: $source) -> Self {
                    Self::$variant(source)
                }
            }
        )+
    };
}

start_error_from!(
    NameError => Name,
    SpecError => Spec,
    Vec<RegistryError> => Registry,
    MachineError => Machine,
    TableStateError => Table,
    EffectError => Setup,
    PipelineError => Pipeline,
    QueueConfigError => Queue,
    SeatError => Seat,
    PurposeError => Purpose,
    AdapterError => Adapter,
);

/// For each machine, the actions a seat can send it: the stand-in for the action registry.
#[derive(Debug, Clone, Default)]
pub struct ActionCatalog {
    by_machine: BTreeMap<MachineId, Vec<ActionId>>,
}

impl ActionCatalog {
    pub fn with_machine(mut self, machine: MachineId, actions: Vec<ActionId>) -> Self {
        self.by_machine.insert(machine, actions);
        self
    }

    /// The actions a seat could send `machine`; empty for a machine the catalog does not know.
    pub fn actions(&self, machine: &MachineId) -> &[ActionId] {
        self.by_machine.get(machine).map_or(&[], Vec::as_slice)
    }
}

/// The dev table's machines, their catalog and where the table starts.
pub struct Machines {
    pub registry: Registry,
    pub catalog: ActionCatalog,
    pub first: MachineId,
    pub check: ActionId,
}

/// Registers the lobby and the minimal explore over the authored scene and beat.
pub fn dev_machines(first_scene: SceneId) -> Result<Machines, EngineStartError> {
    let lobby = LobbyMachine::new(first_scene)?;
    let explore = ExploreMachine::new(AuthoredBeat::new(DEV_BEAT, DEV_CHECK)?)?;
    let lobby_id = lobby.spec().id().clone();
    let explore_id = explore.spec().id().clone();
    let registry = RegistryBuilder::new()
        .register(lobby)
        .register(explore)
        .build(&WIRED_PORTS)?;
    let check = ActionId::new(DEV_CHECK)?;
    let lobby_actions = LobbyAction::ALL
        .iter()
        .map(|action| ActionId::new(action.key()))
        .collect::<Result<Vec<_>, _>>()?;
    let catalog = ActionCatalog::default()
        .with_machine(lobby_id.clone(), lobby_actions)
        .with_machine(explore_id, vec![check.clone()]);
    Ok(Machines {
        registry,
        catalog,
        first: lobby_id,
        check,
    })
}

/// The table with its first machine running, before anything has happened.
pub fn starting_world(
    config: &EngineConfig,
    machines: &Machines,
) -> Result<PipelineTable, EngineStartError> {
    let capacity = usize::try_from(config.seats).unwrap_or(usize::MAX);
    let facts = TableState::new(capacity)?;
    let first = machines
        .registry
        .machine(&machines.first)
        .ok_or_else(|| EngineStartError::FirstMachineMissing(machines.first.clone()))?;
    let slot = first.enter(&facts, &Entry::empty())?;
    let mut table = PipelineTable::with_limits(config.table, facts, &config.limits)
        .hosted_by(SeatId::new(DEV_HOST_SEAT));
    table.start_machine(setup_correlation(), MachineHome::Table, slot)?;
    Ok(table)
}

/// The correlation of the commit that starts the first machine: before the first input, so its
/// turn is 0 and its operation id is the smallest there is.
fn setup_correlation() -> Correlation {
    Correlation {
        operation_id: OperationId::from_nonzero_u128(std::num::NonZeroU128::MIN),
        turn_seq: TurnSeq::new(0),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, 0),
    }
}
