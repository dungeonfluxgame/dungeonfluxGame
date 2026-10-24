//! Development reads and explicit requests for the installed place music director.
use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::place_music::{self, NoSpec, PlaceMusic};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

fn no_spec(reason: NoSpec) -> (StatusCode, Json<Value>) {
    let reason = match reason {
        NoSpec::UnknownPlace => "place has no decided music spec".to_owned(),
        NoSpec::Deciding => "place music spec is still being decided".to_owned(),
        NoSpec::Unavailable(reason) => reason,
    };
    (
        StatusCode::OK,
        Json(json!({"status":"unavailable", "reason":reason, "edited":false})),
    )
}
fn current_place(
    state: &Shared,
    requested: Option<&str>,
) -> Result<String, (StatusCode, Json<Value>)> {
    if let Some(place) = requested.filter(|place| *place != "current") {
        if place.trim().is_empty() {
            return Err(bad_request("music needs a place id or current"));
        }
        return Ok(place.to_owned());
    }
    let current = state.service.updates.borrow().clone();
    let scene = crate::gameplay::beat_script_runtime::scene(current.state())
        .map_err(|_| failed("current scene unavailable"))?;
    scene
        .setting
        .map(|setting| setting.place_id)
        .ok_or_else(|| bad_request("no current place"))
}
fn music_document(music: &PlaceMusic, place: &str) -> Result<Value, (StatusCode, Json<Value>)> {
    let spec = music.spec_report(place).map_err(no_spec)?;
    let tracks = music
        .track_status(place)
        .map_err(no_spec)?
        .into_iter()
        .map(|track| json!({"track":track.track,"status":track.status,"detail":track.detail}))
        .collect::<Vec<_>>();
    Ok(
        json!({"place":spec.place_id,"source":spec.source,"spec":spec.spec,"set_key":spec.set_key,
        "label":spec.label,"fallback_bed":spec.fallback_bed,"tracks":tracks,
        "band":music.band().label(),"variants":music.variants(place).map_err(no_spec)?,
        "ambient_layers":music.ambient_report(place)}),
    )
}
pub(super) async fn read(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let music = place_music::installed().ok_or_else(|| {
        no_spec(NoSpec::Unavailable(
            "place music director is not installed".into(),
        ))
    })?;
    if params.get("place").is_some_and(|place| place == "list") {
        let places = music
            .places()
            .into_iter()
            .map(|place| {
                json!({"place":place.place_id,"source":place.source,
            "label":place.label,"set_key":place.set_key,"status":place.status,
            "band":music.band().label(),"variants":music.variants(&place.place_id).ok(),
            "ambient_layers":music.ambient_report(&place.place_id)})
            })
            .collect::<Vec<_>>();
        return Ok((StatusCode::OK, Json(json!({"places":places}))));
    }
    let place = current_place(&state, params.get("place").map(String::as_str))?;
    Ok((StatusCode::OK, Json(music_document(music, &place)?)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditBody {
    op: String,
    place: String,
    #[serde(default)]
    spec: Option<Value>,
}
pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if !matches!(
        body.op.as_str(),
        "regenerate" | "clear" | "request" | "override" | "clear-override"
    ) {
        return Err(bad_request(
            "music edit takes regenerate|clear|request|override|clear-override",
        ));
    }
    if matches!(body.op.as_str(), "override" | "clear-override") {
        super::require_gameplay_overrides()?;
    }
    let music = place_music::installed().ok_or_else(|| {
        no_spec(NoSpec::Unavailable(
            "place music director is not installed".into(),
        ))
    })?;
    let place = current_place(&state, Some(&body.place))?;
    match body.op.as_str() {
        "regenerate" => music.regenerate(&place).map_err(no_spec)?,
        "clear" => music.clear(&place).map_err(no_spec)?,
        "request" => {
            music.request_now(&place).map_err(no_spec)?;
        }
        "override" => {
            let block = serde_json::from_value(
                body.spec
                    .ok_or_else(|| bad_request("music override needs spec JSON"))?,
            )
            .map_err(|_| bad_request("invalid pack music spec"))?;
            music
                .override_spec(&place, &block)
                .map_err(|_| bad_request("invalid pack music spec"))?;
        }
        "clear-override" => music.clear_override(&place),
        _ => return Err(bad_request("unknown music operation")),
    }
    crate::gameplay::actor::record_audit_line(
        "gameplay.dev.music",
        format!("DEV-MUSIC op={} place={place:?}", body.op),
    );
    let record = json!({"kind":"music", "op":body.op, "place":place});
    state.overrides.lock().await.push(record);
    state.table.settings_changed();
    Ok((
        StatusCode::OK,
        Json(json!({"status":"accepted", "op":body.op,"place":place,"edited":true})),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dev_control_music_read_reports_the_band_and_all_variants() {
        let music = PlaceMusic::new(
            crate::gameplay::approach_model::ApproachModel::default(),
            None,
            tokio::runtime::Handle::current(),
            || {},
        );
        let block: crate::gameplay::place_music::PackMusic = serde_json::from_value(json!({
            "place_id":"shop", "mood":"relaxed", "energy":"low", "setting":"market",
            "time_of_day":"day", "space":"interior", "palette":"light-acoustic"
        }))
        .unwrap();
        music.override_spec("shop", &block).unwrap();
        let read = music_document(&music, "shop").unwrap();
        assert_eq!(read["band"], "steady");
        assert_eq!(read["variants"].as_array().unwrap().len(), 4);
        assert_eq!(read["variants"][0]["bpm"], 63);
        assert_eq!(read["variants"][3]["bpm"], 126);
        assert_eq!(read["spec"]["mood"], "relaxed");
        assert_eq!(read["ambient_layers"]["status"], "not_requested");
        assert_eq!(read["ambient_layers"]["layers"], json!([]));
    }
    #[test]
    fn music_edit_body_rejects_unknown_fields_and_preserves_spec() {
        let body: EditBody =
            serde_json::from_value(json!({"op":"override","place":"shop","spec":{"bed":"warm"}}))
                .unwrap();
        assert!(body.spec.is_some());
        assert!(
            serde_json::from_value::<EditBody>(json!({"op":"clear","place":"shop","typo":1}))
                .is_err()
        );
        assert_eq!(no_spec(NoSpec::Deciding).1.0["edited"], false);
    }
}
