use super::tests::{build, input, tuning_journey};
use super::*;
use df_persistence::local_demo_scope::LocalDemoRole;
use pack::SceneKey;

const PREVIOUS_PACK: &[u8] = include_bytes!("../fixtures/harbor-journey-pack-baseline.json");
const TUNING: &[u8] = include_bytes!("../fixtures/harbor-journey-tuning-baseline.json");

fn player(current: &Checkpoint) -> rpc::PlayerGameplayView {
    let principal = if participants(current).is_empty() {
        bootstrap_member().unwrap()
    } else {
        MemberId::from_bytes(&MEMBERS[0]).unwrap()
    };
    let view = wire::journey_view(current, LocalDemoRole::Player, principal).unwrap();
    let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
        panic!("player projection");
    };
    player
}

fn scenes() -> Vec<(SceneKey, bool, Checkpoint)> {
    let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
    let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
    let journey = tuning_journey();
    let opening = &journey[0];
    let seal = stage_with_supplier(
        opening,
        &input(opening, first, 6, "ask-courier", vec![]),
        &mut |_| panic!("dialogue cannot draw"),
    )
    .unwrap();
    let escort = &journey[1];
    let mut count = 0;
    let combat = stage_with_supplier(
        escort,
        &input(escort, first, 7, "defend-courier", vec![]),
        &mut |sides| {
            if sides == 20 {
                count += 1;
                Ok(if count <= 2 { 1 } else { 20 })
            } else {
                Ok(sides)
            }
        },
    )
    .unwrap();
    let defeated = stage_with_supplier(
        &combat,
        &input(&combat, second, 8, "end-turn", vec![]),
        &mut |sides| Ok(sides),
    )
    .unwrap();
    let rested = stage_with_supplier(
        &defeated,
        &input(&defeated, first, 9, "short-rest", vec![]),
        &mut |_| panic!("rest cannot draw"),
    )
    .unwrap();
    let room = initial().unwrap();
    let room = stage_join(
        &room,
        &input(&room, bootstrap_member().unwrap(), 1, "join-room", vec![]),
    )
    .unwrap();
    vec![
        (SceneKey::Room, false, room),
        (SceneKey::Opening, false, opening.clone()),
        (SceneKey::DialogueSeal, false, seal),
        (SceneKey::DialogueEscort, false, escort.clone()),
        (SceneKey::Combat, false, journey[2].clone()),
        (SceneKey::Victory, false, journey[6].clone()),
        (SceneKey::Victory, true, journey[7].clone()),
        (SceneKey::Inn, true, journey[8].clone()),
        (SceneKey::Defeat, false, defeated),
        (SceneKey::Defeat, true, rested),
    ]
}

