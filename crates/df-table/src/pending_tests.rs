//! Pending resolutions through the pipeline: the register rows M2-D4 to M2-D7 as fixtures, answers
//! by pending id, windows that hold, held turns, rulings with keyed reuse, confirmations and a
//! restart that resumes without drawing again.

use crate::envelope::{EngineControl, EngineInput, TimerName};
use crate::event::{FailureCause, PipelineStep};
use crate::golden::{CountingDice, Leaves, World};
use crate::identity::SceneId;
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pending::{RulingBook, RulingSource, ScopedRuling};
use crate::pipeline::{PENDING_NUDGE_TIMER, PipelineLimits, TurnResult};
use crate::pipeline_fixtures_tests::{
    action, allows, changes, d8, d20, failures_of, lines, lines_after, phase_of, ran, seat,
    steps_of, window_of, world_of,
};
use crate::ports::Clock;
use crate::ports::{
    Awaiting, AwaitingKind, ConfirmationReason, Interpretation, Offer, PortName, Preparation,
    Resolution, RulingAnswer, RulingKey, RulingOption, RulingScope, RulingTier,
};
use crate::table_state::Presence;
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner, correlation};

fn answer(world: &mut World, seat_number: u64, pending: u64, choice: &str) -> TurnResult {
    world
        .answer(seat_number, pending, choice)
        .expect("the answer runs")
}

fn nudge(world: &mut World) -> TurnResult {
    let timer = TimerName::new(PENDING_NUDGE_TIMER).expect("a valid timer name");
    world
        .engine(EngineInput::Timer { timer })
        .expect("the timer runs")
}

fn choice_window(seat_number: u64, choices: &[&str]) -> Awaiting {
    Awaiting {
        kind: AwaitingKind::Choice,
        ..window_of(seat_number, choices)
    }
}

/// The effect the rules owner reports for something the machine did not do itself.
fn beat(name: &str) -> crate::effects::TableEffect {
    crate::effects::TableEffect::BeatResolved {
        scene: SceneId::new(1),
        beat: crate::effects::BeatName::new(name).expect("a beat name"),
    }
}

/// What the rules owner resolved: its own effects and typed facts (RES18).
fn resolved(effects: Vec<crate::effects::TableEffect>, facts: &[(&str, u64)]) -> Resolution {
    let mut resolved = crate::ports::Resolved::with_effects(effects);
    for (label, amount) in facts {
        resolved = resolved.with_fact(label, *amount).expect("a label");
    }
    Resolution::Applied(resolved)
}

/// The typed facts the narration job carries, as label and amount.
fn narrated_facts(result: &TurnResult) -> Vec<(String, u64)> {
    result
        .narration_job
        .as_ref()
        .map(|job| {
            job.committed
                .facts
                .iter()
                .map(|fact| (fact.label.as_str().to_owned(), fact.amount))
                .collect()
        })
        .unwrap_or_default()
}

/// M2-D4 rules owner: a strike rolls a d20, then the roller may use Heroic Inspiration.
fn strike_rules(resumptions: Vec<Resolution>) -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_resolutions(
            action("strike"),
            vec![Resolution::Await(choice_window(
                1,
                &["use_inspiration", "keep"],
            ))],
        )
        .with_resumptions(action("strike"), resumptions)
}

#[test]
fn m2_d4_a_miss_by_one_plus_heroic_inspiration_replaces_the_die_and_commits_one_change() {
    // The reroll turns the miss by one into a hit of 18: the rules owner spends the Inspiration
    // and reports the total of the new die, and the first die's 19 is not in what commits.
    let rules = strike_rules(vec![
        Resolution::NeedsDraws(vec![d20()]),
        resolved(vec![beat("inspiration_spent")], &[("attack_total", 18)]),
    ]);
    let mut world = world_of(&["strike"], 2, rules);
    world.dice = CountingDice::new([19, 17]);

    let opened = ran(&mut world, 1, "strike");
    let first_die_kept: Vec<u16> = world
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .expect("the roller's choice is pending")
        .draws()
        .iter()
        .map(|draw| draw.value())
        .collect();
    let waiting_phase = phase_of(&world);
    let resolved = answer(&mut world, 1, 1, "use_inspiration");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Choice));
    assert!(
        changes(&opened).is_empty(),
        "nothing is committed while the roller chooses"
    );
    assert_eq!(
        (first_die_kept, waiting_phase.as_str()),
        (vec![19], "waiting")
    );
    assert_eq!(resolved.outcome, Outcome::Applied);
    assert_eq!(
        resolved
            .record
            .draws
            .iter()
            .map(|draw| draw.value())
            .collect::<Vec<_>>(),
        [17],
        "the reroll draws one new die"
    );
    assert_eq!(world.dice.drawn(), 2);
    assert_eq!(
        changes(&resolved),
        [&beat("inspiration_spent"), &beat("strike")],
        "the Inspiration the rules owner spent commits with the strike's own beat"
    );
    assert_eq!(
        resolved.readout.as_str(),
        "Seat 1: strike done (attack total 18)."
    );
    assert_eq!(
        narrated_facts(&resolved),
        [("attack_total".to_owned(), 18)],
        "the result of the second die is what is told, and the first die's 19 is not"
    );
    assert!(world.table.pending().is_empty());
    assert_eq!(phase_of(&world), "open");
    assert_eq!(
        world.rules.calls(),
        [
            RulesCall::Prepare(action("strike")),
            RulesCall::Resolve {
                action: action("strike"),
                draws: 1
            },
            RulesCall::Resume {
                action: action("strike"),
                draws: 1,
                answers: vec![(seat(1), action("use_inspiration"))],
                ruling: None
            },
            RulesCall::Resume {
                action: action("strike"),
                draws: 2,
                answers: vec![(seat(1), action("use_inspiration"))],
                ruling: None
            },
        ],
        "the old die stays in the list the rules owner is given, and the new one follows it"
    );
}

