//! `dev lore`: the NPC local canon of this run. Reads list each scene's established facts. An add
//! goes through the same proposal validator a reply's fact meets, and only an established fact is
//! saved and marks the run edited. Edit and delete answer unavailable: changing or removing canon
//! needs a mutator on the canon ledger that this surface does not add.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, failed, write_unavailable};
use crate::gameplay::{
    beat_script_runtime, journey,
    npc_reply::local_lore::{LocalFact, LocalLore, Names},
};

/// The speaker a dev-established fact names, so canon shows that this surface wrote it.
const DEV_SPEAKER: &str = "dev control";

/// The key of the scene being played, in the form the canon stores scenes under.
pub(super) fn current_scene_key(state: &Shared) -> Result<String, (StatusCode, Json<Value>)> {
    let checkpoint = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?;
    Ok(format!("{:?}", scene.key))
}

fn resolve_scene(state: &Shared, text: &str) -> Result<String, (StatusCode, Json<Value>)> {
    if text.eq_ignore_ascii_case("current") {
        current_scene_key(state)
    } else {
        Ok(text.to_owned())
    }
}

fn fact_json(fact: &LocalFact) -> Value {
    json!({
        "kind": fact.kind.label(),
        "name": fact.name,
        "content": fact.content,
        "established_by": fact.established_by,
    })
}

/// The names a new fact may not reuse, from the canonical checkpoint: every hero, and the scene's
/// people, things, offers and place, the same names the reply path hands its validator.
type NameLists = Result<(Vec<String>, Vec<String>), (StatusCode, Json<Value>)>;

fn name_lists(state: &Shared) -> NameLists {
    let checkpoint = state.service.updates.borrow().clone();
    let game = checkpoint.state();
    let scene =
        beat_script_runtime::scene(game).map_err(|_| failed("current scene unavailable"))?;
    let mut heroes = Vec::new();
    for character in &game.characters {
        heroes.push(
            journey::name(game, character.entity).map_err(|_| failed("hero name unavailable"))?,
        );
    }
    let mut known: Vec<String> = scene
        .cast
        .iter()
        .map(|person| person.name.clone())
        .collect();
    known.push(scene.title.clone());
    if let Some(model) = scene
        .setting
        .as_ref()
        .and_then(|setting| setting.model.as_ref())
    {
        known.extend(model.people.iter().map(|person| person.name.clone()));
        known.extend(model.things.iter().map(|thing| thing.name.clone()));
        known.extend(model.offers.iter().map(|offer| offer.name.clone()));
    }
    Ok((heroes, known))
}

