//! Tests of `brief`.

use super::brief::{
    BriefCharacter, BriefError, BriefStyle, LookSheets, build_brief, build_brief_with,
    build_stage_brief, scene_words,
};
use crate::imagegen::limits::MAXIMUM_SENT_TEXT_CHARS;
use crate::movement::types::{Depth, Placement, Pose, RouteSight, SceneDesign, Side};

fn character(id: &str, display_name: &str, job: &str, portrait: Option<&str>) -> BriefCharacter {
    BriefCharacter {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
        job: job.to_owned(),
        portrait: portrait.map(str::to_owned),
        carrying: Vec::new(),
    }
}

fn placement(character: &str, depth: Depth, side: Side, doing: &str) -> Placement {
    Placement {
        character: character.to_owned(),
        depth,
        side,
        doing: doing.to_owned(),
    }
}

fn balcony() -> SceneDesign {
    SceneDesign {
        feature: "tavern_balcony".to_owned(),
        place: "a stone balcony above a harbour".to_owned(),
        mood: "windy dusk".to_owned(),
        elements: vec!["lanterns".to_owned(), "a rope rail".to_owned()],
        things: vec![],
        shown_items: vec![],
        residents: vec![],
        placements: vec![
            placement("ana", Depth::Foreground, Side::Left, "leans on the rail"),
            placement("bran", Depth::Background, Side::Right, "watches the boats"),
        ],
        left_behind: vec!["cora".to_owned(), "dev".to_owned()],
        stages: vec![],
        town_look: None,
    }
}

fn cast() -> Vec<BriefCharacter> {
    vec![
        character("ana", "Ana", "ranger", Some("portraits/ana.png")),
        character("bran", "Bran", "sailor", None),
        character("cora", "Cora", "cook", Some("portraits/cora.png")),
        character("dev", "Dev", "bard", None),
    ]
}

fn looks() -> LookSheets {
    LookSheets::from_json_str(r#"{"ana": "red cloak, grey eyes"}"#).expect("looks read")
}

#[test]
fn references_cover_only_the_placed_characters_in_placement_order() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    let ids: Vec<&str> = brief
        .references
        .iter()
        .map(|card| card.character.as_str())
        .collect();
    assert_eq!(ids, ["ana", "bran"]);
}

#[test]
fn prompt_leaves_out_the_characters_who_stayed_behind() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert!(!brief.prompt.contains("Cora"));
    assert!(!brief.prompt.contains("Dev"));
}

#[test]
fn prompt_names_each_placed_character_with_depth_and_side() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert!(brief.prompt.contains(
        "A person (red cloak, grey eyes) stands in the foreground left, leans on the rail."
    ));
    assert!(
        brief
            .prompt
            .contains("A person (sailor) stands in the background right, watches the boats.")
    );
}

#[test]
fn look_sheet_describes_a_character_over_the_job() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert_eq!(brief.references[0].look, "red cloak, grey eyes");
}

#[test]
fn job_describes_a_character_without_a_look_sheet() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert_eq!(brief.references[1].look, "sailor");
}

#[test]
fn portrait_passes_through_to_the_reference_card() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert_eq!(
        brief.references[0].portrait.as_deref(),
        Some("portraits/ana.png")
    );
    assert_eq!(brief.references[1].portrait, None);
}

#[test]
fn prompt_forbids_writing_in_the_picture() {
    let brief = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert!(brief.avoid.iter().any(|term| term == "readable text"));
    assert!(brief.avoid.iter().any(|term| term == "speech bubbles"));
    assert!(
        brief.prompt.contains("faded, smudged ink"),
        "writing a thing carries is shown unreadable, never left out"
    );
}

#[test]
fn same_design_gives_the_same_brief() {
    let first = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    let second = build_brief(&balcony(), &cast(), &looks()).expect("builds");
    assert_eq!(first, second);
}

