//! Trusted startup installation over the existing imagery, media and hosted migration ledgers.
//! Runtime workers open existing schemas and never call these installers.

use tokio_postgres::Transaction;

const IMAGERY: [(i32, &str); 17] = [
    (
        38,
        include_str!("../migrations/0038_imagery_operations.sql"),
    ),
    (
        39,
        include_str!("../migrations/0039_imagery_admission_bindings.sql"),
    ),
    (40, include_str!("../migrations/0040_imagery_execution.sql")),
    (41, include_str!("../migrations/0041_imagery_identity.sql")),
    (
        42,
        include_str!("../migrations/0042_imagery_publication.sql"),
    ),
    (
        43,
        include_str!("../migrations/0043_imagery_draft_funding.sql"),
    ),
    (44, include_str!("../migrations/0044_imagery_usage.sql")),
    (
        45,
        include_str!("../migrations/0045_imagery_compatible.sql"),
    ),
    (46, include_str!("../migrations/0046_imagery_analysis.sql")),
    (
        47,
        include_str!("../migrations/0047_imagery_execution_outcomes.sql"),
    ),
    (
        48,
        include_str!("../migrations/0048_imagery_artifact_reuse.sql"),
    ),
    (
        49,
        include_str!("../migrations/0049_imagery_compatible_key_version.sql"),
    ),
    (
        50,
        include_str!("../migrations/0050_imagery_campaign_release_guard.sql"),
    ),
    (
        51,
        include_str!("../migrations/0051_imagery_source_denial.sql"),
    ),
    (
        52,
        include_str!("../migrations/0052_imagery_authority_evidence.sql"),
    ),
    (
        53,
        include_str!("../migrations/0053_imagery_source_facts.sql"),
    ),
    (
        55,
        include_str!("../migrations/0055_imagery_submit_decline.sql"),
    ),
];

const FINANCE: [(i32, &str); 6] = [
    (20, include_str!("../migrations/0020_finance_journal.sql")),
    (21, include_str!("../migrations/0021_finance_funding.sql")),
    (22, include_str!("../migrations/0022_finance_admission.sql")),
    (
        23,
        include_str!("../migrations/0023_finance_refunds_disputes.sql"),
    ),
    (24, include_str!("../migrations/0024_finance_boundary.sql")),
    (
        25,
        include_str!("../migrations/0025_finance_refunds_disputes_boundary.sql"),
    ),
];

/// Installs imagery 0038 through 0053 and upgrade 0055 exactly once per ledger.
/// Requires the real media 0010..0014 and finance 0020..0025 schemas. Hosted startup also
/// supplies canonical campaign/session authority; local 0041 uses its source-owned local branch.
/// Any error requires rollback. No transaction is committed and no application records are seeded.
pub async fn migrate_in(tx: &Transaction<'_>) -> Result<(), tokio_postgres::Error> {
    // Old journal and execution openers own one of these locks each, in no reverse order.
    tx.batch_execute(
        "SELECT pg_advisory_xact_lock(hashtext('df_imagery_migration')); \
         SELECT pg_advisory_xact_lock(hashtext('df_imagery_execution')); \
         CREATE SCHEMA IF NOT EXISTS imagery; \
         CREATE TABLE IF NOT EXISTS imagery.schema_migration (\
             version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
    )
    .await?;
    for (version, sql) in IMAGERY {
        if tx
            .query_opt(
                "SELECT 1 FROM imagery.schema_migration WHERE version=$1",
                &[&version],
            )
            .await?
            .is_some()
        {
            continue;
        }
        tx.batch_execute(sql).await?;
        tx.execute(
            "INSERT INTO imagery.schema_migration(version) VALUES($1)",
            &[&version],
        )
        .await?;
    }
    Ok(())
}

/// Trusted local bootstrap, called only after local_demo_scope admits the configured database.
/// Uses the hosted owner's existing finance ledger and lock, then the actual media installer
/// and shared imagery installer. It never invents accounts, campaigns or gameplay identities.
/// The caller commits once after success, or rolls back all schemas and ledgers after an error.
pub async fn migrate_local_in(tx: &Transaction<'_>) -> Result<(), tokio_postgres::Error> {
    tx.batch_execute(
        "SELECT pg_advisory_xact_lock(7000021); \
         CREATE SCHEMA IF NOT EXISTS hosted; \
         CREATE TABLE IF NOT EXISTS hosted.migration (\
             number integer PRIMARY KEY, \
             applied_at timestamptz NOT NULL DEFAULT clock_timestamp())",
    )
    .await?;
    crate::media_catalog::migrate_in(tx).await?;
    for (number, sql) in FINANCE {
        if tx
            .query_opt("SELECT 1 FROM hosted.migration WHERE number=$1", &[&number])
            .await?
            .is_some()
        {
            continue;
        }
        tx.batch_execute(sql).await?;
        tx.execute(
            "INSERT INTO hosted.migration(number) VALUES($1)",
            &[&number],
        )
        .await?;
    }
    migrate_in(tx).await
}
