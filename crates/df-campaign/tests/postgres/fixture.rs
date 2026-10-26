//! The scratch-database fixture: one fresh database per test, B's real accounts migrations (0005,
//! 0006), a copy of O1's service lock and fence functions from 0015 (the full runtime chain needs
//! migrations this fixture cannot apply), then this crate's 0008 and 0009. The store connects
//! through a per-test login role that is a member of the production role `df_campaigns`, so every
//! test runs under the production privileges. Only databases named with this fixture's prefix are
//! ever dropped.

use df_campaign::store::NewInvitation;
use std::{
    sync::OnceLock,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use df_campaign::{
    clock::{Clock, FakeClock},
    config::Tunables,
    errors::StoreError,
    ids::FeedEpoch,
    join_invitation::JoinRole,
    records::{CampaignRow, ContentRating, EventRow, NewCampaign, RoomCodes},
    roles::Delegations,
    secrets::{INVITATION_TOKEN_TAG, SecretToken, tagged_digest},
    store::CampaignStore,
};
use df_types::{AccountId, CampaignId, MemberId, OperationId, SessionId, TenantId};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls};

use crate::campaign_store::{Connect, PgCampaignStore, TimeoutPolicy};

const FORBIDDEN_PORTS: [u16; 1] = [63309];
const DEFAULT_DATABASE_PREFIX: &str = "df_campaigns_test_s1_";
const STALE_AFTER_SECONDS: u64 = 2 * 3_600;

fn database_prefix() -> &'static str {
    static PREFIX: OnceLock<String> = OnceLock::new();
    PREFIX
        .get_or_init(|| {
            let prefix = std::env::var("DUNGEONFLUX_CAMPAIGNS_PG_DATABASE_PREFIX")
                .unwrap_or_else(|_| DEFAULT_DATABASE_PREFIX.to_owned());
            assert!(
                prefix.ends_with('_')
                    && prefix.len() <= 32
                    && prefix.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                "DUNGEONFLUX_CAMPAIGNS_PG_DATABASE_PREFIX must be a <=32 character SQL-safe prefix ending in underscore"
            );
            prefix
        })
        .as_str()
}

/// How old a leftover must be to be dropped; `DUNGEONFLUX_CAMPAIGNS_PG_STALE_SECONDS` overrides it
/// (0 drops every leftover of this fixture, for a run when no other S1 test is live).
fn stale_after() -> u64 {
    std::env::var("DUNGEONFLUX_CAMPAIGNS_PG_STALE_SECONDS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(STALE_AFTER_SECONDS)
}

/// The scratch server's port: `DUNGEONFLUX_CAMPAIGNS_PG_PORT`, default 55911. Never the game
/// server's port.
pub fn port() -> u16 {
    let port = std::env::var("DUNGEONFLUX_CAMPAIGNS_PG_PORT")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(55_911);
    assert!(
        !FORBIDDEN_PORTS.contains(&port),
        "port {port} is not a scratch database"
    );
    port
}

fn config(role: &str, database: &str) -> Config {
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port())
        .user(role)
        .dbname(database);
    config
}

pub struct PgConnector {
    config: Config,
}

impl PgConnector {
    pub fn new(role: &str, database: &str) -> Self {
        Self {
            config: config(role, database),
        }
    }
}

impl Connect for PgConnector {
    fn connect(&self) -> crate::campaign_store::ConnectFuture<'_> {
        Box::pin(async move {
            let (client, connection) = self
                .config
                .connect(NoTls)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            let driver = tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok((client, driver))
        })
    }
}

/// A raw client (the superuser for setup, or a login role for lock-holding sessions).
pub struct Raw {
    pub client: Client,
    driver: JoinHandle<()>,
}

