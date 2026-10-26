use super::fixtures::{Harness, ORIGIN, Reply};

#[tokio::test]
async fn native_oauth_handlers_map_session_store_failures_to_fixed_recovery_screens() {
    use crate::app_api::{AppApiState, guard::ApiFailure, oauth};
    use axum::extract::{Form, Query, State};
    use df_account_app::wire::ErrorCode;
    let harness = Harness::with_google().await;
    let state = AppApiState::new(
        harness.service.clone(),
        crate::PortalOrigin::parse(ORIGIN).unwrap(),
        harness.ports.clone(),
    );
    let refused = || Err(ApiFailure::of(ErrorCode::Unavailable));
    let client = || crate::security::ClientInfo(df_customer::contract::client(1));
    let start = oauth::start(
        State(state.clone()),
        refused(),
        Default::default(),
        client(),
        Ok(Form(
            serde_json::from_value(serde_json::json!({"csrf":"test","intent":"link"})).unwrap(),
        )),
    )
    .await;
    let callback = oauth::callback(
        State(state.clone()),
        refused(),
        Default::default(),
        client(),
        Ok(Query(
            serde_json::from_value(serde_json::json!({})).unwrap(),
        )),
    )
    .await;
    let unlink = oauth::unlink(
        State(state),
        refused(),
        Default::default(),
        Ok(Form(
            serde_json::from_value(serde_json::json!({"csrf":"test","current_password":"test"}))
                .unwrap(),
        )),
    )
    .await;
    for response in [start, callback, unlink] {
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            "/app/oauth/error/unavailable"
        );
    }
    assert_eq!(harness.service.dev_summary().await.unwrap().active, 0);
}
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use df_customer::{
    EmailAddress, OAuthAuthorize, OAuthCall, OAuthExchange, OAuthFailure, OAuthIdentity,
    OAuthProvider, OAuthSecret, pkce_challenge,
};
use std::{collections::HashMap, sync::Mutex};

pub(super) struct Provider {
    codes: Mutex<HashMap<String, ([u8; 32], String)>>,
}
impl Provider {
    pub(super) fn new() -> Self {
        Self {
            codes: Mutex::default(),
        }
    }
}
impl OAuthProvider for Provider {
    fn redirect_uri(&self) -> &str {
        "http://127.0.0.1:8099/app/api/oauth/google/callback"
    }
    fn authorize(&self, request: OAuthAuthorize<'_>) -> Result<OAuthSecret, OAuthFailure> {
        let mut codes = self.codes.lock().unwrap();
        if codes.len() >= 128 {
            return Err(OAuthFailure::Unavailable);
        }
        codes.insert(
            request.state.expose().to_owned(),
            (request.challenge, request.nonce.expose().to_owned()),
        );
        Ok(OAuthSecret::new(format!(
            "{}?state={}&code={}",
            self.redirect_uri(),
            request.state.expose(),
            request.state.expose()
        )))
    }
    fn exchange<'a>(&'a self, request: OAuthExchange<'a>) -> OAuthCall<'a> {
        Box::pin(async move {
            let (challenge, nonce) = self
                .codes
                .lock()
                .unwrap()
                .remove(request.code.expose())
                .ok_or(OAuthFailure::Rejected)?;
            if challenge != pkce_challenge(request.verifier) || nonce != request.nonce.expose() {
                return Err(OAuthFailure::Rejected);
            }
            Ok(OAuthIdentity {
                subject: "google-route-subject".to_owned(),
                email: EmailAddress::parse("google-route@example.test").unwrap(),
                authenticated_at_s: request.require_fresh.then_some(request.now_s),
            })
        })
    }
}