#[test]
fn m2_d4_after_the_reroll_nothing_more_is_offered_and_the_closed_choice_cannot_be_answered() {
    let rules = strike_rules(vec![
        Resolution::NeedsDraws(vec![d20()]),
        Resolution::applied(),
    ]);
    let mut world = world_of(&["strike"], 2, rules);
    world.dice = CountingDice::new([19, 2]);
    ran(&mut world, 1, "strike");
    answer(&mut world, 1, 1, "use_inspiration");

    let second = answer(&mut world, 1, 1, "use_inspiration");

    assert_eq!(second.record.declined, Some(DeclineCode::WindowClosed));
    assert_eq!(
        world.dice.drawn(),
        2,
        "a second miss is not rolled again and offers nothing"
    );
}

#[test]
fn m2_d4_keeping_the_roll_draws_nothing_new_and_commits_one_change() {
    let mut world = world_of(
        &["strike"],
        2,
        strike_rules(vec![resolved(Vec::new(), &[("attack_total", 19)])]),
    );
    world.dice = CountingDice::new([19]);
    ran(&mut world, 1, "strike");

    let kept = answer(&mut world, 1, 1, "keep");

    assert_eq!(kept.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 1);
    assert_eq!(
        changes(&kept),
        [&beat("strike")],
        "no Inspiration is spent, so only the strike's own beat commits"
    );
    assert_eq!(narrated_facts(&kept), [("attack_total".to_owned(), 19)]);
}

/// M2-D5 rules owner: a fireball draws nothing until its window closes.
fn fireball_rules(resumptions: Vec<Resolution>, offers: Vec<Offer>) -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_action(
            action("fireball"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action("fireball"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Reaction,
                offers,
                draws: Vec::new(),
            })],
        )
        .with_resumptions(action("fireball"), resumptions)
}

fn counterspell_offer() -> Vec<Offer> {
    vec![Offer {
        seat: seat(2),
        choices: vec![action("counterspell"), action("pass")],
    }]
}

#[test]
fn m2_d5_a_countered_fireball_spends_the_action_and_consumes_no_damage_draw() {
    // A countered spell resolves to a fact and no damage effect: the action is spent and nothing
    // is hurt.
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(
            vec![resolved(Vec::new(), &[("spell_countered", 1)])],
            counterspell_offer(),
        ),
    );
    world.dice = CountingDice::new([6]);

    let opened = ran(&mut world, 1, "fireball");
    let countered = answer(&mut world, 2, 1, "counterspell");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(
        opened.readout.as_str(),
        "Waiting for seat 2 to answer a reaction."
    );
    assert_eq!(
        opened.record.draws.len(),
        0,
        "phase one draws nothing for effects not yet reached"
    );
    assert_eq!(countered.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 0, "no damage draw was consumed");
    assert_eq!(
        changes(&countered),
        [&beat("fireball")],
        "the action is spent and no damage effect commits"
    );
    assert_eq!(
        countered.readout.as_str(),
        "Seat 1: fireball done (spell countered 1)."
    );
    assert_eq!(
        narrated_facts(&countered),
        [("spell_countered".to_owned(), 1)]
    );
    assert_eq!(
        world.rules.calls().last(),
        Some(&RulesCall::Resume {
            action: action("fireball"),
            draws: 0,
            answers: vec![(seat(2), action("counterspell"))],
            ruling: None
        })
    );
}

#[test]
fn m2_d5_when_the_window_passes_the_damage_is_drawn_and_spent_after_it() {
    let rules = fireball_rules(
        vec![
            Resolution::NeedsDraws(vec![d8()]),
            resolved(vec![beat("fireball_damage")], &[("damage", 7)]),
        ],
        counterspell_offer(),
    );
    let mut world = world_of(&["fireball"], 3, rules);
    world.dice = CountingDice::new([6]);
    ran(&mut world, 1, "fireball");

    let passed = answer(&mut world, 2, 1, "pass");

    assert_eq!(passed.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 1);
    assert_eq!(passed.record.draws.len(), 1);
    assert_eq!(
        changes(&passed),
        [&beat("fireball_damage"), &beat("fireball")],
        "the damage the rules owner resolved commits with the spell's own beat, and a countered \
         spell does not"
    );
    assert_eq!(passed.readout.as_str(), "Seat 1: fireball done (damage 7).");
    assert_eq!(narrated_facts(&passed), [("damage".to_owned(), 7)]);
    assert!(
        steps_of(&world.events())
            .iter()
            .filter(|step| **step == PipelineStep::Decide)
            .count()
            >= 1
    );
}

