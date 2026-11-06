use super::*;

#[test]
fn scan_barriers_refuse_steps_reach_and_line_of_effect_in_both_directions() {
    let bytes = br#"{"schema":2,"grid":{"cols":4,"rows":4,"blocked":[],"blocked_steps":[[[1,1],[2,1]]]},"layouts":[{"id":"scan","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,0]],"enemies":[[0,3]]}}]}"#;
    let map = BattleMap::from_json(bytes, "scan").unwrap();
    assert!(!map.step_clear(Cell(1, 1), Cell(2, 1)));
    assert!(!map.step_clear(Cell(2, 1), Cell(1, 1)));
    assert_eq!(
        map.cover_between(Cell(0, 1), Cell(3, 1), &[]),
        Ok(Cover::Total)
    );
    assert_eq!(
        map.cover_between(Cell(3, 1), Cell(0, 1), &[]),
        Ok(Cover::Total)
    );
    assert_eq!(
        map.cover_between(Cell(0, 0), Cell(3, 0), &[]),
        Ok(Cover::None)
    );
    assert_eq!(
        map.cover_between(Cell(0, 1), Cell(3, 1), &[Cell(99, 99)]),
        Err(RuleError::OutOfBounds)
    );
}
fn scan(steps: &str) -> BattleMap {
    let bytes = format!(
        r#"{{"schema":2,"grid":{{"cols":6,"rows":6,"blocked":[],"blocked_steps":{steps}}},"layouts":[{{"id":"scan","obstacles":[],"difficult":[],"spawns":{{"heroes":[[0,0]],"enemies":[[5,5]]}}}}]}}"#
    );
    BattleMap::from_json(bytes.as_bytes(), "scan").unwrap()
}

#[test]
fn a_blocked_diagonal_step_stops_a_body_and_no_line() {
    let map = scan("[[[1,1],[2,2]]]");
    assert!(!map.step_clear(Cell(1, 1), Cell(2, 2)));
    for (from, to) in [
        (Cell(1, 1), Cell(2, 2)),
        (Cell(0, 2), Cell(1, 2)),
        (Cell(2, 1), Cell(3, 1)),
        (Cell(0, 0), Cell(4, 4)),
    ] {
        assert_eq!(map.cover_between(from, to, &[]), Ok(Cover::None));
    }
}

#[test]
fn a_line_past_the_end_of_a_wall_is_clear_and_a_line_through_its_joint_or_corner_is_not() {
    // A wall along the top edge of row 2, under columns 0 to 2: it ends at the corner
    // shared by cells (2,1), (3,1), (2,2) and (3,2).
    let wall = scan("[[[0,1],[0,2]],[[1,1],[1,2]],[[2,1],[2,2]]]");
    // Through the inside of a piece, and through the joint between two pieces.
    assert_eq!(
        wall.cover_between(Cell(1, 3), Cell(1, 0), &[]),
        Ok(Cover::Total)
    );
    assert_eq!(
        wall.cover_between(Cell(0, 2), Cell(1, 1), &[]),
        Ok(Cover::Total)
    );
    assert_eq!(
        wall.cover_between(Cell(1, 1), Cell(0, 2), &[]),
        Ok(Cover::Total)
    );
    // Past the wall's end, touching its last corner: the weasel in the open can be shot.
    assert_eq!(
        wall.cover_between(Cell(2, 2), Cell(3, 1), &[]),
        Ok(Cover::None)
    );
    assert_eq!(
        wall.cover_between(Cell(1, 3), Cell(4, 0), &[]),
        Ok(Cover::None)
    );
    assert_eq!(
        wall.cover_between(Cell(4, 0), Cell(1, 3), &[]),
        Ok(Cover::None)
    );
    // Clear of it altogether.
    assert_eq!(
        wall.cover_between(Cell(3, 3), Cell(3, 0), &[]),
        Ok(Cover::None)
    );
    // The same wall turning a corner downward at its end: the line through that corner now
    // has wall on both sides of it and is stopped; a line outside the corner is not.
    let corner = scan("[[[0,1],[0,2]],[[1,1],[1,2]],[[2,1],[2,2]],[[2,2],[3,2]]]");
    assert_eq!(
        corner.cover_between(Cell(2, 2), Cell(3, 1), &[]),
        Ok(Cover::Total)
    );
    assert_eq!(
        corner.cover_between(Cell(3, 2), Cell(2, 1), &[]),
        Ok(Cover::None)
    );
}

