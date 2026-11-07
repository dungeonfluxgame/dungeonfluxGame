//! Tests of `design`.

use crate::movement::design::{
    DesignContext, DesignError, DesignTable, SceneDesigner, TableDesigner,
};
use crate::movement::types::{
    Depth, MoveIntent, MoveKind, PartyMember, Placement, SceneDesign, Side,
};

fn party() -> Vec<PartyMember> {
    vec![
        member("elf", "Sindra"),
        member("warrior", "Brann"),
        member("rogue", "Wynn"),
        member("cleric", "Oda"),
    ]
}

fn member(id: &str, display_name: &str) -> PartyMember {
    PartyMember {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
    }
}

fn intent(kind: MoveKind, who: &[&str], to: &str) -> MoveIntent {
    MoveIntent {
        kind,
        who: who.iter().map(|id| (*id).to_owned()).collect(),
        to: to.to_owned(),
        line: format!("we go to {to}"),
    }
}

fn design(intent: &MoveIntent) -> Result<SceneDesign, DesignError> {
    TableDesigner::new(DesignTable::builtin()).design(intent, &party(), &DesignContext::default())
}

fn placement(character: &str, depth: Depth, side: Side, doing: &str) -> Placement {
    Placement {
        character: character.to_owned(),
        depth,
        side,
        doing: doing.to_owned(),
    }
}

#[test]
fn elf_and_warrior_to_the_balcony_are_placed_and_the_rest_left_behind() {
    let scene = design(&intent(MoveKind::Walk, &["elf", "warrior"], "the balcony")).unwrap();

    assert_eq!(
        scene,
        SceneDesign {
            place: "the balcony".to_owned(),
            mood: "cool night air".to_owned(),
            elements: vec![
                "stone balustrade".to_owned(),
                "hanging lanterns".to_owned(),
                "the city beyond".to_owned(),
            ],
            placements: vec![
                placement("elf", Depth::Foreground, Side::Left, "steps out"),
                placement("warrior", Depth::Foreground, Side::Right, "steps out"),
            ],
            left_behind: vec!["rogue".to_owned(), "cleric".to_owned()],
        }
    );
}

#[test]
fn town_square_uses_the_square_kit() {
    let scene = design(&intent(MoveKind::Walk, &["elf"], "the town square")).unwrap();

    assert_eq!(scene.place, "the town square");
    assert_eq!(scene.mood, "open and busy");
    assert!(scene.elements.contains(&"a market well".to_owned()));
}

#[test]
fn hidden_door_matches_the_door_word() {
    let scene = design(&intent(MoveKind::Enter, &["rogue"], "the hidden door")).unwrap();

    assert_eq!(scene.mood, "close and expectant");
    assert!(scene.elements.contains(&"iron hinges".to_owned()));
}

#[test]
fn word_match_ignores_case_and_follows_the_order_of_the_words() {
    let scene = design(&intent(
        MoveKind::Walk,
        &["elf"],
        "The Balcony by the Tavern",
    ))
    .unwrap();

    assert_eq!(scene.mood, "cool night air");
}

#[test]
fn unmatched_destination_uses_the_fallback_kit() {
    let scene = design(&intent(MoveKind::Walk, &["elf"], "the moonlit meadow")).unwrap();

    assert_eq!(scene.mood, "even and unremarkable");
    assert_eq!(
        scene.elements,
        vec!["bare walls".to_owned(), "a plain floor".to_owned()]
    );
}

#[test]
fn depth_and_side_follow_the_order_of_the_movers() {
    let scene = design(&intent(
        MoveKind::Enter,
        &["elf", "warrior", "rogue", "cleric"],
        "the tavern",
    ))
    .unwrap();

    assert_eq!(
        scene.placements,
        vec![
            placement("elf", Depth::Foreground, Side::Left, "enters"),
            placement("warrior", Depth::Foreground, Side::Right, "enters"),
            placement("rogue", Depth::Middle, Side::Centre, "enters"),
            placement("cleric", Depth::Middle, Side::Left, "enters"),
        ]
    );
    assert!(scene.left_behind.is_empty());
}