#[test]
fn shipped_views_match_previous_text_fixtures_for_every_scene() {
    // This frozen fixture captures the pre-wiring literals, independently of the editable pack.
    let previous = pack::load_from_bytes(PREVIOUS_PACK, TUNING).unwrap();
    let baseline: Vec<_> = scenes()
        .iter()
        .map(|(_, _, checkpoint)| player(checkpoint))
        .collect();
    let shipped = pack::load_from_bytes(
        include_bytes!("../../../../../content/harbor-journey/pack.json"),
        TUNING,
    )
    .unwrap();
    let shipped_scenes = shipped.pack.scenes.clone();
    pack::with_test_pack(shipped, || {
        for ((key, rested, checkpoint), expected_view) in scenes().iter().zip(&baseline) {
            let mut actual = player(checkpoint);
            // The cast is additive; the previous literals had none.
            if let Some(scene) = actual.scene.as_mut() {
                scene.cast.clear();
            }
            let shipped_scene = shipped_scenes
                .iter()
                .find(|scene| scene.key == *key)
                .unwrap();
            let split = !shipped_scene.rules_note.is_empty() && !*rested;
            let tactical_text_changed =
                matches!(key, SceneKey::Combat | SceneKey::Victory | SceneKey::Defeat);
            if !split && !tactical_text_changed {
                assert_eq!(&actual, expected_view, "complete view parity for {key:?}");
            }
            let fixture = previous
                .pack
                .scenes
                .iter()
                .find(|scene| scene.key == *key)
                .unwrap();
            let scene = actual.scene.as_ref().unwrap();
            assert_eq!(scene.title, fixture.title);
            assert_eq!(scene.scene_asset, fixture.asset);
            let journey = actual.journey.as_ref().unwrap();
            assert_eq!(journey.speaker, fixture.speaker);
            assert_eq!(journey.dialogue, fixture.dialogue);
            let text = if *rested {
                fixture
                    .narration_rested
                    .as_ref()
                    .unwrap_or(&fixture.narration)
            } else {
                &fixture.narration
            };
            let expected = if matches!(
                key,
                SceneKey::Combat | SceneKey::Victory | SceneKey::Inn | SceneKey::Defeat
            ) {
                format!(
                    "{text} {}",
                    previous
                        .pack
                        .escort_support_suffix
                        .replace("{name}", "Brynn")
                )
            } else {
                text.clone()
            };
            if tactical_text_changed {
                let expected_text = if *rested {
                    shipped_scene.narration_rested.as_ref().unwrap()
                } else {
                    &shipped_scene.narration
                };
                assert!(actual.narration.starts_with(expected_text));
                if *key == SceneKey::Combat && tactical::present(checkpoint.state()) {
                    assert!(scene.rules_note.starts_with(&shipped_scene.rules_note));
                    assert!(scene.rules_note.contains("tactical.cover-centre-line"));
                    assert!(!actual.narration.contains("tactical.cover-centre-line"));
                } else {
                    assert!(!scene.rules_note.contains("tactical.cover-centre-line"));
                    assert_eq!(
                        scene.rules_note,
                        if *rested {
                            ""
                        } else {
                            &shipped_scene.rules_note
                        }
                    );
                }
                continue;
            }
            if split {
                // The harbor pack keeps its rules sentences in `rules_note`, apart from the
                // story text; every one of them was already part of the previous narration.
                for sentence in shipped_scene.rules_note.split_inclusive(". ") {
                    assert!(
                        fixture.narration.contains(sentence.trim()),
                        "{sentence:?} for {key:?}"
                    );
                }
                // The story and the rules note travel separately: the narration is only
                // the story, and the note is its own field of the scene.
                assert!(
                    actual.narration.starts_with(&shipped_scene.narration)
                        && !actual.narration.contains(&shipped_scene.rules_note),
                    "story without the rules note for {key:?}"
                );
                assert_eq!(scene.rules_note, shipped_scene.rules_note, "{key:?}");
                continue;
            }
            assert_eq!(actual.narration, expected, "captured narration for {key:?}");
        }
    });
}

