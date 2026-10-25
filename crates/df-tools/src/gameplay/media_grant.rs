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
const PRINCIPAL_ID_BYTES: usize = 16;
const SIGNATURE_BYTES: usize = 32;
const MAXIMUM_ASSET_KEY_BYTES: usize = 256;
const MAXIMUM_TOKEN_BYTES: usize =
    2 * (3 + PRINCIPAL_ID_BYTES + 8 + 2 + MAXIMUM_ASSET_KEY_BYTES + SIGNATURE_BYTES);
const VERSION: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Audience {
    Imagery = 1,
    Portrait = 2,
    HomeVideo = 3,
    Voice = 4,
    Music = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum PrincipalAudience {
    Seat = 1,
    Display = 2,
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
    principal_audience: PrincipalAudience,
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
    encode_token(
        key,
        audience,
        principal_audience,
        principal,
        expires_at_unix_seconds,
    )
}

fn encode_token(
    key: &str,
    audience: Audience,
    principal_audience: PrincipalAudience,
    principal: &[u8; KEY_BYTES],
    expires_at_unix_seconds: u64,
) -> Option<String> {
    if key.len() > MAXIMUM_ASSET_KEY_BYTES || key.bytes().any(|byte| !byte.is_ascii()) {
        return None;
    }
    let secret = secret()?;
    let mut message = Vec::with_capacity(45 + key.len());
    message.push(VERSION);
    message.push(audience as u8);
    message.push(principal_audience as u8);
    message.extend_from_slice(&principal_id(secret, principal));
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
        (Some(bytes), Some(secret))
            if bytes.len() >= 3 + PRINCIPAL_ID_BYTES + 8 + 2 + SIGNATURE_BYTES =>
        {
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
    let signing_secret = secret?;
    let message_length = bytes.len().checked_sub(SIGNATURE_BYTES)?;
    let message = &bytes[..message_length];
    let mut cursor = 0;
    if take(message, &mut cursor, 1)?[0] != VERSION {
        return None;
    }
    let audience = match take(message, &mut cursor, 1)?[0] {
        1 => Audience::Imagery,
        2 => Audience::Portrait,
        3 => Audience::HomeVideo,
        4 => Audience::Voice,
        5 => Audience::Music,
        _ => return None,
    };
    match take(message, &mut cursor, 1)?[0] {
        1..=2 => {}
        _ => return None,
    }
    let encoded_principal_id = take(message, &mut cursor, PRINCIPAL_ID_BYTES)?;
    let expires_at = u64::from_be_bytes(take(message, &mut cursor, 8)?.try_into().ok()?);
    let key_length = u16::from_be_bytes(take(message, &mut cursor, 2)?.try_into().ok()?) as usize;
    let signed_key = take(message, &mut cursor, key_length)?;
    if cursor != message.len()
        || audience != expected_audience
        || signed_key != key.as_bytes()
        || credential.is_some_and(|credential| {
            !constant_time_equal(
                encoded_principal_id,
                &principal_id(signing_secret, credential),
            )
        })
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

/// Validates a route-specific grant and compares any supplied bearer watch credential.
/// AUTH-03 allows anonymous media fetches because each short-lived grant is itself a bearer capability.
pub(super) fn verify_request(
    headers: &HeaderMap,
    uri: &Uri,
    key: &str,
    expected_audience: Audience,
) -> Option<Verified> {
    let token = uri.query().unwrap_or_default().split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == "g").then_some(value)
    });
    let (credential, malformed_credential) = match headers.get(header::AUTHORIZATION) {
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(parse_principal)
        {
            Some(credential) => (Some(credential), false),
            None => (None, true),
        },
        None => (None, false),
    };
    if malformed_credential {
        return None;
    }
    verify(token, key, expected_audience, credential.as_ref())
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
    principal_audience: PrincipalAudience,
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
    let audience = match prefix {
        "/gameplay/imagery/" => Audience::Imagery,
        "/gameplay/creator-portrait/" => Audience::Portrait,
        "/gameplay/home-video/" => Audience::HomeVideo,
        _ => return,
    };
    let Some(token) = sign(key, audience, principal_audience, principal, expiry) else {
        value.clear();
        return;
    };
    *value = format!("{prefix}{key}?g={token}");
}

