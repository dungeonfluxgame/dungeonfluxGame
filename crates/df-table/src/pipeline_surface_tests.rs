//! The machine surface of the pipeline (`Pipeline::with_machine_surface`, H3): the buttons the
//! running machine states for a seat, and the line it adds to an applied readout. Each test says
//! which rule of `pipeline/offers.rs` or `pipeline/project.rs` it pins, with exact buttons and
//! exact text.

use std::sync::Arc;

use serde_json::json;

use crate::envelope::{InputEnvelope, PlayerInput, ViewStamp};
use crate::event::TableEvent;
use crate::explore::{AuthoredBeat, ExploreMachine};
use crate::golden::{FIRST_SCENE, TABLE, World, compact};
use crate::identity::{MachineId, SeatId};
use crate::lobby::LobbyMachine;
use crate::machine::slot::{MachineSlot, decodes_on_this_thread};
use crate::machine::{
    Decision, Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, ReadoutError, Scope, StandingOffers, StateName, StepInput, StepOutcome,
    StepResult, SuspendPolicy,
};
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pipeline::PipelineLimits;
use crate::pipeline_fixtures_tests::{action, seat};
use crate::ports::{
    ActionId, Audience, Awaiting, AwaitingKind, Interpretation, Offer, PortName, Preparation,
    Resolution,
};
use crate::projection::{AudienceProjection, project};
use crate::table_state::{MachineHome, TableState};
use crate::testkit::{ScriptedInterpreter, ScriptedRulesOwner, correlation};

const STRIKE: &str = "strike";
const DEGRADED: &str = "Some pictures are unavailable. Play continues.";

/// A group machine that states its standing buttons and its public line as it was told to, so a
/// test can put a pending, a clarification or a failure beside them. Any state it holds is
/// `{"phase": "open"}`; any other record is corrupt.
#[derive(Debug, Clone)]
struct Stage {
    spec: MachineSpec,
    standing: Vec<ActionId>,
    line: Option<&'static str>,
}

impl Stage {
    /// `standing` are the buttons it offers every seated seat, `line` its public reading (`None`
    /// is a machine whose projection fails). `strike` is its one rules action.
    fn new(standing: &[&str], line: Option<&'static str>) -> Self {
        let spec = MachineSpec::new(
            "stage",
            Scope::Group,
            1,
            &["open"],
            SuspendPolicy::FinishAndHold,
        )
        .expect("a valid spec")
        .requiring_port(PortName::RulesOwner)
        .with_rules_action(action(STRIKE))
        .expect("one rules action");
        Self {
            spec,
            standing: standing.iter().map(|name| action(name)).collect(),
            line,
        }
    }

    fn takes(&self, wanted: &ActionId) -> bool {
        wanted.as_str() == STRIKE || self.standing.contains(wanted)
    }
}

