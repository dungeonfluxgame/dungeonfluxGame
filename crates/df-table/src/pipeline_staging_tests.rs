//! What the second adversary pass of C3 and C4 asked to be proven: a turn stages what it would make
//! visible and releases it only with the host's commit, a refused batch leaves nothing behind (not
//! a line, not a die, not a released reservation), a duplicate delivery gets the first answer and
//! writes nothing, the views survive a restart, a pending holds the machine and cannot stack, and
//! the copy a checkpoint takes does not grow with the history.

use crate::envelope::{
    CancelCause, EngineControl, EngineInput, InputEnvelope, PendingId, PlayerInput, ViewStamp,
};
use crate::golden::{CountingDice, World};
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pending::ScopedRuling;
use crate::pending::bounded::{BoundedMap, copied_entries, reset_copied_entries};
use crate::pipeline::{PipelineLimits, PipelineTable, TurnResult};
use crate::pipeline_fixtures_tests::{
    action, allows, d20, lines, narrated, ran, ruling_request, seat, world_of,
};
use crate::ports::{
    Awaiting, AwaitingKind, ConfirmationReason, Interpretation, Offer, Preparation, Resolution,
    RulingAnswer, RulingScope, RulingTier,
};
use crate::private_data::PlayerText;
use crate::projection::{AudienceProjection, Viewer};
use crate::testkit::{RulesCall, ScriptedInterpreter, ScriptedRulesOwner, correlation};

/// A tap by `seat_number` as the input numbered `turn`, built so that the same call can be sent
/// again as a duplicate delivery.
fn tap_envelope(turn: u64, seat_number: u64, name: &str) -> InputEnvelope {
    let probe = correlation(turn);
    InputEnvelope::from_player(
        probe.operation_id,
        seat(seat_number),
        ViewStamp::new(probe.revision),
        PlayerInput::Tap {
            action: action(name),
        },
    )
}

/// A typed line as the input numbered `turn`.
fn text_envelope(turn: u64, seat_number: u64, line: &str) -> InputEnvelope {
    let probe = correlation(turn);
    InputEnvelope::from_player(
        probe.operation_id,
        seat(seat_number),
        ViewStamp::new(probe.revision),
        PlayerInput::SubmitText {
            utterance: crate::envelope::UtteranceId::new(turn),
            text: PlayerText::new(line),
        },
    )
}

/// Runs an envelope the way the host does, up to and including its release.
fn send(world: &mut World, envelope: &InputEnvelope, turn: u64) -> TurnResult {
    world
        .run_with(envelope, correlation(turn))
        .expect("the input runs")
}

fn shield_rules() -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("strike"), action("wave")])
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
        .with_action(
            action("wave"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
}

fn shield_world(dice: &[u16]) -> World {
    let mut world = world_of(&["strike", "wave"], 3, shield_rules());
    world.dice = CountingDice::new(dice.iter().copied());
    world
}

// ---------------------------------------------------------------------------------------------
// Staged and released with the commit (#1692, #1722)
// ---------------------------------------------------------------------------------------------

#[test]
fn nothing_but_a_stage_timing_is_recorded_before_the_host_releases_a_turn() {
    let mut world = shield_world(&[14, 3]);
    let strike = tap_envelope(1, 1, "strike");
    let declined = tap_envelope(2, 9, "wave");

    let opened = world
        .run_unreleased(&strike, correlation(1))
        .expect("the strike runs");
    let refused = world
        .run_unreleased(&declined, correlation(2))
        .expect("the tap runs");
    let before_release = lines(&world);

    assert!(
        before_release.iter().all(|line| line.starts_with("STEP ")),
        "no die, decline, pending or effect is recorded before the commit: {before_release:?}"
    );
    assert!(
        opened
            .outcome_events
            .iter()
            .any(|event| event.render_audit_line().starts_with("DICE drawn")),
        "the die is staged with the turn"
    );
    assert!(
        refused
            .outcome_events
            .iter()
            .any(|event| event.render_audit_line().starts_with("INPUT declined")),
        "the decline is staged with the turn"
    );
    opened.release_events(&world.sink);
    refused.release_events(&world.sink);
    assert!(
        lines(&world)
            .iter()
            .any(|line| line.starts_with("DICE drawn") && line.contains("value=14"))
    );
}

#[test]
fn a_refused_batch_gives_its_dice_back_and_the_retried_inputs_meet_the_same_values() {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("strike")])
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        );
    let mut world = world_of(&["strike"], 1, rules);
    world.dice = CountingDice::new([14, 3]);
    let checkpoint = world.table.checkpoint();
    let first = tap_envelope(1, 1, "strike");
    let before_drawn = world.dice.drawn();

    let drawn_first = world
        .run_unreleased(&first, correlation(1))
        .expect("the strike runs");
    assert_eq!(world.dice.drawn(), before_drawn + 1);
    assert_eq!(drawn_first.draws().len(), 1);

    // The host refuses the batch: the table goes back and the dice go back, newest turn first.
    drawn_first.return_draws(&world.dice);
    world.table = checkpoint;
    assert_eq!(world.dice.drawn(), before_drawn, "the die is not used up");

    let retried = world
        .run_unreleased(&first, correlation(1))
        .expect("the strike runs again");

    assert_eq!(
        retried.record, drawn_first.record,
        "the same input, the same turn"
    );
    assert_eq!(
        retried.draws(),
        drawn_first.draws(),
        "the retried input meets the same die"
    );
}