#[test]
fn two_seats_answering_one_window_resume_once_with_the_answers_in_the_order_of_the_offers() {
    let offers = vec![
        Offer {
            seat: seat(2),
            choices: vec![action("shield"), action("pass")],
        },
        Offer {
            seat: seat(3),
            choices: vec![action("pass")],
        },
    ];
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(vec![Resolution::applied()], offers),
    );
    ran(&mut world, 1, "fireball");

    let first = answer(&mut world, 3, 1, "pass");
    let resumed_early = world
        .rules
        .calls()
        .iter()
        .any(|call| matches!(call, RulesCall::Resume { .. }));
    let second = answer(&mut world, 2, 1, "shield");

    assert_eq!(first.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(first.pending, Some(crate::envelope::PendingId::new(1)));
    assert!(
        !resumed_early,
        "the window is not resumed while a seat has not answered"
    );
    assert_eq!(second.outcome, Outcome::Applied);
    let resumes: Vec<RulesCall> = world
        .rules
        .calls()
        .into_iter()
        .filter(|call| matches!(call, RulesCall::Resume { .. }))
        .collect();
    assert_eq!(
        resumes,
        [RulesCall::Resume {
            action: action("fireball"),
            draws: 0,
            answers: vec![(seat(2), action("shield")), (seat(3), action("pass"))],
            ruling: None
        }]
    );
}

#[test]
fn a_seat_answering_twice_a_seat_with_no_offer_and_an_unoffered_choice_are_declined_and_change_nothing()
 {
    let offers = vec![
        Offer {
            seat: seat(2),
            choices: vec![action("shield"), action("pass")],
        },
        Offer {
            seat: seat(3),
            choices: vec![action("pass")],
        },
    ];
    let mut world = world_of(
        &["fireball"],
        4,
        fireball_rules(vec![Resolution::applied()], offers),
    );
    ran(&mut world, 1, "fireball");
    answer(&mut world, 2, 1, "shield");
    let before = world.table.durable_hash();

    let twice = answer(&mut world, 2, 1, "pass");
    let stranger = answer(&mut world, 4, 1, "pass");
    let off_the_offer = answer(&mut world, 3, 1, "shield");

    assert_eq!(twice.record.declined, Some(DeclineCode::IntentInvalid));
    assert_eq!(stranger.record.declined, Some(DeclineCode::NotYourTurn));
    assert_eq!(
        off_the_offer.record.declined,
        Some(DeclineCode::IntentInvalid)
    );
    assert_eq!(before, world.table.durable_hash());
}

#[test]
fn an_answer_for_an_unknown_or_an_already_resolved_pending_is_declined_window_closed() {
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(vec![Resolution::applied()], counterspell_offer()),
    );
    ran(&mut world, 1, "fireball");
    answer(&mut world, 2, 1, "pass");

    let resolved = answer(&mut world, 2, 1, "pass");
    let unknown = answer(&mut world, 2, 77, "pass");

    assert_eq!(resolved.record.declined, Some(DeclineCode::WindowClosed));
    assert_eq!(unknown.record.declined, Some(DeclineCode::WindowClosed));
    assert_eq!(resolved.readout.as_str(), "That chance has passed.");
}

#[test]
fn a_window_that_runs_past_its_nudge_time_is_nudged_once_and_still_resolves_when_answered() {
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(vec![Resolution::applied()], counterspell_offer()),
    );
    world.dice = CountingDice::new([6]);
    ran(&mut world, 1, "fireball");
    world.clock.advance_ms(31_000);

    let seen = world.events().len();
    nudge(&mut world);
    nudge(&mut world);
    let nudges: Vec<String> = lines_after(&world, seen)
        .into_iter()
        .filter(|line| line.starts_with("PENDING nudged"))
        .collect();
    world.clock.advance_ms(600_000);
    let late = answer(&mut world, 2, 1, "pass");

    assert_eq!(
        nudges,
        ["PENDING nudged scene=1 machine=scene pending=1 kind=reaction"]
    );
    assert_eq!(
        late.outcome,
        Outcome::Applied,
        "a window never times out into a decline"
    );
    assert_eq!(world.dice.drawn(), 0, "waiting rolled nothing");
}

#[test]
fn a_held_turn_spends_nothing_and_the_reconnect_finds_the_offer_by_pending_id() {
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(vec![Resolution::applied()], counterspell_offer()),
    );
    ran(&mut world, 1, "fireball");

    let dropped = world
        .engine(EngineInput::Control(EngineControl::SeatPresence {
            seat: seat(2),
            presence: Presence::Disconnected,
        }))
        .expect("the drop runs");
    let held: Vec<_> = world
        .table
        .pending()
        .iter()
        .filter(|pending| pending.is_held())
        .map(|pending| pending.id())
        .collect();
    let held_state = Some(!held.is_empty());
    let returned_input = world
        .engine(EngineInput::Control(EngineControl::SeatPresence {
            seat: seat(2),
            presence: Presence::Connected,
        }))
        .expect("the return runs");
    let returned: Vec<_> = world
        .table
        .pending()
        .offers_awaiting(seat(2))
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let resolved = answer(&mut world, 2, 1, "pass");

    assert_eq!((held.len(), held_state), (1, Some(true)));
    assert_eq!(returned, held, "the reconnect finds the same pending by id");
    assert_eq!(dropped.outcome, Outcome::Applied);
    assert_eq!(returned_input.outcome, Outcome::Applied);
    assert!(
        dropped.record.effects.iter().any(|effect| matches!(
            effect,
            crate::effects::TableEffect::SeatPresenceChanged { .. }
        )),
        "a seat dropping is a committed input with a turn record"
    );
    assert_eq!(resolved.outcome, Outcome::Applied);
    let all = lines(&world);
    assert!(all.contains(&"PENDING held scene=1 machine=scene pending=1 seat=2".to_owned()));
    assert!(all.contains(&"PENDING reacquired scene=1 machine=scene pending=1 seat=2".to_owned()));
    assert_eq!(world.dice.drawn(), 0);
}