fn cookies(reply: &Reply) -> String {
    reply
        .set_cookies()
        .iter()
        .filter_map(|cookie| cookie.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

#[tokio::test]
async fn cancellation_consumes_the_attempt_without_an_account_and_never_renders_provider_text() {
    let harness = Harness::with_google().await;
    let session = harness
        .send(
            Request::builder()
                .uri("/app/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let csrf = session.json()["csrf"].as_str().unwrap().to_owned();
    let started = start(&harness, &csrf, &cookies(&session), ORIGIN, true).await;
    let original = started.headers[header::LOCATION]
        .to_str()
        .unwrap()
        .strip_prefix(ORIGIN)
        .unwrap();
    let state = original
        .split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|part| part.strip_prefix("state="))
        .unwrap();
    let path = format!(
        "/app/api/oauth/google/callback?state={state}&error=access_denied&error_description=provider-private-text"
    );
    let cookie = cookies(&started);
    let answer = harness
        .send(
            Request::builder()
                .uri(path)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        answer.headers[header::LOCATION],
        "/app/oauth/error/canceled"
    );
    assert_eq!(harness.service.dev_summary().await.unwrap().active, 0);
    let replay = harness
        .send(
            Request::builder()
                .uri(original)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        replay.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("expired")
    );
    let screen = harness
        .send(
            Request::builder()
                .uri("/app/oauth/error/canceled")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(screen.body.contains("Google sign-in was canceled."));
    assert!(!screen.body.contains("provider-private-text"));
    assert_eq!(screen.headers[header::REFERRER_POLICY], "no-referrer");
}

async fn start(harness: &Harness, csrf: &str, cookie: &str, origin: &str, adult: bool) -> Reply {
    harness.send(Request::builder().method("POST").uri("/app/api/oauth/google/start")
        .header(header::ORIGIN,origin).header("sec-fetch-site","same-origin")
        .header(header::CONTENT_TYPE,"application/x-www-form-urlencoded").header(header::COOKIE,cookie)
        .body(Body::from(format!("csrf={csrf}&intent=sign_up&is_adult={adult}&accepts_terms=true&terms_version=terms-2026-10"))).unwrap()).await
}

#[tokio::test]
async fn google_start_requires_form_csrf_same_origin_consent_and_configured_provider() {
    let harness = Harness::with_google().await;
    let session = harness
        .send(
            Request::builder()
                .uri("/app/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(session.json()["google_enabled"], true);
    let csrf = session.json()["csrf"].as_str().unwrap().to_owned();
    let cookie = cookies(&session);
    for (token, origin, adult, reason) in [
        ("wrong", ORIGIN, true, "expired"),
        (csrf.as_str(), "https://elsewhere.test", true, "expired"),
        (csrf.as_str(), ORIGIN, false, "consent_required"),
    ] {
        let refused = start(&harness, token, &cookie, origin, adult).await;
        assert_eq!(refused.status, StatusCode::SEE_OTHER);
        assert!(
            refused.headers[header::LOCATION]
                .to_str()
                .unwrap()
                .ends_with(reason)
        );
        assert_eq!(refused.headers[header::CACHE_CONTROL], "no-store");
    }
    let disabled = Harness::new().await;
    let session = disabled
        .send(
            Request::builder()
                .uri("/app/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(session.json()["google_enabled"], false);
}

#[tokio::test]
async fn google_callback_signs_up_verified_clears_cookie_and_refuses_replay_or_bad_query() {
    let harness = Harness::with_google().await;
    let session = harness
        .send(
            Request::builder()
                .uri("/app/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let csrf = session.json()["csrf"].as_str().unwrap().to_owned();
    let started = start(&harness, &csrf, &cookies(&session), ORIGIN, true).await;
    assert_eq!(started.status, StatusCode::SEE_OTHER);
    let attempt = started.set_cookie("__Host-df_oauth").unwrap();
    for attribute in [
        "Secure",
        "HttpOnly",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=600",
    ] {
        assert!(attempt.contains(attribute));
    }
    assert!(!attempt.contains("Domain="));
    let callback = started.headers[header::LOCATION]
        .to_str()
        .unwrap()
        .strip_prefix(ORIGIN)
        .unwrap();
    let callback_cookie = cookies(&started);
    let answer = harness
        .send(
            Request::builder()
                .uri(callback)
                .header(header::COOKIE, &callback_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(answer.headers[header::LOCATION], "/app/dashboard?welcome=1");
    assert!(answer.set_cookie("__Host-df_session").is_some());
    assert!(
        answer
            .set_cookie("__Host-df_oauth")
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(answer.headers[header::REFERRER_POLICY], "no-referrer");
    let signed_in = harness
        .send(
            Request::builder()
                .uri("/app/api/session")
                .header(header::COOKIE, cookies(&answer))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(signed_in.json()["account"]["is_verified"], true);
    let operation = harness.service.new_operation().unwrap();
    let proof_required=harness.send(Request::builder().method("POST").uri("/app/api/settings/export")
        .header(header::ORIGIN,ORIGIN).header("sec-fetch-site","same-origin")
        .header(header::CONTENT_TYPE,"application/json")
        .header("x-df-csrf",signed_in.json()["csrf"].as_str().unwrap())
        .header(header::COOKIE,cookies(&answer))
        .body(Body::from(serde_json::json!({"password":"","operation":crate::app_api::ids::hex(operation.as_bytes())}).to_string())).unwrap()).await;
    assert_eq!(
        proof_required.json()["refusal"],
        "reauthentication_required"
    );
    for path in [callback, "/app/api/oauth/google/callback?state=a&state=b"] {
        let refused = harness
            .send(
                Request::builder()
                    .uri(path)
                    .header(header::COOKIE, &callback_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            refused.headers[header::LOCATION]
                .to_str()
                .unwrap()
                .ends_with("expired")
        );
        assert!(
            refused
                .set_cookie("__Host-df_oauth")
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(refused.headers[header::CACHE_CONTROL], "no-store");
    }
}
