//! Authenticated, bounded display layout controls used by the development CLI.
use axum::{extract::State as AxumState, http::HeaderMap, response::Json};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, watch};

use super::{Outcome, Shared, authorized, bad_request};

const MAX_COORDINATE: f64 = 4096.0;
const MAX_SCALE: f64 = 8.0;
const MAX_ROTATION: f64 = 360.0;

const KNOWN_IDS: &[&str] = &[
    "stage",
    "background",
    "plate",
    "scene-context",
    "headline",
    "rules",
    "narration",
    "choices",
    "title",
    "partybar",
    "hud",
    "item-card",
    "loader",
    "check-card",
    "overlay",
    "speech-bubbles",
    "images",
];

#[derive(Clone)]
pub(crate) struct LayoutState {
    state: Arc<Mutex<LayoutInner>>,
    readback: Arc<Mutex<Value>>,
    changed: watch::Sender<u64>,
}

struct LayoutInner {
    revision: u64,
    overrides: BTreeMap<String, Value>,
}

impl LayoutState {
    pub(crate) fn new() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            state: Arc::new(Mutex::new(LayoutInner {
                revision: 0,
                overrides: BTreeMap::new(),
            })),
            readback: Arc::new(Mutex::new(json!({"viewport": null, "elements": []}))),
            changed,
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(super) async fn snapshot(&self) -> Value {
        let state = self.state.lock().await;
        let readback = self.readback.lock().await.clone();
        json!({"revision": state.revision, "overrides": state.overrides, "readback": readback})
    }

    pub(crate) async fn attach(&self, message: &mut df_protocol::common::ViewMessage) {
        if let Some(df_protocol::common::view_message::Audience::Display(display)) =
            message.audience.as_mut()
        {
            let snapshot = self.snapshot().await;
            display.layout_revision = snapshot
                .get("revision")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            display.layout_json = snapshot.to_string();
        }
    }

    pub(crate) async fn readback(&self, text: &str) -> Result<(), String> {
        if text.len() > 64 * 1024 {
            return Err("layout readback is too large".into());
        }
        let value: Value =
            serde_json::from_str(text).map_err(|_| "layout readback is invalid JSON")?;
        let elements = value
            .get("elements")
            .and_then(Value::as_array)
            .ok_or_else(|| "layout readback needs elements".to_owned())?;
        if elements.len() > 256 {
            return Err("layout readback has too many elements".into());
        }
        for element in elements {
            let id = element
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "layout readback element needs id".to_owned())?;
            validate_id(id)?;
            for field in ["left", "top", "right", "bottom", "width", "height"] {
                let number = element
                    .get(field)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| format!("layout readback {field} must be finite"))?;
                if !number.is_finite() || number.abs() > MAX_COORDINATE * 4.0 {
                    return Err("layout readback element is outside its bound".into());
                }
            }
        }
        let reported_revision = value.get("revision").and_then(Value::as_u64).unwrap_or(0);
        let current_revision = self.state.lock().await.revision;
        let status = if reported_revision == current_revision
            && value.get("viewport").is_some()
            && value.get("elements").is_some()
        {
            "applied"
        } else {
            "stale"
        };
        *self.readback.lock().await = json!({
            "status": status,
            "reported_revision": reported_revision,
            "current_revision": current_revision,
            "payload": value,
        });
        Ok(())
    }

    async fn mutate(&self, id: &str, value: Value) -> Result<u64, String> {
        validate_id(id)?;
        let mut state = self.state.lock().await;
        state.overrides.insert(id.to_owned(), value);
        state.revision = state.revision.saturating_add(1);
        let revision = state.revision;
        let _ = self.changed.send_replace(revision);
        Ok(revision)
    }

    async fn reset(&self, id: Option<&str>) -> Result<u64, String> {
        if let Some(id) = id {
            validate_id(id)?;
        }
        let mut state = self.state.lock().await;
        if let Some(id) = id {
            state.overrides.remove(id);
        } else {
            state.overrides.clear();
        }
        state.revision = state.revision.saturating_add(1);
        let revision = state.revision;
        let _ = self.changed.send_replace(revision);
        Ok(revision)
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':')
    {
        return Err("layout id must be a bounded stable name".into());
    }
    if KNOWN_IDS.contains(&id)
        || id.starts_with("hero-")
        || id.starts_with("npc-")
        || id.starts_with("bubble-")
        || id.starts_with("loader-")
        || id.starts_with("card-")
        || id.starts_with("image-")
        || id.starts_with("hud-")
    {
        return Ok(());
    }
    Err(format!("unknown layout id {id:?}"))
}

