//! The media catalog on PostgreSQL: the shared contract suite, then the races only a real database
//! can show, driven by controlled interleaving. A second connection runs the competing statement in
//! an owned task, the test waits until `pg_locks` shows that backend blocked (`granted = false`),
//! then releases the first transaction. There are no sleeps.
//!
//! Each test creates its own database `df_o2_media_<time>_<pid>_<serial>` on the scratch server
//! named by `DUNGEONFLUX_MEDIA_PG_PORT` (host `DUNGEONFLUX_MEDIA_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_MEDIA_PG_USER`, default postgres) and drops only that database at the end. Without
//! the variable every test reports SKIPPED.

use df_assets::catalog_contract as contract;
use std::sync::Arc;

use df_assets::{
    Admission, AdmitOutcome, Advance, AssetKind, AssetState, AssetVariant, Audience, CatalogConfig,
    CatalogPorts, CatalogService, CatalogStore, ChunkedBytes, ClaimOutcome, DedupeScope,
    DeletionMark, FakeConfig, FakeDispatcher, FakeFinance, FakeObjectStore, FakeValidator,
    FinalizeResult, GenerationAdmission, GenerationRequest, KeyLayout, ManualClock, NotAdmitted,
    ObjectEnvironment, QuotaScope, RefetchAnswer, RejectGuard, RejectReason, RetentionClass,
    SequentialIds, StagingOutcome, StorageDomain, TombstoneReason, TombstoneResult,
};
use df_types::CampaignId;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls};

use crate::media_catalog::PostgresMediaCatalog;
use crate::media_catalog_sql as sql;
use crate::postgres_tls::PostgresTransport;

/// Never these ports: the game's and the existing owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];
const MAXIMUM_POLLS: usize = 200_000;
const MAXIMUM_OPEN_FIXTURES: usize = 4;

fn admin_config() -> Option<Config> {
    let Ok(port) = std::env::var("DUNGEONFLUX_MEDIA_PG_PORT") else {
        eprintln!("SKIPPED: DUNGEONFLUX_MEDIA_PG_PORT must name an owned scratch PostgreSQL");
        return None;
    };
    let port: u16 = port.parse().expect("scratch port");
    assert!(!FORBIDDEN_PORTS.contains(&port));
    let mut config = Config::new();
    config
        .host(std::env::var("DUNGEONFLUX_MEDIA_PG_HOST").unwrap_or("127.0.0.1".into()))
        .port(port)
        .user(std::env::var("DUNGEONFLUX_MEDIA_PG_USER").unwrap_or("postgres".into()))
        .dbname("postgres");
    Some(config)
}

/// A raw connection whose driver is owned by the returned handle.
async fn raw_client(config: &Config) -> (Client, JoinHandle<()>) {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    let driver = tokio::spawn(async move {
        let _ended = connection.await;
    });
    (client, driver)
}

struct Fixture {
    admin_config: Config,
    config: Config,
    database: String,
    catalog: PostgresMediaCatalog,
    drivers: Vec<JoinHandle<()>>,
    _slot: tokio::sync::SemaphorePermit<'static>,
}

impl Fixture {
    async fn open() -> Option<Self> {
        let admin_config = admin_config()?;
        // The scratch server allows 100 connections; a few databases at a time keeps every test
        // well inside that however many run in parallel.
        static OPEN_FIXTURES: tokio::sync::Semaphore =
            tokio::sync::Semaphore::const_new(MAXIMUM_OPEN_FIXTURES);
        let slot = OPEN_FIXTURES.acquire().await.ok()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let database = format!("df_o2_media_{nanos}_{}_{serial}", std::process::id());
        let (admin, driver) = raw_client(&admin_config).await;
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        drop(admin);
        driver.abort();
        let mut config = admin_config.clone();
        config.dbname(&database);
        let catalog =
            PostgresMediaCatalog::open(config.clone(), PostgresTransport::PlaintextLoopbackOnly, 4)
                .await
                .expect("open media catalog");
        Some(Self {
            admin_config,
            config,
            database,
            catalog,
            drivers: Vec::new(),
            _slot: slot,
        })
    }

    /// A new connection to the test database, with its backend pid.
    async fn client(&mut self) -> (Client, i32) {
        let (client, driver) = raw_client(&self.config).await;
        self.drivers.push(driver);
        let pid: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        (client, pid)
    }

    /// Ends the test; the database is dropped by `Drop`, which also runs after a failed assertion.
    async fn finish(self) {}
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for driver in self.drivers.drain(..) {
            driver.abort();
        }
        let admin_config = self.admin_config.clone();
        let database = self.database.clone();
        // A thread of its own: this may run inside the test's runtime, or while unwinding.
        let cleaner = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return false;
            };
            runtime.block_on(async {
                let (admin, driver) = raw_client(&admin_config).await;
                let dropped = admin
                    .batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"))
                    .await;
                driver.abort();
                dropped.is_ok()
            })
        });
        if !cleaner.join().unwrap_or(false) {
            eprintln!("test database {} was not dropped", self.database);
        }
    }
}

