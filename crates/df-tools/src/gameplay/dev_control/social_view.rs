//! `dev social`: one NPC's standing with each hero, read from the social ledger. Only the ledger's
//! plain reads are used (`development_feelings`, `standing`, `grievance_lines`); the ledger's
//! `emotional_state` begins a scene as a side effect, so it is never called from here.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::{
    adventure::dossier::{Attitude, Dossier},
    beat_script_runtime, journey,
    pack::Scene,
    social::{EmotionalState, SocialError, SocialLedger},
};
use df_types::MemberId;

fn role_of(label: &str) -> Option<rpc::SceneRole> {
    match label {
        "patron" => Some(rpc::SceneRole::Patron),
        "contact" => Some(rpc::SceneRole::Contact),
        "antagonist" => Some(rpc::SceneRole::Antagonist),
        "bystander" => Some(rpc::SceneRole::Bystander),
        _ => None,
    }
}

/// The ledger keys a member's emotional record by the lowercase hex of the member bytes.
fn member_hex(member: MemberId) -> String {
    member
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn emotional_json(emotional: &EmotionalState) -> Value {
    json!({
        "fond": emotional.fond.label(),
        "afraid": emotional.afraid.label(),
        "drawn_to": emotional.drawn_to.label(),
        "resentful": emotional.resentful.label(),
        "wary": emotional.wary.label(),
        "never_forgives": emotional.never_forgives,
    })
}

/// The standing document for one NPC of `scene`, with one entry per hero (or only `hero`).
pub(super) fn social_document(
    game: &df_model::checkpoint::GameState,
    social: &SocialLedger,
    scene: &Scene,
    requested: &str,
    hero: Option<&str>,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let id = super::reads::feelings_identity(scene, requested);
    let (profile, emotions) = social
        .development_feelings(&id)
        .ok_or_else(|| bad_request(format!("NPC {requested:?} has no social profile here")))?;
    let (role_label, person_id) = id
        .split_once(':')
        .ok_or_else(|| bad_request(format!("NPC {requested:?} has no social profile here")))?;
    let role = role_of(role_label).ok_or_else(|| bad_request("NPC role unknown"))?;
    let dossier = social
        .dossier_of(role, person_id)
        .ok_or_else(|| bad_request("NPC profile not found"))?;
    let mut relationships = Vec::new();
    for character in &game.characters {
        let name =
            journey::name(game, character.entity).map_err(|_| failed("hero name unavailable"))?;
        if hero.is_some_and(|wanted| !name.eq_ignore_ascii_case(wanted)) {
            continue;
        }
        let member = character.owner;
        let standing = social.standing(member, dossier);
        let key = member_hex(member);
        let emotional = emotions
            .iter()
            .find(|(id, _, _)| *id == key)
            .map(|(_, _, emotional)| emotional);
        relationships.push(json!({
            "hero": name,
            "attitude": standing.attitude.label(),
            "trust": standing.trust,
            "emotion": emotional.map(emotional_json),
            "grievances": social.grievance_lines(member, dossier),
        }));
    }
    Ok(json!({
        "npc": requested,
        "npc_id": id,
        "profile": profile,
        "count": relationships.len(),
        "relationships": relationships,
    }))
}

/// `GET /dev/social?name=<npc>[&hero=<name>]`: attitude, trust, emotional state, the NPC's
/// never-forgives mark and every grievance with the hero's words that caused it.
pub(super) async fn read(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let requested = params
        .get("name")
        .ok_or_else(|| bad_request("social needs ?name=<npc>"))?;
    let checkpoint = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?;
    let document = social_document(
        checkpoint.state(),
        &state.social,
        &scene,
        requested,
        params.get("hero").map(String::as_str),
    )?;
    Ok((StatusCode::OK, Json(document)))
}

/// The body of `POST /dev/social/edit`: one NPC's standing with one hero, set by `attitude` or
/// `trust`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditBody {
    npc: String,
    hero: String,
    op: String,
    value: String,
}

fn attitude_of(text: &str) -> Option<Attitude> {
    match text.trim().to_ascii_lowercase().as_str() {
        "friendly" => Some(Attitude::Friendly),
        "indifferent" => Some(Attitude::Indifferent),
        "hostile" => Some(Attitude::Hostile),
        _ => None,
    }
}

fn standing_label(attitude: Attitude, trust: i8) -> String {
    format!("{} trust {trust}", attitude.label())
}

fn dossier_for<'a>(
    social: &'a SocialLedger,
    scene: &Scene,
    requested: &str,
) -> Result<&'a Dossier, (StatusCode, Json<Value>)> {
    let id = super::reads::feelings_identity(scene, requested);
    let (role_label, person_id) = id
        .split_once(':')
        .ok_or_else(|| bad_request(format!("NPC {requested:?} has no social profile here")))?;
    let role = role_of(role_label).ok_or_else(|| bad_request("NPC role unknown"))?;
    social
        .dossier_of(role, person_id)
        .ok_or_else(|| bad_request("NPC profile not found"))
}

