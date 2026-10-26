//! Loopback PayPal and owned scratch PostgreSQL databases exercise the payment boundary.

use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Path, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use df_persistence::{
    account_closure::TimeoutPolicy, account_store::PostgresAccountStore,
    postgres_tls::PostgresTransport,
};
use df_portal::app_api::ports::{CheckoutPort, PortError};
use df_types::{AccountId, CheckoutId, OperationId};
use futures::FutureExt;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

use super::{Checkout, paypal_provider::Provider, paypal_settings::Settings};
use crate::{
    config::{ConfigError, Profile},
    database::Connection,
    finance::Finance,
};

const ORIGIN: &str = "http://127.0.0.1:8099";
fn account() -> AccountId {
    AccountId::from_bytes(&[71; 16]).unwrap()
}
fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

#[derive(Default)]
struct Calls {
    rows: Mutex<Vec<(String, Value)>>,
    orders: Mutex<BTreeMap<String, Value>>,
    capture_change: Mutex<Option<(&'static str, Value)>>,
    reply: Mutex<Option<(StatusCode, String)>>,
    hold: Mutex<Option<String>>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl Calls {
    async fn record(&self, path: &str, body: Value) {
        self.rows.lock().unwrap().push((path.to_owned(), body));
        let held = self.hold.lock().unwrap().as_deref() == Some(path);
        if held {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
    fn count(&self, path: &str) -> usize {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == path)
            .count()
    }
}

struct Fake {
    endpoint: String,
    calls: Arc<Calls>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fake {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let calls = Arc::new(Calls::default());
        let app = Router::new()
            .route("/v1/oauth2/token", post(token))
            .route("/v2/checkout/orders", post(create))
            .route("/v2/checkout/orders/{id}/capture", post(capture))
            .route("/v2/checkout/orders/{id}", get(inspect))
            .route("/v1/notifications/verify-webhook-signature", post(verify))
            .with_state(Arc::clone(&calls));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            calls,
            task,
        }
    }
    fn settings(&self) -> Settings {
        let mut settings = settings(&[]).unwrap().unwrap();
        settings.endpoint = Some(self.endpoint.clone());
        settings
    }
    fn event(&self, operation: &str, id: &str) -> Value {
        let order = self
            .calls
            .orders
            .lock()
            .unwrap()
            .get(&format!("O{operation}"))
            .unwrap()
            .clone();
        let mut resource = order["purchase_units"][0]["payments"]["captures"][0].clone();
        resource["supplementary_data"] =
            json!({"related_ids":{"order_id":format!("O{operation}")}});
        resource.as_object_mut().unwrap().remove("custom_id");
        json!({"id":id,"event_type":"PAYMENT.CAPTURE.COMPLETED","resource":resource})
    }
}

async fn token(
    State(calls): State<Arc<Calls>>,
    headers: HeaderMap,
    Form(body): Form<BTreeMap<String, String>>,
) -> Response {
    assert!(
        headers["authorization"]
            .to_str()
            .unwrap()
            .starts_with("Basic ")
    );
    assert_eq!(body["grant_type"], "client_credentials");
    calls.record("token", json!(body)).await;
    if let Some((status, body)) = calls.reply.lock().unwrap().clone() {
        return (status, [("location", "/v2/checkout/orders")], body).into_response();
    }
    Json(json!({"access_token":"fake-token","token_type":"Bearer","expires_in":3600}))
        .into_response()
}
async fn create(
    State(calls): State<Arc<Calls>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer fake-token");
    let operation = body["purchase_units"][0]["custom_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(headers["paypal-request-id"], operation);
    assert_eq!(body["intent"], "CAPTURE");
    assert_eq!(body["purchase_units"][0]["amount"]["currency_code"], "USD");
    assert_eq!(
        body["application_context"]["return_url"],
        format!("{ORIGIN}/app/api/payments/paypal/return?checkout={operation}")
    );
    assert_eq!(
        body["application_context"]["cancel_url"],
        format!("{ORIGIN}/app/api/payments/paypal/cancel?checkout={operation}")
    );
    assert_eq!(body["purchase_units"][0].as_object().unwrap().len(), 2);
    calls.record("create", body.clone()).await;
    if let Some((status, body)) = calls.reply.lock().unwrap().clone() {
        return (status, [("location", "/v2/checkout/orders")], body).into_response();
    }
    let order_id = format!("O{operation}");
    let amount = body["purchase_units"][0]["amount"].clone();
    calls.orders.lock().unwrap().entry(order_id.clone()).or_insert(json!({
        "id":order_id,"status":"COMPLETED","purchase_units":[{
            "custom_id":operation,"amount":amount,
            "payments":{"captures":[{"id":format!("C{operation}"),"status":"COMPLETED","amount":amount,"custom_id":operation}]}
        }]
    }));
    Json(json!({"id":order_id,"status":"CREATED","links":[{"rel":"payer-action","href":format!("https://www.sandbox.paypal.com/checkoutnow?token={order_id}")}]})).into_response()
}
async fn capture(
    State(calls): State<Arc<Calls>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    assert_eq!(headers["prefer"], "return=representation");
    let order = calls.orders.lock().unwrap().get(&id).unwrap().clone();
    assert_eq!(
        headers["paypal-request-id"],
        format!(
            "c-{}",
            order["purchase_units"][0]["custom_id"].as_str().unwrap()
        )
    );
    calls.record("capture", json!({"order":id})).await;
    order_response(&calls, &id)
}
async fn inspect(State(calls): State<Arc<Calls>>, Path(id): Path<String>) -> Response {
    calls.record("inspect", json!({"order":id})).await;
    order_response(&calls, &id)
}
fn order_response(calls: &Calls, id: &str) -> Response {
    let mut order = calls.orders.lock().unwrap().get(id).unwrap().clone();
    if let Some((pointer, value)) = calls.capture_change.lock().unwrap().clone() {
        *order.pointer_mut(pointer).unwrap() = value;
    }
    Json(order).into_response()
}
async fn verify(State(calls): State<Arc<Calls>>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["webhook_id"], "fake-webhook");
    assert_eq!(body["auth_algo"], "SHA256withRSA");
    assert!(body.get("webhook_event").is_some());
    calls.record("verify", body.clone()).await;
    Json(
        json!({"verification_status":if body["transmission_sig"] == "verified-fixture" {"SUCCESS"} else {"FAILURE"}}),
    )
}

fn settings(overrides: &[(&str, &str)]) -> Result<Option<Settings>, ConfigError> {
    let mut values = BTreeMap::from([
        ("DF_PAYPAL_ENV", "sandbox"),
        ("DF_PAYPAL_CLIENT_ID", "fake-client"),
        ("DF_PAYPAL_CLIENT_SECRET", "fake-secret"),
        ("DF_PAYPAL_WEBHOOK_ID", "fake-webhook"),
    ]);
    values.extend(overrides.iter().copied());
    Settings::parse(
        &|key| values.get(key).map(|value| (*value).to_owned()),
        Profile::Demo,
    )
}

struct Fixture {
    admin: Connection,
    name: String,
    config: tokio_postgres::Config,
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
        let name = format!(
            "paypaltopup_{}",
            &df_customer::SecretToken::generate().unwrap().text()[..16]
        );
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        let config = format!("host=127.0.0.1 port=55911 user=postgres dbname={name}")
            .parse()
            .unwrap();
        Self {
            admin,
            name,
            config,
        }
    }
    async fn connection(&self) -> Connection {
        Connection::open(&self.config, &PostgresTransport::PlaintextLoopbackOnly)
            .await
            .unwrap()
    }
    async fn checkout(&self, fake: &Fake) -> Checkout {
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
        connection.client.execute("INSERT INTO accounts.account(account_id,email_normalized,email_display,email_verified_at_ms,status,credential_generation,created_at_ms) VALUES($1,'paypal@example.test','paypal@example.test',1,'active',1,1) ON CONFLICT DO NOTHING",&[&&account().as_bytes()[..]]).await.unwrap();
        let finance = Finance::new(connection, accounts, [33; 32]).unwrap();
        Checkout::configured(
            Profile::Demo,
            self.connection().await,
            finance,
            None,
            ORIGIN,
        )
        .await
        .unwrap()
        .with_paypal(self.connection().await, Some(fake.settings()), ORIGIN)
        .unwrap()
    }
    async fn balance(&self) -> (i64, i64) {
        let row = self.connection().await.client.query_one("SELECT count(*),coalesce(sum(original_micro_dollars),0)::bigint FROM finance.funding_lot WHERE processor='paypal'",&[]).await.unwrap();
        (row.get(0), row.get(1))
    }
    async fn finish(self) {
        assert!(
            self.name.starts_with("paypaltopup_")
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
    }
}
async fn with_fixture(test: impl for<'a> AsyncFnOnce(&'a Fixture, &'a Fake)) {
    let fixture = Fixture::new().await;
    let fake = Fake::new().await;
    let result = std::panic::AssertUnwindSafe(test(&fixture, &fake))
        .catch_unwind()
        .await;
    fake.task.abort();
    fixture.finish().await;
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
async fn start(checkout: &Checkout, n: u8) -> super::CheckoutCreated {
    checkout
        .create_with_provider(
            account(),
            2000,
            operation(n),
            df_account_app::wire::CheckoutProvider::Paypal,
        )
        .await
        .unwrap()
}
async fn return_to(app: &Router, id: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::get(format!(
                "/app/api/payments/paypal/return?checkout={id}&token=O{id}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap()
        .status()
}
async fn deliver(app: &Router, event: Value, signature: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::post("/app/api/payments/paypal/webhook")
                .header("content-type", "application/json")
                .header("paypal-auth-algo", "SHA256withRSA")
                .header(
                    "paypal-cert-url",
                    "https://api.sandbox.paypal.com/cert/fake",
                )
                .header("paypal-transmission-id", "fixture-transmission")
                .header("paypal-transmission-time", "2026-10-25T00:00:00Z")
                .header("paypal-transmission-sig", signature)
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[test]
fn paypal_config_requires_paired_credentials_and_guards_live_outside_development() {
    assert!(Settings::parse(&|_| None, Profile::Demo).unwrap().is_none());
    assert!(settings(&[]).unwrap().unwrap().is_test_mode);
    for (key, value) in [
        ("DF_PAYPAL_ENV", "invalid"),
        ("DF_PAYPAL_ENV", ""),
        ("DF_PAYPAL_CLIENT_ID", ""),
        ("DF_PAYPAL_CLIENT_SECRET", ""),
        ("DF_PAYPAL_LIVE", "yes"),
        ("DF_PAYPAL_ENV", "live"),
    ] {
        assert!(settings(&[(key, value)]).is_err(), "{key}");
    }
    assert!(
        !settings(&[("DF_PAYPAL_ENV", "live"), ("DF_PAYPAL_LIVE", "true")])
            .unwrap()
            .unwrap()
            .is_test_mode
    );
    assert!(
        settings(&[("DF_PAYPAL_WEBHOOK_ID", "")])
            .unwrap()
            .unwrap()
            .webhook_id
            .is_none()
    );
    assert!(
        !Settings::parse(
            &|key| match key {
                "DF_PAYPAL_ENV" => Some("live".to_owned()),
                "DF_PAYPAL_CLIENT_ID" | "DF_PAYPAL_CLIENT_SECRET" => Some("fixture".to_owned()),
                _ => None,
            },
            Profile::Development
        )
        .unwrap()
        .unwrap()
        .is_test_mode
    );
}

#[tokio::test]
async fn paypal_tokens_are_cached_and_requests_never_retry_follow_redirects_or_accept_large_responses()
 {
    let fake = Fake::new().await;
    let provider = Provider::new(fake.settings(), ORIGIN).unwrap();
    provider.create(&"01".repeat(16), 2000).await.unwrap();
    provider.create(&"02".repeat(16), 2000).await.unwrap();
    assert_eq!(fake.calls.count("token"), 1);
    provider.expire_token().await;
    provider.create(&"04".repeat(16), 2000).await.unwrap();
    assert_eq!(fake.calls.count("token"), 2);
    for (status, body) in [
        (StatusCode::TEMPORARY_REDIRECT, "".to_owned()),
        (StatusCode::SERVICE_UNAVAILABLE, "".to_owned()),
        (StatusCode::OK, "x".repeat(65_537)),
    ] {
        *fake.calls.reply.lock().unwrap() = Some((status, body));
        assert!(provider.create(&"03".repeat(16), 2000).await.is_err());
    }
    assert_eq!(fake.calls.count("create"), 6);
}

#[tokio::test]
async fn paypal_return_and_webhook_replays_credit_exactly_once_across_restart() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 1).await;
        assert_eq!(start(&checkout, 1).await, created);
        assert_eq!(fixture.balance().await, (0, 0));
        let id = CheckoutId::from_hex(&created.checkout_id).unwrap();
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            super::CheckoutStatus::Pending
        );
        let app = super::router(checkout.clone());
        assert_eq!(
            return_to(&app, &created.checkout_id).await,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            return_to(&app, &created.checkout_id).await,
            StatusCode::SEE_OTHER
        );
        assert_eq!(fake.calls.count("capture"), 1);
        let event = fake.event(&created.checkout_id, "WH-return");
        let (first, second) = tokio::join!(
            deliver(&app, event.clone(), "verified-fixture"),
            deliver(&app, event, "verified-fixture")
        );
        assert_eq!((first, second), (StatusCode::OK, StatusCode::OK));
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        assert_eq!(
            fixture
                .checkout(fake)
                .await
                .status(account(), id)
                .await
                .unwrap()
                .status,
            super::CheckoutStatus::Paid
        );
        assert_eq!(
            checkout
                .status(AccountId::from_bytes(&[99; 16]).unwrap(), id)
                .await,
            Err(PortError::NotFound)
        );
    })
    .await;
}

#[tokio::test]
async fn paypal_webhook_only_capture_is_verified_and_credits_once_without_custom_id_in_event() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 2).await;
        let app = super::router(checkout.clone());
        let event = fake.event(&created.checkout_id, "WH-only");
        assert_eq!(
            deliver(&app, event.clone(), "invalid").await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(fixture.balance().await, (0, 0));
        assert_eq!(
            deliver(&app, event.clone(), "verified-fixture").await,
            StatusCode::OK
        );
        assert_eq!(
            deliver(&app, event, "verified-fixture").await,
            StatusCode::OK
        );
        assert_eq!(
            return_to(&app, &created.checkout_id).await,
            StatusCode::SEE_OTHER
        );
        assert_eq!(fake.calls.count("capture"), 0);
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn paypal_receipt_field_mismatches_refuse_return_and_verified_webhook() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 3).await;
        let app = super::router(checkout.clone());
        for (pointer, value) in [
            (
                "/purchase_units/0/payments/captures/0/amount/value",
                json!("19.99"),
            ),
            (
                "/purchase_units/0/payments/captures/0/amount/currency_code",
                json!("EUR"),
            ),
            ("/purchase_units/0/custom_id", json!("00".repeat(16))),
            (
                "/purchase_units/0/payments/captures/0/status",
                json!("PENDING"),
            ),
            ("/status", json!("APPROVED")),
            ("/id", json!("FOREIGN")),
        ] {
            *fake.calls.capture_change.lock().unwrap() = Some((pointer, value));
            assert_eq!(
                return_to(&app, &created.checkout_id).await,
                StatusCode::BAD_REQUEST,
                "{pointer}"
            );
            assert_eq!(
                deliver(
                    &app,
                    fake.event(&created.checkout_id, "WH-mismatch"),
                    "verified-fixture"
                )
                .await,
                StatusCode::BAD_REQUEST,
                "{pointer}"
            );
            assert_eq!(fixture.balance().await, (0, 0));
        }
        *fake.calls.capture_change.lock().unwrap() = None;
        let mut event = fake.event(&created.checkout_id, "WH-mismatch");
        event["resource"]["custom_id"] = json!("00".repeat(16));
        assert_eq!(
            deliver(&app, event, "verified-fixture").await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(fixture.balance().await, (0, 0));
    })
    .await;
}

