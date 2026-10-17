//! Durable authored choices for scripts whose scenes need no encounter or new rules.
use super::{
    beat_script::{self, Beat, BeatEffect, BeatKind, BeatOffer, BeatScript, BeatTrigger},
    journey, model, pack, refusal_detail,
};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::MemberId;

pub(super) const STEP: &str = "beat-script-step";
/// The submitted action entry of an authored investigation (`observe`) choice.
pub(super) const CHOICE_ACTION: &str = "script-choice";
/// The submitted action entry of a resolution choice.
pub(super) const RESOLVE_ACTION: &str = "resolve-errand";
const OFFER_PREFIX: &str = "beat-script-offer-";
fn invalid<T>() -> Result<T, RepositoryError> {
    refusal_detail::note("beat-script-state");
    Err(RepositoryError::InvalidCandidate)
}
pub(super) fn marker(offer: &str) -> Result<ContentReference, RepositoryError> {
    model::content(&format!("{OFFER_PREFIX}{offer}"))
}
pub(super) fn choice<'a>(
    state: &'a GameState,
    decision: &'a AcceptedDecision,
) -> Result<Option<(&'a GameFact, &'a str, MemberId)>, RepositoryError> {
    let mut chosen = None;
    for fact in &state.facts {
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &fact.value
        else {
            continue;
        };
        let Some(offer) = definition.entry.as_str().strip_prefix(OFFER_PREFIX) else {
            continue;
        };
        if !decision.facts.contains(&fact.id) {
            continue;
        }
        if chosen.is_some()
            || fact.operation != decision.operation
            || fact.revision != decision.revision
            || decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
            || fact.audience != AudienceScope::Shared
            || decision.source_policy.as_str() != journey::THREAD_POLICY
        {
            return invalid();
        }
        let [actor] = subjects.as_slice() else {
            return invalid();
        };
        if *definition != model::content(definition.entry.as_str())? {
            return invalid();
        }
        let index = state
            .facts
            .iter()
            .position(|f| f.id == fact.id)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let preceding = index.checked_sub(1).and_then(|i| state.facts.get(i));
        if fact.cause != preceding.map(|f| f.id)
            || (fact.ordinal > 0
                && preceding.is_none_or(|f| {
                    f.operation != fact.operation
                        || f.revision != fact.revision
                        || f.ordinal.checked_add(1) != Some(fact.ordinal)
                        || decision.facts.get(f.ordinal as usize) != Some(&f.id)
                }))
        {
            return invalid();
        }
        let bootstrap = journey::bootstrap_member()?;
        let member = state
            .members
            .iter()
            .find(|link| link.character == Some(*actor) && link.member != bootstrap)
            .map(|link| link.member)
            .ok_or(RepositoryError::InvalidCandidate)?;
        chosen = Some((fact, offer, member));
    }
    Ok(chosen)
}
pub(super) fn last_choice(state: &GameState) -> Result<Option<String>, RepositoryError> {
    let mut latest = None;
    let mut revision = None;
    for decision in &state.decisions {
        if revision.is_some_and(|previous| decision.revision <= previous) {
            return invalid();
        }
        revision = Some(decision.revision);
        if let Some((_, offer, _)) = choice(state, decision)? {
            latest = Some(offer.to_owned());
        }
    }
    // A marker without its accepted decision is not an authored choice.
    let actual=state.facts.iter().filter(|fact|matches!(&fact.value,FactValue::ContentEvent{definition,..} if definition.entry.as_str().starts_with(OFFER_PREFIX))).count();
    let admitted = state
        .decisions
        .iter()
        .map(|decision| choice(state, decision).map(|c| usize::from(c.is_some())))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum::<usize>();
    if actual != admitted {
        return invalid();
    }
    Ok(latest)
}
pub(super) fn scripted_id(current: &Checkpoint, action: rpc::GameplayActionKind) -> Option<String> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return None;
        }
        let offer = script
            .current(current.state())
            .ok()?
            .offers
            .iter()
            .find(|o| o.effect.action() == action)?;
        Some(format!(
            "beat-{}:{}:{}",
            offer.id,
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence()
        ))
    })
}
pub(super) fn phase_of(beat: &Beat) -> rpc::JourneyPhase {
    if beat.state.durable == "room" {
        return rpc::JourneyPhase::Room;
    }
    if beat.state.durable == "opening" {
        return rpc::JourneyPhase::Opening;
    }
    match beat.kind {
        BeatKind::Gather => rpc::JourneyPhase::Room,
        BeatKind::Converse | BeatKind::Investigate | BeatKind::Narrate => {
            rpc::JourneyPhase::Dialogue
        }
        BeatKind::Fight => rpc::JourneyPhase::Combat,
        BeatKind::Rest | BeatKind::Travel | BeatKind::Conclude | BeatKind::Ending => {
            rpc::JourneyPhase::Complete
        }
    }
}

/// SRD 5.2.1 p. 6: "Add your Proficiency Bonus to an ability check when the GM determines that a
/// skill or tool proficiency is relevant to the check and you have that proficiency." Search
/// (p. 187) says "you make a Wisdom check"; Study (p. 189) says "you make an Intelligence check";
/// Hide (p. 183) requires a "DC 15 Dexterity (Stealth) check."
/// Ability and proficiency components for the local SRD Search, Study and Hide skills.
pub(super) fn skill_check_modifiers(
    state: &GameState,
    actor: EntityId,
    skill: &str,
) -> Result<(i8, u8), RepositoryError> {
    let ability_name = match skill {
        "perception" | "insight" | "survival" | "medicine" | "animal-handling" => "wisdom",
        "investigation" | "arcana" | "history" | "nature" | "religion" => "intelligence",
        "stealth" | "sleight-of-hand" | "acrobatics" => "dexterity",
        "athletics" => "strength",
        "intimidation" | "persuasion" | "deception" | "performance" => "charisma",
        _ => {
            refusal_detail::note("search-skill");
            return Err(RepositoryError::InvalidCandidate);
        }
    };
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == actor)
        .ok_or_else(|| {
            refusal_detail::note("search-character");
            RepositoryError::InvalidCandidate
        })?;
    let score = u8::try_from(journey::value(state, actor, ability_name)?).map_err(|_| {
        refusal_detail::note("search-ability-score");
        RepositoryError::InvalidCandidate
    })?;
    let background_proficiency = character.build.entry.as_str() == "dwarf-fighter-soldier"
        && matches!(skill, "athletics" | "intimidation");
    let chosen_proficiency = character.choices.iter().any(|choice| {
        matches!(choice.offer.as_str(), "skill-1" | "skill-2") && choice.selected.as_str() == skill
    });
    let proficiency = if background_proficiency || chosen_proficiency {
        pack::with_creation(|creation| creation.proficiency_bonus)
    } else {
        0
    };
    let ability_modifier = df_rules::local_journey::ability_modifier(score);
    let ability_modifier = i8::try_from(ability_modifier).map_err(|_| {
        refusal_detail::note("search-ability-modifier");
        RepositoryError::InvalidCandidate
    })?;
    Ok((ability_modifier, proficiency))
}

