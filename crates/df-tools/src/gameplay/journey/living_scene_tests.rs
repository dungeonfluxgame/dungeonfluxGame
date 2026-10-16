use super::*;
use crate::gameplay::beat_script_tests::input;
use df_persistence::local_demo_scope::LocalDemoRole;

fn taproom(run: impl FnOnce()) {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut pack: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    pack.creation.pick_lists.skill = vec!["perception".into(), "survival".into()];
    for scene in &mut pack.scenes {
        scene.setting = Some(generated.settings.scene(scene.key).clone());
        let source = generated
            .pack
            .scenes
            .iter()
            .find(|source| source.key == scene.key)
            .unwrap();
        scene.asset = source.asset.clone();
        scene.cast = source.cast.clone();
        scene.speaker = source.speaker.clone();
    }
    let loaded = pack::load_from_bytes(
        &serde_json::to_vec(&pack).unwrap(),
        include_bytes!("../fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, run);
}

fn opened() -> (Checkpoint, MemberId) {
    let mut current = initial().unwrap();
    for operation in [1, 2] {
        current = stage_join(
            &current,
            &input(
                &current,
                bootstrap_member().unwrap(),
                operation,
                "join-room",
                vec![],
            ),
        )
        .unwrap();
    }
    for (operation, bytes, hero) in [(3, MEMBERS[0], "Brynn"), (4, MEMBERS[1], "Vale")] {
        let choices = crate::gameplay::beat_script_tests::build(hero);
        let values = choices
            .iter()
            .filter(|(group, _)| group.as_str() != "name")
            .map(|(group, value)| (group.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        pack::validate_character(hero, &values)
            .expect("fixture choices must match this source build");
        current = stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&bytes).unwrap(),
                operation,
                "create-character",
                choices,
            ),
            &mut |_| panic!("creation must not roll"),
        )
        .unwrap_or_else(|error| {
            panic!(
                "fixture creation: {error:?}, step={:?}",
                refusal_detail::take()
            )
        });
    }
    let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
    let current = stage_with_supplier(
        &current,
        &input(&current, member, 5, "begin-story", vec![]),
        &mut |_| panic!("opening must not roll"),
    )
    .unwrap();
    (current, member)
}

fn interact(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
    target: &str,
    option: &str,
) -> Checkpoint {
    let choices = vec![
        (
            model::label("scene-target").unwrap(),
            model::label(target).unwrap(),
        ),
        (
            model::label("scene-option").unwrap(),
            model::label(option).unwrap(),
        ),
    ];
    stage_with_supplier(
        current,
        &input(current, member, op, "script-choice", choices),
        &mut |_| panic!("ordinary scene interaction must not roll"),
    )
    .unwrap_or_else(|error| {
        panic!(
            "{option} {target}: {error:?}; step={:?}",
            refusal_detail::take()
        )
    })
}

#[test]
fn a_taken_object_and_open_door_survive_checkpoint_encoding_without_advancing_the_story() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let thing = model
            .things
            .iter()
            .find(|thing| thing.portable && !thing.in_use && thing.holder_id.is_none())
            .unwrap();
        let door = model.ways_out.first().unwrap();
        let taken = interact(&current, member, 10, &thing.id, "take");
        let opened = interact(&taken, member, 11, &door.id, "open");
        assert_eq!(
            opened.state().narrative.active_beats,
            current.state().narrative.active_beats
        );
        let now = living_scene::snapshot(opened.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            now.model
                .things
                .iter()
                .find(|candidate| candidate.id == thing.id)
                .unwrap()
                .holder_id
                .as_deref(),
            Some("Brynn")
        );
        assert!(
            now.model
                .ways_out
                .iter()
                .find(|candidate| candidate.id == door.id)
                .unwrap()
                .open
        );
        let codec = df_persistence::NativeCodecLimits {
            maximum_document_bytes: 2 * 1024 * 1024,
            maximum_allocated_bytes: 2 * 1024 * 1024,
            maximum_collection_items: 4096,
            maximum_text_bytes: 4096,
        };
        let bytes =
            df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&opened, codec).unwrap();
        let restored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
            &bytes,
            opened.basis(),
            &model::pins().unwrap(),
            &model::recovery().unwrap(),
            codec,
        )
        .unwrap();
        assert_eq!(
            living_scene::snapshot(restored.state(), &scene).unwrap(),
            Some(now)
        );
        assert_eq!(restored, opened);
    });
}

