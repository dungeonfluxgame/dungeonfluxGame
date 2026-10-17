//! Authored social slice. Only core SRD creation/mechanics are shared with other packs.
use super::{
    GenerateError, GeneratedAdventure, RulesMode, dossier::Dossier, setting::ChapterSettings,
};
use crate::gameplay::{beat_script::ScriptMode, pack, startup_configuration::ContentRating};
use sha2::{Digest, Sha256};

const PACK: &[u8] = include_bytes!("../../../../../content/merchant-shop/pack.json");
const TUNING: &[u8] = include_bytes!("../../../../../content/merchant-shop/tuning.json");
const MODEL: &[u8] = include_bytes!("../../../../../content/merchant-shop/scene-model.json");
const DOSSIERS: &[u8] = include_bytes!("../../../../../content/merchant-shop/dossiers.json");
const ARCHETYPES: &[u8] =
    include_bytes!("../../../../../content/adventure-tables/npc-archetypes.json");

/// Stable shop personas use authored role snapshots from the reusable library. Keep this
/// mapping on canonical dossier IDs, never cast order or display name.
fn expected_archetype(npc_id: &str) -> Option<&'static str> {
    match npc_id {
        "patron:merchant" => Some("cloth-merchant"),
        "bystander:shop-adventurer" => Some("adventurer"),
        "bystander:shop-soldier" => Some("soldier"),
        _ => None,
    }
}

pub(super) fn tuning_bytes(tuning: &pack::TuningFile) -> Result<Vec<u8>, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct CreationTuning<'a> {
        schema: &'a str,
        pack: &'a str,
        hero: &'a pack::HeroTuning,
    }
    serde_json::to_vec_pretty(&CreationTuning {
        schema: &tuning.schema,
        pack: &tuning.pack,
        hero: &tuning.hero,
    })
}

pub(crate) fn generate(
    seed: u64,
    rules_mode: RulesMode,
    rating: ContentRating,
) -> Result<GeneratedAdventure, GenerateError> {
    // Use the production loader before attaching immutable authoring to the generated record.
    let mut loaded = pack::load_from_bytes(PACK, TUNING).map_err(GenerateError::Pack)?;
    let mut model: super::scene_model::SceneModel =
        serde_json::from_slice(MODEL).map_err(|_| GenerateError::Serialization)?;
    model.migrate_inventory_version()?;
    model.validate_structure()?;
    let dossiers: Vec<Dossier> =
        serde_json::from_slice(DOSSIERS).map_err(|_| GenerateError::Serialization)?;
    let archetypes = super::dossier_archetypes::ArchetypeLibrary::from_bytes(ARCHETYPES)
        .map_err(|_| GenerateError::Serialization)?;
    for dossier in &dossiers {
        let expected = expected_archetype(&dossier.npc_id).ok_or(GenerateError::Serialization)?;
        let profile = dossier
            .archetype_profile
            .as_ref()
            .ok_or(GenerateError::Serialization)?;
        if profile.source_id != expected || archetypes.validate_snapshot(profile).is_err() {
            return Err(GenerateError::Serialization);
        }
    }
    loaded.pack.id = format!("adv-{seed:016x}");
    loaded.tuning.pack = loaded.pack.id.clone();
    for scene in &mut loaded.pack.scenes {
        if let Some(setting) = &mut scene.setting {
            setting.model = Some(model.clone());
        }
    }
    let setting = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .and_then(|scene| scene.setting.clone())
        .ok_or(GenerateError::Serialization)?;
    let settings = ChapterSettings {
        meeting: setting.clone(),
        confrontation: setting.clone(),
        destination: setting,
    };
    validate_cast(&loaded.pack, &dossiers).map_err(|_| GenerateError::Serialization)?;
    let mut revision = Sha256::new();
    for bytes in [PACK, TUNING, MODEL, DOSSIERS, ARCHETYPES] {
        revision.update(bytes);
    }
    Ok(GeneratedAdventure {
        seed,
        tables_revision: format!("{:x}", revision.finalize()),
        rules_mode,
        rating,
        facts: Default::default(),
        fact_sheet: Vec::new(),
        rolls: Vec::new(),
        pack: loaded.pack,
        tuning: loaded.tuning,
        settings,
        dossiers,
    })
}