impl Drop for Raw {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

pub async fn raw(role: &str, database: &str) -> Raw {
    let (client, connection) = config(role, database)
        .connect(NoTls)
        .await
        .expect("the scratch database is reachable");
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    Raw { client, driver }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Copied from O1's `0015_runtime_directory.sql` (the lock key, `enter_service`,
/// `require_service` and `fence_current`) with the campaign-owner table reduced to the columns
/// the fence reads. TODO(S1-03): replace with the real chain when the fixture can apply it.
const RUNTIME_STAND_IN: &str = "
CREATE SCHEMA df_meta;
CREATE SCHEMA df_runtime;
CREATE TABLE df_meta.service_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    serving text NOT NULL, recovery_epoch bigint NOT NULL, checked_at timestamptz NOT NULL);
INSERT INTO df_meta.service_state VALUES (true, 'serving', 1, clock_timestamp());
CREATE TABLE df_runtime.campaign_owners (
    tenant_id bytea NOT NULL, campaign_id bytea NOT NULL, recovery_epoch bigint NOT NULL,
    claim_seq bigint NOT NULL, state text NOT NULL, placement_id bytea,
    lease_until timestamptz NOT NULL, money_state text NOT NULL DEFAULT 'reconciled',
    PRIMARY KEY (tenant_id, campaign_id));
CREATE FUNCTION df_meta.service_lock_key() RETURNS bigint
    LANGUAGE sql IMMUTABLE AS $$ SELECT 7283410100::bigint $$;
CREATE TYPE df_runtime.fence_path AS ENUM ('game', 'paid');
CREATE FUNCTION df_runtime.enter_service() RETURNS void
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
    AS $$ SELECT pg_advisory_xact_lock_shared(df_meta.service_lock_key()) $$;
CREATE FUNCTION df_runtime.require_service() RETURNS void
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
    AS $$ BEGIN
        IF NOT pg_try_advisory_xact_lock_shared(df_meta.service_lock_key()) THEN
            RAISE EXCEPTION 'service halting' USING ERRCODE = 'DF002';
        END IF;
    END $$;
CREATE FUNCTION df_runtime.fence_current(
    p_tenant_id bytea, p_campaign_id bytea, p_recovery_epoch bigint, p_claim_seq bigint,
    p_placement_id bytea, p_path df_runtime.fence_path) RETURNS boolean
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
    AS $$ BEGIN
        PERFORM df_runtime.require_service();
        RETURN EXISTS (
            SELECT 1 FROM df_meta.service_state s, df_runtime.campaign_owners o
             WHERE s.serving = 'serving'
               AND o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
               AND o.recovery_epoch = p_recovery_epoch AND o.claim_seq = p_claim_seq
               AND o.placement_id = p_placement_id
               AND o.state IN ('serving', 'draining') AND o.lease_until > clock_timestamp()
               AND (p_path = 'game' OR o.money_state = 'reconciled')
             FOR SHARE OF o);
    END $$;
GRANT USAGE ON SCHEMA df_runtime, df_meta TO df_campaigns;
GRANT EXECUTE ON FUNCTION df_runtime.enter_service(), df_runtime.fence_current(
    bytea, bytea, bigint, bigint, bytea, df_runtime.fence_path) TO df_campaigns;
";

/// At most this many test databases are live at once, so the scratch server's connection limit
/// is not the thing the suite measures.
const LIVE_DATABASES: usize = 6;

fn gate() -> std::sync::Arc<tokio::sync::Semaphore> {
    static GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(LIVE_DATABASES)))
        .clone()
}

