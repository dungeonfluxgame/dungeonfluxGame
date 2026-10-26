//! Prevents account-app requests from reaching account handlers while accounts are closed.
//! API requests receive a typed refusal; browser pages receive the public coming-soon copy.

use axum::{
    Json, Router,
    extract::State,
    http::{Method, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
};
use serde::Serialize;

const COMING_SOON_PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>DungeonFlux account access</title><main><h1>Sign-in is coming soon</h1><p>Sign-up is coming soon.</p><p>Sign-in and sign-up are coming soon. When sign-up opens, new accounts will get $10 of promotional credit, subject to the final terms.</p><p>Follow <a href=\"https://x.com/DungeonFluxGame\">@DungeonFluxGame on X</a> for launch updates.</p></main></html>";

#[derive(Serialize)]
struct ClosedError {
    error: ClosedErrorCode,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ClosedErrorCode {
    AccountsClosed,
}

/// Adds the account closure check before every route in the hosted portal.
pub(crate) fn apply(router: Router, accounts_open: bool) -> Router {
    router.layer(middleware::from_fn_with_state(accounts_open, gate))
}

async fn gate(
    State(accounts_open): State<bool>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if accounts_open || request.uri().path() == "/app/healthz" {
        return next.run(request).await;
    }

    let path = request.uri().path();
    let is_payment_settlement_route = (request.method() == Method::POST
        && matches!(
            path,
            "/app/api/payments/stripe/webhook" | "/app/api/payments/paypal/webhook"
        ))
        || (request.method() == Method::GET && path == "/app/api/payments/paypal/return");
    if is_payment_settlement_route {
        df_observe::record_audit_line(
            "hosted.accounts_closed",
            "HOSTED accounts_closed request=passed class=payment_route".to_owned(),
        );
        return next.run(request).await;
    }

    if path == "/app/api" || path.starts_with("/app/api/") {
        df_observe::record_audit_line(
            "hosted.accounts_closed",
            "HOSTED accounts_closed request=refused class=api".to_owned(),
        );
        return (
            StatusCode::FORBIDDEN,
            Json(ClosedError {
                error: ClosedErrorCode::AccountsClosed,
            }),
        )
            .into_response();
    }

    if path == "/app" || path.starts_with("/app/") {
        df_observe::record_audit_line(
            "hosted.accounts_closed",
            "HOSTED accounts_closed request=served class=page".to_owned(),
        );
        return Html(COMING_SOON_PAGE).into_response();
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::State,
        http::{HeaderMap, Request, StatusCode},
        routing::{get, post},
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    use super::apply;

    async fn account_handler(State(calls): State<Arc<AtomicUsize>>) -> StatusCode {
        calls.fetch_add(1, Ordering::Relaxed);
        StatusCode::CREATED
    }

    async fn health_handler() -> &'static str {
        "ok"
    }

    struct PaymentGateCalls {
        verifications: AtomicUsize,
        account_mutations: AtomicUsize,
    }

    async fn verify_payment_callback(
        State(calls): State<Arc<PaymentGateCalls>>,
        headers: HeaderMap,
    ) -> StatusCode {
        calls.verifications.fetch_add(1, Ordering::Relaxed);
        if headers.contains_key("x-provider-signature") {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::UNAUTHORIZED
        }
    }

    async fn paypal_approval_return(State(calls): State<Arc<PaymentGateCalls>>) -> StatusCode {
        calls.verifications.fetch_add(1, Ordering::Relaxed);
        StatusCode::OK
    }

    async fn account_mutation(State(calls): State<Arc<PaymentGateCalls>>) -> StatusCode {
        calls.account_mutations.fetch_add(1, Ordering::Relaxed);
        StatusCode::CREATED
    }

    fn router(accounts_open: bool) -> (Router, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let routes = Router::new()
            .route("/app/api/sign-up", post(account_handler))
            .route("/app/sign-in", get(account_handler))
            .route("/app/healthz", get(health_handler))
            .with_state(Arc::clone(&calls));
        (apply(routes, accounts_open), calls)
    }

    #[tokio::test]
    async fn closed_accounts_refuse_api_and_serve_the_page_before_handlers() {
        let (app, calls) = router(false);
        let api = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/app/api/sign-up")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        assert_eq!(api.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            to_bytes(api.into_body(), 1024)
                .await
                .expect("small response body")
                .as_ref(),
            br#"{"error":"accounts_closed"}"#
        );

        let page = app
            .oneshot(
                Request::builder()
                    .uri("/app/sign-in")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        assert_eq!(page.status(), StatusCode::OK);
        let body = to_bytes(page.into_body(), 2048)
            .await
            .expect("small response body");
        assert!(
            std::str::from_utf8(&body)
                .expect("HTML is UTF-8")
                .contains("Sign-in is coming soon")
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn open_accounts_pass_requests_and_closed_health_remains_available() {
        let (open_app, calls) = router(true);
        let created = open_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/app/api/sign-up")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        assert_eq!(created.status(), StatusCode::CREATED);

        let (closed_app, _) = router(false);
        let health = closed_app
            .oneshot(
                Request::builder()
                    .uri("/app/healthz")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn closed_accounts_allow_only_payment_settlement_callbacks() {
        let calls = Arc::new(PaymentGateCalls {
            verifications: AtomicUsize::new(0),
            account_mutations: AtomicUsize::new(0),
        });
        let routes = Router::new()
            .route(
                "/app/api/payments/stripe/webhook",
                post(verify_payment_callback),
            )
            .route(
                "/app/api/payments/paypal/webhook",
                post(verify_payment_callback),
            )
            .route(
                "/app/api/payments/paypal/return",
                get(paypal_approval_return),
            )
            .route("/app/api/sign-up", post(account_mutation))
            .route("/app/api/checkout", post(account_mutation))
            .with_state(Arc::clone(&calls));
        let app = apply(routes, false);

        let unsigned_callback = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/app/api/payments/stripe/webhook")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        assert_eq!(unsigned_callback.status(), StatusCode::UNAUTHORIZED);

        for (request, expected_status) in [
            (
                Request::builder()
                    .method("POST")
                    .uri("/app/api/payments/paypal/webhook")
                    .header("x-provider-signature", "verified-test-signature")
                    .body(Body::empty())
                    .expect("request is valid"),
                StatusCode::NO_CONTENT,
            ),
            (
                Request::builder()
                    .method("GET")
                    .uri("/app/api/payments/paypal/return")
                    .body(Body::empty())
                    .expect("request is valid"),
                StatusCode::OK,
            ),
        ] {
            let response = app.clone().oneshot(request).await.expect("router responds");
            assert_eq!(response.status(), expected_status);
        }

        for request in [
            Request::builder()
                .method("POST")
                .uri("/app/api/payments/paypal/webhook/extra")
                .body(Body::empty())
                .expect("request is valid"),
            Request::builder()
                .method("GET")
                .uri("/app/api/payments/stripe/webhook")
                .body(Body::empty())
                .expect("request is valid"),
            Request::builder()
                .method("POST")
                .uri("/app/api/sign-up")
                .body(Body::empty())
                .expect("request is valid"),
            Request::builder()
                .method("POST")
                .uri("/app/api/checkout")
                .body(Body::empty())
                .expect("request is valid"),
        ] {
            let response = app.clone().oneshot(request).await.expect("router responds");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }

        assert_eq!(calls.verifications.load(Ordering::Relaxed), 3);
        assert_eq!(calls.account_mutations.load(Ordering::Relaxed), 0);
    }
}