/// Freeplay records have no invented story rolls. Every public speaker must instead bind to
/// its own durable persona, and every scene model person must have that same canonical ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CastBindingError {
    Mode,
    MissingCast,
    UnsupportedRole,
    MissingDossier,
    Identity,
    MissingPerson,
}
pub(super) fn validate_cast(
    content: &pack::PackFile,
    dossiers: &[Dossier],
) -> Result<(), CastBindingError> {
    if !content
        .beat_script
        .as_ref()
        .is_some_and(|script| script.mode == ScriptMode::Freeplay)
    {
        return Err(CastBindingError::Mode);
    }
    let scene = content
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .ok_or(CastBindingError::MissingCast)?;
    if scene.cast.is_empty()
        || dossiers.len() != scene.cast.len()
        || scene
            .cast
            .iter()
            .map(|actor| &actor.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != scene.cast.len()
    {
        return Err(CastBindingError::MissingCast);
    }
    for actor in &scene.cast {
        let label = match actor.role {
            pack::CastRole::Patron => "patron",
            pack::CastRole::Bystander => "bystander",
            pack::CastRole::Contact => "contact",
            pack::CastRole::Antagonist => return Err(CastBindingError::UnsupportedRole),
        };
        let id = format!("{label}:{}", actor.id);
        let Some(dossier) = dossiers.iter().find(|dossier| dossier.npc_id == id) else {
            return Err(CastBindingError::MissingDossier);
        };
        if dossier.name != actor.name || dossier.role.label() != label || dossier.leak().is_some() {
            return Err(CastBindingError::Identity);
        }
        let models = content
            .scenes
            .iter()
            .filter_map(|scene| scene.setting.as_ref()?.model.as_ref())
            .collect::<Vec<_>>();
        if models.len() != content.scenes.len() {
            return Err(CastBindingError::MissingPerson);
        }
        for model in models {
            if !model
                .people
                .iter()
                .any(|person| person.id == actor.id && person.name == actor.name)
            {
                return Err(CastBindingError::MissingPerson);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        beat_script_runtime as runtime,
        beat_script_tests::input,
        beat_script_variety_tests as native, journey, model,
        npc_reply::{NpcReplyContext, StoryFacts},
        social::SocialLedger,
        wire,
    };
    use df_model::checkpoint::Checkpoint;
    use df_persistence::local_demo_scope::LocalDemoRole;
    use df_protocol::common as rpc;
    use serde_json::json;

    fn interaction(current: &Checkpoint, operation: u8, target: &str, option: &str) -> Checkpoint {
        interaction_with_choices(current, operation, target, option, &[])
    }
    /// An open take of an item the merchant owns is an attempted theft: the d20 decides it.
    fn open_take(current: &Checkpoint, operation: u8, target: &str, face: u32) -> Checkpoint {
        let choices = [("scene-target", target), ("scene-option", "take")]
            .into_iter()
            .map(|(key, value)| (model::label(key).unwrap(), model::label(value).unwrap()))
            .collect();
        journey::stage_with_supplier(
            current,
            &input(
                current,
                native::first(),
                operation,
                "script-choice",
                choices,
            ),
            &mut |sides| {
                assert_eq!(sides, 20, "a theft uses one d20");
                Ok(face)
            },
        )
        .unwrap()
    }
    fn interaction_with_choices(
        current: &Checkpoint,
        operation: u8,
        target: &str,
        option: &str,
        extra: &[(&str, &str)],
    ) -> Checkpoint {
        let choices = [("scene-target", target), ("scene-option", option)]
            .into_iter()
            .chain(extra.iter().copied())
            .map(|(key, value)| (model::label(key).unwrap(), model::label(value).unwrap()))
            .collect();
        journey::stage_with_supplier(
            current,
            &input(
                current,
                native::first(),
                operation,
                "script-choice",
                choices,
            ),
            &mut |_| panic!("ordinary transfer does not roll"),
        )
        .unwrap()
    }

    #[test]
    fn authored_store_loads_reloads_and_keeps_native_interactions_in_freeplay() {
        let generated = generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, record) = super::super::record::reload(&documents).unwrap();
        assert!(record.rolls.is_empty());
        assert_eq!(record.dossiers.len(), 3);
        assert_eq!(record.dossiers, generated.dossiers);
        for (npc_id, expected) in [
            ("patron:merchant", "cloth-merchant"),
            ("bystander:shop-adventurer", "adventurer"),
            ("bystander:shop-soldier", "soldier"),
        ] {
            let profile = record
                .dossiers
                .iter()
                .find(|dossier| dossier.npc_id == npc_id)
                .and_then(|dossier| dossier.archetype_profile.as_ref())
                .expect("authored shop cast has a role profile");
            assert_eq!(profile.source_id, expected);
        }
        let notebook = crate::gameplay::notebook::NotebookLedger::open(
            &record.seed,
            record.dossiers.clone(),
            record.fact_sheet.clone(),
            None,
        )
        .unwrap();
        let social = SocialLedger::open(record.dossiers, &record.seed, None).unwrap();
        let story = StoryFacts::from_pack(&loaded.pack);
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            assert!(opening.state().narrative.open_threads.is_empty());
            assert!(opening.state().encounters.is_empty());
            let stage = runtime::scene(opening.state()).unwrap();
            notebook
                .observe_scene(native::first(), "Brynn", &stage, "", false)
                .unwrap();
            let notes = notebook.visible_to(native::first());
            assert!(
                !notes
                    .iter()
                    .any(|entry| entry.kind == crate::gameplay::notebook::Kind::Task)
            );
            assert_eq!(
                notes
                    .iter()
                    .filter(|entry| entry.kind == crate::gameplay::notebook::Kind::Person)
                    .count(),
                3
            );
            assert!(
                !opening
                    .state()
                    .entities
                    .iter()
                    .any(|entity| entity.definition.entry.as_str() == "lantern-wharf-courier")
            );
            let offers = runtime::view_offers(&opening, native::first()).unwrap();
            assert_eq!(offers.len(), 1);
            assert_eq!(offers[0].1, "Look around");
            let view =
                wire::journey_view(&opening, LocalDemoRole::Player, native::first()).unwrap();
            for (id, name) in [
                ("merchant", "Marek"),
                ("shop-adventurer", "Corin"),
                ("shop-soldier", "Jessa"),
            ] {
                let context = NpcReplyContext::for_target(
                    &view,
                    native::first(),
                    &format!("{name}, tell me about your past."),
                    &story,
                    &social,
                    "Opening",
                    Some(id),
                )
                .unwrap();
                assert_eq!(context.speaker, name);
                assert_eq!(context.actor_id, id);
                assert_eq!(context.facts["npc"]["name"], name);
                assert!(
                    !context
                        .facts
                        .to_string()
                        .contains("private watch passphrase")
                );
            }
            let road_clothes_question = "Corin, I am choosing road clothes. You look as though you travel often. What wears out first on a long journey?";
            let corin = NpcReplyContext::for_target(
                &view,
                native::first(),
                road_clothes_question,
                &story,
                &social,
                "Opening",
                Some("shop-adventurer"),
            )
            .unwrap();
            let role = &corin.facts["role_archetype"];
            assert_eq!(role["provenance"]["source_id"], "adventurer");
            assert_eq!(role["experience"]["id"], "seasoned");
            assert_eq!(role["competence"]["id"], "skilled");
            assert!(
                role["knowledge_domains"][0]
                    .as_str()
                    .unwrap()
                    .contains("Road wear")
            );
            assert_eq!(corin.facts["player"]["exact_words"], road_clothes_question);
            assert!(corin.facts["answer_from"].as_array().unwrap().is_empty());
            assert!(!corin.facts.to_string().contains("private watch passphrase"));
            let checked = native::act(&opening, native::first(), 6, "look-around");
            assert_eq!(checked.state().draws.len(), 1);
            let checked = native::persisted(&checked);
            // Taking the merchant's label openly is an attempted theft: a failed check leaves it on
            // the counter, and a successful one hands it over without changing its owner.
            let failed = open_take(&checked, 7, "folded-label", 1);
            assert!(
                journey::living_scene::last_line(failed.state())
                    .unwrap()
                    .unwrap()
                    .contains("fails to steal"),
                "an owned display item taken openly is a checked theft"
            );
            let taken = open_take(&checked, 17, "folded-label", 20);
            assert!(
                journey::living_scene::last_line(taken.state())
                    .unwrap()
                    .unwrap()
                    .contains("steals"),
                "a successful check takes the owned display item"
            );
            let scene = runtime::scene(taken.state()).unwrap();
            assert_eq!(
                journey::living_scene::interaction_for_line(
                    &scene,
                    "I put the folded paper label back on the counter."
                ),
                Some(("folded-label".into(), "return"))
            );
            let returned = interaction(&taken, 8, "folded-label", "return");
            let returned = native::persisted(&returned);
            let scene = runtime::scene(returned.state()).unwrap();
            let label = scene
                .setting
                .as_ref()
                .unwrap()
                .model
                .as_ref()
                .unwrap()
                .things
                .iter()
                .find(|thing| thing.id == "folded-label")
                .unwrap();
            assert!(label.holder_id.is_none());
            assert_eq!(label.owner_id.as_deref(), Some("merchant"));
            assert_eq!(label.position, "on the wooden counter");
            let who = journey::player_entity(native::first(), &returned).unwrap();
            let before = journey::living_scene::balance_cp(returned.state(), who).unwrap();
            let bought = interaction(&returned, 9, "clothes-travelers", "buy");
            assert_eq!(
                journey::living_scene::balance_cp(bought.state(), who).unwrap(),
                before - 200
            );
            let bought = native::persisted(&bought);
            let scene = runtime::scene(bought.state()).unwrap();
            let card = journey::living_scene::item_for(bought.state(), &scene, "Brynn")
                .unwrap()
                .unwrap();
            assert_eq!(card.thing_id, "item-clothes-travelers");
            let projected =
                wire::journey_view(&bought, LocalDemoRole::Player, native::first()).unwrap();
            let Some(df_protocol::common::view_message::Audience::Player(projected)) =
                projected.audience
            else {
                panic!("player projection");
            };
            assert_eq!(
                projected.scene.unwrap().item.unwrap().thing_id,
                card.thing_id
            );
            assert!(
                journey::living_scene::item_for(bought.state(), &scene, "Vale")
                    .unwrap()
                    .is_none()
            );
            let now = journey::living_scene::snapshot(bought.state(), &scene)
                .unwrap()
                .unwrap();
            let public: serde_json::Value =
                serde_json::from_str(&now.model.public_contents().unwrap()).unwrap();
            // The bought stock leaves the public offers; the shop's other stock stays on sale.
            assert!(
                public["offers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|offer| offer["item_id"] != "item-clothes-travelers")
            );
            let owned = now
                .model
                .things
                .iter()
                .find(|thing| thing.id == card.thing_id)
                .unwrap();
            assert_eq!(owned.owner_id.as_deref(), Some("Brynn"));
            assert_eq!(owned.holder_id.as_deref(), Some("Brynn"));
            let examined = interaction(&bought, 10, &card.thing_id, "examine");
            let label_examined = interaction(&examined, 11, "folded-label", "examine");
            let scene = runtime::scene(label_examined.state()).unwrap();
            assert_eq!(
                journey::living_scene::item_for(label_examined.state(), &scene, "Brynn")
                    .unwrap()
                    .unwrap()
                    .thing_id,
                "folded-label"
            );
            let repeated = interaction(&label_examined, 12, "clothes-travelers", "buy");
            assert_eq!(
                journey::living_scene::balance_cp(repeated.state(), who).unwrap(),
                before - 200
            );
            assert!(
                journey::living_scene::last_line(repeated.state())
                    .unwrap()
                    .unwrap()
                    .contains("already sold")
            );
            let repeated = native::persisted(&repeated);
            assert_eq!(
                repeated.state().narrative.active_beats,
                opening.state().narrative.active_beats
            );
            assert!(!runtime::ended(repeated.state()).unwrap());
            assert!(repeated.state().narrative.open_threads.is_empty());
            let mut checked = repeated;
            for (operation, target) in [
                (13, "thing-item-clothes-travelers"),
                (14, "thing-folded-label"),
            ] {
                let choices = [
                    (runtime::CHOICE_GROUP, "look-around"),
                    ("attempt-thing", target),
                    ("attempt-skill", "investigation"),
                ]
                .into_iter()
                .map(|(key, value)| (model::label(key).unwrap(), model::label(value).unwrap()))
                .collect();
                checked = journey::stage_with_supplier(
                    &checked,
                    &input(
                        &checked,
                        native::first(),
                        operation,
                        "script-choice",
                        choices,
                    ),
                    &mut |_| Ok(12),
                )
                .unwrap();
                checked = native::persisted(&checked);
                let scene = runtime::scene(checked.state()).unwrap();
                let card = journey::living_scene::item_for(checked.state(), &scene, "Brynn")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    format!("thing-{}", card.thing_id),
                    target,
                    "latest rolled inspection must supersede historical purchase"
                );
            }
            // Three inspection checks plus the d20 of the open take of the merchant's label.
            assert_eq!(checked.state().draws.len(), 4);
        });
    }

    #[test]
    fn freeplay_mode_refuses_story_offers_and_does_not_disable_ordinary_story_validation() {
        let mut generated = generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
        let script = generated.pack.beat_script.as_mut().unwrap();
        script.beats[1].offers[0].effect = crate::gameplay::beat_script::BeatEffect::DefendCourier;
        assert!(
            pack::load_from_bytes(
                &serde_json::to_vec(&generated.pack).unwrap(),
                &serde_json::to_vec(&generated.tuning).unwrap()
            )
            .is_err()
        );
        let mut generated = generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
        generated.pack.beat_script.as_mut().unwrap().mode = ScriptMode::Story;
        assert!(
            pack::load_from_bytes(
                &serde_json::to_vec(&generated.pack).unwrap(),
                &serde_json::to_vec(&generated.tuning).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn quoted_coin_trade_and_owned_gift_replay_with_exact_payment_and_recipient() {
        let generated = generate(43, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) =
            super::super::record::reload(&generated.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let who = journey::player_entity(native::first(), &opening).unwrap();
            let initial = journey::living_scene::balance_cp(opening.state(), who).unwrap();
            let low_quote = interaction_with_choices(
                &opening,
                6,
                "clothes-travelers",
                "trade",
                &[("scene-payment", "100")],
            );
            assert_eq!(
                journey::living_scene::balance_cp(low_quote.state(), who).unwrap(),
                initial
            );
            let traded = interaction_with_choices(
                &low_quote,
                7,
                "clothes-travelers",
                "trade",
                &[("scene-payment", "200")],
            );
            assert_eq!(
                journey::living_scene::balance_cp(traded.state(), who).unwrap(),
                initial - 200
            );
            let scene = runtime::scene(traded.state()).unwrap();
            assert_eq!(
                journey::living_scene::recipient_for_line(
                    &scene,
                    "I give the travelers clothes to Corin."
                ),
                Some("shop-adventurer".into())
            );
            let given = interaction_with_choices(
                &traded,
                8,
                "item-clothes-travelers",
                "give",
                &[("scene-recipient", "shop-adventurer")],
            );
            let given = native::persisted(&given);
            let scene = runtime::scene(given.state()).unwrap();
            let item = scene
                .setting
                .as_ref()
                .unwrap()
                .model
                .as_ref()
                .unwrap()
                .things
                .iter()
                .find(|thing| thing.id == "item-clothes-travelers")
                .unwrap();
            assert_eq!(item.holder_id.as_deref(), Some("shop-adventurer"));
            assert_eq!(item.owner_id.as_deref(), Some("shop-adventurer"));
            let borrowed = open_take(&given, 9, "folded-label", 1);
            assert!(
                journey::living_scene::last_line(borrowed.state())
                    .unwrap()
                    .unwrap()
                    .contains("fails to steal")
            );
            let refused = interaction_with_choices(
                &borrowed,
                10,
                "folded-label",
                "give",
                &[("scene-recipient", "shop-adventurer")],
            );
            assert!(
                journey::living_scene::last_line(refused.state())
                    .unwrap()
                    .unwrap()
                    .contains("belongs to someone else")
            );
            let scene = runtime::scene(refused.state()).unwrap();
            let label = scene
                .setting
                .as_ref()
                .unwrap()
                .model
                .as_ref()
                .unwrap()
                .things
                .iter()
                .find(|thing| thing.id == "folded-label")
                .unwrap();
            assert_eq!(label.owner_id.as_deref(), Some("merchant"));
            assert_eq!(label.holder_id.as_deref(), None);
            assert_eq!(
                journey::living_scene::payment_amount_cp("I trade 2 GP for the travelers clothes."),
                Some(200)
            );
            assert_eq!(
                journey::living_scene::payment_amount_cp(
                    "I trade my cloak for the travelers clothes."
                ),
                None
            );
        });
    }

    #[test]
    fn merchant_can_lend_the_same_catalog_item_for_inspection_and_replay_the_custody() {
        let generated = generate(45, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) =
            super::super::record::reload(&generated.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let scene = runtime::scene(opening.state()).unwrap();
            let model = scene
                .setting
                .as_ref()
                .and_then(|setting| setting.model.as_ref())
                .expect("shop opening has its canonical location model");
            let offer = model
                .offer("item-clothes-travelers")
                .expect("the stable clothes item resolves in stock");
            let seller_id = offer.seller_id.clone();
            let source_revision = opening.basis().revision;
            let proposal = journey::living_scene::NpcInventoryProposal {
                operation: df_types::OperationId::from_bytes(&[45; 16]).unwrap(),
                expected_basis: opening.basis(),
                actor_id: seller_id.clone(),
                target_id: offer.canonical_item_id(),
                action: journey::living_scene::NpcInventoryAction::Give {
                    recipient_id: "shop-adventurer".into(),
                    transfer_ownership: false,
                    owner_consent_id: None,
                },
                source: Some(journey::living_scene::NpcInventorySource {
                    source_line_id: "merchant-inspection-loan-1".into(),
                    source_revision,
                    source_line_sequence: 1,
                    source_actor_id: seller_id.clone(),
                    source_text: "Marek lends the traveler's clothes for inspection.".into(),
                }),
            };

            let mut unbound = proposal.clone();
            unbound.source = None;
            assert!(journey::living_scene::commit_npc_inventory_action(&opening, unbound).is_err());
            let mut stale = proposal.clone();
            stale.source.as_mut().unwrap().source_revision =
                source_revision.next_sequence().unwrap();
            assert!(journey::living_scene::commit_npc_inventory_action(&opening, stale).is_err());

            let borrowed = journey::living_scene::commit_npc_inventory_action(&opening, proposal)
                .expect("the seller can hand stock to a present character for inspection");
            let semantic = borrowed
                .state()
                .decisions
                .last()
                .and_then(|decision| decision.semantic_output.as_deref())
                .expect("the accepted event stores its source-bound transaction");
            assert!(semantic.contains("merchant-inspection-loan-1"));
            assert!(semantic.contains("Marek lends the traveler's clothes for inspection."));

            let borrowed = native::persisted(&borrowed);
            let scene = runtime::scene(borrowed.state()).unwrap();
            let model = scene
                .setting
                .as_ref()
                .and_then(|setting| setting.model.as_ref())
                .expect("the persisted shop scene replays its item state");
            let item = model
                .things
                .iter()
                .find(|thing| thing.id == "item-clothes-travelers")
                .expect("the catalog identity survives replay");
            assert_eq!(item.owner_id.as_deref(), Some(seller_id.as_str()));
            assert_eq!(item.holder_id.as_deref(), Some("shop-adventurer"));
            assert_eq!(item.provenance.source_id, "clothes-travelers");
            assert_eq!(
                item.provenance.seller_id.as_deref(),
                Some(seller_id.as_str())
            );
            assert!(!model.offer_available("item-clothes-travelers"));
            assert_eq!(
                model
                    .inventory()
                    .iter()
                    .filter(|entry| entry.item_id == "item-clothes-travelers")
                    .count(),
                1
            );
        });
    }

    #[test]
    fn merchant_quotes_buy_and_buyback_exactly_once_with_conserved_copper() {
        let generated = generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) =
            super::super::record::reload(&generated.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let actor = journey::player_entity(native::first(), &opening).unwrap();
            let initial_balance =
                journey::living_scene::balance_cp(opening.state(), actor).unwrap();
            let scene = runtime::scene(opening.state()).unwrap();
            let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
            let offer = model.offer("item-clothes-travelers").unwrap();
            let merchant_id = offer.seller_id.clone();
            let buy_source = journey::living_scene::NpcInventorySource {
                source_line_id: "merchant-quote-buy-1".into(),
                source_revision: opening.basis().revision,
                source_line_sequence: 1,
                source_actor_id: merchant_id.clone(),
                source_text: "I can do those clothes for 150 CP.".into(),
            };
            let buy_quote_checkpoint = journey::living_scene::commit_npc_trade_quote(
                &opening,
                journey::living_scene::NpcTradeQuoteProposal {
                    operation: df_types::OperationId::from_bytes(&[46; 16]).unwrap(),
                    expected_basis: opening.basis(),
                    actor_id: merchant_id.clone(),
                    customer_id: "Brynn".into(),
                    direction: journey::living_scene::SceneTradeDirection::HeroBuys,
                    item_id: offer.canonical_item_id(),
                    offer_id: Some(offer.id.clone()),
                    price_cp: 150,
                    source: buy_source,
                },
            )
            .expect("the seller can issue a bounded exact quote");
            let superseding_checkpoint = journey::living_scene::commit_npc_trade_quote(
                &buy_quote_checkpoint,
                journey::living_scene::NpcTradeQuoteProposal {
                    operation: df_types::OperationId::from_bytes(&[47; 16]).unwrap(),
                    expected_basis: buy_quote_checkpoint.basis(),
                    actor_id: merchant_id.clone(),
                    customer_id: "Brynn".into(),
                    direction: journey::living_scene::SceneTradeDirection::HeroBuys,
                    item_id: offer.canonical_item_id(),
                    offer_id: Some(offer.id.clone()),
                    price_cp: 140,
                    source: journey::living_scene::NpcInventorySource {
                        source_line_id: "merchant-quote-buy-revised".into(),
                        source_revision: buy_quote_checkpoint.basis().revision,
                        source_line_sequence: 2,
                        source_actor_id: merchant_id.clone(),
                        source_text: "I can lower that to 140 CP.".into(),
                    },
                },
            )
            .expect("a newer exact quote replaces the earlier pending offer");
            let history =
                journey::living_scene::merchant_quote_history(&superseding_checkpoint).unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(
                history[0].status,
                journey::living_scene::MerchantQuoteStatus::Stale
            );
            assert_eq!(
                history[1].status,
                journey::living_scene::MerchantQuoteStatus::Pending
            );
            let stale_attempt = interaction_with_choices(
                &superseding_checkpoint,
                50,
                "item-clothes-travelers",
                "trade",
                &[
                    ("scene-quote-id", &history[0].quote.quote_id),
                    ("scene-payment", "150"),
                ],
            );
            let history = journey::living_scene::merchant_quote_history(&stale_attempt).unwrap();
            assert_eq!(
                history[0].status,
                journey::living_scene::MerchantQuoteStatus::Stale
            );
            assert_eq!(history[0].attempt_count, 1);
            assert_eq!(history[0].last_rejection.as_deref(), Some("stale_quote"));
            let buy_quote = journey::living_scene::pending_merchant_quote(&stale_attempt, "Brynn")
                .unwrap()
                .expect("the quote is pending for its player buyer");
            assert_eq!(
                buy_quote.direction,
                journey::living_scene::SceneTradeDirection::HeroBuys
            );
            assert_eq!(buy_quote.price_cp, 140);
            assert_eq!(
                journey::living_scene::merchant_quote_history(&stale_attempt).unwrap()[1].status,
                journey::living_scene::MerchantQuoteStatus::Pending
            );

            let bought = interaction_with_choices(
                &stale_attempt,
                51,
                &buy_quote.item_id,
                "trade",
                &[
                    ("scene-quote-id", &buy_quote.quote_id),
                    ("scene-payment", "140"),
                ],
            );
            assert_eq!(
                journey::living_scene::balance_cp(bought.state(), actor).unwrap(),
                initial_balance - 140
            );
            let item_id = buy_quote.item_id.clone();
            let purchase = journey::living_scene::latest_merchant_item_transaction(
                &bought,
                &runtime::scene(bought.state())
                    .unwrap()
                    .setting
                    .as_ref()
                    .unwrap()
                    .place_id,
                &item_id,
                &merchant_id,
                "Brynn",
            )
            .unwrap()
            .expect("an accepted merchant sale is available from the durable journal");
            assert_eq!(
                purchase.direction,
                journey::living_scene::MerchantItemTransactionDirection::MerchantSoldToHero
            );
            assert_eq!(purchase.price_cp, 140);
            assert_eq!(purchase.item_id, item_id);
            assert_eq!(purchase.merchant_id, merchant_id);
            assert_eq!(purchase.hero_id, "Brynn");
            assert_eq!(
                purchase.revision,
                bought.state().decisions.last().unwrap().revision
            );
            let bought_scene = runtime::scene(bought.state()).unwrap();
            let bought_model = bought_scene
                .setting
                .as_ref()
                .unwrap()
                .model
                .as_ref()
                .unwrap();
            assert_eq!(
                bought_model
                    .people
                    .iter()
                    .find(|person| person.id == merchant_id)
                    .unwrap()
                    .coin_cp,
                5_140
            );
            let bought_view =
                wire::journey_view(&bought, LocalDemoRole::Player, native::first()).unwrap();
            let Some(rpc::view_message::Audience::Player(bought_player)) = bought_view.audience
            else {
                panic!("player projection");
            };
            let bought_sheet = bought_player
                .journey
                .as_ref()
                .and_then(|journey| journey.own_character.as_ref())
                .expect("the buyer phone shows the current character sheet");
            assert!(
                bought_sheet
                    .owned_items
                    .iter()
                    .any(|item| item.thing_id == item_id),
                "a successful purchase appears in permanent inventory"
            );
            assert!(
                bought_player
                    .scene
                    .as_ref()
                    .unwrap()
                    .hand_items
                    .iter()
                    .all(|item| item.thing_id != item_id),
                "an owned purchase is not rendered as an observation-only hand item"
            );
            let bought_coin = bought_sheet
                .facts
                .iter()
                .find(|fact| fact.label == "Coin")
                .expect("the phone sheet shows current coin");
            let bought_balance = journey::living_scene::balance_cp(bought.state(), actor).unwrap();
            assert_eq!(
                bought_coin.value,
                format!("{} GP, {} CP", bought_balance / 100, bought_balance % 100)
            );
            let sell_terms = journey::living_scene::merchant_trade_terms(&bought, "Brynn")
                .unwrap()
                .into_iter()
                .find(|terms| {
                    terms.direction == journey::living_scene::SceneTradeDirection::HeroSells
                        && terms.item_id == item_id
                        && terms.buyer_id == merchant_id
                })
                .expect("the hero may offer the exact owned and held instance");
            assert_eq!(sell_terms.maximum_price_cp, 100);
            let buyback_request =
                "Marek, will you buy these travelers clothes back from me? What would you pay?";
            // Jev is not called in this test; the phrase fallback reads the same buyback line.
            let buyback_line = journey::merchant_bargaining::HeroLine {
                intent: journey::merchant_bargaining::rules_hero_intent(
                    buyback_request,
                    None,
                    crate::gameplay::approach_model::JevUnavailable::ProviderUnavailable,
                ),
                words: buyback_request,
            };
            let mut trade_terms =
                journey::living_scene::merchant_trade_terms(&bought, "Brynn").unwrap();
            assert_eq!(
                journey::merchant_bargaining::referenced_item_id(
                    &trade_terms,
                    "Brynn",
                    &merchant_id,
                    buyback_line,
                )
                .as_deref(),
                Some(item_id.as_str()),
                "the real post-purchase terms resolve the spoken buyback to the held instance"
            );
            let place_id = bought_scene.setting.as_ref().unwrap().place_id.as_str();
            let memory = journey::merchant_bargaining::apply_committed_buyback_context(
                &bought,
                &mut trade_terms,
                place_id,
                &item_id,
                "Brynn",
                &merchant_id,
            )
            .unwrap()
            .expect("the exact accepted purchase is available to the merchant reply context");
            assert_eq!(memory.last_purchase_price_cp, 140);
            let mut trade_context = journey::merchant_bargaining::reply_context(
                &trade_terms,
                "Brynn",
                &merchant_id,
                buyback_line,
                Some(&item_id),
                &json!({"npc": {"name": "Marek", "role": "merchant"}}),
            )
            .expect("the production reply context recognizes this as merchant buying");
            journey::merchant_bargaining::attach_buyback_memory(&mut trade_context, &memory);
            assert_eq!(
                trade_context["available_trades"][0]["direction"],
                "merchant_buys"
            );
            assert_eq!(trade_context["recent_committed_purchase"]["price_cp"], 140);
            assert!(
                trade_context["negotiation_rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|rule| rule
                        .as_str()
                        .is_some_and(|rule| rule.contains("trade, not a quest reward")))
            );
            let sell_quote_checkpoint = journey::living_scene::commit_npc_trade_quote(
                &bought,
                journey::living_scene::NpcTradeQuoteProposal {
                    operation: df_types::OperationId::from_bytes(&[48; 16]).unwrap(),
                    expected_basis: bought.basis(),
                    actor_id: merchant_id.clone(),
                    customer_id: "Brynn".into(),
                    direction: journey::living_scene::SceneTradeDirection::HeroSells,
                    item_id: item_id.clone(),
                    offer_id: None,
                    price_cp: 90,
                    source: journey::living_scene::NpcInventorySource {
                        source_line_id: "merchant-quote-buyback-1".into(),
                        source_revision: bought.basis().revision,
                        source_line_sequence: 2,
                        source_actor_id: merchant_id.clone(),
                        source_text: "I can buy those back for nine silver pieces.".into(),
                    },
                },
            )
            .expect("the merchant purse bounds an exact buyback quote");
            let sell_quote =
                journey::living_scene::pending_merchant_quote(&sell_quote_checkpoint, "Brynn")
                    .unwrap()
                    .expect("HeroSells quotes are pending for the authenticated seller");
            assert_eq!(
                sell_quote.direction,
                journey::living_scene::SceneTradeDirection::HeroSells
            );
            let sold = interaction_with_choices(
                &sell_quote_checkpoint,
                49,
                &item_id,
                "trade",
                &[
                    ("scene-quote-id", &sell_quote.quote_id),
                    ("scene-payment", "90"),
                ],
            );
            let buyback = journey::living_scene::latest_merchant_item_transaction(
                &sold,
                &sell_quote.place_id,
                &item_id,
                &merchant_id,
                "Brynn",
            )
            .unwrap()
            .expect("the committed buyback is replay-derived for the exact item");
            assert_eq!(
                buyback.direction,
                journey::living_scene::MerchantItemTransactionDirection::HeroSoldToMerchant
            );
            assert_eq!(buyback.price_cp, 90);
            assert_ne!(buyback.revision, purchase.revision);
            assert!(
                journey::living_scene::coin_was_transferred(
                    sold.state(),
                    df_types::OperationId::from_bytes(&[49; 16]).unwrap()
                )
                .unwrap()
            );
            assert_eq!(
                journey::living_scene::balance_cp(sold.state(), actor).unwrap(),
                initial_balance - 50
            );
            let sold_scene = runtime::scene(sold.state()).unwrap();
            let sold_model = sold_scene.setting.as_ref().unwrap().model.as_ref().unwrap();
            let item = sold_model
                .things
                .iter()
                .find(|thing| thing.id == item_id)
                .unwrap();
            assert_eq!(item.owner_id.as_deref(), Some(merchant_id.as_str()));
            assert_eq!(item.holder_id.as_deref(), Some(merchant_id.as_str()));
            let sold_view =
                wire::journey_view(&sold, LocalDemoRole::Player, native::first()).unwrap();
            let Some(rpc::view_message::Audience::Player(sold_player)) = sold_view.audience else {
                panic!("player projection");
            };
            let sold_sheet = sold_player
                .journey
                .as_ref()
                .and_then(|journey| journey.own_character.as_ref())
                .expect("the seller phone shows the current character sheet");
            assert!(
                sold_sheet
                    .owned_items
                    .iter()
                    .all(|owned| owned.thing_id != item_id),
                "a sold item is removed from permanent inventory"
            );
            assert_ne!(
                sold_player
                    .scene
                    .as_ref()
                    .and_then(|scene| scene.item.as_ref())
                    .map(|card| card.thing_id.as_str()),
                Some(item_id.as_str()),
                "a sold item is not left as the phone's current item card"
            );
            let sold_coin = sold_sheet
                .facts
                .iter()
                .find(|fact| fact.label == "Coin")
                .expect("the phone sheet shows current coin after the sale");
            let sold_balance = journey::living_scene::balance_cp(sold.state(), actor).unwrap();
            assert_eq!(
                sold_coin.value,
                format!("{} GP, {} CP", sold_balance / 100, sold_balance % 100)
            );
            let history = journey::living_scene::merchant_quote_history(&sold).unwrap();
            assert_eq!(history.len(), 3);
            assert_eq!(
                history[0].status,
                journey::living_scene::MerchantQuoteStatus::Stale
            );
            assert_eq!(history[0].attempt_count, 1);
            assert!(history[1..].iter().all(|entry| {
                entry.status == journey::living_scene::MerchantQuoteStatus::Accepted
                    && entry.attempt_count == 1
            }));
        });
    }

    #[test]
    fn merchant_quote_decline_and_withdrawal_are_durable_and_block_acceptance() {
        let generated = generate(47, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) =
            super::super::record::reload(&generated.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let model = runtime::scene(opening.state())
                .unwrap()
                .setting
                .unwrap()
                .model
                .unwrap();
            let offer = model.offer("item-clothes-travelers").unwrap().clone();
            let merchant_id = offer.seller_id.clone();
            let quote_checkpoint = journey::living_scene::commit_npc_trade_quote(
                &opening,
                journey::living_scene::NpcTradeQuoteProposal {
                    operation: df_types::OperationId::from_bytes(&[47; 16]).unwrap(),
                    expected_basis: opening.basis(),
                    actor_id: merchant_id.clone(),
                    customer_id: "Brynn".into(),
                    direction: journey::living_scene::SceneTradeDirection::HeroBuys,
                    item_id: offer.canonical_item_id(),
                    offer_id: Some(offer.id.clone()),
                    price_cp: 150,
                    source: journey::living_scene::NpcInventorySource {
                        source_line_id: "merchant-quote-close-1".into(),
                        source_revision: opening.basis().revision,
                        // The quote's original TableLog can be older than a restarted ring;
                        // its new public decline line may therefore have a smaller sequence.
                        source_line_sequence: 100,
                        source_actor_id: merchant_id.clone(),
                        source_text: "I can do those clothes for 150 CP.".into(),
                    },
                },
            )
            .unwrap();
            let quote = journey::living_scene::pending_merchant_quote(&quote_checkpoint, "Brynn")
                .unwrap()
                .unwrap();
            let declined = journey::living_scene::close_merchant_quote(
                &quote_checkpoint,
                journey::living_scene::MerchantQuoteClosureProposal {
                    operation: df_types::OperationId::from_bytes(&[48; 16]).unwrap(),
                    expected_basis: quote_checkpoint.basis(),
                    quote_id: quote.quote_id.clone(),
                    actor_id: "Brynn".into(),
                    customer_id: "Brynn".into(),
                    reason: journey::living_scene::MerchantQuoteClosureReason::HeroDeclined,
                    source: journey::living_scene::MerchantQuoteClosureSource::PlayerDecision {
                        source_line_id: "player-declined-quote-1".into(),
                        source_revision: quote_checkpoint.basis().revision,
                        source_line_sequence: 2,
                        source_actor_id: "Brynn".into(),
                        source_text: "I decline the offer.".into(),
                    },
                },
            )
            .expect("an authenticated customer's explicit decline closes the quote");
            let declined = native::persisted(&declined);
            let history = journey::living_scene::merchant_quote_history(&declined).unwrap();
            assert_eq!(
                history[0].status,
                journey::living_scene::MerchantQuoteStatus::Declined
            );
            assert_eq!(
                history[0].close_reason,
                Some(journey::living_scene::MerchantQuoteClosureReason::HeroDeclined)
            );
            assert!(
                journey::living_scene::pending_merchant_quote(&declined, "Brynn")
                    .unwrap()
                    .is_none()
            );
            let balance = journey::living_scene::balance_cp(
                declined.state(),
                journey::player_entity(native::first(), &declined).unwrap(),
            )
            .unwrap();
            let attempted = interaction_with_choices(
                &declined,
                49,
                &quote.item_id,
                "trade",
                &[
                    ("scene-quote-id", &quote.quote_id),
                    ("scene-payment", "150"),
                ],
            );
            assert_eq!(
                journey::living_scene::balance_cp(
                    attempted.state(),
                    journey::player_entity(native::first(), &attempted).unwrap(),
                )
                .unwrap(),
                balance,
                "a declined quote cannot later debit the buyer"
            );
            assert!(
                runtime::scene(attempted.state())
                    .unwrap()
                    .setting
                    .unwrap()
                    .model
                    .unwrap()
                    .offer_available(&quote.item_id)
            );
            assert_eq!(
                journey::living_scene::merchant_quote_history(&attempted).unwrap()[0].status,
                journey::living_scene::MerchantQuoteStatus::Declined
            );

            let revised = journey::living_scene::commit_npc_trade_quote(
                &attempted,
                journey::living_scene::NpcTradeQuoteProposal {
                    operation: df_types::OperationId::from_bytes(&[50; 16]).unwrap(),
                    expected_basis: attempted.basis(),
                    actor_id: merchant_id.clone(),
                    customer_id: "Brynn".into(),
                    direction: journey::living_scene::SceneTradeDirection::HeroBuys,
                    item_id: quote.item_id.clone(),
                    offer_id: Some(offer.id),
                    price_cp: 140,
                    source: journey::living_scene::NpcInventorySource {
                        source_line_id: "merchant-quote-close-2".into(),
                        source_revision: attempted.basis().revision,
                        source_line_sequence: 3,
                        source_actor_id: merchant_id.clone(),
                        source_text: "I can do those clothes for 140 CP.".into(),
                    },
                },
            )
            .unwrap();
            let revised_quote = journey::living_scene::pending_merchant_quote(&revised, "Brynn")
                .unwrap()
                .unwrap();
            let withdrawn = journey::living_scene::close_merchant_quote(
                &revised,
                journey::living_scene::MerchantQuoteClosureProposal {
                    operation: df_types::OperationId::from_bytes(&[51; 16]).unwrap(),
                    expected_basis: revised.basis(),
                    quote_id: revised_quote.quote_id.clone(),
                    actor_id: merchant_id,
                    customer_id: "Brynn".into(),
                    reason: journey::living_scene::MerchantQuoteClosureReason::MerchantWithdrew,
                    source: journey::living_scene::MerchantQuoteClosureSource::MerchantSpeech {
                        source_line_id: "merchant-withdrew-quote-1".into(),
                        source_revision: revised.basis().revision,
                        source_line_sequence: 4,
                        source_actor_id: offer.seller_id,
                        source_text: "I withdraw that offer.".into(),
                    },
                },
            )
            .expect("the quoted merchant can withdraw its own offer");
            let withdrawn = native::persisted(&withdrawn);
            let history = journey::living_scene::merchant_quote_history(&withdrawn).unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(
                history[0].status,
                journey::living_scene::MerchantQuoteStatus::Declined
            );
            assert_eq!(
                history[1].status,
                journey::living_scene::MerchantQuoteStatus::Withdrawn
            );
            assert_eq!(
                history[1].close_reason,
                Some(journey::living_scene::MerchantQuoteClosureReason::MerchantWithdrew)
            );
            assert!(
                journey::living_scene::pending_merchant_quote(&withdrawn, "Brynn")
                    .unwrap()
                    .is_none()
            );
        });
    }

    #[test]
    fn merchant_startup_ignores_random_tables_and_resume_keeps_the_pinned_freeplay_record() {
        use crate::gameplay::startup_configuration::{AdventureMode, AdventureSettings};
        use df_persistence::local_demo_scope::AdventureStartup;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp")
            .join(format!(
                "standalone-shop-startup-{}-{}",
                std::process::id(),
                u128::from_le_bytes(model::random::<16>().unwrap())
            ));
        let mut settings = AdventureSettings {
            mode: AdventureMode::MerchantShop,
            seed: Some(44),
            tables: root.join("intentionally-missing-tables"),
            rules_mode: RulesMode::Standard,
            chapter_type: None,
        };
        let fresh = super::super::startup::select(
            &settings,
            &root,
            "df_gameplay_demo_shop_startup",
            AdventureStartup::Fresh,
        )
        .unwrap();
        assert!(fresh.fresh);
        assert!(fresh.record.rolls.is_empty());
        settings.mode = AdventureMode::Random;
        settings.seed = Some(999);
        let restored = super::super::startup::select(
            &settings,
            &root,
            "df_gameplay_demo_shop_startup",
            AdventureStartup::Stored(fresh.documents.clone()),
        )
        .unwrap();
        assert!(!restored.fresh);
        assert_eq!(fresh.loaded.pack, restored.loaded.pack);
        assert!(fresh.documents == restored.documents);
    }

    #[test]
    fn serialized_freeplay_documents_contain_only_the_authored_store_and_creation_rules() {
        let generated = generate(45, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        for bytes in [
            PACK,
            TUNING,
            MODEL,
            DOSSIERS,
            &documents.pack,
            &documents.tuning,
            &documents.record,
        ] {
            let text = std::str::from_utf8(bytes).unwrap().to_lowercase();
            for forbidden in [
                "courier",
                "harbor",
                "eldric",
                "sealed-packet",
                "brass key",
                "unused creature",
                "antagonist",
                "\"bandit\"",
                "\"combat\"",
                "\"victory\"",
                "\"defeat\"",
                "dialogue-seal",
                "dialogue-escort",
            ] {
                assert!(
                    !text.contains(forbidden),
                    "standalone authored documents must not contain {forbidden}"
                );
            }
        }
        let content: serde_json::Value = serde_json::from_slice(&documents.pack).unwrap();
        assert_eq!(content["scenes"].as_array().unwrap().len(), 2);
        assert_eq!(content["offer_labels"].as_object().unwrap().len(), 2);
        let tuning: serde_json::Value = serde_json::from_slice(&documents.tuning).unwrap();
        assert_eq!(tuning.as_object().unwrap().len(), 3);
        let record: serde_json::Value = serde_json::from_slice(&documents.record).unwrap();
        assert!(record.get("settings").is_none());
        assert!(record.get("freeplay_setting").is_some());
        let (loaded, _) = super::super::record::reload(&documents).unwrap();
        assert!(loaded.pack.is_freeplay());
        assert!(loaded.pack.bandit.name.is_empty());
        assert_eq!(loaded.pack.scenes.len(), 2);
        let mut invalid = generated.pack.clone();
        invalid.courier_clue = "An injected delivery.".into();
        assert!(
            pack::load_from_bytes(&serde_json::to_vec(&invalid).unwrap(), &documents.tuning)
                .is_err()
        );
    }
}