fn finite(value: Option<&Value>, name: &str, limit: f64) -> Result<Option<f64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let number = value
        .as_f64()
        .ok_or_else(|| format!("{name} must be finite"))?;
    if !number.is_finite() || number.abs() > limit {
        return Err(format!("{name} is outside its bound"));
    }
    Ok(Some(number))
}

fn unit(body: &Value, name: &str) -> Result<&'static str, String> {
    match body.get(name).and_then(Value::as_str) {
        Some("px") => Ok("px"),
        Some("percent") => Ok("percent"),
        _ => Err(format!("{name} must be px or percent")),
    }
}

pub(super) async fn list(AxumState(state): AxumState<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((
        axum::http::StatusCode::OK,
        Json(state.layout.snapshot().await),
    ))
}

pub(super) async fn transform(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("layout transform needs id"))?;
    let tx = finite(body.get("translate_x"), "translate_x", MAX_COORDINATE).map_err(bad_request)?;
    let ty = finite(body.get("translate_y"), "translate_y", MAX_COORDINATE).map_err(bad_request)?;
    let sx = finite(
        body.get("scale_x").or_else(|| body.get("scale")),
        "scale_x",
        MAX_SCALE,
    )
    .map_err(bad_request)?;
    let sy = finite(
        body.get("scale_y").or_else(|| body.get("scale")),
        "scale_y",
        MAX_SCALE,
    )
    .map_err(bad_request)?;
    let rotate = finite(body.get("rotate"), "rotate", MAX_ROTATION).map_err(bad_request)?;
    let translate_unit = unit(&body, "unit").map_err(bad_request)?;
    if tx.is_none() && ty.is_none() && sx.is_none() && sy.is_none() && rotate.is_none() {
        return Err(bad_request("layout transform needs a transform value"));
    }
    let revision = state.layout.mutate(id, json!({"kind":"transform","translate_x":tx,"translate_y":ty,"unit":translate_unit,"scale_x":sx,"scale_y":sy,"rotate":rotate})).await.map_err(bad_request)?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status":"applied","id":id,"revision":revision})),
    ))
}

pub(super) async fn move_to(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("layout move needs id"))?;
    let left = finite(body.get("left"), "left", MAX_COORDINATE).map_err(bad_request)?;
    let top = finite(body.get("top"), "top", MAX_COORDINATE).map_err(bad_request)?;
    let width = finite(body.get("width"), "width", MAX_COORDINATE).map_err(bad_request)?;
    let height = finite(body.get("height"), "height", MAX_COORDINATE).map_err(bad_request)?;
    let position_unit = unit(&body, "unit").map_err(bad_request)?;
    if left.is_none() && top.is_none() && width.is_none() && height.is_none() {
        return Err(bad_request("layout move needs a position or size"));
    }
    let revision = state.layout.mutate(id, json!({"kind":"move","left":left,"top":top,"width":width,"height":height,"unit":position_unit})).await.map_err(bad_request)?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status":"applied","id":id,"revision":revision})),
    ))
}

pub(super) async fn reset(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let id = body.get("id").and_then(Value::as_str);
    let revision = state.layout.reset(id).await.map_err(bad_request)?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status":"reset","id":id,"revision":revision})),
    ))
}

#[cfg(test)]
mod tests {
    use super::{finite, validate_id};
    use serde_json::json;

    #[test]
    fn accepts_known_and_dynamic_visual_ids_but_rejects_selectors() {
        assert!(validate_id("background").is_ok());
        assert!(validate_id("bubble-seat-1-42").is_ok());
        assert!(validate_id("hero-seat-1").is_ok());
        assert!(validate_id(".tv-stage").is_err());
        assert!(validate_id("unknown").is_err());
    }

    #[test]
    fn rejects_non_finite_and_unbounded_values() {
        assert!(finite(Some(&json!(1.0)), "x", 2.0).unwrap().is_some());
        assert!(finite(Some(&json!(3.0)), "x", 2.0).is_err());
        assert!(finite(Some(&json!("NaN")), "x", 2.0).is_err());
    }
}
