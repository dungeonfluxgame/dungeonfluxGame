#![cfg(not(target_arch = "wasm32"))]

mod support;

use df_commerce::ledger::AccountCode;
use df_finance::reconciliation::{
    FakeReconciliationSource, ReconciliationPorts, SourceLine, reconcile_period,
    verify_journal_hash_chain,
};
use tokio::time::{Duration, timeout};

#[test]
fn ops_04_detects_unexplained_account_imbalance_and_serializes_period_runs() {
    support::run(async {
        let Some(world) = support::world::World::open().await else {
            return;
        };
        let period_id = 202610;
        let clean_sources = ReconciliationPorts {
            processor: FakeReconciliationSource::default(),
            invoices: FakeReconciliationSource::default(),
            dispatches: FakeReconciliationSource::default(),
        };
        let clean = reconcile_period(
            &mut world.scratch.connect().await,
            period_id,
            &clean_sources,
        )
        .await
        .expect("clean reconciliation");
        assert!(clean.is_clean());

        let imbalanced_sources = ReconciliationPorts {
            processor: FakeReconciliationSource::new(vec![SourceLine {
                kind: df_finance::reconciliation::ReferenceKind::ProcessorCapture,
                reference: "capture-ops-04".into(),
                period_id,
                account: AccountCode::ProcessorClearing,
                amount_micro_dollars: 125_000,
            }]),
            invoices: FakeReconciliationSource::default(),
            dispatches: FakeReconciliationSource::default(),
        };

        let blocker = world.scratch.connect().await;
        let held = blocker.transaction().await.expect("lock transaction");
        held.query_one("SELECT pg_advisory_xact_lock($1::integer)", &[&period_id])
            .await
            .expect("hold period lock");
        let mut reader = world.scratch.connect().await;
        let backend_pid: i32 = reader
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("reader pid")
            .get(0);
        let task = tokio::spawn(async move {
            reconcile_period(&mut reader, period_id, &imbalanced_sources).await
        });
        timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = world
                    .admin
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM pg_locks \
                         WHERE pid = $1 AND locktype = 'advisory' AND NOT granted)",
                        &[&backend_pid],
                    )
                    .await
                    .expect("inspect advisory lock wait")
                    .get(0);
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reconciliation must wait on the period lock");
        held.commit().await.expect("release period lock");
        let report = timeout(Duration::from_secs(5), task)
            .await
            .expect("reconciliation completes")
            .expect("reconciliation task")
            .expect("imbalanced reconciliation");
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].account_code, 1100);
        assert_eq!(report.differences[0].unexplained_micro_dollars, 125_000);

        let verification = verify_journal_hash_chain(&world.admin)
            .await
            .expect("verify journal chain");
        assert!(verification.valid);
        assert!(verification.machine_readable().is_ok());
        world.finish().await;
    });
}
