//! Authenticated presentation diagnostics over the existing generated RPC bridge.
//! A receipt means native audit emission, not durable telemetry storage or game authority.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use df_observe::presentation;
use df_persistence::local_demo_scope::LocalDemoRole;
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, oneshot};
use tonic::{Request, Response, Status};

use super::{Service, actor, credential};

const MAX_PRODUCERS: usize = 128;
const UPLOAD_DEADLINE: Duration = Duration::from_secs(10);

pub(super) fn scope_hash(kind: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"df-presentation-scope-v1\0");
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    format!(
        "sha256:{}",
        hash.finalize()
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScopeCorrelation {
    room_id_hash: Option<String>,
    run_id_hash: Option<String>,
    build_id: String,
}

impl ScopeCorrelation {
    fn log_suffix(&self) -> String {
        format!(
            "room_id_hash={} run_id_hash={} build={}",
            self.room_id_hash.as_deref().unwrap_or("none"),
            self.run_id_hash.as_deref().unwrap_or("none"),
            self.build_id,
        )
    }
}

fn scope_correlation(session: Option<&[u8]>, run: Option<&[u8]>, build: &str) -> ScopeCorrelation {
    let room_id_hash = session
        .filter(|id| !id.is_empty())
        .map(|id| scope_hash("room", id));
    let run_id_hash = run
        .filter(|id| !id.is_empty())
        .map(|id| scope_hash("run", id));
    ScopeCorrelation {
        room_id_hash,
        run_id_hash,
        build_id: build.to_owned(),
    }
}

#[derive(Clone)]
pub(super) struct PresentationIngress {
    service: Service,
    ledger: Arc<Mutex<Ledger>>,
    admission: Arc<Semaphore>,
}

#[derive(Default)]
struct Ledger {
    // Never evict a producer while this process lives: an old retry must not become new.
    producers: HashMap<([u8; 32], [u8; 16]), Accepted>,
}
struct Accepted {
    sequence: u64,
    digest: [u8; 32],
    records: usize,
}

struct TimingUpload<'a> {
    binding: [u8; 32],
    producer: [u8; 16],
    sequence: u64,
    dropped: u64,
    bytes: &'a [u8],
    role: LocalDemoRole,
    correlation: &'a ScopeCorrelation,
    receive_unix_ms: u64,
}