#[tokio::test]
async fn paypal_cancellation_denied_and_reversed_are_markers_and_never_post_refunds() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 4).await;
        let app = super::router(checkout.clone());
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/app/api/payments/paypal/cancel?checkout={}",
                    created.checkout_id
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            checkout
                .status(
                    account(),
                    CheckoutId::from_hex(&created.checkout_id).unwrap()
                )
                .await
                .unwrap()
                .status,
            super::CheckoutStatus::Cancelled
        );
        assert_eq!(
            deliver(
                &app,
                fake.event(&created.checkout_id, "WH-cancel-paid"),
                "verified-fixture"
            )
            .await,
            StatusCode::OK
        );
        for kind in ["DENIED", "REVERSED"] {
            let mut event = fake.event(&created.checkout_id, &format!("WH-{kind}"));
            event["event_type"] = json!(format!("PAYMENT.CAPTURE.{kind}"));
            event["resource"]["status"] = json!(kind);
            assert_eq!(
                deliver(&app, event, "verified-fixture").await,
                StatusCode::OK
            );
        }
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn paypal_http_calls_release_the_database_lock_and_account_capacity_is_shared() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let paypal = checkout.paypal.as_ref().unwrap();
        for path in ["token", "create"] {
            *fake.calls.hold.lock().unwrap() = Some(path.to_owned());
            let request = paypal.create(
                account(),
                2000,
                operation(if path == "token" { 10 } else { 11 }),
            );
            let observe = async {
                tokio::time::timeout(Duration::from_secs(3), fake.calls.entered.notified())
                    .await
                    .unwrap();
                let connection =
                    tokio::time::timeout(Duration::from_secs(1), paypal.connection.lock())
                        .await
                        .unwrap();
                connection.client.simple_query("SELECT 1").await.unwrap();
                drop(connection);
                fake.calls.release.notify_one();
            };
            let (result, ()) = tokio::join!(request, observe);
            result.unwrap();
        }
        *fake.calls.hold.lock().unwrap() = None;
        let created = start(&checkout, 12).await;
        let app = super::router(checkout.clone());
        for path in ["capture", "verify", "inspect"] {
            *fake.calls.hold.lock().unwrap() = Some(path.to_owned());
            let request = async {
                if path == "capture" {
                    return_to(&app, &created.checkout_id).await
                } else {
                    deliver(
                        &app,
                        fake.event(&created.checkout_id, &format!("WH-lock-{path}")),
                        "verified-fixture",
                    )
                    .await
                }
            };
            let observe = async {
                tokio::time::timeout(Duration::from_secs(3), fake.calls.entered.notified())
                    .await
                    .unwrap();
                let connection =
                    tokio::time::timeout(Duration::from_secs(1), paypal.connection.lock())
                        .await
                        .unwrap();
                connection.client.simple_query("SELECT 1").await.unwrap();
                drop(connection);
                fake.calls.release.notify_one();
            };
            let (status, ()) = tokio::join!(request, observe);
            assert!(status.is_success() || status.is_redirection());
        }
        *fake.calls.hold.lock().unwrap() = None;
        let first =
            super::stripe::claim_creation(&paypal.creations, *account().as_bytes(), [91; 16])
                .unwrap();
        let second =
            super::stripe::claim_creation(&paypal.creations, *account().as_bytes(), [92; 16])
                .unwrap();
        assert!(matches!(
            paypal.create(account(), 2000, operation(13)).await,
            Err(PortError::Throttled)
        ));
        drop((first, second));
        assert!(paypal.create(account(), 2000, operation(13)).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn paypal_finance_commit_before_receipt_acknowledgement_is_repaired_once() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 15).await;
        checkout
            .finance
            .paypal_purchase(account(), 2000, &created.checkout_id)
            .await
            .unwrap();
        let app = super::router(checkout.clone());
        assert_eq!(
            return_to(&app, &created.checkout_id).await,
            StatusCode::SEE_OTHER
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        let id = CheckoutId::from_hex(&created.checkout_id).unwrap();
        assert_eq!(
            checkout.status(account(), id).await.unwrap().status,
            super::CheckoutStatus::Paid
        );
    })
    .await;
}

