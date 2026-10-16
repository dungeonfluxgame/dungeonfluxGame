//! Committed scene interactions. Presentation caches are rebuilt from this checkpoint journal.
use super::*;
use crate::gameplay::adventure::scene_model::{
    SceneChange, SceneChangeKind, SceneNow, ScenePerson, StartingAttitude,
};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "living-scene-1:";
const POLICY: &str = "living-scene-1";
static MODELS: std::sync::OnceLock<
    std::sync::Mutex<
        std::collections::BTreeMap<String, crate::gameplay::adventure::scene_model::SceneModel>,
    >,
> = std::sync::OnceLock::new();

pub(in crate::gameplay) fn install(
    settings: &crate::gameplay::adventure::setting::ChapterSettings,
) {
    crate::gameplay::adventure::scene_model::install_scenes(settings);
    let mut models = MODELS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    models.clear();
    for setting in [
        &settings.meeting,
        &settings.confrontation,
        &settings.destination,
    ] {
        if let Some(model) = &setting.model {
            models.insert(setting.place_id.clone(), model.clone());
        }
    }
}

pub(in crate::gameplay) fn clear() {
    if let Some(models) = MODELS.get() {
        models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Interaction {
    place: String,
    hero: String,
    change: Option<SceneChange>,
    paid_cp: u32,
    #[serde(default)]
    actor: String,
    #[serde(default)]
    recipient: String,
    line: String,
    accepted: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    option: String,
}

fn invalid() -> RepositoryError {
    refusal_detail::note("living-scene-state");
    RepositoryError::InvalidCandidate
}

fn read(decision: &AcceptedDecision) -> Result<Option<Interaction>, RepositoryError> {
    if decision.source_policy.as_str() != POLICY {
        return Ok(None);
    }
    let text = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(PREFIX))
        .ok_or_else(invalid)?;
    serde_json::from_str(text).map(Some).map_err(|_| invalid())
}

pub(in crate::gameplay) fn accepted(
    decision: &AcceptedDecision,
) -> Result<Option<rpc::AcceptedAction>, RepositoryError> {
    read(decision)?
        .map(|interaction| {
            rpc::AcceptedAction::decode(unhex(&interaction.accepted)?.as_slice())
                .map_err(|_| invalid())
        })
        .transpose()
}

pub(in crate::gameplay) fn snapshot(
    state: &GameState,
    scene: &pack::Scene,
) -> Result<Option<SceneNow>, RepositoryError> {
    let Some(setting) = &scene.setting else {
        return Ok(None);
    };
    // Callers may pass a projected scene whose model already includes this journal. Always
    // replay from the immutable loaded authoring, never from that projected copy.
    let authored = pack::with_content(|content| {
        let mut models = content
            .scenes
            .iter()
            .chain(
                content
                    .beat_script
                    .as_ref()
                    .into_iter()
                    .flat_map(|script| script.scenes.values()),
            )
            .filter_map(|scene| scene.setting.as_ref())
            .filter(|source| source.place_id == setting.place_id)
            .filter_map(|source| source.model.as_ref());
        let Some(first) = models.next() else {
            return Ok(None);
        };
        if models.any(|other| other != first) {
            return Err(invalid());
        }
        Ok(Some(first.clone()))
    })?;
    let Some(mut model) = authored
        .or_else(|| {
            MODELS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&setting.place_id)
                .cloned()
        })
        .or_else(|| setting.model.clone())
    else {
        return Ok(None);
    };
    for character in &state.characters {
        let hero = name(state, character.entity)?;
        if !model.people.iter().any(|person| person.id == hero) {
            model.people.push(ScenePerson {
                id: hero.clone(),
                name: hero.clone(),
                role: "hero".into(),
                description: "Standing with the party.".into(),
                relation_to_speaker: "visitor".into(),
                position: "foreground".into(),
                aliases: vec![hero],
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
                attitudes: Default::default(),
                knowledge: Vec::new(),
                wants: "Stay with the party.".into(),
            });
        }
    }
    let mut changes = Vec::new();
    for decision in &state.decisions {
        if let Some(interaction) = read(decision)?
            && interaction.place == setting.place_id
            && let Some(change) = interaction.change
        {
            changes.push(change);
        }
    }
    let encoded = model
        .as_it_is_now(scene.key, &changes)
        .map_err(|_| invalid())?;
    let mut snapshot: SceneNow = serde_json::from_str(&encoded).map_err(|_| invalid())?;
    for change in &changes {
        if matches!(change.kind, SceneChangeKind::PersonMoved { .. })
            && let Some(person) = snapshot
                .model
                .people
                .iter_mut()
                .find(|person| person.id == change.entity_id)
        {
            person.description = change.description.clone();
        }
    }
    Ok(Some(snapshot))
}

