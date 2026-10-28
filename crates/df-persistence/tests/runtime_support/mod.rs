//! Scratch-database harness for the hosted runtime tests. Each test gets a new database named
//! `df_runtime_test_<time>_<pid>_<serial>` on the owned scratch server named by
//! `DUNGEONFLUX_RUNTIME_PG_PORT` (host `DUNGEONFLUX_RUNTIME_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_RUNTIME_PG_USER`, default postgres) and drops only that database at the end.
//! Without the port a test reports SKIPPED, unless `DUNGEONFLUX_REQUIRE_SCRATCH_PG` is set (CI):
//! then a missing server fails the test. Tests connect as the server's superuser and act as a
//! service role with `SET LOCAL ROLE` inside a transaction, so definer functions, row security
//! and transaction-local tokens run for real and nothing leaks into the next call.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the harness"
)]

use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tokio_postgres::{Client, Config, NoTls, Row, types::ToSql};

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

/// One hosted migration and how the store that owns it applies it.
pub struct Migration {
    pub name: &'static str,
    pub file: &'static str,
    pub sql: &'static str,
    /// Schema the owner's store sets as `search_path` while applying it (B's accounts).
    pub search_path: Option<&'static str>,
}

macro_rules! migration {
    ($name:literal, $file:literal) => {
        Migration {
            name: $name,
            file: $file,
            sql: include_str!(concat!("../../migrations/", $file)),
            search_path: None,
        }
    };
    ($name:literal, $file:literal, in $schema:literal) => {
        Migration {
            name: $name,
            file: $file,
            sql: include_str!(concat!("../../migrations/", $file)),
            search_path: Some($schema),
        }
    };
}

/// The whole hosted migration set in order (0004 needs pgvector and 0002 is local-only), so the
/// catalog walk sees every owner's grants. `every_hosted_migration_file_is_in_the_harness`
/// fails when a file is added to the directory and not listed here.
pub const MIGRATIONS: [Migration; 33] = [
    migration!("0001", "0001_game_decisions.sql"),
    migration!("0003", "0003_story_memory.sql"),
    migration!("0005", "0005_accounts.sql", in "accounts"),
    migration!("0006", "0006_account_closure.sql", in "accounts"),
    migration!("0008", "0008_campaigns.sql"),
    migration!("0009", "0009_lobby.sql"),
    migration!("0010", "0010_media_catalog.sql"),
    migration!("0011", "0011_character_library.sql"),
    migration!("0012", "0012_media_exports.sql"),
    migration!("0013", "0013_recovery_journal.sql"),
    migration!("0014", "0014_media_sweep.sql"),
    migration!("0015", "0015_runtime_directory.sql"),
    migration!("0016", "0016_table_sessions.sql"),
    migration!("0018", "0018_runtime_roles.sql"),
    migration!("0019", "0019_story_memory_scope.sql"),
    migration!("0020", "0020_finance_journal.sql"),
    migration!("0021", "0021_finance_funding.sql"),
    migration!("0022", "0022_finance_admission.sql"),
    migration!("0023", "0023_finance_refunds_disputes.sql"),
    migration!("0024", "0024_finance_boundary.sql"),
    migration!("0025", "0025_finance_refunds_disputes_boundary.sql"),
    migration!("0027", "0027_support_inbox.sql"),
    migration!("0028", "0028_save_points.sql"),
    migration!("0029", "0029_google_oauth.sql", in "accounts"),
    migration!("0030", "0030_personal_tenants.sql", in "accounts"),
    migration!("0031", "0031_host_funding.sql"),
    migration!("0032", "0032_integration_tokens.sql", in "accounts"),
    migration!("0033", "0033_session_shares.sql"),
    migration!("0034", "0034_support_triage.sql"),
    migration!("0035", "0035_operator_access.sql"),
    migration!("0036", "0036_campaign_settings.sql"),
    migration!("0037", "0037_campaign_preferences_owner_boundary.sql"),
    migration!("0038", "0038_imagery_operations.sql"),
];

/// Migration files that are deliberately not applied to the hosted database.
pub const NOT_HOSTED: [&str; 3] = [
    "0002_local_room_reclaim.sql",
    "0004_story_memory_vector.sql",
    "0007_processor_records.sql",
];

