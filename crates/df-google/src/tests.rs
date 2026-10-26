use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use df_customer::{
    HostConfig, OAuthAuthorize, OAuthExchange, OAuthFailure, OAuthProvider, OAuthSecret,
    pkce_challenge,
};
use serde_json::{Value, json};

fn claims() -> Value {
    json!({"iss":"https://accounts.google.com","sub":"subject","aud":"client", "exp":2000,
        "iat":900,"nonce":"nonce","email":"google@example.test","email_verified":true})
}

fn validate(value: Value, fresh: bool) -> Result<df_customer::OAuthIdentity, OAuthFailure> {
    let token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
    );
    super::claims::validate(
        &token,
        "client",
        &OAuthExchange {
            code: &OAuthSecret::new("code".to_owned()),
            verifier: &OAuthSecret::new("verifier".to_owned()),
            nonce: &OAuthSecret::new("nonce".to_owned()),
            now_s: 1000,
            require_fresh: fresh,
        },
    )
}

#[test]
fn authorization_contains_exact_google_parameters_and_s256_vector() {
    let hosts = HostConfig::local("http://127.0.0.1:8099").unwrap();
    let google = Google::new(
        Config::new(
            "client".to_owned(),
            OAuthSecret::new("secret".to_owned()),
            &hosts,
        )
        .unwrap(),
    )
    .unwrap();
    let verifier = OAuthSecret::new("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned());
    let url = google
        .authorize(OAuthAuthorize {
            state: &OAuthSecret::new("a&b".to_owned()),
            nonce: &OAuthSecret::new("nonce".to_owned()),
            challenge: pkce_challenge(&verifier),
            require_fresh: true,
        })
        .unwrap();
    let url = reqwest::Url::parse(url.expose()).unwrap();
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://accounts.google.com"
    );
    assert_eq!(url.path(), "/o/oauth2/v2/auth");
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
    for (key, value) in [
        ("state", "a&b"),
        ("nonce", "nonce"),
        ("scope", "openid email profile"),
        ("response_type", "code"),
        ("client_id", "client"),
        (
            "redirect_uri",
            "http://127.0.0.1:8099/app/api/oauth/google/callback",
        ),
        ("code_challenge_method", "S256"),
        (
            "code_challenge",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        ),
        ("max_age", "0"),
        ("prompt", "select_account"),
    ] {
        assert_eq!(pairs.get(key).unwrap(), value);
    }
}

#[test]
fn claims_reject_wrong_issuer_audience_party_expiry_nonce_and_unverified_email() {
    assert!(validate(claims(), false).is_ok());
    for (key, value) in [
        ("iss", json!("evil")),
        ("aud", json!("evil")),
        ("azp", json!("evil")),
        ("azp", Value::Null),
        ("exp", json!(1000)),
        ("iat", json!(1100)),
        ("nonce", json!("wrong")),
        ("email_verified", json!(false)),
    ] {
        let mut data = claims();
        data[key] = value;
        assert!(validate(data, false).is_err(), "{key}");
    }
}

#[test]
fn fresh_authentication_requires_recent_auth_time() {
    assert!(matches!(
        validate(claims(), true),
        Err(OAuthFailure::FreshAuthenticationRequired)
    ));
    let mut data = claims();
    data["auth_time"] = json!(699);
    assert!(validate(data.clone(), true).is_err());
    data["auth_time"] = json!(700);
    assert!(validate(data, true).is_ok());
}

