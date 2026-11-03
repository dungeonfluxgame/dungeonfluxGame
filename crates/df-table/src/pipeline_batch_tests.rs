//! The pipeline keeps no state of its own, so a batch is only a way of grouping commits: running N
//! inputs one after another in memory and stamping one commit gives the state of N batches of one
//! with a stamp after each, a refused batch is undone by a checkpoint and its dice, and the table's
//! revision belongs to the host's commit, never to an input (S1 group commit).
//!
//! "The same state" is defined in the S1 decision: the digest of the table without the commit
//! stamps (`PipelineTable::state_hash` leaves out the revision and the pause stamp, and a commit id
//! is not an operation), so the host's own stamping, once per batch or once per input, does not
//! move it.

use crate::envelope::{
    EngineControl, EngineInput, InputEnvelope, PlayerInput, TimerName, ViewStamp,
};
use crate::golden::{CountingDice, TABLE, World};
use crate::identity::{RecoveryEpoch, SessionRevision};
use crate::outcome::{DeclineCode, Outcome, PendingKind};
use crate::pipeline::{PENDING_NUDGE_TIMER, PipelineLimits, PipelineTable, TurnRecord, TurnResult};
use crate::pipeline_fixtures_tests::{
    action, allows, d20, narrated, ruling_request, seat, world_of,
};
use crate::ports::Clock;
use crate::ports::{
    Awaiting, AwaitingKind, ConfirmationReason, Interpretation, Offer, Preparation, Resolution,
    RulingScope, RulingTier,
};
use crate::table_state::Presence;
use crate::testkit::{ScriptedInterpreter, ScriptedRulesOwner, correlation};

/// How many windows the scripted rules owner can open in one world.
const WINDOWS: usize = 40;

/// How many dice a world has queued: every input of a script may draw one.
const DICE: usize = 80;