fn member_of(
    game: &df_model::checkpoint::GameState,
    hero: &str,
) -> Result<MemberId, (StatusCode, Json<Value>)> {
    for character in &game.characters {
        let name =
            journey::name(game, character.entity).map_err(|_| failed("hero name unavailable"))?;
        if name.eq_ignore_ascii_case(hero) {
            return Ok(character.owner);
        }
    }
    Err(bad_request(format!("unknown hero {hero:?}")))
}

/// `POST /dev/social/edit {"npc", "hero", "op": "attitude"|"trust", "value"}`: sets one hero's
/// standing with one NPC. A value the dossier cannot hold is refused and changes nothing. An
/// applied edit is saved with the ledger, audited, and marks the run edited like an override.
pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let (attitude, trust) = match body.op.as_str() {
        "attitude" => (
            Some(
                attitude_of(&body.value)
                    .ok_or_else(|| bad_request("attitude is friendly, indifferent or hostile"))?,
            ),
            None,
        ),
        "trust" => (
            None,
            Some(
                body.value
                    .trim()
                    .parse::<i8>()
                    .map_err(|_| bad_request("trust is a whole number"))?,
            ),
        ),
        _ => return Err(bad_request("social edit takes attitude or trust")),
    };
    let checkpoint = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?;
    let dossier = dossier_for(&state.social, &scene, &body.npc)?;
    let member = member_of(checkpoint.state(), &body.hero)?;
    let before = state.social.standing(member, dossier);
    let after = match state
        .social
        .dev_set_standing(member, dossier, attitude, trust)
    {
        Ok(after) => after,
        Err(SocialError::Invalid(reason)) => {
            crate::gameplay::actor::record_audit_line(
                "gameplay.dev.social_edit",
                format!(
                    "DEV-SOCIAL edit status=refused npc={:?} reason={reason:?}",
                    body.npc
                ),
            );
            return Ok((
                StatusCode::OK,
                Json(json!({"status": "refused", "reason": reason, "edited": false})),
            ));
        }
        Err(_) => return Err(failed("social standing could not be changed")),
    };
    let ledger = state.social.clone();
    match tokio::task::spawn_blocking(move || ledger.save()).await {
        Ok(Ok(())) => {}
        _ => {
            return Err(failed(
                "standing changed but the social ledger could not be saved",
            ));
        }
    }
    let before_label = standing_label(before.attitude, before.trust);
    let after_label = standing_label(after.attitude, after.trust);
    crate::gameplay::actor::record_audit_line(
        "gameplay.dev.social_edit",
        format!(
            "DEV-SOCIAL edit status=edited npc={:?} hero={:?} before={before_label:?} after={after_label:?}",
            body.npc, body.hero
        ),
    );
    state.overrides.lock().await.push(json!({
        "kind": "social",
        "target": body.npc,
        "hero": body.hero,
        "before": before_label,
        "after": after_label,
    }));
    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "edited",
            "npc": body.npc,
            "hero": body.hero,
            "before": before_label,
            "after": after_label,
            "edited": true,
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_variety_tests as native, pack,
        social::SocialLedger,
        startup_configuration::ContentRating,
    };

    #[test]
    fn social_read_reports_each_hero_standing_with_the_npc_and_no_grievance_until_one_is_made() {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, record) = record::reload(&documents).unwrap();
        let social = SocialLedger::open(record.dossiers, &record.seed, None).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let game = opening.state();
            let scene = beat_script_runtime::scene(game).unwrap();
            let dossier = social
                .dossier_of(rpc::SceneRole::Patron, "merchant")
                .expect("the cloth merchant has a dossier");

            let before = social_document(game, &social, &scene, "merchant", None).unwrap();
            assert_eq!(before["npc_id"], "patron:merchant");
            assert_eq!(before["count"], game.characters.len());
            let brynn = &before["relationships"][0];
            assert_eq!(brynn["hero"], "Brynn");
            assert_eq!(brynn["attitude"], dossier.starting_attitude.label());
            assert!(brynn["grievances"].as_array().unwrap().is_empty());

            let only = social_document(game, &social, &scene, "merchant", Some("brynn")).unwrap();
            assert_eq!(only["count"], 1);

            let member = game.characters[0].owner;
            let _shift = social.record_assault(member, dossier, "shop", "I hit you.");
            let after = social_document(game, &social, &scene, "merchant", Some("Brynn")).unwrap();
            let standing = &after["relationships"][0];
            assert_eq!(
                standing["attitude"], "hostile",
                "an assault leaves the NPC hostile"
            );
            let grievances = standing["grievances"].as_array().unwrap();
            assert_eq!(
                grievances.len(),
                1,
                "a recorded wrong is listed with its cause"
            );
            assert!(grievances[0].as_str().unwrap().contains("I hit you."));

            let reread = social_document(game, &social, &scene, "merchant", Some("Brynn")).unwrap();
            assert_eq!(reread, after, "a read changes nothing");
        });
    }
}