#[test]
fn scan_barrier_heights_grade_cover_against_medium_height_srd_p15() {
    let map_at = |height_dm: Option<u16>| {
        let mut data = serde_json::json!({
            "schema": 2,
            "grid": {
                "cols": 4,
                "rows": 4,
                "blocked": [],
                "blocked_steps": [[[1, 1], [2, 1]]]
            },
            "layouts": [{
                "id": "scan",
                "obstacles": [],
                "difficult": [],
                "spawns": {"heroes": [[0, 0]], "enemies": [[0, 3]]}
            }]
        });
        if let Some(height_dm) = height_dm {
            data["grid"]["step_heights_dm"] = serde_json::json!([height_dm]);
        }
        BattleMap::from_json(data.to_string().as_bytes(), "scan").unwrap()
    };

    for (height_dm, expected) in [
        (Some(4), Cover::None),
        (Some(5), Cover::None),
        (Some(9), Cover::Half),
        (Some(10), Cover::Half),
        (Some(13), Cover::Half),
        (Some(14), Cover::ThreeQuarters),
        (Some(18), Cover::Total),
        (None, Cover::Total),
    ] {
        assert_eq!(
            map_at(height_dm).cover_between(Cell(0, 1), Cell(3, 1), &[]),
            Ok(expected),
            "height {height_dm:?} dm"
        );
    }
}

#[test]
fn scan_barrier_cover_scales_to_the_target_size_srd_p15() {
    let map = serde_json::from_value::<BattleSceneData>(serde_json::json!({
        "schema": 2,
        "grid": {
            "cols": 4,
            "rows": 4,
            "blocked": [],
            "blocked_steps": [[[1, 1], [2, 1]]],
            "step_heights_dm": [4]
        },
        "layouts": [{
            "id": "scan",
            "obstacles": [],
            "difficult": [],
            "spawns": {"heroes": [[0, 0]], "enemies": [[0, 3]]}
        }]
    }))
    .unwrap();
    let map = BattleMap::from_data(&map, "scan").unwrap();
    assert_eq!(
        map.cover_between_for_size(Cell(0, 1), Cell(3, 1), &[], Size::Medium),
        Ok(Cover::None)
    );
    assert_eq!(
        map.cover_between_for_size(Cell(0, 1), Cell(3, 1), &[], Size::Small),
        Ok(Cover::None)
    );
}

#[test]
fn obstacle_cover_uses_height_against_target_size_srd_521_p15() {
    let map = |height_m: f64| {
        let data = serde_json::json!({
            "schema": 2,
            "grid": {"cols": 5, "rows": 1, "blocked": []},
            "layouts": [{
                "id": "height",
                "obstacles": [{
                    "id": "bush",
                    "kind": "bush",
                    "cells": [[2, 0]],
                    "blocks_movement": false,
                    "cover": "total",
                    "height_m": height_m
                }],
                "difficult": [],
                "spawns": {"heroes": [[0, 0]], "enemies": [[4, 0]]}
            }]
        });
        BattleMap::from_json(data.to_string().as_bytes(), "height").unwrap()
    };
    assert_eq!(
        map(0.4).cover_between(Cell(0, 0), Cell(4, 0), &[]),
        Ok(Cover::None)
    );
    assert_eq!(
        map(1.0).cover_between(Cell(0, 0), Cell(4, 0), &[]),
        Ok(Cover::Half)
    );
    assert_eq!(
        map(2.0).cover_between(Cell(0, 0), Cell(4, 0), &[]),
        Ok(Cover::Total)
    );
}
