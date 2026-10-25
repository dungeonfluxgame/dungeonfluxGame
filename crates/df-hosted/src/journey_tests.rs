//! JOURNEY-01 exercises the real HTTP router with scratch PostgreSQL and no provider traffic.

use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use futures::FutureExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::{Hosted, config::Config, database::Connection};

const ORIGIN: &str = "http://127.0.0.1:8099";
const PASSWORD: &str = "lantern under the old mill";

struct Fixture {
    admin: Connection,
    database: String,
    url: String,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let config = "host=127.0.0.1 port=55911 user=postgres dbname=postgres"
            .parse()
            .unwrap();
        let admin = Connection::open(
            &config,
            &df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
        )
        .await
        .unwrap();
        let token = df_customer::SecretToken::generate().unwrap();
        let suffix = &token.text()[..16];
        let database = format!("journeywire_{suffix}");
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/agents/JOURNEYWIRE")
            .join(&database);
        std::fs::create_dir_all(root.join("bundle")).unwrap();
        std::fs::write(
            root.join("bundle/index.html"),
            "<!doctype html><title>Hosted fixture</title>",
        )
        .unwrap();
        Self {
            admin,
            url: format!("host=127.0.0.1 port=55911 user=postgres dbname={database}"),
            database,
            root,
        }
    }

    fn config(&self) -> Config {
        let values = BTreeMap::from([
            ("DF_PROFILE", "development".to_owned()),
            ("DF_TRIAL_EMAIL_HMAC_KEY", "33".repeat(32)),
            ("DF_CAMPAIGN_IP_KEY", "11".repeat(32)),
            ("DF_CAMPAIGN_CODE_KEY", "22".repeat(32)),
            ("DF_GAMEPLAY_PUBLIC_URL", ORIGIN.to_owned()),
            ("DF_GAMEPLAY_DATABASE_TLS", "loopback-plaintext".to_owned()),
            ("DF_PORTAL_DATABASE_URL", self.url.clone()),
            ("DF_PORTAL_MIGRATOR_DATABASE_URL", self.url.clone()),
            (
                "DF_PORTAL_BUNDLE_DIR",
                self.root.join("bundle").to_string_lossy().into_owned(),
            ),
            (
                "DF_PORTAL_OUTBOX",
                self.root.join("outbox").to_string_lossy().into_owned(),
            ),
        ]);
        Config::parse(|key| values.get(key).cloned()).unwrap()
    }

    async fn finish(self) {
        assert!(self.database.starts_with("journeywire_"));
        assert!(
            self.database
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        );
        self.admin
            .client
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.database))
            .await
            .unwrap();
        let root = self.root.canonicalize().unwrap();
        let permitted = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/agents/JOURNEYWIRE")
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(permitted));
        std::fs::remove_dir_all(root).unwrap();
    }

    async fn token(&self) -> String {
        let outbox = df_customer::DevOutbox::open(self.root.join("outbox"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(entry) = outbox.index().await.unwrap().last() {
                    let mail = outbox.message(entry.sequence).await.unwrap().unwrap();
                    let (_, tail) = mail.split_once("#token=").unwrap();
                    break tail.chars().take_while(char::is_ascii_hexdigit).collect();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
}

struct Browser {
    router: Router,
    cookies: BTreeMap<String, String>,
    csrf: String,
}

impl Browser {
    fn new(router: Router) -> Self {
        Self {
            router,
            cookies: BTreeMap::new(),
            csrf: String::new(),
        }
    }

    async fn request(&mut self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("origin", ORIGIN)
            .header("sec-fetch-site", "same-origin")
            .header(
                header::CONTENT_TYPE,
                if path.starts_with("/app/dev/") && method == "POST" {
                    "application/x-www-form-urlencoded"
                } else {
                    "application/json"
                },
            )
            .header("x-df-csrf", &self.csrf)
            .header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
            .body(if method == "GET" {
                Body::empty()
            } else {
                Body::from(if path.starts_with("/app/dev/") {
                    format!("csrf={}", self.csrf)
                } else {
                    body.to_string()
                })
            })
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 43210))));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        for cookie in response.headers().get_all(header::SET_COOKIE) {
            let pair = cookie.to_str().unwrap().split(';').next().unwrap();
            let (key, value) = pair.split_once('=').unwrap();
            self.cookies.insert(key.to_owned(), value.to_owned());
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(csrf) = parsed.get("csrf").and_then(Value::as_str) {
            self.csrf = csrf.to_owned();
        }
        (status, parsed)
    }

    async fn operation(&mut self) -> String {
        let (status, body) = self.request("GET", "/app/api/operation", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        body["operation"].as_str().unwrap().to_owned()
    }

    async fn verified(&mut self, fixture: &Fixture) {
        assert_eq!(
            self.request("GET", "/app/api/session", Value::Null).await.0,
            StatusCode::OK
        );
        let operation = self.operation().await;
        let (status, body) = self.request("POST", "/app/api/sign-up", json!({"email":"journey@example.test","password":PASSWORD,"is_adult":true,"accepts_terms":true,"terms_version":"terms-2026-10","operation":operation})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let token = fixture.token().await;
        let (status, body) = self
            .request(
                "POST",
                "/app/api/verify-email",
                json!({"token":token,"password":PASSWORD,"confirmation":PASSWORD}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            self.request("GET", "/app/api/session", Value::Null).await.0,
            StatusCode::OK
        );
        let (status, body) = self
            .request(
                "POST",
                "/app/api/sign-in",
                json!({"email":"journey@example.test","password":PASSWORD}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["account"]["is_verified"], true);
    }

    async fn top_up(&mut self) {
        let (status, before) = self.request("GET", "/app/api/overview", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{before}");
        assert_eq!(before["balance"]["purchased_available_micro_dollars"], 0);
        assert_eq!(
            before["balance"]["promotional_available_micro_dollars"],
            10_000_000
        );
        let operation = self.operation().await;
        let body = json!({"amount_cents":2000,"operation":operation});
        let (status, checkout) = self
            .request("POST", "/app/api/credit/checkouts", body.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{checkout}");
        assert_eq!(
            self.request("POST", "/app/api/credit/checkouts", body)
                .await
                .1,
            checkout
        );
        let id = checkout["checkout_id"].as_str().unwrap();
        let (status, pending) = self
            .request(
                "GET",
                &format!("/app/api/credit/checkouts/{id}"),
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(pending["status"], "pending");
        assert_eq!(
            self.request("GET", "/app/api/overview", Value::Null)
                .await
                .1["balance"]["purchased_available_micro_dollars"],
            0
        );
        let (status, _) = self
            .request("GET", &format!("/app/dev/checkouts/{id}"), Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK);
        let original_csrf = std::mem::replace(&mut self.csrf, "00".repeat(32));
        let (status, _) = self
            .request("POST", &format!("/app/dev/checkouts/{id}"), json!({}))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        self.csrf = original_csrf;
        for _ in 0..2 {
            assert_eq!(
                self.request("POST", &format!("/app/dev/checkouts/{id}"), json!({}))
                    .await
                    .0,
                StatusCode::SEE_OTHER
            );
        }
        let (status, after) = self.request("GET", "/app/api/overview", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{after}");
        assert_eq!(
            after["balance"]["purchased_available_micro_dollars"],
            20_000_000
        );
        assert_eq!(
            after["balance"]["promotional_available_micro_dollars"],
            10_000_000
        );
    }
}

#[tokio::test]
async fn real_http_accounts_and_finance_survive_restart_and_campaign_creation_refuses_missing_tenant()
 {
    with_fixture(async |fixture| {
        let hosted = Hosted::open(fixture.config()).await.unwrap();
        let mut browser = Browser::new(hosted.router());
        browser.verified(fixture).await;
        browser.top_up().await;
        let (status, body) = browser
            .request("POST", "/app/api/campaigns/drafts", json!({}))
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["code"], "unavailable");
        drop(browser.router);
        hosted.shutdown().await;
        let reopened = Hosted::open(fixture.config()).await.unwrap();
        browser.router = reopened.router();
        let (status, body) = browser
            .request("GET", "/app/api/overview", Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["balance"]["purchased_available_micro_dollars"],
            20_000_000
        );
        assert_eq!(
            body["balance"]["promotional_available_micro_dollars"],
            10_000_000
        );
        drop(browser);
        reopened.shutdown().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "JOURNEY-01 pending account-owner tenant issuance, finance sponsor consent/open-funding hook, runtime owner placement, and STRIPETEST checkout qualification"]
async fn journey_01_signup_verify_signin_topup_create_campaign_open_lobby_start_session() {
    with_fixture(async |fixture| {
    let hosted = Hosted::open(fixture.config()).await.unwrap();
    let mut browser = Browser::new(hosted.router());
    browser.verified(fixture).await;
    browser.top_up().await;
    let (status, draft) = browser
        .request("POST", "/app/api/campaigns/drafts", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "JOURNEY-01 tenant binding: {draft}");
    let id = draft["draft_id"].as_str().unwrap();
    let draft = json!({"name":"Lantern Road","adventure_id":"bundled","content_rating":"teen","seat_count":1,"hero_choice":"in_lobby","has_voice":false,"pictures":"existing_art_only","pace":"story","custom_ceiling_cents_per_hour":null,"co_hosts_can_invite":false,"co_hosts_can_start":false,"co_hosts_can_export":false,"join_mode":"link_only","funding":"sponsor"});
    for step in 1..=5 {
        assert_eq!(
            browser
                .request(
                    "PUT",
                    &format!("/app/api/campaigns/drafts/{id}/steps/{step}"),
                    json!({"draft":draft})
                )
                .await
                .0,
            StatusCode::OK
        );
    }
    let operation = browser.operation().await;
    let (status, campaign) = browser
        .request(
            "POST",
            &format!("/app/api/campaigns/drafts/{id}/finish"),
            json!({"operation":operation}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{campaign}");
    let campaign = campaign["campaign_id"].as_str().unwrap();
    let operation = browser.operation().await;
    let (status, lobby) = browser
        .request(
            "POST",
            &format!("/app/api/campaigns/{campaign}/lobby"),
            json!({"operation":operation}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{lobby}");
    let session = lobby["session_id"].as_str().unwrap();
    let operation = browser.operation().await;
    let (status, chosen) = browser.request("POST", &format!("/app/api/sessions/{session}/lobby/character"),
        json!({"choice":"template", "reference":"12".repeat(16), "operation":operation})).await;
    assert_eq!(status, StatusCode::OK, "{chosen}");
    let operation = browser.operation().await;
    assert_eq!(
        browser
            .request(
                "POST",
                &format!("/app/api/sessions/{session}/lobby/microphone"),
                json!({"prefers_typing":true,"permission":null,"operation":operation})
            )
            .await
            .0,
        StatusCode::OK
    );
    let operation = browser.operation().await;
    assert_eq!(
        browser
            .request(
                "POST",
                &format!("/app/api/sessions/{session}/lobby/ready"),
                json!({"operation":operation})
            )
            .await
            .0,
        StatusCode::OK
    );
    let operation = browser.operation().await;
    let (status, started) = browser
        .request(
            "POST",
            &format!("/app/api/sessions/{session}/lobby/start"),
            json!({"operation":operation}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["has_started"], true);
    drop(browser);
    hosted.shutdown().await;
    }).await;
}

#[tokio::test]
async fn migration_failure_rolls_back_the_wrapped_first_migration_and_reruns_are_idempotent() {
    with_fixture(async |fixture| {
        let config = fixture.url.parse().unwrap();
        let mut connection = Connection::open(
            &config,
            &df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
        )
        .await
        .unwrap();
        let first = include_str!("../../df-persistence/migrations/0001_game_decisions.sql");
        let error = crate::database::apply(
            &mut connection,
            &[(1, first), (999, "SELECT journeywire_missing_function()")],
        )
        .await;
        assert!(matches!(
            error,
            Err(crate::database::DatabaseError::Migration { number: 999, .. })
        ));
        let row = connection
            .client
            .query_one("SELECT to_regnamespace('df_game') IS NULL", &[])
            .await
            .unwrap();
        assert!(row.get::<_, bool>(0));
        crate::database::apply(&mut connection, &[(1, first)])
            .await
            .unwrap();
        crate::database::apply(&mut connection, &[(1, first)])
            .await
            .unwrap();
        let row = connection
            .client
            .query_one("SELECT count(*) FROM hosted.migration", &[])
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), 1);
        drop(connection);
    })
    .await;
}

async fn with_fixture(test: impl for<'a> AsyncFnOnce(&'a Fixture)) {
    let fixture = Fixture::new().await;
    let outcome = std::panic::AssertUnwindSafe(test(&fixture))
        .catch_unwind()
        .await;
    fixture.finish().await;
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

// The account owner's tenant-creation migration/verification hook is still pending.
// This fixture provisions its specified binding explicitly, never deriving a tenant from an account.
async fn provision_tenant(fixture: &Fixture) {
    let connection = Connection::open(
        &fixture.url.parse().unwrap(),
        &df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
    )
    .await
    .unwrap();
    connection.client.batch_execute("CREATE TABLE accounts.tenant (tenant_id bytea PRIMARY KEY, owner_account_id bytea NOT NULL UNIQUE REFERENCES accounts.account(account_id)); INSERT INTO accounts.tenant SELECT decode('11121314151617181920212223242526', 'hex'), account_id FROM accounts.account WHERE email_normalized = 'journey@example.test'").await.unwrap();
}

async fn create_bound_campaign(browser: &mut Browser) -> String {
    let (status, view) = browser
        .request("POST", "/app/api/campaigns/drafts", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let id = view["draft_id"].as_str().unwrap();
    let draft = json!({"name":"Lantern Road","adventure_id":"bundled","content_rating":"teen","seat_count":1,"hero_choice":"in_lobby","has_voice":false,"pictures":"existing_art_only","pace":"story","custom_ceiling_cents_per_hour":null,"co_hosts_can_invite":false,"co_hosts_can_start":false,"co_hosts_can_export":false,"join_mode":"link_only","funding":"sponsor"});
    for step in 1..=5 {
        let (status, body) = browser
            .request(
                "PUT",
                &format!("/app/api/campaigns/drafts/{id}/steps/{step}"),
                json!({"draft":draft}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "step {step}: {body}");
    }
    let operation = browser.operation().await;
    let request = json!({"operation":operation});
    let path = format!("/app/api/campaigns/drafts/{id}/finish");
    let (status, created) = browser.request("POST", &path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(browser.request("POST", &path, request).await.1, created);
    created["campaign_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_bound_tenant_can_create_a_campaign_and_reopen_its_lobby_with_hero_and_ready_persisted() {
    with_fixture(async |fixture| {
        let hosted = Hosted::open(fixture.config()).await.unwrap();
        let mut browser = Browser::new(hosted.router());
        browser.verified(fixture).await;
        browser.top_up().await;
        provision_tenant(fixture).await;
        let campaign = create_bound_campaign(&mut browser).await;
        let operation = browser.operation().await;
        let path = format!("/app/api/campaigns/{campaign}/lobby");
        let (status, opened) = browser
            .request("POST", &path, json!({"operation":operation}))
            .await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        let operation = browser.operation().await;
        assert_eq!(
            browser
                .request("POST", &path, json!({"operation":operation}))
                .await
                .1,
            opened
        );
        let session = opened["session_id"].as_str().unwrap();
        let path = format!("/app/api/sessions/{session}/lobby");
        let operation = browser.operation().await;
        assert_ne!(
            browser
                .request(
                    "POST",
                    &format!("{path}/ready"),
                    json!({"operation":operation})
                )
                .await
                .0,
            StatusCode::OK
        );
        let operation = browser.operation().await;
        let (status, chosen) = browser
            .request(
                "POST",
                &format!("{path}/character"),
                json!({"choice":"template", "reference":"12".repeat(16), "operation":operation}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{chosen}");
        assert!(chosen["members"][0]["character_name"].is_string());
        let operation = browser.operation().await;
        let (status, ready) = browser
            .request(
                "POST",
                &format!("{path}/ready"),
                json!({"operation":operation}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{ready}");
        assert_eq!(ready["members"][0]["is_ready"], true);
        assert_eq!(ready["funding_note"], "waiting_for_sponsor");
        assert!(
            ready["start_blockers"]
                .as_array()
                .unwrap()
                .contains(&json!("funding_not_ready"))
        );
        // The $30 wallet is real; it has not been consented/allocated to this session.
        // A successful balance read must not silently approve sponsorship.
        drop(browser.router);
        hosted.shutdown().await;
        let reopened = Hosted::open(fixture.config()).await.unwrap();
        browser.router = reopened.router();
        let (status, view) = browser.request("GET", &path, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["members"][0]["is_ready"], true);
        assert!(view["members"][0]["character_name"].is_string());
        let connection = Connection::open(&fixture.url.parse().unwrap(), &df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly).await.unwrap();
        let consumed = connection.client.query_one("SELECT f.last_event_seq = c.event_seq AND (SELECT count(*) FROM finance.funding_effect e WHERE e.campaign_id = c.campaign_id) = c.event_seq FROM finance.funding_feed_position f JOIN campaigns.campaign c USING (campaign_id)", &[]).await.unwrap();
        assert!(consumed.get::<_, bool>(0));
        drop(connection);
        drop(browser);
        reopened.shutdown().await;
    })
    .await;
}

#[tokio::test]
async fn session_start_requires_a_live_owner_fence_and_freezes_the_ready_roster() {
    use df_campaign::lobby_state::SeatedMember;
    use df_campaign::store::{SessionDirectory, SessionPhase};
    with_fixture(async |fixture| {
        let hosted = Hosted::open(fixture.config()).await.unwrap();
        let mut browser = Browser::new(hosted.router());
        browser.verified(fixture).await;
        provision_tenant(fixture).await;
        let campaign = create_bound_campaign(&mut browser).await;
        let campaign = df_types::CampaignId::from_hex(&campaign).unwrap();
        let transport = df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly;
        let connection = Connection::open(&fixture.url.parse().unwrap(), &transport).await.unwrap();
        let sessions = crate::campaign_sessions::Sessions::new(connection);
        let (session, phase) = sessions.open_session(campaign).await.unwrap();
        assert_eq!(phase, SessionPhase::Lobby);
        let db = Connection::open(&fixture.url.parse().unwrap(), &transport).await.unwrap();
        let row = db.client.query_one("SELECT member_id, account_id FROM campaigns.campaign_member WHERE campaign_id = $1", &[&campaign.as_bytes().as_slice()]).await.unwrap();
        let member = df_types::MemberId::from_bytes(&row.get::<_, Vec<u8>>(0)).unwrap();
        let account = df_types::AccountId::from_bytes(&row.get::<_, Vec<u8>>(1)).unwrap();
        db.client.execute("INSERT INTO campaigns.lobby_member (session_id, member_id, campaign_id, character_choice, character_ref, ready, updated_at_ms) VALUES ($1, $2, $3, 'template', $4, true, 1)", &[&session.as_bytes().as_slice(), &member.as_bytes().as_slice(), &campaign.as_bytes().as_slice(), &vec![12_u8; 16]]).await.unwrap();
        let roster = [SeatedMember { member, account }];
        assert_eq!(sessions.start_session(session, campaign, &roster).await, Err(df_campaign::errors::StoreError::FenceStale));
        assert_eq!(sessions.session(session).await.unwrap().unwrap().phase, SessionPhase::Lobby);
        // Fixture-only O1 owner. The production adapter never inserts or renews this fence.
        db.client.execute("INSERT INTO df_runtime.campaign_owners (tenant_id, campaign_id, recovery_epoch, claim_seq, state, placement_id, lease_until) SELECT tenant_id, campaign_id, 1, 1, 'serving', $2, clock_timestamp() + interval '1 hour' FROM campaigns.campaign WHERE campaign_id = $1", &[&campaign.as_bytes().as_slice(), &vec![13_u8; 16]]).await.unwrap();
        sessions.start_session(session, campaign, &roster).await.unwrap();
        assert_eq!(sessions.session(session).await.unwrap().unwrap().phase, SessionPhase::PreparingFundingOpen);
        let row = db.client.query_one("SELECT frozen_at_ms IS NOT NULL FROM campaigns.lobby_member WHERE session_id = $1 AND member_id = $2", &[&session.as_bytes().as_slice(), &member.as_bytes().as_slice()]).await.unwrap();
        assert!(row.get::<_, bool>(0));
        drop(sessions);
        drop(db);
        drop(browser);
        hosted.shutdown().await;
    }).await;
}