/// A world with every kind of turn the plan names: a plain action, a strike that opens a
/// reaction window, a stab that needs the seat's yes when it is typed, a swing that needs the
/// host's ruling, an action the rules refuse, and a typed line for each.
fn scenario_world(windows: bool) -> World {
    let window = || {
        Resolution::Await(Awaiting {
            kind: AwaitingKind::Reaction,
            offers: vec![Offer {
                seat: seat(2),
                choices: vec![action("shield"), action("pass")],
            }],
            draws: Vec::new(),
        })
    };
    let mut ruling = ruling_request("swing_from_set_piece", "no_advantage", RulingTier::Ordinary);
    ruling.scope = RulingScope::Table;
    let mut rules = allows(ScriptedRulesOwner::default(), "wave")
        .with_choices(seat(1), vec![action("wave"), action("strike")])
        .with_choices(seat(2), vec![action("wave"), action("swing")])
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_action(
            action("stab"),
            Preparation::Ready { draws: Vec::new() },
            Resolution::applied(),
        )
        .with_confirmation(action("stab"), ConfirmationReason::HarmsANeutral)
        .with_action(
            action("swing"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_ruling(action("swing"), ruling);
    if windows {
        rules = rules
            .with_resolutions(action("strike"), (0..WINDOWS).map(|_| window()).collect())
            .with_resumptions(
                action("strike"),
                (0..WINDOWS).map(|_| Resolution::applied()).collect(),
            );
    }
    let rules = crate::pipeline_fixtures_tests::refuses(rules, "bad", DeclineCode::EconomySpent);
    let mut world = world_of(&["wave", "strike", "stab", "swing", "bad"], 2, rules);
    world.dice = CountingDice::new((0..DICE).map(|number| 1 + (number as u16 * 7) % 20));
    world.interpreter = ScriptedInterpreter::default()
        .with_line("stab him", Interpretation::Action(action("stab")))
        .with_line("swing", Interpretation::Action(action("swing")))
        .with_line(
            "wave twice",
            Interpretation::Plan(vec![action("wave"), action("wave")]),
        );
    world.table = world.table.clone().hosted_by(seat(1));
    world
}

/// What a host does with the table when it commits: its stamp, and the table's saved form read
/// back (a restart between batches).
fn commit_and_reload(world: &mut World, commit: u64) {
    world
        .table
        .stamp_commit_revision(correlation(commit))
        .expect("the host stamps the commit");
    let now_ms = world.clock.now_ms();
    let text = serde_json::to_string(&world.table.to_record(now_ms)).expect("the table writes");
    let record = serde_json::from_str(&text).expect("the record reads");
    world.table = PipelineTable::from_record(
        TABLE,
        world.table.facts().clone(),
        record,
        now_ms,
        &PipelineLimits::default(),
    )
    .expect("the record loads");
}

/// A small deterministic generator, so a script is a function of its seed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

/// One random input of the script: taps, typed lines, answers to whatever is pending, timers and
/// presence changes, in any order. Many are declined, which is part of the script.
fn one_input(world: &mut World, random: &mut Lcg) -> TurnResult {
    let tick = TimerName::new("tick").expect("a valid timer name");
    let nudge = TimerName::new(PENDING_NUDGE_TIMER).expect("a valid timer name");
    let open = world
        .table
        .pending()
        .iter()
        .next()
        .map_or((99, 1), |pending| {
            (pending.id().get(), pending.opened_by().get())
        });
    let answerer = 1 + random.next(2);
    match random.next(14) {
        0 => world.tap(1, "wave"),
        1 => world.tap(2, "wave"),
        2 => world.tap(1, "strike"),
        3 => world.answer(2, open.0, "pass"),
        4 => world.engine(EngineInput::Timer {
            timer: tick.clone(),
        }),
        5 => world.engine(EngineInput::Timer {
            timer: nudge.clone(),
        }),
        6 => world.engine(EngineInput::Control(EngineControl::SeatPresence {
            seat: seat(2),
            presence: if random.next(2) == 0 {
                Presence::Disconnected
            } else {
                Presence::Connected
            },
        })),
        7 => world.text(1, "stab him"),
        8 => world.answer(
            open.1,
            open.0,
            if random.next(2) == 0 {
                "confirm"
            } else {
                "cancel"
            },
        ),
        9 => world.text(2, "swing"),
        10 => world.answer(1, open.0, "accept_suggestion"),
        11 => world.tap(9, "wave"),
        12 => world.tap(answerer, "bad"),
        _ => world.text(1, "wave twice"),
    }
    .expect("the input runs")
}

/// What a turn did, without the digests: the digests of a turn that follows a narration differ
/// between a world that narrated after each input and one that narrated after the batch, while
/// what the turns did and the state they end in do not.
fn what_it_did(record: &TurnRecord) -> impl PartialEq + std::fmt::Debug {
    (
        record.correlation,
        record.sender,
        record.kind,
        record.outcome,
        record.declined,
        record.pending,
        record.draws.clone(),
        record.effects.clone(),
        record.replayed,
    )
}

/// The inputs of a script run in one batch (stamped once at the end) or as batches of one (a stamp
/// and a restart after each), narrated after the commit in both cases.
fn run_script(seed: u64, steps: usize, one_batch: bool) -> (Vec<TurnRecord>, u64) {
    let mut world = scenario_world(true);
    let mut random = Lcg(seed);
    let mut records = Vec::new();
    let mut results = Vec::new();
    for step in 0..steps {
        world.clock.advance_ms(7_000);
        let result = one_input(&mut world, &mut random);
        records.push(result.record.clone());
        if one_batch {
            results.push(result);
        } else {
            commit_and_reload(&mut world, 900 + step as u64);
            narrated(&mut world, &result);
        }
    }
    if one_batch {
        commit_and_reload_once(&mut world, 900);
        for result in &results {
            narrated(&mut world, result);
        }
    }
    (records, world.table.state_hash())
}

/// The host's stamp for a batch, without a restart (the saved form is read back by the singles).
fn commit_and_reload_once(world: &mut World, commit: u64) {
    world
        .table
        .stamp_commit_revision(correlation(commit))
        .expect("the host stamps the commit");
}

#[test]
fn n_inputs_in_one_batch_end_in_the_state_of_n_batches_of_one_for_many_scripts() {
    for seed in 1..=30 {
        let (batched, batched_hash) = run_script(seed, 16, true);
        let (single, single_hash) = run_script(seed, 16, false);

        let batched: Vec<_> = batched.iter().map(what_it_did).collect();
        let single: Vec<_> = single.iter().map(what_it_did).collect();
        assert_eq!(batched, single, "what the turns of seed {seed} did");
        assert_eq!(batched_hash, single_hash, "the final digest of seed {seed}");
    }
}

#[test]
fn the_scripts_reach_confirmations_rulings_declines_and_narration() {
    let mut reached = std::collections::BTreeSet::new();
    for seed in 1..=30 {
        let (records, _) = run_script(seed, 16, true);
        for record in &records {
            reached.insert(format!("{:?}", record.outcome));
            if let Some(code) = record.declined {
                reached.insert(format!("declined {}", code.key()));
            }
        }
    }

    assert!(reached.contains("Pending"), "{reached:?}");
    assert!(reached.contains("Applied"), "{reached:?}");
    assert!(reached.contains("declined not_viable"), "{reached:?}");
    assert!(reached.contains("declined economy_spent"), "{reached:?}");
    assert!(reached.contains("declined pending_open"), "{reached:?}");
}

#[test]
fn the_two_phases_of_a_pending_are_the_same_in_one_batch_or_in_two() {
    let make = || {
        let mut world = scenario_world(true);
        world.dice = CountingDice::new([14, 3]);
        world
    };
    let mut together = make();
    let opened = together.tap(1, "strike").expect("opens");
    let resolved = together.answer(2, 1, "pass").expect("resolves");
    together
        .table
        .stamp_commit_revision(correlation(900))
        .expect("stamps");
    let mut apart = make();
    apart.tap(1, "strike").expect("opens");
    commit_and_reload(&mut apart, 901);
    apart.answer(2, 1, "pass").expect("resolves");
    commit_and_reload(&mut apart, 902);

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(resolved.outcome, Outcome::Applied);
    assert_eq!(together.table.state_hash(), apart.table.state_hash());
}

#[test]
fn a_refused_batch_is_undone_by_the_checkpoint_and_its_dice() {
    let mut world = scenario_world(true);
    world.tap(1, "wave").expect("the first input runs");
    let before = world.table.state_hash();
    let drawn = world.dice.drawn();
    let checkpoint = world.table.checkpoint();

    let first = world.tap(1, "strike").expect("runs");
    let second = world.answer(2, 1, "pass").expect("runs");
    let third = world.tap(2, "wave").expect("runs");
    let after_the_batch = world.table.state_hash();
    for result in [&third, &second, &first] {
        result.return_draws(&world.dice);
    }
    world.table = checkpoint;

    assert_ne!(before, after_the_batch);
    assert_eq!(
        before,
        world.table.state_hash(),
        "the refused batch left nothing in the table"
    );
    assert_eq!(world.dice.drawn(), drawn, "and used no die");
    assert!(world.table.pending().is_empty());
    assert!(world.table.requests().is_empty());
    assert_eq!(first.outcome, Outcome::Pending(PendingKind::Reaction));
}

/// Stages are re-entrant: restoring a refused batch's checkpoint and giving its dice back, the
/// same inputs give the same turns and the same state, for scripts of every kind of turn.
#[test]
fn a_refused_batch_run_again_gives_equal_records_and_an_equal_state() {
    for seed in 1..=20 {
        // No windows: a window is a script the scripted rules owner consumes, which is the fake's
        // state and not the pipeline's.
        let mut world = scenario_world(false);
        let checkpoint = world.table.checkpoint();
        let mut first_run = Vec::new();
        let mut random = Lcg(seed);
        for _ in 0..14 {
            world.clock.advance_ms(7_000);
            first_run.push(one_input(&mut world, &mut random));
        }
        let first_hash = world.table.state_hash();
        for result in first_run.iter().rev() {
            result.return_draws(&world.dice);
        }
        world.table = checkpoint;
        // The inputs are numbered from the start again, as the host's retry numbers them.
        world = world.resuming_after(0);
        world.clock = std::sync::Arc::new(crate::testkit::ManualClock::starting_at(1_000));

        let mut second_run = Vec::new();
        let mut random = Lcg(seed);
        for _ in 0..14 {
            world.clock.advance_ms(7_000);
            second_run.push(one_input(&mut world, &mut random));
        }

        let records = |run: &[TurnResult]| {
            run.iter()
                .map(|result| result.record.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(records(&first_run), records(&second_run), "seed {seed}");
        assert_eq!(first_hash, world.table.state_hash(), "seed {seed}");
    }
}

#[test]
fn the_pipeline_never_stamps_the_table_revision_from_an_inputs_own_correlation() {
    let revision_after = |first_turn: u64| {
        let mut world = scenario_world(true);
        for turn in first_turn..first_turn + 3 {
            let probe = correlation(turn);
            let envelope = InputEnvelope::from_player(
                probe.operation_id,
                seat(1),
                ViewStamp::new(probe.revision),
                PlayerInput::Tap {
                    action: action("wave"),
                },
            );
            world.run_with(&envelope, probe).expect("the input runs");
        }
        world.table.facts().snapshot().revision()
    };

    let early = revision_after(1);
    let late = revision_after(500);

    assert_eq!(early, late);
    assert_eq!(early, SessionRevision::new(RecoveryEpoch::FIRST, 0));
}

#[test]
fn the_host_stamps_the_table_revision_once_at_the_batch_commit_and_the_digest_ignores_it() {
    let mut world = scenario_world(true);
    world.tap(1, "wave").expect("runs");
    world.tap(2, "wave").expect("runs");
    let before = world.table.facts().snapshot().revision();
    let digest_before = world.table.state_hash();

    world
        .table
        .stamp_commit_revision(correlation(900))
        .expect("the host stamps the commit");

    assert_eq!(before, SessionRevision::new(RecoveryEpoch::FIRST, 0));
    assert_eq!(
        world.table.facts().snapshot().revision(),
        SessionRevision::new(RecoveryEpoch::FIRST, 900)
    );
    assert_eq!(
        world.table.state_hash(),
        digest_before,
        "the digest is of the table without the commit stamps (S1)"
    );
}

#[test]
fn a_turn_record_carries_no_commit_identity_only_what_the_turn_did() {
    let mut world = scenario_world(true);
    let wave = world.tap(1, "wave").expect("runs");
    let strike = world.tap(1, "strike").expect("runs");
    let declined = world.tap(9, "wave").expect("runs");

    assert_eq!(strike.record.pending.map(|pending| pending.get()), Some(1));
    assert_eq!(strike.record.draws.len(), 1);
    assert_ne!(
        strike.record.state_hash_before,
        strike.record.state_hash_after
    );
    assert_ne!(wave.record.state_hash_before, wave.record.state_hash_after);
    assert_eq!(
        declined.record.durable_hash_before, declined.record.durable_hash_after,
        "a declined input changes nothing a checkpoint keeps"
    );
    assert_eq!(declined.record.declined, Some(DeclineCode::NotViable));
}

#[test]
fn a_batch_that_only_declined_changes_nothing_a_checkpoint_keeps_and_the_views_still_say_so() {
    let mut world = scenario_world(true);
    let durable = world.table.durable_hash();

    let declined = world.tap(9, "wave").expect("runs");

    assert_eq!(declined.record.declined, Some(DeclineCode::NotViable));
    assert_eq!(
        world.table.durable_hash(),
        durable,
        "a host writes no checkpoint for it"
    );
}
