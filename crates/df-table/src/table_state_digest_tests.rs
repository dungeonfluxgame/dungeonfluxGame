//! The digest walks every fact in a fixed order (#1682): equal tables give equal digests, and
//! every commit that changes the world moves it.

use crate::effects::{BeatName, TableEffect};
use crate::table_state::{
    CastChange, ConditionName, Presence, SplitPolicy, TableState, Visibility,
};
use crate::table_state_tests::{
    apply, creature, element, hero, machine, scene, seat, set_active, slot_with, table_in_scene_ten,
};

/// A table with a hero, a creature, an active machine and one frame on the stack, which allows a
/// split so a seat can move.
fn rich_table() -> TableState {
    let mut table = table_in_scene_ten().with_split_policy(SplitPolicy::Allowed);
    apply(
        &mut table,
        2,
        &[
            TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            TableEffect::CastEntered {
                entry: creature("goblin", "Goblin", 10, 60),
            },
            set_active(10, "explore"),
            TableEffect::HandOffPushed {
                scene: scene(10),
                machine: machine("dialogue"),
                slot: slot_with("dialogue", "talking"),
            },
        ],
    )
    .expect("the table is set");
    table
}

fn changes(effect: TableEffect) -> TableEffect {
    effect
}

/// One effect of every kind that changes the world, each valid against `rich_table`.
fn world_changing_effects() -> Vec<(&'static str, TableEffect)> {
    vec![
        ("a seat joins", TableEffect::SeatJoined { seat: seat(3) }),
        ("a seat leaves", TableEffect::SeatLeft { seat: seat(2) }),
        (
            "presence changes",
            TableEffect::SeatPresenceChanged {
                seat: seat(1),
                presence: Presence::VoluntaryAfk,
            },
        ),
        (
            "a seat moves alone",
            TableEffect::SeatMoved {
                seat: seat(2),
                scene: scene(11),
            },
        ),
        (
            "the party enters a scene",
            TableEffect::SceneEntered { scene: scene(11) },
        ),
        (
            "the same scene is entered again",
            TableEffect::SceneEntered { scene: scene(10) },
        ),
        (
            "a beat resolves",
            TableEffect::BeatResolved {
                scene: scene(10),
                beat: BeatName::new("locked_gate").unwrap(),
            },
        ),
        (
            "a hand-off is recorded",
            TableEffect::HandOffRecorded {
                from: machine("lobby"),
                to: machine("explore"),
                carried: 2,
            },
        ),
        ("time advances", TableEffect::TimeAdvanced { advance_s: 1 }),
        ("the table pauses", TableEffect::TablePaused),
        ("the table idles", TableEffect::TableIdled),
        ("the table resumes", TableEffect::TableResumed),
        (
            "a creature enters",
            TableEffect::CastEntered {
                entry: creature("ogre", "Ogre", 10, 40),
            },
        ),
        (
            "a creature leaves",
            TableEffect::CastLeft {
                cast: element("goblin"),
            },
        ),
        (
            "hit points change",
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::HitPoints { hit_points: 1 },
            },
        ),
        (
            "visibility changes",
            TableEffect::CastChanged {
                cast: element("goblin"),
                change: CastChange::Visibility {
                    visibility: Visibility::Hidden,
                },
            },
        ),
        (
            "a creature is placed elsewhere",
            TableEffect::CastChanged {
                cast: element("goblin"),
                change: CastChange::Placement {
                    placement: Some(crate::table_state_tests::place(1, 1)),
                },
            },
        ),
        (
            "a condition starts",
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::Conditions {
                    conditions: vec![ConditionName::new("prone").unwrap()],
                },
            },
        ),
        ("another machine is set", set_active(10, "lobby")),
        (
            "the table machine is set",
            TableEffect::ActiveMachineSet {
                home: crate::table_state::MachineHome::Table,
                machine: machine("lobby"),
                slot: slot_with("lobby", "gathering"),
            },
        ),
        (
            "the active machine's slot changes",
            TableEffect::ActiveSlotUpdated {
                home: crate::table_state::MachineHome::Scene { scene: scene(10) },
                slot: slot_with("dialogue", "done"),
            },
        ),
        (
            "another machine is handed to",
            TableEffect::HandOffPushed {
                scene: scene(10),
                machine: machine("shop"),
                slot: slot_with("shop", "browsing"),
            },
        ),
        (
            "a machine is handed back to",
            TableEffect::HandOffPopped { scene: scene(10) },
        ),
    ]
}