#[test]
fn a_resume_the_rules_owner_declines_does_not_take_the_answer_and_asks_the_seat_again() {
    let rules = fireball_rules(
        vec![
            Resolution::Declined(Decline::with_default_reason(DeclineCode::ResourceSpent)),
            Resolution::applied(),
        ],
        counterspell_offer(),
    );
    let mut world = world_of(&["fireball"], 3, rules);
    ran(&mut world, 1, "fireball");

    let refused = answer(&mut world, 2, 1, "counterspell");
    let still_asked = world
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .map(|pending| {
            pending
                .window()
                .offer_for(seat(2))
                .map(|offer| offer.answer().is_none())
        });
    let retried = answer(&mut world, 2, 1, "pass");

    assert_eq!(refused.record.declined, Some(DeclineCode::ResourceSpent));
    assert_eq!(still_asked, Some(Some(true)));
    assert_eq!(retried.outcome, Outcome::Applied);
}

#[test]
fn m2_d6_the_order_of_simultaneous_effects_is_asked_of_the_controller_whose_turn_it_is() {
    let rules = ScriptedRulesOwner::default()
        .with_action(
            action("end_turn"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action("end_turn"),
            vec![Resolution::Await(choice_window(
                1,
                &["order_ongoing_first", "order_save_first"],
            ))],
        );
    let mut world = world_of(&["end_turn"], 2, rules);
    let opened = ran(&mut world, 1, "end_turn");

    let by_other = answer(&mut world, 2, 1, "order_save_first");
    let by_controller = answer(&mut world, 1, 1, "order_save_first");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Choice));
    assert_eq!(by_other.record.declined, Some(DeclineCode::NotYourTurn));
    assert_eq!(by_controller.outcome, Outcome::Applied);
    assert!(world.rules.calls().contains(&RulesCall::Resume {
        action: action("end_turn"),
        draws: 0,
        answers: vec![(seat(1), action("order_save_first"))],
        ruling: None
    }));
}

fn ready_request(reviewed_automatic: bool) -> crate::ports::RulingRequest {
    let option = |name: &str| RulingOption {
        answer: RulingAnswer::new(name).expect("valid"),
        tier: RulingTier::Ordinary,
    };
    crate::ports::RulingRequest {
        key: RulingKey::new("ready_trigger_unbound").expect("valid"),
        scope: RulingScope::Scene,
        suggested: option("fired"),
        alternatives: vec![option("not_fired")],
        reviewed_automatic,
    }
}

fn ready_world(request: crate::ports::RulingRequest) -> World {
    let rules = allows(ScriptedRulesOwner::default(), "ready_spell")
        .with_ruling(action("ready_spell"), request);
    let mut world =
        World::with_scene(&[("ready_spell", Leaves::Nothing)], 2, None).expect("builds");
    world.rules = rules;
    world.table = world.table.clone().hosted_by(seat(1));
    world
}

#[test]
fn m2_d7_a_ready_the_adapter_cannot_run_yet_is_declined_not_supported_yet() {
    let rules = ScriptedRulesOwner::default().with_unsupported_action(action("ready_spell"));
    let mut world = world_of(&["ready_spell"], 2, rules);

    let result = ran(&mut world, 2, "ready_spell");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(result.readout.as_str(), "I can't run that yet.");
}

#[test]
fn m2_d7_an_unbound_trigger_is_a_pending_ruling_and_a_concentration_that_breaks_fires_nothing() {
    let mut world = ready_world(ready_request(false));

    let opened = ran(&mut world, 2, "ready_spell");
    let status = world
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .map(|pending| pending.status().key());
    let untouched_phase = phase_of(&world);
    let fired_nothing = answer(&mut world, 1, 1, "not_fired");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Ruling));
    assert_eq!(opened.readout.as_str(), "Needs a ruling.");
    assert_eq!(
        (status, untouched_phase.as_str()),
        (Some("awaiting_adjudicator"), "waiting")
    );
    assert_eq!(fired_nothing.outcome, Outcome::Applied);
    assert!(
        changes(&fired_nothing).is_empty(),
        "nothing fires and nothing is committed"
    );
    assert!(world.rules.calls().iter().any(|call| matches!(
        call,
        RulesCall::Resume { ruling: Some(ruling), .. } if ruling.as_str() == "not_fired"
    )));
    assert_eq!(world.table.rulings().len(), 1);
    assert!(
        lines(&world).contains(
            &"RULING stored scene=1 machine=scene key=ready_trigger_unbound tier=ordinary source=host"
                .to_owned()
        )
    );
}

