use super::{
    super::{
        super::{
            pack::{Age, Gender, SceneKey},
            scene_mood::{Expression as FaceExpression, HealthTier},
            shot::{self, ShotInput, SpeakerInput, vocabulary_phrases},
        },
        prompt::{Part, denied},
    },
    *,
};

const KINDS: [&str; 11] = [
    "inn",
    "temple",
    "archive",
    "herb-shed",
    "mill-house",
    "market-street",
    "quay",
    "farmyard",
    "forest-trail",
    "guardhouse",
    "bindery",
];
const SPEAKERS: [Option<&str>; 6] = [
    None,
    Some("assets/ui/generated/sprite-patron-herbwife"),
    Some("assets/ui/generated/sprite-patron-innkeeper"),
    Some("assets/ui/generated/sprite-contact-watch-sergeant"),
    Some("assets/ui/generated/sprite-contact-tinker"),
    Some("assets/ui/generated/sprite-patron-bell-ringer"),
];
const PARTIES: [&[u8]; 4] = [&[], &[2], &[1, 3], &[1, 2, 3]];
const OTHERS: [&str; 1] = ["assets/ui/generated/sprite-ant-guard"];

fn spec_for(
    kind: &str,
    key: SceneKey,
    speaker: Option<&str>,
    heroes: &[u8],
    title: &str,
) -> Option<ShotSpec> {
    shot::direct(&ShotInput {
        scene_key: key,
        place_id: "place-x",
        place_kind: kind,
        exterior: false,
        title,
        story_text: "A night of lamplight.",
        speaker: speaker.map(|sprite| SpeakerInput {
            sprite,
            gender: Some(Gender::She),
            age: Some(Age::Elder),
        }),
        others: &OTHERS,
        heroes,
    })
    .ok()
}

fn place() -> PlaceFacts {
    PlaceFacts::new("The Heron Shrine", "a shrine beside the marsh").unwrap()
}

fn references_for(spec: &ShotSpec) -> PlateReferences {
    PlateReferences {
        party_sheet: !spec.heroes.is_empty(),
        speaker: spec.speaker.is_some(),
        speaker_face: spec.speaker.is_some(),
        place: true,
    }
}

fn allowed(rating: ContentRating) -> Vec<&'static str> {
    let mut allowed = closed_vocabulary(rating);
    allowed.extend(vocabulary_phrases());
    allowed
}

#[test]
fn a_plate_prompt_is_built_from_fixed_phrases_and_two_kinds_of_fact_only_within_the_adapter_limit()
{
    let mut longest = 0;
    let mut built = 0;
    for rating in [ContentRating::Mature, ContentRating::Restrained] {
        let allowed = allowed(rating);
        for kind in KINDS {
            for key in [
                SceneKey::Opening,
                SceneKey::DialogueSeal,
                SceneKey::Inn,
                SceneKey::Victory,
            ] {
                for speaker in SPEAKERS {
                    for heroes in PARTIES {
                        for title in ["One", "Two", "Three"] {
                            let Some(spec) = spec_for(kind, key, speaker, heroes, title) else {
                                continue;
                            };
                            let prompt =
                                scene_plate(rating, &spec, Some(&place()), references_for(&spec))
                                    .unwrap();
                            assert!(prompt.as_str().contains("named film grade"));
                            assert!(prompt.as_str().contains("fine film grain"));
                            assert!(!prompt.as_str().contains("repaint"));
                            for part in prompt.parts() {
                                match part {
                                    Part::Fixed(phrase) => assert!(
                                        allowed.contains(phrase),
                                        "{kind} {key:?}: a phrase outside the closed vocabulary: {phrase}"
                                    ),
                                    Part::Fact(fact) => assert!(
                                        fact == "The Heron Shrine:"
                                            || fact.chars().all(|digit| digit.is_ascii_digit()),
                                        "a fact that is neither the place name nor a position: {fact}"
                                    ),
                                }
                            }
                            let length = prompt.as_str().chars().count();
                            assert!(length <= MAXIMUM_PROMPT_CHARACTERS, "{length} characters");
                            longest = longest.max(length);
                            built += 1;
                        }
                    }
                }
            }
        }
    }
    println!("plate prompts built: {built}, longest {longest} characters");
    assert!(built > 1500);
}

#[test]
fn no_plate_prompt_names_a_child_a_brand_or_a_real_person() {
    for kind in KINDS {
        for speaker in SPEAKERS {
            let Some(spec) = spec_for(kind, SceneKey::Opening, speaker, &[1, 2, 3], "T") else {
                continue;
            };
            let prompt = scene_plate(
                ContentRating::Mature,
                &spec,
                Some(&place()),
                references_for(&spec),
            )
            .unwrap();
            let lowered = prompt.as_str().to_ascii_lowercase();
            for word in lowered.split(|character: char| !character.is_ascii_alphabetic()) {
                // "not a real person"-style negations are the prompt's own limits; the denied
                // list is for names, instructions and anyone younger than an adult.
                assert!(
                    !denied(word) || ["system", "apple"].contains(&word),
                    "{kind}: denied word {word}"
                );
            }
            assert!(lowered.contains("every person is clearly an adult"));
        }
    }
}