/// Polls until the backend is waiting for a lock, then asserts which statement it is stuck in
/// (`statement_fragment`) and that `pg_locks` shows an ungranted lock for it.
async fn wait_until_blocked(observer: &Client, pid: i32, statement_fragment: &str) {
    for _ in 0..MAXIMUM_POLLS {
        let activity = observer
            .query_opt(
                "SELECT wait_event_type, query FROM pg_stat_activity WHERE pid = $1",
                &[&pid],
            )
            .await
            .unwrap();
        if let Some(activity) = activity {
            let wait_event_type: Option<String> = activity.get(0);
            let query: String = activity.get(1);
            if wait_event_type.as_deref() == Some("Lock") {
                assert!(
                    query.contains(statement_fragment),
                    "backend {pid} waits in {query:?}, expected {statement_fragment:?}"
                );
                let ungranted: i64 = observer
                    .query_one(
                        "SELECT count(*) FROM pg_locks WHERE pid = $1 AND NOT granted",
                        &[&pid],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert!(
                    ungranted > 0,
                    "backend {pid} waits but holds no pending lock"
                );
                return;
            }
        }
        tokio::task::yield_now().await;
    }
    panic!("backend {pid} never waited for a lock");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

macro_rules! postgres_contract {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                runtime().block_on(async {
                    let Some(fixture) = Fixture::open().await else {
                        return;
                    };
                    contract::$scenario(&fixture.catalog).await;
                    fixture.finish().await;
                });
            }
        )+
    };
}

postgres_contract!(
    admission_reserves_once_and_refuses_a_second_live_row_for_the_key,
    admission_beyond_the_limit_is_refused_with_the_room_left_and_clears_when_space_frees,
    uploads_are_limited_by_their_in_flight_count,
    a_step_claim_is_exclusive_until_its_lease_expires_and_each_claim_has_a_new_owner,
    staging_is_recorded_once_and_only_for_the_current_attempt,
    finalize_commits_the_quota_and_reports_ready_exactly_once,
    finalize_names_only_written_winners_and_a_rewrite_under_a_new_write_succeeds,
    a_worker_whose_lease_was_replaced_can_neither_write_nor_finalize,
    an_expired_lease_cannot_start_a_write_even_before_anyone_replaces_it,
    rejection_releases_the_reservation_and_reports_once_and_frees_the_key,
    a_replaced_attempt_refuses_the_old_attempts_late_work_and_reports_superseded,
    the_orphan_sweep_removes_an_older_claims_writes_and_never_a_winner,
    a_write_a_sweep_marked_deleting_makes_finalize_answer_winner_missing,
    a_write_finalize_named_as_a_winner_is_no_longer_deletable,
    a_tombstone_frees_the_bytes_and_a_purge_claim_leaves_the_winners_to_the_purge,
    due_rows_are_those_without_a_live_lease_oldest_first,
    a_quota_full_finalize_rejects_an_upload_instead_of_overcommitting,
    generated_output_commits_even_beyond_its_reservation_and_the_scope_limit,
    a_late_finalize_after_a_takeover_finalized_reports_lease_lost_with_one_outbox_row,
    a_rejection_guard_never_ends_newer_work,
    a_staged_object_outside_quarantine_is_refused,
    a_reservation_too_large_to_add_is_refused_and_never_wraps,
);

#[test]
fn opening_the_catalog_twice_applies_each_migration_once() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        PostgresMediaCatalog::open(
            fixture.config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .expect("second open");
        let (observer, driver) = raw_client(&fixture.config).await;
        let applied: i64 = observer
            .query_one("SELECT count(*) FROM media.schema_migration", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(applied, 5);
        driver.abort();
        fixture.finish().await;
    });
}

#[test]
fn the_database_itself_refuses_a_second_live_row_for_one_request_key() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        assert_eq!(
            fixture
                .catalog
                .admit(&contract::admission(1), 0)
                .await
                .unwrap(),
            AdmitOutcome::Admitted
        );
        let (client, driver) = raw_client(&fixture.config).await;
        let duplicate = client
            .execute(
                "INSERT INTO media.asset (asset_id, kind, provenance, charge_scope_kind, \
                 charge_scope_id, audience_kind, audience_campaign, audience_members, \
                 dedupe_scope, request_key, state, attempt, retention_class, created_at_ms, \
                 updated_at_ms) \
                 SELECT $1, kind, provenance, charge_scope_kind, charge_scope_id, audience_kind, \
                 audience_campaign, audience_members, dedupe_scope, request_key, state, $2, \
                 retention_class, 0, 0 \
                 FROM media.asset WHERE asset_id = $3",
                &[
                    &contract::asset_id(2).as_bytes().as_slice(),
                    &contract::attempt_id(2).as_bytes().as_slice(),
                    &contract::asset_id(1).as_bytes().as_slice(),
                ],
            )
            .await;
        let error = duplicate.expect_err("the unique index must refuse a second live row");
        assert_eq!(error.code().map(|code| code.code()), Some("23505"));
        let unfinished = client
            .execute(
                "UPDATE media.asset SET state = 'ready', ready_at_ms = 1 WHERE asset_id = $1",
                &[&contract::asset_id(1).as_bytes().as_slice()],
            )
            .await;
        assert!(
            unfinished.is_err(),
            "a ready row must name its winners and bytes"
        );
        driver.abort();
        fixture.finish().await;
    });
}