#[test]
fn a_stored_ruling_with_an_equal_key_and_scope_is_reused_without_asking_and_is_shown_as_earlier() {
    let mut world = ready_world(ready_request(false));
    ran(&mut world, 2, "ready_spell");
    answer(&mut world, 1, 1, "not_fired");

    let seen = world.events().len();
    let reused = ran(&mut world, 2, "ready_spell");

    assert_eq!(reused.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert!(
        reused
            .readout
            .as_str()
            .starts_with("Ruling (earlier): not fired. ")
    );
    assert!(lines_after(&world, seen).contains(
        &"RULING reused scene=1 machine=scene key=ready_trigger_unbound tier=ordinary".to_owned()
    ));
}

#[test]
fn a_different_question_does_not_reuse_a_stored_ruling() {
    let mut world = ready_world(ready_request(false));
    ran(&mut world, 2, "ready_spell");
    answer(&mut world, 1, 1, "not_fired");
    let mut other = ready_request(false);
    other.key = RulingKey::new("ready_trigger_bound").expect("valid");
    world.rules = allows(ScriptedRulesOwner::default(), "ready_spell")
        .with_ruling(action("ready_spell"), other);

    let asked = ran(&mut world, 2, "ready_spell");

    assert_eq!(asked.outcome, Outcome::Pending(PendingKind::Ruling));
}

#[test]
fn a_reviewed_automatic_ruling_settles_at_once_and_is_stored_for_reuse() {
    let mut world = ready_world(ready_request(true));

    let settled = ran(&mut world, 2, "ready_spell");

    assert_eq!(settled.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert_eq!(world.table.rulings().len(), 1);
    assert!(lines(&world).contains(
        &"RULING stored scene=1 machine=scene key=ready_trigger_unbound tier=ordinary source=reviewed_automatic"
            .to_owned()
    ));
}

#[test]
fn the_host_can_refuse_a_ruling_and_the_requester_can_cancel_it() {
    let mut world = ready_world(ready_request(false));
    ran(&mut world, 2, "ready_spell");
    let refused = answer(&mut world, 1, 1, "decline_ruling");
    ran(&mut world, 2, "ready_spell");
    let cancelled = answer(&mut world, 2, 2, "cancel_ruling");

    assert_eq!(refused.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(cancelled.outcome, Outcome::Applied);
    assert_eq!(cancelled.readout.as_str(), "Cancelled.");
    assert!(
        world.table.rulings().is_empty(),
        "neither answer stores a ruling"
    );
    assert!(world.table.pending().is_empty());
}

#[test]
fn the_requester_cannot_adjudicate_and_a_third_seat_has_no_say() {
    let mut world =
        World::with_scene(&[("ready_spell", Leaves::Nothing)], 3, None).expect("builds");
    world.rules = allows(ScriptedRulesOwner::default(), "ready_spell")
        .with_ruling(action("ready_spell"), ready_request(false));
    world.table = world.table.clone().hosted_by(seat(1));
    ran(&mut world, 2, "ready_spell");

    let by_requester = answer(&mut world, 2, 1, "accept_suggestion");
    let by_third = answer(&mut world, 3, 1, "accept_suggestion");

    assert_eq!(
        by_requester.record.declined,
        Some(DeclineCode::IntentInvalid)
    );
    assert_eq!(by_third.record.declined, Some(DeclineCode::NotYourTurn));
    assert_eq!(
        world.table.pending().len(),
        1,
        "the ruling is still waiting"
    );
}

#[test]
fn a_ruling_nobody_has_answered_is_nudged_after_its_time_and_never_ends_on_its_own() {
    let mut world = ready_world(ready_request(false));
    ran(&mut world, 2, "ready_spell");

    world.clock.advance_ms(19_000);
    let early = lines(&world).len();
    nudge(&mut world);
    let before_due = lines(&world)
        .iter()
        .filter(|line| line.starts_with("PENDING nudged"))
        .count();
    world.clock.advance_ms(1_000);
    nudge(&mut world);
    world.clock.advance_ms(3_600_000);
    nudge(&mut world);

    let nudged = lines(&world)
        .iter()
        .filter(|line| line.starts_with("PENDING nudged"))
        .count();
    assert!(early > 0);
    assert_eq!((before_due, nudged), (0, 1));
    assert_eq!(
        world.table.pending().len(),
        1,
        "no timeout spends a resource, rolls a die or invents a ruling"
    );
}

#[test]
fn a_reuse_that_grants_advantage_in_a_new_scene_asks_the_host_once_and_then_reuses_it() {
    let mut request = ready_request(false);
    request.scope = RulingScope::Table;
    request.suggested.tier = RulingTier::Advantage;
    request.suggested.answer = RulingAnswer::new("advantage_on_swing").expect("valid");
    let mut world = ready_world(request.clone());
    let mut book = RulingBook::default();
    book.store(ScopedRuling::new(
        request.key.clone(),
        RulingScope::Table,
        Some(SceneId::new(99)),
        RulingAnswer::new("advantage_on_swing").expect("valid"),
        RulingTier::Advantage,
        RulingSource::Host,
        action("ready_spell"),
    ));
    // The stored ruling was confirmed in scene 99 and the table is in scene 1, so reusing it
    // asks the host once.
    world.table = world.table.clone().with_rulings(book);
    let first = ran(&mut world, 2, "ready_spell");
    let accepted = answer(&mut world, 1, 1, "accept_suggestion");
    let second = ran(&mut world, 2, "ready_spell");

    assert_eq!(first.outcome, Outcome::Pending(PendingKind::Ruling));
    assert_eq!(accepted.outcome, Outcome::Applied);
    assert_eq!(second.outcome, Outcome::Applied);
    assert!(world.table.pending().is_empty());
    assert!(
        lines(&world)
            .iter()
            .any(|line| line.starts_with("RULING reused")),
        "the third use reuses it without asking"
    );
}

#[test]
fn a_ruling_the_host_revoked_is_asked_again() {
    let mut world = ready_world(ready_request(false));
    ran(&mut world, 2, "ready_spell");
    answer(&mut world, 1, 1, "not_fired");
    let key = RulingKey::new("ready_trigger_unbound").expect("valid");

    let revoked = world
        .engine(EngineInput::Control(EngineControl::RevokeRuling {
            key: key.clone(),
            scope: RulingScope::Scene,
        }))
        .expect("the revoke runs");
    let revoked_again = world
        .engine(EngineInput::Control(EngineControl::RevokeRuling {
            key,
            scope: RulingScope::Scene,
        }))
        .expect("the second revoke runs");
    let again = ran(&mut world, 2, "ready_spell");

    assert_eq!(revoked.outcome, Outcome::Applied);
    assert_eq!(
        revoked_again.record.declined,
        Some(DeclineCode::NotViable),
        "a ruling that is gone cannot be revoked twice"
    );
    assert_eq!(again.outcome, Outcome::Pending(PendingKind::Ruling));
    assert!(
        lines(&world)
            .contains(&"RULING revoked scene=1 machine=scene key=ready_trigger_unbound".to_owned())
    );
}

fn lockpick_rules(resumptions: Vec<Resolution>) -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_action(
            action("pick_lock"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action("pick_lock"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Roll,
                offers: Vec::new(),
                draws: vec![d20()],
            })],
        )
        .with_resumptions(action("pick_lock"), resumptions)
}

#[test]
fn a_roll_the_player_must_make_is_pending_and_draws_only_when_answered() {
    let mut world = world_of(
        &["pick_lock"],
        1,
        lockpick_rules(vec![Resolution::applied()]),
    );
    world.dice = CountingDice::new([15]);

    let opened = ran(&mut world, 1, "pick_lock");
    let waiting_draws = world.dice.drawn();
    let rolled = answer(&mut world, 1, 1, "roll");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Roll));
    assert_eq!(opened.readout.as_str(), "Waiting for seat 1 to roll.");
    assert_eq!(waiting_draws, 0);
    assert_eq!(rolled.outcome, Outcome::Applied);
    assert_eq!(rolled.record.draws.len(), 1);
    assert_eq!(phase_of(&world), "open");
}

