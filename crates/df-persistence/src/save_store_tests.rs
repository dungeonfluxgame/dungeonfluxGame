//! PostgreSQL save-store tests against the scratch server (skipped without
//! `DUNGEONFLUX_RUNTIME_PG_PORT`, like the runtime suites).

use df_types::{CampaignId, OperationId, TenantId};

#[path = "../tests/runtime_support/mod.rs"]
mod runtime_support;

use runtime_support::{Table, as_runtime, serving_table_of};
use tokio_postgres::{Client, Config, NoTls};

use crate::save_store::{
    BranchId, CheckpointEnvelope, CreateSavePoint, PostgresSaveStore, SaveError, SaveLink,
    SaveLinkKind, SavePointId,
};

fn id<T>(byte: u8, make: impl Fn(&[u8]) -> T) -> T {
    make(&[byte; 16])
}

fn request(seed: u8) -> CreateSavePoint {
    CreateSavePoint {
        save_point_id: SavePointId::from_bytes(&[seed; 16]).expect("id"),
        tenant_id: id(0xa1, |b| TenantId::from_bytes(b).expect("tenant")),
        campaign_id: id(0xc1, |b| CampaignId::from_bytes(b).expect("campaign")),
        branch_id: BranchId::from_bytes(&[1; 16]).expect("branch"),
        parent_save_point_id: None,
        checkpoint: CheckpointEnvelope {
            bytes: vec![seed; 32],
            codec_version: 3,
            schema_version: 1,
            content_version: 1,
        },
        links: vec![
            SaveLink {
                kind: SaveLinkKind::FinanceAdmission,
                linked_id: [0xf1; 16],
            },
            SaveLink {
                kind: SaveLinkKind::PaidGeneration,
                linked_id: [0x61; 16],
            },
        ],
        operation_id: OperationId::from_bytes(&[seed; 16]).expect("op"),
        fingerprint: [seed; 32],
        result_bytes: b"original-result".to_vec(),
    }
}

fn run_save_test<F, Fut>(test: F)
where
    F: FnOnce(PostgresSaveStore, Client) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    run_save_test_with_scope_migration(test, true);
}

fn run_save_test_with_scope_migration<F, Fut>(test: F, apply_scope_migration: bool)
where
    F: FnOnce(PostgresSaveStore, Client) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let Some(port) = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
    else {
        assert!(std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none());
        eprintln!("SKIPPED: DUNGEONFLUX_RUNTIME_PG_PORT not set");
        return;
    };
    assert_eq!(port, 55911, "SAVESEC uses only its assigned scratch server");
    runtime_support::run(async move {
        let database = format!(
            "savesec_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname("postgres");
        let (admin, connection) = config.connect(NoTls).await.expect("admin");
        let admin_driver = tokio::spawn(connection);
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create db");
        // Roles are cluster-wide; share the hosted harness's migration lock.
        admin
            .execute("SELECT pg_advisory_lock($1)", &[&7_283_410_199_i64])
            .await
            .expect("migration lock");
        config.dbname(&database);
        let (client, connection) = config.connect(NoTls).await.expect("store connection");
        let store_driver = tokio::spawn(connection);
        let (operator, connection) = config.connect(NoTls).await.expect("operator connection");
        let operator_driver = tokio::spawn(connection);
        let outcome =
            tokio::spawn(async move {
                for migration in runtime_support::MIGRATIONS {
                    if !apply_scope_migration && migration.file == "0036_save_control_scope.sql" {
                        continue;
                    }
                    if let Some(schema) = migration.search_path {
                        client.batch_execute(&format!(
                        "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}; \
                         CREATE TABLE IF NOT EXISTS schema_migration (version integer PRIMARY KEY, \
                         applied_at timestamptz NOT NULL DEFAULT now())"
                    )).await.expect("migration search path");
                    }
                    client
                        .batch_execute(migration.sql)
                        .await
                        .expect(migration.file);
                    client
                        .batch_execute("RESET search_path")
                        .await
                        .expect("reset search path");
                }
                client
                    .batch_execute("INSERT INTO df_save.schema_migration (version) VALUES (28)")
                    .await
                    .expect("record fixture save migration");
                if apply_scope_migration {
                    client
                        .batch_execute(
                            "INSERT INTO df_save.schema_migration (version) VALUES (36)",
                        )
                        .await
                        .expect("record fixture control-scope migration");
                }
                let store = PostgresSaveStore::open(client)
                    .await
                    .expect("save migration");
                test(store, operator).await;
            })
            .await;
        admin
            .execute("SELECT pg_advisory_unlock($1)", &[&7_283_410_199_i64])
            .await
            .expect("unlock");
        store_driver.abort();
        operator_driver.abort();
        admin
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .expect("drop db");
        drop(admin);
        admin_driver.abort();
        if let Err(error) = outcome {
            std::panic::resume_unwind(error.into_panic());
        }
    });
}

