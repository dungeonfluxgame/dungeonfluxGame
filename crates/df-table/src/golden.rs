//! The golden-session runner: a script of inputs replayed against scripted ports (O.6, H.17).
//!
//! It owns `GoldenStep` (an input and what must come of it), `play` (feeds the steps to a `World`
//! one by one and compares each turn) and `replay_twice` (plays the script in two fresh worlds and
//! requires equal `TurnRecord`s and state digests, which is the determinism of replay). What a
//! step expects is written out by hand as an explicit sequence: how the input ended, whether the
//! state digest moved, the compact line of every event in order (the audit line without the keys
//! every line shares and without timings), the readout and the options offered. A golden is
//! written only after its trace was reviewed against the pinned source (RES52); nothing here
//! records one from a run.
//!
//! The runner reports every difference as text in the report and never panics, so a failing
//! golden says which step and what differed. Fakes outside their script still fail loudly, as
//! `testkit` says.

pub mod scene_machine;
pub mod world;

use crate::envelope::{EngineInput, ProposalOrigin, TimerName};
use crate::event::TableEvent;
use crate::identity::{Correlation, SessionRevision};
use crate::outcome::{DeclineCode, Outcome, PendingKind};
use crate::pipeline::TurnRecord;
use crate::testkit::correlation;

pub use scene_machine::{Leaves, SceneMachine};
pub use world::{CountingDice, FIRST_SCENE, FixedNarrator, SlowNarrator, TABLE, World, WorldError};

/// The keys a compact line leaves out: the table (one per world), the identity of the input, which
/// `play` checks on every line instead of making each golden repeat it, and the timings. The scene
/// and the machine stay, so a golden catches a line filed under the wrong scene or machine.
const LEFT_OUT_KEYS: [&str; 5] = [
    "table=",
    "operation_id=",
    "turn_seq=",
    "epoch=",
    "revision=",
];
const TIMING_KEY: &str = "ms=";

/// One input of a golden script.
#[derive(Debug, Clone)]
pub enum GoldenInput {
    Tap {
        seat: u64,
        action: &'static str,
    },
    Text {
        seat: u64,
        line: &'static str,
    },
    Answer {
        seat: u64,
        pending: u64,
        choice: &'static str,
    },
    Timer(&'static str),
    MachineProposal,
}

/// How an input must end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Applied,
    Clarify,
    Declined(DeclineCode),
    Pending(PendingKind),
}

/// Whether the state digest must move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    Changed,
    Unchanged,
}

/// What a step must come to.
#[derive(Debug, Clone, Copy)]
pub struct Expect {
    pub ending: Ending,
    pub state: StateChange,
    /// The compact line of every event of the step, in order.
    pub events: &'static [&'static str],
    pub readout: &'static str,
    /// The names the seat is asked to pick from, when it is.
    pub options: &'static [&'static str],
}

/// One input and what must come of it.
#[derive(Debug, Clone)]
pub struct GoldenStep {
    pub name: &'static str,
    pub input: GoldenInput,
    pub expect: Expect,
}

/// What a run found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoldenReport {
    pub records: Vec<TurnRecord>,
    pub failures: Vec<String>,
    pub final_hash: u64,
}

/// An event as a golden writes it: the audit line without the keys every line shares.
pub fn compact(event: &TableEvent) -> String {
    let line = event.render_audit_line();
    let kept: Vec<&str> = line
        .split(' ')
        .filter(|token| {
            !token.starts_with(TIMING_KEY)
                && !LEFT_OUT_KEYS.iter().any(|key| token.starts_with(key))
        })
        .collect();
    kept.join(" ")
}

fn ending_of(outcome: &Outcome) -> Ending {
    match outcome {
        Outcome::Applied => Ending::Applied,
        Outcome::NeedsClarification => Ending::Clarify,
        Outcome::Declined(decline) => Ending::Declined(decline.code()),
        Outcome::Pending(kind) => Ending::Pending(*kind),
    }
}

/// Plays the steps in `world` and compares each with what it expects.
pub fn play(world: &mut World, steps: &[GoldenStep]) -> GoldenReport {
    let mut report = GoldenReport::default();
    for step in steps {
        let seen_events = world.events().len();
        let expected_identity = correlation(world.next_turn());
        let table_revision = world.table.facts().snapshot().revision();
        let result = match step.input {
            GoldenInput::Tap { seat, action } => world.tap(seat, action),
            GoldenInput::Text { seat, line } => world.text(seat, line),
            GoldenInput::Answer {
                seat,
                pending,
                choice,
            } => world.answer(seat, pending, choice),
            GoldenInput::Timer(name) => match TimerName::new(name) {
                Ok(timer) => world.engine(EngineInput::Timer { timer }),
                Err(error) => {
                    report
                        .failures
                        .push(format!("{}: bad timer name: {error}", step.name));
                    continue;
                }
            },
            GoldenInput::MachineProposal => world.engine(EngineInput::Proposal {
                origin: ProposalOrigin::Machine,
                proposer: match crate::envelope::ProposerRef::new("scene") {
                    Ok(proposer) => proposer,
                    Err(error) => {
                        report
                            .failures
                            .push(format!("{}: bad proposer: {error}", step.name));
                        continue;
                    }
                },
            }),
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                report
                    .failures
                    .push(format!("{}: could not run: {error}", step.name));
                continue;
            }
        };
        if let Err(error) = world.narrate(&result) {
            report
                .failures
                .push(format!("{}: could not narrate: {error}", step.name));
        }
        let step_events: Vec<TableEvent> = world.events().into_iter().skip(seen_events).collect();
        check_identity(
            step,
            &step_events,
            Identity {
                input: expected_identity,
                table_revision,
            },
            &mut report.failures,
        );
        let seen: Vec<String> = step_events.iter().map(compact).collect();
        compare(step, &result, &seen, &mut report.failures);
        report.records.push(result.record);
    }
    report.final_hash = world.table.state_hash();
    report
}

