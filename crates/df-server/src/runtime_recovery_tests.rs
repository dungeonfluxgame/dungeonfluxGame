use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::watch;

#[path = "../../df-persistence/tests/runtime_support/mod.rs"]
mod runtime_support;

use crate::runtime_recovery::{
    PostgresRecoveryStore, RecoveryFailure, RecoveryFuture, RecoveryStore, StartupRefusal,
    start_hosted,
};

struct FakeRecoveryStore {
    prepared_failure: Option<RecoveryFailure>,
    halted: bool,
    break_failures: u8,
    attempts: AtomicUsize,
    changed: watch::Sender<usize>,
}

impl RecoveryStore for Arc<FakeRecoveryStore> {
    fn require_no_prepared_transactions(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            match self.prepared_failure {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        })
    }

    fn service_is_halted(&self) -> RecoveryFuture<'_, bool> {
        Box::pin(async move { Ok(self.halted) })
    }

    fn break_service_lock(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            self.changed.send_replace(attempt);
            if attempt <= usize::from(self.break_failures) {
                Err(RecoveryFailure::ProcedureFailed)
            } else {
                Ok(())
            }
        })
    }
}

fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
        .block_on(future);
}

#[test]
fn startup_refuses_when_prepared_transactions_are_enabled() {
    run(async {
        let store = Arc::new(FakeRecoveryStore {
            prepared_failure: Some(RecoveryFailure::PreparedTransactionsEnabled),
            halted: false,
            break_failures: 0,
            attempts: AtomicUsize::new(0),
            changed: watch::channel(0).0,
        });
        assert_eq!(
            start_hosted(store).await.err(),
            Some(StartupRefusal::PreparedTransactionsEnabled)
        );
    });
}

#[test]
fn df002_retries_with_a_bounded_count_and_non_df002_is_ignored() {
    run(async {
        let store = Arc::new(FakeRecoveryStore {
            prepared_failure: None,
            halted: false,
            break_failures: 3,
            attempts: AtomicUsize::new(0),
            changed: watch::channel(0).0,
        });
        let mut attempts = store.changed.subscribe();
        let handle = start_hosted(Arc::clone(&store)).await.expect("startup");
        handle.report_fenced_sqlstate("40001");
        assert_eq!(store.attempts.load(Ordering::SeqCst), 0);
        handle.report_fenced_sqlstate("DF002");
        tokio::time::timeout(Duration::from_secs(4), async {
            while store.attempts.load(Ordering::SeqCst) < 3 {
                attempts.changed().await.expect("attempt update");
            }
        })
        .await
        .expect("three recovery attempts");
        assert_eq!(store.attempts.load(Ordering::SeqCst), 3);
        drop(handle);
    });
}

#[test]
fn a_recorded_halt_prevents_breaking_the_service_lock() {
    run(async {
        let store = Arc::new(FakeRecoveryStore {
            prepared_failure: None,
            halted: true,
            break_failures: 0,
            attempts: AtomicUsize::new(0),
            changed: watch::channel(0).0,
        });
        let handle = start_hosted(Arc::clone(&store)).await.expect("startup");
        handle.report_fenced_sqlstate("DF002");
        tokio::task::yield_now().await;
        assert_eq!(store.attempts.load(Ordering::SeqCst), 0);
        drop(handle);
    });
}

use std::time::Duration;

struct SignalingPostgresStore {
    store: PostgresRecoveryStore,
    broken: watch::Sender<bool>,
}

impl RecoveryStore for SignalingPostgresStore {
    fn require_no_prepared_transactions(&self) -> RecoveryFuture<'_> {
        self.store.require_no_prepared_transactions()
    }

    fn service_is_halted(&self) -> RecoveryFuture<'_, bool> {
        self.store.service_is_halted()
    }

    fn break_service_lock(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            self.store.break_service_lock().await?;
            self.broken.send_replace(true);
            Ok(())
        })
    }
}

#[test]
fn the_sweeper_breaks_a_hostile_postgres_lock_after_a_fenced_df002() {
    runtime_support::run(async {
        let Some(scratch) = runtime_support::Scratch::open().await else {
            return;
        };
        let admin = scratch.connect().await;
        let (table, placement, fence) = runtime_support::serving_table(&admin, 1, 1).await;
        let key = runtime_support::service_key(&admin).await;
        let hostile = scratch.login_as("df_campaign_runtime").await;
        hostile
            .batch_execute(&format!(
                "BEGIN; SET LOCAL idle_in_transaction_session_timeout = 0; \
                 SET LOCAL statement_timeout = 0; \
                 SELECT pg_advisory_xact_lock({}, {})",
                key.0, key.1
            ))
            .await
            .expect("hostile holder takes the service lock");
        let fenced = runtime_support::as_role(
            &admin,
            "df_control",
            "SELECT df_runtime.fence_current($1, $2, $3, $4, $5, 'game')",
            &[
                &table.tenant,
                &table.campaign,
                &fence.recovery_epoch,
                &fence.claim_seq,
                &placement.id,
            ],
        )
        .await;
        assert_eq!(
            runtime_support::sqlstate(&fenced.expect_err("hostile lock reports DF002")).as_deref(),
            Some("DF002")
        );

        let control = scratch.login_as("df_control").await;
        let (broken, mut broken_event) = watch::channel(false);
        let store = SignalingPostgresStore {
            store: PostgresRecoveryStore::from_client(control),
            broken,
        };
        let handle = start_hosted(store).await.expect("prepared check passes");
        handle.report_fenced_sqlstate("DF002");
        tokio::time::timeout(Duration::from_secs(12), broken_event.changed())
            .await
            .expect("sweeper breaks the lock")
            .expect("break notification");
        assert!(hostile.query("SELECT 1", &[]).await.is_err());
        drop(handle);
        scratch.finish().await;
    });
}
