use super::{
    scene_model::{
        SceneChange, SceneChangeKind, SceneModel, ScenePerson, ScenePlace, SceneTargetKind,
        StartingAttitude,
    },
    setting::{SceneSetting, SettingKind},
};
use std::collections::{BTreeMap, BTreeSet};

const SEED: u64 = 0xa7d1_f297_b79d_0c83;

fn inn() -> SceneSetting {
    SceneSetting {
        place_id: "taproom".into(),
        name: "The Lantern Taproom".into(),
        short: "a warm taproom".into(),
        kind: SettingKind::Inn,
        exterior: false,
        art_key: "taproom".into(),
        model: None,
    }
}

fn facts() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("stake.short".into(), "recover a fever-herb".into()),
        ("patron.full".into(), "Ilsa".into()),
        ("patron.role".into(), "innkeeper".into()),
        (
            "patron.desc".into(),
            "a gray-haired woman at the bar".into(),
        ),
        ("ant.full".into(), "giant rats".into()),
        ("ant.group".into(), "giant rats".into()),
        ("ant.short".into(), "restless giant rats".into()),
        ("ant.voice".into(), "a chittering presence".into()),
    ])
}

fn taproom() -> SceneModel {
    SceneModel::fallback(&inn(), ScenePlace::Meeting, SEED, &facts())
        .expect("the taproom tables provide all named scene facts")
}

fn merchant_shop() -> SceneModel {
    let setting = SceneSetting {
        place_id: "cloth-shop".into(),
        name: "the cloth shop near the market".into(),
        short: "the cloth shop".into(),
        kind: SettingKind::Shop,
        exterior: false,
        art_key: "place-cloth-shop".into(),
        model: None,
    };
    let mut shop_facts = facts();
    shop_facts.insert("patron.full".into(), "Marten Vell".into());
    shop_facts.insert("patron.role".into(), "cloth merchant".into());
    shop_facts.insert(
        "patron.desc".into(),
        "a round merchant in a russet wool cap".into(),
    );
    SceneModel::fallback(&setting, ScenePlace::Meeting, SEED, &shop_facts)
        .expect("the merchant shop tables provide all named scene facts")
}

fn add_test_hero(model: &mut SceneModel, id: &str, name: &str) {
    model.people.push(ScenePerson {
        id: id.to_owned(),
        name: name.to_owned(),
        role: "hero".into(),
        description: "Standing with the party.".into(),
        relation_to_speaker: "visitor".into(),
        position: "foreground".into(),
        aliases: vec![name.to_owned()],
        is_background: false,
        present_in: vec![
            "opening".into(),
            "dialogue_seal".into(),
            "dialogue_escort".into(),
            "room".into(),
            "combat".into(),
            "defeat".into(),
            "victory".into(),
            "inn".into(),
        ],
        attitude: StartingAttitude::Indifferent,
        attitudes: BTreeMap::new(),
        knowledge: Vec::new(),
        wants: "Stay with the party.".into(),
        coin_cp: 0,
    });
}

#[test]
fn authored_merchant_model_declares_current_item_provenance() {
    let model: SceneModel = serde_json::from_str(include_str!(
        "../../../../../content/merchant-shop/scene-model.json"
    ))
    .expect("authored merchant scene model loads");
    assert_eq!(model.inventory_version, 1);
    model
        .validate_structure()
        .expect("authored merchant model meets current inventory bounds");
}

#[test]
fn picture_state_changes_for_visible_actions_but_not_a_search_or_private_knowledge() {
    let model = taproom();
    let initial = model.picture_state().unwrap();
    let mut searched = model.clone();
    searched.things[0].searched_by.push("Brakka".into());
    searched.people[0].knowledge.clear();
    assert_eq!(initial, searched.picture_state().unwrap());
    searched.things[0].holder_id = Some("Brakka".into());
    searched.things[0].position = "held by Brakka".into();
    assert_ne!(initial, searched.picture_state().unwrap());
    assert!(!initial.contains("clue_fact_id"));
    assert!(!initial.contains("closer_look"));
}

