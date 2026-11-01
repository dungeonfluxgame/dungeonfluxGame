//! Real PostgreSQL and native-store draft delivery through the composed route.
//!
//! Root runs these against the owned scratch server; no green run is claimed here. Each test
//! opens its own database, publishes a Ready account draft through the raw catalog and native
//! store, and drives the exact router the host mounts. Store handles and draft connections are
//! opened per adapter; production wires singletons in `runtime.rs`.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use df_assets::{
    AdmitOutcome, AssetId, AssetManifest, AssetState, AttemptId, Audience, CatalogStore,
    ClaimOutcome, ClaimToken, Deadline, DedupeScope, DraftDeliveryError, DraftDeliveryLimits,
    DraftImageVariant, FinalizeRequest, FinalizeResult, HeldStep, KeyLayout, LeaseOutcome,
    MediaRequestKey, NativeFileStore, ObjectEnvironment, ObjectKey, ObjectStore, ObjectStoreError,
    ObjectVersion, QuotaScope, StageOutcome, StagedObjectRecord, Step, StepClaim, StorageDomain,
    StoredObject, TombstoneReason, VariantIntent, VariantWritten, Winner,
};
use df_assets::{AssetKind, DeliveryBudget, Provenance, RetentionClass, WriteId};
use df_customer::{
    AccountStore, BuiltInCommonPasswords, CustomerConfig, CustomerService, DeviceLabel,
    EmailAddress, EmailError, HashingParameters, HostConfig, IdentityProviderKind, IdentitySource,
    NewIdentityAccount, NewSession, RevokeReason, RevokeScope, SecretToken, ServiceTasks, Tunables,
};
use df_persistence::{
    account_closure::TimeoutPolicy, account_store::PostgresAccountStore,
    media_catalog::PostgresMediaCatalog, postgres_tls::PostgresTransport,
};
use df_types::{AccountId, WebSessionId};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use crate::database::Connection;
use crate::draft_images::DraftImageAdapter;
use df_portal::draft_images::DraftImageSource;

const ORIGIN: &str = "http://127.0.0.1:8099";
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
const MAX_IMAGE_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
struct NoMail;

