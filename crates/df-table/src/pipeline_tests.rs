//! The stages of the pipeline over scripted ports: admission, interpretation, plans, engine
//! inputs, hand-offs and the correlation every port call carries.

use std::sync::Mutex;

use crate::effects::TableEffect;
use crate::envelope::{
    EngineInput, InputEnvelope, PlayerInput, ProposalOrigin, ProposerRef, TimerName, ViewStamp,
};
use crate::event::{FailureCause, PipelineStep, TableEvent};
use crate::golden::{FIRST_SCENE, Leaves, SceneMachine, World};
use crate::identity::{Correlation, SeatId};
use crate::outcome::{DeclineCode, Outcome, OutcomeKind, PendingKind};
use crate::pipeline::{MAXIMUM_PLAN_STEPS, PENDING_NUDGE_TIMER, Pipeline, PipelineLimits, Ports};
use crate::pipeline_fixtures_tests::{
    action, allows, changes, failures_of, finishing_world, lines, lines_after, narrated, ran,
    refuses, seat, steps_of, world_of,
};
use crate::ports::{
    ActionId, Broker, CommittedOutcome, ConfirmationReason, Dice, Draw, DrawRequest,
    Interpretation, Interpreter, Narrator, PortError, PortName, Preparation, PurposeName,
    Readiness, Resolution, ResourceHandle, Resumption, RulesOwner, RulingRequest,
};
use crate::private_data::{Narration, PlayerText};
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner, correlation};

fn wave_world() -> World {
    world_of(&["wave"], 2, allows(ScriptedRulesOwner::default(), "wave"))
}

#[test]
fn a_tap_runs_every_stage_in_order_and_releases_what_it_changed_after_the_commit() {
    let mut world = wave_world();

    let result = ran(&mut world, 1, "wave");
    let narration = narrated(&mut world, &result);

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(
        lines(&world),
        [
            "STEP finished scene=1 machine=scene step=admit",
            "STEP finished scene=1 machine=scene step=check",
            "STEP finished scene=1 machine=scene step=decide",
            "STEP finished scene=1 machine=scene step=apply",
            "STEP finished scene=1 machine=scene step=narrate",
            "STEP finished scene=1 machine=scene step=project outcome=applied",
            "EFFECT applied scene=1 machine=scene effect=beat_resolved",
            "EFFECT applied scene=1 machine=scene effect=active_slot_updated",
            "TURN applied scene=1 machine=scene",
            "NARRATION attached scene=1 machine=scene",
        ],
        "the stages record when they finish; the changes are released by the host after its commit"
    );
    assert_eq!(result.readout.as_str(), "Seat 1: wave done.");
    assert_eq!(narration.as_deref(), Some("The moment passes."));
    assert_eq!(
        changes(&result),
        [&TableEffect::BeatResolved {
            scene: FIRST_SCENE,
            beat: crate::effects::BeatName::new("wave").expect("a valid beat")
        }]
    );
    assert_ne!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
}

#[test]
fn nothing_a_turn_changed_is_visible_before_the_host_releases_it() {
    let mut world = wave_world();
    let probe = correlation(world.next_turn());
    let envelope = InputEnvelope::from_player(
        probe.operation_id,
        seat(1),
        ViewStamp::new(probe.revision),
        PlayerInput::Tap {
            action: action("wave"),
        },
    );

    let result = world
        .run_unreleased(&envelope, probe)
        .expect("the turn runs");
    let before_release = lines(&world);
    result.release_events(&world.sink);

    assert!(
        !before_release
            .iter()
            .any(|line| line.starts_with("EFFECT") || line.starts_with("TURN")),
        "no change of the table is recorded before the commit: {before_release:?}"
    );
    assert!(
        lines(&world)
            .iter()
            .any(|line| line.starts_with("TURN applied"))
    );
    assert!(result.committed_events.len() >= 3);
}

#[test]
fn a_tap_by_a_seat_that_is_not_at_the_table_is_declined_and_changes_nothing() {
    let mut world = wave_world();

    let result = ran(&mut world, 9, "wave");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
    assert_eq!(world.rules.calls(), Vec::<RulesCall>::new());
}

