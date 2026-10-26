//! OIDC Core 3.1.3.7: trusted token-endpoint TLS authenticates these directly received claims.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use df_customer::{EmailAddress, OAuthExchange, OAuthFailure, OAuthIdentity};
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// Google token lifetime and clock skew are bounded even though claims arrive over trusted TLS.
const MAXIMUM_TOKEN_AGE_S: u64 = 3600;
const CLOCK_SKEW_S: u64 = 60;
const FRESH_AUTHENTICATION_S: u64 = 300;

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    #[serde(default, deserialize_with = "authorized_party")]
    azp: Option<String>,
    exp: u64,
    iat: u64,
    nonce: String,
    email: String,
    email_verified: bool,
    auth_time: Option<u64>,
}

fn authorized_party<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

pub(super) fn validate(
    token: &str,
    client_id: &str,
    request: &OAuthExchange<'_>,
) -> Result<OAuthIdentity, OAuthFailure> {
    let mut segments = token.split('.');
    let header = segments.next().filter(|part| !part.is_empty());
    let payload = segments.next().ok_or(OAuthFailure::Malformed)?;
    let signature = segments.next().filter(|part| !part.is_empty());
    if header.is_none() || signature.is_none() || segments.next().is_some() {
        return Err(OAuthFailure::Malformed);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| OAuthFailure::Malformed)?;
    let claims: Claims = serde_json::from_slice(&bytes).map_err(|_| OAuthFailure::Malformed)?;
    if !matches!(
        claims.iss.as_str(),
        "https://accounts.google.com" | "accounts.google.com"
    ) || claims.aud != client_id
        || claims
            .azp
            .as_deref()
            .is_some_and(|party| party != client_id)
        || claims.exp <= request.now_s
        || claims.iat > request.now_s.saturating_add(CLOCK_SKEW_S)
        || claims.iat >= claims.exp
        || request.now_s.saturating_sub(claims.iat) > MAXIMUM_TOKEN_AGE_S
        || claims.sub.is_empty()
        || claims.sub.len() > 256
        || claims.sub.chars().any(char::is_control)
        || !bool::from(
            claims
                .nonce
                .as_bytes()
                .ct_eq(request.nonce.expose().as_bytes()),
        )
    {
        return Err(OAuthFailure::Rejected);
    }
    if !claims.email_verified {
        return Err(OAuthFailure::EmailNotVerified);
    }
    if request.require_fresh
        && !claims.auth_time.is_some_and(|time| {
            time <= request.now_s && request.now_s.saturating_sub(time) <= FRESH_AUTHENTICATION_S
        })
    {
        return Err(OAuthFailure::FreshAuthenticationRequired);
    }
    Ok(OAuthIdentity {
        subject: claims.sub,
        email: EmailAddress::parse(&claims.email).map_err(|_| OAuthFailure::Malformed)?,
        authenticated_at_s: claims.auth_time,
    })
}