async fn scope_store(store: &mut PostgresSaveStore, token: &str) {
    store
        .client_mut()
        .batch_execute("SET ROLE df_campaign_runtime")
        .await
        .expect("runtime role");
    store
        .client_mut()
        .query_one(
            "SELECT set_config('df.placement_token', $1, false)",
            &[&token],
        )
        .await
        .expect("placement token");
}

async fn place_request(client: &Client, request: &CreateSavePoint, tag: u8) -> String {
    let table = Table {
        tenant: request.tenant_id.as_bytes().to_vec(),
        campaign: request.campaign_id.as_bytes().to_vec(),
    };
    let (_, placement, _) = serving_table_of(client, &table, tag).await;
    placement.token
}

#[test]
fn postgres_save_point_commit_before_reply_and_branch_keep_links_forward_only() {
    run_save_test(|mut store, client| async move {
        let first = request(7);
        let token = place_request(&client, &first, 1).await;
        scope_store(&mut store, &token).await;
        let committed = store.create_save_point(&first).await.expect("commit");
        assert!(committed.committed_now);
        // Injected failure after commit, before reply: the caller lost `committed`.
        // The retry returns the stored result and writes no second save point.
        let retried = store.create_save_point(&first).await.expect("retry");
        assert!(!retried.committed_now);
        assert_eq!(retried.save_point_id, committed.save_point_id);
        assert_eq!(retried.result_bytes, b"original-result".to_vec());
        let mut conflict = first.clone();
        conflict.fingerprint = [9; 32];
        assert_eq!(
            store.create_save_point(&conflict).await,
            Err(crate::save_store::SaveError::OperationConflict)
        );

        // Branch/restore from the save: a new head, the same forward-only links.
        let mut branch = request(8);
        branch.save_point_id = SavePointId::from_bytes(&[8; 16]).expect("id");
        branch.branch_id = BranchId::from_bytes(&[2; 16]).expect("branch");
        branch.operation_id = OperationId::from_bytes(&[8; 16]).expect("op");
        let branched = store
            .branch_from(first.save_point_id, &branch)
            .await
            .expect("branch");
        assert!(branched.committed_now);
        assert_eq!(
            store
                .link_count(branched.save_point_id)
                .await
                .expect("links"),
            2
        );
        assert_eq!(
            store.link_count(first.save_point_id).await.expect("links"),
            2
        );

        client
            .batch_execute("UPDATE df_meta.service_state SET recovery_epoch = 7")
            .await
            .expect("current recovery epoch");
        let fence = store
            .claim_resume_fence(first.tenant_id, first.campaign_id, [4; 16])
            .await
            .expect("fence");
        let fence2 = store
            .claim_resume_fence(first.tenant_id, first.campaign_id, [4; 16])
            .await
            .expect("fence 2");
        assert!(fence2.claim_seq > fence.claim_seq);
        assert_eq!(fence.recovery_epoch, 7);
        assert_eq!(fence2.recovery_epoch, 7);
    });
}

