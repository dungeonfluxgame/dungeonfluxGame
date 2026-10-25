use axum::http::{Method, StatusCode};
use df_customer::MailPurpose;
use serde_json::{Value, json};

use super::fixtures::{Harness, ORIGIN, OTHER_PASSWORD, PASSWORD, Req};

async fn sign_up_body(browser: &mut super::fixtures::Browser<'_>, email: &str) -> Value {
    let operation = browser.operation().await;
    json!({"email": email, "password": PASSWORD, "is_adult": true, "accepts_terms": true,
        "terms_version": df_account_app::wire::CURRENT_TERMS_VERSION, "operation": operation})
}

#[tokio::test]
async fn an_anonymous_session_read_issues_a_token_and_a_pre_session_cookie_once() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    let first = visitor.get("/app/api/session").await;
    let cookie = first.set_cookie(crate::PRESESSION_COOKIE).unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("Secure"));
    assert!(cookie.contains("SameSite=Lax") && cookie.contains("Path=/"));
    assert_eq!(first.json()["account"], Value::Null);
    assert!(first.json().get("dev_sign_in").is_none());
    let second = visitor.get("/app/api/session").await;
    assert!(second.set_cookie(crate::PRESESSION_COOKIE).is_none());
    assert_eq!(first.json()["csrf"], second.json()["csrf"]);
}

#[tokio::test]
async fn sign_up_requires_the_adult_and_terms_confirmations() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    for (field, refusal) in [
        ("is_adult", "adult_required"),
        ("accepts_terms", "terms_required"),
    ] {
        let mut body = sign_up_body(&mut visitor, "robin@example.test").await;
        body[field] = json!(false);
        let reply = visitor.post("/app/api/sign-up", &body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.json()["refusal"], json!(refusal));
    }
    let mut old_terms = sign_up_body(&mut visitor, "robin@example.test").await;
    old_terms["terms_version"] = json!("terms-2025-01");
    let reply = visitor.post("/app/api/sign-up", &old_terms).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.json()["refusal"], json!("terms_required"));
    assert!(harness.mail.messages().is_empty());
}

#[tokio::test]
async fn sign_up_names_the_refused_field_and_sends_one_mail_for_a_new_address() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let mut weak = sign_up_body(&mut visitor, "robin@example.test").await;
    weak["password"] = json!("short");
    assert_eq!(
        visitor.post("/app/api/sign-up", &weak).await.json()["refusal"],
        json!("password_too_short")
    );
    let mut malformed = sign_up_body(&mut visitor, "not an address").await;
    malformed["email"] = json!("not an address");
    assert_eq!(
        visitor.post("/app/api/sign-up", &malformed).await.json()["refusal"],
        json!("email_malformed")
    );
    let good = sign_up_body(&mut visitor, "robin@example.test").await;
    let reply = visitor.post("/app/api/sign-up", &good).await;
    assert_eq!(reply.json(), json!({"accepted": true}));
    harness.mail.wait_for(1).await;
    assert_eq!(harness.mail.messages().len(), 1);
}

#[tokio::test]
async fn sign_up_answers_the_same_for_a_registered_address() {
    let harness = Harness::new().await;
    let mut first = harness.browser();
    first.sign_up_verified("robin@example.test").await;
    let mut again = harness.browser();
    again.session().await;
    let body = sign_up_body(&mut again, "robin@example.test").await;
    let reply = again.post("/app/api/sign-up", &body).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({"accepted": true}));
}

#[tokio::test]
async fn the_verification_and_reset_mails_link_to_app_pages_with_the_token_in_the_fragment() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let verify = harness.mailed_link("robin@example.test", MailPurpose::VerifyEmail);
    assert!(
        verify.starts_with(&format!("{ORIGIN}/app/verify-email#token=")),
        "{verify}"
    );
    let mut visitor = harness.browser();
    visitor.session().await;
    visitor
        .post(
            "/app/api/password/forgot",
            &json!({"email": "robin@example.test"}),
        )
        .await;
    harness.mail.wait_for(2).await;
    let reset = harness.mailed_link("robin@example.test", MailPurpose::PasswordReset);
    assert!(
        reset.starts_with(&format!("{ORIGIN}/app/reset-password#token=")),
        "{reset}"
    );
}

