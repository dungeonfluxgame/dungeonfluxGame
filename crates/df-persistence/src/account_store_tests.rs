//! The store contract on PostgreSQL, plus checks only a real database can make. Each test runs
//! in its own new database (`df_service_test_<time>_<random>`) on the owned scratch server named
//! by `DUNGEONFLUX_ACCOUNTS_PG_PORT` (host `DUNGEONFLUX_ACCOUNTS_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_ACCOUNTS_PG_USER`, default postgres), dropped at the end.

use df_customer::contract::{Fixture, Harness};
use tokio_postgres::{Config, NoTls};

use std::sync::Arc;

use super::PostgresAccountStore;
use crate::{
    account_closure::TimeoutPolicy,
    postgres_tls::{PostgresTransport, TransportRefusal},
};

/// Never these ports: the game's and the existing owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

fn admin_config() -> Option<Config> {
    let Ok(port) = std::env::var("DUNGEONFLUX_ACCOUNTS_PG_PORT") else {
        eprintln!("SKIPPED: DUNGEONFLUX_ACCOUNTS_PG_PORT must name an owned scratch PostgreSQL");
        return None;
    };
    let port: u16 = port.parse().expect("scratch port");
    assert!(!FORBIDDEN_PORTS.contains(&port));
    let mut config = Config::new();
    config
        .host(std::env::var("DUNGEONFLUX_ACCOUNTS_PG_HOST").unwrap_or("127.0.0.1".into()))
        .port(port)
        .user(std::env::var("DUNGEONFLUX_ACCOUNTS_PG_USER").unwrap_or("postgres".into()))
        .dbname("postgres");
    Some(config)
}

async fn admin(config: &Config) -> tokio_postgres::Client {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

pub(super) struct PostgresFixture {
    admin: Config,
    database: String,
    store: Option<PostgresAccountStore>,
}

impl PostgresFixture {
    /// The configuration of this fixture's database, as the scratch administrator.
    pub(super) fn database_config(&self) -> Config {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        config
    }

    /// An administrator connection to this fixture's database.
    pub(super) async fn admin_client(&self) -> tokio_postgres::Client {
        admin(&self.database_config()).await
    }
}

impl Fixture for PostgresFixture {
    type Store = PostgresAccountStore;

    fn store(&self) -> PostgresAccountStore {
        self.store.clone().expect("store open until finish")
    }

    async fn finish(mut self) {
        // Close the store's connections before dropping its database.
        drop(self.store.take());
        admin(&self.admin)
            .await
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop test database");
    }
}

/// A fresh migrated database, or `None` (SKIPPED) without a scratch server.
pub(super) async fn fresh_database() -> Option<PostgresFixture> {
    fresh_database_with(true).await
}

fn store_config_for_admin(config: &Config, database: &str) -> Config {
    let mut config = config.clone();
    config.dbname(database);
    config
}

/// `relax_provider_check` lets the fake provider's links in; the production schema refuses them.
pub(super) async fn fresh_database_with(relax_provider_check: bool) -> Option<PostgresFixture> {
    let config = admin_config()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // The clock alone can repeat between parallel tests; the counter cannot.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let database = format!("df_service_test_{nanos}_{}_{serial}", std::process::id());
    admin(&config)
        .await
        .batch_execute(&format!("CREATE DATABASE {database}"))
        .await
        .expect("create test database");
    let mut store_config = config.clone();
    store_config.dbname(&database);
    // The scratch administrator has no transaction_timeout default; the timeout policy is
    // tested on its own with the provisioned role.
    let store = PostgresAccountStore::open(
        store_config,
        PostgresTransport::PlaintextLoopbackOnly,
        4,
        TimeoutPolicy::NotChecked,
    )
    .await
    .expect("open account store")
    .with_closure_participant(Arc::new(super::closure_tests::TestCampaigns));
    store.test_batch(super::closure_tests::SCHEMA_SQL).await;
    if relax_provider_check {
        // Production allows one identity provider; tests that use the fake provider relax the
        // constraint in this scratch database only.
        admin(&store_config_for_admin(&config, &database))
            .await
            .batch_execute(
                "ALTER TABLE accounts.identity_link DROP CONSTRAINT identity_link_provider_check; \
                 ALTER TABLE accounts.identity_link ADD CONSTRAINT identity_link_provider_check \
                 CHECK (provider IN ('workos', 'fake'))",
            )
            .await
            .expect("relax the provider check in the scratch database");
    }
    Some(PostgresFixture {
        admin: config,
        database,
        store: Some(store),
    })
}

/// One test per contract scenario; each reports SKIPPED without a scratch server.
macro_rules! postgres_contract {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let Some(fixture) = fresh_database().await else {
                        return;
                    };
                    df_customer::contract::$scenario(fixture.store()).await;
                    fixture.finish().await;
                });
            }
        )+
    };
}

