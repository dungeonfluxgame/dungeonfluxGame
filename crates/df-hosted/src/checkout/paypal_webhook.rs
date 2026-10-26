//! PayPal verifies every notification remotely before a capture may reach FinanceService.

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, Uri},
    response::Redirect,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;

use super::{
    Checkout,
    paypal::{Paypal, verified_capture},
    paypal_provider::Capture,
    provider::Error,
};

pub(super) fn router(checkout: Checkout) -> Router {
    Router::new()
        .route("/app/api/payments/paypal/webhook", post(receive))
        .route("/app/api/payments/paypal/return", get(return_page))
        .route("/app/api/payments/paypal/cancel", get(cancel_page))
        .layer(DefaultBodyLimit::max(262_144))
        .with_state(checkout)
}

fn query(uri: &Uri) -> Result<(df_types::CheckoutId, Option<String>), StatusCode> {
    let fields: Vec<_> = uri
        .query()
        .ok_or(StatusCode::BAD_REQUEST)?
        .split('&')
        .collect();
    let mut operations = fields
        .iter()
        .filter_map(|field| field.strip_prefix("checkout="));
    let operation = operations.next().ok_or(StatusCode::BAD_REQUEST)?;
    if operations.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let operation =
        df_types::CheckoutId::from_hex(operation).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut tokens = fields
        .iter()
        .filter_map(|field| field.strip_prefix("token="));
    let order = tokens.next().map(str::to_owned);
    if tokens.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((operation, order))
}

fn destination(operation: df_types::CheckoutId) -> Redirect {
    Redirect::to(&format!(
        "/app/credit/return?checkout={}",
        df_commerce::finance_ids::hex(operation.as_bytes())
    ))
}

async fn return_page(State(checkout): State<Checkout>, uri: Uri) -> Result<Redirect, StatusCode> {
    let paypal = checkout
        .paypal
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (operation, order) = query(&uri)?;
    let order = order.ok_or(StatusCode::BAD_REQUEST)?;
    if let Err(error) = paypal.complete(operation, &order, &checkout.finance).await {
        error.audit_for("paypal");
        if matches!(error, Error::Mismatch) {
            return Err(StatusCode::BAD_REQUEST);
        }
        // Unknown provider/finance outcomes remain Pending; the verified webhook can repair them.
    }
    Ok(destination(operation))
}

async fn cancel_page(State(checkout): State<Checkout>, uri: Uri) -> Result<Redirect, StatusCode> {
    let paypal = checkout
        .paypal
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (operation, _) = query(&uri)?;
    paypal.cancel(operation).await.map_err(|error| {
        error.audit_for("paypal");
        StatusCode::BAD_REQUEST
    })?;
    Ok(destination(operation))
}

#[derive(Deserialize)]
struct Event {
    id: String,
    event_type: String,
    resource: Value,
}

async fn receive(State(checkout): State<Checkout>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(paypal) = checkout.paypal.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if paypal.provider.verify(&headers, &value).await.is_err() {
        df_observe::record_audit_line(
            "hosted.paypal_signature_refused",
            "SPEND paypal_refused error=signature".to_owned(),
        );
        return StatusCode::BAD_REQUEST;
    }
    let Ok(event) = serde_json::from_value::<Event>(value) else {
        return StatusCode::BAD_REQUEST;
    };
    if event.id.is_empty() || event.id.len() > 255 {
        return StatusCode::BAD_REQUEST;
    }
    let kind = match event.event_type.as_str() {
        "PAYMENT.CAPTURE.COMPLETED" => "COMPLETED",
        "PAYMENT.CAPTURE.DENIED" => "DENIED",
        "PAYMENT.CAPTURE.REVERSED" => "REVERSED",
        _ => return StatusCode::OK,
    };
    let Ok(capture) = serde_json::from_value::<Capture>(event.resource) else {
        return StatusCode::BAD_REQUEST;
    };
    let result = if kind == "COMPLETED" {
        completed(paypal, &event.id, capture, &checkout.finance).await
    } else {
        paypal.marker(&event.id, &capture, kind).await
    };
    match result {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            error.audit_for("paypal");
            if matches!(error, Error::Mismatch) {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            }
        }
    }
}

async fn completed(
    paypal: &Paypal,
    event: &str,
    capture: Capture,
    finance: &crate::finance::Finance,
) -> Result<(), Error> {
    let order_id = &capture
        .supplementary_data
        .as_ref()
        .ok_or(Error::Mismatch)?
        .related_ids
        .order_id;
    // Real capture notifications may omit custom_id. Always inspect the authenticated order,
    // binding the notification's capture to its canonical server-created purchase unit.
    let order = paypal.provider.inspect(order_id).await?;
    let unit = order.purchase_units.first().ok_or(Error::Mismatch)?;
    let operation = df_types::CheckoutId::from_hex(&unit.custom_id).map_err(|_| Error::Mismatch)?;
    let canonical = verified_capture(&order, order_id, &unit.custom_id)?;
    if canonical.id != capture.id
        || canonical.amount != capture.amount
        || capture.status != "COMPLETED"
        || capture
            .custom_id
            .as_ref()
            .is_some_and(|id| id != &unit.custom_id)
    {
        return Err(Error::Mismatch);
    }
    paypal
        .credit(operation, order_id, canonical, Some(event), finance)
        .await
}
