use df_playground::look::wire::{
    BodyRegion, ControlValue, DetailKind, LockState, LookDetail, LookDraft, LookLockAsk,
    LookVersion, PictureFailure, VersionState,
};
use df_playground::look::wire::{HeroPack, PackStatus};
use serde_json::json;

use super::catalog::{PublicHero, built_in_studio, fill};
use super::jev::{DETAILS_INSTRUCTION, details_facts, details_schema, parse_details};
use super::request::{AnswerCause, LookError};
use super::store::StudioRecord;
use super::studio::{arrive, check_details, keep_in};
use crate::seat::book::{HeroOwner, HeroRecord, Holder};
use crate::seat::key::KeyHash;
use crate::test_support::{STARTER_FILES, start};

fn scar() -> LookDetail {
    LookDetail {
        kind: DetailKind::Scar,
        place: BodyRegion::Face,
        what: "a jagged scar across the left cheek".to_owned(),
    }
}

fn elf() -> PublicHero {
    PublicHero {
        species: "elf".to_owned(),
        class: "wizard".to_owned(),
    }
}

fn version(id: u64, state: VersionState) -> LookVersion {
    LookVersion {
        id,
        parent: None,
        draft: LookDraft::default(),
        changes: Vec::new(),
        picture: None,
        state,
        asked_ms: 0,
        took_ms: None,
    }
}

#[test]
fn the_built_in_studio_file_reads_and_every_species_has_controls() {
    let studio = built_in_studio();
    for species in [
        "dragonborn",
        "dwarf",
        "elf",
        "gnome",
        "goliath",
        "halfling",
        "human",
        "orc",
        "tiefling",
    ] {
        assert!(!studio.controls(species, 1).unwrap().controls.is_empty());
    }
}

#[test]
fn the_change_list_names_only_what_changed_between_two_drafts() {
    let studio = built_in_studio();
    let before = studio.settle("elf", &LookDraft::default()).unwrap();
    let mut after = before.clone();
    after
        .values
        .insert("hair_length".to_owned(), ControlValue::Number(4));
    after.details.push(scar());
    let changes = studio.changes("elf", &before, &after).unwrap();
    assert_eq!(
        changes,
        vec![
            "hair length from short to shoulder-length".to_owned(),
            "add a jagged scar across the left cheek (scar on the face)".to_owned(),
        ]
    );
    assert!(studio.changes("elf", &after, &after).unwrap().is_empty());
}

#[test]
fn a_draft_with_a_control_the_species_lacks_or_a_value_out_of_range_is_refused() {
    let studio = built_in_studio();
    let mut horns = LookDraft::default();
    horns
        .values
        .insert("horns".to_owned(), ControlValue::Choice("ram".to_owned()));
    assert!(matches!(
        studio.settle("elf", &horns),
        Err(LookError::InvalidDraft { .. })
    ));
    let mut tall = LookDraft::default();
    tall.values
        .insert("height".to_owned(), ControlValue::Number(200));
    assert!(matches!(
        studio.settle("elf", &tall),
        Err(LookError::InvalidDraft { .. })
    ));
}

#[test]
fn a_late_result_after_a_newer_refine_is_stale_and_is_never_kept_ref_032() {
    let mut record = StudioRecord {
        next_version: 3,
        ..StudioRecord::default()
    };
    record.view.versions = vec![
        version(1, VersionState::Pending),
        version(2, VersionState::Pending),
    ];
    let late = arrive(&mut record, 1, Ok("look-1-v1.png".to_owned()), 40).unwrap();
    assert_eq!(late.state, VersionState::Stale);
    assert!(matches!(
        keep_in(&mut record, 1),
        Err(LookError::NotKeepable { version: 1, .. })
    ));
    assert_eq!(record.view.kept, None);
    let current = arrive(&mut record, 2, Ok("look-1-v2.png".to_owned()), 50).unwrap();
    assert_eq!(current.state, VersionState::Ready);
    assert_eq!(keep_in(&mut record, 2).unwrap(), None);
    assert_eq!(record.view.kept, Some(2));
}