#[tokio::test]
async fn fake_codes_are_bound_to_pkce_nonce_and_single_use() {
    let fake = fake::FakeGoogle::new("http://127.0.0.1:8099");
    let state = OAuthSecret::new("state".to_owned());
    let nonce = OAuthSecret::new("nonce".to_owned());
    let verifier = OAuthSecret::new("verifier".to_owned());
    fake.authorize(OAuthAuthorize {
        state: &state,
        nonce: &nonce,
        challenge: pkce_challenge(&verifier),
        require_fresh: false,
    })
    .unwrap();
    let exchange = || OAuthExchange {
        code: &state,
        verifier: &verifier,
        nonce: &nonce,
        now_s: 1000,
        require_fresh: false,
    };
    assert!(fake.exchange(exchange()).await.is_ok());
    assert!(matches!(
        fake.exchange(exchange()).await,
        Err(OAuthFailure::Rejected)
    ));
    for (wrong_verifier, wrong_nonce) in [(true, false), (false, true)] {
        fake.authorize(OAuthAuthorize {
            state: &state,
            nonce: &nonce,
            challenge: pkce_challenge(&verifier),
            require_fresh: false,
        })
        .unwrap();
        let wrong = OAuthSecret::new("wrong".to_owned());
        assert!(matches!(
            fake.exchange(OAuthExchange {
                code: &state,
                verifier: if wrong_verifier { &wrong } else { &verifier },
                nonce: if wrong_nonce { &wrong } else { &nonce },
                now_s: 1000,
                require_fresh: false,
            })
            .await,
            Err(OAuthFailure::Rejected)
        ));
        assert!(fake.exchange(exchange()).await.is_err());
    }
}

#[tokio::test]
async fn fake_cancellation_returns_state_and_no_authorization_code() {
    let fake = fake::FakeGoogle::new("http://127.0.0.1:8099");
    let state = OAuthSecret::new("cancel-state".to_owned());
    let nonce = OAuthSecret::new("nonce".to_owned());
    fake.cancel_next_authorization().unwrap();
    let answer = fake
        .authorize(OAuthAuthorize {
            state: &state,
            nonce: &nonce,
            challenge: [0; 32],
            require_fresh: false,
        })
        .unwrap();
    let url = reqwest::Url::parse(answer.expose()).unwrap();
    let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(fields.get("state").unwrap(), "cancel-state");
    assert_eq!(fields.get("error").unwrap(), "access_denied");
    assert!(!fields.contains_key("code"));
}

#[tokio::test(start_paused = true)]
async fn fake_abandoned_codes_expire_and_failure_controls_do_not_retain_codes() {
    let fake = fake::FakeGoogle::new("http://127.0.0.1:8099");
    let nonce = OAuthSecret::new("nonce".to_owned());
    let verifier = OAuthSecret::new("verifier".to_owned());
    for number in 0..128 {
        fake.authorize(OAuthAuthorize {
            state: &OAuthSecret::new(number.to_string()),
            nonce: &nonce,
            challenge: pkce_challenge(&verifier),
            require_fresh: false,
        })
        .unwrap();
    }
    let state = OAuthSecret::new("after-expiry".to_owned());
    assert!(
        fake.authorize(OAuthAuthorize {
            state: &state,
            nonce: &nonce,
            challenge: pkce_challenge(&verifier),
            require_fresh: false
        })
        .is_err()
    );
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    for failure in [
        OAuthFailure::EmailNotVerified,
        OAuthFailure::Unavailable,
        OAuthFailure::Malformed,
        OAuthFailure::FreshAuthenticationRequired,
    ] {
        fake.authorize(OAuthAuthorize {
            state: &state,
            nonce: &nonce,
            challenge: pkce_challenge(&verifier),
            require_fresh: false,
        })
        .unwrap();
        fake.fail_next_exchange(failure).unwrap();
        let result = fake
            .exchange(OAuthExchange {
                code: &state,
                verifier: &verifier,
                nonce: &nonce,
                now_s: 1000,
                require_fresh: false,
            })
            .await;
        assert!(matches!(result,Err(error) if error==failure));
    }
}

async fn transport_exchange(
    response: Option<(u16, Vec<u8>)>,
) -> (Result<df_customer::OAuthIdentity, OAuthFailure>, String) {
    delayed_transport_exchange(response, 0).await
}