fn presentation_fields(
    event: &presentation::TimingEvent,
    correlation: &ScopeCorrelation,
    page_producer_hash: &str,
) -> (
    serde_json::Map<String, serde_json::Value>,
    Vec<(&'static str, String)>,
) {
    let resource_type = match event.kind {
        "voice-start" | "voice-end" | "voice-failed" => "voice",
        "plate-requested" | "plate-loaded" | "plate-decoded" | "plate-shown" | "plate-kept"
        | "plate-crossfade" => "plate",
        "clip-requested" | "clip-ready" | "clip-shown" | "clip-ended" | "clip-hidden"
        | "clip-failed" => "video",
        _ => "browser_media",
    };
    let component_stage = event.component_phase.map(|phase| match phase {
        presentation::ComponentPhase::Requested => "requested",
        presentation::ComponentPhase::Queued => "queued",
        presentation::ComponentPhase::Opened | presentation::ComponentPhase::Activated => {
            "projected"
        }
        presentation::ComponentPhase::Readable => "played",
        presentation::ComponentPhase::Closing | presentation::ComponentPhase::Hidden => "hidden",
        presentation::ComponentPhase::Failed => "failed",
    });
    let stage = component_stage.unwrap_or_else(|| match event.kind {
        "line-open" | "figure" | "figure-mounted" | "figure-shown" | "scene" | "plate-shown"
        | "plate-kept" | "plate-crossfade" | "visual-source" | "check-shown" | "paint"
        | "paint-opportunity" => "projected",
        "line" | "voice-start" | "voice-end" | "clip-shown" | "clip-ended" => "played",
        "voice-failed" | "clip-failed" => "failed",
        "plate-requested" | "clip-requested" => "requested",
        "plate-loaded" | "plate-decoded" | "clip-ready" | "usable" => "ready",
        "figure-hidden" | "clip-hidden" | "check-hidden" => "hidden",
        _ => "observed",
    });
    let status = match (event.component_phase, event.reason, stage) {
        (Some(presentation::ComponentPhase::Failed), _, _) => "failed",
        (Some(presentation::ComponentPhase::Hidden), _, _) => "cancelled",
        (Some(presentation::ComponentPhase::Readable), _, _) => "played",
        (_, "failed", _) => "failed",
        (_, _, "requested") => "requested",
        (_, _, "queued") => "queued",
        (_, _, "ready") => "ready",
        (_, _, "played") => "partial",
        (_, _, "projected") => "projected",
        (_, _, "hidden") => "cancelled",
        (_, _, _) => match (event.reason, stage) {
            ("failed", _) => "failed",
            (_, "requested") => "requested",
            (_, "ready") => "ready",
            (_, "played")
                if event.reason == "read"
                    || event.reason == "voice-ended"
                    || event.reason == "ended" =>
            {
                "played"
            }
            (_, "played") => "partial",
            (_, "projected") => "projected",
            (_, "hidden") => "cancelled",
            _ => "observed",
        },
    };
    let mut fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!(resource_type)),
        ("resource_id".to_owned(), serde_json::json!(event.id)),
        (
            "table_sequence".to_owned(),
            serde_json::json!(event.table_sequence),
        ),
        ("scene_id".to_owned(), serde_json::json!(event.scene)),
        ("kind".to_owned(), serde_json::json!(event.kind)),
        ("stage".to_owned(), serde_json::json!(stage)),
        ("status".to_owned(), serde_json::json!(status)),
        ("reason".to_owned(), serde_json::json!(event.reason)),
        ("at_ms".to_owned(), serde_json::json!(event.at_ms)),
        ("char_count".to_owned(), serde_json::json!(event.chars)),
        ("read_ms".to_owned(), serde_json::json!(event.read_ms)),
        ("opened_ms".to_owned(), serde_json::json!(event.opened_ms)),
        (
            "voice_start_ms".to_owned(),
            serde_json::json!(event.voice_start_ms),
        ),
        (
            "voice_end_ms".to_owned(),
            serde_json::json!(event.voice_end_ms),
        ),
        ("closed_ms".to_owned(), serde_json::json!(event.closed_ms)),
        (
            "crossfade_ms".to_owned(),
            serde_json::json!(event.crossfade_ms),
        ),
        (
            "page_producer_hash".to_owned(),
            serde_json::json!(page_producer_hash),
        ),
    ]);
    fields.insert(
        "room_id_hash".to_owned(),
        serde_json::json!(correlation.room_id_hash.as_deref()),
    );
    fields.insert(
        "run_id_hash".to_owned(),
        serde_json::json!(correlation.run_id_hash.as_deref()),
    );
    fields.insert(
        "build_id".to_owned(),
        serde_json::json!(correlation.build_id.as_str()),
    );
    if let (Some(component), Some(phase), Some(visibility)) =
        (event.component, event.component_phase, event.visibility)
    {
        fields.insert(
            "component_type".to_owned(),
            serde_json::json!(component.label()),
        );
        fields.insert(
            "component_phase".to_owned(),
            serde_json::json!(phase.label()),
        );
        fields.insert(
            "visibility".to_owned(),
            serde_json::json!(visibility.label()),
        );
        fields.insert(
            "visible_held_ms".to_owned(),
            serde_json::json!(event.visible_held_ms),
        );
        fields.insert("lag_ms".to_owned(), serde_json::json!(event.lag_ms));
    }
    let mut attributes = vec![
        (
            "gameplay.presentation.resource_type",
            resource_type.to_owned(),
        ),
        ("gameplay.presentation.resource_id", event.id.to_string()),
        (
            "gameplay.presentation.table_sequence",
            event
                .table_sequence
                .map_or_else(|| "none".to_owned(), |sequence| sequence.to_string()),
        ),
        ("gameplay.presentation.scene_id", event.scene.to_string()),
        ("gameplay.presentation.kind", event.kind.to_owned()),
        ("gameplay.presentation.stage", stage.to_owned()),
        ("gameplay.presentation.status", status.to_owned()),
        ("gameplay.presentation.reason", event.reason.to_owned()),
        ("gameplay.presentation.at_ms", event.at_ms.to_string()),
        ("gameplay.presentation.char_count", event.chars.to_string()),
        ("gameplay.presentation.read_ms", event.read_ms.to_string()),
        (
            "gameplay.presentation.room_id_hash",
            correlation
                .room_id_hash
                .as_deref()
                .unwrap_or("none")
                .to_owned(),
        ),
        (
            "gameplay.presentation.run_id_hash",
            correlation
                .run_id_hash
                .as_deref()
                .unwrap_or("none")
                .to_owned(),
        ),
        (
            "gameplay.presentation.build_id",
            correlation.build_id.clone(),
        ),
        (
            "gameplay.presentation.page_producer_hash",
            page_producer_hash.to_owned(),
        ),
    ];
    if let (Some(component), Some(phase), Some(visibility)) =
        (event.component, event.component_phase, event.visibility)
    {
        attributes.extend([
            (
                "gameplay.presentation.component_type",
                component.label().to_owned(),
            ),
            (
                "gameplay.presentation.component_phase",
                phase.label().to_owned(),
            ),
            (
                "gameplay.presentation.visibility",
                visibility.label().to_owned(),
            ),
        ]);
        if let Some(visible_held_ms) = event.visible_held_ms {
            attributes.push((
                "gameplay.presentation.visible_held_ms",
                visible_held_ms.to_string(),
            ));
        }
        if let Some(lag_ms) = event.lag_ms {
            attributes.push(("gameplay.presentation.lag_ms", lag_ms.to_string()));
        }
    }
    if let Some(details) = &event.visual_source {
        let requested_asset_id = details
            .requested_asset
            .map_or_else(|| "none".to_owned(), |id| id.to_string());
        let effective_asset_id = details
            .effective_asset
            .map_or_else(|| "none".to_owned(), |id| id.to_string());
        for (key, value) in [
            ("requested_source", details.requested.label()),
            ("effective_source", details.effective.label()),
            ("shot", details.shot.label()),
            ("plate_state", details.plate_state.label()),
        ] {
            attributes.push((
                match key {
                    "requested_source" => "gameplay.presentation.requested_source",
                    "effective_source" => "gameplay.presentation.effective_source",
                    "shot" => "gameplay.presentation.shot",
                    _ => "gameplay.presentation.plate_state",
                },
                value.to_owned(),
            ));
        }
        attributes.push((
            "gameplay.presentation.requested_asset_id",
            requested_asset_id.clone(),
        ));
        attributes.push((
            "gameplay.presentation.effective_asset_id",
            effective_asset_id.clone(),
        ));
        fields.insert(
            "requested_source".to_owned(),
            serde_json::json!(details.requested.label()),
        );
        fields.insert(
            "effective_source".to_owned(),
            serde_json::json!(details.effective.label()),
        );
        fields.insert("shot".to_owned(), serde_json::json!(details.shot.label()));
        fields.insert(
            "plate_state".to_owned(),
            serde_json::json!(details.plate_state.label()),
        );
        fields.insert(
            "requested_asset_id".to_owned(),
            serde_json::json!(details.requested_asset),
        );
        fields.insert(
            "effective_asset_id".to_owned(),
            serde_json::json!(details.effective_asset),
        );
        return (fields, attributes);
    }
    (fields, attributes)
}

