#![cfg(not(target_arch = "wasm32"))]

mod support;

use df_finance::exports::reserve::report_as_of;
use df_finance::reconcile_bank::reconcile_imports;
use support::world::{World, payer, purchase};

#[test]
fn books_reserve_report_reads_purchased_credit_from_journal_as_of_time() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "books_reserve_fixture", 1_250_000))
            .await
            .expect("invented purchase posts");

        let report = report_as_of(&world.admin, support::attack::now_ms())
            .await
            .expect("read reserve report");
        assert_eq!(report.purchased_unspent_micro_dollars, 1_250_000);
        assert_eq!(report.refund_payable_micro_dollars, 0);
        assert_eq!(
            report.customer_credit_refund_reserve_micro_dollars,
            1_250_000
        );
        assert_eq!(report.promotional_credit_micro_dollars, 0);
        assert_eq!(report.unknown_supplier_exposure_micro_dollars, 0);
    });
}

#[test]
fn books_reconciliation_reports_imported_rows_without_ledger_payouts_as_unmatched() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let report = reconcile_imports(
            &world.admin,
            "date,description,reference,amount_cents,currency\n2026-10-25,Invented bank payout,po_fixture,1250,USD\n",
            "payout_id,date,amount_cents,currency\npo_fixture,2026-10-25,1250,USD\n",
        )
        .await
        .expect("reconciliation query succeeds");
        assert!(report.mercury_format_verify_required);
        assert_eq!(report.unmatched_bank_lines.len(), 1);
        assert_eq!(report.unmatched_processor_payouts.len(), 1);
        assert!(report.unmatched_ledger_payouts.is_empty());
    });
}
