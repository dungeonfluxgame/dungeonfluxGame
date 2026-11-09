//! Reference sheets: a thing looked at is found among what the place holds, or designed once.

use serde_json::json;

use super::story::{
    Story, StoryBeat, StoryEvidence, StoryItem, StoryScene, read_story, write_story,
};
use super::story_items::{Kept, SheetKind, keep_sheet, known_at, read_design, read_which};

/// A vault with a seal on show and a door already given a sheet.
fn story() -> Story {
    Story {
        title: "The Seal".to_owned(),
        hook: String::new(),
        truth: String::new(),
        culprit: "rival".to_owned(),
        motive: String::new(),
        scenes: vec![StoryScene {
            id: "vault".to_owned(),
            place: "the vault".to_owned(),
            dressing: Vec::new(),
            danger: String::new(),
            music: String::new(),
        }],
        characters: Vec::new(),
        evidence: vec![StoryEvidence {
            id: "seal".to_owned(),
            what: "a wax seal".to_owned(),
            planted_by: "rival".to_owned(),
            flaw: "the ridge is smooth".to_owned(),
            up_close: "red wax with a smooth ridge".to_owned(),
            needs: Vec::new(),
        }],
        beats: vec![StoryBeat {
            id: "beat_vault".to_owned(),
            scene: "vault".to_owned(),
            happens: String::new(),
            turns_on: String::new(),
            leads: String::new(),
            moves: Vec::new(),
            exhibits: vec!["seal".to_owned()],
        }],
        ways: Vec::new(),
        items: vec![StoryItem {
            id: "stone_door".to_owned(),
            name: "the stone door".to_owned(),
            place: "vault".to_owned(),
            look: "a heavy slab with a scratched iron latch".to_owned(),
            evidence: None,
            picture: None,
        }],
        narration: Vec::new(),
        struck: Vec::new(),
        revealable: None,
        findings: super::story_clues::Findings::default(),
        phase: None,
    }
}

#[test]
fn a_place_offers_its_sheets_and_the_evidence_on_show_that_has_none_yet() {
    let story = story();
    let ids: Vec<String> = known_at(&story, "vault")
        .into_iter()
        .map(|known| known.id)
        .collect();
    assert_eq!(ids, ["stone_door", "seal"]);
    let mut sheeted = story;
    sheeted.items.push(StoryItem {
        id: "red_wax_seal".to_owned(),
        name: "the seal".to_owned(),
        place: "vault".to_owned(),
        look: "red wax".to_owned(),
        evidence: Some("seal".to_owned()),
        picture: None,
    });
    let ids: Vec<String> = known_at(&sheeted, "vault")
        .into_iter()
        .map(|known| known.id)
        .collect();
    assert_eq!(
        ids,
        ["stone_door", "red_wax_seal"],
        "a piece is offered by its sheet once it has one"
    );
}

#[test]
fn the_sheet_judge_names_a_known_thing_or_none() {
    let known = ["stone_door".to_owned(), "seal".to_owned()];
    assert_eq!(
        read_which(&json!({"item": "seal"}), &known).unwrap(),
        Some("seal".to_owned())
    );
    assert_eq!(read_which(&json!({"item": null}), &known).unwrap(), None);
    assert!(
        read_which(&json!({"item": "torch"}), &known).is_err(),
        "an unknown id is refused"
    );
}

#[test]
fn a_designed_sheet_takes_a_new_plain_id_at_its_place() {
    let story = story();
    let answer = json!({"id": "iron_latch", "name": "the latch", "look": "black iron, scratched"});
    let sheet = read_design(&answer, &story, ("vault", None)).unwrap();
    assert_eq!(sheet.place, "vault");
    assert_eq!(sheet.evidence, None);
    let for_seal = read_design(
        &json!({"id": "red_wax_seal", "name": "the seal", "look": "red wax"}),
        &story,
        ("vault", Some("seal")),
    )
    .unwrap();
    assert_eq!(for_seal.evidence.as_deref(), Some("seal"));
    for (id, why) in [
        ("stone_door", "an id kept already"),
        ("seal", "an evidence id"),
        ("Iron Latch", "not plain"),
    ] {
        let refused = json!({"id": id, "name": "the latch", "look": "black iron"});
        assert!(
            read_design(&refused, &story, ("vault", None)).is_err(),
            "{why}"
        );
    }
    let empty = json!({"id": "iron_latch", "name": " ", "look": "black iron"});
    assert!(
        read_design(&empty, &story, ("vault", None)).is_err(),
        "a sheet needs a name"
    );
}

#[test]
fn every_sheet_kind_key_is_its_serde_name() {
    for kind in [SheetKind::Known, SheetKind::Evidence, SheetKind::New] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.key()));
    }
}

fn sheet(id: &str, evidence: Option<&str>) -> StoryItem {
    StoryItem {
        id: id.to_owned(),
        name: "a thing".to_owned(),
        place: "vault".to_owned(),
        look: "how it looks".to_owned(),
        evidence: evidence.map(str::to_owned),
        picture: None,
    }
}

#[test]
fn a_sheet_another_look_kept_meanwhile_stands_and_otherwise_the_new_one_is_added() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("story.json");
    let judged = story();

    let mut now = judged.clone();
    now.items.push(sheet("red_seal", Some("seal")));
    write_story(&path, &now).unwrap();
    match keep_sheet(&path, &judged, sheet("wax_seal", Some("seal"))).unwrap() {
        Kept::First(first) => assert_eq!(first.id, "red_seal", "the first sheet stands"),
        _ => panic!("the seal's sheet kept first is answered"),
    }

    let mut now = judged.clone();
    now.items.push(sheet("niche", None));
    write_story(&path, &now).unwrap();
    assert!(
        matches!(
            keep_sheet(&path, &judged, sheet("lamp", None)).unwrap(),
            Kept::Changed(_)
        ),
        "a thing kept at the place meanwhile may be the same: judged again"
    );

    write_story(&path, &judged).unwrap();
    assert!(matches!(
        keep_sheet(&path, &judged, sheet("lamp", None)).unwrap(),
        Kept::Added
    ));
    let ids: Vec<String> = read_story(&path)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert!(ids.contains(&"lamp".to_owned()), "{ids:?}");
}