/// Passive Perception for this local normal-check profile.
pub(super) fn passive_perception(
    state: &GameState,
    actor: EntityId,
) -> Result<i16, RepositoryError> {
    let (ability_modifier, proficiency) = skill_check_modifiers(state, actor, "perception")?;
    Ok(10 + i16::from(ability_modifier) + i16::from(proficiency))
}

pub(super) fn resolve_skill_check(
    state: &GameState,
    actor: EntityId,
    skill: &str,
    die: u8,
    difficulty_class: u8,
) -> Result<df_rules::ability_check::AbilityCheckResult, RepositoryError> {
    let (ability_modifier, proficiency_bonus) = skill_check_modifiers(state, actor, skill)?;
    df_rules::ability_check::resolve(df_rules::ability_check::AbilityCheckInput {
        die,
        ability_modifier,
        proficiency_bonus,
        difficulty_class,
    })
    .map_err(|_| {
        refusal_detail::note("search-check");
        RepositoryError::InvalidCandidate
    })
}

pub(super) fn scene(state: &GameState) -> Result<pack::Scene, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        let variant = shown_variant(script, beat, state)?;
        let mut scene = match &variant {
            Some(variant) => {
                let mut authored = script
                    .scenes
                    .get(variant)
                    .cloned()
                    .ok_or(RepositoryError::InvalidCandidate)?;
                authored.source_variant_id = Some(variant.clone());
                // Who stands on stage is the adventure's own cast, never the static base's.
                authored.cast = pack::scene(beat.scene).cast;
                if beat.kind == BeatKind::Ending {
                    // The closing scenes keep the patron on stage beside the contact.
                    for actor in pack::scene(pack::SceneKey::Opening).cast {
                        if !authored.cast.iter().any(|present| present.id == actor.id) {
                            authored.cast.push(actor);
                        }
                    }
                }
                authored
            }
            None => pack::scene(beat.scene),
        };
        if let Some(variant) = &variant
            && let Some(asset) =
                pack::active().and_then(|loaded| loaded.script_scene_assets.get(variant).cloned())
        {
            scene.asset = asset;
        }
        if let Some(snapshot) = journey::living_scene::snapshot(state, &scene)?
            && let Some(setting) = scene.setting.as_mut()
        {
            setting.model = Some(snapshot.model);
        }
        // People who fled a fight are gone from the scene, and people it left down neither answer
        // nor can be spoken to: only those still standing are on stage. The narration says where
        // the others lie (`scene_fight::aftermath_narration`).
        let mut standing = Vec::with_capacity(scene.cast.len());
        for member in std::mem::take(&mut scene.cast) {
            if journey::scene_fight::presence(state, &member.id)?
                == journey::scene_fight::Presence::Standing
            {
                standing.push(member);
            }
        }
        scene.cast = standing;
        Ok(scene)
    })
}