pub(in crate::gameplay) fn publish(current: &Checkpoint) -> Result<(), RepositoryError> {
    let scene = crate::gameplay::beat_script_runtime::scene(current.state())?;
    if let (Some(setting), Some(snapshot)) = (&scene.setting, snapshot(current.state(), &scene)?) {
        crate::gameplay::adventure::scene_model::publish_scene(&setting.place_id, snapshot);
    }
    Ok(())
}

pub(in crate::gameplay) fn balance_cp(
    state: &GameState,
    who: EntityId,
) -> Result<u64, RepositoryError> {
    let hero = name(state, who)?;
    let gold = state
        .inventory
        .iter()
        .filter(|item| item.owner == who && item.origin.entry.as_str() == "gold-piece")
        .try_fold(0u64, |total, item| {
            total.checked_add(item.quantity).ok_or_else(invalid)
        })?;
    let actor = hex(who.as_bytes());
    let (paid, received) =
        state
            .decisions
            .iter()
            .try_fold((0u64, 0u64), |(paid, received), decision| {
                let Some(entry) = read(decision)? else {
                    return Ok((paid, received));
                };
                let owns = entry.actor == actor || (entry.actor.is_empty() && entry.hero == hero);
                Ok((
                    paid.checked_add(if owns { u64::from(entry.paid_cp) } else { 0 })
                        .ok_or_else(invalid)?,
                    received
                        .checked_add(if entry.recipient == actor {
                            u64::from(entry.paid_cp)
                        } else {
                            0
                        })
                        .ok_or_else(invalid)?,
                ))
            })?;
    gold.checked_mul(100)
        .and_then(|amount| amount.checked_add(received))
        .and_then(|amount| amount.checked_sub(paid))
        .ok_or_else(invalid)
}

pub(super) fn has_interactions(state: &GameState) -> bool {
    state
        .decisions
        .iter()
        .any(|decision| decision.source_policy.as_str() == POLICY)
}

pub(in crate::gameplay) fn offer_id(current: &Checkpoint) -> String {
    offer_id_at(current.basis().revision)
}

pub(in crate::gameplay) fn offer_id_at(revision: df_types::SessionRevision) -> String {
    format!(
        "scene-interact-{}-{}",
        revision.epoch().get(),
        revision.sequence()
    )
}

pub(in crate::gameplay) fn available(
    current: &Checkpoint,
    member: MemberId,
) -> Result<bool, RepositoryError> {
    let who = player_entity(member, current)?;
    Ok(matches!(
        phase(current)?,
        rpc::JourneyPhase::Opening | rpc::JourneyPhase::Dialogue | rpc::JourneyPhase::Complete
    ) && current
        .state()
        .characters
        .iter()
        .any(|character| character.entity == who)
        && value(current.state(), who, "unconscious")? == 0
        && !crate::gameplay::beat_script_runtime::ended(current.state())?
        && !(tactical::present(current.state()) && tactical::hero_is_dead(current.state(), who)?)
        && snapshot(
            current.state(),
            &crate::gameplay::beat_script_runtime::scene(current.state())?,
        )?
        .is_some())
}

pub(in crate::gameplay) fn last_line(state: &GameState) -> Result<Option<String>, RepositoryError> {
    state
        .decisions
        .last()
        .map(read)
        .transpose()
        .map(|entry| entry.flatten().map(|entry| entry.line))
}

pub(in crate::gameplay) fn line(
    state: &GameState,
    operation: OperationId,
) -> Result<String, RepositoryError> {
    let decision = state
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .ok_or_else(invalid)?;
    read(decision)?.map(|entry| entry.line).ok_or_else(invalid)
}

pub(in crate::gameplay) fn hero(
    state: &GameState,
    operation: OperationId,
) -> Result<String, RepositoryError> {
    let decision = state
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .ok_or_else(invalid)?;
    read(decision)?.map(|entry| entry.hero).ok_or_else(invalid)
}