#[test]
fn a_live_reference_refuses_the_tombstone_and_leaves_the_quota_alone() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        contract::make_ready(&fixture.catalog, 1, 1, 10).await;
        let (client, driver) = raw_client(&fixture.config).await;
        client
            .execute(
                "INSERT INTO media.asset_reference (asset_id, holder_kind, holder_id, \
                 presentation, created_at_ms) VALUES ($1, 'save_point', $2, 'live', 0)",
                &[
                    &contract::asset_id(1).as_bytes().as_slice(),
                    &[9_u8; 16].as_slice(),
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            fixture
                .catalog
                .tombstone(contract::asset_id(1), TombstoneReason::UserDelete, 20)
                .await
                .unwrap(),
            TombstoneResult::LiveReferences
        );
        let usage = fixture
            .catalog
            .quota_usage(contract::scope())
            .await
            .unwrap();
        assert_eq!(usage.committed_bytes, 60);
        client
            .execute(
                "UPDATE media.asset_reference SET presentation = 'placeholder'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            fixture
                .catalog
                .tombstone(contract::asset_id(1), TombstoneReason::UserDelete, 21)
                .await
                .unwrap(),
            TombstoneResult::Deleted { released_bytes: 60 }
        );
        driver.abort();
        fixture.finish().await;
    });
}

#[test]
fn an_upload_ticket_is_consumed_once_for_its_account_campaign_purpose_key_and_digest() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        use df_assets::{ObjectEnvironment, UploadId, UploadPurpose, UploadTicket, WriteId};
        use sha2::{Digest, Sha256};

        let account = df_types::AccountId::from_bytes(&[31; 16]).unwrap();
        let campaign = contract::campaign();
        let mut admission = contract::admission(31);
        admission.provenance = df_assets::Provenance::Uploaded;
        admission.origin_account = Some(account);
        assert_eq!(
            fixture.catalog.admit(&admission, 10).await.unwrap(),
            AdmitOutcome::Admitted
        );
        let upload = UploadId::from_bytes(&[32; 16]).unwrap();
        let write = WriteId::from_bytes(&[33; 16]).unwrap();
        let layout = KeyLayout::new(ObjectEnvironment::new("contract").unwrap());
        let key = layout.quarantine_source(upload, write);
        let bytes = b"portrait upload";
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let (client, driver) = raw_client(&fixture.config).await;
        client
            .execute(
                "INSERT INTO media.upload (upload_id, asset_id, account_id, purpose, \
                 declared_mime, declared_bytes, declared_sha256, attempt, object_key, \
                 ticket_expires_ms, attests_rights, depicts_real_person) \
                 VALUES ($1, $2, $3, 'portrait', 'image/png', $4, $5, $6, $7, 100, true, false)",
                &[
                    &upload.as_bytes().as_slice(),
                    &admission.asset.as_bytes().as_slice(),
                    &account.as_bytes().as_slice(),
                    &(bytes.len() as i64),
                    &digest.as_slice(),
                    &admission.attempt.as_bytes().as_slice(),
                    &key.path(),
                ],
            )
            .await
            .unwrap();
        let ticket = UploadTicket {
            upload,
            asset: admission.asset,
            attempt: admission.attempt,
            account,
            campaign,
            purpose: UploadPurpose::Portrait,
            object_key: key.clone(),
            declared_bytes: bytes.len() as u64,
            expected_sha256: digest,
            issued_at_ms: 10,
            expires_at_ms: 100,
        };
        assert_eq!(
            fixture
                .catalog
                .claim_upload_ticket(&ticket, account, campaign, UploadPurpose::Portrait, 11,)
                .await
                .unwrap(),
            df_assets::UploadTicketClaim::Claimed
        );
        assert_eq!(
            fixture
                .catalog
                .claim_upload_ticket(&ticket, account, campaign, UploadPurpose::Portrait, 12,)
                .await,
            Ok(df_assets::UploadTicketClaim::Refused(
                df_assets::UploadTicketRejection::AlreadyUsed
            ))
        );
        drop(client);
        driver.abort();
        fixture.finish().await;
    });
}

