//! Real PostgreSQL and HTTP boundaries, synthetic accounts only; no provider adapters.
use super::*;
use crate::database::Connection;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use df_customer::{
    AccountStore, Authenticated, BuiltInCommonPasswords, Clock, CsrfSecret, CustomerConfig,
    CustomerService, DeviceLabel, EmailAddress, EmailError, EmailSender, HashingParameters,
    HostConfig, IdentityProviderKind, IdentitySource, NewIdentityAccount, NewSession, SecretToken,
    ServiceTasks, Tunables,
    integration::{IssueToken, Peppers, Scope},
};
use df_persistence::{
    account_closure::TimeoutPolicy, account_store::PostgresAccountStore,
    postgres_tls::PostgresTransport,
};
use df_portal::{PortalOrigin, share::Sharing};
use df_types::{AccountId, WebSessionId};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

const ORIGIN: &str = "http://127.0.0.1:8099";
const NOW: u64 = 1_800_000_000_000;
#[derive(Clone)]
struct NoMail;
impl EmailSender for NoMail {
    async fn send(&self, _message: df_customer::Message) -> Result<(), EmailError> {
        panic!("integration tests never send email");
    }
}
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
struct Fixture {
    admin: Connection,
    database: String,
    data: Connection,
    service: CustomerService<PostgresAccountStore, NoMail>,
    tasks: ServiceTasks,
    accounts: PostgresAccountStore,
    peppers: Arc<Peppers>,
    reads: Arc<Reads>,
    shares: Arc<Shares>,
    caller: Authenticated,
    cookie: String,
}
impl Fixture {
    async fn open() -> Self {
        let mut db: tokio_postgres::Config =
            "host=127.0.0.1 port=55911 user=postgres dbname=postgres"
                .parse()
                .unwrap();
        let transport = PostgresTransport::PlaintextLoopbackOnly;
        let admin = Connection::open(&db, &transport).await.unwrap();
        let database = format!(
            "apibuild2_http_{}",
            &SecretToken::generate().unwrap().text()[..16]
        );
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        db.dbname(&database);
        let accounts =
            PostgresAccountStore::open(db.clone(), transport.clone(), 2, TimeoutPolicy::NotChecked)
                .await
                .unwrap();
        let mut data = Connection::open(&db, &transport).await.unwrap();
        crate::database::migrate(&mut data).await.unwrap();
        // This owner migration is also registered in production composition.
        if !data
            .client
            .query_one(
                "SELECT to_regclass('campaigns.share_preview') IS NOT NULL",
                &[],
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
        {
            data.client
                .batch_execute(include_str!(
                    "../../../df-persistence/migrations/0033_session_shares.sql"
                ))
                .await
                .unwrap();
        }
        let (service, tasks) = CustomerService::start(
            CustomerConfig {
                hosts: HostConfig::local(ORIGIN).unwrap(),
                identity: IdentitySource::local_passwords(),
                tunables: Tunables::default(),
                hashing: HashingParameters {
                    memory_kib: 8,
                    passes: 1,
                    lanes: 1,
                },
                maximum_concurrent_hashes: 1,
                hash_permit_wait: Duration::from_secs(1),
                provider_signup_floor: Duration::ZERO,
                dev_allow_local_passwords_on_https: false,
            },
            accounts.clone(),
            NoMail,
            Arc::new(TestClock(AtomicU64::new(NOW))),
            Arc::new(BuiltInCommonPasswords),
        )
        .await
        .unwrap();
        let cookie = seed_account(&accounts, 1).await;
        let caller = service.authenticate(&cookie).await.unwrap().unwrap();
        let reads = Arc::new(Reads::new(Connection::open(&db, &transport).await.unwrap()));
        let shares = Arc::new(Shares::new(
            Connection::open(&db, &transport).await.unwrap(),
        ));
        Self {
            admin,
            database,
            data,
            service,
            tasks,
            accounts,
            peppers: Arc::new(Peppers::new(1, [71; 32], None).unwrap()),
            reads,
            shares,
            caller,
            cookie,
        }
    }
    fn router(&self) -> Router {
        let origin = PortalOrigin::parse(ORIGIN).unwrap();
        df_portal::integration_api::router(
            self.service.clone(),
            Arc::clone(&self.peppers),
            Arc::clone(&self.reads),
            df_portal::integration_api::RatePolicy::default(),
        )
        .merge(df_portal::app_api::integration::router(
            self.service.clone(),
            origin.clone(),
            Arc::clone(&self.peppers),
        ))
        .merge(df_portal::share::router(
            self.service.clone(),
            origin,
            Arc::clone(&self.shares),
            df_portal::share::PublicSharePolicy::default(),
        ))
    }
    async fn token(&self, scopes: Vec<Scope>) -> String {
        match self
            .service
            .issue_personal_token(
                &self.caller,
                df_customer::Reauthentication::RecentSignIn,
                &self.peppers,
                "HTTP fixture",
                scopes,
                7,
            )
            .await
            .unwrap()
        {
            IssueToken::Issued(t) => t.secret.reveal(),
            IssueToken::Refused(r) => panic!("token refused {r:?}"),
        }
    }
    async fn finish(self) {
        let Self {
            admin,
            database,
            data,
            service,
            tasks,
            accounts,
            peppers: _,
            reads,
            shares,
            caller: _,
            cookie: _,
        } = self;
        tasks.shutdown().await;
        drop(service);
        drop(accounts);
        drop(reads);
        drop(shares);
        drop(data);
        wait_for_connections(&admin,&database).await;
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database}"))
            .await
            .unwrap();
    }
}
async fn seed_account(store: &PostgresAccountStore, key: u8) -> String {
    let account = AccountId::from_bytes(&[key; 16]).unwrap();
    store
        .create_identity_account(NewIdentityAccount {
            id: account,
            email: EmailAddress::parse(&format!("person{key}@example.test")).unwrap(),
            is_verified: true,
            provider: IdentityProviderKind::Fake,
            subject: format!("synthetic-{key}"),
            created_at_ms: NOW,
            adult_attested_at: Some(NOW),
            terms_version: Some("terms-2026-10".into()),
            terms_accepted_at: Some(NOW),
            verification: None,
        })
        .await
        .unwrap();
    let token = SecretToken::generate().unwrap();
    store
        .create_session(
            NewSession {
                id: WebSessionId::from_bytes(&[key; 16]).unwrap(),
                digest: token.digest(),
                account,
                credential_generation: 1,
                created_at_ms: NOW,
                absolute_expires_at_ms: NOW + 86_400_000,
                csrf_secret: CsrfSecret::from_bytes([key; 32]),
                device: DeviceLabel::Desktop,
            },
            None,
        )
        .await
        .unwrap();
    token.text()
}
async fn request(
    app: Router,
    path: &str,
    bearer: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder().uri(path);
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, headers, json)
}