#[test]
fn a_refused_batch_has_told_the_rules_owner_nothing_and_the_commit_tells_it_once() {
    let mut world = shield_world(&[14]);
    send(&mut world, &tap_envelope(1, 1, "strike"), 1);
    let abandons = |world: &World| {
        world
            .rules
            .calls()
            .into_iter()
            .filter(|call| matches!(call, RulesCall::Abandon(_)))
            .count()
    };
    let checkpoint = world.table.checkpoint();
    let cancel = InputEnvelope::from_engine(
        correlation(2).operation_id,
        ViewStamp::new(correlation(2).revision),
        EngineInput::Control(EngineControl::CancelPending {
            pending: PendingId::new(1),
            cause: CancelCause::HostDecision,
        }),
    );

    let ended = world
        .run_unreleased(&cancel, correlation(2))
        .expect("the exit runs");
    assert_eq!(ended.outcome, Outcome::Applied);
    assert_eq!(
        world.table.releases().len(),
        1,
        "the release is recorded with the turn"
    );
    assert_eq!(
        abandons(&world),
        0,
        "the rules owner is told after a commit, not before"
    );

    // The batch is refused: the checkpoint brings the pending back and nothing was released.
    world.table = checkpoint;
    assert!(world.table.releases().is_empty());
    assert_eq!(abandons(&world), 0);
    assert_eq!(world.table.pending().len(), 1);

    // The retry commits: now the rules owner is told, once.
    let again = world
        .run_unreleased(&cancel, correlation(2))
        .expect("the exit runs again");
    again.release_events(&world.sink);
    let told = world.with_pipeline(|pipeline, table| pipeline.issue_requests(table));
    assert_eq!(told.ok(), Some(0), "no broker request was due");
    assert_eq!(abandons(&world), 1);
    assert!(world.table.releases().is_empty());
    let _ = world.with_pipeline(|pipeline, table| pipeline.issue_requests(table));
    assert_eq!(
        abandons(&world),
        1,
        "a release already told is not told again"
    );
}

fn swing_decline() -> Decline {
    Decline::new(
        DeclineCode::NotViable,
        "The swing has nothing to hold on to.",
    )
    .expect("a reason")
}

fn stored_swing_ruling_world() -> World {
    let request = ruling_request("swing_from_set_piece", "no_advantage", RulingTier::Ordinary);
    let mut request = request;
    request.scope = RulingScope::Table;
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("swing"), action("wave")])
        .with_action(
            action("swing"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::Declined(swing_decline()),
        )
        .with_ruling(action("swing"), request.clone());
    let rules = allows(rules, "wave");
    let mut world = world_of(&["swing", "wave"], 1, rules);
    let mut book = crate::pending::RulingBook::remembering(8);
    book.store(ScopedRuling::new(
        request.key,
        RulingScope::Table,
        None,
        RulingAnswer::new("no_advantage").expect("an answer"),
        RulingTier::Ordinary,
        crate::pending::RulingSource::Host,
        action("swing"),
    ));
    world.table = world.table.clone().with_rulings(book);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("swing", Interpretation::Action(action("swing")))
        .with_line(
            "swing then wave",
            Interpretation::Plan(vec![action("wave"), action("swing")]),
        );
    world
}

#[test]
fn reusing_a_ruling_and_then_being_declined_releases_no_ruling_line_and_shows_no_note() {
    let mut world = stored_swing_ruling_world();

    let result = world.text(1, "swing").expect("the line runs");

    assert!(matches!(result.outcome, Outcome::Declined(_)));
    assert!(
        result.committed_events.is_empty(),
        "a turn that committed nothing releases no change"
    );
    assert!(
        !lines(&world)
            .iter()
            .any(|line| line.contains("RULING reused")),
        "{:?}",
        lines(&world)
    );
    assert!(!result.readout.as_str().contains("Ruling (earlier)"));
}

