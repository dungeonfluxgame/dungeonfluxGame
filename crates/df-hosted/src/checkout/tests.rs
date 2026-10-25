//! Loopback Stripe and scratch PostgreSQL exercise the real payment boundary.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{Form, State},
    http::{HeaderMap, Request, StatusCode},
    routing::post,
};
use df_persistence::{
    account_closure::TimeoutPolicy, account_store::PostgresAccountStore,
    postgres_tls::PostgresTransport,
};
use futures::FutureExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

const ORIGIN: &str = "http://127.0.0.1:8099";
const SECRET: &str = "whsec_stripetest_fixture";
fn account() -> AccountId {
    AccountId::from_bytes(&[51; 16]).unwrap()
}

#[derive(Clone, Copy, Default)]
enum Reply {
    #[default]
    Session,
    Redirect,
    Unavailable,
    ForeignUrl,
    Live,
    Large,
}

#[derive(Default)]
struct Requests {
    rows: tokio::sync::Mutex<Vec<BTreeMap<String, String>>>,
    reply: Reply,
}

struct FakeStripe {
    endpoint: String,
    requests: Arc<Requests>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeStripe {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeStripe {
    async fn new() -> Self {
        Self::with_reply(Reply::Session).await
    }

    async fn with_reply(reply: Reply) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/checkout/sessions",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(Requests {
            reply,
            ..Requests::default()
        });
        let app = Router::new()
            .route("/v1/checkout/sessions", post(create_session))
            .with_state(Arc::clone(&requests));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }

    fn settings(&self) -> Settings {
        let mut settings = Settings::parse(&|key| match key {
            "DF_STRIPE_SECRET_KEY" => Some("sk_test_stripetest_fixture".to_owned()),
            "DF_STRIPE_WEBHOOK_SECRET" => Some(SECRET.to_owned()),
            _ => None,
        })
        .unwrap()
        .unwrap();
        settings.endpoint = Some(self.endpoint.clone());
        settings
    }
}

async fn create_session(
    State(requests): State<Arc<Requests>>,
    headers: HeaderMap,
    Form(form): Form<BTreeMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let operation = form.get("metadata[operation_id]").unwrap().clone();
    assert_eq!(headers.get("idempotency-key").unwrap(), operation.as_str());
    assert!(headers.contains_key("authorization"));
    assert_eq!(form.get("mode").unwrap(), "payment");
    assert_eq!(
        form.get("line_items[0][price_data][currency]").unwrap(),
        "usd"
    );
    assert_eq!(form.get("line_items[0][quantity]").unwrap(), "1");
    assert_eq!(
        form.get("success_url").unwrap(),
        &format!("{ORIGIN}/app/funds?checkout={operation}")
    );
    assert_eq!(
        form.keys()
            .filter(|key| key.starts_with("metadata"))
            .count(),
        1
    );
    assert!(!form.keys().any(|key| key.contains("email")));
    requests.rows.lock().await.push(form);
    match requests.reply {
        Reply::Redirect => {
            return (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", "/v1/checkout/sessions")],
            )
                .into_response();
        }
        Reply::Unavailable => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Reply::Large => return "x".repeat(65_537).into_response(),
        _ => {}
    }
    let mut value = json!({"id":format!("cs_test_{operation}"),"url":format!("https://checkout.stripe.com/c/pay/{operation}"),"livemode":false});
    if matches!(requests.reply, Reply::ForeignUrl) {
        value["url"] = json!("https://example.test/checkout");
    }
    if matches!(requests.reply, Reply::Live) {
        value["livemode"] = json!(true);
    }
    Json(value).into_response()
}