#[test]
fn fallback_in_scene_has_people_things_exits_and_srd_521_prices_p101() {
    let model = taproom();

    assert_eq!(model.people.len(), 5);
    assert_eq!(
        model
            .people
            .iter()
            .filter(|person| person.is_background)
            .count(),
        3
    );
    assert_eq!(model.things.len(), 6);
    assert_eq!(model.ways_out.len(), 3);
    assert_eq!(model.offers.len(), 5);
    assert!(
        model
            .offers
            .iter()
            .any(|offer| { offer.name == "ale (mug)" && offer.cost_cp == 4 })
    );
    assert!(
        model
            .offers
            .iter()
            .any(|offer| { offer.name == "bread (loaf)" && offer.cost_cp == 2 })
    );
    assert!(
        model
            .offers
            .iter()
            .any(|offer| { offer.name == "one night's poor lodging" && offer.cost_cp == 10 })
    );
}

#[test]
fn merchant_shop_has_named_interactable_background_people_and_stock() {
    let model = merchant_shop();
    let background = model
        .people
        .iter()
        .filter(|person| person.is_background)
        .collect::<Vec<_>>();
    assert_eq!(background.len(), 2);
    assert!(background.iter().any(|person| person.role == "adventurer"));
    assert!(background.iter().any(|person| person.role == "soldier"));
    assert!(model.resolve_target("Talk to the adventurer").is_some());
    assert!(
        model
            .resolve_target("Ask the soldier about the shop")
            .is_some()
    );
    assert!(
        model
            .resolve_target("What does the merchant sell?")
            .is_some()
    );
    assert_eq!(model.offers.len(), 3);
    assert!(
        model
            .offers
            .iter()
            .any(|offer| offer.name == "traveler's clothes" && offer.cost_cp == 200)
    );
    assert!(
        model
            .offers
            .iter()
            .any(|offer| offer.name == "fine clothes" && offer.cost_cp == 1500)
    );
    let picture = model.picture_state().expect("shop picture state");
    assert!(picture.contains("adventurer"));
    assert!(picture.contains("soldier"));
}

#[test]
fn location_inventory_keeps_stock_identity_separate_from_custody() {
    let model = merchant_shop();
    let stock = model.inventory();
    let clothes = stock
        .iter()
        .find(|entry| entry.source_id == "clothes-travelers")
        .expect("shop stock has a stable clothes entry");
    assert_eq!(clothes.item_id, "item-clothes-travelers");
    assert_eq!(clothes.provenance_kind, "generated_offer");
    assert_eq!(
        clothes.location,
        super::scene_model::SceneInventoryLocation::Stock
    );
    assert!(clothes.available);

    let purchased = model
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: "clothes-travelers".into(),
                description: "Brynn buys traveler's clothes.".into(),
                kind: SceneChangeKind::Purchased {
                    holder_id: "Brynn".into(),
                    seller_id: "patron".into(),
                    cost_cp: 200,
                    quote_id: None,
                },
            }],
        )
        .expect("purchase is a valid committed scene change");
    let snapshot: serde_json::Value = serde_json::from_str(&purchased).expect("snapshot JSON");
    let inventory = snapshot["model"]["things"]
        .as_array()
        .expect("purchased item remains in the model")
        .iter()
        .find(|thing| thing["id"] == "item-clothes-travelers")
        .expect("exactly one purchased instance exists");
    assert_eq!(inventory["provenance"]["kind"], "generated_offer");
    assert_eq!(inventory["provenance"]["source_id"], "clothes-travelers");
    assert_eq!(inventory["holder_id"], "Brynn");
    assert_eq!(inventory["owner_id"], "Brynn");
}

