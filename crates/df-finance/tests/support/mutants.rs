//! Mutants: one guard of the boundary removed or broken in a scratch database, so a test can show
//! that the invariant or attack paired with it fails when the guard is gone. A mutant is applied
//! with the superuser connection and changes only the scratch database.

#![allow(dead_code, reason = "each test binary uses a different set of mutants")]

use tokio_postgres::Client;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mutant {
    /// The deferred check that an entry's lines sum to zero, and the event-level check beside it.
    NoBalanceCheck,
    /// The CHECK that a balance-enforced account never goes below zero.
    NoNonNegativity,
    /// The per-lot conservation check, in the deferred triggers and in the event-level pass.
    NoLotConservation,
    /// The triggers that make the journal append-only.
    NoAppendOnly,
    /// The deferred check that an entry chains to its stream head.
    NoChainCheck,
    /// The allocation totals check, in the deferred triggers and in the event-level pass.
    NoAllocationTotals,
    /// Per-kind equations that always pass.
    EquationsAreNoOps(&'static [&'static str]),
    /// The balance maintenance in `post_entries` over-credits every credit by one unit.
    BalanceRowDrifts,
    /// `post_entries` posts the entry at a hash that skips the previous hash.
    HashIgnoresPrevious,
    /// The "already checked" memo trusts a session setting, which any session can set, and the
    /// event-level balance check is gone.
    MemoInSessionSettings,
    /// The write guard that refuses a write outside `commit_event`.
    NoWriteGuard,
    /// The allowlist: any account may take any side.
    NoAllowlist,
    /// The trial rules: the same-event trial_grant row, one trial lot per account, and the
    /// database-derived business key.
    NoTrialRules,
    /// The per-kind column allowlists of `apply_change`.
    NoColumnAllowlist,
    /// The anchor's stream and correlation expectations.
    NoAnchorExpectations,
    /// The back-dating rule.
    NoBackdatingCheck,
    /// The system-actor rule, in `check_entry` and as a CHECK.
    NoSystemActorCheck,
    /// The closed reason list, in `check_entry` and as a foreign key.
    NoReasonCheck,
    /// The event-level pass (`check_touched`, `check_event_balance`) never runs and never marks an
    /// object checked, so the per-row deferred triggers are the only guard left.
    EventLevelPassSkipped,
    /// The write of the session's charged total adds one to what the row says.
    ChargedTotalDrifts,
    /// The journal payload allowlist keeps the lot's `billing_region`.
    JournalLeaksBillingRegion,
}

async fn run(admin: &Client, sql: &str) {
    admin
        .batch_execute(sql)
        .await
        .unwrap_or_else(|error| panic!("mutant failed: {error:?}\n{sql}"));
}

/// Rewrites a function's source with one textual replacement (which must find its text).
async fn replace_in_function(admin: &Client, signature: &str, from: &str, to: &str) {
    let definition: String = admin
        .query_one(
            "SELECT pg_get_functiondef($1::text::regprocedure)",
            &[&signature],
        )
        .await
        .unwrap_or_else(|error| panic!("function {signature}: {error:?}"))
        .get(0);
    assert!(
        definition.contains(from),
        "mutant text not found in {signature}: {from}"
    );
    run(admin, &definition.replacen(from, to, 1)).await;
}

async fn noop(admin: &Client, signature: &str, arguments: &str, returns: &str, body: &str) {
    run(
        admin,
        &format!(
            "CREATE OR REPLACE FUNCTION {signature}({arguments}) RETURNS {returns} LANGUAGE plpgsql AS $$ BEGIN {body} END $$"
        ),
    )
    .await;
}