struct Fixture {
    admin: Connection,
    name: String,
    config: tokio_postgres::Config,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let admin = Connection::open(
            &"host=127.0.0.1 port=55911 user=postgres dbname=postgres"
                .parse()
                .unwrap(),
            &PostgresTransport::PlaintextLoopbackOnly,
        )
        .await
        .unwrap();
        let token = df_customer::SecretToken::generate().unwrap().text();
        let name = format!("stripetest_{}", &token[..16]);
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        let config = format!("host=127.0.0.1 port=55911 user=postgres dbname={name}")
            .parse()
            .unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/agents/STRIPETEST")
            .join(&name);
        Self {
            admin,
            name,
            config,
            root,
        }
    }

    async fn connection(&self) -> Connection {
        Connection::open(&self.config, &PostgresTransport::PlaintextLoopbackOnly)
            .await
            .unwrap()
    }

    async fn checkout(&self, fake: &FakeStripe) -> Checkout {
        let accounts = PostgresAccountStore::open(
            self.config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap();
        let mut connection = self.connection().await;
        crate::database::migrate(&mut connection).await.unwrap();
        crate::database::migrate(&mut connection).await.unwrap();
        connection.client.execute(
            "INSERT INTO accounts.account(account_id,email_normalized,email_display,email_verified_at_ms,status,credential_generation,created_at_ms) \
             VALUES($1,'stripetest@example.test','stripetest@example.test',1,'active',1,1) ON CONFLICT DO NOTHING", &[&&account().as_bytes()[..]],
        ).await.unwrap();
        let finance = Finance::new(connection, accounts, [33; 32]).unwrap();
        Checkout::configured(
            Profile::Demo,
            self.connection().await,
            finance,
            Some(fake.settings()),
            ORIGIN,
        )
        .await
        .unwrap()
    }

    async fn balance(&self) -> (i64, i64) {
        let connection = self.connection().await;
        let row = connection.client.query_one(
            "SELECT count(*),coalesce(sum(original_micro_dollars),0)::bigint FROM finance.funding_lot WHERE processor='stripe'", &[],
        ).await.unwrap();
        (row.get(0), row.get(1))
    }

    fn hosted_config(&self, fake: &FakeStripe) -> crate::config::Config {
        std::fs::create_dir_all(self.root.join("bundle")).unwrap();
        std::fs::write(
            self.root.join("bundle/index.html"),
            "<!doctype html><title>Stripe fixture</title>",
        )
        .unwrap();
        let values = BTreeMap::from([
            ("DF_PROFILE", "development".to_owned()),
            ("DF_TRIAL_EMAIL_HMAC_KEY", "33".repeat(32)),
            ("DF_CAMPAIGN_IP_KEY", "11".repeat(32)),
            ("DF_CAMPAIGN_CODE_KEY", "22".repeat(32)),
            ("DF_GAMEPLAY_PUBLIC_URL", ORIGIN.to_owned()),
            ("DF_GAMEPLAY_DATABASE_TLS", "loopback-plaintext".to_owned()),
            (
                "DF_PORTAL_DATABASE_URL",
                format!(
                    "host=127.0.0.1 port=55911 user=postgres dbname={}",
                    self.name
                ),
            ),
            (
                "DF_PORTAL_MIGRATOR_DATABASE_URL",
                format!(
                    "host=127.0.0.1 port=55911 user=postgres dbname={}",
                    self.name
                ),
            ),
            (
                "DF_PORTAL_BUNDLE_DIR",
                self.root.join("bundle").to_string_lossy().into_owned(),
            ),
            (
                "DF_PORTAL_OUTBOX",
                self.root.join("outbox").to_string_lossy().into_owned(),
            ),
        ]);
        let mut config = crate::config::Config::parse(|key| values.get(key).cloned()).unwrap();
        config.stripe = Some(fake.settings());
        config
    }

    async fn finish(self) {
        assert!(
            self.name.starts_with("stripetest_")
                && self
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        );
        self.admin
            .client
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .await
            .unwrap();
        if self.root.exists() {
            let root = self.root.canonicalize().unwrap();
            let permitted = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/agents/STRIPETEST")
                .canonicalize()
                .unwrap();
            assert!(root.starts_with(permitted));
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

async fn with_fixture(test: impl for<'a> AsyncFnOnce(&'a Fixture, &'a FakeStripe)) {
    let fixture = Fixture::new().await;
    let fake = FakeStripe::new().await;
    let outcome = std::panic::AssertUnwindSafe(test(&fixture, &fake))
        .catch_unwind()
        .await;
    fake.task.abort();
    fixture.finish().await;
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

fn event(operation: &str, id: &str) -> Value {
    json!({"id":id,"type":"checkout.session.completed","data":{"object":{
        "id":format!("cs_test_{operation}"),"payment_status":"paid","amount_total":2000,
        "currency":"usd","livemode":false,"mode":"payment","metadata":{"operation_id":operation}
    }}})
}

async fn deliver(app: &Router, event: &Value, secret: &str) -> StatusCode {
    let body = event.to_string();
    let timestamp = df_customer::Clock::now_ms(&df_customer::SystemClock) / 1000;
    let signed = webhook::signature(secret, body.as_bytes(), timestamp);
    app.clone()
        .oneshot(
            Request::post("/app/api/payments/stripe/webhook")
                .header("stripe-signature", signed)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn paid_webhooks_credit_once_across_replays_duplicate_events_and_restart() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = checkout
            .create(account(), 2000, OperationId::from_bytes(&[12; 16]).unwrap())
            .await
            .unwrap();
        let id = CheckoutId::from_hex(&created.checkout_id).unwrap();
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            CheckoutStatus::Pending
        );
        assert_eq!(fixture.balance().await, (0, 0));
        assert!(matches!(
            checkout
                .status(AccountId::from_bytes(&[52; 16]).unwrap(), id)
                .await,
            Err(PortError::NotFound)
        ));
        assert!(matches!(
            checkout.complete(account(), id).await,
            Err(PortError::Unavailable)
        ));
        assert_eq!(
            checkout
                .create(account(), 2000, OperationId::from_bytes(&[12; 16]).unwrap())
                .await
                .unwrap(),
            created
        );
        assert!(matches!(
            checkout
                .create(account(), 1000, OperationId::from_bytes(&[12; 16]).unwrap())
                .await,
            Err(PortError::Forbidden)
        ));
        assert!(matches!(
            checkout
                .create(account(), 1, OperationId::from_bytes(&[13; 16]).unwrap())
                .await,
            Err(PortError::Forbidden)
        ));
        let requests = fake.requests.rows.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .get("line_items[0][price_data][unit_amount]")
                .unwrap(),
            "2000"
        );
        drop(requests);
        let hosted = crate::Hosted::open(fixture.hosted_config(fake))
            .await
            .unwrap();
        let app = hosted.router();
        let returning = app
            .clone()
            .oneshot(
                Request::get(format!("/app/funds?checkout={}", created.checkout_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(returning.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            returning.headers().get("location").unwrap(),
            format!("/app/credit/return?checkout={}", created.checkout_id).as_str()
        );
        assert_eq!(fixture.balance().await, (0, 0));
        let paid = event(&created.checkout_id, "evt_stripetest_paid");
        assert_eq!(deliver(&app, &paid, SECRET).await, StatusCode::OK);
        assert_eq!(deliver(&app, &paid, SECRET).await, StatusCode::OK);
        assert_eq!(
            deliver(
                &app,
                &event(&created.checkout_id, "evt_stripetest_duplicate"),
                SECRET
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            CheckoutStatus::Paid
        );
        drop(app);
        hosted.shutdown().await;
        drop(checkout);
        let reopened = fixture.checkout(fake).await;
        assert_eq!(
            reopened.status(account(), id).await.unwrap().status,
            CheckoutStatus::Paid
        );
        assert_eq!(
            deliver(&router(reopened), &paid, SECRET).await,
            StatusCode::OK
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn bad_signatures_and_wrong_receipt_fields_never_credit() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = checkout
            .create(account(), 2000, OperationId::from_bytes(&[14; 16]).unwrap())
            .await
            .unwrap();
        let app = router(checkout.clone());
        let paid = event(&created.checkout_id, "evt_stripetest_refused");
        assert_eq!(
            deliver(&app, &paid, "whsec_wrong_fixture").await,
            StatusCode::BAD_REQUEST
        );
        for (field, value) in [
            ("amount_total", json!(1999)),
            ("currency", json!("eur")),
            ("livemode", json!(true)),
            ("id", json!("cs_test_other")),
            ("mode", json!("subscription")),
        ] {
            let mut wrong = paid.clone();
            wrong["data"]["object"][field] = value;
            assert_eq!(
                deliver(&app, &wrong, SECRET).await,
                StatusCode::BAD_REQUEST,
                "{field}"
            );
        }
        let mut unpaid = paid.clone();
        unpaid["data"]["object"]["payment_status"] = json!("unpaid");
        assert_eq!(deliver(&app, &unpaid, SECRET).await, StatusCode::OK);
        assert_eq!(
            deliver(
                &app,
                &json!({"id":"evt_unknown","type":"unhandled.fixture","data":{}}),
                SECRET
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(fixture.balance().await, (0, 0));
        let row = fixture
            .connection()
            .await
            .client
            .query_one("SELECT count(*) FROM hosted.stripe_event", &[])
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), 0);
        assert_eq!(deliver(&app, &paid, SECRET).await, StatusCode::OK);
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn finance_commit_before_receipt_acknowledgement_is_repaired_without_second_credit() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = checkout
            .create(account(), 2000, OperationId::from_bytes(&[15; 16]).unwrap())
            .await
            .unwrap();
        checkout
            .finance
            .stripe_purchase(account(), 2000, &created.checkout_id)
            .await
            .unwrap();
        let id = CheckoutId::from_hex(&created.checkout_id).unwrap();
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            CheckoutStatus::Pending
        );
        assert_eq!(
            deliver(
                &router(checkout.clone()),
                &event(&created.checkout_id, "evt_stripetest_recovery"),
                SECRET
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            CheckoutStatus::Paid
        );
    })
    .await;
}

#[test]
fn stripe_keys_require_a_pair_and_explicit_live_opt_in() {
    for profile in ["demo", "production"] {
        let values = BTreeMap::from([
            ("DF_PROFILE", profile),
            ("DF_STRIPE_SECRET_KEY", "sk_live_fixture"),
            ("DF_STRIPE_WEBHOOK_SECRET", SECRET),
        ]);
        assert!(matches!(
            crate::config::Config::parse(|key| values.get(key).map(|value| (*value).to_owned())),
            Err(crate::config::ConfigError::Invalid("DF_STRIPE_SECRET_KEY"))
        ));
    }
    assert!(matches!(
        Settings::parse(
            &|key| (key == "DF_STRIPE_SECRET_KEY").then(|| "sk_test_fixture".to_owned())
        ),
        Err(crate::config::ConfigError::Missing(_))
    ));
    let live = Settings::parse(&|key| match key {
        "DF_STRIPE_SECRET_KEY" => Some("sk_live_fixture".to_owned()),
        "DF_STRIPE_WEBHOOK_SECRET" => Some(SECRET.to_owned()),
        "DF_STRIPE_LIVE" => Some("true".to_owned()),
        _ => None,
    })
    .unwrap()
    .unwrap();
    assert!(!live.is_test_mode);
}

#[tokio::test]
async fn stripe_session_requests_never_follow_redirects_retry_or_accept_unsafe_responses() {
    for reply in [
        Reply::Redirect,
        Reply::Unavailable,
        Reply::ForeignUrl,
        Reply::Live,
        Reply::Large,
    ] {
        let fake = FakeStripe::with_reply(reply).await;
        let provider = provider::Provider::new(fake.settings(), ORIGIN).unwrap();
        let result = provider
            .create("00112233445566778899aabbccddeeff", 2000)
            .await;
        assert!(matches!(
            result,
            Err(provider::Error::Rejected | provider::Error::Response)
        ));
        assert_eq!(fake.requests.rows.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn concurrent_deliveries_and_an_event_rebound_to_another_operation_cannot_double_credit() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let first = checkout
            .create(account(), 2000, OperationId::from_bytes(&[16; 16]).unwrap())
            .await
            .unwrap();
        let second = checkout
            .create(account(), 2000, OperationId::from_bytes(&[17; 16]).unwrap())
            .await
            .unwrap();
        let other_instance = fixture.checkout(fake).await;
        let first_app = router(checkout);
        let second_app = router(other_instance);
        let original = event(&first.checkout_id, "evt_stripetest_concurrent");
        let duplicate = event(&first.checkout_id, "evt_stripetest_concurrent_duplicate");
        let results = tokio::join!(
            deliver(&first_app, &original, SECRET),
            deliver(&second_app, &duplicate, SECRET)
        );
        assert_eq!(results, (StatusCode::OK, StatusCode::OK));
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        assert_eq!(
            deliver(
                &second_app,
                &event(&second.checkout_id, "evt_stripetest_concurrent"),
                SECRET
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn uncertain_creation_older_than_the_processor_idempotency_window_is_never_redispatched() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = checkout.create(account(),2000,OperationId::from_bytes(&[18;16]).unwrap()).await.unwrap();
        let connection = fixture.connection().await;
        connection.client.execute("UPDATE hosted.stripe_checkout SET stripe_session_id=NULL,redirect_url=NULL,created_at=clock_timestamp()-interval '24 hours'",&[]).await.unwrap();
        assert!(matches!(checkout.create(account(),2000,OperationId::from_bytes(&[18;16]).unwrap()).await,Err(PortError::Forbidden)));
        assert_eq!(fake.requests.rows.lock().await.len(),1);
        assert_eq!(checkout.status(account(),CheckoutId::from_hex(&created.checkout_id).unwrap()).await.unwrap().status,CheckoutStatus::Pending);
    }).await;
}
