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
    correlation: &'a str,
    receive_unix_ms: u64,
}

impl Ledger {
    fn record(
        &mut self,
        upload: TimingUpload<'_>,
        emit: impl Fn(&'static str, String),
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
        let producer = producer
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
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
            );
        }
        for event in &events {
            emit(
                "gameplay.presentation",
                format!(
                    "{} {correlation}",
                    event.log_line(audience, receive_ms, &producer, sequence)
                ),
            );
        }
        if dropped != 0 {
            emit(
                "gameplay.presentation_gap",
                format!(
                    "{audience} gap producer={producer} batch={sequence} receive_unix_ms={receive_ms} dropped_records={dropped}"
                ),
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
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let correlation = format!(
            "session={} run={} build={}",
            scope
                .session_id
                .as_ref()
                .and_then(|id| id.value.as_deref())
                .map(hex)
                .unwrap_or_default(),
            scope
                .run_id
                .as_ref()
                .and_then(|id| id.value.as_deref())
                .map(hex)
                .unwrap_or_default(),
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
            actor::record_audit_line,
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
    use df_observe::presentation::TimingEvent;
    use std::cell::RefCell;

    fn upload(bytes: &[u8]) -> TimingUpload<'_> {
        TimingUpload {
            binding: [1; 32],
            producer: [2; 16],
            sequence: 1,
            dropped: 0,
            bytes,
            role: LocalDemoRole::Display,
            correlation: "session=test run=test build=test",
            receive_unix_ms: 123_456,
        }
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
        let emit = |_, line| lines.borrow_mut().push(line);
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