#[test]
fn postgres_save_runtime_cannot_read_or_write_other_campaigns_or_tenants() {
    run_save_test(|mut store, client| async move {
        let first = request(1);
        let token = place_request(&client, &first, 1).await;
        store.create_save_point(&first).await.expect("own save");
        for other_tenant in [false, true] {
            let mut foreign = request(if other_tenant { 3 } else { 2 });
            if other_tenant {
                foreign.tenant_id = id(0xa2, |b| TenantId::from_bytes(b).expect("tenant"));
            } else {
                foreign.campaign_id = id(0xc2, |b| CampaignId::from_bytes(b).expect("campaign"));
            }
            store
                .create_save_point(&foreign)
                .await
                .expect("foreign save");
            store
                .claim_resume_fence(foreign.tenant_id, foreign.campaign_id, [2; 16])
                .await
                .expect("foreign fence");
            let tenant = foreign.tenant_id.as_bytes().as_slice();
            let campaign = foreign.campaign_id.as_bytes().as_slice();
            let new_id = [9_u8; 16].as_slice();
            let save = foreign.save_point_id.as_bytes().as_slice();
            for table in [
                "save_points",
                "branch_heads",
                "save_operations",
                "resume_fences",
            ] {
                let rows = as_runtime(&client, &token,
                    &format!("SELECT count(*) FROM df_save.{table} WHERE tenant_id = $1 AND campaign_id = $2"),
                    &[&tenant, &campaign]
                ).await.expect("foreign rows hidden");
                assert_eq!(
                    rows[0].get::<_, i64>(0),
                    0,
                    "{table}, other_tenant={other_tenant}"
                );
            }
            let rows = as_runtime(
                &client,
                &token,
                "SELECT count(*) FROM df_save.save_links WHERE save_point_id = $1",
                &[&save],
            )
            .await
            .expect("foreign links hidden");
            assert_eq!(rows[0].get::<_, i64>(0), 0);
            let error = as_runtime(
                &client, &token,
                "INSERT INTO df_save.save_links (save_point_id, link_kind, linked_id) VALUES ($1, 'paid_generation', $2)",
                &[&save, &new_id],
            ).await.expect_err("foreign link insert");
            assert_eq!(runtime_support::sqlstate(&error).as_deref(), Some("42501"));
            for sql in [
                "INSERT INTO df_save.save_points (save_point_id, tenant_id, campaign_id, branch_id, codec_version, schema_version, content_version, checkpoint) VALUES ($3,$1,$2,$3,1,1,1,'x')",
                "INSERT INTO df_save.resume_fences (tenant_id, campaign_id, recovery_epoch, claim_seq, placement_id) VALUES ($1,$2,1,99,$3)",
                "INSERT INTO df_save.branch_heads (tenant_id, campaign_id, branch_id, head_save_point_id) VALUES ($1,$2,$3,$4)",
                "INSERT INTO df_save.save_operations (tenant_id, campaign_id, operation_id, fingerprint, result_save_point_id, result_bytes) VALUES ($1,$2,$3,decode(repeat('11',32),'hex'),$4,'x')",
            ] {
                let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                    vec![&tenant, &campaign, &new_id];
                if sql.contains("$4") {
                    params.push(&save);
                }
                let error = as_runtime(&client, &token, sql, &params)
                    .await
                    .expect_err("foreign insert");
                assert_eq!(
                    runtime_support::sqlstate(&error).as_deref(),
                    Some("42501"),
                    "{sql}"
                );
            }
            for (sql, key) in [
                (
                    "UPDATE df_save.save_points SET checkpoint = 'changed' WHERE save_point_id = $1 RETURNING save_point_id",
                    save,
                ),
                (
                    "UPDATE df_save.resume_fences SET recovery_epoch = 9 WHERE tenant_id = $1 RETURNING placement_id",
                    tenant,
                ),
            ] {
                let result = as_runtime(&client, &token, sql, &[&key]).await;
                match result {
                    Ok(rows) => assert!(rows.is_empty(), "foreign update: {sql}"),
                    Err(error) => {
                        assert_eq!(runtime_support::sqlstate(&error).as_deref(), Some("42501"))
                    }
                }
            }
            let moved = as_runtime(&client, &token,
                "UPDATE df_save.branch_heads SET head_save_point_id = $3 WHERE tenant_id = $1 AND campaign_id = $2 RETURNING branch_id",
                &[&tenant, &campaign, &first.save_point_id.as_bytes().as_slice()]
            ).await.expect("foreign head hidden");
            assert!(moved.is_empty());
            let error = as_runtime(&client, &token,
                "UPDATE df_save.branch_heads SET head_save_point_id = $1 WHERE tenant_id = $2 AND campaign_id = $3",
                &[&save, &first.tenant_id.as_bytes().as_slice(), &first.campaign_id.as_bytes().as_slice()]
            ).await.expect_err("own head cannot reference a foreign save");
            assert_eq!(runtime_support::sqlstate(&error).as_deref(), Some("23503"));
        }
    });
}

#[test]
fn postgres_save_append_only_grants_and_every_table_force_rls() {
    run_save_test(|store, client| async move {
        drop(store);
        let tables = client
            .query(
                "SELECT relname, relrowsecurity, relforcerowsecurity FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'df_save' AND c.relkind = 'r'",
                &[],
            )
            .await
            .expect("catalog");
        assert_eq!(tables.len(), 6);
        for row in tables {
            assert!(
                row.get::<_, bool>(1) && row.get::<_, bool>(2),
                "{} lacks forced RLS",
                row.get::<_, String>(0)
            );
        }
        for role in ["df_campaign_runtime", "df_control"] {
            for table in [
                "save_points",
                "save_operations",
                "resume_fences",
                "save_links",
                "schema_migration",
            ] {
                for privilege in ["UPDATE", "DELETE", "TRUNCATE"] {
                    let row = client
                        .query_one(
                            "SELECT has_table_privilege($1, $2, $3)",
                            &[&role, &format!("df_save.{table}"), &privilege],
                        )
                        .await
                        .expect("privileges");
                    assert!(!row.get::<_, bool>(0), "{role} {privilege} {table}");
                }
            }
            for privilege in ["SELECT", "INSERT"] {
                let row = client
                    .query_one(
                        "SELECT has_table_privilege($1, 'df_save.schema_migration', $2)",
                        &[&role, &privilege],
                    )
                    .await
                    .expect("ledger privileges");
                assert!(!row.get::<_, bool>(0), "{role} {privilege} ledger");
            }
        }
    });
}