impl df_customer::EmailSender for NoMail {
    async fn send(&self, _message: df_customer::Message) -> Result<(), EmailError> {
        panic!("integration tests never send email");
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

fn layout() -> KeyLayout {
    KeyLayout::new(ObjectEnvironment::new("test").unwrap())
}

struct Fixture {
    admin: Connection,
    database: String,
    db: tokio_postgres::Config,
    transport: PostgresTransport,
    data: Connection,
    service: CustomerService<PostgresAccountStore, NoMail>,
    tasks: ServiceTasks,
    accounts: PostgresAccountStore,
    catalog: PostgresMediaCatalog,
    root: std::path::PathBuf,
    roles: Vec<String>,
    tunables: Tunables,
}

fn pg_config(dbname: &str) -> Option<tokio_postgres::Config> {
    let port_text = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT").unwrap_or("55471".to_owned());
    let port: u16 = port_text.parse().expect("scratch port");
    assert!(port == 55471, "unowned PostgreSQL port");
    let mut config = tokio_postgres::Config::new();
    let host = std::env::var("DUNGEONFLUX_RUNTIME_PG_HOST").unwrap_or("127.0.0.1".into());
    let user = std::env::var("DUNGEONFLUX_RUNTIME_PG_USER").unwrap_or("postgres".into());
    config
        .host(host.as_str())
        .port(port)
        .user(user.as_str())
        .dbname(dbname);
    Some(config)
}

fn require_scratch() -> bool {
    std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_some()
        || std::env::var_os("REQUIRE_SCRATCH_PG").is_some()
}

impl Fixture {
    async fn open() -> Option<Self> {
        Self::open_schema(Tunables::default(), false).await
    }

    async fn with_tunables(tunables: Tunables) -> Option<Self> {
        Self::open_schema(tunables, false).await
    }

    async fn open_schema(tunables: Tunables, hosted_schema: bool) -> Option<Self> {
        let admin_config = pg_config("postgres")?;
        let transport = PostgresTransport::PlaintextLoopbackOnly;
        let admin = match Connection::open(&admin_config, &transport).await {
            Ok(admin) => admin,
            Err(_) => {
                assert!(
                    !require_scratch(),
                    "DUNGEONFLUX_REQUIRE_SCRATCH_PG is set but no scratch PostgreSQL answers"
                );
                eprintln!("SKIPPED: no owned scratch PostgreSQL");
                return None;
            }
        };
        // Creating/removing a scratch database can wait for filesystem checkpoint work.
        admin
            .client
            .batch_execute("SET statement_timeout='30s'; SET lock_timeout='30s'")
            .await
            .unwrap();
        let mut db = admin_config.clone();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!("draftimg_{nanos}_{}", std::process::id());
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
        let catalog = if hosted_schema {
            crate::database::migrate(&mut data).await.unwrap();
            PostgresMediaCatalog::open_existing(db.clone(), transport.clone(), 2)
                .await
                .unwrap()
        } else {
            PostgresMediaCatalog::open(db.clone(), transport.clone(), 2)
                .await
                .unwrap()
        };
        let root = std::env::temp_dir().join(&database);
        let (service, tasks) = CustomerService::start(
            CustomerConfig {
                hosts: HostConfig::local(ORIGIN).unwrap(),
                identity: IdentitySource::local_passwords(),
                tunables: tunables.clone(),
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
            Arc::new(df_customer::SystemClock),
            Arc::new(BuiltInCommonPasswords),
        )
        .await
        .unwrap();
        Some(Self {
            admin,
            database,
            db,
            transport,
            data,
            service,
            tasks,
            accounts,
            catalog,
            root,
            roles: Vec::new(),
            tunables,
        })
    }

    fn objects(&self) -> NativeFileStore {
        NativeFileStore::new(&self.root, 8 * 1024 * 1024).unwrap()
    }

    async fn drafts(&self) -> Connection {
        Connection::open(&self.db, &self.transport).await.unwrap()
    }

    fn adapter(
        &self,
        drafts: Connection,
        concurrent: usize,
    ) -> DraftImageAdapter<PostgresMediaCatalog, NativeFileStore> {
        DraftImageAdapter::new(
            self.catalog.clone(),
            self.objects(),
            drafts,
            self.tunables.sessions,
            DraftDeliveryLimits {
                max_image_bytes: MAX_IMAGE_BYTES,
            },
            DeliveryBudget::new(concurrent),
            Duration::from_secs(10),
        )
    }

    fn router<C: CatalogStore + 'static, O: ObjectStore + 'static>(
        &self,
        adapter: DraftImageAdapter<C, O>,
    ) -> Router {
        crate::draft_images::router(self.service.clone(), Arc::new(adapter))
            .layer(axum::middleware::from_fn(crate::runtime::private_headers))
    }

    async fn finish(self) {
        let Self {
            admin,
            database,
            data,
            service,
            tasks,
            accounts,
            catalog,
            root,
            roles,
            tunables: _,
            db: _,
            transport: _,
        } = self;
        tasks.shutdown().await;
        drop(service);
        drop(accounts);
        drop(catalog);
        drop(data);
        assert_eq!(root, std::env::temp_dir().join(&database));
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
        wait_for_connections(&admin, &database).await;
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database}"))
            .await
            .unwrap();
        for role in roles {
            admin
                .client
                .batch_execute(&format!("DROP ROLE {role}"))
                .await
                .unwrap();
        }
    }
}

async fn wait_for_connections(admin: &Connection, database: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count: i64 = admin
                .client
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname=$1",
                    &[&database],
                )
                .await
                .unwrap()
                .get(0);
            if count == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned PostgreSQL connections close before scratch database removal");
}

async fn seed_account(
    accounts: &PostgresAccountStore,
    key: u8,
    created_at_ms: u64,
    credential_generation: u64,
) -> (AccountId, String) {
    let account = AccountId::from_bytes(&[key; 16]).unwrap();
    accounts
        .create_identity_account(NewIdentityAccount {
            id: account,
            email: EmailAddress::parse(&format!("person{key}@example.test")).unwrap(),
            is_verified: true,
            // A synthetic persisted link uses the production schema vocabulary; no provider is called.
            provider: IdentityProviderKind::WorkOs,
            subject: format!("synthetic-{key}"),
            created_at_ms,
            adult_attested_at: Some(created_at_ms),
            terms_version: Some("terms-2026-10".into()),
            terms_accepted_at: Some(created_at_ms),
            verification: None,
        })
        .await
        .unwrap();
    let token = SecretToken::generate().unwrap();
    accounts
        .create_session(
            NewSession {
                id: WebSessionId::from_bytes(&[key; 16]).unwrap(),
                digest: token.digest(),
                account,
                credential_generation,
                created_at_ms,
                absolute_expires_at_ms: created_at_ms + 30 * DAY_MS as u64,
                csrf_secret: df_customer::CsrfSecret::from_bytes([key; 32]),
                device: DeviceLabel::Desktop,
            },
            None,
        )
        .await
        .unwrap();
    (account, token.text())
}

struct ReadyDraft {
    asset: AssetId,
    master: Vec<u8>,
    display: Vec<u8>,
}

/// Publishes a Ready account draft through the raw catalog and native store. `master_only`
/// leaves the display variant unwritten; `mime` is stored as the validated publication fact.
async fn seed_ready_draft(
    catalog: &PostgresMediaCatalog,
    objects: &NativeFileStore,
    account: AccountId,
    tag: u8,
    mime: &str,
    master_only: bool,
) -> ReadyDraft {
    let asset = AssetId::from_bytes(&[tag + 10; 16]).unwrap();
    let attempt = AttemptId::from_bytes(&[tag + 20; 16]).unwrap();
    let claim = ClaimToken::from_bytes(&[tag + 30; 16]).unwrap();
    let master_write = WriteId::from_bytes(&[tag + 40; 16]).unwrap();
    let display_write = WriteId::from_bytes(&[tag + 41; 16]).unwrap();
    let now = now_ms();
    let admission = df_assets::Admission {
        asset,
        attempt,
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(account),
        origin_account: Some(account),
        campaign: None,
        audience: Audience::OwnerPrivate(account),
        dedupe_scope: DedupeScope::account(&account),
        request_key: Some(MediaRequestKey::from_bytes([tag; 32])),
        retention: RetentionClass::Personal,
        reserve_bytes: 10_000,
        byte_limit: u64::MAX,
        in_flight_upload_limit: None,
    };
    assert_eq!(
        catalog.admit(&admission, now).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let staged = StagedObjectRecord {
        key: layout().staged(
            asset,
            attempt,
            WriteId::from_bytes(&[tag + 50; 16]).unwrap(),
        ),
        version: ObjectVersion::new("s1".to_owned()).unwrap(),
        manifest: manifest_of(&[9; 4]),
        staged_at_ms: now,
    };
    assert_eq!(
        catalog
            .record_staged(asset, attempt, &staged, now)
            .await
            .unwrap(),
        StageOutcome::Staged
    );
    let held = HeldStep {
        asset,
        attempt,
        owner: claim,
    };
    assert_eq!(
        catalog
            .claim_step(
                StepClaim {
                    asset,
                    attempt,
                    step: Step::Validate,
                    expected_state: AssetState::Quarantined,
                    owner: claim,
                    lease_until_ms: now + 60_000,
                },
                now
            )
            .await
            .unwrap(),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        catalog.mark_validated(held, now).await.unwrap(),
        LeaseOutcome::Done
    );
    let master = vec![7; 512];
    let display = vec![8; 256];
    let master_key = layout().account_draft_asset(
        &account,
        asset,
        attempt,
        master_write,
        df_assets::AssetVariant::Master,
    );
    write_variant(
        catalog,
        objects,
        held,
        df_assets::AssetVariant::Master,
        &master,
        &master_key,
        master_write,
    )
    .await;
    let mut winners = vec![Winner {
        variant: df_assets::AssetVariant::Master,
        write: master_write,
    }];
    if !master_only {
        let display_key = layout().account_draft_asset(
            &account,
            asset,
            attempt,
            display_write,
            df_assets::AssetVariant::Display,
        );
        write_variant(
            catalog,
            objects,
            held,
            df_assets::AssetVariant::Display,
            &display,
            &display_key,
            display_write,
        )
        .await;
        winners.push(Winner {
            variant: df_assets::AssetVariant::Display,
            write: display_write,
        });
    }
    assert!(matches!(
        catalog
            .finalize(
                held,
                &FinalizeRequest {
                    winners,
                    mime: mime.to_owned(),
                    width_px: Some(2),
                    height_px: Some(2),
                    byte_limit: u64::MAX,
                },
                now
            )
            .await
            .unwrap(),
        FinalizeResult::Ready { .. }
    ));
    ReadyDraft {
        asset,
        master,
        display,
    }
}

async fn write_variant(
    catalog: &PostgresMediaCatalog,
    objects: &NativeFileStore,
    held: HeldStep,
    variant: df_assets::AssetVariant,
    bytes: &[u8],
    key: &ObjectKey,
    write: WriteId,
) {
    assert_eq!(
        catalog
            .intend_variant(
                held,
                &VariantIntent {
                    variant,
                    write,
                    key: key.clone()
                },
                now_ms()
            )
            .await
            .unwrap(),
        LeaseOutcome::Done
    );
    let stored = objects
        .put_new(
            key,
            bytes,
            manifest_of(bytes),
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap();
    assert!(
        catalog
            .mark_variant_written(
                held.asset,
                held.attempt,
                &VariantWritten {
                    variant,
                    write,
                    version: stored.version,
                    manifest: manifest_of(bytes),
                },
            )
            .await
            .unwrap()
    );
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

async fn get(router: Router, path: &str, cookie: Option<&str>) -> Reply {
    let mut builder = Request::builder().uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(
            header::COOKIE,
            format!("{}={cookie}", df_portal::SESSION_COOKIE),
        );
    }
    let response = router
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

fn private(reply: &Reply) {
    assert_eq!(private_cache(reply), "private, no-store");
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

fn private_cache(reply: &Reply) -> &str {
    reply.headers[header::CACHE_CONTROL].to_str().unwrap()
}

fn path_of(asset: AssetId, query: &str) -> String {
    format!("/app/api/draft-images/{}{query}", asset.to_hex())
}

#[tokio::test]
async fn owner_get_is_exact_and_private() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 3, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 3, "image/png", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let reply = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, draft.display);
    assert_eq!(
        reply.headers[header::CONTENT_TYPE].to_str().unwrap(),
        "image/png"
    );
    assert_eq!(
        reply.headers[header::CONTENT_LENGTH].to_str().unwrap(),
        draft.display.len().to_string()
    );
    private(&reply);
    fixture.finish().await;
}

#[tokio::test]
async fn master_selection_is_exact() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 4, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 4, "image/png", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let reply = get(
        router,
        &path_of(draft.asset, "?variant=master"),
        Some(&cookie),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, draft.master);
    private(&reply);
    fixture.finish().await;
}

#[tokio::test]
async fn strangers_and_bad_sessions_see_nothing() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 5, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 5, "image/png", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let path = path_of(draft.asset, "");
    let anonymous = get(router.clone(), &path, None).await;
    assert_eq!(anonymous.status, StatusCode::NOT_FOUND);
    assert!(anonymous.body.is_empty());
    private(&anonymous);

    let (_, foreign_cookie) = seed_account(&fixture.accounts, 6, now, 1).await;
    let foreign = get(router.clone(), &path, Some(&foreign_cookie)).await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);
    assert!(foreign.body.is_empty());
    private(&foreign);

