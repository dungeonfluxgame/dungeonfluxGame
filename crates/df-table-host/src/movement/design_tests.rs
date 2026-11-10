//! Tests of `design`.

use crate::movement::design::{
    DesignError, check_shown_items, design_facts, design_schema, parse_design,
};
use crate::movement::types::{
    Depth, MoveIntent, MoveKind, PartyMember, Placement, Resident, SceneDesign, ShownItem, Side,
};
use crate::world::InventoryItem;
use df_content::world_bible::Thing;
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
        "things": [],
        "shown_items": [],
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
            things: vec![],
            shown_items: vec![],
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
fn a_lone_traveller_arrives_with_the_things_in_sight_worth_a_close_look() {
    let lone = vec![member("painter", "Danny")];
    let mut arrival = answer(vec![mover("painter", "centre")], vec![]);
    arrival["things"] = json!([
        {"id": "well", "name": "the old well", "look": " mossy stones, a rope worn pale "},
        {"id": "shrine-bowl", "name": "an offering bowl", "look": "chipped clay, three coins"}
    ]);
    let design = parse_design(&arrival, &intent(&["painter"], "the square"), &lone).unwrap();
    assert_eq!(
        design.things,
        vec![
            Thing {
                id: "well".to_owned(),
                name: "the old well".to_owned(),
                look: "mossy stones, a rope worn pale".to_owned(),
                picture: None,
            },
            Thing {
                id: "shrine-bowl".to_owned(),
                name: "an offering bowl".to_owned(),
                look: "chipped clay, three coins".to_owned(),
                picture: None,
            },
        ]
    );
    assert!(design.residents.is_empty() && design.left_behind.is_empty());
    assert_eq!(design.placements.len(), 1);
}

#[test]
fn a_thing_with_a_bad_or_repeated_id_or_no_look_is_refused() {
    let with_things = |things: Value| {
        let mut arrival = answer(vec![mover("elf", "left")], vec![]);
        arrival["things"] = things;
        parse_design(&arrival, &intent(&["elf"], "the square"), &party())
    };
    let well = |id: &str| json!({"id": id, "name": "the well", "look": "mossy stones"});
    for id in ["Well", "the well", ""] {
        let error = with_things(json!([well(id)])).unwrap_err();
        if id.is_empty() {
            assert_eq!(error.key(), "malformed_design");
        } else {
            assert_eq!(error, DesignError::BadThing { id: id.to_owned() });
            assert_eq!(error.key(), "bad_thing");
        }
    }
    assert_eq!(
        with_things(json!([well("well"), well("well")])).unwrap_err(),
        DesignError::BadThing {
            id: "well".to_owned()
        }
    );
    let blank = with_things(json!([{"id": "well", "name": "the well", "look": "  "}]));
    assert!(blank.unwrap_err().to_string().contains("'look' is blank"));
    let missing = with_things(json!(null)).unwrap_err();
    assert!(missing.to_string().contains("'things' is not a list"));
}

fn easel() -> InventoryItem {
    InventoryItem {
        id: "easel".to_owned(),
        name: "folding easel".to_owned(),
        look: "pale ash legs, brass wing nuts".to_owned(),
        usage: crate::world::ItemUse::Kept,
        count: 1,
    }
}