#[test]
fn giving_catalog_stock_materializes_the_same_item_as_a_temporary_loan() {
    let model = merchant_shop();
    let offer = model
        .offer("item-clothes-travelers")
        .expect("the catalog resolves by its stable item ID");
    let seller_id = offer.seller_id.clone();
    let item_id = offer.canonical_item_id();
    let recipient_id = "cloth-shop-bystander-1";

    let snapshot = model
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: item_id.clone(),
                description: "Marek lends the traveler's clothes for inspection.".into(),
                kind: SceneChangeKind::Given {
                    holder_id: recipient_id.into(),
                    owner_id: None,
                },
            }],
        )
        .expect("catalog stock can be materialized for an inspection loan");
    let snapshot: super::scene_model::SceneNow =
        serde_json::from_str(&snapshot).expect("scene snapshot JSON");
    let item = snapshot
        .model
        .things
        .iter()
        .find(|thing| thing.id == item_id)
        .expect("loaned catalog item is materialized once");
    assert_eq!(item.owner_id.as_deref(), Some(seller_id.as_str()));
    assert_eq!(item.holder_id.as_deref(), Some(recipient_id));
    assert_eq!(item.provenance.source_id, "clothes-travelers");
    assert_eq!(
        item.provenance.seller_id.as_deref(),
        Some(seller_id.as_str())
    );
    assert!(!snapshot.model.offer_available(&item_id));

    let inventory = snapshot.model.inventory();
    let entry = inventory
        .iter()
        .find(|entry| entry.item_id == item_id)
        .expect("the canonical inventory reflects the holder");
    assert_eq!(entry.owner_id.as_deref(), Some(seller_id.as_str()));
    assert_eq!(entry.holder_id.as_deref(), Some(recipient_id));
    assert!(matches!(
        entry.location,
        super::scene_model::SceneInventoryLocation::Held { .. }
    ));
    assert_eq!(
        inventory
            .iter()
            .filter(|entry| entry.item_id == item_id)
            .count(),
        1
    );
}

#[test]
fn generated_offer_item_ids_are_canonical_and_legacy_stock_resolves() {
    let mut model = merchant_shop();
    model
        .validate_structure()
        .expect("fallback IDs are canonical");

    let mut wrong_id = model.clone();
    wrong_id.offers[0].item_id = "item-invented-stock".into();
    assert_eq!(
        wrong_id.validate_structure(),
        Err(super::scene_model::SceneModelError::InvalidShape)
    );

    let mut collision = merchant_shop();
    collision.offers[0].item_id = collision.things[0].id.clone();
    assert_eq!(
        collision.validate_structure(),
        Err(super::scene_model::SceneModelError::DuplicateId)
    );

    let mut serialized = serde_json::to_value(&model).expect("model JSON");
    serialized
        .as_object_mut()
        .expect("scene model object")
        .remove("inventory_version");
    for thing in serialized["things"].as_array_mut().expect("things array") {
        thing
            .as_object_mut()
            .expect("thing object")
            .remove("provenance");
    }
    serialized["offers"][0]
        .as_object_mut()
        .expect("offer object")
        .remove("item_id");
    model = serde_json::from_value(serialized).expect("legacy offer row loads");
    assert_eq!(model.inventory_version, 0);
    model
        .migrate_inventory_version()
        .expect("legacy item identities upgrade deterministically");
    model
        .validate_structure()
        .expect("migrated legacy model meets current authoring bounds");
    let entry = model
        .inventory()
        .into_iter()
        .find(|entry| entry.source_id == "clothes-travelers")
        .expect("legacy stock resolves to a concrete item ID");
    assert_eq!(entry.item_id, "item-clothes-travelers");
    assert!(model.offer_available(&entry.item_id));
}

#[test]
fn explicit_current_version_does_not_migrate_malformed_item_provenance() {
    let mut model = merchant_shop();
    model.things[0].provenance.source_id.clear();
    assert_eq!(
        model.validate_structure(),
        Err(super::scene_model::SceneModelError::InvalidBounds)
    );

    model.inventory_version = 2;
    assert_eq!(
        model.validate_structure(),
        Err(super::scene_model::SceneModelError::InvalidBounds)
    );
}