#[tokio::test]
async fn verification_accepts_the_token_in_the_body_only() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let body = sign_up_body(&mut visitor, "robin@example.test").await;
    visitor.post("/app/api/sign-up", &body).await;
    let token = harness
        .mailed_token(0, "robin@example.test", MailPurpose::VerifyEmail)
        .await;
    let in_query = visitor
        .send(
            Req::new(
                Method::POST,
                &format!("/app/api/verify-email?token={token}"),
            )
            .json(&json!({})),
        )
        .await;
    assert_eq!(in_query.code(), "invalid");
    let wrong_token = visitor
        .send(
            Req::new(Method::POST, "/app/api/verify-email")
                .json(&json!({"token": "x", "password": PASSWORD, "confirmation": PASSWORD})),
        )
        .await;
    assert_eq!(wrong_token.json(), json!("invalid_link"));
    let in_path = visitor.get(&format!("/app/api/verify-email/{token}")).await;
    assert_eq!(in_path.status, StatusCode::NOT_FOUND);
    let in_body = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": PASSWORD, "confirmation": PASSWORD}),
        )
        .await;
    assert_eq!(in_body.json(), json!("verified"));
    let again = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": PASSWORD, "confirmation": PASSWORD}),
        )
        .await;
    assert_eq!(again.json(), json!("already_verified"));
}

#[tokio::test]
async fn verifying_without_a_password_is_refused_and_the_link_stays_usable() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let body = sign_up_body(&mut visitor, "robin@example.test").await;
    visitor.post("/app/api/sign-up", &body).await;
    let token = harness
        .mailed_token(0, "robin@example.test", MailPurpose::VerifyEmail)
        .await;
    let no_password = visitor
        .post("/app/api/verify-email", &json!({"token": token}))
        .await;
    assert_eq!(no_password.code(), "invalid");
    let empty = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": "", "confirmation": ""}),
        )
        .await;
    assert_eq!(empty.json()["refusal"], json!("password_too_short"));
    let verified = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": PASSWORD, "confirmation": PASSWORD}),
        )
        .await;
    assert_eq!(verified.json(), json!("verified"));
}

#[tokio::test]
async fn a_mismatched_verification_password_leaves_the_token_usable() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let body = sign_up_body(&mut visitor, "robin@example.test").await;
    visitor.post("/app/api/sign-up", &body).await;
    let token = harness
        .mailed_token(0, "robin@example.test", MailPurpose::VerifyEmail)
        .await;
    let mismatch = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": PASSWORD, "confirmation": OTHER_PASSWORD}),
        )
        .await;
    assert_eq!(mismatch.json()["refusal"], json!("password_mismatch"));
    let verified = visitor
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": OTHER_PASSWORD, "confirmation": OTHER_PASSWORD}),
        )
        .await;
    assert_eq!(verified.json(), json!("verified"));
}