#[test]
fn placement_of_an_unknown_character_is_refused() {
    let mut design = balcony();
    design
        .placements
        .push(placement("zed", Depth::Middle, Side::Centre, "waits"));
    let error = build_brief(&design, &cast(), &looks()).expect_err("refused");
    assert_eq!(
        error,
        BriefError::UnknownCharacter {
            id: "zed".to_owned()
        }
    );
    assert_eq!(error.key(), "unknown_character");
}

#[test]
fn prompt_over_the_limit_is_refused_not_cut() {
    let mut design = balcony();
    design.place = "x".repeat(MAXIMUM_SENT_TEXT_CHARS);
    match build_brief(&design, &cast(), &looks()).expect_err("refused") {
        BriefError::PromptTooLong { length, maximum } => {
            assert_eq!(maximum, MAXIMUM_SENT_TEXT_CHARS);
            assert!(length > maximum);
        }
        other => panic!("expected PromptTooLong, got {other:?}"),
    }
}

#[test]
fn look_sheets_read_a_json_object_of_looks() {
    let sheets = LookSheets::from_json_str(r#"{"ana": "red cloak"}"#).expect("reads");
    assert_eq!(sheets.look_of("ana"), Some("red cloak"));
    assert_eq!(sheets.look_of("bran"), None);
}

#[test]
fn look_sheets_refuse_malformed_content() {
    for text in ["[1, 2]", "not json", r#"{"ana": 3}"#, r#"{"ana": " "}"#] {
        let error = LookSheets::from_json_str(text).expect_err("refused");
        assert_eq!(error.key(), "bad_looks", "input {text}");
    }
}

#[test]
fn empty_look_sheets_describe_everyone_by_job() {
    let brief = build_brief(&balcony(), &cast(), &LookSheets::empty()).expect("builds");
    assert_eq!(brief.references[0].look, "ranger");
}

#[test]
fn custom_style_replaces_the_direction_and_ban() {
    let style = BriefStyle {
        direction: "A pencil sketch of".to_owned(),
        writing: "Lettering is a blur.".to_owned(),
        avoid: vec!["ink".to_owned()],
    };
    let brief = build_brief_with(&style, &balcony(), &cast(), &looks()).expect("builds");
    assert!(
        brief
            .prompt
            .starts_with("A pencil sketch of a stone balcony")
    );
    assert!(brief.prompt.ends_with("Lettering is a blur."));
    assert_eq!(brief.avoid, ["ink"]);
}

/// A mover whose id and display name are the same word, as a roamer's are: the image model paints
/// any word it is given (a signboard once read "Danny traveling").
fn roamer() -> (SceneDesign, Vec<BriefCharacter>) {
    let mut design = balcony();
    design.placements = vec![placement(
        "danny",
        Depth::Middle,
        Side::Left,
        "walks the road, traveling light",
    )];
    design.left_behind = Vec::new();
    let mut danny = character("danny", "Danny", "a lean painter with a sandy beard", None);
    danny.carrying = vec!["folding easel".to_owned(), "leather satchel".to_owned()];
    (design, vec![danny])
}

#[test]
fn the_brief_never_holds_a_placed_character_s_name_or_id() {
    let (design, cast) = roamer();
    let brief = build_brief(&design, &cast, &LookSheets::empty()).expect("builds");
    for word in ["Danny", "danny"] {
        assert!(!brief.prompt.contains(word), "{word} in: {}", brief.prompt);
    }
    // Nor does what a frame judge reads as the picture on screen.
    let words = scene_words(&design, &cast, &LookSheets::empty()).expect("words");
    assert!(
        !words.contains("Danny") && !words.contains("danny"),
        "{words}"
    );
    // The scan that names them in the picture still has the name: only the prompt is nameless.
    assert_eq!(cast[0].display_name, "Danny");
}

#[test]
fn what_a_character_carries_in_view_is_said_in_their_sentence() {
    let (design, cast) = roamer();
    let words = scene_words(&design, &cast, &LookSheets::empty()).expect("words");
    assert!(
        words.contains(
            "A person (a lean painter with a sandy beard) stands in the middle left, walks the \
             road, traveling light, with their folding easel and leather satchel in view."
        ),
        "{words}"
    );
}

const TOWN: &str = "half-timbered houses with steep red tile roofs and narrow cobbled lanes";

fn fountain() -> RouteSight {
    RouteSight {
        id: "fountain_square".to_owned(),
        sight: "a stone fountain in a small square.".to_owned(),
        pose: Pose::Walking,
    }
}

#[test]
fn the_town_look_is_in_the_place_prompt_only_when_the_design_has_one() {
    let (mut design, cast) = roamer();
    let without = build_brief(&design, &cast, &LookSheets::empty()).expect("builds");
    assert!(!without.prompt.contains("The town"), "{}", without.prompt);

    design.town_look = Some(TOWN.to_owned());
    let with = build_brief(&design, &cast, &LookSheets::empty()).expect("builds");
    assert!(
        with.prompt.contains(&format!(
            "The town, the same in every picture of it: {TOWN}."
        )),
        "{}",
        with.prompt
    );
    let words = scene_words(&design, &cast, &LookSheets::empty()).expect("words");
    assert!(
        words.contains(TOWN),
        "a frame judge reads the town too: {words}"
    );
}

#[test]
fn a_sight_on_the_way_is_a_wide_view_of_the_mover_in_the_towns_look() {
    let (mut design, cast) = roamer();
    design.town_look = Some(TOWN.to_owned());

    let brief =
        build_stage_brief(&design, &fountain(), &cast, &LookSheets::empty()).expect("builds");

    for part in [
        TOWN,
        "a wide view of a stone fountain in a small square.",
        "A person (a lean painter with a sandy beard) is walking on through it",
        "The cast in view is one person",
    ] {
        assert!(brief.prompt.contains(part), "{part} in: {}", brief.prompt);
    }
    for word in ["Danny", "danny", "folding easel", "satchel"] {
        assert!(!brief.prompt.contains(word), "{word} in: {}", brief.prompt);
    }
    let ids: Vec<&str> = brief
        .references
        .iter()
        .map(|card| card.character.as_str())
        .collect();
    assert_eq!(ids, ["danny"], "the mover's sheet is the one reference");
    assert_eq!(brief.avoid, BriefStyle::default().avoid);
}

#[test]
fn a_pausing_mover_has_stopped_to_take_the_sight_in() {
    let (design, cast) = roamer();
    let sight = RouteSight {
        pose: Pose::Pausing,
        ..fountain()
    };

    let brief = build_stage_brief(&design, &sight, &cast, &LookSheets::empty()).expect("builds");

    assert!(
        brief.prompt.contains("has stopped there to take it in"),
        "{}",
        brief.prompt
    );
    assert!(
        !brief.prompt.contains("The town"),
        "no town, no town sentence"
    );
}

#[test]
fn a_sight_with_two_movers_says_both_are_in_view_once() {
    let (design, mut cast) = roamer();
    cast.push(character(
        "mara",
        "Mara",
        "a tall woman in a green hood",
        None,
    ));

    let brief =
        build_stage_brief(&design, &fountain(), &cast, &LookSheets::empty()).expect("builds");

    assert!(
        brief.prompt.contains("The cast in view is 2 people"),
        "{}",
        brief.prompt
    );
    assert_eq!(brief.references.len(), 2);
}

#[test]
fn a_sight_over_the_image_service_s_limit_is_refused() {
    let (design, cast) = roamer();
    let sight = RouteSight {
        sight: "a cobbled lane ".repeat(MAXIMUM_SENT_TEXT_CHARS / 10),
        ..fountain()
    };

    let error = build_stage_brief(&design, &sight, &cast, &LookSheets::empty()).unwrap_err();

    assert_eq!(error.key(), "prompt_too_long");
}