    fixture
        .accounts
        .revoke_sessions(
            owner,
            RevokeScope::All,
            RevokeReason::RevokedByUser,
            now,
            None,
        )
        .await
        .unwrap();
    let revoked = get(router.clone(), &path, Some(&cookie)).await;
    assert_eq!(revoked.status, StatusCode::NOT_FOUND);
    assert!(revoked.body.is_empty());
    private(&revoked);

    let expired_at = now - 40 * DAY_MS as u64;
    let (_, expired_cookie) = seed_account(&fixture.accounts, 7, expired_at, 1).await;
    let expired = get(router.clone(), &path, Some(&expired_cookie)).await;
    assert_eq!(expired.status, StatusCode::NOT_FOUND);
    assert!(expired.body.is_empty());

    let (_, stale_cookie) = seed_account(&fixture.accounts, 8, now, 99).await;
    let stale = get(router.clone(), &path, Some(&stale_cookie)).await;
    assert_eq!(stale.status, StatusCode::NOT_FOUND);
    assert!(stale.body.is_empty());

    let guessed = AssetId::from_bytes(&[77; 16]).unwrap();
    let missing = get(router.clone(), &path_of(guessed, ""), Some(&foreign_cookie)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.is_empty());

    let forged = path_of(draft.asset, "?account=99999999999999999999999999999999");
    let denied = get(router, &forged, Some(&foreign_cookie)).await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    assert!(denied.body.is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn wrong_winner_and_unsupported_mime_are_denied() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 9, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 9, "image/png", true).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let missing = get(
        router.clone(),
        &path_of(draft.asset, "?variant=display"),
        Some(&cookie),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.is_empty());
    private(&missing);
    let master = get(
        router,
        &path_of(draft.asset, "?variant=master"),
        Some(&cookie),
    )
    .await;
    assert_eq!(master.status, StatusCode::OK);
    assert_eq!(master.body, draft.master);

