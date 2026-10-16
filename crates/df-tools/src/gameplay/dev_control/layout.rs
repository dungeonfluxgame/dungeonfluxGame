//! Authenticated layout state. One mutex makes revision, configuration and acknowledgments atomic.
use super::{Outcome, Shared, authorized, bad_request};
use crate::layout_contract::{
    self as contract, ElementStatus, LayoutDocument, LayoutError, LayoutReadback, MoveRequest,
    ResetRequest, Transform, TransformRequest,
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
    enabled: Arc<AtomicBool>,
    changed: watch::Sender<u64>,
}
struct LayoutInner {
    document: LayoutDocument,
    clients: BTreeMap<u64, ClientReadback>,
}
struct ClientReadback {
    report: LayoutReadback,
    seen: Instant,
}
impl LayoutState {
    pub(crate) fn new() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            state: Arc::new(Mutex::new(LayoutInner {
                document: LayoutDocument::default(),
                clients: BTreeMap::new(),
            })),
            enabled: Arc::new(AtomicBool::new(false)),
            changed,
        }
    }
    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
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
            display.layout_revision = state.document.revision;
            display.layout_json =
                serde_json::to_string(&state.document).map_err(|_| LayoutError::Shape)?;
        }
        Ok(())
    }
    async fn snapshot(&self, id: Option<&str>) -> Result<Value, LayoutError> {
        if let Some(id) = id {
            contract::validate_id(id)?;
        }
        let state = self.state.lock().await;
        let clients:Vec<Value>=state.clients.iter().map(|(instance,client)| {
            let elements:Vec<_>=client.report.elements.iter().filter(|element|id.is_none_or(|id|id==element.id)).collect();
            let status=if client.report.revision!=state.document.revision||client.seen.elapsed()>INVENTORY_LIFETIME { "stale" }
                else if id.is_some()&&elements.is_empty()||elements.iter().any(|e|e.status==ElementStatus::Absent) { "absent" } else { "applied" };
            json!({"display_instance":instance,"status":status,"reported_revision":client.report.revision,"sample":client.report.sample,"viewport":client.report.viewport,"overflow":client.report.overflow,"elements":elements})
        }).collect();
        if let Some(id) = id {
            if !state.document.overrides.contains_key(id)
                && !state
                    .clients
                    .values()
                    .any(|client| client.report.elements.iter().any(|e| e.id == id))
            {
                return Err(LayoutError::UnknownId);
            }
        }
        let overrides: BTreeMap<_, _> = state
            .document
            .overrides
            .iter()
            .filter(|(key, _)| id.is_none_or(|id| id == key.as_str()))
            .collect();
        Ok(
            json!({"revision":state.document.revision,"status":if clients.is_empty(){"pending"}else if clients.iter().all(|c|c["status"]=="applied"){"applied"}else{"pending"},"overrides":overrides,"clients":clients,"capacity":contract::MAX_ELEMENTS}),
        )
    }
    pub(crate) async fn readback(&self, instance: u64, text: &str) -> Result<(), LayoutError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(LayoutError::Unavailable);
        }
        if instance == 0 {
            return Err(LayoutError::Id);
        }
        let report = LayoutReadback::parse(text)?;
        let mut state = self.state.lock().await;
        if report.revision > state.document.revision {
            return Err(LayoutError::Revision);
        }
        if let Some(previous) = state.clients.get(&instance) {
            if report.sample <= previous.report.sample || report.revision < previous.report.revision
            {
                return Err(LayoutError::Revision);
            }
        }
        if report.revision == state.document.revision {
            for element in &report.elements {
                if element.configured.as_ref() != state.document.overrides.get(&element.id) {
                    return Err(LayoutError::Shape);
                }
            }
            for id in state.document.overrides.keys() {
                if !report.elements.iter().any(|e| e.id == *id) {
                    return Err(LayoutError::Shape);
                }
            }
        }
        state
            .clients
            .retain(|_, client| client.seen.elapsed() <= INVENTORY_LIFETIME);
        if !state.clients.contains_key(&instance) && state.clients.len() >= MAX_CLIENTS {
            return Err(LayoutError::Limit);
        }
        let revision = report.revision;
        let current = state.document.revision;
        let changed = state.clients.get(&instance).is_none_or(|old| {
            old.report.revision != revision
                || old
                    .report
                    .elements
                    .iter()
                    .map(|e| (&e.id, e.status, &e.configured))
                    .collect::<Vec<_>>()
                    != report
                        .elements
                        .iter()
                        .map(|e| (&e.id, e.status, &e.configured))
                        .collect::<Vec<_>>()
        });
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
                seen: Instant::now(),
            },
        );
        drop(state);
        if changed {
            event(
                "gameplay.layout.applied",
                revision,
                None,
                if revision != current {
                    "stale"
                } else if !absent.is_empty() {
                    "absent"
                } else {
                    "applied"
                },
            );
            for id in absent {
                event("gameplay.layout.absent", revision, Some(&id), "absent");
            }
        }
        Ok(())
    }
    async fn mutate(&self, id: &str, patch: Transform) -> Result<u64, LayoutError> {
        contract::validate_id(id)?;
        patch.validate()?;
        let mut state = self.state.lock().await;
        let admitted = state.clients.values().any(|client| {
            client.seen.elapsed() <= INVENTORY_LIFETIME
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
        if let Some(id) = id {
            if !state.document.overrides.contains_key(id)
                && !state.clients.values().any(|c| {
                    c.seen.elapsed() <= INVENTORY_LIFETIME
                        && c.report.elements.iter().any(|e| e.id == id)
                })
            {
                return Err(LayoutError::UnknownId);
            }
        }
        let revision = state
            .document
            .revision
            .checked_add(1)
            .ok_or(LayoutError::Limit)?;
        if let Some(id) = id {
            state.document.overrides.remove(id);
        } else {
            state.document.overrides.clear();
        }
        state.document.revision = revision;
        self.changed.send_replace(revision);
        drop(state);
        event("gameplay.layout.reset", revision, id, "pending");
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
            }],
        }
    }
    async fn admit(
        state: &LayoutState,
        instance: u64,
        report: &LayoutReadback,
    ) -> Result<(), LayoutError> {
        state
            .readback(instance, &serde_json::to_string(report).unwrap())
            .await
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
}
