//! What each audience is not shown: sentinel secrets (a hidden creature's name, a machine's saved
//! note, a ruling's key, another seat's offers) never appear in the JSON or the Debug text of a
//! report built for a seat or the host, and a state that differs only in something an audience may
//! not see gives that audience an equal report (the paired-state rule of H.11).

use std::sync::Arc;

use crate::effects::TableEffect;
use crate::envelope::PendingId;
use crate::golden::CountingDice;
use crate::identity::{OperationId, SceneId, TurnSeq};
use crate::inspect::{
    ContinuationDetail, Dump, Inspect, InspectAudience, MachineStanding, MachinesOf, Needs,
    PendingReport, ProjectionStanding, Visible, WiredPorts,
};
use crate::inspect_tests::{
    everything, host, operator, scene_world, seat_view, strike_rules, table_of_three,
    table_with_machines, view_of,
};
use crate::machine::Scope;
use crate::outcome::PendingKind;
use crate::owner_queue_tests::{NullSink, proposal, seated_queue, settings, tap};
use crate::pending::{
    Completion, Continuation, PendingBook, PendingOpening, PendingResolution, ResolutionWindow,
    RulingBook, RulingSource, ScopedRuling, StoredDraw,
};
use crate::pipeline_fixtures_tests::{action, world_of};
use crate::ports::{
    ActionId, Draw, Interpretation, Offer, PortName, RulingAnswer, RulingKey, RulingOption,
    RulingRequest, RulingScope, RulingTier,
};
use crate::projection::Viewer;
use crate::table_state::{CastChange, Visibility};
use crate::table_state_tests::{apply, element, machine, scene, seat};
use crate::testkit::ScriptedInterpreter;

pub(crate) const HIDDEN_NAME: &str = "SENTINEL-HIDDEN-Ogre";
const PAYLOAD_SECRET: &str = "SENTINEL-PAYLOAD-note";
const RULING_KEY: &str = "sentinel-ruling-key";
const OFFER_SECRET: &str = "sentinel-seat-two-choice";

fn audiences() -> [InspectAudience; 4] {
    [
        InspectAudience::Host,
        InspectAudience::Seat { seat: seat(1) },
        InspectAudience::Seat { seat: seat(2) },
        InspectAudience::Seat { seat: seat(3) },
    ]
}

// The table state.

#[test]
fn a_hidden_creature_is_in_no_host_or_seat_report_and_changes_none_of_them() {
    let with_ogre = table_of_three(true);
    let without = table_of_three(false);

    for audience in audiences() {
        let shown = with_ogre.inspect(view_of(audience));
        let text = everything(&shown);
        assert!(
            !text.contains(HIDDEN_NAME) && !text.contains("ogre"),
            "{audience:?} was shown the hidden creature"
        );
        assert_eq!(
            shown,
            without.inspect(view_of(audience)),
            "{audience:?} could tell the hidden creature from none"
        );
    }
}

#[test]
fn the_operator_sees_the_hidden_creature_and_tells_the_tables_apart() {
    let with_ogre = table_of_three(true).inspect(operator());
    let without = table_of_three(false).inspect(operator());

    assert!(everything(&with_ogre).contains("ogre"));
    assert_ne!(with_ogre, without);
}

#[test]
fn a_creature_the_party_can_only_sense_or_not_see_is_left_out_whatever_the_reason() {
    for visibility in [
        Visibility::Invisible,
        Visibility::Sensed,
        Visibility::Hidden,
    ] {
        let mut table = table_of_three(true);
        apply(
            &mut table,
            3,
            &[TableEffect::CastChanged {
                cast: element("ogre"),
                change: CastChange::Visibility { visibility },
            }],
        )
        .expect("the change applies");

        for audience in audiences() {
            let text = everything(&table.inspect(view_of(audience)));
            assert!(
                !text.contains("ogre"),
                "{visibility:?} shown to {audience:?}"
            );
        }
    }
}