impl Ledger {
    fn record(
        &mut self,
        upload: TimingUpload<'_>,
        emit: impl Fn(&'static str, String, Option<presentation::TimingEvent>, &str),
    ) -> Result<(usize, bool), Status> {
        let TimingUpload {
            binding,
            producer,
            sequence,
            dropped,
            bytes,
            role,
            correlation,
            receive_unix_ms: receive_ms,
        } = upload;
        let events = if bytes.is_empty() && dropped != 0 {
            Vec::new()
        } else {
            presentation::decode(bytes)
                .map_err(|_| Status::invalid_argument("timing catalog refused"))?
        };
        if sequence == 0 || sequence > i64::MAX as u64 || producer == [0; 16] {
            return Err(Status::invalid_argument("timing identity refused"));
        }
        if role == LocalDemoRole::Player
            && events.iter().any(|event| {
                !matches!(
                    event.kind,
                    "paint" | "paint-opportunity" | "usable" | "sound"
                )
            })
        {
            return Err(Status::permission_denied("player timing catalog refused"));
        }
        let mut hash = Sha256::new();
        hash.update(bytes);
        hash.update(dropped.to_le_bytes());
        let digest = hash.finalize().into();
        let key = (binding, producer);
        if let Some(accepted) = self.producers.get(&key) {
            if sequence == accepted.sequence && digest == accepted.digest {
                return Ok((accepted.records, true));
            }
            if sequence != accepted.sequence + 1 {
                return Err(Status::already_exists("timing batch identity conflict"));
            }
        } else if self.producers.len() == MAX_PRODUCERS {
            return Err(Status::resource_exhausted("timing producer capacity"));
        }
        let producer = producer.as_slice();
        let producer = scope_hash("display-producer", producer);
        let audience = if role == LocalDemoRole::Display {
            "DISPLAY"
        } else {
            "PHONE"
        };
        if !self.producers.contains_key(&key) && sequence != 1 {
            emit(
                "gameplay.presentation_gap",
                format!(
                    "{audience} gap producer={producer} batch={sequence} receive_unix_ms={receive_ms} reason=receiver-restart-or-missing-prefix"
                ),
                None,
                &producer,
            );
        }
        for event in &events {
            emit(
                "gameplay.presentation",
                format!(
                    "{} {}{}",
                    event.log_line(audience, receive_ms, &producer, sequence),
                    correlation.log_suffix(),
                    crate::client_diagnostics::log_suffix(event),
                ),
                Some(event.clone()),
                &producer,
            );
        }
        if dropped != 0 {
            emit(
                "gameplay.presentation_gap",
                format!(
                    "{audience} gap producer={producer} batch={sequence} receive_unix_ms={receive_ms} dropped_records={dropped}"
                ),
                None,
                &producer,
            );
        }
        self.producers.insert(
            key,
            Accepted {
                sequence,
                digest,
                records: events.len(),
            },
        );
        Ok((events.len(), false))
    }
}

impl PresentationIngress {
    pub(super) fn new(service: Service) -> Self {
        Self {
            service,
            ledger: Arc::new(Mutex::new(Ledger::default())),
            admission: Arc::new(Semaphore::new(4)),
        }
    }