    let (owner, cookie) = seed_account(&fixture.accounts, 10, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 10, "image/gif", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let denied = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    assert!(denied.body.is_empty());
    private(&denied);
    fixture.finish().await;
}

/// A native store that tombstones the asset right after the byte load, before final release.
struct TombstoningStore {
    inner: NativeFileStore,
    catalog: PostgresMediaCatalog,
    asset: AssetId,
    fire: AtomicBool,
}

impl ObjectStore for TombstoningStore {
    fn conditional_writes(&self) -> df_assets::ConditionalWrites {
        self.inner.conditional_writes()
    }

    async fn put_new(
        &self,
        key: &ObjectKey,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        self.inner.put_new(key, body, expected, deadline).await
    }

    async fn put_if_match(
        &self,
        key: &ObjectKey,
        current: &ObjectVersion,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        self.inner
            .put_if_match(key, current, body, expected, deadline)
            .await
    }

    async fn head(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Option<StoredObject>, ObjectStoreError> {
        self.inner.head(key, deadline).await
    }

    async fn get_version(
        &self,
        object: &StoredObject,
        deadline: Deadline,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let bytes = self.inner.get_version(object, deadline).await?;
        if self.fire.swap(false, Ordering::SeqCst) {
            self.catalog
                .tombstone(self.asset, TombstoneReason::UserDelete, now_ms())
                .await
                .unwrap();
        }
        Ok(bytes)
    }

    async fn list_versions(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Vec<ObjectVersion>, df_assets::VersionListing> {
        self.inner.list_versions(key, deadline).await
    }

    async fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
        deadline: Deadline,
    ) -> Result<(), ObjectStoreError> {
        self.inner.delete_version(key, version, deadline).await
    }

    async fn list_page(
        &self,
        prefix: &df_assets::ObjectPrefix,
        after: Option<&str>,
        limit: usize,
        deadline: Deadline,
    ) -> Result<df_assets::ListPage, ObjectStoreError> {
        self.inner.list_page(prefix, after, limit, deadline).await
    }

    async fn versioning_status(
        &self,
        domain: StorageDomain,
        deadline: Deadline,
    ) -> Result<df_assets::VersioningStatus, ObjectStoreError> {
        self.inner.versioning_status(domain, deadline).await
    }
}

#[tokio::test]
async fn tombstone_during_load_discards_the_bytes() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 11, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 11, "image/png", false).await;
    let hooked = TombstoningStore {
        inner: fixture.objects(),
        catalog: fixture.catalog.clone(),
        asset: draft.asset,
        fire: AtomicBool::new(true),
    };
    let adapter = DraftImageAdapter::new(
        fixture.catalog.clone(),
        hooked,
        fixture.drafts().await,
        Tunables::default().sessions,
        DraftDeliveryLimits {
            max_image_bytes: MAX_IMAGE_BYTES,
        },
        DeliveryBudget::new(4),
        Duration::from_secs(10),
    );
    let router = fixture.router(adapter);
    let reply = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
    private(&reply);
    assert_eq!(
        fixture
            .catalog
            .asset(draft.asset)
            .await
            .unwrap()
            .unwrap()
            .state,
        AssetState::Deleted
    );
    fixture.finish().await;
}