#[test]
fn a_seat_sees_its_own_hidden_hero_and_no_other_audience_does() {
    let mut table = table_of_three(false);
    apply(
        &mut table,
        3,
        &[TableEffect::CastChanged {
            cast: element("hero2"),
            change: CastChange::Visibility {
                visibility: Visibility::Hidden,
            },
        }],
    )
    .expect("hero two hides");

    let heroes_in = |audience| -> Vec<String> {
        table
            .inspect(view_of(audience))
            .cast
            .iter()
            .filter(|line| line.id.as_str().starts_with("hero"))
            .map(|line| line.id.as_str().to_owned())
            .collect()
    };

    assert_eq!(
        heroes_in(InspectAudience::Seat { seat: seat(2) }),
        ["hero1", "hero2", "hero3"]
    );
    assert_eq!(
        heroes_in(InspectAudience::Seat { seat: seat(1) }),
        ["hero1", "hero3"]
    );
    assert_eq!(
        heroes_in(InspectAudience::Seat { seat: seat(3) }),
        ["hero1", "hero3"]
    );
    assert_eq!(heroes_in(InspectAudience::Host), ["hero1", "hero3"]);
    assert_eq!(
        heroes_in(InspectAudience::Operator),
        ["hero1", "hero2", "hero3"]
    );
}

#[test]
fn a_foes_hit_points_are_the_operators_and_the_partys_are_public() {
    let table = table_of_three(false);

    for audience in audiences() {
        let report = table.inspect(view_of(audience));
        for line in &report.cast {
            let is_party = line.id.as_str().starts_with("hero");
            assert_eq!(line.vitals.is_shown(), is_party, "{audience:?} {}", line.id);
        }
    }
    let goblin = table
        .inspect(operator())
        .cast
        .into_iter()
        .find(|line| line.id.as_str() == "goblin")
        .expect("the goblin is listed");
    assert_eq!(
        goblin.vitals.shown().map(|vitals| vitals.hit_points),
        Some(12)
    );
}

#[test]
fn the_table_digest_is_the_operators_because_it_moves_with_every_hidden_fact() {
    let with_ogre = table_of_three(true);
    let without = table_of_three(false);

    assert_ne!(with_ogre.digest(), without.digest());
    for audience in audiences() {
        assert_eq!(
            with_ogre.inspect(view_of(audience)).digest,
            Visible::Redacted {
                needs: Needs::Operator
            }
        );
    }
}

#[test]
fn a_scenes_beat_is_story_progress_the_host_may_read_and_a_seat_may_not() {
    let mut table = table_of_three(false);
    apply(
        &mut table,
        3,
        &[TableEffect::BeatResolved {
            scene: scene(10),
            beat: crate::effects::BeatName::new("sentinel-beat").expect("valid beat"),
        }],
    )
    .expect("the beat is resolved");

    for number in [1, 2, 3] {
        assert!(!everything(&table.inspect(seat_view(number))).contains("sentinel-beat"));
    }
    for view in [host(), operator()] {
        assert!(everything(&table.inspect(view)).contains("sentinel-beat"));
    }
}

// Machines.

#[test]
fn a_machines_saved_note_is_the_operators_and_a_seat_sees_only_the_machine_its_screen_names() {
    let world = scene_world();
    let table = table_with_machines(PAYLOAD_SECRET);
    let snapshot = table.snapshot();
    let machines = MachinesOf {
        registry: &world.registry,
        snapshot: &snapshot,
    };

    let for_operator = machines.inspect(operator());
    let for_host = machines.inspect(host());
    let for_seat = machines.inspect(seat_view(2));

    assert!(everything(&for_operator).contains(PAYLOAD_SECRET));
    assert_eq!(for_host.machines.len(), 2);
    assert!(!everything(&for_host).contains(PAYLOAD_SECRET));
    assert_eq!(for_seat.machines.len(), 1);
    assert_eq!(for_seat.machines[0].machine, machine("scene"));
    assert!(!everything(&for_seat).contains(PAYLOAD_SECRET));
    match &for_seat.machines[0].standing {
        MachineStanding::Reported { payload, .. } => {
            assert_eq!(
                *payload,
                Visible::Redacted {
                    needs: Needs::Operator
                }
            );
        }
        other => panic!("expected a report, got {other:?}"),
    }
}

#[test]
fn two_tables_that_differ_only_in_a_machines_note_give_the_host_and_every_seat_equal_reports() {
    let world = scene_world();
    let first = table_with_machines("one note");
    let second = table_with_machines("another note");
    let (first_snapshot, second_snapshot) = (first.snapshot(), second.snapshot());
    let of = |snapshot| MachinesOf {
        registry: &world.registry,
        snapshot,
    };

    for audience in audiences() {
        assert_eq!(
            of(&first_snapshot).inspect(view_of(audience)),
            of(&second_snapshot).inspect(view_of(audience)),
            "{audience:?}"
        );
    }
    assert_ne!(
        of(&first_snapshot).inspect(operator()),
        of(&second_snapshot).inspect(operator())
    );
}

// The pending book and the rulings.