#[test]
fn a_plan_whose_second_step_reuses_a_ruling_and_is_declined_shows_no_ruling_note() {
    let mut world = stored_swing_ruling_world();

    let result = world.text(1, "swing then wave").expect("the line runs");

    // "wave" ran first, "swing" reused the ruling and was declined by the rules owner.
    assert_eq!(result.outcome, Outcome::Applied);
    assert!(
        !result.readout.as_str().contains("Ruling (earlier)"),
        "the ruling note belongs to a step that ran: {:?}",
        result.readout.as_str()
    );
    assert!(
        !result
            .committed_events
            .iter()
            .any(|event| event.render_audit_line().starts_with("RULING reused")),
        "the step that was declined takes its staged lines with it"
    );
}

#[test]
fn a_failed_die_on_an_answer_releases_nothing_and_leaves_the_pending_open() {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("pick")])
        .with_action(
            action("pick"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(
            action("pick"),
            vec![Resolution::Await(Awaiting {
                kind: AwaitingKind::Roll,
                offers: Vec::new(),
                draws: vec![d20()],
            })],
        )
        .with_resumptions(action("pick"), vec![Resolution::applied()]);
    let mut world = world_of(&["pick"], 1, rules);
    // A face the die does not have is an invalid answer of the dice port, never a roll.
    world.dice = CountingDice::new([99]);
    let opened = ran(&mut world, 1, "pick");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Roll));

    let answered = world.answer(1, 1, "roll").expect("the answer runs");

    assert!(matches!(answered.outcome, Outcome::Declined(_)));
    assert!(answered.committed_events.is_empty());
    assert!(
        !lines(&world)
            .iter()
            .any(|line| line.starts_with("PENDING answered")),
        "{:?}",
        lines(&world)
    );
    assert_eq!(world.table.pending().len(), 1, "the roll is still waiting");
}

// ---------------------------------------------------------------------------------------------
// A duplicate delivery gets the first answer and writes nothing (#1723, #1732)
// ---------------------------------------------------------------------------------------------

#[test]
fn a_duplicate_of_an_applied_input_changes_neither_the_views_nor_the_story() {
    let mut world = shield_world(&[]);
    let wave = tap_envelope(1, 1, "wave");
    let first = send(&mut world, &wave, 1);
    let views_before = world.table.attachments().digest();
    let rules_calls = world.rules.calls().len();

    let again = send(&mut world, &wave, 1);

    assert!(again.record.replayed);
    assert_eq!(again.outcome, Outcome::Applied);
    assert_eq!(
        again.readout, first.readout,
        "the first readout, not a placeholder"
    );
    assert_eq!(
        world.table.attachments().digest(),
        views_before,
        "the views are untouched"
    );
    assert_eq!(world.rules.calls().len(), rules_calls);
    assert!(again.outcome_events.is_empty() && again.committed_events.is_empty());
}

#[test]
fn a_duplicate_of_an_input_that_opened_a_pending_changes_nothing() {
    let mut world = shield_world(&[14]);
    let strike = tap_envelope(1, 1, "strike");
    let first = send(&mut world, &strike, 1);
    let views_before = world.table.attachments().digest();
    let drawn = world.dice.drawn();

    let again = send(&mut world, &strike, 1);

    assert_eq!(again.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(again.readout, first.readout);
    assert_eq!(world.table.attachments().digest(), views_before);
    assert_eq!(world.dice.drawn(), drawn, "no die is drawn again");
    assert_eq!(world.table.pending().len(), 1, "no second pending");
}

#[test]
fn a_duplicate_of_a_declined_input_returns_the_same_decline_and_asks_nothing() {
    let decline = Decline::new(DeclineCode::NotViable, "There is no chandelier here.")
        .and_then(|decline| decline.with_alternative("shoot"))
        .expect("a decline");
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("shoot"), action("swing")])
        .with_action(
            action("swing"),
            Preparation::Declined(decline.clone()),
            Resolution::Declined(decline),
        );
    let mut world = world_of(&["swing"], 1, rules);
    world.interpreter =
        ScriptedInterpreter::default().with_line("swing", Interpretation::Action(action("swing")));
    let line = text_envelope(1, 1, "swing");
    let first = send(&mut world, &line, 1);
    let calls = world.rules.calls().len();
    let reads = world.interpreter.calls();

    let again = send(&mut world, &line, 1);

    assert!(matches!(again.outcome, Outcome::Declined(_)));
    assert_eq!(again.readout, first.readout);
    assert_eq!(
        again.readout.as_str(),
        "There is no chandelier here. You could shoot."
    );
    assert_eq!(
        world.rules.calls().len(),
        calls,
        "the rules owner is not asked again"
    );
    assert_eq!(
        world.interpreter.calls(),
        reads,
        "the model is not paid for twice"
    );
}