pub async fn apply(admin: &Client, mutant: Mutant) {
    match mutant {
        Mutant::NoBalanceCheck => {
            run(
                admin,
                "ALTER TABLE finance.journal_entry DISABLE TRIGGER journal_entry_balanced; \
                 ALTER TABLE finance.journal_line DISABLE TRIGGER journal_line_balanced",
            )
            .await;
            noop(
                admin,
                "finance.check_event_balance",
                "p_entry_ids bytea[], p_keys jsonb",
                "void",
                "RETURN;",
            )
            .await;
        }
        Mutant::NoNonNegativity => {
            run(
                admin,
                "DO $$ DECLARE c record; BEGIN \
                   FOR c IN SELECT conname FROM pg_constraint \
                             WHERE conrelid = 'finance.account_balance'::regclass AND contype = 'c' LOOP \
                     EXECUTE format('ALTER TABLE finance.account_balance DROP CONSTRAINT %I', c.conname); \
                   END LOOP; END $$",
            )
            .await;
        }
        Mutant::NoLotConservation => {
            noop(
                admin,
                "finance.check_lot_conservation",
                "p_lot bytea",
                "void",
                "RETURN;",
            )
            .await;
        }
        Mutant::NoAppendOnly => {
            run(
                admin,
                "ALTER TABLE finance.journal_entry DISABLE TRIGGER journal_entry_append_only; \
                 ALTER TABLE finance.journal_line DISABLE TRIGGER journal_line_append_only",
            )
            .await;
        }
        Mutant::NoAllocationTotals => {
            noop(
                admin,
                "finance.check_allocation_totals",
                "p_allocation bytea",
                "void",
                "RETURN;",
            )
            .await;
        }
        Mutant::NoChainCheck => {
            run(
                admin,
                "ALTER TABLE finance.journal_entry DISABLE TRIGGER journal_entry_chain",
            )
            .await;
        }
        Mutant::EquationsAreNoOps(names) => {
            for name in names {
                noop(
                    admin,
                    &format!("finance.{name}"),
                    "p jsonb",
                    "void",
                    "RETURN;",
                )
                .await;
            }
        }
        Mutant::BalanceRowDrifts => {
            replace_in_function(
                admin,
                "finance.post_entries(bytea,jsonb,jsonb,jsonb,bigint,integer)",
                "b.natural_micro_dollars + d.delta",
                "b.natural_micro_dollars + d.delta + CASE WHEN d.delta > 0 THEN 1 ELSE 0 END",
            )
            .await;
        }
        Mutant::HashIgnoresPrevious => {
            replace_in_function(
                admin,
                "finance.post_entries(bytea,jsonb,jsonb,jsonb,bigint,integer)",
                "v_hash := finance.entry_hash(1::smallint, v_head.last_hash, v_ej);",
                "v_hash := finance.entry_hash(1::smallint, decode(repeat('00', 32), 'hex'), v_ej);",
            )
            .await;
        }
        Mutant::MemoInSessionSettings => {
            run(
                admin,
                "CREATE OR REPLACE FUNCTION finance.already_checked(p_tag text) RETURNS boolean LANGUAGE sql STABLE \
                 AS $$ SELECT coalesce(current_setting('finance.checked_' || p_tag, true), '') = '1' $$",
            )
            .await;
            noop(
                admin,
                "finance.check_event_balance",
                "p_entry_ids bytea[], p_keys jsonb",
                "void",
                "RETURN;",
            )
            .await;
        }
        Mutant::NoWriteGuard => {
            run(
                admin,
                "CREATE OR REPLACE FUNCTION finance.write_guard() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$",
            )
            .await;
        }
        Mutant::NoAllowlist => {
            run(
                admin,
                "ALTER TABLE finance.entry_kind_rule DISABLE TRIGGER entry_kind_rule_append_only",
            )
            .await;
            run(admin, "DELETE FROM finance.entry_kind_rule").await;
            run(
                admin,
                "INSERT INTO finance.entry_kind_rule (kind, code, side) \
                 SELECT k.kind, a.code, s.side FROM finance.entry_kind k, finance.ledger_account a, (VALUES (1::smallint), (-1::smallint)) s(side)",
            )
            .await;
        }
        Mutant::NoTrialRules => {
            noop(
                admin,
                "finance.check_trial_entry",
                "p_entry jsonb, p_event jsonb",
                "void",
                "RETURN;",
            )
            .await;
            run(
                admin,
                "CREATE OR REPLACE FUNCTION finance.derived_business_key(p_kind text, p_entry jsonb) RETURNS bytea \
                 LANGUAGE sql IMMUTABLE AS $$ SELECT NULL::bytea $$",
            )
            .await;
        }
        Mutant::NoColumnAllowlist => {
            replace_in_function(
                admin,
                "finance.apply_change(text,jsonb)",
                "IF v_extra IS NOT NULL THEN",
                "IF false THEN",
            )
            .await;
        }
        Mutant::EventLevelPassSkipped => {
            noop(
                admin,
                "finance.check_touched",
                "p_event jsonb, p_keys jsonb, p_entry_ids bytea[]",
                "void",
                "RETURN;",
            )
            .await;
            noop(
                admin,
                "finance.check_event_balance",
                "p_entry_ids bytea[], p_keys jsonb",
                "void",
                "RETURN;",
            )
            .await;
        }
        Mutant::ChargedTotalDrifts => {
            replace_in_function(
                admin,
                "finance.apply_session_charged_total_update(jsonb,bigint)",
                "THEN r.charged_total_micro_dollars ELSE",
                "THEN r.charged_total_micro_dollars + 1 ELSE",
            )
            .await;
        }
        Mutant::NoAnchorExpectations => {
            run(
                admin,
                "CREATE OR REPLACE FUNCTION finance.anchor_expectation(p_anchor text, p_entry jsonb, p_event jsonb) RETURNS jsonb \
                 LANGUAGE sql STABLE AS $$ SELECT '{}'::jsonb $$",
            )
            .await;
        }
        Mutant::NoBackdatingCheck => {
            replace_in_function(
                admin,
                "finance.check_entry(jsonb,jsonb,text[],bigint,integer)",
                "IF abs(v_effective - p_now) > 60000 THEN",
                "IF false THEN",
            )
            .await;
        }
        Mutant::NoSystemActorCheck => {
            replace_in_function(
                admin,
                "finance.check_entry(jsonb,jsonb,text[],bigint,integer)",
                "IF (p_entry ->> 'actor_kind') = 'system' AND (",
                "IF false AND (",
            )
            .await;
            run(
                admin,
                "DO $$ DECLARE c record; BEGIN \
                   FOR c IN SELECT conname FROM pg_constraint \
                             WHERE conrelid = 'finance.journal_entry'::regclass AND contype = 'c' \
                               AND pg_get_constraintdef(oid) LIKE '%<> ''system''%' LOOP \
                     EXECUTE format('ALTER TABLE finance.journal_entry DROP CONSTRAINT %I', c.conname); \
                   END LOOP; END $$",
            )
            .await;
        }
        Mutant::NoReasonCheck => {
            replace_in_function(
                admin,
                "finance.check_entry(jsonb,jsonb,text[],bigint,integer)",
                "IF NOT EXISTS (SELECT 1 FROM finance.entry_reason WHERE code = p_entry ->> 'reason') THEN",
                "IF false THEN",
            )
            .await;
            run(
                admin,
                "ALTER TABLE finance.journal_entry DROP CONSTRAINT journal_entry_reason_fkey",
            )
            .await;
        }
        Mutant::JournalLeaksBillingRegion => {
            run(
                admin,
                "ALTER TABLE finance.change_kind DISABLE TRIGGER change_kind_append_only; \
                 UPDATE finance.change_kind SET journal_cols = journal_cols || 'billing_region'::text WHERE kind = 'funding_lot_insert'; \
                 ALTER TABLE finance.change_kind ENABLE TRIGGER change_kind_append_only",
            )
            .await;
        }
    }
}