#[test]
fn a_roll_that_was_made_is_kept_when_the_resume_fails_and_the_next_answer_never_draws_again() {
    let rules = lockpick_rules(vec![
        Resolution::Declined(Decline::with_default_reason(DeclineCode::ResourceSpent)),
        Resolution::applied(),
    ]);
    let mut world = world_of(&["pick_lock"], 1, rules);
    world.dice = CountingDice::new([15, 3]);
    ran(&mut world, 1, "pick_lock");

    let failed = answer(&mut world, 1, 1, "roll");
    let kept = world
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .map(|pending| pending.draws().len());
    let finished = answer(&mut world, 1, 1, "roll");

    assert_eq!(failed.record.declined, Some(DeclineCode::ResourceSpent));
    assert_eq!(kept, Some(1));
    assert_eq!(finished.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 1, "the one roll was made once");
}

#[test]
fn a_window_whose_resume_waits_again_nests_and_the_second_window_resolves_it() {
    let rules = fireball_rules(
        vec![
            Resolution::Await(Awaiting {
                kind: AwaitingKind::Reaction,
                offers: vec![Offer {
                    seat: seat(3),
                    choices: vec![action("absorb"), action("pass")],
                }],
                draws: Vec::new(),
            }),
            Resolution::applied(),
        ],
        counterspell_offer(),
    );
    let mut world = world_of(&["fireball"], 3, rules);
    ran(&mut world, 1, "fireball");

    let nested = answer(&mut world, 2, 1, "counterspell");
    let wrong_seat = answer(&mut world, 2, 1, "pass");
    let done = answer(&mut world, 3, 1, "absorb");

    assert_eq!(nested.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(wrong_seat.record.declined, Some(DeclineCode::NotYourTurn));
    assert_eq!(done.outcome, Outcome::Applied);
}

#[test]
fn a_nesting_past_the_bound_is_refused_recorded_once_and_leaves_the_window_open() {
    let rules = fireball_rules(
        vec![Resolution::Await(Awaiting {
            kind: AwaitingKind::Reaction,
            offers: counterspell_offer(),
            draws: Vec::new(),
        })],
        counterspell_offer(),
    );
    let mut world = world_of(&["fireball"], 3, rules);
    world.limits = PipelineLimits {
        maximum_pending_nesting: 0,
        ..PipelineLimits::default()
    };
    ran(&mut world, 1, "fireball");

    let refused = answer(&mut world, 2, 1, "counterspell");

    assert_eq!(refused.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(world.table.pending().len(), 1);
    assert_eq!(
        failures_of(&world.events()),
        [(PortName::RulesOwner, FailureCause::InvalidAnswer)]
    );
}

#[test]
fn a_full_pending_book_declines_table_busy_and_commits_nothing() {
    let mut world = world_of(
        &["fireball"],
        3,
        fireball_rules(vec![Resolution::applied()], counterspell_offer()),
    );
    world.table = crate::pipeline::PipelineTable::with_limits(
        crate::golden::TABLE,
        world.table.facts().clone(),
        &PipelineLimits {
            maximum_open_pendings: 0,
            ..PipelineLimits::default()
        },
    );
    let before = world.table.durable_hash();

    let refused = ran(&mut world, 1, "fireball");

    assert_eq!(refused.record.declined, Some(DeclineCode::TableBusy));
    assert_eq!(before, world.table.durable_hash());
}

fn stab_world() -> World {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("stab")]),
        "stab",
    )
    .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let mut world = world_of(&["stab"], 2, rules);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab the innkeeper", Interpretation::Action(action("stab")));
    world
}

