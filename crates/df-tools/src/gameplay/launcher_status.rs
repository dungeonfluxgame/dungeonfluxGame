//! What the desktop launcher shows about the running table (REF-064): the phase, adventure, scene
//! and party, read from the same display view the TV receives. The route exists only when the
//! launcher that started this server handed it a token (`DF_GAMEPLAY_LAUNCHER_TOKEN`), and any
//! request without that exact token gets 404, so phones on the network learn nothing from it.
//!
//! The answer is plain lines of tab-separated fields, which the launcher reads without a JSON
//! library: `phase`, `adventure`, `scene` and `place` lines, then one `hero` line per party member
//! (`hero <ready 0|1> <hit points> <maximum> <portrait path> <name>`). Pending asks include
//! the member id, request id and name; a host decision must name that same pending request.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::Request,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use df_protocol::common as rpc;

const TOKEN_VARIABLE: &str = "DF_GAMEPLAY_LAUNCHER_TOKEN";
const TOKEN_HEADER: &str = "x-dungeonflux-launcher";
/// The launcher's token is 32 random bytes in hex; anything shorter is refused at startup.
const MINIMUM_TOKEN_BYTES: usize = 32;
/// The display role's watch binding: the owner accepts no other for the display credential
/// (`actor::watch_scope_identity`).
const STATUS_BINDING: [u8; 16] = [0x72; 16];
const OWNER_WAIT: Duration = Duration::from_millis(1500);
const DECISION_BODY_LIMIT: usize = 128;

#[derive(Clone)]
struct StatusState {
    service: super::Service,
    token: Arc<str>,
}

pub(super) fn router(service: super::Service) -> Router {
    let Some(token) = std::env::var(TOKEN_VARIABLE)
        .ok()
        .filter(|token| token.len() >= MINIMUM_TOKEN_BYTES)
    else {
        return Router::new();
    };
    Router::new()
        .route("/gameplay/launcher-status", get(status))
        .route("/gameplay/launcher-rejoin", post(decision))
        .with_state(StatusState {
            service,
            token: token.into(),
        })
}

