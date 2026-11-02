//! The conformance suite: the definition of done for a machine (O.5, H.17, the new-machine
//! checklist of the working plan).
//!
//! `run` plays one machine alone against scripted ports, in the part of the pipeline that exists
//! at step C2 (the pipeline itself is C3): it asks `accepts`, sends rules actions to the scripted
//! rules owner, opens a pending for a roll, draws dice from the queued dice, calls `step` twice,
//! commits the effects to a minimal table state, hands resource requests to the scripted broker
//! and records every event. Around that script it probes the machine's contract: its registration,
//! its closed state names, a projection and an inspect report that are never empty, the saved
//! record and its refusal to load what it cannot migrate, a typed failure on a damaged record, a
//! refusal of an entry the table disagrees with, a hand-off that names a declared target with a
//! typed carry list and leaves the table intact, `accepts` against unaddressed input, the dice
//! and `Waiting{pending_id}` flow, stray resource results, determinism, and effects that apply
//! cleanly. Each check has a name in `ConformanceCheck`, and the tests prove that a machine broken
//! in exactly that way fails exactly that check.
//!
//! Not yet run, because the part of the engine they test does not exist: the sentinel-secret and
//! paired-state tests (C5, H.11), the `dump`, `force`, `input` and `viable` verbs (C7, H.16),
//! suspend behaviour (C6; the declaration is part of the spec), the view revision of an input
//! (C3; machines never see it) and the golden sessions (C3 onward, H.17). A report is data: it
//! never panics, and it never fails a test by itself.

mod probes;
mod script;

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::effects::TableState;
use crate::event::TableEvent;
use crate::identity::SeatId;
use crate::listed::listed_enum;
use crate::machine::{Entry, Machine, MachineInput};
use crate::outcome::OutcomeKind;
use crate::ports::ActionId;
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner};

listed_enum! {
    /// One rule a machine must keep. The tests need a broken machine for each.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum ConformanceCheck {
        /// The spec registers: declared hand-off targets exist, scopes agree, ports are wired.
        Registration,
        /// Every state the machine reaches is one its spec names.
        StateNames,
        /// Every transition shows as a change of state name, and so as an event.
        TransitionEvents,
        /// A damaged record is a typed failure and never a guess.
        FailureEvents,
        /// A projection exists for every audience in every state.
        Projection,
        /// The inspect report names the machine, its state and its schema version.
        InspectReport,
        /// Saved state carries the machine's id and schema version and loads back unchanged.
        SavedState,
        /// A record the machine cannot migrate is refused.
        LoadRefusal,
        /// An entry the table disagrees with is refused.
        EntryRefusal,
        /// Exit names a declared target, carries only what the table holds and leaves it intact.
        ExitHandOff,
        /// Input that is not addressed to the machine is declined and changes nothing.
        Accepts,
        /// A roll opens a pending the machine waits on, and an unknown pending is declined.
        DiceAndWaiting,
        /// A resource request reaches the broker, and a result for no request is declined.
        ResourceResults,
        /// Equal state and input give equal results.
        Determinism,
        /// Every effect the machine proposes applies to the table it ran against.
        EffectsApply,
        /// The scripted run ends as the script says.
        RunAlone,
    }
}

/// One check that failed, and what was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: ConformanceCheck,
    pub detail: String,
}

/// What a run found, and every event it produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConformanceReport {
    findings: Vec<Finding>,
    events: Vec<TableEvent>,
}

impl ConformanceReport {
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub fn events(&self) -> &[TableEvent] {
        &self.events
    }

    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn failed_checks(&self) -> BTreeSet<ConformanceCheck> {
        self.findings.iter().map(|finding| finding.check).collect()
    }

    fn fail(&mut self, check: ConformanceCheck, detail: impl Into<String>) {
        self.findings.push(Finding {
            check,
            detail: detail.into(),
        });
    }
}

/// How one scripted step must end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedResult {
    Settled(OutcomeKind),
    Waiting,
}

/// What a scripted step must show: whether the machine accepts the input, how the step ends, the
/// state change it makes and whether the machine asks to hand off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
    pub accepted: bool,
    pub result: ExpectedResult,
    pub transition: Option<(&'static str, &'static str)>,
    pub asks_for_hand_off: bool,
}

impl Expectation {
    fn new(accepted: bool, result: ExpectedResult) -> Self {
        Self {
            accepted,
            result,
            transition: None,
            asks_for_hand_off: false,
        }
    }

    /// Accepted and applied, staying in the same state.
    pub fn applied() -> Self {
        Self::new(true, ExpectedResult::Settled(OutcomeKind::Applied))
    }

    /// Accepted, then declined (by the machine or by the rules owner).
    pub fn declined() -> Self {
        Self::new(true, ExpectedResult::Settled(OutcomeKind::Declined))
    }

    /// Not addressed to the machine, so declined with nothing changed.
    pub fn refused() -> Self {
        Self::new(false, ExpectedResult::Settled(OutcomeKind::Declined))
    }

    /// Accepted, and the machine reports an open pending.
    pub fn waiting() -> Self {
        Self::new(true, ExpectedResult::Waiting)
    }

    /// The step moves the machine between two of its states.
    pub fn moving(mut self, from: &'static str, to: &'static str) -> Self {
        self.transition = Some((from, to));
        self
    }

    /// The step ends with the machine asking the DM for a hand-off.
    pub fn handing_off(mut self) -> Self {
        self.asks_for_hand_off = true;
        self
    }
}

/// What the stand-in pipeline sends the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptedInput {
    /// A seat's intent. If the action is one of the spec's rules actions and the machine accepts
    /// it, the rules owner decides it first, and a roll opens a pending.
    Player { seat: SeatId, action: ActionId },
    /// The answer to the oldest open roll: the dice are drawn, the rules owner resolves the
    /// action, and the machine gets the `Dice` input with the resolution.
    AnswerOpenRoll,
    /// An input sent as it is, with no decision.
    Raw(MachineInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedStep {
    pub input: ScriptedInput,
    pub expect: Expectation,
}

impl ScriptedStep {
    pub fn player(seat: SeatId, action: ActionId, expect: Expectation) -> Self {
        Self {
            input: ScriptedInput::Player { seat, action },
            expect,
        }
    }

    pub fn answering_open_roll(expect: Expectation) -> Self {
        Self {
            input: ScriptedInput::AnswerOpenRoll,
            expect,
        }
    }

    pub fn raw(input: MachineInput, expect: Expectation) -> Self {
        Self {
            input: ScriptedInput::Raw(input),
            expect,
        }
    }
}

/// Everything a run needs: the machine, its neighbours for registration, the table it starts
/// on, how it is entered, the script and the scripted ports.
pub struct ConformanceFixture {
    pub machine: Arc<dyn Machine>,
    /// The machines the machine hands off to, so the registry can validate its targets.
    pub neighbours: Vec<Arc<dyn Machine>>,
    pub table: TableState,
    pub entry: Entry,
    /// An entry the table disagrees with and `enter` must refuse; none for a machine that reads
    /// nothing from its entry.
    pub refused_entry: Option<Entry>,
    pub script: Vec<ScriptedStep>,
    pub rules: ScriptedRulesOwner,
    pub dice: QueuedDice,
    pub broker: ScriptedBroker,
}

/// Plays the machine alone and reports every rule it broke.
pub fn run(fixture: ConformanceFixture) -> ConformanceReport {
    let mut report = ConformanceReport::default();
    probes::registration(&fixture, &mut report);
    probes::entry_refusal(&fixture, &mut report);
    script::play(fixture, &mut report);
    report
}
