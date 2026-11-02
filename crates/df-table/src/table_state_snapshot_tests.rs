use crate::effects::TableEffect;
use crate::element::{CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS};
use crate::table_state::{
    CastChange, CastEntry, DisplayName, DisplayNameError, Placement, PlacementError, TableState,
    Vitals, VitalsError,
};
use crate::table_state_tests::{
    apply, creature, display_name, element, hero, place, scene, seat, table_in_scene_ten,
};

fn table_with_a_cast() -> TableState {
    let mut table = table_in_scene_ten();
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
        ],
    )
    .expect("the cast enters");
    table
}

#[test]
fn a_snapshot_is_unaffected_by_later_commits() {
    let mut table = table_with_a_cast();
    let snapshot = table.snapshot();
    let copy_of_what_it_showed = snapshot.clone();
    apply(
        &mut table,
        3,
        &[
            TableEffect::TimeAdvanced { advance_s: 600 },
            TableEffect::CastChanged {
                cast: element("pip"),
                change: CastChange::HitPoints { hit_points: 1 },
            },
            TableEffect::CastLeft {
                cast: element("goblin"),
            },
            TableEffect::SeatLeft { seat: seat(2) },
            TableEffect::TablePaused,
            TableEffect::SceneEntered { scene: scene(11) },
        ],
    )
    .expect("applies");
    assert_eq!(snapshot, copy_of_what_it_showed);
    assert_eq!(snapshot.world_time_s(), 0);
    assert_eq!(snapshot.seats().len(), 2);
    assert!(snapshot.scene(scene(10)).is_some());
    assert!(snapshot.cast_entry(&element("goblin")).is_some());
    assert_eq!(
        snapshot
            .cast_entry(&element("pip"))
            .unwrap()
            .vitals()
            .hit_points(),
        7
    );
    let now = table.snapshot();
    assert_ne!(snapshot, now);
    assert_eq!(now.world_time_s(), 600);
    assert!(now.cast_entry(&element("goblin")).is_none());
}

#[test]
fn two_snapshots_with_no_commit_between_them_are_the_same_facts() {
    let table = table_with_a_cast();
    assert_eq!(table.snapshot(), table.snapshot());
}

#[test]
fn a_commit_shares_every_collection_it_did_not_touch() {
    let mut table = table_with_a_cast();
    let before = table.snapshot();
    apply(&mut table, 3, &[TableEffect::TimeAdvanced { advance_s: 6 }]).expect("applies");
    let after = table.snapshot();
    assert!(before.shares_cast_with(&after));
    assert!(before.shares_scenes_with(&after));
    assert!(before.shares_seats_with(&after));
    apply(
        &mut table,
        4,
        &[TableEffect::CastChanged {
            cast: element("pip"),
            change: CastChange::HitPoints { hit_points: 2 },
        }],
    )
    .expect("applies");
    let changed = table.snapshot();
    assert!(
        !after.shares_cast_with(&changed),
        "the cast was copied for the write"
    );
    assert!(after.shares_scenes_with(&changed));
    assert!(after.shares_seats_with(&changed));
}

#[test]
fn taking_a_snapshot_copies_no_collection() {
    let table = table_with_a_cast();
    let first = table.snapshot();
    let second = table.snapshot();
    assert!(first.shares_cast_with(&second));
    assert!(first.shares_scenes_with(&second));
    assert!(first.shares_seats_with(&second));
}

#[test]
fn a_refused_commit_leaves_snapshots_and_sharing_alone() {
    let mut table = table_with_a_cast();
    let before = table.snapshot();
    let refused = apply(
        &mut table,
        3,
        &[
            TableEffect::CastLeft {
                cast: element("pip"),
            },
            TableEffect::SeatLeft { seat: seat(9) },
        ],
    );
    assert!(refused.is_err());
    let after = table.snapshot();
    assert_eq!(before, after);
    assert!(before.shares_cast_with(&after));
}

#[test]
fn the_heroes_are_the_party_entries_and_a_seat_finds_its_own() {
    let table = table_with_a_cast();
    let snapshot = table.snapshot();
    let heroes: Vec<_> = snapshot.heroes().map(|entry| entry.id().as_str()).collect();
    assert_eq!(heroes, ["pip"]);
    assert_eq!(snapshot.hero_of(seat(1)).unwrap().id().as_str(), "pip");
    assert!(snapshot.hero_of(seat(2)).is_none());
    assert_eq!(snapshot.cast_len(), 2);
    assert_eq!(snapshot.cast().count(), 2);
}

