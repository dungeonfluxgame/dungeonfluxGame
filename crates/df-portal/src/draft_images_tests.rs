//! Route checks over the memory account store with a fake narrow adapter: owner success,
//! exact headers, HEAD/range behavior, concealed denials, and the re-authentication order.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
};
use df_assets::{
    AssetId, AttemptId, Deadline, DeliveryBasis, DraftDeliveryError, DraftImageCandidate,
    DraftImageVariant, WriteId,
};
use df_customer::{
    AccountStore, Authenticated, BuiltInCommonPasswords, Client, CustomerConfig, CustomerService,
    DeviceLabel, EmailAddress, FakeClock, HashingParameters, HostConfig, IdentityProviderKind,
    IdentitySource, NewIdentityAccount, NewSession, RevokeScope, SecretToken, Tunables,
    contract::START_MS,
    fakes::{MemoryAccountStore, RecordingMail},
};
use df_types::{AccountId, WebSessionId};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::{DraftImagePolicy, DraftImageSource, router};
use crate::SESSION_COOKIE;

const ORIGIN: &str = "http://127.0.0.1:8099";
const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

type BufferAnswer = Result<(DeliveryBasis, String, Vec<u8>), DraftDeliveryError>;
type BufferHook = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

struct FakeSource {
    buffer: Mutex<BufferAnswer>,
    authorize: Mutex<Result<(), DraftDeliveryError>>,
    buffer_calls: AtomicUsize,
    authorize_calls: AtomicUsize,
    on_buffer: Mutex<Option<BufferHook>>,
    seen_owner: Mutex<Vec<AccountId>>,
    seen_variant: Mutex<Vec<DraftImageVariant>>,
}

impl FakeSource {
    fn serving(asset: AssetId, owner: AccountId, bytes: Vec<u8>) -> Self {
        let basis = DeliveryBasis::for_tests(df_assets::DeliveryBasisFixture {
            asset: asset,
            owner: owner,
            attempt: AttemptId::from_bytes(&[7; 16]).unwrap(),
            write: WriteId::from_bytes(&[8; 16]).unwrap(),
            variant: df_assets::AssetVariant::Display,
            version: "v1".to_owned(),
            digest: [9; 32],
            byte_len: bytes.len() as u64,
            suppression_generation: 0,
            mime: "image/png".to_owned(),
            published_byte_len: bytes.len() as u64,
            published_digest: [9; 32],
        });
        Self::answering(Ok((basis, "image/png".to_owned(), bytes)), Ok(()))
    }

    fn answering(buffer: BufferAnswer, authorize: Result<(), DraftDeliveryError>) -> Self {
        Self {
            buffer: Mutex::new(buffer),
            authorize: Mutex::new(authorize),
            buffer_calls: AtomicUsize::new(0),
            authorize_calls: AtomicUsize::new(0),
            on_buffer: Mutex::new(None),
            seen_owner: Mutex::new(Vec::new()),
            seen_variant: Mutex::new(Vec::new()),
        }
    }
}

impl DraftImageSource for FakeSource {
    async fn buffer_candidate(
        &self,
        owner: AccountId,
        _asset: AssetId,
        variant: DraftImageVariant,
        _deadline: Deadline,
    ) -> Result<DraftImageCandidate, DraftDeliveryError> {
        self.buffer_calls.fetch_add(1, Ordering::SeqCst);
        self.seen_owner.lock().unwrap().push(owner);
        self.seen_variant.lock().unwrap().push(variant);
        if let Some(hook) = self.on_buffer.lock().unwrap().clone() {
            hook().await;
        }
        match self.buffer.lock().unwrap().clone() {
            Ok((basis, mime, bytes)) => Ok(DraftImageCandidate::for_tests(basis, mime, bytes)),
            Err(error) => Err(error),
        }
    }

    async fn authorize_release(
        &self,
        _owner: AccountId,
        _session: WebSessionId,
        _basis: DeliveryBasis,
        deadline: Deadline,
    ) -> Result<(), DraftDeliveryError> {
        self.authorize_calls.fetch_add(1, Ordering::SeqCst);
        if deadline.has_passed() {
            return Err(DraftDeliveryError::DeadlineExceeded);
        }
        self.authorize.lock().unwrap().clone()
    }
}

