//! The behaviours the adversary review of C3 and C4 asked to be proven: a plan's later steps are
//! checked against its earlier ones, the table's own judgements come first, a pending ends for a
//! seat or a scene that left, a machine that breaks its contract is a machine failure, a stale
//! result is refused and forgotten, every audience gets its projection and a control input is
//! staged like any other.

use crate::effects::TableEffect;
use crate::envelope::{CancelCause, EngineControl, EngineInput, PendingId};
use crate::golden::{FIRST_SCENE, Leaves, SceneMachine, TABLE, World};
use crate::identity::{SceneId, SeatId, TableScope};
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pipeline::{PipelineLimits, PipelineTable, SPLIT_ACTION};
use crate::pipeline_fixtures_tests::{
    action, allows, changes, failures_of, lines, phase_of, ran, seat, world_of,
};
use crate::ports::Clock;
use crate::ports::{
    Awaiting, AwaitingKind, ConfirmationReason, Interpretation, Offer, PortName, Preparation,
    Readiness, Resolution, Resolved, ResourceHandle,
};
use crate::projection::{AudienceProjection, Viewer};
use crate::table_state::Presence;
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner, correlation};

/// The operation numbers of changes a test makes to the table outside any input, far from the
/// numbers of the inputs.
const OUTSIDE_CHANGE: u64 = 3_000_000;

/// Changes the table's facts the way the host's other owners would, keeping everything the
/// pipeline holds (the pendings, the rulings, the requests).
fn change_table(world: &mut World, step: u64, effects: &[TableEffect]) {
    let mut facts = world.table.facts().clone();
    facts
        .apply(
            &TableScope::of_table(TABLE),
            correlation(OUTSIDE_CHANGE + step),
            effects,
        )
        .expect("the change applies");
    let now_ms = world.clock.now_ms();
    let record = world.table.to_record(now_ms);
    world.table =
        PipelineTable::from_record(TABLE, facts, record, now_ms, &PipelineLimits::default())
            .expect("the table loads again");
}

fn calls_of(world: &World, wanted: fn(&RulesCall) -> bool) -> Vec<RulesCall> {
    world.rules.calls().into_iter().filter(wanted).collect()
}

#[test]
fn a_later_step_is_checked_against_the_steps_applied_before_it() {
    let decline = Decline::new(
        DeclineCode::NotViable,
        "You have one action and it is spent.",
    )
    .expect("a reason");
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("drink")]),
        "drink",
    )
    .with_once_per_plan(action("drink"), decline.clone());
    let mut world = world_of(&["drink"], 1, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "drink twice",
        Interpretation::Plan(vec![action("drink"), action("drink")]),
    );

    let result = world.text(1, "drink twice").expect("the line runs");

    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(result.unfinished, Some(decline));
    let checks: Vec<(usize, usize)> = world
        .rules
        .plan_checks()
        .into_iter()
        .map(|check| (check.steps_before, check.effects_before))
        .collect();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0], (0, 0), "the first step sees nothing before it");
    assert_eq!(
        checks[1].0, 1,
        "the second step sees the first step applied"
    );
    assert!(
        checks[1].1 >= 1,
        "and the effects the first step staged, not the table as it was"
    );
    let again = ran(&mut world, 1, "drink");
    assert_eq!(
        again.outcome,
        Outcome::Applied,
        "what a plan spent is not carried into the next input"
    );
}

fn paused_world() -> World {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );
    let mut world = world_of(&["wave"], 2, rules);
    change_table(&mut world, 1, &[TableEffect::TablePaused]);
    world
}

#[test]
fn a_paused_table_declines_play_time_locked_until_it_is_resumed() {
    let mut world = paused_world();

    let blocked = world.tap(1, "wave").expect("the tap runs");
    let typed = world.text(1, "wave at them").expect("the line runs");
    let before_resume = world.table.state_hash();

    assert_eq!(
        blocked.record.declined,
        Some(DeclineCode::TimeLocked),
        "the table's own judgement comes first"
    );
    assert_eq!(typed.record.declined, Some(DeclineCode::TimeLocked));
    assert!(changes(&blocked).is_empty());
    assert_eq!(world.table.state_hash(), before_resume);
    assert_eq!(world.rules.calls(), Vec::<RulesCall>::new());
    assert_eq!(world.interpreter.calls().first_attempts, 0);

    let resumed = world
        .engine(EngineInput::Control(EngineControl::ResumeTable))
        .expect("the resume runs");
    let played = ran(&mut world, 1, "wave");

    assert_eq!(resumed.outcome, Outcome::Applied);
    assert_eq!(played.outcome, Outcome::Applied);
}

