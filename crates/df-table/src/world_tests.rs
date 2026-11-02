use std::collections::BTreeMap;

use crate::element::{
    Appearance, Degrees, ElementId, FieldName, FieldValue, KindName, Percent, Pivot, Position,
};
use crate::motion::{Motion, Speed};
use crate::world::{Fields, World};

fn id(text: &str) -> ElementId {
    ElementId::new(text).expect("valid id")
}

fn kind() -> KindName {
    KindName::new("character").expect("valid kind")
}

fn fields(name: &str) -> Fields {
    BTreeMap::from([(
        FieldName::new("name").expect("valid field"),
        FieldValue::Text(name.to_owned()),
    )])
}

fn world_with(names: &[&str]) -> World {
    let mut world = World::default();
    for name in names {
        world
            .spawn(id(name), kind(), Position::new(1.0, 2.0), fields(name))
            .expect("spawns");
    }
    world
}

fn travel_to(x: f32, y: f32, speed: f32) -> Motion {
    Motion {
        target: Position::new(x, y),
        speed: Speed::new(speed).expect("valid"),
    }
}

#[test]
fn spawning_makes_an_element_with_every_component_and_a_taken_name_is_refused() {
    let mut world = world_with(&["pip"]);
    let element = world.element(&id("pip")).expect("exists");
    assert_eq!(element.kind, kind());
    assert_eq!(element.position, Position::new(1.0, 2.0));
    assert_eq!(element.motion, None);
    assert_eq!(world.len(), 1);
    assert!(
        world
            .spawn(id("pip"), kind(), Position::CENTER, fields("again"))
            .is_none()
    );
    assert_eq!(world.len(), 1);
}

#[test]
fn elements_are_listed_in_name_order_whatever_the_order_they_were_made_in() {
    let world = world_with(&["vex", "anselm", "pip"]);
    let names: Vec<String> = world
        .elements()
        .map(|element| element.id.to_string())
        .collect();
    assert_eq!(names, vec!["anselm", "pip", "vex"]);
}

#[test]
fn despawning_removes_every_component_and_the_name_can_be_used_again() {
    let mut world = world_with(&["pip", "vex"]);
    world.start_travel(&id("pip"), travel_to(10.0, 10.0, 5.0));
    assert!(world.despawn(&id("pip")));
    assert!(!world.contains(&id("pip")));
    assert!(world.element(&id("pip")).is_none());
    assert!(!world.is_moving(), "its motion went with it");
    assert!(!world.despawn(&id("pip")));
    world
        .spawn(
            id("pip"),
            kind(),
            Position::new(9.0, 9.0),
            fields("New Pip"),
        )
        .expect("the name is free again");
    let element = world.element(&id("pip")).expect("exists");
    assert_eq!(element.position, Position::new(9.0, 9.0));
    assert_eq!(element.motion, None, "the new element inherits nothing");
}

#[test]
fn a_stale_travel_never_follows_a_name_to_a_new_element() {
    let mut world = world_with(&["pip"]);
    world.start_travel(&id("pip"), travel_to(50.0, 50.0, 10.0));
    world.despawn(&id("pip"));
    world
        .spawn(id("pip"), kind(), Position::new(1.0, 1.0), fields("Pip"))
        .expect("spawns");
    world.advance_motion(0.1);
    assert_eq!(
        world.element(&id("pip")).expect("exists").position,
        Position::new(1.0, 1.0)
    );
}

#[test]
fn clearing_removes_everything_and_says_how_many() {
    let mut world = world_with(&["a", "b", "c"]);
    world.start_travel(&id("a"), travel_to(5.0, 5.0, 5.0));
    assert_eq!(world.clear(), 3);
    assert!(world.is_empty() && !world.is_moving());
}

#[test]
fn placing_an_element_puts_it_there_and_ends_its_travel() {
    let mut world = world_with(&["pip"]);
    world.start_travel(&id("pip"), travel_to(50.0, 50.0, 10.0));
    assert!(world.place(&id("pip"), Position::new(7.0, 7.0)));
    let element = world.element(&id("pip")).expect("exists");
    assert_eq!(
        (element.position, element.motion),
        (Position::new(7.0, 7.0), None)
    );
    assert!(!world.place(&id("ghost"), Position::CENTER));
}