#[tokio::test]
async fn integration_every_endpoint_enforces_its_exact_scope_without_cookie_fallback() {
    let f = Fixture::open().await;
    let routes = [
        ("/api/v1/account", Scope::Account),
        ("/api/v1/auth-sessions", Scope::AuthSessions),
        ("/api/v1/campaigns", Scope::PlaySessions),
        ("/api/v1/play-sessions", Scope::PlaySessions),
        ("/api/v1/funding", Scope::Funding),
    ];
    for scope in [
        Scope::Account,
        Scope::AuthSessions,
        Scope::PlaySessions,
        Scope::Funding,
        Scope::Stats,
    ] {
        let token = f.token(vec![scope]).await;
        for (path, required) in routes {
            let (status, headers, body) = request(f.router(), path, Some(&token)).await;
            assert_eq!(
                status,
                if scope == required {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                },
                "{path} under {scope:?}: {body}"
            );
            assert_eq!(
                headers[header::CACHE_CONTROL],
                if scope == required && matches!(required, Scope::Account | Scope::PlaySessions) {
                    "private, no-cache"
                } else {
                    "no-store"
                }
            );
            assert!(!headers.contains_key("access-control-allow-origin"));
        }
    }
    for (path, _) in routes {
        let (status, headers, _) = request(f.router(), path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    }
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/account")
                .header(header::COOKIE, format!("__Host-df_session={}", f.cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    f.finish().await;
}

#[tokio::test]
async fn integration_email_is_opt_in_auth_metadata_is_secret_free_and_funding_get_does_not_grant_credit()
 {
    let f = Fixture::open().await;
    let token = f
        .token(vec![Scope::Account, Scope::AuthSessions, Scope::Funding])
        .await;
    let (_, _, account) = request(f.router(), "/api/v1/account", Some(&token)).await;
    assert!(account["data"].get("email").is_none());
    assert!(
        account["data"]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    let (_, _, sessions) = request(f.router(), "/api/v1/auth-sessions", Some(&token)).await;
    let text = sessions.to_string();
    for forbidden in [
        "csrf",
        "token_digest",
        "fingerprint",
        "ip_address",
        &f.cookie,
    ] {
        assert!(!text.contains(forbidden));
    }
    let before: i64 = f
        .data
        .client
        .query_one("SELECT count(*) FROM finance.trial_grant", &[])
        .await
        .unwrap()
        .get(0);
    let (_, _, funding) = request(f.router(), "/api/v1/funding", Some(&token)).await;
    for forbidden in ["provider", "multiplier", "table_fee", "supplier"] {
        assert!(!funding.to_string().contains(forbidden));
    }
    let after: i64 = f
        .data
        .client
        .query_one("SELECT count(*) FROM finance.trial_grant", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after);
    let token = f.token(vec![Scope::Account, Scope::Email]).await;
    let (_, _, account) = request(f.router(), "/api/v1/account", Some(&token)).await;
    assert_eq!(account["data"]["email"], "person1@example.test");
    f.finish().await;
}

#[path = "share_tests.rs"]
mod share_tests;

#[tokio::test]
async fn integration_conditional_reads_authorize_before_returning_not_modified(){
    let f=Fixture::open().await;let token=f.token(vec![Scope::Account]).await;let(_,headers,_)=request(f.router(),"/api/v1/account",Some(&token)).await;
    let etag=headers[header::ETAG].clone();
    for(token,expected)in[(Some(token),StatusCode::NOT_MODIFIED),(None,StatusCode::UNAUTHORIZED)]{
        let mut request=Request::builder().uri("/api/v1/account").header(header::IF_NONE_MATCH,etag.clone());if let Some(token)=token{request=request.header(header::AUTHORIZATION,format!("Bearer {token}"));}
        assert_eq!(f.router().oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status(),expected);
    }
    let token=f.token(vec![Scope::Funding]).await;
    assert_eq!(f.router().oneshot(Request::builder().uri("/api/v1/account").header(header::AUTHORIZATION,format!("Bearer {token}")).header(header::IF_NONE_MATCH,etag).body(Body::empty()).unwrap()).await.unwrap().status(),StatusCode::FORBIDDEN);
    f.finish().await;
}

async fn wait_for_connections(admin:&Connection,database:&str){
    tokio::time::timeout(Duration::from_secs(5),async{
        loop {
            let count:i64=admin.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname=$1",&[&database]).await.unwrap().get(0);
            if count==0 {break;}tokio::task::yield_now().await;
        }
    }).await.expect("owned PostgreSQL connections close before scratch database removal");
}

async fn token_mutation(f:&Fixture,path:&str,body:Value,csrf:bool)->(StatusCode,Value){
    let mut builder=Request::builder().method("POST").uri(path).header(header::COOKIE,format!("{}={}",df_portal::SESSION_COOKIE,f.cookie)).header(header::ORIGIN,ORIGIN).header(header::CONTENT_TYPE,"application/json");
    if csrf{builder=builder.header(df_account_app::wire::CSRF_HEADER,f.caller.csrf.form_token(df_account_app::wire::CSRF_FORM));}
    let response=f.router().oneshot(builder.body(Body::from(body.to_string())).unwrap()).await.unwrap();let status=response.status();let body=response.into_body().collect().await.unwrap().to_bytes();(status,serde_json::from_slice(&body).unwrap())
}
#[tokio::test]
async fn integration_cookie_lifecycle_requires_csrf_reveals_once_and_revokes_the_bearer(){
    let f=Fixture::open().await;let body=json!({"label":"Trusted script","scopes":["account:read"],"expiry_days":7,"password":null});
    assert_eq!(token_mutation(&f,"/app/api/developer-tokens",body.clone(),false).await.0,StatusCode::FORBIDDEN);
    let(status,issued)=token_mutation(&f,"/app/api/developer-tokens",body,true).await;assert_eq!(status,StatusCode::OK);let secret=issued["secret"].as_str().unwrap();let id=issued["token"]["token_id"].as_str().unwrap();
    assert_eq!(request(f.router(),"/api/v1/account",Some(secret)).await.0,StatusCode::OK);
    let response=f.router().oneshot(Request::builder().uri("/app/api/developer-tokens").header(header::COOKIE,format!("{}={}",df_portal::SESSION_COOKIE,f.cookie)).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let bytes=response.into_body().collect().await.unwrap().to_bytes();let metadata=String::from_utf8(bytes.to_vec()).unwrap();assert!(!metadata.contains(secret));assert!(!metadata.contains("df_pat_v1_"));assert!(!metadata.contains("pepper"));assert!(!metadata.contains("digest"));
    assert_eq!(token_mutation(&f,&format!("/app/api/developer-tokens/{id}/revoke"),json!({"password":null}),true).await.0,StatusCode::OK);
    let(status,_,body)=request(f.router(),"/api/v1/account",Some(secret)).await;assert_eq!(status,StatusCode::UNAUTHORIZED);assert_eq!(body["error"]["code"],"invalid_token");f.finish().await;
}