#[test]
fn a_split_is_judged_by_the_table_not_by_the_rules_owner() {
    let mut together = world_of(&[SPLIT_ACTION], 2, ScriptedRulesOwner::default());
    let mut alone = world_of(&[SPLIT_ACTION], 1, ScriptedRulesOwner::default());

    let kept_together = together.tap(1, SPLIT_ACTION).expect("the tap runs");
    let nobody_to_split_from = alone.tap(1, SPLIT_ACTION).expect("the tap runs");

    assert_eq!(
        kept_together.record.declined,
        Some(DeclineCode::NotSupportedYet),
        "while the table keeps the party together every split is declined (RES25)"
    );
    assert_eq!(
        nobody_to_split_from.record.declined,
        Some(DeclineCode::NotViable),
        "a seat with no company has nothing to split from"
    );
    assert_eq!(together.rules.calls(), Vec::<RulesCall>::new());
    assert_eq!(alone.rules.calls(), Vec::<RulesCall>::new());
}

fn shield_world() -> World {
    let rules = ScriptedRulesOwner::default()
        .with_action(
            action("strike"),
            Preparation::Ready {
                draws: vec![crate::pipeline_fixtures_tests::d20()],
            },
            Resolution::applied(),
        )
        .with_resolutions(
            action("strike"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Reaction,
                offers: vec![Offer {
                    seat: seat(2),
                    choices: vec![action("shield"), action("pass")],
                }],
                draws: Vec::new(),
            })],
        )
        .with_resumptions(action("strike"), vec![Resolution::applied()]);
    let mut world = world_of(&["strike"], 3, rules);
    world.dice = crate::golden::CountingDice::new([14, 3]);
    world
}

fn cancelled_lines(world: &World) -> Vec<String> {
    lines(world)
        .into_iter()
        .filter(|line| line.starts_with("PENDING cancelled"))
        .collect()
}

#[test]
fn a_window_whose_seat_left_for_good_ends_by_the_exit_and_spends_nothing() {
    let mut world = shield_world();
    let opened = ran(&mut world, 1, "strike");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(phase_of(&world), "waiting");
    change_table(&mut world, 1, &[TableEffect::SeatLeft { seat: seat(1) }]);
    let drawn = world.dice.drawn();

    let ended = world
        .engine(EngineInput::Control(EngineControl::CancelPending {
            pending: PendingId::new(1),
            cause: CancelCause::SeatLeft,
        }))
        .expect("the exit runs");

    assert_eq!(ended.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert_eq!(phase_of(&world), "open", "the machine that waited is told");
    assert_eq!(world.dice.drawn(), drawn, "nothing is rolled by an exit");
    assert_eq!(
        calls_of(&world, |call| matches!(call, RulesCall::Abandon(_))),
        [RulesCall::Abandon(action("strike"))],
        "the rules owner releases what it reserved"
    );
    assert_eq!(
        calls_of(&world, |call| matches!(call, RulesCall::Resume { .. })),
        Vec::<RulesCall>::new()
    );
    assert_eq!(
        cancelled_lines(&world),
        ["PENDING cancelled scene=1 machine=scene pending=1 kind=reaction cause=seat_left"]
    );
}

#[test]
fn an_answer_after_the_seat_of_the_action_left_cancels_the_window_and_is_declined() {
    let mut world = shield_world();
    ran(&mut world, 1, "strike");
    change_table(&mut world, 1, &[TableEffect::SeatLeft { seat: seat(1) }]);

    let answer = world.answer(2, 1, "pass").expect("the answer runs");

    assert_eq!(answer.record.declined, Some(DeclineCode::WindowClosed));
    assert!(world.table.pending().is_empty());
    assert_eq!(phase_of(&world), "open");
    assert_eq!(
        cancelled_lines(&world),
        ["PENDING cancelled scene=1 machine=scene pending=1 kind=reaction cause=seat_left"]
    );
}

#[test]
fn an_answer_after_the_party_moved_to_another_scene_cancels_the_window() {
    let mut world = shield_world();
    ran(&mut world, 1, "strike");
    change_table(
        &mut world,
        1,
        &[TableEffect::SceneEntered {
            scene: SceneId::new(2),
        }],
    );

    let answer = world.answer(2, 1, "pass").expect("the answer runs");

    assert_eq!(answer.record.declined, Some(DeclineCode::WindowClosed));
    assert!(world.table.pending().is_empty());
    assert_eq!(
        cancelled_lines(&world),
        ["PENDING cancelled scene=2 pending=1 kind=reaction cause=scene_changed"]
    );
    assert_ne!(FIRST_SCENE, SceneId::new(2));
}

#[test]
fn exiting_a_pending_that_is_not_there_is_declined_and_changes_nothing() {
    let mut world = shield_world();
    let before = world.table.state_hash();

    let ended = world
        .engine(EngineInput::Control(EngineControl::CancelPending {
            pending: PendingId::new(9),
            cause: CancelCause::HostDecision,
        }))
        .expect("the exit runs");

    assert_eq!(ended.record.declined, Some(DeclineCode::WindowClosed));
    assert_eq!(world.table.state_hash(), before);
    assert_eq!(cancelled_lines(&world), Vec::<String>::new());
}

#[test]
fn exiting_a_confirmation_closes_it_without_asking_the_rules_owner_to_release_anything() {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("stab")]),
        "stab",
    )
    .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let mut world = world_of(&["stab"], 2, rules);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab him", Interpretation::Action(action("stab")));
    world.text(1, "stab him").expect("the line runs");
    assert_eq!(world.table.pending().len(), 1);

    let ended = world
        .engine(EngineInput::Control(EngineControl::CancelPending {
            pending: PendingId::new(1),
            cause: CancelCause::HostDecision,
        }))
        .expect("the exit runs");

    assert_eq!(ended.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert_eq!(
        calls_of(&world, |call| matches!(call, RulesCall::Abandon(_))),
        Vec::<RulesCall>::new(),
        "a confirmation reserved nothing"
    );
}