pub(super) fn scene_scope(state: &GameState) -> Result<String, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        Ok(shown_variant(script, beat, state)?
            .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase()))
    })
}
/// How the heroes' fight ended, from the tactical snapshot (or, in a legacy knock-out fight, from
/// which heroes are down).
pub(super) fn fight_outcome(
    state: &GameState,
) -> Result<journey::tactical::FightOutcome, RepositoryError> {
    use journey::tactical::FightOutcome as O;
    if journey::tactical::present(state) {
        return journey::tactical::outcome(state);
    }
    let won = journey::combat_victory(state)?;
    let mut down = false;
    for character in &state.characters {
        down |= journey::value(state, character.entity, "unconscious")? != 0;
    }
    Ok(match (won, down) {
        (true, false) => O::Won,
        (true, true) => O::WonCostly,
        (false, _) => O::Lost,
    })
}
/// The scene variant a beat shows. A fight's closing steps name their victory variant; the one
/// shown is the variant of the way the fight actually ended.
fn shown_variant(
    script: &BeatScript,
    beat: &Beat,
    state: &GameState,
) -> Result<Option<String>, RepositoryError> {
    let Some(variant) = &beat.scene_variant else {
        return Ok(None);
    };
    if !(script.uses_encounter() && beat.kind == BeatKind::Ending) {
        return Ok(Some(variant.clone()));
    }
    let stem = variant
        .strip_suffix(journey::tactical::FightOutcome::Won.slug())
        .ok_or(RepositoryError::InvalidCandidate)?;
    Ok(Some(format!("{stem}{}", fight_outcome(state)?.slug())))
}
pub(super) fn native_inn(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        Ok(script.uses_encounter()
            && beat.scene == pack::SceneKey::Inn
            && beat.state.durable == "harbor-inn"
            && beat.scene_variant.is_none())
    })
}
pub(super) fn destination_scene(beat: &Beat) -> Result<rpc::GameplayScene, RepositoryError> {
    let scene = beat_script::with_script(|script| match &beat.scene_variant {
        Some(id) => {
            let mut scene = script
                .scenes
                .get(id)
                .cloned()
                .ok_or(RepositoryError::InvalidCandidate)?;
            scene.source_variant_id = Some(id.clone());
            Ok(scene)
        }
        None => {
            let mut scene = pack::scene(beat.scene);
            scene.asset = pack::source_scene_asset(beat.scene)?;
            Ok(scene)
        }
    })?;
    Ok(rpc::GameplayScene {
        destination: rpc::HarborDestination::HarborInn as i32,
        title: scene.title,
        scene_asset: scene.asset,
        source_scene_key: scene.key.source_label().to_owned(),
        source_scene_variant: scene.source_variant_id.unwrap_or_default(),
        speaker_portrait: scene.speaker_portrait,
        cast: Vec::new(),
        rules_note: String::new(),
        place: String::new(),
        location_id: String::new(),
        live_backdrop: String::new(),
        live_plate: String::new(),
        plate_layout: None,
        plate_state: rpc::PlateState::Unspecified as i32,
        ..Default::default()
    })
}
pub(super) fn pure_target(
    current: &Checkpoint,
    selected: &BeatOffer,
) -> Result<Option<Beat>, RepositoryError> {
    beat_script::with_script(|script| {
        // A fight's native handlers own everything up to its ending; only the ending's own
        // steps are pure.
        let pure = if script.uses_encounter() {
            matches!(selected.effect, BeatEffect::Observe | BeatEffect::Resolve)
        } else {
            matches!(
                selected.effect,
                BeatEffect::Continue
                    | BeatEffect::Observe
                    | BeatEffect::Resolve
                    | BeatEffect::ChooseHarborScene
            )
        };
        if !pure {
            return Ok(None);
        }
        let beat = script.current(current.state())?;
        let trigger = BeatTrigger::Offer {
            offer: selected.id.clone(),
        };
        let next = beat
            .transitions
            .iter()
            .find(|t| t.trigger == trigger)
            .and_then(|t| script.find(&t.target))
            .ok_or(RepositoryError::InvalidCandidate)?;
        Ok(Some(next.clone()))
    })
}
/// Content entries the ending steps of every script (compiled or authored) use.
pub(super) const ENDING_ENTRIES: &[&str] = &[STEP, RESOLVE_ACTION, "epilogue", "the-end"];
pub(super) fn contents() -> Vec<String> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            let mut entries = vec![CHOICE_ACTION.into()];
            for offer in script.beats.iter().flat_map(|beat| &beat.offers) {
                if offer.effect == BeatEffect::Observe {
                    let entry = format!("{OFFER_PREFIX}{}", offer.id);
                    if !entries.contains(&entry) {
                        entries.push(entry);
                    }
                }
            }
            return entries;
        }
        let mut entries = vec![STEP.into(), CHOICE_ACTION.into(), RESOLVE_ACTION.into()];
        for beat in &script.beats {
            entries.push(beat.state.durable.clone());
            for offer in &beat.offers {
                entries.push(format!("{OFFER_PREFIX}{}", offer.id));
            }
        }
        entries
    })
}
fn terminal<'a>(
    state: &'a GameState,
    decision: &AcceptedDecision,
) -> Result<&'a GameFact, RepositoryError> {
    let fact = decision
        .facts
        .last()
        .and_then(|id| state.facts.iter().find(|f| f.id == *id))
        .ok_or(RepositoryError::InvalidCandidate)?;
    if fact.operation != decision.operation
        || fact.revision != decision.revision
        || decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
    {
        return invalid();
    }
    let index = state
        .facts
        .iter()
        .position(|f| f.id == fact.id)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let previous = index.checked_sub(1).and_then(|i| state.facts.get(i));
    if fact.cause != previous.map(|f| f.id)
        || (fact.ordinal > 0
            && previous.is_none_or(|f| {
                f.operation != fact.operation
                    || f.revision != fact.revision
                    || f.ordinal.checked_add(1) != Some(fact.ordinal)
                    || decision.facts.get(f.ordinal as usize) != Some(&f.id)
            }))
    {
        return invalid();
    }
    Ok(fact)
}
/// Reconstruct script progress from current accepted choices. A phase marker alone never
/// establishes a path. Repeated observations record separate operation decisions while
/// same-durable-beat transitions leave the existing completed-beat ledger unchanged.
pub(super) fn validate(
    current: &Checkpoint,
    script: &BeatScript,
) -> Result<rpc::JourneyPhase, RepositoryError> {
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let state = current.state();
    // A freeplay state may hold one encounter: the open fight of a hero's scene assault.
    if !state.encounters.is_empty()
        && !(script.mode == beat_script::ScriptMode::Freeplay
            && journey::scene_fight::encounters_admissible(current)?)
    {
        refusal_detail::note("scene-fight-encounter");
        return invalid();
    }
    if script.uses_encounter()
        || state.narrative.definition != model::content("room")?
        || !state.schedules.is_empty()
        || state.narrative.remaining_budget != pack::narrative_budget()
    {
        return invalid();
    }
    let mut beat = script
        .find(&script.entry)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let mut completed = Vec::new();
    let mut accepted = Vec::new();
    let mut started = false;
    let mut native_durable = "room";
    let mut creations = 0usize;
    let mut restart_seen = false;
    let mut observation_draws = 0usize;
    // Freeplay scene fights: whether one is open in the replay so far, and what they drew.
    let mut fight_open = false;
    let mut fight_draws = 0usize;
    let mut fought = false;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != journey::THREAD_POLICY {
            continue;
        }
        let terminal = terminal(state, decision)?;
        let receipt = journey::accepted(decision)?;
        if matches!(&terminal.value, FactValue::ContentEvent { definition, subjects }
            if definition.entry.as_str() == "start-new-adventure" && subjects.is_empty())
        {
            if restart_seen
                || beat.kind != BeatKind::Ending
                || !beat.offers.is_empty()
                || terminal.audience != AudienceScope::Shared
                || decision.facts.len() != 1
                || !decision.draws.is_empty()
                || !decision.effects.is_empty()
                || receipt
                    != (rpc::AcceptedAction {
                        phase: rpc::JourneyPhase::Complete as i32,
                        ..Default::default()
                    })
            {
                return invalid();
            }
            restart_seen = true;
            continue;
        }
        if script.mode == beat_script::ScriptMode::Freeplay
            && let FactValue::ContentEvent {
                definition,
                subjects,
            } = &terminal.value
            && let Some(step) = journey::scene_fight::replay_step(
                definition.entry.as_str(),
                receipt.phase,
                fight_open,
            )?
        {
            // A scene fight is an encounter inside the freeplay beat: its decisions record rules
            // outcomes, never a script choice, so they leave the beat and its ledgers as they were.
            if !subjects.is_empty()
                || terminal.audience != AudienceScope::Shared
                || choice(state, decision)?.is_some()
                || receipt.check.is_some()
                || receipt.scene.is_some()
                || receipt.character.is_some()
            {
                refusal_detail::note("scene-fight-replay");
                return invalid();
            }
            fought = true;
            fight_draws += decision.draws.len();
            fight_open = step != journey::scene_fight::Step::Closes;
            continue;
        }
        let Some((choice_fact, id, member)) = choice(state, decision)? else {
            // A fight changes a hero's combat resources (and a fight in progress adds weapon
            // rows to the sheet), so a fought scene reads each sheet as creation left it.
            let mut creation_state = if journey::scene_fight::fought(state) {
                let mut recreated = journey::recreated_heroes(state)?;
                recreated.encounters.clear();
                recreated
            } else {
                state.clone()
            };
            creation_state
                .decisions
                .retain(|record| record.revision <= decision.revision);
            let character = state
                .characters
                .get(creations)
                .ok_or(RepositoryError::InvalidCandidate)?;
            creations += 1;
            if started
                || receipt.phase != rpc::JourneyPhase::Room as i32
                || receipt.character
                    != Some(journey::character_sheet(&creation_state, character.entity)?)
                || receipt.check.is_some()
                || receipt.scene.is_some()
                || !receipt.combat.is_empty()
                || !matches!(&terminal.value,FactValue::ContentEvent{definition,subjects} if definition.entry.as_str()=="create-character" && subjects.is_empty())
                || terminal.audience != AudienceScope::Shared
            {
                return invalid();
            }
            continue;
        };
        let selected = beat
            .offers
            .iter()
            .find(|o| o.id == id)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let trigger = BeatTrigger::Offer { offer: id.into() };
        let next = beat
            .transitions
            .iter()
            .find(|t| t.trigger == trigger)
            .and_then(|t| script.find(&t.target))
            .ok_or(RepositoryError::InvalidCandidate)?;
        if receipt.phase != phase_of(next) as i32
            || !receipt.combat.is_empty()
            || receipt.character.is_some()
        {
            return invalid();
        }
        if selected.effect == BeatEffect::Observe {
            let source_scene = beat
                .scene_variant
                .as_ref()
                .and_then(|id| script.scenes.get(id))
                .cloned()
                .unwrap_or_else(|| pack::scene(beat.scene));
            let source_scope = beat
                .scene_variant
                .clone()
                .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase());
            if let Some(check) = receipt.check.as_ref() {
                validate_observation_check(
                    state,
                    decision,
                    member,
                    &source_scene,
                    &source_scope,
                    Some(check),
                )?;
                observation_draws += 1;
            }
        } else if receipt.check.is_some() {
            return invalid();
        }
        let native = match selected.effect {
            BeatEffect::BeginStory => {
                started = true;
                Some("begin-story")
            }
            BeatEffect::AskCourier => Some("private-courier-note"),
            BeatEffect::EscortCourier | BeatEffect::ThirdApproach => Some("escort-courier"),
            BeatEffect::Continue
            | BeatEffect::Observe
            | BeatEffect::Resolve
            | BeatEffect::ChooseHarborScene => None,
            _ => return invalid(),
        };
        let expected = native.unwrap_or(STEP);
        let private = selected.effect == BeatEffect::AskCourier;
        let scope = if private {
            AudienceScope::Members(vec![member])
        } else {
            AudienceScope::Shared
        };
        let actual_event = matches!(&terminal.value,FactValue::ContentEvent{definition,subjects} if definition.entry.as_str()==expected && subjects.is_empty());
        // A revoked/replaced private payload retains its recorded disclosure's historical
        // progress, exactly as the native courier source owner does. Rendering rechecks scope.
        let retained_private = private
            && state.intents.iter().any(|intent| {
                intent.operation == decision.operation
                    && intent.kind == EffectKind::RunAi
                    && intent.definition.entry.as_str() == super::courier_ai::DEFINITION
                    && decision.effects == [intent.id]
            });
        if private
            && matches!(&terminal.audience, AudienceScope::Members(_))
            && terminal.audience != scope
        {
            return invalid();
        }
        if (!actual_event || terminal.audience != scope) && !retained_private {
            return invalid();
        }
        if choice_fact.ordinal >= terminal.ordinal {
            return invalid();
        }
        if native.is_some() {
            accepted.push(terminal.id);
            native_durable = &next.state.durable;
        }
        if selected.effect == BeatEffect::ChooseHarborScene {
            let expected = destination_scene(next)?;
            if receipt.scene != Some(expected) {
                return invalid();
            }
        } else if receipt.scene.is_some() {
            return invalid();
        }
        if beat.state.durable != next.state.durable {
            let previous = model::content(&beat.state.durable)?;
            if !completed.contains(&previous) {
                completed.push(previous);
            }
        }
        beat = next;
    }
    // Every die is an observation check or a scene-interaction theft check; the scene replay
    // re-validates each theft draw against its receipt.
    // Time passes in a fight by whole rounds and in no other freeplay step.
    if state.draws.len()
        != observation_draws + fight_draws + journey::living_scene::interaction_draws(state)
        || fight_open != !state.encounters.is_empty()
        || (state.logical_time.ticks != 0
            && !(fought
                && state.logical_time.ticks_per_second == 1
                && state.logical_time.ticks % 6 == 0))
    {
        refusal_detail::note("scene-fight-accounting");
        return invalid();
    }
    if creations != state.characters.len()
        || script.current(state)?.id != beat.id
        || state.narrative.completed_beats != completed
        || state.narrative.accepted_facts != accepted
        || state.narrative.open_threads
            != if started && script.mode != beat_script::ScriptMode::Freeplay {
                vec![model::content("sealed-packet-delivery-thread")?]
            } else {
                Vec::new()
            }
    {
        return invalid();
    }
    if started && (journey::participants(current).len() != 2 || state.characters.len() != 2) {
        return invalid();
    }
    if script.mode != beat_script::ScriptMode::Freeplay {
        journey::validate_script_native_prefix(current, native_durable)?;
    }
    journey::validate_script_characters(state)?;
    if state
        .entities
        .iter()
        .any(|entity| entity.definition.entry.as_str() == "bandit")
    {
        return invalid();
    }
    Ok(journey::scene_fight::phase_after(state, phase_of(beat)))
}

