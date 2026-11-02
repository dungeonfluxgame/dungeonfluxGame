//! The pipeline keeps no state of its own, so a batch is only a way of grouping commits: running N
//! inputs one after another in memory gives the state of N batches of one, a refused batch is
//! undone by a checkpoint, and the table's revision belongs to the host's commit (S1 group commit).

use crate::envelope::{
    EngineControl, EngineInput, InputEnvelope, PlayerInput, TimerName, ViewStamp,
};
use crate::golden::{CountingDice, TABLE, World};
use crate::identity::{RecoveryEpoch, SessionRevision};
use crate::outcome::{Outcome, PendingKind};
use crate::pipeline::{PENDING_NUDGE_TIMER, PipelineLimits, PipelineTable, TurnRecord};
use crate::pipeline_fixtures_tests::{action, allows, d20, seat, world_of};
use crate::ports::{Awaiting, AwaitingKind, Clock, Offer, Preparation, Resolution};
use crate::table_state::Presence;
use crate::testkit::{ScriptedRulesOwner, correlation};

/// How many windows the scripted rules owner can open in one world.
const WINDOWS: usize = 40;

fn scenario_world() -> World {
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
    let rules = allows(ScriptedRulesOwner::default(), "wave")
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_resolutions(action("strike"), (0..WINDOWS).map(|_| window()).collect())
        .with_resumptions(
            action("strike"),
            (0..WINDOWS).map(|_| Resolution::applied()).collect(),
        );
    let mut world = world_of(&["wave", "strike"], 2, rules);
    world.dice = CountingDice::new((0..WINDOWS).map(|_| 14));
    world
}

/// What a host does between batches: the pipeline's state as the commit saved it, read back.
fn commit_and_reload(world: &mut World) {
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

fn run_script(seed: u64, steps: usize, reload_between_inputs: bool) -> (Vec<TurnRecord>, u64) {
    let mut world = scenario_world();
    let mut random = Lcg(seed);
    let tick = TimerName::new("tick").expect("a valid timer name");
    let nudge = TimerName::new(PENDING_NUDGE_TIMER).expect("a valid timer name");
    let mut records = Vec::new();
    for _ in 0..steps {
        world.clock.advance_ms(7_000);
        let open = world
            .table
            .pending()
            .iter()
            .next()
            .map_or(99, |pending| pending.id().get());
        let result = match random.next(7) {
            0 => world.tap(1, "wave"),
            1 => world.tap(2, "wave"),
            2 => world.tap(1, "strike"),
            3 => world.answer(2, open, "pass"),
            4 => world.engine(EngineInput::Timer {
                timer: tick.clone(),
            }),
            5 => world.engine(EngineInput::Timer {
                timer: nudge.clone(),
            }),
            _ => world.engine(EngineInput::Control(EngineControl::SeatPresence {
                seat: seat(2),
                presence: if random.next(2) == 0 {
                    Presence::Disconnected
                } else {
                    Presence::Connected
                },
            })),
        }
        .expect("the input runs");
        records.push(result.record);
        if reload_between_inputs {
            commit_and_reload(&mut world);
        }
    }
    (records, world.table.state_hash())
}

#[test]
fn n_inputs_in_one_batch_end_in_the_state_of_n_batches_of_one_for_many_scripts() {
    for seed in 1..=30 {
        let (batched, batched_hash) = run_script(seed, 14, false);
        let (single, single_hash) = run_script(seed, 14, true);

        assert_eq!(batched, single, "the records of seed {seed}");
        assert_eq!(batched_hash, single_hash, "the final digest of seed {seed}");
    }
}

#[test]
fn the_two_phases_of_a_pending_are_the_same_in_one_batch_or_in_two() {
    let mut together = scenario_world();
    let opened = together.tap(1, "strike").expect("opens");
    let resolved = together.answer(2, 1, "pass").expect("resolves");
    let mut apart = scenario_world();
    apart.tap(1, "strike").expect("opens");
    commit_and_reload(&mut apart);
    apart.answer(2, 1, "pass").expect("resolves");

    assert_eq!(opened.outcome, Outcome::Pending(PendingKind::Reaction));
    assert_eq!(resolved.outcome, Outcome::Applied);
    assert_eq!(together.table.state_hash(), apart.table.state_hash());
}

#[test]
fn a_refused_batch_is_undone_by_the_checkpoint() {
    let mut world = scenario_world();
    world.tap(1, "wave").expect("the first input runs");
    let before = world.table.state_hash();
    let checkpoint = world.table.checkpoint();

    let first = world.tap(1, "strike").expect("runs");
    world.answer(2, 1, "pass").expect("runs");
    world.tap(2, "wave").expect("runs");
    let after_the_batch = world.table.state_hash();
    world.table = checkpoint;

    assert_ne!(before, after_the_batch);
    assert_eq!(
        before,
        world.table.state_hash(),
        "the refused batch left nothing in the table"
    );
    assert!(world.table.pending().is_empty());
    assert!(world.table.requests().is_empty());
    assert_eq!(first.outcome, Outcome::Pending(PendingKind::Reaction));
}

#[test]
fn the_pipeline_never_stamps_the_table_revision_from_an_inputs_own_correlation() {
    let revision_after = |first_turn: u64| {
        let mut world = scenario_world();
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
fn the_host_stamps_the_table_revision_once_at_the_batch_commit() {
    let mut world = scenario_world();
    world.tap(1, "wave").expect("runs");
    world.tap(2, "wave").expect("runs");
    let before = world.table.facts().snapshot().revision();

    world
        .table
        .stamp_commit_revision(correlation(900))
        .expect("the host stamps the commit");

    assert_eq!(before, SessionRevision::new(RecoveryEpoch::FIRST, 0));
    assert_eq!(
        world.table.facts().snapshot().revision(),
        SessionRevision::new(RecoveryEpoch::FIRST, 900)
    );
}

#[test]
fn a_turn_record_carries_no_commit_identity_only_what_the_turn_did() {
    let mut world = scenario_world();
    let wave = world.tap(1, "wave").expect("runs");
    let strike = world.tap(1, "strike").expect("runs");
    let declined = world.tap(2, "wave").expect("runs");

    assert_eq!(strike.record.pending.map(|pending| pending.get()), Some(1));
    assert_eq!(strike.record.draws.len(), 1);
    assert_ne!(
        strike.record.state_hash_before,
        strike.record.state_hash_after
    );
    assert_ne!(wave.record.state_hash_before, wave.record.state_hash_after);
    assert_eq!(
        declined.record.state_hash_before, declined.record.state_hash_after,
        "a declined input changes nothing"
    );
}
