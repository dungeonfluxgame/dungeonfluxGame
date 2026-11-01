#![cfg(not(target_arch = "wasm32"))]
//! Owned disposable PostgreSQL only; no fabricated account/campaign fixtures.

use df_persistence::imagery_migrations::{migrate_in, migrate_local_in};
use std::time::{SystemTime, UNIX_EPOCH};
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
    let (client, connection) = config.connect(NoTls).await.unwrap();
    let driver = tokio::spawn(async move {
        connection.await.unwrap();
    });
    client
        .batch_execute("SET statement_timeout='10s'; SET lock_timeout='5s'")
        .await
        .unwrap();
    Connection { client, driver }
}

struct Scratch {
    config: Config,
    admin: Connection,
    database: String,
}
impl Scratch {
    async fn open() -> Self {
        assert_eq!(
            std::env::var("DUNGEONFLUX_REQUIRE_SCRATCH_PG").as_deref(),
            Ok("1")
        );
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(55471)
            .user("postgres")
            .dbname("postgres");
        let admin = connect(&config).await;
        admin
            .client
            .query_one(
                "SELECT pg_advisory_lock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .unwrap();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!("df_img_migrations_{}_{unique}", std::process::id());
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        config.dbname(&database);
        Self {
            config,
            admin,
            database,
        }
    }
    async fn finish(self) {
        assert!(self.database.starts_with("df_img_migrations_"));
        self.admin
            .client
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.database))
            .await
            .unwrap();
        self.admin
            .client
            .query_one(
                "SELECT pg_advisory_unlock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .unwrap();
    }
}

