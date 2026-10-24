use super::*;
use crate::gameplay::beat_script_tests::input;
use df_persistence::local_demo_scope::LocalDemoRole;

fn taproom(run: impl FnOnce()) {
    taproom_with_model(|_| {}, run);
}

fn taproom_with_model(
    mut adjust: impl FnMut(&mut crate::gameplay::adventure::scene_model::SceneModel),
    run: impl FnOnce(),
) {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut pack: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    pack.creation.pick_lists.skill = vec!["perception".into(), "survival".into()];
    for scene in &mut pack.scenes {
        scene.setting = Some(generated.settings.scene(scene.key).clone());
        adjust(scene.setting.as_mut().unwrap().model.as_mut().unwrap());
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

pub(super) fn barter_market(run: impl FnOnce()) {
    taproom_with_model(
        |model| {
            let Some(template) = model.offers.first().cloned() else {
                return;
            };
            for (id, name, cost) in [
                ("barter-knife", "Dagger", 200),
                ("barter-rope", "Rope", 100),
                ("barter-lantern", "Hooded lantern", 500),
                ("barter-blanket", "Blanket", 50),
            ] {
                let mut offer = template.clone();
                offer.id = id.into();
                offer.item_id = format!("item-{id}");
                offer.name = name.into();
                offer.aliases = vec![name.into()];
                offer.cost_cp = cost;
                model.offers.push(offer);
            }
            let merchant = model
                .people
                .iter_mut()
                .find(|person| person.id == template.seller_id)
                .unwrap();
            merchant.coin_cp = 0;
        },
        run,
    );
}

pub(super) fn barter_inventory() -> (Checkpoint, MemberId, merchant_bargaining::ExchangeJudgement) {
    use merchant_bargaining::{
        ExchangeChoice, ExchangeItem, ExchangeJudgement, ExchangeOffer, MerchantStance,
    };
    let (current, member) = opened();
    let current = interact(&current, member, 80, "barter-knife", "buy");
    let current = interact(&current, member, 81, "barter-rope", "buy");
    let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
    let merchant_id = scene
        .setting
        .unwrap()
        .model
        .unwrap()
        .offer("barter-knife")
        .unwrap()
        .seller_id
        .clone();
    let item = |id: &str, catalog: &str| ExchangeItem {
        item_id: format!("item-barter-{id}"),
        catalog_id: catalog.into(),
    };
    let judgement = ExchangeJudgement {
        merchant_id,
        offer: ExchangeOffer {
            hero_items: vec![item("knife", "dagger"), item("rope", "rope")],
            merchant_items: vec![item("lantern", "lantern-hooded")],
            hero_cp: 350,
            merchant_cp: 0,
        },
        counter: None,
        hero_commits_now: true,
        choice: ExchangeChoice::Accept,
        stance: MerchantStance::Fair,
    };
    (current, member, judgement)
}

fn stage_barter(
    current: &Checkpoint,
    op: u8,
    judgement: merchant_bargaining::ExchangeJudgement,
) -> Checkpoint {
    living_scene::stage_exchange(
        current,
        current.basis(),
        OperationId::from_bytes(&[op; 16]).unwrap(),
        "Brynn",
        "fixture-fingerprint",
        judgement,
    )
    .unwrap_or_else(|error| panic!("{error:?}; {:?}", refusal_detail::take()))
}

fn hero_copper(current: &Checkpoint, member: MemberId) -> u64 {
    living_scene::balance_cp(current.state(), player_entity(member, current).unwrap()).unwrap()
}

#[test]
fn merchant_barter_settles_exact_custody_coin_and_revisions_and_replays() {
    barter_market(|| {
        let (current, member, judgement) = barter_inventory();
        let before_balance = hero_copper(&current, member);
        let merchant_id = judgement.merchant_id.clone();
        let before_terms = living_scene::merchant_trade_terms(&current, "Brynn").unwrap();
        let after = stage_barter(&current, 82, judgement.clone());
        assert_eq!(
            after.basis().revision.sequence(),
            current.basis().revision.sequence() + 1
        );
        assert_eq!(
            after.state().decisions.len(),
            current.state().decisions.len() + 1
        );
        assert_eq!(hero_copper(&after, member), before_balance - 350);
        let scene = crate::gameplay::beat_script_runtime::scene(after.state()).unwrap();
        let now = living_scene::snapshot(after.state(), &scene)
            .unwrap()
            .unwrap();
        for (id, owner) in [
            ("item-barter-knife", merchant_id.as_str()),
            ("item-barter-rope", merchant_id.as_str()),
            ("item-barter-lantern", "Brynn"),
        ] {
            let thing = now
                .model
                .things
                .iter()
                .find(|thing| thing.id == id)
                .unwrap();
            assert_eq!(thing.owner_id.as_deref(), Some(owner));
            assert_eq!(thing.holder_id.as_deref(), Some(owner));
        }
        assert_eq!(
            now.model
                .people
                .iter()
                .find(|person| person.id == merchant_id)
                .unwrap()
                .coin_cp,
            650
        );
        let terms = living_scene::merchant_trade_terms(&after, "Brynn").unwrap();
        let before = before_terms
            .iter()
            .find(|term| term.item_id == "item-barter-lantern")
            .unwrap();
        let after_term = terms
            .iter()
            .find(|term| term.item_id == "item-barter-lantern")
            .unwrap();
        assert_eq!(after_term.stock_revision, before.stock_revision + 1);
        assert_eq!(after_term.purse_revision, before.purse_revision + 1);
        let text =
            living_scene::line(after.state(), OperationId::from_bytes(&[82; 16]).unwrap()).unwrap();
        for expected in [
            "Done.",
            "1 x Dagger",
            "1 x Rope",
            "1 x Hooded lantern",
            "350 CP",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        let codec = df_persistence::NativeCodecLimits {
            maximum_document_bytes: 4 * 1024 * 1024,
            maximum_allocated_bytes: 8 * 1024 * 1024,
            maximum_collection_items: 4096,
            maximum_text_bytes: model::limits().maximum_text_bytes,
        };
        let encoded =
            df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&after, codec).unwrap();
        let recovered = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
            &encoded,
            after.basis(),
            after.pins(),
            &model::recovery().unwrap(),
            codec,
        )
        .unwrap();
        assert_eq!(
            living_scene::snapshot(recovered.state(), &scene)
                .unwrap()
                .unwrap(),
            now
        );
        assert!(
            living_scene::stage_exchange(
                &after,
                current.basis(),
                OperationId::from_bytes(&[83; 16]).unwrap(),
                "Brynn",
                "fixture-fingerprint",
                judgement
            )
            .is_err()
        );
    });
}

#[test]
fn merchant_barter_counter_selects_goods_without_moving_them_then_acceptance_settles() {
    barter_market(|| {
        let (current, member, mut judgement) = barter_inventory();
        let before = hero_copper(&current, member);
        judgement.offer.hero_cp = 0;
        judgement.counter = Some(merchant_bargaining::ExchangeOffer {
            hero_items: judgement.offer.hero_items[..1].to_vec(),
            merchant_items: judgement.offer.merchant_items.clone(),
            hero_cp: 400,
            merchant_cp: 0,
        });
        let countered = stage_barter(&current, 82, judgement);
        let pending = living_scene::pending_exchange(&countered, "Brynn")
            .unwrap()
            .unwrap();
        assert_eq!(
            pending.outcome,
            living_scene::MerchantExchangeOutcome::Countered
        );
        assert_eq!(hero_copper(&countered, member), before);
        assert_eq!(pending.judgement.offer.hero_items.len(), 1);
        let mut acceptance = pending.judgement;
        acceptance.choice = merchant_bargaining::ExchangeChoice::Accept;
        acceptance.hero_commits_now = true;
        let settled = stage_barter(&countered, 83, acceptance);
        assert_eq!(hero_copper(&settled, member), before - 400);
        let scene = crate::gameplay::beat_script_runtime::scene(settled.state()).unwrap();
        let now = living_scene::snapshot(settled.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            now.model
                .things
                .iter()
                .find(|thing| thing.id == "item-barter-rope")
                .unwrap()
                .owner_id
                .as_deref(),
            Some("Brynn")
        );
    });
}

#[test]
fn merchant_barter_refusal_and_unowned_goods_leave_every_purse_and_item_unchanged() {
    barter_market(|| {
        let (current, member, judgement) = barter_inventory();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let before = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap()
            .model;
        for (op, mut offer) in [(82, judgement.clone()), (83, judgement)] {
            if op == 82 {
                offer.choice = merchant_bargaining::ExchangeChoice::Refuse;
            } else {
                offer.offer.hero_items[0].item_id = "unowned-knife".into();
            }
            let after = stage_barter(&current, op, offer);
            let record = living_scene::exchange_record(
                after.state(),
                OperationId::from_bytes(&[op; 16]).unwrap(),
            )
            .unwrap();
            assert_eq!(
                record.outcome,
                living_scene::MerchantExchangeOutcome::Refused
            );
            assert_eq!(
                living_scene::snapshot(after.state(), &scene)
                    .unwrap()
                    .unwrap()
                    .model,
                before
            );
            assert_eq!(hero_copper(&after, member), hero_copper(&current, member));
            assert!(
                !living_scene::line(after.state(), OperationId::from_bytes(&[op; 16]).unwrap())
                    .unwrap()
                    .contains("Done.")
            );
        }
    });
}

#[test]
fn merchant_barter_sale_without_enough_purse_counters_and_srd_half_price_is_exact() {
    barter_market(|| {
        let (current, member, mut judgement) = barter_inventory();
        judgement.offer.merchant_items.clear();
        judgement.offer.hero_cp = 0;
        judgement.offer.merchant_cp = 500;
        let counter = stage_barter(&current, 82, judgement);
        let pending = living_scene::pending_exchange(&counter, "Brynn")
            .unwrap()
            .unwrap();
        assert_eq!(pending.judgement.offer.merchant_cp, 150);
        assert_eq!(hero_copper(&counter, member), hero_copper(&current, member));
        let merchant_id = pending.judgement.merchant_id.clone();
        let mut sale = pending.judgement;
        sale.offer.merchant_cp = 150;
        sale.choice = merchant_bargaining::ExchangeChoice::Accept;
        let after = stage_barter(&counter, 83, sale);
        assert_eq!(
            hero_copper(&after, member),
            hero_copper(&current, member) + 150
        );
        let scene = crate::gameplay::beat_script_runtime::scene(after.state()).unwrap();
        assert_eq!(
            living_scene::snapshot(after.state(), &scene)
                .unwrap()
                .unwrap()
                .model
                .people
                .iter()
                .find(|person| person.id == merchant_id)
                .unwrap()
                .coin_cp,
            150
        );
    });
}

#[test]
fn merchant_barter_goods_for_goods_needs_no_coin_and_preserves_other_items() {
    barter_market(|| {
        let (current, member, mut judgement) = barter_inventory();
        judgement.offer.hero_items.remove(0);
        judgement.offer.merchant_items = vec![merchant_bargaining::ExchangeItem {
            item_id: "item-barter-blanket".into(),
            catalog_id: "blanket".into(),
        }];
        judgement.offer.hero_cp = 0;
        let after = stage_barter(&current, 82, judgement);
        assert_eq!(hero_copper(&after, member), hero_copper(&current, member));
        let record = living_scene::exchange_record(
            after.state(),
            OperationId::from_bytes(&[82; 16]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record.outcome,
            living_scene::MerchantExchangeOutcome::Settled
        );
        let scene = crate::gameplay::beat_script_runtime::scene(after.state()).unwrap();
        let now = living_scene::snapshot(after.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            now.model
                .things
                .iter()
                .find(|thing| thing.id == "item-barter-knife")
                .unwrap()
                .owner_id
                .as_deref(),
            Some("Brynn")
        );
        assert_eq!(
            now.model
                .things
                .iter()
                .find(|thing| thing.id == "item-barter-blanket")
                .unwrap()
                .owner_id
                .as_deref(),
            Some("Brynn")
        );
        assert_eq!(
            now.model
                .things
                .iter()
                .find(|thing| thing.id == "item-barter-rope")
                .unwrap()
                .owner_id
                .as_deref(),
            Some(record.judgement.merchant_id.as_str())
        );
    });
}

#[test]
fn merchant_barter_empty_purse_counters_for_stock_without_taking_the_heros_goods() {
    barter_market(|| {
        use living_scene::{NpcInventoryAction, NpcInventoryProposal, NpcInventorySource};
        let (_, _, mut judgement) = barter_inventory();
        let (mut current, member) = opened();
        for (op, target) in [(80, "item-barter-knife"), (81, "item-barter-rope")] {
            current = living_scene::commit_npc_inventory_action(
                &current,
                NpcInventoryProposal {
                    operation: OperationId::from_bytes(&[op; 16]).unwrap(),
                    expected_basis: current.basis(),
                    actor_id: judgement.merchant_id.clone(),
                    target_id: target.into(),
                    action: NpcInventoryAction::Give {
                        recipient_id: "Brynn".into(),
                        transfer_ownership: true,
                        owner_consent_id: None,
                    },
                    source: Some(NpcInventorySource {
                        source_line_id: format!("gift-{op}"),
                        source_line_sequence: u64::from(op),
                        source_revision: current.basis().revision,
                        source_actor_id: judgement.merchant_id.clone(),
                        source_text: "A committed fixture gift of the listed goods.".into(),
                    }),
                },
            )
            .unwrap();
        }
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let before = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap()
            .model;
        judgement.offer.merchant_items.clear();
        judgement.offer.hero_cp = 0;
        judgement.offer.merchant_cp = 150;
        let after = stage_barter(&current, 82, judgement);
        let record = living_scene::exchange_record(
            after.state(),
            OperationId::from_bytes(&[82; 16]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            record.outcome,
            living_scene::MerchantExchangeOutcome::Countered
        );
        assert_eq!(
            living_scene::snapshot(after.state(), &scene)
                .unwrap()
                .unwrap()
                .model,
            before
        );
        assert_eq!(hero_copper(&after, member), hero_copper(&current, member));
        assert!(
            living_scene::line(after.state(), OperationId::from_bytes(&[82; 16]).unwrap())
                .unwrap()
                .contains("choose stock")
        );
    });
}

#[test]
fn merchant_barter_judged_stance_moves_the_margin_without_changing_source_values() {
    use merchant_bargaining::MerchantStance;
    barter_market(|| {
        let (current, member, judgement) = barter_inventory();
        for (stance, expected_coin, expected_outcome) in [
            (
                MerchantStance::Eager,
                300,
                living_scene::MerchantExchangeOutcome::Settled,
            ),
            (
                MerchantStance::Fair,
                350,
                living_scene::MerchantExchangeOutcome::Countered,
            ),
            (
                MerchantStance::TightFisted,
                375,
                living_scene::MerchantExchangeOutcome::Countered,
            ),
            (
                MerchantStance::Offended,
                400,
                living_scene::MerchantExchangeOutcome::Countered,
            ),
        ] {
            let mut offer = judgement.clone();
            offer.stance = stance;
            // Equipment still contributes 150 CP; 300 CP makes this a 10% concession.
            offer.offer.hero_cp = 300;
            let after = stage_barter(&current, 82, offer);
            let record = living_scene::exchange_record(
                after.state(),
                OperationId::from_bytes(&[82; 16]).unwrap(),
            )
            .unwrap();
            assert_eq!(record.outcome, expected_outcome);
            assert_eq!(record.judgement.offer.hero_cp, expected_coin);
            assert_eq!(
                hero_copper(&after, member),
                hero_copper(&current, member)
                    - if expected_outcome == living_scene::MerchantExchangeOutcome::Settled {
                        300
                    } else {
                        0
                    }
            );
        }
    });
}

fn interact(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
    target: &str,
    option: &str,
) -> Checkpoint {
    // Taking someone else's item is a checked theft: a failing steal rolls 1, and a take of an
    // owned item rolls a natural 20 so it succeeds. Ordinary takes of unowned items never roll.
    let roll = match option {
        "steal" => Some(1),
        "take" => Some(20),
        _ => None,
    };
    interact_with_roll(current, member, op, target, option, roll)
}

fn interact_with_recipient(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
    target: &str,
    option: &str,
    recipient: &str,
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
        (
            model::label("scene-recipient").unwrap(),
            model::label(recipient).unwrap(),
        ),
    ];
    stage_with_supplier(
        current,
        &input(current, member, op, "script-choice", choices),
        &mut |_| panic!("an ordinary hand-over must not roll"),
    )
    .unwrap_or_else(|error| {
        panic!(
            "{option} {target} to {recipient}: {error:?}; step={:?}",
            refusal_detail::take()
        )
    })
}

fn interact_with_roll(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
    target: &str,
    option: &str,
    roll: Option<u32>,
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
        &mut |sides| match roll {
            Some(face) if sides == 20 => Ok(face),
            Some(_) => panic!("theft uses one d20"),
            None => panic!("ordinary scene interaction must not roll"),
        },
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
        let hand_card = living_scene::item_for(opened.state(), &scene, "Brynn")
            .unwrap()
            .expect("a taken object remains reviewable on the player's phone");
        assert_eq!(hand_card.thing_id, thing.id);
        assert!(hand_card.observation_only);
        assert_eq!(hand_card.holder, "Brynn");
        let returned = interact(&opened, member, 12, &thing.id, "return");
        assert!(
            living_scene::item_for(returned.state(), &scene, "Brynn")
                .unwrap()
                .is_none(),
            "returning an observation-only object clears the hand review card"
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
fn inventory_projection_uses_durable_actor_refs_for_held_items() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let thing = model
            .things
            .iter()
            .find(|thing| thing.portable && !thing.in_use && thing.holder_id.is_none())
            .unwrap();
        let taken = interact(&current, member, 30, &thing.id, "take");
        let now = living_scene::snapshot(taken.state(), &scene)
            .unwrap()
            .unwrap();
        let hero = taken
            .state()
            .characters
            .iter()
            .find(|character| character.owner == member)
            .unwrap();
        let expected = super::canonical_actor_ref(taken.state(), hero.entity).unwrap();
        let entries = living_scene::inventory_projection(
            taken.state(),
            &now.model,
            &scene.setting.as_ref().unwrap().place_id,
        )
        .unwrap();
        let held = entries
            .iter()
            .find(|entry| entry.item_id == thing.id)
            .unwrap();
        assert_eq!(held.holder_id.as_deref(), Some(expected.as_str()));
        assert_eq!(
            held.location,
            crate::gameplay::adventure::scene_model::SceneInventoryLocation::Held {
                holder_id: expected,
            }
        );
    });
}

#[test]
fn explicit_steal_text_routes_to_server_inventory_interaction() {
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let thing = model
            .things
            .iter()
            .find(|thing| {
                thing.portable
                    && !thing.in_use
                    && thing.holder_id.is_none()
                    && thing.owner_id.is_some()
            })
            .unwrap();
        let steal = format!("I steal the {}.", thing.name);
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &steal, Ok("steal")).0,
            Some((thing.id.clone(), "steal"))
        );
        let attempted = interact(&current, _member, 20, &thing.id, "steal");
        let after = living_scene::snapshot(attempted.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            after
                .model
                .things
                .iter()
                .find(|candidate| candidate.id == thing.id)
                .unwrap()
                .holder_id,
            None
        );
        assert!(
            living_scene::last_line(attempted.state())
                .unwrap()
                .unwrap()
                .contains("fails to steal")
        );
        assert_eq!(attempted.state().draws.last().unwrap().value, 1);
    });
}

