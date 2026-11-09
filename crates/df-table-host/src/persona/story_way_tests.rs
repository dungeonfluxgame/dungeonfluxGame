//! The way between the story's places: a plotted way is kept only when it joins a place to the
//! next in two or three stages and carries no clue, play goes along it stage by stage to the
//! arrival, and each stage's plate is made before play and kept on it.

use serde_json::{Value, json};

use super::fixtures::{CHARACTER, innkeeper, scripted_host};
use super::storage::write_persona;
use super::story::{Story, StoryScene, read_story, story_path, write_story};
use super::story_clues::Findings;
use super::story_next::parse_move;
use super::story_way::{
    MAXIMUM_STAGE_MOVES, PlayPhase, StageTurn, StoryWay, WayStage, current_stage, onward,
    parse_ways, read_stage_served, set_off, stage_scene, stage_situation, stage_turn,
    way_plate_name, way_plates,
};
use crate::test_support::layout_of_cast;

fn scenes() -> Vec<String> {
    ["scriptorium", "reliquary", "vault"]
        .map(str::to_owned)
        .to_vec()
}

/// The ids of the story's evidence, each on show at one of its places.
fn evidence() -> Vec<String> {
    ["corridor_inscriptions", "fresh_mud_scuffs"]
        .map(str::to_owned)
        .to_vec()
}

fn stage(id: &str, worth_a_look: Value) -> Value {
    json!({
        "id": id, "sight": format!("The {id} as the party goes."), "worth_a_look": worth_a_look,
        "touches": []
    })
}

fn way(from: &str, to: &str, stages: &[Value]) -> Value {
    json!({"from": from, "to": to, "stages": stages})
}

/// A way from each place to the next, the second listed first.
fn answer() -> Value {
    json!({"ways": [
        way("reliquary", "vault", &[
            stage("corridor_end", json!("Fresh scratches on a latch.")),
            stage("vault_door", Value::Null),
        ]),
        way("scriptorium", "reliquary", &[
            stage("flooded_stair", Value::Null),
            stage("niche_row", json!("A drip echoing ahead.")),
            stage("tiled_run", Value::Null),
        ]),
    ]})
}

fn refused(answer: &Value) -> bool {
    parse_ways(answer, &scenes(), &evidence()).is_err()
}

#[test]
fn a_way_of_two_or_three_stages_from_each_place_to_the_next_is_read_in_the_scenes_order() {
    let ways = parse_ways(&answer(), &scenes(), &evidence()).unwrap();
    let joined: Vec<(&str, &str, usize)> = ways
        .iter()
        .map(|way| (way.from.as_str(), way.to.as_str(), way.stages.len()))
        .collect();
    assert_eq!(
        joined,
        [("scriptorium", "reliquary", 3), ("reliquary", "vault", 2)]
    );
    assert_eq!(ways[0].stages[0].worth_a_look, None);
    assert_eq!(
        ways[0].stages[1].worth_a_look.as_deref(),
        Some("A drip echoing ahead.")
    );
}

#[test]
fn a_way_with_too_few_or_too_many_stages_is_refused() {
    let with_stages = |stages: &[Value]| {
        json!({"ways": [
            way("scriptorium", "reliquary", stages),
            answer()["ways"][0].clone(),
        ]})
    };
    assert!(refused(&with_stages(&[stage("one", Value::Null)])));
    let four: Vec<Value> = ["one", "two", "three", "four"]
        .iter()
        .map(|id| stage(id, Value::Null))
        .collect();
    assert!(refused(&with_stages(&four)));
}

#[test]
fn a_way_that_does_not_join_a_place_to_the_next_once_is_refused() {
    let two = [stage("one", Value::Null), stage("two", Value::Null)];
    let skipping = json!({"ways": [
        way("scriptorium", "vault", &two), answer()["ways"][0].clone()
    ]});
    assert!(refused(&skipping), "a way skips no place");
    let backwards = json!({"ways": [
        way("reliquary", "scriptorium", &two), answer()["ways"][0].clone()
    ]});
    assert!(refused(&backwards), "a way goes on, never back");
    let missing = json!({"ways": [answer()["ways"][0].clone()]});
    assert!(refused(&missing), "every place has its way to the next");
    let mut twice = answer();
    twice["ways"]
        .as_array_mut()
        .unwrap()
        .push(way("reliquary", "vault", &two));
    assert!(refused(&twice), "one way between two places");
}