fn sign_image_mask_url(
    value: &mut String,
    principal_audience: PrincipalAudience,
    principal: &[u8; KEY_BYTES],
) {
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
    let Some(token) = sign(
        key,
        Audience::Imagery,
        principal_audience,
        principal,
        expiry,
    ) else {
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
    let principal_audience = if display {
        PrincipalAudience::Display
    } else {
        PrincipalAudience::Seat
    };
    let Some(view) = message.audience.as_mut() else {
        return;
    };
    match view {
        crate::gameplay::rpc::view_message::Audience::Player(view) => {
            if let Some(scene) = view.scene.as_mut() {
                sign_scene(scene, principal_audience, principal);
            }
            if let Some(journey) = view.journey.as_mut() {
                sign_journey(journey, principal_audience, principal);
            }
        }
        crate::gameplay::rpc::view_message::Audience::Display(view) => {
            sign_gameplay_url(
                &mut view.scene_asset,
                "/gameplay/imagery/",
                principal_audience,
                principal,
            );
            if let Some(scene) = view.scene.as_mut() {
                sign_scene(scene, principal_audience, principal);
            }
            if let Some(journey) = view.journey.as_mut() {
                sign_journey(journey, principal_audience, principal);
            }
            if let Some(graph) = view.scene_graph.as_mut() {
                if let Some(set) = graph.set.as_mut() {
                    sign_gameplay_url(
                        &mut set.asset,
                        "/gameplay/imagery/",
                        principal_audience,
                        principal,
                    );
                    sign_gameplay_url(
                        &mut set.fallback_asset,
                        "/gameplay/imagery/",
                        principal_audience,
                        principal,
                    );
                }
                for actor in &mut graph.actors {
                    sign_gameplay_url(
                        &mut actor.art,
                        "/gameplay/imagery/",
                        principal_audience,
                        principal,
                    );
                    sign_gameplay_url(
                        &mut actor.fallback_art,
                        "/gameplay/imagery/",
                        principal_audience,
                        principal,
                    );
                }
            }
        }
    }
}

fn sign_scene(
    scene: &mut crate::gameplay::rpc::GameplayScene,
    principal_audience: PrincipalAudience,
    principal: &[u8; KEY_BYTES],
) {
    for value in [
        &mut scene.scene_asset,
        &mut scene.speaker_portrait,
        &mut scene.live_backdrop,
        &mut scene.live_plate,
    ] {
        sign_gameplay_url(value, "/gameplay/imagery/", principal_audience, principal);
        sign_gameplay_url(
            value,
            "/gameplay/creator-portrait/",
            principal_audience,
            principal,
        );
    }
    sign_gameplay_url(
        &mut scene.scene_video_path,
        "/gameplay/home-video/",
        principal_audience,
        principal,
    );
    for actor in &mut scene.cast {
        for value in [
            &mut actor.sprite,
            &mut actor.portrait,
            &mut actor.live_sprite,
        ] {
            sign_gameplay_url(value, "/gameplay/imagery/", principal_audience, principal);
            sign_gameplay_url(
                value,
                "/gameplay/creator-portrait/",
                principal_audience,
                principal,
            );
        }
    }
    if let Some(layout) = scene.plate_layout.as_mut() {
        for mask in &mut layout.figure_masks {
            sign_image_mask_url(&mut mask.mask_url, principal_audience, principal);
        }
    }
}

fn sign_journey(
    journey: &mut crate::gameplay::rpc::JourneyView,
    principal_audience: PrincipalAudience,
    principal: &[u8; KEY_BYTES],
) {
    sign_gameplay_url(
        &mut journey.home_video_path,
        "/gameplay/home-video/",
        principal_audience,
        principal,
    );
    for member in &mut journey.party {
        sign_gameplay_url(
            &mut member.portrait,
            "/gameplay/imagery/",
            principal_audience,
            principal,
        );
        sign_gameplay_url(
            &mut member.portrait,
            "/gameplay/creator-portrait/",
            principal_audience,
            principal,
        );
    }
    if let Some(character) = journey.own_character.as_mut() {
        sign_gameplay_url(
            &mut character.portrait_uri,
            "/gameplay/imagery/",
            principal_audience,
            principal,
        );
        sign_gameplay_url(
            &mut character.portrait_uri,
            "/gameplay/creator-portrait/",
            principal_audience,
            principal,
        );
    }
    if let Some(combat) = journey.combat.as_mut() {
        for member in &mut combat.participants {
            sign_gameplay_url(
                &mut member.portrait,
                "/gameplay/imagery/",
                principal_audience,
                principal,
            );
            sign_gameplay_url(
                &mut member.portrait,
                "/gameplay/creator-portrait/",
                principal_audience,
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
                sign_voice_url(&mut line.audio_url, principal_audience, principal);
                for part in &mut line.audio_part_urls {
                    sign_voice_url(part, principal_audience, principal);
                }
            }
        }
        for line in &mut combat.closing_lines {
            sign_voice_url(&mut line.audio_url, principal_audience, principal);
            for part in &mut line.audio_part_urls {
                sign_voice_url(part, principal_audience, principal);
            }
        }
    }
    if let Some(kill_shot) = journey.kill_shot.as_mut() {
        sign_gameplay_url(
            &mut kill_shot.clip_path,
            "/gameplay/home-video/",
            principal_audience,
            principal,
        );
        sign_gameplay_url(
            &mut kill_shot.still_path,
            "/gameplay/imagery/",
            principal_audience,
            principal,
        );
    }
    for line in &mut journey.table_log {
        sign_voice_url(&mut line.audio_url, principal_audience, principal);
        for part in &mut line.audio_part_urls {
            sign_voice_url(part, principal_audience, principal);
        }
    }
}

fn sign_voice_url(
    value: &mut String,
    principal_audience: PrincipalAudience,
    principal: &[u8; KEY_BYTES],
) {
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
    let Some(token) = sign(key, Audience::Voice, principal_audience, principal, expiry) else {
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

fn principal_id(secret: &[u8], principal: &[u8; KEY_BYTES]) -> [u8; PRINCIPAL_ID_BYTES] {
    let mut message = b"df-media-principal-v2".to_vec();
    message.extend_from_slice(principal);
    let digest = hmac_sha256(secret, &message);
    let mut identifier = [0; PRINCIPAL_ID_BYTES];
    identifier.copy_from_slice(&digest[..PRINCIPAL_ID_BYTES]);
    identifier
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

    fn grant(
        audience: Audience,
        principal_audience: PrincipalAudience,
        principal: &[u8; KEY_BYTES],
    ) -> String {
        sign(
            "asset-key",
            audience,
            principal_audience,
            principal,
            unix_seconds().unwrap() + 60,
        )
        .unwrap()
    }

    #[test]
    fn a_signed_grant_is_bound_to_its_route_audience_key_principal_and_expiry() {
        let principal = [9; KEY_BYTES];
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &principal);
        assert!(
            verify(
                Some(&token),
                "asset-key",
                Audience::Imagery,
                Some(&principal)
            )
            .is_some()
        );
        assert!(
            verify(
                Some(&token),
                "other-key",
                Audience::Imagery,
                Some(&principal)
            )
            .is_none()
        );
        assert!(verify(Some(&token), "asset-key", Audience::Voice, Some(&principal)).is_none());
        assert!(
            verify(
                Some(&token),
                "asset-key",
                Audience::Imagery,
                Some(&[8; KEY_BYTES])
            )
            .is_none()
        );
    }

    #[test]
    fn a_token_does_not_contain_its_principal_credential() {
        let principal = [0x5a; KEY_BYTES];
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &principal);
        let decoded = hex_decode(&token).unwrap();
        assert!(
            !decoded
                .windows(principal.len())
                .any(|bytes| bytes == principal)
        );
    }

    #[test]
    fn a_token_minted_for_one_principal_fails_for_another() {
        let owner = [0x31; KEY_BYTES];
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &owner);
        assert!(verify(Some(&token), "asset-key", Audience::Imagery, Some(&owner)).is_some());
        assert!(
            verify(
                Some(&token),
                "asset-key",
                Audience::Imagery,
                Some(&[0x32; KEY_BYTES])
            )
            .is_none()
        );
    }

    #[test]
    fn a_validly_signed_old_version_token_is_refused() {
        let principal = [0x43; KEY_BYTES];
        let expiry = unix_seconds().unwrap() + 60;
        let key = "asset-key";
        let mut message = Vec::with_capacity(44 + key.len());
        message.push(VERSION - 1);
        message.push(Audience::Imagery as u8);
        message.extend_from_slice(&principal_id(secret().unwrap(), &principal));
        message.extend_from_slice(&expiry.to_be_bytes());
        message.extend_from_slice(&(key.len() as u16).to_be_bytes());
        message.extend_from_slice(key.as_bytes());
        let signature = hmac_sha256(secret().unwrap(), &message);
        message.extend_from_slice(&signature);
        let old_token = hex_encode(&message);
        assert!(verify(Some(&old_token), key, Audience::Imagery, Some(&principal)).is_none());
    }

    #[test]
    fn expired_tampered_and_overlong_grants_are_refused() {
        let principal = [3; KEY_BYTES];
        let expired = encode_token(
            "asset-key",
            Audience::Imagery,
            PrincipalAudience::Display,
            &principal,
            unix_seconds().unwrap(),
        )
        .unwrap();
        assert!(verify(Some(&expired), "asset-key", Audience::Imagery, None).is_none());
        let mut tampered = grant(Audience::Imagery, PrincipalAudience::Display, &principal);
        tampered.replace_range(0..2, "00");
        assert!(verify(Some(&tampered), "asset-key", Audience::Imagery, None).is_none());
        let overlong = "a".repeat(MAXIMUM_TOKEN_BYTES + 1);
        assert!(
            verify(
                Some(overlong.as_str()),
                "asset-key",
                Audience::Imagery,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn a_grant_is_restricted_to_the_route_audience_it_was_minted_for() {
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &[9; KEY_BYTES]);
        assert!(verify(Some(&token), "asset-key", Audience::Imagery, None).is_some());
        assert!(verify(Some(&token), "asset-key", Audience::Voice, None).is_none());
    }

    #[test]
    fn bearer_only_requests_accept_an_unexpired_grant_and_copied_token_headers_have_no_effect() {
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &[7; KEY_BYTES]);
        let uri: Uri = format!("/asset/asset-key?g={token}").parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-df-local-binding", token.parse().unwrap());
        assert!(verify_request(&headers, &uri, "asset-key", Audience::Imagery).is_some());
    }

    #[test]
    fn a_present_watch_credential_must_match_the_grant_principal() {
        let principal = [4; KEY_BYTES];
        let token = grant(Audience::Imagery, PrincipalAudience::Seat, &principal);
        let uri: Uri = format!("/asset/asset-key?g={token}").parse().unwrap();
        let mut matching = HeaderMap::new();
        matching.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", hex_encode(&principal))
                .parse()
                .unwrap(),
        );
        assert!(verify_request(&matching, &uri, "asset-key", Audience::Imagery).is_some());
        let mut mismatched = HeaderMap::new();
        mismatched.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", hex_encode(&[5; KEY_BYTES]))
                .parse()
                .unwrap(),
        );
        assert!(verify_request(&mismatched, &uri, "asset-key", Audience::Imagery).is_none());
    }

    #[test]
    fn seat_and_display_grants_bind_the_same_asset_to_different_credentials() {
        let seat = [1; KEY_BYTES];
        let display = [2; KEY_BYTES];
        let seat_token = grant(Audience::Imagery, PrincipalAudience::Seat, &seat);
        let display_token = grant(Audience::Imagery, PrincipalAudience::Display, &display);
        assert!(
            verify(
                Some(&seat_token),
                "asset-key",
                Audience::Imagery,
                Some(&seat)
            )
            .is_some()
        );
        assert!(
            verify(
                Some(&seat_token),
                "asset-key",
                Audience::Imagery,
                Some(&display)
            )
            .is_none()
        );
        assert!(
            verify(
                Some(&display_token),
                "asset-key",
                Audience::Imagery,
                Some(&display)
            )
            .is_some()
        );
        assert!(
            verify(
                Some(&display_token),
                "asset-key",
                Audience::Imagery,
                Some(&seat)
            )
            .is_none()
        );
    }

    #[test]
    fn view_publication_signs_imagery_for_its_recipient() {
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
        let uri: Uri = display.scene.unwrap().live_plate.parse().unwrap();
        assert!(verify_request(&HeaderMap::new(), &uri, "plate-key", Audience::Imagery).is_some());
        let line = &display.journey.unwrap().table_log[0];
        let voice_uri: Uri = line.audio_url.parse().unwrap();
        let voice_key = crate::voice_line::VoiceLine {
            role: "npc-id:npc-guide|Guide".to_owned(),
            text: "Welcome to the square.".to_owned(),
        }
        .key();
        assert!(
            verify_request(&HeaderMap::new(), &voice_uri, &voice_key, Audience::Voice).is_some()
        );
    }

    #[tokio::test]
    async fn unsigned_and_invalid_grants_share_the_concealed_not_found_response() {
        use axum::http::{Request, StatusCode};
        use tonic::codegen::Service;

        let mut router = axum::Router::new().route(
            "/asset/{key}",
            axum::routing::get(
                |axum::extract::Path(key): axum::extract::Path<String>,
                 uri: Uri,
                 headers: HeaderMap| async move {
                    verify_request(&headers, &uri, &key, Audience::Imagery)
                        .map(|grant| {
                            secure_media_response(StatusCode::OK.into_response(), grant.remaining)
                        })
                        .unwrap_or_else(concealed_not_found)
                },
            ),
        );
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