impl Machine for Stage {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, _table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        Ok(MachineSlot::encode(
            &self.spec,
            &json!({ "phase": "open" }),
        )?)
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        self.validate(state).is_ok()
            && match input {
                MachineInput::Intent { seat, action } => {
                    self.takes(action) && table.seats().contains(seat)
                }
                MachineInput::Dice { .. } | MachineInput::PendingCancelled { .. } => true,
                _ => false,
            }
    }

    fn step(&self, state: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        self.validate(state)?;
        if !self.accepts(input.table, &input.input, state) {
            return Ok(StepResult::declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )));
        }
        let mut result = StepResult::applied_in_place(Vec::new());
        if let Some(Decision::RollPending(pending) | Decision::Pending(pending)) = &input.decision {
            result.outcome = StepOutcome::Waiting(*pending);
        }
        Ok(result)
    }

    fn standing_offers(
        &self,
        table: &TableState,
        state: &MachineSlot,
        seats: &[SeatId],
    ) -> Result<StandingOffers, MachineError> {
        self.validate(state)?;
        Ok(seats
            .iter()
            .map(|seat| {
                let held = if table.seats().contains(seat) {
                    self.standing.clone()
                } else {
                    Vec::new()
                };
                (*seat, held)
            })
            .collect())
    }

    fn wants_hand_off(&self, _state: &MachineSlot) -> Result<bool, MachineError> {
        Ok(false)
    }

    fn exit(&self, _table: &TableState, _state: &MachineSlot) -> Result<HandOff, MachineError> {
        Ok(HandOff::new(self.spec.id().clone(), Vec::new())?)
    }

    fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError> {
        Ok(state.named_state(&self.spec, "phase")?)
    }

    fn validate(&self, state: &MachineSlot) -> Result<(), MachineError> {
        self.state_name(state).map(|_| ())
    }

    fn project(
        &self,
        _table: &TableState,
        state: &MachineSlot,
        _audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        self.validate(state)?;
        let line = self.line.ok_or(ReadoutError::Blank)?;
        Ok(MachineReadout::new(line)?)
    }

    fn inspect(&self, state: &MachineSlot) -> Result<InspectReport, MachineError> {
        Ok(InspectReport {
            machine: self.spec.id().clone(),
            state: self.state_name(state)?,
            schema_version: state.schema_version(),
        })
    }
}

/// The machines every registry needs (a table machine that starts the game, and what it hands to).
fn base_machines() -> Vec<Arc<dyn Machine>> {
    let beat = AuthoredBeat::new("gate", "open_gate").expect("a valid beat");
    vec![
        Arc::new(LobbyMachine::new(FIRST_SCENE).expect("the lobby builds")),
        Arc::new(ExploreMachine::new(beat).expect("the explore builds")),
    ]
}

fn stage_world(stage: Stage, seats: u64) -> World {
    let mut machines = base_machines();
    machines.push(Arc::new(stage));
    World::running(machines, "stage", seats, true)
        .expect("the world builds")
        .with_machine_surface()
}

fn lobby_world(seats: u64) -> World {
    World::running(base_machines(), "lobby", seats, false)
        .expect("the world builds")
        .with_machine_surface()
}

/// The buttons a seat's phone holds, as `id:label`.
fn buttons(world: &World, who: u64) -> Vec<String> {
    let snapshot = world.table.facts().snapshot();
    let attachments = world.table.attachments();
    match project(&snapshot, attachments, Audience::Seat(seat(who))).expect("the seat projects") {
        AudienceProjection::Seat(view) => view
            .offers()
            .iter()
            .map(|offer| format!("{}:{}", offer.offer_id.as_str(), offer.label.as_str()))
            .collect(),
        AudienceProjection::Shared(_) => Vec::new(),
    }
}

/// The part of a seat's attachment version that its own publishes made.
fn own_version(world: &World, who: u64) -> u64 {
    let attachments = world.table.attachments();
    attachments.version_for(Audience::Seat(seat(who))).get()
        - attachments.version_for(Audience::Public).get()
}

fn failed_parts(world: &World) -> Vec<String> {
    world
        .events()
        .iter()
        .filter(|event| matches!(event, TableEvent::Pipeline(_)))
        .map(compact)
        .filter(|line| line.contains("view_failed"))
        .collect()
}

fn strike_rules() -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_action(
            action(STRIKE),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action(STRIKE),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Reaction,
                offers: vec![Offer {
                    seat: seat(2),
                    choices: vec![action("shield"), action("pass")],
                }],
                draws: Vec::new(),
            })],
        )
        .with_resumptions(action(STRIKE), vec![Resolution::applied()])
}