#[test]
fn a_duplicate_of_a_clarified_input_does_not_ask_the_interpreter_again() {
    let rules = ScriptedRulesOwner::default().with_choices(
        seat(1),
        vec![action("attack_innkeeper"), action("attack_goblin")],
    );
    let mut world = world_of(&["attack_innkeeper", "attack_goblin"], 1, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "I attack",
        Interpretation::Ambiguous {
            candidates: vec![action("attack_innkeeper"), action("attack_goblin")],
        },
    );
    let line = text_envelope(1, 1, "I attack");
    let first = send(&mut world, &line, 1);
    let reads = world.interpreter.calls();
    let legal_asked = world.rules.calls().len();

    let again = send(&mut world, &line, 1);

    assert_eq!(again.outcome, Outcome::NeedsClarification);
    assert_eq!(again.clarification, first.clarification);
    assert_eq!(world.interpreter.calls(), reads);
    assert_eq!(world.rules.calls().len(), legal_asked);
}

// ---------------------------------------------------------------------------------------------
// One story line per turn (#1724) and one memory of operations (#1731)
// ---------------------------------------------------------------------------------------------

#[test]
fn a_narrated_two_step_plan_leaves_one_story_line_that_keeps_its_readout() {
    let rules = allows(
        allows(
            ScriptedRulesOwner::default()
                .with_choices(seat(1), vec![action("wave"), action("bow")]),
            "wave",
        ),
        "bow",
    );
    let mut world = world_of(&["wave", "bow"], 1, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "wave and bow",
        Interpretation::Plan(vec![action("wave"), action("bow")]),
    );
    let result = world.text(1, "wave and bow").expect("the line runs");
    let story_before = result
        .projections
        .iter()
        .map(|projection| projection.record().lines_shown)
        .max();

    let told = narrated(&mut world, &result);

    assert_eq!(told.as_deref(), Some("The moment passes."));
    let after = ran(&mut world, 1, "bow");
    let lines_of_seat_1: Vec<String> = after
        .projections
        .iter()
        .find_map(|projection| match projection {
            AudienceProjection::Seat(view) => Some(
                view.view()
                    .narration
                    .iter()
                    .map(|line| line.as_str().to_owned())
                    .collect(),
            ),
            AudienceProjection::Shared(_) => None,
        })
        .unwrap_or_default();
    assert_eq!(story_before, Some(1));
    assert_eq!(
        lines_of_seat_1,
        [
            "Seat 1: bow done.",
            "The moment passes.",
            "Seat 1: bow done."
        ],
        "the narrated turn is one story entry: its readout, then the storyteller's words, and the          next turn follows; the readout is not shown a second time"
    );
}

#[test]
fn the_table_and_the_result_book_cannot_disagree_about_a_forgotten_operation() {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );
    let mut world = world_of(&["wave"], 1, rules);
    world.limits = PipelineLimits {
        maximum_remembered_results: 1,
        ..PipelineLimits::default()
    };
    world.table = PipelineTable::with_limits(
        crate::golden::TABLE,
        world.table.facts().clone(),
        &world.limits,
    );
    let first = tap_envelope(1, 1, "wave");
    send(&mut world, &first, 1);
    send(&mut world, &tap_envelope(2, 1, "wave"), 2);
    send(&mut world, &tap_envelope(3, 1, "wave"), 3);
    assert!(
        world
            .table
            .results()
            .find(crate::pipeline::ResultKey::new(
                first.sender(),
                world.table.facts().membership_of(first.sender()),
                first.operation_id()
            ))
            .is_none(),
        "the result book has forgotten the first operation"
    );
    let calls = world.rules.calls().len();
    let drawn = world.dice.drawn();

    let again = send(&mut world, &first, 1);

    assert_eq!(
        again.record.declined,
        Some(DeclineCode::IntentInvalid),
        "the table still remembers it, so it is not run again"
    );
    assert_eq!(world.rules.calls().len(), calls, "no port is asked for it");
    assert_eq!(world.dice.drawn(), drawn);
}