    async fn upload_inner(
        &self,
        request: Request<rpc::PresentationTelemetryRequest>,
    ) -> Result<Response<rpc::PresentationTelemetryResponse>, Status> {
        let receive_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Status::unavailable("timing receive clock unavailable"))?
            .as_millis();
        let receive_unix_ms = u64::try_from(receive_unix_ms)
            .map_err(|_| Status::unavailable("timing receive clock overflow"))?;
        let _admission = self
            .admission
            .try_acquire()
            .map_err(|_| Status::resource_exhausted("timing upload capacity"))?;
        let binding = credential(&request)?;
        let request = request.into_inner();
        if request.otlp_logs.len() > presentation::MAX_BATCH_BYTES {
            return Err(Status::resource_exhausted("timing upload bytes"));
        }
        let producer: [u8; 16] = request
            .producer_id
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("timing producer invalid"))?;
        let scope = request
            .scope
            .ok_or_else(|| Status::invalid_argument("timing scope required"))?;
        let correlation = scope_correlation(
            scope.session_id.as_ref().and_then(|id| id.value.as_deref()),
            scope.run_id.as_ref().and_then(|id| id.value.as_deref()),
            crate::BUILD_ID,
        );
        // Authenticate scope without projecting a view or refreshing seat presence. A telemetry
        // request must never create notebook evidence or otherwise influence the game.
        let (reply, wait) = oneshot::channel();
        self.service
            .actor
            .try_submit(actor::Call::TelemetryIdentity {
                credential: binding,
                request: scope,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("timing authentication busy"))?;
        let role = wait
            .await
            .map_err(|_| Status::unavailable("timing owner stopped"))??;
        let basis = self.service.updates.borrow().basis();
        let audit = df_observe::AuditContext {
            operation: None,
            seat: Some(df_observe::seat_hash(&binding)),
            room: Some(df_observe::operation_hash(basis.session.as_bytes())),
            revision: Some(format!(
                "{}:{}",
                basis.revision.epoch().get(),
                basis.revision.sequence(),
            )),
        };
        let (records, replayed) = self.ledger.lock().await.record(
            TimingUpload {
                binding,
                producer,
                sequence: request.batch_sequence,
                dropped: request.dropped_records,
                bytes: &request.otlp_logs,
                role,
                correlation: &correlation,
                receive_unix_ms,
            },
            |event, line, timing, page_producer_hash| {
                let _audit_scope = df_observe::audit_scope(audit.clone());
                if let Some(timing) = timing.as_ref() {
                    let (mut fields, attributes) =
                        presentation_fields(timing, &correlation, page_producer_hash);
                    fields.insert(
                        "audience".to_owned(),
                        serde_json::json!(if role == LocalDemoRole::Display {
                            "display"
                        } else {
                            "player"
                        }),
                    );
                    actor::record_audit_diagnostic(event, line, fields, &attributes);
                } else {
                    actor::record_audit_line(event, line);
                }
            },
        )?;
        Ok(Response::new(rpc::PresentationTelemetryResponse {
            batch_sequence: request.batch_sequence,
            accepted_records: records as u32,
            replayed,
        }))
    }
}

