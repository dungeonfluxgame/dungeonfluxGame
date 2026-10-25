//! Short-lived media capabilities bind one asset to one gameplay principal.
//!
//! This module signs only server-selected URLs; it never reads media or decides audience policy.
use axum::{
    http::{HeaderMap, Uri, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAXIMUM_LIFETIME_SECONDS: u64 = 15 * 60;
const KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 32;
const MAXIMUM_ASSET_KEY_BYTES: usize = 256;
const MAXIMUM_TOKEN_BYTES: usize =
    2 * (2 + KEY_BYTES + 8 + 2 + MAXIMUM_ASSET_KEY_BYTES + SIGNATURE_BYTES);
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Audience {
    Seat = 1,
    Display = 2,
    Table = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Verified {
    pub(super) remaining: Duration,
}

static SECRET: OnceLock<Option<[u8; KEY_BYTES]>> = OnceLock::new();

fn secret() -> Option<&'static [u8; KEY_BYTES]> {
    SECRET
        .get_or_init(|| {
            let mut key = [0; KEY_BYTES];
            getrandom::getrandom(&mut key).ok().map(|()| key)
        })
        .as_ref()
}

pub(super) fn sign(
    key: &str,
    audience: Audience,
    principal: &[u8; KEY_BYTES],
    expires_at_unix_seconds: u64,
) -> Option<String> {
    if key.len() > MAXIMUM_ASSET_KEY_BYTES || key.bytes().any(|byte| !byte.is_ascii()) {
        return None;
    }
    let now = unix_seconds()?;
    if expires_at_unix_seconds <= now || expires_at_unix_seconds - now > MAXIMUM_LIFETIME_SECONDS {
        return None;
    }
    encode_token(key, audience, principal, expires_at_unix_seconds)
}

fn encode_token(
    key: &str,
    audience: Audience,
    principal: &[u8; KEY_BYTES],
    expires_at_unix_seconds: u64,
) -> Option<String> {
    if key.len() > MAXIMUM_ASSET_KEY_BYTES || key.bytes().any(|byte| !byte.is_ascii()) {
        return None;
    }
    let secret = secret()?;
    let mut message = Vec::with_capacity(44 + key.len());
    message.push(VERSION);
    message.push(audience as u8);
    message.extend_from_slice(principal);
    message.extend_from_slice(&expires_at_unix_seconds.to_be_bytes());
    message.extend_from_slice(&(key.len() as u16).to_be_bytes());
    message.extend_from_slice(key.as_bytes());
    let signature = hmac_sha256(secret, &message);
    message.extend_from_slice(&signature);
    Some(hex_encode(&message))
}

pub(super) fn verify(
    token: Option<&str>,
    key: &str,
    expected_audience: Audience,
    credential: Option<&[u8; KEY_BYTES]>,
) -> Option<Verified> {
    let decoded = token.and_then(hex_decode);
    let secret = secret();
    let valid_signature = match (&decoded, secret) {
        (Some(bytes), Some(secret)) if bytes.len() >= 2 + KEY_BYTES + 8 + 2 + SIGNATURE_BYTES => {
            let (message, supplied) = bytes.split_at(bytes.len() - SIGNATURE_BYTES);
            constant_time_equal(&hmac_sha256(secret, message), supplied)
        }
        _ => {
            if let Some(secret) = secret {
                let _ = hmac_sha256(secret, &[]);
            }
            false
        }
    };
    let bytes = decoded?;
    if !valid_signature {
        return None;
    }
    let message_length = bytes.len().checked_sub(SIGNATURE_BYTES)?;
    let message = &bytes[..message_length];
    let mut cursor = 0;
    if take(message, &mut cursor, 1)?[0] != VERSION {
        return None;
    }
    let audience = match take(message, &mut cursor, 1)?[0] {
        1 => Audience::Seat,
        2 => Audience::Display,
        3 => Audience::Table,
        _ => return None,
    };
    let principal: [u8; KEY_BYTES] = take(message, &mut cursor, KEY_BYTES)?.try_into().ok()?;
    let expires_at = u64::from_be_bytes(take(message, &mut cursor, 8)?.try_into().ok()?);
    let key_length = u16::from_be_bytes(take(message, &mut cursor, 2)?.try_into().ok()?) as usize;
    let signed_key = take(message, &mut cursor, key_length)?;
    if cursor != message.len()
        || audience != expected_audience
        || signed_key != key.as_bytes()
        || credential.is_some_and(|credential| !constant_time_equal(credential, &principal))
    {
        return None;
    }
    let remaining_seconds = expires_at.checked_sub(unix_seconds()?)?;
    if remaining_seconds == 0 || remaining_seconds > MAXIMUM_LIFETIME_SECONDS {
        return None;
    }
    Some(Verified {
        remaining: Duration::from_secs(remaining_seconds),
    })
}

pub(super) fn verify_request(headers: &HeaderMap, uri: &Uri, key: &str) -> Option<Verified> {
    verify_request_for(
        headers,
        uri,
        key,
        &[Audience::Seat, Audience::Display, Audience::Table],
    )
}

pub(super) fn verify_seat_request(headers: &HeaderMap, uri: &Uri, key: &str) -> Option<Verified> {
    verify_request_for(headers, uri, key, &[Audience::Seat])
}

fn verify_request_for(
    headers: &HeaderMap,
    uri: &Uri,
    key: &str,
    audiences: &[Audience],
) -> Option<Verified> {
    let token = uri.query().unwrap_or_default().split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == "g").then_some(value)
    });
    let (credential, malformed_credential) = match headers.get("x-df-local-binding") {
        Some(value) => match value.to_str().ok().and_then(parse_principal) {
            Some(credential) => (Some(credential), false),
            None => (None, true),
        },
        None => (None, false),
    };
    let verified = audiences.into_iter().fold(None, |accepted, audience| {
        let verified = verify(token, key, *audience, credential.as_ref());
        accepted.or(verified)
    });
    if malformed_credential { None } else { verified }
}