#[test]
fn a_failed_refine_keeps_the_old_kept_version() {
    let mut record = StudioRecord::default();
    record.view.versions = vec![
        version(1, VersionState::Ready),
        version(2, VersionState::Pending),
    ];
    record.view.kept = Some(1);
    let failed = VersionState::Failed {
        cause: PictureFailure::Backend,
    };
    arrive(&mut record, 2, Err(failed), 10).unwrap();
    assert_eq!(record.view.kept, Some(1));
}

#[test]
fn details_read_against_the_closed_lists_and_unknown_keys_or_empty_words_are_refused() {
    let controls = built_in_studio().word_controls("human").unwrap();
    let parse_details = |answer: &serde_json::Value| {
        parse_details(answer, &controls).map(|read| (read.details, read.category))
    };
    let answer = json!({
        "controls": [],
        "details": [
            {"kind": "scar", "where": "face", "what": " a jagged scar across the left cheek "},
            {"kind": "scar", "where": "face", "what": "a jagged scar across the left cheek"},
            {"kind": "piercing", "where": "ears", "what": "three silver rings"}
        ],
        "category": "general"
    });
    let (details, category) = parse_details(&answer).unwrap();
    assert_eq!(details.len(), 2);
    assert_eq!(details[0], scar());
    assert_eq!(category.key(), "general");
    let unknown = json!({"controls": [],
        "details": [{"kind": "wings", "where": "back", "what": "bat wings"}],
        "category": "general"});
    assert!(matches!(
        parse_details(&unknown),
        Err(AnswerCause::UnknownKey { .. })
    ));
    let empty = json!({"controls": [],
        "details": [{"kind": "scar", "where": "face", "what": "  "}],
        "category": "general"});
    assert_eq!(parse_details(&empty), Err(AnswerCause::EmptyDetail));
    let category = json!({"controls": [], "details": [], "category": "spicy"});
    assert!(matches!(
        parse_details(&category),
        Err(AnswerCause::UnknownCategory { .. })
    ));
}

#[test]
fn the_heros_name_never_reaches_a_look_prompt() {
    let studio = built_in_studio();
    let hero = HeroRecord {
        id: 7,
        owner: HeroOwner::Friend { profile: 1 },
        name: "Thorin Oakenshield".to_owned(),
        species: "dwarf".to_owned(),
        class: "fighter".to_owned(),
        portrait: None,
        pack: HeroPack::default(),
        locked: true,
        look: None,
        legacy_sheet: None,
        legacy_ready: None,
        last_played_ms: None,
    };
    let public = PublicHero::of(&hero);
    let mut draft = studio.settle("dwarf", &LookDraft::default()).unwrap();
    draft.details.push(scar());
    let description = studio.description(&public, &draft).unwrap();
    let first = studio.first_prompt(&description);
    let edit = studio.edit_prompt(
        &studio
            .changes("dwarf", &LookDraft::default(), &draft)
            .unwrap(),
    );
    for prompt in [&description, &first, &edit] {
        assert!(!prompt.contains("Thorin"), "{prompt}");
        assert!(!prompt.contains("Oakenshield"), "{prompt}");
    }
    assert!(first.contains("species dwarf, class fighter"));
}

#[test]
fn a_players_words_never_fill_a_placeholder() {
    assert_eq!(
        fill("add {detail}", &[("detail", "a {detail} tattoo")]),
        "add a {detail} tattoo"
    );
    assert_eq!(fill("{missing} stays", &[]), "{missing} stays");
}

#[test]
fn the_description_carries_species_class_controls_and_details() {
    let studio = built_in_studio();
    let mut draft = studio.settle("elf", &LookDraft::default()).unwrap();
    draft.details.push(scar());
    let description = studio.description(&elf(), &draft).unwrap();
    // Who the hero presents as is a control like the others, and the first one.
    assert!(
        description
            .starts_with("species elf, class wizard; an androgynous person; about 5 ft 6 in tall"),
        "{description}"
    );
    assert!(description.ends_with("a jagged scar across the left cheek (scar on the face)"));
}

