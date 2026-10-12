use std::path::PathBuf;

use super::*;

const SPEAKER_STEMS: [&str; 8] = [
    "assets/ui/generated/sprite-patron-herbwife",
    "assets/ui/generated/sprite-patron-innkeeper",
    "assets/ui/generated/sprite-contact-watch-sergeant",
    "assets/ui/generated/sprite-ant-guard",
    "assets/ui/generated/sprite-patron-bell-ringer",
    "assets/ui/generated/sprite-contact-tinker",
    "assets/ui/generated/sprite-patron-merchant",
    "assets/ui/generated/sprite-unknown-newcomer",
];

const PARTIES: [&[u8]; 6] = [&[], &[1], &[1, 2], &[1, 2, 3], &[2, 3], &[1, 3]];

fn all_place_kinds() -> Vec<&'static str> {
    vocabulary()
        .unwrap()
        .families
        .values()
        .flat_map(|family| family.table_kinds.iter().map(String::as_str))
        .collect()
}

fn input<'a>(
    kind: &'a str,
    key: SceneKey,
    speaker: Option<&'a str>,
    heroes: &'a [u8],
    text: &'a str,
) -> ShotInput<'a> {
    ShotInput {
        scene_key: key,
        place_id: "place-test",
        place_kind: kind,
        exterior: false,
        title: "A Test Scene",
        story_text: text,
        speaker: speaker.map(|sprite| SpeakerInput {
            sprite,
            gender: Some(Gender::She),
            age: Some(Age::Adult),
        }),
        others: &[],
        heroes,
    }
}

#[test]
fn every_place_kind_scene_key_speaker_and_party_is_staged_validly() {
    let mut staged = 0;
    for kind in all_place_kinds() {
        for key in SceneKey::ALL {
            for speaker in std::iter::once(None).chain(SPEAKER_STEMS.map(Some)) {
                for heroes in PARTIES {
                    let scene = input(kind, key, speaker, heroes, "A quiet evening.");
                    match direct(&scene) {
                        Ok(spec) => {
                            assert_eq!(spec.validate(), Ok(()), "{kind} {key:?} {speaker:?}");
                            assert_eq!(spec.heroes.len(), heroes.len(), "every hero has a slot");
                            staged += 1;
                        }
                        Err(error) => assert!(
                            matches!(error, ShotError::EmptyScene | ShotError::NoStageablePerson),
                            "{kind} {key:?} {speaker:?}: {error:?}"
                        ),
                    }
                }
            }
        }
    }
    assert!(staged > 1000, "the matrix staged {staged} scenes");
}

#[test]
fn the_same_scene_always_gets_the_same_spec_and_other_scenes_can_differ() {
    let heroes = [1, 2, 3];
    let scene = input(
        "inn",
        SceneKey::Opening,
        Some(SPEAKER_STEMS[1]),
        &heroes,
        "x",
    );
    assert_eq!(direct(&scene), direct(&scene));
    let mut seen = std::collections::BTreeSet::new();
    for title in [
        "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight",
    ] {
        let other = ShotInput { title, ..scene };
        seen.insert(format!("{:?}", direct(&other).unwrap()));
    }
    assert!(
        seen.len() > 3,
        "staging varies between scenes: {}",
        seen.len()
    );
}

#[test]
fn every_plate_of_a_place_keeps_its_camera_sides_props_light_and_hour_and_no_hero_holds_another_weapon()
 {
    let heroes = [1, 2];
    let reference = direct(&ShotInput {
        title: "First",
        ..input(
            "lamp-room",
            SceneKey::Opening,
            Some(SPEAKER_STEMS[1]),
            &heroes,
            "x",
        )
    })
    .unwrap();
    for title in ["Second", "Third", "Fourth", "Fifth", "Sixth"] {
        let spec = direct(&ShotInput {
            title,
            ..input(
                "lamp-room",
                SceneKey::Opening,
                Some(SPEAKER_STEMS[1]),
                &heroes,
                "x",
            )
        })
        .unwrap();
        assert_eq!(spec.camera, reference.camera, "{title}");
        assert_eq!(spec.time_of_day, reference.time_of_day, "{title}");
        assert_eq!(spec.lights, reference.lights, "{title}");
        assert_eq!(spec.extras, reference.extras, "{title}");
        assert_eq!(spec.dressing, reference.dressing, "{title}");
        let sides: Vec<(u8, HeroSlot)> = spec.heroes.iter().map(|h| (h.art, h.slot)).collect();
        let before: Vec<(u8, HeroSlot)> =
            reference.heroes.iter().map(|h| (h.art, h.slot)).collect();
        assert_eq!(sides, before, "the heroes keep their sides: {title}");
        for hero in &spec.heroes {
            assert!(
                !handles_a_weapon(hero.activity),
                "{title}: {}",
                hero.activity
            );
        }
    }
    assert!(
        !reference.heroes[0].slot.is_right() && reference.heroes[1].slot.is_right(),
        "the first hero stands on the left and the second on the right"
    );
}

