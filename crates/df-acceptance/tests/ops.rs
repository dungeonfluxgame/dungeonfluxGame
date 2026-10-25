//! Plan section 17, OPS-01 to OPS-04: drain, restore, backup and reconciliation (plan 12.5, 13.10,
//! 16.4). The runner, owner directory, backup tooling and reconciliation job are owned by O1, S2
//! and F, so every scenario here is blocked. They are written against
//! `planning/hosted-runtime-research.md` 6.5 and `planning/finance-ledger-research.md` 21 and 22.

use df_acceptance::blocked_scenario;

blocked_scenario! {
    fn ops_01_a_worker_drain_during_an_active_session_checkpoints_and_reconnects_without_a_duplicate_owner,
    blocked_on: "blocked on O1 and F API: POST /internal/drain with AdmissionPolicy::begin_shutdown, the owner directory's claim_seq and fence_current, and F's settlement that needs no owner (hosted runtime design 6.5, finance design 26.1)",
    setup: [
        "two runner fakes W1 and W2 and an owner directory; campaign C Active on W1 with six clients connected, one paid effect in flight and its attempt Sent, controlled clock",
    ],
    steps: [
        "drain W1: readiness goes false at once, new commands get the typed table-is-moving refusal, a save point of kind drain is written, the segment closes with reason drain and the placement is released",
        "W2 claims with claim_seq + 1 and loads the checkpoint; clients reconnect; the whole transfer is measured against the two minute target",
        "let the old W1 try to commit a decision, a meter roll and a settlement under its obsolete fence",
        "assert: W1's late commits are rejected and only W2 owns C; the reconnected clients see the same state with no duplicate decision",
        "assert: the in-flight effect, if still Dispatching, stays unknown exposure on the next owner and is never resent",
        "assert: settlement and release complete while no owner holds the campaign (settlement_and_release_complete_while_no_owner_holds_the_campaign)",
    ],
    outcome: "Controlled checkpoint/reconnect without duplicate owner",
}

blocked_scenario! {
    fn ops_02_restoring_the_database_to_before_a_recent_payment_or_dispatch_blocks_unsafe_reopening,
    blocked_on: "blocked on F and S2 API: the recovery journal replay and the per-campaign money freeze after a restore (finance design 22.2, 22.3)",
    setup: [
        "database backup at T0; after T0 payer A paid $20.00 (processor record exists) and a paid image was dispatched (journal and provider records exist)",
        "restore the database to T0 with the journal intact",
    ],
    steps: [
        "start the platform after the restore and attempt to reopen paid sessions",
        "assert: paid admission stays frozen until the journal tail is replayed (a_restore_missing_recent_finance_entries_keeps_paid_work_frozen_until_the_journal_tail_is_replayed)",
        "assert: the missing payment is re-posted exactly once from the processor's authoritative record and the dispatch returns as unknown exposure, never resent (a_journaled_dispatch_missing_from_the_database_returns_as_unknown_exposure_never_resent)",
        "assert: per-campaign reconstruction finishes within its target and clears the freeze; a failed reconstruction leaves the campaign read-only",
    ],
    outcome: "Reconciliation blocks unsafe reopening",
}

blocked_scenario! {
    fn ops_03_an_independent_backup_restore_verifies_integrity_secrets_versions_and_recovery_time,
    blocked_on: "blocked on S2 and O2 API: restore drill tooling that restores into a clean environment from the object-store backup bundle (plan 13.10)",
    setup: [
        "a populated staging fake: 20 campaigns, assets in the object store fake, finance rows, schema version N, and a backup bundle taken by the production backup job",
    ],
    steps: [
        "restore the bundle into a fresh environment that shares no state with the source",
        "assert: row counts, object digests and the journal checksum match the manifest; schema and content versions match the build under test",
        "assert: no plaintext secret is in the bundle and runtime credentials are re-issued, not copied",
        "assert: a purged asset restored from backup stays refused",
        "assert: the measured recovery time is recorded and is within the documented objective",
    ],
    outcome: "Integrity, secrets, versions, and recovery time verified",
}

blocked_scenario! {
    fn ops_04_a_full_finance_reconciliation_shows_no_unexplained_imbalance,
    blocked_on: "blocked on F API: the reconciliation jobs of finance design 21 (processor captures, refunds and disputes against the journal, payouts against clearing, supplier usage against attempts)",
    setup: [
        "a seeded month: purchases, refunds, one dispute, supplier usage with one unknown outcome, payouts and one manual adjustment with reason and case reference",
    ],
    steps: [
        "run the reconciliation job",
        "assert: a full reconciliation of the seeded month reports no unexplained difference",
        "plant a missing purchase: assert it is reported, not balanced away (a_planted_missing_purchase_is_reported_not_balanced_away)",
        "assert: invoice spend no attempt explains is reported as an unmetered route with an alert, and the unknown outcome appears as exposure, never as a balancing entry",
        "assert: the report is deterministic across two runs",
    ],
    outcome: "No unexplained imbalance",
}
