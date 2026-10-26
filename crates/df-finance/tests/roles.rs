#![cfg(not(target_arch = "wasm32"))]

//! Roles and ownership (planning/finance-ledger-research.md 17.9): a real login that is only a
//! member of `df_finance` can use the flows and nothing else, the registries are immutable, the
//! replay role cannot log in until S2 gives it a credential, and a later migration's objects start
//! with nothing for PUBLIC.

mod support;

use df_finance::fakes::{FakeFence, FakeMedia, FakeMembers, SeededIds, SystemClock};
use df_finance::ports::{FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceService};
use support::sqlstate;
use support::world::{CURRENCY, World, operation, payer, purchase};
use tokio_postgres::{Config, NoTls};

#[test]
fn a_real_login_that_is_only_a_member_of_the_service_role_runs_the_flows_and_nothing_else() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        // S2 gives the provisioned service role its credential; the harness only turns the login on.
        let login = "df_finance_service".to_owned();
        world
            .admin
            .batch_execute(&format!("ALTER ROLE {login} LOGIN"))
            .await
            .expect("enable the login");
        let mut config: Config = world.scratch.config();
        config.user(&login);
        let (client, connection) = config
            .connect(NoTls)
            .await
            .expect("log in as the service role");
        tokio::spawn(async move {
            let _ended = connection.await;
        });

        // The role's own defaults arrive with the login, with no SET ROLE from a superuser.
        let timeouts = client
            .query_one(
                "SELECT current_setting('lock_timeout'), current_setting('statement_timeout'), \
                        current_setting('idle_in_transaction_session_timeout')",
                &[],
            )
            .await
            .expect("timeouts");
        assert_eq!(
            (
                timeouts.get::<_, String>(0),
                timeouts.get::<_, String>(1),
                timeouts.get::<_, String>(2)
            ),
            ("2s".to_owned(), "10s".to_owned(), "15s".to_owned())
        );
        let superuser: bool = client
            .query_one(
                "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
                &[],
            )
            .await
            .expect("role")
            .get(0);
        assert!(!superuser);

        // It cannot take another role, write a table, create anything, or call a write definer.
        for sql in [
            "SET ROLE df_finance_definer",
            "SET ROLE postgres",
            "SET SESSION AUTHORIZATION postgres",
            "INSERT INTO finance.journal_entry DEFAULT VALUES",
            "UPDATE finance.funding_lot SET revision = revision",
            "DELETE FROM finance.event_marker",
            "TRUNCATE finance.journal_line",
            "CREATE TABLE finance.planted (x int)",
            "CREATE FUNCTION finance.planted() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "SELECT finance.apply_change('trial_granted', '{}'::jsonb)",
            "SELECT finance.apply_insert('payer_insert', '{}'::jsonb)",
            "SELECT finance.note_locks('payer', ARRAY['x'])",
            "ALTER TABLE finance.payer DISABLE TRIGGER ALL",
        ] {
            let error = client.batch_execute(sql).await.expect_err(sql);
            assert_eq!(
                sqlstate(&error).as_deref(),
                Some("42501"),
                "{sql}: {error:?}"
            );
        }
        // RESET ROLE and SET ROLE NONE give a login no more than it had.
        client
            .batch_execute("RESET ROLE")
            .await
            .expect("reset role");
        let after: String = client
            .query_one("SELECT current_user::text", &[])
            .await
            .expect("user")
            .get(0);
        assert_eq!(after, login);

        // The flows work over this very connection.
        let mut service = FinanceService::new(
            client,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            FakeMembers::new(world.payers.iter().copied()),
            Box::new(SystemClock),
            Box::new(SeededIds::new(99)),
            FinanceConfig::default(),
        );
        service
            .apply_purchase(purchase(payer(1), "pay_login", 5_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                2_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let committed = service
            .admit(&world.admit_command(1, 1000, 1000))
            .await
            .expect("admit");
        assert!(matches!(
            committed,
            df_finance::admit::AdmissionOutcome::Admitted { .. }
        ));
        assert!(
            service
                .grant_trial(payer(2), [2; 32], CURRENCY)
                .await
                .expect("trial")
                .is_some()
        );
        drop(service);
        world
            .admin
            .batch_execute(&format!("ALTER ROLE {login} NOLOGIN"))
            .await
            .expect("turn the login off again");
        world.finish().await;
    });
}

#[test]
fn the_registries_and_the_journal_keys_are_immutable_and_their_triggers_are_what_says_so() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        for (table, update, trigger) in [
            (
                "event_kind",
                "UPDATE finance.event_kind SET carries_entries = NOT carries_entries",
                "event_kind_append_only",
            ),
            (
                "change_kind",
                "UPDATE finance.change_kind SET columns = '{}'",
                "change_kind_append_only",
            ),
            (
                "accounting_time_zone_history",
                "UPDATE finance.accounting_time_zone_history SET time_zone = 'UTC'",
                "accounting_time_zone_history_append_only",
            ),
            (
                "account_key",
                "UPDATE finance.account_key SET currency = 'EUR'",
                "account_key_append_only",
            ),
            (
                "entry_reason",
                "UPDATE finance.entry_reason SET code = code || '_x'",
                "entry_reason_append_only",
            ),
        ] {
            let error = world.admin.batch_execute(update).await.expect_err(update);
            assert_eq!(
                sqlstate(&error).as_deref(),
                Some("23000"),
                "{table}: {error:?}"
            );
            // With the trigger off the same statement goes through (the write guard aside), so the
            // trigger is the guard.
            world
                .admin
                .batch_execute(&format!(
                    "ALTER TABLE finance.{table} DISABLE TRIGGER {trigger}"
                ))
                .await
                .expect("disable");
            let outcome = world.admin.batch_execute(update).await;
            world
                .admin
                .batch_execute(&format!(
                    "ALTER TABLE finance.{table} ENABLE TRIGGER {trigger}"
                ))
                .await
                .expect("enable");
            if table != "account_key" {
                // account_key is also behind the write guard, which is a different message.
                assert!(outcome.is_ok(), "{table}: {outcome:?}");
            }
        }
        world.finish().await;
    });
}