pub(super) fn concealed_not_found() -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        [(header::REFERRER_POLICY, "no-referrer")],
    )
        .into_response()
}

pub(super) fn secure_media_response(mut response: Response, remaining: Duration) -> Response {
    let seconds = remaining.as_secs().max(1);
    if let Ok(value) = format!("private, max-age={seconds}").parse() {
        response.headers_mut().insert(header::CACHE_CONTROL, value);
    }
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}

pub(super) fn public_media_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}

pub(super) fn sign_gameplay_url(
    value: &mut String,
    prefix: &str,
    audience: Audience,
    principal: &[u8; KEY_BYTES],
) {
    let Some(path) = value.strip_prefix(prefix) else {
        return;
    };
    let key = path.split_once('?').map_or(path, |(key, _)| key);
    if key.contains(['/', '#']) || key.is_empty() {
        return;
    }
    let Some(expiry) = unix_seconds().and_then(|now| now.checked_add(MAXIMUM_LIFETIME_SECONDS))
    else {
        value.clear();
        return;
    };
    let Some(token) = sign(key, audience, principal, expiry) else {
        value.clear();
        return;
    };
    *value = format!("{prefix}{key}?g={token}");
}

fn sign_image_mask_url(value: &mut String, audience: Audience, principal: &[u8; KEY_BYTES]) {
    let Some(path) = value.strip_prefix("/gameplay/imagery/") else {
        return;
    };
    let Some((key, suffix)) = path.split_once('/') else {
        return;
    };
    let Some(expiry) = unix_seconds().and_then(|now| now.checked_add(MAXIMUM_LIFETIME_SECONDS))
    else {
        value.clear();
        return;
    };
    let Some(token) = sign(key, audience, principal, expiry) else {
        value.clear();
        return;
    };
    *value = format!("/gameplay/imagery/{key}/{suffix}?g={token}");
}

pub(super) fn sign_view(
    message: &mut crate::gameplay::rpc::ViewMessage,
    display: bool,
    principal: &[u8; KEY_BYTES],
) {
    let audience = if display {
        Audience::Display
    } else {
        Audience::Seat
    };
    let Some(view) = message.audience.as_mut() else {
        return;
    };
    match view {
        crate::gameplay::rpc::view_message::Audience::Player(view) => {
            if let Some(scene) = view.scene.as_mut() {
                sign_scene(scene, audience, principal);
            }
            if let Some(journey) = view.journey.as_mut() {
                sign_journey(journey, audience, principal);
            }
        }
        crate::gameplay::rpc::view_message::Audience::Display(view) => {
            sign_gameplay_url(
                &mut view.scene_asset,
                "/gameplay/imagery/",
                audience,
                principal,
            );
            if let Some(scene) = view.scene.as_mut() {
                sign_scene(scene, audience, principal);
            }
            if let Some(journey) = view.journey.as_mut() {
                sign_journey(journey, audience, principal);
            }
            if let Some(graph) = view.scene_graph.as_mut() {
                if let Some(set) = graph.set.as_mut() {
                    sign_gameplay_url(&mut set.asset, "/gameplay/imagery/", audience, principal);
                    sign_gameplay_url(
                        &mut set.fallback_asset,
                        "/gameplay/imagery/",
                        audience,
                        principal,
                    );
                }
                for actor in &mut graph.actors {
                    sign_gameplay_url(&mut actor.art, "/gameplay/imagery/", audience, principal);
                    sign_gameplay_url(
                        &mut actor.fallback_art,
                        "/gameplay/imagery/",
                        audience,
                        principal,
                    );
                }
            }
        }
    }
}