#[test]
fn control_role_cannot_read_private_checkpoint_payload() {
    run_save_test_with_scope_migration(|mut store, client| async move {
        let first = request(1);
        let first_token = place_request(&client, &first, 1).await;
        scope_store(&mut store, &first_token).await;
        store.create_save_point(&first).await.expect("first save");

        store
            .client_mut()
            .batch_execute("RESET ROLE")
            .await
            .expect("reset first runtime role");
        let mut second = request(2);
        second.tenant_id = id(0xa2, |bytes| TenantId::from_bytes(bytes).expect("tenant"));
        second.campaign_id = id(0xc2, |bytes| CampaignId::from_bytes(bytes).expect("campaign"));
        let second_token = place_request(&client, &second, 2).await;
        scope_store(&mut store, &second_token).await;
        store.create_save_point(&second).await.expect("second save");

        let control = client;
        control
            .batch_execute("SET ROLE df_control")
            .await
            .expect("control role");
        let metadata = control
            .query(
                "SELECT save_point_id, tenant_id, campaign_id, branch_id, created_at \
                 FROM df_save.save_points",
                &[],
            )
            .await
            .expect("control metadata read");
        assert_eq!(metadata.len(), 2);

        let checkpoint = control
            .query("SELECT checkpoint FROM df_save.save_points", &[])
            .await
            .expect_err("control role must not read any checkpoint payload");
        assert_eq!(runtime_support::sqlstate(&checkpoint).as_deref(), Some("42501"));
    }, false);
}

#[test]
fn postgres_save_missing_or_inaccessible_epoch_refuses_without_a_fence() {
    run_save_test(|mut store, client| async move {
        let first = request(1);
        // Superuser can insert fences, so RLS cannot conceal a fabricated epoch here.
        client
            .batch_execute("DELETE FROM df_meta.service_state")
            .await
            .expect("remove state");
        assert_eq!(
            store
                .claim_resume_fence(first.tenant_id, first.campaign_id, [1; 16])
                .await,
            Err(SaveError::Unavailable)
        );
        let row = client
            .query_one("SELECT count(*) FROM df_save.resume_fences", &[])
            .await
            .expect("fence count");
        assert_eq!(row.get::<_, i64>(0), 0);
        client
            .batch_execute("DROP FUNCTION df_runtime.current_recovery_epoch()")
            .await
            .expect("remove epoch reader");
        assert_eq!(
            store
                .claim_resume_fence(first.tenant_id, first.campaign_id, [1; 16])
                .await,
            Err(SaveError::Unavailable)
        );
        let row = client
            .query_one("SELECT count(*) FROM df_save.resume_fences", &[])
            .await
            .expect("fence count");
        assert_eq!(row.get::<_, i64>(0), 0);
    });
}

#[test]
fn postgres_save_foreign_parent_is_refused_by_writer_and_database() {
    run_save_test(|mut store, client| async move {
        let first = request(1);
        store.create_save_point(&first).await.expect("source save");
        for other_tenant in [false, true] {
            let mut branch = request(if other_tenant { 3 } else { 2 });
            if other_tenant {
                branch.tenant_id = id(0xa2, |b| TenantId::from_bytes(b).expect("tenant"));
            } else {
                branch.campaign_id = id(0xc2, |b| CampaignId::from_bytes(b).expect("campaign"));
            }
            branch.parent_save_point_id = Some(first.save_point_id);
            assert_eq!(
                store.create_save_point(&branch).await,
                Err(SaveError::SavePointNotFound)
            );
            assert_eq!(
                store.branch_from(first.save_point_id, &branch).await,
                Err(SaveError::SavePointNotFound)
            );
            let error = client.execute(
                "INSERT INTO df_save.save_points (save_point_id, tenant_id, campaign_id, branch_id, \
                 parent_save_point_id, codec_version, schema_version, content_version, checkpoint) \
                 VALUES ($1,$2,$3,$4,$5,1,1,1,'x')",
                &[&branch.save_point_id.as_bytes().as_slice(), &branch.tenant_id.as_bytes().as_slice(),
                  &branch.campaign_id.as_bytes().as_slice(), &branch.branch_id.as_bytes().as_slice(),
                  &first.save_point_id.as_bytes().as_slice()]
            ).await.expect_err("database rejects foreign parent even as superuser");
            assert_eq!(runtime_support::sqlstate(&error).as_deref(), Some("23503"));
        }
        let row = client
            .query_one("SELECT count(*) FROM df_save.save_points", &[])
            .await
            .expect("save count");
        assert_eq!(row.get::<_, i64>(0), 1);
    });
}