#[test]
fn the_replay_role_cannot_log_in_and_a_later_function_starts_with_nothing_for_public() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let login: bool = world
            .admin
            .query_one(
                "SELECT rolcanlogin FROM pg_roles WHERE rolname = 'df_finance_replay'",
                &[],
            )
            .await
            .expect("role")
            .get(0);
        assert!(
            !login,
            "S2 enables the login when it provisions the credential"
        );
        const EXPOSED: &str = "SELECT count(*) FROM pg_proc p WHERE p.pronamespace = 'finance'::regnamespace                           AND (p.proacl IS NULL OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a                                 WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))";
        assert_eq!(
            world
                .admin
                .query_one(EXPOSED, &[])
                .await
                .expect("scan")
                .get::<_, i64>(0),
            0,
            "no finance function is executable by PUBLIC"
        );
        // The scan finds the one a careless later migration would add.
        world
            .admin
            .batch_execute("CREATE FUNCTION finance.a_later_migrations_function() RETURNS int LANGUAGE sql AS 'SELECT 1'")
            .await
            .expect("create");
        assert_eq!(
            world
                .admin
                .query_one(EXPOSED, &[])
                .await
                .expect("scan")
                .get::<_, i64>(0),
            1
        );
        let settings: Vec<String> = world
            .admin
            .query_one(
                "SELECT rolconfig FROM pg_roles WHERE rolname = 'df_finance_service'",
                &[],
            )
            .await
            .expect("settings")
            .get(0);
        for expected in [
            "lock_timeout=2s",
            "statement_timeout=10s",
            "idle_in_transaction_session_timeout=15s",
        ] {
            assert!(
                settings.iter().any(|setting| setting == expected),
                "{expected} in {settings:?}"
            );
        }
        world.finish().await;
    });
}

#[test]
fn the_definer_holds_exactly_the_writes_the_registry_names() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let can = |privilege: &'static str, table: &'static str| {
            let admin = &world.admin;
            async move {
                admin
                    .query_one(
                        "SELECT has_table_privilege('df_finance_definer', $1, $2)",
                        &[&format!("finance.{table}"), &privilege],
                    )
                    .await
                    .expect("privilege")
                    .get::<_, bool>(0)
            }
        };
        // The journal is insert-only; the registries are never written; a mutable row updates.
        assert!(
            can("INSERT", "journal_entry").await
                && !can("UPDATE", "journal_entry").await
                && !can("DELETE", "journal_entry").await
        );
        assert!(can("INSERT", "journal_line").await && !can("UPDATE", "journal_line").await);
        assert!(
            !can("INSERT", "change_kind").await
                && !can("UPDATE", "event_kind").await
                && !can("INSERT", "ledger_account").await
        );
        assert!(
            !can("UPDATE", "funding_lot").await && !can("DELETE", "funding_lot").await,
            "a lot is updated by column, never as a whole table"
        );
        // The columns the definer may update are exactly the registry's, plus `revision`; the two
        // tables a flow only locks hold the lock-only grant on `revision` alone.
        let granted: Vec<(String, String)> = world
            .admin
            .query(
                "SELECT c.relname::text, a.attname::text FROM pg_class c                    JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = 'finance'                    JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped                   WHERE c.relkind IN ('r', 'p')                     AND has_column_privilege('df_finance_definer', c.oid, a.attnum, 'UPDATE')                     AND NOT has_table_privilege('df_finance_definer', c.oid, 'UPDATE')                   ORDER BY 1, 2",
                &[],
            )
            .await
            .expect("column grants")
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        let mut registry: Vec<(String, String)> = world
            .admin
            .query(
                "SELECT DISTINCT k.table_name::text, c FROM finance.change_kind k, unnest(k.updatable_cols || ARRAY['revision']) c                   WHERE k.op = 'update'                  UNION SELECT t, 'revision' FROM unnest(ARRAY['payer', 'processor_payment_state']) t                  ORDER BY 1, 2",
                &[],
            )
            .await
            .expect("registry columns")
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        registry.sort();
        assert_eq!(
            granted, registry,
            "the definer's column grants are the registry"
        );
        // `funding_lot.state` and `allocation.status` leave this list: the dispute flow
        // (FIN-05) and the withdrawal flow change them only through typed update kinds
        // named in the registry, and the grant == registry equality above still holds.
        for (table, column) in [
            ("funding_lot", "original_micro_dollars"),
            ("session_funding", "session_ceiling_micro_dollars"),
        ] {
            assert!(
                !granted.contains(&(table.to_owned(), column.to_owned())),
                "{table}.{column} is not updatable by the definer"
            );
        }
        assert!(can("UPDATE", "account_balance").await && !can("DELETE", "account_balance").await);
        assert!(!can("UPDATE", "account_key").await, "keys are immutable");
        assert!(
            !can("UPDATE", "price_version").await,
            "a published version is immutable"
        );
        world.finish().await;
    });
}
