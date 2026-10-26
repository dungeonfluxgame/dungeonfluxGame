//! Detached inspection for roleplay tests. Unknown observations remain null.
use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::{beat_script_runtime, journey, social::SocialLedger};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

pub(super) async fn transcript(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let since = params
        .get("since")
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| bad_request("since must be an unsigned sequence"))?
        .unwrap_or(0);
    let current = state.service.updates.borrow().clone();
    let hero = match params.get("hero") {
        Some(requested) => {
            let name = canonical_hero(&state, &current, requested).await?;
            let hero_index = current
                .state()
                .characters
                .iter()
                .position(|character| {
                    journey::name(current.state(), character.entity)
                        .is_ok_and(|hero| hero.eq_ignore_ascii_case(&name))
                })
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(|| bad_request("hero index unavailable"))?;
            Some((name, hero_index))
        }
        None => None,
    };
    let entries = transcript_document(
        super::reads::transcript_timings(
            state.table.development_transcript(),
            current.basis().run.as_bytes(),
        ),
        since,
        hero.as_ref()
            .map(|(name, hero_index)| (name.as_str(), *hero_index)),
        params.get("npc").map(String::as_str),
    );
    Ok((StatusCode::OK, Json(entries)))
}
fn transcript_document(
    lines: Vec<Value>,
    since: u64,
    hero: Option<(&str, u32)>,
    npc: Option<&str>,
) -> Value {
    json!(
        lines
            .into_iter()
            .filter(|line| line["seq"].as_u64().is_some_and(|seq| seq > since))
            .filter(|line| hero.is_none_or(|(hero, hero_index)| {
                line["speaker"]
                    .as_str()
                    .is_some_and(|speaker| speaker.eq_ignore_ascii_case(hero))
                    || (line["hero_index"].as_u64() == Some(u64::from(hero_index))
                        && line["role"].as_str()
                            == Some(df_protocol::common::TableRole::Npc.as_str_name()))
            }))
            .filter(
                |line| npc.is_none_or(|npc| ["speaker", "actor_id"].iter().any(|key| line[*key]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(npc)
                        || name
                            .rsplit_once(':')
                            .is_some_and(|(_, id)| id.eq_ignore_ascii_case(npc)))))
            )
            .collect::<Vec<_>>()
    )
}
pub(super) async fn trade_terms(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let requested = params
        .get("hero")
        .ok_or_else(|| bad_request("trade-terms needs hero"))?;
    let current = state.service.updates.borrow().clone();
    let hero = canonical_hero(&state, &current, requested).await?;
    let terms = journey::living_scene::merchant_trade_terms(&current, &hero).map_err(|_| {
        bad_request("trade terms unavailable: hero must be present in a modeled place")
    })?;
    let npc = params.get("npc");
    let document = terms
        .into_iter()
        .filter_map(|term| {
            let (direction, merchant) = match term.direction {
                journey::living_scene::SceneTradeDirection::HeroBuys => ("buys", &term.seller_id),
                journey::living_scene::SceneTradeDirection::HeroSells => ("sells", &term.buyer_id),
            };
            if npc.is_some_and(|npc| {
                !merchant.eq_ignore_ascii_case(npc)
                    && !term.counterparty_name.eq_ignore_ascii_case(npc)
            }) {
                return None;
            }
            Some(json!({
                "direction": direction,
                "item": term.item_id,
                "item_name": term.item_name,
                "merchant": merchant,
                "merchant_name": term.counterparty_name,
                "min_cp": term.minimum_price_cp,
                "max_cp": term.maximum_price_cp,
                "available": term.available,
                "stock_rev": term.stock_revision,
                "purse_rev": term.purse_revision,
            }))
        })
        .collect::<Vec<_>>();
    Ok((StatusCode::OK, Json(json!(document))))
}
async fn canonical_hero(
    state: &Shared,
    current: &df_model::checkpoint::Checkpoint,
    requested: &str,
) -> Result<String, (StatusCode, Json<Value>)> {
    let seats = state.seats.lock().await;
    let seat_named = super::seat_index(requested).is_ok()
        || seats.seats.iter().any(|seat| {
            seat.hero
                .as_deref()
                .is_some_and(|hero| hero.eq_ignore_ascii_case(requested))
        });
    if seat_named {
        let index = super::resolve_hero(&seats, requested).map_err(bad_request)?;
        return seats.seats[index]
            .hero
            .clone()
            .ok_or_else(|| bad_request("hero has not been created"));
    }
    let mut matched_hero = None;
    for character in &current.state().characters {
        let name = journey::name(current.state(), character.entity)
            .map_err(|_| failed("hero name unavailable"))?;
        if name.eq_ignore_ascii_case(requested) {
            if matched_hero.is_some() {
                return Err(bad_request("hero name is ambiguous"));
            }
            matched_hero = Some(name);
        }
    }
    matched_hero.ok_or_else(|| bad_request("unknown hero"))
}
pub(super) async fn npc_show(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    super::social_view::read(State(state), headers, Query(params)).await
}
pub(super) async fn npc_last(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let npc = params
        .get("name")
        .ok_or_else(|| bad_request("npc last needs name"))?;
    let current = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(current.state())
        .map_err(|_| failed("current scene unavailable"))?;
    let npc_id = super::reads::feelings_identity(&scene, npc);
    if state.social.development_feelings(&npc_id).is_none() {
        return Err(bad_request("NPC has no profile here"));
    }
    if let Some(hero) = params.get("hero") {
        let hero = canonical_hero(&state, &current, hero).await?;
        let member = current
            .state()
            .characters
            .iter()
            .find_map(|character| {
                journey::name(current.state(), character.entity)
                    .ok()
                    .filter(|name| name.eq_ignore_ascii_case(&hero))
                    .map(|_| character.owner)
            })
            .ok_or_else(|| bad_request("hero member unavailable"))?;
        let recent = state.social.recent(member, &npc_id);
        return Ok((
            StatusCode::OK,
            Json(json!({"status":"unavailable","npc_id":npc_id,"hero":hero,
            "reason":"the stored global latest exchange has no hero correlation; recent is this hero's detached conversation memory",
            "recent":recent,"exchange":null,"edited":false})),
        ));
    }
    let mut query = HashMap::new();
    query.insert("npc".into(), npc.clone());
    let decisions = super::timeline::query(&query, "decisions", 1000, None, None, false)
        .map_err(bad_request)?;
    Ok((
        StatusCode::OK,
        Json(last_document(
            &state.social,
            &npc_id,
            decisions,
            params.get("hero").map(String::as_str),
        )),
    ))
}
fn last_document(
    social: &SocialLedger,
    npc_id: &str,
    decisions: Value,
    hero: Option<&str>,
) -> Value {
    let exchange = social
        .latest_exchange()
        .filter(|exchange| exchange.npc_id == npc_id);
    json!({"npc_id":npc_id,"hero_filter":hero,"exchange":exchange.map(|exchange| json!({
        "exchange_id":exchange.sequence,"approach":exchange.approach.label(),"effect":exchange.effect,
        "reaction":exchange.reaction,"emotion_after":exchange.emotional_state,"attitude":exchange.attitude.label()})),
        "decisions":decisions["decisions"],"hero_line":null,"line_route":null,"emotion_before":null,
        "checks":null,"judgement":null,"reply":null,"timings":null,
        "scope":"latest run exchange only; unavailable or uncorrelated observations are null; inspect npc show for per-hero recent lines"})
}
pub(super) async fn narrative(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let current = state.service.updates.borrow().clone();
    let narrative = &current.state().narrative;
    let phase = journey::phase(&current).map_err(|_| failed("phase unavailable"))?;
    let beats = narrative
        .active_beats
        .iter()
        .map(|beat| format!("{beat:?}"))
        .collect::<Vec<_>>();
    Ok((
        StatusCode::OK,
        Json(json!({"phase":super::phase_name(phase as i32),"beat":beats,
        "budget":narrative.remaining_budget,"spend":state.service.voice.development_text_spend(),"scope":"budget is remaining narrative units; spend is the shared text-token ledger when attached to enabled synthesis, otherwise null; no currency estimate"})),
    ))
}
pub(super) async fn present(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((
        StatusCode::OK,
        Json(
            json!({"presentation":state.layout.presentation_read().await.map_err(failed)?,"edited":false,"display_only":true}),
        ),
    ))
}
pub(super) async fn voice(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((
        StatusCode::OK,
        Json({
            let voice = state.service.voice.clone();
            tokio::task::spawn_blocking(move || voice.development_report())
                .await
                .map_err(|_| failed("voice inspection task failed"))?
        }),
    ))
}
pub(super) async fn mood(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let events = super::timeline::snapshot(4096, None, None, false);
    let mut scene_diagnostics = json!({
        "mood": null,
        "shot": null,
        "lighting": null,
        "reason": null,
        "faces": [],
        "scope": "last observed living-scene events; null before observation",
    });
    let mut faces = std::collections::BTreeMap::new();
    for event in events["events"].as_array().into_iter().flatten() {
        let event_name = event["event"].as_str().unwrap_or("");
        if !event_name.starts_with("gameplay.living_scene.") && event_name != "SCENE GRAPH" {
            continue;
        }
        for key in ["mood", "shot", "lighting", "reason"] {
            if !event["fields"][format!("scene.{key}")].is_null() {
                scene_diagnostics[key] = event["fields"][format!("scene.{key}")].clone();
            }
        }
        if event_name.ends_with("face_requested") {
            faces.insert(event["fields"]["scene.actor"].to_string(), event.clone());
        }
    }
    scene_diagnostics["faces"] = json!(faces.into_values().collect::<Vec<_>>());
    Ok((StatusCode::OK, Json(scene_diagnostics)))
}
/// Discoverable roleplay contracts. Null means unavailable observation, never a guessed value.
pub(super) fn commands() -> Vec<Value> {
    [
        ("transcript", "GET", "/dev/transcript", "[--since seq] [--hero name] [--npc name] [--out file]", "[{unix_ms,seq,place,speaker,role,text,exchange_id,reply_ms,voice{start,end},subtitle{shown,hidden}}]; bounded 4096 public lines; unobserved timings null; --out is a CLI-local JSON export returning {status:saved,file,count}"),
        ("npc show", "GET", "/dev/npc/show?name=<npc>&hero=<hero>", "<npc> [--hero name]", "{npc,npc_id,dossier,profile,count,relationships[{hero,attitude,trust,emotion{fond,afraid,drawn_to,resentful,wary,patience,never_forgives,last_reaction},details{scene,recent,voiced,emotional,influence_failed,try_on_consents,...},grievances}]}; unknown NPC HTTP 400; read never resets patience"),
        ("npc last", "GET", "/dev/npc/last?name=<npc>&hero=<hero>", "<npc> [--hero name]", "{npc_id,hero_filter,exchange{exchange_id,approach,effect,reaction,emotion_after,attitude}|null,decisions,hero_line,line_route,emotion_before,checks,judgement,reply,timings,scope}; uncorrelated observations null; --hero returns {status:unavailable,reason,recent,exchange:null,edited:false} because the global record has no hero binding; unknown NPC/hero HTTP 400"),
        ("trade-terms", "GET", "/dev/trade-terms?hero=<hero>&npc=<npc>", "<hero> [--npc merchant]", "[{direction:buys|sells,item,item_name,merchant,merchant_name,min_cp,max_cp,available,stock_rev,purse_rev}]; unknown hero or no modeled place HTTP 400; canonical owner terms, no new quote"),
        ("music", "GET", "/dev/music?place=<place|current|list>", "[place|list]", "{place,source,spec,set_key,label,fallback_bed,tracks[{track,status,detail}]} or {places[{place,source,label,set_key,status}]}; pure reads; absent director/spec -> {status:unavailable,reason,edited:false}"),
        ("music regenerate|clear|request|override|clear-override", "POST", "/dev/music/edit", "[place=current]; override --spec <pack music JSON>", "{status:accepted,op,place,edited:true}; absent director/spec unavailable; invalid spec HTTP 400; override and clear-override require DF_GAMEPLAY_DEV_OVERRIDES=on; explicit bounded director/cache operation"),
        ("voice", "GET", "/dev/voice", "", "[{speaker,voice_id,lines[{key,state:registered|cached|cache_unavailable|off,ms:null}]}]; actual chosen voices; synthesis and playback are not requested; duration is not observed by registry"),
        ("mood", "GET", "/dev/mood", "", "{mood,shot,lighting,reason,faces,scope}; last observed living-scene diagnostic values; null before observation"),
        ("narrative", "GET", "/dev/narrative", "", "{phase,beat:[active content refs],budget:remaining narrative units,spend:{unit:text_token,committed,reserved,scope}|null,scope}; spend reads the shared synthesis-attached ledger when available; no currency estimate"),
        ("present read", "GET", "/dev/presentation", "present (no args)", "{presentation{schema,revision,paused,steps,replays,speed,timing},edited:false,display_only:true}; pure read"),
        ("screen", "POST", "/dev/screens/ticket", "<1..6>", "{status:issued,path,expires_seconds}; joined seat required; one-use ticket expires in 60 seconds, token never audited; unavailable off loopback"),
        ("place travel", "POST", "/dev/place/go", "<destination words> or place go landmark:<name>", "{status:moved|refused,target,from,to,place,exit,created,line,narration,revision,edited}; travel goal is quoted data for existing owner movement path; unknown landmark HTTP 400, ordinary owner refusals preserve state"),
        ("wait --for", "LOCAL", "", "<1..86400 seconds>", "{status:waited,seconds}; wall-clock wait, does not advance game time; cannot combine --until"),
        ("timeline|audit|decisions cursor", "GET", "/dev/timeline|/dev/audit|/dev/decisions", "--since <seq> [--npc name] [--last 1..1000]", "existing events/decisions shape plus next_since,oldest_available,cursor_gap; filters before paging, cursor yields next page in ascending sequence; gap true after eviction; invalid cursor HTTP 400"),
    ].into_iter().map(|(command,method,path,args,output)| json!({"command":command,"method":method,"path":path,"args":args,"output":output})).collect()
}

pub(super) fn routes() -> axum::Router<Arc<Shared>> {
    use axum::routing::get;
    axum::Router::new()
        .route("/dev/transcript", get(transcript))
        .route("/dev/trade-terms", get(trade_terms))
        .route("/dev/npc/show", get(npc_show))
        .route("/dev/npc/last", get(npc_last))
        .route("/dev/narrative", get(narrative))
        .route("/dev/voice", get(voice))
        .route("/dev/mood", get(mood))
}

#[cfg(test)]
#[path = "roleplay_tests.rs"]
mod tests;