#[test]
fn a_declined_action_carries_the_rules_owners_decline_and_commits_nothing() {
    let rules = refuses(
        ScriptedRulesOwner::default(),
        "wave",
        DeclineCode::EconomySpent,
    );
    let mut world = world_of(&["wave"], 2, rules);

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.record.declined, Some(DeclineCode::EconomySpent));
    assert_eq!(
        result.readout.as_str(),
        "You have already used that this turn."
    );
    assert_eq!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
    assert!(changes(&result).is_empty());
    assert!(
        lines(&world)
            .contains(&"INPUT declined scene=1 machine=scene code=economy_spent".to_owned())
    );
}

#[test]
fn every_outcome_of_the_stage_events_names_the_machine_and_the_stage() {
    let mut world = wave_world();
    ran(&mut world, 1, "wave");
    ran(&mut world, 9, "wave");

    let events = world.events();

    let steps = steps_of(&events);
    assert!(steps.contains(&PipelineStep::Project));
    assert_eq!(
        steps
            .iter()
            .filter(|step| **step == PipelineStep::Project)
            .count(),
        2
    );
    assert!(events.iter().all(|event| match event {
        TableEvent::StepFinished { scope, .. } => scope.machine.is_some(),
        _ => true,
    }));
}

#[test]
fn a_typed_line_is_interpreted_into_a_registered_action_and_applied() {
    let mut world = wave_world();
    world.interpreter = ScriptedInterpreter::default()
        .with_line("I wave hello", Interpretation::Action(action("wave")));
    world.rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );

    let result = world.text(1, "I wave hello").expect("the line runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert!(steps_of(&world.events()).contains(&PipelineStep::Interpret));
    assert_eq!(world.interpreter.calls().first_attempts, 1);
    assert_eq!(world.interpreter.calls().retries, 0);
}

#[test]
fn a_plan_of_two_steps_is_applied_in_one_commit() {
    let rules = allows(allows(ScriptedRulesOwner::default(), "wave"), "bow")
        .with_choices(seat(1), vec![action("wave"), action("bow")]);
    let mut world = world_of(&["wave", "bow"], 2, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "wave then bow",
        Interpretation::Plan(vec![action("wave"), action("bow")]),
    );

    let result = world.text(1, "wave then bow").expect("the line runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(changes(&result).len(), 2);
    let applied = lines(&world)
        .iter()
        .filter(|line| line.starts_with("TURN applied"))
        .count();
    assert_eq!(applied, 1);
}

#[test]
fn a_plan_that_fails_at_its_second_step_applies_the_prefix_and_asks_about_the_step() {
    let rules = refuses(
        allows(ScriptedRulesOwner::default(), "wave"),
        "bow",
        DeclineCode::MissingRequirement,
    )
    .with_choices(seat(1), vec![action("wave"), action("bow")]);
    let mut world = world_of(&["wave", "bow"], 2, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "wave then bow",
        Interpretation::Plan(vec![action("wave"), action("bow")]),
    );

    let result = world.text(1, "wave then bow").expect("the line runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(changes(&result).len(), 1);
    assert_eq!(
        result.unfinished.as_ref().map(|decline| decline.code()),
        Some(DeclineCode::MissingRequirement)
    );
    assert!(
        lines(&world).contains(
            &"INPUT plan_partly_applied scene=1 machine=scene applied=1 total=2".to_owned()
        )
    );
    assert!(
        result
            .readout
            .as_str()
            .ends_with("You don't have what that needs.")
    );
}

#[test]
fn a_plan_of_more_than_six_steps_is_refused_as_text_that_is_not_valid() {
    let steps: Vec<ActionId> = (0..=MAXIMUM_PLAN_STEPS).map(|_| action("wave")).collect();
    let mut world = wave_world();
    world.rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );
    world.interpreter =
        ScriptedInterpreter::default().with_line("wave a lot", Interpretation::Plan(steps));

    let result = world.text(1, "wave a lot").expect("the line runs");

    assert_eq!(result.outcome, Outcome::NeedsClarification);
    assert_eq!(
        result
            .clarification
            .as_ref()
            .map(|c| (c.cause, c.keeps_draft)),
        Some((crate::pipeline::ClarifyCause::Unreadable, true)),
        "a plan of the wrong shape is a failed reading like any other (C.7)"
    );
    assert_eq!(world.interpreter.calls().retries, 1);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Interpreter, FailureCause::TextInvalid)]
    );
}