#[test]
fn a_purchase_spends_exact_copper_and_cannot_overdraw_the_heros_purse() {
    taproom(|| {
        let (current, member) = opened();
        let who = player_entity(member, &current).unwrap();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let offer = scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .offers
            .first()
            .unwrap();
        let before = living_scene::balance_cp(current.state(), who).unwrap();
        let bought = interact(&current, member, 12, &offer.id, "buy");
        assert_eq!(
            living_scene::balance_cp(bought.state(), who).unwrap(),
            before - u64::from(offer.cost_cp)
        );
        assert!(
            living_scene::last_line(bought.state())
                .unwrap()
                .unwrap()
                .contains(&format!("{} CP", offer.cost_cp))
        );
        let npc = &scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .people[0]
            .id;
        let emptying_gift = input(
            &current,
            member,
            15,
            "script-choice",
            vec![
                (
                    model::label("scene-target").unwrap(),
                    model::label(npc).unwrap(),
                ),
                (
                    model::label("scene-option").unwrap(),
                    model::label("gift").unwrap(),
                ),
                (
                    model::label("scene-amount").unwrap(),
                    model::label(&(before / 100).to_string()).unwrap(),
                ),
            ],
        );
        let empty = stage_with_supplier(&current, &emptying_gift, &mut |_| {
            panic!("a gift cannot roll")
        })
        .unwrap();
        let refused = interact(&empty, member, 13, &offer.id, "buy");
        assert_eq!(living_scene::balance_cp(refused.state(), who).unwrap(), 0);
        assert!(
            living_scene::last_line(refused.state())
                .unwrap()
                .unwrap()
                .contains("not have enough coin")
        );
        assert!(
            !living_scene::snapshot(refused.state(), &scene)
                .unwrap()
                .unwrap()
                .model
                .things
                .iter()
                .any(|thing| thing.id == format!("purchased-{}", offer.id))
        );
    });
}

#[test]
fn the_display_receives_the_current_check_faces_outcome_and_target_without_dc_prose() {
    taproom(|| {
        let (current, member) = opened();
        let choices = vec![
            (
                model::label("attempt-thing").unwrap(),
                model::label("room").unwrap(),
            ),
            (
                model::label("attempt-skill").unwrap(),
                model::label("perception").unwrap(),
            ),
        ];
        let searched = stage_with_supplier(
            &current,
            &input(&current, member, 14, "script-choice", choices),
            &mut |_| Ok(20),
        )
        .unwrap();
        let view = wire::journey_view(&searched, LocalDemoRole::Display, member).unwrap();
        let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
            panic!("display required");
        };
        let check = display.check.unwrap();
        assert_eq!(check.faces, vec![20]);
        assert_eq!(check.roll_mode, 0);
        assert!(check.succeeded);
        assert!(!display.narration.contains("DC"));
        assert!(!display.narration.contains("Wisdom (Perception)"));
    });
}

#[test]
fn soldier_dwarf_fighter_sheet_exposes_its_source_traits_and_both_class_skills() {
    // SRD 5.2.1 pp. 47, 83-84: accepted Fighter/Soldier/Dwarf build.
    let (current, member) = opened();
    let sheet = character_sheet(current.state(), player_entity(member, &current).unwrap()).unwrap();
    for label in [
        "First class skill",
        "Second class skill",
        "Soldier skill proficiencies",
        "Saving throw proficiencies",
        "Proficiency Bonus",
        "Speed",
        "Initiative",
        "Armor training",
        "Weapon proficiencies",
        "Darkvision",
        "Dwarven Resilience",
        "Stonecunning",
        "Hit Point Dice",
    ] {
        assert!(
            sheet
                .facts
                .iter()
                .any(|fact| fact.label == label && !fact.value.is_empty()),
            "{label}"
        );
    }
}