#[test]
fn dynamic_inventory_rejects_unknown_custody_actor_references() {
    let model = merchant_shop();
    let thing = model
        .things
        .iter()
        .find(|thing| thing.portable && thing.owner_id.is_none() && thing.holder_id.is_none())
        .expect("shop has an unowned portable prop");
    let result = model.as_it_is_now(
        super::super::pack::SceneKey::Opening,
        &[SceneChange {
            entity_id: thing.id.clone(),
            description: "An unknown visitor takes the item.".into(),
            kind: SceneChangeKind::Taken {
                holder_id: "untrusted-actor-id".into(),
            },
        }],
    );
    assert_eq!(
        result,
        Err(super::scene_model::SceneModelError::UnknownPerson),
        "journal custody must reference an actor in the trusted scene/party state"
    );
}

#[test]
fn dynamic_inventory_can_exceed_authored_room_item_count() {
    let model = merchant_shop();
    assert!(model.offers.len() > 1);
    let changes = model
        .offers
        .iter()
        .map(|offer| SceneChange {
            entity_id: offer.canonical_item_id(),
            description: format!("Marek lends {} for inspection.", offer.name),
            kind: SceneChangeKind::Given {
                holder_id: "shop-adventurer".into(),
                owner_id: None,
            },
        })
        .collect::<Vec<_>>();
    let snapshot = model
        .as_it_is_now(super::super::pack::SceneKey::Opening, &changes)
        .expect("durable item growth does not reuse authored cardinality bounds");
    let snapshot: super::scene_model::SceneNow =
        serde_json::from_str(&snapshot).expect("snapshot JSON");
    assert!(snapshot.model.things.len() > 6);
    snapshot
        .model
        .validate_state_structure()
        .expect("dynamic inventory is structurally valid");
    assert_eq!(
        snapshot.model.validate_structure(),
        Err(super::scene_model::SceneModelError::InvalidBounds),
        "authored room-shape validation remains a separate constraint"
    );
}

#[test]
fn materialized_offer_keeps_its_stock_id_without_allowing_real_collisions() {
    let original = merchant_shop();
    let offer = original.offers[0].clone();
    let snapshot = original
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: offer.canonical_item_id(),
                description: "Marek lends the clothes for inspection.".into(),
                kind: SceneChangeKind::Given {
                    holder_id: "shop-adventurer".into(),
                    owner_id: None,
                },
            }],
        )
        .expect("the canonical offer can materialize with stable provenance");
    let snapshot: super::scene_model::SceneNow =
        serde_json::from_str(&snapshot).expect("snapshot JSON");
    snapshot
        .model
        .validate_structure()
        .expect("the same canonical ID may be stock or its generated item");
    assert_eq!(
        snapshot
            .model
            .things
            .iter()
            .find(|thing| thing.id == offer.canonical_item_id())
            .unwrap()
            .provenance
            .source_id,
        offer.id
    );

    let mut authored_collision = original.clone();
    authored_collision.things[0].id = offer.canonical_item_id();
    assert!(authored_collision.validate_structure().is_err());

    let mut wrong_source = snapshot.model;
    let materialized = wrong_source
        .things
        .iter_mut()
        .find(|thing| thing.id == offer.canonical_item_id())
        .unwrap();
    materialized.provenance.source_id = "another-offer".into();
    assert!(wrong_source.validate_structure().is_err());
}