#[test]
fn postgres_save_no_expired_or_superseded_token_sees_or_writes_rows() {
    run_save_test(|mut store, client| async move {
        let first = request(1);
        let token = place_request(&client, &first, 1).await;
        store.create_save_point(&first).await.expect("save");
        for invalid in ["", &"00".repeat(32)] {
            let rows = as_runtime(&client, invalid, "SELECT * FROM df_save.save_points", &[])
                .await
                .expect("hidden");
            assert!(rows.is_empty());
            scope_store(&mut store, invalid).await;
            assert_eq!(
                store
                    .claim_resume_fence(first.tenant_id, first.campaign_id, [1; 16])
                    .await,
                Err(SaveError::Unavailable)
            );
        }
        for update in [
            "UPDATE df_runtime.placement_secrets SET is_current = false",
            "UPDATE df_runtime.placement_secrets SET is_current = true, lease_until = clock_timestamp() - interval '1 second'",
        ] {
            client
                .batch_execute(update)
                .await
                .expect("invalidate lease");
            let rows = as_runtime(&client, &token, "SELECT * FROM df_save.save_points", &[])
                .await
                .expect("hidden");
            assert!(rows.is_empty());
            scope_store(&mut store, &token).await;
            assert_eq!(
                store
                    .claim_resume_fence(first.tenant_id, first.campaign_id, [1; 16])
                    .await,
                Err(SaveError::Unavailable)
            );
        }
    });
}

#[test]
fn postgres_save_managed_runtime_needs_no_migration_privileges() {
    run_save_test(|store, client| async move {
        drop(store);
        let first = request(1);
        let token = place_request(&client, &first, 1).await;
        client
            .batch_execute("SET ROLE df_campaign_runtime")
            .await
            .expect("restricted role");
        let mut store = PostgresSaveStore::from_migrated_client(client);
        scope_store(&mut store, &token).await;
        assert!(
            store
                .create_save_point(&first)
                .await
                .expect("restricted save")
                .committed_now
        );
        assert_eq!(
            store
                .client_mut()
                .query_one("SELECT version FROM df_save.schema_migration", &[])
                .await
                .expect_err("ledger remains private")
                .as_db_error()
                .expect("db error")
                .code()
                .code(),
            "42501"
        );
    });
}

#[test]
fn postgres_save_epoch_read_and_fence_insert_share_one_transaction() {
    run_save_test(|mut store, client| async move {
        client
            .batch_execute(
                "CREATE OR REPLACE FUNCTION df_runtime.current_recovery_epoch() RETURNS bigint \
             LANGUAGE plpgsql VOLATILE SECURITY DEFINER \
             SET search_path = pg_catalog, df_meta, pg_temp AS $$ BEGIN \
             PERFORM set_config('savesec.epoch_transaction', pg_current_xact_id()::text, true); \
             RETURN (SELECT recovery_epoch FROM df_meta.service_state); END $$; \
             CREATE FUNCTION df_save.check_epoch_transaction() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN \
             IF current_setting('savesec.epoch_transaction', true) IS DISTINCT FROM \
                pg_current_xact_id()::text THEN RAISE EXCEPTION 'epoch read outside transaction'; \
             END IF; RETURN NEW; END $$; \
             CREATE TRIGGER check_epoch_transaction BEFORE INSERT ON df_save.resume_fences \
             FOR EACH ROW EXECUTE FUNCTION df_save.check_epoch_transaction()",
            )
            .await
            .expect("transaction probe");
        let first = request(1);
        let token = place_request(&client, &first, 1).await;
        scope_store(&mut store, &token).await;
        assert_eq!(
            store
                .claim_resume_fence(first.tenant_id, first.campaign_id, [1; 16])
                .await
                .expect("atomic fence")
                .recovery_epoch,
            1
        );
    });
}