#[tokio::test]
async fn verifying_revokes_a_squatters_session_and_replaces_the_sign_up_password() {
    let harness = Harness::new().await;
    // A squatter signs up with someone else's address and signs in before verification.
    let mut squatter = harness.browser();
    squatter.session().await;
    let body = sign_up_body(&mut squatter, "victim@example.test").await;
    squatter.post("/app/api/sign-up", &body).await;
    squatter.sign_in("victim@example.test", PASSWORD).await;
    assert_eq!(
        squatter.get("/app/api/overview").await.status,
        StatusCode::OK
    );
    // The mailbox owner verifies with their own password.
    let token = harness
        .mailed_token(0, "victim@example.test", MailPurpose::VerifyEmail)
        .await;
    let mut owner = harness.browser();
    owner.session().await;
    let verified = owner
        .post(
            "/app/api/verify-email",
            &json!({"token": token, "password": OTHER_PASSWORD, "confirmation": OTHER_PASSWORD}),
        )
        .await;
    assert_eq!(verified.json(), json!("verified"));
    assert_eq!(owner.get("/app/api/overview").await.status, StatusCode::OK);
    // Verification issues this browser a new session while revoking all earlier sessions.
    assert_eq!(squatter.get("/app/api/overview").await.code(), "signed_out");
    let mut visitor = harness.browser();
    visitor.session().await;
    let old = visitor
        .post(
            "/app/api/sign-in",
            &json!({"email": "victim@example.test", "password": PASSWORD}),
        )
        .await;
    assert_eq!(old.json()["refusal"], json!("wrong_password"));
    visitor.sign_in("victim@example.test", OTHER_PASSWORD).await;
}

#[tokio::test]
async fn a_password_reset_works_once_from_the_body_token_and_ends_other_sessions() {
    let harness = Harness::new().await;
    let mut owner = harness.browser();
    owner.sign_up_verified("robin@example.test").await;
    let mut visitor = harness.browser();
    visitor.session().await;
    visitor
        .post(
            "/app/api/password/forgot",
            &json!({"email": "robin@example.test"}),
        )
        .await;
    harness.mail.wait_for(2).await;
    let token = harness
        .mailed_token(1, "robin@example.test", MailPurpose::PasswordReset)
        .await;
    let query = visitor
        .send(
            Req::new(
                Method::POST,
                &format!("/app/api/password/reset?token={token}"),
            )
            .json(&json!({"password": OTHER_PASSWORD, "confirmation": OTHER_PASSWORD})),
        )
        .await;
    assert_eq!(query.code(), "invalid");
    let mismatch = visitor
        .post(
            "/app/api/password/reset",
            &json!({"token": token, "password": OTHER_PASSWORD, "confirmation": "different one here"}),
        )
        .await;
    assert_eq!(mismatch.json()["refusal"], json!("password_mismatch"));
    let reset = visitor
        .post(
            "/app/api/password/reset",
            &json!({"token": token, "password": OTHER_PASSWORD, "confirmation": OTHER_PASSWORD}),
        )
        .await;
    assert_eq!(reset.json(), json!("reset"));
    let reused = visitor
        .post(
            "/app/api/password/reset",
            &json!({"token": token, "password": OTHER_PASSWORD, "confirmation": OTHER_PASSWORD}),
        )
        .await;
    assert_eq!(reused.json(), json!("invalid_link"));
    assert_eq!(owner.get("/app/api/overview").await.code(), "signed_out");
    visitor.sign_in("robin@example.test", OTHER_PASSWORD).await;
}

#[tokio::test]
async fn forgot_password_answers_the_same_for_known_and_unknown_addresses() {
    let harness = Harness::new().await;
    let mut known = harness.browser();
    known.sign_up_verified("robin@example.test").await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let known_reply = visitor
        .post(
            "/app/api/password/forgot",
            &json!({"email": "robin@example.test"}),
        )
        .await;
    let unknown_reply = visitor
        .post(
            "/app/api/password/forgot",
            &json!({"email": "nobody@example.test"}),
        )
        .await;
    assert_eq!(known_reply.status, unknown_reply.status);
    assert_eq!(known_reply.body, unknown_reply.body);
}