#[test]
fn hypothetical_negative_and_questioned_interactions_do_not_execute() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    scene.setting = Some(generated.settings.meeting);
    for words in [
        "What if I open the door?",
        "I do not pick up the tankard",
        "I might open the door",
        "I open the door?",
        "I do not want a pint of beer",
    ] {
        assert!(
            living_scene::interaction_for_line(&scene, words).is_none(),
            "{words}"
        );
    }
    assert_eq!(
        living_scene::interaction_for_line(&scene, "I open the door")
            .unwrap()
            .1,
        "open"
    );
    assert_eq!(living_scene::gift_amount("I give Ilsa 5 GP."), Some(5));
    assert_eq!(living_scene::gift_amount("I give Ilsa 5 GP or 10 GP"), None);
}

#[test]
fn a_named_npc_gift_spends_owned_gold_and_an_unaffordable_gift_transfers_nothing() {
    taproom(|| {
        let (current, member) = opened();
        let who = player_entity(member, &current).unwrap();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let npc = &scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .people[0]
            .id;
        let before = living_scene::balance_cp(current.state(), who).unwrap();
        let make = |current: &Checkpoint, operation, amount: u32| {
            input(
                current,
                member,
                operation,
                "script-choice",
                vec![
                    (
                        model::label("scene-target").unwrap(),
                        model::label(npc).unwrap(),
                    ),
                    (
                        model::label("scene-option").unwrap(),
                        model::label("gift").unwrap(),
                    ),
                    (
                        model::label("scene-amount").unwrap(),
                        model::label(&amount.to_string()).unwrap(),
                    ),
                ],
            )
        };
        let paid = stage_with_supplier(&current, &make(&current, 80, 5), &mut |_| {
            panic!("a gift does not roll Influence")
        })
        .unwrap();
        assert_eq!(
            living_scene::balance_cp(paid.state(), who).unwrap(),
            before - 500
        );
        assert!(
            living_scene::last_line(paid.state())
                .unwrap()
                .unwrap()
                .contains("5 GP")
        );
        let refused = stage_with_supplier(&paid, &make(&paid, 81, 1000), &mut |_| {
            panic!("an unaffordable gift cannot roll")
        })
        .unwrap();
        assert_eq!(
            living_scene::balance_cp(refused.state(), who).unwrap(),
            before - 500
        );
        assert!(
            living_scene::last_line(refused.state())
                .unwrap()
                .unwrap()
                .contains("do not have")
        );
    });
}

#[test]
fn an_absent_scene_target_is_described_without_drawing_a_die() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let target =
            crate::gameplay::investigation::local_target("I examine the golden telescope.", &[]);
        let after = interact(&current, member, 30, &target.id, "examine");
        let receipt = accepted(after.state().decisions.last().unwrap()).unwrap();
        assert!(receipt.check.is_none());
        assert!(
            living_scene::last_line(after.state())
                .unwrap()
                .unwrap()
                .contains("no golden telescope")
        );
        assert_eq!(phase(&after).unwrap(), phase(&current).unwrap());
        assert_eq!(
            living_scene::snapshot(after.state(), &scene).unwrap(),
            living_scene::snapshot(current.state(), &scene).unwrap()
        );
    });
}

#[test]
fn an_ordinary_counter_without_a_finding_is_described_without_drawing_a_die() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let catalog = crate::gameplay::investigation::targets(&scene, &[]);
        let target =
            crate::gameplay::investigation::local_target("I look behind the counter.", &catalog);
        assert!(target.present && target.id.starts_with("surface-"));
        assert!(
            crate::gameplay::investigation::finding_on(&scene, &target, "perception").is_none()
        );
        let after = interact(&current, member, 30, &target.id, "examine");
        let receipt = accepted(after.state().decisions.last().unwrap()).unwrap();
        assert!(receipt.check.is_none());
        assert!(
            living_scene::last_line(after.state())
                .unwrap()
                .unwrap()
                .contains("bar")
        );
        assert_eq!(phase(&after).unwrap(), phase(&current).unwrap());
        assert_eq!(
            living_scene::snapshot(after.state(), &scene).unwrap(),
            living_scene::snapshot(current.state(), &scene).unwrap()
        );
    });
}

