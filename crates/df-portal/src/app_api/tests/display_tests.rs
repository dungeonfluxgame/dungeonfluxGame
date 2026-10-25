//! Display pairing and the device guard: redeem and renew need the Strict device cookie, a
//! present matching Origin, `Sec-Fetch-Site: same-origin`, a JSON content type and `x-df-device`;
//! a CSRF token alone authorizes neither.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::fixtures::{Browser, Harness, Req};
use crate::app_api::{ids::hex, ports::Actor};

const DEVICE: &str = "__Host-df_display_device";
const TICKET: &str = "__Host-df_display_ticket";

/// A host with an open lobby, and a TV that has asked for a code.
struct Pairing<'h> {
    host: Browser<'h>,
    tv: Browser<'h>,
    session: String,
    code: String,
}

async fn pairing(harness: &Harness) -> Pairing<'_> {
    let mut host = harness.browser();
    host.sign_up_verified("robin@example.test").await;
    let who = Actor {
        account: host.account_id().await,
        display_name: "Robin".into(),
        is_verified: true,
    };
    let campaign = harness
        .ports
        .campaigns
        .seed_campaign(&who, "The Lantern Road");
    let session = hex(harness
        .ports
        .campaigns
        .seed_lobby(campaign)
        .unwrap()
        .as_bytes());
    let mut tv = harness.browser();
    tv.session().await;
    let challenge = tv.post("/app/api/display/challenge", &json!({})).await;
    assert_eq!(challenge.status, StatusCode::OK, "{}", challenge.body);
    let code = challenge.json()["user_code"].as_str().unwrap().to_owned();
    Pairing {
        host,
        tv,
        session,
        code,
    }
}

impl Pairing<'_> {
    /// The host looks the code up and confirms it.
    async fn approve(&mut self) -> Value {
        let lookup = self
            .host
            .post(
                &format!("/app/api/sessions/{}/lobby/display/lookup", self.session),
                &json!({"code": self.code}),
            )
            .await
            .json();
        assert_eq!(lookup["result"], json!("found"), "{lookup}");
        let operation = self.host.operation().await;
        self.host
            .post(
                &format!("/app/api/sessions/{}/lobby/display/approve", self.session),
                &json!({"pairing_id": lookup["pairing_id"], "operation": operation}),
            )
            .await
            .json()
    }
}

fn device_post(path: &str) -> Req {
    Req::new(Method::POST, path)
        .json(&json!({}))
        .device_header()
}

#[tokio::test]
async fn a_challenge_sets_a_strict_http_only_device_cookie_and_needs_the_pre_session_token() {
    let harness = Harness::new().await;
    let mut tv = harness.browser();
    tv.session().await;
    let refused = tv
        .send(
            Req::new(Method::POST, "/app/api/display/challenge")
                .csrf(None)
                .json(&json!({})),
        )
        .await;
    assert_eq!(refused.code(), "csrf_refused");
    assert!(refused.set_cookie(DEVICE).is_none());
    let reply = tv.post("/app/api/display/challenge", &json!({})).await;
    let cookie = reply.set_cookie(DEVICE).unwrap();
    for attribute in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(attribute), "{attribute} in {cookie}");
    }
    let code = reply.json()["user_code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 9);
    assert!(
        !reply
            .body
            .contains(cookie.split('=').nth(1).unwrap().split(';').next().unwrap())
    );
}

#[tokio::test]
async fn the_status_poll_follows_the_pairing_without_side_effects() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    for _ in 0..2 {
        assert_eq!(
            pair.tv.get("/app/api/display/status").await.json(),
            json!({"state": "pending"})
        );
    }
    pair.approve().await;
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "approved"})
    );
    let mut stranger = harness.browser();
    assert_eq!(
        stranger.get("/app/api/display/status").await.json(),
        json!({"state": "expired"})
    );
}

#[tokio::test]
async fn a_lookup_never_approves_and_an_unknown_code_is_not_found() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    let path = format!("/app/api/sessions/{}/lobby/display/lookup", pair.session);
    let found = pair
        .host
        .post(&path, &json!({"code": pair.code}))
        .await
        .json();
    assert_eq!(found["device"], json!("unknown"));
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "pending"})
    );
    let unknown = pair
        .host
        .post(&path, &json!({"code": "ZZZZ-ZZZZ"}))
        .await
        .json();
    assert_eq!(unknown, json!({"result": "not_found"}));
}

#[tokio::test]
async fn redeeming_issues_a_strict_ticket_cookie_and_repeating_it_reissues_the_same_ticket() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    pair.approve().await;
    let first = pair.tv.send(device_post("/app/api/display/redeem")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.json()["session_id"], json!(pair.session));
    let cookie = first.set_cookie(TICKET).unwrap();
    for attribute in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(attribute), "{attribute} in {cookie}");
    }
    let repeat = pair.tv.send(device_post("/app/api/display/redeem")).await;
    assert_eq!(repeat.status, StatusCode::OK);
    assert_eq!(
        repeat.set_cookie(TICKET).unwrap().split(';').next(),
        cookie.split(';').next()
    );
}