#[test]
fn a_host_commit_stamp_is_never_an_operation_and_never_pushes_one_out_of_memory() {
    let mut world = world_of(&["wave"], 1, allows(ScriptedRulesOwner::default(), "wave"));
    world.table = PipelineTable::new(
        crate::golden::TABLE,
        world.table.facts().clone().remembering_operations(3),
    );
    for commit in 0..50 {
        world
            .table
            .stamp_commit_revision(correlation(10_000 + commit))
            .expect("the commit stamps");
    }

    for commit in 0..50 {
        assert!(
            !world.table.facts().has_applied(
                crate::envelope::Sender::Engine,
                &correlation(10_000 + commit).operation_id
            ),
            "a commit id is not remembered as an operation"
        );
    }
    assert_eq!(
        world.table.facts().snapshot().revision(),
        correlation(10_049).revision,
        "the revision follows the commits"
    );
}

// ---------------------------------------------------------------------------------------------
// Copies that do not grow with history (#1702)
// ---------------------------------------------------------------------------------------------

#[test]
fn a_checkpoint_copies_no_entry_and_the_first_change_after_it_copies_a_bucket_not_the_history() {
    let copies_after_a_checkpoint = |entries: u64| {
        let mut map: BoundedMap<u64, u64> = BoundedMap::holding_at_most(1_000_000);
        for number in 0..entries {
            map.insert(number, number);
        }
        reset_copied_entries();
        let checkpoint = map.clone();
        let copied_by_the_checkpoint = copied_entries();
        map.insert(entries + 1, 0);
        let copied_by_the_first_change = copied_entries();
        assert_eq!(map.len() as u64, entries + 1);
        assert_eq!(
            checkpoint.len() as u64,
            entries,
            "the checkpoint is as it was"
        );
        (copied_by_the_checkpoint, copied_by_the_first_change)
    };

    let small = copies_after_a_checkpoint(100);
    let large = copies_after_a_checkpoint(10_000);

    assert_eq!(small.0, 0);
    assert_eq!(large.0, 0);
    let map: BoundedMap<u64, u64> = BoundedMap::holding_at_most(10);
    assert!(
        map.bucket_count() <= 4_096,
        "a clone copies at most this many pointers"
    );
    // A bucket holds a handful of entries and an order chunk at most a thousand, whatever the
    // history: the first change copies those and nothing else.
    assert!(large.1 <= 2_048, "{large:?}");
    assert!(large.1 <= small.1 + 2_048, "{small:?} {large:?}");
}

#[test]
fn the_tables_checkpoint_and_its_first_write_do_not_copy_the_remembered_operations() {
    let first_write_copies = |remembered: u64| {
        let mut world = world_of(&["wave"], 1, wave_rules());
        // The rules owner is scripted per world; the table remembers as many operations as asked.
        for number in 0..remembered {
            let mut facts = world.table.facts().clone();
            facts
                .apply(
                    &crate::identity::TableScope::of_table(crate::golden::TABLE),
                    correlation(50_000 + number),
                    &[],
                )
                .expect("an empty commit applies");
            let record = world.table.to_record(0);
            world.table = PipelineTable::from_record(
                crate::golden::TABLE,
                facts,
                record,
                0,
                &PipelineLimits::default(),
            )
            .expect("the table loads");
        }
        reset_copied_entries();
        let checkpoint = world.table.checkpoint();
        let after_checkpoint = copied_entries();
        send(&mut world, &tap_envelope(1, 1, "wave"), 1);
        let after_write = copied_entries();
        drop(checkpoint);
        (after_checkpoint, after_write)
    };

    let few = first_write_copies(100);
    let many = first_write_copies(5_000);

    assert_eq!(few.0, 0);
    assert_eq!(many.0, 0);
    assert!(many.1 <= few.1 + 2_048, "{few:?} {many:?}");
}

fn wave_rules() -> ScriptedRulesOwner {
    allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    )
}

// ---------------------------------------------------------------------------------------------
// A pending holds the machine and cannot stack (#1725), and its exits release it
// ---------------------------------------------------------------------------------------------

fn stab_world() -> World {
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("stab")]),
        "stab",
    )
    .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral);
    let mut world = world_of(&["stab"], 2, rules);
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab him", Interpretation::Action(action("stab")));
    world
}

#[test]
fn a_second_confirmation_for_the_same_action_cannot_stack_on_the_first() {
    let mut world = stab_world();
    let first = world.text(1, "stab him").expect("the line runs");
    assert_eq!(first.outcome, Outcome::Pending(PendingKind::Choice));
    assert_eq!(world.machine_state().as_deref(), Some("waiting"));

    let second = world.text(1, "stab him").expect("the line runs");

    assert_eq!(second.record.declined, Some(DeclineCode::PendingOpen));
    assert_eq!(world.table.pending().len(), 1, "one confirmation, not two");
}