#[test]
fn a_successful_rules_theft_changes_custody_and_preserves_the_owner() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let thing = model
            .things
            .iter()
            .find(|thing| {
                thing.portable
                    && !thing.in_use
                    && thing.holder_id.is_none()
                    && thing
                        .owner_id
                        .as_deref()
                        .is_some_and(|owner| owner != "Brynn")
            })
            .expect("the room has an owned portable object");
        let owner = thing.owner_id.clone();
        let stolen = interact_with_roll(&current, member, 21, &thing.id, "steal", Some(20));
        let after = living_scene::snapshot(stolen.state(), &scene)
            .unwrap()
            .unwrap();
        let result = after
            .model
            .things
            .iter()
            .find(|candidate| candidate.id == thing.id)
            .unwrap();
        assert_eq!(result.owner_id, owner);
        assert_eq!(result.holder_id.as_deref(), Some("Brynn"));
        assert_eq!(stolen.state().draws.last().unwrap().value, 20);
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
                .ends_with(&format!(" {}.", living_scene::coin_phrase(offer.cost_cp)))
        );
        let bought_card = living_scene::item_for(bought.state(), &scene, "Brynn")
            .unwrap()
            .expect("the purchased item remains reviewable");
        assert!(!bought_card.observation_only);
        assert_eq!(bought_card.holder, "Brynn");
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
                .any(|thing| thing.id == offer.item_id)
        );
    });
}