fn finding_skill_name(skill: pack::FindingSkill) -> &'static str {
    match skill {
        pack::FindingSkill::Perception => "perception",
        pack::FindingSkill::Investigation => "investigation",
        pack::FindingSkill::Insight => "insight",
        pack::FindingSkill::Survival => "survival",
        pack::FindingSkill::Medicine => "medicine",
        pack::FindingSkill::Arcana => "arcana",
        pack::FindingSkill::History => "history",
        pack::FindingSkill::Nature => "nature",
        pack::FindingSkill::Religion => "religion",
    }
}

pub(super) fn observation_finding<'a>(
    scene: &'a pack::Scene,
    check: &rpc::AbilityCheckResult,
) -> Option<&'a pack::Finding> {
    if !check.succeeded {
        return None;
    }
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    let [_, _, skill, detail] = parts.as_slice() else {
        return None;
    };
    if detail.starts_with("target-") || detail.starts_with("repeat-target-") {
        if detail.starts_with("repeat-") {
            return None;
        }
        let target = super::investigation::check_target(scene, check)?;
        return super::investigation::finding_on(scene, &target, skill);
    }
    if let Some(id) = detail.strip_prefix("finding-") {
        return scene.findings.iter().find(|finding| finding.id == id);
    }
    if detail.starts_with("watch-") {
        return None;
    }
    let skill = match *skill {
        "perception" => Some(pack::FindingSkill::Perception),
        "investigation" => Some(pack::FindingSkill::Investigation),
        "insight" => Some(pack::FindingSkill::Insight),
        "survival" => Some(pack::FindingSkill::Survival),
        "medicine" => Some(pack::FindingSkill::Medicine),
        "arcana" => Some(pack::FindingSkill::Arcana),
        "history" => Some(pack::FindingSkill::History),
        "nature" => Some(pack::FindingSkill::Nature),
        "religion" => Some(pack::FindingSkill::Religion),
        "stealth" => None,
        _ => return None,
    }?;
    scene.findings.iter().find(|finding| finding.skill == skill)
}