#[tokio::test]
async fn sign_in_issues_a_new_session_and_csrf_token_and_clears_the_pre_session() {
    let harness = Harness::new().await;
    let mut owner = harness.browser();
    owner.sign_up_verified("robin@example.test").await;
    let mut browser = harness.browser();
    let anonymous = browser.session().await;
    let before = browser.cookie(crate::PRESESSION_COOKIE).unwrap().to_owned();
    let reply = browser
        .post(
            "/app/api/sign-in",
            &json!({"email": "robin@example.test", "password": PASSWORD}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let session_cookie = reply.set_cookie(crate::SESSION_COOKIE).unwrap();
    assert!(session_cookie.contains("HttpOnly") && session_cookie.contains("Secure"));
    assert!(session_cookie.contains("SameSite=Lax"));
    assert_ne!(browser.cookie(crate::SESSION_COOKIE), Some(before.as_str()));
    assert!(browser.cookie(crate::PRESESSION_COOKIE).is_none());
    let view = reply.json();
    assert_ne!(view["csrf"], anonymous["csrf"]);
    assert_eq!(view["account"]["email"], json!("robin@example.test"));
    let old_token = anonymous["csrf"].as_str().unwrap().to_owned();
    browser.csrf = view["csrf"].as_str().unwrap().to_owned();
    let stale = browser
        .send(
            Req::new(Method::POST, "/app/api/settings/sessions/revoke-others")
                .csrf(Some(&old_token))
                .json(&json!({})),
        )
        .await;
    assert_eq!(stale.code(), "csrf_refused");
}

#[tokio::test]
async fn signing_in_again_from_a_signed_in_browser_ends_the_earlier_session() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let first = browser.cookie(crate::SESSION_COOKIE).unwrap().to_owned();
    browser.sign_in("robin@example.test", PASSWORD).await;
    let second = browser.cookie(crate::SESSION_COOKIE).unwrap().to_owned();
    assert_ne!(first, second);
    assert!(
        harness
            .service
            .authenticate(&first)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        harness
            .service
            .authenticate(&second)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_address_are_refused_identically() {
    let harness = Harness::new().await;
    let mut owner = harness.browser();
    owner.sign_up_verified("robin@example.test").await;
    let mut visitor = harness.browser();
    visitor.session().await;
    let wrong = visitor
        .post(
            "/app/api/sign-in",
            &json!({"email": "robin@example.test", "password": "not the password"}),
        )
        .await;
    let unknown = visitor
        .post(
            "/app/api/sign-in",
            &json!({"email": "nobody@example.test", "password": "not the password"}),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(wrong.status, unknown.status);
    assert_eq!(wrong.body, unknown.body);
    assert_eq!(wrong.json()["refusal"], json!("wrong_password"));
}

#[tokio::test]
async fn sign_out_ends_the_session_clears_the_cookie_and_returns_an_anonymous_view() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.sign_up_verified("robin@example.test").await;
    let token = browser.cookie(crate::SESSION_COOKIE).unwrap().to_owned();
    let reply = browser.post("/app/api/sign-out", &json!({})).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["account"], Value::Null);
    assert!(
        reply
            .set_cookie(crate::SESSION_COOKIE)
            .unwrap()
            .contains("Max-Age=0")
    );
    assert!(
        harness
            .service
            .authenticate(&token)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(browser.get("/app/api/overview").await.code(), "signed_out");
}

#[tokio::test]
async fn an_operation_read_returns_a_fresh_key_each_time() {
    let harness = Harness::new().await;
    let mut visitor = harness.browser();
    let first = visitor.operation().await;
    let second = visitor.operation().await;
    assert_eq!(first.len(), 32);
    assert_ne!(first, second);
}

#[tokio::test]
async fn resending_the_verification_mail_needs_a_session_and_answers_for_verified_accounts_too() {
    let harness = Harness::new().await;
    let mut browser = harness.browser();
    browser.session().await;
    let anonymous = browser
        .post("/app/api/verify-email/resend", &json!({}))
        .await;
    assert_eq!(anonymous.code(), "signed_out");
    browser.sign_up_verified("robin@example.test").await;
    let reply = browser
        .post("/app/api/verify-email/resend", &json!({}))
        .await;
    assert_eq!(reply.json(), json!({"accepted": true}));
}
