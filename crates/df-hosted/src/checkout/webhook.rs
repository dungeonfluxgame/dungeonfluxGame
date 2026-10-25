//! Stripe signatures cover the raw bytes; only verified paid sessions can reach finance.

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, Uri},
    response::Redirect,
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use super::Checkout;
use super::provider::Error;

pub(crate) fn router(checkout: Checkout) -> Router {
    Router::new()
        .route("/app/api/payments/stripe/webhook", post(receive))
        .route("/app/funds", get(return_page))
        .layer(DefaultBodyLimit::max(262_144))
        .with_state(checkout)
}

async fn return_page(uri: Uri) -> Result<Redirect, StatusCode> {
    let operation = uri
        .query()
        .and_then(|query| {
            query
                .split('&')
                .find_map(|field| field.strip_prefix("checkout="))
        })
        .ok_or(StatusCode::BAD_REQUEST)?;
    let operation =
        df_types::CheckoutId::from_hex(operation).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Redirect::to(&format!(
        "/app/credit/return?checkout={}",
        df_commerce::finance_ids::hex(operation.as_bytes())
    )))
}

#[derive(Deserialize)]
struct Event {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
pub(super) struct PaidSession {
    pub(super) id: String,
    pub(super) amount_total: i64,
    pub(super) currency: String,
    pub(super) livemode: bool,
    pub(super) mode: String,
    pub(super) metadata: Metadata,
}

#[derive(Deserialize)]
pub(super) struct Metadata {
    pub(super) operation_id: String,
}

async fn receive(State(checkout): State<Checkout>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(stripe) = &checkout.stripe else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let now_s = df_customer::Clock::now_ms(&df_customer::SystemClock) / 1000;
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok());
    if !signature.is_some_and(|signature| {
        verify(
            &stripe.provider.settings.webhook_secret,
            signature,
            &body,
            now_s,
        )
    }) {
        df_observe::record_audit_line(
            "hosted.stripe_signature_refused",
            "SPEND stripe_refused error=signature".to_owned(),
        );
        return StatusCode::BAD_REQUEST;
    }
    let Ok(event) = serde_json::from_slice::<Event>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if event.kind != "checkout.session.completed" {
        return StatusCode::OK;
    }
    let Some(object) = event.data.get("object") else {
        return StatusCode::BAD_REQUEST;
    };
    if object
        .get("payment_status")
        .and_then(serde_json::Value::as_str)
        != Some("paid")
    {
        return StatusCode::OK;
    }
    if event.id.is_empty() || event.id.len() > 255 {
        return StatusCode::BAD_REQUEST;
    }
    let Ok(session) = serde_json::from_value::<PaidSession>(object.clone()) else {
        return StatusCode::BAD_REQUEST;
    };
    match stripe.credit(&event.id, session, &checkout.finance).await {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            error.audit();
            match error {
                Error::Mismatch => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            }
        }
    }
}

fn verify(secret: &str, header: &str, body: &[u8], now_s: u64) -> bool {
    let mut timestamp = None;
    for field in header.split(',') {
        if let Some(value) = field.strip_prefix("t=") {
            if timestamp.is_some() {
                return false;
            }
            timestamp = Some(value);
        }
    }
    let Some(timestamp) = timestamp else {
        return false;
    };
    let Ok(timestamp_s) = timestamp.parse::<u64>() else {
        return false;
    };
    if now_s.abs_diff(timestamp_s) > 300 {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    header
        .split(',')
        .filter_map(|field| field.strip_prefix("v1="))
        .any(|signature| {
            let Some(signature) = df_commerce::finance_ids::unhex(signature) else {
                return false;
            };
            mac.clone().verify_slice(&signature).is_ok()
        })
}

#[cfg(test)]
pub(super) fn signature(secret: &str, body: &[u8], timestamp: u64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "t={timestamp},v1={}",
        df_commerce::finance_ids::hex(&mac.finalize().into_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_cover_raw_bytes_accept_rotation_and_reject_stale_or_future_deliveries() {
        let body = br#"{ "fixture": true }"#;
        let signed = signature("whsec_fixture", body, 1_000);
        assert!(verify("whsec_fixture", &signed, body, 1_300));
        assert!(verify(
            "whsec_fixture",
            &format!("v1={},{}", "00".repeat(32), signed),
            body,
            1_000
        ));
        assert!(!verify("whsec_fixture", &signed, body, 1_301));
        assert!(!verify("whsec_fixture", &signed, body, 699));
        assert!(!verify("whsec_fixture", &signed, b"{}", 1_000));
        assert!(!verify("whsec_wrong", &signed, body, 1_000));
        assert!(!verify(
            "whsec_fixture",
            &signed.replace("t=1000", "t=01000"),
            body,
            1_000
        ));
        assert!(!verify(
            "whsec_fixture",
            &format!("t=1000,{signed}"),
            body,
            1_000
        ));
    }
}