#[test]
fn a_ruling_asked_of_the_host_is_not_asked_twice() {
    let request = ruling_request("swing_from_set_piece", "no_advantage", RulingTier::Ordinary);
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(2), vec![action("swing")])
        .with_action(
            action("swing"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_ruling(action("swing"), request);
    let mut world = world_of(&["swing"], 2, rules);
    world.table = world.table.clone().hosted_by(seat(1));
    world.dice = CountingDice::new([15]);
    let opened = ran(&mut world, 2, "swing");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Ruling));

    let again = ran(&mut world, 2, "swing");

    assert_eq!(again.record.declined, Some(DeclineCode::PendingOpen));
    assert_eq!(world.table.pending().len(), 1, "the host is asked once");
}

#[test]
fn every_exit_of_a_confirmation_or_a_ruling_returns_the_machine_to_its_normal_state() {
    // The seat's own cancel.
    let mut world = stab_world();
    world.text(1, "stab him").expect("the line runs");
    world.answer(1, 1, "cancel").expect("the answer runs");
    assert_eq!(world.machine_state().as_deref(), Some("open"));

    // The host moves past it.
    let mut world = stab_world();
    world.text(1, "stab him").expect("the line runs");
    let ended = world
        .engine(EngineInput::Control(EngineControl::CancelPending {
            pending: PendingId::new(1),
            cause: CancelCause::HostDecision,
        }))
        .expect("the exit runs");
    assert_eq!(ended.outcome, Outcome::Applied);
    assert_eq!(world.machine_state().as_deref(), Some("open"));
    assert!(world.table.pending().is_empty());

    // The host refuses a ruling, and the requester cancels another.
    let request = ruling_request("swing_from_set_piece", "no_advantage", RulingTier::Ordinary);
    let rules = || {
        ScriptedRulesOwner::default()
            .with_choices(seat(2), vec![action("swing")])
            .with_action(
                action("swing"),
                Preparation::Ready { draws: Vec::new() },
                Resolution::applied(),
            )
            .with_ruling(action("swing"), request.clone())
    };
    for (answerer, choice) in [(1, "decline_ruling"), (2, "cancel_ruling")] {
        let mut world = world_of(&["swing"], 2, rules());
        world.table = world.table.clone().hosted_by(seat(1));
        ran(&mut world, 2, "swing");
        assert_eq!(world.machine_state().as_deref(), Some("waiting"));
        world.answer(answerer, 1, choice).expect("the answer runs");
        assert_eq!(world.machine_state().as_deref(), Some("open"), "{choice}");
    }
}

// ---------------------------------------------------------------------------------------------
// Ambiguity buttons are the seat's legal choices (#1726)
// ---------------------------------------------------------------------------------------------

#[test]
fn an_ambiguity_offers_only_the_candidates_the_seat_may_take_now() {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("attack_goblin"), action("dodge")]);
    let mut world = world_of(
        &["attack_innkeeper", "attack_goblin", "attack_dain", "dodge"],
        1,
        rules,
    );
    world.interpreter = ScriptedInterpreter::default()
        .with_line(
            "I attack",
            Interpretation::Ambiguous {
                candidates: vec![action("attack_innkeeper"), action("attack_goblin")],
            },
        )
        .with_line(
            "I attack someone",
            Interpretation::Ambiguous {
                candidates: vec![
                    action("attack_innkeeper"),
                    action("attack_goblin"),
                    action("dodge"),
                ],
            },
        );

    let one_legal = world.text(1, "I attack").expect("the line runs");
    let two_legal = world.text(1, "I attack someone").expect("the line runs");

    assert_eq!(
        one_legal.clarification.as_ref().map(|c| c.cause),
        Some(crate::pipeline::ClarifyCause::Unreadable),
        "one legal candidate is not an ambiguity: the seat is given its legal choices"
    );
    let options: Vec<&str> = two_legal
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
    assert_eq!(
        options,
        ["attack_goblin", "dodge"],
        "the refused candidate is not a button"
    );
}

// ---------------------------------------------------------------------------------------------
// Requests carry the correlation of the input that asked (#1730)
// ---------------------------------------------------------------------------------------------

