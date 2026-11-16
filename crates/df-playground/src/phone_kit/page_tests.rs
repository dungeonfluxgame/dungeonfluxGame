//! The page blocks' decisions: the fight's grid, the map's ways and the words of a tally.

use crate::phone_kit::page::{
    Grid, MapNode, MapWay, NodeState, Side, Square, Token, Wound, tally_text,
};
use crate::phone_kit::shell::PillTone;

fn grid() -> Grid {
    Grid {
        terrain: vec!["..#".to_owned(), "c~.".to_owned()],
        reach: vec![[0, 0], [1, 0]],
        path: vec![[1, 0], [2, 1]],
        tokens: vec![Token {
            at: [0, 1],
            name: "Rylan".to_owned(),
            portrait: None,
            side: Side::Me,
            wound: None,
        }],
        target: Some([2, 1]),
    }
}

#[test]
fn a_grid_reads_its_terrain_and_marks() {
    let grid = grid();
    assert_eq!(grid.size(), (3, 2));
    assert_eq!(grid.square(2, 0), Square::Wall);
    assert_eq!(grid.square(0, 1), Square::Cover);
    assert_eq!(grid.square(1, 1), Square::Difficult);
    assert_eq!(grid.square(9, 9), Square::Floor);
    assert!(grid.is_lit(1, 0));
    assert!(!grid.is_lit(2, 0));
    assert_eq!(grid.path_step(2, 1), Some(2));
    assert!(grid.is_target(2, 1));
    assert_eq!(
        grid.token_at(0, 1).map(|token| token.name.as_str()),
        Some("Rylan")
    );
    assert_eq!(grid.problem(), None);
}

#[test]
fn a_ragged_or_unknown_grid_says_what_is_wrong() {
    let mut ragged = grid();
    ragged.terrain.push("..".to_owned());
    assert!(
        ragged
            .problem()
            .is_some_and(|words| words.contains("row 3"))
    );
    let mut unknown = grid();
    unknown.terrain[0] = "..?".to_owned();
    assert!(unknown.problem().is_some_and(|words| words.contains('?')));
}

#[test]
fn a_way_needs_both_its_places() {
    let nodes = vec![
        MapNode {
            name: "Quay".to_owned(),
            x: 10,
            y: 20,
            plate: None,
            state: NodeState::Here,
        },
        MapNode {
            name: "Keep".to_owned(),
            x: 80,
            y: 70,
            plate: None,
            state: NodeState::Unvisited,
        },
    ];
    let way = |from: &str, to: &str| MapWay {
        from: from.to_owned(),
        to: to.to_owned(),
        blocked: false,
    };
    assert_eq!(way("Quay", "Keep").ends(&nodes), Some(([10, 20], [80, 70])));
    assert_eq!(way("Quay", "Nowhere").ends(&nodes), None);
}

#[test]
fn wounds_are_words_with_a_tone_and_the_tally_counts_in_words() {
    assert_eq!(Wound::NearDeath.word(), "Near death");
    assert_eq!(Wound::Unhurt.tone(), PillTone::Good);
    assert_eq!(tally_text(2, 1), "2 successes, 1 failure");
    assert_eq!(tally_text(0, 3), "0 successes, 3 failures");
    assert_eq!(tally_text(9, 9), "3 successes, 3 failures");
}