#[test]
fn the_safety_card_pauses_without_a_model_call_or_a_check() {
    let mut world = wave_world();
    let probe = correlation(world.next_turn());
    let envelope = InputEnvelope::from_player(
        probe.operation_id,
        seat(1),
        ViewStamp::new(probe.revision),
        PlayerInput::SafetyCard,
    );

    let result = world.run_envelope(&envelope).expect("the card runs");

    assert!(result.pause_requested);
    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(world.rules.calls(), Vec::<RulesCall>::new());
    assert_eq!(world.interpreter.calls().first_attempts, 0);
}

#[test]
fn a_private_message_is_not_run_yet_and_is_declined_as_not_supported() {
    let mut world = wave_world();
    let probe = correlation(world.next_turn());
    let envelope = InputEnvelope::from_player(
        probe.operation_id,
        seat(1),
        ViewStamp::new(probe.revision),
        PlayerInput::PrivateMessage {
            utterance: crate::envelope::UtteranceId::new(1),
            to: seat(2),
            text: PlayerText::new("psst"),
        },
    );

    let result = world.run_envelope(&envelope).expect("the message runs");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
}

#[test]
fn a_line_that_is_not_an_action_is_routed_to_its_handler_and_runs_nothing() {
    for kind in crate::ports::NonActionKind::ALL {
        let mut world = wave_world();
        world.rules = ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]);
        world.interpreter = ScriptedInterpreter::default()
            .with_line("is the door locked", Interpretation::NonAction(*kind));
        let before = world.table.state_hash();

        let result = world.text(1, "is the door locked").expect("the line runs");

        assert_eq!(result.outcome, Outcome::Applied, "{kind:?}");
        assert_eq!(result.routed, Some(*kind));
        assert!(
            changes(&result).is_empty(),
            "nothing is committed for {kind:?}"
        );
        assert_eq!(world.dice.drawn(), 0);
        assert!(
            !world
                .rules
                .calls()
                .iter()
                .any(|call| matches!(call, RulesCall::Prepare(_) | RulesCall::Resolve { .. })),
            "a line that is not an action is never prepared or resolved"
        );
        assert!(lines(&world).contains(&format!(
            "INPUT routed scene=1 machine=scene kind={}",
            kind.key()
        )));
        assert!(result.narration_job.is_none());
        let _ = before;
    }
}

#[test]
fn a_view_older_than_the_seats_last_change_of_choices_is_checked_again_not_dropped() {
    let mut world = wave_world();
    let newer = correlation(50).revision;
    world.table.stamp_choices_changed(seat(1), newer);

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.outcome, Outcome::Applied);
    assert!(lines(&world).contains(&"INPUT rechecked scene=1 machine=scene".to_owned()));
}

#[test]
fn a_timer_the_machine_claims_changes_its_state_and_is_applied() {
    let mut world = wave_world();
    let timer = TimerName::new("tick").expect("a valid timer name");

    let result = world
        .engine(EngineInput::Timer { timer })
        .expect("the timer runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_ne!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
    assert!(!steps_of(&world.events()).contains(&PipelineStep::Interpret));
    assert!(!steps_of(&world.events()).contains(&PipelineStep::Decide));
}

#[test]
fn a_proposal_the_machine_claims_is_applied_and_skips_interpretation_and_dice() {
    let mut world = wave_world();
    let proposer = ProposerRef::new("innkeeper").expect("a valid proposer");

    let result = world
        .engine(EngineInput::Proposal {
            origin: ProposalOrigin::Npc,
            proposer,
        })
        .expect("the proposal runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 0);
}

#[test]
fn a_resource_result_reaches_the_machine_that_made_the_request_by_its_handle() {
    let scene = SceneMachine::new(&[("ask_map", Leaves::Nothing)])
        .and_then(|scene| {
            scene
                .requesting_on("ask_map", "map", 7)
                .map_err(crate::machine::SpecError::InvalidName)
        })
        .expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 1).expect("the world builds");
    world.rules = allows(ScriptedRulesOwner::default(), "ask_map");
    ran(&mut world, 1, "ask_map");
    assert_eq!(
        world.broker.requests(),
        [PurposeName::new("map").expect("valid")]
    );

    let result = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(1),
            readiness: Readiness::Ready,
        })
        .expect("the result runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_ne!(
        result.record.state_hash_before,
        result.record.state_hash_after
    );
}