#[test]
fn the_display_receives_the_current_check_faces_outcome_and_target_without_dc_prose() {
    taproom(|| {
        let (current, member) = opened();
        let opening = wire::journey_view(&current, LocalDemoRole::Display, member).unwrap();
        let Some(rpc::view_message::Audience::Display(opening)) = opening.audience else {
            panic!("display required");
        };
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
        assert_eq!(display.narration, opening.narration);
        assert!(!display.narration.contains("DC"));
        assert!(!display.narration.contains("Wisdom (Perception)"));

        let scene = crate::gameplay::beat_script_runtime::scene(searched.state()).unwrap();
        let thing = scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .things
            .iter()
            .find(|thing| thing.portable && !thing.in_use && thing.holder_id.is_none())
            .unwrap();
        let taken = interact(&searched, member, 15, &thing.id, "take");
        let view = wire::journey_view(&taken, LocalDemoRole::Display, member).unwrap();
        let Some(rpc::view_message::Audience::Display(after_take)) = view.audience else {
            panic!("display required");
        };
        assert!(after_take.check.is_none());
        assert_eq!(after_take.narration, opening.narration);
        assert!(
            living_scene::last_line(taken.state())
                .unwrap()
                .unwrap()
                .contains(&thing.name)
        );
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
    use crate::gameplay::approach_model::JevUnavailable;
    use crate::gameplay::journey::merchant_bargaining::TradeJudgement;
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    scene.setting = Some(generated.settings.meeting);
    // Jev's none label executes nothing for these lines, and a missing label is neutral too.
    for words in [
        "What if I open the door?",
        "I do not pick up the tankard",
        "I might open the door",
        "I open the door?",
        "I do not want a pint of beer",
    ] {
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, words, Ok("none")).0,
            None,
            "{words}"
        );
        assert_eq!(
            living_scene::interaction_from_judgement(
                &scene,
                words,
                Err(JevUnavailable::ProviderUnavailable)
            )
            .0,
            None,
            "{words}"
        );
    }
    // A plain open line still runs when Jev labels it open.
    let (door, judgement) =
        living_scene::interaction_from_judgement(&scene, "I open the door", Ok("open"));
    assert_eq!(door.map(|(_, option)| option), Some("open"));
    assert_eq!(judgement, TradeJudgement::Jev);
}

#[test]
fn a_give_line_without_a_jev_label_is_neutral_and_interacts_with_nothing() {
    use crate::gameplay::approach_model::JevUnavailable;
    use crate::gameplay::journey::merchant_bargaining::TradeJudgement;
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        // The keyword gift and give rules are gone: a provider outage names no interaction.
        assert_eq!(
            living_scene::interaction_from_judgement(
                &scene,
                "I give the travelers clothes to Corin.",
                Err(JevUnavailable::ProviderUnavailable),
            ),
            (
                None,
                TradeJudgement::Rules(JevUnavailable::ProviderUnavailable)
            )
        );
    });
}

#[test]
fn a_coin_amount_is_one_unit_phrase_in_copper_and_several_or_none_is_no_amount() {
    assert_eq!(living_scene::coin_amount_cp("I give Ilsa 5 GP."), Some(500));
    assert_eq!(living_scene::coin_amount_cp("12 gold"), Some(1200));
    assert_eq!(
        living_scene::coin_amount_cp("I give Ilsa 5 GP or 10 GP"),
        None
    );
    assert_eq!(living_scene::coin_amount_cp("no coins"), None);
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
                .contains("five gold pieces")
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
    use crate::gameplay::approach_model::tests::{answer, model as jev_model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        // With no scene options, Jev's only option is the absent one.
        let named = FakeEndpoint::start(vec![answer("o1")]);
        let target = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(
                jev_model(&named)
                    .resolve_investigation_target("I examine the golden telescope.", &[]),
            );
        let after = interact(&current, member, 30, &target.id, "examine");
        let receipt = accepted(after.state().decisions.last().unwrap()).unwrap();
        assert!(receipt.check.is_none());
        // ab1865ef changed the replayed absence line to third-person observation wording.
        assert_eq!(
            living_scene::last_line(after.state()).unwrap().unwrap(),
            "Nothing of that is visible here."
        );
        assert_eq!(after.state().draws, current.state().draws);
        assert_eq!(phase(&after).unwrap(), phase(&current).unwrap());
        assert_eq!(
            living_scene::snapshot(after.state(), &scene).unwrap(),
            living_scene::snapshot(current.state(), &scene).unwrap()
        );
    });
}

#[test]
fn an_ordinary_counter_without_a_finding_is_described_without_drawing_a_die() {
    use crate::gameplay::approach_model::tests::{answer, model as jev_model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let catalog = crate::gameplay::investigation::targets(&scene, &[]);
        // Jev picks the first listed surface for the counter.
        let surface = catalog
            .iter()
            .position(|entry| entry.id.starts_with("surface-"))
            .unwrap();
        let named = FakeEndpoint::start(vec![answer(&format!("o{}", surface + 1))]);
        let target = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(
                jev_model(&named)
                    .resolve_investigation_target("I look behind the counter.", &catalog),
            );
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
        assert_eq!(card.thing_id, offer.item_id);
        assert!(
            card.detail
                .contains(offer.description.trim_end_matches('.'))
        );
        assert!(
            living_scene::item_for(after.state(), &scene, "Vale")
                .unwrap()
                .is_none()
        );
        let owned_id = offer.item_id.clone();
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
        let phone = wire::journey_view(&after, LocalDemoRole::Player, member).unwrap();
        let Some(rpc::view_message::Audience::Player(phone)) = phone.audience else {
            panic!("player view");
        };
        let sheet = phone
            .journey
            .as_ref()
            .and_then(|journey| journey.own_character.as_ref())
            .expect("the buyer's phone includes their character sheet");
        let owned_card = sheet
            .owned_items
            .iter()
            .find(|item| item.thing_id == owned_id)
            .expect("the purchase appears in permanent inventory");
        let actor_ref =
            super::canonical_actor_ref(after.state(), player_entity(member, &after).unwrap())
                .unwrap();
        assert_eq!(owned_card.owner_id, actor_ref);
        assert_eq!(owned_card.holder_id, actor_ref);
        assert!(!owned_card.observation_only);
        assert!(
            phone
                .scene
                .as_ref()
                .unwrap()
                .hand_items
                .iter()
                .all(|item| item.thing_id != owned_id),
            "owned property is not mislabeled as a temporary hand observation"
        );
        let coin = sheet
            .facts
            .iter()
            .find(|fact| fact.label == "Coin")
            .expect("the current coin balance remains on the phone sheet");
        let balance =
            living_scene::balance_cp(after.state(), player_entity(member, &after).unwrap())
                .unwrap();
        assert_eq!(
            coin.value,
            format!("{} GP, {} CP", balance / 100, balance % 100)
        );
        let other = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let other_phone = wire::journey_view(&after, LocalDemoRole::Player, other).unwrap();
        let Some(rpc::view_message::Audience::Player(other_phone)) = other_phone.audience else {
            panic!("other player view");
        };
        assert!(
            other_phone
                .journey
                .as_ref()
                .and_then(|journey| journey.own_character.as_ref())
                .unwrap()
                .owned_items
                .iter()
                .all(|item| item.thing_id != owned_id)
        );
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
                .contains("Brynn owns ")
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
fn hand_inventory_lists_each_current_observation_and_clears_only_the_returned_item() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let targets = model
            .things
            .iter()
            .filter(|thing| thing.portable && !thing.in_use && thing.holder_id.is_none())
            .take(2)
            .map(|thing| thing.id.clone())
            .collect::<Vec<_>>();
        assert!(
            targets.len() == 2,
            "shop fixture needs two portable objects"
        );
        let first = interact(&current, member, 41, &targets[0], "take");
        let second = interact(&first, member, 42, &targets[1], "take");

        let held = living_scene::hand_items(
            second.state(),
            &scene,
            Some("Brynn"),
            second.basis().revision.sequence(),
        )
        .unwrap();
        assert_eq!(held.len(), 2);
        assert_eq!(
            held.iter()
                .map(|item| item.thing_id.as_str())
                .collect::<Vec<_>>(),
            targets.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(held.iter().all(|item| item.observation_only));
        assert!(
            living_scene::hand_items(
                second.state(),
                &scene,
                Some("Vale"),
                second.basis().revision.sequence(),
            )
            .unwrap()
            .is_empty(),
            "another hero cannot review Brynn's private hand collection"
        );

        let examined = interact(&second, member, 43, &targets[0], "examine");
        let returned = interact(&examined, member, 44, &targets[0], "return");
        let remaining = living_scene::hand_items(
            returned.state(),
            &scene,
            Some("Brynn"),
            returned.basis().revision.sequence(),
        )
        .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].thing_id, targets[1]);
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
        let projected_scene = player.scene.unwrap();
        let item = projected_scene.item.as_ref().unwrap();
        assert_eq!(item.thing_id, model.things[0].id);
        assert_eq!(item.holder, "Brynn");
        assert_eq!(
            item.holder_id,
            canonical_actor_ref(current.state(), player_entity(member, &current).unwrap()).unwrap()
        );
        let contents = projected_scene.scene_contents_json;
        let public: serde_json::Value = serde_json::from_str(&contents).unwrap();
        // 7bac72cb moved custody out of the art's thing rows into canonical inventory.
        let held = public["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["item_id"] == model.things[0].id)
            .unwrap();
        assert_eq!(held["holder_id"], "Brynn");
        assert_eq!(held["location"]["held"]["holder_id"], "Brynn");
        assert_eq!(held["available"], false);
        assert_eq!(public["ways_out"][0]["open"], true);
        assert!(!contents.contains("knowledge"));
        assert!(!contents.contains("finding_id"));
    });
}

#[test]
fn interaction_labels_are_a_closed_set_that_names_existing_kinds() {
    assert_eq!(living_scene::InteractionMeaning::CHOICES.len(), 15);
    for (label, _) in living_scene::InteractionMeaning::CHOICES {
        assert!(
            living_scene::InteractionMeaning::from_label(label).is_some(),
            "{label}"
        );
    }
    assert_eq!(
        living_scene::InteractionMeaning::from_label("none"),
        Some(living_scene::InteractionMeaning::NotInteraction)
    );
    assert_eq!(living_scene::InteractionMeaning::from_label("use"), None);
}

#[test]
fn the_give_and_gift_options_are_the_labels_own_and_none_has_no_option() {
    use living_scene::InteractionMeaning;
    assert_eq!(InteractionMeaning::Give.option(), Some("give"));
    assert_eq!(InteractionMeaning::Gift.option(), Some("gift"));
    assert_eq!(InteractionMeaning::NotInteraction.option(), None);
}

#[test]
fn jev_gift_and_give_labels_pick_their_own_option_even_when_the_line_names_coin() {
    use crate::gameplay::journey::merchant_bargaining::TradeJudgement;
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let people = scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .for_scene(scene.key);
        let npc = people
            .people
            .iter()
            .find(|person| person.role != "hero" && !person.is_background)
            .unwrap();
        let line = format!("I give {} 5 GP.", npc.name);
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &line, Ok("gift")),
            (Some((npc.id.clone(), "gift")), TradeJudgement::Jev)
        );
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &line, Ok("give")),
            (Some((npc.id.clone(), "give")), TradeJudgement::Jev)
        );
    });
}

