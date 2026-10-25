use axum::http::{Method, StatusCode, header};
use df_account_app::wire::ErrorCode;
use serde_json::{Value, json};

use super::{
    fixtures::{Harness, Req},
    routes::{admin_posts, admin_reads, customer_posts, customer_reads},
};
use crate::app_api::guard::status_of;

fn method_of(path: &str) -> Method {
    if path.contains("/steps/") {
        Method::PUT
    } else {
        Method::POST
    }
}

#[tokio::test]
async fn anonymous_visitors_are_signed_out_on_every_customer_and_operator_route() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    assert!(visitor.cookie(crate::PRESESSION_COOKIE).is_some());
    for path in customer_reads().into_iter().chain(admin_reads()) {
        let reply = visitor.get(&path).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "GET {path}");
        assert_eq!(reply.code(), "signed_out", "GET {path}");
    }
    for (path, body) in customer_posts().into_iter().chain(admin_posts()) {
        let reply = visitor
            .send(Req::new(method_of(&path), &path).json(&body))
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "POST {path}");
        assert_eq!(reply.code(), "signed_out", "POST {path}");
    }
}

#[tokio::test]
async fn a_pre_session_cookie_a_display_cookie_or_a_made_up_session_authorizes_nothing() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    visitor.post("/app/api/display/challenge", &json!({})).await;
    assert!(visitor.cookie("__Host-df_display_device").is_some());
    visitor
        .cookies
        .insert(crate::SESSION_COOKIE.to_owned(), "ab".repeat(32));
    for path in customer_reads().into_iter().chain(admin_reads()) {
        let reply = visitor.get(&path).await;
        assert_eq!(reply.code(), "signed_out", "GET {path}");
    }
}

#[tokio::test]
async fn a_customer_is_forbidden_on_every_operator_route() {
    let harness = Harness::new().await;
    let mut customer = harness.browser();
    customer.sign_up_verified("robin@example.test").await;
    for path in admin_reads() {
        let reply = customer.get(&path).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "GET {path}");
        assert_eq!(reply.code(), "forbidden", "GET {path}");
    }
    for (path, body) in admin_posts() {
        let reply = customer.post(&path, &body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "POST {path}");
        assert_eq!(reply.code(), "forbidden", "POST {path}");
    }
}

#[tokio::test]
async fn an_operator_without_the_second_factor_is_forbidden_on_every_operator_route() {
    let harness = Harness::new().await;
    let mut operator = harness.browser();
    operator.sign_up_verified("ops@example.test").await;
    let id = operator.account_id().await;
    harness.ports.operators.grant_all(id, false);
    for path in admin_reads() {
        assert_eq!(operator.get(&path).await.code(), "forbidden", "GET {path}");
    }
    for (path, body) in admin_posts() {
        assert_eq!(
            operator.post(&path, &body).await.code(),
            "forbidden",
            "POST {path}"
        );
    }
}

#[tokio::test]
async fn an_operator_with_the_second_factor_reads_the_console() {
    let harness = Harness::new().await;
    let mut operator = harness.browser();
    operator.become_operator("ops@example.test").await;
    let reply = operator.get("/app/api/admin/summary").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let session = operator.session().await;
    assert_eq!(
        session["account"]["operator"]["has_second_factor"],
        json!(true)
    );
}

#[tokio::test]
async fn a_customer_session_never_carries_an_operator_view() {
    let harness = Harness::new().await;
    let mut customer = harness.browser();
    let view = customer.sign_up_verified("robin@example.test").await;
    assert_eq!(view["account"]["operator"], Value::Null);
}

async fn display_name_of(customer: &mut super::fixtures::Browser<'_>) -> Value {
    customer.get("/app/api/settings").await.json()["display_name"].clone()
}