#[test]
fn a_failed_resource_result_is_recorded_once_and_still_reaches_the_machine() {
    let scene = SceneMachine::new(&[("ask_map", Leaves::Nothing)])
        .and_then(|scene| {
            scene
                .requesting_on("ask_map", "map", 7)
                .map_err(crate::machine::SpecError::InvalidName)
        })
        .expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 1).expect("the world builds");
    world.rules = allows(ScriptedRulesOwner::default(), "ask_map");
    ran(&mut world, 1, "ask_map");

    let result = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(1),
            readiness: Readiness::Failed(FailureCause::TimedOut),
        })
        .expect("the result runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Broker, FailureCause::TimedOut)]
    );
}

#[test]
fn a_resource_result_for_a_handle_nobody_made_is_stale_and_declined() {
    let mut world = wave_world();

    let result = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(99),
            readiness: Readiness::Ready,
        })
        .expect("the result runs");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Broker, FailureCause::Stale)]
    );
}

#[test]
fn a_resource_result_that_is_still_pending_is_not_an_input() {
    let mut world = wave_world();

    let result = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(1),
            readiness: Readiness::Pending,
        })
        .expect("the result runs");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
}

#[test]
fn a_machine_that_is_finished_asks_for_a_hand_off_and_the_proposal_moves_the_table() {
    let mut world = finishing_world(
        &["wave"],
        2,
        1,
        allows(ScriptedRulesOwner::default(), "wave"),
    );
    let turn = ran(&mut world, 1, "wave");
    assert!(turn.hand_off_wanted);

    let seen = world.events().len();
    let handed = world
        .engine(EngineInput::Proposal {
            origin: ProposalOrigin::Machine,
            proposer: ProposerRef::new("scene").expect("valid"),
        })
        .expect("the proposal runs");

    assert_eq!(handed.outcome, Outcome::Applied);
    assert_eq!(
        world
            .table
            .facts()
            .snapshot()
            .table_machine()
            .map(|machine| machine.machine().as_str().to_owned()),
        Some("lobby".to_owned()),
        "the lobby is the table machine now; the scene's own record is closed by the scene end"
    );
    let after = lines_after(&world, seen);
    assert!(after.contains(
        &"MACHINE exited scene=1 machine=scene kind=machine target=lobby carried=0".to_owned()
    ));
    assert!(after.contains(&"MACHINE entered scene=1 machine=lobby state=gathering".to_owned()));
}

#[test]
fn a_hand_off_is_declined_pending_open_while_a_pending_resolution_is_open() {
    let rules = allows(allows(ScriptedRulesOwner::default(), "wave"), "stab")
        .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let mut world = finishing_world(&["wave", "stab"], 2, 1, rules);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab him", Interpretation::Action(action("stab")))
        .with_line("wave", Interpretation::Action(action("wave")));
    world.rules = allows(
        allows(
            ScriptedRulesOwner::default().with_choices(seat(2), vec![action("stab")]),
            "wave",
        ),
        "stab",
    )
    .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let opened = world.text(2, "stab him").expect("the line runs");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Choice));
    let finished = ran(&mut world, 1, "wave");
    assert!(finished.hand_off_wanted);

    let proposal = world
        .engine(EngineInput::Proposal {
            origin: ProposalOrigin::Machine,
            proposer: ProposerRef::new("scene").expect("valid"),
        })
        .expect("the proposal runs");

    assert_eq!(proposal.record.declined, Some(DeclineCode::PendingOpen));
    assert!(world.table.facts().snapshot().table_machine().is_none());
}

#[test]
fn a_hand_off_proposal_from_a_machine_that_is_not_finished_is_declined() {
    let mut world = wave_world();

    let result = world
        .engine(EngineInput::Proposal {
            origin: ProposalOrigin::Machine,
            proposer: ProposerRef::new("scene").expect("valid"),
        })
        .expect("the proposal runs");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
}