#[test]
fn the_items_a_plate_shows_are_ones_the_mover_holds_in_the_design_order() {
    let inventories = vec![("painter".to_owned(), vec![easel()])];
    let lone = vec![member("painter", "Danny")];
    let shows = |shown: Value| {
        let mut arrival = answer(vec![mover("painter", "left")], vec![]);
        arrival["shown_items"] = shown;
        let design = parse_design(&arrival, &intent(&["painter"], "the square"), &lone)?;
        check_shown_items(&design, &inventories).map(|()| design.shown_items)
    };
    assert_eq!(
        shows(json!([{"owner": "painter", "item": "easel"}])).unwrap(),
        vec![ShownItem {
            owner: "painter".to_owned(),
            item: "easel".to_owned()
        }]
    );
    let not_held = shows(json!([{"owner": "painter", "item": "lute"}])).unwrap_err();
    assert_eq!(not_held.key(), "invalid_shown_item");
    let twice = json!([
        {"owner": "painter", "item": "easel"},
        {"owner": "painter", "item": "easel"}
    ]);
    assert_eq!(shows(twice).unwrap_err().key(), "invalid_shown_item");
    assert_eq!(shows(json!([])).unwrap(), vec![]);
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
    for id in [
        "Fruit_Vendor",
        "fruit-vendor",
        "1vendor",
        "fruit vendor",
        "_vendor",
    ] {
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
        json!([
            "place",
            "mood",
            "elements",
            "things",
            "shown_items",
            "residents",
            "placements"
        ])
    );
    assert_eq!(
        schema["properties"]["things"]["items"]["required"],
        json!(["id", "name", "look"])
    );

    let inventories = vec![("elf".to_owned(), vec![easel()])];
    let facts = design_facts(
        &intent(&["elf"], "the market"),
        &party(),
        Some("on the balcony"),
        &inventories,
        None,
    );
    assert_eq!(facts["inventory"][0]["items"][0]["id"], json!("easel"));
    assert_eq!(facts["move"]["to"], json!("the market"));
    assert_eq!(facts["party"][0], json!({"id": "elf"}));
    assert!(
        !facts.to_string().contains("Sindra"),
        "a name is not told to the design call"
    );
    assert_eq!(facts["context"], json!("on the balcony"));
    assert_eq!(facts["refused_before"], Value::Null);

    let refused = DesignError::BadThing {
        id: "Well".to_owned(),
    };
    let second = design_facts(
        &intent(&["elf"], "the market"),
        &party(),
        None,
        &inventories,
        Some(&refused),
    );
    assert_eq!(
        second["refused_before"],
        json!({"error": "bad_thing", "detail": refused.to_string()})
    );
    assert!(
        !second.to_string().contains("Well"),
        "the model's own id is not sent back as text"
    );
}

#[test]
fn no_length_or_count_is_refused_only_a_wrong_type_or_a_blank() {
    let long = "a worn grey cobble ".repeat(60);
    let mut arrival = answer(vec![mover("elf", "left")], vec![]);
    arrival["elements"] = json!((0..30).map(|n| format!("stall {n}")).collect::<Vec<_>>());
    arrival["place"] = json!(long);
    arrival["residents"] = json!(
        (0..9)
            .map(|n| {
                let mut resident = vendor(&format!("vendor_{n}"));
                resident["look"] = json!(long);
                resident["doing"] = json!(long);
                resident
            })
            .collect::<Vec<_>>()
    );
    arrival["things"] = json!(
        (0..15)
            .map(|n| json!({"id": format!("thing_{n}"), "name": "a thing", "look": long}))
            .collect::<Vec<_>>()
    );
    let design =
        parse_design(&arrival, &intent(&["elf"], "the market"), &party()).expect("a long answer");
    assert_eq!(design.elements.len(), 30);
    assert_eq!(design.residents.len(), 9);
    assert_eq!(design.things.len(), 15);

    let spice_merchant = "spice_and_dried_herb_merchant_with_the_red_awning";
    let design = parse_design(
        &answer(vec![mover("elf", "left")], vec![vendor(spice_merchant)]),
        &intent(&["elf"], "the market"),
        &party(),
    )
    .expect("a long resident id is a persona id");
    assert_eq!(design.residents[0].id, spice_merchant);
}