#[test]
fn two_admissions_of_one_key_make_one_reservation_and_f_rolls_back_the_loser_cleanly() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let (mut first, _) = fixture.client().await;
        let (mut second, second_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        first
            .batch_execute("CREATE TABLE public.fake_provider_attempt (attempt bytea PRIMARY KEY)")
            .await
            .unwrap();
        let seed = first.transaction().await.unwrap();
        sql::lock_quota(&seed, contract::scope()).await.unwrap();
        seed.commit().await.unwrap();
        let first_tx = first.transaction().await.unwrap();
        first_tx
            .execute(
                "INSERT INTO public.fake_provider_attempt VALUES ($1)",
                &[&contract::attempt_id(1).as_bytes().as_slice()],
            )
            .await
            .unwrap();
        let outcome = sql::admit_in(&first_tx, &contract::admission(1), 0)
            .await
            .unwrap();
        assert_eq!(outcome, AdmitOutcome::Admitted);
        let competitor = tokio::spawn(async move {
            let tx = second.transaction().await.unwrap();
            tx.execute(
                "INSERT INTO public.fake_provider_attempt VALUES ($1)",
                &[&contract::attempt_id(2).as_bytes().as_slice()],
            )
            .await
            .unwrap();
            let mut rival = contract::admission(2);
            rival.request_key = Some(contract::request_key(1));
            let outcome = sql::admit_in(&tx, &rival, 1).await.unwrap();
            // F rolls back its whole transaction, the attempt row included, on NotAdmitted.
            tx.rollback().await.unwrap();
            outcome
        });
        wait_until_blocked(&observer, second_pid, "INSERT INTO media.quota_usage").await;
        first_tx.commit().await.unwrap();
        assert_eq!(
            competitor.await.unwrap(),
            AdmitOutcome::NotAdmitted(NotAdmitted::InFlight {
                attempt: contract::attempt_id(1)
            })
        );
        let attempts: i64 = observer
            .query_one("SELECT count(*) FROM public.fake_provider_attempt", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(attempts, 1);
        let usage = fixture
            .catalog
            .quota_usage(contract::scope())
            .await
            .unwrap();
        assert_eq!(usage.reserved_bytes, contract::RESERVE_BYTES);
        fixture.finish().await;
    });
}

#[test]
fn concurrent_first_reservations_of_a_new_scope_both_succeed_and_add_up() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let fresh = CampaignId::from_bytes(&[77; 16]).unwrap();
        let in_fresh_scope = |n: u8| Admission {
            charge_scope: QuotaScope::Campaign(fresh),
            campaign: Some(fresh),
            dedupe_scope: DedupeScope::campaign(&fresh),
            audience: Audience::CampaignShared(fresh),
            ..contract::admission(n)
        };
        let (mut first, _) = fixture.client().await;
        let (mut second, second_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        let first_tx = first.transaction().await.unwrap();
        assert_eq!(
            sql::admit_in(&first_tx, &in_fresh_scope(1), 0)
                .await
                .unwrap(),
            AdmitOutcome::Admitted
        );
        let rival = in_fresh_scope(2);
        let competitor = tokio::spawn(async move {
            let tx = second.transaction().await.unwrap();
            let outcome = sql::admit_in(&tx, &rival, 1).await.unwrap();
            tx.commit().await.unwrap();
            outcome
        });
        wait_until_blocked(&observer, second_pid, "INSERT INTO media.quota_usage").await;
        first_tx.commit().await.unwrap();
        assert_eq!(competitor.await.unwrap(), AdmitOutcome::Admitted);
        let usage = fixture
            .catalog
            .quota_usage(QuotaScope::Campaign(fresh))
            .await
            .unwrap();
        assert_eq!(usage.reserved_bytes, 2 * contract::RESERVE_BYTES);
        fixture.finish().await;
    });
}

/// Sets up asset 1 with an older claim's write (id 2) that a takeover (owner 2) may name.
async fn takeover_with_an_orphan_looking_write(catalog: &PostgresMediaCatalog) {
    contract::admit_and_stage(catalog, 1, 0).await;
    contract::validate_as(catalog, 1, 1, 0).await;
    contract::write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    let takeover = contract::claim(1, 1, 2, AssetState::Validated, 5_000);
    assert_eq!(
        catalog
            .claim_step(takeover, contract::LEASE_MS + 1)
            .await
            .unwrap(),
        ClaimOutcome::Claimed
    );
}

#[test]
fn a_sweep_waiting_on_finalize_leaves_the_winner_intact() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        takeover_with_an_orphan_looking_write(&fixture.catalog).await;
        let orphans = fixture.catalog.orphan_variants(i64::MAX, 10).await.unwrap();
        assert_eq!(orphans.len(), 1);
        let (mut finalizer, _) = fixture.client().await;
        let (mut sweeper, sweeper_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        let finalize_tx = finalizer.transaction().await.unwrap();
        let request = contract::finalize_request(&[(AssetVariant::Master, 2)]);
        let result = sql::finalize_in(
            &finalize_tx,
            contract::held(1, 1, 2),
            &request,
            contract::LEASE_MS + 2,
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            FinalizeResult::Ready {
                committed_bytes: 40
            }
        );
        let orphan = orphans[0].clone();
        let sweep = tokio::spawn(async move {
            let tx = sweeper.transaction().await.unwrap();
            let mark = sql::begin_variant_deletion_in(&tx, &orphan).await.unwrap();
            tx.commit().await.unwrap();
            mark
        });
        wait_until_blocked(&observer, sweeper_pid, "FOR SHARE").await;
        finalize_tx.commit().await.unwrap();
        assert_eq!(sweep.await.unwrap(), DeletionMark::NoLongerDeletable);
        let variants = fixture
            .catalog
            .variants(contract::asset_id(1))
            .await
            .unwrap();
        assert_eq!(variants.len(), 1);
        fixture.finish().await;
    });
}