#[test]
fn a_stage_id_is_a_new_plain_id_and_a_thing_worth_a_look_is_said_or_null() {
    let mut taken = answer();
    taken["ways"][0]["stages"][1]["id"] = json!("flooded_stair");
    assert!(refused(&taken), "another stage's id");
    let mut place = answer();
    place["ways"][0]["stages"][1]["id"] = json!("vault");
    assert!(refused(&place), "a place's id");
    let mut plain = answer();
    plain["ways"][0]["stages"][1]["id"] = json!("Vault Door");
    assert!(refused(&plain));
    let mut blank = answer();
    blank["ways"][0]["stages"][1]["worth_a_look"] = json!("  ");
    assert!(refused(&blank), "a blank is neither a thing nor null");
    let mut unseen = answer();
    unseen["ways"][0]["stages"][1]["sight"] = json!("");
    assert!(refused(&unseen), "every stage has its sight");
}

#[test]
fn a_stage_that_touches_a_piece_of_evidence_is_refused_with_the_piece_named() {
    // Dungeon run 24: the way to the reliquary showed its carved inscriptions, a reliquary exhibit.
    let mut inscribed = answer();
    inscribed["ways"][1]["stages"][1]["touches"] = json!(["corridor_inscriptions"]);
    let Err(refusal) = parse_ways(&inscribed, &scenes(), &evidence()) else {
        panic!("a stage that touches an exhibit is kept");
    };
    let why = refusal.to_string();
    assert!(
        why.contains("niche_row") && why.contains("corridor_inscriptions"),
        "{why}"
    );
    assert!(why.contains("a way carries no clue"), "{why}");
}

#[test]
fn a_stage_that_names_no_piece_of_the_evidence_as_touched_is_refused() {
    let mut unknown = answer();
    unknown["ways"][0]["stages"][0]["touches"] = json!(["a_painted_door"]);
    assert!(refused(&unknown), "an id that is no piece of evidence");
    let mut unsaid = answer();
    unsaid["ways"][0]["stages"][0]
        .as_object_mut()
        .unwrap()
        .remove("touches");
    assert!(refused(&unsaid), "every stage says what it touches");
}

fn stored_ways() -> Vec<StoryWay> {
    parse_ways(&answer(), &scenes(), &evidence()).unwrap()
}

#[test]
fn the_phase_goes_along_the_way_stage_by_stage_and_then_arrives() {
    let ways = stored_ways();
    let mut phase = set_off(&ways, "scriptorium", "reliquary");
    let mut stages_seen = Vec::new();
    while let Some((_, at, stage)) = current_stage(&ways, Some(&phase)) {
        stages_seen.push((at, stage.id.clone()));
        assert_eq!(
            phase.place(),
            "scriptorium",
            "the place left, until arrival"
        );
        phase = onward(phase.after_move(), &ways);
    }
    assert_eq!(
        stages_seen,
        [
            (0, "flooded_stair".to_owned()),
            (1, "niche_row".to_owned()),
            (2, "tiled_run".to_owned())
        ]
    );
    assert_eq!(
        phase,
        PlayPhase::Place {
            place: "reliquary".to_owned()
        },
        "the last stage gives way to the place ahead"
    );
    assert_eq!(onward(phase.clone(), &ways), phase, "a place stays put");
}

#[test]
fn a_stage_waits_for_a_move_and_is_served_after_its_most_moves_without_a_judgement() {
    assert_eq!(stage_turn(0), StageTurn::Unplayed);
    assert_eq!(stage_turn(MAXIMUM_STAGE_MOVES - 1), StageTurn::Judged);
    assert_eq!(stage_turn(MAXIMUM_STAGE_MOVES), StageTurn::Served);
    let ways = stored_ways();
    let phase = set_off(&ways, "reliquary", "vault")
        .after_move()
        .after_move();
    assert_eq!(
        phase,
        PlayPhase::Way {
            from: "reliquary".to_owned(),
            to: "vault".to_owned(),
            stage: 0,
            moves: 2
        },
        "a move on the way counts toward its stage"
    );
    let at_a_place = PlayPhase::Place {
        place: "vault".to_owned(),
    };
    assert_eq!(at_a_place.clone().after_move(), at_a_place);
}

#[test]
fn a_stage_with_nothing_worth_a_look_still_moves_on_and_shows_only_its_sight() {
    let ways = stored_ways();
    let phase = set_off(&ways, "reliquary", "vault");
    let phase = onward(phase.after_move(), &ways);
    let (_, _, stage) = current_stage(&ways, Some(&phase)).unwrap();
    assert_eq!(stage.id, "vault_door");
    assert_eq!(stage.worth_a_look, None);
    assert_eq!(
        onward(phase.after_move(), &ways),
        PlayPhase::Place {
            place: "vault".to_owned()
        }
    );
    let party = ["the tall man in grey".to_owned()];
    assert_eq!(
        stage_scene(stage, &party),
        "The vault_door as the party goes. Going on together: the tall man in grey."
    );
    let place = |id: &str| StoryScene {
        id: id.to_owned(),
        place: format!("The {id}."),
        dressing: Vec::new(),
        danger: String::new(),
        music: String::new(),
    };
    let situation = stage_situation(&place("reliquary"), &place("vault"), stage);
    assert!(!situation.contains("Worth a look"), "{situation}");
    let marked = WayStage {
        worth_a_look: Some("Fresh scratches on a latch.".to_owned()),
        ..stage.clone()
    };
    assert!(stage_scene(&marked, &party).contains("In sight there: Fresh scratches on a latch."));
}