fn offer(seat_number: u64, choices: &[&str]) -> Offer {
    Offer {
        seat: seat(seat_number),
        choices: choices.iter().map(|name| action(name)).collect(),
    }
}

fn operation(number: u128) -> OperationId {
    OperationId::from_nonzero_u128(std::num::NonZeroU128::new(number).expect("nonzero"))
}

fn ruling_request() -> RulingRequest {
    let option = |answer: &str| RulingOption {
        answer: RulingAnswer::new(answer).expect("valid answer"),
        tier: RulingTier::Ordinary,
    };
    RulingRequest {
        key: RulingKey::new(RULING_KEY).expect("valid key"),
        scope: RulingScope::Scene,
        suggested: option("sentinel-suggested"),
        alternatives: vec![option("sentinel-alternative")],
        reviewed_automatic: false,
    }
}

fn open(book: &mut PendingBook, opened_by: u64, window: Vec<Offer>, draw: Option<u16>) {
    let draws = draw
        .map(|value| {
            let die = Draw::new(std::num::NonZeroU16::new(20).expect("nonzero"), value)
                .expect("a possible die");
            vec![StoredDraw::of(die)]
        })
        .unwrap_or_default();
    let number = book.next_id().get();
    let pending = PendingResolution::open(PendingOpening {
        id: book.next_id(),
        kind: PendingKind::Ruling,
        opened_by: seat(opened_by),
        operation: operation(u128::from(number)),
        turn: TurnSeq::new(number),
        now_ms: 1_000,
        scene: Some(SceneId::new(1)),
        continuation: Continuation::Ruling {
            seat: seat(opened_by),
            action: action("climb"),
            request: ruling_request(),
            reuse: false,
        },
        draws,
        window: ResolutionWindow::new(window, Completion::EveryOffer).expect("a window"),
    });
    book.insert(pending).expect("inserted");
}

/// A pending seat 1 opened that asks seats 2 and 3, and another only seat 1 is asked by.
fn book_of_two() -> PendingBook {
    let mut book = PendingBook::default();
    open(
        &mut book,
        1,
        vec![offer(2, &["shield", OFFER_SECRET]), offer(3, &["pass"])],
        Some(17),
    );
    open(&mut book, 1, vec![offer(1, &["keep"])], None);
    book
}

fn ids_of(report: &PendingReport) -> Vec<PendingId> {
    report.pendings.iter().map(|line| line.id).collect()
}

#[test]
fn a_seat_sees_only_the_pendings_it_opened_or_is_asked_and_only_its_own_offer() {
    let book = book_of_two();

    let for_three = book.inspect(seat_view(3));
    let for_two = book.inspect(seat_view(2));
    let for_one = book.inspect(seat_view(1));

    assert_eq!(ids_of(&for_three), [PendingId::new(1)]);
    assert_eq!(for_three.pendings[0].offers.len(), 1);
    assert_eq!(for_three.pendings[0].offers[0].choices, [action("pass")]);
    assert!(!everything(&for_three).contains(OFFER_SECRET));
    assert_eq!(
        for_two.pendings[0].offers[0].choices,
        [action("shield"), action(OFFER_SECRET)]
    );
    assert_eq!(ids_of(&for_one), [PendingId::new(1), PendingId::new(2)]);
    assert!(
        for_one.pendings[0].offers.is_empty(),
        "seat 1 opened the first and is not asked by it"
    );
}

#[test]
fn what_a_pending_will_resume_and_the_dice_it_drew_are_not_a_seats_to_read() {
    let book = book_of_two();

    for number in [1, 2, 3] {
        let text = everything(&book.inspect(seat_view(number)));
        assert!(
            !text.contains(RULING_KEY) && !text.contains("sentinel-suggested"),
            "seat {number} read the continuation"
        );
    }
    let for_host = book.inspect(host());
    let continuation = for_host.pendings[0]
        .continuation
        .shown()
        .expect("the host sees what a pending resumes");
    assert!(matches!(
        &continuation.detail,
        ContinuationDetail::Ruling { request, .. } if request.key.as_str() == RULING_KEY
    ));
    assert_eq!(
        for_host.pendings[0].draws,
        Visible::Redacted {
            needs: Needs::Operator
        },
        "a hidden roll is not read off the host's report"
    );
    let drawn = book.inspect(operator()).pendings[0]
        .draws
        .shown()
        .expect("the operator sees the dice")
        .iter()
        .map(|draw| draw.value())
        .collect::<Vec<_>>();
    assert_eq!(drawn, [17]);
}