#[test]
fn jev_meaning_decides_the_scene_interaction_and_no_label_is_neutral() {
    use crate::gameplay::approach_model::JevUnavailable;
    use crate::gameplay::journey::merchant_bargaining::TradeJudgement;
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let thing = model
            .things
            .iter()
            .find(|thing| {
                thing.portable
                    && !thing.in_use
                    && thing.holder_id.is_none()
                    && thing.owner_id.is_some()
            })
            .unwrap();
        let steal = format!("I steal the {}.", thing.name);
        let question = format!("Can I take the {}?", thing.name);
        // Jev's take label is executed on a question-form line: the keyword gate is not in Jev's path.
        assert_eq!(
            living_scene::interaction_for_meaning(
                &scene,
                &question,
                living_scene::InteractionMeaning::Take
            ),
            Some((thing.id.clone(), "take"))
        );
        // Jev's none label is not an interaction, even for a line that names a steal.
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &steal, Ok("none")),
            (None, TradeJudgement::Jev)
        );
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &steal, Ok("steal")),
            (Some((thing.id.clone(), "steal")), TradeJudgement::Jev)
        );
        // Without a Jev label the line is neutral: no interaction, and the reason is audited.
        let unavailable = Err(JevUnavailable::ProviderUnavailable);
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &steal, unavailable),
            (
                None,
                TradeJudgement::Rules(JevUnavailable::ProviderUnavailable)
            )
        );
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &question, unavailable).0,
            None
        );
        // A label outside the closed set is neutral and audited as an unknown choice.
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &steal, Ok("use")),
            (None, TradeJudgement::Rules(JevUnavailable::UnknownChoice))
        );
    });
}

#[test]
fn merchant_trade_terms_name_the_merchant_and_every_party_hero() {
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let ale = model
            .offers
            .iter()
            .find(|offer| offer.name.contains("ale"))
            .unwrap();
        let seller = model
            .people
            .iter()
            .find(|person| person.id == ale.seller_id)
            .unwrap();
        let terms = living_scene::merchant_trade_terms(&current, "Brynn").unwrap();
        let term = terms
            .iter()
            .find(|term| term.item_id == ale.canonical_item_id())
            .unwrap();
        assert_eq!(term.counterparty_name, seller.name);
        assert!(term.party_hero_names.contains(&"Brynn".to_owned()));
        assert!(term.party_hero_names.contains(&"Vale".to_owned()));
    });
}

#[test]
fn a_purchase_line_names_the_item_and_not_its_description() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let offer = model
            .offers
            .iter()
            .find(|offer| offer.name.contains("ale"))
            .unwrap();
        let after = interact(&current, member, 42, &offer.id, "buy");
        let line = living_scene::last_line(after.state()).unwrap().unwrap();
        assert!(
            line.contains(&format!("Brynn the {}", offer.name)),
            "{line}"
        );
        assert!(
            !line.contains(offer.description.trim_end_matches('.')),
            "the narrator line must name the item, not its description: {line}"
        );
    });
}

#[test]
fn a_hand_over_names_a_party_hero_after_to_and_moves_the_held_item() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let ale = model
            .offers
            .iter()
            .find(|offer| offer.name.contains("ale"))
            .unwrap();
        let item_id = ale.canonical_item_id();
        let companion = model
            .people
            .iter()
            .find(|person| person.name == "Vale" && person.role == "hero")
            .unwrap();
        // This test hands the item over by its recipient id; Jev's recipient choice is tested below.
        let bought = interact(&current, member, 40, &ale.id, "buy");
        let owned = living_scene::snapshot(bought.state(), &scene)
            .unwrap()
            .unwrap();
        let before = owned
            .model
            .things
            .iter()
            .find(|thing| thing.id == item_id)
            .unwrap();
        assert_eq!(before.holder_id.as_deref(), Some("Brynn"));
        assert_eq!(before.owner_id.as_deref(), Some("Brynn"));
        let handed = interact_with_recipient(&bought, member, 41, &item_id, "give", &companion.id);
        let now = living_scene::snapshot(handed.state(), &scene)
            .unwrap()
            .unwrap();
        let thing = now
            .model
            .things
            .iter()
            .find(|thing| thing.id == item_id)
            .unwrap();
        assert_eq!(thing.holder_id.as_deref(), Some(companion.id.as_str()));
        // The existing custody rule: when the giver owns the item, the recipient becomes its owner.
        assert_eq!(thing.owner_id.as_deref(), Some(companion.id.as_str()));
        assert!(
            living_scene::last_line(handed.state())
                .unwrap()
                .unwrap()
                .contains(&format!("gives the {} to {}", ale.name, companion.name))
        );
    });
}

#[test]
fn jev_names_the_listed_person_a_hand_over_goes_to_and_none_names_no_one() {
    use crate::gameplay::approach_model::tests::{answer, model as jev_model};
    use crate::gameplay::approach_model::{ApproachModel, JevUnavailable};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        // Party heroes reach the recipient list through the snapshot that dialogue projects.
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let vale = snapshot
            .model
            .people
            .iter()
            .find(|person| person.name == "Vale" && person.role == "hero")
            .unwrap()
            .id
            .clone();
        let mut projected = scene.clone();
        projected.setting.as_mut().unwrap().model = Some(snapshot.model);
        let choices = living_scene::recipient_choices(&projected).unwrap();
        let vale_position = choices
            .person_ids
            .iter()
            .position(|id| *id == vale)
            .unwrap();
        let words = "I hand the travelers clothes to Vale.";
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let named = FakeEndpoint::start(vec![answer(&format!("o{}", vale_position + 1))]);
            assert_eq!(
                living_scene::judge_recipient(&jev_model(&named), words, &choices).await,
                Ok(Some(vale.clone()))
            );
            assert_eq!(named.bodies()[0]["state"]["hero_line"], words);

            let none = FakeEndpoint::start(vec![answer(&format!("o{}", choices.criteria.len()))]);
            assert_eq!(
                living_scene::judge_recipient(&jev_model(&none), words, &choices).await,
                Ok(None)
            );

            let unavailable =
                living_scene::judge_recipient(&ApproachModel::default(), words, &choices).await;
            assert_eq!(unavailable, Err(JevUnavailable::ProviderUnavailable));
        });
    });
}

/// The first owned portable scene item that is not already held by the test hero.
fn owned_portable_thing(model: &crate::gameplay::adventure::scene_model::SceneModel) -> String {
    model
        .things
        .iter()
        .find(|thing| {
            thing.portable
                && !thing.in_use
                && thing.holder_id.is_none()
                && thing
                    .owner_id
                    .as_deref()
                    .is_some_and(|owner| owner != "Brynn")
        })
        .expect("the room has an owned portable object")
        .id
        .clone()
}

#[test]
fn a_steal_receipt_with_a_lower_dc_is_rejected_on_replay() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let attempted = interact_with_roll(&current, member, 22, &item, "steal", Some(1));
        assert!(
            living_scene::snapshot(attempted.state(), &scene)
                .unwrap()
                .is_some()
        );
        let mut forged = attempted.state().clone();
        let decision = forged.decisions.last_mut().unwrap();
        let semantic = decision.semantic_output.as_mut().unwrap();
        let honest = semantic.clone();
        *semantic = semantic.replacen("\"difficulty_class\":10", "\"difficulty_class\":5", 1);
        assert_ne!(
            *semantic, honest,
            "the receipt records its difficulty class"
        );
        assert!(
            living_scene::snapshot(&forged, &scene).is_err(),
            "replay recomputes the DC and refuses a receipt that lowers it"
        );
    });
}

#[test]
fn a_successful_steal_records_no_witness_and_a_noticed_failure_names_the_noticer() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let stolen = interact_with_roll(&current, member, 23, &item, "steal", Some(20));
        let after = living_scene::snapshot(stolen.state(), &scene)
            .unwrap()
            .unwrap();
        assert!(
            after.model.witnessed_thefts.is_empty(),
            "a successful theft is not noticed"
        );
        let failed = interact_with_roll(&current, member, 24, &item, "steal", Some(1));
        let after = living_scene::snapshot(failed.state(), &scene)
            .unwrap()
            .unwrap();
        let [witness] = after.model.witnessed_thefts.as_slice() else {
            panic!("a failed theft is one witnessed attempt");
        };
        assert!(!witness.noticer_id.is_empty());
        assert!(!witness.member_id.is_empty());
        assert!(
            living_scene::last_line(failed.state())
                .unwrap()
                .unwrap()
                .contains("notices")
        );
    });
}

#[test]
fn a_theft_attempt_orders_the_hero_row_then_the_roll_row_then_the_outcome() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let attempted = interact_with_roll(&current, member, 24, &item, "steal", Some(1));
        let operation = OperationId::from_bytes(&[24; 16]).unwrap();
        let hero = living_scene::hero(attempted.state(), operation).unwrap();
        let outcome = living_scene::last_line(attempted.state()).unwrap().unwrap();
        let check_row = living_scene::last_check_line(attempted.state())
            .unwrap()
            .unwrap();
        let roll = check_row.roll.clone().expect("a theft roll is structured");
        assert_eq!(roll.faces, [1]);
        assert_eq!(roll.kept, 1);
        assert!(!roll.succeeded);
        assert_eq!(roll.label, format!("{hero} \u{b7} Sleight of Hand"));
        let check = check_row.text.clone();
        assert!(check.starts_with(&format!("{hero}: ")), "{check}");
        assert!(check.contains("Dexterity (Sleight of Hand) 1 "), "{check}");
        assert!(check.ends_with("— failure."), "{check}");
        assert!(outcome.contains("fails to steal"), "{outcome}");

        let log = crate::gameplay::table_log::TableLog::new();
        let words = "I slip the gloves into my satchel.";
        crate::gameplay::freeform_action::publish_hero_line(&log, &hero, 0, words);
        log.push_scene_outcome(log.reserve(), 0, Some(check_row), &outcome);
        let lines = log.lines();
        let [hero_row, roll_row, outcome_row] = lines.as_slice() else {
            panic!("a theft is three table rows: {lines:?}");
        };
        assert_eq!(hero_row.speaker, hero);
        assert_eq!(hero_row.role, rpc::TableRole::Hero as i32);
        assert_eq!(hero_row.kind, rpc::TableKind::Action as i32);
        assert_eq!(hero_row.text, words);
        assert_eq!(roll_row.speaker, "Roll");
        assert_eq!(roll_row.role, rpc::TableRole::Narrator as i32);
        assert_eq!(roll_row.text, check);
        assert_eq!(outcome_row.speaker, "Narrator");
        assert_eq!(outcome_row.text, outcome);
    });
}

#[test]
fn taking_an_owned_item_openly_requires_the_check() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let attempted = interact_with_roll(&current, member, 25, &item, "take", Some(1));
        assert!(
            living_scene::last_line(attempted.state())
                .unwrap()
                .unwrap()
                .contains("fails to steal"),
            "an open take of someone else's item is checked, not handed over"
        );
        let after = living_scene::snapshot(attempted.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            after
                .model
                .things
                .iter()
                .find(|thing| thing.id == item)
                .unwrap()
                .holder_id,
            None
        );
    });
}