#[test]
fn edited_text_options_equipment_and_room_drive_complete_journey() {
    let mut content: serde_json::Value = serde_json::from_slice(PREVIOUS_PACK).unwrap();
    content["room_code"] = "TUNED".into();
    content["creation"]["description"] = "Create a harbor guardian.".into();
    content["creation"]["groups"][1]["options"][0]["label"] = "Harbor Fighter".into();
    content["creation"]["groups"][0]["options"][0]["description"] = "A trusted dwarf.".into();
    content["creation"]["pick_lists"]["alignment"] = serde_json::json!(["lawful-good"]);
    content["bandit"]["name"] = "Harbor Enforcer".into();
    content["offer_labels"]["defend-courier"] = "Protect our courier".into();
    content["escort_support_suffix"] = "The courier trusts {name}.".into();
    for scene in content["scenes"].as_array_mut().unwrap() {
        let key = scene["key"].as_str().unwrap().to_owned();
        scene["title"] = format!("Tuned {key}").into();
        scene["narration"] = format!("Live narration for {key}.").into();
        scene["speaker"] = "Harbor guide".into();
        scene["dialogue"] = format!("Live dialogue for {key}.").into();
        if scene.get("narration_rested").is_some() {
            scene["narration_rested"] = format!("Live rested {key}.").into();
        }
    }
    content["scenes"][1]["asset"] = "assets/concept-art/scene-tavern-barkeep-talk-rain.webp".into();
    for row in content["equipment"].as_array_mut().unwrap() {
        if row["item"] == "greatsword" {
            row["quantity"] = 2.into();
        }
        if row["item"] == "gold-piece" {
            row["quantity"] = 25.into();
        }
    }
    content["equipment"].as_array_mut().unwrap().reverse();
    let loaded = pack::load_from_bytes(&serde_json::to_vec(&content).unwrap(), TUNING).unwrap();
    pack::with_test_pack(loaded, || {
        let offer = wire::creation_offer().unwrap();
        assert_eq!(offer.description, "Create a harbor guardian.");
        assert_eq!(offer.groups[0].options[0].description, "A trusted dwarf.");
        assert_eq!(
            offer
                .groups
                .iter()
                .find(|group| group.group_id == "alignment")
                .unwrap()
                .options
                .len(),
            1
        );
        let mut refused = build("Brynn");
        refused
            .iter_mut()
            .find(|(key, _)| key.as_str() == "alignment")
            .unwrap()
            .1 = model::label("neutral-good").unwrap();
        let pairs: Vec<_> = refused
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert!(pack::validate_character("Brynn", &pairs).is_err());
        let checkpoints = tuning_journey();
        let hero = entity(ENTITIES[0]).unwrap();
        let sheet = character_sheet(checkpoints[0].state(), hero).unwrap();
        assert!(
            sheet
                .facts
                .iter()
                .any(|fact| fact.label == "Class" && fact.value == "Harbor Fighter")
        );
        assert!(
            sheet
                .facts
                .iter()
                .any(|fact| fact.label == "Starting equipment" && fact.value.ends_with("25 GP"))
        );
        assert_eq!(
            checkpoints[0]
                .state()
                .inventory
                .iter()
                .find(|item| item.item == held_weapon_id(hero).unwrap())
                .unwrap()
                .quantity,
            2
        );
        assert_eq!(
            name(checkpoints[2].state(), entity(BANDIT).unwrap()).unwrap(),
            "Harbor Enforcer"
        );
        let dialogue = player(&checkpoints[1]);
        assert!(
            dialogue
                .offers
                .iter()
                .any(|offer| offer.label == "Protect our courier")
        );
        for (key, rested, checkpoint) in scenes() {
            let actual = player(&checkpoint);
            let key_text = content["scenes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|scene| {
                    serde_json::from_value::<SceneKey>(scene["key"].clone()).unwrap() == key
                })
                .unwrap()["key"]
                .as_str()
                .unwrap();
            let scene = actual.scene.unwrap();
            assert_eq!(scene.title, format!("Tuned {key_text}"));
            assert_eq!(
                scene.scene_asset,
                content["scenes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|scene| scene["key"] == key_text)
                    .unwrap()["asset"]
                    .as_str()
                    .unwrap()
            );
            assert!(actual.narration.starts_with(&if rested
                && matches!(key, SceneKey::Victory | SceneKey::Defeat)
            {
                format!("Live rested {key_text}.")
            } else {
                format!("Live narration for {key_text}.")
            }));
            let journey = actual.journey.unwrap();
            assert_eq!(journey.speaker, "Harbor guide");
            assert_eq!(journey.dialogue, format!("Live dialogue for {key_text}."));
            assert_eq!(journey.room_code, "TUNED");
            assert_eq!(journey.join_path, "/gameplay/player?room=TUNED");
        }
        assert!(
            player(&checkpoints[2])
                .narration
                .contains("The courier trusts Brynn.")
        );
        assert!(crate::gameplay::room::matches_room_code("TUNED"));
        assert!(!crate::gameplay::room::matches_room_code("LANTERN"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let axum::response::Html(body) =
            runtime.block_on(crate::gameplay::room::player_page_for("0123456789abcdef"));
        assert!(body.contains("data-room=\"TUNED\" data-run=\"0123456789abcdef\""));
        assert!(!body.contains("data-room=\"LANTERN\""));
    });
}

#[test]
fn custom_rules_remain_visible_to_both_roles_before_and_after_creation() {
    let mut tuning: serde_json::Value = serde_json::from_slice(TUNING).unwrap();
    tuning["hero"]["armor_class"] = 18.into();
    let loaded =
        pack::load_from_bytes(PREVIOUS_PACK, &serde_json::to_vec(&tuning).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let notice = format!("Custom rules: {}.", model::source_revision());
        let display = |checkpoint: &Checkpoint| {
            let view = wire::journey_view(
                checkpoint,
                LocalDemoRole::Display,
                MemberId::from_bytes(&df_persistence::local_demo_scope::DISPLAY).unwrap(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
                panic!("display");
            };
            display
        };
        assert!(display(&initial().unwrap()).narration.ends_with(&notice));
        for (_, _, checkpoint) in scenes() {
            let actual = player(&checkpoint);
            assert!(actual.narration.ends_with(&notice));
            assert!(display(&checkpoint).narration.ends_with(&notice));
            if let Some(sheet) = actual.journey.unwrap().own_character {
                assert!(
                    sheet
                        .facts
                        .iter()
                        .any(|fact| fact.label == "Supported rules"
                            && fact.value == model::source_revision())
                );
            }
        }
    });
}

fn display_view(current: &Checkpoint) -> rpc::DisplayGameplayView {
    let view = wire::journey_view(
        current,
        LocalDemoRole::Display,
        MemberId::from_bytes(&df_persistence::local_demo_scope::DISPLAY).unwrap(),
    )
    .unwrap();
    let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
        panic!("display projection");
    };
    display
}

#[test]
fn the_display_lists_the_pack_offer_labels_and_nothing_private() {
    use prost::Message;
    let shipped = pack::load_from_bytes(
        include_bytes!("../../../../../content/harbor-journey/pack.json"),
        TUNING,
    )
    .unwrap();
    pack::with_test_pack(shipped, || {
        let labels = [
            rpc::GameplayActionKind::AskCourier,
            rpc::GameplayActionKind::EscortCourier,
            rpc::GameplayActionKind::DefendCourier,
            rpc::GameplayActionKind::ShortRest,
            rpc::GameplayActionKind::ChooseHarborScene,
        ]
        .map(pack::offer_label);
        let mut listed = 0;
        for (key, _, checkpoint) in scenes() {
            let display = display_view(&checkpoint);
            let shown = display.journey.as_ref().unwrap().table_choices.clone();
            let encoded = display.encode_to_vec();
            let text = String::from_utf8_lossy(&encoded);
            let player = player(&checkpoint);
            for offer in &player.offers {
                assert!(
                    !text.contains(&offer.offer_id),
                    "no offer id on the display for {key:?}"
                );
            }
            if !player.private_clue.is_empty() {
                assert!(!text.contains(&player.private_clue), "{key:?}");
            }
            for label in &shown {
                assert!(
                    labels.contains(label),
                    "{label:?} is a pack offer label for {key:?}"
                );
            }
            let mut unique = shown.clone();
            unique.dedup();
            assert_eq!(unique, shown, "no repeated label for {key:?}");
            match key {
                SceneKey::Opening => assert_eq!(
                    shown,
                    [
                        pack::offer_label(rpc::GameplayActionKind::AskCourier),
                        pack::offer_label(rpc::GameplayActionKind::EscortCourier)
                    ]
                ),
                SceneKey::Combat => assert!(shown.is_empty(), "a fight turn is one hero's"),
                _ => {}
            }
            listed += shown.len();
            // The player's own view never carries the display's list.
            assert!(player.journey.unwrap().table_choices.is_empty());
        }
        assert!(listed > 0, "some scene lists choices");
    });
}

#[test]
fn the_rules_note_is_a_separate_field_and_never_part_of_the_narration() {
    let shipped = pack::load_from_bytes(
        include_bytes!("../../../../../content/harbor-journey/pack.json"),
        TUNING,
    )
    .unwrap();
    let authored = shipped.pack.scenes.clone();
    pack::with_test_pack(shipped, || {
        let mut with_note = 0;
        for (key, rested, checkpoint) in scenes() {
            if rested {
                continue;
            }
            let display = display_view(&checkpoint);
            let scene = display.scene.as_ref().unwrap();
            let note = &authored
                .iter()
                .find(|candidate| candidate.key == key)
                .unwrap()
                .rules_note;
            assert_eq!(&scene.rules_note, note, "{key:?}");
            if !note.is_empty() {
                with_note += 1;
                assert!(!display.narration.contains(note.as_str()), "{key:?}");
            }
        }
        assert!(with_note > 0, "the shipped pack has rules notes");
    });
}