#[tokio::test]
async fn revocation_between_buffer_and_release_denies() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 12, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 12, "image/png", false).await;
    let adapter = Arc::new(fixture.adapter(fixture.drafts().await, 4));
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap();
    fixture
        .accounts
        .revoke_sessions(
            owner,
            RevokeScope::All,
            RevokeReason::RevokedByUser,
            now,
            None,
        )
        .await
        .unwrap();
    let session = fixture.service.authenticate(&cookie).await.unwrap();
    assert!(session.is_none());
    let revoked_session = WebSessionId::from_bytes(&[12; 16]).unwrap();
    let denied = adapter
        .authorize_release(
            owner,
            revoked_session,
            candidate.basis().clone(),
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, DraftDeliveryError::NotFound);
    drop(candidate);
    drop(adapter);

    let (owner, cookie) = seed_account(&fixture.accounts, 14, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 14, "image/png", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let reply = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, draft.display);
    fixture.finish().await;
}

#[tokio::test]
async fn exhausted_concurrency_is_a_503_without_body() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 15, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 15, "image/png", false).await;
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 0));
    let reply = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(reply.body.is_empty());
    private(&reply);
    fixture.finish().await;
}

#[tokio::test]
async fn display_serves_when_the_aggregate_is_larger() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 16, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 16, "image/png", false).await;
    assert_ne!(draft.master, draft.display);
    let record = fixture.catalog.asset(draft.asset).await.unwrap().unwrap();
    let published = record.published.as_ref().unwrap();
    assert_eq!(
        published.byte_len,
        draft.master.len() as u64 + draft.display.len() as u64
    );
    assert!(published.byte_len > draft.display.len() as u64);
    let router = fixture.router(fixture.adapter(fixture.drafts().await, 4));
    let display = get(router.clone(), &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(display.status, StatusCode::OK);
    assert_eq!(display.body, draft.display);
    private(&display);
    let master = get(
        router,
        &path_of(draft.asset, "?variant=master"),
        Some(&cookie),
    )
    .await;
    assert_eq!(master.status, StatusCode::OK);
    assert_eq!(master.body, draft.master);
    private(&master);
    fixture.finish().await;
}

#[tokio::test]
async fn unpolled_body_retains_its_permit() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, _) = seed_account(&fixture.accounts, 17, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 17, "image/png", false).await;
    let adapter = Arc::new(fixture.adapter(fixture.drafts().await, 1));
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap();
    let body = candidate.into_body();
    let denied = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, DraftDeliveryError::Busy);
    drop(body);
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap();
    assert_eq!(candidate.bytes(), draft.display.as_slice());
    drop(candidate);
    drop(adapter);
    fixture.finish().await;
}