pub(in crate::gameplay) fn coin_was_transferred(
    state: &GameState,
    operation: OperationId,
) -> Result<bool, RepositoryError> {
    let decision = state
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .ok_or_else(invalid)?;
    read(decision)?
        .map(|entry| entry.paid_cp > 0)
        .ok_or_else(invalid)
}

pub(in crate::gameplay) fn item_for(
    state: &GameState,
    scene: &pack::Scene,
    hero: &str,
) -> Result<Option<rpc::SceneItemCard>, RepositoryError> {
    let Some(setting) = &scene.setting else {
        return Ok(None);
    };
    let Some(snapshot) = snapshot(state, scene)? else {
        return Ok(None);
    };
    let model = &snapshot.model;
    let scope = crate::gameplay::beat_script_runtime::scene_scope(state)?;
    let mut projected = scene.clone();
    if let Some(setting) = projected.setting.as_mut() {
        setting.model = Some(model.clone());
    }
    let mut retired_targets = std::collections::BTreeSet::new();
    for decision in state.decisions.iter().rev() {
        let mut was_take = false;
        let target = if let Some(entry) = read(decision)? {
            if entry.place != setting.place_id || entry.hero != hero {
                continue;
            }
            if matches!(entry.option.as_str(), "return" | "drop" | "give") {
                retired_targets.insert(entry.target.clone());
                continue;
            }
            if entry.paid_cp > 0 && matches!(entry.option.as_str(), "buy" | "use" | "trade") {
                format!("purchased-{}", entry.target)
            } else if matches!(
                entry.option.as_str(),
                "examine" | "take" | "steal" | "share"
            ) {
                was_take = entry.option == "take";
                entry.target
            } else {
                continue;
            }
        } else if decision.source_policy.as_str() == THREAD_POLICY {
            // This branch is a registered rules receipt, not this module's optional scene
            // journal receipt (which deliberately returns None for THREAD_POLICY).
            let Some(check) = super::accepted(decision)?.check else {
                continue;
            };
            if !check
                .source_revision
                .starts_with(&format!("srd521-p6:{scope}:"))
            {
                continue;
            }
            let Some((_, _, member)) =
                crate::gameplay::beat_script_runtime::choice(state, decision)?
            else {
                continue;
            };
            let who = state
                .members
                .iter()
                .find(|link| link.member == member)
                .and_then(|link| link.character)
                .ok_or_else(invalid)?;
            if name(state, who)? != hero {
                continue;
            }
            let Some(target) = crate::gameplay::investigation::check_target(&projected, &check)
            else {
                continue;
            };
            let Some(thing) = target.id.strip_prefix("thing-") else {
                continue;
            };
            thing.to_owned()
        } else {
            continue;
        };
        let Some(thing) = model.things.iter().find(|thing| thing.id == target) else {
            continue;
        };
        if retired_targets.contains(&thing.id) {
            continue;
        }
        // A stale take receipt must not keep an object in the phone's hand after a
        // return, drop, or give. A later examine can still deliberately select it
        // for review from its new location.
        if was_take && thing.holder_id.as_deref() != Some(hero) {
            continue;
        }
        return Ok(Some(rpc::SceneItemCard {
            thing_id: thing.id.clone(),
            name: thing.name.clone(),
            detail: thing.detail.clone(),
            image_path: String::new(),
            image_state: rpc::PlateState::Failed as i32,
            state_revision: decision.revision.sequence(),
            observation_only: thing.holder_id.as_deref() == Some(hero)
                && thing.owner_id.as_deref() != Some(hero),
            holder: thing.holder_id.clone().unwrap_or_default(),
        }));
    }
    Ok(None)
}

/// Returns every currently held, non-owned scene thing for a phone or public display.
/// The authoritative snapshot, rather than the last decision, prevents an old take
/// receipt from resurrecting an item after it was returned, dropped, or given away.
pub(in crate::gameplay) fn hand_items(
    state: &GameState,
    scene: &pack::Scene,
    holder: Option<&str>,
    state_revision: u64,
) -> Result<Vec<rpc::SceneItemCard>, RepositoryError> {
    let Some(snapshot) = snapshot(state, scene)? else {
        return Ok(Vec::new());
    };
    Ok(snapshot
        .model
        .things
        .iter()
        .filter(|thing| {
            thing.in_scene
                && thing.holder_id.is_some()
                && thing.owner_id.as_deref() != thing.holder_id.as_deref()
                && (holder.is_some() || thing.publicly_visible)
                && holder.is_none_or(|expected| thing.holder_id.as_deref() == Some(expected))
        })
        .map(|thing| rpc::SceneItemCard {
            thing_id: thing.id.clone(),
            name: thing.name.clone(),
            detail: thing.detail.clone(),
            image_path: String::new(),
            image_state: rpc::PlateState::Failed as i32,
            state_revision,
            observation_only: true,
            holder: thing.holder_id.clone().unwrap_or_default(),
        })
        .collect())
}