async fn status(State(state): State<StatusState>, headers: HeaderMap) -> Response {
    let offered = headers
        .get(TOKEN_HEADER)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    if !same_token(offered, state.token.as_bytes()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let basis = state.service.updates.borrow().basis();
    let request = rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(STATUS_BINDING.to_vec()),
        }),
        after_revision: None,
    };
    let pending = match pending_rejoins(&state.service).await {
        Ok(pending) => pending,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match state
        .service
        .view(state.service.display_credential, request)
        .await
    {
        Ok((_, view)) => (
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            summary_with_rejoins(&view, &pending),
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn pending_rejoins(service: &super::Service) -> Result<Vec<super::room::PendingRejoin>, ()> {
    let (reply, wait) = tokio::sync::oneshot::channel();
    service
        .actor
        .try_submit(super::actor::Call::PendingRejoins { reply })
        .map_err(|_| ())?;
    tokio::time::timeout(OWNER_WAIT, wait)
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn decision(State(state): State<StatusState>, request: Request<Body>) -> Response {
    let offered = request
        .headers()
        .get(TOKEN_HEADER)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    if !same_token(offered, state.token.as_bytes()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(Ok(bytes)) = tokio::time::timeout(
        OWNER_WAIT,
        to_bytes(request.into_body(), DECISION_BODY_LIMIT),
    )
    .await
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some((member, operation, allow)) = parse_decision(&bytes) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let (reply, wait) = tokio::sync::oneshot::channel();
    let deadline = Instant::now() + OWNER_WAIT;
    if state
        .service
        .actor
        .try_submit(super::actor::Call::DecideRejoin {
            member,
            operation,
            allow,
            deadline,
            reply,
        })
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match tokio::time::timeout(OWNER_WAIT, wait).await {
        Ok(Ok(Ok(true))) => ([(header::CACHE_CONTROL, "no-store")], StatusCode::OK).into_response(),
        Ok(Ok(Ok(false))) => StatusCode::CONFLICT.into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn parse_decision(bytes: &[u8]) -> Option<([u8; 16], [u8; 16], bool)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.split('\n');
    let member = parse_id(lines.next()?.strip_prefix("member=")?)?;
    let operation = parse_id(lines.next()?.strip_prefix("request=")?)?;
    let allow = match lines.next()?.strip_prefix("allow=")? {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    if lines.next().is_some() {
        return None;
    }
    Some((member, operation, allow))
}

fn parse_id(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    super::journey::unhex(&text.to_ascii_lowercase())
        .ok()?
        .try_into()
        .ok()
}

fn summary_with_rejoins(view: &rpc::ViewMessage, pending: &[super::room::PendingRejoin]) -> String {
    let mut value = summary(view);
    for ask in pending.iter().take(6) {
        if !value.is_empty() {
            value.push('\n');
        }
        value.push_str(&format!(
            "rejoin\t{}\t{}\t{}",
            super::journey::hex(&ask.member),
            super::journey::hex(&ask.operation),
            field(&ask.name)
        ));
    }
    value
}

/// Compares every byte, so the time taken says nothing about how much of a guess was right.
fn same_token(offered: &[u8], expected: &[u8]) -> bool {
    offered.len() == expected.len()
        && offered
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn summary(view: &rpc::ViewMessage) -> String {
    let Some(rpc::view_message::Audience::Display(display)) = view.audience.as_ref() else {
        return String::new();
    };
    let mut lines = Vec::new();
    if let Some(journey) = display.journey.as_ref() {
        lines.push(format!("phase\t{}", phase_name(journey.phase)));
        lines.push(format!("adventure\t{}", field(&journey.adventure_title)));
        for member in &journey.party {
            lines.push(format!(
                "hero\t{}\t{}\t{}\t{}\t{}",
                u8::from(member.character_ready),
                member.hit_points,
                member.maximum_hit_points,
                field(&member.portrait),
                field(&member.name)
            ));
        }
    }
    if let Some(scene) = display.scene.as_ref() {
        lines.push(format!("scene\t{}", field(&scene.title)));
        lines.push(format!("place\t{}", field(&scene.place)));
    }
    lines.join("\n")
}

/// One field of a line: tabs and line breaks become spaces.
fn field(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn phase_name(phase: i32) -> &'static str {
    match rpc::JourneyPhase::try_from(phase) {
        Ok(rpc::JourneyPhase::Room) => "room",
        Ok(rpc::JourneyPhase::CharacterCreation) => "creation",
        Ok(rpc::JourneyPhase::Opening) => "opening",
        Ok(rpc::JourneyPhase::Dialogue) => "dialogue",
        Ok(rpc::JourneyPhase::Combat) => "combat",
        Ok(rpc::JourneyPhase::Complete) => "complete",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_session::inbox::{AdmissionSequence, Reducer};

    struct RejoinOwner {
        pending: bool,
        expected_allow: bool,
    }
    impl Reducer<super::super::actor::Call> for RejoinOwner {
        fn reduce(&mut self, _: AdmissionSequence, call: super::super::actor::Call) {
            match call {
                super::super::actor::Call::PendingRejoins { reply } => {
                    let pending = if self.pending {
                        vec![super::super::room::PendingRejoin {
                            member: [1; 16],
                            operation: [2; 16],
                            name: "Brannoc".to_owned(),
                        }]
                    } else {
                        vec![]
                    };
                    let _ = reply.send(Ok(pending));
                }
                super::super::actor::Call::DecideRejoin {
                    member,
                    operation,
                    allow,
                    reply,
                    ..
                } => {
                    let accepted = self.pending
                        && member == [1; 16]
                        && operation == [2; 16]
                        && allow == self.expected_allow;
                    if accepted {
                        self.pending = false;
                    }
                    let _ = reply.send(Ok(accepted));
                }
                super::super::actor::Call::View { reply, .. } => {
                    let view = rpc::ViewMessage {
                        audience: Some(rpc::view_message::Audience::Display(
                            rpc::DisplayGameplayView::default(),
                        )),
                        ..Default::default()
                    };
                    let _ = reply.send(Ok((super::super::LocalDemoRole::Display, view)));
                }
                _ => panic!("unexpected call"),
            }
        }
    }

    #[tokio::test]
    async fn authenticated_host_can_list_and_decide_exact_pending_request() {
        use super::super::*;
        for allow in [true, false] {
            let (_, updates) = tokio::sync::watch::channel(journey::initial().unwrap());
            let (actor, inbox) = df_session::inbox::bounded_inbox();
            let owner = std::thread::spawn(move || {
                inbox.run(&mut RejoinOwner {
                    pending: true,
                    expected_allow: allow,
                })
            });
            let service = Service {
                look_ahead: Default::default(),
                actor: actor.clone(),
                display_credential: [11; 32],
                updates,
                streams: Arc::new(tokio::sync::Semaphore::new(1)),
                voice: voice::Voice::disabled(startup_configuration::VoiceOff::Qualification),
                npc_voice: Arc::new(npc_reply::NpcVoice::silent()),
                table_log: table_log::TableLog::new(),
                social: social::SocialLedger::none(),
                living: living_scenes::LivingScenes::disabled(),
                layout: dev_control::layout::LayoutState::new(),
                portraits: creator_portraits::Portraits::disabled(),
            };
            let state = StatusState {
                service,
                token: Arc::from("0123456789abcdef0123456789abcdef"),
            };
            let request = |token: &str, body: String| {
                axum::http::Request::builder()
                    .header(TOKEN_HEADER, token)
                    .body(Body::from(body))
                    .unwrap()
            };
            let body = format!(
                "member=01010101010101010101010101010101\nrequest=02020202020202020202020202020202\nallow={}",
                u8::from(allow)
            );
            assert_eq!(
                decision(State(state.clone()), request("wrong", body.clone()))
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
            assert_eq!(
                status(State(state.clone()), HeaderMap::new())
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
            let mut headers = HeaderMap::new();
            headers.insert(TOKEN_HEADER, state.token.parse().unwrap());
            let response = status(State(state.clone()), headers.clone()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(std::str::from_utf8(&bytes).unwrap().contains(
            "rejoin\t01010101010101010101010101010101\t02020202020202020202020202020202\tBrannoc"
        ));
            assert_eq!(
                decision(State(state.clone()), request(&state.token, body.clone()))
                    .await
                    .status(),
                StatusCode::OK
            );
            assert_eq!(
                decision(State(state.clone()), request(&state.token, body))
                    .await
                    .status(),
                StatusCode::CONFLICT
            );
            let response = status(State(state), headers).await;
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(!std::str::from_utf8(&bytes).unwrap().contains("rejoin\t"));
            actor.stop().unwrap();
            owner.join().unwrap().unwrap();
        }
    }

    #[test]
    fn the_summary_lists_phase_scene_and_every_hero_on_one_line_each() {
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView {
                        phase: rpc::JourneyPhase::Dialogue as i32,
                        adventure_title: "The Broken Seal".to_owned(),
                        party: vec![
                            rpc::PartyMemberView {
                                name: "Vix Ashlantern".to_owned(),
                                character_ready: true,
                                hit_points: 13,
                                maximum_hit_points: 13,
                                portrait: "/gameplay/imagery/abc".to_owned(),
                                ..Default::default()
                            },
                            rpc::PartyMemberView {
                                name: "Mossgrave\tthe\nGreen".to_owned(),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    }),
                    scene: Some(rpc::GameplayScene {
                        title: "The Lantern Wharf".to_owned(),
                        place: "salt quay".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert_eq!(
            summary(&view),
            "phase\tdialogue\nadventure\tThe Broken Seal\n\
             hero\t1\t13\t13\t/gameplay/imagery/abc\tVix Ashlantern\n\
             hero\t0\t0\t0\t\tMossgrave the Green\n\
             scene\tThe Lantern Wharf\nplace\tsalt quay"
        );
    }

    #[test]
    fn only_the_exact_token_is_accepted() {
        let token = b"0123456789abcdef0123456789abcdef";
        assert!(same_token(token, token));
        assert!(!same_token(b"0123456789abcdef0123456789abcdeX", token));
        assert!(!same_token(b"0123456789abcdef", token));
        assert!(!same_token(b"", token));
    }

    #[test]
    fn host_decision_requires_exact_bounded_fields() {
        let valid = b"member=01010101010101010101010101010101\nrequest=02020202020202020202020202020202\nallow=1";
        assert_eq!(parse_decision(valid), Some(([1; 16], [2; 16], true)));
        assert_eq!(
            parse_decision(b"member=01010101010101010101010101010101\nallow=1"),
            None
        );
        assert_eq!(parse_decision(b"member=01010101010101010101010101010101\nrequest=02020202020202020202020202020202\nallow=2"), None);
        assert_eq!(parse_decision(b"member=01010101010101010101010101010101\nrequest=02020202020202020202020202020202\nallow=1\nmember=bad"), None);
    }

    #[test]
    fn pending_names_cannot_add_status_fields() {
        let view = rpc::ViewMessage::default();
        let pending = vec![super::super::room::PendingRejoin {
            member: [1; 16],
            operation: [2; 16],
            name: "Brannoc\nrejoin\tforged".to_owned(),
        }];
        assert_eq!(
            summary_with_rejoins(&view, &pending),
            "rejoin\t01010101010101010101010101010101\t02020202020202020202020202020202\tBrannoc rejoin forged"
        );
    }
}