postgres_contract!(
    signup_verify_login_and_logout_work_end_to_end,
    a_double_submitted_signup_creates_one_account_and_one_message,
    signing_up_with_a_registered_address_looks_the_same_and_warns_its_owner,
    a_password_change_signs_out_other_devices_and_rotates_this_one,
    a_reset_link_works_once_within_fifteen_minutes_and_signs_out_everywhere,
    a_reset_for_an_unknown_address_sends_nothing,
    an_email_change_needs_both_addresses_to_confirm,
    the_old_address_can_cancel_an_email_change,
    signing_out_other_devices_keeps_this_one,
    one_account_cannot_revoke_or_see_another_accounts_sessions,
    closing_an_account_signs_out_and_logging_in_within_the_window_reopens_it,
    retention_deletes_a_closed_accounts_personal_data_after_the_window,
    an_export_holds_the_account_and_its_security_history_and_no_secrets,
    provider_signup_verify_login_and_logout_work_end_to_end,
    a_registered_address_signs_up_like_a_new_one_and_warns_its_owner_at_the_provider,
    a_wrong_password_and_an_unknown_address_look_alike_at_the_provider,
    a_provider_that_requires_its_own_verification_is_a_refused_sign_in_like_any_other,
    a_provider_that_does_not_require_verification_signs_an_unverified_person_in,
    a_provider_reset_works_once_and_signs_out_everywhere,
    a_provider_reset_for_an_unknown_address_sends_nothing,
    a_leftover_provider_user_is_replaced_and_a_verified_one_is_not,
    a_provider_outage_is_unavailable_and_a_lost_sign_up_answer_is_unknown,
    the_provider_checks_the_password_for_sensitive_actions_and_closing_deletes_its_user,
    password_and_email_changes_are_not_offered_while_the_provider_holds_them,
    an_operator_lookup_finds_an_account_by_exact_address_or_id_only,
    verifying_replaces_the_sign_up_password_issues_a_session_and_revokes_earlier_sessions,
    verifying_at_the_provider_replaces_its_password_and_a_refused_password_leaves_the_link,
    a_login_after_the_closing_window_does_not_reopen_the_account,
    closing_needs_a_fresh_proof_within_five_minutes,
    two_sign_ins_for_an_address_run_at_once_and_a_third_waits,
    a_pass_closes_out_a_bounded_batch_and_the_next_pass_continues,
    a_provider_failure_keeps_one_account_and_the_pass_goes_on,
    provider_sign_up_takes_the_same_time_for_a_new_and_a_registered_address,
    local_passwords_are_refused_on_a_public_https_origin_unless_a_dev_flag_says_otherwise,
    a_second_provider_user_with_the_same_address_cannot_take_over_a_linked_account,
    a_slow_provider_reset_does_not_hold_up_the_mail_behind_it,
);

#[test]
fn stored_sessions_and_tokens_hold_digests_never_the_raw_secret() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some(fixture) = fresh_database().await else {
            return;
        };
        let harness = Harness::start(fixture.store()).await;
        let (_caller, cookie) = harness.verified_account("robin@example.test").await;
        let raw: Vec<u8> = (0..cookie.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&cookie[index..index + 2], 16).unwrap())
            .collect();
        let mut config = admin_config().unwrap();
        config.dbname(&fixture.database);
        let client = admin(&config).await;
        let matching = client
            .query(
                "SELECT 1 FROM accounts.web_session WHERE token_digest = $1 OR csrf_secret = $1 \
                 UNION ALL SELECT 1 FROM accounts.account_token WHERE token_digest = $1",
                &[&raw],
            )
            .await
            .unwrap();
        assert!(matching.is_empty(), "a raw token was stored");
        let stored: i64 = client
            .query_one("SELECT count(*) FROM accounts.web_session", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(stored, 1);
        drop(client);
        drop(harness);
        fixture.finish().await;
    });
}

#[test]
fn opening_the_store_twice_applies_the_migration_once() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some(fixture) = fresh_database().await else {
            return;
        };
        let mut config = admin_config().unwrap();
        config.dbname(&fixture.database);
        let again = PostgresAccountStore::open(
            config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
            TimeoutPolicy::NotChecked,
        )
        .await;
        assert!(again.is_ok());
        drop(again);
        let client = admin(&config).await;
        let versions: i64 = client
            .query_one("SELECT count(*) FROM accounts.schema_migration", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(versions, 2, "each migration is recorded once");
        drop(client);
        fixture.finish().await;
    });
}

#[test]
fn signup_consent_is_persisted_with_the_account() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some(fixture) = fresh_database().await else {
            return;
        };
        let harness = Harness::start(fixture.store()).await;
        assert_eq!(
            harness.sign_up("consent@example.test").await,
            df_customer::SignupOutcome::CheckYourEmail
        );
        let client = fixture.admin_client().await;
        let row = client
            .query_one(
                "SELECT adult_attested_at, terms_version, terms_accepted_at \
                 FROM accounts.account WHERE email_normalized = $1",
                &[&"consent@example.test"],
            )
            .await
            .unwrap();
        let adult_at: i64 = row.get(0);
        let terms_version: String = row.get(1);
        let terms_at: i64 = row.get(2);
        assert_eq!(adult_at, df_customer::contract::START_MS as i64);
        assert_eq!(terms_version, "terms-2026-10");
        assert_eq!(terms_at, adult_at);
        drop(client);
        drop(harness);
        fixture.finish().await;
    });
}

#[test]
fn a_stalled_account_statement_is_cut_off_by_transaction_timeout() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some(fixture) = fresh_database().await else {
            return;
        };
        assert!(
            fixture
                .store()
                .stalled_statement_is_cut_off()
                .await
                .unwrap()
        );
        fixture.finish().await;
    });
}

#[test]
fn opening_the_store_without_tls_to_a_remote_host_is_refused_before_connecting() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut config = Config::new();
    config
        .host("db.example.test")
        .port(25060)
        .user("doadmin")
        .dbname("df_service");
    let opened = runtime.block_on(PostgresAccountStore::open(
        config,
        PostgresTransport::PlaintextLoopbackOnly,
        1,
        TimeoutPolicy::NotChecked,
    ));
    assert!(matches!(
        opened,
        Err(super::AccountStoreOpenError::Transport(
            TransportRefusal::PlaintextToRemoteHost
        ))
    ));
}