pub(in crate::gameplay) fn gift_amount(words: &str) -> Option<u32> {
    let tokens = words.split_whitespace().collect::<Vec<_>>();
    let amounts = tokens
        .windows(2)
        .filter_map(|pair| {
            pair[1]
                .trim_end_matches(['.', ',', '?'])
                .eq_ignore_ascii_case("gp")
                .then(|| pair[0].parse::<u32>().ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    match amounts.as_slice() {
        [amount] if *amount > 0 => Some(*amount),
        _ => None,
    }
}

/// One explicit coin quote for a catalog trade. Goods have no implied barter valuation.
pub(in crate::gameplay) fn payment_amount_cp(words: &str) -> Option<u32> {
    let tokens = words.split_whitespace().collect::<Vec<_>>();
    let amounts = tokens
        .windows(2)
        .filter_map(|pair| {
            let amount = pair[0].parse::<u32>().ok()?;
            let currency = pair[1].trim_end_matches(['.', ',', '?']).to_lowercase();
            match currency.as_str() {
                "cp" | "copper" => Some(amount),
                "gp" | "gold" => amount.checked_mul(100),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    let [amount] = amounts.as_slice() else {
        return None;
    };
    (*amount > 0).then_some(*amount)
}

pub(in crate::gameplay) fn interaction_for_line(
    scene: &pack::Scene,
    words: &str,
) -> Option<(String, &'static str)> {
    let model = scene.setting.as_ref()?.model.as_ref()?.for_scene(scene.key);
    let lower = words.trim().to_lowercase();
    if words.contains('"')
        || words.matches('\'').count() >= 2
        || lower.ends_with('?')
        || [
            "if i ",
            "can i ",
            "could i ",
            "i will ",
            "i won't ",
            "i do not ",
            "i don't ",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return None;
    }
    let direct = [
        "i give ",
        "i return ",
        "i put ",
        "i drop ",
        "i pick up ",
        "i take the ",
        "i grab the ",
        "i steal ",
        "i open the ",
        "i close the ",
        "i buy ",
        "i trade ",
        "i want a pint",
        "i pour myself",
        "i order a ",
        "i show ",
        "i share ",
        "i lean on ",
        "i sit by ",
        "i sit at ",
        "i join ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    if !lower.starts_with("whose ") && (!direct || lower.ends_with('?')) {
        return None;
    }
    let option = if lower.starts_with("i trade ") {
        "trade"
    } else if lower.starts_with("i return ")
        || lower.starts_with("i put ") && lower.contains(" back")
    {
        "return"
    } else if lower.starts_with("i drop ") {
        "drop"
    } else if lower.starts_with("i give ") && gift_amount(words).is_some() {
        "gift"
    } else if lower.starts_with("i give ") {
        "give"
    } else if lower.starts_with("i steal ") {
        "steal"
    } else if ["pick up", "take the ", "grab the "]
        .iter()
        .any(|verb| lower.contains(verb))
    {
        "take"
    } else if lower.contains("open the ") {
        "open"
    } else if lower.contains("close the ") {
        "close"
    } else if lower.contains("show ") || lower.contains("share ") {
        "share"
    } else if ["buy ", "want a pint", "pour myself", "order a "]
        .iter()
        .any(|verb| lower.contains(verb))
    {
        "buy"
    } else if lower.contains("whose mug") || lower.contains("whose tankard") {
        "examine"
    } else if ["lean on", "sit by", "sit at", "join "]
        .iter()
        .any(|verb| lower.contains(verb))
    {
        "join"
    } else {
        return None;
    };
    let resolved = model.resolve_target(words)?;
    let id = resolved.ids.first()?.clone();
    Some((id, option))
}

pub(in crate::gameplay) fn recipient_for_line(scene: &pack::Scene, words: &str) -> Option<String> {
    let lower = words.to_lowercase();
    let (_, recipient) = lower.split_once(" to ")?;
    let model = scene.setting.as_ref()?.model.as_ref()?.for_scene(scene.key);
    let resolved = model.resolve_target(recipient)?;
    if resolved.kind != crate::gameplay::adventure::scene_model::SceneTargetKind::Person {
        return None;
    }
    let [id] = resolved.ids.as_slice() else {
        return None;
    };
    Some(id.clone())
}

pub(super) fn stage(
    current: &Checkpoint,
    command: &CommandInput,
) -> Result<Checkpoint, RepositoryError> {
    let GameCommand::ProposeAction {
        actor: who,
        choices,
        targets,
        ..
    } = &command.command
    else {
        return Err(invalid());
    };
    if command.basis != current.basis()
        || command.observed_revision != current.basis().revision
        || !targets.is_empty()
        || !available(current, command.member)?
        || !(2..=3).contains(&choices.len())
    {
        return Err(invalid());
    }
    let get = |group| {
        let mut values = choices.iter().filter(|(key, _)| key.as_str() == group);
        let value = values.next().map(|(_, value)| value.as_str());
        if values.next().is_some() { None } else { value }
    };
    let target = get("scene-target").ok_or_else(invalid)?;
    let option = get("scene-option").ok_or_else(invalid)?;
    let named_recipient = get("scene-recipient");
    if choices.len()
        != if matches!(option, "gift" | "trade") || option == "give" && named_recipient.is_some() {
            3
        } else {
            2
        }
    {
        return Err(invalid());
    }
    let scene = crate::gameplay::beat_script_runtime::scene(current.state())?;
    let snapshot = snapshot(current.state(), &scene)?.ok_or_else(invalid)?;
    let hero = name(current.state(), *who)?;
    let mut change = None;
    let mut paid_cp = 0;
    let mut recipient = String::new();
    let line = if option == "examine"
        && (target.starts_with("absent-") || target.starts_with("surface-"))
    {
        // SRD 5.2.1 p. 6: dice resolve uncertain outcomes; visible surfaces without findings need no check.
        let probe = rpc::AbilityCheckResult {
            source_revision: format!("srd521-p6:scene:perception:target-{target}"),
            ..Default::default()
        };
        let mut visible = scene.clone();
        if let Some(setting) = visible.setting.as_mut() {
            setting.model = Some(snapshot.model.clone());
        }
        let observed = crate::gameplay::investigation::check_target(&visible, &probe)
            .filter(|entry| {
                !entry.present
                    || entry.id.starts_with("surface-")
                        && crate::gameplay::investigation::finding_on(&visible, entry, "perception")
                            .is_none()
            })
            .ok_or_else(invalid)?;
        observed.description
    } else if let Some(thing) = snapshot
        .model
        .things
        .iter()
        .find(|thing| thing.id == target)
    {
        if !thing.in_scene && thing.holder_id.as_deref() != Some(&hero) {
            format!("You cannot see {} here.", thing.name)
        } else {
            match option {
                "examine" => {
                    let owner = thing.owner_id.as_ref().and_then(|id| {
                        snapshot.model.people.iter().find(|person| &person.id == id)
                    });
                    format!(
                        "{} {}",
                        thing.detail,
                        owner.map_or_else(
                            || "You cannot tell who owns it.".to_owned(),
                            |owner| format!("It belongs to {}.", owner.name)
                        )
                    )
                }
                "take" if !thing.portable => format!("{} cannot be carried away.", thing.name),
                "take" if thing.holder_id.as_deref() == Some(&hero) => {
                    format!("You already hold {}.", thing.name)
                }
                "take" if thing.in_use || thing.holder_id.is_some() => {
                    format!("{} is being used; ask its holder first.", thing.name)
                }
                "take" => {
                    let description = format!("{hero} picks up {}.", thing.name);
                    change = Some(SceneChange {
                        entity_id: thing.id.clone(),
                        description: description.clone(),
                        kind: SceneChangeKind::Taken {
                            holder_id: hero.clone(),
                        },
                    });
                    description
                }
                "steal" if !thing.portable => format!("{} cannot be carried away.", thing.name),
                "steal" if thing.holder_id.is_some() => {
                    format!("{} is already being held.", thing.name)
                }
                "steal" => {
                    let description = format!("{hero} steals {}.", thing.name);
                    change = Some(SceneChange {
                        entity_id: thing.id.clone(),
                        description: description.clone(),
                        kind: SceneChangeKind::Stolen {
                            holder_id: hero.clone(),
                        },
                    });
                    description
                }
                "open" | "close" => {
                    let description = format!(
                        "{hero} {} {}.",
                        if option == "open" { "opens" } else { "closes" },
                        thing.name
                    );
                    change = Some(SceneChange {
                        entity_id: thing.id.clone(),
                        description: description.clone(),
                        kind: if option == "open" {
                            SceneChangeKind::Opened
                        } else {
                            SceneChangeKind::Closed
                        },
                    });
                    description
                }
                "use" => format!("{hero} uses {}. {}", thing.name, thing.use_effect),
                "return" | "drop" if thing.holder_id.as_deref() != Some(&hero) => {
                    format!("You are not holding {}.", thing.name)
                }
                "return" | "drop" => {
                    let position = if option == "drop" {
                        "on the floor".to_owned()
                    } else {
                        pack::with_content(|content| {
                            content
                                .scenes
                                .iter()
                                .filter_map(|source| source.setting.as_ref())
                                .filter(|source| {
                                    source.place_id
                                        == scene
                                            .setting
                                            .as_ref()
                                            .map_or("", |setting| setting.place_id.as_str())
                                })
                                .filter_map(|source| source.model.as_ref())
                                .flat_map(|model| &model.things)
                                .find(|source| source.id == thing.id)
                                .map(|source| source.position.clone())
                        })
                        .unwrap_or_else(|| "on the floor".into())
                    };
                    let description = format!("{hero} puts {} {position}.", thing.name);
                    change = Some(SceneChange {
                        entity_id: thing.id.clone(),
                        description: description.clone(),
                        kind: SceneChangeKind::Returned { position },
                    });
                    description
                }
                "give"
                    if thing.holder_id.as_deref() == Some(&hero)
                        && (named_recipient.is_some()
                            || thing.owner_id.as_deref() != Some(&hero)) =>
                {
                    let recipient = named_recipient
                        .or(thing.owner_id.as_deref())
                        .ok_or_else(invalid)?;
                    let person = snapshot
                        .model
                        .people
                        .iter()
                        .find(|person| person.id == recipient)
                        .ok_or_else(invalid)?;
                    if thing
                        .owner_id
                        .as_deref()
                        .is_some_and(|owner| owner != hero && owner != recipient)
                    {
                        format!(
                            "{} belongs to someone else; return it to its owner.",
                            thing.name
                        )
                    } else {
                        let description =
                            format!("{hero} gives {} to {}.", thing.name, person.name);
                        let owner_id = (thing.owner_id.as_deref() == Some(hero.as_str()))
                            .then(|| recipient.to_owned());
                        change = Some(SceneChange {
                            entity_id: thing.id.clone(),
                            description: description.clone(),
                            kind: SceneChangeKind::Given {
                                holder_id: recipient.to_owned(),
                                owner_id,
                            },
                        });
                        description
                    }
                }
                "give" if thing.holder_id.as_deref() == Some(&hero) => format!(
                    "You must name a recipient before giving {} away.",
                    thing.name
                ),
                "join" => {
                    let description = format!("{hero} sits beside {}.", thing.name);
                    change = Some(SceneChange {
                        entity_id: hero.clone(),
                        description: description.clone(),
                        kind: SceneChangeKind::PersonMoved {
                            position: thing.position.clone(),
                        },
                    });
                    description
                }
                "share" => format!("{hero} shows {} to the party. {}", thing.name, thing.detail),
                _ => return Err(invalid()),
            }
        }
    } else if let Some(way) = snapshot.model.ways_out.iter().find(|way| way.id == target) {
        match option {
            "open" | "close" => {
                let description = format!(
                    "{hero} {} {}.",
                    if option == "open" { "opens" } else { "closes" },
                    way.name
                );
                change = Some(SceneChange {
                    entity_id: way.id.clone(),
                    description: description.clone(),
                    kind: if option == "open" {
                        SceneChangeKind::Opened
                    } else {
                        SceneChangeKind::Closed
                    },
                });
                description
            }
            "examine" => format!("{} leads to {}.", way.name, way.beyond),
            _ => return Err(invalid()),
        }
    } else if let Some(offer) = snapshot
        .model
        .offers
        .iter()
        .find(|offer| offer.id == target)
    {
        if !matches!(option, "buy" | "use" | "trade" | "examine") {
            return Err(invalid());
        }
        if option == "examine" {
            format!("{} It costs {} CP.", offer.description, offer.cost_cp)
        } else if option == "trade"
            && get("scene-payment").and_then(|amount| amount.parse::<u32>().ok())
                != Some(offer.cost_cp)
        {
            format!(
                "{} costs {} CP; that trade has not been agreed.",
                offer.name, offer.cost_cp
            )
        } else if !snapshot.model.offer_available(&offer.id) {
            format!("{} is already sold.", offer.name)
        } else if balance_cp(current.state(), *who)? < u64::from(offer.cost_cp) {
            format!(
                "{} costs {} CP; you do not have enough coin.",
                offer.name, offer.cost_cp
            )
        } else {
            paid_cp = offer.cost_cp;
            change = Some(SceneChange {
                entity_id: offer.id.clone(),
                description: format!("{hero} buys {}.", offer.name),
                kind: SceneChangeKind::Purchased {
                    holder_id: hero.clone(),
                    seller_id: offer.seller_id.clone(),
                },
            });
            let seller = snapshot
                .model
                .people
                .iter()
                .find(|person| person.id == offer.seller_id)
                .ok_or_else(invalid)?;
            let ale = offer.name.to_lowercase().contains("ale");
            let server = if ale {
                snapshot
                    .model
                    .people
                    .iter()
                    .find(|person| person.is_background && person.role == "serving woman")
                    .unwrap_or(seller)
            } else {
                seller
            };
            let serving = if ale { "pours" } else { "hands over" };
            format!(
                "{} {serving} {} for {hero}, who pays {} {} CP.",
                server.name,
                offer.description.trim_end_matches('.'),
                seller.name,
                offer.cost_cp
            )
        }
    } else if let Some(person) = snapshot
        .model
        .people
        .iter()
        .find(|person| person.id == target)
    {
        match option {
            "examine" | "talk" => format!(
                "{}: {} {}",
                person.name, person.description, person.relation_to_speaker
            ),
            "gift" => {
                let amount = get("scene-amount")
                    .and_then(|amount| amount.parse::<u32>().ok())
                    .filter(|amount| *amount > 0)
                    .ok_or_else(invalid)?;
                let cost = amount.checked_mul(100).ok_or_else(invalid)?;
                if balance_cp(current.state(), *who)? < u64::from(cost) {
                    format!("You do not have {amount} GP to give {}.", person.name)
                } else {
                    paid_cp = cost;
                    if person.role == "hero" && !person.is_background {
                        let mut matches = Vec::new();
                        for character in &current.state().characters {
                            if name(current.state(), character.entity)? == person.name {
                                matches.push(character.entity);
                            }
                        }
                        if matches.len() != 1 {
                            return Err(invalid());
                        }
                        recipient = hex(matches[0].as_bytes());
                    }
                    format!("{hero} gives {} {amount} GP.", person.name)
                }
            }
            "join" => {
                let description = format!("{hero} joins {}.", person.name);
                change = Some(SceneChange {
                    entity_id: hero.clone(),
                    description: description.clone(),
                    kind: SceneChangeKind::PersonMoved {
                        position: person.position.clone(),
                    },
                });
                description
            }
            _ => return Err(invalid()),
        }
    } else {
        return Err(invalid());
    };
    if line.is_empty() {
        return Err(invalid());
    }
    let accepted = rpc::AcceptedAction {
        phase: phase(current)? as i32,
        ..Default::default()
    };
    let interaction = Interaction {
        place: scene.setting.as_ref().ok_or_else(invalid)?.place_id.clone(),
        hero,
        change,
        paid_cp,
        actor: hex(who.as_bytes()),
        recipient,
        line,
        accepted: hex(&accepted.encode_to_vec()),
        target: target.to_owned(),
        option: option.to_owned(),
    };
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().map_err(|_| invalid())?;
    finish(
        current,
        basis,
        current.state().clone(),
        command.operation,
        Vec::new(),
        DecisionContent {
            entry: "scene-interaction",
            semantic: Some(format!(
                "{PREFIX}{}",
                serde_json::to_string(&interaction).map_err(|_| invalid())?
            )),
            policy: POLICY,
            audience: AudienceScope::Shared,
            reaction: None,
            escort_target: None,
            objective_command: None,
            script_choice: None,
        },
    )
}