#[test]
fn the_nudge_timer_sweeps_pendings_and_resolves_nothing() {
    let rules = allows(ScriptedRulesOwner::default(), "stab")
        .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let mut world = world_of(&["stab"], 2, rules);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab him", Interpretation::Action(action("stab")));
    world.rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("stab")]),
        "stab",
    )
    .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    world.text(1, "stab him").expect("the line runs");
    let timer = TimerName::new(PENDING_NUDGE_TIMER).expect("a valid timer name");

    world.clock.advance_ms(1_000);
    let early = world
        .engine(EngineInput::Timer {
            timer: timer.clone(),
        })
        .expect("runs");
    world.clock.advance_ms(30_000);
    let late = world
        .engine(EngineInput::Timer {
            timer: timer.clone(),
        })
        .expect("runs");
    let again = world.engine(EngineInput::Timer { timer }).expect("runs");

    let nudges = lines(&world)
        .iter()
        .filter(|line| line.starts_with("PENDING nudged"))
        .count();
    assert_eq!(
        (
            early.outcome.kind(),
            late.outcome.kind(),
            again.outcome.kind()
        ),
        (
            OutcomeKind::Applied,
            OutcomeKind::Applied,
            OutcomeKind::Applied
        )
    );
    assert_eq!(nudges, 1);
    assert_eq!(world.table.pending().len(), 1);
}

/// Records the correlation of every call a port receives.
struct Spy<T> {
    inner: T,
    seen: Mutex<Vec<Correlation>>,
}

impl<T> Spy<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, correlation: Correlation) {
        self.seen.lock().expect("the spy lock").push(correlation);
    }

    fn correlations(&self) -> Vec<Correlation> {
        self.seen.lock().expect("the spy lock").clone()
    }
}