#[tokio::test]
async fn expired_deadline_denies_release() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, _) = seed_account(&fixture.accounts, 18, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 18, "image/png", false).await;
    let adapter = Arc::new(fixture.adapter(fixture.drafts().await, 4));
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(30)),
        )
        .await
        .unwrap();
    let session = WebSessionId::from_bytes(&[18; 16]).unwrap();
    let denied = adapter
        .authorize_release(
            owner,
            session,
            candidate.basis().clone(),
            Deadline::after(Duration::ZERO),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, DraftDeliveryError::DeadlineExceeded);
    drop(candidate);
    drop(adapter);
    fixture.finish().await;
}

async fn disclosure_reader(fixture: &mut Fixture) -> tokio_postgres::Config {
    let reader = format!(
        "draftreader_{}",
        df_customer::SecretToken::generate()
            .unwrap()
            .text()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            .chars()
            .take(40)
            .collect::<String>()
    );
    fixture
        .admin
        .client
        .batch_execute(&format!(
            "CREATE ROLE {reader} WITH LOGIN; GRANT CONNECT ON DATABASE {} TO {reader}",
            fixture.database,
        ))
        .await
        .unwrap();
    fixture.roles.push(reader.clone());
    fixture
        .data
        .client
        .batch_execute(&format!(
            "GRANT USAGE ON SCHEMA accounts, media TO {reader}; \
         GRANT SELECT (session_id, account_id, credential_generation, created_at_ms, \
         last_seen_at_ms, revoked_at_ms, absolute_expires_at_ms) \
         ON accounts.web_session TO {reader}; \
         GRANT SELECT (account_id, status, credential_generation) \
         ON accounts.account TO {reader}; \
         GRANT SELECT (asset_id, state, kind, provenance, campaign_id, charge_scope_kind, \
         charge_scope_id, origin_account, audience_kind, audience_account, attempt, \
         winning_writes, mime, byte_length, sha256, suppression_generation) \
         ON media.asset TO {reader}; \
         GRANT SELECT (asset_id, attempt, write_id, variant, state, object_domain, \
         object_version, byte_length, sha256) \
         ON media.asset_variant TO {reader};"
        ))
        .await
        .unwrap();
    let mut reader_config = fixture.db.clone();
    reader_config.user(&reader);
    reader_config
}

