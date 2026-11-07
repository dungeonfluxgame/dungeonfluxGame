//! Tests of `design`.

use crate::movement::design::{DesignError, design_facts, design_schema, parse_design};
use crate::movement::types::{
    Depth, MoveIntent, MoveKind, PartyMember, Placement, Resident, SceneDesign, Side,
};
use serde_json::{Value, json};

fn party() -> Vec<PartyMember> {
    vec![
        member("elf", "Sindra"),
        member("warrior", "Brann"),
        member("rogue", "Wynn"),
    ]
}

fn member(id: &str, display_name: &str) -> PartyMember {
    PartyMember {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
    }
}

fn intent(who: &[&str], to: &str) -> MoveIntent {
    MoveIntent {
        kind: MoveKind::Walk,
        who: who.iter().map(|id| (*id).to_owned()).collect(),
        to: to.to_owned(),
        line: format!("we go to {to}"),
    }
}

fn mover(character: &str, side: &str) -> Value {
    json!({"character": character, "depth": "foreground", "side": side, "doing": "walks in"})
}

fn vendor(id: &str) -> Value {
    json!({
        "id": id,
        "display_name": "Mara",
        "job": "fruit stand vendor",
        "look": "stout woman in her fifties, grey braid, green apron",
        "depth": "middle",
        "side": "right",
        "doing": "arranges pears"
    })
}

fn answer(placements: Vec<Value>, residents: Vec<Value>) -> Value {
    json!({
        "place": "a busy market street",
        "mood": "bright and noisy",
        "elements": ["striped awnings", "crates of fruit"],
        "residents": residents,
        "placements": placements
    })
}

#[test]
fn a_valid_design_with_a_resident_places_everyone_and_computes_left_behind() {
    let design = parse_design(
        &answer(
            vec![mover("warrior", "right"), mover("elf", "left")],
            vec![vendor("fruit_vendor")],
        ),
        &intent(&["elf", "warrior"], "the market"),
        &party(),
    )
    .unwrap();

    assert_eq!(
        design,
        SceneDesign {
            place: "a busy market street".to_owned(),
            mood: "bright and noisy".to_owned(),
            elements: vec!["striped awnings".to_owned(), "crates of fruit".to_owned()],
            residents: vec![Resident {
                id: "fruit_vendor".to_owned(),
                display_name: "Mara".to_owned(),
                job: "fruit stand vendor".to_owned(),
                look: "stout woman in her fifties, grey braid, green apron".to_owned(),
            }],
            placements: vec![
                Placement {
                    character: "elf".to_owned(),
                    depth: Depth::Foreground,
                    side: Side::Left,
                    doing: "walks in".to_owned(),
                },
                Placement {
                    character: "warrior".to_owned(),
                    depth: Depth::Foreground,
                    side: Side::Right,
                    doing: "walks in".to_owned(),
                },
                Placement {
                    character: "fruit_vendor".to_owned(),
                    depth: Depth::Middle,
                    side: Side::Right,
                    doing: "arranges pears".to_owned(),
                },
            ],
            left_behind: vec!["rogue".to_owned()],
        }
    );
}

#[test]
fn a_balcony_has_no_residents() {
    let design = parse_design(
        &answer(vec![mover("elf", "centre")], vec![]),
        &intent(&["elf"], "the balcony"),
        &party(),
    )
    .unwrap();

    assert!(design.residents.is_empty());
    assert_eq!(design.placements.len(), 1);
    assert_eq!(
        design.left_behind,
        vec!["warrior".to_owned(), "rogue".to_owned()]
    );
}

#[test]
fn an_unknown_mover_in_the_move_is_refused() {
    let error = parse_design(
        &answer(vec![mover("elf", "left")], vec![]),
        &intent(&["elf", "ghost"], "the market"),
        &party(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        DesignError::UnknownMover {
            id: "ghost".to_owned()
        }
    );
    assert_eq!(error.key(), "unknown_mover");
}

#[test]
fn a_resident_id_that_is_a_party_id_or_repeated_is_refused() {
    let in_party = parse_design(
        &answer(vec![mover("elf", "left")], vec![vendor("rogue")]),
        &intent(&["elf"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert_eq!(
        in_party,
        DesignError::BadResident {
            id: "rogue".to_owned()
        }
    );
    assert_eq!(in_party.key(), "bad_resident");

    let twice = parse_design(
        &answer(
            vec![mover("elf", "left")],
            vec![vendor("fruit_vendor"), vendor("fruit_vendor")],
        ),
        &intent(&["elf"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert!(matches!(twice, DesignError::BadResident { .. }));
}

#[test]
fn a_resident_id_outside_the_persona_id_grammar_is_refused() {
    for id in ["Fruit_Vendor", "fruit-vendor", "1vendor", "fruit vendor", "_vendor"] {
        let error = parse_design(
            &answer(vec![mover("elf", "left")], vec![vendor(id)]),
            &intent(&["elf"], "the market"),
            &party(),
        )
        .unwrap_err();
        assert_eq!(error, DesignError::BadResident { id: id.to_owned() });
    }
}

#[test]
fn a_mover_without_a_placement_or_placed_twice_is_refused() {
    let missing = parse_design(
        &answer(vec![mover("elf", "left")], vec![]),
        &intent(&["elf", "warrior"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert_eq!(missing.key(), "malformed_design");
    assert!(missing.to_string().contains("warrior"));

    let twice = parse_design(
        &answer(vec![mover("elf", "left"), mover("elf", "right")], vec![]),
        &intent(&["elf", "warrior"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert!(twice.to_string().contains("twice"));

    let stranger = parse_design(
        &answer(vec![mover("elf", "left"), mover("rogue", "right")], vec![]),
        &intent(&["elf", "warrior"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert!(stranger.to_string().contains("does not move"));
}

#[test]
fn a_word_outside_the_closed_enums_or_a_blank_text_is_refused() {
    let mut bad_depth = mover("elf", "left");
    bad_depth["depth"] = json!("far away");
    let error = parse_design(
        &answer(vec![bad_depth], vec![]),
        &intent(&["elf"], "the market"),
        &party(),
    )
    .unwrap_err();
    assert_eq!(error.key(), "malformed_design");

    let mut blank = answer(vec![mover("elf", "left")], vec![]);
    blank["place"] = json!("   ");
    let error = parse_design(&blank, &intent(&["elf"], "the market"), &party()).unwrap_err();
    assert!(error.to_string().contains("'place' is blank"));
}

#[test]
fn a_move_with_no_movers_or_no_destination_is_refused_before_the_answer_is_read() {
    let none = parse_design(&json!(7), &intent(&[], "the market"), &party()).unwrap_err();
    assert_eq!(none, DesignError::NoMovers);

    let blank = parse_design(&json!(7), &intent(&["elf"], "  "), &party()).unwrap_err();
    assert_eq!(blank, DesignError::EmptyDestination);

    let not_object = parse_design(&json!(7), &intent(&["elf"], "x"), &party()).unwrap_err();
    assert_eq!(not_object.key(), "malformed_design");
}

#[test]
fn the_schema_is_closed_and_the_facts_carry_the_move_and_the_party() {
    let schema = design_schema();
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(
        schema["required"],
        json!(["place", "mood", "elements", "residents", "placements"])
    );

    let facts = design_facts(&intent(&["elf"], "the market"), &party(), Some("on the balcony"));
    assert_eq!(facts["move"]["to"], json!("the market"));
    assert_eq!(facts["party"][0]["id"], json!("elf"));
    assert_eq!(facts["context"], json!("on the balcony"));
}