#[test]
fn travel_is_started_replaced_reported_and_stopped_by_name() {
    let mut world = world_with(&["pip"]);
    assert!(world.start_travel(&id("pip"), travel_to(10.0, 10.0, 5.0)));
    assert!(world.start_travel(&id("pip"), travel_to(20.0, 20.0, 8.0)));
    assert_eq!(
        world.element(&id("pip")).expect("exists").motion,
        Some(travel_to(20.0, 20.0, 8.0))
    );
    assert!(world.is_moving());
    assert_eq!(world.stop_travel(&id("pip")), Some(true));
    assert_eq!(
        world.stop_travel(&id("pip")),
        Some(false),
        "it was not travelling"
    );
    assert_eq!(world.stop_travel(&id("ghost")), None);
    assert!(!world.start_travel(&id("ghost"), travel_to(1.0, 1.0, 1.0)));
}

#[test]
fn advancing_motion_moves_travellers_and_names_the_arrivals_in_order() {
    let mut world = world_with(&["b", "a", "slow"]);
    world.start_travel(&id("b"), travel_to(1.5, 2.0, 100.0));
    world.start_travel(&id("a"), travel_to(1.0, 2.5, 100.0));
    world.start_travel(&id("slow"), travel_to(90.0, 2.0, 1.0));
    let travelled = world.advance_motion(0.1);
    assert_eq!(travelled.moved, 3);
    assert_eq!(travelled.arrived, vec![id("a"), id("b")]);
    assert!(world.is_moving(), "slow is still travelling");
    assert_eq!(world.element(&id("a")).expect("exists").motion, None);
}

#[test]
fn fields_can_be_changed_through_the_world_and_a_missing_element_has_none() {
    let mut world = world_with(&["pip"]);
    let name = FieldName::new("name").expect("valid");
    world
        .fields_mut(&id("pip"))
        .expect("has fields")
        .insert(name.clone(), FieldValue::Text("Pippin".into()));
    assert_eq!(
        world.element(&id("pip")).expect("exists").fields.get(&name),
        Some(&FieldValue::Text("Pippin".into()))
    );
    assert!(world.fields_mut(&id("ghost")).is_none());
    assert_eq!(world.kind(&id("pip")), Some(&kind()));
    assert_eq!(world.kind(&id("ghost")), None);
}

fn quarter_turn() -> Degrees {
    Degrees::new(90).expect("valid degrees")
}

#[test]
fn rotating_gives_the_element_a_rotation_and_a_pivot_that_the_snapshot_carries() {
    let mut world = world_with(&["pip", "vex"]);
    let hinge = Pivot {
        x: Percent::new(0).expect("valid percent"),
        y: Percent::new(50).expect("valid percent"),
    };
    assert!(world.rotate(&id("pip"), quarter_turn(), Some(hinge)));
    let turned = world.element(&id("pip")).expect("exists").appearance;
    assert_eq!((turned.rotation, turned.pivot), (quarter_turn(), hinge));
    assert_eq!(
        world.element(&id("vex")).expect("exists").appearance,
        Appearance::default()
    );
}

#[test]
fn rotating_an_unknown_element_changes_nothing() {
    let mut world = world_with(&["pip"]);
    assert!(!world.rotate(&id("ghost"), quarter_turn(), Some(Pivot::CENTER)));
    assert_eq!(world.rotation_count(), 0);
}

#[test]
fn despawning_and_clearing_remove_the_rotation_component() {
    let mut world = world_with(&["pip", "vex", "ash"]);
    for name in ["pip", "vex", "ash"] {
        world.rotate(&id(name), quarter_turn(), Some(Pivot::CENTER));
    }
    assert_eq!(world.rotation_count(), 3);
    assert!(world.despawn(&id("pip")));
    assert_eq!(world.rotation_count(), 2, "the component went with it");
    world.clear();
    assert_eq!(world.rotation_count(), 0);
}
