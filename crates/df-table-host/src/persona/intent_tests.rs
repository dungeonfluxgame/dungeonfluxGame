use serde_json::{Value, json};

use super::intent::{
    Approaching, DistanceSource, Intent, IntentBounds, IntentCommand, IntentKind, IntentRequest,
    IntentScene, IntentWay, ScenePerson, SceneThing, ThingState, VisitedPlace, intent_of,
    parse_intent, read_intent, scene_facts,
};
use crate::movement::types::{Direction, Distance, Pace, TargetDistance};

fn scene() -> IntentScene {
    IntentScene {
        place: Some("the market square".to_owned()),
        things: vec![
            SceneThing {
                id: "well".to_owned(),
                name: "stone well".to_owned(),
                look: Some("mossy rim".to_owned()),
            },
            SceneThing {
                id: "tower".to_owned(),
                name: "old watchtower".to_owned(),
                look: None,
            },
        ],
        people: vec![
            ScenePerson {
                id: "baker".to_owned(),
                name: "The baker".to_owned(),
                sells: true,
            },
            ScenePerson {
                id: "guard".to_owned(),
                name: "The gate guard".to_owned(),
                sells: false,
            },
        ],
        carries: vec![SceneThing {
            id: "spear".to_owned(),
            name: "Spear".to_owned(),
            look: Some("an ash shaft with an iron head".to_owned()),
        }],
    }
}

fn ways() -> Vec<IntentWay> {
    ["road", "town", "market"]
        .into_iter()
        .map(|id| IntentWay {
            id: id.to_owned(),
            meaning: format!("go to the {id}"),
        })
        .collect()
}

fn answer(fields: Value) -> Value {
    let mut answer = json!({
        "intent": "look", "target": null, "way": null, "direction": null, "toward": null,
        "distance": null, "candidates": [], "question": null, "because": "it says so"
    });
    for (key, value) in fields.as_object().expect("fields are an object") {
        answer[key] = value.clone();
    }
    answer
}

/// The answer read and checked against the scene, the ways and `distances`, or the refusal's
/// reason key.
fn judged(fields: Value, distances: &[TargetDistance]) -> Result<Intent, String> {
    judged_during(fields, distances, None)
}

/// The line every judged answer is read for, for what is said aloud in it.
const LINE: &str = "I wave and shout Hello, anyone home? and open the door";

/// [`judged`] while the approach `approaching` is under way.
fn judged_during(
    fields: Value,
    distances: &[TargetDistance],
    approaching: Option<&Approaching>,
) -> Result<Intent, String> {
    judged_having_been(fields, distances, approaching, &[])
}

/// [`judged_during`] by a character who has been to the places `visited`.
fn judged_having_been(
    fields: Value,
    distances: &[TargetDistance],
    approaching: Option<&Approaching>,
    visited: &[VisitedPlace],
) -> Result<Intent, String> {
    let ways = ways();
    let bounds = IntentBounds {
        line: LINE,
        ways: &ways,
        distances,
        approaching,
        visited,
    };
    let answer = read_intent(&answer(fields)).map_err(|_| "malformed".to_owned())?;
    intent_of(answer, &scene(), &bounds).map_err(|miss| miss.reason)
}

#[test]
fn a_move_back_goes_to_a_place_been_to_and_only_to_one() {
    // "I go back to the market": the market square the hero was at, shown again as it was left.
    let been = [
        VisitedPlace {
            id: "explore-1117-19".to_owned(),
            name: "a busy market square".to_owned(),
        },
        VisitedPlace {
            id: "roam-00-hero-11".to_owned(),
            name: "the road before the town gate".to_owned(),
        },
    ];
    let back = judged_having_been(
        json!({"intent": "move", "revisit": "explore-1117-19", "distance": "down_the_road"}),
        &[],
        None,
        &been,
    )
    .expect("a place been to stands");
    assert_eq!(back.revisit.as_deref(), Some("explore-1117-19"));
    assert_eq!(back.way, None);
    assert_eq!(
        back.distance, None,
        "a place gone back to is shown as it was left"
    );
    // A place not been to is refused, and so is a place with a way besides (one destination).
    assert_eq!(
        judged_having_been(
            json!({"intent": "move", "revisit": "explore-0000-99"}),
            &[],
            None,
            &been
        )
        .map(|intent| intent.kind),
        Err("unknown_place".to_owned())
    );
    assert_eq!(
        judged_having_been(
            json!({"intent": "move", "revisit": "explore-1117-19", "way": "market"}),
            &[],
            None,
            &been
        )
        .map(|intent| intent.kind),
        Err("move_unclear".to_owned())
    );
    // Only a move goes back: on a look it is left out.
    let look = judged_having_been(
        json!({"intent": "look", "target": "well", "revisit": "explore-1117-19"}),
        &[],
        None,
        &been,
    )
    .expect("a look stands");
    assert_eq!(look.revisit, None);
    assert!(
        look.dropped.contains(&"revisit".to_owned()),
        "{:?}",
        look.dropped
    );
}