#[test]
fn a_finalize_waiting_on_a_sweep_that_marked_first_answers_winner_missing() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        takeover_with_an_orphan_looking_write(&fixture.catalog).await;
        let orphans = fixture.catalog.orphan_variants(i64::MAX, 10).await.unwrap();
        let (mut sweeper, _) = fixture.client().await;
        let (mut finalizer, finalizer_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        let sweep_tx = sweeper.transaction().await.unwrap();
        assert_eq!(
            sql::begin_variant_deletion_in(&sweep_tx, &orphans[0])
                .await
                .unwrap(),
            DeletionMark::Marked
        );
        let finalize = tokio::spawn(async move {
            let tx = finalizer.transaction().await.unwrap();
            let request = contract::finalize_request(&[(AssetVariant::Master, 2)]);
            let result = sql::finalize_in(
                &tx,
                contract::held(1, 1, 2),
                &request,
                contract::LEASE_MS + 2,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            result
        });
        wait_until_blocked(
            &observer,
            finalizer_pid,
            "media.asset WHERE asset_id = $1 FOR UPDATE",
        )
        .await;
        sweep_tx.commit().await.unwrap();
        assert_eq!(
            finalize.await.unwrap(),
            FinalizeResult::WinnerMissing {
                variant: AssetVariant::Master
            }
        );
        fixture.finish().await;
    });
}

/// Each quota-changing transition locks the quota row before the asset row (D25, section 10): with
/// the quota row held by another transaction the operation waits, and the asset row is still free.
#[test]
fn every_quota_changing_transition_waits_on_the_quota_row_before_touching_the_asset_row() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        for operation in ["reject", "finalize", "tombstone", "supersede"] {
            let catalog = fixture.catalog.clone();
            let n: u8 = match operation {
                "reject" => 1,
                "finalize" => 2,
                "tombstone" => 3,
                _ => 4,
            };
            match operation {
                "reject" | "supersede" => contract::admit_and_stage(&catalog, n, 0).await,
                "finalize" => {
                    contract::admit_and_stage(&catalog, n, 0).await;
                    contract::validate_as(&catalog, n, 1, 0).await;
                    contract::write_variant(&catalog, n, 1, 20, AssetVariant::Master, 40, 1).await;
                }
                _ => contract::make_ready(&catalog, n, 3, 0).await,
            }
            let (mut holder, _) = fixture.client().await;
            let (mut worker, worker_pid) = fixture.client().await;
            let (mut probe, _) = fixture.client().await;
            let (observer, _) = fixture.client().await;
            let holder_tx = holder.transaction().await.unwrap();
            sql::lock_quota(&holder_tx, contract::scope())
                .await
                .unwrap();
            let task = tokio::spawn(async move {
                let tx = worker.transaction().await.unwrap();
                match operation {
                    "reject" => {
                        sql::reject_in(
                            &tx,
                            contract::asset_id(1),
                            contract::attempt_id(1),
                            RejectReason::NotDispatched,
                            RejectGuard::Current,
                            5,
                        )
                        .await
                        .unwrap();
                    }
                    "finalize" => {
                        let request = contract::finalize_request(&[(AssetVariant::Master, 20)]);
                        sql::finalize_in(&tx, contract::held(2, 2, 1), &request, 5)
                            .await
                            .unwrap();
                    }
                    "supersede" => {
                        sql::supersede_in(
                            &tx,
                            contract::asset_id(4),
                            contract::attempt_id(4),
                            contract::attempt_id(40),
                            5,
                        )
                        .await
                        .unwrap();
                    }
                    _ => {
                        sql::tombstone_in(
                            &tx,
                            contract::asset_id(3),
                            TombstoneReason::UserDelete,
                            5,
                        )
                        .await
                        .unwrap();
                    }
                }
                tx.commit().await.unwrap();
            });
            wait_until_blocked(&observer, worker_pid, "media.quota_usage").await;
            let probe_tx = probe.transaction().await.unwrap();
            let asset = contract::asset_id(n);
            probe_tx
                .query_one(
                    "SELECT 1 FROM media.asset WHERE asset_id = $1 FOR UPDATE NOWAIT",
                    &[&asset.as_bytes().as_slice()],
                )
                .await
                .unwrap_or_else(|error| {
                    panic!("{operation} locked the asset row before the quota row: {error}")
                });
            probe_tx.rollback().await.unwrap();
            holder_tx.commit().await.unwrap();
            task.await.unwrap();
        }
        fixture.finish().await;
    });
}