#[test]
fn no_two_heroes_share_a_slot_and_nobody_stands_where_the_speaker_does() {
    let heroes = [1, 2, 3];
    for title in ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"] {
        let scene = ShotInput {
            title,
            ..input(
                "mill-house",
                SceneKey::DialogueSeal,
                Some(SPEAKER_STEMS[5]),
                &heroes,
                "x",
            )
        };
        let spec = direct(&scene).unwrap();
        let slots: Vec<HeroSlot> = spec.heroes.iter().map(|hero| hero.slot).collect();
        for (index, slot) in slots.iter().enumerate() {
            assert!(!slots[..index].contains(slot), "{title}: {slots:?}");
        }
        let speaker = spec.speaker.unwrap();
        assert!(
            !slots.contains(&speaker.slot.blocked_hero_slot()),
            "{title}"
        );
    }
}

#[test]
fn the_speaker_is_placed_for_the_bubble_with_heads_below_the_top_band_and_inside_the_frame() {
    for slot in [SpeakerSlot::MidLeft, SpeakerSlot::MidRight] {
        for camera in [
            Camera::OverShoulderLeft,
            Camera::OverShoulderRight,
            Camera::WideEstablishing,
            Camera::SpeakerClose,
        ] {
            let layout = layout_for(slot, camera);
            let head = layout.head_box();
            let calm = layout.calm_box();
            assert!(head.top >= 280, "heads stay below the top 28%: {head:?}");
            assert!(head.left >= 0 && head.right <= 1000 && head.bottom <= 1000);
            assert!(
                calm.left >= 0 && calm.right <= 1000 && calm.top >= 0,
                "{calm:?}"
            );
            assert_eq!(layout.bubble_side, slot.side().opposite());
            assert!(!head.intersects(calm), "the calm space is beside the head");
        }
    }
}

#[test]
fn a_measured_head_gives_the_layout_with_the_bubble_on_the_side_of_the_middle() {
    let right = measured_layout(0.328, 0.25, 0.396, 0.413);
    assert_eq!(right.bubble_side, Side::Right, "a head left of the middle");
    assert_eq!((right.head_x_permille, right.head_y_permille), (362, 332));
    assert_eq!(
        right.head_width_permille, 90,
        "a small head still gets the least box"
    );
    let left = measured_layout(0.62, 0.2, 0.72, 0.4);
    assert_eq!(left.bubble_side, Side::Left);
    assert!(left.head_box().left < 620 && left.head_box().right > 720);
    let huge = measured_layout(0.0, 0.0, 1.0, 1.0);
    assert_eq!(
        (huge.head_width_permille, huge.head_height_permille),
        (240, 300)
    );
}

#[test]
fn the_head_is_asked_for_lower_than_it_is_expected_to_land_than_it_is_expected_to_land() {
    let layout = layout_for(SpeakerSlot::MidRight, Camera::OverShoulderLeft);
    let (x, y) = asked_head(&layout);
    assert_eq!((x, y), (700, 560));
    assert_eq!(layout.head_y_permille, 380);
}

#[test]
fn a_creature_speaker_an_empty_scene_and_an_unknown_place_are_not_staged() {
    let heroes = [1];
    let creature = input(
        "inn",
        SceneKey::Opening,
        Some("assets/ui/generated/sprite-ant-giant-rat"),
        &heroes,
        "x",
    );
    assert_eq!(direct(&creature), Err(ShotError::NoStageablePerson));
    assert_eq!(
        direct(&input("inn", SceneKey::Opening, None, &[], "x")),
        Err(ShotError::EmptyScene)
    );
    assert_eq!(
        direct(&input(
            "no-such-place",
            SceneKey::Opening,
            None,
            &heroes,
            "x"
        )),
        Err(ShotError::UnknownPlaceKind)
    );
}