#[test]
fn a_machine_that_settles_as_pending_is_a_machine_failure_recorded_once() {
    let scene = SceneMachine::new(&[("odd", Leaves::SettlesPending)]).expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 1).expect("the world builds");
    world.rules = allows(ScriptedRulesOwner::default(), "odd");
    let before = world.table.state_hash();

    let result = ran(&mut world, 1, "odd");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(world.table.state_hash(), before, "nothing was committed");
    assert!(world.table.pending().is_empty());
    let failures: Vec<String> = lines(&world)
        .into_iter()
        .filter(|line| line.starts_with("MACHINE failed"))
        .collect();
    assert_eq!(
        failures,
        ["MACHINE failed scene=1 machine=scene cause=state_invalid"]
    );
}

fn requesting_world() -> World {
    let scene = SceneMachine::new(&[("ask_map", Leaves::Nothing)])
        .and_then(|scene| {
            scene
                .requesting_on("ask_map", "map", 7)
                .map_err(crate::machine::SpecError::InvalidName)
        })
        .expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 1).expect("the world builds");
    world.rules = allows(ScriptedRulesOwner::default(), "ask_map");
    world
}

#[test]
fn a_result_made_before_a_pause_is_refused_by_the_table_and_its_request_is_forgotten() {
    let mut world = requesting_world();
    ran(&mut world, 1, "ask_map");
    assert_eq!(world.table.requests().len(), 1);
    change_table(&mut world, 1, &[TableEffect::TablePaused]);
    let machine_before = phase_of(&world);

    let result = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(1),
            readiness: Readiness::Ready,
        })
        .expect("the result runs");

    assert!(matches!(result.outcome, Outcome::Declined(_)));
    assert!(
        world.table.requests().is_empty(),
        "a refused result is not kept waiting"
    );
    assert_eq!(phase_of(&world), machine_before);
    assert!(
        lines(&world)
            .iter()
            .any(|line| line.contains("rejected") && line.contains("table_paused")),
        "the rejection is recorded once with its cause: {:?}",
        lines(&world)
    );
    let second = world
        .engine(EngineInput::ResourceResult {
            handle: ResourceHandle::new(1),
            readiness: Readiness::Ready,
        })
        .expect("the result runs");
    assert!(matches!(second.outcome, Outcome::Declined(_)));
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::Broker, crate::event::FailureCause::Stale)],
        "the second delivery finds no request"
    );
}

#[test]
fn every_audience_gets_a_projection_of_the_table_as_the_turn_left_it() {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );
    let mut world = world_of(&["wave"], 2, rules);

    let result = ran(&mut world, 1, "wave");

    let viewers: Vec<Viewer> = result
        .projections
        .iter()
        .map(AudienceProjection::viewer)
        .collect();
    assert_eq!(
        viewers,
        [
            Viewer::Shared,
            Viewer::Seat { seat: seat(1) },
            Viewer::Seat { seat: seat(2) },
        ]
    );
    let readouts: Vec<String> = result
        .projections
        .iter()
        .map(|projection| match projection {
            AudienceProjection::Shared(shared) => shared.readout().as_str().to_owned(),
            AudienceProjection::Seat(seat) => seat.readout().as_str().to_owned(),
        })
        .collect();
    assert!(
        readouts[0].contains("wave"),
        "the shared screen shows what was done: {readouts:?}"
    );
    let built_at = world.table.facts().snapshot().revision();
    assert!(
        result
            .projections
            .iter()
            .all(|projection| projection.record().revision == built_at),
        "every projection is built from the table as the turn left it"
    );
}