#[test]
fn a_purchase_names_the_seller_and_keeps_a_card_for_the_buyer() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let offer = model
            .offers
            .iter()
            .find(|offer| offer.name.contains("ale"))
            .unwrap();
        let seller = model
            .people
            .iter()
            .find(|person| person.id == offer.seller_id)
            .unwrap();
        let after = interact(&current, member, 31, &offer.id, "buy");
        let line = living_scene::last_line(after.state()).unwrap().unwrap();
        assert!(line.contains(&seller.name), "{line}");
        assert!(line.contains("pours"), "{line}");
        let server = model
            .people
            .iter()
            .find(|person| person.role == "serving woman")
            .unwrap();
        assert!(line.contains(&server.name), "{line}");
        let card = living_scene::item_for(after.state(), &scene, "Brynn")
            .unwrap()
            .unwrap();
        assert_eq!(card.thing_id, offer.id);
        assert_eq!(card.detail, offer.description);
        assert!(
            living_scene::item_for(after.state(), &scene, "Vale")
                .unwrap()
                .is_none()
        );
        let owned_id = format!("purchased-{}", offer.id);
        let owned = living_scene::snapshot(after.state(), &scene)
            .unwrap()
            .unwrap()
            .model
            .things
            .into_iter()
            .find(|thing| thing.id == owned_id)
            .expect("purchase must create a durable held scene thing");
        assert_eq!(owned.holder_id.as_deref(), Some("Brynn"));
        assert_eq!(owned.owner_id.as_deref(), Some("Brynn"));
        assert!(owned.in_scene);
        let codec = df_persistence::NativeCodecLimits {
            maximum_document_bytes: 2 * 1024 * 1024,
            maximum_allocated_bytes: 2 * 1024 * 1024,
            maximum_collection_items: 4096,
            maximum_text_bytes: 4096,
        };
        let bytes =
            df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&after, codec).unwrap();
        let restored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
            &bytes,
            after.basis(),
            &model::pins().unwrap(),
            &model::recovery().unwrap(),
            codec,
        )
        .unwrap();
        assert_eq!(
            living_scene::snapshot(restored.state(), &scene).unwrap(),
            living_scene::snapshot(after.state(), &scene).unwrap()
        );
        let examined = interact(&after, member, 32, &owned_id, "examine");
        assert!(
            living_scene::last_line(examined.state())
                .unwrap()
                .unwrap()
                .contains("It belongs to Brynn.")
        );
        let attempted_give = interact(&after, member, 33, &owned_id, "give");
        assert!(
            living_scene::last_line(attempted_give.state())
                .unwrap()
                .unwrap()
                .contains("must name a recipient")
        );
        let balance_after_first =
            living_scene::balance_cp(after.state(), player_entity(member, &after).unwrap())
                .unwrap();
        let repeated = interact(&after, member, 34, &offer.id, "buy");
        assert_eq!(
            living_scene::balance_cp(repeated.state(), player_entity(member, &repeated).unwrap())
                .unwrap(),
            balance_after_first,
            "a finite offer cannot charge twice"
        );
        assert!(
            living_scene::last_line(repeated.state())
                .unwrap()
                .unwrap()
                .contains("already sold")
        );
    });
}

#[test]
fn first_player_projection_contains_committed_room_state_without_a_display_observation() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let taken = interact(&current, member, 32, &model.things[0].id, "take");
        let opened = interact(&taken, member, 33, &model.ways_out[0].id, "open");
        let current = crate::gameplay::beat_script_variety_tests::persisted(&opened);
        let view = wire::journey_view(&current, LocalDemoRole::Player, member).unwrap();
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            panic!("player view");
        };
        let contents = player.scene.unwrap().scene_contents_json;
        let public: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(public["things"][0]["holder"], "Brynn");
        assert_eq!(public["ways_out"][0]["open"], true);
        assert!(!contents.contains("knowledge"));
        assert!(!contents.contains("finding_id"));
    });
}