#[test]
fn a_seat_with_an_open_pending_holds_only_the_pendings_buttons_and_gets_the_standing_ones_back() {
    let mut world = stage_world(Stage::new(&["look", "wave"], Some("The stage is set.")), 3);
    world.rules = strike_rules();

    let opened = world.tap(1, STRIKE).expect("the tap runs");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(buttons(&world, 2), ["shield:Shield", "pass:Pass"]);
    assert_eq!(buttons(&world, 1), ["look:Look", "wave:Wave"]);
    assert_eq!(buttons(&world, 3), ["look:Look", "wave:Wave"]);

    let answered = world.answer(2, 1, "shield").expect("the answer runs");

    assert_eq!(answered.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert_eq!(buttons(&world, 2), ["look:Look", "wave:Wave"]);
}

#[test]
fn a_seat_with_a_clarification_open_holds_only_its_options_until_it_closes() {
    let mut world = stage_world(Stage::new(&["look", "wave"], Some("The stage is set.")), 2);
    world.rules =
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action(STRIKE), action("search")]);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "do something",
        Interpretation::Ambiguous {
            candidates: vec![action(STRIKE), action("search")],
        },
    );

    let asked = world.text(1, "do something").expect("the line runs");

    assert_eq!(asked.outcome, Outcome::NeedsClarification);
    assert_eq!(buttons(&world, 1), ["strike:Strike", "search:Search"]);
    assert_eq!(buttons(&world, 2), ["look:Look", "wave:Wave"]);

    let closed = world.tap(1, "look").expect("the tap runs");

    assert_eq!(closed.outcome, Outcome::Applied);
    assert_eq!(buttons(&world, 1), ["look:Look", "wave:Wave"]);
}

#[test]
fn the_standing_buttons_follow_the_machine_across_a_hand_off() {
    let mut world = lobby_world(2);
    assert_eq!(buttons(&world, 1), Vec::<String>::new());

    world.tap(1, "ready").expect("seat 1 readies");
    assert_eq!(
        buttons(&world, 1),
        ["unready:Unready", "start:Start", "leave:Leave"]
    );
    assert_eq!(
        buttons(&world, 2),
        ["ready:Ready", "start:Start", "leave:Leave"]
    );
    world.tap(2, "ready").expect("seat 2 readies");
    world.tap(1, "start").expect("seat 1 starts");

    let moved = world
        .engine(crate::envelope::EngineInput::Proposal {
            origin: crate::envelope::ProposalOrigin::Machine,
            proposer: crate::envelope::ProposerRef::new("lobby").expect("a proposer"),
        })
        .expect("the hand-off runs");

    assert_eq!(moved.outcome, Outcome::Applied);
    assert_eq!(
        world.table.running_machine(),
        MachineId::new("explore").ok()
    );
    assert_eq!(buttons(&world, 1), ["open_gate:Open gate"]);
    assert_eq!(buttons(&world, 2), ["open_gate:Open gate"]);
}

#[test]
fn a_turn_that_changes_no_buttons_does_not_raise_the_seats_attachment_version() {
    let mut world = lobby_world(3);
    world.tap(1, "ready").expect("seat 1 readies");
    let before: Vec<u64> = (1..=3).map(|who| own_version(&world, who)).collect();

    world.tap(2, "ready").expect("seat 2 readies");

    let after: Vec<u64> = (1..=3).map(|who| own_version(&world, who)).collect();
    assert_eq!(
        buttons(&world, 3),
        ["ready:Ready", "start:Start", "leave:Leave"]
    );
    assert_eq!(after[0], before[0], "seat 1's buttons did not change");
    assert_eq!(after[2], before[2], "seat 3's buttons did not change");
    assert!(after[1] > before[1], "seat 2's buttons changed");
}

#[test]
fn the_machine_line_is_added_to_an_applied_readout_and_to_nothing_else() {
    let mut world = lobby_world(2);
    world.interpreter =
        ScriptedInterpreter::default().with_line("garbled", Interpretation::NeedsClarification);
    world.rules =
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("ready"), action("start")]);

    let declined = world.tap(1, "start").expect("the tap runs");
    let unclear = world.text(1, "garbled").expect("the line runs");
    let applied = world.tap(1, "ready").expect("the tap runs");

    assert!(matches!(declined.outcome, Outcome::Declined(_)));
    assert_eq!(
        declined.readout.as_str(),
        "Everyone at the table needs to be ready first."
    );
    assert_eq!(unclear.outcome, Outcome::NeedsClarification);
    assert_eq!(unclear.readout.as_str(), "I could not read that. Pick one:");
    assert_eq!(
        applied.readout.as_str(),
        "Seat 1: ready done. 1 of 2 seats are ready."
    );
}