#[tokio::test]
async fn paypal_old_uncertain_creates_are_not_redispatched_and_capture_ids_cannot_rebind() {
    with_fixture(async |fixture,fake| {
        let checkout=fixture.checkout(fake).await;
        let first=start(&checkout,16).await;
        let second=start(&checkout,17).await;
        let app=super::router(checkout.clone());
        assert_eq!(return_to(&app,&first.checkout_id).await,StatusCode::SEE_OTHER);
        *fake.calls.capture_change.lock().unwrap()=Some(("/purchase_units/0/payments/captures/0/id",json!(format!("C{}",first.checkout_id))));
        assert_eq!(return_to(&app,&second.checkout_id).await,StatusCode::BAD_REQUEST);
        assert_eq!(fixture.balance().await,(1,20_000_000));
        *fake.calls.capture_change.lock().unwrap()=None;
        fixture.connection().await.client.execute("UPDATE hosted.paypal_checkout SET paypal_order_id=NULL,redirect_url=NULL,created_at=clock_timestamp()-interval '7 hours' WHERE operation_id=$1",&[&&CheckoutId::from_hex(&second.checkout_id).unwrap().as_bytes()[..]]).await.unwrap();
        assert!(matches!(checkout.paypal.as_ref().unwrap().create(account(),2000,operation(17)).await,Err(PortError::Forbidden)));
        assert_eq!(fake.calls.count("create"),2);
    }).await;
}