#[test]
fn hero_numbers_outside_the_art_are_ignored_and_a_party_is_capped() {
    let spec = direct(&input(
        "inn",
        SceneKey::Opening,
        None,
        &[0, 1, 2, 3, 9],
        "x",
    ))
    .unwrap();
    let mut arts: Vec<u8> = spec.heroes.iter().map(|hero| hero.art).collect();
    arts.sort_unstable();
    assert_eq!(arts, [1, 2, 3]);
}

#[test]
fn story_cues_pick_the_time_of_day_and_the_expression() {
    let heroes = [1];
    let night = direct(&input(
        "inn",
        SceneKey::Opening,
        Some(SPEAKER_STEMS[0]),
        &heroes,
        "Someone must hurry before dawn, please.",
    ))
    .unwrap();
    assert_eq!(night.time_of_day, TimeOfDay::Dawn);
    assert_eq!(night.speaker.unwrap().expression, Expression::Anxious);
    assert_eq!(night.mood, Mood::Urgent);
}

/// The saved adventures of this checkout (and, in a worktree, of the main checkout). Absent on a
/// fresh clone, where only the synthetic matrix above runs.
fn saved_packs() -> Vec<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut packs = Vec::new();
    for ancestor in manifest.ancestors().take(7) {
        let runs = ancestor.join("development/runs");
        let Ok(entries) = std::fs::read_dir(&runs) else {
            continue;
        };
        for entry in entries.flatten() {
            let pack = entry.path().join("pack.json");
            if pack.is_file() {
                packs.push(pack);
            }
        }
        if !packs.is_empty() {
            break;
        }
    }
    packs
}

#[test]
fn every_scene_of_every_saved_adventure_is_staged_validly_for_every_party() {
    let mut scenes_staged = 0;
    for path in saved_packs() {
        let text = std::fs::read_to_string(&path).unwrap();
        let pack: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut scenes: Vec<&serde_json::Value> =
            pack["scenes"].as_array().unwrap().iter().collect();
        if let Some(script) = pack["beat_script"]["scenes"].as_object() {
            scenes.extend(script.values());
        }
        for scene in scenes {
            let Some(kind) = scene["setting"]["kind"].as_str() else {
                continue;
            };
            let key: SceneKey = serde_json::from_value(scene["key"].clone()).unwrap();
            let speaker_name = scene["speaker"].as_str().unwrap_or_default();
            let cast = scene["cast"].as_array().cloned().unwrap_or_default();
            let speaker = cast
                .iter()
                .find(|member| !speaker_name.is_empty() && member["name"] == speaker_name);
            let others: Vec<&str> = cast
                .iter()
                .filter(|member| Some(*member) != speaker)
                .filter_map(|member| member["sprite"].as_str())
                .collect();
            let story = format!(
                "{} {}",
                scene["narration"].as_str().unwrap_or_default(),
                scene["dialogue"].as_str().unwrap_or_default()
            );
            for heroes in PARTIES {
                let scene = ShotInput {
                    scene_key: key,
                    place_id: scene["setting"]["place_id"].as_str().unwrap(),
                    place_kind: kind,
                    exterior: scene["setting"]["exterior"].as_bool().unwrap_or(false),
                    title: scene["title"].as_str().unwrap(),
                    story_text: &story,
                    speaker: speaker.map(|member| SpeakerInput {
                        sprite: member["sprite"].as_str().unwrap(),
                        gender: None,
                        age: None,
                    }),
                    others: &others,
                    heroes,
                };
                match direct(&scene) {
                    Ok(spec) => {
                        assert_eq!(spec.validate(), Ok(()), "{path:?} {}", scene.title);
                        scenes_staged += 1;
                    }
                    Err(error) => assert!(
                        matches!(error, ShotError::EmptyScene | ShotError::NoStageablePerson),
                        "{path:?} {}: {error:?}",
                        scene.title
                    ),
                }
            }
        }
    }
    println!("saved-adventure scenes staged: {scenes_staged}");
}

#[test]
fn the_embedded_vocabulary_is_complete_for_every_family() {
    let vocabulary = vocabulary().unwrap();
    assert_eq!(vocabulary.families.len(), 11);
    for (name, family) in &vocabulary.families {
        assert!(
            family
                .speaker_tasks
                .iter()
                .any(|task| task.tags.iter().any(|tag| tag == "any")),
            "{name}"
        );
        assert!(
            family
                .activities
                .iter()
                .any(|a| a.tags.iter().any(|tag| tag == "any")),
            "{name}"
        );
        assert!(
            family
                .activities
                .iter()
                .any(|a| a.tags.iter().any(|tag| tag == "fighter")),
            "{name}"
        );
    }
}