fn validate_observation_check(
    state: &GameState,
    decision: &AcceptedDecision,
    member: MemberId,
    scene: &pack::Scene,
    scene_scope: &str,
    check: Option<&rpc::AbilityCheckResult>,
) -> Result<(), RepositoryError> {
    let mut source_scene = scene.clone();
    if let Some(snapshot) = journey::living_scene::snapshot(state, &source_scene)?
        && let Some(setting) = source_scene.setting.as_mut()
    {
        setting.model = Some(snapshot.model);
    }
    let scene = &source_scene;
    let invalid = || {
        refusal_detail::note("search-replay-check");
        RepositoryError::InvalidCandidate
    };
    let check = check.ok_or_else(invalid)?;
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != "srd521-p6" || parts[1] != scene_scope {
        return Err(invalid());
    }
    let skill = parts[2];
    let detail = parts[3];
    if !matches!(
        skill,
        "perception"
            | "investigation"
            | "insight"
            | "survival"
            | "medicine"
            | "arcana"
            | "history"
            | "nature"
            | "religion"
            | "stealth"
            | "sleight-of-hand"
    ) {
        return Err(invalid());
    }
    let selected_id = detail.strip_prefix("finding-");
    let selected =
        selected_id.and_then(|id| scene.findings.iter().find(|finding| finding.id == id));
    if selected_id.is_some() && selected.is_none() {
        return Err(invalid());
    }
    if let Some(finding) = selected
        && finding_skill_name(finding.skill) != skill
    {
        return Err(invalid());
    }
    if detail.starts_with("watch-")
        && (skill != "insight"
            || !scene
                .cast
                .iter()
                .any(|person| format!("watch-{}", person.id) == detail))
    {
        return Err(invalid());
    }
    let matching = selected.or_else(|| {
        if detail.starts_with("watch-") {
            return None;
        }
        scene
            .findings
            .iter()
            .find(|finding| finding_skill_name(finding.skill) == skill)
    });
    if decision.draws.len() != 1 {
        return Err(invalid());
    }
    let ordinal = decision.draws[0];
    let die = state
        .draws
        .iter()
        .find(|draw| draw.operation == decision.operation && draw.ordinal == ordinal)
        .filter(|draw| draw.sides == 20 && draw.value == check.die)
        .ok_or_else(invalid)?;
    let _ = die;
    let actor = state
        .members
        .iter()
        .find(|link| link.member == member)
        .and_then(|link| link.character)
        .ok_or_else(invalid)?;
    let (ability_modifier, proficiency_bonus) =
        skill_check_modifiers(state, actor, skill).map_err(|_| invalid())?;
    let total = i32::try_from(check.die).map_err(|_| invalid())?
        + i32::from(ability_modifier)
        + i32::from(proficiency_bonus);
    // SRD 5.2.1 p. 183: "you must succeed on a DC 15 Dexterity (Stealth) check" to Hide.
    let difficulty_class = if detail.starts_with("target-") || detail.starts_with("repeat-target-")
    {
        let target = super::investigation::check_target(scene, check).ok_or_else(invalid)?;
        super::investigation::difficulty_class(state, scene, &target, skill)?
    } else if skill == "stealth" {
        15
    } else {
        matching.map_or(10, |finding| finding.dc)
    };
    if !(1..=20).contains(&check.die)
        || check.ability_modifier != i32::from(ability_modifier)
        || check.proficiency_bonus != u32::from(proficiency_bonus)
        || check.total != total
        || check.difficulty_class != u32::from(difficulty_class)
        || check.succeeded != (total >= i32::from(difficulty_class))
    {
        return Err(invalid());
    }
    Ok(())
}
pub(super) const CHOICE_GROUP: &str = "beat-script-offer";
const RESOLVE_ID_PREFIX: &str = "journey-resolve-";
/// The opaque id of a fight ending's step, which names its authored offer and the revision it was
/// offered at. Authored (marker) scripts use their own ids.
pub(super) fn resolve_id(current: &Checkpoint) -> Option<String> {
    beat_script::with_script(|script| {
        if !script.is_canonical() {
            return None;
        }
        let offer = script
            .current(current.state())
            .ok()?
            .offers
            .iter()
            .find(|offer| offer.effect == BeatEffect::Resolve)?;
        Some(format!(
            "{RESOLVE_ID_PREFIX}{}-{}-{}",
            offer.id,
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence()
        ))
    })
}
fn resolve_offer_id(id: &str) -> Option<&str> {
    let rest = id.strip_prefix(RESOLVE_ID_PREFIX)?;
    let mut parts = rest.rsplitn(3, '-');
    let (_sequence, _epoch, offer) = (parts.next()?, parts.next()?, parts.next()?);
    Some(offer)
}
pub(super) fn submitted_id(text: &str) -> Option<&str> {
    text.strip_prefix("beat-")?
        .split_once(':')
        .map(|(id, _)| id)
}
fn id_for(current: &Checkpoint, offer: &BeatOffer) -> String {
    format!(
        "beat-{}:{}:{}",
        offer.id,
        current.basis().revision.epoch().get(),
        current.basis().revision.sequence()
    )
}
pub(super) fn select(
    current: &Checkpoint,
    action: rpc::GameplayActionKind,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
) -> Result<BeatOffer, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(current.state())?;
        // A freeplay scene fight has no fight beat to select from: its steps are the engine's own
        // tactical actions, which carry the effect of the attack, Second Wind or End Turn offer.
        if journey::scene_fight::open(current.state()) && journey::tactical::combat_action(action) {
            return journey::scene_fight::step_offer(action);
        }
        if journey::tactical::present(current.state()) && journey::tactical::combat_action(action) {
            let source = if beat.kind == BeatKind::Fight {
                beat
            } else {
                if action != rpc::GameplayActionKind::EndTurn
                    || !journey::tactical::dying(current.state())?
                {
                    return invalid();
                }
                script
                    .beats
                    .iter()
                    .find(|beat| beat.kind == BeatKind::Fight)
                    .ok_or(RepositoryError::InvalidCandidate)?
            };
            let source_kind = if matches!(
                action,
                rpc::GameplayActionKind::SecondWind | rpc::GameplayActionKind::EndTurn
            ) {
                action
            } else {
                rpc::GameplayActionKind::GreatswordAttack
            };
            return source
                .offers
                .iter()
                .find(|offer| offer.effect.action() == source_kind)
                .cloned()
                .ok_or(RepositoryError::InvalidCandidate);
        }
        let mut submitted = choices
            .iter()
            .filter(|(group, _)| group.as_str() == CHOICE_GROUP);
        let id = submitted.next().map(|(_, value)| value.as_str());
        if submitted.next().is_some()
            || (!script.is_canonical() && id.is_none())
            || (script.is_canonical() && id.is_some())
        {
            return invalid();
        }
        beat.offers
            .iter()
            .find(|offer| offer.effect.action() == action && id.is_none_or(|id| offer.id == id))
            .cloned()
            .ok_or(RepositoryError::InvalidCandidate)
    })
}
pub(super) fn matches_id(
    current: &Checkpoint,
    action: rpc::GameplayActionKind,
    id: &str,
) -> Result<bool, RepositoryError> {
    if action == rpc::GameplayActionKind::StartNewAdventure
        || action == rpc::GameplayActionKind::SceneAssault
    {
        return Ok(id == journey::offer_id(current, action));
    }
    beat_script::with_script(|script| {
        if script.is_canonical()
            || (journey::tactical::present(current.state())
                && journey::tactical::combat_action(action))
        {
            return Ok(id == journey::offer_id(current, action));
        }
        Ok(script
            .current(current.state())?
            .offers
            .iter()
            .any(|offer| offer.effect.action() == action && id_for(current, offer) == id))
    })
}
pub(super) fn view_offers(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String, String)>, RepositoryError> {
    let available = journey::offered(current, member)?;
    if ended(current.state())? {
        return Ok(available
            .into_iter()
            .map(|(kind, label)| (kind, label, journey::offer_id(current, kind)))
            .collect());
    }
    beat_script::with_script(|script| {
        let dying = journey::tactical::present(current.state())
            && journey::tactical::dying(current.state())?;
        if script.is_canonical()
            || (journey::tactical::present(current.state())
                && (journey::phase(current)? == rpc::JourneyPhase::Combat || dying))
        {
            let mut result = available
                .into_iter()
                .map(|(kind, label)| (kind, label, journey::offer_id(current, kind)))
                .collect::<Vec<_>>();
            // The ending steps of a fight are the script's own, offered once nobody is dying.
            if script.is_canonical() && !dying {
                for offer in &script.current(current.state())?.offers {
                    if offer.effect == BeatEffect::Resolve {
                        result.push((
                            offer.effect.action(),
                            script.label(offer)?,
                            journey::offer_id(current, offer.effect.action()),
                        ));
                    }
                }
            }
            return Ok(result);
        }
        let mut result = Vec::new();
        // A hero who died in a freeplay fight has nothing left to choose.
        if journey::player_entity(member, current)
            .is_ok_and(|who| journey::scene_fight::hero_dead(current.state(), who))
        {
            return Ok(result);
        }
        for offer in &script.current(current.state())?.offers {
            // Authored pure steps are the script's own; no handler affordance backs them.
            let pure = matches!(
                offer.effect,
                BeatEffect::Observe | BeatEffect::Resolve | BeatEffect::Continue
            );
            if pure
                || available
                    .iter()
                    .any(|(kind, _)| *kind == offer.effect.action())
            {
                result.push((
                    offer.effect.action(),
                    if offer.effect == BeatEffect::Observe {
                        "Look around".to_owned()
                    } else {
                        script.label(offer)?
                    },
                    id_for(current, offer),
                ));
            }
        }
        // A freeplay scene may offer to start a fight with any of its people who can be fought.
        if !journey::scene_fight::offered_targets(current, member)?.is_empty() {
            result.push((
                rpc::GameplayActionKind::SceneAssault,
                journey::scene_fight::OFFER_LABEL.to_owned(),
                journey::scene_fight::offer_id(current),
            ));
        }
        Ok(result)
    })
}
pub(super) fn is_observation_offer(
    current: &Checkpoint,
    id: &str,
) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return Ok(id
                == journey::offer_id(current, rpc::GameplayActionKind::ExamineHarborSeal)
                && script
                    .current(current.state())?
                    .offers
                    .iter()
                    .any(|offer| offer.effect == BeatEffect::Observe));
        }
        let Some(submitted) = submitted_id(id) else {
            return Ok(false);
        };
        Ok(script
            .current(current.state())?
            .offers
            .iter()
            .any(|offer| offer.id == submitted && offer.effect == BeatEffect::Observe))
    })
}
/// The shared display's labels for a conversation: the current beat's own authored offers, which
/// never depend on a private note's audience. `None` for the canonical script, which keeps its
/// handler label.
pub(super) fn dialogue_labels(
    current: &Checkpoint,
) -> Result<Option<Vec<String>>, RepositoryError> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return Ok(None);
        }
        let beat = script.current(current.state())?;
        let mut labels = Vec::new();
        for offer in &beat.offers {
            if offer.effect.action() != rpc::GameplayActionKind::CreateCharacter {
                labels.push(script.label(offer)?);
            }
        }
        Ok(Some(labels))
    })
}
/// The adventure's title for its end card: the opening scene's title, which names the errand.
pub(super) fn adventure_title(state: &GameState) -> Result<String, RepositoryError> {
    if !ended(state)? {
        return Ok(String::new());
    }
    Ok(pack::scene(pack::SceneKey::Opening).title)
}
/// The adventure's end card: an authored ending beat that offers nothing more.
pub(super) fn ended(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        // A script without an ending beat in its current state is not at an end card.
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.kind == BeatKind::Ending && beat.offers.is_empty())
    })
}
/// Whether the story is in its closing steps: the resolution's epilogue or the end card.
pub(super) fn closing(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.kind == BeatKind::Ending)
    })
}
/// Whether a lost fight has settled: nobody is still making death saves, so the party's next
/// choice (a rest, or closing the chapter) is a story choice and no longer part of the fight.
pub(super) fn settled_defeat(state: &GameState) -> Result<bool, RepositoryError> {
    beat_script::with_script(|script| {
        let Ok(beat) = script.current(state) else {
            return Ok(false);
        };
        Ok(beat.state.victory == Some(false)
            && journey::tactical::present(state)
            && !journey::tactical::dying(state)?)
    })
}
pub(super) fn action_label(
    id: &str,
    kind: rpc::GameplayActionKind,
) -> Result<String, RepositoryError> {
    if kind == rpc::GameplayActionKind::StartNewAdventure {
        return Ok("Start a new adventure".to_owned());
    }
    if kind == rpc::GameplayActionKind::SceneAssault {
        return Ok(journey::scene_fight::OFFER_LABEL.to_owned());
    }
    if journey::tactical::enabled() && journey::tactical::combat_action(kind) {
        return Ok(match kind {
            rpc::GameplayActionKind::GreatswordAttack => "Attack",
            rpc::GameplayActionKind::Move => "Move",
            rpc::GameplayActionKind::Dash => "Dash",
            rpc::GameplayActionKind::Dodge => "Dodge",
            rpc::GameplayActionKind::Disengage => "Disengage",
            rpc::GameplayActionKind::SecondWind => "Second Wind",
            rpc::GameplayActionKind::StabiliseWithHealersKit => "Stabilize with Healer's Kit",
            rpc::GameplayActionKind::StabiliseMedicine => "Stabilize with Wisdom (Medicine)",
            rpc::GameplayActionKind::EndTurn => "End turn",
            rpc::GameplayActionKind::OpportunityAttack => "Opportunity attack",
            rpc::GameplayActionKind::DeclineReaction => "Decline opportunity attack",
            rpc::GameplayActionKind::Stand => "Stand up",
            rpc::GameplayActionKind::PickUpWeapon => "Pick up weapon",
            rpc::GameplayActionKind::HelpAttack => "Help an ally attack",
            _ => return Err(RepositoryError::InvalidCandidate),
        }
        .to_owned());
    }
    beat_script::with_script(|script| {
        if script.is_canonical() {
            if kind == rpc::GameplayActionKind::ResolveErrand {
                let offer = resolve_offer_id(id)
                    .and_then(|offer_id| {
                        script
                            .beats
                            .iter()
                            .flat_map(|beat| &beat.offers)
                            .find(|offer| {
                                offer.id == offer_id && offer.effect == BeatEffect::Resolve
                            })
                    })
                    .ok_or(RepositoryError::InvalidCandidate)?;
                return script.label(offer);
            }
            if kind == rpc::GameplayActionKind::ExamineHarborSeal {
                return Ok("Look around".to_owned());
            }
            return Ok(pack::offer_label(kind));
        }
        let submitted = submitted_id(id).ok_or(RepositoryError::InvalidCandidate)?;
        let offer = script
            .beats
            .iter()
            .flat_map(|beat| &beat.offers)
            .find(|o| o.id == submitted && o.effect.action() == kind)
            .ok_or(RepositoryError::InvalidCandidate)?;
        script.label(offer)
    })
}