#[tokio::test]
async fn a_lock_in_whose_sheet_fails_still_locks_the_hero_with_its_portrait_ref_059() {
    let running = start(&STARTER_FILES).await;
    let state = &running.state;
    let key = KeyHash::of("seat-key-for-the-lock-in-test");
    let hero = {
        let mut open = state.seats().open().await.unwrap();
        open.books().seat(&key, 1, Holder::Guest, "Ann", 0);
        let card = open
            .books()
            .add_hero(1, "Thorin", "dwarf", "fighter")
            .unwrap();
        open.books().choose_hero(&key, card.id).unwrap();
        card.id
    };
    let studio = state.look().cloned().unwrap();
    studio
        .with_record(state, 1, |record| {
            record.next_version = 2;
            record.hero = Some(hero);
            let mut kept = version(1, VersionState::Ready);
            kept.picture = Some("look-1-v1.png".to_owned());
            record.view.versions.push(kept);
            record.view.kept = Some(1);
            Ok(())
        })
        .await
        .unwrap();
    let ask = LookLockAsk {
        draft: LookDraft::default(),
    };
    // No three-view template is chosen, so the sheet fails before any picture is asked for.
    let view = studio.lock(state, 1, ask, true).await.unwrap();
    let lock = view.lock.unwrap();
    assert_eq!(lock.hero, hero);
    assert_eq!(
        lock.state,
        LockState::Locked {
            portrait: Some("look-1-v1.png".to_owned()),
            sheet: None,
        }
    );
    let mut open = state.seats().open().await.unwrap();
    let record = open.books().hero(hero).unwrap();
    assert!(record.locked);
    assert_eq!(record.portrait.as_deref(), Some("look-1-v1.png"));
    assert_eq!(
        record.pack.sheet,
        PackStatus::Failed {
            cause: PictureFailure::NoTemplate
        }
    );
}
// TODO(DEV-12): a refine through the routes with a scripted Jev and a fake image backend: first
// render edits the base pose, a second edits the kept version with the base pose as a thing.
// TODO(DEV-12): pose candidates and choose through the routes write poses.json and copy the file.
// TODO(DEV-12): the phone's look routes take the seat from the key header, never the body.

#[test]
fn the_start_form_shows_an_example_picture_only_when_the_library_has_it() {
    let studio = built_in_studio();
    let options = studio.start_options(|picture| picture == "look-species-dwarf.png");
    let dwarf = options
        .species
        .iter()
        .find(|card| card.id == "dwarf")
        .unwrap();
    assert_eq!(dwarf.picture.as_deref(), Some("look-species-dwarf.png"));
    assert!(!dwarf.line.is_empty());
    let elf = options
        .species
        .iter()
        .find(|card| card.id == "elf")
        .unwrap();
    assert_eq!(elf.picture, None);
    assert_eq!(options.classes.len(), 12);
}

#[test]
fn a_roll_sets_every_control_within_its_values_with_one_or_two_details() {
    let studio = built_in_studio();
    let controls = studio.controls("tiefling", 1).unwrap().controls.len();
    for seed in 0..64 {
        let mut entropy = crate::adapters::SeededEntropy::new(seed);
        let rolled = studio
            .roll("tiefling", &LookDraft::default(), true, &mut entropy)
            .unwrap();
        assert_eq!(rolled.values.len(), controls);
        assert!((1..=2).contains(&rolled.details.len()));
        assert_eq!(studio.settle("tiefling", &rolled).unwrap(), rolled);
    }
}

#[test]
fn a_refine_rolls_only_the_controls_the_player_left_unset() {
    let studio = built_in_studio();
    let mut draft = LookDraft::default();
    draft
        .values
        .insert("build".to_owned(), ControlValue::Number(4));
    draft.details.push(scar());
    let mut entropy = crate::adapters::SeededEntropy::new(7);
    let rolled = studio.roll("elf", &draft, false, &mut entropy).unwrap();
    assert_eq!(rolled.values["build"], ControlValue::Number(4));
    assert_eq!(rolled.details, vec![scar()]);
    assert_eq!(
        rolled.values.len(),
        studio.controls("elf", 1).unwrap().controls.len()
    );
}
// TODO(DEV-12): an older heroes.json with `sheet`/`ready` loads them into the pack, and an item a
// stopped run left pending reads as missing.
// TODO(DEV-12): the reference route answers a seated hero's pack sheet for its persona id, and
// `ref-<id>.png` for a persona without one.