#[test]
fn shared_scene_inventory_hides_private_custody_and_redacts_remote_owners() {
    let mut model = merchant_shop();
    let visible_id = model.things[0].id.clone();
    model.things[0].owner_id = Some("unlisted-owner".into());
    model.things[0].holder_id = Some("cloth-shop-bystander-1".into());
    model.things[0].position = "held by cloth-shop-bystander-1".into();

    let private_id = model.things[1].id.clone();
    model.things[1].publicly_visible = false;
    model.things[1].holder_id = Some("unlisted-holder".into());
    model.things[1].owner_id = Some("unlisted-owner".into());

    let public: serde_json::Value =
        serde_json::from_str(&model.public_contents().expect("shared contents")).unwrap();
    assert!(
        public["things"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thing| thing["id"] == visible_id)
    );
    assert!(
        !public["things"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thing| thing["id"] == private_id)
    );
    let visible_inventory = public["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["item_id"] == visible_id)
        .expect("the public held item remains projected");
    assert!(visible_inventory["owner_id"].is_null());
    assert_eq!(visible_inventory["holder_id"], "cloth-shop-bystander-1");
    assert!(
        !public["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["item_id"] == private_id)
    );

    assert!(
        model
            .inventory()
            .iter()
            .any(|entry| entry.item_id == private_id)
    );
}

#[test]
fn public_picture_includes_explicitly_visible_held_items_without_actor_id_leaks() {
    let mut model = merchant_shop();
    let item_id = model.things[0].id.clone();
    model.things[0].holder_id = Some("hero:0123456789abcdef".into());
    model.things[0].position = "held by hero:0123456789abcdef".into();
    let picture: serde_json::Value =
        serde_json::from_str(&model.picture_state().expect("picture state")).unwrap();
    let item = picture["things"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thing| thing["id"] == item_id)
        .expect("explicitly visible item is included without an NPC roster lookup");
    assert!(item.get("holder").is_none());
    assert!(!item.to_string().contains("0123456789abcdef"));
}

#[test]
fn npc_purchase_uses_actor_purse_and_promotes_same_stock_item() {
    let model = merchant_shop();
    let snapshot = model
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: "clothes-travelers".into(),
                description: "Corin buys traveler's clothes.".into(),
                kind: SceneChangeKind::NpcPurchased {
                    holder_id: "cloth-shop-bystander-1".into(),
                    seller_id: "patron".into(),
                    cost_cp: 200,
                },
            }],
        )
        .expect("NPC purchase is a valid committed scene change");
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot JSON");
    let patron = snapshot["model"]["people"]
        .as_array()
        .unwrap()
        .iter()
        .find(|person| person["id"] == "cloth-shop-bystander-1")
        .unwrap();
    assert_eq!(patron["coin_cp"], 300);
    let merchant = snapshot["model"]["people"]
        .as_array()
        .unwrap()
        .iter()
        .find(|person| person["id"] == "patron")
        .unwrap();
    assert_eq!(merchant["coin_cp"], 5_200);
    let item = snapshot["model"]["things"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thing| thing["id"] == "item-clothes-travelers")
        .unwrap();
    assert_eq!(item["holder_id"], "cloth-shop-bystander-1");
    assert_eq!(item["owner_id"], "cloth-shop-bystander-1");
}

#[test]
fn purchase_change_cannot_claim_a_different_seller() {
    let model = merchant_shop();
    let result = model.as_it_is_now(
        super::super::pack::SceneKey::Opening,
        &[SceneChange {
            entity_id: "clothes-travelers".into(),
            description: "Corin buys traveler clothes.".into(),
            kind: SceneChangeKind::Purchased {
                holder_id: "cloth-shop-bystander-1".into(),
                seller_id: "invented-seller".into(),
                cost_cp: 200,
                quote_id: None,
            },
        }],
    );
    assert!(result.is_err());
}

#[test]
fn theft_moves_custody_without_rewriting_owner() {
    let mut model = merchant_shop();
    add_test_hero(&mut model, "hero-brynn", "Brynn");
    model.things[0].owner_id = Some("patron".into());
    let target = model
        .things
        .iter()
        .find(|thing| thing.owner_id.is_some() && thing.portable)
        .expect("shop has an owned portable object")
        .clone();
    let snapshot = model
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: target.id.clone(),
                description: "Brynn steals the object.".into(),
                kind: SceneChangeKind::Stolen {
                    holder_id: "Brynn".into(),
                    adjudication: Some(super::scene_model::SceneStealAdjudication {
                        operation_id: "01010101010101010101010101010101".into(),
                        source_revision: format!(
                            "srd521-p6:test:sleight-of-hand:target-thing-{}",
                            target.id
                        ),
                        actor_id: "Brynn".into(),
                        die: 20,
                        ability_modifier: 0,
                        proficiency_bonus: 0,
                        difficulty_class: 10,
                    }),
                },
            }],
        )
        .expect("theft is a durable custody change");
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot JSON");
    let item = snapshot["model"]["things"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thing| thing["id"] == target.id)
        .unwrap();
    assert_eq!(item["owner_id"], target.owner_id.as_deref().unwrap());
    assert_eq!(item["holder_id"], "Brynn");
    assert_eq!(item["position"], "held by Brynn");
}