#[test]
fn the_service_drives_a_generated_picture_to_ready_on_postgres_and_a_second_pass_changes_nothing() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let store = Arc::new(FakeObjectStore::new(FakeConfig::default()));
        let service = CatalogService::new(
            CatalogPorts {
                catalog: Arc::new(fixture.catalog.clone()),
                objects: Arc::clone(&store),
                finance: Arc::new(FakeFinance::default()),
                dispatcher: Arc::new(FakeDispatcher::answering(RefetchAnswer::Unavailable)),
                validator: Arc::new(FakeValidator::accepting()),
                clock: Arc::new(ManualClock::new(1_000_000)),
                ids: Arc::new(SequentialIds::new()),
            },
            KeyLayout::new(ObjectEnvironment::new("pg").unwrap()),
            CatalogConfig::default(),
        );
        let campaign = contract::campaign();
        let request = GenerationRequest {
            attempt: contract::attempt_id(1),
            kind: AssetKind::Image,
            charge_scope: contract::scope(),
            origin_account: None,
            campaign: Some(campaign),
            audience: Audience::CampaignShared(campaign),
            dedupe_scope: DedupeScope::campaign(&campaign),
            request_key: contract::request_key(1),
            retention: RetentionClass::CampaignMedia,
        };
        let GenerationAdmission::Admitted { asset } =
            service.admit_generation(&request).await.unwrap()
        else {
            panic!("the first admission must succeed");
        };
        let mut stream = ChunkedBytes::new(vec![5; 2_000], 512);
        let staged = service
            .stage_output(asset, contract::attempt_id(1), &mut stream)
            .await
            .unwrap();
        assert_eq!(staged, StagingOutcome::Staged { bytes: 2_000 });
        assert_eq!(service.advance(asset).await.unwrap(), Advance::Ready);
        assert_eq!(store.key_count(StorageDomain::PrivateMedia), 2);
        let report = service.sweep_once().await.unwrap();
        assert_eq!(report.variants_deleted, 0);
        assert_eq!(report.staged_deleted, 1);
        assert_eq!(store.key_count(StorageDomain::Quarantine), 0);
        let usage = fixture
            .catalog
            .quota_usage(contract::scope())
            .await
            .unwrap();
        assert_eq!(
            (usage.reserved_bytes, usage.committed_bytes),
            (0, 2_000 + 1_001)
        );
        fixture.finish().await;
    });
}

#[test]
fn a_catalog_that_cannot_reach_the_database_answers_unavailable() {
    runtime().block_on(async {
        let Some(admin) = admin_config() else {
            return;
        };
        let mut unreachable = admin.clone();
        unreachable.dbname("df_o2_media_never_created");
        let result =
            PostgresMediaCatalog::open(unreachable, PostgresTransport::PlaintextLoopbackOnly, 1)
                .await;
        assert!(result.is_err());
    });
}

#[test]
fn a_sweeper_decision_waiting_on_a_new_claim_keeps_the_live_lease() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        contract::admit_and_stage(&fixture.catalog, 1, 0).await;
        assert_eq!(fixture.catalog.due_rows(0, 10).await.unwrap().len(), 1);

        let (mut worker, _) = fixture.client().await;
        let (mut sweeper, sweeper_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        let worker_tx = worker.transaction().await.unwrap();
        assert_eq!(
            sql::claim_step_in(
                &worker_tx,
                contract::claim(1, 1, 2, AssetState::Quarantined, 5_000),
                1,
            )
            .await
            .unwrap(),
            ClaimOutcome::Claimed
        );
        let rejection = tokio::spawn(async move {
            let tx = sweeper.transaction().await.unwrap();
            let result = sql::reject_in(
                &tx,
                contract::asset_id(1),
                contract::attempt_id(1),
                RejectReason::StagedExpired,
                RejectGuard::WhileUnleased,
                2,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            result
        });
        wait_until_blocked(
            &observer,
            sweeper_pid,
            "media.asset WHERE asset_id = $1 FOR UPDATE",
        )
        .await;
        worker_tx.commit().await.unwrap();
        assert_eq!(
            rejection.await.unwrap(),
            df_assets::RejectResult::GuardFailed {
                state: AssetState::Quarantined
            }
        );
        assert_eq!(
            fixture
                .catalog
                .asset(contract::asset_id(1))
                .await
                .unwrap()
                .unwrap()
                .state,
            AssetState::Quarantined
        );
        fixture.finish().await;
    });
}

#[test]
fn admissions_of_one_key_charged_to_different_scopes_meet_at_the_unique_index() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let account = df_types::AccountId::from_bytes(&[88; 16]).unwrap();
        let by_account = Admission {
            charge_scope: QuotaScope::Account(account),
            ..contract::admission(1)
        };
        let mut by_campaign = contract::admission(2);
        by_campaign.request_key = Some(contract::request_key(1));
        let (mut first, _) = fixture.client().await;
        let (mut second, second_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        let first_tx = first.transaction().await.unwrap();
        assert_eq!(
            sql::admit_in(&first_tx, &by_account, 0).await.unwrap(),
            AdmitOutcome::Admitted
        );
        let competitor = tokio::spawn(async move {
            let tx = second.transaction().await.unwrap();
            let outcome = sql::admit_in(&tx, &by_campaign, 1).await.unwrap();
            tx.rollback().await.unwrap();
            outcome
        });
        // Different quota rows, so the only thing to wait for is the unique index entry.
        wait_until_blocked(&observer, second_pid, "INSERT INTO media.asset").await;
        first_tx.commit().await.unwrap();
        assert_eq!(
            competitor.await.unwrap(),
            AdmitOutcome::NotAdmitted(NotAdmitted::InFlight {
                attempt: contract::attempt_id(1)
            })
        );
        let account_usage = fixture
            .catalog
            .quota_usage(QuotaScope::Account(account))
            .await
            .unwrap();
        let campaign_usage = fixture
            .catalog
            .quota_usage(contract::scope())
            .await
            .unwrap();
        assert_eq!(account_usage.reserved_bytes, contract::RESERVE_BYTES);
        assert_eq!(campaign_usage.reserved_bytes, 0);
        fixture.finish().await;
    });
}

