//! Tests of `design`.

use crate::movement::design::{
    DesignError, ShownItemCause, WalkNeeds, check_feature, check_shown_items, check_walk,
    design_facts, design_schema, parse_design,
};
use crate::movement::types::{
    Depth, MoveIntent, MoveKind, PartyMember, Placement, Pose, Resident, RouteSight, SceneDesign,
    ShownItem, Side, TownRole, WalkAsk,
};
use crate::world::{InventoryItem, Town, Visited};
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
        "feature": "fruit_stalls",
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
            feature: "fruit_stalls".to_owned(),
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
            stages: vec![],
            town_look: None,
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
    let schema = design_schema(None);
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(
        schema["required"],
        json!([
            "place",
            "mood",
            "feature",
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
        &[],
        None,
        &[],
    );
    assert_eq!(facts["previously_designed"], json!([]));
    assert_eq!(
        facts["shown_item_choices"],
        json!([{"owner": "elf", "item": "easel"}])
    );
    assert_eq!(facts["inventory"][0]["items"][0]["id"], json!("easel"));
    assert_eq!(facts["move"]["to"], json!("the market"));
    assert_eq!(facts["party"][0], json!({"id": "elf"}));
    assert!(
        !facts.to_string().contains("Sindra"),
        "a name is not told to the design call"
    );
    assert_eq!(facts["context"], json!("on the balcony"));
    assert_eq!(facts["refused_before"], json!([]));

    let refused = DesignError::BadThing {
        id: "Well".to_owned(),
    };
    let second = design_facts(
        &intent(&["elf"], "the market"),
        &party(),
        None,
        &inventories,
        std::slice::from_ref(&refused),
        None,
        &[],
    );
    assert_eq!(
        second["refused_before"],
        json!([{"error": "bad_thing", "detail": refused.to_string()}])
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
            "names an owner who holds nothing in inventory",
        ),
        // A resident whose id is a party member's.
        (
            refused(&|arrival| arrival["residents"][0]["id"] = json!("painter")),
            "bad_resident",
            "a resident id is already taken",
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

fn sight(id: &str, text: &str, pose: &str) -> Value {
    json!({"id": id, "sight": text, "pose": pose})
}

fn walking_answer(stages: Value, town_look: Option<&str>) -> Value {
    let mut arrival = answer(vec![mover("painter", "centre")], vec![]);
    arrival["stages"] = stages;
    if let Some(look) = town_look {
        arrival["town_look"] = json!(look);
    }
    arrival
}

fn lone_painter() -> Vec<PartyMember> {
    vec![member("painter", "Danny")]
}

fn ask(stages: usize, town: TownRole) -> WalkAsk {
    WalkAsk {
        stages,
        town,
        move_id: "roam-01".to_owned(),
    }
}

fn town(look: &str) -> Town {
    Town {
        id: "town-roam-00".to_owned(),
        look: look.to_owned(),
        created_at: "roam-00".to_owned(),
    }
}

fn parsed_stages(stages: Value) -> Result<SceneDesign, DesignError> {
    parse_design(
        &walking_answer(stages, None),
        &intent(&["painter"], "the market"),
        &lone_painter(),
    )
}

#[test]
fn the_sights_on_the_way_are_read_in_order_with_their_poses() {
    let design = parsed_stages(json!([
        sight("fountain_square", " a stone fountain ", "walking"),
        sight("shop-row", "a row of shopfronts", "pausing")
    ]))
    .unwrap();

    assert_eq!(
        design.stages,
        vec![
            RouteSight {
                id: "fountain_square".to_owned(),
                sight: "a stone fountain".to_owned(),
                pose: Pose::Walking
            },
            RouteSight {
                id: "shop-row".to_owned(),
                sight: "a row of shopfronts".to_owned(),
                pose: Pose::Pausing
            }
        ]
    );
}

#[test]
fn an_answer_without_stages_has_none() {
    let plain = answer(vec![mover("painter", "centre")], vec![]);

    let design =
        parse_design(&plain, &intent(&["painter"], "the market"), &lone_painter()).unwrap();

    assert!(design.stages.is_empty());
    assert_eq!(design.town_look, None);
}

#[test]
fn a_stage_id_that_repeats_or_is_no_id_is_refused() {
    let repeated = parsed_stages(json!([
        sight("arch", "an archway", "walking"),
        sight("arch", "a bridge", "walking")
    ]))
    .unwrap_err();
    let no_id = parsed_stages(json!([sight("Old Arch!", "an archway", "walking")])).unwrap_err();
    let blank_id = parsed_stages(json!([sight("  ", "an archway", "walking")])).unwrap_err();

    assert_eq!(repeated.key(), "invalid_stage");
    assert_eq!(no_id.key(), "invalid_stage");
    assert_eq!(blank_id.key(), "malformed_design");
    assert!(
        !no_id.to_string().contains("Old Arch"),
        "the model's id is not in the words: {no_id}"
    );
}

#[test]
fn a_blank_sight_or_a_pose_outside_the_closed_words_is_refused() {
    let blank = parsed_stages(json!([sight("arch", "   ", "walking")])).unwrap_err();
    let run = parsed_stages(json!([sight("arch", "an archway", "running")])).unwrap_err();
    let not_a_list = parsed_stages(json!("an archway")).unwrap_err();
    let not_an_object = parsed_stages(json!(["an archway"])).unwrap_err();

    assert!(blank.to_string().contains("'sight' is blank"), "{blank}");
    assert!(
        run.to_string()
            .contains("'pose' is not one of the allowed words"),
        "{run}"
    );
    assert!(
        not_a_list.to_string().contains("'stages' is not a list"),
        "{not_a_list}"
    );
    assert!(
        not_an_object
            .to_string()
            .contains("a stage is not an object"),
        "{not_an_object}"
    );
}

#[test]
fn a_blank_town_look_is_refused() {
    let blank = parse_design(
        &walking_answer(json!([]), Some("  ")),
        &intent(&["painter"], "the market"),
        &lone_painter(),
    )
    .unwrap_err();

    assert!(
        blank.to_string().contains("'town_look' is blank"),
        "{blank}"
    );
}

#[test]
fn the_move_bounds_the_stages_by_what_it_asked_for() {
    let two = parsed_stages(json!([
        sight("one", "a well", "walking"),
        sight("two", "an arch", "walking")
    ]))
    .unwrap();
    let within = WalkNeeds::of(&ask(2, TownRole::Within), None);
    let fewer = WalkNeeds::of(&ask(3, TownRole::Within), None);
    let one = WalkNeeds::of(&ask(1, TownRole::Within), None);

    assert_eq!(check_walk(&two, Some(&within)), Ok(()));
    assert_eq!(check_walk(&two, Some(&fewer)), Ok(()));
    assert_eq!(
        check_walk(&two, Some(&one)),
        Err(DesignError::TooManyStages {
            given: 2,
            allowed: 1
        })
    );
    assert_eq!(
        check_walk(&two, None),
        Err(DesignError::TooManyStages {
            given: 2,
            allowed: 0
        }),
        "a move that asked for no walk takes no sights"
    );
    assert_eq!(check_walk(&parsed_stages(json!([])).unwrap(), None), Ok(()));
}

#[test]
fn a_move_that_seeds_a_town_needs_its_look_and_one_that_has_a_town_does_not() {
    let bare = parsed_stages(json!([])).unwrap();
    let seeding = WalkNeeds::of(&ask(2, TownRole::Seeds), None);
    let kept = town("red roofs");
    let within_town = WalkNeeds::of(&ask(2, TownRole::Seeds), Some(&kept));

    assert!(seeding.asks_town_look);
    assert_eq!(
        check_walk(&bare, Some(&seeding)),
        Err(DesignError::MissingTownLook)
    );
    assert!(!within_town.asks_town_look);
    assert_eq!(check_walk(&bare, Some(&within_town)), Ok(()));
}

#[test]
fn the_town_look_in_force_is_what_a_design_carries_unless_the_move_seeds_one() {
    let kept = town("red roofs");
    let mut given = parse_design(
        &walking_answer(json!([]), Some("grey roofs")),
        &intent(&["painter"], "the market"),
        &lone_painter(),
    )
    .unwrap();

    WalkNeeds::of(&ask(0, TownRole::Seeds), None).settle(&mut given);
    assert_eq!(given.town_look.as_deref(), Some("grey roofs"));

    WalkNeeds::of(&ask(0, TownRole::Within), Some(&kept)).settle(&mut given);
    assert_eq!(given.town_look.as_deref(), Some("red roofs"));

    WalkNeeds::of(&ask(0, TownRole::Seeds), Some(&kept)).settle(&mut given);
    assert_eq!(given.town_look.as_deref(), Some("red roofs"));

    WalkNeeds::of(&ask(0, TownRole::Leaves), Some(&kept)).settle(&mut given);
    assert_eq!(
        given.town_look, None,
        "leaving the town takes its look away"
    );

    WalkNeeds::of(&ask(0, TownRole::Within), None).settle(&mut given);
    assert_eq!(given.town_look, None);
}

#[test]
fn the_schema_asks_for_stages_and_a_town_look_only_when_the_move_does() {
    let plain = design_schema(None);
    let walking = design_schema(Some(&WalkNeeds::of(&ask(2, TownRole::Seeds), None)));
    let kept = town("x");
    let in_town = design_schema(Some(&WalkNeeds::of(&ask(2, TownRole::Within), Some(&kept))));
    let no_sights = design_schema(Some(&WalkNeeds::of(&ask(0, TownRole::Seeds), None)));

    assert!(plain["properties"].get("stages").is_none());
    assert!(plain["properties"].get("town_look").is_none());
    assert_eq!(walking["properties"]["stages"]["maxItems"], json!(2));
    assert_eq!(
        walking["properties"]["stages"]["items"]["required"],
        json!(["id", "sight", "pose"])
    );
    assert_eq!(
        walking["properties"]["stages"]["items"]["properties"]["pose"]["enum"],
        json!(["walking", "pausing"])
    );
    let required = walking["required"].as_array().unwrap();
    assert!(required.contains(&json!("stages")) && required.contains(&json!("town_look")));
    assert!(in_town["properties"].get("town_look").is_none());
    assert!(in_town["properties"].get("stages").is_some());
    assert!(no_sights["properties"].get("stages").is_none());
    assert!(no_sights["properties"].get("town_look").is_some());
}

#[test]
fn the_facts_tell_a_walking_move_how_many_sights_and_the_look_of_its_town() {
    let kept = town("red roofs");
    let needs = WalkNeeds::of(&ask(2, TownRole::Within), Some(&kept));

    let facts = design_facts(
        &intent(&["painter"], "the market"),
        &lone_painter(),
        None,
        &vec![],
        &[],
        Some(&needs),
        &[],
    );

    assert_eq!(
        facts["route"],
        json!({"stages_wanted": 2, "town": "red roofs"})
    );
    let plain = design_facts(
        &intent(&["painter"], "the market"),
        &lone_painter(),
        None,
        &vec![],
        &[],
        None,
        &[],
    );
    assert!(plain.get("route").is_none());
}

fn visited(name: &str, feature: &str) -> Visited {
    Visited {
        name: name.to_owned(),
        feature: feature.to_owned(),
    }
}

#[test]
fn a_design_names_the_one_feature_that_sets_the_place_apart_as_a_plain_id() {
    let designed = |feature: Value| {
        let mut arrival = answer(vec![mover("elf", "left")], vec![]);
        arrival["feature"] = feature;
        parse_design(&arrival, &intent(&["elf"], "the road"), &party())
    };
    assert_eq!(designed(json!(" ford ")).unwrap().feature, "ford");
    for bad in [
        json!("A Ford"),
        json!("the ford"),
        json!(""),
        json!("  "),
        json!(3),
    ] {
        let error = designed(bad).unwrap_err();
        assert_eq!(error.key(), "malformed_design", "{error}");
    }
    let mut missing = answer(vec![mover("elf", "left")], vec![]);
    missing.as_object_mut().unwrap().remove("feature");
    let error = parse_design(&missing, &intent(&["elf"], "the road"), &party()).unwrap_err();
    assert!(
        error.to_string().contains("'feature' is missing"),
        "{error}"
    );
}

#[test]
fn a_feature_a_place_already_has_is_refused_and_a_distinct_one_passes() {
    let kept = vec![
        visited("a bare country road", "open_road"),
        visited("a mill", "mill"),
    ];
    let design_with = |feature: &str| {
        let mut arrival = answer(vec![mover("elf", "left")], vec![]);
        arrival["feature"] = json!(feature);
        parse_design(&arrival, &intent(&["elf"], "the road"), &party()).unwrap()
    };
    for repeated in ["open_road", "mill"] {
        let error = check_feature(&design_with(repeated), &kept).unwrap_err();
        assert_eq!(error, DesignError::RepeatedFeature);
        assert_eq!(error.key(), "repeated_feature");
        assert!(
            !error.to_string().contains(repeated),
            "no model text in {error}"
        );
    }
    // An exact comparison of ids: a different id is a different feature.
    for distinct in ["ford", "open_road_2", "mill_race"] {
        let design = design_with(distinct);
        assert_eq!(check_feature(&design, &kept), Ok(()), "{distinct}");
    }
    assert_eq!(check_feature(&design_with("open_road"), &[]), Ok(()));
}

#[test]
fn the_facts_carry_the_places_already_designed_oldest_first() {
    let kept = vec![
        visited("a bare country road", "open_road"),
        visited("a mill", "mill"),
    ];
    let facts = design_facts(
        &intent(&["elf"], "the road"),
        &party(),
        None,
        &vec![],
        &[],
        None,
        &kept,
    );
    assert_eq!(
        facts["previously_designed"],
        json!([
            {"name": "a bare country road", "feature": "open_road"},
            {"name": "a mill", "feature": "mill"}
        ])
    );
}

#[test]
fn a_refused_shown_item_and_a_refused_resident_id_each_say_which_rule_they_broke() {
    let lone = vec![member("painter", "Danny")];
    let inventories = vec![("painter".to_owned(), vec![easel()])];
    let shown = |entries: Value| {
        let mut arrival = answer(vec![mover("painter", "left")], vec![vendor("fruit_vendor")]);
        arrival["shown_items"] = entries;
        let design = parse_design(&arrival, &intent(&["painter"], "the market"), &lone).unwrap();
        check_shown_items(&design, &inventories).unwrap_err()
    };
    // A resident invented in the same answer is no owner of anything.
    let resident_owner = shown(json!([{"owner": "fruit_vendor", "item": "easel"}]));
    assert!(matches!(
        resident_owner,
        DesignError::InvalidShownItem {
            cause: ShownItemCause::NoSuchOwner,
            ..
        }
    ));
    assert!(
        resident_owner
            .to_string()
            .contains("a resident invented in this answer holds none")
    );
    let not_held = shown(json!([{"owner": "painter", "item": "lute"}]));
    assert!(matches!(
        not_held,
        DesignError::InvalidShownItem {
            cause: ShownItemCause::NotHeld,
            ..
        }
    ));
    let twice = shown(json!([
        {"owner": "painter", "item": "easel"},
        {"owner": "painter", "item": "easel"}
    ]));
    assert!(matches!(
        twice,
        DesignError::InvalidShownItem {
            cause: ShownItemCause::Twice,
            ..
        }
    ));
    assert!(twice.to_string().contains("the same item twice"));

    let grammar = DesignError::BadResident {
        id: "fruit-vendor".to_owned(),
    };
    assert!(
        grammar
            .to_string()
            .contains("only lowercase letters, digits and underscores")
    );
    let taken = DesignError::BadResident {
        id: "painter".to_owned(),
    };
    assert!(taken.to_string().contains("already taken"));
}

#[test]
fn the_instruction_tells_the_rules_the_refusals_come_from() {
    use crate::movement::design::DESIGN_INSTRUCTION;
    for rule in [
        "copied exactly from shown_item_choices",
        "a resident you invent in this answer owns none",
        "only lowercase letters, digits and underscores (no hyphens, spaces or capitals)",
        "give no two residents one id",
        "holds every refusal of your earlier answers",
    ] {
        assert!(DESIGN_INSTRUCTION.contains(rule), "{rule}");
    }
}
