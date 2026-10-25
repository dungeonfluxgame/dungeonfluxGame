//! The invariants of planning/finance-ledger-research.md 17, re-derived from the tables with
//! plain queries (not by calling the guards under test): balance, non-negativity, the balance
//! rows reconciling to the journal, lot conservation, the hash chain, the charged totals and the held
//! totals.

#![allow(dead_code, reason = "each test binary uses a different invariant set")]

use tokio_postgres::Client;

async fn rows(admin: &Client, sql: &str) -> Vec<String> {
    admin
        .query(sql, &[])
        .await
        .unwrap_or_else(|error| panic!("invariant query failed: {error:?}\n{sql}"))
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect()
}

/// Every line sums to zero within its book and currency.
pub async fn balance(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT a.book || '/' || k.currency || ' sums to ' || sum(l.amount_micro_dollars)::text \
         FROM finance.journal_line l JOIN finance.account_key k ON k.key_id = l.key_id \
         JOIN finance.ledger_account a ON a.code = k.code \
         GROUP BY a.book, k.currency HAVING sum(l.amount_micro_dollars) <> 0",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("unbalanced: {bad:?}"))
    }
}

/// No balance-enforced account is negative.
pub async fn non_negative(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT key_id::text || '=' || natural_micro_dollars::text FROM finance.account_balance WHERE natural_micro_dollars < 0",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("negative balances: {bad:?}"))
    }
}

/// The balance rows equal the journal's lines times the account's normal side.
pub async fn reconciles(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT b.key_id::text || ': row ' || b.natural_micro_dollars::text || ' journal ' || (coalesce(s.total, 0) * a.normal_side)::text \
         FROM finance.account_balance b JOIN finance.account_key k ON k.key_id = b.key_id \
         JOIN finance.ledger_account a ON a.code = k.code \
         LEFT JOIN (SELECT key_id, sum(amount_micro_dollars) AS total FROM finance.journal_line GROUP BY key_id) s ON s.key_id = b.key_id \
         WHERE b.natural_micro_dollars <> coalesce(s.total, 0) * a.normal_side",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("balance rows differ from the journal: {bad:?}"))
    }
}

/// original = keys + consumed + refunded + extinguished + rounded + expired + forfeited, per lot.
pub async fn conserved(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT encode(l.lot_id, 'hex') || ' off by ' || (l.original_micro_dollars - coalesce(k.total, 0) - l.consumed_micro_dollars \
             - l.refunded_micro_dollars - l.extinguished_micro_dollars - l.rounded_micro_dollars - l.expired_micro_dollars \
             - l.forfeited_micro_dollars)::text \
         FROM finance.funding_lot l LEFT JOIN (SELECT k.lot_id, sum(b.natural_micro_dollars) AS total FROM finance.account_key k \
             JOIN finance.account_balance b ON b.key_id = k.key_id WHERE k.lot_id IS NOT NULL GROUP BY k.lot_id) k ON k.lot_id = l.lot_id \
         WHERE l.original_micro_dollars - coalesce(k.total, 0) - l.consumed_micro_dollars - l.refunded_micro_dollars \
             - l.extinguished_micro_dollars - l.rounded_micro_dollars - l.expired_micro_dollars - l.forfeited_micro_dollars <> 0",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("lots do not conserve: {bad:?}"))
    }
}

/// Each stream is contiguous from 1, links to the previous hash, and each hash recomputes.
pub async fn chains(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT encode(e.entry_id, 'hex') || ' seq ' || e.stream_seq::text FROM ( \
            SELECT entry_id, stream_seq, prev_hash, entry_hash, entry_hash_version, \
                   lag(entry_hash) OVER w AS before_hash, lag(stream_seq) OVER w AS before_seq \
              FROM finance.journal_entry WINDOW w AS (PARTITION BY stream_kind, stream_id ORDER BY stream_seq)) e \
         WHERE e.stream_seq <> coalesce(e.before_seq, 0) + 1 \
            OR e.prev_hash <> coalesce(e.before_hash, decode(repeat('00', 32), 'hex')) \
            OR e.entry_hash <> finance.entry_hash(e.entry_hash_version, e.prev_hash, finance.entry_json(e.entry_id))",
    )
    .await;
    if !bad.is_empty() {
        return Err(format!("broken hash chain at: {bad:?}"));
    }
    let heads = rows(
        admin,
        "SELECT s.stream_kind || '/' || encode(s.stream_id, 'hex') FROM finance.ledger_stream s \
         LEFT JOIN (SELECT stream_kind, stream_id, max(stream_seq) AS seq FROM finance.journal_entry GROUP BY 1, 2) e \
           ON e.stream_kind = s.stream_kind AND e.stream_id = s.stream_id \
         WHERE s.last_seq <> coalesce(e.seq, 0)",
    )
    .await;
    if heads.is_empty() {
        Ok(())
    } else {
        Err(format!("stream heads differ from the journal: {heads:?}"))
    }
}

/// The reserved keys hold exactly what the open shares still hold.
pub async fn held(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT 'reserved keys ' || coalesce(k.total, 0)::text || ' open shares ' || coalesce(s.total, 0)::text FROM \
            (SELECT sum(b.natural_micro_dollars) AS total FROM finance.account_balance b JOIN finance.account_key k ON k.key_id = b.key_id \
              WHERE k.code IN (2130, 2230)) k, \
            (SELECT sum(held_micro_dollars - charged_micro_dollars - released_micro_dollars) AS total FROM finance.reservation_share) s \
         WHERE coalesce(k.total, 0) <> coalesce(s.total, 0)",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("held totals differ: {bad:?}"))
    }
}

/// A session's running charged total equals the retail of its usage events.
pub async fn charged_total(admin: &Client) -> Result<(), String> {
    let bad = rows(
        admin,
        "SELECT encode(s.session_id, 'hex') || ': total ' || s.charged_total_micro_dollars::text || ' usage ' || coalesce(u.total, 0)::text          FROM finance.session_funding s          LEFT JOIN (SELECT session_id, sum(retail_micro_dollars) AS total FROM finance.usage_event GROUP BY session_id) u            ON u.session_id = s.session_id          WHERE s.charged_total_micro_dollars <> coalesce(u.total, 0)",
    )
    .await;
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "charged total differs from the usage events: {bad:?}"
        ))
    }
}

/// All of them; the first failure wins.
pub async fn all(admin: &Client) -> Result<(), String> {
    balance(admin).await?;
    non_negative(admin).await?;
    reconciles(admin).await?;
    conserved(admin).await?;
    chains(admin).await?;
    charged_total(admin).await?;
    held(admin).await
}
