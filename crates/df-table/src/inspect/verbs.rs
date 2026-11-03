//! The verb set behind the dev CLI, as data (H.16, RES33).
//!
//! It owns `Verb`, the closed list of verbs the engine fixes, and `VerbSpec`, what the host needs
//! to build each command: its words, usage and summary, whether it only reads or drives the table,
//! the least privileged audience that may run it and whether a machine must support it. The host
//! generates its commands, its `help` and the hosted build's list from this table, so a verb is
//! added in one place and a command that is not here does not exist. `force`, `input` and `dump`
//! are the verbs every machine must support (H.16); the others apply "where meaningful".
//!
//! A verb that can read hidden or private content, or that injects a fault, a fixture or a forced
//! state, is operator-only and absent from a hosted build (RES43, H.3, H.16). The family verbs of
//! later machines (combat, sheet, camera, director) are registered by their owners with their
//! machines and are not part of this set. It runs nothing: the host runs a verb through the
//! normal entry points.

use serde::Serialize;

use crate::listed::listed_enum;

listed_enum! {
    /// The verbs the engine fixes. A verb is added here and in `Verb::spec` or the build fails.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Verb {
        Describe,
        Dump,
        Force,
        Input,
        Viable,
        Dice,
        Clock,
        Replay,
        FixtureLoad,
        SimRun,
        Soak,
        Fuzz,
        Chaos,
        Health,
        Cost,
        Trace,
        PendingList,
        PendingShow,
        RulingsList,
        RulingsShow,
        ProjectionShow,
        Queue,
        Broker,
    }
}

listed_enum! {
    /// What a verb does to the table.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VerbEffect {
        /// Reads state and changes nothing.
        Reads,
        /// Changes the table, the clock, the dice or the fixture, through the normal entry points.
        Drives,
    }
}

listed_enum! {
    /// The least privileged audience that may run a verb. A hosted build offers `Host` verbs only.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum VerbAccess {
        Operator,
        Host,
    }
}

impl VerbAccess {
    /// Whether a hosted build offers the verb.
    pub fn in_hosted_build(self) -> bool {
        matches!(self, Self::Host)
    }
}

listed_enum! {
    /// Whether a machine must support a verb.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum MachineSupport {
        /// Every registered machine has this verb (`force`, `input`, `dump`).
        Required,
        /// A machine has it where it means something.
        Meaningful,
        /// The verb is about the table or a surface, not a machine.
        NotApplicable,
    }
}

/// What the host needs to build one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VerbSpec {
    pub verb: Verb,
    /// The words that name the command, as typed.
    pub words: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub effect: VerbEffect,
    pub access: VerbAccess,
    pub machines: MachineSupport,
}

const fn read(
    verb: Verb,
    words: &'static str,
    usage: &'static str,
    summary: &'static str,
    access: VerbAccess,
    machines: MachineSupport,
) -> VerbSpec {
    VerbSpec {
        verb,
        words,
        usage,
        summary,
        effect: VerbEffect::Reads,
        access,
        machines,
    }
}

const fn drive(
    verb: Verb,
    words: &'static str,
    usage: &'static str,
    summary: &'static str,
    machines: MachineSupport,
) -> VerbSpec {
    VerbSpec {
        verb,
        words,
        usage,
        summary,
        effect: VerbEffect::Drives,
        access: VerbAccess::Operator,
        machines,
    }
}

const NOT_A_MACHINE: MachineSupport = MachineSupport::NotApplicable;