#[tokio::test]
async fn a_lock_in_whose_portrait_fails_still_locks_the_hero_in_ref_059() {
    let running = start(&STARTER_FILES).await;
    let state = &running.state;
    let key = KeyHash::of("seat-key-for-the-failed-portrait-test");
    let hero = {
        let mut open = state.seats().open().await.unwrap();
        open.books().seat(&key, 1, Holder::Guest, "Ann", 0);
        let card = open
            .books()
            .add_hero(1, "Thorin", "dwarf", "fighter")
            .unwrap();
        open.books().choose_hero(&key, card.id).unwrap();
        card.id
    };
    let studio = state.look().cloned().unwrap();
    let ask = LookLockAsk {
        draft: LookDraft::default(),
    };
    // No base pose is chosen, so no portrait can be made: the hero locks in without one.
    let view = studio.lock(state, 1, ask, true).await.unwrap();
    assert!(matches!(
        view.lock.unwrap().state,
        LockState::Locked { portrait: None, .. }
    ));
    let mut open = state.seats().open().await.unwrap();
    let record = open.books().hero(hero).unwrap();
    assert!(record.locked, "a hero whose portrait failed still plays");
    assert!(matches!(record.pack.portrait, PackStatus::Failed { .. }));
}

#[test]
fn a_detail_jev_did_not_read_never_reaches_a_picture() {
    let studio = built_in_studio();
    let mut record = StudioRecord::default();
    let mut draft = LookDraft::default();
    draft.details.push(scar());
    assert!(matches!(
        check_details(&studio, &record, &draft),
        Err(LookError::InvalidDraft { .. })
    ));
    record.read_details.push(scar());
    assert!(check_details(&studio, &record, &draft).is_ok());
}

#[tokio::test]
async fn a_locked_heros_details_are_read_for_the_sheet_and_the_runner() {
    let running = start(&STARTER_FILES).await;
    let state = &running.state;
    let key = KeyHash::of("seat-key-for-the-hero-look-test");
    let hero = {
        let mut open = state.seats().open().await.unwrap();
        open.books().seat(&key, 1, Holder::Guest, "Ann", 0);
        let card = open
            .books()
            .add_hero(1, "Thorin", "dwarf", "fighter")
            .unwrap();
        open.books().choose_hero(&key, card.id).unwrap();
        card.id
    };
    // The kept version's look is the one locked in.
    let mut draft = built_in_studio()
        .settle("dwarf", &LookDraft::default())
        .unwrap();
    draft.details.push(scar());
    let studio = state.look().cloned().unwrap();
    let kept_draft = draft.clone();
    studio
        .with_record(state, 1, move |record| {
            record.next_version = 2;
            record.hero = Some(hero);
            let mut kept = version(1, VersionState::Ready);
            kept.picture = Some("look-1-v1.png".to_owned());
            kept.draft = kept_draft;
            record.view.versions.push(kept);
            record.view.kept = Some(1);
            Ok(())
        })
        .await
        .unwrap();
    studio
        .lock(state, 1, LookLockAsk { draft }, true)
        .await
        .unwrap();
    let look = super::LookStudio::hero_look(state, 1).await.unwrap();
    assert_eq!(look.hero, hero);
    assert_eq!(
        look.details.as_deref(),
        Some("a jagged scar across the left cheek (scar on the face)")
    );
    assert!(look.look.is_some_and(|look| look.contains("jagged scar")));
    assert!(matches!(
        super::LookStudio::hero_look(state, 9).await,
        Err(LookError::Seat(_))
    ));
}