#[tokio::test]
async fn paypal_webhook_ids_cannot_move_to_another_receipt_and_missing_verifier_refuses() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let first = start(&checkout, 20).await;
        let second = start(&checkout, 21).await;
        let app = super::router(checkout.clone());
        assert_eq!(
            deliver(
                &app,
                fake.event(&first.checkout_id, "WH-bound"),
                "verified-fixture"
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            deliver(
                &app,
                fake.event(&second.checkout_id, "WH-bound"),
                "verified-fixture"
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        let mut without_verifier = fixture.checkout(fake).await;
        Arc::get_mut(without_verifier.paypal.as_mut().unwrap())
            .unwrap()
            .provider
            .settings
            .webhook_id = None;
        let missing_app = super::router(without_verifier);
        assert_eq!(
            deliver(
                &missing_app,
                fake.event(&second.checkout_id, "WH-missing"),
                "verified-fixture"
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
    })
    .await;
}

#[tokio::test]
async fn paypal_return_is_mounted_in_the_real_hosted_composition_with_fake_configuration() {
    with_fixture(async |fixture, fake| {
        let checkout = fixture.checkout(fake).await;
        let created = start(&checkout, 22).await;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/agents/PAYPALTOPUP")
            .join(&fixture.name);
        std::fs::create_dir_all(root.join("bundle")).unwrap();
        std::fs::write(
            root.join("bundle/index.html"),
            "<!doctype html><title>PayPal fixture</title>",
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
                    fixture.name
                ),
            ),
            (
                "DF_PORTAL_BUNDLE_DIR",
                root.join("bundle").to_string_lossy().into_owned(),
            ),
            (
                "DF_PORTAL_OUTBOX",
                root.join("outbox").to_string_lossy().into_owned(),
            ),
            ("DF_PAYPAL_ENV", "sandbox".to_owned()),
            ("DF_PAYPAL_CLIENT_ID", "fake-client".to_owned()),
            ("DF_PAYPAL_CLIENT_SECRET", "fake-secret".to_owned()),
            ("DF_PAYPAL_WEBHOOK_ID", "fake-webhook".to_owned()),
        ]);
        let mut config = crate::config::Config::parse(|key| values.get(key).cloned()).unwrap();
        assert!(config.paypal.as_ref().unwrap().is_test_mode);
        config.paypal.as_mut().unwrap().endpoint = Some(fake.endpoint.clone());
        let hosted = crate::runtime::Hosted::open(config).await.unwrap();
        assert_eq!(
            return_to(&hosted.router(), &created.checkout_id).await,
            StatusCode::SEE_OTHER
        );
        assert_eq!(fixture.balance().await, (1, 20_000_000));
        hosted.shutdown().await;
    })
    .await;
}