async fn delayed_transport_exchange(
    response: Option<(u16, Vec<u8>)>,
    delay_s: u64,
) -> (Result<df_customer::OAuthIdentity, OAuthFailure>, String) {
    let mut endpoint = super::token_endpoint::Endpoint::open().await;
    let hosts = HostConfig::local("http://127.0.0.1:8099").unwrap();
    let google = Google::loopback_fixture(
        Config::new(
            "client".to_owned(),
            OAuthSecret::new("test-secret".to_owned()),
            &hosts,
        )
        .unwrap(),
        &endpoint.url,
    )
    .unwrap();
    let exchange = tokio::spawn(async move {
        google
            .exchange(OAuthExchange {
                code: &OAuthSecret::new("code&value".to_owned()),
                verifier: &OAuthSecret::new("verifier".to_owned()),
                nonce: &OAuthSecret::new("nonce".to_owned()),
                now_s: 1000,
                require_fresh: false,
            })
            .await
    });
    let request = (&mut endpoint.request).await.unwrap();
    if delay_s > 0 {
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(delay_s)).await;
        tokio::time::resume();
    }
    let timeout = response.is_none();
    endpoint.release.take().unwrap().send(response).unwrap();
    if timeout {
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(11)).await;
    }
    let result = exchange.await.unwrap();
    if timeout {
        tokio::time::resume();
    }
    (result, request)
}

#[tokio::test]
async fn token_endpoint_claim_expiring_during_exchange_is_refused() {
    let mut data = claims();
    data["exp"] = json!(1001);
    let token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&data).unwrap())
    );
    let response = serde_json::to_vec(&json!({"id_token":token})).unwrap();
    let (result, _) = delayed_transport_exchange(Some((200, response)), 2).await;
    assert!(matches!(result, Err(OAuthFailure::Rejected)));
}

#[tokio::test]
async fn token_endpoint_exchange_sends_server_form_and_discards_other_tokens() {
    let token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims()).unwrap())
    );
    let body = serde_json::to_vec(
        &json!({"id_token":token,"access_token":"discarded","refresh_token":"discarded"}),
    )
    .unwrap();
    let (result, request) = transport_exchange(Some((200, body))).await;
    assert_eq!(result.unwrap().subject, "subject");
    assert!(request.starts_with("POST /token HTTP/1.1"));
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let url = reqwest::Url::parse(&format!("http://127.0.0.1/?{body}")).unwrap();
    let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
    for (key, value) in [
        ("grant_type", "authorization_code"),
        ("code", "code&value"),
        ("client_id", "client"),
        ("client_secret", "test-secret"),
        ("code_verifier", "verifier"),
        (
            "redirect_uri",
            "http://127.0.0.1:8099/app/api/oauth/google/callback",
        ),
    ] {
        assert_eq!(fields.get(key).unwrap(), value);
    }
}

#[tokio::test]
async fn token_endpoint_refuses_non_success_oversize_malformed_and_timeout() {
    for (response, expected) in [
        (Some((400, b"refused".to_vec())), OAuthFailure::Rejected),
        (
            Some((500, b"unavailable".to_vec())),
            OAuthFailure::Unavailable,
        ),
        (Some((302, b"redirect".to_vec())), OAuthFailure::Rejected),
        (Some((200, vec![b'x'; 65537])), OAuthFailure::Oversize),
        (Some((200, b"{}".to_vec())), OAuthFailure::Malformed),
        (None, OAuthFailure::Timeout),
    ] {
        let (result, _) = transport_exchange(response).await;
        assert!(
            matches!(result,Err(error) if error==expected),
            "expected {expected:?}"
        );
    }
}

#[test]
fn oauth_secrets_are_redacted() {
    let secret = OAuthSecret::new("never-print-me".to_owned());
    assert_eq!(format!("{secret:?}"), "OAuthSecret(<redacted>)");
}