#[test]
fn authored_purchased_prefix_object_survives_without_a_purchase_journal() {
    let mut model = merchant_shop();
    model.things[0].id = "authored-purchased-cloth".into();
    let snapshot = model
        .as_it_is_now(super::super::pack::SceneKey::DialogueSeal, &[])
        .expect("authored item remains valid");
    assert!(snapshot.contains("authored-purchased-cloth"));
}

#[test]
fn legacy_scene_setting_without_a_model_still_loads() {
    let mut legacy = serde_json::to_value(inn()).expect("scene setting serializes");
    legacy
        .as_object_mut()
        .expect("scene setting is an object")
        .remove("model");

    let setting: SceneSetting =
        serde_json::from_value(legacy).expect("legacy settings omit the optional model");
    assert!(setting.model.is_none());
}

#[test]
fn thirteen_pointing_lines_resolve_people_things_exits_offers_and_owners() {
    let model = taproom();
    let lines = [
        ("Who are those men in the back?", SceneTargetKind::Person),
        ("I want a pint of beer.", SceneTargetKind::Offer),
        ("Whose mug is that?", SceneTargetKind::Thing),
        ("Talk to the carters.", SceneTargetKind::Person),
        (
            "Does the lady with the tray work here?",
            SceneTargetKind::Person,
        ),
        ("Does she work here with you?", SceneTargetKind::Person),
        ("Look at the mug on the bar.", SceneTargetKind::Thing),
        ("Take the dice at the back table.", SceneTargetKind::Thing),
        ("What's on the left by the hearth?", SceneTargetKind::Thing),
        ("Read the clock on the wall.", SceneTargetKind::Thing),
        ("Sit beside the fire.", SceneTargetKind::Thing),
        ("Open the window.", SceneTargetKind::WayOut),
        ("Take the herbs above the bar.", SceneTargetKind::Thing),
    ];

    for (line, expected_kind) in lines {
        let target = model
            .resolve_target(line)
            .unwrap_or_else(|| panic!("no target for {line:?}"));
        assert_eq!(target.kind, expected_kind, "{line:?}");
    }

    let carters = model
        .resolve_target("Who are those men in the back?")
        .expect("the pictured carters resolve as people");
    assert_eq!(carters.ids.len(), 2);
    let mug = model
        .resolve_target("Whose mug is that?")
        .expect("the pictured nearest mug resolves as a thing");
    let tankard = model
        .things
        .iter()
        .find(|thing| thing.id == mug.ids[0])
        .expect("resolved thing is in the model");
    assert_eq!(tankard.name, "dented pewter tankard");
    assert_eq!(tankard.owner_id.as_deref(), Some("patron"));
}

