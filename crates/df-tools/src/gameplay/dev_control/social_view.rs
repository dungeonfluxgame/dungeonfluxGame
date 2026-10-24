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
        "patience": emotional.patience,
        "last_reaction": emotional.last_reaction,
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
    let mut id = super::reads::feelings_identity(scene, requested);
    let dossier = id
        .split_once(':')
        .and_then(|(role, person)| role_of(role).and_then(|role| social.dossier_of(role, person)));
    let person = scene
        .setting
        .as_ref()
        .and_then(|setting| setting.model.as_ref())
        .and_then(|model| {
            model.people.iter().find(|person| {
                person.id.eq_ignore_ascii_case(requested)
                    || person.name.eq_ignore_ascii_case(requested)
            })
        });
    if dossier.is_none() {
        let actor_id = person
            .map(|person| person.id.as_str())
            .or_else(|| {
                scene
                    .cast
                    .iter()
                    .find(|actor| {
                        actor.id.eq_ignore_ascii_case(requested)
                            || actor.name.eq_ignore_ascii_case(requested)
                    })
                    .map(|actor| actor.id.as_str())
            })
            .ok_or_else(|| bad_request(format!("NPC {requested:?} is not present")))?;
        id = crate::gameplay::npc_event_classifier::scene_person_binding(actor_id);
    }
    let feelings = social.development_feelings(&id);
    let profile = feelings.as_ref().map(|(profile, _)| profile);
    let emotions = feelings
        .as_ref()
        .map(|(_, emotions)| emotions.as_slice())
        .unwrap_or_default();
    let mut relationships = Vec::new();
    for character in &game.characters {
        let name =
            journey::name(game, character.entity).map_err(|_| failed("hero name unavailable"))?;
        if hero.is_some_and(|wanted| !name.eq_ignore_ascii_case(wanted)) {
            continue;
        }
        let member = character.owner;
        let standing = dossier.map(|dossier| social.standing(member, dossier));
        let key = member_hex(member);
        let emotional = emotions
            .iter()
            .find(|(id, _, _)| *id == key)
            .map(|(_, _, emotional)| emotional);
        relationships.push(json!({
            "hero": name,
            "details": social.development_relationship(member, &id),
            "acquaintance": social.acquaintance(&name, &id),
            "attitude": standing.map(|standing| standing.attitude.label()),
            "trust": standing.map(|standing| standing.trust),
            "emotion": emotional.map(emotional_json),
            "grievances": dossier.map(|dossier| social.grievance_lines(member, dossier)).unwrap_or_default(),
        }));
    }
    Ok(json!({
        "npc": requested,
        "npc_id": id,
        "profile": profile,
        "persona": person.and_then(|person| person.persona.as_ref()),
        "dossier": dossier,
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
    if let Some(document) = state
        .social
        .development_opening_social(
            &checkpoint,
            requested,
            params.get("hero").map(String::as_str),
        )
        .map_err(|_| failed("opening social standing unavailable"))?
    {
        let mut document = document;
        let npc_key = state
            .social
            .dossier_of(rpc::SceneRole::Antagonist, "bandit")
            .map(|dossier| dossier.npc_id.clone())
            .unwrap_or_else(|| {
                crate::gameplay::npc_event_classifier::scene_person_binding("bandit")
            });
        if let Some(rows) = document
            .get_mut("relationships")
            .and_then(Value::as_array_mut)
        {
            for row in rows {
                if let Some(hero) = row["hero"].as_str() {
                    row["acquaintance"] = json!(state.social.acquaintance(hero, &npc_key));
                }
            }
        }
        return Ok((StatusCode::OK, Json(document)));
    }
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
    fn opening_social_dev_read_dispatches_authored_dain_party_and_caused_exceptions() {
        use super::super::{Seats, history, layout, screen_preview};
        use crate::gameplay::{Service, model, npc_reply::NpcVoice};
        use df_rules::influence::{Attitude as RulesAttitude, Willingness};
        use tokio::sync::{Mutex, Semaphore, watch};
        let mut loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            include_bytes!("../fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        loaded
            .pack
            .scenes
            .iter_mut()
            .find(|scene| scene.key == pack::SceneKey::Opening)
            .unwrap()
            .setting
            .as_mut()
            .unwrap()
            .model
            .as_mut()
            .unwrap()
            .people
            .iter_mut()
            .find(|person| person.id == "bandit")
            .unwrap()
            .attitudes
            .insert(
                "Orla".to_owned(),
                crate::gameplay::adventure::scene_model::StartingAttitude::Hostile,
            );
        pack::with_test_pack(loaded, || {
            let opening = beat_script_runtime::opening_social_tests::opening(3);
            let member = opening.state().characters[0].owner;
            let mut eased = beat_script_runtime::stage_opening_social_appraised(
                &opening,
                df_types::OperationId::from_bytes(&[178; 16]).unwrap(),
                beat_script_runtime::OpeningSocialAttempt {
                    member,
                    skill: beat_script_runtime::OpeningSkill::Persuasion,
                    attitude: RulesAttitude::Indifferent,
                    willingness: Willingness::Hesitant,
                    faces: &[1],
                    changed_after: None,
                },
                &[],
                Some(RulesAttitude::Hostile),
            )
            .unwrap();
            beat_script_runtime::bind_opening_social_source(
                &mut eased,
                "I will regrind your chipped scimitar if Finn can pass.",
            )
            .unwrap();
            let current = eased.checkpoint;
            let before = current.basis();
            let (_, updates) = watch::channel(current.clone());
            let (actor, _inbox) = df_session::inbox::bounded_inbox();
            let social = SocialLedger::none();
            let table = crate::gameplay::table_log::TableLog::new();
            let layout = layout::LayoutState::new();
            let (shutdown, _) = watch::channel(false);
            let service = Service {
                look_ahead: Default::default(),
                portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
                actor,
                display_credential: [0x82; 32],
                updates,
                streams: Arc::new(Semaphore::new(2)),
                voice: crate::gameplay::voice::Voice::disabled(
                    crate::gameplay::startup_configuration::VoiceOff::Qualification,
                ),
                npc_voice: Arc::new(NpcVoice::silent()),
                table_log: table.clone(),
                social: social.clone(),
                living: crate::gameplay::living_scenes::LivingScenes::disabled(),
                layout: layout.clone(),
            };
            let state = Arc::new(Shared {
                service,
                display_credential: [0x82; 32],
                token: Arc::from("hs47-read-test"),
                screen_tickets: Arc::new(Mutex::new(screen_preview::Tickets::default())),
                seat_file: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../artifacts/tmp/lanes/HS47/unused-test-seats"),
                shutdown,
                seats: Arc::new(Mutex::new(Seats {
                    session_id: None,
                    run_id: None,
                    seats: Vec::new(),
                })),
                overrides: Arc::new(Mutex::new(Vec::new())),
                layout,
                recovery: df_persistence::NativeRecoverySource {
                    rules: vec![model::rule().unwrap(), journey::rule().unwrap()],
                    content: model::contents().unwrap(),
                    resources: journey::resources_for(current.state()).unwrap(),
                    resource_resolver: None,
                    asset_resolver: None,
                    assets: Vec::new(),
                    limits: model::limits(),
                },
                history: Arc::new(std::sync::Mutex::new(history::History::new())),
                table,
                social,
                notebook: crate::gameplay::notebook::NotebookLedger::none(),
                lore: Arc::new(crate::gameplay::npc_reply::local_lore::LocalLore::in_memory()),
            });
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                "Bearer hs47-read-test".parse().unwrap(),
            );
            let params = HashMap::from([("name".to_owned(), "Dain Rusk".to_owned())]);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let (status, Json(document)) = runtime
                .block_on(read(
                    State(state.clone()),
                    headers.clone(),
                    Query(params.clone()),
                ))
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!(document["party_attitude"], "indifferent");
            assert!(
                document["party_cause"]
                    .as_str()
                    .unwrap()
                    .contains("regrind")
            );
            assert_eq!(document["count"], 3);
            assert!(
                document["relationships"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["hero"] == "Orla"
                        && row["exception"] == true
                        && row["attitude"] == "hostile"
                        && row["cause"].as_str().unwrap().contains("Authored"))
            );
            let (_, Json(again)) = runtime
                .block_on(read(State(state.clone()), headers, Query(params)))
                .unwrap();
            assert_eq!(again, document);
            assert_eq!(state.service.updates.borrow().basis(), before);
        });
    }

    #[test]
    fn hs68_npc_show_hero_reports_acquaintance_and_the_public_source_without_a_dossier() {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, _) = record::reload(&documents).unwrap();
        let social = SocialLedger::none();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let game = opening.state();
            let scene = beat_script_runtime::scene(game).unwrap();
            let before = social_document(game, &social, &scene, "merchant", Some("Brynn")).unwrap();
            assert_eq!(before["count"], 1);
            assert_eq!(
                before["relationships"][0]["acquaintance"]["state"],
                "unknown"
            );
            social.learn_name(
                "Brynn",
                &["person:merchant".into()],
                &crate::gameplay::social::Acquaintance {
                    state: crate::gameplay::social::AcquaintanceState::Introduced,
                    speaker: "Brynn".into(),
                    place: "Basket Court".into(),
                    line: "I'm Brynn".into(),
                },
            );
            let after = social_document(game, &social, &scene, "merchant", Some("brynn")).unwrap();
            assert_eq!(
                after["relationships"][0]["acquaintance"]["state"],
                "introduced"
            );
            assert_eq!(
                after["relationships"][0]["acquaintance"]["line"],
                "I'm Brynn"
            );
            assert_eq!(
                after["relationships"][0]["acquaintance"]["place"],
                "Basket Court"
            );
        });
    }

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