#[test]
fn keep_walking_continues_the_approach_under_way_and_only_a_different_target_starts_anew() {
    let hills = Approaching {
        id: "tower".to_owned(),
        name: "old watchtower".to_owned(),
        steps: 1,
    };
    // E16: "keep walking", read as going on toward what the approach goes toward.
    let keep = judged_during(
        json!({"intent": "move", "continue": true, "toward": "the hills",
               "distance": "on_the_horizon"}),
        &[],
        Some(&hills),
    )
    .expect("a continuation stands");
    assert!(keep.continues);
    assert_eq!(keep.target.as_deref(), Some("tower"));
    assert!(keep.toward.is_none() && keep.direction.is_none());
    assert_eq!(keep.dropped, vec!["toward", "distance"]);
    // Naming the same target again is the same continuation.
    let same = judged_during(
        json!({"intent": "move", "continue": true, "target": "tower"}),
        &[],
        Some(&hills),
    )
    .expect("the same target continues");
    assert!(same.continues);
    // A clearly different target is a new move, not a continuation.
    assert_eq!(
        judged_during(
            json!({"intent": "move", "continue": true, "target": "well"}),
            &[],
            Some(&hills)
        ),
        Err("continue_conflict".to_owned())
    );
    let new_move = judged_during(
        json!({"intent": "move", "target": "well", "distance": "across_the_square"}),
        &[],
        Some(&hills),
    )
    .expect("a new target is a new move");
    assert!(!new_move.continues);
    // With no approach under way there is nothing to continue.
    assert_eq!(
        judged(json!({"intent": "move", "continue": true}), &[]),
        Err("no_approach".to_owned())
    );
    // On anything but a move, continue is left out.
    let look = judged_during(
        json!({"intent": "look", "continue": true}),
        &[],
        Some(&hills),
    )
    .expect("a look stands");
    assert!(!look.continues && look.dropped == vec!["continue"]);
}

#[test]
fn every_kind_is_accepted_with_the_fields_it_uses() {
    let cases = [
        (json!({"intent": "move", "way": "market"}), IntentKind::Move),
        (
            json!({"intent": "move", "target": "tower", "distance": "on_the_horizon"}),
            IntentKind::Move,
        ),
        (
            json!({"intent": "move", "direction": "left", "toward": "the hills",
                   "distance": "down_the_road"}),
            IntentKind::Move,
        ),
        (json!({"intent": "look"}), IntentKind::Look),
        (
            json!({"intent": "look", "target": "well"}),
            IntentKind::Look,
        ),
        (
            json!({"intent": "talk", "target": "guard"}),
            IntentKind::Talk,
        ),
        (json!({"intent": "talk"}), IntentKind::Talk),
        (
            json!({"intent": "trade", "target": "baker"}),
            IntentKind::Trade,
        ),
        (json!({"intent": "act", "target": "well"}), IntentKind::Act),
        (json!({"intent": "act"}), IntentKind::Act),
        (json!({"intent": "done"}), IntentKind::Done),
        (json!({"intent": "ask"}), IntentKind::Ask),
        (json!({"intent": "ask", "target": "baker"}), IntentKind::Ask),
        (
            json!({"intent": "look", "direction": "behind", "toward": "the road behind"}),
            IntentKind::Look,
        ),
        // Something named in sight that the scene has no id for yet: the picture is searched.
        (
            json!({"intent": "look", "toward": "the sign on the wall"}),
            IntentKind::Look,
        ),
        (
            json!({"intent": "move", "toward": "the sign", "distance": "across_the_square"}),
            IntentKind::Move,
        ),
        (
            json!({"intent": "clarify", "candidates": ["well", "tower"],
                   "question": "Do you mean the well or the tower?"}),
            IntentKind::Clarify,
        ),
        (
            json!({"intent": "clarify", "candidates": ["market", "guard"],
                   "question": "Do you head for the market, or ask the guard?"}),
            IntentKind::Clarify,
        ),
    ];
    for (fields, kind) in cases {
        let intent = judged(fields.clone(), &[]).unwrap_or_else(|why| panic!("{fields}: {why}"));
        assert_eq!(intent.kind, kind, "{fields}");
    }
    for kind in IntentKind::ALL {
        assert_eq!(json!(kind), json!(kind.key()));
        assert_eq!(IntentKind::from_key(kind.key()), Some(kind));
    }
}

#[test]
fn an_id_the_scene_does_not_have_is_refused() {
    for fields in [
        json!({"intent": "look", "target": "fountain"}),
        json!({"intent": "move", "target": "castle", "distance": "on_the_horizon"}),
        json!({"intent": "talk", "target": "innkeeper"}),
    ] {
        assert_eq!(judged(fields, &[]), Err("unknown_target".to_owned()));
    }
    // The speaker is no one to talk to.
    assert_eq!(
        judged(json!({"intent": "talk", "target": "hero_11"}), &[]),
        Err("unknown_target".to_owned())
    );
    // Talk is with a person, not a thing.
    assert_eq!(
        judged(json!({"intent": "talk", "target": "well"}), &[]),
        Err("not_a_person".to_owned())
    );
}

#[test]
fn a_way_outside_the_scenarios_ways_or_a_word_outside_the_enums_is_refused() {
    assert_eq!(
        judged(json!({"intent": "move", "way": "harbour"}), &[]),
        Err("unknown_way".to_owned())
    );
    assert_eq!(
        judged(json!({"intent": "wander"}), &[]),
        Err("malformed".to_owned())
    );
    assert_eq!(
        judged(
            json!({"intent": "move", "direction": "up", "toward": "the sky",
                   "distance": "on_the_horizon"}),
            &[]
        ),
        Err("malformed".to_owned())
    );
}