#[test]
fn setting_off_where_no_way_is_plotted_arrives_at_once() {
    assert_eq!(
        set_off(&[], "scriptorium", "reliquary"),
        PlayPhase::Place {
            place: "reliquary".to_owned()
        }
    );
}

#[test]
fn the_stage_judge_answers_true_or_false() {
    assert!(read_stage_served(&json!({"why": "talked", "served": true})).unwrap());
    assert!(!read_stage_served(&json!({"why": "a question waits", "served": false})).unwrap());
    assert!(read_stage_served(&json!({"why": "no flag"})).is_err());
}

#[test]
fn a_move_on_the_way_never_sets_off_or_ends_the_story() {
    let here = ["person-1".to_owned(), "person-2".to_owned()];
    let next = parse_move(
        &json!({"speaker": "person-1", "to": "room", "goal": "Point out the scratches."}),
        (&here, &here),
    )
    .unwrap();
    assert_eq!((next.served, next.move_to, next.end), (false, None, false));
    assert!(
        parse_move(
            &json!({"speaker": "person-9", "to": "room", "goal": "Go on."}),
            (&here, &here)
        )
        .is_err()
    );
}

/// A plotted story with the ways of [`answer`] and nothing else of a plot.
fn story_with_ways() -> Story {
    let place = |id: &str| StoryScene {
        id: id.to_owned(),
        place: format!("The {id}."),
        dressing: Vec::new(),
        danger: String::new(),
        music: String::new(),
    };
    Story {
        title: "The Charter".to_owned(),
        hook: "A charter is lost.".to_owned(),
        truth: "A rival took it.".to_owned(),
        culprit: "rival".to_owned(),
        motive: "Gain.".to_owned(),
        scenes: scenes().iter().map(|id| place(id)).collect(),
        characters: Vec::new(),
        evidence: Vec::new(),
        beats: Vec::new(),
        ways: stored_ways(),
        items: Vec::new(),
        narration: Vec::new(),
        struck: Vec::new(),
        revealable: None,
        findings: Findings::default(),
        phase: None,
    }
}

#[test]
fn a_stage_plate_is_named_after_its_stage() {
    assert_eq!(way_plate_name("flooded_stair"), "way-flooded_stair");
}

#[tokio::test]
async fn way_plates_keeps_each_stage_plate_already_made_and_records_it_on_its_stage() {
    // No image backend is served: a stage whose plate were not kept would fail to be made.
    let host = scripted_host(Vec::new()).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    host.running.write_asset("persona/story.json", "{}");
    let story = story_with_ways();
    write_story(&story_path(&host.running.state), &story).unwrap();
    let stages: Vec<String> = story
        .ways
        .iter()
        .flat_map(|way| &way.stages)
        .map(|stage| stage.id.clone())
        .collect();
    for stage in &stages {
        let plate = format!("{}.png", way_plate_name(stage));
        host.running.write_asset(&format!("media/{plate}"), &plate);
        crate::locate::write_layout(
            &host.running.playground_path("assets/layouts"),
            &plate,
            &layout_of_cast(&plate, &[CHARACTER]),
        )
        .unwrap();
    }

    let made = way_plates(
        host.port,
        &[CHARACTER.to_owned()],
        "the keeper in her apron",
    )
    .await
    .unwrap();

    for stage in &stages {
        assert!(
            made.contains(&format!(
                "stage {stage}: kept way-{stage}.png and its scan, not rendered again"
            )),
            "{made}"
        );
    }
    let kept = read_story(&story_path(&host.running.state)).unwrap();
    let plates: Vec<(String, Option<String>)> = kept
        .ways
        .iter()
        .flat_map(|way| &way.stages)
        .map(|stage| (stage.id.clone(), stage.plate.clone()))
        .collect();
    let expected: Vec<(String, Option<String>)> = stages
        .iter()
        .map(|stage| (stage.clone(), Some(format!("way-{stage}.png"))))
        .collect();
    assert_eq!(plates, expected);
    assert!(
        host.persona_lines().contains(
            &"PERSONA story_way_plate stage=flooded_stair plate=way-flooded_stair.png".to_owned()
        ),
        "{:?}",
        host.persona_lines()
    );
}