#[test]
fn scene_narration_gives_each_thing_its_article_and_a_capital_at_the_start() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let name = model
            .things
            .iter()
            .find(|thing| thing.id == item)
            .unwrap()
            .name
            .clone();
        // A failed theft names the item with its article.
        let attempted = interact_with_roll(&current, member, 25, &item, "take", Some(1));
        let theft = living_scene::last_line(attempted.state()).unwrap().unwrap();
        assert!(
            theft.contains(&format!("fails to steal the {name}")),
            "{theft}"
        );
        // Handing over an item that is not held by the hero, and that belongs to someone else.
        let handed = interact(&current, member, 26, &item, "give");
        let owned = living_scene::last_line(handed.state()).unwrap().unwrap();
        assert_eq!(
            owned,
            format!("The {name} belongs to someone else; ask its owner first."),
        );
        // Returning an item the hero does not hold.
        let returned = interact(&current, member, 27, &item, "return");
        let unheld = living_scene::last_line(returned.state()).unwrap().unwrap();
        assert_eq!(unheld, format!("You are not holding the {name}."));
        // Opening a way out names it with its article, as the scene narrates it.
        let way = model.ways_out[0].name.clone();
        let opened = interact(&current, member, 28, &model.ways_out[0].id, "open");
        let door = living_scene::last_line(opened.state()).unwrap().unwrap();
        assert!(
            door.ends_with(&format!(
                " opens {}.",
                crate::gameplay::investigation::the_phrase(&way)
            )),
            "{door}"
        );
    });
}

#[test]
fn reading_a_scene_item_while_picking_it_up_is_an_examine_and_never_a_take() {
    use crate::gameplay::approach_model::JevUnavailable;
    use crate::gameplay::journey::merchant_bargaining::TradeJudgement;
    taproom(|| {
        let (current, _member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let name = model
            .things
            .iter()
            .find(|thing| thing.id == item)
            .unwrap()
            .name
            .clone();
        let reads = format!("I pick up the {name} and read it.");
        // Jev's examine label routes a line that picks the item up and reads it to examine.
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &reads, Ok("examine")),
            (Some((item.clone(), "examine")), TradeJudgement::Jev)
        );
        // Without a Jev label the line is neutral: no examine, and no take either.
        let unavailable = Err(JevUnavailable::ProviderUnavailable);
        assert_eq!(
            living_scene::interaction_from_judgement(&scene, &reads, unavailable).0,
            None
        );
        // A clear wish to keep the item, or a bare pick-up, is a take when Jev labels it so.
        let keeps = format!("I pick up the {name} and keep it.");
        let bare = format!("I pick up the {name}.");
        for line in [keeps, bare] {
            assert_eq!(
                living_scene::interaction_from_judgement(&scene, &line, Ok("take"))
                    .0
                    .map(|(_, option)| option),
                Some("take"),
                "{line}"
            );
        }
    });
}

#[test]
fn picking_up_an_owned_item_only_to_read_it_is_an_examine_with_no_theft() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
        let item = owned_portable_thing(model);
        let owner = model
            .things
            .iter()
            .find(|thing| thing.id == item)
            .unwrap()
            .owner_id
            .clone();
        let after = interact(&current, member, 25, &item, "examine");
        let line = living_scene::last_line(after.state()).unwrap().unwrap();
        assert!(
            line.contains("picks up") && line.contains("puts it back"),
            "the hero handles the item and puts it back: {line}"
        );
        assert!(
            last_offence(&after).is_none(),
            "reading someone else's item in front of its owner is not theft"
        );
        let snapshot = living_scene::snapshot(after.state(), &scene)
            .unwrap()
            .unwrap();
        let thing = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.id == item)
            .unwrap();
        assert_eq!(thing.holder_id, None, "an examine changes no custody");
        assert_eq!(thing.owner_id, owner, "an examine changes no ownership");
    });
}

/// Stages one scene action with the server's dice given in order. Every scene check uses one d20.
fn act(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
    choices: &[(&str, &str)],
    faces: &[u32],
) -> Result<Checkpoint, RepositoryError> {
    let mut faces = faces
        .iter()
        .copied()
        .collect::<std::collections::VecDeque<u32>>();
    let choices = choices
        .iter()
        .map(|(key, value)| (model::label(key).unwrap(), model::label(value).unwrap()))
        .collect();
    stage_with_supplier(
        current,
        &input(current, member, op, "script-choice", choices),
        &mut |sides| {
            assert_eq!(sides, 20, "a scene check uses one d20");
            faces.pop_front().ok_or(RepositoryError::InvalidCandidate)
        },
    )
}

/// The copper a party hero holds, from the same ledger a purchase or theft reads.
fn hero_cp(current: &Checkpoint, hero: &str) -> u64 {
    let who = current
        .state()
        .characters
        .iter()
        .find(|character| {
            name(current.state(), character.entity).is_ok_and(|candidate| candidate == hero)
        })
        .expect("a party hero of this name")
        .entity;
    living_scene::balance_cp(current.state(), who).unwrap()
}

fn scene_snapshot(current: &Checkpoint) -> crate::gameplay::adventure::scene_model::SceneNow {
    let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
    living_scene::snapshot(current.state(), &scene)
        .unwrap()
        .unwrap()
}

/// The copper in an NPC's purse as the current scene projects it.
fn npc_cp(current: &Checkpoint, npc: &str) -> u64 {
    let snapshot = scene_snapshot(current);
    u64::from(
        snapshot
            .model
            .people
            .iter()
            .find(|person| person.id == npc)
            .expect("a person in the scene")
            .coin_cp,
    )
}

fn first_npc(current: &Checkpoint) -> String {
    scene_snapshot(current)
        .model
        .people
        .iter()
        .find(|person| person.role != "hero")
        .expect("the room has an NPC")
        .id
        .clone()
}

/// A portable scene item nobody holds.
fn portable_thing(current: &Checkpoint) -> String {
    scene_snapshot(current)
        .model
        .things
        .iter()
        .find(|thing| {
            thing.portable && !thing.in_use && thing.in_scene && thing.holder_id.is_none()
        })
        .expect("the room has a portable object nobody holds")
        .id
        .clone()
}

/// The hero takes a portable item: openly when nobody else owns it, and by a theft (one d20 that
/// wins) when another person owns it.
fn take_thing(current: &Checkpoint, member: MemberId, op: u8, thing: &str) -> Checkpoint {
    match thing_in(current, thing).owner_id {
        Some(owner) if owner != "Brynn" => {
            interact_with_roll(current, member, op, thing, "steal", Some(20))
        }
        _ => interact(current, member, op, thing, "take"),
    }
}

fn thing_in(
    current: &Checkpoint,
    thing_id: &str,
) -> crate::gameplay::adventure::scene_model::SceneThing {
    scene_snapshot(current)
        .model
        .things
        .into_iter()
        .find(|thing| thing.id == thing_id)
        .expect("the thing is in the scene")
}

fn last_offence(current: &Checkpoint) -> Option<living_scene::SceneOffence> {
    let operation = current.state().decisions.last().unwrap().operation;
    living_scene::offence(current.state(), operation).unwrap()
}

