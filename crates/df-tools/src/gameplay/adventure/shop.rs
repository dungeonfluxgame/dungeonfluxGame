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

pub(crate) fn generate(
    seed: u64,
    rules_mode: RulesMode,
    rating: ContentRating,
) -> Result<GeneratedAdventure, GenerateError> {
    // Use the production loader before attaching immutable authoring to the generated record.
    let mut loaded = pack::load_from_bytes(PACK, TUNING).map_err(GenerateError::Pack)?;
    let model: super::scene_model::SceneModel =
        serde_json::from_slice(MODEL).map_err(|_| GenerateError::Serialization)?;
    model.validate_structure()?;
    let dossiers: Vec<Dossier> =
        serde_json::from_slice(DOSSIERS).map_err(|_| GenerateError::Serialization)?;
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
    for bytes in [PACK, TUNING, MODEL, DOSSIERS] {
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

    fn interaction(current: &Checkpoint, operation: u8, target: &str, option: &str) -> Checkpoint {
        let choices = [("scene-target", target), ("scene-option", option)]
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
        let social = SocialLedger::open(record.dossiers, &record.seed, None).unwrap();
        let story = StoryFacts::from_pack(&loaded.pack);
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            assert!(opening.state().narrative.open_threads.is_empty());
            assert!(opening.state().encounters.is_empty());
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
            let checked = native::act(&opening, native::first(), 6, "look-around");
            assert_eq!(checked.state().draws.len(), 1);
            let checked = native::persisted(&checked);
            let taken = interaction(&checked, 7, "folded-label", "take");
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
            assert_eq!(card.thing_id, "purchased-clothes-travelers");
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
            assert!(public["offers"].as_array().unwrap().is_empty());
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
}
