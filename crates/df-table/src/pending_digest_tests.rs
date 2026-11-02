//! The digests of the books are kept up to date where an entry changes and cost nothing to ask for
//! (O.16 S1): after any run of changes a book's kept digest equals the digest worked out again
//! from its entries, a book never holds more than its bound, and an input walks no book however
//! many entries the books hold.

use crate::ports::{ActionId, PurposeName, RulingAnswer, RulingKey, RulingScope, RulingTier};

use crate::envelope::{EngineInput, TimerName};
use crate::golden::{CountingDice, World};
use crate::identity::{MachineId, SceneId};
use crate::machine::RequestKey;
use crate::outcome::Outcome;
use crate::pending::bounded::BoundedMap;
use crate::pending::digest::{reset_walked_entries, walked_entries};
use crate::pending::{RulingBook, RulingSource, ScopedRuling};
use crate::pipeline::{PENDING_NUDGE_TIMER, RequestBook};
use crate::pipeline_fixtures_tests::{action, allows, d20, seat, world_of};
use crate::ports::{ConfirmationReason, Interpretation, Preparation, Resolution};
use crate::testkit::{ScriptedInterpreter, ScriptedRulesOwner};

/// A small deterministic generator, so a script is a function of its seed.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, limit: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % limit
    }
}

const SEEDS: u64 = 40;
const OPERATIONS: usize = 120;
/// How many different questions a script draws from: few enough that stores collide and replace.
const QUESTIONS: u64 = 9;
const BOOK_BOUND: usize = 5;
const SEATS: u64 = 6;

fn ruling(random: &mut Lcg) -> ScopedRuling {
    let key = RulingKey::new(&format!("question_{}", random.below(QUESTIONS))).expect("a key");
    let scope = if random.below(2) == 0 {
        RulingScope::Table
    } else {
        RulingScope::Scene
    };
    let scene = Some(SceneId::new(1 + random.below(3)));
    ScopedRuling::new(
        key,
        scope,
        scene,
        RulingAnswer::new(&format!("answer_{}", random.below(3))).expect("an answer"),
        if random.below(2) == 0 {
            RulingTier::Ordinary
        } else {
            RulingTier::Advantage
        },
        if random.below(2) == 0 {
            RulingSource::Host
        } else {
            RulingSource::ReviewedAutomatic
        },
        ActionId::new("swing").expect("an action"),
    )
}

#[test]
fn the_ruling_book_keeps_a_digest_equal_to_the_one_worked_out_from_its_entries() {
    for seed in 1..=SEEDS {
        let mut random = Lcg(seed);
        let mut book = RulingBook::remembering(BOOK_BOUND);
        reset_walked_entries();
        for step in 0..OPERATIONS {
            let probe = ruling(&mut random);
            let scene = Some(SceneId::new(1 + random.below(3)));
            match random.below(4) {
                0 | 1 => book.store(probe),
                2 => book.reconfirm(probe.key(), RulingScope::Scene, scene),
                _ => {
                    book.revoke(probe.key(), RulingScope::Table, scene);
                }
            }
            assert!(book.len() <= BOOK_BOUND, "seed {seed} step {step}");
            assert_eq!(
                book.digest(),
                book.recomputed_digest(),
                "seed {seed} step {step}: the kept digest drifted"
            );
        }
    }
}

#[test]
fn the_ruling_book_digest_depends_on_what_is_held_not_on_the_order_it_was_stored_in() {
    let mut random = Lcg(7);
    let rulings: Vec<ScopedRuling> = (0..4)
        .map(|index| {
            let mut one = ruling(&mut random);
            one = ScopedRuling::new(
                RulingKey::new(&format!("distinct_{index}")).expect("a key"),
                RulingScope::Table,
                None,
                one.answer().clone(),
                one.tier(),
                one.source(),
                one.asked_by_action().clone(),
            );
            one
        })
        .collect();
    let mut forward = RulingBook::remembering(BOOK_BOUND);
    let mut backward = RulingBook::remembering(BOOK_BOUND);
    for one in &rulings {
        forward.store(one.clone());
    }
    for one in rulings.iter().rev() {
        backward.store(one.clone());
    }

    assert_eq!(forward.digest(), backward.digest());
    backward.revoke(rulings[0].key(), RulingScope::Table, None);
    assert_ne!(forward.digest(), backward.digest());
}