#[test]
fn a_pending_outcome_gets_no_machine_line() {
    let mut world = World::running(base_machines(), "explore", 2, true)
        .expect("the world builds")
        .with_machine_surface();
    world.rules = ScriptedRulesOwner::default()
        .with_action(
            action("open_gate"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action("open_gate"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Roll,
                offers: Vec::new(),
                draws: vec![crate::pipeline_fixtures_tests::d20()],
            })],
        );

    let waiting = world.tap(1, "open_gate").expect("the tap runs");

    assert_eq!(waiting.outcome, Outcome::Pending(PendingKind::Roll));
    assert_eq!(waiting.readout.as_str(), "Waiting for seat 1 to roll.");
}

#[test]
fn a_replayed_delivery_shows_the_line_it_showed_the_first_time_once() {
    let mut world = lobby_world(2);
    let probe = correlation(1);
    let envelope = InputEnvelope::from_player(
        probe.operation_id,
        seat(1),
        ViewStamp::new(probe.revision),
        PlayerInput::Tap {
            action: action("ready"),
        },
    );

    let first = world.run_with(&envelope, probe).expect("the input runs");
    let again = world.run_with(&envelope, probe).expect("the repeat runs");

    assert!(!first.record.replayed && again.record.replayed);
    assert_eq!(
        first.readout.as_str(),
        "Seat 1: ready done. 1 of 2 seats are ready."
    );
    assert_eq!(again.readout.as_str(), first.readout.as_str());
}

#[test]
fn without_the_flag_the_readout_and_the_buttons_are_the_ones_the_table_always_had() {
    let mut world = lobby_world(2);
    world.machine_surface = false;

    let applied = world.tap(1, "ready").expect("the tap runs");

    assert_eq!(applied.readout.as_str(), "Seat 1: ready done.");
    assert_eq!(buttons(&world, 1), Vec::<String>::new());
    assert_eq!(buttons(&world, 2), Vec::<String>::new());
}

#[test]
fn a_machine_that_cannot_project_leaves_the_readout_as_it_was_and_the_failure_is_recorded() {
    let mut world = stage_world(Stage::new(&["look"], None), 2);

    let applied = world.tap(1, "look").expect("the tap runs");

    assert_eq!(applied.outcome, Outcome::Applied);
    assert_eq!(applied.readout.as_str(), "Seat 1: look done.");
    let failures = failed_parts(&world);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].ends_with("part=machine"), "{failures:?}");
    assert_eq!(buttons(&world, 1), ["look:Look"]);
    assert_eq!(projected_reason(&applied, 1).as_deref(), Some(DEGRADED));
}

fn projected_reason(result: &crate::pipeline::TurnResult, who: u64) -> Option<String> {
    result
        .projections
        .iter()
        .find_map(|projection| match projection {
            AudienceProjection::Seat(view)
                if view.viewer() == crate::projection::Viewer::Seat { seat: seat(who) } =>
            {
                Some(view.reason().map(|reason| reason.as_str().to_owned()))
            }
            _ => None,
        })
        .flatten()
}