async fn versions(client: &Client, sql: &str) -> Vec<i32> {
    client
        .query(sql, &[])
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

#[tokio::test]
async fn local_bootstrap_installs_real_prerequisites_once_and_restarts_without_fake_records() {
    let scratch = Scratch::open().await;
    let mut connection = connect(&scratch.config).await;
    let probe = connect(&scratch.config).await;
    let tx = connection.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    for lock in [
        "SELECT pg_try_advisory_xact_lock(7000021)",
        "SELECT pg_try_advisory_xact_lock(hashtext('df_imagery_migration'))",
        "SELECT pg_try_advisory_xact_lock(hashtext('df_imagery_execution'))",
    ] {
        assert!(
            !probe
                .client
                .query_one(lock, &[])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    tx.commit().await.unwrap();
    let ledger = connection
        .client
        .query(
            "SELECT version,applied_at::text FROM imagery.schema_migration ORDER BY version",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, i32>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    assert_eq!(
        ledger.iter().map(|row| row.0).collect::<Vec<_>>(),
        (38..=53).chain([55]).collect::<Vec<_>>()
    );
    assert_eq!(
        versions(
            &connection.client,
            "SELECT version FROM media.schema_migration ORDER BY version"
        )
        .await,
        vec![10, 11, 12, 13, 14]
    );
    assert_eq!(
        versions(
            &connection.client,
            "SELECT number FROM hosted.migration ORDER BY number"
        )
        .await,
        vec![20, 21, 22, 23, 24, 25]
    );
    let tables=connection.client.query_one(concat!("SELECT to_regclass('imagery.image_dispatch_usage') IS NOT NULL, ",
        "to_regclass('imagery.image_job_result') IS NOT NULL, to_regclass('finance.draft_image_consent') IS NOT NULL, ",
        "to_regnamespace('accounts') IS NULL, to_regnamespace('campaigns') IS NULL, to_regnamespace('df_game') IS NULL"),&[]).await.unwrap();
    for column in 0..6 {
        assert!(tables.get::<_, bool>(column));
    }
    for sql in [
        "SELECT count(*) FROM finance.payer",
        "SELECT count(*) FROM media.asset",
        "SELECT count(*) FROM imagery.image_job",
        "SELECT count(*) FROM imagery.image_local_guest",
    ] {
        assert_eq!(
            connection
                .client
                .query_one(sql, &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
    }
    drop(connection);
    let mut restarted = connect(&scratch.config).await;
    let tx = restarted.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    migrate_in(&tx).await.unwrap();
    tx.commit().await.unwrap();
    let after = restarted
        .client
        .query(
            "SELECT version,applied_at::text FROM imagery.schema_migration ORDER BY version",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, i32>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    assert_eq!(ledger, after);
    drop(probe);
    drop(restarted);
    scratch.finish().await;
}

#[tokio::test]
async fn standalone_install_without_composition_prerequisites_fails_and_rolls_back() {
    let scratch = Scratch::open().await;
    let mut connection = connect(&scratch.config).await;
    let tx = connection.client.transaction().await.unwrap();
    let error = migrate_in(&tx).await.unwrap_err();
    assert!(matches!(error.code(), Some(code) if
        *code == tokio_postgres::error::SqlState::UNDEFINED_TABLE
        || *code == tokio_postgres::error::SqlState::INVALID_SCHEMA_NAME));
    tx.rollback().await.unwrap();
    assert!(
        connection
            .client
            .query_one("SELECT to_regnamespace('imagery') IS NULL", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let tx = connection.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    tx.rollback().await.unwrap();
    for schema in ["imagery", "media", "finance", "hosted"] {
        assert!(
            connection
                .client
                .query_one("SELECT to_regnamespace($1) IS NULL", &[&schema])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    let tx = connection.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    tx.commit().await.unwrap();
    drop(connection);
    scratch.finish().await;
}

#[tokio::test]
async fn local_install_adopts_existing_hosted_media_and_finance_history_without_reapplying() {
    let scratch = Scratch::open().await;
    let mut connection = connect(&scratch.config).await;
    let tx = connection.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    tx.commit().await.unwrap();
    // Real catalog schemas remain; model the hosted owner's historical media ledger.
    connection.client.batch_execute("INSERT INTO hosted.migration(number) SELECT version FROM media.schema_migration; DELETE FROM media.schema_migration").await.unwrap();
    let tx = connection.client.transaction().await.unwrap();
    migrate_local_in(&tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        versions(
            &connection.client,
            "SELECT version FROM media.schema_migration ORDER BY version"
        )
        .await,
        vec![10, 11, 12, 13, 14]
    );
    assert_eq!(
        versions(
            &connection.client,
            "SELECT number FROM hosted.migration ORDER BY number"
        )
        .await,
        vec![10, 11, 12, 13, 14, 20, 21, 22, 23, 24, 25]
    );
    drop(connection);
    scratch.finish().await;
}

#[tokio::test]
async fn concurrent_shared_installers_adopt_actual_legacy_journal_and_execution_ledgers() {
    use df_persistence::PostgresImageJournal;
    use df_persistence::imagery_execution::{ImageExecutionOpenError, PostgresImageExecution};
    use df_persistence::postgres_tls::PostgresTransport;
    let scratch = Scratch::open().await;
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    assert!(matches!(
        PostgresImageExecution::open_existing(scratch.config.clone(), transport.clone(), 1).await,
        Err(ImageExecutionOpenError::MissingSchema)
    ));
    let journal = PostgresImageJournal::open(scratch.config.clone(), transport.clone(), 1)
        .await
        .unwrap();
    let execution = PostgresImageExecution::open(scratch.config.clone(), transport.clone(), 1)
        .await
        .unwrap();
    let mut first = connect(&scratch.config).await;
    let mut second = connect(&scratch.config).await;
    assert_eq!(
        versions(
            &first.client,
            "SELECT version FROM imagery.schema_migration ORDER BY version"
        )
        .await,
        vec![38, 40, 45, 49]
    );
    let original = first
        .client
        .query(
            "SELECT version,applied_at::text FROM imagery.schema_migration ORDER BY version",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, i32>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    let install_first = async {
        let tx = first.client.transaction().await?;
        migrate_local_in(&tx).await?;
        tx.commit().await
    };
    let install_second = async {
        let tx = second.client.transaction().await?;
        migrate_local_in(&tx).await?;
        tx.commit().await
    };
    let (first_result, second_result) = tokio::join!(install_first, install_second);
    first_result.unwrap();
    second_result.unwrap();
    assert_eq!(
        versions(
            &first.client,
            "SELECT version FROM imagery.schema_migration ORDER BY version"
        )
        .await,
        (38..=53).chain([55]).collect::<Vec<_>>()
    );
    let retained=first.client.query("SELECT version,applied_at::text FROM imagery.schema_migration WHERE version IN(38,40,45,49) ORDER BY version",&[]).await.unwrap()
        .iter().map(|row|(row.get::<_,i32>(0),row.get::<_,String>(1))).collect::<Vec<_>>();
    assert_eq!(retained, original);
    let runtime = PostgresImageExecution::open_existing(scratch.config.clone(), transport, 1)
        .await
        .unwrap();
    drop(runtime);
    drop(journal);
    drop(execution);
    drop(first);
    drop(second);
    scratch.finish().await;
}
