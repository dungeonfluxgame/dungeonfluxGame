//! Weapon readiness is an owned scene condition, distinct from custody and ownership.
use super::*;
use crate::gameplay::adventure::scene_model::{SceneItemProvenance, SceneThing};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum WeaponReadiness {
    #[default]
    Sheathed,
    Drawn,
}

impl WeaponReadiness {
    fn label(self) -> &'static str {
        match self {
            Self::Sheathed => "sheathed",
            Self::Drawn => "drawn",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct WeaponCondition {
    pub(in crate::gameplay) holder_id: String,
    pub(in crate::gameplay) item_id: String,
    pub(in crate::gameplay) readiness: WeaponReadiness,
    #[serde(default)]
    pub(in crate::gameplay) main_weapon_instead: bool,
}

/// One confrontation begins when the first weapon comes out. Further weapons in the same
/// confrontation update the photograph without asking every NPC to repeat a reply.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct Confrontation {
    pub(in crate::gameplay) npc_id: String,
    pub(in crate::gameplay) reason: String,
}

/// Canonical weapon type from its source identity, independent of the item's in-fiction name.
pub(in crate::gameplay) fn weapon_source_name(
    thing: &SceneThing,
) -> Result<Option<String>, RepositoryError> {
    let catalog = super::super::merchant_bargaining::equipment_catalog().map_err(|_| invalid())?;
    let items = catalog["items"].as_array().ok_or_else(invalid)?;
    Ok(items
        .iter()
        .find(|item| {
            item["kind"] == "weapon"
                && item["id"].as_str() == Some(thing.provenance.source_id.as_str())
        })
        .and_then(|item| item["id"].as_str())
        .map(|id| id.replace('-', " ")))
}

pub(super) fn weapon(thing: &SceneThing) -> Result<bool, RepositoryError> {
    static WEAPONS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    let weapons = match WEAPONS.get() {
        Some(weapons) => weapons,
        None => {
            let catalog =
                super::super::merchant_bargaining::equipment_catalog().map_err(|_| invalid())?;
            let items = catalog["items"].as_array().ok_or_else(invalid)?;
            let built = items
                .iter()
                .filter(|item| item["kind"] == "weapon")
                .filter_map(|item| item["id"].as_str())
                .map(|id| (id.to_owned(), id.replace('-', " ")))
                .collect::<Vec<_>>();
            WEAPONS.get_or_init(|| built)
        }
    };
    Ok(weapons.iter().any(|(id, spaced)| {
        *id == thing.provenance.source_id || spaced.eq_ignore_ascii_case(&thing.name)
    }))
}

/// The source inventory already owns these instances, including dev-seated saved heroes.
/// The scene merely projects them under their existing entity id; it never grants gear.
pub(super) fn install_weapons(
    state: &GameState,
    model: &mut SceneModel,
) -> Result<(), RepositoryError> {
    let catalog = super::super::merchant_bargaining::equipment_catalog().map_err(|_| invalid())?;
    let items = catalog["items"].as_array().ok_or_else(invalid)?;
    for item in state.inventory.iter().filter(|item| item.quantity > 0) {
        let Some(character) = state
            .characters
            .iter()
            .find(|hero| hero.entity == item.owner)
        else {
            continue;
        };
        if !state
            .entities
            .iter()
            .any(|entity| entity.id == item.item && entity.location == Some(character.entity))
        {
            continue;
        }
        let source = item.origin.entry.as_str();
        if !items
            .iter()
            .any(|row| row["id"] == source && row["kind"] == "weapon")
        {
            continue;
        }
        let id = format!("equipment-{}", hex(item.item.as_bytes()));
        if model.things.iter().any(|thing| thing.id == id) {
            continue;
        }
        let hero = name(state, character.entity)?;
        let name = source.replace('-', " ");
        model.things.push(SceneThing {
            id,
            name: name.clone(),
            short: format!("{hero}'s {name}"),
            detail: format!("This {name} belongs to {hero}. It is carried with the party."),
            picture_description: format!("{name}, sheathed and carried; no blade in hand"),
            closer_look: format!("The {name} carried by {hero}."),
            skill: pack::FindingSkill::Perception,
            dc: 10,
            finding_id: None,
            clue_fact_id: None,
            portable: true,
            value_cp: None,
            use_effect: "A carried weapon.".into(),
            position: "sheathed".into(),
            owner_id: Some(hero.clone()),
            in_use: false,
            in_scene: true,
            publicly_visible: true,
            holder_id: Some(hero),
            open: false,
            lit: false,
            broken: false,
            searched_by: Vec::new(),
            aliases: vec![name],
            provenance: SceneItemProvenance {
                source_id: source.to_owned(),
                ..SceneItemProvenance::default()
            },
        });
    }
    Ok(())
}

pub(in crate::gameplay) fn apply(
    model: &mut SceneModel,
    condition: &WeaponCondition,
) -> Result<(), RepositoryError> {
    let thing = model
        .things
        .iter_mut()
        .find(|thing| thing.id == condition.item_id)
        .ok_or_else(invalid)?;
    if thing.holder_id.as_deref() != Some(condition.holder_id.as_str()) || !weapon(thing)? {
        return Err(invalid());
    }
    thing.position = condition.readiness.label().into();
    thing.picture_description = match condition.readiness {
        WeaponReadiness::Drawn => format!("{}, drawn and ready in hand", thing.name),
        WeaponReadiness::Sheathed => {
            format!("{}, sheathed and carried; no blade in hand", thing.name)
        }
    };
    Ok(())
}

/// The native owner may use the same condition for an NPC's own carried weapon. The caller
/// must authorize the NPC action before staging; this does not accept a player-selected NPC.
pub(in crate::gameplay) fn commit(
    current: &Checkpoint,
    operation: OperationId,
    scene: &pack::Scene,
    model: &SceneModel,
    condition: WeaponCondition,
) -> Result<Checkpoint, RepositoryError> {
    let target = condition.item_id.as_str();
    let holder = condition.holder_id.as_str();
    let readiness = condition.readiness;
    let thing = model
        .things
        .iter()
        .find(|thing| thing.id == target)
        .ok_or_else(invalid)?;
    if thing.holder_id.as_deref() != Some(holder) || !weapon(thing)? {
        return Err(invalid());
    }
    let holder_name = model
        .people
        .iter()
        .find(|person| person.id == holder)
        .map(|person| person.name.as_str())
        .ok_or_else(invalid)?;
    let pronouns = current
        .state()
        .characters
        .iter()
        .find(|hero| name(current.state(), hero.entity).is_ok_and(|name| name == holder))
        .and_then(|hero| {
            crate::gameplay::freeform_action::hero_possessive(current.state(), hero.entity)
        });
    let possessive = pronouns.unwrap_or_else(|| format!("{holder_name}'s"));
    let line = match readiness {
        WeaponReadiness::Drawn if condition.main_weapon_instead => format!(
            "{holder_name} has no such weapon; {holder_name} readies {possessive} {} instead.",
            thing.name
        ),
        WeaponReadiness::Drawn => format!("{holder_name} draws {possessive} {}.", thing.name),
        WeaponReadiness::Sheathed if condition.main_weapon_instead => format!(
            "{holder_name} has no such weapon; {holder_name} puts {possessive} {} away instead.",
            thing.name
        ),
        WeaponReadiness::Sheathed => format!("{holder_name} sheathes {possessive} {}.", thing.name),
    };
    let already_drawn = model.things.iter().any(|item| {
        item.position == "drawn"
            && item.holder_id.as_deref().is_some_and(|holder| {
                model
                    .people
                    .iter()
                    .any(|person| person.id == holder && person.role == "hero")
            })
    });
    let confrontation = if readiness == WeaponReadiness::Drawn && !already_drawn {
        model
            .people
            .iter()
            .find(|person| person.id != holder && person.role != "hero" && !person.is_background)
            .map(|person| Confrontation {
                npc_id: person.id.clone(),
                reason: line.clone(),
            })
    } else {
        None
    };
    let mut projected = model.clone();
    apply(&mut projected, &condition)?;
    let interaction = Interaction {
        place: scene.setting.as_ref().ok_or_else(invalid)?.place_id.clone(),
        hero: holder.into(),
        actor: holder.into(),
        target: target.into(),
        option: match readiness {
            WeaponReadiness::Drawn => "draw",
            WeaponReadiness::Sheathed => "sheathe",
        }
        .into(),
        change: Some(SceneChange {
            entity_id: target.into(),
            description: line.clone(),
            kind: SceneChangeKind::Moved {
                position: readiness.label().into(),
            },
        }),
        weapon_readiness: Some(condition.clone()),
        confrontation,
        line,
        accepted: hex(&rpc::AcceptedAction {
            phase: phase(current)? as i32,
            ..Default::default()
        }
        .encode_to_vec()),
        ..Interaction::default()
    };
    let candidate = finish_interaction(current, operation, interaction)?;
    full_snapshot(candidate.state(), scene)?.ok_or_else(invalid)?;
    Ok(candidate)
}

pub(in crate::gameplay) fn confrontation(
    state: &GameState,
    operation: OperationId,
) -> Result<Option<Confrontation>, RepositoryError> {
    state
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .map(read)
        .transpose()
        .map(|entry| entry.flatten().and_then(|entry| entry.confrontation))
}

/// Repeated demands within this visit are one confrontation, independent of which hero spoke.
pub(super) fn demand_confrontation(
    state: &GameState,
    place: &str,
    npc_id: &str,
    reason: &str,
) -> Result<Option<Confrontation>, RepositoryError> {
    let departure = crate::gameplay::place::entries(state)?
        .into_iter()
        .filter(|(_, entry)| entry.change.from == place)
        .map(|(revision, _)| revision)
        .last();
    for decision in state.decisions.iter().rev() {
        if departure.is_some_and(|revision| {
            !revision_is_after(&revision_text(decision.revision), &revision_text(revision))
        }) {
            break;
        }
        if let Some(entry) = read(decision)?
            && entry.place == place
            && entry
                .confrontation
                .is_some_and(|confrontation| confrontation.npc_id == npc_id)
        {
            return Ok(None);
        }
    }
    Ok(Some(Confrontation {
        npc_id: npc_id.into(),
        reason: reason.into(),
    }))
}
