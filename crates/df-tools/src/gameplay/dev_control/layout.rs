//! Authenticated layout state. One mutex makes revision, configuration and acknowledgments atomic.
use super::{Outcome, Shared, authorized, bad_request};
use crate::layout_contract::{
    self as contract, ElementStatus, LayoutDocument, LayoutError, LayoutPage, LayoutReadback,
    MoveRequest, ResetRequest, Transform, TransformRequest, VisibilityRequest,
};
use axum::{
    extract::{Query, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, watch};
const MAX_CLIENTS: usize = 8;
const INVENTORY_LIFETIME: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(crate) struct LayoutState {
    state: Arc<Mutex<LayoutInner>>,
    pub(super) camera: Arc<Mutex<super::camera::CameraState>>,
    enabled: Arc<AtomicBool>,
    changed: watch::Sender<u64>,
}
struct LayoutInner {
    document: LayoutDocument,
    presentation: super::presentation::PresentationState,
    clients: BTreeMap<u64, ClientReadback>,
    assemblies: BTreeMap<u64, Assembly>,
}
struct ClientReadback {
    report: LayoutReadback,
    last_readback_at: Instant,
}
const ASSEMBLY_LIFETIME: Duration = Duration::from_secs(10);
struct Assembly {
    started: Instant,
    sample: u64,
    revision: u64,
    viewport: contract::Viewport,
    overflow: usize,
    parts: Vec<Option<Vec<contract::ElementReadback>>>,
    received: usize,
    bytes: usize,
}
impl LayoutState {
    pub(crate) fn new() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            camera: Arc::new(Mutex::new(super::camera::CameraState::default())),
            state: Arc::new(Mutex::new(LayoutInner {
                document: LayoutDocument::default(),
                presentation: super::presentation::PresentationState::default(),
                clients: BTreeMap::new(),
                assemblies: BTreeMap::new(),
            })),
            enabled: Arc::new(AtomicBool::new(false)),
            changed,
        }
    }
    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }
    pub(super) fn camera_changed(&self) {
        self.changed
            .send_modify(|revision| *revision = revision.saturating_add(1));
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub(crate) async fn attach(
        &self,
        message: &mut df_protocol::common::ViewMessage,
    ) -> Result<(), LayoutError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(df_protocol::common::view_message::Audience::Display(display)) =
            message.audience.as_mut()
        {
            let state = self.state.lock().await;
            display.camera_override = Some(self.camera.lock().await.command.clone());
            display.layout_revision = state.document.revision;
            let mut document =
                serde_json::to_value(&state.document).map_err(|_| LayoutError::Shape)?;
            document["development_presentation"] =
                serde_json::to_value(&state.presentation).map_err(|_| LayoutError::Shape)?;
            display.layout_json =
                serde_json::to_string(&document).map_err(|_| LayoutError::Shape)?;
            if display.layout_json.len() > contract::MAX_DOCUMENT_BYTES {
                return Err(LayoutError::Limit);
            }
        }
        Ok(())
    }
    pub(super) async fn presentation_read(&self) -> Result<Value, String> {
        serde_json::to_value(&self.state.lock().await.presentation)
            .map_err(|_| "presentation serialization failed".to_owned())
    }

    pub(super) async fn presentation(&self, body: &Value) -> Result<Value, String> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err("development controls are disabled".to_owned());
        }
        let mut state = self.state.lock().await;
        state.presentation.apply(body)?;
        state.document.revision = state
            .document
            .revision
            .checked_add(1)
            .ok_or("layout revision exhausted")?;
        self.changed.send_replace(state.document.revision);
        serde_json::to_value(&state.presentation)
            .map_err(|_| "presentation serialization failed".to_owned())
    }
    async fn snapshot(&self, id: Option<&str>) -> Result<Value, LayoutError> {
        if let Some(id) = id {
            contract::validate_id(id)?;
        }
        let state = self.state.lock().await;
        let clients:Vec<Value>=state.clients.iter().map(|(instance,client)| {
            let elements:Vec<_>=client.report.elements.iter().filter(|element|id.is_none_or(|id|id==element.id)).collect();
            let status=if client.report.revision!=state.document.revision||client.last_readback_at.elapsed()>INVENTORY_LIFETIME { "stale" }
                else if id.is_some()&&elements.is_empty()||elements.iter().any(|e|e.status==ElementStatus::Absent) { "absent" } else { "applied" };
            json!({"display_instance":instance,"status":status,"reported_revision":client.report.revision,"sample":client.report.sample,"viewport":client.report.viewport,"overflow":client.report.overflow,"elements":elements})
        }).collect();
        if let Some(id) = id
            && !state.document.overrides.contains_key(id)
            && !state.document.visibility.contains_key(id)
            && !state
                .clients
                .values()
                .any(|client| client.report.elements.iter().any(|e| e.id == id))
        {
            return Err(LayoutError::UnknownId);
        }
        let pending_pages:Vec<_>=state.assemblies.iter().map(|(instance,a)|json!({"display_instance":instance,"revision":a.revision,"sample":a.sample,"received":a.received,"pages":a.parts.len(),"status":if a.started.elapsed()<=ASSEMBLY_LIFETIME{"receiving"}else{"expired"}})).collect();
        let overrides: BTreeMap<_, _> = state
            .document
            .overrides
            .iter()
            .filter(|(key, _)| id.is_none_or(|id| id == key.as_str()))
            .collect();
        let visibility: BTreeMap<_, _> = state
            .document
            .visibility
            .iter()
            .filter(|(key, _)| id.is_none_or(|id| id == key.as_str()))
            .collect();
        Ok(
            json!({"revision":state.document.revision,"status":if clients.is_empty(){"pending"}else if clients.iter().all(|c|c["status"]=="applied"){"applied"}else{"pending"},"overrides":overrides,"visibility":visibility,"clients":clients,"capacity":contract::MAX_ELEMENTS,"pending_pages":pending_pages}),
        )
    }
    pub(crate) async fn readback(&self, instance: u64, text: &str) -> Result<(), LayoutError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(LayoutError::Unavailable);
        }
        if instance == 0 {
            return Err(LayoutError::Id);
        }
        if text.starts_with("{\"battle_camera\":") {
            return self.camera.lock().await.readback(instance, text);
        }
        let page = LayoutPage::parse(text)?;
        let report = {
            let mut state = self.state.lock().await;
            state
                .assemblies
                .retain(|_, a| a.started.elapsed() <= ASSEMBLY_LIFETIME);
            state
                .clients
                .retain(|_, c| c.last_readback_at.elapsed() <= INVENTORY_LIFETIME);
            if page.revision > state.document.revision {
                return Err(LayoutError::Revision);
            }
            if let Some(previous) = state.clients.get(&instance) {
                if page.sample == previous.report.sample
                    && page.revision == previous.report.revision
                    && page.viewport == previous.report.viewport
                    && page.overflow == previous.report.overflow
                    && page
                        .elements
                        .iter()
                        .all(|e| previous.report.elements.contains(e))
                {
                    return Ok(());
                }
                if page.sample <= previous.report.sample || page.revision < previous.report.revision
                {
                    return Err(LayoutError::Revision);
                }
            }
            if state
                .assemblies
                .get(&instance)
                .is_none_or(|a| a.sample != page.sample)
            {
                if page.page != 0
                    || state
                        .assemblies
                        .get(&instance)
                        .is_some_and(|a| page.sample <= a.sample)
                {
                    return Err(LayoutError::Revision);
                }
                let clients: std::collections::BTreeSet<_> = state
                    .clients
                    .keys()
                    .chain(state.assemblies.keys())
                    .copied()
                    .collect();
                if !clients.contains(&instance) && clients.len() >= MAX_CLIENTS {
                    return Err(LayoutError::Limit);
                }
                state.assemblies.insert(
                    instance,
                    Assembly {
                        started: Instant::now(),
                        sample: page.sample,
                        revision: page.revision,
                        viewport: page.viewport.clone(),
                        overflow: page.overflow,
                        parts: vec![None; page.pages],
                        received: 0,
                        bytes: 0,
                    },
                );
            }
            let assembly = state
                .assemblies
                .get_mut(&instance)
                .ok_or(LayoutError::Revision)?;
            if assembly.revision != page.revision
                || assembly.parts.len() != page.pages
                || assembly.viewport != page.viewport
                || assembly.overflow != page.overflow
            {
                return Err(LayoutError::Shape);
            }
            let part = assembly
                .parts
                .get_mut(page.page)
                .ok_or(LayoutError::Shape)?;
            if let Some(previous) = part {
                if *previous != page.elements {
                    return Err(LayoutError::Shape);
                }
            } else {
                if assembly.bytes + text.len() > contract::MAX_READBACK_BYTES {
                    return Err(LayoutError::Limit);
                }
                assembly.bytes += text.len();
                assembly.received += 1;
                *part = Some(page.elements);
            }
            if assembly.received != assembly.parts.len() {
                return Ok(());
            }
            let assembly = state
                .assemblies
                .remove(&instance)
                .ok_or(LayoutError::Revision)?;
            let report = LayoutReadback {
                sample: assembly.sample,
                revision: assembly.revision,
                viewport: assembly.viewport,
                overflow: assembly.overflow,
                elements: assembly.parts.into_iter().flatten().flatten().collect(),
            };
            report.validate()?;
            report
        };
        let mut state = self.state.lock().await;
        if report.revision > state.document.revision {
            return Err(LayoutError::Revision);
        }
        if let Some(previous) = state.clients.get(&instance)
            && (report.sample <= previous.report.sample
                || report.revision < previous.report.revision)
        {
            return Err(LayoutError::Revision);
        }
        if report.revision == state.document.revision {
            for element in &report.elements {
                if element.configured.as_ref() != state.document.overrides.get(&element.id)
                    || element.visibility
                        != state
                            .document
                            .visibility
                            .get(&element.id)
                            .copied()
                            .unwrap_or_default()
                {
                    return Err(LayoutError::Shape);
                }
            }
            for id in state.document.overrides.keys() {
                if !report.elements.iter().any(|e| e.id == *id) {
                    return Err(LayoutError::Shape);
                }
            }
            for id in state.document.visibility.keys() {
                if !report.elements.iter().any(|e| e.id == *id) {
                    return Err(LayoutError::Shape);
                }
            }
        }
        state
            .clients
            .retain(|_, client| client.last_readback_at.elapsed() <= INVENTORY_LIFETIME);
        if !state.clients.contains_key(&instance) && state.clients.len() >= MAX_CLIENTS {
            return Err(LayoutError::Limit);
        }
        let revision = report.revision;
        let current = state.document.revision;
        let visibility_changes: Vec<_> = state
            .clients
            .get(&instance)
            .map(|previous| {
                report
                    .elements
                    .iter()
                    .filter_map(|element| {
                        let old = previous
                            .report
                            .elements
                            .iter()
                            .find(|candidate| candidate.id == element.id)
                            .map(|candidate| candidate.visibility);
                        (old.is_some() && old != Some(element.visibility))
                            .then_some((element.id.clone(), element.visibility))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let changed = state.clients.get(&instance).is_none_or(|old| {
            old.report.revision != revision
                || old
                    .report
                    .elements
                    .iter()
                    .map(|e| (&e.id, e.status, &e.configured, e.visibility))
                    .collect::<Vec<_>>()
                    != report
                        .elements
                        .iter()
                        .map(|e| (&e.id, e.status, &e.configured, e.visibility))
                        .collect::<Vec<_>>()
        });
        let applied: Vec<_> = report
            .elements
            .iter()
            .filter(|e| e.configured.is_some() && e.status == ElementStatus::Applied)
            .map(|e| e.id.clone())
            .collect();
        let absent: Vec<_> = report
            .elements
            .iter()
            .filter(|e| e.status == ElementStatus::Absent)
            .map(|e| e.id.clone())
            .collect();
        state.clients.insert(
            instance,
            ClientReadback {
                report,
                last_readback_at: Instant::now(),
            },
        );
        drop(state);
        if changed {
            if revision != current {
                event("gameplay.layout.readback", revision, None, "stale");
            } else {
                if applied.is_empty() && absent.is_empty() {
                    event("gameplay.layout.applied", revision, None, "applied");
                }
                for id in applied {
                    event("gameplay.layout.applied", revision, Some(&id), "applied");
                }
                for id in absent {
                    event("gameplay.layout.absent", revision, Some(&id), "absent");
                }
                for (id, visibility) in visibility_changes {
                    event(
                        "gameplay.layout.visibility",
                        revision,
                        Some(&id),
                        match visibility {
                            contract::ElementVisibility::Hidden => "hidden",
                            contract::ElementVisibility::Visible => "visible",
                        },
                    );
                }
            }
        }
        Ok(())
    }
    async fn mutate(&self, id: &str, patch: Transform) -> Result<u64, LayoutError> {
        contract::validate_id(id)?;
        patch.validate()?;
        let mut state = self.state.lock().await;
        let admitted = state.clients.values().any(|client| {
            client.last_readback_at.elapsed() <= INVENTORY_LIFETIME
                && client
                    .report
                    .elements
                    .iter()
                    .any(|e| e.id == id && e.status == ElementStatus::Applied)
        });
        if !admitted {
            return Err(LayoutError::UnknownId);
        }
        if !state.document.overrides.contains_key(id)
            && state.document.overrides.len() >= contract::MAX_OVERRIDES
        {
            return Err(LayoutError::Limit);
        }
        let revision = state
            .document
            .revision
            .checked_add(1)
            .ok_or(LayoutError::Limit)?;
        state
            .document
            .overrides
            .entry(id.to_owned())
            .or_default()
            .merge(patch);
        state.document.revision = revision;
        self.changed.send_replace(revision);
        drop(state);
        event("gameplay.layout.accepted", revision, Some(id), "pending");
        Ok(revision)
    }
    async fn reset(&self, id: Option<&str>) -> Result<u64, LayoutError> {
        if let Some(id) = id {
            contract::validate_id(id)?;
        }
        let mut state = self.state.lock().await;
        if let Some(id) = id
            && !state.document.overrides.contains_key(id)
            && !state.document.visibility.contains_key(id)
            && !state.clients.values().any(|c| {
                c.last_readback_at.elapsed() <= INVENTORY_LIFETIME
                    && c.report.elements.iter().any(|e| e.id == id)
            })
        {
            return Err(LayoutError::UnknownId);
        }
        let revision = state
            .document
            .revision
            .checked_add(1)
            .ok_or(LayoutError::Limit)?;
        if let Some(id) = id {
            state.document.overrides.remove(id);
            state.document.visibility.remove(id);
        } else {
            state.document.overrides.clear();
            state.document.visibility.clear();
        }
        state.document.revision = revision;
        self.changed.send_replace(revision);
        drop(state);
        event("gameplay.layout.reset", revision, id, "pending");
        Ok(revision)
    }
    async fn set_visibility(&self, body: &VisibilityRequest) -> Result<u64, LayoutError> {
        contract::validate_id(&body.id)?;
        let mut state = self.state.lock().await;
        let admitted = state.clients.values().any(|client| {
            client.last_readback_at.elapsed() <= INVENTORY_LIFETIME
                && client
                    .report
                    .elements
                    .iter()
                    .any(|e| e.id == body.id && e.status == ElementStatus::Applied)
        });
        if !admitted {
            return Err(LayoutError::UnknownId);
        }
        if !state.document.visibility.contains_key(&body.id)
            && state.document.visibility.len() >= contract::MAX_OVERRIDES
        {
            return Err(LayoutError::Limit);
        }
        let revision = state
            .document
            .revision
            .checked_add(1)
            .ok_or(LayoutError::Limit)?;
        if body.visible {
            state.document.visibility.remove(&body.id);
        } else {
            state
                .document
                .visibility
                .insert(body.id.clone(), contract::ElementVisibility::Hidden);
        }
        state.document.revision = revision;
        self.changed.send_replace(revision);
        drop(state);
        event(
            if body.visible {
                "gameplay.layout.shown"
            } else {
                "gameplay.layout.hidden"
            },
            revision,
            Some(&body.id),
            "pending",
        );
        Ok(revision)
    }
}
fn event(name: &'static str, revision: u64, id: Option<&str>, status: &str) {
    let fields = serde_json::Map::from_iter([
        ("layout_revision".to_owned(), json!(revision)),
        ("target_id".to_owned(), json!(id)),
        ("status".to_owned(), json!(status)),
    ]);
    crate::gameplay::actor::record_audit_diagnostic(
        name,
        format!(
            "LAYOUT revision={revision} target={} status={status}",
            id.unwrap_or("all")
        ),
        fields,
        &[
            ("layout.revision", revision.to_string()),
            ("layout.status", status.to_owned()),
        ],
    );
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    id: Option<String>,
}
pub(super) async fn list(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Query(selection): Query<Selection>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((
        StatusCode::OK,
        Json(
            state
                .layout
                .snapshot(selection.id.as_deref())
                .await
                .map_err(|e| bad_request(e.to_string()))?,
        ),
    ))
}
pub(super) async fn transform(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<TransformRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let patch = body.patch().map_err(|e| bad_request(e.to_string()))?;
    let revision = state
        .layout
        .mutate(&body.id, patch)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok((
        StatusCode::OK,
        Json(json!({"status":"pending","id":body.id,"revision":revision})),
    ))
}
pub(super) async fn move_to(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<MoveRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let patch = body.patch().map_err(|e| bad_request(e.to_string()))?;
    let revision = state
        .layout
        .mutate(&body.id, patch)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok((
        StatusCode::OK,
        Json(json!({"status":"pending","id":body.id,"revision":revision})),
    ))
}
pub(super) async fn reset(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ResetRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let revision = state
        .layout
        .reset(body.id.as_deref())
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok((
        StatusCode::OK,
        Json(json!({"status":"pending","id":body.id,"revision":revision})),
    ))
}
pub(super) async fn visibility(
    AxumState(state): AxumState<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<VisibilityRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let revision = state
        .layout
        .set_visibility(&body)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "pending",
            "id": body.id,
            "visibility": if body.visible { "visible" } else { "hidden" },
            "revision": revision
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_contract::{Bounds, EffectiveTransform, ElementReadback, Viewport};
    fn report(
        revision: u64,
        sample: u64,
        id: &str,
        configured: Option<Transform>,
    ) -> LayoutReadback {
        LayoutReadback {
            sample,
            revision,
            viewport: Viewport {
                width: 1920.0,
                height: 1080.0,
            },
            overflow: 0,
            elements: vec![ElementReadback {
                id: id.into(),
                tag: "div".into(),
                classes: String::new(),
                owner: None,
                configured,
                effective: Some(EffectiveTransform {
                    translate: "none".into(),
                    scale: "none".into(),
                    rotate: "none".into(),
                    matrix: "none".into(),
                }),
                rect: Some(Bounds {
                    left: 0.0,
                    top: 0.0,
                    width: 100.0,
                    height: 100.0,
                }),
                status: ElementStatus::Applied,
                visibility: contract::ElementVisibility::Visible,
            }],
        }
    }
    async fn admit(
        state: &LayoutState,
        instance: u64,
        report: &LayoutReadback,
    ) -> Result<(), LayoutError> {
        for page in report.pages()? {
            state
                .readback(instance, &serde_json::to_string(&page).unwrap())
                .await?;
        }
        Ok(())
    }
    #[tokio::test]
    async fn development_presentation_wakes_and_attaches_without_a_game_revision_or_player_data() {
        let state = LayoutState::new();
        state.enable();
        let mut wake = state.subscribe();
        let mut message = df_protocol::common::ViewMessage {
            audience: Some(df_protocol::common::view_message::Audience::Display(
                Default::default(),
            )),
            ..Default::default()
        };
        state.attach(&mut message).await.unwrap();
        let Some(df_protocol::common::view_message::Audience::Display(initial)) =
            message.audience.as_ref()
        else {
            panic!("display")
        };
        let initial: Value = serde_json::from_str(&initial.layout_json).unwrap();
        assert_eq!(initial["development_presentation"]["revision"], 0);
        state.presentation(&json!({"op":"step"})).await.unwrap();
        assert!(wake.has_changed().unwrap());
        wake.borrow_and_update();
        state.attach(&mut message).await.unwrap();
        let Some(df_protocol::common::view_message::Audience::Display(display)) =
            message.audience.as_ref()
        else {
            panic!("display")
        };
        let document: Value = serde_json::from_str(&display.layout_json).unwrap();
        assert_eq!(document["development_presentation"]["steps"], 1);
        assert_eq!(document["development_presentation"]["paused"], true);
        assert!(document["development_presentation"].get("heroes").is_none());
        assert!(document["development_presentation"].get("rules").is_none());
    }
    #[tokio::test]
    async fn inventory_admission_partial_updates_and_exact_revision_ack() {
        let state = LayoutState::new();
        state.enable();
        let mut wake = state.subscribe();
        assert_eq!(
            state.mutate("hero-made-up", Transform::default()).await,
            Err(LayoutError::UnknownId)
        );
        admit(&state, 1, &report(0, 1, "dm-overlay", None))
            .await
            .unwrap();
        assert_eq!(
            state
                .mutate(
                    "dm-overlay",
                    Transform {
                        rotate: Some(10.0),
                        ..Default::default()
                    }
                )
                .await
                .unwrap(),
            1
        );
        assert_eq!(*wake.borrow_and_update(), 1);
        assert_eq!(
            state.snapshot(None).await.unwrap()["clients"][0]["status"],
            "stale"
        );
        state
            .mutate(
                "dm-overlay",
                Transform {
                    scale_x: Some(2.0),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let configured = state.state.lock().await.document.overrides["dm-overlay"].clone();
        assert_eq!(configured.rotate, Some(10.0));
        assert_eq!(
            admit(&state, 1, &report(2, 2, "dm-overlay", None)).await,
            Err(LayoutError::Shape)
        );
        admit(&state, 1, &report(2, 3, "dm-overlay", Some(configured)))
            .await
            .unwrap();
        assert_eq!(state.snapshot(None).await.unwrap()["status"], "applied");
        assert_eq!(
            admit(&state, 1, &report(0, 2, "dm-overlay", None)).await,
            Err(LayoutError::Revision)
        );
        state.reset(Some("dm-overlay")).await.unwrap();
        assert_eq!(state.snapshot(None).await.unwrap()["status"], "pending");
        admit(&state, 2, &report(3, 1, "dm-overlay", None))
            .await
            .unwrap();
        assert_eq!(
            state.snapshot(None).await.unwrap()["clients"][1]["status"],
            "applied"
        );
    }
    #[tokio::test]
    async fn visibility_round_trips_without_resetting_transforms() {
        let state = LayoutState::new();
        state.enable();
        admit(&state, 1, &report(0, 1, "dm-overlay", None))
            .await
            .unwrap();
        state
            .mutate(
                "dm-overlay",
                Transform {
                    translate_y: Some(crate::layout_contract::Distance {
                        value: 24.0,
                        unit: crate::layout_contract::Unit::Px,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let configured = Transform {
            translate_y: Some(crate::layout_contract::Distance {
                value: 24.0,
                unit: crate::layout_contract::Unit::Px,
            }),
            ..Default::default()
        };
        let hidden_revision = state
            .set_visibility(&VisibilityRequest {
                id: "dm-overlay".into(),
                visible: false,
            })
            .await
            .unwrap();
        let mut hidden = report(hidden_revision, 2, "dm-overlay", Some(configured.clone()));
        hidden.elements[0].visibility = contract::ElementVisibility::Hidden;
        admit(&state, 1, &hidden).await.unwrap();
        let snapshot = state.snapshot(Some("dm-overlay")).await.unwrap();
        assert_eq!(snapshot["visibility"]["dm-overlay"], "hidden");
        assert_eq!(
            snapshot["overrides"]["dm-overlay"]["translate_y"]["value"],
            24.0
        );
        let shown_revision = state
            .set_visibility(&VisibilityRequest {
                id: "dm-overlay".into(),
                visible: true,
            })
            .await
            .unwrap();
        let shown = report(shown_revision, 3, "dm-overlay", Some(configured));
        admit(&state, 1, &shown).await.unwrap();
        let snapshot = state.snapshot(Some("dm-overlay")).await.unwrap();
        assert_eq!(snapshot["visibility"], serde_json::json!({}));
        assert_eq!(
            snapshot["overrides"]["dm-overlay"]["translate_y"]["value"],
            24.0
        );
        assert_eq!(snapshot["status"], "applied");
    }
    #[tokio::test]
    async fn expired_inventory_cannot_admit_visibility_until_exact_readback_refreshes() {
        let state = LayoutState::new();
        state.enable();
        admit(&state, 1, &report(0, 1, "dm-overlay", None))
            .await
            .unwrap();

        state
            .state
            .lock()
            .await
            .clients
            .get_mut(&1)
            .unwrap()
            .last_readback_at = Instant::now() - INVENTORY_LIFETIME - Duration::from_secs(1);

        let stale = state.snapshot(Some("dm-overlay")).await.unwrap();
        assert_eq!(stale["clients"][0]["status"], "stale");
        assert_eq!(
            state
                .set_visibility(&VisibilityRequest {
                    id: "dm-overlay".into(),
                    visible: false,
                })
                .await,
            Err(LayoutError::UnknownId)
        );
        assert_eq!(
            state.mutate("dm-overlay", Transform::default()).await,
            Err(LayoutError::UnknownId)
        );

        admit(&state, 1, &report(0, 2, "dm-overlay", None))
            .await
            .unwrap();
        let revision = state
            .set_visibility(&VisibilityRequest {
                id: "dm-overlay".into(),
                visible: false,
            })
            .await
            .unwrap();
        let mut hidden = report(revision, 3, "dm-overlay", None);
        hidden.elements[0].visibility = contract::ElementVisibility::Hidden;
        admit(&state, 1, &hidden).await.unwrap();

        let applied = state.snapshot(Some("dm-overlay")).await.unwrap();
        assert_eq!(applied["status"], "applied");
        assert_eq!(applied["visibility"]["dm-overlay"], "hidden");
    }
    #[tokio::test]
    async fn vanished_ids_and_future_or_malformed_readbacks_do_not_fabricate_success() {
        let state = LayoutState::new();
        state.enable();
        admit(&state, 1, &report(0, 1, "dm-overlay", None))
            .await
            .unwrap();
        let mut empty = report(0, 2, "other", None);
        empty.elements.clear();
        admit(&state, 1, &empty).await.unwrap();
        assert_eq!(
            state.mutate("dm-overlay", Transform::default()).await,
            Err(LayoutError::UnknownId)
        );
        assert_eq!(
            admit(&state, 1, &report(1, 3, "other", None)).await,
            Err(LayoutError::Revision)
        );
        assert!(
            state
                .readback(1, "{\"revision\":0,\"css\":\"x\"}")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn paged_inventory_is_atomic_and_mismatched_or_incomplete_samples_never_admit_ids() {
        let state = LayoutState::new();
        state.enable();
        let mut full = report(0, 1, "visual-0", None);
        full.elements = (0..512)
            .map(|index| {
                let mut element = full.elements[0].clone();
                element.id = format!("visual-{index}");
                element.effective.as_mut().unwrap().matrix = "x".repeat(512);
                element
            })
            .collect();
        let pages = full.pages().unwrap();
        assert!(pages.len() > 1);
        for page in pages.iter().take(pages.len() - 1) {
            state
                .readback(1, &serde_json::to_string(page).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(
            state
                .mutate(
                    "visual-511",
                    Transform {
                        rotate: Some(10.0),
                        ..Default::default()
                    }
                )
                .await,
            Err(LayoutError::UnknownId)
        );
        assert_eq!(
            state.snapshot(None).await.unwrap()["pending_pages"][0]["received"],
            pages.len() - 1
        );
        let last = pages.last().unwrap();
        state
            .readback(1, &serde_json::to_string(last).unwrap())
            .await
            .unwrap();
        assert_eq!(
            state.snapshot(None).await.unwrap()["clients"][0]["elements"]
                .as_array()
                .unwrap()
                .len(),
            512
        );
        state
            .readback(1, &serde_json::to_string(last).unwrap())
            .await
            .unwrap();
        assert_eq!(
            state
                .mutate(
                    "visual-511",
                    Transform {
                        rotate: Some(10.0),
                        ..Default::default()
                    }
                )
                .await
                .unwrap(),
            1
        );
        let mut wrong = pages[0].clone();
        wrong.sample = 2;
        wrong.revision = 1;
        state
            .readback(2, &serde_json::to_string(&wrong).unwrap())
            .await
            .unwrap();
        wrong.page = 1;
        wrong.revision = 0;
        assert_eq!(
            state
                .readback(2, &serde_json::to_string(&wrong).unwrap())
                .await,
            Err(LayoutError::Shape)
        );
    }
}