#[test]
fn each_resource_request_is_issued_under_the_input_that_asked_for_it() {
    let scene = crate::golden::SceneMachine::new(&[("ask_map", crate::golden::Leaves::Nothing)])
        .and_then(|scene| {
            scene
                .requesting_on("ask_map", "map", 7)
                .map_err(crate::machine::SpecError::InvalidName)
        })
        .expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 2).expect("the world builds");
    world.rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("ask_map")]),
        "ask_map",
    );
    // Two inputs in one batch each ask for a resource; nothing is issued until the host commits.
    let first = world
        .run_unreleased(&tap_envelope(1, 1, "ask_map"), correlation(1))
        .expect("the first runs");
    world.table = {
        let mut table = world.table.clone();
        table
            .stamp_commit_revision(correlation(500))
            .expect("the commit stamps");
        table
    };
    let _ = first;
    let issued = world.with_pipeline(|pipeline, table| pipeline.issue_requests(table));

    assert_eq!(issued.ok(), Some(1));
    assert_eq!(
        world.broker.request_correlations(),
        [correlation(1)],
        "the request carries its own input's correlation, not the commit's"
    );
}

// ---------------------------------------------------------------------------------------------
// A restart does not blank the views (#1734), and offers are rebuilt by pending id (#1721)
// ---------------------------------------------------------------------------------------------

type Shown = Vec<(Viewer, String, Option<String>, Vec<String>)>;

/// What every audience is shown now, projected from the table and its attachments.
fn shown_now(world: &World) -> Shown {
    let snapshot = world.table.facts().snapshot();
    let attachments = world.table.attachments();
    let mut audiences = vec![crate::ports::Audience::Public];
    audiences.extend(
        snapshot
            .seats()
            .iter()
            .map(|seat| crate::ports::Audience::Seat(*seat)),
    );
    let projections: Vec<AudienceProjection> = audiences
        .into_iter()
        .filter_map(|audience| crate::projection::project(&snapshot, attachments, audience).ok())
        .collect();
    projections
        .iter()
        .map(|projection| match projection {
            AudienceProjection::Shared(view) => (
                projection.viewer(),
                view.readout().as_str().to_owned(),
                view.reason().map(|reason| reason.as_str().to_owned()),
                view.offers()
                    .iter()
                    .map(|offer| offer.offer_id.as_str().to_owned())
                    .collect(),
            ),
            AudienceProjection::Seat(view) => (
                projection.viewer(),
                view.readout().as_str().to_owned(),
                view.reason().map(|reason| reason.as_str().to_owned()),
                view.offers()
                    .iter()
                    .map(|offer| offer.offer_id.as_str().to_owned())
                    .collect(),
            ),
        })
        .collect()
}

/// What a restart does: the table's saved form read back over the facts the host loaded.
fn restart(world: &mut World) {
    let now_ms = crate::ports::Clock::now_ms(&*world.clock);
    let text = serde_json::to_string(&world.table.to_record(now_ms)).expect("the table writes");
    let record = serde_json::from_str(&text).expect("the record reads");
    world.table = PipelineTable::from_record(
        crate::golden::TABLE,
        world.table.facts().clone(),
        record,
        now_ms,
        &PipelineLimits::default(),
    )
    .expect("the record loads");
}

#[test]
fn a_restart_shows_every_audience_what_it_showed_and_keeps_the_offers_by_pending_id() {
    let mut world = shield_world(&[14]);
    ran(&mut world, 1, "wave");
    let opened = ran(&mut world, 1, "strike");
    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    let before = shown_now(&world);
    let hash_before = world.table.state_hash();
    assert!(
        before.iter().any(
            |(viewer, _, _, offers)| *viewer == Viewer::Seat { seat: seat(2) }
                && offers == &["shield".to_owned(), "pass".to_owned()]
        ),
        "the seat the window waits on holds its buttons: {before:?}"
    );

    restart(&mut world);

    assert_eq!(
        shown_now(&world),
        before,
        "no audience is blanked by a restart"
    );
    assert_eq!(world.table.state_hash(), hash_before);
    let answered = world.answer(2, 1, "pass").expect("the answer runs");
    assert_eq!(
        answered.outcome,
        Outcome::Applied,
        "the reconnecting seat answers by the same id"
    );
    let after = shown_now(&world);
    assert!(
        after
            .iter()
            .all(
                |(viewer, _, _, offers)| *viewer == Viewer::Shared || offers.is_empty() || {
                    false
                }
            ),
        "the buttons go when the pending closes: {after:?}"
    );
}