#[test]
fn tone_replaces_the_mood_and_keeps_the_elements() {
    let context = DesignContext {
        tone: Some("tense and quiet".to_owned()),
        ..Default::default()
    };
    let scene = TableDesigner::new(DesignTable::builtin())
        .design(
            &intent(MoveKind::Sneak, &["rogue"], "the cellar"),
            &party(),
            &context,
        )
        .unwrap();

    assert_eq!(scene.mood, "tense and quiet");
    assert!(scene.elements.contains(&"barrels on a rack".to_owned()));
}

#[test]
fn time_of_day_and_weather_are_appended_to_the_mood() {
    let context = DesignContext {
        time_of_day: Some("late".to_owned()),
        weather: Some("light rain".to_owned()),
        ..Default::default()
    };
    let scene = TableDesigner::new(DesignTable::builtin())
        .design(
            &intent(MoveKind::Walk, &["elf"], "the balcony"),
            &party(),
            &context,
        )
        .unwrap();

    assert_eq!(scene.mood, "cool night air, late, light rain");
}

#[test]
fn the_same_move_gives_the_same_design() {
    let move_intent = intent(MoveKind::Run, &["warrior", "elf"], "the hall");

    assert_eq!(design(&move_intent).unwrap(), design(&move_intent).unwrap());
}

#[test]
fn a_move_with_no_movers_is_refused() {
    let error = design(&intent(MoveKind::Walk, &[], "the balcony")).unwrap_err();

    assert_eq!(error, DesignError::NoMovers);
    assert_eq!(error.key(), "no_movers");
}

#[test]
fn a_mover_outside_the_party_is_refused() {
    let error = design(&intent(MoveKind::Walk, &["elf", "ghost"], "the balcony")).unwrap_err();

    assert_eq!(
        error,
        DesignError::UnknownMover {
            id: "ghost".to_owned()
        }
    );
    assert_eq!(error.key(), "unknown_mover");
    assert_eq!(error.to_string(), "ghost is not in the party");
}

#[test]
fn a_blank_destination_is_refused() {
    let error = design(&intent(MoveKind::Walk, &["elf"], "   ")).unwrap_err();

    assert_eq!(error, DesignError::EmptyDestination);
    assert_eq!(error.key(), "empty_destination");
}

#[test]
fn a_design_table_loads_from_json() {
    let table = DesignTable::from_json_str(
        r#"{
            "kits": {
                "dock": { "mood": "salt and tar", "elements": ["a creaking jetty"] }
            },
            "fallback": { "mood": "plain", "elements": [] }
        }"#,
    )
    .unwrap();
    let scene = TableDesigner::new(table)
        .design(
            &intent(MoveKind::Walk, &["elf"], "the old dock"),
            &party(),
            &DesignContext::default(),
        )
        .unwrap();

    assert_eq!(scene.mood, "salt and tar");
    assert_eq!(scene.elements, vec!["a creaking jetty".to_owned()]);
}

#[test]
fn a_malformed_design_table_is_refused() {
    let malformed = DesignTable::from_json_str(r#"{ "kits": { "dock": "#).unwrap_err();
    assert!(matches!(malformed, DesignError::BadTable { .. }));
    assert_eq!(malformed.key(), "bad_table");

    let unknown_field = DesignTable::from_json_str(
        r#"{ "kits": {}, "fallback": { "mood": "plain", "elements": [] }, "extra": 1 }"#,
    )
    .unwrap_err();
    assert!(matches!(unknown_field, DesignError::BadTable { .. }));

    let capitalised_key = DesignTable::from_json_str(
        r#"{ "kits": { "Dock": { "mood": "salt", "elements": [] } },
            "fallback": { "mood": "plain", "elements": [] } }"#,
    )
    .unwrap_err();
    assert!(matches!(capitalised_key, DesignError::BadTable { .. }));
}