/// Roles are cluster-wide and advisory locks are per database, so every test binary takes this
/// lock on the maintenance database while it applies the migrations.
const MIGRATION_LOCK_KEY: i64 = 7_283_410_199;

pub fn run<T>(test: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(test)
}

pub struct Scratch {
    admin: Config,
    database: String,
    logins: std::sync::Mutex<Vec<String>>,
}

impl Scratch {
    /// A new migrated database, or `None` (SKIPPED) without a scratch server.
    pub async fn open() -> Option<Self> {
        let scratch = Self::open_unmigrated().await?;
        let client = scratch.connect().await;
        // Held on the maintenance database until the migrations are applied.
        let guard = connect(&scratch.admin).await;
        guard
            .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
            .await
            .expect("migration lock");
        for migration in &MIGRATIONS {
            if let Some(schema) = migration.search_path {
                client
                    .batch_execute(&format!(
                        "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}; \n                         CREATE TABLE IF NOT EXISTS schema_migration (version integer PRIMARY KEY, \n                         applied_at timestamptz NOT NULL DEFAULT now())"
                    ))
                    .await
                    .expect("search_path");
            }
            apply(&client, migration).await;
            if migration.search_path.is_some() {
                client
                    .batch_execute("RESET search_path")
                    .await
                    .expect("reset search_path");
            }
        }
        guard
            .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
            .await
            .expect("release the migration lock");
        Some(scratch)
    }