#[test]
fn a_tap_on_an_offered_choice_skips_the_confirmation_but_a_typed_reading_asks_first() {
    let mut tapped = stab_world();
    let direct = ran(&mut tapped, 1, "stab");
    let mut typed = stab_world();

    let asked = typed.text(1, "stab the innkeeper").expect("the line runs");

    assert_eq!(direct.outcome, Outcome::Applied);
    assert_eq!(asked.outcome, Outcome::Pending(PendingKind::Choice));
    assert_eq!(asked.readout.as_str(), "Waiting for seat 1 to choose.");
    assert_eq!(
        phase_of(&typed),
        "waiting",
        "the machine holds only the id of the confirmation it waits on (C.3, RES26)"
    );
    assert!(
        !typed
            .rules
            .calls()
            .iter()
            .any(|call| matches!(call, RulesCall::Resolve { .. })),
        "nothing is resolved before the yes"
    );
}

#[test]
fn the_seats_yes_runs_the_action_after_checking_it_again_and_the_pending_closes() {
    let mut world = stab_world();
    world.text(1, "stab the innkeeper").expect("the line runs");

    let other = answer(&mut world, 2, 1, "confirm");
    let confirmed = answer(&mut world, 1, 1, "confirm");

    assert_eq!(other.record.declined, Some(DeclineCode::NotYourTurn));
    assert_eq!(confirmed.outcome, Outcome::Applied);
    assert_eq!(changes(&confirmed).len(), 1);
    assert!(world.table.pending().is_empty());
    let prepares = world
        .rules
        .calls()
        .iter()
        .filter(|call| matches!(call, RulesCall::Prepare(_)))
        .count();
    assert_eq!(
        prepares, 2,
        "the action is re-validated against the table as it is then"
    );
}

#[test]
fn the_seats_no_drops_the_action_and_commits_nothing() {
    let mut world = stab_world();
    world.text(1, "stab the innkeeper").expect("the line runs");

    let cancelled = answer(&mut world, 1, 1, "cancel");

    assert_eq!(cancelled.outcome, Outcome::Applied);
    assert_eq!(cancelled.readout.as_str(), "Cancelled.");
    assert!(changes(&cancelled).is_empty());
    assert!(world.table.pending().is_empty());
    assert!(
        !world
            .rules
            .calls()
            .iter()
            .any(|call| matches!(call, RulesCall::Resolve { .. })),
        "a cancelled action never reaches the dice"
    );
}

/// The window of a strike a Shield could change, after the attack die was drawn in phase one.
fn shield_rules() -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
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
        .with_resumptions(action("strike"), vec![Resolution::applied()])
}

#[test]
fn a_committed_pending_survives_a_restart_and_resumes_with_its_stored_dice_without_drawing() {
    let mut before = world_of(&["strike"], 2, shield_rules());
    before.dice = CountingDice::new([14]);
    let opened = ran(&mut before, 1, "strike");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(before.dice.drawn(), 1);
    let now_ms = before.clock.now_ms();
    let saved = serde_json::to_string(&before.table.to_record(now_ms)).expect("the table writes");

    let mut after = restarted(&before, &saved, now_ms + 5_000_000);
    let resumed = answer(&mut after, 2, 1, "pass");

    assert_eq!(resumed.outcome, Outcome::Applied);
    assert_eq!(after.dice.drawn(), 0, "a restart never draws again");
    assert!(resumed.record.draws.is_empty());
    assert!(after.rules.calls().contains(&RulesCall::Resume {
        action: action("strike"),
        draws: 1,
        answers: vec![(seat(2), action("pass"))],
        ruling: None
    }));
}

/// A process that started again: new fakes, a clock that begins somewhere else, the table state
/// the host loaded and the pipeline's own record read back.
fn restarted(before: &World, saved: &str, now_ms: u64) -> World {
    let mut after = world_of(&["strike"], 2, shield_rules()).resuming_after(10);
    after.dice = CountingDice::default();
    after.clock = std::sync::Arc::new(crate::testkit::ManualClock::starting_at(now_ms));
    let record: crate::pipeline::PipelineRecord = serde_json::from_str(saved).expect("reads");
    after.table = crate::pipeline::PipelineTable::from_record(
        crate::golden::TABLE,
        before.table.facts().clone(),
        record,
        now_ms,
        &PipelineLimits::default(),
    )
    .expect("the record loads");
    after
}