#[test]
fn a_field_the_kind_does_not_use_is_left_out_and_named_never_a_refusal() {
    for (fields, kind, dropped) in [
        (
            json!({"intent": "look", "way": "road"}),
            IntentKind::Look,
            vec!["way"],
        ),
        (
            json!({"intent": "talk", "target": "guard", "distance": "within_reach"}),
            IntentKind::Talk,
            vec!["distance"],
        ),
        (
            json!({"intent": "done", "target": "well"}),
            IntentKind::Done,
            vec!["target"],
        ),
        (
            json!({"intent": "act", "question": "Really?"}),
            IntentKind::Act,
            vec!["question"],
        ),
        (
            json!({"intent": "ask", "way": "market"}),
            IntentKind::Ask,
            vec!["way"],
        ),
        (
            json!({"intent": "ask", "direction": "left", "toward": "the hills"}),
            IntentKind::Ask,
            vec!["direction", "toward"],
        ),
        (
            json!({"intent": "look", "candidates": ["well", "tower"]}),
            IntentKind::Look,
            vec!["candidates"],
        ),
        // The answer host 33 lost (G24): a talk that also gave where the walk went.
        (
            json!({"intent": "talk", "target": "guard", "direction": "left", "toward": "x"}),
            IntentKind::Talk,
            vec!["direction", "toward"],
        ),
        (
            json!({"intent": "move", "way": "road", "distance": "down_the_road"}),
            IntentKind::Move,
            vec!["distance"],
        ),
        // A field left out is not checked: an unknown way on a look costs nothing.
        (
            json!({"intent": "look", "way": "harbour"}),
            IntentKind::Look,
            vec!["way"],
        ),
    ] {
        let intent = judged(fields.clone(), &[]).unwrap_or_else(|why| panic!("{fields}: {why}"));
        assert_eq!(intent.kind, kind, "{fields}");
        assert_eq!(intent.dropped, dropped, "{fields}");
        assert!(intent.way.is_none() || kind == IntentKind::Move, "{fields}");
    }
}

#[test]
fn a_line_that_does_more_than_one_thing_is_a_bounded_plan_checked_step_by_step() {
    // G24: thank the apple seller, wave goodbye, then wander off toward the church tower.
    let goodbye = judged(
        json!({"intent": "talk", "target": "baker", "then": [
            {"intent": "move", "target": null, "way": null, "direction": null,
             "toward": "the old church tower", "distance": "down_the_road",
             "candidates": [], "question": null, "because": "then walks off"}
        ]}),
        &[],
    )
    .expect("a talk then a move stands");
    assert_eq!(goodbye.kind, IntentKind::Talk);
    assert_eq!(goodbye.then.len(), 1);
    assert_eq!(goodbye.then[0].kind, IntentKind::Move);
    assert_eq!(
        goodbye.then[0].toward.as_deref(),
        Some("the old church tower")
    );
    let step = |intent: &str, extra: Value| {
        let mut step = answer(json!({"intent": intent}));
        for (key, value) in extra.as_object().expect("an object") {
            step[key] = value.clone();
        }
        step
    };
    for (plan, why) in [
        (
            json!({"intent": "talk", "target": "baker", "then": [
                step("act", json!({})), step("look", json!({})), step("act", json!({}))
            ]}),
            "unclear_plan",
        ),
        (
            json!({"intent": "talk", "target": "baker", "then": [
                step("clarify", json!({"candidates": ["well", "tower"], "question": "Which?"}))
            ]}),
            "unclear_plan",
        ),
        (
            json!({"intent": "done", "then": [step("act", json!({}))]}),
            "unclear_plan",
        ),
        // Each step is checked as an intent of its own.
        (
            json!({"intent": "talk", "target": "baker", "then": [
                step("trade", json!({"target": "guard"}))
            ]}),
            "not_a_seller",
        ),
    ] {
        assert_eq!(judged(plan.clone(), &[]), Err(why.to_owned()), "{plan}");
    }
}

#[test]
fn a_line_no_reading_stands_for_is_played_as_an_act_never_dropped() {
    let fallback = Intent::fallback();
    assert_eq!(fallback.kind, IntentKind::Act);
    assert!(fallback.fallback && fallback.then.is_empty() && fallback.target.is_none());
}

#[test]
fn a_move_names_one_destination() {
    for fields in [
        json!({"intent": "move"}),
        json!({"intent": "move", "way": "road", "target": "tower", "distance": "down_the_road"}),
        json!({"intent": "move", "direction": "ahead", "distance": "down_the_road"}),
        json!({"intent": "move", "target": "tower", "toward": "the tower",
               "distance": "down_the_road"}),
    ] {
        assert_eq!(judged(fields, &[]), Err("move_unclear".to_owned()));
    }
    assert_eq!(
        judged(json!({"intent": "move", "target": "tower"}), &[]),
        Err("missing_distance".to_owned())
    );
    assert_eq!(
        judged(json!({"intent": "trade", "target": "guard"}), &[]),
        Err("not_a_seller".to_owned())
    );
    assert_eq!(
        judged(
            json!({"intent": "clarify", "candidates": ["well", "tower"], "question": "  "}),
            &[]
        ),
        Err("missing_question".to_owned())
    );
    for (fields, why) in [
        (
            json!({"intent": "look", "direction": "behind"}),
            "look_unclear",
        ),
        (
            json!({"intent": "look", "target": "well", "toward": "the well"}),
            "look_unclear",
        ),
        (
            json!({"intent": "look", "target": "well", "direction": "left", "toward": "x"}),
            "look_unclear",
        ),
    ] {
        assert_eq!(judged(fields, &[]), Err(why.to_owned()));
    }
}

#[test]
fn a_clarify_without_two_real_candidates_is_refused() {
    let question = "Which sign do you mean?";
    for candidates in [json!([]), json!(["well"]), json!(["well", "well"])] {
        assert_eq!(
            judged(
                json!({"intent": "clarify", "candidates": candidates, "question": question}),
                &[]
            ),
            Err("no_conflict".to_owned()),
            "{candidates}"
        );
    }
    assert_eq!(
        judged(
            json!({"intent": "clarify", "candidates": ["sign", "well"], "question": question}),
            &[]
        ),
        Err("unknown_candidate".to_owned())
    );
    // The speaker is no reading of their own line.
    assert_eq!(
        judged(
            json!({"intent": "clarify", "candidates": ["hero_11", "well"], "question": question}),
            &[]
        ),
        Err("unknown_candidate".to_owned())
    );
    let judged_clarify = judged(
        json!({"intent": "clarify", "candidates": ["town", "ahead", "town"],
               "question": "Into town, or on along the road ahead?"}),
        &[],
    )
    .expect("a way and a direction conflict");
    assert_eq!(judged_clarify.candidates, vec!["town", "ahead"]);
}