#[test]
fn a_seat_that_drops_and_comes_back_holds_the_same_buttons() {
    let mut world = shield_world(&[14]);
    ran(&mut world, 1, "strike");
    let before = shown_now(&world);

    world
        .engine(EngineInput::Control(EngineControl::SeatPresence {
            seat: seat(2),
            presence: crate::table_state::Presence::Disconnected,
        }))
        .expect("the drop runs");
    world
        .engine(EngineInput::Control(EngineControl::SeatPresence {
            seat: seat(2),
            presence: crate::table_state::Presence::Connected,
        }))
        .expect("the return runs");

    let after = shown_now(&world);
    let offers_of = |shown: &Shown, wanted: Viewer| {
        shown
            .iter()
            .find(|(viewer, _, _, _)| *viewer == wanted)
            .map(|(_, _, _, offers)| offers.clone())
    };
    let seat_2 = Viewer::Seat { seat: seat(2) };
    assert_eq!(offers_of(&after, seat_2), offers_of(&before, seat_2));
    assert_eq!(
        offers_of(&after, Viewer::Shared),
        Some(Vec::new()),
        "the shared screen holds no buttons"
    );
}

// ---------------------------------------------------------------------------------------------
// Failures that were swallowed are typed events (#1733)
// ---------------------------------------------------------------------------------------------

#[test]
fn a_machine_that_cannot_say_whether_it_is_finished_is_recorded_once_and_asks_for_no_hand_off() {
    let scene = crate::golden::SceneMachine::new(&[("wave", crate::golden::Leaves::Effect)])
        .map(crate::golden::SceneMachine::unable_to_say_if_finished)
        .expect("the scene builds");
    let mut world = World::with_scene_machine(scene, 1).expect("the world builds");
    world.rules = allows(ScriptedRulesOwner::default(), "wave");

    let result = ran(&mut world, 1, "wave");

    assert_eq!(result.outcome, Outcome::Applied);
    assert!(!result.hand_off_wanted);
    let failures: Vec<String> = lines(&world)
        .into_iter()
        .filter(|line| line.starts_with("MACHINE failed"))
        .collect();
    assert_eq!(
        failures,
        ["MACHINE failed scene=1 machine=scene cause=state_invalid"],
        "the failure is recorded, not turned into a hand-off that is quietly never asked for"
    );
}

#[test]
fn a_view_that_cannot_be_published_is_recorded_once_with_its_part_and_the_turn_still_ends() {
    let huge = "x".repeat(70_000);
    let decline = Decline::new(DeclineCode::NotViable, &huge).expect("a reason");
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("wave")])
        .with_action(
            action("wave"),
            Preparation::Declined(decline.clone()),
            Resolution::Declined(decline),
        );
    let mut world = world_of(&["wave"], 1, rules);

    let result = ran(&mut world, 1, "wave");

    assert!(matches!(result.outcome, Outcome::Declined(_)));
    let failures: Vec<String> = lines(&world)
        .into_iter()
        .filter(|line| line.starts_with("PROJECT view_failed"))
        .collect();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("part=publish"), "{failures:?}");
    assert_eq!(
        result.projections.len(),
        2,
        "every audience still has a projection"
    );
}

#[test]
fn an_input_starts_from_the_digest_the_one_before_it_ended_on() {
    let mut world = shield_world(&[]);

    let first = ran(&mut world, 1, "wave");
    let second = ran(&mut world, 1, "wave");
    let declined = ran(&mut world, 9, "wave");

    assert_eq!(
        second.record.state_hash_before,
        first.record.state_hash_after
    );
    assert_eq!(
        declined.record.state_hash_before,
        second.record.state_hash_after
    );
}

#[test]
fn a_rules_owner_that_asks_for_no_dice_at_all_is_an_invalid_answer_not_a_loop() {
    let rules = ScriptedRulesOwner::default()
        .with_choices(seat(1), vec![action("strike")])
        .with_action(
            action("strike"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_resolutions(action("strike"), vec![Resolution::NeedsDraws(Vec::new())]);
    let mut world = world_of(&["strike"], 1, rules);

    let result = ran(&mut world, 1, "strike");

    assert_eq!(result.record.declined, Some(DeclineCode::NotSupportedYet));
    assert_eq!(
        crate::pipeline_fixtures_tests::failures_of(&world.events()),
        [(
            crate::ports::PortName::RulesOwner,
            crate::event::FailureCause::InvalidAnswer
        )],
        "an empty request is no progress: it is recorded once and not asked again until the deadline"
    );
    assert_eq!(
        world
            .rules
            .calls()
            .iter()
            .filter(|call| matches!(call, RulesCall::Resolve { .. }))
            .count(),
        1
    );
}