#[test]
fn a_restart_keeps_the_waits_as_ages_not_as_readings_of_the_old_clock() {
    let mut before = world_of(&["strike"], 2, shield_rules());
    before.dice = CountingDice::new([14]);
    ran(&mut before, 1, "strike");
    before.clock.advance_ms(25_000);
    let now_ms = before.clock.now_ms();
    let saved = serde_json::to_string(&before.table.to_record(now_ms)).expect("the table writes");

    let mut after = restarted(&before, &saved, 7);
    let waited = after
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .map(|pending| pending.waited_ms(after.clock.now_ms()));
    after.clock.advance_ms(10_000);
    let nudged = nudge(&mut after);

    assert_eq!(
        waited,
        Some(25_000),
        "a clock that restarted below the age still counts the wait the pending had"
    );
    assert_eq!(nudged.outcome, Outcome::Applied);
    let waited_later = after
        .table
        .pending()
        .get(crate::envelope::PendingId::new(1))
        .map(|pending| pending.waited_ms(after.clock.now_ms()));
    assert_eq!(waited_later, Some(35_000));
    assert!(
        lines(&after)
            .iter()
            .any(|line| line.starts_with("PENDING nudged")),
        "35 seconds is past the nudge time of 30 although the new clock only reads 10"
    );
}

#[test]
fn replaying_the_input_after_a_restart_finds_its_stored_result_and_runs_nothing() {
    let mut world = world_of(&["strike"], 2, shield_rules());
    world.dice = CountingDice::new([14, 5]);
    let opened = ran(&mut world, 1, "strike");
    answer(&mut world, 2, 1, "pass");
    let now_ms = world.clock.now_ms();
    let saved = serde_json::to_string(&world.table.to_record(now_ms)).expect("the table writes");
    let mut after = restarted(&world, &saved, now_ms);
    let envelope = crate::envelope::InputEnvelope::from_player(
        correlation(1).operation_id,
        seat(1),
        crate::envelope::ViewStamp::new(correlation(1).revision),
        crate::envelope::PlayerInput::Tap {
            action: action("strike"),
        },
    );

    let replayed = after
        .run_with(&envelope, correlation(1))
        .expect("the replay runs");

    assert!(replayed.record.replayed);
    assert!(
        matches!(replayed.outcome, Outcome::Declined(ref decline) if decline.code() == DeclineCode::WindowClosed),
        "the pending resolved since: the seat is not told it is still waiting"
    );
    assert_eq!(replayed.pending, None, "no phantom pending");
    assert_eq!(after.dice.drawn(), 0);
    assert_eq!(after.rules.calls(), Vec::<RulesCall>::new());
    assert!(after.table.pending().is_empty());
    assert_eq!(opened.pending.map(|id| id.get()), Some(1));
}

#[test]
fn replaying_an_input_that_applied_draws_nothing_and_calls_nothing() {
    let mut world = world_of(&["wave"], 2, allows(ScriptedRulesOwner::default(), "wave"));
    world.dice = CountingDice::new([3]);
    ran(&mut world, 1, "wave");
    let calls_before = world.rules.calls().len();
    let envelope = crate::envelope::InputEnvelope::from_player(
        correlation(1).operation_id,
        seat(1),
        crate::envelope::ViewStamp::new(correlation(1).revision),
        crate::envelope::PlayerInput::Tap {
            action: action("wave"),
        },
    );

    let replayed = world
        .run_with(&envelope, correlation(1))
        .expect("the replay runs");

    assert!(replayed.record.replayed);
    assert_eq!(replayed.outcome, Outcome::Applied);
    assert_eq!(world.dice.drawn(), 0);
    assert_eq!(world.rules.calls().len(), calls_before);
    assert!(replayed.narration_job.is_none());
}

#[test]
fn replaying_the_input_that_opened_a_committed_pending_finds_it_and_runs_nothing() {
    let mut world = world_of(&["strike"], 2, shield_rules());
    world.dice = CountingDice::new([14, 5]);
    let opened = ran(&mut world, 1, "strike");
    let calls_before = world.rules.calls().len();
    let envelope = crate::envelope::InputEnvelope::from_player(
        correlation(1).operation_id,
        seat(1),
        crate::envelope::ViewStamp::new(correlation(1).revision),
        crate::envelope::PlayerInput::Tap {
            action: action("strike"),
        },
    );

    let replayed = world
        .run_with(&envelope, correlation(1))
        .expect("the replay runs");

    assert_eq!(replayed.outcome, Outcome::Pending(PendingKind::Reaction));
    assert!(replayed.record.replayed);
    assert_eq!(replayed.pending, opened.pending);
    assert_eq!(world.dice.drawn(), 1, "the replay drew nothing");
    assert_eq!(
        world.rules.calls().len(),
        calls_before,
        "the replay asked the rules owner nothing"
    );
    assert_eq!(world.table.pending().len(), 1);
}