#[test]
fn the_designs_distance_wins_over_the_judges() {
    let designed = [TargetDistance {
        id: "tower".to_owned(),
        distance: Distance::DownTheRoad,
    }];
    let intent = judged(
        json!({"intent": "move", "target": "tower", "distance": "within_reach"}),
        &designed,
    )
    .expect("a move toward the tower stands");
    assert_eq!(intent.distance, Some(Distance::DownTheRoad));
    assert_eq!(intent.distance_from, Some(DistanceSource::Design));
    let ahead = judged(
        json!({"intent": "move", "direction": "ahead", "toward": "the gate",
               "distance": "across_the_square"}),
        &designed,
    )
    .expect("a move ahead stands");
    assert_eq!(ahead.direction, Some(Direction::Ahead));
    assert_eq!(ahead.distance, Some(Distance::AcrossTheSquare));
    assert_eq!(ahead.distance_from, Some(DistanceSource::Judge));
}

#[test]
fn the_command_line_reads_ways_distances_and_the_scene_verb() {
    let words = |text: &[&str]| {
        text.iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let Ok(IntentCommand::Judge(request)) = parse_intent(&words(&[
        "way=market|Find the market",
        "distance=tower:on_the_horizon",
        "approaching=tower|old watchtower|2",
        "picked=tower",
        "visited=explore-1117-19|a busy market square",
        "hero_11",
        "I",
        "keep",
        "walking",
    ])) else {
        panic!("the judge command reads");
    };
    assert_eq!(request.visited[0].id, "explore-1117-19");
    assert_eq!(request.visited[0].name, "a busy market square");
    assert!(parse_intent(&words(&["visited=Bad Id|x", "hero_11", "hi"])).is_err());
    assert_eq!(request.line, "I keep walking");
    assert_eq!(request.ways[0].id, "market");
    assert_eq!(request.distances[0].distance, Distance::OnTheHorizon);
    assert_eq!(
        request.approaching,
        Some(Approaching {
            id: "tower".to_owned(),
            name: "old watchtower".to_owned(),
            steps: 2
        })
    );
    assert!(matches!(
        parse_intent(&words(&["scene", "hero_11"])),
        Ok(IntentCommand::Scene(_))
    ));
    assert!(parse_intent(&words(&["way=Market|x", "hero_11", "hi"])).is_err());
    assert!(parse_intent(&words(&["distance=tower:far", "hero_11", "hi"])).is_err());
    let refused: Result<IntentWay, _> =
        serde_json::from_value(json!({"id": "Bad Id", "meaning": "x"}));
    assert!(refused.is_err());
}

#[test]
fn a_named_thing_absent_from_the_scene_is_never_mapped_to_an_existing_id_it_is_searched() {
    // E17: "I walk over to the big rock right beside the path", while the tower is in the scene.
    // An id Jev does not assert is the thing named is left out, and the plate searched.
    for kind in ["move", "look"] {
        let searched = judged(
            json!({"intent": kind, "target": "tower", "named": "the big rock",
                   "distance": "within_reach"}),
            &[],
        )
        .expect("an unconfirmed target goes to the search");
        assert_eq!(searched.target, None, "{kind}");
        assert_eq!(searched.toward.as_deref(), Some("the big rock"), "{kind}");
        assert!(searched.dropped.contains(&"target".to_owned()), "{kind}");
    }
    // Nothing in the scene is the rock: no target, the words go on and the plate is searched.
    let search = judged(
        json!({"intent": "move", "named": "the big rock", "distance": "within_reach",
               "noun": "big rock", "named_id": "big_rock"}),
        &[],
    )
    .expect("an unknown named thing is searched for");
    assert!(search.target.is_none());
    assert_eq!(search.toward.as_deref(), Some("the big rock"));
    assert_eq!(search.named.as_deref(), Some("the big rock"));
    // What the picture search looks for, and the id it is added under once found.
    assert_eq!(search.noun.as_deref(), Some("big rock"));
    assert_eq!(search.named_id.as_deref(), Some("big_rock"));
    let look = judged(
        json!({"intent": "look", "named": "the sign hanging over the gate",
               "noun": "hanging signboard", "named_id": "hanging_signboard"}),
        &[],
    )
    .expect("a look at an unknown named thing is searched for");
    assert_eq!(
        look.toward.as_deref(),
        Some("the sign hanging over the gate")
    );
    assert_eq!(look.noun.as_deref(), Some("hanging signboard"));
    // A search goes by the judge's noun, else the line's own words, and takes a new id the scene
    // does not have: the judge's when it is one, else one made from the noun (host 45: three
    // refusals for a missing noun lost "I jump over the wall into the garden").
    for (fields, noun, id) in [
        (
            json!({"intent": "look", "named": "the sign"}),
            "the sign",
            "the_sign",
        ),
        (
            json!({"intent": "look", "named": "the sign", "noun": "well", "named_id": "well"}),
            "well",
            "well_2",
        ),
        (
            json!({"intent": "look", "named": "the sign", "noun": "sign", "named_id": "Sign"}),
            "sign",
            "sign",
        ),
    ] {
        let searched = judged(fields.clone(), &[]).expect("a search always has its noun and id");
        assert_eq!(searched.noun.as_deref(), Some(noun), "{fields}");
        assert_eq!(searched.named_id.as_deref(), Some(id), "{fields}");
    }
    // When Jev asserts the id is the very thing named, it stands.
    let well = judged(
        json!({"intent": "look", "target": "well", "named": "the old well",
               "target_is_named": true}),
        &[],
    )
    .expect("a confirmed target stands");
    assert_eq!(well.target.as_deref(), Some("well"));
    // A step of a plan is checked the same way.
    let step = judged(
        json!({"intent": "move", "named": "the big rock", "distance": "within_reach",
               "noun": "big rock", "named_id": "big_rock",
               "then": [answer(json!({"intent": "look", "target": "tower",
                                      "named": "the big rock"}))]}),
        &[],
    )
    .expect("a step with an unconfirmed target is searched too");
    assert_eq!(step.then[0].target, None);
}

#[test]
fn a_line_with_no_meaning_is_only_ever_a_clarify_even_during_an_approach() {
    let hills = Approaching {
        id: "tower".to_owned(),
        name: "old watchtower".to_owned(),
        steps: 2,
    };
    // "blargh flibber wumpus" during an approach: no move, talk, trade, act or done stands.
    for fields in [
        json!({"intent": "move", "continue": true, "meaningful": false}),
        json!({"intent": "move", "target": "tower", "distance": "on_the_horizon",
               "meaningful": false}),
        json!({"intent": "act", "meaningful": false}),
        json!({"intent": "talk", "target": "guard", "meaningful": false}),
        json!({"intent": "done", "meaningful": false}),
    ] {
        assert_eq!(
            judged_during(fields.clone(), &[], Some(&hills)).map(|intent| intent.kind),
            Err("not_meaningful".to_owned()),
            "{fields}"
        );
    }
    // Its clarify offers what the player could mean here.
    let asked = judged_during(
        json!({"intent": "clarify", "meaningful": false, "candidates": ["tower", "well"],
               "question": "Keep walking toward the watchtower, or look at the well?"}),
        &[],
        Some(&hills),
    )
    .expect("a clarify of a meaningless line stands");
    assert_eq!(asked.kind, IntentKind::Clarify);
    assert_eq!(asked.candidates, vec!["tower", "well"]);
}

#[test]
fn a_talk_or_a_trade_names_its_subject_and_its_target_is_the_person_addressed() {
    // Host 35, ask 10: "Good day. How much for one of those loaves of bread?" to the seller.
    let price = judged(
        json!({"intent": "trade", "target": "baker", "named": "a loaf of bread"}),
        &[],
    )
    .expect("a price question to a seller stands as a trade");
    assert_eq!(price.kind, IntentKind::Trade);
    assert_eq!(price.target.as_deref(), Some("baker"));
    assert_eq!(price.named.as_deref(), Some("a loaf of bread"));
    let topic = judged(
        json!({"intent": "talk", "target": "guard", "named": "the road east"}),
        &[],
    )
    .expect("a talk naming a topic stands");
    assert_eq!(topic.target.as_deref(), Some("guard"));
    // The addressee is still a person of the scene.
    assert_eq!(
        judged(
            json!({"intent": "trade", "target": "well", "named": "bread"}),
            &[]
        )
        .map(|intent| intent.kind),
        Err("not_a_person".to_owned())
    );
}

#[test]
fn two_steps_of_the_same_kind_to_the_same_one_are_one_step() {
    // G24: "thank her" and "wave goodbye" to the seller, then walk off: one talk, then the move.
    let farewell = judged(
        json!({"intent": "talk", "target": "baker", "then": [
            answer(json!({"intent": "talk", "target": "baker"})),
            answer(json!({"intent": "move", "target": "tower", "distance": "down_the_road"}))
        ]}),
        &[],
    )
    .expect("the farewell stands");
    assert_eq!(farewell.kind, IntentKind::Talk);
    assert_eq!(farewell.then.len(), 1);
    assert_eq!(farewell.then[0].kind, IntentKind::Move);
    // A talk to someone else is a step of its own.
    let two = judged(
        json!({"intent": "talk", "target": "baker", "then": [
            answer(json!({"intent": "talk", "target": "guard"}))
        ]}),
        &[],
    )
    .expect("two people, two talks");
    assert_eq!(two.then.len(), 1);
}

#[test]
fn a_talk_and_a_trade_to_the_same_person_are_one_turn_with_them() {
    // Host 37: "Good day. How much for two of those red apples?" read as a trade, then a talk, to
    // the vendor: one line said, one answer, the deal judge reading it once.
    let price = judged(
        json!({"intent": "trade", "target": "baker", "named": "two red apples", "then": [
            answer(json!({"intent": "talk", "target": "baker"}))
        ]}),
        &[],
    )
    .expect("the price question stands");
    assert_eq!((price.kind, price.then.len()), (IntentKind::Trade, 0));
    // A greeting then a price: the turn is a trade.
    let greeting = judged(
        json!({"intent": "talk", "target": "baker", "then": [
            answer(json!({"intent": "trade", "target": "baker"}))
        ]}),
        &[],
    )
    .expect("the greeting and the price stand");
    assert_eq!((greeting.kind, greeting.then.len()), (IntentKind::Trade, 0));
    // To two people, two turns.
    let two = judged(
        json!({"intent": "trade", "target": "baker", "then": [
            answer(json!({"intent": "talk", "target": "guard"}))
        ]}),
        &[],
    )
    .expect("two people stand");
    assert_eq!(two.then.len(), 1);
}

#[test]
fn a_line_that_commits_to_nothing_never_executes() {
    // "I don't go to the market", "maybe later I'll climb the wall", "what if I jumped?", a joke.
    for fields in [
        json!({"intent": "move", "way": "market", "committed": false}),
        json!({"intent": "act", "committed": false}),
        json!({"intent": "done", "committed": false}),
        json!({"intent": "look", "target": "well", "committed": false}),
        json!({"intent": "ask", "committed": false,
               "then": [answer(json!({"intent": "act"}))]}),
    ] {
        assert_eq!(
            judged(fields.clone(), &[]).map(|intent| intent.kind),
            Err("not_committed".to_owned()),
            "{fields}"
        );
    }
    // Answered by the game master, or asked about: never executed.
    for (fields, kind) in [
        (
            json!({"intent": "ask", "committed": false}),
            IntentKind::Ask,
        ),
        (
            json!({"intent": "clarify", "committed": false, "candidates": ["market", "road"],
                   "question": "Do you want to go to the market or not?"}),
            IntentKind::Clarify,
        ),
    ] {
        assert_eq!(judged(fields, &[]).map(|intent| intent.kind), Ok(kind));
    }
}

#[test]
fn speech_with_a_no_in_it_is_still_said_to_whom_it_is_said() {
    // "No, I won't pay that", to the seller: a trade, said, committed or not (B4).
    for (fields, kind) in [
        (
            json!({"intent": "trade", "target": "baker", "committed": false}),
            IntentKind::Trade,
        ),
        (
            json!({"intent": "talk", "target": "baker", "committed": false}),
            IntentKind::Talk,
        ),
    ] {
        assert_eq!(judged(fields, &[]).map(|intent| intent.kind), Ok(kind));
    }
}

#[test]
fn a_step_of_the_plan_the_line_does_not_commit_to_is_never_carried_out() {
    // "I walk to the well, and maybe later I'll climb down": the climb is no step of the plan.
    let well = json!({"intent": "move", "target": "well", "distance": "across_the_square"});
    let mut later = well.clone();
    later["then"] = json!([answer(json!({"intent": "act", "target": "well",
                                         "committed": false}))]);
    assert_eq!(
        judged(later, &[]).map(|intent| intent.kind),
        Err("not_committed".to_owned())
    );
    // Left out of then, the walk stands; a committed step, or speech, stays in it.
    assert_eq!(
        judged(well.clone(), &[]).map(|intent| intent.then.len()),
        Ok(0)
    );
    let mut now = well.clone();
    now["then"] = json!([answer(json!({"intent": "act", "target": "well"}))]);
    assert_eq!(judged(now, &[]).map(|intent| intent.then.len()), Ok(1));
    let mut said = well;
    said["then"] = json!([answer(json!({"intent": "talk", "target": "baker",
                                        "committed": false}))]);
    assert_eq!(judged(said, &[]).map(|intent| intent.then.len()), Ok(1));
}

#[test]
fn a_move_says_its_pace_and_nothing_else_does() {
    // "I run down the lane": a move at a run; a pace on a look is left out.
    let run = judged(
        json!({"intent": "move", "direction": "ahead", "toward": "the lane",
               "distance": "down_the_road", "pace": "run"}),
        &[],
    )
    .expect("a run stands");
    assert_eq!(run.pace, Some(Pace::Run));
    let look = judged(
        json!({"intent": "look", "target": "well", "pace": "sneak"}),
        &[],
    )
    .expect("a look stands");
    assert_eq!(look.pace, None);
    assert!(
        look.dropped.contains(&"pace".to_owned()),
        "{:?}",
        look.dropped
    );
    assert_eq!(
        judged(
            json!({"intent": "move", "target": "well", "pace": "gallop"}),
            &[]
        )
        .map(|intent| intent.kind),
        Err("malformed".to_owned())
    );
}

#[test]
fn an_act_or_an_ask_about_a_thing_not_yet_seen_searches_the_picture() {
    // "I search the base of the signpost", with no signpost in the scene: the picture is searched
    // for it before anyone says it is not there.
    let search = judged(
        json!({"intent": "act", "named": "the base of the signpost", "noun": "wooden signpost",
               "named_id": "wooden_signpost"}),
        &[],
    )
    .expect("an act naming an unseen thing stands");
    assert_eq!(search.noun.as_deref(), Some("wooden signpost"));
    assert_eq!(search.named_id.as_deref(), Some("wooden_signpost"));
    // Something carried is never searched for: no noun, no search, and no refusal for it.
    let eat = judged(
        json!({"intent": "act", "named": "a ration", "noun": null, "named_id": null}),
        &[],
    )
    .expect("an act on something carried stands");
    assert_eq!((eat.noun, eat.named_id), (None, None));
    // An ask about an unseen thing takes a new id, never one the scene has.
    let asked = judged(
        json!({"intent": "ask", "named": "the well", "noun": "stone well",
               "named_id": "well"}),
        &[],
    )
    .expect("an ask about an unseen thing stands");
    assert_eq!(asked.named_id.as_deref(), Some("stone_well"));
}

#[test]
fn an_act_names_whom_it_concerns_and_a_move_may_back_away() {
    // "While she turns to her crates, I slip a pear off her stall": the act concerns the baker,
    // who answers it; the speaker is never one it concerns.
    let theft = judged(
        json!({"intent": "act", "target": "well", "affects": "baker"}),
        &[],
    )
    .expect("an act on someone's things stands");
    assert_eq!(theft.affects.as_deref(), Some("baker"));
    assert_eq!(
        judged(json!({"intent": "act", "affects": "hero_11"}), &[]).map(|intent| intent.kind),
        Err("not_a_person".to_owned())
    );
    let talk = judged(
        json!({"intent": "talk", "target": "baker", "affects": "baker"}),
        &[],
    )
    .expect("a talk stands");
    assert_eq!(talk.affects, None, "only an act concerns someone");
    // "I walk backwards away from the gate, eyes on it": a move toward it, away.
    let back = judged(
        json!({"intent": "move", "target": "tower", "away": true, "distance": "across_the_square"}),
        &[],
    )
    .expect("a move away stands");
    assert!(back.away);
    let look = judged(
        json!({"intent": "look", "target": "well", "away": true}),
        &[],
    )
    .expect("a look stands");
    assert!(!look.away && look.dropped.contains(&"away".to_owned()));
}

#[test]
fn a_move_into_something_of_the_scene_enters_it_and_names_one_destination() {
    // "I step inside the cottage": its inside is a place of its own, never another approach step.
    let inside = judged(
        json!({"intent": "move", "enter": "tower", "distance": "within_reach"}),
        &[],
    )
    .expect("a move into a thing of the scene stands");
    assert_eq!(inside.enter.as_deref(), Some("tower"));
    assert_eq!(inside.distance, None);
    assert_eq!(
        judged(json!({"intent": "move", "enter": "baker"}), &[]).map(|intent| intent.kind),
        Err("unknown_target".to_owned())
    );
    assert_eq!(
        judged(
            json!({"intent": "move", "enter": "tower", "target": "well"}),
            &[]
        )
        .map(|intent| intent.kind),
        Err("move_unclear".to_owned())
    );
}

#[test]
fn a_step_may_depend_on_the_one_before_and_the_first_on_none() {
    // "I climb up onto the fence to see over the hedge": the look depends on the climb.
    let mut climb = json!({"intent": "act", "target": "well"});
    climb["then"] = json!([answer(
        json!({"intent": "look", "target": "tower", "depends": true})
    )]);
    let judged_climb = judged(climb, &[]).expect("a plan with a dependent step stands");
    assert!(judged_climb.then[0].depends);
    let first = judged(
        json!({"intent": "act", "target": "well", "depends": true}),
        &[],
    )
    .expect("an act stands");
    assert!(!first.depends && first.dropped.contains(&"depends".to_owned()));
}

#[test]
fn going_into_a_thing_the_scene_names_needs_no_search_noun() {
    // Host 45: "I open the timber door and step inside" fell back three times on missing_noun:
    // a move into a thing the scene has an id for searches nothing.
    let inside = judged(
        json!({"intent": "move", "enter": "tower", "named": "the timber door",
               "target_is_named": true, "noun": null, "named_id": null}),
        &[],
    )
    .expect("a move into a thing named by its id stands");
    assert_eq!(inside.enter.as_deref(), Some("tower"));
    assert_eq!((inside.noun, inside.named_id), (None, None));
}

#[test]
fn a_look_into_something_tells_its_inside_and_searches_nothing() {
    // Host 45: "I peek inside the cottage window" searched the picture outside for its inside.
    let peek = judged(json!({"intent": "look", "enter": "tower"}), &[])
        .expect("a look into a thing of the scene stands");
    assert_eq!(peek.enter.as_deref(), Some("tower"));
    assert_eq!((peek.toward, peek.noun), (None, None));
    assert_eq!(
        judged(
            json!({"intent": "look", "enter": "tower", "target": "well"}),
            &[]
        )
        .map(|intent| intent.kind),
        Err("look_unclear".to_owned())
    );
    let talk = judged(json!({"intent": "talk", "enter": "tower"}), &[]).expect("a talk stands");
    assert_eq!(talk.enter, None);
}

#[test]
fn a_line_with_no_readable_meaning_is_never_an_ask() {
    // Host 45: "hmm" came back as an ask the game master answered with a deed.
    assert_eq!(
        judged(json!({"intent": "ask", "meaningful": false}), &[]).map(|intent| intent.kind),
        Err("not_meaningful".to_owned())
    );
}

#[test]
fn going_into_something_the_judge_does_not_confirm_searches_for_it() {
    // Host 49: "I walk over to the nearest house and go inside" went into a fruit stall.
    let house = judged(
        json!({"intent": "move", "enter": "tower", "named": "the nearest house",
               "distance": "down_the_road"}),
        &[],
    )
    .expect("an unconfirmed thing gone into is searched for");
    assert_eq!(house.enter, None);
    assert_eq!(house.toward.as_deref(), Some("the nearest house"));
    assert!(house.dropped.contains(&"enter".to_owned()));
}

#[test]
fn only_the_words_spoken_are_said_and_a_deed_may_set_a_things_state_and_the_heros_pose() {
    // The owner: an action is never said aloud; only the words the line speaks are.
    let shout = judged(
        json!({"intent": "act", "spoken": "Hello, anyone home?"}),
        &[],
    )
    .expect("words of the line stand");
    assert_eq!(shout.spoken.as_deref(), Some("Hello, anyone home?"));
    assert_eq!(
        judged(json!({"intent": "act", "spoken": "I crouch"}), &[]).map(|intent| intent.kind),
        Err("unspoken_words".to_owned())
    );
    // Reported speech to someone may be rendered in the first person.
    let asked = judged(
        json!({"intent": "talk", "target": "baker", "spoken": "Where is the inn?"}),
        &[],
    )
    .expect("a talk may render what is said");
    assert_eq!(asked.spoken.as_deref(), Some("Where is the inn?"));
    // Opening a door leaves it open; only a thing of the scene has a state.
    let door = judged(
        json!({"intent": "act", "target": "tower", "sets": "open"}),
        &[],
    )
    .expect("an act on a thing sets its state");
    assert_eq!(door.sets, Some(ThingState::Open));
    assert_eq!(
        judged(
            json!({"intent": "act", "target": "baker", "sets": "open"}),
            &[]
        )
        .map(|intent| intent.kind),
        Err("no_thing_to_set".to_owned())
    );
    // A crouch changes the hero's stance; a look changes no pose.
    let crouch = judged(
        json!({"intent": "act", "pose": "crouched low by the crates"}),
        &[],
    )
    .expect("a pose stands");
    assert_eq!(crouch.pose.as_deref(), Some("crouched low by the crates"));
    let look = judged(
        json!({"intent": "look", "target": "well", "pose": "kneeling"}),
        &[],
    )
    .expect("a look stands");
    assert_eq!(look.pose, None);
}

#[test]
fn a_line_about_what_the_hero_carries_is_an_act_on_it_with_its_pose() {
    // Host 52, the owner: "list the spear above my head" (lift; the hero carries a spear) was read
    // as a look toward a spear far off in the world. The judge is told what the hero carries.
    let request: IntentRequest = serde_json::from_value(json!({
        "speaker": "hero_11", "line": "list the spear above my head"
    }))
    .expect("a request");
    let facts = scene_facts(&scene(), &request);
    assert_eq!(facts["carries"][0]["id"], "spear");
    assert_eq!(
        facts["carries"][0]["look"],
        "an ash shaft with an iron head"
    );
    let lift = judged(
        json!({"intent": "act", "pose": "holding the spear raised overhead", "spoken": null}),
        &[],
    )
    .expect("an act with a pose stands");
    assert_eq!(
        (lift.kind, lift.pose.as_deref(), lift.spoken),
        (
            IntentKind::Act,
            Some("holding the spear raised overhead"),
            None
        )
    );
}

#[test]
fn a_thing_whose_name_told_its_state_is_renamed_for_the_new_one() {
    // Host 53: the "open timber gate" shut kept its name, and "the timber gate" was not found as
    // it. The judge gives the name for the new state, only with a state set.
    let shut = judged(
        json!({"intent": "act", "target": "tower", "sets": "closed", "renamed": "old watchtower"}),
        &[],
    )
    .expect("a renamed thing stands");
    assert_eq!(
        (shut.sets, shut.renamed.as_deref()),
        (Some(ThingState::Closed), Some("old watchtower"))
    );
    let unset = judged(
        json!({"intent": "act", "target": "tower", "renamed": "old watchtower"}),
        &[],
    )
    .expect("a rename without a state is left out");
    assert_eq!(unset.renamed, None);
}

#[test]
fn a_line_said_to_several_people_present_is_answered_by_each_in_turn() {
    // Host 53: "Good day to you both! What are your names?" with two sellers there was said to no
    // one. Jev lists everyone it is said to, in the order they answer.
    let both = judged(
        json!({"intent": "talk", "addressees": ["baker", "guard"]}),
        &[],
    )
    .expect("a line to both stands");
    assert_eq!(
        (both.target.as_deref(), both.addressees.clone()),
        (Some("baker"), vec!["baker".to_owned(), "guard".to_owned()])
    );
    let one = judged(json!({"intent": "talk", "addressees": ["guard"]}), &[])
        .expect("a line to one stands");
    assert_eq!(
        (one.target.as_deref(), one.addressees.len()),
        (Some("guard"), 0)
    );
    for (fields, reason) in [
        (
            json!({"intent": "talk", "addressees": ["baker", "hero_11"]}),
            "unknown_addressee",
        ),
        (
            json!({"intent": "talk", "target": "well", "addressees": ["baker", "guard"]}),
            "target_not_addressed",
        ),
    ] {
        assert_eq!(
            judged(fields, &[]).map(|intent| intent.kind),
            Err(reason.to_owned())
        );
    }
    let act = judged(
        json!({"intent": "act", "addressees": ["baker", "guard"]}),
        &[],
    )
    .expect("an act stands");
    assert!(act.addressees.is_empty());
}

#[test]
fn a_way_on_never_takes_a_state() {
    // Host 55: "I push the gate doors shut" set the state of `ahead`, the pose place's way on.
    let mut scene = scene();
    scene.things.push(SceneThing {
        id: "ahead".to_owned(),
        name: "Open stone gate".to_owned(),
        look: None,
    });
    let ways = ways();
    let bounds = IntentBounds {
        line: LINE,
        ways: &ways,
        distances: &[],
        approaching: None,
        visited: &[],
    };
    let answer = read_intent(&answer(
        json!({"intent": "act", "target": "ahead", "sets": "closed"}),
    ))
    .expect("an answer");
    assert_eq!(
        intent_of(answer, &scene, &bounds).map_err(|miss| miss.reason),
        Err("no_thing_to_set".to_owned())
    );
}

#[test]
fn a_look_into_a_building_the_scene_has_no_thing_for_stays_a_look_inside() {
    // Host 57: "I peek through the window of the nearest shop" told the outside again.
    let peek = judged(
        json!({"intent": "look", "into": true, "named": "the nearest shop", "noun": "shop"}),
        &[],
    )
    .expect("a look into stands");
    assert_eq!((peek.into, peek.noun.as_deref()), (true, Some("shop")));
    let act = judged(json!({"intent": "act", "into": true}), &[]).expect("an act stands");
    assert!(!act.into);
}

#[test]
fn a_look_into_keeps_the_thing_that_holds_what_is_named_and_a_move_into_does_not() {
    // The prompt's own example: the market's shopfronts for the nearest shop.
    let peek = judged(
        json!({"intent": "look", "into": true, "enter": "tower", "named": "the nearest shop",
               "target_is_named": false}),
        &[],
    )
    .expect("a look into stands");
    assert_eq!(peek.enter.as_deref(), Some("tower"));
    let walk_in = judged(
        json!({"intent": "move", "enter": "tower", "named": "the nearest shop",
               "target_is_named": false, "noun": "shop",
               "distance": "across_the_square"}),
        &[],
    )
    .expect("a move into something else is searched for");
    assert_eq!(walk_in.enter, None);
}