impl RulesOwner for Spy<ScriptedRulesOwner> {
    fn legal_choices(
        &self,
        correlation: Correlation,
        seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError> {
        self.note(correlation);
        self.inner.legal_choices(correlation, seat)
    }
    fn prepare(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError> {
        self.note(correlation);
        self.inner.prepare(correlation, seat, action)
    }
    fn resolve(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
    ) -> Result<Resolution, PortError> {
        self.note(correlation);
        self.inner.resolve(correlation, seat, action, draws)
    }
    fn confirmation_needed(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<ConfirmationReason>, PortError> {
        self.note(correlation);
        self.inner.confirmation_needed(correlation, seat, action)
    }
    fn ruling_needed(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<RulingRequest>, PortError> {
        self.note(correlation);
        self.inner.ruling_needed(correlation, seat, action)
    }
    fn resume(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        resumption: &Resumption<'_>,
    ) -> Result<Resolution, PortError> {
        self.note(correlation);
        self.inner.resume(correlation, seat, action, resumption)
    }
}

impl Interpreter for Spy<ScriptedInterpreter> {
    fn interpret(
        &self,
        correlation: Correlation,
        seat: SeatId,
        text: &PlayerText,
        legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        self.note(correlation);
        self.inner.interpret(correlation, seat, text, legal_choices)
    }
}

impl Narrator for Spy<crate::golden::FixedNarrator> {
    fn narrate(
        &self,
        correlation: Correlation,
        committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        self.note(correlation);
        self.inner.narrate(correlation, committed)
    }
}

impl Dice for Spy<crate::golden::CountingDice> {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        self.note(correlation);
        self.inner.draw(correlation, request)
    }
}

impl Broker for Spy<crate::testkit::ScriptedBroker> {
    fn request(
        &self,
        correlation: Correlation,
        purpose: &PurposeName,
    ) -> Result<ResourceHandle, PortError> {
        self.note(correlation);
        self.inner.request(correlation, purpose)
    }
    fn readiness(
        &self,
        correlation: Correlation,
        handle: ResourceHandle,
    ) -> Result<Readiness, PortError> {
        self.note(correlation);
        self.inner.readiness(correlation, handle)
    }
}

#[test]
fn one_correlation_goes_through_every_port_call_of_an_input() {
    let rules = Spy::new(
        allows(
            ScriptedRulesOwner::default().with_choices(seat(1), vec![action("roll_check")]),
            "roll_check",
        )
        .with_action(
            action("roll_check"),
            Preparation::Ready {
                draws: vec![crate::pipeline_fixtures_tests::d20()],
            },
            Resolution::applied(),
        ),
    );
    let interpreter = Spy::new(
        ScriptedInterpreter::default()
            .with_line("roll", Interpretation::Action(action("roll_check"))),
    );
    let narrator = Spy::new(crate::golden::FixedNarrator("Fine.".to_owned()));
    let dice = Spy::new(crate::golden::CountingDice::new([12]));
    let broker = Spy::new(crate::testkit::ScriptedBroker::default());
    let world = world_of(&["roll_check"], 1, ScriptedRulesOwner::default());
    let clock = crate::testkit::ManualClock::default();
    let knowledge = crate::testkit::FixedKnowledgeView::default();
    let sink = crate::testkit::RecordingSink::default();
    let ports = Ports {
        rules: &rules,
        interpreter: &interpreter,
        narrator: &narrator,
        broker: &broker,
        clock: &clock,
        dice: &dice,
        knowledge: &knowledge,
        sink: &sink,
    };
    let pipeline =
        Pipeline::new(ports, &world.registry, &world.settings).expect("the pipeline builds");
    let mut table = world.table;
    let probe = correlation(7);
    let envelope = InputEnvelope::from_player(
        probe.operation_id,
        seat(1),
        ViewStamp::new(probe.revision),
        PlayerInput::SubmitText {
            utterance: crate::envelope::UtteranceId::new(1),
            text: PlayerText::new("roll"),
        },
    );

    let result = pipeline.run(&mut table, &envelope, probe);

    assert_eq!(result.outcome, Outcome::Applied);
    let all: Vec<Correlation> = [
        rules.correlations(),
        interpreter.correlations(),
        narrator.correlations(),
        dice.correlations(),
    ]
    .concat();
    assert!(all.len() >= 6, "the input made {} port calls", all.len());
    assert!(all.iter().all(|seen| *seen == probe));
}

#[test]
fn a_failed_machine_step_declines_and_records_the_machine_failure_once() {
    let mut world = wave_world();
    // A saved state the scene cannot read makes its step fail, as a damaged record would.
    let broken = crate::machine_slot::MachineSlot::from_parts(
        crate::machine::MachineId::new("scene").expect("valid"),
        1,
        serde_json::json!({"phase": "gone"}),
    );
    world
        .table
        .start_machine(
            correlation(2_000_000),
            crate::table_state::MachineHome::Scene { scene: FIRST_SCENE },
            broken,
        )
        .expect("the damaged state is installed");

    let result = ran(&mut world, 1, "wave");

    assert!(matches!(result.outcome, Outcome::Declined(_)));
    let failed: Vec<String> = lines(&world)
        .into_iter()
        .filter(|line| line.starts_with("MACHINE failed"))
        .collect();
    assert_eq!(
        failed,
        ["MACHINE failed scene=1 machine=scene cause=slot_malformed"]
    );
}

#[test]
fn limits_of_the_pipeline_start_very_large() {
    let limits = PipelineLimits::default();

    assert!(limits.maximum_open_pendings >= 1_000_000);
    assert!(limits.maximum_pending_nesting >= 1_000_000);
}

fn declined_with(alternatives: &[&str]) -> String {
    let mut decline = crate::outcome::Decline::new(DeclineCode::NotViable, "That is not here.")
        .expect("a reason");
    for alternative in alternatives {
        decline = decline
            .with_alternative(alternative)
            .expect("an alternative");
    }
    let rules = ScriptedRulesOwner::default().with_action(
        action("wave"),
        Preparation::Declined(decline.clone()),
        Resolution::Declined(decline),
    );
    let mut world = world_of(&["wave"], 1, rules);
    ran(&mut world, 1, "wave").readout.as_str().to_owned()
}

#[test]
fn a_decline_names_what_would_be_possible_in_a_plain_list() {
    assert_eq!(declined_with(&[]), "That is not here.");
    assert_eq!(
        declined_with(&["shoot"]),
        "That is not here. You could shoot."
    );
    assert_eq!(
        declined_with(&["run", "shoot"]),
        "That is not here. You could run or shoot."
    );
    assert_eq!(
        declined_with(&["run", "climb", "shoot"]),
        "That is not here. You could run, climb, or shoot."
    );
}

#[test]
fn a_readout_built_from_blank_text_is_never_blank() {
    let readout = crate::pipeline::Readout::new("   ".to_owned());

    assert!(!readout.as_str().trim().is_empty());
}

#[test]
fn equal_tables_have_equal_state_digests_and_a_changed_table_does_not() {
    let mut world = wave_world();
    let before = world.table.state_hash();
    let copy = world.table.clone().state_hash();

    ran(&mut world, 1, "wave");

    assert_eq!(before, copy);
    assert_ne!(before, world.table.state_hash());
}