#[test]
fn a_supersede_and_an_intended_write_serialise_on_the_asset_row_either_way_round() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let catalog = fixture.catalog.clone();
        for n in [1_u8, 2] {
            contract::admit_and_stage(&catalog, n, 0).await;
            contract::validate_as(&catalog, n, 1, 0).await;
        }
        let intent = |n: u8| df_assets::VariantIntent {
            variant: AssetVariant::Master,
            write: contract::write_id(30 + n),
            key: contract::variant_key(n, n, 30 + n, AssetVariant::Master),
        };
        let (mut superseder, superseder_pid) = fixture.client().await;
        let (mut writer, writer_pid) = fixture.client().await;
        let (observer, _) = fixture.client().await;
        // Supersede commits first: the write sees the new attempt and is refused.
        let supersede_tx = superseder.transaction().await.unwrap();
        sql::supersede_in(
            &supersede_tx,
            contract::asset_id(1),
            contract::attempt_id(1),
            contract::attempt_id(10),
            5,
        )
        .await
        .unwrap();
        let write_intent = intent(1);
        let refused = tokio::spawn(async move {
            let tx = writer.transaction().await.unwrap();
            let outcome = sql::intend_variant_in(&tx, contract::held(1, 1, 1), &write_intent, 6)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            (outcome, writer)
        });
        wait_until_blocked(&observer, writer_pid, "FOR SHARE").await;
        supersede_tx.commit().await.unwrap();
        let (outcome, mut writer) = refused.await.unwrap();
        assert_eq!(outcome, df_assets::LeaseOutcome::Superseded);
        assert!(
            catalog
                .variants(contract::asset_id(1))
                .await
                .unwrap()
                .is_empty()
        );

        // The write commits first: the supersede waits, and the write becomes an orphan.
        let write_tx = writer.transaction().await.unwrap();
        assert_eq!(
            sql::intend_variant_in(&write_tx, contract::held(2, 2, 1), &intent(2), 6)
                .await
                .unwrap(),
            df_assets::LeaseOutcome::Done
        );
        let superseding = tokio::spawn(async move {
            let tx = superseder.transaction().await.unwrap();
            let outcome = sql::supersede_in(
                &tx,
                contract::asset_id(2),
                contract::attempt_id(2),
                contract::attempt_id(20),
                7,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            outcome
        });
        wait_until_blocked(
            &observer,
            superseder_pid,
            "media.asset WHERE asset_id = $1 FOR UPDATE",
        )
        .await;
        write_tx.commit().await.unwrap();
        assert_eq!(
            superseding.await.unwrap(),
            df_assets::SupersedeResult::Superseded
        );
        let orphans = catalog.orphan_variants(i64::MAX, 10).await.unwrap();
        assert_eq!(orphans.len(), 1, "the write that slipped past is swept");
        assert_eq!(orphans[0].asset, contract::asset_id(2));
        fixture.finish().await;
    });
}

#[test]
fn a_winner_of_a_ready_row_survives_the_sweep_even_when_the_attempt_column_moved() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        contract::make_ready(&fixture.catalog, 1, 1, 10).await;
        let (client, _) = fixture.client().await;
        client
            .execute(
                "UPDATE media.asset SET attempt = $2 WHERE asset_id = $1",
                &[
                    &contract::asset_id(1).as_bytes().as_slice(),
                    &contract::attempt_id(99).as_bytes().as_slice(),
                ],
            )
            .await
            .unwrap();
        assert!(
            fixture
                .catalog
                .orphan_variants(i64::MAX, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let winner = df_assets::OrphanVariant {
            asset: contract::asset_id(1),
            attempt: contract::attempt_id(1),
            write: contract::write_id(2),
            variant: AssetVariant::Master,
            key: contract::variant_key(1, 1, 2, AssetVariant::Master),
            version: None,
        };
        assert_eq!(
            fixture
                .catalog
                .begin_variant_deletion(&winner)
                .await
                .unwrap(),
            DeletionMark::NoLongerDeletable
        );
        fixture.finish().await;
    });
}

