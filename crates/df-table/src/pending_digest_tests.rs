//! The digests of the books are kept up to date where an entry changes and cost nothing to ask for
//! (O.16 S1): after any run of changes a book's kept digest equals the digest worked out again
//! from its entries, a book never holds more than its bound, and an input walks no book however
//! many entries the books hold.

use crate::ports::{ActionId, PurposeName, RulingAnswer, RulingKey, RulingScope, RulingTier};

use crate::envelope::PendingId;
use crate::golden::World;
use crate::identity::{MachineId, SceneId, TurnSeq};
use crate::machine::RequestKey;
use crate::outcome::PendingKind;
use crate::pending::bounded::BoundedMap;
use crate::pending::digest::{reset_walked_entries, walked_entries};
use crate::pending::window::{Completion, ResolutionWindow};
use crate::pending::{Continuation, PendingBook, PendingMark, PendingOpening, PendingResolution};
use crate::pending::{RulingBook, RulingSource, ScopedRuling};
use crate::pipeline::RequestBook;
use crate::pipeline_fixtures_tests::{action, allows, seat, world_of};
use crate::ports::{ConfirmationReason, Offer};
use crate::testkit::{ScriptedRulesOwner, correlation};

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
                0 | 1 => book.record_intent(machine, key, purpose.clone(), None, correlation(1)),
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

/// A pending of `seat` that offers the seat a confirm and a cancel, as the book would hold it.
fn opening(id: u64, seat_number: u64) -> PendingResolution {
    let window = ResolutionWindow::new(
        vec![Offer {
            seat: seat(seat_number),
            choices: vec![action("confirm"), action("cancel")],
        }],
        Completion::AnyOffer,
    )
    .expect("a window");
    PendingResolution::open(PendingOpening {
        id: PendingId::new(id),
        kind: PendingKind::Choice,
        opened_by: seat(seat_number),
        operation: correlation(id).operation_id,
        turn: TurnSeq::new(id),
        now_ms: id * 1_000,
        scene: Some(SceneId::new(1)),
        continuation: Continuation::Confirm {
            seat: seat(seat_number),
            action: action("strike"),
            reason: ConfirmationReason::HarmsANeutral,
        },
        draws: Vec::new(),
        window,
    })
}

#[test]
fn the_pending_book_keeps_a_digest_equal_to_the_one_worked_out_from_its_entries() {
    for seed in 1..=SEEDS {
        let mut random = Lcg(seed);
        let mut book = PendingBook::holding_at_most(BOOK_BOUND);
        for step in 0..OPERATIONS {
            let seat_number = 1 + random.below(SEATS);
            match random.below(5) {
                0 | 1 => {
                    let next = book.next_id().get();
                    // A full book refuses, which leaves the digest as it was.
                    let _ = book.insert(opening(next, seat_number));
                }
                2 => {
                    let held: Vec<PendingId> = book.iter().map(PendingResolution::id).collect();
                    if let Some(id) = held.get(random.below(held.len().max(1) as u64) as usize) {
                        book.remove(*id);
                    }
                }
                3 => book.mark(if random.below(2) == 0 {
                    PendingMark::Hold(seat(seat_number))
                } else {
                    PendingMark::Release(seat(seat_number))
                }),
                _ => {
                    let held: Vec<PendingId> = book.iter().map(PendingResolution::id).collect();
                    if let Some(id) = held.first() {
                        book.mark(PendingMark::Nudged(*id));
                    }
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

/// A world whose table already remembers `rulings` stored rulings.
fn world_with_rulings(rulings: usize) -> World {
    let mut book = RulingBook::remembering(rulings + 1);
    let mut random = Lcg(3);
    for index in 0..rulings {
        let mut one = ruling(&mut random);
        one = ScopedRuling::new(
            RulingKey::new(&format!("remembered_{index}")).expect("a key"),
            RulingScope::Table,
            None,
            one.answer().clone(),
            one.tier(),
            one.source(),
            one.asked_by_action().clone(),
        );
        book.store(one);
    }
    let rules = allows(
        ScriptedRulesOwner::default().with_choices(seat(1), vec![action("wave")]),
        "wave",
    );
    let mut world = world_of(&["wave"], 1, rules);
    world.table = world.table.clone().with_rulings(book);
    world
}

#[test]
fn an_input_walks_no_book_whether_the_books_hold_few_entries_or_many() {
    let walked_by_a_wave_with = |rulings: usize| {
        let mut world = world_with_rulings(rulings);
        reset_walked_entries();
        world.tap(1, "wave").expect("the wave runs");
        world.tap(1, "wave").expect("the wave runs again");
        let issued = world.with_pipeline(|pipeline, table| pipeline.issue_requests(table));
        assert_eq!(issued.ok(), Some(0));
        (world.table.rulings().len(), walked_entries())
    };

    let few = walked_by_a_wave_with(4);
    let many = walked_by_a_wave_with(2_000);

    assert_eq!(few.0, 4);
    assert_eq!(many.0, 2_000);
    assert_eq!(
        few.1, many.1,
        "an input costs the same with 4 stored rulings or with 2000"
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