pub struct TestDb {
    pub name: String,
    pub admin: Raw,
    roles: Vec<String>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl TestDb {
    /// A fresh database with every migration applied. Stale databases of this fixture are dropped
    /// first; nobody else's is touched.
    pub async fn create(label: &str) -> Self {
        let permit = gate()
            .acquire_owned()
            .await
            .expect("the gate is never closed");
        let postgres = raw("postgres", "postgres").await;
        scavenge(&postgres).await;
        let name = format!(
            "{}{}_{}_{}",
            database_prefix(),
            unix_seconds(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
            label
        );
        assert!(name.len() <= 63, "database name {name} is too long");
        postgres
            .client
            .batch_execute(&format!("CREATE DATABASE \"{name}\""))
            .await
            .expect("create the test database");
        let admin = raw("postgres", &name).await;
        admin
            .client
            .batch_execute("CREATE SCHEMA accounts; SET search_path TO accounts; \n                 CREATE TABLE schema_migration (version integer PRIMARY KEY, \n                 applied_at timestamptz NOT NULL DEFAULT now())")
            .await
            .expect("accounts schema");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0005_accounts.sql"
            ))
            .await
            .expect("migration 0005");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0006_account_closure.sql"
            ))
            .await
            .expect("migration 0006");
        admin
            .client
            .batch_execute("RESET search_path")
            .await
            .expect("reset search_path");
        admin
            .client
            .batch_execute(RUNTIME_STAND_IN)
            .await
            .expect("runtime stand-in");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0008_campaigns.sql"
            ))
            .await
            .expect("migration 0008");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0009_lobby.sql"
            ))
            .await
            .expect("migration 0009");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0036_campaign_settings.sql"
            ))
            .await
            .expect("migration 0036");
        admin
            .client
            .batch_execute(include_str!(
                "../../../df-persistence/migrations/0037_campaign_preferences_owner_boundary.sql"
            ))
            .await
            .expect("migration 0037");
        Self {
            name,
            admin,
            roles: Vec::new(),
            _permit: permit,
        }
    }

    /// A login role that is a member of the production role. `timeout` is its own
    /// `transaction_timeout` default (settings are not inherited through membership).
    pub async fn login_role(&mut self, timeout: Option<&str>) -> String {
        let role = format!(
            "df_s1_{}_{}",
            unix_seconds(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        self.admin
            .client
            .batch_execute(&format!("CREATE ROLE {role} LOGIN IN ROLE df_campaigns"))
            .await
            .expect("create the login role");
        if let Some(timeout) = timeout {
            self.admin
                .client
                .batch_execute(&format!(
                    "ALTER ROLE {role} SET transaction_timeout = '{timeout}'"
                ))
                .await
                .expect("set the role default");
        }
        self.roles.push(role.clone());
        role
    }

    pub fn connector(&self, role: &str) -> Box<dyn Connect> {
        Box::new(PgConnector::new(role, &self.name))
    }

    pub async fn raw_as(&self, role: &str) -> Raw {
        raw(role, &self.name).await
    }

    /// Drops the database and the roles this test created.
    pub async fn cleanup(self) {
        let Self {
            name,
            admin,
            roles,
            _permit,
        } = self;
        drop(admin);
        let postgres = raw("postgres", "postgres").await;
        let _ = postgres
            .client
            .batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .await;
        for role in roles {
            let _ = postgres
                .client
                .batch_execute(&format!("DROP ROLE IF EXISTS {role}"))
                .await;
        }
    }

    pub async fn seed_account(&self, number: u8) -> AccountId {
        let id = account_id(number);
        self.admin
            .client
            .execute(
                "INSERT INTO accounts.account (account_id, email_normalized, email_display, \
                 email_verified_at_ms, display_name, status, credential_generation, created_at_ms) \
                 VALUES ($1, $2, $2, 1, $3, 'active', 1, 1) ON CONFLICT DO NOTHING",
                &[
                    &id.as_bytes().as_slice(),
                    &format!("user{number}@example.test"),
                    &format!("Person {number}"),
                ],
            )
            .await
            .expect("seed the account");
        id
    }

    pub async fn set_account_status(&self, account: AccountId, status: &str) {
        let result = if status == "closed" {
            self.admin
                .client
                .execute(
                    "UPDATE accounts.account SET status = 'closed', email_normalized = NULL, \
                     email_display = NULL WHERE account_id = $1",
                    &[&account.as_bytes().as_slice()],
                )
                .await
        } else {
            self.admin
                .client
                .execute(
                    "UPDATE accounts.account SET status = $2 WHERE account_id = $1",
                    &[&account.as_bytes().as_slice(), &status],
                )
                .await
        };
        result.expect("set the account status");
    }
}

async fn scavenge(postgres: &Raw) {
    let rows = postgres
        .client
        .query(
            "SELECT datname FROM pg_database WHERE left(datname, char_length($1)) = $1",
            &[&database_prefix()],
        )
        .await
        .expect("list test databases");
    for row in rows {
        let name: String = row.get(0);
        let created: Option<u64> = name
            .trim_start_matches(database_prefix())
            .split('_')
            .next()
            .and_then(|text| text.parse().ok());
        if created.is_some_and(|created| unix_seconds().saturating_sub(created) >= stale_after()) {
            let _ = postgres
                .client
                .batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
                .await;
        }
    }
    // Login roles of dropped tests: their names carry the creation time too.
    let roles = postgres
        .client
        .query(
            "SELECT rolname FROM pg_roles WHERE rolname LIKE 'df_s1%'",
            &[],
        )
        .await
        .expect("list test roles");
    for row in roles {
        let name: String = row.get(0);
        let created: Option<u64> = name.split('_').nth(2).and_then(|text| text.parse().ok());
        if created.is_some_and(|created| unix_seconds().saturating_sub(created) >= stale_after()) {
            let _ = postgres
                .client
                .batch_execute(&format!("DROP ROLE IF EXISTS {name}"))
                .await;
        }
    }
}

pub fn account_id(number: u8) -> AccountId {
    let mut bytes = [0_u8; 16];
    bytes[0] = 0xA0;
    bytes[1] = number;
    bytes[15] = 1;
    AccountId::from_bytes(&bytes).expect("a nonzero id")
}

pub fn tenant_id() -> TenantId {
    TenantId::from_bytes(&[0xB0; 16]).expect("a nonzero id")
}

pub fn session_id(number: u8) -> SessionId {
    let mut bytes = [0_u8; 16];
    bytes[0] = 0xC0;
    bytes[1] = number;
    bytes[15] = 1;
    SessionId::from_bytes(&bytes).expect("a nonzero id")
}

/// A fresh operation id issued at the fixture's start time, in the layout B's form ids use.
pub fn operation_id() -> OperationId {
    df_campaign::ids::new_operation_id(START_MS).expect("entropy")
}

/// A fresh invitation token's stored digest.
pub fn token_digest() -> [u8; 32] {
    SecretToken::generate()
        .expect("entropy")
        .digest(INVITATION_TOKEN_TAG)
}

pub fn digest_of(text: &str) -> [u8; 32] {
    tagged_digest("df-campaign test v1", text.as_bytes())
}

/// The store, its fixture and a clock the test moves by hand.
pub struct World {
    pub db: TestDb,
    pub store: PgCampaignStore,
    pub clock: FakeClock,
    pub role: String,
}

pub const START_MS: u64 = 1_800_000_000_000;

impl World {
    pub async fn new(label: &str) -> Self {
        Self::with_tunables(label, Tunables::default()).await
    }

    pub async fn with_tunables(label: &str, tunables: Tunables) -> Self {
        let mut db = TestDb::create(label).await;
        let role = db.login_role(Some("10s")).await;
        let store = PgCampaignStore::open(
            db.connector(&role),
            db.connector(&role),
            4,
            tunables,
            TimeoutPolicy::Enforced,
        )
        .await
        .expect("the store opens under the production role defaults");
        Self {
            db,
            store,
            clock: FakeClock::at(START_MS),
            role,
        }
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    pub async fn account(&self, number: u8) -> AccountId {
        self.db.seed_account(number).await
    }

    /// A campaign owned and hosted by `owner`, with `seats` seats.
    pub async fn campaign(&self, owner: AccountId, seats: u32) -> CampaignRow {
        self.store
            .create_campaign(
                NewCampaign {
                    tenant: tenant_id(),
                    owner,
                    name: "The Sunken Bell".to_owned(),
                    seat_count: seats,
                    content_rating: ContentRating::Standard,
                    room_codes: RoomCodes::WithApproval,
                    default_co_host_delegations: Delegations::NONE,
                },
                operation_id(),
                digest_of("create"),
                self.now(),
            )
            .await
            .expect("the campaign is created")
    }

    /// Adds `account` through a real invitation created by `host`.
    pub async fn join(
        &self,
        campaign: CampaignId,
        host: AccountId,
        account: AccountId,
        role: JoinRole,
    ) -> MemberId {
        let digest = token_digest();
        self.store
            .create_invitation(
                host,
                campaign,
                NewInvitation {
                    joins_as: role,
                    lifetime_ms: 3_600_000,
                    max_uses: 10,
                    token_digest: digest,
                },
                self.now(),
            )
            .await
            .expect("the invitation is created");
        self.store
            .accept_invitation(account, digest, operation_id(), self.now())
            .await
            .expect("the invitation is accepted")
            .member
    }

    pub async fn events(&self, campaign: CampaignId) -> Vec<EventRow> {
        self.store
            .events_after(campaign, 0, 1_000)
            .await
            .expect("events")
    }

    /// The number of events written for a campaign given as hex (a draft has none).
    pub async fn events_of_draft(&self, draft: &str) -> i64 {
        let id = CampaignId::from_hex(draft).expect("a campaign id");
        self.db
            .admin
            .client
            .query_one(
                "SELECT count(*) FROM campaigns.campaign_event WHERE campaign_id = $1",
                &[&id.as_bytes().as_slice()],
            )
            .await
            .expect("event count")
            .get(0)
    }

    /// (status, has a sponsor, custom hourly figure or 0, the draft step is null) of a row.
    pub async fn draft_columns(&self, draft: &str) -> (String, bool, i64, bool) {
        let id = CampaignId::from_hex(draft).expect("a campaign id");
        let row = self
            .db
            .admin
            .client
            .query_one(
                "SELECT status, sponsor_account_id IS NOT NULL,                  COALESCE(custom_hourly_micro_dollars, 0), draft_step IS NULL                  FROM campaigns.campaign WHERE campaign_id = $1",
                &[&id.as_bytes().as_slice()],
            )
            .await
            .expect("campaign row");
        (row.get(0), row.get(1), row.get(2), row.get(3))
    }

    pub async fn draft_has_sponsor(&self, draft: &str) -> Option<bool> {
        let id = CampaignId::from_hex(draft).expect("a campaign id");
        self.db
            .admin
            .client
            .query_opt(
                "SELECT sponsor_account_id IS NOT NULL FROM campaigns.campaign                  WHERE campaign_id = $1",
                &[&id.as_bytes().as_slice()],
            )
            .await
            .expect("campaign row")
            .map(|row| row.get(0))
    }

    pub async fn feed_epoch(&self, campaign: CampaignId) -> FeedEpoch {
        self.store.campaign_heads(&[campaign]).await.expect("heads")[0].feed_epoch
    }

    pub async fn cleanup(self) {
        let Self { db, store, .. } = self;
        drop(store);
        db.cleanup().await;
    }
}

impl World {
    /// Waits until `count` backends of this database are blocked on a lock, asserted through
    /// `pg_stat_activity`. It never sleeps: every probe is a database round trip, and the other
    /// tasks run between probes. It fails if that never happens.
    pub async fn wait_for_lock_waiters(&self, count: i64) {
        for _ in 0..20_000 {
            let row = self
                .db
                .admin
                .client
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = $1 AND wait_event_type = 'Lock'",
                    &[&self.db.name],
                )
                .await
                .expect("pg_stat_activity");
            let waiting: i64 = row.get(0);
            if waiting >= count {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("{count} backends never blocked on a lock");
    }

    /// The database's own time in milliseconds.
    pub async fn db_now_ms(&self) -> u64 {
        self.store.database_time_ms().await.expect("database time")
    }
}

impl TestDb {
    /// A login role standing in for B's connection: a member of `df_accounts` with the 10 s
    /// `transaction_timeout` default B's migration gives that role. The closure methods run on
    /// this role inside B's transaction.
    pub async fn accounts_login_role(&mut self, timeout: Option<&str>) -> String {
        let role = format!(
            "df_s1b_{}_{}",
            unix_seconds(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        self.admin
            .client
            .batch_execute(&format!("CREATE ROLE {role} LOGIN IN ROLE df_accounts"))
            .await
            .expect("create the accounts login role");
        if let Some(timeout) = timeout {
            self.admin
                .client
                .batch_execute(&format!(
                    "ALTER ROLE {role} SET transaction_timeout = '{timeout}'"
                ))
                .await
                .expect("set the role default");
        }
        self.roles.push(role.clone());
        role
    }
}

impl World {
    /// Waits until a backend of this database is queued behind another on a row's tuple lock (the
    /// second waiter for one row: the first holds the tuple lock and waits for the transaction).
    /// Like the other waits it is a round trip, never a sleep.
    pub async fn wait_for_tuple_queue(&self, count: i64) {
        for _ in 0..20_000 {
            let row = self
                .db
                .admin
                .client
                .query_one(
                    "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
                     WHERE a.datname = $1 AND l.locktype = 'tuple' AND NOT l.granted",
                    &[&self.db.name],
                )
                .await
                .expect("pg_locks");
            let queued: i64 = row.get(0);
            if queued >= count {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("{count} backends never queued on a tuple lock");
    }
}
