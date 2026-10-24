//! What the desktop launcher shows about the running table (REF-064): the phase, adventure, scene
//! and party, read from the same display view the TV receives. The route exists only when the
//! launcher that started this server handed it a token (`DF_GAMEPLAY_LAUNCHER_TOKEN`), and any
//! request without that exact token gets 404, so phones on the network learn nothing from it.
//!
//! The answer is plain lines of tab-separated fields, which the launcher reads without a JSON
//! library: `phase`, `adventure`, `scene` and `place` lines, then one `hero` line per party member
//! (`hero <ready 0|1> <hit points> <maximum> <portrait path> <name>`).

use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_protocol::common as rpc;

const TOKEN_VARIABLE: &str = "DF_GAMEPLAY_LAUNCHER_TOKEN";
const TOKEN_HEADER: &str = "x-dungeonflux-launcher";
/// The launcher's token is 32 random bytes in hex; anything shorter is refused at startup.
const MINIMUM_TOKEN_BYTES: usize = 32;
/// The display role's watch binding: the owner accepts no other for the display credential
/// (`actor::watch_scope_identity`).
const STATUS_BINDING: [u8; 16] = [0x72; 16];

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
            summary(&view),
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
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
}