#[tokio::test]
async fn a_mutation_without_a_valid_token_origin_fetch_site_or_content_type_changes_nothing() {
    let harness = Harness::new().await;
    let mut customer = harness.browser();
    customer.sign_up_verified("robin@example.test").await;
    let operation = customer.operation().await;
    let body = json!({"display_name": "Robin", "operation": operation});
    let attempts = [
        (
            "no token",
            Req::new(Method::POST, "/app/api/settings/profile").csrf(None),
        ),
        (
            "a wrong token",
            Req::new(Method::POST, "/app/api/settings/profile").csrf(Some(&"ab".repeat(32))),
        ),
        (
            "not hex",
            Req::new(Method::POST, "/app/api/settings/profile").csrf(Some("nope")),
        ),
        (
            "cross-site",
            Req::new(Method::POST, "/app/api/settings/profile").fetch_site(Some("cross-site")),
        ),
        (
            "same-site",
            Req::new(Method::POST, "/app/api/settings/profile").fetch_site(Some("same-site")),
        ),
        (
            "a foreign origin",
            Req::new(Method::POST, "/app/api/settings/profile").origin(Some("http://evil.test")),
        ),
        (
            "a form content type",
            Req::new(Method::POST, "/app/api/settings/profile")
                .content_type("application/x-www-form-urlencoded"),
        ),
        (
            "plain text",
            Req::new(Method::POST, "/app/api/settings/profile").content_type("text/plain"),
        ),
    ];
    for (label, request) in attempts {
        let reply = customer.send(request.json(&body)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{label}");
        assert_eq!(reply.code(), "csrf_refused", "{label}");
    }
    assert_eq!(display_name_of(&mut customer).await, Value::Null);
    let accepted = customer
        .send(
            Req::new(Method::POST, "/app/api/settings/profile")
                .origin(None)
                .fetch_site(None)
                .json(&body),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    assert_eq!(display_name_of(&mut customer).await, json!("Robin"));
}

#[tokio::test]
async fn a_token_issued_for_another_session_is_refused() {
    let harness = Harness::new().await;
    let mut robin = harness.browser();
    robin.sign_up_verified("robin@example.test").await;
    let mut sam = harness.browser();
    sam.sign_up_verified("sam@example.test").await;
    let borrowed = sam.csrf.clone();
    let reply = robin
        .send(
            Req::new(Method::POST, "/app/api/settings/sessions/revoke-others")
                .csrf(Some(&borrowed))
                .json(&json!({})),
        )
        .await;
    assert_eq!(reply.code(), "csrf_refused");
}

#[tokio::test]
async fn a_duplicated_origin_header_is_refused() {
    let harness = Harness::new().await;
    let mut customer = harness.browser();
    customer.sign_up_verified("robin@example.test").await;
    let mut request = axum::http::Request::post("/app/api/settings/sessions/revoke-others")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, super::fixtures::ORIGIN)
        .header(header::ORIGIN, super::fixtures::ORIGIN)
        .header("x-df-csrf", &customer.csrf)
        .header(
            header::COOKIE,
            format!(
                "{}={}",
                crate::SESSION_COOKIE,
                customer.cookie(crate::SESSION_COOKIE).unwrap()
            ),
        )
        .body(axum::body::Body::from("{}"))
        .unwrap();
    request.extensions_mut().clear();
    let reply = harness.send(request).await;
    assert_eq!(reply.code(), "csrf_refused");
}

#[tokio::test]
async fn every_api_reply_carries_the_cache_sniffing_and_referrer_headers() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    let replies = [
        visitor.get("/app/api/session").await,
        visitor.get("/app/api/overview").await,
        visitor.get("/app/api/does-not-exist").await,
        visitor
            .send(Req::new(Method::POST, "/app/api/sign-in").json(&json!({})))
            .await,
    ];
    for reply in replies {
        let headers = &reply.headers;
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    }
}

#[tokio::test]
async fn a_malformed_unknown_field_or_oversized_body_is_invalid() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let broken = visitor
        .send(Req::new(Method::POST, "/app/api/sign-in").json(&json!("{")))
        .await;
    assert_eq!(broken.code(), "invalid");
    let extra = visitor
        .post(
            "/app/api/sign-in",
            &json!({"email": "a@example.test", "password": "x", "role": "admin"}),
        )
        .await;
    assert_eq!(extra.code(), "invalid");
    let large = "x".repeat(20 * 1024);
    let oversized = visitor
        .post(
            "/app/api/sign-in",
            &json!({"email": "a@example.test", "password": large}),
        )
        .await;
    assert_eq!(oversized.code(), "invalid");
}

#[test]
fn every_error_code_has_its_documented_status() {
    let table = [
        (ErrorCode::SignedOut, 401),
        (ErrorCode::Forbidden, 403),
        (ErrorCode::NotVerified, 403),
        (ErrorCode::CsrfRefused, 403),
        (ErrorCode::Throttled, 429),
        (ErrorCode::NotFound, 404),
        (ErrorCode::Invalid, 422),
        (ErrorCode::StaleForm, 409),
        (ErrorCode::Unavailable, 503),
        (ErrorCode::Busy, 503),
        (ErrorCode::UnknownOutcome, 504),
    ];
    for (code, status) in table {
        assert_eq!(status_of(code).as_u16(), status, "{code:?}");
    }
}

#[tokio::test]
async fn the_api_never_answers_a_cross_origin_preflight() {
    let harness = Harness::new().await;
    for path in [
        "/app/api/sign-in",
        "/app/api/session",
        "/app/api/display/redeem",
    ] {
        let request = axum::http::Request::builder()
            .method(Method::OPTIONS)
            .uri(path)
            .header(header::ORIGIN, "http://evil.test")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "x-df-csrf, x-df-device")
            .body(axum::body::Body::empty())
            .unwrap();
        let reply = harness.send(request).await;
        assert!(
            reply
                .headers
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "{path}"
        );
        assert_ne!(reply.status, StatusCode::OK, "{path}");
    }
}