#[test]
fn the_request_key_lookup_uses_an_index_that_covers_every_state() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let (mut client, _) = fixture.client().await;
        let tx = client.transaction().await.unwrap();
        // Sequential scans off: the plan then shows which index the planner can use at all.
        tx.batch_execute("SET LOCAL enable_seqscan = off")
            .await
            .unwrap();
        let query = format!(
            "EXPLAIN SELECT asset_id, attempt, state FROM media.asset \
             WHERE dedupe_scope = '\\x{}'::bytea AND request_key = '\\x{}'::bytea \
             ORDER BY updated_at_ms DESC",
            "01".repeat(16),
            "02".repeat(32)
        );
        let plan: Vec<String> = tx
            .query(&query, &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect();
        let plan = plan.join("\n");
        assert!(plan.contains("asset_request_key_lookup"), "{plan}");
        assert!(!plan.contains("Seq Scan"), "{plan}");
        tx.rollback().await.unwrap();
        fixture.finish().await;
    });
}

/// Every privilege a role other than the owner, a superuser and `df_control` holds on the
/// `media`, `library` or `journal` schemas, directly, through PUBLIC or through membership.
async fn foreign_privileges(client: &Client) -> Vec<String> {
    let mut found = Vec::new();
    let tables = client
        .query(
            "SELECT r.rolname, n.nspname || '.' || c.relname FROM pg_roles r \
             CROSS JOIN pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname IN ('media', 'library', 'journal') AND c.relkind IN ('r', 'S', 'v', 'm', 'p') \
             AND NOT r.rolsuper AND r.rolname NOT LIKE 'pg\\_%' AND r.rolname <> 'df_control' AND r.oid <> c.relowner \
             AND CASE WHEN c.relkind = 'S' THEN has_sequence_privilege(r.oid, c.oid, 'USAGE,SELECT,UPDATE') \
             ELSE has_table_privilege(r.oid, c.oid, 'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') END",
            &[],
        )
        .await
        .unwrap();
    found.extend(
        tables
            .iter()
            .map(|row| format!("{} on {}", row.get::<_, String>(0), row.get::<_, String>(1))),
    );
    let schemas = client
        .query(
            "SELECT r.rolname, n.nspname FROM pg_roles r CROSS JOIN pg_namespace n \
             WHERE n.nspname IN ('media', 'library', 'journal') AND NOT r.rolsuper AND r.rolname NOT LIKE 'pg\\_%' \
             AND r.rolname <> 'df_control' AND r.oid <> n.nspowner \
             AND has_schema_privilege(r.oid, n.oid, 'USAGE,CREATE')",
            &[],
        )
        .await
        .unwrap();
    found.extend(schemas.iter().map(|row| {
        format!(
            "{} on schema {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1)
        )
    }));
    let functions = client
        .query(
            "SELECT r.rolname, n.nspname || '.' || p.proname FROM pg_roles r \
             CROSS JOIN pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname IN ('media', 'library', 'journal') AND NOT r.rolsuper AND r.rolname NOT LIKE 'pg\\_%' \
             AND r.rolname <> 'df_control' AND r.oid <> p.proowner \
             AND has_function_privilege(r.oid, p.oid, 'EXECUTE')",
            &[],
        )
        .await
        .unwrap();
    found.extend(functions.iter().map(|row| {
        format!(
            "{} runs {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1)
        )
    }));
    let public = client
        .query(
            "SELECT n.nspname || '.' || c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, \
             LATERAL aclexplode(c.relacl) a WHERE n.nspname IN ('media', 'library', 'journal') AND a.grantee = 0              UNION ALL SELECT 'schema ' || n.nspname FROM pg_namespace n, LATERAL aclexplode(n.nspacl) a              WHERE n.nspname IN ('media', 'library', 'journal') AND a.grantee = 0",
            &[],
        )
        .await
        .unwrap();
    found.extend(
        public
            .iter()
            .map(|row| format!("PUBLIC on {}", row.get::<_, String>(0))),
    );
    found
}

#[test]
fn no_role_but_the_owner_and_the_control_plane_holds_any_privilege_on_the_media_schemas() {
    runtime().block_on(async {
        let Some(mut fixture) = Fixture::open().await else {
            return;
        };
        let (client, _) = fixture.client().await;
        assert_eq!(foreign_privileges(&client).await, Vec::<String>::new());
        let control_can: bool = client
            .query_one(
                "SELECT has_table_privilege('df_control', 'media.asset', 'SELECT,INSERT,UPDATE,DELETE') \
                 AND has_schema_privilege('df_control', 'media', 'USAGE')",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert!(control_can, "the control plane must reach the catalog");

        // The walk must notice a grant: a probe role is given one, found, and removed again.
        let probe = format!("df_o2_probe_{}", std::process::id());
        client
            .batch_execute(&format!("CREATE ROLE {probe} NOLOGIN"))
            .await
            .unwrap();
        client
            .batch_execute(&format!("GRANT SELECT ON media.asset TO {probe}"))
            .await
            .unwrap();
        let found = foreign_privileges(&client).await;
        client
            .batch_execute(&format!("REVOKE ALL ON media.asset FROM {probe}; DROP ROLE {probe}"))
            .await
            .unwrap();
        assert_eq!(found, vec![format!("{probe} on media.asset")]);
        client
            .batch_execute("GRANT USAGE ON SCHEMA media TO PUBLIC")
            .await
            .unwrap();
        assert!(!foreign_privileges(&client).await.is_empty(), "PUBLIC access must be seen");
        fixture.finish().await;
    });
}