#[test]
fn hero_to_hero_coin_moves_the_exact_copper_between_purses() {
    taproom(|| {
        let (current, member) = opened();
        let (brynn, vale) = (hero_cp(&current, "Brynn"), hero_cp(&current, "Vale"));
        let paid = act(
            &current,
            member,
            30,
            &[
                ("scene-target", "Vale"),
                ("scene-option", "gift"),
                ("scene-payment", "20"),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(hero_cp(&paid, "Brynn"), brynn - 20);
        assert_eq!(hero_cp(&paid, "Vale"), vale + 20);
        assert!(
            living_scene::last_line(paid.state())
                .unwrap()
                .unwrap()
                .contains("Brynn hands Vale two silver pieces.")
        );
    });
}

#[test]
fn a_tip_to_an_npc_credits_its_purse_the_exact_copper() {
    taproom(|| {
        let (current, member) = opened();
        let npc = first_npc(&current);
        let (hero_before, purse_before) = (hero_cp(&current, "Brynn"), npc_cp(&current, &npc));
        let tipped = act(
            &current,
            member,
            31,
            &[
                ("scene-target", &npc),
                ("scene-option", "gift"),
                ("scene-payment", "50"),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(hero_cp(&tipped, "Brynn"), hero_before - 50);
        assert_eq!(npc_cp(&tipped, &npc), purse_before + 50);
        // Conservation: the copper the hero gave is exactly the copper the NPC gained.
        assert_eq!(
            hero_cp(&tipped, "Brynn") + npc_cp(&tipped, &npc),
            hero_before + purse_before
        );
    });
}

#[test]
fn an_overdraw_of_coin_transfers_nothing() {
    taproom(|| {
        let (current, member) = opened();
        let npc = first_npc(&current);
        let (hero_before, purse_before) = (hero_cp(&current, "Brynn"), npc_cp(&current, &npc));
        let refused = act(
            &current,
            member,
            32,
            &[
                ("scene-target", &npc),
                ("scene-option", "gift"),
                ("scene-payment", "100000000"),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(hero_cp(&refused, "Brynn"), hero_before);
        assert_eq!(npc_cp(&refused, &npc), purse_before);
        assert!(
            living_scene::last_line(refused.state())
                .unwrap()
                .unwrap()
                .contains("do not have")
        );
    });
}

#[test]
fn a_pickpocket_success_moves_the_bounded_amount_and_a_failure_is_noticed() {
    taproom(|| {
        let (current, member) = opened();
        let npc = first_npc(&current);
        let (hero_before, purse_before) = (hero_cp(&current, "Brynn"), npc_cp(&current, &npc));
        // One tenth of the purse, never more than the cap (scene_model::pickpocket_haul_cp).
        let haul = (purse_before / 10).min(1_000);
        let lifted = act(
            &current,
            member,
            33,
            &[("scene-target", &npc), ("scene-option", "steal")],
            &[20],
        )
        .unwrap();
        assert_eq!(npc_cp(&lifted, &npc), purse_before - haul);
        assert_eq!(hero_cp(&lifted, "Brynn"), hero_before + haul);
        assert!(
            living_scene::last_line(lifted.state())
                .unwrap()
                .unwrap()
                .contains("lifts")
        );
        let missed = act(
            &current,
            member,
            34,
            &[("scene-target", &npc), ("scene-option", "steal")],
            &[1],
        )
        .unwrap();
        assert_eq!(npc_cp(&missed, &npc), purse_before);
        assert_eq!(hero_cp(&missed, "Brynn"), hero_before);
        let failure = living_scene::last_line(missed.state()).unwrap().unwrap();
        assert!(failure.contains("fails to steal"));
        // The owner is on stage, so the owner sets the DC and notices the failed attempt.
        assert!(failure.contains("notices"), "{failure}");
    });
}

#[test]
fn planting_moves_custody_without_changing_the_owner() {
    taproom(|| {
        let (current, member) = opened();
        let thing = portable_thing(&current);
        let taken = take_thing(&current, member, 35, &thing);
        let npc = first_npc(&taken);
        let owner_before = thing_in(&taken, &thing).owner_id;
        let planted = act(
            &taken,
            member,
            36,
            &[
                ("scene-target", &thing),
                ("scene-option", "plant"),
                ("scene-recipient", &npc),
            ],
            &[20],
        )
        .unwrap();
        let after = thing_in(&planted, &thing);
        assert_eq!(after.holder_id.as_deref(), Some(npc.as_str()));
        assert_eq!(after.owner_id, owner_before);
        assert!(
            living_scene::last_line(planted.state())
                .unwrap()
                .unwrap()
                .contains("slips")
        );
    });
}

/// Brynn takes an unowned item and plants it on the NPC, so the NPC holds it for a demand.
fn npc_holding_item(
    current: &Checkpoint,
    member: MemberId,
    op: u8,
) -> (Checkpoint, String, String) {
    let thing = portable_thing(current);
    let taken = take_thing(current, member, op, &thing);
    let npc = first_npc(&taken);
    let planted = act(
        &taken,
        member,
        op + 1,
        &[
            ("scene-target", &thing),
            ("scene-option", "plant"),
            ("scene-recipient", &npc),
        ],
        &[20],
    )
    .unwrap();
    (planted, thing, npc)
}

#[test]
fn a_successful_intimidation_demand_hands_over_the_item_and_records_the_offence() {
    taproom(|| {
        let (current, member) = opened();
        let (planted, thing, npc) = npc_holding_item(&current, member, 37);
        let demanded = act(
            &planted,
            member,
            39,
            &[("scene-target", &thing), ("scene-option", "intimidate")],
            &[20],
        )
        .unwrap();
        assert_eq!(
            thing_in(&demanded, &thing).holder_id.as_deref(),
            Some("Brynn")
        );
        assert_eq!(
            last_offence(&demanded),
            Some(living_scene::SceneOffence::Coerced {
                npc_id: npc,
                complied: true,
            })
        );
    });
}

#[test]
fn a_failed_intimidation_demand_refuses_the_item_and_records_the_offence() {
    taproom(|| {
        let (current, member) = opened();
        let (planted, thing, npc) = npc_holding_item(&current, member, 40);
        let refused = act(
            &planted,
            member,
            42,
            &[("scene-target", &thing), ("scene-option", "intimidate")],
            &[1],
        )
        .unwrap();
        assert_eq!(
            thing_in(&refused, &thing).holder_id.as_deref(),
            Some(npc.as_str())
        );
        assert_eq!(
            last_offence(&refused),
            Some(living_scene::SceneOffence::Coerced {
                npc_id: npc,
                complied: false,
            })
        );
    });
}

#[test]
fn a_forged_coerced_handover_receipt_is_rejected_on_replay() {
    taproom(|| {
        let (current, member) = opened();
        let (planted, thing, _npc) = npc_holding_item(&current, member, 43);
        let refused = act(
            &planted,
            member,
            45,
            &[("scene-target", &thing), ("scene-option", "intimidate")],
            &[1],
        )
        .unwrap();
        let scene = crate::gameplay::beat_script_runtime::scene(refused.state()).unwrap();
        assert!(
            living_scene::snapshot(refused.state(), &scene)
                .unwrap()
                .is_some()
        );
        // Rewrite the refused demand as a handover that the same die does not win.
        let mut forged = refused.state().clone();
        let index = forged.decisions.len() - 1;
        let text = forged.decisions[index]
            .semantic_output
            .clone()
            .expect("the refusal is journaled");
        let text = text
            .replacen("\"coercion_refused\"", "\"coerced_handover\"", 1)
            .replacen("\"succeeded\":false", "\"succeeded\":true", 1);
        forged.decisions[index].semantic_output = Some(text);
        assert!(living_scene::snapshot(&forged, &scene).is_err());
    });
}

#[test]
fn buying_for_another_hero_sets_owner_and_holder_and_the_buyer_pays() {
    taproom(|| {
        let (current, member) = opened();
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
            .unwrap()
            .clone();
        let (brynn, vale) = (hero_cp(&current, "Brynn"), hero_cp(&current, "Vale"));
        let seller_before = npc_cp(&current, &offer.seller_id);
        let bought = act(
            &current,
            member,
            50,
            &[
                ("scene-target", &offer.id),
                ("scene-option", "buy"),
                ("scene-recipient", "Vale"),
            ],
            &[],
        )
        .unwrap();
        let thing = thing_in(&bought, &offer.canonical_item_id());
        assert_eq!(thing.owner_id.as_deref(), Some("Vale"));
        assert_eq!(thing.holder_id.as_deref(), Some("Vale"));
        // Brynn pays the merchant; Vale is neither charged nor credited.
        assert_eq!(hero_cp(&bought, "Brynn"), brynn - u64::from(offer.cost_cp));
        assert_eq!(hero_cp(&bought, "Vale"), vale);
        assert_eq!(
            npc_cp(&bought, &offer.seller_id),
            seller_before + u64::from(offer.cost_cp)
        );
        let line = living_scene::last_line(bought.state()).unwrap().unwrap();
        assert!(line.contains(" Vale the "), "{line}");
        assert!(line.contains("Brynn pays "), "{line}");
        assert!(
            line.ends_with(&format!(" {}.", living_scene::coin_phrase(offer.cost_cp))),
            "{line}"
        );
    });
}

#[test]
fn coin_narration_names_copper_silver_and_gold_coins() {
    assert_eq!(living_scene::coin_phrase(10), "one silver piece");
    assert_eq!(living_scene::coin_phrase(4), "four copper pieces");
    assert_eq!(living_scene::coin_phrase(40), "four silver pieces");
    assert_eq!(
        living_scene::coin_phrase(790),
        "seven gold pieces and nine silver pieces"
    );
    assert_eq!(living_scene::coin_phrase(1_000), "ten gold pieces");
}

#[test]
fn examining_or_talking_to_a_person_reads_the_act_and_the_description_never_the_relation() {
    use crate::gameplay::adventure::scene_model::{ScenePerson, StartingAttitude};
    let person = ScenePerson {
        id: "corin".into(),
        name: "Corin".into(),
        role: "customer".into(),
        description: "Corin examines a jacket seam, a worn leather pack at his feet.".into(),
        relation_to_speaker: "fellow customer".into(),
        position: "table".into(),
        aliases: vec!["Corin".into()],
        is_background: false,
        present_in: vec!["inn".into()],
        attitude: StartingAttitude::Indifferent,
        attitudes: Default::default(),
        knowledge: Vec::new(),
        wants: "Stay out of trouble.".into(),
        coin_cp: 0,
        passive_perception: 10,
    };
    let examined = living_scene::person_look_line("Brakka", &person, "examine");
    assert_eq!(
        examined,
        "Brakka studies Corin. Corin examines a jacket seam, a worn leather pack at his feet."
    );
    assert!(examined.contains(&person.description));
    assert!(!examined.contains(&person.relation_to_speaker));
    let talked = living_scene::person_look_line("Brakka", &person, "talk");
    assert!(talked.contains(&person.description));
    assert!(!talked.contains(&person.relation_to_speaker));
}

#[test]
fn a_purchase_that_names_the_seller_as_recipient_leaves_the_item_with_the_hero() {
    taproom(|| {
        let (current, member) = opened();
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
            .unwrap()
            .clone();
        // "Here you are, Marek": the seller is paid, so naming them never makes them the buyer.
        let bought =
            interact_with_recipient(&current, member, 12, &offer.id, "buy", &offer.seller_id);
        let card = living_scene::item_for(bought.state(), &scene, "Brynn")
            .unwrap()
            .expect("the purchased item remains reviewable");
        assert_eq!(card.holder, "Brynn");
    });
}

#[test]
fn a_ware_bought_in_a_created_place_replays_once_and_the_scene_still_reads() {
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_runtime as runtime, beat_script_variety_tests as native,
        place_creation::tests::{people_slot, place_slot, request, things_slot},
        startup_configuration::ContentRating,
    };
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let made = crate::gameplay::place_creation::build(
            &request(),
            &place_slot().to_string(),
            &people_slot().to_string(),
            &things_slot().to_string(),
        )
        .unwrap();
        let out = match crate::gameplay::place::plan(
            &opening,
            "Brynn",
            OperationId::from_bytes(&[40; 16]).unwrap(),
            "place-mareks-cloth-shop",
            "street-door",
            &made.place_id,
            Some(&made),
        ) {
            Ok(planned) => planned.candidate,
            Err(_) => panic!("the move to the street is refused"),
        };
        let model_of = |current: &Checkpoint| {
            let scene = runtime::scene(current.state()).unwrap();
            living_scene::snapshot(current.state(), &scene)
                .unwrap()
                .unwrap()
                .model
        };
        let offer = model_of(&out).offers[0].clone();
        let bought = interact(&out, native::first(), 41, &offer.id, "buy");
        // The scene the screens read carries the purchase; reading it again replays the journal
        // from the place's own record, so the ware is sold once and nothing is refused.
        let model = model_of(&bought);
        let held = model
            .things
            .iter()
            .filter(|thing| thing.id == offer.canonical_item_id())
            .collect::<Vec<_>>();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].holder_id.as_deref(), Some("Brynn"));
        assert!(!model.offer_available(&offer.id));
        living_scene::publish(&bought).unwrap();
        assert_eq!(model_of(&bought).things, model.things);
    });
}

#[test]
fn hs39_living_scene_trade_unheld_goods_get_a_spoken_refusal_and_held_question_gets_srd_quote() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, teller};
    use merchant_bargaining::{ExchangeChoice, MerchantStance};
    barter_market(|| {
        let (current, member, mut judgement) = barter_inventory();
        judgement.offer.merchant_items.clear();
        judgement.offer.hero_cp = 0;
        judgement.offer.merchant_cp = 0;
        judgement.hero_commits_now = false;
        judgement.choice = ExchangeChoice::Counter;
        judgement.stance = MerchantStance::Eager;
        let held = stage_barter(&current, 181, judgement.clone());
        let quote = living_scene::pending_exchange(&held, "Brynn")
            .unwrap()
            .unwrap();
        assert_eq!(
            quote.judgement.offer.merchant_cp, 150,
            "SRD half price, no eager inflation"
        );
        assert_eq!(hero_copper(&held, member), hero_copper(&current, member));

        let mut plan = serde_json::to_value(judgement).unwrap();
        plan["offer"]["hero_items"] = serde_json::json!([{"item_id":null,"catalog_id":null}]);
        let endpoint = FakeEndpoint::start(vec![Reply::Output(
            serde_json::json!({"exchange":plan.to_string()}),
        )]);
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS39/unheld-sale")
            .join(crate::gameplay::journey::hex(
                &crate::gameplay::model::random::<16>().unwrap(),
            ));
        let storyteller = teller(
            endpoint.address,
            0,
            std::time::Duration::from_secs(5),
            &cache,
        );
        let judgement = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(merchant_bargaining::judge_exchange(
                &storyteller,
                &living_scene::exchange_facts(
                    &current,
                    "Brynn",
                    "Would you buy my crate of rings?",
                )
                .unwrap(),
            ))
            .expect("unheld goods are a valid semantic judgement, not an error toast");
        let refused = stage_barter(&current, 182, judgement);
        assert_eq!(
            living_scene::exchange_record(
                refused.state(),
                OperationId::from_bytes(&[182; 16]).unwrap()
            )
            .unwrap()
            .outcome,
            living_scene::MerchantExchangeOutcome::Refused
        );
        let line = living_scene::line(
            refused.state(),
            OperationId::from_bytes(&[182; 16]).unwrap(),
        )
        .unwrap();
        assert!(line.contains("Show me the goods first"), "{line}");
        assert_eq!(hero_copper(&refused, member), hero_copper(&current, member));
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        assert_eq!(
            living_scene::snapshot(current.state(), &scene).unwrap(),
            living_scene::snapshot(refused.state(), &scene).unwrap()
        );
    });
}