/// What every line of a step must name: the input it was run with, and the revision the table
/// stood at when the step began.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Identity {
    pub(crate) input: Correlation,
    pub(crate) table_revision: SessionRevision,
}

/// Every line that names an input names this step's operation, sequence and epoch, and carries the
/// input's own revision. A table effect carries the table's revision instead, which is the
/// commit's and which the host stamps after the batch (S1), never the input's: a line that carried
/// the input's revision there would mean the pipeline stamped the table from an input.
pub(crate) fn check_identity(
    step: &GoldenStep,
    events: &[TableEvent],
    expected: Identity,
    failures: &mut Vec<String>,
) {
    for event in events {
        let line = event.render_audit_line();
        let names_an_input = line
            .split(' ')
            .any(|token| token.starts_with("operation_id="));
        if !names_an_input {
            continue;
        }
        let revision = if matches!(
            event,
            TableEvent::EffectApplied { .. } | TableEvent::EffectRefused { .. }
        ) {
            expected.table_revision
        } else {
            expected.input.revision
        };
        let wanted = [
            format!("operation_id={}", expected.input.operation_id),
            format!("turn_seq={}", expected.input.turn_seq),
            format!("epoch={}", revision.epoch().get()),
            format!("revision={}", revision.sequence()),
        ];
        for token in &wanted {
            if !line.split(' ').any(|found| found == token) {
                failures.push(format!(
                    "{}: the line `{}` does not carry {token}",
                    step.name,
                    compact(event)
                ));
            }
        }
    }
}

fn compare(
    step: &GoldenStep,
    result: &crate::pipeline::TurnResult,
    seen_events: &[String],
    failures: &mut Vec<String>,
) {
    let expect = &step.expect;
    let ending = ending_of(&result.outcome);
    if ending != expect.ending {
        failures.push(format!(
            "{}: ended {ending:?}, expected {:?}",
            step.name, expect.ending
        ));
    }
    let moved = result.record.state_hash_before != result.record.state_hash_after;
    if moved != (expect.state == StateChange::Changed) {
        failures.push(format!(
            "{}: the state digest {}, expected it to {:?}",
            step.name,
            if moved { "moved" } else { "stayed" },
            expect.state
        ));
    }
    let expected_events: Vec<&str> = expect.events.to_vec();
    let found_events: Vec<&str> = seen_events.iter().map(String::as_str).collect();
    if found_events != expected_events {
        failures.push(format!(
            "{}: events were\n  {}\nexpected\n  {}",
            step.name,
            found_events.join("\n  "),
            expected_events.join("\n  ")
        ));
    }
    if result.readout.as_str() != expect.readout {
        failures.push(format!(
            "{}: readout was {:?}, expected {:?}",
            step.name,
            result.readout.as_str(),
            expect.readout
        ));
    }
    let options: Vec<&str> = result
        .clarification
        .as_ref()
        .map(|clarification| {
            clarification
                .options
                .iter()
                .map(|option| option.as_str())
                .collect()
        })
        .unwrap_or_default();
    if options != expect.options {
        failures.push(format!(
            "{}: options were {options:?}, expected {:?}",
            step.name, expect.options
        ));
    }
}

/// Plays the script in two fresh worlds: both must meet every expectation and agree on every
/// record and on the final digest (a replay reproduces the state hash, H.17).
pub fn replay_twice(
    make: impl Fn() -> Result<World, WorldError>,
    steps: &[GoldenStep],
) -> GoldenReport {
    let mut first_world = match make() {
        Ok(world) => world,
        Err(error) => return failed_to_start(&error),
    };
    let first = play(&mut first_world, steps);
    let mut second_world = match make() {
        Ok(world) => world,
        Err(error) => return failed_to_start(&error),
    };
    let second = play(&mut second_world, steps);
    let mut report = first;
    if report.records != second.records || report.final_hash != second.final_hash {
        report
            .failures
            .push("a second replay of the script gave other records or another digest".to_owned());
    }
    report
}

fn failed_to_start(error: &WorldError) -> GoldenReport {
    GoldenReport {
        failures: vec![format!("the world could not be built: {error}")],
        ..GoldenReport::default()
    }
}
