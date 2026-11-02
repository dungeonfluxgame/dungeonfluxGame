//! The conformance suite: the definition of done for a machine (O.5, H.17, the new-machine
//! checklist of the working plan).
//!
//! `run` plays one machine alone against scripted ports, in the part of the pipeline that exists
//! at step C2 (the pipeline itself is C3): it asks `accepts`, sends rules actions to the scripted
//! rules owner, opens a pending for a roll, draws dice from the queued dice, calls `step` twice,
//! commits the effects to a minimal table state, hands resource requests to the scripted broker
//! and records every event. Around that script it probes the machine in every state it reaches,
//! whatever the script says: the saved record and its refusal to load what it cannot migrate or
//! read (each field removed, each field nulled, an older version), a projection and an inspect
//! report that are never empty and the same twice, input that is addressed to nothing the machine
//! holds (a stray roll, a stray resource result, a timer or proposal it did not claim, an unknown
//! action, a seat that is not at the table), a hand-off that the registry rules accept and the
//! table backs, and an ask for a hand-off that stays true until it is taken. A machine that
//! panics is reported, not allowed to end the run.
//!
//! Each check has a name in `ConformanceCheck`, and the tests prove that a machine broken in
//! exactly that way fails exactly that check. Rules that the types already guarantee (an event
//! built by `step_events`, a readout that cannot be blank) are not checked again here. A check
//! that could not run is listed in `ConformanceReport::skipped`, never counted as passed.
//!
//! Not yet run, because the part of the engine they test does not exist: the sentinel-secret and
//! paired-state tests (C5, H.11), the `dump`, `force`, `input` and `viable` verbs (C7, H.16),
//! suspend behaviour (C6; the declaration is part of the spec), the view revision of an input
//! (C3; machines never see it) and the golden sessions (C3 onward, H.17). A report is data: it
//! never fails a test by itself.

mod probes;
mod script;

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use crate::effects::{TableEffect, TableState};
use crate::event::TableEvent;
use crate::identity::SeatId;
use crate::listed::listed_enum;
use crate::machine::{Entry, Machine, MachineInput};
use crate::outcome::OutcomeKind;
use crate::ports::{ActionId, PortName};
use crate::testkit::{QueuedDice, ScriptedBroker, ScriptedRulesOwner};

listed_enum! {
    /// One rule a machine must keep. The tests need a broken machine for each.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum ConformanceCheck {
        /// The spec registers: declared hand-off targets exist, scopes agree, and every port the
        /// machine requires is one the fixture wires.
        Registration,
        /// Every state the machine reaches is one its spec names.
        StateNames,
        /// Every transition shows as a change of state name.
        TransitionEvents,
        /// A damaged record is a typed failure and never a guess.
        FailureEvents,
        /// A projection exists for every audience in every state.
        Projection,
        /// The inspect report names the machine, its state and its schema version.
        InspectReport,
        /// Saved state carries the machine's id and schema version and loads back unchanged.
        SavedState,
        /// A record the machine cannot migrate or read is refused, however it is damaged.
        LoadRefusal,
        /// An entry the table disagrees with is refused.
        EntryRefusal,
        /// Exit names a hand-off the registry rules accept and carries only what the table holds.
        ExitHandOff,
        /// Input that is not addressed to the machine is declined and changes nothing.
        Accepts,
        /// A declined or clarified result changes nothing, and a waiting one changes only state.
        DeclineChangesNothing,
        /// A roll opens a pending the machine waits on, and an unknown pending is declined.
        DiceAndWaiting,
        /// A resource request reaches the broker, and a result for a key no request used is
        /// declined.
        ResourceResults,
        /// The ask for a hand-off is derived from the state: it is the same every time it is
        /// asked, so a refused hand-off leaves the machine asking.
        HandOffAsk,
        /// Equal state and input give equal results, for every call that has one.
        Determinism,
        /// Every effect the machine proposes applies to the table it ran against.
        EffectsApply,
        /// The machine never panics.
        NoPanic,
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

/// What a run found, every event it produced, and the checks it could not run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConformanceReport {
    findings: Vec<Finding>,
    events: Vec<TableEvent>,
    skipped: Vec<ConformanceCheck>,
}

impl ConformanceReport {
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub fn events(&self) -> &[TableEvent] {
        &self.events
    }

    /// The checks that did not run, for a reason the spec allows. A skipped check is not a pass.
    pub fn skipped(&self) -> &[ConformanceCheck] {
        &self.skipped
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

/// Runs `part` and turns a panic in the machine into a `NoPanic` finding. The part is abandoned
/// where it panicked; the run goes on with the next.
fn guarded<R>(
    report: &mut ConformanceReport,
    label: &str,
    part: impl FnOnce(&mut ConformanceReport) -> R,
) -> Option<R> {
    let outcome = catch_unwind(AssertUnwindSafe(|| part(report)));
    match outcome {
        Ok(value) => Some(value),
        Err(_) => {
            report.fail(
                ConformanceCheck::NoPanic,
                format!("{label}: the machine panicked"),
            );
            None
        }
    }
}

/// How one scripted step must end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedResult {
    Settled(OutcomeKind),
    Waiting,
}

/// What a scripted step must show: whether the machine accepts the input, how the step ends, the
/// state change it makes and whether the machine then asks to hand off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
    pub accepted: bool,
    pub result: ExpectedResult,
    pub transition: Option<(&'static str, &'static str)>,
    pub wants_hand_off: bool,
}

impl Expectation {
    fn new(accepted: bool, result: ExpectedResult) -> Self {
        Self {
            accepted,
            result,
            transition: None,
            wants_hand_off: false,
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

    /// After the step the machine asks the DM for a hand-off.
    pub fn handing_off(mut self) -> Self {
        self.wants_hand_off = true;
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
    /// The host changes the table between inputs (a seat removed, a scene entered). The effect
    /// must apply; the machine is not called, and the expectation is not read.
    TableChange(TableEffect),
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

    pub fn table_change(effect: TableEffect) -> Self {
        Self {
            input: ScriptedInput::TableChange(effect),
            expect: Expectation::applied(),
        }
    }
}

/// Everything a run needs: the machine, its neighbours for registration, the ports the host
/// wires, the table it starts on, how it is entered, the script and the scripted ports.
pub struct ConformanceFixture {
    pub machine: Arc<dyn Machine>,
    /// The machines the machine hands off to, so the registry can validate its targets.
    pub neighbours: Vec<Arc<dyn Machine>>,
    /// The ports the host provides. A machine that requires another fails registration.
    pub wired: Vec<PortName>,
    pub table: TableState,
    pub entry: Entry,
    /// An entry the table disagrees with and `enter` must refuse. A machine whose spec says it
    /// reads its entry needs one; for one that does not, the check is listed as skipped.
    pub refused_entry: Option<Entry>,
    pub script: Vec<ScriptedStep>,
    pub rules: ScriptedRulesOwner,
    pub dice: QueuedDice,
    pub broker: ScriptedBroker,
}

/// Plays the machine alone and reports every rule it broke.
pub fn run(fixture: ConformanceFixture) -> ConformanceReport {
    let mut report = ConformanceReport::default();
    guarded(&mut report, "registration", |report| {
        probes::registration(&fixture, report);
    });
    guarded(&mut report, "entry", |report| {
        probes::entry_refusal(&fixture, report);
    });
    script::play(fixture, &mut report);
    report
}
