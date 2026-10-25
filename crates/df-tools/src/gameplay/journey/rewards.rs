//! Source-defined errand payment is committed with the won ending, never with an image or reply.
use super::*;

pub(super) const PAID: &str = "errand-reward-paid";

pub(in crate::gameplay) fn paid(state: &GameState) -> bool {
    state.facts.iter().any(|fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == PAID))
}

/// Public encounter total, derived from the canonical won ending, not a projected image.
/// SRD 5.2.1 p. 256: "XP is awarded for defeating the monster in combat or otherwise neutralizing it."
/// Character allocation and advancement remain a separately configured progression policy.
/// House decision: a freeplay scene assault awards no XP. Freeplay has no ending and its fight
/// closes its encounter in `scene_fight::close`, so the `ended` guard below returns `None` for it;
/// that must stay so.
pub(in crate::gameplay) fn encounter_experience(
    state: &GameState,
) -> Result<Option<u32>, RepositoryError> {
    if !super::super::beat_script_runtime::ended(state)?
        || !tactical::present(state)
        || !combat_victory(state)?
    {
        return Ok(None);
    }
    let tactical_battle = tactical::read(state)?;
    tactical_battle
        .experience_award()
        .map(|award| award.map(|award| award.total_xp))
        .map_err(|_| {
            refusal_detail::note("reward-experience");
            RepositoryError::InvalidCandidate
        })
}

pub(super) fn pay(state: &mut GameState) -> Result<Option<Vec<EntityId>>, RepositoryError> {
    if paid(state) || !super::super::beat_script_runtime::ended(state)? {
        return Ok(None);
    }
    // Assaulting a scene NPC is a separate objective. It must never satisfy
    // the courier delivery errand or award its generic payment.
    if state.encounters.iter().any(|encounter| {
        encounter
            .objectives
            .iter()
            .any(|objective| objective.entry.as_str() == "scene-npc-assault")
    }) {
        return Ok(None);
    }
    let Some(reward) = pack::with_content(|pack| pack.reward.clone()) else {
        return Ok(None);
    };
    if (!tactical::present(state) && state.encounters.is_empty()) || !combat_victory(state)? {
        return Ok(None);
    }
    let hero_entities = state
        .characters
        .iter()
        .map(|character| character.entity)
        .collect::<Vec<_>>();
    for hero_entity in &hero_entities {
        let gold_piece_item = state
            .inventory
            .iter_mut()
            .find(|item| item.owner == *hero_entity && item.origin.entry.as_str() == "gold-piece")
            .ok_or_else(|| {
                refusal_detail::note("reward-purse");
                RepositoryError::InvalidCandidate
            })?;
        gold_piece_item.quantity = gold_piece_item
            .quantity
            .checked_add(u64::from(reward.gold_per_hero))
            .ok_or_else(|| {
                refusal_detail::note("reward-purse-overflow");
                RepositoryError::InvalidCandidate
            })?;
    }
    Ok(Some(hero_entities))
}