#[test]
fn a_market_whose_thing_ids_are_a_travellers_item_ids_is_a_valid_design() {
    // A thing's id may not be a bible item's (the world refuses that when the place is made);
    // what a mover carries is another list, and Danny's painting ids and a market's goods meet.
    let lone = vec![member("painter", "Danny")];
    let mut arrival = answer(vec![mover("painter", "left")], vec![vendor("fruit_vendor")]);
    arrival["things"] = json!([
        {"id": "painting_01", "name": "a hung painting", "look": "oil on a board, a harbour"},
        {"id": "boards", "name": "stacked boards", "look": "primed pine"}
    ]);
    arrival["shown_items"] = json!([{"owner": "painter", "item": "painting_01"}]);
    let held = InventoryItem {
        id: "painting_01".to_owned(),
        ..easel()
    };
    let inventories = vec![("painter".to_owned(), vec![held])];
    let design = parse_design(&arrival, &intent(&["painter"], "the market"), &lone).unwrap();
    assert_eq!(check_shown_items(&design, &inventories), Ok(()));
    assert_eq!(design.things.len(), 2);
}

#[test]
fn each_likely_cause_of_a_refusal_has_its_own_key_and_a_message_without_the_models_text() {
    let lone = vec![member("painter", "Danny")];
    let inventories = vec![("painter".to_owned(), vec![easel()])];
    let refused = |change: &dyn Fn(&mut Value)| {
        let mut arrival = answer(vec![mover("painter", "left")], vec![vendor("fruit_vendor")]);
        change(&mut arrival);
        parse_design(&arrival, &intent(&["painter"], "the market"), &lone)
            .and_then(|design| check_shown_items(&design, &inventories).map(|()| design))
            .unwrap_err()
    };
    let cases: Vec<(DesignError, &str, &str)> = vec![
        // The tone of an id the instruction forbids: capitals and spaces.
        (
            refused(&|arrival| {
                arrival["things"] = json!([{"id": "Market Cross", "name": "n", "look": "l"}]);
            }),
            "bad_thing",
            "a thing id is not a place id",
        ),
        // Two things with one id.
        (
            refused(&|arrival| {
                let thing = json!({"id": "cross", "name": "n", "look": "l"});
                arrival["things"] = json!([thing.clone(), thing]);
            }),
            "bad_thing",
            "repeats another thing's",
        ),
        // An item named by its name, not its id; the owner by display name.
        (
            refused(&|arrival| {
                arrival["shown_items"] = json!([{"owner": "painter", "item": "folding easel"}]);
            }),
            "invalid_shown_item",
            "names an item its owner does not hold",
        ),
        (
            refused(&|arrival| {
                arrival["shown_items"] = json!([{"owner": "Danny", "item": "easel"}]);
            }),
            "invalid_shown_item",
            "names an item its owner does not hold",
        ),
        // A resident whose id is a party member's.
        (
            refused(&|arrival| arrival["residents"][0]["id"] = json!("painter")),
            "bad_resident",
            "a resident id is not a persona id",
        ),
        // A resident also listed among the placements, which hold only the movers.
        (
            refused(&|arrival| {
                arrival["placements"]
                    .as_array_mut()
                    .unwrap()
                    .push(mover("fruit_vendor", "right"));
            }),
            "malformed_design",
            "a placement names someone who does not move",
        ),
        // A mover left out of the placements.
        (
            refused(&|arrival| arrival["placements"] = json!([])),
            "malformed_design",
            "painter' moves but is not placed",
        ),
        // A field that is not there at all, and an empty list where text belongs.
        (
            refused(&|arrival| {
                arrival.as_object_mut().unwrap().remove("shown_items");
            }),
            "malformed_design",
            "'shown_items' is missing",
        ),
        (
            refused(&|arrival| arrival["elements"] = json!(["striped awnings", " "])),
            "malformed_design",
            "an element is not non-blank text",
        ),
    ];
    for (error, key, words) in cases {
        assert_eq!(error.key(), key, "{error}");
        assert!(error.to_string().contains(words), "{error}");
        for model_text in ["Market Cross", "folding easel", "Danny", "fruit_vendor"] {
            assert!(
                !error.to_string().contains(model_text),
                "{model_text} in {error}"
            );
        }
    }
}
