//! Authenticated, read-only inspection of the current presentation owner and cue ring.
use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};

use super::{Outcome, Shared, authorized, bad_request, parse};

pub(super) async fn read(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last =
        parse::parse_last(params.get("last").map_or("50", String::as_str)).map_err(bad_request)?;
    // Refresh only the owner projection. Service::view also registers voices and music jobs;
    // a diagnostic read must neither start those nor bypass the tempo owner via a cache read.
    let basis = state.service.updates.borrow().basis();
    let (session_id, run_id) = super::display_scope(basis);
    let request = df_protocol::common::WatchViewRequest {
        session_id: Some(df_protocol::common::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(df_protocol::common::RunId {
            value: Some(run_id),
        }),
        client_binding_id: Some(df_protocol::common::ClientBindingId {
            value: Some(vec![0x72; 16]),
        }),
        after_revision: None,
    };
    super::super::room::actor_view(&state.service.actor, state.display_credential, request)
        .await
        .map_err(super::owner_view_failure)?;
    Ok((
        StatusCode::OK,
        Json(super::super::actor::tempo::diagnostic_snapshot(last)),
    ))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn dev_control_tempo_handler_authenticates_reads_and_rejects_writes() {
        use super::super::super::{
            Service, actor, journey, living_scenes::LivingScenes, model, npc_reply::NpcVoice, pack,
            startup_configuration::VoiceOff, table_log::TableLog, wire,
        };
        use super::*;
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
        use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::{Mutex, Semaphore, watch};

        const DISPLAY_CREDENTIAL: [u8; 32] = [0x72; 32];
        struct ReadOwner {
            current: df_model::checkpoint::Checkpoint,
            tempo: actor::tempo::TempoOwner,
            reads: Arc<AtomicUsize>,
        }
        impl Reducer<actor::Call> for ReadOwner {
            fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
                match call {
                    actor::Call::View {
                        credential,
                        request,
                        reply,
                    } => {
                        assert_eq!(credential, DISPLAY_CREDENTIAL);
                        assert_eq!(
                            request.client_binding_id.unwrap().value,
                            Some(vec![0x72; 16])
                        );
                        let (session, run) = super::super::display_scope(self.current.basis());
                        assert_eq!(request.session_id.unwrap().value, Some(session));
                        assert_eq!(request.run_id.unwrap().value, Some(run));
                        self.reads.fetch_add(1, Ordering::SeqCst);
                        let mut view = wire::journey_view(
                            &self.current,
                            LocalDemoRole::Display,
                            df_types::MemberId::from_bytes(&DISPLAY).unwrap(),
                        )
                        .unwrap();
                        self.tempo
                            .decorate(&mut view, &self.current, std::time::Instant::now())
                            .unwrap();
                        assert!(
                            actor::tempo::diagnostic_snapshot(3)["current"]["phase"].is_string(),
                            "owner snapshot missing phase"
                        );
                        reply.send(Ok((LocalDemoRole::Display, view))).unwrap();
                    }
                    _ => panic!("a tempo read must never submit a game action"),
                }
            }
        }
        let current = journey::initial().unwrap();
        let tempo = actor::tempo::TempoOwner::start(
            &current,
            pack::TempoTuning::default(),
            std::time::Instant::now(),
        )
        .unwrap();
        let (_, updates) = watch::channel(current.clone());
        let (sender, inbox) = bounded_inbox();
        let reads = Arc::new(AtomicUsize::new(0));
        let worker_reads = reads.clone();
        let worker_current = current.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            inbox
                .run(&mut ReadOwner {
                    current: worker_current,
                    tempo,
                    reads: worker_reads,
                })
                .unwrap();
            done.send(()).unwrap();
        });
        let table = TableLog::new();
        let social = crate::gameplay::social::SocialLedger::none();
        let layout = super::super::layout::LayoutState::new();
        let (shutdown, _) = watch::channel(false);
        let state = Arc::new(Shared {
            service: Service {
                look_ahead: Default::default(),
                portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
                actor: sender,
                display_credential: DISPLAY_CREDENTIAL,
                updates,
                streams: Arc::new(Semaphore::new(2)),
                voice: crate::gameplay::voice::Voice::disabled(VoiceOff::Qualification),
                npc_voice: Arc::new(NpcVoice::silent()),
                table_log: table.clone(),
                social: social.clone(),
                living: LivingScenes::disabled(),
                layout: layout.clone(),
            },
            display_credential: DISPLAY_CREDENTIAL,
            token: Arc::from("tempo-test-token"),
            screen_tickets: Arc::new(Mutex::new(super::super::screen_preview::Tickets::default())),
            seat_file: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/lanes/HS18/unused-seats"),
            shutdown,
            seats: Arc::new(Mutex::new(super::super::Seats {
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
            history: Arc::new(std::sync::Mutex::new(super::super::history::History::new())),
            table,
            social,
            notebook: crate::gameplay::notebook::NotebookLedger::none(),
            lore: Arc::new(crate::gameplay::npc_reply::local_lore::LocalLore::in_memory()),
        });
        let mut router = axum::Router::new()
            .route("/dev/tempo", axum::routing::get(read))
            .with_state(state.clone());
        for (method, path, authenticated, status) in [
            ("GET", "/dev/tempo", false, StatusCode::UNAUTHORIZED),
            ("GET", "/dev/tempo?last=0", true, StatusCode::BAD_REQUEST),
            ("POST", "/dev/tempo", true, StatusCode::METHOD_NOT_ALLOWED),
            ("GET", "/dev/tempo?last=3", true, StatusCode::OK),
        ] {
            let mut request = axum::http::Request::builder().method(method).uri(path);
            if authenticated {
                request = request.header("authorization", "Bearer tempo-test-token");
            }
            let response = tonic::codegen::Service::call(
                &mut router,
                request.body(axum::body::Body::empty()).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), status, "{method} {path}");
            if status == StatusCode::OK {
                let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .unwrap();
                let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert!(document["ambient_layers"].is_object());
                for field in [
                    "phase",
                    "intensity_permille",
                    "target",
                    "release",
                    "sequence",
                    "pacing_band",
                    "pacing_review_period_s",
                    "next_pacing_review_unix_ms",
                ] {
                    assert!(
                        document["current"].get(field).is_some(),
                        "missing {field}: {document}; reads={}",
                        reads.load(Ordering::SeqCst)
                    );
                }
                assert!(document["history"].as_array().unwrap().len() <= 3);
            }
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(
            state.service.voice.development_report(),
            serde_json::json!([])
        );
        state.service.actor.stop().unwrap();
        drop(router);
        drop(state);
        finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        worker.join().unwrap();
    }
}