#[test]
fn hs39_living_scene_trade_srd_sale_quote_can_be_accepted_for_every_merchant_stance() {
    use merchant_bargaining::{ExchangeChoice, MerchantStance};
    barter_market(|| {
        let (current, member, judgement) = barter_inventory();
        for stance in [
            MerchantStance::Eager,
            MerchantStance::Fair,
            MerchantStance::TightFisted,
            MerchantStance::Offended,
        ] {
            let mut question = judgement.clone();
            question.offer.merchant_items.clear();
            question.offer.hero_cp = 0;
            question.offer.merchant_cp = 0;
            question.hero_commits_now = false;
            question.choice = ExchangeChoice::Counter;
            question.stance = stance;
            let quoted = stage_barter(&current, 183, question);
            let mut accepted = living_scene::pending_exchange(&quoted, "Brynn")
                .unwrap()
                .unwrap()
                .judgement;
            assert_eq!(accepted.offer.merchant_cp, 150);
            accepted.hero_commits_now = true;
            accepted.choice = ExchangeChoice::Accept;
            let settled = stage_barter(&quoted, 184, accepted);
            assert_eq!(
                living_scene::exchange_record(
                    settled.state(),
                    OperationId::from_bytes(&[184; 16]).unwrap()
                )
                .unwrap()
                .outcome,
                living_scene::MerchantExchangeOutcome::Settled
            );
            assert_eq!(
                hero_copper(&settled, member),
                hero_copper(&current, member) + 150
            );
        }
    });
}

#[test]
fn hs41_living_scene_weapon_readiness_is_journaled_per_hero_and_clears() {
    use living_scene::readiness::confrontation;
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let weapon = |holder: &str| {
            snapshot
                .model
                .things
                .iter()
                .find(|thing| {
                    thing.holder_id.as_deref() == Some(holder) && thing.name == "greatsword"
                })
                .unwrap()
                .id
                .clone()
        };
        let brynn_weapon = weapon("Brynn");
        let vale_weapon = weapon("Vale");
        assert_ne!(brynn_weapon, vale_weapon);
        let drawn = interact(&current, member, 190, &brynn_weapon, "draw");
        let after = living_scene::snapshot(drawn.state(), &scene)
            .unwrap()
            .unwrap();
        assert!(
            after
                .model
                .things
                .iter()
                .find(|thing| thing.id == brynn_weapon)
                .unwrap()
                .picture_description
                .contains("drawn and ready")
        );
        assert_eq!(
            after
                .model
                .things
                .iter()
                .find(|thing| thing.id == vale_weapon)
                .unwrap()
                .position,
            "sheathed"
        );
        assert!(
            confrontation(drawn.state(), OperationId::from_bytes(&[190; 16]).unwrap())
                .unwrap()
                .is_some()
        );
        assert!(
            living_scene::scene_change_journal(&drawn)
                .unwrap()
                .iter()
                .any(|entry| entry.change.entity_id == brynn_weapon
                    && entry.change.description.contains("draws"))
        );
        let vale_member = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let both = interact(&drawn, vale_member, 191, &vale_weapon, "draw");
        assert!(
            confrontation(both.state(), OperationId::from_bytes(&[191; 16]).unwrap())
                .unwrap()
                .is_none()
        );
        let first_away = interact(&both, member, 192, &brynn_weapon, "sheathe");
        let away = interact(&first_away, vale_member, 193, &vale_weapon, "sheathe");
        let after = living_scene::snapshot(away.state(), &scene)
            .unwrap()
            .unwrap();
        assert!(
            after
                .model
                .things
                .iter()
                .filter(|thing| thing.id == brynn_weapon || thing.id == vale_weapon)
                .all(|thing| thing.position == "sheathed"
                    && thing.picture_description.contains("no blade in hand"))
        );
        let again = interact(&away, member, 194, &brynn_weapon, "draw");
        assert!(
            confrontation(again.state(), OperationId::from_bytes(&[194; 16]).unwrap())
                .unwrap()
                .is_some()
        );
    });
}

#[test]
fn hs41_living_scene_departure_clears_drawn_weapons_on_return() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Brynn") && thing.name == "greatsword")
            .unwrap()
            .id
            .clone();
        let drawn = interact(&current, member, 195, &weapon, "draw");
        let entry = crate::gameplay::place::PlaceEntry {
            hero: "Brynn".into(),
            line: "Brynn leaves the taproom.".into(),
            change: crate::gameplay::place::PlaceMove {
                from: scene.setting.as_ref().unwrap().place_id.clone(),
                to: "elsewhere".into(),
                exit: "dev".into(),
                beat: "departure-readiness-fixture".into(),
                created: None,
            },
        };
        let moved = living_scene::commit_place_move(
            &drawn,
            OperationId::from_bytes(&[196; 16]).unwrap(),
            &entry,
        )
        .unwrap();
        let returned = living_scene::snapshot(moved.state(), &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            returned
                .model
                .things
                .iter()
                .find(|thing| thing.id == weapon)
                .unwrap()
                .position,
            "sheathed"
        );
        assert!(
            returned
                .model
                .things
                .iter()
                .find(|thing| thing.id == weapon)
                .unwrap()
                .picture_description
                .contains("no blade in hand")
        );
    });
}

#[tokio::test(flavor = "current_thread")]
async fn hs41_living_scene_model_routes_courier_packet_grab_to_influence_not_coin() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let mut room: crate::gameplay::adventure::scene_model::SceneModel = serde_json::from_slice(
        include_bytes!("../../../../../content/merchant-shop/scene-model.json"),
    )
    .unwrap();
    let courier = room
        .people
        .iter_mut()
        .find(|person| person.role != "hero")
        .unwrap();
    courier.name = "Finn Alder".into();
    courier.coin_cp = 0;
    let courier_id = courier.id.clone();
    room.people.retain(|person| person.id == courier_id);
    let mut packet = room.things[0].clone();
    packet.id = "courier-packet".into();
    packet.name = "packet".into();
    packet.short = "A sealed packet in Finn's coat.".into();
    packet.portable = true;
    packet.in_scene = true;
    packet.holder_id = Some(courier_id.clone());
    packet.owner_id = Some(courier_id);
    room.things = vec![packet];
    let endpoint = FakeEndpoint::start(vec![answer("o1")]);
    let result = living_scene::judge_take_target(&model(&endpoint), &room,
        "Kazrak steps right up to Finn, greatsword out, and reaches for the packet in his coat. Hand it over, little courier.",
        living_scene::InteractionMeaning::Steal).await.unwrap();
    assert_eq!(result, Some(("courier-packet".into(), "intimidate")));
    let body = endpoint.bodies();
    let criteria = &body[0]["questions"]["q1"]["criteria"];
    assert!(
        criteria["o1"]
            .as_str()
            .unwrap()
            .contains("packet carried by Finn Alder")
    );
    assert!(criteria["o3"].as_str().unwrap().contains("coin"));
}

#[test]
fn hs41_living_scene_readiness_rejects_another_heros_weapon_and_supports_npc_custody() {
    use living_scene::readiness::{self, WeaponReadiness};
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let other_weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Vale") && thing.name == "greatsword")
            .unwrap();
        let choices = vec![
            (
                model::label("scene-target").unwrap(),
                model::label(&other_weapon.id).unwrap(),
            ),
            (
                model::label("scene-option").unwrap(),
                model::label("draw").unwrap(),
            ),
        ];
        let refused = stage_with_supplier(
            &current,
            &input(&current, member, 197, "script-choice", choices),
            &mut |_| panic!("draw never rolls"),
        );
        assert!(
            refused.is_err(),
            "a hero cannot ready someone else's weapon"
        );
        let weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Brynn") && thing.name == "greatsword")
            .unwrap()
            .id
            .clone();
        let npc = snapshot
            .model
            .people
            .iter()
            .find(|person| person.role != "hero" && !person.is_background)
            .unwrap()
            .id
            .clone();
        let given = interact_with_recipient(&current, member, 198, &weapon, "give", &npc);
        let after_give = living_scene::snapshot(given.state(), &scene)
            .unwrap()
            .unwrap();
        let drawn = readiness::commit(
            &given,
            OperationId::from_bytes(&[199; 16]).unwrap(),
            &scene,
            &after_give.model,
            readiness::WeaponCondition {
                item_id: weapon.clone(),
                holder_id: npc.clone(),
                readiness: WeaponReadiness::Drawn,
                main_weapon_instead: false,
            },
        )
        .unwrap();
        let after_draw = living_scene::snapshot(drawn.state(), &scene)
            .unwrap()
            .unwrap();
        let held = after_draw
            .model
            .things
            .iter()
            .find(|thing| thing.id == weapon)
            .unwrap();
        assert_eq!(held.holder_id.as_deref(), Some(npc.as_str()));
        assert_eq!(held.position, "drawn");
    });
}