#[test]
fn structured_scene_model_accepts_grounded_data_and_rejects_untrusted_claims() {
    let original = taproom();
    let schema = SceneModel::schema().expect("scene model schema is valid JSON");
    assert_eq!(schema["additionalProperties"], false);
    assert!(
        schema["properties"]["offers"]["items"]["required"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "item_id"))
    );
    assert!(schema["required"].as_array().is_some_and(|fields| {
        fields.iter().any(|field| field == "people")
            && fields.iter().any(|field| field == "things")
            && fields.iter().any(|field| field == "ways_out")
    }));
    assert!(!SceneModel::prompt().trim().is_empty());
    let mut cast = BTreeMap::from([
        ("patron".into(), "Ilsa".into()),
        ("antagonist".into(), "giant rats".into()),
    ]);
    let place_ids = BTreeSet::from(["orrery-library".to_owned()]);
    let finding_ids = BTreeSet::from(["errand-truth".to_owned(), "enemy-position".to_owned()]);
    let canonical_facts = canonical_facts(&original);
    let valid = serde_json::to_value(&original).expect("model serializes to structured JSON");

    assert!(
        SceneModel::validate_candidate(
            valid.clone(),
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_ok()
    );

    let mut unknown_person = valid.clone();
    unknown_person["people"][0]["id"] = "unlisted-stranger".into();
    assert!(
        SceneModel::validate_candidate(
            unknown_person,
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );

    let mut unknown_clue = valid.clone();
    unknown_clue["things"][0]["clue_fact_id"] = "missing-fact".into();
    assert!(
        SceneModel::validate_candidate(
            unknown_clue,
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );

    let mut unknown_place = valid.clone();
    unknown_place["ways_out"][0]["beyond_place_id"] = "unlisted-place".into();
    assert!(
        SceneModel::validate_candidate(
            unknown_place,
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );

    let mut contradiction = valid.clone();
    contradiction["people"][0]["knowledge"][0]["statement"] = "an unrelated place".into();
    assert!(
        SceneModel::validate_candidate(
            contradiction,
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );

    let mut missing_field = valid;
    missing_field
        .as_object_mut()
        .expect("top-level object")
        .remove("weather");
    assert!(
        SceneModel::validate_candidate(
            missing_field,
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );

    cast.insert("patron".into(), "someone else".into());
    assert!(
        SceneModel::validate_candidate(
            serde_json::to_value(&original).expect("model serializes"),
            &cast,
            &place_ids,
            &canonical_facts,
            &finding_ids,
        )
        .is_err()
    );
}

#[test]
fn scene_now_applies_item_and_person_changes_to_the_prompt_projection() {
    let mut model = taproom();
    add_test_hero(&mut model, "hero-brakka", "Brakka");
    add_test_hero(&mut model, "hero-sigrun", "Sigrun");
    let mug_id = model
        .things
        .iter()
        .find(|thing| thing.name == "dented pewter tankard")
        .expect("taproom model has its bar tankard")
        .id
        .clone();
    let door_id = model.ways_out[0].id.clone();
    let serving_woman_id = model
        .people
        .iter()
        .find(|person| person.role == "serving woman")
        .expect("taproom model has its server")
        .id
        .clone();
    let changes = [
        SceneChange {
            entity_id: mug_id.clone(),
            description: "Brakka took the dented tankard from the bar".into(),
            kind: SceneChangeKind::Taken {
                holder_id: "hero-brakka".into(),
            },
        },
        SceneChange {
            entity_id: mug_id.clone(),
            description: "Brakka gave the tankard to Sigrun".into(),
            kind: SceneChangeKind::Given {
                holder_id: "hero-sigrun".into(),
                owner_id: Some("hero-sigrun".into()),
            },
        },
        SceneChange {
            entity_id: door_id.clone(),
            description: "the front door now stands open".into(),
            kind: SceneChangeKind::Opened,
        },
        SceneChange {
            entity_id: mug_id.clone(),
            description: "Sigrun broke the tankard's handle".into(),
            kind: SceneChangeKind::Broken,
        },
        SceneChange {
            entity_id: serving_woman_id.clone(),
            description: "Mara moved to the doorway".into(),
            kind: SceneChangeKind::PersonMoved {
                position: "at the doorway".into(),
            },
        },
        SceneChange {
            entity_id: serving_woman_id.clone(),
            description: "Mara now distrusts Brakka".into(),
            kind: SceneChangeKind::AttitudeChanged {
                hero_id: "hero-brakka".into(),
                attitude: StartingAttitude::Hostile,
            },
        },
    ];

    let prompt = model
        .as_it_is_now(super::super::pack::SceneKey::Opening, &changes)
        .expect("bounded scene changes form a prompt projection");
    let snapshot: serde_json::Value = serde_json::from_str(&prompt).expect("snapshot is JSON");
    let tankard = snapshot["model"]["things"]
        .as_array()
        .expect("snapshot has things")
        .iter()
        .find(|thing| thing["id"] == mug_id)
        .expect("the taken item remains represented by its state");
    assert_eq!(tankard["holder_id"], "hero-sigrun");
    assert!(!tankard["in_scene"].as_bool().unwrap_or(true));
    assert!(tankard["broken"].as_bool().unwrap_or(false));
    let door = snapshot["model"]["ways_out"]
        .as_array()
        .expect("snapshot has ways out")
        .iter()
        .find(|way| way["id"] == door_id)
        .expect("opened door remains in scene");
    assert!(door["open"].as_bool().unwrap_or(false));
    let serving_woman = snapshot["model"]["people"]
        .as_array()
        .expect("snapshot has people")
        .iter()
        .find(|person| person["id"] == serving_woman_id)
        .expect("moved person remains in scene");
    assert_eq!(serving_woman["position"], "at the doorway");
    assert_eq!(serving_woman["attitudes"]["hero-brakka"], "hostile");
    assert_eq!(
        snapshot["changes"].as_array().map(Vec::len),
        Some(changes.len())
    );
}

#[test]
fn giving_a_borrowed_observation_changes_holder_without_minting_ownership() {
    let mut model = taproom();
    add_test_hero(&mut model, "hero-brakka", "Brakka");
    add_test_hero(&mut model, "hero-sigrun", "Sigrun");
    let mug_id = model
        .things
        .iter()
        .find(|thing| thing.name == "dented pewter tankard")
        .expect("taproom model has its bar tankard")
        .id
        .clone();
    let changes = [
        SceneChange {
            entity_id: mug_id.clone(),
            description: "Brakka took the tankard for a closer look".into(),
            kind: SceneChangeKind::Taken {
                holder_id: "hero-brakka".into(),
            },
        },
        SceneChange {
            entity_id: mug_id.clone(),
            description: "Brakka handed the tankard to Sigrun for inspection".into(),
            kind: SceneChangeKind::Given {
                holder_id: "hero-sigrun".into(),
                owner_id: None,
            },
        },
    ];
    let prompt = model
        .as_it_is_now(super::super::pack::SceneKey::Opening, &changes)
        .expect("borrowed handoff remains a valid scene projection");
    let snapshot: serde_json::Value = serde_json::from_str(&prompt).expect("snapshot is JSON");
    let tankard = snapshot["model"]["things"]
        .as_array()
        .and_then(|things| things.iter().find(|thing| thing["id"] == mug_id))
        .expect("tankard remains in the scene snapshot");
    assert_eq!(tankard["holder_id"], "hero-sigrun");
    assert!(tankard["owner_id"].is_null());
}

#[test]
fn ordinary_given_handoff_preserves_canonical_owner() {
    let mut model = taproom();
    add_test_hero(&mut model, "hero-brakka", "Brakka");
    add_test_hero(&mut model, "hero-sigrun", "Sigrun");
    let thing = model
        .things
        .iter_mut()
        .find(|thing| thing.portable)
        .expect("taproom has portable thing");
    let thing_id = thing.id.clone();
    thing.owner_id = Some("patron".into());
    thing.holder_id = Some("hero-brakka".into());
    let snapshot = model
        .as_it_is_now(
            super::super::pack::SceneKey::Opening,
            &[SceneChange {
                entity_id: thing_id.clone(),
                description: "Brakka loans the item to Sigrun.".into(),
                kind: SceneChangeKind::Given {
                    holder_id: "hero-sigrun".into(),
                    owner_id: None,
                },
            }],
        )
        .expect("handoff remains a valid scene change");
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    let thing = snapshot["model"]["things"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thing| thing["id"] == thing_id)
        .unwrap();
    assert_eq!(thing["holder_id"], "hero-sigrun");
    assert_eq!(thing["owner_id"], "patron");
}

fn canonical_facts(model: &SceneModel) -> BTreeMap<String, String> {
    model
        .people
        .iter()
        .flat_map(|person| &person.knowledge)
        .filter(|knowledge| !knowledge.fact_id.starts_with("scene-person:"))
        .map(|knowledge| (knowledge.fact_id.clone(), knowledge.statement.clone()))
        .chain([
            ("secret.id".into(), "the fever-herb errand".into()),
            ("ant.id".into(), "giant rats".into()),
        ])
        .collect()
}