#[test]
fn the_whole_closed_vocabulary_has_no_word_for_anyone_younger_than_an_adult() {
    for phrase in allowed(ContentRating::Mature) {
        for word in phrase
            .to_ascii_lowercase()
            .split(|character: char| !character.is_ascii_alphabetic())
        {
            assert!(
                !denied(word) || word == "system",
                "{phrase:?} contains {word}"
            );
            assert!(
                ![
                    "apprentice",
                    "novice",
                    "student",
                    "youth",
                    "lad",
                    "juvenile"
                ]
                .contains(&word),
                "{phrase:?} contains {word}"
            );
        }
    }
}

#[test]
fn the_references_are_numbered_in_the_order_they_are_sent() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[1, 2], "T").unwrap();
    let all = scene_plate(
        ContentRating::Mature,
        &spec,
        None,
        PlateReferences {
            party_sheet: true,
            speaker: true,
            speaker_face: false,
            place: true,
        },
    )
    .unwrap();
    let text = all.as_str();
    assert!(text.contains("Image 1 is the party sheet"));
    assert!(text.contains("Image 2 shows this person"));
    assert!(text.contains("Image 3 is the place reference"));
    let without_place = scene_plate(
        ContentRating::Mature,
        &spec,
        None,
        PlateReferences {
            party_sheet: true,
            speaker: false,
            speaker_face: false,
            place: true,
        },
    )
    .unwrap();
    assert!(
        without_place
            .as_str()
            .contains("Image 2 is the place reference")
    );
    assert!(!without_place.as_str().contains("shows this person"));
}

#[test]
fn a_party_without_a_party_sheet_or_too_many_references_is_refused() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[1, 2], "T").unwrap();
    assert_eq!(
        scene_plate(
            ContentRating::Mature,
            &spec,
            None,
            PlateReferences {
                party_sheet: false,
                speaker: true,
                speaker_face: false,
                place: true,
            }
        )
        .unwrap_err(),
        PlatePromptRefusal::References
    );
}

#[test]
fn the_heroes_are_named_by_their_place_on_the_sheet_and_the_speaker_by_the_speaker_slot() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[1, 2, 3], "T").unwrap();
    let text = scene_plate(
        ContentRating::Mature,
        &spec,
        Some(&place()),
        references_for(&spec),
    )
    .unwrap()
    .as_str()
    .to_owned();
    for needle in [
        "first figure on the sheet",
        "second figure on the sheet",
        "third figure on the sheet",
        "no wider than a quarter of the frame",
        "All heads below the top 30%",
        "Directly",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
    assert_eq!(sheet_position(&spec.heroes, 3), 2);
    assert_eq!(sheet_position(&spec.heroes, 1), 0);
}

#[test]
fn a_mood_edit_names_the_previous_plate_the_change_and_what_stays_in_both_ratings() {
    for rating in [ContentRating::Mature, ContentRating::Restrained] {
        for expression in FaceExpression::ALL {
            for tier in HealthTier::ALL {
                let prompt = plate_moment(rating, expression, tier, true).unwrap();
                let text = prompt.as_str();
                assert!(text.contains("named film grade"));
                assert!(text.contains("fine film grain"));
                assert!(!text.contains("repaint"));
                assert!(text.starts_with("Image 1 is the previous film still"));
                assert!(text.contains("poses and small actions move on naturally"));
                assert!(text.contains(
                    "same place, cast, faces, hair, clothing, weapons and photographic art style"
                ));
                assert!(!text.contains(
                    "camera position, framing, light, props, extras and all people exactly"
                ));
                assert!(text.contains("speaker's head stays roughly in"));
                assert!(text.chars().count() < 1500);
            }
        }
    }
}

#[test]
fn the_speakers_figure_and_face_both_lead_the_references_and_the_place_moves_up() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[1, 2], "T").unwrap();
    let text = scene_plate(
        ContentRating::Mature,
        &spec,
        None,
        PlateReferences {
            party_sheet: true,
            speaker: true,
            speaker_face: true,
            place: true,
        },
    )
    .unwrap()
    .as_str()
    .to_owned();
    for needle in [
        "Image 1 is the party sheet",
        "Image 2 shows this person",
        "Image 3 is a close portrait of their face",
        "Image 4 is the place reference",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
}