#[tokio::test]
async fn paypal_and_stripe_share_account_capacity_and_cannot_change_an_existing_request() {
    with_fixture(async |fixture, fake| {
        use df_account_app::wire::CheckoutProvider;
        let initial = fixture.checkout(fake).await;
        let mut stripe_settings = super::Settings::parse(&|key| match key {
            "DF_STRIPE_SECRET_KEY" => Some("sk_test_paypal_fixture".to_owned()),
            "DF_STRIPE_WEBHOOK_SECRET" => Some("whsec_paypal_fixture".to_owned()),
            _ => None,
        })
        .unwrap()
        .unwrap();
        stripe_settings.endpoint = Some(format!("{}stripe-should-not-run", fake.endpoint));
        let checkout = Checkout::configured(
            Profile::Demo,
            fixture.connection().await,
            initial.finance.clone(),
            Some(stripe_settings),
            ORIGIN,
        )
        .await
        .unwrap()
        .with_paypal(fixture.connection().await, Some(fake.settings()), ORIGIN)
        .unwrap();
        assert_eq!(
            checkout.offers().payment_methods,
            vec![CheckoutProvider::Stripe, CheckoutProvider::Paypal]
        );
        let paypal = checkout.paypal.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &paypal.creations,
            &checkout.stripe.as_ref().unwrap().creations
        ));
        let first =
            super::stripe::claim_creation(&paypal.creations, *account().as_bytes(), [81; 16])
                .unwrap();
        let second =
            super::stripe::claim_creation(&paypal.creations, *account().as_bytes(), [82; 16])
                .unwrap();
        assert_eq!(
            checkout
                .create_with_provider(account(), 2000, operation(83), CheckoutProvider::Stripe)
                .await,
            Err(PortError::Throttled)
        );
        drop((first, second));
        start(&checkout, 84).await;
        assert_eq!(
            checkout
                .create_with_provider(account(), 2000, operation(84), CheckoutProvider::Stripe)
                .await,
            Err(PortError::Forbidden)
        );
        assert_eq!(
            checkout
                .create_with_provider(account(), 1000, operation(84), CheckoutProvider::Paypal)
                .await,
            Err(PortError::Forbidden)
        );
        assert_eq!(fake.calls.count("create"), 1);
    })
    .await;
}