#[tonic::async_trait]
impl rpc::presentation_telemetry_service_server::PresentationTelemetryService
    for PresentationIngress
{
    async fn upload(
        &self,
        request: Request<rpc::PresentationTelemetryRequest>,
    ) -> Result<Response<rpc::PresentationTelemetryResponse>, Status> {
        let outcome = match tokio::time::timeout(UPLOAD_DEADLINE, self.upload_inner(request)).await
        {
            Ok(outcome) => outcome,
            Err(_) => Err(Status::deadline_exceeded("timing upload deadline")),
        };
        if let Err(status) = &outcome {
            // Status codes are fixed catalog values; never log rejected payloads/credentials.
            actor::record_audit_line(
                "gameplay.presentation_refused",
                format!("TELEMETRY refused code={:?}", status.code()),
            );
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_observe::presentation::{
        ComponentPhase, EffectiveVisibility, PresentationComponent, TimingEvent, VisualPlateState,
        VisualShot, VisualSource, VisualSourceDetails,
    };
    use std::cell::RefCell;
    use std::sync::OnceLock;

    #[test]
    fn phone_portrait_and_reconnect_diagnostics_are_ingested_with_semantic_labels() {
        use crate::client_diagnostics::Diagnostic;
        for diagnostic in [
            Diagnostic::PortraitRequestStart,
            Diagnostic::PortraitPollPending,
            Diagnostic::PortraitPollReady,
            Diagnostic::PortraitPollFailed,
            Diagnostic::PortraitPollUnknown,
            Diagnostic::PortraitDisplayed,
            Diagnostic::PortraitStoppedExhausted,
            Diagnostic::ReconnectScheduled,
            Diagnostic::ReconnectAttempt,
            Diagnostic::ReconnectReady,
            Diagnostic::ReconnectFailed,
        ] {
            let bytes = presentation::encode(&[diagnostic.event(1234, 12, 2_000, 30_000)]).unwrap();
            let mut upload = upload(&bytes);
            upload.role = LocalDemoRole::Player;
            let lines = RefCell::new(Vec::new());
            let result =
                Ledger::default().record(upload, |_, line, _, _| lines.borrow_mut().push(line));
            assert_eq!(result.unwrap(), (1, false));
            let lines = lines.into_inner();
            assert_eq!(lines.len(), 1);
            assert!(lines[0].starts_with("PHONE usable"));
            assert!(lines[0].contains(diagnostic.label()));
            assert!(lines[0].contains("operation_hash=1234 attempt=12 delay_ms=2000"));
        }
    }

    static TEST_CORRELATION: OnceLock<ScopeCorrelation> = OnceLock::new();

    fn upload(bytes: &[u8]) -> TimingUpload<'_> {
        TimingUpload {
            binding: [1; 32],
            producer: [2; 16],
            sequence: 1,
            dropped: 0,
            bytes,
            role: LocalDemoRole::Display,
            correlation: TEST_CORRELATION.get_or_init(|| ScopeCorrelation {
                room_id_hash: Some("sha256:test-room".to_owned()),
                run_id_hash: Some("sha256:test-run".to_owned()),
                build_id: "build-test".to_owned(),
            }),
            receive_unix_ms: 123_456,
        }
    }

    #[test]
    fn visual_source_ingress_exposes_only_closed_labels_and_hashes() {
        let event = TimingEvent::visual_source(
            17,
            23,
            100,
            VisualSourceDetails {
                requested: VisualSource::ScenePlate,
                effective: VisualSource::EstablishingPhoto,
                shot: VisualShot::Establishing,
                plate_state: VisualPlateState::Failed,
                requested_asset: Some(31),
                effective_asset: Some(37),
            },
        );
        let correlation =
            scope_correlation(Some(b"private-room"), Some(b"private-run"), "test-build");
        let (fields, attributes) = presentation_fields(&event, &correlation, "sha256:test-page");
        assert_eq!(fields["resource_type"], "browser_media");
        assert_eq!(fields["stage"], "projected");
        assert_eq!(fields["resource_id"], 17);
        assert_eq!(fields["requested_source"], "scene_plate");
        assert_eq!(fields["effective_source"], "establishing_photo");
        assert_eq!(fields["shot"], "establishing");
        assert_eq!(fields["plate_state"], "failed");
        assert_eq!(fields["requested_asset_id"], 31);
        assert_eq!(fields["effective_asset_id"], 37);
        assert_eq!(
            fields["room_id_hash"].as_str(),
            correlation.room_id_hash.as_deref()
        );
        assert_eq!(
            fields["run_id_hash"].as_str(),
            correlation.run_id_hash.as_deref()
        );
        assert_eq!(fields["build_id"], "test-build");
        let serialized = serde_json::to_string(&fields).unwrap();
        assert!(!serialized.contains("private-room"));
        assert!(!serialized.contains("private-run"));
        assert!(attributes.iter().all(|(_, value)| value != "private-url"));
    }

    #[test]
    fn component_ingress_carries_phase_visibility_and_page_correlation_without_content() {
        let mut event = TimingEvent::component_lifecycle(
            17,
            23,
            8_500,
            PresentationComponent::SpeechBubble,
            ComponentPhase::Readable,
            EffectiveVisibility::Hidden,
            "visibility-lost",
        );
        event.opened_ms = Some(2_000);
        event.closed_ms = Some(8_500);
        event.read_ms = 5_000;
        event.visible_held_ms = Some(3_200);
        let correlation =
            scope_correlation(Some(b"private-room"), Some(b"private-run"), "test-build");
        let (fields, attributes) = presentation_fields(&event, &correlation, "sha256:a1b2c3");
        assert_eq!(fields["resource_type"], "browser_media");
        assert_eq!(fields["resource_id"], 17);
        assert_eq!(fields["scene_id"], 23);
        assert_eq!(fields["component_type"], "speech_bubble");
        assert_eq!(fields["component_phase"], "readable");
        assert_eq!(fields["visibility"], "hidden");
        assert_eq!(fields["visible_held_ms"], 3_200);
        assert_eq!(fields["page_producer_hash"], "sha256:a1b2c3");
        assert_eq!(
            fields["room_id_hash"].as_str(),
            correlation.room_id_hash.as_deref()
        );
        assert_eq!(fields["build_id"], "test-build");
        let serialized = serde_json::to_string(&fields).unwrap();
        assert!(!serialized.contains("private-room"));
        assert!(!serialized.contains("private-run"));
        assert!(!serialized.contains("private-page-id"));
        assert!(!serialized.contains("transcript"));
        assert!(attributes.iter().any(|(name, value)| {
            *name == "gameplay.presentation.component_type" && value == "speech_bubble"
        }));
        assert!(attributes.iter().any(|(name, value)| {
            *name == "gameplay.presentation.visibility" && value == "hidden"
        }));
    }

    #[test]
    fn presentation_correlation_hashes_scope_and_producer_ids() {
        let session = b"private-session-id";
        let run = b"private-run-id";
        let correlation = scope_correlation(Some(session), Some(run), "build-test");
        let suffix = correlation.log_suffix();
        assert!(suffix.contains("room_id_hash=sha256:"));
        assert!(suffix.contains("run_id_hash=sha256:"));
        assert!(suffix.contains("build=build-test"));
        assert!(!suffix.contains("private-session-id"));
        assert!(!suffix.contains("private-run-id"));
        assert!(!suffix.contains("session="));
        assert!(!suffix.contains("run="));
        assert_eq!(
            scope_correlation(Some(&[]), Some(&[]), "build-test").log_suffix(),
            "room_id_hash=none run_id_hash=none build=build-test"
        );
        assert_eq!(
            scope_hash("display-producer", b"producer"),
            scope_hash("display-producer", b"producer")
        );
        assert_ne!(scope_hash("room", session), scope_hash("run", session));
    }

    #[test]
    fn server_timing_log_records_once_rejects_conflicts_and_excludes_private_payloads() {
        let mut ledger = Ledger::default();
        let line = TimingEvent {
            chars: 89,
            read_ms: 8_285,
            opened_ms: Some(100),
            closed_ms: Some(10_000),
            ..TimingEvent::lifecycle("line", "read", 29, 7, 10_000)
        };
        let bytes = presentation::encode(&[line]).unwrap();
        let lines = RefCell::new(Vec::new());
        let emit = |_: &'static str,
                    line: String,
                    _: Option<presentation::TimingEvent>,
                    _: &str| { lines.borrow_mut().push(line) };
        assert_eq!(ledger.record(upload(&bytes), emit).unwrap(), (1, false));
        assert_eq!(ledger.record(upload(&bytes), emit).unwrap(), (1, true));
        assert_eq!(lines.borrow().len(), 1);
        assert!(lines.borrow()[0].contains("DISPLAY line"));
        assert!(lines.borrow()[0].contains("held_ms=9900"));
        assert!(lines.borrow()[0].contains("receive_unix_ms=123456"));
        assert!(
            ledger
                .record(
                    TimingUpload {
                        dropped: 1,
                        ..upload(&bytes)
                    },
                    emit
                )
                .is_err()
        );
        assert!(
            ledger
                .record(
                    TimingUpload {
                        producer: [3; 16],
                        role: LocalDemoRole::Player,
                        ..upload(&bytes)
                    },
                    emit
                )
                .is_err()
        );
        assert!(
            ledger
                .record(
                    TimingUpload {
                        producer: [3; 16],
                        ..upload(b"private text")
                    },
                    emit,
                )
                .is_err()
        );
        assert_eq!(lines.borrow().len(), 1);
    }
}

/// Development-only injection of a clearly labelled fixture into the real authenticated
/// handler. This proves ingress/allowlisting/replay, never actual browser presentation.
pub(super) async fn synthetic_probe(
    service: Service,
    binding: [u8; 32],
    scope: rpc::WatchViewRequest,
) -> Result<serde_json::Value, Status> {
    use df_observe::presentation::TimingEvent;
    use rpc::presentation_telemetry_service_server::PresentationTelemetryService;

    if std::env::var("DF_GAMEPLAY_DEV_CONTROL").as_deref() != Ok("on") {
        return Err(Status::permission_denied(
            "timing probe is development only",
        ));
    }
    let producer = *b"T01-synthetic-01";
    let producer_hex = producer
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let events = [
        TimingEvent::lifecycle("plate-requested", "ready", 5, 7, 100),
        TimingEvent::lifecycle("plate-loaded", "ready", 5, 7, 200),
        TimingEvent::lifecycle("plate-decoded", "ready", 5, 7, 300),
        TimingEvent {
            crossfade_ms: 1_200,
            ..TimingEvent::lifecycle("plate-shown", "ready", 5, 7, 500)
        },
        TimingEvent::lifecycle("figure-shown", "appeared", 2, 7, 600),
        TimingEvent {
            slot: Some(2),
            chars: 89,
            read_ms: 8_285,
            opened_ms: Some(1_000),
            voice_start_ms: Some(1_100),
            voice_end_ms: Some(7_000),
            closed_ms: Some(10_000),
            ..TimingEvent::lifecycle("line", "read", 29, 7, 10_000)
        },
        TimingEvent::lifecycle("check-shown", "ready", 10, 7, 11_000),
        TimingEvent::lifecycle("check-hidden", "ended", 10, 7, 15_000),
        TimingEvent::lifecycle("plate-kept", "failed", 6, 7, 16_000),
        TimingEvent::lifecycle("figure-hidden", "scene-change", 2, 7, 17_000),
    ];
    let body = rpc::PresentationTelemetryRequest {
        scope: Some(scope),
        producer_id: producer.to_vec(),
        batch_sequence: 1,
        otlp_logs: presentation::encode(&events)
            .map_err(|_| Status::internal("synthetic timing fixture invalid"))?,
        dropped_records: 0,
    };
    let ingress = PresentationIngress::new(service);
    let request = |body| -> Result<_, Status> {
        let mut request = Request::new(body);
        let credential = binding
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        request.metadata_mut().insert(
            "x-df-local-binding",
            credential
                .parse()
                .map_err(|_| Status::internal("probe credential encoding failed"))?,
        );
        Ok(request)
    };
    // The actor authentication occurs before the fixture is admitted, including on the retry.
    let first = ingress.upload(request(body.clone())?).await?.into_inner();
    actor::record_audit_line(
        "gameplay.presentation_synthetic",
        format!(
            "DISPLAY synthetic-probe producer={producer_hex} synthetic=true records={} evidence=ingress-only",
            first.accepted_records
        ),
    );
    let replay = ingress.upload(request(body)?).await?.into_inner();
    Ok(
        serde_json::json!({ "synthetic": true, "evidence": "authenticated ingress only; no browser or media observed", "producer": producer_hex, "accepted_records": first.accepted_records, "replayed": replay.replayed }),
    )
}

#[cfg(test)]
pub(super) fn fake_display_upload(
    events: Vec<presentation::TimingEvent>,
    run: &[u8],
) -> serde_json::Value {
    let mut buffer = presentation::TimingBuffer::default();
    for event in events {
        buffer.push(event).unwrap();
    }
    let batch = buffer.batch().unwrap();
    let correlation = scope_correlation(Some(b"test-room"), Some(run), "hs48-test");
    let accepted = std::cell::RefCell::new(Vec::new());
    let receipt = Ledger::default()
        .record(
            TimingUpload {
                binding: [1; 32],
                producer: [2; 16],
                sequence: batch.sequence,
                dropped: batch.dropped,
                bytes: &batch.bytes,
                role: LocalDemoRole::Display,
                correlation: &correlation,
                receive_unix_ms: 123_456,
            },
            |_, _, event, producer| {
                if let Some(event) = event {
                    let (mut fields, _) = presentation_fields(&event, &correlation, producer);
                    fields.insert("audience".to_owned(), serde_json::json!("display"));
                    accepted
                        .borrow_mut()
                        .push(serde_json::json!({"unix_ms":123456,"fields":fields}));
                }
            },
        )
        .unwrap();
    assert_eq!(receipt, (batch.records, false));
    assert!(buffer.acknowledge(batch.sequence));
    serde_json::json!({"events":accepted.into_inner()})
}