struct Harness {
    router: Router,
    service: CustomerService<MemoryAccountStore, RecordingMail>,
    store: MemoryAccountStore,
    _tasks: df_customer::ServiceTasks,
    source: Arc<FakeSource>,
    owner: AccountId,
    asset: AssetId,
    cookie: String,
}

async fn harness(source: FakeSource) -> Harness {
    harness_with(source, START_MS, 1, START_MS + 30 * DAY_MS).await
}

async fn harness_with(
    source: FakeSource,
    created_at_ms: u64,
    credential_generation: u64,
    absolute_expires_at_ms: u64,
) -> Harness {
    let mail = RecordingMail::default();
    let store = MemoryAccountStore::default();
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts: HostConfig::local(ORIGIN).unwrap(),
            identity: IdentitySource::local_passwords(),
            tunables: Tunables::default(),
            hashing: HashingParameters::TEST,
            maximum_concurrent_hashes: 4,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        store.clone(),
        mail,
        Arc::new(FakeClock::at(START_MS)),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .unwrap();
    let owner = AccountId::from_bytes(&[3; 16]).unwrap();
    let asset = AssetId::from_bytes(&[11; 16]).unwrap();
    let cookie = seed_session(
        &store,
        owner,
        3,
        created_at_ms,
        credential_generation,
        absolute_expires_at_ms,
    )
    .await;
    let source = Arc::new(source);
    let router = router(
        service.clone(),
        Arc::clone(&source),
        DraftImagePolicy::default(),
    );
    Harness {
        router,
        service,
        store,
        _tasks: tasks,
        source,
        owner,
        asset,
        cookie,
    }
}

async fn seed_session(
    store: &MemoryAccountStore,
    account: AccountId,
    key: u8,
    created_at_ms: u64,
    credential_generation: u64,
    absolute_expires_at_ms: u64,
) -> String {
    seed_account(store, account, key, created_at_ms).await;
    let token = SecretToken::generate().unwrap();
    store
        .create_session(
            NewSession {
                id: WebSessionId::from_bytes(&[key; 16]).unwrap(),
                digest: token.digest(),
                account,
                credential_generation,
                created_at_ms,
                absolute_expires_at_ms,
                csrf_secret: df_customer::CsrfSecret::from_bytes([key; 32]),
                device: DeviceLabel::Desktop,
            },
            None,
        )
        .await
        .unwrap();
    token.text()
}

async fn seed_account(store: &MemoryAccountStore, account: AccountId, key: u8, created_at_ms: u64) {
    store
        .create_identity_account(NewIdentityAccount {
            id: account,
            email: EmailAddress::parse(&format!("person{key}@example.test")).unwrap(),
            is_verified: true,
            provider: IdentityProviderKind::Fake,
            subject: format!("synthetic-{key}"),
            created_at_ms,
            adult_attested_at: Some(created_at_ms),
            terms_version: Some("terms-2026-10".into()),
            terms_accepted_at: Some(created_at_ms),
            verification: None,
        })
        .await
        .unwrap();
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Harness {
    async fn send(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        extra: HashMap<String, String>,
    ) -> Reply {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(cookie) = cookie {
            builder = builder.header(
                header::COOKIE,
                HeaderValue::from_str(&format!("{SESSION_COOKIE}={cookie}")).unwrap(),
            );
        }
        for (name, value) in extra {
            builder = builder.header(name, HeaderValue::from_str(&value).unwrap());
        }
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        Reply {
            status,
            headers,
            body,
        }
    }

    fn path(&self, query: &str) -> String {
        format!("/app/api/draft-images/{}{query}", self.asset.to_hex())
    }

    async fn caller(&self) -> Authenticated {
        self.service
            .authenticate(&self.cookie)
            .await
            .unwrap()
            .unwrap()
    }
}

fn private(reply: &Reply) {
    assert_eq!(cache_control(reply), "private, no-store");
    assert_eq!(
        reply.headers[header::X_CONTENT_TYPE_OPTIONS]
            .to_str()
            .unwrap(),
        "nosniff"
    );
    assert_eq!(
        reply.headers["cross-origin-resource-policy"]
            .to_str()
            .unwrap(),
        "same-origin"
    );
    assert!(!reply.headers.contains_key(header::ETAG));
    assert!(!reply.headers.contains_key(header::SET_COOKIE));
}

fn cache_control(reply: &Reply) -> &str {
    reply.headers[header::CACHE_CONTROL].to_str().unwrap()
}

#[tokio::test]
async fn owner_get_is_exact_and_private() {
    let bytes = vec![1, 2, 3, 4];
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        bytes.clone(),
    ))
    .await;
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, bytes);
    assert_eq!(
        reply.headers[header::CONTENT_TYPE].to_str().unwrap(),
        "image/png"
    );
    assert_eq!(reply.headers[header::CONTENT_LENGTH].to_str().unwrap(), "4");
    private(&reply);
    assert_eq!(harness.source.buffer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.source.authorize_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness.source.seen_owner.lock().unwrap().as_slice(),
        &[harness.owner]
    );
    assert_eq!(
        harness.source.seen_variant.lock().unwrap().as_slice(),
        &[DraftImageVariant::Display]
    );
}