#[tokio::test]
async fn every_device_guard_rule_refuses_alone_and_changes_nothing() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    pair.approve().await;
    let path = "/app/api/display/redeem";
    let attempts = [
        ("no device cookie", device_post(path).only_cookies(&[])),
        (
            "a wrong origin",
            device_post(path).origin(Some("http://evil.test")),
        ),
        ("a missing origin", device_post(path).origin(None)),
        ("same-site", device_post(path).fetch_site(Some("same-site"))),
        (
            "cross-site",
            device_post(path).fetch_site(Some("cross-site")),
        ),
        (
            "an absent Sec-Fetch-Site",
            device_post(path).fetch_site(None),
        ),
        (
            "none as Sec-Fetch-Site",
            device_post(path).fetch_site(Some("none")),
        ),
        (
            "a wrong content type",
            device_post(path).content_type("text/plain"),
        ),
        (
            "a missing x-df-device",
            Req::new(Method::POST, path).json(&json!({})),
        ),
    ];
    for (label, request) in attempts {
        let reply = pair.tv.send(request).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{label}");
        assert_eq!(reply.code(), "csrf_refused", "{label}");
        assert!(reply.set_cookie(TICKET).is_none(), "{label}");
    }
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "approved"})
    );
    let accepted = pair.tv.send(device_post(path)).await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
}

#[tokio::test]
async fn a_csrf_token_alone_authorizes_neither_redeem_nor_renew() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    pair.approve().await;
    pair.tv.session().await;
    let token = pair.tv.csrf.clone();
    for path in ["/app/api/display/redeem", "/app/api/display/renew"] {
        let with_token_only = pair
            .tv
            .send(
                Req::new(Method::POST, path)
                    .csrf(Some(&token))
                    .json(&json!({})),
            )
            .await;
        assert_eq!(with_token_only.code(), "csrf_refused", "{path}");
        let without_cookie = pair
            .tv
            .send(device_post(path).csrf(Some(&token)).only_cookies(&[]))
            .await;
        assert_eq!(without_cookie.code(), "csrf_refused", "{path}");
    }
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "approved"})
    );
}

#[tokio::test]
async fn redeeming_before_approval_or_with_a_wrong_secret_is_one_neutral_answer_that_clears_the_cookie()
 {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    let early = pair.tv.send(device_post("/app/api/display/redeem")).await;
    assert_eq!(early.status, StatusCode::NOT_FOUND);
    assert_eq!(early.json()["reason"], json!("no_longer_valid"));
    assert!(early.set_cookie(DEVICE).unwrap().contains("Max-Age=0"));
    let mut impostor = harness.browser();
    let wrong = impostor
        .send(device_post("/app/api/display/redeem").only_cookies(&[(DEVICE, &"cd".repeat(32))]))
        .await;
    assert_eq!(wrong.body, early.body);
}

#[tokio::test]
async fn renewing_extends_the_ticket_only_with_both_cookies() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    pair.approve().await;
    let redeemed = pair
        .tv
        .send(device_post("/app/api/display/redeem"))
        .await
        .json();
    let expires = redeemed["ticket_expires_at_ms"].as_u64().unwrap();
    harness.clock.advance_ms(60 * 60 * 1000);
    let device = pair.tv.cookie(DEVICE).unwrap().to_owned();
    let without_ticket = pair
        .tv
        .send(device_post("/app/api/display/renew").only_cookies(&[(DEVICE, &device)]))
        .await;
    assert_eq!(without_ticket.status, StatusCode::NOT_FOUND);
    let mut tv = harness.browser();
    let ticket = pair.tv.cookie(TICKET).unwrap().to_owned();
    let renewed = tv
        .send(
            device_post("/app/api/display/renew")
                .only_cookies(&[(DEVICE, &device), (TICKET, &ticket)]),
        )
        .await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.body);
    assert!(renewed.json()["ticket_expires_at_ms"].as_u64().unwrap() > expires);
    assert!(
        renewed
            .set_cookie(TICKET)
            .unwrap()
            .contains("SameSite=Strict")
    );
}

#[tokio::test]
async fn unpairing_ends_the_display_and_its_redemption() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    let lobby = pair.approve().await;
    assert_eq!(lobby["displays"].as_array().unwrap().len(), 1);
    let ticket_id = lobby["displays"][0]["ticket_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let operation = pair.host.operation().await;
    let after = pair
        .host
        .post(
            &format!(
                "/app/api/sessions/{}/lobby/display/{ticket_id}/unpair",
                pair.session
            ),
            &json!({"operation": operation}),
        )
        .await;
    assert_eq!(after.json()["displays"], json!([]));
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "cancelled"})
    );
    let redeem = pair.tv.send(device_post("/app/api/display/redeem")).await;
    assert_eq!(redeem.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_a_member_who_can_start_may_pair_a_display() {
    let harness = Harness::new().await;
    let mut pair = pairing(&harness).await;
    let mut stranger = harness.browser();
    stranger.sign_up_verified("sam@example.test").await;
    let path = format!("/app/api/sessions/{}/lobby/display/lookup", pair.session);
    let reply = stranger.post(&path, &json!({"code": pair.code})).await;
    assert_eq!(reply.code(), "not_found");
    assert_eq!(
        pair.tv.get("/app/api/display/status").await.json(),
        json!({"state": "pending"})
    );
}
