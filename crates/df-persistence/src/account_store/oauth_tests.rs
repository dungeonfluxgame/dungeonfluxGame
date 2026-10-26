use super::*;
use crate::{account_closure::TimeoutPolicy, postgres_tls::PostgresTransport};
use df_customer::{AccountStore, Clock, FakeClock, contract::START_MS};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls};

struct Connection {
    client: Client,
    driver: JoinHandle<()>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
async fn connect(config: &Config) -> Connection {
    let mut config = config.clone();
    config.options(
        "-c statement_timeout=5s -c lock_timeout=5s -c idle_in_transaction_session_timeout=10s",
    );
    let (client, connection) =
        tokio::time::timeout(std::time::Duration::from_secs(5), config.connect(NoTls))
            .await
            .expect("owned scratch PostgreSQL connect deadline")
            .expect("owned scratch PostgreSQL 127.0.0.1:55911");
    Connection {
        client,
        driver: tokio::spawn(async move {
            connection.await.expect("owned PG connection");
        }),
    }
}

struct Fixture {
    admin: Connection,
    database: String,
    store: PostgresAccountStore,
    privileged: PostgresAccountStore,
}
impl Fixture {
    async fn open() -> Self {
        Self::open_from(false).await
    }

    async fn open_from(legacy: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!(
            "tenantissue_google_{}_{}_{}",
            std::process::id(),
            clock,
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(55911)
            .user("postgres")
            .dbname("postgres");
        let admin = connect(&config).await;
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        config.dbname(&database);
        if legacy {
            let old = connect(&config).await;
            old.client.batch_execute("CREATE SCHEMA accounts; SET search_path TO accounts; CREATE TABLE schema_migration(version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())").await.unwrap();
            old.client
                .batch_execute(include_str!("../../migrations/0005_accounts.sql"))
                .await
                .unwrap();
            old.client
                .batch_execute(include_str!("../../migrations/0006_account_closure.sql"))
                .await
                .unwrap();
            old.client.batch_execute("ALTER TABLE account DROP COLUMN adult_attested_at CASCADE, DROP COLUMN terms_version CASCADE, DROP COLUMN terms_accepted_at CASCADE; INSERT INTO schema_migration(version) VALUES (5),(6),(9998),(9999)").await.unwrap();
            old.client.execute("INSERT INTO account(account_id,email_normalized,email_display,email_verified_at_ms,status,credential_generation,created_at_ms) VALUES ($1,$2,$2,$3,'active',1,$3)", &[&&[42_u8;16][..],&"legacy@example.test",&i64::try_from(START_MS).unwrap()]).await.unwrap();
        }
        let privileged = PostgresAccountStore::open(
            config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap();
        privileged
            .test_batch(super::super::closure_tests::SCHEMA_SQL)
            .await;
        privileged.test_batch(
            "GRANT USAGE ON SCHEMA campaigns_test TO df_accounts; \
             GRANT SELECT, UPDATE ON campaigns_test.campaign, campaigns_test.proposal TO df_accounts; \
             GRANT SELECT ON campaigns_test.member TO df_accounts"
        ).await;
        config.options("-c role=df_accounts -c transaction_timeout=10s");
        let store = PostgresAccountStore::open(
            config,
            PostgresTransport::PlaintextLoopbackOnly,
            2,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap()
        .with_closure_participant(Arc::new(super::super::closure_tests::TestCampaigns));
        Self {
            admin,
            database,
            store,
            privileged,
        }
    }

    async fn finish(self) {
        let Self {
            admin,
            database,
            store,
            privileged,
        } = self;
        drop(store);
        drop(privileged);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let row = admin
                    .client
                    .query_one(
                        "SELECT count(*) FROM pg_stat_activity WHERE datname = $1",
                        &[&database],
                    )
                    .await
                    .unwrap();
                if row.get::<_, i64>(0) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all owned database connections close");
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database}"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn googleauth_upgrade_preserves_existing_accounts_and_adds_missing_consent_columns() {
    let fixture = Fixture::open_from(true).await;
    let account = fixture
        .store
        .account_by_email("legacy@example.test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.id.as_bytes(), &[42_u8; 16]);
    assert!(account.is_verified);
    let tenant = fixture.store.tenant_for(account.id).await.unwrap().unwrap();
    assert_ne!(tenant.as_bytes(), account.id.as_bytes());
    df_customer::oauth_contract::new_returning_and_single_use(fixture.store.clone()).await;
    assert_eq!(
        fixture.store.tenant_for(account.id).await.unwrap(),
        Some(tenant)
    );
    fixture.finish().await;
}

#[tokio::test]
async fn googleauth_service_role_signup_returning_replay_expiry_and_consent() {
    let fixture = Fixture::open().await;
    df_customer::oauth_contract::new_returning_and_single_use(fixture.store.clone()).await;
    let rows=fixture.store.test_query("SELECT adult_attested_at,terms_version,terms_accepted_at FROM account WHERE email_normalized = 'google-new@example.test'",&[]).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i64>(0), i64::try_from(START_MS).unwrap());
    assert_eq!(rows[0].get::<_, String>(1), "terms-2026-10");
    assert_eq!(rows[0].get::<_, i64>(2), i64::try_from(START_MS).unwrap());
    let row = fixture
        .store
        .test_query(
            "SELECT t.tenant_id, t.owner_account_id FROM tenant t JOIN account a \
         ON a.account_id = t.owner_account_id WHERE a.email_normalized = 'google-new@example.test'",
            &[],
        )
        .await;
    assert_eq!(row.len(), 1);
    assert_ne!(row[0].get::<_, Vec<u8>>(0), row[0].get::<_, Vec<u8>>(1));
    fixture.finish().await;
}

#[tokio::test]
async fn verification_issues_a_distinct_stable_personal_tenant() {
    let fixture = Fixture::open().await;
    df_customer::tenant_contract::verification_issues_a_distinct_stable_personal_tenant(
        fixture.store.clone(),
    )
    .await;
    fixture.finish().await;
}

#[tokio::test]
async fn identity_signup_issues_a_tenant_only_for_verified_email() {
    let fixture = Fixture::open().await;
    df_customer::tenant_contract::identity_signup_issues_a_tenant_only_for_verified_email(
        fixture.store.clone(),
    )
    .await;
    fixture.finish().await;
}

#[tokio::test]
async fn password_reset_proves_an_unverified_email_and_issues_one_tenant() {
    let fixture = Fixture::open().await;
    df_customer::tenant_contract::password_reset_proves_an_unverified_email_and_issues_one_tenant(
        fixture.store.clone(),
    )
    .await;
    fixture.finish().await;
}

fn signup_resolution() -> GoogleResolution {
    GoogleResolution {
        subject: "tenantissue-google-subject".to_owned(),
        email: df_customer::EmailAddress::parse("tenantissue-google@example.test").unwrap(),
        intent: OAuthIntent::SignUp,
        account: None,
        credential_generation: None,
        originating_session: None,
        idle_before_ms: 0,
        new_account: AccountId::from_bytes(&[74; 16]).unwrap(),
        terms_version: Some("terms-2026-10".to_owned()),
        accepted_at_ms: START_MS,
        now_ms: START_MS,
    }
}

#[tokio::test]
async fn google_tenant_failure_rolls_back_signup_and_concurrent_retries_keep_one_binding() {
    let fixture = Fixture::open().await;
    fixture
        .privileged
        .test_batch("ALTER TABLE tenant ADD CONSTRAINT reject_tenant CHECK (false) NOT VALID")
        .await;
    assert!(matches!(
        fixture.store.resolve(signup_resolution()).await,
        Err(StoreError::Unavailable)
    ));
    for table in ["account", "google_identity", "tenant", "account_event"] {
        assert_eq!(
            fixture
                .store
                .test_query(&format!("SELECT count(*) FROM {table}"), &[])
                .await[0]
                .get::<_, i64>(0),
            0,
            "{table} rolled back"
        );
    }
    fixture
        .privileged
        .test_batch("ALTER TABLE tenant DROP CONSTRAINT reject_tenant")
        .await;
    let (first, second) = tokio::join!(
        fixture.store.resolve(signup_resolution()),
        fixture.store.resolve(signup_resolution()),
    );
    assert!(matches!(
        (&first, &second),
        (
            Ok(GoogleResolved::Created(_)),
            Ok(GoogleResolved::Account(_))
        ) | (
            Ok(GoogleResolved::Account(_)),
            Ok(GoogleResolved::Created(_))
        )
    ));
    let id = signup_resolution().new_account;
    let tenant = fixture.store.tenant_for(id).await.unwrap().unwrap();
    assert_ne!(tenant.as_bytes(), id.as_bytes());
    assert_eq!(
        fixture
            .store
            .test_query("SELECT count(*) FROM tenant", &[])
            .await[0]
            .get::<_, i64>(0),
        1
    );
    assert!(matches!(
        fixture.store.resolve(signup_resolution()).await.unwrap(),
        GoogleResolved::Account(_)
    ));
    assert_eq!(fixture.store.tenant_for(id).await.unwrap(), Some(tenant));
    fixture.finish().await;
}

#[tokio::test]
async fn googleauth_service_role_password_collision_link_and_unlink() {
    let fixture = Fixture::open().await;
    df_customer::oauth_contract::email_collision_link_and_unlink(fixture.store.clone()).await;
    fixture.finish().await;
}

fn attempt(state: TokenDigest, cookie: TokenDigest, now_ms: u64) -> OAuthAttempt {
    OAuthAttempt {
        redirect_uri: "http://127.0.0.1:8099/app/api/oauth/google/callback".to_owned(),
        state_digest: state,
        cookie_digest: cookie,
        client_digest: [3; 32],
        nonce: OAuthSecret::new("a".repeat(64)),
        verifier: OAuthSecret::new("b".repeat(64)),
        intent: OAuthIntent::SignIn,
        account: None,
        session: None,
        credential_generation: None,
        terms_version: None,
        created_at_ms: now_ms,
        expires_at_ms: now_ms + 600_000,
    }
}

#[tokio::test]
async fn googleauth_atomic_claim_wins_once_and_periodic_purge_removes_expired_secrets() {
    let fixture = Fixture::open().await;
    let state = TokenDigest::from_bytes([1; 32]);
    let cookie = TokenDigest::from_bytes([2; 32]);
    assert!(
        fixture
            .store
            .save_attempt(attempt(state, cookie, START_MS))
            .await
            .unwrap()
    );
    let (first, second) = tokio::join!(
        fixture.store.claim_attempt(state, cookie, START_MS),
        fixture.store.claim_attempt(state, cookie, START_MS)
    );
    assert_eq!(
        usize::from(first.unwrap().is_some()) + usize::from(second.unwrap().is_some()),
        1
    );
    assert!(
        fixture
            .store
            .save_attempt(attempt(state, cookie, START_MS))
            .await
            .unwrap()
    );
    assert!(
        fixture
            .store
            .claim_attempt(state, cookie, START_MS + 600_000)
            .await
            .unwrap()
            .is_none()
    );
    let clock = FakeClock::at(START_MS + 600_000);
    fixture
        .store
        .purge(df_customer::RetentionCutoffs {
            now_ms: clock.now_ms(),
            sessions_ended_before_ms: 0,
            sessions_last_seen_before_ms: 0,
            tokens_before_ms: 0,
            receipts_before_ms: 0,
        })
        .await
        .unwrap();
    assert_eq!(
        fixture
            .store
            .test_query("SELECT count(*) FROM oauth_attempt", &[])
            .await[0]
            .get::<_, i64>(0),
        0
    );
    fixture.finish().await;
}