/// `GET /dev/lore[?scene=<key|current>]`: the canon each scene holds, oldest fact first.
pub(super) async fn read(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let scene = match params.get("scene") {
        Some(text) => Some(resolve_scene(&state, text)?),
        None => None,
    };
    let scenes: Vec<Value> = state
        .lore
        .development_facts(scene.as_deref())
        .iter()
        .map(|(key, facts)| {
            json!({
                "scene": key,
                "count": facts.len(),
                "facts": facts.iter().map(fact_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok((
        StatusCode::OK,
        Json(json!({
            "scene_filter": scene,
            "scene_count": scenes.len(),
            "scenes": scenes,
        })),
    ))
}

/// The body of `POST /dev/lore`: one fact as its kind, name and one-sentence content.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddFact {
    scene: String,
    kind: String,
    name: String,
    content: String,
}

/// Proposes one fact through the reply validator and reports the outcome. Nothing is saved here:
/// the caller saves and marks the run edited only when the status is `added`.
async fn propose(
    lore: &LocalLore,
    model: &crate::gameplay::approach_model::ApproachModel,
    scene: &str,
    fact: &AddFact,
    names: &Names<'_>,
) -> Value {
    let text = format!("{}|{}|{}", fact.kind, fact.name, fact.content);
    match lore
        .propose_judged(model, scene, &text, DEV_SPEAKER, names)
        .await
    {
        Ok(Some(established)) => json!({
            "status": "added",
            "scene": scene,
            "fact": fact_json(&established),
            "edited": true,
        }),
        Ok(None) => refusal(scene, "malformed"),
        Err(rejection) => refusal(scene, rejection.label()),
    }
}

fn refusal(scene: &str, reason: &str) -> Value {
    json!({"status": "refused", "scene": scene, "reason": reason, "edited": false})
}

/// `POST /dev/lore {"scene", "kind", "name", "content"}`. The scene is `current` or a scene that
/// already holds canon, so a typo cannot open a new one.
pub(super) async fn add(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<AddFact>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let scene = resolve_scene(&state, &body.scene)?;
    let current = current_scene_key(&state)?;
    if scene != current && state.lore.development_facts(Some(&scene)).is_empty() {
        return Err(bad_request(format!(
            "scene {scene:?} is neither current nor holds canon; use scene \"current\""
        )));
    }
    let (heroes, known) = name_lists(&state)?;
    let names = Names {
        heroes: &heroes,
        game_state: &known,
    };
    let (_, approach_model) = state.service.npc_voice.hero_identity_models();
    let document = propose(&state.lore, approach_model, &scene, &body, &names).await;
    if document["status"] != "added" {
        return Ok((StatusCode::OK, Json(document)));
    }
    let lore = state.lore.clone();
    if !matches!(
        tokio::task::spawn_blocking(move || lore.save()).await,
        Ok(Ok(()))
    ) {
        return Err(failed(
            "local lore state unavailable; the fact is held in memory but not saved",
        ));
    }
    state.overrides.lock().await.push(json!({
        "kind": "lore-add",
        "scene": scene,
        "fact": document["fact"],
    }));
    Ok((StatusCode::OK, Json(document)))
}

/// `POST /dev/lore/edit`: canon is not changed in place by this surface.
pub(super) async fn unsupported_edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    write_unavailable(
        "lore",
        "an established fact cannot be edited here: changing canon needs a mutator on the canon ledger that this surface does not add",
    )
}

/// `POST /dev/lore/delete`: canon is not removed by this surface.
pub(super) async fn unsupported_delete(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    write_unavailable(
        "lore",
        "an established fact cannot be deleted here: removing canon needs a mutator on the canon ledger that this surface does not add",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::FakeEndpoint,
    };

    fn fact(kind: &str, name: &str, content: &str) -> AddFact {
        AddFact {
            scene: "scene-harbor".to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            content: content.to_owned(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dev_add_is_validated_like_a_reply_fact_and_refusals_change_nothing() {
        let lore = LocalLore::in_memory();
        let endpoint = FakeEndpoint::start(vec![answer("o1"), answer("o1"), answer("o2")]);
        let model = model(&endpoint);
        let heroes = vec!["Brynn".to_owned()];
        let known = vec!["Marek".to_owned(), "Mareks Cloth Shop".to_owned()];
        let names = Names {
            heroes: &heroes,
            game_state: &known,
        };

        let added = propose(
            &lore,
            &model,
            "scene-harbor",
            &fact(
                "place",
                "Salt Quay",
                "The salt quay runs along the harbour wall.",
            ),
            &names,
        )
        .await;
        assert_eq!(added["status"], "added");
        assert_eq!(added["edited"], true);
        assert_eq!(added["fact"]["established_by"], DEV_SPEAKER);

        let listed = lore.development_facts(Some("scene-harbor"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].1[0].name, "Salt Quay");

        let taken = propose(
            &lore,
            &model,
            "scene-harbor",
            &fact("custom", "salt quay", "Gulls wheel over the harbour wall."),
            &names,
        )
        .await;
        assert_eq!(taken["status"], "refused");
        assert_eq!(taken["reason"], "name_taken");
        assert_eq!(taken["edited"], false);

        let names_hero = propose(
            &lore,
            &model,
            "scene-harbor",
            &fact("person", "Brynn", "Brynn keeps the stall by the wall."),
            &names,
        )
        .await;
        assert_eq!(names_hero["reason"], "names_hero");

        let piped = propose(
            &lore,
            &model,
            "scene-harbor",
            &fact("custom", "Bells", "Bells ring | twice daily."),
            &names,
        )
        .await;
        assert_eq!(piped["reason"], "malformed");

        let claims_coin = propose(
            &lore,
            &model,
            "scene-harbor",
            &fact(
                "rumour",
                "Coin Lane",
                "They say the lane pays out two gold each morning.",
            ),
            &names,
        )
        .await;
        assert_eq!(claims_coin["reason"], "claims_coin_items_or_rules");

        let after = lore.development_facts(Some("scene-harbor"));
        assert_eq!(after[0].1.len(), 1, "refusals must not add or rename facts");
    }

    #[test]
    fn edit_and_delete_answer_unavailable_and_mark_nothing_edited() {
        let (status, Json(edit)) = match write_unavailable("lore", "reason") {
            Ok(answer) => answer,
            Err(error) => panic!("unavailable is an answer, not a failure: {error:?}"),
        };
        assert_eq!(status, StatusCode::OK);
        assert_eq!(edit["status"], "unavailable");
        assert_eq!(edit["surface"], "lore");
        assert_eq!(edit["edited"], false);
    }
}