pub(super) fn advance_state(state: &mut GameState, next: &str) -> Result<(), RepositoryError> {
    for active in std::mem::take(&mut state.narrative.active_beats) {
        if !state.narrative.completed_beats.contains(&active) {
            state.narrative.completed_beats.push(active);
        }
    }
    state.narrative.active_beats = vec![model::content(next)?];
    Ok(())
}

/// A party with no living heroes cannot choose the authored steps that close its defeat.
pub(super) fn end_for_total_party_death(state: &mut GameState) -> Result<(), RepositoryError> {
    beat_script::with_script(|script| {
        let mut beat = script.current(state)?;
        for _ in 0..script.beats.len() {
            if script.terminal.contains(&beat.id) && beat.offers.is_empty() {
                return Ok(());
            }
            let mut resolutions = beat
                .offers
                .iter()
                .filter(|offer| offer.effect == BeatEffect::Resolve);
            let Some(resolve) = resolutions.next() else {
                return invalid();
            };
            if resolutions.next().is_some() {
                return invalid();
            }
            let Some(transition) = beat.transitions.iter().find(|transition| {
                transition.trigger
                    == (BeatTrigger::Offer {
                        offer: resolve.id.clone(),
                    })
            }) else {
                return invalid();
            };
            let Some(next) = script.find(&transition.target) else {
                return invalid();
            };
            let durable = next.state.durable.clone();
            advance_state(state, &durable)?;
            beat = script.current(state)?;
        }
        invalid()
    })
}