#[test]
fn a_corrupt_slot_is_a_recorded_failure_with_a_degraded_view_and_no_standing_buttons() {
    let mut world = stage_world(Stage::new(&["look"], Some("The stage is set.")), 2);
    let corrupt = MachineSlot::from_parts(
        MachineId::new("stage").expect("an id"),
        1,
        json!({ "phase": "bogus" }),
    );
    world
        .table
        .start_machine(
            correlation(900_001),
            MachineHome::Scene { scene: FIRST_SCENE },
            corrupt,
        )
        .expect("the slot is set");

    let declined = world.tap(1, "look").expect("the tap runs");

    assert!(matches!(declined.outcome, Outcome::Declined(_)));
    let failures = failed_parts(&world);
    assert!(
        failures.iter().any(|line| line.ends_with("part=machine")),
        "{failures:?}"
    );
    assert_eq!(buttons(&world, 1), Vec::<String>::new());
    assert_eq!(buttons(&world, 2), Vec::<String>::new());
    assert_eq!(projected_reason(&declined, 1).as_deref(), Some(DEGRADED));
}

#[test]
fn a_running_machine_the_registry_does_not_hold_is_a_recorded_failure() {
    let mut world = stage_world(Stage::new(&["look"], Some("The stage is set.")), 2);
    let ghost = MachineSlot::from_parts(
        MachineId::new("ghost").expect("an id"),
        1,
        json!({ "phase": "open" }),
    );
    world
        .table
        .start_machine(
            correlation(900_002),
            MachineHome::Scene { scene: FIRST_SCENE },
            ghost,
        )
        .expect("the slot is set");

    let result = world.tap(1, "look").expect("the tap runs");

    let failures = failed_parts(&world);
    assert!(
        failures.iter().any(|line| line.ends_with("part=machine")),
        "{failures:?}"
    );
    assert_eq!(buttons(&world, 1), Vec::<String>::new());
    assert_eq!(projected_reason(&result, 1).as_deref(), Some(DEGRADED));
}

#[test]
fn a_restored_lobby_phone_holds_the_buttons_it_held_and_the_load_counts_no_publish() {
    let mut world = lobby_world(2);
    world.tap(1, "ready").expect("seat 1 readies");
    let now_ms = crate::ports::Clock::now_ms(world.clock.as_ref());
    let record = world.table.to_record(now_ms);
    let facts = world.table.facts().clone();
    let audiences = [
        Audience::Public,
        Audience::Seat(seat(1)),
        Audience::Seat(seat(2)),
    ];
    let saved: Vec<u64> = audiences
        .iter()
        .map(|audience| world.table.attachments().version_for(*audience).get())
        .collect();

    let loaded = world
        .with_pipeline(|pipeline, _| {
            pipeline.load_table(TABLE, facts.clone(), record.clone(), now_ms)
        })
        .expect("the world runs")
        .expect("the record loads");

    let held = |who: u64| -> Vec<String> {
        loaded
            .attachments()
            .offers_of(seat(who))
            .iter()
            .map(|offer| offer.id().as_str().to_owned())
            .collect()
    };
    assert_eq!(held(1), ["unready", "start", "leave"]);
    assert_eq!(held(2), ["ready", "start", "leave"]);
    let restored: Vec<u64> = audiences
        .iter()
        .map(|audience| loaded.attachments().version_for(*audience).get())
        .collect();
    assert_eq!(restored, saved);

    let plain = crate::pipeline::PipelineTable::from_record(
        TABLE,
        facts,
        record,
        now_ms,
        &PipelineLimits::default(),
    )
    .expect("the record loads without the surface");
    assert!(
        plain.attachments().offers_of(seat(1)).is_empty(),
        "a table loaded for a pipeline without the surface holds no standing buttons"
    );
}

#[test]
fn the_decodes_of_a_turn_do_not_grow_with_the_seats_at_the_table() {
    let decodes_of_a_turn = |seats: u64| {
        let mut world = lobby_world(seats);
        let before = decodes_on_this_thread();
        world.tap(1, "ready").expect("seat 1 readies");
        decodes_on_this_thread() - before
    };

    let one = decodes_of_a_turn(1);
    let six = decodes_of_a_turn(6);

    assert!(one > 0);
    assert_eq!(six, one, "one decode for every seat, not one per seat");
}