#[test]
fn the_request_book_keeps_a_digest_equal_to_the_one_worked_out_from_its_entries() {
    let machines: Vec<MachineId> = ["scene", "lobby", "explore"]
        .into_iter()
        .map(|name| MachineId::new(name).expect("a machine"))
        .collect();
    let purpose = PurposeName::new("plate").expect("a purpose");
    for seed in 1..=SEEDS {
        let mut random = Lcg(seed);
        let mut book = RequestBook::holding_at_most(BOOK_BOUND);
        let mut next_handle = 100;
        for step in 0..OPERATIONS {
            let machine = machines[random.below(machines.len() as u64) as usize].clone();
            let key = RequestKey::new(random.below(QUESTIONS));
            match random.below(4) {
                0 | 1 => book.record_intent(machine, key, purpose.clone(), None),
                2 => {
                    next_handle += 1;
                    book.bind_handle(&machine, key, next_handle);
                }
                _ => {
                    let handle = 100 + random.below(next_handle - 99);
                    book.take_by_handle(handle);
                }
            }
            assert!(book.len() <= BOOK_BOUND, "seed {seed} step {step}");
            assert_eq!(
                book.digest(),
                book.recomputed_digest(),
                "seed {seed} step {step}: the kept digest drifted"
            );
        }
    }
}

/// A world in which every typed "hit" is read as a strike that asks its seat for a yes, so each seat that taps leaves
/// one pending open without the machine waiting for any of them.
fn crowded_world(windows: usize, seats: u64) -> World {
    let mut rules = ScriptedRulesOwner::default();
    for number in 1..=seats {
        rules = rules.with_choices(seat(number), vec![action("wave"), action("strike")]);
    }
    let rules = allows(rules, "wave")
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_confirmation(action("strike"), ConfirmationReason::HarmsANeutral);
    let mut world = world_of(&["wave", "strike"], seats, rules);
    world.interpreter =
        ScriptedInterpreter::default().with_line("hit", Interpretation::Action(action("strike")));
    world.dice = CountingDice::new((0..windows).map(|_| 12));
    world
}

fn pending_digest_is_kept(world: &World) -> bool {
    world.table.pending().digest() == world.table.pending().recomputed_digest()
}

#[test]
fn the_pending_book_keeps_a_digest_equal_to_the_one_worked_out_from_its_entries() {
    for seed in 1..=SEEDS {
        let mut random = Lcg(seed);
        let mut world = crowded_world(OPERATIONS, SEATS);
        for step in 0..OPERATIONS {
            let open = world
                .table
                .pending()
                .iter()
                .next()
                .map_or((99, 1), |pending| {
                    (pending.id().get(), pending.opened_by().get())
                });
            let result = match random.below(4) {
                0 => world.text(1 + random.below(SEATS), "hit"),
                1 => world.answer(
                    open.1,
                    open.0,
                    if random.below(2) == 0 {
                        "confirm"
                    } else {
                        "cancel"
                    },
                ),
                2 => world.tap(1 + random.below(SEATS), "wave"),
                _ => {
                    world.clock.advance_ms(40_000);
                    world.engine(EngineInput::Timer {
                        timer: TimerName::new(PENDING_NUDGE_TIMER).expect("a timer"),
                    })
                }
            }
            .expect("the input runs");
            assert!(
                matches!(
                    result.outcome,
                    Outcome::Applied | Outcome::Pending(_) | Outcome::Declined(_)
                ),
                "seed {seed} step {step}"
            );
            assert!(pending_digest_is_kept(&world), "seed {seed} step {step}");
        }
    }
}

#[test]
fn an_input_walks_no_book_whether_the_books_hold_few_entries_or_many() {
    let walked_by_a_wave_with = |windows: u64| {
        let mut world = crowded_world(windows as usize + 1, windows + 1);
        for opener in 1..=windows {
            world.text(opener, "hit").expect("the confirmation opens");
        }
        reset_walked_entries();
        world.tap(windows + 1, "wave").expect("the wave runs");
        (world.table.pending().len(), walked_entries())
    };

    let few = walked_by_a_wave_with(4);
    let many = walked_by_a_wave_with(24);

    assert_eq!(few.0, 4);
    assert_eq!(many.0, 24);
    assert_eq!(
        few.1, many.1,
        "an input costs the same with 4 open pendings or with 24"
    );
    assert_eq!(few.1, 0);
}

#[test]
fn the_bounded_map_forgets_the_oldest_first_and_compacts_its_order_list() {
    let mut map: BoundedMap<u64, u64> = BoundedMap::holding_at_most(3);
    let mut evicted = Vec::new();
    for number in 0..40_u64 {
        evicted.extend(map.insert(number % 7, number).evicted);
    }

    let held: Vec<u64> = map.oldest_first().into_iter().copied().collect();
    assert_eq!(
        held,
        [37, 38, 39],
        "the three newest stores stay, oldest first"
    );
    assert_eq!(map.values().count(), 3);
    assert!(
        map.order_len() <= 3 * 2 + 16,
        "the order list stays bounded"
    );
    assert!(!evicted.is_empty());
    assert!(map.insert(100, 1).replaced.is_none());
    let mut none: BoundedMap<u64, u64> = BoundedMap::holding_at_most(0);
    assert_eq!(
        none.insert(1, 1).evicted,
        [1],
        "a bound of zero stores nothing"
    );
    assert_eq!(none.len(), 0);
}
