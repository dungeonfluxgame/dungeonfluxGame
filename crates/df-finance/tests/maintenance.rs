#![cfg(not(target_arch = "wasm32"))]

//! What keeps the service healthy by itself: the bookkeeping of finished transactions is swept on a
//! schedule an owned worker drives, the accounting periods are kept created ahead, and a cross-owner call
//! that does not answer in time leaves nothing written (planning/finance-ledger-research.md 17.8,
//! 19.1).

mod support;

use std::time::Duration;

use df_commerce::finance_ids::{CampaignKey, PayerId};
use df_finance::fakes::{FakeFence, FakeMedia, SeededIds, SystemClock};
use df_finance::maintenance::{MaintenanceConfig, MaintenanceWorker};
use df_finance::ports::{FenceVerdict, MediaVerdict, MemberPort, PortUnavailable};
use df_finance::service::{FinanceConfig, FinanceError, FinanceService};
use support::attack::service_client;
use support::world::{World, payer, purchase};

async fn marker_count(world: &World) -> i64 {
    world.count("event_marker").await + world.count("check_mark").await
}

#[test]
fn the_worker_sweeps_the_bookkeeping_of_finished_transactions_on_its_own() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut flows = world.service().await;
        let before = marker_count(&world).await;
        for index in 0..12 {
            let reference = format!("pay_swept_{index}");
            flows
                .apply_purchase(purchase(payer(1), &reference, 1_000_000))
                .await
                .expect("purchase");
        }
        let unswept = marker_count(&world).await - before;
        assert!(unswept >= 12, "twelve flows left {unswept} rows");
        let handle = MaintenanceWorker::new(
            service_client(&world).await,
            MaintenanceConfig {
                every: Duration::from_millis(50),
                ..MaintenanceConfig::default()
            },
        )
        .expect("a valid configuration")
        .spawn();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while handle.runs() < 2 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(handle.runs() >= 2, "the worker ran on its own schedule");
        assert_eq!(handle.failures(), 0);
        assert_eq!(handle.drifted_sessions(), 0, "the charged-total invariant");
        assert!(handle.is_running());
        handle.stop().await;
        let after = marker_count(&world).await - before;
        assert!(after < unswept, "the sweep left {after} of {unswept} rows");
        assert_eq!(
            world.count("flow_lock").await,
            0,
            "no flow lock outlives its transaction"
        );
        world.finish().await;
    });
}

#[test]
fn the_accounting_periods_are_extended_ahead_of_the_clock_and_the_run_is_idempotent() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut worker = MaintenanceWorker::new(
            service_client(&world).await,
            MaintenanceConfig {
                period_horizon_months: 60,
                ..MaintenanceConfig::default()
            },
        )
        .expect("a valid configuration");
        let before = world.count("accounting_period").await;
        let first = worker.run_once().await.expect("maintenance");
        assert!(
            first.periods_created > 0,
            "a 60-month horizon needs more than 36 periods"
        );
        assert_eq!(first.drifted_sessions, Some(0));
        let after = world.count("accounting_period").await;
        assert_eq!(after - before, i64::from(first.periods_created));
        let second = worker.run_once().await.expect("again");
        assert_eq!(second.periods_created, 0, "a second run creates nothing");
        assert_eq!(
            second.drifted_sessions, None,
            "the invariant is read on every 60th run"
        );
        let months_ahead: f64 = world
            .admin
            .query_one(
                "SELECT ((max(ends_at_ms) - finance.now_ms()) / (30.4375 * 86400000.0))::float8 FROM finance.accounting_period",
                &[],
            )
            .await
            .expect("horizon")
            .get(0);
        assert!(
            months_ahead >= 59.0,
            "the last period ends {months_ahead} months ahead"
        );
        world.finish().await;
    });
}

#[test]
fn the_horizon_is_capped_at_sixty_months_in_the_database_and_in_the_worker() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let client = service_client(&world).await;
        let before = world.count("accounting_period").await;
        for months in [61_i32, 100_000, -1] {
            let error = client
                .query_one("SELECT finance.extend_period_horizon($1)", &[&months])
                .await
                .expect_err("a horizon beyond five years");
            assert_eq!(
                support::sqlstate(&error).as_deref(),
                Some("22023"),
                "{months}: {error:?}"
            );
        }
        assert_eq!(
            world.count("accounting_period").await,
            before,
            "a refused call creates no period"
        );
        for months in [61, -1] {
            let refused = MaintenanceWorker::new(
                service_client(&world).await,
                MaintenanceConfig {
                    period_horizon_months: months,
                    ..MaintenanceConfig::default()
                },
            );
            assert!(refused.is_err(), "{months} months is refused at start-up");
        }
        let zero = MaintenanceWorker::new(
            service_client(&world).await,
            MaintenanceConfig {
                every: Duration::ZERO,
                ..MaintenanceConfig::default()
            },
        );
        assert!(zero.is_err(), "a zero interval is refused at start-up");
        world.finish().await;
    });
}

/// A membership directory that never answers.
struct SilentMembers;

impl MemberPort for SilentMembers {
    async fn authorized_payers(
        &self,
        _campaign: CampaignKey,
        _payers: &[PayerId],
    ) -> Result<std::collections::BTreeSet<PayerId>, PortUnavailable> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Err(PortUnavailable)
    }
}

#[test]
fn a_cross_owner_call_that_does_not_answer_in_time_leaves_the_admission_unwritten() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut funding = world.service().await;
        funding
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        funding
            .allocate(
                payer(1),
                world.session,
                support::world::operation(1),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let mut service = FinanceService::new(
            service_client(&world).await,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            SilentMembers,
            Box::new(SystemClock),
            Box::new(SeededIds::new(7)),
            FinanceConfig {
                port_deadline_ms: 100,
                ..FinanceConfig::default()
            },
        );
        let started = std::time::Instant::now();
        let error = service
            .admit(&world.admit_command(1, 1000, 1000))
            .await
            .expect_err("no answer");
        assert!(matches!(error, FinanceError::PortUnavailable), "{error:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline bounds the wait"
        );
        assert_eq!(world.count("reservation").await, 0);
        assert_eq!(
            world.count("journal_entry").await,
            2,
            "only the purchase and the allocation"
        );
        world.finish().await;
    });
}