#[test]
fn a_pending_that_does_not_involve_a_seat_leaves_that_seats_report_as_it_was() {
    let mut with_second = book_of_two();
    let mut without = PendingBook::default();
    open(
        &mut without,
        1,
        vec![offer(2, &["shield", OFFER_SECRET]), offer(3, &["pass"])],
        Some(17),
    );
    open(&mut with_second, 1, vec![offer(1, &["keep"])], Some(3));

    for number in [2, 3] {
        assert_eq!(
            with_second.inspect(seat_view(number)),
            without.inspect(seat_view(number)),
            "seat {number}"
        );
    }
}

#[test]
fn the_count_of_open_pendings_is_the_hosts_and_a_seat_is_told_it_is_hidden() {
    let book = book_of_two();

    assert_eq!(book.inspect(host()).open.shown(), Some(&2));
    assert_eq!(
        book.inspect(seat_view(1)).open,
        Visible::Redacted { needs: Needs::Host }
    );
}

#[test]
fn stored_rulings_are_the_hosts_and_never_a_seats() {
    let mut rulings = RulingBook::default();
    rulings.store(ScopedRuling::new(
        RulingKey::new(RULING_KEY).expect("valid key"),
        RulingScope::Scene,
        Some(scene(10)),
        RulingAnswer::new("sentinel-answer").expect("valid answer"),
        RulingTier::Ordinary,
        RulingSource::Host,
        ActionId::new("climb").expect("valid action"),
    ));

    for number in [1, 2, 3] {
        let text = everything(&rulings.inspect(seat_view(number)));
        assert!(!text.contains(RULING_KEY) && !text.contains("sentinel-answer"));
    }
    let for_host = rulings.inspect(host());
    assert_eq!(for_host.stored.shown(), Some(&1));
    assert!(matches!(
        for_host.entries.shown(),
        Some(Dump::Json { value }) if value.to_string().contains(RULING_KEY)
    ));
}

// The queue, the registry and the ports.

#[test]
fn a_seat_sees_its_own_queue_line_and_operations_and_neither_another_seats_nor_the_engines() {
    let mut queue = seated_queue(3, &settings(4, 4, 1), Arc::new(NullSink));
    queue.admit(tap(1, 1), 100).expect("admitted");
    queue.admit(tap(2, 2), 100).expect("admitted");
    queue.admit(proposal(3), 100).expect("admitted");
    let first = queue
        .serve_next(200)
        .expect("serves")
        .expect("something waits");
    let second = queue
        .serve_next(200)
        .expect("serves")
        .expect("something waits");
    let third = queue
        .serve_next(200)
        .expect("serves")
        .expect("something waits");
    assert_eq!(
        [
            first.turn_seq.get(),
            second.turn_seq.get(),
            third.turn_seq.get()
        ],
        [1, 2, 3]
    );

    let for_one = queue.inspect(seat_view(1));
    let for_host = queue.inspect(host());

    assert_eq!(for_one.seats.len(), 1);
    assert_eq!(for_one.seats[0].seat, seat(1));
    assert!(
        for_one
            .in_progress
            .iter()
            .all(|line| line.sender == Some(seat(1)))
    );
    assert_eq!(for_one.in_progress.len(), 1);
    assert_eq!(for_one.lanes, Visible::Redacted { needs: Needs::Host });
    assert_eq!(
        for_one.last_turn_seq,
        Visible::Redacted { needs: Needs::Host }
    );
    assert_eq!(for_host.seats.len(), 3);
    assert_eq!(for_host.in_progress.len(), 3);
    assert!(
        for_host
            .in_progress
            .iter()
            .any(|line| line.sender.is_none())
    );
}

#[test]
fn a_seat_is_told_which_machines_exist_and_run_and_nothing_of_their_specs_or_the_wiring() {
    let world = scene_world();
    let wired = [PortName::Broker];

    let registry = world.registry.inspect(seat_view(1));
    let ports = WiredPorts {
        registry: &world.registry,
        wired: &wired,
    }
    .inspect(seat_view(1));

    assert_eq!(registry.machines.len(), 3);
    for line in &registry.machines {
        assert_eq!(line.enablement, "enabled");
        assert_eq!(line.spec, Visible::Redacted { needs: Needs::Host });
    }
    assert!(
        registry
            .machines
            .iter()
            .any(|line| line.scope == Scope::Group)
    );
    assert_eq!(ports.ports, Visible::Redacted { needs: Needs::Host });
    assert!(!everything(&registry).contains("rules_owner"));
}