#[test]
fn a_control_input_is_staged_like_any_other_and_released_with_its_turn() {
    let mut world = shield_world();
    ran(&mut world, 1, "strike");
    let seen = world.events().len();
    let probe = correlation(world.next_turn());
    let envelope = crate::envelope::InputEnvelope::from_engine(
        probe.operation_id,
        crate::envelope::ViewStamp::new(probe.revision),
        EngineInput::Control(EngineControl::SeatPresence {
            seat: SeatId::new(2),
            presence: Presence::Disconnected,
        }),
    );

    let result = world
        .run_unreleased(&envelope, probe)
        .expect("the control runs");

    let before_release = crate::pipeline_fixtures_tests::lines_after(&world, seen);
    assert!(
        !before_release
            .iter()
            .any(|line| line.starts_with("PENDING") || line.starts_with("TURN")),
        "no change of the table is recorded before the host's commit: {before_release:?}"
    );
    assert!(
        result
            .committed_events
            .iter()
            .any(|event| event.render_audit_line().contains("held")),
        "the hold is among the events the commit releases"
    );
    assert!(
        world
            .table
            .pending()
            .iter()
            .any(|pending| pending.is_held()),
        "the table in memory already holds the turn, so a later input in the batch sees it"
    );
    let undo_before = world.table.state_hash();
    result.release_events(&world.sink);
    let released = crate::pipeline_fixtures_tests::lines_after(&world, seen);
    assert!(
        released.iter().any(|line| line.starts_with("PENDING held")),
        "{released:?}"
    );
    assert_eq!(world.table.state_hash(), undo_before);
}

fn beat(scene: u64, name: &str) -> TableEffect {
    TableEffect::BeatResolved {
        scene: SceneId::new(scene),
        beat: crate::effects::BeatName::new(name).expect("a beat name"),
    }
}

fn striking_world(resolved: Resolved) -> World {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("strike")])
        .with_action(
            action("strike"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::Applied(resolved),
        );
    world_of(&["strike"], 1, rules)
}

#[test]
fn what_the_rules_owner_resolved_is_committed_with_its_effects_and_named_by_its_typed_facts() {
    let resolved = Resolved::with_effects(vec![beat(1, "the_blow_lands")])
        .with_fact("damage", 7)
        .expect("a label");
    let mut world = striking_world(resolved);

    let result = ran(&mut world, 1, "strike");

    assert_eq!(result.outcome, Outcome::Applied);
    let committed: Vec<&TableEffect> = changes(&result);
    assert_eq!(
        committed.first(),
        Some(&&beat(1, "the_blow_lands")),
        "the rules owner's effects come first, then the machine's own"
    );
    assert!(committed.contains(&&beat(1, "strike")));
    assert_eq!(result.readout.as_str(), "Seat 1: strike done (damage 7).");
    let job = result.narration_job.as_ref().expect("a narration job");
    assert_eq!(job.committed.facts.len(), 1);
    assert_eq!(job.committed.facts[0].amount, 7);
}

#[test]
fn an_effect_the_table_refuses_declines_the_input_and_commits_nothing() {
    let resolved = Resolved::with_effects(vec![beat(1, "first"), beat(9, "in_a_scene_nobody_has")]);
    let mut world = striking_world(resolved);
    let before = world.table.state_hash();

    let result = ran(&mut world, 1, "strike");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(
        world.table.state_hash(),
        before,
        "all or nothing: the first effect was not kept"
    );
    assert!(changes(&result).is_empty());
    let refusals = lines(&world)
        .into_iter()
        .filter(|line| line.starts_with("EFFECT refused"))
        .count();
    assert_eq!(refusals, 1, "the refusal is recorded once");
    assert!(result.narration_job.is_none());
}

#[test]
fn an_automatic_ruling_is_not_stored_for_a_step_the_rules_owner_then_declines() {
    let decline =
        Decline::new(DeclineCode::NotViable, "There is nothing to swing from.").expect("a reason");
    let mut request = crate::pipeline_fixtures_tests::ruling_request(
        "swing_from_set_piece",
        "no_advantage",
        crate::ports::RulingTier::Ordinary,
    );
    request.reviewed_automatic = true;
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("swing")])
        .with_action(
            action("swing"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::Declined(decline),
        )
        .with_ruling(action("swing"), request);
    let mut world = world_of(&["swing"], 1, rules);

    let result = ran(&mut world, 1, "swing");

    assert_eq!(result.record.declined, Some(DeclineCode::NotViable));
    assert!(
        world.table.rulings().is_empty(),
        "a ruling made for a step that did not commit is not kept"
    );
    assert!(
        !lines(&world)
            .iter()
            .any(|line| line.starts_with("RULING stored")),
        "{:?}",
        lines(&world)
    );
}