#[test]
fn every_effect_that_changes_the_world_moves_the_digest() {
    for (what, effect) in world_changing_effects() {
        let mut table = rich_table();
        if effect.kind() == crate::effects::TableEffectKind::TableResumed {
            apply(&mut table, 3, &[TableEffect::TablePaused]).expect("pauses first");
        }
        let before = table.digest();
        let facts_before = table.snapshot().digest();
        apply(&mut table, 4, &[changes(effect)]).unwrap_or_else(|error| panic!("{what}: {error}"));
        assert_ne!(
            table.digest(),
            before,
            "{what}: the table digest did not move"
        );
        assert_ne!(
            table.snapshot().digest(),
            facts_before,
            "{what}: the facts digest did not move"
        );
    }
}

#[test]
fn every_kind_of_effect_is_exercised_by_the_digest_test() {
    use crate::effects::TableEffectKind;
    let covered: Vec<TableEffectKind> = world_changing_effects()
        .iter()
        .map(|(_, effect)| effect.kind())
        .collect();
    for kind in TableEffectKind::ALL {
        let is_covered = covered.contains(kind);
        assert!(is_covered, "{} has no case in the digest test", kind.key());
    }
}

#[test]
fn two_tables_built_the_same_way_have_the_same_digest_in_every_walk() {
    let one = rich_table();
    let other = rich_table();
    assert_eq!(one.digest(), other.digest());
    assert_eq!(one.snapshot().digest(), other.snapshot().digest());
    assert_eq!(one.digest(), one.clone().digest());
    assert_eq!(one.digest(), one.digest());
}

#[test]
fn the_order_cast_entries_arrive_in_does_not_change_the_digest() {
    let build = |first: &str, second: &str| {
        let mut table = table_in_scene_ten();
        let entries = |name: &str| match name {
            "pip" => TableEffect::CastEntered {
                entry: hero("pip", "Pip", 1, 10, 30),
            },
            _ => TableEffect::CastEntered {
                entry: hero("vex", "Vex", 2, 10, 50),
            },
        };
        apply(&mut table, 2, &[entries(first), entries(second)]).unwrap();
        table
    };
    assert_eq!(build("pip", "vex").digest(), build("vex", "pip").digest());
}

#[test]
fn a_machine_slot_digests_by_its_content_and_not_by_the_order_of_its_keys() {
    let slot = |text: &str| {
        crate::machine_slot::MachineSlot::from_parts(
            machine("explore"),
            1,
            serde_json::from_str(text).unwrap(),
        )
    };
    let digest_of = |slot: crate::machine_slot::MachineSlot| {
        let mut table = table_in_scene_ten();
        apply(
            &mut table,
            2,
            &[TableEffect::ActiveMachineSet {
                home: crate::table_state::MachineHome::Scene { scene: scene(10) },
                machine: machine("explore"),
                slot,
            }],
        )
        .unwrap();
        table.digest()
    };
    let a = digest_of(slot(
        r#"{"phase":"awaiting_roll","pending":3,"list":[1,2]}"#,
    ));
    let b = digest_of(slot(
        r#"{"list":[1,2],"pending":3,"phase":"awaiting_roll"}"#,
    ));
    let c = digest_of(slot(
        r#"{"phase":"awaiting_roll","pending":4,"list":[1,2]}"#,
    ));
    let d = digest_of(slot(
        r#"{"phase":"awaiting_roll","pending":3,"list":[2,1]}"#,
    ));
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
}

#[test]
fn replaying_a_committed_operation_leaves_the_digest_alone() {
    let mut table = rich_table();
    let effects = [TableEffect::TimeAdvanced { advance_s: 6 }];
    apply(&mut table, 3, &effects).unwrap();
    let after = table.digest();
    apply(&mut table, 3, &effects).unwrap();
    assert_eq!(table.digest(), after);
}

#[test]
fn an_empty_commit_moves_the_revision_and_the_operation_memory_and_so_the_digest() {
    let mut table = rich_table();
    let before = table.digest();
    apply(&mut table, 3, &[]).unwrap();
    assert_ne!(table.digest(), before);
}

#[test]
fn the_operation_memory_is_part_of_the_table_digest_but_not_of_the_facts_digest() {
    let build = |remembered: usize| {
        let mut table = TableState::seated(2, &[seat(1)])
            .unwrap()
            .remembering_operations(remembered);
        apply(&mut table, 1, &[TableEffect::TableIdled]).unwrap();
        table
    };
    let remembers = build(10);
    let forgets = build(0);
    assert_eq!(remembers.snapshot().digest(), forgets.snapshot().digest());
    assert_ne!(remembers.digest(), forgets.digest());
}

#[test]
fn the_split_policy_and_the_seat_capacity_are_part_of_the_digest() {
    let declined = table_in_scene_ten();
    let allowed = table_in_scene_ten().with_split_policy(SplitPolicy::Allowed);
    assert_ne!(declined.digest(), allowed.digest());
    let small = TableState::seated(2, &[seat(1)]).unwrap();
    let large = TableState::seated(3, &[seat(1)]).unwrap();
    assert_ne!(small.digest(), large.digest());
}