fn sign_scene(
    scene: &mut crate::gameplay::rpc::GameplayScene,
    audience: Audience,
    principal: &[u8; KEY_BYTES],
) {
    for value in [
        &mut scene.scene_asset,
        &mut scene.speaker_portrait,
        &mut scene.live_backdrop,
        &mut scene.live_plate,
    ] {
        sign_gameplay_url(value, "/gameplay/imagery/", audience, principal);
        sign_gameplay_url(value, "/gameplay/creator-portrait/", audience, principal);
    }
    sign_gameplay_url(
        &mut scene.scene_video_path,
        "/gameplay/home-video/",
        audience,
        principal,
    );
    for actor in &mut scene.cast {
        for value in [
            &mut actor.sprite,
            &mut actor.portrait,
            &mut actor.live_sprite,
        ] {
            sign_gameplay_url(value, "/gameplay/imagery/", audience, principal);
            sign_gameplay_url(value, "/gameplay/creator-portrait/", audience, principal);
        }
    }
    if let Some(layout) = scene.plate_layout.as_mut() {
        for mask in &mut layout.figure_masks {
            sign_image_mask_url(&mut mask.mask_url, audience, principal);
        }
    }
}

fn sign_journey(
    journey: &mut crate::gameplay::rpc::JourneyView,
    audience: Audience,
    principal: &[u8; KEY_BYTES],
) {
    sign_gameplay_url(
        &mut journey.home_video_path,
        "/gameplay/home-video/",
        audience,
        principal,
    );
    for member in &mut journey.party {
        sign_gameplay_url(
            &mut member.portrait,
            "/gameplay/imagery/",
            audience,
            principal,
        );
        sign_gameplay_url(
            &mut member.portrait,
            "/gameplay/creator-portrait/",
            audience,
            principal,
        );
    }
    if let Some(character) = journey.own_character.as_mut() {
        sign_gameplay_url(
            &mut character.portrait_uri,
            "/gameplay/imagery/",
            audience,
            principal,
        );
        sign_gameplay_url(
            &mut character.portrait_uri,
            "/gameplay/creator-portrait/",
            audience,
            principal,
        );
    }
    if let Some(combat) = journey.combat.as_mut() {
        for member in &mut combat.participants {
            sign_gameplay_url(
                &mut member.portrait,
                "/gameplay/imagery/",
                audience,
                principal,
            );
            sign_gameplay_url(
                &mut member.portrait,
                "/gameplay/creator-portrait/",
                audience,
                principal,
            );
        }
        for outcome in combat
            .outcomes
            .iter_mut()
            .chain(&mut combat.move_outcomes)
            .chain(&mut combat.death_save_outcomes)
        {
            for line in &mut outcome.lines {
                sign_voice_url(&mut line.audio_url, audience, principal);
                for part in &mut line.audio_part_urls {
                    sign_voice_url(part, audience, principal);
                }
            }
        }
        for line in &mut combat.closing_lines {
            sign_voice_url(&mut line.audio_url, audience, principal);
            for part in &mut line.audio_part_urls {
                sign_voice_url(part, audience, principal);
            }
        }
    }
    if let Some(kill_shot) = journey.kill_shot.as_mut() {
        sign_gameplay_url(
            &mut kill_shot.clip_path,
            "/gameplay/home-video/",
            audience,
            principal,
        );
        sign_gameplay_url(
            &mut kill_shot.still_path,
            "/gameplay/imagery/",
            audience,
            principal,
        );
    }
    for line in &mut journey.table_log {
        sign_voice_url(&mut line.audio_url, audience, principal);
        for part in &mut line.audio_part_urls {
            sign_voice_url(part, audience, principal);
        }
    }
}

