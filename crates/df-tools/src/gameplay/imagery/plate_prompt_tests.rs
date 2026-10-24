use super::{
    super::{
        super::{
            hero_identity::{Identity, ValidatedIdentity},
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
    Some("assets/ui/generated/sprite-patron-merchant-documentary-photo"),
    Some("assets/ui/generated/sprite-shop-adventurer-documentary-photo"),
    Some("assets/ui/generated/sprite-shop-soldier-documentary-photo"),
    Some("assets/ui/generated/sprite-courier-documentary-photo"),
    Some("assets/ui/generated/sprite-bandit-documentary-photo"),
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

fn hero_identity(choices: &[(&str, &str)], backstory: &str) -> ValidatedIdentity {
    let mut identity = Identity::from_menus(choices)
        .expect("valid menu choices")
        .identity()
        .clone();
    identity.backstory = backstory.to_owned();
    identity.validate().expect("the checked hero identity")
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
                            assert!(prompt.as_str().contains("Documentary color photograph"));
                            assert!(prompt.as_str().contains("fine film grain"));
                            assert!(prompt.as_str().contains(
                                "Supplied reference photographs fix identity, layout and costume"
                            ));
                            assert!(prompt.as_str().contains(
                                "ordinary human skin, practical garments and natural camera finish"
                            ));
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
fn a_party_sheet_without_heroes_is_refused() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[], "T").unwrap();
    assert_eq!(
        scene_plate(
            ContentRating::Mature,
            &spec,
            None,
            PlateReferences {
                party_sheet: true,
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
fn a_plate_can_stage_heroes_without_painted_references() {
    let spec = spec_for("inn", SceneKey::Opening, None, &[1, 2], "T").unwrap();
    let prompt = scene_plate(
        ContentRating::Mature,
        &spec,
        None,
        PlateReferences {
            party_sheet: false,
            speaker: false,
            speaker_face: false,
            place: false,
        },
    )
    .expect("a photographic plate can omit painted references");
    assert!(
        prompt
            .as_str()
            .contains("No photographic hero reference is available")
    );
    assert!(prompt.as_str().contains("the staged hero in this slot"));
    assert!(!prompt.as_str().contains("Image 1 is the party sheet"));
    assert!(prompt.as_str().chars().count() <= MAXIMUM_PROMPT_CHARACTERS);
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
        "Image 2 shows this person",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
    assert_eq!(sheet_position(&spec.heroes, 3), 2);
    assert_eq!(sheet_position(&spec.heroes, 1), 0);
}

#[test]
fn a_party_plate_does_not_impose_the_stock_dwarf_appearances() {
    let spec = spec_for("inn", SceneKey::Opening, None, &[1, 2, 3], "T").unwrap();
    let text = scene_plate(
        ContentRating::Mature,
        &spec,
        Some(&place()),
        references_for(&spec),
    )
    .unwrap()
    .as_str()
    .to_owned();

    assert!(text.contains("Keep each hero's exact face, hair, build, costume and weapon"));
    assert!(!text.contains("red-bearded"));
    assert!(!text.contains("red-haired adult dwarf"));
    assert!(!text.contains("blonde-braided adult dwarf"));
}

#[test]
fn a_two_hero_plate_is_wider_with_heroes_facing_each_other_and_a_lone_hero_is_unchanged() {
    let two = spec_for("inn", SceneKey::DialogueSeal, SPEAKERS[2], &[1, 2], "T").unwrap();
    let two_text = scene_plate(
        ContentRating::Mature,
        &two,
        Some(&place()),
        references_for(&two),
    )
    .unwrap()
    .as_str()
    .to_owned();
    assert!(
        two_text.contains("Camera pulled about 5 feet (1.5 metres) further back for a wider shot"),
        "{two_text}"
    );
    assert!(
        two_text.contains("facing each other as if talking to each other"),
        "{two_text}"
    );
    assert!(
        two_text.contains("not all turned toward the speaker"),
        "{two_text}"
    );
    assert!(
        !two_text.contains("three-quarter back, all turned"),
        "{two_text}"
    );

    let one = spec_for("inn", SceneKey::DialogueSeal, SPEAKERS[2], &[2], "T").unwrap();
    let one_text = scene_plate(
        ContentRating::Mature,
        &one,
        Some(&place()),
        references_for(&one),
    )
    .unwrap()
    .as_str()
    .to_owned();
    assert!(
        one_text.contains("Camera pulled about 5 feet (1.5 metres) further back for a wider shot"),
        "{one_text}"
    );
    assert!(
        one_text.contains("seen from behind or three-quarter back, turned toward"),
        "{one_text}"
    );
    assert!(!one_text.contains("facing each other"), "{one_text}");

    let flux = flux_scene_plate(
        ContentRating::Mature,
        Camera::DialogueMedium,
        Some(&place()),
        &FluxCast {
            sheet: vec![
                FluxPerson {
                    name: "Brakka".into(),
                    appearance: None,
                },
                FluxPerson {
                    name: "Sigrun".into(),
                    appearance: None,
                },
            ],
            speaker: None,
            portrait: false,
        },
        "{}",
        None,
    )
    .unwrap();
    assert!(
        flux.as_str()
            .contains("Camera pulled about 5 feet (1.5 m) further back for a wider shot"),
        "{}",
        flux.as_str()
    );
    assert!(
        flux.as_str().contains("heroes face each other"),
        "{}",
        flux.as_str()
    );
}

#[test]
fn a_party_plate_stages_each_checked_identity_in_sheet_order_without_backstory() {
    let spec = spec_for("inn", SceneKey::Opening, None, &[1, 2], "T").unwrap();
    let first = hero_identity(
        &[
            ("presentation", "feminine"),
            ("age", "elder"),
            ("build", "slender"),
            ("skin", "olive"),
            ("hair", "braids"),
            ("hair-color", "grey"),
            ("eyes", "green"),
            ("mark", "scar"),
            ("attire", "red-cloak"),
        ],
        "I keep a violet tin beneath the west bridge.",
    );
    let second = hero_identity(
        &[
            ("presentation", "masculine"),
            ("age", "middle-aged"),
            ("build", "broad"),
            ("skin", "deep-brown"),
            ("hair", "shaved"),
            ("eyes", "amber"),
            ("attire", "plain-robes"),
        ],
        "I once hid a copper compass in the old mill.",
    );
    let heroes = [
        HeroVisualIdentity {
            identity: &first,
            species: "dwarf",
            held_gear: &["a longbow"],
        },
        HeroVisualIdentity {
            identity: &second,
            species: "human",
            held_gear: &["a longsword"],
        },
    ];
    let text = scene_plate_for_heroes(
        ContentRating::Mature,
        &spec,
        Some(&place()),
        references_for(&spec),
        &heroes,
    )
    .unwrap()
    .as_str()
    .to_owned();

    for detail in [
        "Hero 1 (first figure on the sheet)",
        "Hero 2 (second figure on the sheet)",
        "feminine presentation",
        "olive skin",
        "red cloak",
        "canonical equipment: a longbow",
        "masculine presentation",
        "deep brown skin",
        "shaved head",
        "plain woven robes",
        "canonical equipment: a longsword",
        "at the left side of the frame",
        "at the right side of the frame",
    ] {
        assert!(text.contains(detail), "missing {detail:?} in {text}");
    }
    assert!(!text.contains("violet tin"));
    assert!(!text.contains("copper compass"));
    assert!(!text.contains("red-bearded"));
}

#[test]
fn a_plate_refuses_a_hero_identity_list_that_does_not_match_the_cast() {
    let spec = spec_for("inn", SceneKey::Opening, None, &[1, 2], "T").unwrap();
    let identity = hero_identity(&[], "A short backstory.");
    let heroes = [HeroVisualIdentity {
        identity: &identity,
        species: "dwarf",
        held_gear: &[],
    }];

    assert_eq!(
        scene_plate_for_heroes(
            ContentRating::Mature,
            &spec,
            Some(&place()),
            references_for(&spec),
            &heroes,
        )
        .unwrap_err(),
        PlatePromptRefusal::HeroIdentity
    );
}

#[test]
fn a_mood_edit_names_the_previous_plate_the_change_and_what_stays_in_both_ratings() {
    for rating in [ContentRating::Mature, ContentRating::Restrained] {
        for expression in FaceExpression::ALL {
            for tier in HealthTier::ALL {
                let prompt = plate_moment(rating, expression, tier, true).unwrap();
                let text = prompt.as_str();
                assert!(text.contains("Documentary color photograph"));
                assert!(text.contains("fine film grain"));
                assert!(!text.contains("repaint"));
                // The photographic direction and the one-frame rule lead an edit, too.
                assert!(text.starts_with("A still frame from a live-action fantasy TV drama"));
                assert!(text.contains("ONE single continuous photographic camera frame"));
                assert!(text.contains("Image 1 is the previous film still"));
                assert!(text.contains("poses and small actions move on naturally"));
                assert!(text.contains(
                    "same place, cast, faces, hair, clothing, weapons and photographic art style"
                ));
                assert!(!text.contains(
                    "camera position, framing, light, props, extras and all people exactly"
                ));
                assert!(text.contains("speaker's head stays roughly in"));
                // The bound grew with the strengthened photographic direction and the
                // one-frame rule; the adapter limit remains the hard cap.
                assert!(text.chars().count() < 2000, "{}", text.chars().count());
                assert!(text.chars().count() <= MAXIMUM_PROMPT_CHARACTERS);
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

#[test]
fn a_flux_plate_names_its_people_in_order_and_trims_long_text_to_the_limit() {
    let long = "a long one-line description of a person in a worn costume, ".repeat(12);
    let cast = FluxCast {
        sheet: vec![
            FluxPerson {
                name: "Marek".into(),
                appearance: Some(long.clone()),
            },
            FluxPerson {
                name: "Jessa".into(),
                appearance: Some(long.clone()),
            },
            FluxPerson {
                name: "Corin".into(),
                appearance: None,
            },
        ],
        speaker: Some(FluxPerson {
            name: "Jessa".into(),
            appearance: None,
        }),
        portrait: true,
    };
    let facts = "{\"id\": \"shop\", \"item\": \"blue linen\"}, ".repeat(300);
    let prompt = flux_scene_plate(
        ContentRating::Restrained,
        Camera::SpeakerClose,
        Some(&place()),
        &cast,
        &facts,
        None,
    )
    .expect("the FLUX plate prompt");
    let text = prompt.as_str();
    assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
    // The photographic direction leads the lean prompt; the JSON facts are what gets trimmed.
    assert!(text.starts_with("Photographic still from a live-action fantasy TV drama"));
    assert!(text.contains("ONE single continuous camera frame only"));
    assert!(text.contains("Close shot on a 75mm lens at eye level on Jessa, the speaker"));
    let order = [
        "Image 1 is the place photograph",
        "Image 2, photograph 1: Marek,",
        "Image 2, photograph 2: Jessa,",
        "Image 2, photograph 3: Corin \u{2014}",
        "Image 3 is a close portrait of Jessa",
        "The place is The Heron Shrine.",
        "Only the listed people and things exist.",
        "The scene as it is now (JSON):",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|needle| {
            text.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    assert!(
        !text.contains(&long),
        "the appearance is cut to its opening"
    );
}

#[test]
fn a_lean_flux_plate_keeps_complete_cast_identities_and_preindustrial_setting() {
    let cast = FluxCast {
        sheet: vec![
            FluxPerson {
                name: "Brakka".into(),
                appearance: Some("Brakka wears a travel cloak.".into()),
            },
            FluxPerson {
                name: "Sigrun".into(),
                appearance: Some("Sigrun wears a tunic.".into()),
            },
            FluxPerson {
                name: "Marek".into(),
                appearance: Some("Marek, a short, round middle-aged man with a neatly trimmed beard, wears a rich velvet cap and a measuring ribbon around his neck as he folds blue linen behind the counter.".into()),
            },
            FluxPerson {
                name: "Corin".into(),
                appearance: Some("Corin examines a jacket seam.".into()),
            },
            FluxPerson {
                name: "Jessa".into(),
                appearance: Some("Jessa compares warm cloaks.".into()),
            },
        ],
        speaker: Some(FluxPerson {
            name: "Corin".into(),
            appearance: None,
        }),
        portrait: true,
    };
    let facts = "{\"people\":[\"Marek\",\"Jessa\"]}, ".repeat(120);
    let prompt = flux_scene_plate(
        ContentRating::Mature,
        Camera::WideEstablishing,
        Some(&place()),
        &cast,
        &facts,
        Some((FaceExpression::Neutral, HealthTier::Healthy)),
    )
    .expect("the lean FLUX plate prompt");
    let text = prompt.as_str();

    assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
    assert!(text.starts_with("Photographic still from a live-action fantasy TV drama"));
    assert!(text.contains("Wide establishing shot on a 28mm lens"));
    assert!(text.contains(
        "Pre-industrial fantasy world: period clothing only, no modern dress or objects."
    ));
    for (cell, name) in ["Brakka", "Sigrun", "Marek", "Corin", "Jessa"]
        .into_iter()
        .enumerate()
    {
        assert!(text.contains(&format!("Image 2, photograph {}: {name}", cell + 1)));
    }
    assert!(text.contains("The place is The Heron Shrine."));
    // Appearance state is shortened before the art direction: Marek keeps his opening clause.
    assert!(text.contains("Marek, a short, round middle-aged man with a neatly trimmed beard."));
    assert!(!text.contains("measuring ribbon"));
    assert!(text.contains("The speaker's face shows a calm, neutral expression."));
    assert!(!text.contains("cell "));
}

#[test]
fn every_plate_prompt_leads_with_the_photographic_direction_and_one_frame_rule() {
    let spec = spec_for("inn", SceneKey::Opening, SPEAKERS[2], &[1, 2], "T").unwrap();
    let plate = scene_plate(
        ContentRating::Mature,
        &spec,
        Some(&place()),
        references_for(&spec),
    )
    .unwrap();
    let text = plate.as_str();
    assert!(text.starts_with("A still frame from a live-action fantasy TV drama"));
    for needle in [
        "real camera with a prime lens",
        "skin pores",
        "woven fabric",
        "one single continuous photographic camera frame",
        "one continuous camera frame",
        "no grid",
        "no panels",
        "no numbers",
        "no text",
        "signage",
        "no letters",
        "separate full-body identity photographs side by side",
        "never copy the sheet's side-by-side layout",
    ] {
        assert!(
            text.to_ascii_lowercase().contains(needle),
            "plate missing {needle:?} in {text}"
        );
    }
    let lean = flux_scene_plate(
        ContentRating::Mature,
        Camera::DialogueMedium,
        Some(&place()),
        &FluxCast {
            sheet: vec![FluxPerson {
                name: "Marek".into(),
                appearance: None,
            }],
            speaker: None,
            portrait: false,
        },
        "{}",
        None,
    )
    .unwrap();
    assert!(lean.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
    assert!(lean.as_str().starts_with("Photographic still"));
    assert!(
        lean.as_str()
            .contains("ONE single continuous camera frame only")
    );
}

#[test]
fn every_fixed_look_names_only_the_photograph_and_no_other_medium() {
    // Naming another medium, even to forbid it, steers the model toward it (owner, 2026-10-17).
    for (name, text) in [
        ("FILM_LOOK", super::FILM_LOOK),
        ("SINGLE_FRAME", super::SINGLE_FRAME),
        ("NEGATIVE", NEGATIVE),
        ("FLUX_LOOK", FLUX_LOOK),
        ("FLUX_SHEET_IMAGE", FLUX_SHEET_IMAGE),
        ("ONE_FRAME_EDIT", ONE_FRAME_EDIT),
    ] {
        let lower = text.to_ascii_lowercase();
        for style_word in super::super::NON_PHOTOGRAPHIC_WORDS {
            assert!(
                !lower.contains(&style_word.to_ascii_lowercase()),
                "{name} names {style_word:?}: {text}"
            );
        }
    }
}

#[test]
fn a_six_hero_plate_numbers_every_sheet_position_and_keeps_four_reference_roles() {
    let spec = spec_for(
        "inn",
        SceneKey::DialogueSeal,
        SPEAKERS[2],
        &[1, 2, 3, 4, 5, 6],
        "Six heroes",
    )
    .unwrap();
    assert_eq!(spec.heroes.len(), 6);
    let prompt = scene_plate(
        ContentRating::Restrained,
        &spec,
        Some(&place()),
        PlateReferences {
            party_sheet: true,
            speaker: true,
            speaker_face: true,
            place: true,
        },
    )
    .unwrap();
    let text = prompt.as_str();
    for (index, position) in SHEET_POSITION.iter().enumerate() {
        assert!(
            text.contains(&format!("Hero {} {position}", index + 1)),
            "{text}"
        );
    }
    assert!(text.contains("two rows of three"));
    assert!(text.contains("two loose groups facing each other"));
    assert!(text.contains("5 feet"));
    assert!(text.contains("Image 4 is the place reference"));
    assert!(text.contains("Image 3 is a close portrait"));
    assert!(prompt.character_count() <= MAXIMUM_PROMPT_CHARACTERS);
}

#[test]
fn a_flux_six_hero_and_three_npc_sheet_keeps_every_identity_within_the_lean_budget() {
    let mut sheet: Vec<FluxPerson> = (1..=6)
        .map(|index| FluxPerson {
            name: format!("Hero {index} ({})", "A".repeat(32)),
            appearance: Some("A complete accepted appearance description, ".repeat(20)),
        })
        .collect();
    sheet.extend((1..=3).map(|index| FluxPerson {
        name: format!("Contact {index} {}", "N".repeat(22)),
        appearance: None,
    }));
    let cast = FluxCast {
        sheet,
        speaker: Some(FluxPerson {
            name: "Contact 1".into(),
            appearance: None,
        }),
        portrait: true,
    };
    for tier in [HealthTier::Healthy, HealthTier::Critical, HealthTier::Down] {
        let prompt = flux_scene_plate(
            ContentRating::Mature,
            Camera::WideEstablishing,
            Some(&place()),
            &cast,
            &"facts ".repeat(100),
            Some((FaceExpression::Neutral, tier)),
        )
        .unwrap();
        for (index, person) in cast.sheet.iter().enumerate() {
            assert!(prompt.as_str().contains(&format!(
                "Photograph {}: {}",
                index + 1,
                person.name
            )));
            assert!(prompt.as_str().contains(&format!(
                "Photograph {}: {}; both hands empty.",
                index + 1,
                person.name
            )));
        }
        assert!(
            prompt
                .as_str()
                .contains("two loose groups facing each other")
        );
        assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
    }
}

#[test]
fn four_and_five_long_named_heroes_also_fit_the_lean_prompt_without_losing_identity() {
    for count in [4, 5] {
        let cast = FluxCast {
            sheet: (1..=count).map(|index| FluxPerson {
                name: format!("Hero {index} ({})", "A".repeat(32)),
                appearance: Some("An adult performer, in their exact accepted clothes and carrying their own gear, with distinctive facial features, ".repeat(8)),
            }).collect(),
            speaker: Some(FluxPerson {name: "A patient courier".into(), appearance: None}),
            portrait: true,
        };
        let prompt = flux_scene_plate(
            ContentRating::Mature,
            Camera::WideEstablishing,
            Some(&place()),
            &cast,
            "{}",
            Some((FaceExpression::Neutral, HealthTier::Critical)),
        )
        .unwrap();
        for (index, person) in cast.sheet.iter().enumerate() {
            assert!(prompt.as_str().contains(&format!(
                "Photograph {}: {}",
                index + 1,
                person.name
            )));
        }
        assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
    }
}

#[test]
fn plate_prompt_names_exactly_the_seated_hero_count() {
    for count in [1, 2, 4, 6] {
        let heroes: Vec<u8> = (1..=count).collect();
        let spec = spec_for("inn", SceneKey::DialogueSeal, SPEAKERS[2], &heroes, "Party").unwrap();
        let prompt = scene_plate(
            ContentRating::Restrained,
            &spec,
            Some(&place()),
            references_for(&spec),
        )
        .unwrap();
        assert!(prompt.as_str().contains(&format!(
            "exactly {count} heroes, each the person on their reference sheet; no other figure dressed or posed as a hero"
        )));
        for ordinal in 1..=count {
            assert!(prompt.as_str().contains(&format!("Hero {ordinal}")));
        }
        assert!(!prompt.as_str().contains(&format!("Hero {}", count + 1)));
    }
}

#[test]
fn scene_party_refuses_zero_or_more_than_six_heroes_and_invalid_sheet_numbers() {
    for count in 1..=6 {
        assert_eq!(SceneParty::new(count, 2).unwrap().heroes(), count as u8);
    }
    for count in [0, 7, usize::MAX] {
        assert_eq!(
            SceneParty::new(count, 2),
            Err(PlatePromptRefusal::HeroIdentity)
        );
    }
    for sheet_image in [0, 5, usize::MAX] {
        assert_eq!(
            SceneParty::new(2, sheet_image),
            Err(PlatePromptRefusal::HeroIdentity)
        );
    }
}

#[test]
fn counted_flux_prompt_keeps_the_exact_party_and_npc_identities_under_its_limit() {
    for heroes in [1, 2, 4, 6] {
        let mut sheet: Vec<FluxPerson> = (1..=heroes)
            .map(|ordinal| FluxPerson {
                name: format!("Hero {ordinal} ({})", "A".repeat(32)),
                appearance: Some("Accepted public appearance. ".repeat(20)),
            })
            .collect();
        sheet.extend((1..=3).map(|ordinal| FluxPerson {
            name: format!("Contact {ordinal} {}", "N".repeat(22)),
            appearance: None,
        }));
        let cast = FluxCast {
            sheet,
            speaker: Some(FluxPerson {
                name: "Contact 1".into(),
                appearance: None,
            }),
            portrait: true,
        };
        for tier in [HealthTier::Healthy, HealthTier::Critical, HealthTier::Down] {
            let prompt = flux_scene_plate_for_party(
                ContentRating::Mature,
                Camera::WideEstablishing,
                Some(&place()),
                &cast,
                &"facts ".repeat(500),
                Some((FaceExpression::Neutral, tier)),
                heroes,
            )
            .unwrap();
            assert!(prompt.as_str().contains(&format!("exactly {heroes} heroes, each the person on their reference sheet; no other figure dressed or posed as a hero")));
            for person in &cast.sheet {
                assert!(prompt.as_str().contains(&person.name));
            }
            assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
            assert_eq!(
                prompt.scene_party(),
                Some(SceneParty::new(heroes, 2).unwrap())
            );
        }
    }
}

#[test]
fn six_hero_hand_custody_is_reserved_inside_the_flux_cap() {
    use crate::gameplay::adventure::scene_model::SceneModel;
    let mut model: SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../../content/merchant-shop/scene-model.json"
    ))
    .unwrap();
    let person = model.people[0].clone();
    let item = model.things[0].clone();
    model.people.clear();
    model.things.clear();
    let objects = ["cup", "packet", "lantern", "coin purse"];
    for ordinal in 0..6 {
        let mut hero = person.clone();
        hero.id = format!("hero-{ordinal}");
        hero.name = format!("Hero {ordinal}");
        model.people.push(hero);
        if let Some(name) = objects.get(ordinal) {
            let mut held = item.clone();
            held.id = format!("item-{ordinal}");
            held.name = (*name).to_owned();
            held.position = "left hand".into();
            held.holder_id = Some(format!("hero-{ordinal}"));
            // Ownership differs from physical custody on purpose.
            held.owner_id = Some("merchant".into());
            held.publicly_visible = true;
            held.in_scene = true;
            model.things.push(held);
        }
    }
    for count in [1, 4, 6] {
        let cast = FluxCast {
            sheet: model
                .people
                .iter()
                .take(count)
                .map(|person| FluxPerson {
                    name: person.name.clone(),
                    appearance: Some("Accepted physical appearance. ".repeat(20)),
                })
                .collect(),
            speaker: None,
            portrait: false,
        };
        let hands = cast
            .sheet
            .iter()
            .map(|person| person.fixture_hands(Some(&model)))
            .collect::<Vec<_>>();
        let prompt = cast
            .plate_with_hands(
                ContentRating::Restrained,
                Camera::WideEstablishing,
                Some(&place()),
                "",
                None,
                (count, &hands),
            )
            .unwrap();
        println!(
            "HS37 heroes={count} prompt_characters={}\n{}",
            prompt.character_count(),
            prompt.as_str()
        );
        assert!(prompt.character_count() <= FLUX_MAXIMUM_PROMPT_CHARACTERS);
        for (person, hands) in cast.sheet.iter().zip(&hands) {
            assert!(prompt.as_str().contains(&person.name));
            assert!(prompt.as_str().contains(hands));
        }
        assert!(
            prompt
                .as_str()
                .contains("Weapons stay sheathed at hip or slung on back")
        );
        assert!(prompt.as_str().contains("bows unstrung on shoulder"));
        assert!(prompt.as_str().contains("spare hands empty"));
        assert!(prompt.scene_hand_custody().contains("Hero 0"));
        assert!(prompt.scene_hand_custody().contains("cup (left hand)"));
        let custody = prompt.scene_hand_custody().to_owned();
        let contextual = prompt
            .with_scene_context(&"Narrative without extra gear. ".repeat(400), Some(&model))
            .unwrap();
        assert_eq!(contextual.scene_hand_custody(), custody);
        assert!(!custody.contains("Narrative"));
    }
    let mut hidden = model.things[0].clone();
    hidden.name = "PRIVATE_HELD_ITEM".into();
    hidden.publicly_visible = false;
    model.things.push(hidden);
    let hero = FluxPerson {
        name: "Hero 0".into(),
        appearance: None,
    };
    assert!(
        !hero
            .fixture_hands(Some(&model))
            .contains("PRIVATE_HELD_ITEM")
    );
    model.things[0].name = "sword".into();
    model.things[0].position = "sheathed".into();
    assert_eq!(hero.fixture_hands(Some(&model)), "both hands empty");
    model.things[0].position = "drawn".into();
    assert!(hero.fixture_hands(Some(&model)).contains("sword (drawn)"));
    model.things[0].position = "drawn in right hand".into();
    let hands = hero.fixture_hands(Some(&model));
    assert!(hands.contains("sword (drawn in right hand)"));
    let cast = FluxCast {
        sheet: vec![hero],
        speaker: None,
        portrait: false,
    };
    let prompt = cast
        .plate_with_hands(
            ContentRating::Restrained,
            Camera::DialogueMedium,
            None,
            "",
            None,
            (1, &[hands]),
        )
        .unwrap();
    assert!(prompt.as_str().contains("sword (drawn in right hand)"));
    assert!(prompt.as_str().contains("drawn only where recorded"));
}

#[test]
fn worn_and_stowed_gear_leave_hands_empty_and_never_displace_a_real_hand_prop() {
    use crate::gameplay::adventure::scene_model::SceneModel;
    let mut model: SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../../content/merchant-shop/scene-model.json"
    ))
    .unwrap();
    let mut armor = model.things[0].clone();
    armor.id = "hero-chainmail".into();
    armor.name = "chainmail".into();
    armor.position = "worn".into();
    armor.holder_id = Some("Vesper".into());
    armor.in_scene = true;
    armor.publicly_visible = true;
    model.things = vec![armor.clone()];
    assert_eq!(
        FluxPerson::hands_for_holder(Some(&model), "Vesper"),
        "both hands empty"
    );
    let mut pack = armor.clone();
    pack.id = "hero-pack".into();
    pack.name = "bedroll".into();
    pack.position = "stowed".into();
    model.things.push(pack);
    assert_eq!(
        FluxPerson::hands_for_holder(Some(&model), "Vesper"),
        "both hands empty"
    );
    let mut cup = armor;
    cup.id = "hero-cup".into();
    cup.name = "cup".into();
    cup.position = "right hand".into();
    model.things.push(cup);
    assert_eq!(
        FluxPerson::hands_for_holder(Some(&model), "Vesper"),
        "hands: cup (right hand); spare hands empty"
    );
    let prompt = AssembledPrompt::from_parts(vec![Part::Fixed("Photographic TV drama.")])
        .with_scene_context("A quiet conversation.", Some(&model))
        .unwrap();
    assert!(prompt.as_str().contains("chainmail"));
    assert!(
        prompt.as_str().contains("worn"),
        "full physical positions are retained"
    );
}