pub(super) fn first_action(
    current: &Checkpoint,
) -> Result<Option<(BeatEffect, MemberId)>, RepositoryError> {
    beat_script::with_script(|script| {
        for decision in &current.state().decisions {
            if let Some((_, id, member)) = choice(current.state(), decision)? {
                let effect = script
                    .beats
                    .iter()
                    .flat_map(|beat| &beat.offers)
                    .find(|o| o.id == id)
                    .map(|o| o.effect)
                    .ok_or(RepositoryError::InvalidCandidate)?;
                if effect != BeatEffect::BeginStory {
                    return Ok(Some((effect, member)));
                }
            }
        }
        Ok(None)
    })
}

pub(super) fn private_action(
    current: &Checkpoint,
) -> Result<Option<(BeatEffect, MemberId)>, RepositoryError> {
    beat_script::with_script(|script| {
        for decision in &current.state().decisions {
            if let Some((_, id, member)) = choice(current.state(), decision)? {
                let offer = script
                    .beats
                    .iter()
                    .flat_map(|beat| &beat.offers)
                    .find(|offer| offer.id == id)
                    .ok_or(RepositoryError::InvalidCandidate)?;
                if offer.effect == BeatEffect::AskCourier {
                    return Ok(Some((offer.effect, member)));
                }
            }
        }
        Ok(None)
    })
}