#[tokio::test]
async fn narrow_reader_role_serves_the_owner() {
    let Some(mut fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 19, now, 1).await;
    let objects = fixture.objects();
    let draft = seed_ready_draft(&fixture.catalog, &objects, owner, 19, "image/png", false).await;
    let reader_config = disclosure_reader(&mut fixture).await;
    let reader_connection = Connection::open(&reader_config, &fixture.transport)
        .await
        .unwrap();
    crate::draft_images::check_disclosure_reader(&reader_connection)
        .await
        .unwrap();
    let excessive = fixture.drafts().await;
    assert_eq!(
        crate::draft_images::check_disclosure_reader(&excessive)
            .await
            .unwrap_err(),
        DraftDeliveryError::Unavailable
    );
    drop(excessive);
    for statement in [
        "UPDATE media.asset SET state = state",
        "UPDATE accounts.web_session SET revoked_at_ms = 1",
        "SELECT token_digest FROM accounts.web_session",
    ] {
        let error = reader_connection
            .client
            .batch_execute(statement)
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    let adapter = DraftImageAdapter::new(
        fixture.catalog.clone(),
        fixture.objects(),
        reader_connection,
        Tunables::default().sessions,
        DraftDeliveryLimits {
            max_image_bytes: MAX_IMAGE_BYTES,
        },
        DeliveryBudget::new(4),
        Duration::from_secs(10),
    );
    let router = fixture.router(adapter);
    let reply = get(router, &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, draft.display);
    private(&reply);
    fixture.finish().await;
}

#[tokio::test]
async fn configured_composition_uses_the_writer_volume_and_two_read_roles() {
    let Some(mut fixture) = Fixture::open_schema(Tunables::default(), true).await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 21, now, 1).await;
    let draft = seed_ready_draft(
        &fixture.catalog,
        &fixture.objects(),
        owner,
        21,
        "image/png",
        false,
    )
    .await;
    // Recreate the prior hosted-only ledger after publishing bytes, then adopt it in place.
    fixture
        .data
        .client
        .batch_execute(
            "INSERT INTO hosted.migration(number) SELECT version FROM media.schema_migration \
         ON CONFLICT(number) DO NOTHING; DROP TABLE media.schema_migration",
        )
        .await
        .unwrap();
    crate::database::migrate(&mut fixture.data).await.unwrap();
    let adopted = fixture
        .data
        .client
        .query(
            "SELECT version FROM media.schema_migration ORDER BY version",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<_, i32>(0))
        .collect::<Vec<_>>();
    assert_eq!(adopted, vec![10, 11, 12, 13, 14]);
    let disclosure = disclosure_reader(&mut fixture).await;
    let reader = format!("catalog_{}", fixture.database);
    fixture
        .admin
        .client
        .batch_execute(&format!(
            "CREATE ROLE {reader} WITH LOGIN; GRANT CONNECT ON DATABASE {} TO {reader}",
            fixture.database,
        ))
        .await
        .unwrap();
    fixture.roles.push(reader.clone());
    fixture
        .data
        .client
        .batch_execute(&format!(
            "GRANT USAGE ON SCHEMA media TO {reader}; \
         GRANT SELECT ON media.asset, media.asset_variant, media.schema_migration TO {reader}"
        ))
        .await
        .unwrap();
    let mut catalog = fixture.db.clone();
    catalog.user(&reader);
    let ordinary = Connection::open(&catalog, &fixture.transport)
        .await
        .unwrap();
    for statement in [
        "UPDATE media.asset SET state=state",
        "SELECT session_id FROM accounts.web_session",
    ] {
        assert_eq!(
            ordinary
                .client
                .batch_execute(statement)
                .await
                .unwrap_err()
                .code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    drop(ordinary);
    std::fs::create_dir_all(fixture.root.join("bundle")).unwrap();
    std::fs::write(
        fixture.root.join("bundle/index.html"),
        "<!doctype html><title>Draft delivery</title>",
    )
    .unwrap();
    let url = |database: &tokio_postgres::Config| {
        format!(
            "host=127.0.0.1 port=55471 user={} dbname={}",
            database.get_user().unwrap(),
            fixture.database,
        )
    };
    let values = std::collections::HashMap::from([
        ("DF_PROFILE", "development".to_owned()),
        ("DF_GAMEPLAY_PUBLIC_URL", ORIGIN.to_owned()),
        ("DF_GAMEPLAY_DATABASE_TLS", "loopback-plaintext".to_owned()),
        ("DF_PORTAL_DATABASE_URL", url(&fixture.db)),
        ("DF_PORTAL_MIGRATOR_DATABASE_URL", url(&fixture.db)),
        (
            "DF_PORTAL_BUNDLE_DIR",
            fixture.root.join("bundle").display().to_string(),
        ),
        (
            "DF_PORTAL_OUTBOX",
            fixture.root.join("outbox").display().to_string(),
        ),
        ("DF_TRIAL_EMAIL_HMAC_KEY", "33".repeat(32)),
        ("DF_CAMPAIGN_IP_KEY", "11".repeat(32)),
        ("DF_CAMPAIGN_CODE_KEY", "22".repeat(32)),
        ("DF_DRAFT_DELIVERY_ENABLED", "true".to_owned()),
        ("DF_MEDIA_OBJECT_ROOT", fixture.root.display().to_string()),
        ("DF_MEDIA_CATALOG_DATABASE_URL", url(&catalog)),
        ("DF_DRAFT_DISCLOSURE_DATABASE_URL", url(&disclosure)),
        ("DF_DRAFT_MAX_IMAGE_BYTES", MAX_IMAGE_BYTES.to_string()),
        ("DF_DRAFT_MAX_DELIVERY_BYTES", MAX_IMAGE_BYTES.to_string()),
        ("DF_DRAFT_OPERATION_TIMEOUT_MS", "10000".to_owned()),
    ]);
    let config = crate::config::Config::parse(|key| values.get(key).cloned()).unwrap();
    let hosted = crate::Hosted::open(config).await.unwrap();
    let router = hosted.router();
    let request = Request::builder()
        .uri(path_of(draft.asset, ""))
        .header(
            header::COOKIE,
            format!("{}={cookie}", df_portal::SESSION_COOKIE),
        )
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let blocked = get(router.clone(), &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(blocked.status, StatusCode::SERVICE_UNAVAILABLE);
    // Retaining the emitted HTTP allocation must retain the singleton delivery budget.
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), draft.display.as_slice());
    let blocked = get(router.clone(), &path_of(draft.asset, ""), Some(&cookie)).await;
    assert_eq!(blocked.status, StatusCode::SERVICE_UNAVAILABLE);
    drop(bytes);
    let reply = get(
        router,
        &path_of(draft.asset, "?variant=master"),
        Some(&cookie),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, draft.master);
    private(&reply);
    hosted.shutdown().await;
    fixture.finish().await;
}

#[tokio::test]
async fn disabled_composition_keeps_a_private_unavailable_route() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (_, cookie) = seed_account(&fixture.accounts, 22, now, 1).await;
    let router = crate::draft_composition::open(
        crate::draft_composition::Settings::Disabled,
        &fixture.transport,
        fixture.tunables.sessions,
        fixture.service.clone(),
    )
    .await
    .unwrap()
    .layer(axum::middleware::from_fn(crate::runtime::private_headers));
    let path = path_of(AssetId::from_bytes(&[1; 16]).unwrap(), "");
    let anonymous = get(router.clone(), &path, None).await;
    assert_eq!(anonymous.status, StatusCode::NOT_FOUND);
    private(&anonymous);
    let unavailable = get(router, &path, Some(&cookie)).await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(unavailable.body.is_empty());
    private(&unavailable);
    fixture.finish().await;
}