#[test]
fn the_details_schema_offers_the_studios_controls_and_their_options() {
    let studio = built_in_studio();
    let controls = studio.word_controls("human").unwrap();
    let schema = details_schema(&controls);
    let pair = &schema["properties"]["controls"]["items"]["properties"];
    let ids: Vec<&str> = pair["control"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|id| id.as_str())
        .collect();
    // Every control of the hero but the height slider, from the studio file, presentation first.
    assert_eq!(ids.first(), Some(&"presentation"), "{ids:?}");
    for id in ["hair_colour", "eyes", "age", "hair_length"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    assert!(
        !ids.contains(&"height") && !ids.contains(&"horns"),
        "{ids:?}"
    );
    let options = pair["option"]["enum"].as_array().unwrap();
    for option in ["woman", "white", "grey", "1"] {
        assert!(options.contains(&json!(option)), "{option} in {options:?}");
    }
    // The facts give each control's options with their words.
    let facts = details_facts("words", "species human, class wizard", &controls);
    let presentation = &facts["controls"][0];
    assert_eq!(presentation["control"], "presentation");
    assert_eq!(presentation["options"][0]["words"], "a woman");
    let age = facts["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|control| control["control"] == "age")
        .unwrap();
    assert_eq!(
        age["options"][4],
        json!({"option": "4", "words": "elderly"})
    );
}

#[test]
fn words_set_controls_and_a_pair_the_studio_does_not_have_is_dropped() {
    let controls = built_in_studio().word_controls("human").unwrap();
    let answer = json!({
        "controls": [
            {"control": "presentation", "option": "woman"},
            {"control": "hair_colour", "option": "white"},
            {"control": "eyes", "option": "grey"},
            {"control": "age", "option": "3"},
            {"control": "hair_length", "option": "1"},
            // An option of another control, a step past the end, a control set twice and a
            // species' extra the human does not have.
            {"control": "eyes", "option": "white"},
            {"control": "age", "option": "9"},
            {"control": "presentation", "option": "man"},
            {"control": "horns", "option": "curled"}
        ],
        "details": [
            {"kind": "other", "where": "face", "what": "round wire-rimmed spectacles"},
            {"kind": "other", "where": "right_hand",
             "what": "a tall wooden staff with a small brass lantern hanging from its crook"}
        ],
        "category": "general"
    });
    let read = parse_details(&answer, &controls).unwrap();
    assert_eq!(read.values.len(), 5, "{:?}", read.values);
    assert_eq!(
        read.values.get("presentation"),
        Some(&ControlValue::Choice("woman".to_owned()))
    );
    assert_eq!(
        read.values.get("hair_colour"),
        Some(&ControlValue::Choice("white".to_owned()))
    );
    assert_eq!(read.values.get("age"), Some(&ControlValue::Number(3)));
    assert_eq!(
        read.values.get("hair_length"),
        Some(&ControlValue::Number(1))
    );
    assert_eq!(read.dropped.len(), 4, "{:?}", read.dropped);
    assert!(
        read.dropped
            .contains(&("horns".to_owned(), "curled".to_owned()))
    );
    assert_eq!(read.details.len(), 2);
    // The values the words set settle as a draft of the hero, and reach the description.
    let draft = LookDraft {
        values: read.values,
        details: read.details,
    };
    let studio = built_in_studio();
    let settled = studio.settle("human", &draft).unwrap();
    let public = PublicHero {
        species: "human".to_owned(),
        class: "wizard".to_owned(),
    };
    let description = studio.description(&public, &settled).unwrap();
    for words in [
        "a woman",
        "white hair",
        "grey eyes",
        "older, greying",
        "cropped hair",
    ] {
        assert!(description.contains(words), "{words} in {description}");
    }
    assert!(description.contains("spectacles") && description.contains("lantern"));
}

#[test]
fn the_details_instruction_asks_for_every_visible_thing_and_for_controls_first() {
    for asked in [
        "nothing visible is left out",
        "every accessory or prop worn or carried",
        "spectacles",
        "a staff with a lantern",
        "twin daggers at the hips",
        "brass caps on horns",
        "Words a control covers never also become a detail",
        "put one pair in controls",
    ] {
        assert!(DETAILS_INSTRUCTION.contains(asked), "{asked}");
    }
}