/// Native handlers qualify mechanical history; this independently qualifies every
/// authored choice and transition, including logical aliases of the same durable beat.
pub(super) fn validate_encounter_choices(
    current: &Checkpoint,
    script: &BeatScript,
) -> Result<(), RepositoryError> {
    if script.is_canonical() {
        return Ok(());
    }
    let mut beat = script
        .find(&script.entry)
        .ok_or(RepositoryError::InvalidCandidate)?;
    for decision in &current.state().decisions {
        let Some((_, id, member)) = choice(current.state(), decision)? else {
            continue;
        };
        let offer = beat
            .offers
            .iter()
            .find(|offer| offer.id == id)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if offer.effect == BeatEffect::ShortRest
            && !decision.facts.iter().any(|fact_id| {
                current.state().facts.iter().any(|fact| {
                    fact.id == *fact_id && matches!(fact.value, FactValue::TimeAdvanced { .. })
                })
            })
        {
            continue;
        }
        let receipt = journey::accepted(decision)?;
        if offer.effect == BeatEffect::Observe && receipt.check.is_some() {
            let source_scene = beat
                .scene_variant
                .as_ref()
                .and_then(|id| script.scenes.get(id))
                .cloned()
                .unwrap_or_else(|| pack::scene(beat.scene));
            let source_scope = beat
                .scene_variant
                .clone()
                .unwrap_or_else(|| format!("{:?}", beat.scene).to_lowercase());
            validate_observation_check(
                current.state(),
                decision,
                member,
                &source_scene,
                &source_scope,
                receipt.check.as_ref(),
            )?;
        } else if receipt.check.is_some() {
            refusal_detail::note("search-unexpected-check");
            return Err(RepositoryError::InvalidCandidate);
        }
        let trigger = if offer.effect == BeatEffect::ShortRest {
            BeatTrigger::RestComplete
        } else if beat.kind == BeatKind::Fight {
            let outcome = current.state().facts.iter().find_map(|fact| {
                if !decision.facts.contains(&fact.id) {
                    return None;
                }
                match &fact.value {
                    FactValue::ContentEvent { definition, .. }
                        if definition.entry.as_str() == "combat-victory" =>
                    {
                        Some(BeatTrigger::FightWon)
                    }
                    FactValue::ContentEvent { definition, .. }
                        if definition.entry.as_str() == "combat-defeat" =>
                    {
                        Some(BeatTrigger::FightLost)
                    }
                    _ => None,
                }
            });
            outcome.unwrap_or_else(|| BeatTrigger::Offer { offer: id.into() })
        } else {
            BeatTrigger::Offer { offer: id.into() }
        };
        beat = beat
            .transitions
            .iter()
            .find(|transition| transition.trigger == trigger)
            .and_then(|transition| script.find(&transition.target))
            .ok_or(RepositoryError::InvalidCandidate)?;
        if journey::accepted(decision)?.phase != phase_of(beat) as i32 {
            return invalid();
        }
    }
    if beat.id != script.current(current.state())?.id {
        return invalid();
    }
    Ok(())
}

/// Resolve the selected fight-start transition before the durable fight beat is installed.
pub(super) fn fight_scene(
    current: &Checkpoint,
    selected: &BeatOffer,
) -> Result<pack::Scene, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(current.state())?;
        let fight = beat
            .transitions
            .iter()
            .find(|transition| {
                transition.trigger
                    == BeatTrigger::Offer {
                        offer: selected.id.clone(),
                    }
            })
            .and_then(|transition| script.find(&transition.target))
            .filter(|beat| beat.kind == BeatKind::Fight)
            .ok_or(RepositoryError::InvalidCandidate)?;
        match &fight.scene_variant {
            Some(id) => script
                .scenes
                .get(id)
                .cloned()
                .ok_or(RepositoryError::InvalidCandidate),
            None => Ok(pack::scene(fight.scene)),
        }
    })
}