#[tokio::test]
async fn head_shares_authorization_and_omits_bytes() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let reply = harness
        .send(
            Method::HEAD,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.is_empty());
    assert_eq!(reply.headers[header::CONTENT_LENGTH].to_str().unwrap(), "4");
    private(&reply);
}

#[tokio::test]
async fn range_receives_the_full_bounded_response() {
    let bytes = vec![1, 2, 3, 4];
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        bytes.clone(),
    ))
    .await;
    let mut extra = HashMap::new();
    extra.insert("range".to_owned(), "bytes=0-1".to_owned());
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            extra,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, bytes);
}

#[tokio::test]
async fn anonymous_and_unknown_assets_are_concealed() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let reply = harness
        .send(Method::GET, &harness.path(""), None, HashMap::new())
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    for path in [
        "/app/api/draft-images/not-hex".to_owned(),
        format!(
            "/app/api/draft-images/{}",
            AssetId::from_bytes(&[77; 16]).unwrap().to_hex()
        ),
    ] {
        let reply = harness
            .send(
                Method::GET,
                &path,
                Some(&harness.cookie.clone()),
                HashMap::new(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(reply.body.is_empty());
        private(&reply);
    }
}

#[tokio::test]
async fn foreign_account_is_concealed_and_seen_as_itself() {
    let asset = AssetId::from_bytes(&[11; 16]).unwrap();
    let harness = harness(FakeSource::answering(
        Err(DraftDeliveryError::NotFound),
        Ok(()),
    ))
    .await;
    let foreign = AccountId::from_bytes(&[5; 16]).unwrap();
    seed_account(&harness.store, foreign, 5, START_MS).await;
    let cookie = seed_session(
        &harness.store,
        foreign,
        5,
        START_MS,
        1,
        START_MS + 30 * DAY_MS,
    )
    .await;
    let path = format!("/app/api/draft-images/{}", asset.to_hex());
    let reply = harness
        .send(Method::GET, &path, Some(&cookie), HashMap::new())
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    assert_eq!(
        harness.source.seen_owner.lock().unwrap().as_slice(),
        &[foreign]
    );
    assert_eq!(harness.source.authorize_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn request_account_claims_are_not_authority() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let mut extra = HashMap::new();
    extra.insert(
        "x-account-id".to_owned(),
        "99999999999999999999999999999999".to_owned(),
    );
    let path = format!(
        "/app/api/draft-images/{}?account={}",
        harness.asset.to_hex(),
        AccountId::from_bytes(&[5; 16]).unwrap().to_hex()
    );
    let reply = harness
        .send(Method::GET, &path, Some(&harness.cookie.clone()), extra)
        .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "forgery must not deny the owner"
    );
    assert_eq!(reply.body, vec![1, 2, 3, 4]);
    assert_eq!(
        harness.source.seen_owner.lock().unwrap().as_slice(),
        &[harness.owner]
    );
}

#[tokio::test]
async fn revoked_expired_and_stale_sessions_are_concealed() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let caller = harness.caller().await;
    harness
        .service
        .revoke_sessions(
            &caller,
            RevokeScope::All,
            Client {
                ip: None,
                device: DeviceLabel::Desktop,
            },
        )
        .await
        .unwrap();
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    assert_eq!(harness.source.buffer_calls.load(Ordering::SeqCst), 0);

    let expired = harness_with(
        FakeSource::answering(Err(DraftDeliveryError::NotFound), Ok(())),
        START_MS - 40 * DAY_MS,
        1,
        START_MS - 10 * DAY_MS,
    )
    .await;
    let reply = expired
        .send(
            Method::GET,
            &expired.path(""),
            Some(&expired.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    assert_eq!(expired.source.buffer_calls.load(Ordering::SeqCst), 0);

    let stale = harness_with(
        FakeSource::answering(Err(DraftDeliveryError::NotFound), Ok(())),
        START_MS,
        99,
        START_MS + 30 * DAY_MS,
    )
    .await;
    let reply = stale
        .send(
            Method::GET,
            &stale.path(""),
            Some(&stale.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    assert_eq!(stale.source.buffer_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn revocation_between_buffer_and_release_is_denied() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let caller = harness.caller().await;
    let service = harness.service.clone();
    harness
        .source
        .on_buffer
        .lock()
        .unwrap()
        .replace(Arc::new(move || {
            let service = service.clone();
            let caller = caller.clone();
            Box::pin(async move {
                service
                    .revoke_sessions(
                        &caller,
                        RevokeScope::All,
                        Client {
                            ip: None,
                            device: DeviceLabel::Desktop,
                        },
                    )
                    .await
                    .unwrap();
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        }));
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    assert_eq!(harness.source.buffer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.source.authorize_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tombstone_before_release_denies_without_bytes() {
    let harness = harness(FakeSource::answering(
        Ok((
            DeliveryBasis::for_tests(df_assets::DeliveryBasisFixture {
                asset: AssetId::from_bytes(&[11; 16]).unwrap(),
                owner: AccountId::from_bytes(&[3; 16]).unwrap(),
                attempt: AttemptId::from_bytes(&[7; 16]).unwrap(),
                write: WriteId::from_bytes(&[8; 16]).unwrap(),
                variant: df_assets::AssetVariant::Display,
                version: "v1".to_owned(),
                digest: [9; 32],
                byte_len: 4,
                suppression_generation: 1,
                mime: "image/png".to_owned(),
                published_byte_len: 4,
                published_digest: [9; 32],
            }),
            "image/png".to_owned(),
            vec![1, 2, 3, 4],
        )),
        Err(DraftDeliveryError::NotFound),
    ))
    .await;
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    assert_eq!(harness.source.authorize_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn authority_failures_are_503_without_body() {
    for error in [DraftDeliveryError::Unavailable, DraftDeliveryError::Busy] {
        let harness = harness(FakeSource::answering(Err(error), Ok(()))).await;
        let reply = harness
            .send(
                Method::GET,
                &harness.path(""),
                Some(&harness.cookie.clone()),
                HashMap::new(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{error:?}");
        assert!(reply.body.is_empty());
        private(&reply);
    }
    let harness = harness(FakeSource::answering(
        Ok((
            DeliveryBasis::for_tests(df_assets::DeliveryBasisFixture {
                asset: AssetId::from_bytes(&[11; 16]).unwrap(),
                owner: AccountId::from_bytes(&[3; 16]).unwrap(),
                attempt: AttemptId::from_bytes(&[7; 16]).unwrap(),
                write: WriteId::from_bytes(&[8; 16]).unwrap(),
                variant: df_assets::AssetVariant::Display,
                version: "v1".to_owned(),
                digest: [9; 32],
                byte_len: 4,
                suppression_generation: 1,
                mime: "image/png".to_owned(),
                published_byte_len: 4,
                published_digest: [9; 32],
            }),
            "image/png".to_owned(),
            vec![1, 2, 3, 4],
        )),
        Err(DraftDeliveryError::Unknown),
    ))
    .await;
    let reply = harness
        .send(
            Method::GET,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(reply.body.is_empty());
    private(&reply);
}

#[tokio::test]
async fn variant_selector_is_closed_and_exact() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let reply = harness
        .send(
            Method::GET,
            &harness.path("?variant=master"),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        harness.source.seen_variant.lock().unwrap().as_slice(),
        &[DraftImageVariant::Master]
    );
    let reply = harness
        .send(
            Method::GET,
            &harness.path("?variant=original"),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    assert_eq!(harness.source.buffer_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unknown_and_malformed_queries_are_concealed() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    for query in [
        "?foo=bar",
        "?variant=display&foo=bar",
        "?variant",
        "?variant=DISPLAY",
        "?variant=master&variant=display",
        "?variant=%ZZ",
        "?variant=display%ZZ",
    ] {
        let reply = harness
            .send(
                Method::GET,
                &harness.path(query),
                Some(&harness.cookie.clone()),
                HashMap::new(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{query}");
        assert!(reply.body.is_empty(), "{query}");
        private(&reply);
    }
    assert_eq!(harness.source.buffer_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn extractor_and_method_failures_stay_private() {
    let harness = harness(FakeSource::serving(
        AssetId::from_bytes(&[11; 16]).unwrap(),
        AccountId::from_bytes(&[3; 16]).unwrap(),
        vec![1, 2, 3, 4],
    ))
    .await;
    let reply = harness
        .send(
            Method::GET,
            "/app/api/draft-images/%ZZ",
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert!(reply.status == StatusCode::NOT_FOUND || reply.status == StatusCode::BAD_REQUEST);
    assert!(reply.body.is_empty());
    private(&reply);
    let reply = harness
        .send(
            Method::POST,
            &harness.path(""),
            Some(&harness.cookie.clone()),
            HashMap::new(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    private(&reply);
}

#[tokio::test]
async fn stalled_buffer_exceeds_the_operation_deadline() {
    struct Stalled;
    impl DraftImageSource for Stalled {
        async fn buffer_candidate(
            &self,
            _owner: AccountId,
            _asset: AssetId,
            _variant: DraftImageVariant,
            _deadline: Deadline,
        ) -> Result<DraftImageCandidate, DraftDeliveryError> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Err(DraftDeliveryError::Unavailable)
        }

        async fn authorize_release(
            &self,
            _owner: AccountId,
            _session: WebSessionId,
            _basis: DeliveryBasis,
            _deadline: Deadline,
        ) -> Result<(), DraftDeliveryError> {
            Ok(())
        }
    }
    let mail = RecordingMail::default();
    let store = MemoryAccountStore::default();
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts: HostConfig::local(ORIGIN).unwrap(),
            identity: IdentitySource::local_passwords(),
            tunables: Tunables::default(),
            hashing: HashingParameters::TEST,
            maximum_concurrent_hashes: 4,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        store.clone(),
        mail,
        Arc::new(FakeClock::at(START_MS)),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .unwrap();
    let owner = AccountId::from_bytes(&[3; 16]).unwrap();
    let asset = AssetId::from_bytes(&[11; 16]).unwrap();
    let cookie = seed_session(&store, owner, 3, START_MS, 1, START_MS + 30 * DAY_MS).await;
    let router = router(
        service,
        Arc::new(Stalled),
        DraftImagePolicy {
            operation_timeout: Duration::from_millis(50),
        },
    );
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/app/api/draft-images/{}", asset.to_hex()))
                .header(
                    header::COOKIE,
                    HeaderValue::from_str(&format!("{SESSION_COOKIE}={cookie}")).unwrap(),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.is_empty());
    assert_eq!(
        headers[header::CACHE_CONTROL].to_str().unwrap(),
        "private, no-store"
    );
    tasks.shutdown().await;
}