// The pipeline table as a whole.

fn strike_world_with_a_clarification() -> crate::golden::World {
    let rules = strike_rules()
        .with_choices(seat(1), vec![action("strike")])
        .with_choices(seat(2), vec![action("strike"), action("sentinel-option")]);
    let mut world = world_of(&["strike", "sentinel-option"], 2, rules);
    world.interpreter = ScriptedInterpreter::default().with_line(
        "which one",
        Interpretation::Ambiguous {
            candidates: vec![action("strike"), action("sentinel-option")],
        },
    );
    world.dice = CountingDice::new([19]);
    world.tap(1, "strike").expect("the strike runs");
    world.text(2, "which one").expect("the line runs");
    world
}

#[test]
fn the_pipeline_report_gives_a_seat_only_its_own_clarification_pending_and_projection() {
    let world = strike_world_with_a_clarification();

    let for_two = world.table.inspect(seat_view(2));
    let for_one = world.table.inspect(seat_view(1));
    let for_host = world.table.inspect(host());

    assert_eq!(for_two.clarifications.len(), 1);
    assert_eq!(for_two.clarifications[0].seat, seat(2));
    assert!(for_one.clarifications.is_empty());
    assert!(!everything(&for_one).contains("sentinel-option"));
    assert_eq!(for_host.clarifications.len(), 1);
    assert!(for_two.pending.pendings.is_empty(), "the roll is seat 1's");
    assert_eq!(for_one.pending.pendings.len(), 1);
    let viewers = |report: &crate::inspect::PipelineReport| -> Vec<Viewer> {
        report.projections.iter().map(|line| line.viewer).collect()
    };
    assert_eq!(
        viewers(&for_two),
        [Viewer::Shared, Viewer::Seat { seat: seat(2) }]
    );
    assert_eq!(viewers(&for_host).len(), 3);
}

#[test]
fn a_projection_line_hides_the_work_counter_and_what_belongs_to_another_viewer() {
    let world = strike_world_with_a_clarification();

    let for_two = world.table.inspect(seat_view(2));
    let for_host = world.table.inspect(host());
    let for_operator = world.table.inspect(operator());

    let standing = |report: &crate::inspect::PipelineReport, viewer: Viewer| {
        report
            .projections
            .iter()
            .find(|line| line.viewer == viewer)
            .map(|line| line.standing.clone())
            .expect("the viewer has a line")
    };
    let built = |standing: ProjectionStanding| match standing {
        ProjectionStanding::Built { detail, work, .. } => (detail, work),
        ProjectionStanding::Refused { .. } => panic!("the seat is at the table"),
    };
    let own = Viewer::Seat { seat: seat(2) };
    let other = Viewer::Seat { seat: seat(1) };

    let (own_detail, own_work) = built(standing(&for_two, own));
    let (shared_detail, _) = built(standing(&for_two, Viewer::Shared));
    let (host_sees_other, _) = built(standing(&for_host, other));
    let (operator_detail, operator_work) = built(standing(&for_operator, other));

    assert!(own_detail.is_shown() && shared_detail.is_shown());
    assert_eq!(
        own_work,
        Visible::Redacted {
            needs: Needs::Operator
        }
    );
    assert_eq!(
        host_sees_other,
        Visible::Redacted {
            needs: Needs::OwnSeat
        }
    );
    assert!(operator_detail.is_shown() && operator_work.is_shown());
    assert_eq!(
        for_two.state_hash,
        Visible::Redacted {
            needs: Needs::Operator
        }
    );
    assert_eq!(for_host.state_hash, for_two.state_hash);
    assert_eq!(
        for_operator.state_hash.shown(),
        Some(&world.table.state_hash())
    );
}

#[test]
fn a_hidden_creature_changes_nothing_a_seat_is_told_in_the_pipeline_report() {
    use crate::pipeline::PipelineTable;
    use crate::testkit::correlation;
    let build = |with_ogre: bool| {
        let facts = table_of_three(with_ogre);
        let mut table = PipelineTable::new(crate::golden::TABLE, facts);
        table
            .stamp_commit_revision(correlation(5))
            .expect("the commit stamps");
        table
    };
    let with_ogre = build(true);
    let without = build(false);

    for audience in audiences() {
        assert_eq!(
            with_ogre.inspect(view_of(audience)),
            without.inspect(view_of(audience)),
            "{audience:?}"
        );
    }
    assert_ne!(with_ogre.inspect(operator()), without.inspect(operator()));
}
