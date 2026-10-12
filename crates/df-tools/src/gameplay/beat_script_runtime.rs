//! Durable authored choices for scripts whose scenes need no encounter or new rules.
use super::{
    beat_script::{self, Beat, BeatEffect, BeatKind, BeatOffer, BeatScript, BeatTrigger},
    journey, model, pack,
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
pub(super) fn scene(state: &GameState) -> Result<pack::Scene, RepositoryError> {
    beat_script::with_script(|script| {
        let beat = script.current(state)?;
        let mut scene = match &beat.scene_variant {
            Some(variant) => script
                .scenes
                .get(variant)
                .cloned()
                .ok_or(RepositoryError::InvalidCandidate)?,
            None => pack::scene(beat.scene),
        };
        if let Some(variant) = &beat.scene_variant
            && let Some(asset) =
                pack::active().and_then(|loaded| loaded.script_scene_assets.get(variant).cloned())
        {
            scene.asset = asset;
        }
        Ok(scene)
    })
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
        Some(id) => script
            .scenes
            .get(id)
            .cloned()
            .ok_or(RepositoryError::InvalidCandidate),
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
        speaker_portrait: scene.speaker_portrait,
        cast: Vec::new(),
        rules_note: String::new(),
        place: String::new(),
        location_id: String::new(),
        live_backdrop: String::new(),
    })
}
pub(super) fn pure_target(
    current: &Checkpoint,
    selected: &BeatOffer,
) -> Result<Option<Beat>, RepositoryError> {
    beat_script::with_script(|script| {
        if script.uses_encounter()
            || !matches!(
                selected.effect,
                BeatEffect::Continue
                    | BeatEffect::Observe
                    | BeatEffect::Resolve
                    | BeatEffect::ChooseHarborScene
            )
        {
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
pub(super) fn contents() -> Vec<String> {
    beat_script::with_script(|script| {
        if script.is_canonical() {
            return Vec::new();
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
    if script.uses_encounter()
        || state.narrative.definition != model::content("room")?
        || !state.encounters.is_empty()
        || !state.draws.is_empty()
        || !state.schedules.is_empty()
        || state.logical_time.ticks != 0
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
    for decision in &state.decisions {
        if decision.source_policy.as_str() != journey::THREAD_POLICY {
            continue;
        }
        let terminal = terminal(state, decision)?;
        let receipt = journey::accepted(decision)?;
        let Some((choice_fact, id, member)) = choice(state, decision)? else {
            let character = state
                .characters
                .get(creations)
                .ok_or(RepositoryError::InvalidCandidate)?;
            creations += 1;
            if started
                || receipt.phase != rpc::JourneyPhase::Room as i32
                || receipt.character != Some(journey::character_sheet(state, character.entity)?)
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
            || receipt.check.is_some()
            || receipt.character.is_some()
        {
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
    if creations != state.characters.len()
        || script.current(state)?.id != beat.id
        || state.narrative.completed_beats != completed
        || state.narrative.accepted_facts != accepted
        || state.narrative.open_threads
            != if started {
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
    journey::validate_script_native_prefix(current, native_durable)?;
    journey::validate_script_characters(state)?;
    if state
        .entities
        .iter()
        .any(|entity| entity.definition.entry.as_str() == "bandit")
    {
        return invalid();
    }
    Ok(phase_of(beat))
}
pub(super) const CHOICE_GROUP: &str = "beat-script-offer";
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
    beat_script::with_script(|script| {
        if script.is_canonical()
            || (journey::tactical::present(current.state())
                && (journey::phase(current)? == rpc::JourneyPhase::Combat
                    || journey::tactical::dying(current.state())?))
        {
            return Ok(available
                .into_iter()
                .map(|(kind, label)| (kind, label, journey::offer_id(current, kind)))
                .collect());
        }
        let mut result = Vec::new();
        for offer in &script.current(current.state())?.offers {
            // Authored pure steps are the script's own; no handler affordance backs them.
            let pure = matches!(offer.effect, BeatEffect::Resolve | BeatEffect::Continue);
            if pure
                || available
                    .iter()
                    .any(|(kind, _)| *kind == offer.effect.action())
            {
                result.push((
                    offer.effect.action(),
                    script.label(offer)?,
                    id_for(current, offer),
                ));
            }
        }
        Ok(result)
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
pub(super) fn action_label(
    id: &str,
    kind: rpc::GameplayActionKind,
) -> Result<String, RepositoryError> {
    if journey::tactical::enabled() && journey::tactical::combat_action(kind) {
        return Ok(match kind {
            rpc::GameplayActionKind::GreatswordAttack => "Attack",
            rpc::GameplayActionKind::Move => "Move",
            rpc::GameplayActionKind::Dash => "Dash",
            rpc::GameplayActionKind::Dodge => "Dodge",
            rpc::GameplayActionKind::Disengage => "Disengage",
            rpc::GameplayActionKind::SecondWind => "Second Wind",
            rpc::GameplayActionKind::EndTurn => "End turn",
            rpc::GameplayActionKind::OpportunityAttack => "Opportunity attack",
            rpc::GameplayActionKind::DeclineReaction => "Decline opportunity attack",
            rpc::GameplayActionKind::Stand => "Stand up",
            rpc::GameplayActionKind::PickUpWeapon => "Pick up weapon",
            _ => return Err(RepositoryError::InvalidCandidate),
        }
        .to_owned());
    }
    beat_script::with_script(|script| {
        if script.is_canonical() {
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
        let Some((_, id, _)) = choice(current.state(), decision)? else {
            continue;
        };
        let offer = beat
            .offers
            .iter()
            .find(|offer| offer.id == id)
            .ok_or(RepositoryError::InvalidCandidate)?;
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