    /// A new empty database, or `None` (SKIPPED) without a scratch server.
    pub async fn open_unmigrated() -> Option<Self> {
        let Ok(port) = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT") else {
            assert!(
                std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
                "DUNGEONFLUX_REQUIRE_SCRATCH_PG is set but DUNGEONFLUX_RUNTIME_PG_PORT is not"
            );
            eprintln!("SKIPPED: DUNGEONFLUX_RUNTIME_PG_PORT must name an owned scratch PostgreSQL");
            return None;
        };
        let port: u16 = port.parse().expect("scratch port");
        assert!(!FORBIDDEN_PORTS.contains(&port));
        let mut admin = Config::new();
        admin
            .host(std::env::var("DUNGEONFLUX_RUNTIME_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port)
            .user(std::env::var("DUNGEONFLUX_RUNTIME_PG_USER").unwrap_or("postgres".into()))
            .dbname("postgres");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let prefix = std::env::var("DUNGEONFLUX_RUNTIME_PG_PREFIX").unwrap_or_else(|_| "df".into());
        assert!(
            !prefix.is_empty()
                && prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "scratch database prefix must contain only ASCII letters, digits and underscores"
        );
        let database = format!(
            "{prefix}_runtime_test_{nanos}_{}_{serial}",
            std::process::id()
        );
        connect(&admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        Some(Self {
            admin,
            database,
            logins: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// A new connection to the test database as the server's superuser.
    pub async fn connect(&self) -> Client {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        connect(&config).await
    }

    pub async fn finish(self) {
        let admin = connect(&self.admin).await;
        admin
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop test database");
        let logins = std::mem::take(&mut *self.logins.lock().expect("logins"));
        if !logins.is_empty() {
            admin
                .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
                .await
                .expect("role lock");
        }
        for login in logins {
            admin
                .batch_execute(&format!("DROP ROLE IF EXISTS {login}"))
                .await
                .expect("drop the test login");
        }
    }

    /// A connection that LOGS IN as a new member of `role`, not a superuser session that
    /// switched role: the way a runtime really connects (a superuser session cannot be
    /// terminated by the definer). The login is dropped by `finish`.
    pub async fn login_as(&self, role: &str) -> Client {
        static NEXT_LOGIN: AtomicU64 = AtomicU64::new(0);
        let login = format!(
            "{role}_login_{}_{}",
            std::process::id(),
            NEXT_LOGIN.fetch_add(1, Ordering::Relaxed)
        );
        let creator = connect(&self.admin).await;
        creator
            .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
            .await
            .expect("role lock");
        creator
            .batch_execute(&format!("CREATE ROLE {login} LOGIN IN ROLE {role}"))
            .await
            .expect("create the test login");
        self.logins.lock().expect("logins").push(login.clone());
        let mut config = self.admin.clone();
        config.dbname(&self.database).user(&login);
        connect(&config).await
    }
}

async fn connect(config: &Config) -> Client {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

/// The SQLSTATE of a database error, or `None` for any other failure.
pub fn sqlstate(error: &tokio_postgres::Error) -> Option<String> {
    error.as_db_error().map(|db| db.code().code().to_string())
}

/// Runs one statement inside its own transaction after `setup` (transaction-local role and
/// token); commits on success, rolls back on error. Nothing set up survives the call.
async fn in_transaction(
    client: &Client,
    setup: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    if let Err(error) = client.batch_execute(&format!("BEGIN; {setup}")).await {
        let _ = client.batch_execute("ROLLBACK").await;
        return Err(error);
    }
    match client.query(sql, params).await {
        Ok(rows) => {
            client.batch_execute("COMMIT").await?;
            Ok(rows)
        }
        Err(error) => {
            let _rolled_back = client.batch_execute("ROLLBACK").await;
            Err(error)
        }
    }
}

/// Runs one statement as `role`.
pub async fn as_role(
    client: &Client,
    role: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    in_transaction(client, &format!("SET LOCAL ROLE {role}"), sql, params).await
}

/// Runs one statement as `df_campaign_runtime` presenting `token` as its placement token.
pub async fn as_runtime(
    client: &Client,
    token: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    in_transaction(client, &runtime_setup(token), sql, params).await
}

fn runtime_setup(token: &str) -> String {
    format!(
        "SET LOCAL ROLE df_campaign_runtime; SELECT set_config('df.placement_token', '{token}', true)"
    )
}

/// Runs one statement as `df_egress` presenting `token` as its dispatcher token.
pub async fn as_dispatcher(
    client: &Client,
    token: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    let setup = format!(
        "SET LOCAL ROLE df_egress; SELECT set_config('df.dispatcher_token', '{token}', true)"
    );
    in_transaction(client, &setup, sql, params).await
}

/// Runs one statement as `df_importer` presenting `token` as its import job token.
pub async fn as_importer(
    client: &Client,
    token: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    let setup = format!(
        "SET LOCAL ROLE df_importer; SELECT set_config('df.import_token', '{token}', true)"
    );
    in_transaction(client, &setup, sql, params).await
}

/// Opens an import job for the table's campaign, as the operator.
pub async fn open_import(client: &Client, table: &Table, token: &str, seconds: i32) {
    client
        .execute(
            "SELECT df_runtime.open_import($1, $2, $3, $4)",
            &[
                &table.tenant,
                &table.campaign,
                &digest_of_token(token),
                &seconds,
            ],
        )
        .await
        .expect("open import");
}

/// Opens a transaction as `role` (with the placement token when given) and leaves it open, for
/// tests that must hold row locks while another session runs.
pub async fn begin_as(client: &Client, role: &str, token: Option<&str>) {
    let setup = match token {
        Some(token) => format!(
            "SET LOCAL ROLE {role}; SELECT set_config('df.placement_token', '{token}', true)"
        ),
        None => format!("SET LOCAL ROLE {role}"),
    };
    client
        .batch_execute(&format!("BEGIN; {setup}"))
        .await
        .expect("open a held transaction");
}

pub async fn commit(client: &Client) {
    client.batch_execute("COMMIT").await.expect("commit");
}

pub async fn backend_pid(client: &Client) -> i32 {
    client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("pid")
        .get(0)
}

/// Waits (bounded) until the backend `pid` is waiting on a lock, so a test knows the second
/// session really queued behind the first.
pub async fn wait_until_blocked(observer: &Client, pid: i32) {
    let blocked = async {
        loop {
            let waiting: bool = observer
                .query_one(
                    "SELECT COALESCE((SELECT wait_event_type = 'Lock' FROM pg_stat_activity \
                     WHERE pid = $1), false)",
                    &[&pid],
                )
                .await
                .expect("pg_stat_activity")
                .get(0);
            if waiting {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), blocked)
        .await
        .expect("the second session must block on the first");
}

/// The two-int advisory key of the service lock, read by the superuser.
pub async fn service_key(client: &Client) -> (i32, i32) {
    let row = client
        .query_one(
            "SELECT lock_key_high, lock_key_low FROM df_meta.service_state",
            &[],
        )
        .await
        .expect("service key");
    (row.get(0), row.get(1))
}

/// A tenant and campaign pair with recognisable 16-byte ids.
#[derive(Clone)]
pub struct Table {
    pub tenant: Vec<u8>,
    pub campaign: Vec<u8>,
}

pub fn table_for(tag: u8) -> Table {
    Table {
        tenant: vec![0xa0 + tag; 16],
        campaign: vec![0xc0 + tag; 16],
    }
}

/// One placement: its id, its worker and the raw token its runtime presents.
#[derive(Clone)]
pub struct Placement {
    pub id: Vec<u8>,
    pub worker: Vec<u8>,
    pub token: String,
}

pub fn placement_for(tag: u8) -> Placement {
    Placement {
        id: vec![tag; 16],
        worker: vec![0x77; 16],
        token: format!("{tag:02x}").repeat(32),
    }
}

impl Placement {
    pub fn token_digest(&self) -> Vec<u8> {
        digest_of_token(&self.token)
    }
}

/// The SHA-256 of the 32 raw bytes a hex token stands for, as the database stores it.
pub fn digest_of_token(token: &str) -> Vec<u8> {
    let raw: Vec<u8> = (0..token.len() / 2)
        .map(|i| u8::from_str_radix(&token[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect();
    Sha256::digest(raw).to_vec()
}

pub async fn register_worker(client: &Client, worker: &[u8]) {
    as_role(
        client,
        "df_runner",
        "SELECT df_runtime.worker_heartbeat($1, 'test-worker', 8)",
        &[&worker],
    )
    .await
    .expect("worker heartbeat");
}

pub async fn request_placement(client: &Client, table: &Table) {
    as_role(
        client,
        "df_control",
        "SELECT df_runtime.request_placement($1, $2)",
        &[&table.tenant, &table.campaign],
    )
    .await
    .expect("request placement");
}

/// The fence a successful place or claim returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fence {
    pub recovery_epoch: i64,
    pub claim_seq: i64,
}

pub const PLACE_SQL: &str =
    "SELECT out_recovery_epoch, out_claim_seq FROM df_runtime.place($1, $2, $3, $4, $5, 60)";

pub async fn place(client: &Client, table: &Table, placement: &Placement) -> Option<Fence> {
    let rows = as_role(
        client,
        "df_runner",
        PLACE_SQL,
        &[
            &table.tenant,
            &table.campaign,
            &placement.id,
            &placement.worker,
            &placement.token_digest(),
        ],
    )
    .await
    .expect("place");
    rows.first().map(|row| Fence {
        recovery_epoch: row.get(0),
        claim_seq: row.get(1),
    })
}

pub struct Claimed {
    pub fence: Fence,
    pub money_state: String,
    pub reconstruction_id: Option<Vec<u8>>,
}

pub const CLAIM_SQL: &str = "SELECT out_recovery_epoch, out_claim_seq, out_money_state, \
     out_reconstruction_id FROM df_runtime.claim($1, $2, $3, 'https://runtime.internal:9000', $4, $5)";

pub async fn claim(client: &Client, table: &Table, placement: &Placement) -> Option<Claimed> {
    claim_with(client, table, placement, &placement.token, 30, &[0xee; 16]).await
}

pub async fn claim_with(
    client: &Client,
    table: &Table,
    placement: &Placement,
    token: &str,
    lease_seconds: i32,
    reconstruction_id: &[u8],
) -> Option<Claimed> {
    let rows = as_runtime(
        client,
        token,
        CLAIM_SQL,
        &[
            &table.tenant,
            &table.campaign,
            &placement.id,
            &lease_seconds,
            &reconstruction_id,
        ],
    )
    .await
    .expect("claim");
    rows.first().map(|row| Claimed {
        fence: Fence {
            recovery_epoch: row.get(0),
            claim_seq: row.get(1),
        },
        money_state: row.get(2),
        reconstruction_id: row.get(3),
    })
}

/// What `renew` returned.
pub struct Renewed {
    pub lease_until_epoch_s: f64,
    pub rolled: bool,
    pub roll_failed: bool,
    pub funding_required: bool,
}

pub const RENEW_SQL: &str = "SELECT extract(epoch FROM out_lease_until)::float8, out_rolled, \
     out_roll_failed, out_funding_required FROM df_runtime.renew($1, $2, $3, $4, $5, 30, $6, $7, 60)";

pub async fn renew(
    client: &Client,
    table: &Table,
    placement: &Placement,
    fence: Fence,
    progressed: bool,
    roll: bool,
) -> Option<Renewed> {
    let rows = as_runtime(
        client,
        &placement.token,
        RENEW_SQL,
        &[
            &table.tenant,
            &table.campaign,
            &fence.recovery_epoch,
            &fence.claim_seq,
            &placement.id,
            &progressed,
            &roll,
        ],
    )
    .await
    .expect("renew");
    rows.first().map(|row| Renewed {
        lease_until_epoch_s: row.get(0),
        rolled: row.get(1),
        roll_failed: row.get(2),
        funding_required: row.get(3),
    })
}

/// The directory row of a table, read by the superuser.
pub struct Owner {
    pub state: String,
    pub recovery_epoch: i64,
    pub claim_seq: i64,
    pub money_state: String,
    pub released_cleanly: bool,
    pub placement_id: Option<Vec<u8>>,
}

pub async fn owner_row(client: &Client, table: &Table) -> Owner {
    let row = client
        .query_one(
            "SELECT state, recovery_epoch, claim_seq, money_state, released_cleanly, placement_id \
             FROM df_runtime.campaign_owners WHERE tenant_id = $1 AND campaign_id = $2",
            &[&table.tenant, &table.campaign],
        )
        .await
        .expect("owner row");
    Owner {
        state: row.get(0),
        recovery_epoch: row.get(1),
        claim_seq: row.get(2),
        money_state: row.get(3),
        released_cleanly: row.get(4),
        placement_id: row.get(5),
    }
}

/// Makes the owner's lease and placing lease already over, as the passing of time would.
pub async fn expire_leases(client: &Client, table: &Table) {
    client
        .execute(
            "UPDATE df_runtime.campaign_owners SET lease_until = clock_timestamp() - interval '1 second', \
             placement_lease_until = CASE WHEN placement_lease_until IS NULL THEN NULL \
             ELSE clock_timestamp() - interval '1 second' END \
             WHERE tenant_id = $1 AND campaign_id = $2",
            &[&table.tenant, &table.campaign],
        )
        .await
        .expect("expire leases");
    client
        .execute(
            "UPDATE df_runtime.placement_secrets SET lease_until = clock_timestamp() - interval '1 second' \
             WHERE tenant_id = $1 AND campaign_id = $2",
            &[&table.tenant, &table.campaign],
        )
        .await
        .expect("expire placement secrets");
}

/// A placed and claimed table: the common start of most directory tests.
pub async fn serving_table(
    client: &Client,
    tag: u8,
    placement_tag: u8,
) -> (Table, Placement, Fence) {
    serving_table_of(client, &table_for(tag), placement_tag).await
}

pub async fn serving_table_of(
    client: &Client,
    table: &Table,
    placement_tag: u8,
) -> (Table, Placement, Fence) {
    let placement = placement_for(placement_tag);
    register_worker(client, &placement.worker).await;
    request_placement(client, table).await;
    place(client, table, &placement).await.expect("placed");
    let claimed = claim(client, table, &placement).await.expect("claimed");
    (table.clone(), placement, claimed.fence)
}

pub const SHORT: Duration = Duration::from_secs(3);

/// A game session row for the table, mapped to its campaign, written by the superuser.
pub async fn add_game_session(client: &Client, table: &Table, session: &[u8]) {
    client
        .execute(
            "INSERT INTO df_runtime.campaign_game_sessions (tenant_id, campaign_id, game_session_id) \
             VALUES ($1, $2, $3)",
            &[&table.tenant, &table.campaign, &session],
        )
        .await
        .expect("map game session");
    client
        .execute(
            "INSERT INTO df_game.sessions (tenant_id, session_id, run_id, recovery_epoch, in_epoch_sequence, \
             owner_fence, lease_until) VALUES ($1::bytea, $2::bytea, $3::bytea, 1, 0, $4::bytea, \
             clock_timestamp() + interval '1 hour')",
            &[&table.tenant, &session, &vec![0x01_u8; 16], &vec![0x02_u8; 16]],
        )
        .await
        .expect("game session");
}

/// Seeds one checkpoint, operation, fact and intent for a game session.
pub async fn add_game_rows(client: &Client, table: &Table, session: &[u8]) {
    let principal = vec![0x71_u8; 16];
    let namespace = vec![0x72_u8; 16];
    let operation = vec![0x73_u8; 16];
    let effect = vec![0x74_u8; 16];
    let run_id = vec![0x01_u8; 16];
    client
        .execute(
            "INSERT INTO df_game.checkpoints (tenant_id, session_id, recovery_epoch, \
             in_epoch_sequence, run_id, schema_version, codec_version, complete_envelope) \
             VALUES ($1::bytea, $2::bytea, 1, 1, $3::bytea, 1, 1, '\\x00')",
            &[&table.tenant, &session, &run_id],
        )
        .await
        .expect("checkpoint fixture");
    client
        .execute(
            "INSERT INTO df_game.operations (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, fingerprint_version, canonical_fingerprint, committed_epoch, \
             committed_sequence, receipt_version, receipt) VALUES ($1::bytea, $2::bytea, $3::bytea, \
             $4::bytea, 1, $5::bytea, 1, $6, 1, 1, 1, NULL)",
            &[
                &table.tenant,
                &session,
                &principal,
                &namespace,
                &operation,
                &vec![0x75_u8; 32],
            ],
        )
        .await
        .expect("operation fixture");
    client
        .execute(
            "INSERT INTO df_game.facts (tenant_id, session_id, committed_epoch, \
             committed_sequence, ordinal, fact_version, fact) VALUES ($1::bytea, $2::bytea, 1, 1, 0, 1, '\\x01')",
            &[&table.tenant, &session],
        )
        .await
        .expect("fact fixture");
    client
        .execute(
            "INSERT INTO df_game.intents (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, slot, effect_id, run_id, committed_epoch, committed_sequence, \
             process_generation, execution_mode, intent_kind, intent_version, intent) \
             VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, 1, $5::bytea, 0, $6::bytea, \
             $7::bytea, 1, 1, 1, 1, 1, 1, '\\x01')",
            &[
                &table.tenant,
                &session,
                &principal,
                &namespace,
                &operation,
                &effect,
                &run_id,
            ],
        )
        .await
        .expect("intent fixture");
}

/// Whether `fence` passes `fence_current` on `path` (`game` or `paid`), asked as `df_control`.
pub async fn fence_is_current(
    client: &Client,
    table: &Table,
    fence: Fence,
    placement: &[u8],
    path: &str,
) -> bool {
    as_role(
        client,
        "df_control",
        &format!("SELECT df_runtime.fence_current($1, $2, $3, $4, $5, '{path}')"),
        &[
            &table.tenant,
            &table.campaign,
            &fence.recovery_epoch,
            &fence.claim_seq,
            &placement,
        ],
    )
    .await
    .expect("fence_current")[0]
        .get(0)
}

/// The operator's restore step, as the superuser.
pub async fn begin_restore(client: &Client, epoch: i64) -> bool {
    client
        .query_one("SELECT df_runtime.begin_restore($1)", &[&epoch])
        .await
        .expect("begin restore")
        .get(0)
}

pub async fn restore_complete(
    client: &Client,
    role: &str,
    epoch: i64,
) -> Result<bool, tokio_postgres::Error> {
    as_role(
        client,
        role,
        "SELECT df_runtime.restore_complete($1)",
        &[&epoch],
    )
    .await
    .map(|rows| rows[0].get(0))
}

pub async fn service_state(client: &Client, role: &str) -> (String, i64) {
    let rows = as_role(
        client,
        role,
        "SELECT out_serving, out_recovery_epoch FROM df_runtime.service_state()",
        &[],
    )
    .await
    .expect("service_state");
    (rows[0].get(0), rows[0].get(1))
}

/// SQL that takes the service lock exclusively, as a halt does, for the rest of the transaction.
pub async fn halt_sql(client: &Client) -> String {
    let (high, low) = service_key(client).await;
    format!("SELECT pg_advisory_xact_lock({high}, {low})")
}

/// Applies one migration (one implicit transaction). Other agents' tests change roles on the
/// same scratch cluster at the same time, and PostgreSQL reports a clash on a role row as
/// "tuple concurrently updated": the migration was rolled back whole, so it is tried again.
async fn apply(client: &Client, migration: &Migration) {
    for attempt in 1..=5 {
        match client.batch_execute(migration.sql).await {
            Ok(()) => return,
            Err(error)
                if attempt < 5
                    && error
                        .as_db_error()
                        .is_some_and(|db| db.message().contains("concurrently updated")) =>
            {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(error) => panic!("migration {}: {error:?}", migration.name),
        }
    }
}