#[tokio::test]
async fn final_snapshot_rejects_account_closure_suppression_and_changed_manifests() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, _) = seed_account(&fixture.accounts, 23, now, 1).await;
    let draft = seed_ready_draft(
        &fixture.catalog,
        &fixture.objects(),
        owner,
        23,
        "image/png",
        false,
    )
    .await;
    let adapter = fixture.adapter(fixture.drafts().await, 1);
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    let session = WebSessionId::from_bytes(&[23; 16]).unwrap();
    for change in [
        "UPDATE accounts.account SET status='closing'",
        "UPDATE media.asset SET suppression_generation=suppression_generation+1",
        "UPDATE media.asset SET byte_length=byte_length+1",
        "UPDATE media.asset_variant SET byte_length=byte_length+1 WHERE variant='display'",
        "UPDATE accounts.web_session SET absolute_expires_at_ms=1",
    ] {
        fixture.data.client.batch_execute("BEGIN").await.unwrap();
        fixture.data.client.batch_execute(change).await.unwrap();
        // The mutation must commit before the final authorization snapshot.
        fixture.data.client.batch_execute("COMMIT").await.unwrap();
        let denied = adapter
            .authorize_release(
                owner,
                session,
                candidate.basis().clone(),
                Deadline::after(Duration::from_secs(10)),
            )
            .await
            .unwrap_err();
        assert_eq!(denied, DraftDeliveryError::NotFound);
        fixture
            .data
            .client
            .batch_execute(
                "UPDATE accounts.account SET status='active'; \
            UPDATE media.asset SET suppression_generation=0, byte_length=768; \
            UPDATE media.asset_variant SET byte_length=256 WHERE variant='display'; \
            UPDATE accounts.web_session SET absolute_expires_at_ms=9223372036854775807",
            )
            .await
            .unwrap();
    }
    drop(candidate);
    drop(adapter);
    fixture.finish().await;
}

#[tokio::test]
async fn final_snapshot_uses_the_customer_session_lifetimes() {
    let mut tunables = Tunables::default();
    tunables.sessions.idle = Duration::from_secs(60);
    let Some(fixture) = Fixture::with_tunables(tunables).await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, cookie) = seed_account(&fixture.accounts, 24, now - 120_000, 1).await;
    let draft = seed_ready_draft(
        &fixture.catalog,
        &fixture.objects(),
        owner,
        24,
        "image/png",
        false,
    )
    .await;
    assert!(
        fixture
            .service
            .authenticate(&cookie)
            .await
            .unwrap()
            .is_none()
    );
    let adapter = fixture.adapter(fixture.drafts().await, 1);
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    let denied = adapter
        .authorize_release(
            owner,
            WebSessionId::from_bytes(&[24; 16]).unwrap(),
            candidate.basis().clone(),
            Deadline::after(Duration::from_secs(10)),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, DraftDeliveryError::NotFound);
    drop(candidate);
    drop(adapter);
    fixture.finish().await;
}

#[tokio::test]
async fn blocked_final_snapshot_obeys_the_deadline_and_releases_its_budget() {
    let Some(fixture) = Fixture::open().await else {
        return;
    };
    let now = u64::try_from(now_ms()).unwrap();
    let (owner, _) = seed_account(&fixture.accounts, 25, now, 1).await;
    let draft = seed_ready_draft(
        &fixture.catalog,
        &fixture.objects(),
        owner,
        25,
        "image/png",
        false,
    )
    .await;
    let adapter = fixture.adapter(fixture.drafts().await, 1);
    let candidate = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    fixture
        .data
        .client
        .batch_execute("BEGIN; LOCK TABLE media.asset IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let denied = adapter
        .authorize_release(
            owner,
            WebSessionId::from_bytes(&[25; 16]).unwrap(),
            candidate.basis().clone(),
            Deadline::after(Duration::from_millis(25)),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, DraftDeliveryError::DeadlineExceeded);
    fixture.data.client.batch_execute("ROLLBACK").await.unwrap();
    drop(candidate);
    let next = adapter
        .buffer_candidate(
            owner,
            draft.asset,
            DraftImageVariant::Display,
            Deadline::after(Duration::from_secs(10)),
        )
        .await
        .unwrap();
    assert_eq!(next.bytes(), draft.display.as_slice());
    drop(next);
    drop(adapter);
    fixture.finish().await;
}
