use crate::ecs::{Entities, Entity, Store};
use crate::element::Position;
use crate::motion::{
    MAXIMUM_SPEED, MAXIMUM_TICK_SECONDS, MINIMUM_SPEED, Motion, Speed, SpeedError, advance,
};

fn speed(value: f32) -> Speed {
    Speed::new(value).expect("a valid speed")
}

fn one_mover(
    from: Position,
    target: Position,
    units_per_second: f32,
) -> (Entity, Store<Position>, Store<Motion>) {
    let mut entities = Entities::default();
    let entity = entities.spawn();
    let mut positions = Store::default();
    let mut motions = Store::default();
    positions.insert(entity, from);
    motions.insert(
        entity,
        Motion {
            target,
            speed: speed(units_per_second),
        },
    );
    (entity, positions, motions)
}

#[test]
fn a_speed_inside_the_limits_is_accepted_and_anything_else_is_refused() {
    assert!(Speed::new(MINIMUM_SPEED).is_ok() && Speed::new(MAXIMUM_SPEED).is_ok());
    for bad in [
        0.0,
        -1.0,
        MINIMUM_SPEED / 2.0,
        MAXIMUM_SPEED + 1.0,
        f32::NAN,
        f32::INFINITY,
    ] {
        assert!(matches!(Speed::new(bad), Err(SpeedError(_))), "{bad}");
    }
    assert_eq!(speed(12.5).units_per_second(), 12.5);
}

#[test]
fn a_refused_speed_says_the_limits() {
    let message = Speed::new(0.0).expect_err("refused").to_string();
    assert!(
        message.contains("0.1") && message.contains("400"),
        "{message}"
    );
}

#[test]
fn a_tick_moves_an_entity_toward_its_target_by_speed_times_time() {
    let (entity, mut positions, mut motions) =
        one_mover(Position::new(0.0, 0.0), Position::new(30.0, 40.0), 10.0);
    let report = advance(&mut positions, &mut motions, 0.2);
    assert_eq!(report.moved, 1);
    assert!(report.arrived.is_empty());
    let at = *positions.get(entity).expect("has a position");
    // Two units along a 3-4-5 direction.
    assert!(
        (at.x - 1.2).abs() < 1e-5 && (at.y - 1.6).abs() < 1e-5,
        "{at:?}"
    );
    assert!(motions.contains(entity), "still travelling");
}

#[test]
fn an_entity_that_would_pass_its_target_stops_on_it_and_loses_its_motion() {
    let (entity, mut positions, mut motions) =
        one_mover(Position::new(0.0, 0.0), Position::new(1.0, 0.0), 100.0);
    let report = advance(&mut positions, &mut motions, 0.1);
    assert_eq!(report.arrived, vec![entity]);
    assert_eq!(report.moved, 1);
    assert_eq!(positions.get(entity), Some(&Position::new(1.0, 0.0)));
    assert!(!motions.contains(entity));
    let again = advance(&mut positions, &mut motions, 0.1);
    assert_eq!(
        (again.moved, again.arrived.len()),
        (0, 0),
        "nothing left to move"
    );
}

#[test]
fn an_entity_already_on_its_target_arrives_without_moving() {
    let (entity, mut positions, mut motions) =
        one_mover(Position::new(5.0, 5.0), Position::new(5.0, 5.0), 10.0);
    let report = advance(&mut positions, &mut motions, 0.1);
    assert_eq!((report.moved, report.arrived), (0, vec![entity]));
}

#[test]
fn a_long_gap_is_cut_so_a_stalled_host_cannot_teleport_things() {
    let (entity, mut positions, mut motions) =
        one_mover(Position::new(0.0, 0.0), Position::new(100.0, 0.0), 10.0);
    advance(&mut positions, &mut motions, 60.0);
    let at = positions.get(entity).expect("has a position").x;
    assert!((at - 10.0 * MAXIMUM_TICK_SECONDS).abs() < 1e-4, "{at}");
}

#[test]
fn a_tick_that_is_negative_or_not_a_number_moves_nothing() {
    for bad in [-1.0, 0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let (entity, mut positions, mut motions) =
            one_mover(Position::new(0.0, 0.0), Position::new(10.0, 0.0), 5.0);
        let report = advance(&mut positions, &mut motions, bad);
        assert_eq!(report.moved, 0, "{bad}");
        assert_eq!(
            positions.get(entity),
            Some(&Position::new(0.0, 0.0)),
            "{bad}"
        );
    }
}

#[test]
fn many_travellers_each_move_independently() {
    let mut entities = Entities::default();
    let mut positions = Store::default();
    let mut motions = Store::default();
    let mut all = Vec::new();
    for index in 0..50 {
        let entity = entities.spawn();
        positions.insert(entity, Position::new(0.0, 0.0));
        motions.insert(
            entity,
            Motion {
                target: Position::new(100.0, 0.0),
                speed: speed(10.0 + index as f32),
            },
        );
        all.push(entity);
    }
    let report = advance(&mut positions, &mut motions, 0.1);
    assert_eq!(report.moved, 50);
    for (index, entity) in all.iter().enumerate() {
        let expected = (10.0 + index as f32) * 0.1;
        let at = positions.get(*entity).expect("has a position").x;
        assert!((at - expected).abs() < 1e-4, "{index}: {at}");
    }
}