#[test]
fn a_display_name_is_checked_where_it_is_made_and_never_printed_in_debug() {
    assert_eq!(DisplayName::new("   "), Err(DisplayNameError::Blank));
    assert_eq!(
        DisplayName::new("Pip\nthe Bold"),
        Err(DisplayNameError::ControlCharacter)
    );
    assert_eq!(
        DisplayName::new(&"p".repeat(257)),
        Err(DisplayNameError::TooLong)
    );
    assert!(DisplayName::new(&"p".repeat(256)).is_ok());
    let name = display_name("Sentinel Pip");
    assert_eq!(name.as_str(), "Sentinel Pip");
    assert_eq!(format!("{name:?}"), "DisplayName(length=short)");
    let table = table_with_a_cast();
    assert!(!format!("{:?}", table.snapshot()).contains("Pip"));
    assert!(!format!("{table:?}").contains("Goblin"));
}

#[test]
fn hit_points_are_never_above_the_maximum() {
    assert_eq!(Vitals::new(10, 9), Err(VitalsError::AboveMaximum));
    assert!(Vitals::new(9, 9).is_ok());
    assert!(Vitals::new(0, 9).is_ok());
    let bad = r#"{"hit_points":10,"maximum_hit_points":9}"#;
    assert!(serde_json::from_str::<Vitals>(bad).is_err());
    let unknown = r#"{"hit_points":1,"maximum_hit_points":9,"extra":1}"#;
    assert!(serde_json::from_str::<Vitals>(unknown).is_err());
}

#[test]
fn a_placement_is_on_the_canvas_edges_included_and_exact() {
    assert_eq!(CANVAS_WIDTH_UNITS, 100.0);
    assert_eq!(CANVAS_HEIGHT_UNITS, 56.25);
    assert!(Placement::from_milli_units(0, 0).is_ok());
    assert!(Placement::from_milli_units(100_000, 56_250).is_ok());
    assert_eq!(
        Placement::from_milli_units(100_001, 0),
        Err(PlacementError::OffCanvas)
    );
    assert_eq!(
        Placement::from_milli_units(0, 56_251),
        Err(PlacementError::OffCanvas)
    );
    let spot = Placement::from_milli_units(12_500, 56_250).unwrap();
    assert_eq!((spot.x_units(), spot.y_units()), (12.5, 56.25));
    let off = r#"{"x_milli_units":100001,"y_milli_units":0}"#;
    assert!(serde_json::from_str::<Placement>(off).is_err());
}

#[test]
fn a_cast_entry_and_a_cast_change_round_trip_through_json() {
    let entry = hero("pip", "Pip", 1, 10, 30);
    let json = serde_json::to_string(&entry).expect("serializes");
    assert_eq!(
        serde_json::from_str::<CastEntry>(&json).expect("parses"),
        entry
    );
    for change in [
        CastChange::Visibility {
            visibility: crate::table_state::Visibility::Invisible,
        },
        CastChange::Placement {
            placement: Some(place(5, 5)),
        },
        CastChange::Placement { placement: None },
        CastChange::HitPoints { hit_points: 4 },
        CastChange::Conditions {
            conditions: Vec::new(),
        },
    ] {
        let json = serde_json::to_string(&change).expect("serializes");
        assert_eq!(
            serde_json::from_str::<CastChange>(&json).expect("parses"),
            change
        );
    }
    let unknown = r#"{"change":"hit_points","hit_points":1,"extra":true}"#;
    assert!(serde_json::from_str::<CastChange>(unknown).is_err());
}

#[test]
fn every_new_effect_round_trips_through_json_under_its_own_key() {
    use crate::effects::EffectRecord;
    use crate::table_state::Presence;
    let samples = [
        TableEffect::SeatPresenceChanged {
            seat: seat(1),
            presence: Presence::VoluntaryAfk,
        },
        TableEffect::SeatMoved {
            seat: seat(1),
            scene: scene(2),
        },
        TableEffect::TimeAdvanced { advance_s: 6 },
        TableEffect::TablePaused,
        TableEffect::TableResumed,
        TableEffect::TableIdled,
        TableEffect::CastEntered {
            entry: creature("goblin", "Goblin", 10, 60),
        },
        TableEffect::CastLeft {
            cast: element("goblin"),
        },
        TableEffect::CastChanged {
            cast: element("goblin"),
            change: CastChange::HitPoints { hit_points: 1 },
        },
        crate::table_state_tests::set_active(10, "explore"),
        TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Scene { scene: scene(10) },
            slot: crate::table_state_tests::slot_with("explore", "resolved"),
        },
        TableEffect::HandOffPopped { scene: scene(10) },
    ];
    let sample_count = samples.len();
    for effect in samples {
        let json = serde_json::to_value(&effect).expect("serializes");
        assert_eq!(json["effect"], effect.kind().key());
        assert_eq!(
            serde_json::from_value::<TableEffect>(json).expect("parses"),
            effect
        );
        let record = EffectRecord::stamp(&effect).expect("stamps");
        assert_eq!(record.open(), Ok(effect));
    }
    assert_eq!(sample_count, 12);
}