#[test]
fn hs41_living_scene_npc_draw_is_not_self_addressed_and_does_not_suppress_hero_reply() {
    use living_scene::readiness::{self, WeaponCondition, WeaponReadiness};
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Brynn") && thing.name == "greatsword")
            .unwrap()
            .id
            .clone();
        let npc = snapshot
            .model
            .people
            .iter()
            .find(|person| person.role != "hero" && !person.is_background)
            .unwrap()
            .id
            .clone();
        let given = interact_with_recipient(&current, member, 201, &weapon, "give", &npc);
        let after_give = living_scene::snapshot(given.state(), &scene)
            .unwrap()
            .unwrap();
        let npc_drawn = readiness::commit(
            &given,
            OperationId::from_bytes(&[202; 16]).unwrap(),
            &scene,
            &after_give.model,
            WeaponCondition {
                item_id: weapon,
                holder_id: npc.clone(),
                readiness: WeaponReadiness::Drawn,
                main_weapon_instead: false,
            },
        )
        .unwrap();
        assert!(
            readiness::confrontation(
                npc_drawn.state(),
                OperationId::from_bytes(&[202; 16]).unwrap()
            )
            .unwrap()
            .is_none_or(|reaction| reaction.npc_id != npc)
        );
        let hero_weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Vale") && thing.name == "greatsword")
            .unwrap()
            .id
            .clone();
        let hero_drawn = interact(
            &npc_drawn,
            MemberId::from_bytes(&MEMBERS[1]).unwrap(),
            203,
            &hero_weapon,
            "draw",
        );
        assert!(
            readiness::confrontation(
                hero_drawn.state(),
                OperationId::from_bytes(&[203; 16]).unwrap()
            )
            .unwrap()
            .is_some()
        );
    });
}

#[test]
fn hs41_living_scene_model_approved_main_weapon_substitution_is_journaled_and_acknowledged() {
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let snapshot = living_scene::snapshot(current.state(), &scene)
            .unwrap()
            .unwrap();
        let weapon = snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.holder_id.as_deref() == Some("Brynn") && thing.name == "greatsword")
            .unwrap()
            .id
            .clone();
        let choices = vec![
            (
                model::label("scene-target").unwrap(),
                model::label(&weapon).unwrap(),
            ),
            (
                model::label("scene-option").unwrap(),
                model::label("draw").unwrap(),
            ),
            (
                model::label("scene-weapon-reading").unwrap(),
                model::label("main_weapon_instead").unwrap(),
            ),
        ];
        let committed = stage_with_supplier(
            &current,
            &input(&current, member, 204, "script-choice", choices),
            &mut |_| panic!("readying does not roll"),
        )
        .unwrap();
        let line = living_scene::line(
            committed.state(),
            OperationId::from_bytes(&[204; 16]).unwrap(),
        )
        .unwrap();
        assert!(line.contains("has no such weapon"));
        let entity = player_entity(member, &current).unwrap();
        assert_eq!(
            crate::gameplay::freeform_action::hero_possessive(current.state(), entity),
            Some("their".into())
        );
        assert!(line.contains("readies their greatsword instead"), "{line}");
        let semantic = committed
            .state()
            .decisions
            .last()
            .unwrap()
            .semantic_output
            .as_deref()
            .unwrap();
        assert!(semantic.contains("\"main_weapon_instead\":true"));
    });
}

#[test]
fn hs41_living_scene_renamed_weapon_keeps_its_source_type_and_real_committed_name() {
    use crate::gameplay::adventure::scene_model::SceneItemProvenance;
    taproom_with_model(
        |model| {
            let mut weapon = model.things[0].clone();
            let owner = model
                .people
                .iter()
                .find(|person| !person.is_background)
                .unwrap()
                .id
                .clone();
            weapon.id = "source-dawnblade".into();
            weapon.name = "Dawnblade".into();
            weapon.short = "A greatsword named Dawnblade.".into();
            weapon.detail = "Dawnblade is a steel greatsword. Its hilt is bound in leather.".into();
            weapon.picture_description = "A sheathed greatsword with a leather-bound hilt.".into();
            weapon.portable = true;
            weapon.in_use = false;
            weapon.in_scene = true;
            weapon.owner_id = Some(owner.clone());
            weapon.holder_id = Some(owner);
            weapon.aliases = vec!["Dawnblade".into()];
            weapon.finding_id = None;
            weapon.clue_fact_id = None;
            weapon.provenance = SceneItemProvenance {
                source_id: "greatsword".into(),
                ..Default::default()
            };
            *model.things.last_mut().unwrap() = weapon;
            model.validate_structure().unwrap();
        },
        || {
            let (current, member) = opened();
            let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
            let held = interact_with_roll(
                &current,
                member,
                205,
                "source-dawnblade",
                "intimidate",
                Some(20),
            );
            let entity = player_entity(member, &held).unwrap();
            let situation = crate::gameplay::freeform_action::Situation::read(
                held.state(),
                entity,
                &scene,
                &crate::gameplay::wire::scene_cast(&scene),
            )
            .unwrap();
            let target = situation
                .options()
                .into_iter()
                .find(|target| target.id == "source-dawnblade")
                .unwrap();
            assert_eq!(target.name, "greatsword");
            assert!(target.detail.contains("Dawnblade is a greatsword"));
            let drawn = interact(&held, member, 206, "source-dawnblade", "draw");
            assert!(
                living_scene::line(drawn.state(), OperationId::from_bytes(&[206; 16]).unwrap())
                    .unwrap()
                    .contains("Dawnblade")
            );
            let after = living_scene::snapshot(drawn.state(), &scene)
                .unwrap()
                .unwrap();
            let actual = after
                .model
                .things
                .iter()
                .find(|thing| thing.id == "source-dawnblade")
                .unwrap();
            assert_eq!(actual.name, "Dawnblade");
            assert_eq!(actual.position, "drawn");
        },
    );
}

#[test]
fn hs50_living_scene_trade_item_details_are_first_write_and_replay_without_changing_stock_or_coin()
{
    taproom(|| {
        let (current, member) = opened();
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let setting = scene.setting.as_ref().unwrap();
        let model = setting.model.as_ref().unwrap();
        let offer = model.offers.first().unwrap();
        let details = crate::gameplay::journey::merchant_bargaining::OfferDetails {
            item_id: offer.canonical_item_id(),
            material: None,
            colours: vec!["amber".into()],
            sizes: Some("a clay mug".into()),
            quality: Some("freshly drawn".into()),
            origin: Some("locally brewed".into()),
        };
        let operation = OperationId::from_bytes(&[192; 16]).unwrap();
        let candidate = living_scene::commit_offer_details(
            &current,
            current.basis(),
            operation,
            "Brynn",
            &offer.seller_id,
            details.clone(),
        )
        .unwrap();
        let facts = living_scene::merchant_offer_facts(
            candidate.state(),
            model,
            &setting.place_id,
            &offer.seller_id,
        )
        .unwrap();
        let saved = facts
            .iter()
            .find(|fact| fact["item_id"] == details.item_id)
            .unwrap();
        assert_eq!(saved["details"], serde_json::to_value(&details).unwrap());
        assert_eq!(
            living_scene::merchant_offer_facts(
                candidate.state(),
                model,
                &setting.place_id,
                &offer.seller_id
            )
            .unwrap(),
            facts
        );
        assert_eq!(
            living_scene::merchant_trade_terms(&candidate, "Brynn").unwrap(),
            living_scene::merchant_trade_terms(&current, "Brynn").unwrap()
        );
        assert_eq!(
            hero_copper(&candidate, member),
            hero_copper(&current, member)
        );
        assert!(
            living_scene::commit_offer_details(
                &candidate,
                candidate.basis(),
                OperationId::from_bytes(&[193; 16]).unwrap(),
                "Brynn",
                &offer.seller_id,
                details.clone()
            )
            .is_err()
        );
        assert!(
            living_scene::commit_offer_details(
                &candidate,
                current.basis(),
                operation,
                "Brynn",
                &offer.seller_id,
                details.clone()
            )
            .is_err()
        );
        assert!(
            living_scene::commit_offer_details(
                &current,
                current.basis(),
                operation,
                "Brynn",
                "someone-else",
                details
            )
            .is_err()
        );
    });
}
#[test]
fn market_stall_purchase_replays_once_across_the_square_and_close_view() {
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_runtime as runtime, beat_script_variety_tests as native, place,
        place_creation::{self, tests as drafts},
        startup_configuration::ContentRating,
    };
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let market = place_creation::build(
            &drafts::request(),
            &drafts::place_slot().to_string(),
            &drafts::people_slot().to_string(),
            &drafts::things_slot().to_string(),
        )
        .unwrap();
        let moved = |current: &Checkpoint,
                     number: u8,
                     from: &str,
                     exit: &str,
                     to: &str,
                     created: Option<&place::PlaceRecord>| {
            place::plan(
                current,
                "Brynn",
                OperationId::from_bytes(&[number; 16]).unwrap(),
                from,
                exit,
                to,
                created,
            )
            .unwrap_or_else(|error| panic!("market move refused: {error:?}"))
            .candidate
        };
        let model_of = |current: &Checkpoint| {
            let scene = runtime::scene(current.state()).unwrap();
            living_scene::snapshot(current.state(), &scene)
                .unwrap()
                .unwrap()
                .model
        };
        let square = moved(
            &opening,
            40,
            "place-mareks-cloth-shop",
            "street-door",
            &market.place_id,
            Some(&market),
        );
        let keeper = market
            .people
            .iter()
            .find(|person| person.name == "Yara")
            .unwrap()
            .clone();
        let mut assistant = market.people[0].clone();
        assistant.name = "Talla".into();
        let mut spot = market.clone();
        spot.place_id = "place-fish-stall".into();
        spot.origin.id = market.place_id.clone();
        spot.origin.name = market.name.clone();
        spot.name = "fish stall".into();
        spot.short = "the fish stall beside the fountain".into();
        spot.narration = "The party reaches Yara's fish stall.".into();
        spot.speaker.clear();
        spot.greeting.clear();
        spot.market = false;
        spot.stalls.clear();
        spot.exits.clear();
        spot.landmarks.clear();
        spot.local_spot = Some("the stall beside the fountain".into());
        spot.local_keeper = Some(keeper.name.clone());
        spot.people = vec![keeper.clone(), assistant];
        spot.wares.retain(|ware| ware.seller == keeper.name);
        spot.services.clear();
        let exit = place::spot_exit(&spot.name);
        let close = moved(
            &square,
            41,
            &market.place_id,
            &exit,
            &spot.place_id,
            Some(&spot),
        );
        let offer = model_of(&close).offers[0].clone();
        let bought = interact(&close, native::first(), 42, &offer.id, "buy");
        let returned = moved(
            &bought,
            43,
            &spot.place_id,
            place::WAY_BACK,
            &market.place_id,
            None,
        );
        let back = model_of(&returned);
        assert!(!back.offer_available(&offer.canonical_item_id()));
        assert_eq!(
            back.people
                .iter()
                .find(|person| person.id == offer.seller_id)
                .unwrap()
                .coin_cp,
            keeper.purse_cp + offer.cost_cp
        );
        assert_eq!(back, model_of(&returned));
        let again = moved(&returned, 44, &market.place_id, &exit, &spot.place_id, None);
        let close_again = model_of(&again);
        assert!(!close_again.offer_available(&offer.canonical_item_id()));
        assert_eq!(
            close_again
                .people
                .iter()
                .find(|person| person.id == offer.seller_id)
                .unwrap()
                .coin_cp,
            keeper.purse_cp + offer.cost_cp
        );
    });
}