const DESCRIBE: VerbSpec = read(
    Verb::Describe,
    "describe",
    "describe",
    "List the verbs, the registered machines and the fields of every report.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const DUMP: VerbSpec = read(
    Verb::Dump,
    "dump",
    "dump <machine|table> [--json]",
    "Print the state of a machine or of the table, including saved payloads.",
    VerbAccess::Operator,
    MachineSupport::Required,
);
const FORCE: VerbSpec = drive(
    Verb::Force,
    "force",
    "force <machine> <state>",
    "Put a machine into one of its states.",
    MachineSupport::Required,
);
const INPUT: VerbSpec = drive(
    Verb::Input,
    "input",
    "input <seat> <tap or text>",
    "Send an input as a seat would, through the owner queue.",
    MachineSupport::Required,
);
const VIABLE: VerbSpec = read(
    Verb::Viable,
    "viable",
    "viable <seat>",
    "List what the rules allow a seat now.",
    VerbAccess::Host,
    MachineSupport::Meaningful,
);
const DICE: VerbSpec = drive(
    Verb::Dice,
    "dice",
    "dice seed <number> | dice force <values>",
    "Seed the dice or queue the next draws.",
    NOT_A_MACHINE,
);
const CLOCK: VerbSpec = drive(
    Verb::Clock,
    "clock",
    "clock now | clock advance <ms> | clock freeze",
    "Read, advance or freeze the clock the table runs on.",
    NOT_A_MACHINE,
);
const REPLAY: VerbSpec = read(
    Verb::Replay,
    "replay",
    "replay <log> [--until seq] [--diff]",
    "Replay a recorded session against scripted ports and compare every record and state hash.",
    VerbAccess::Operator,
    NOT_A_MACHINE,
);
const FIXTURE_LOAD: VerbSpec = drive(
    Verb::FixtureLoad,
    "fixture load",
    "fixture load <bundle>",
    "Load a table bundle: state, recorded inputs and a recorded broker script.",
    NOT_A_MACHINE,
);
const SIM_RUN: VerbSpec = drive(
    Verb::SimRun,
    "sim run",
    "sim run --seats <n> --script <file> --broker scripted|real",
    "Run a scripted table of n seats.",
    NOT_A_MACHINE,
);
const SOAK: VerbSpec = drive(
    Verb::Soak,
    "soak",
    "soak [--tables <n>] [--seats <n>]",
    "Run many tables for a long time and check that work returns to zero and memory stays flat.",
    NOT_A_MACHINE,
);
const FUZZ: VerbSpec = drive(
    Verb::Fuzz,
    "fuzz",
    "fuzz --seed <number>",
    "Send random input sequences from a seed and check the invariants.",
    NOT_A_MACHINE,
);
const CHAOS: VerbSpec = drive(
    Verb::Chaos,
    "chaos",
    "chaos <dependency> <fault>",
    "Make one dependency fail in one way.",
    NOT_A_MACHINE,
);
const HEALTH: VerbSpec = read(
    Verb::Health,
    "health",
    "health",
    "Say whether the table is making progress.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const COST: VerbSpec = read(
    Verb::Cost,
    "cost",
    "cost",
    "Print what the table has spent.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const TRACE: VerbSpec = read(
    Verb::Trace,
    "trace",
    "trace <id>",
    "Follow one operation through every stage and port.",
    VerbAccess::Operator,
    NOT_A_MACHINE,
);
const PENDING_LIST: VerbSpec = read(
    Verb::PendingList,
    "pending list",
    "pending list",
    "List the open pending resolutions.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const PENDING_SHOW: VerbSpec = read(
    Verb::PendingShow,
    "pending show",
    "pending show <id>",
    "Show one pending with its offers and its continuation.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const RULINGS_LIST: VerbSpec = read(
    Verb::RulingsList,
    "rulings list",
    "rulings list",
    "List the stored rulings.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const RULINGS_SHOW: VerbSpec = read(
    Verb::RulingsShow,
    "rulings show",
    "rulings show <key>",
    "Show one stored ruling.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const PROJECTION_SHOW: VerbSpec = read(
    Verb::ProjectionShow,
    "projection show",
    "projection show <seat|tv>",
    "Print exactly what the shared screen or one seat's phone is sent.",
    VerbAccess::Operator,
    NOT_A_MACHINE,
);
const QUEUE: VerbSpec = read(
    Verb::Queue,
    "queue",
    "queue",
    "Show the owner queue: lane depths, seats and what is in progress.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);
const BROKER: VerbSpec = read(
    Verb::Broker,
    "broker",
    "broker",
    "Show the resource requests that are out.",
    VerbAccess::Host,
    NOT_A_MACHINE,
);

impl Verb {
    pub fn spec(self) -> &'static VerbSpec {
        match self {
            Self::Describe => &DESCRIBE,
            Self::Dump => &DUMP,
            Self::Force => &FORCE,
            Self::Input => &INPUT,
            Self::Viable => &VIABLE,
            Self::Dice => &DICE,
            Self::Clock => &CLOCK,
            Self::Replay => &REPLAY,
            Self::FixtureLoad => &FIXTURE_LOAD,
            Self::SimRun => &SIM_RUN,
            Self::Soak => &SOAK,
            Self::Fuzz => &FUZZ,
            Self::Chaos => &CHAOS,
            Self::Health => &HEALTH,
            Self::Cost => &COST,
            Self::Trace => &TRACE,
            Self::PendingList => &PENDING_LIST,
            Self::PendingShow => &PENDING_SHOW,
            Self::RulingsList => &RULINGS_LIST,
            Self::RulingsShow => &RULINGS_SHOW,
            Self::ProjectionShow => &PROJECTION_SHOW,
            Self::Queue => &QUEUE,
            Self::Broker => &BROKER,
        }
    }

    /// The verbs every machine must support: `force`, `input` and `dump`.
    pub fn machine_required() -> impl Iterator<Item = Verb> {
        Self::ALL
            .iter()
            .copied()
            .filter(|verb| verb.spec().machines == MachineSupport::Required)
    }
}