fn sign_voice_url(value: &mut String, audience: Audience, principal: &[u8; KEY_BYTES]) {
    let Some(path) = value.strip_prefix("/gameplay/voice/") else {
        return;
    };
    let (key, suffix) = match path.split_once('/') {
        Some((key, part)) => (key, format!("/{part}")),
        None => (path, String::new()),
    };
    let Some(expiry) = unix_seconds().and_then(|now| now.checked_add(MAXIMUM_LIFETIME_SECONDS))
    else {
        value.clear();
        return;
    };
    let Some(token) = sign(key, audience, principal, expiry) else {
        value.clear();
        return;
    };
    *value = format!("/gameplay/voice/{key}{suffix}?g={token}");
}

fn parse_principal(value: &str) -> Option<[u8; KEY_BYTES]> {
    if value.len() != KEY_BYTES * 2 {
        return None;
    }
    let mut principal = [0; KEY_BYTES];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        principal[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(principal)
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(length)?;
    let value = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(value)
}

fn unix_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .checked_add(1)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0; 64];
    if key.len() > block.len() {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend(block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);
    let inner_hash = Sha256::digest(&inner);
    let mut outer = Vec::with_capacity(96);
    outer.extend(block.iter().map(|byte| byte ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    Sha256::digest(outer).into()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = (left.len() ^ right.len()) as u8;
    for index in 0..left.len().max(right.len()) {
        difference |= left.get(index).copied().unwrap_or_default()
            ^ right.get(index).copied().unwrap_or_default();
    }
    difference == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() > MAXIMUM_TOKEN_BYTES || value.len() % 2 != 0 || !value.is_ascii() {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

fn nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_grant_is_bound_to_its_audience_key_principal_and_expiry() {
        let principal = [9; KEY_BYTES];
        let expiry = unix_seconds().unwrap() + 60;
        let token = sign("asset-key", Audience::Seat, &principal, expiry).unwrap();
        assert!(verify(Some(&token), "asset-key", Audience::Seat, Some(&principal)).is_some());
        assert!(verify(Some(&token), "other-key", Audience::Seat, Some(&principal)).is_none());
        assert!(
            verify(
                Some(&token),
                "asset-key",
                Audience::Display,
                Some(&principal)
            )
            .is_none()
        );
        assert!(
            verify(
                Some(&token),
                "asset-key",
                Audience::Seat,
                Some(&[8; KEY_BYTES])
            )
            .is_none()
        );
    }

    #[test]
    fn expired_tampered_and_overlong_grants_are_refused() {
        let principal = [3; KEY_BYTES];
        let expired = encode_token(
            "asset",
            Audience::Display,
            &principal,
            unix_seconds().unwrap(),
        )
        .unwrap();
        assert!(verify(Some(&expired), "asset", Audience::Display, None).is_none());
        let expiry = unix_seconds().unwrap() + 60;
        let mut tampered = sign("asset", Audience::Display, &principal, expiry).unwrap();
        tampered.replace_range(0..2, "00");
        assert!(verify(Some(&tampered), "asset", Audience::Display, None).is_none());
        assert!(
            sign(
                "asset",
                Audience::Display,
                &principal,
                expiry + MAXIMUM_LIFETIME_SECONDS
            )
            .is_none()
        );
    }

    #[test]
    fn a_grant_accepts_its_owner_and_anonymous_bearer_but_refuses_a_mismatched_credential() {
        let owner = [4; KEY_BYTES];
        let token = sign(
            "table-asset",
            Audience::Table,
            &owner,
            unix_seconds().unwrap() + 60,
        )
        .unwrap();
        assert!(verify(Some(&token), "table-asset", Audience::Table, Some(&owner)).is_some());
        assert!(verify(Some(&token), "table-asset", Audience::Table, None).is_some());
        assert!(
            verify(
                Some(&token),
                "table-asset",
                Audience::Table,
                Some(&[5; KEY_BYTES])
            )
            .is_none()
        );
        assert!(verify(Some(&token), "table-asset", Audience::Seat, None).is_none());
        let uri: Uri = format!("/asset/table-asset?g={token}").parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-df-local-binding", "invalid".parse().unwrap());
        assert!(verify_request(&headers, &uri, "table-asset").is_none());
    }

    #[test]
    fn each_table_seat_and_display_receive_a_principal_bound_grant_for_the_same_asset() {
        let first_seat = [1; KEY_BYTES];
        let second_seat = [2; KEY_BYTES];
        let display = [3; KEY_BYTES];
        let expiry = unix_seconds().unwrap() + 60;
        let first = sign("plate-key", Audience::Seat, &first_seat, expiry).unwrap();
        let second = sign("plate-key", Audience::Seat, &second_seat, expiry).unwrap();
        let display_token = sign("plate-key", Audience::Display, &display, expiry).unwrap();
        assert!(verify(Some(&first), "plate-key", Audience::Seat, Some(&first_seat)).is_some());
        assert!(
            verify(
                Some(&second),
                "plate-key",
                Audience::Seat,
                Some(&second_seat)
            )
            .is_some()
        );
        assert!(
            verify(
                Some(&first),
                "plate-key",
                Audience::Seat,
                Some(&second_seat)
            )
            .is_none()
        );
        assert!(
            verify(
                Some(&display_token),
                "plate-key",
                Audience::Display,
                Some(&display)
            )
            .is_some()
        );
        assert!(
            verify(
                Some(&display_token),
                "plate-key",
                Audience::Seat,
                Some(&display)
            )
            .is_none()
        );
    }

    #[test]
    fn view_publication_signs_private_media_urls_for_its_recipient() {
        let principal = [7; KEY_BYTES];
        let mut view = crate::gameplay::rpc::ViewMessage {
            audience: Some(crate::gameplay::rpc::view_message::Audience::Display(
                crate::gameplay::rpc::DisplayGameplayView {
                    scene: Some(crate::gameplay::rpc::GameplayScene {
                        live_plate: "/gameplay/imagery/plate-key".to_owned(),
                        ..Default::default()
                    }),
                    journey: Some(crate::gameplay::rpc::JourneyView {
                        table_log: vec![crate::gameplay::rpc::TableLine {
                            role: crate::gameplay::rpc::TableRole::Npc as i32,
                            kind: crate::gameplay::rpc::TableKind::Speech as i32,
                            actor_id: "npc-guide".to_owned(),
                            speaker: "Guide".to_owned(),
                            text: "Welcome to the square.".to_owned(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        crate::gameplay::voice::attach_urls(&mut view);
        sign_view(&mut view, true, &principal);
        let Some(crate::gameplay::rpc::view_message::Audience::Display(display)) = view.audience
        else {
            panic!("display view retained its audience");
        };
        let path = &display.scene.unwrap().live_plate;
        let uri: Uri = path.parse().unwrap();
        assert!(verify_request(&HeaderMap::new(), &uri, "plate-key").is_some());
        let line = &display.journey.unwrap().table_log[0];
        let voice_uri: Uri = line.audio_url.parse().unwrap();
        let key = crate::voice_line::VoiceLine {
            role: "npc-id:npc-guide|Guide".to_owned(),
            text: "Welcome to the square.".to_owned(),
        }
        .key();
        assert!(verify_request(&HeaderMap::new(), &voice_uri, &key).is_some());
        let mut mismatched = HeaderMap::new();
        mismatched.insert(
            "x-df-local-binding",
            "08".repeat(KEY_BYTES).parse().unwrap(),
        );
        assert!(verify_request(&mismatched, &uri, "plate-key").is_none());
    }

    #[tokio::test]
    async fn unsigned_and_invalid_grants_share_the_concealed_not_found_response() {
        use axum::http::{Request, StatusCode};
        use tonic::codegen::Service;

        let route = || {
            axum::Router::new().route(
                "/asset/{key}",
                axum::routing::get(
                    |axum::extract::Path(key): axum::extract::Path<String>,
                     uri: Uri,
                     headers: HeaderMap| async move {
                        verify_request(&headers, &uri, &key)
                            .map(|grant| {
                                secure_media_response(
                                    StatusCode::OK.into_response(),
                                    grant.remaining,
                                )
                            })
                            .unwrap_or_else(concealed_not_found)
                    },
                ),
            )
        };
        let mut router = route();
        let mut results = Vec::new();
        for path in ["/asset/key", "/asset/key?g=bad"] {
            let request = Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap();
            let response = Service::call(&mut router, request).await.unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            results.push((status, body));
        }
        assert_eq!(results[0].0, StatusCode::NOT_FOUND);
        assert_eq!(results[0], results[1]);
    }
}
