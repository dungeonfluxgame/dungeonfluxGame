//! Generic finance APIs preserve the canonical stage protocol and money-lock order.
use std::collections::BTreeSet;

use df_commerce::finance_ids::{AttemptId, ReservationId, UsageEventId, unhex};
use df_commerce::plan::{ChangeKind, FinancePlan, Value};
use tokio_postgres::Transaction;

use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{FinanceError, FinanceService, bad_sql};

pub(crate) struct LockedStagePlan {
    dependencies: serde_json::Value,
    attempts: Vec<Vec<u8>>,
    inserted: BTreeSet<AttemptId>,
    selectors: Selectors,
}
struct Selectors {
    attempts: Vec<Vec<u8>>,
    reservations: Vec<Vec<u8>>,
    usages: Vec<Vec<u8>>,
}
impl Selectors {
    fn from_plan(plan: &FinancePlan) -> Result<(Self, BTreeSet<AttemptId>), FinanceError> {
        let mut attempts = BTreeSet::new();
        let mut reservations = BTreeSet::new();
        let mut usages = BTreeSet::new();
        let mut inserted = BTreeSet::new();
        for change in &plan.changes {
            if let Some(Value::Bytes(bytes)) = change.row.get("attempt_id") {
                let attempt = AttemptId::from_slice(bytes).ok_or_else(|| bad_sql("attempt id"))?;
                attempts.insert(attempt);
                if change.kind == ChangeKind::ProviderAttemptInsert {
                    inserted.insert(attempt);
                }
            }
            if let Some(Value::Bytes(bytes)) = change.row.get("reservation_id") {
                reservations.insert(
                    ReservationId::from_slice(bytes).ok_or_else(|| bad_sql("reservation id"))?,
                );
            }
            if let Some(Value::Bytes(bytes)) = change.row.get("usage_event_id") {
                usages.insert(
                    UsageEventId::from_slice(bytes).ok_or_else(|| bad_sql("usage event id"))?,
                );
            }
        }
        Ok((
            Self {
                attempts: attempts.iter().map(|id| id.as_bytes().to_vec()).collect(),
                reservations: reservations
                    .iter()
                    .map(|id| id.as_bytes().to_vec())
                    .collect(),
                usages: usages.iter().map(|id| id.as_bytes().to_vec()).collect(),
            },
            inserted,
        ))
    }
    async fn resolve(
        &self,
        tx: &Transaction<'_>,
        lock: bool,
    ) -> Result<serde_json::Value, FinanceError> {
        let sql = if lock {
            "SELECT finance.lock_image_stage_plan_targets($1,$2,$3)::text"
        } else {
            "SELECT finance.image_stage_plan_targets($1,$2,$3)::text"
        };
        let text: String = tx
            .query_one(sql, &[&self.attempts, &self.reservations, &self.usages])
            .await?
            .get(0);
        serde_json::from_str(&text).map_err(|_| bad_sql("stage plan targets"))
    }
}
fn target_attempts(value: &serde_json::Value) -> Result<Vec<Vec<u8>>, FinanceError> {
    value
        .get("attempts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| bad_sql("stage plan attempts"))?
        .iter()
        .map(|value| {
            let bytes = value
                .as_str()
                .and_then(unhex)
                .ok_or_else(|| bad_sql("stage plan attempt"))?;
            AttemptId::from_slice(&bytes).ok_or_else(|| bad_sql("stage plan attempt"))?;
            Ok(bytes)
        })
        .collect()
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub(crate) async fn refuse_stage_protocol_bypass(
        tx: &Transaction<'_>,
        attempt: AttemptId,
    ) -> Result<(), FinanceError> {
        let protected: bool = tx
            .query_one(
                "SELECT finance.image_stage_protocol_required($1)",
                &[&attempt.as_bytes().as_slice()],
            )
            .await?
            .get(0);
        if protected {
            return Err(FinanceError::ImageStageConflict);
        }
        Ok(())
    }
    pub(crate) async fn guard_recorded_stage_plan(
        tx: &Transaction<'_>,
        plan: &FinancePlan,
    ) -> Result<LockedStagePlan, FinanceError> {
        let (selectors, inserted) = Selectors::from_plan(plan)?;
        let dependencies = selectors.resolve(tx, true).await?;
        let attempts = target_attempts(&dependencies)?;
        for bytes in &attempts {
            Self::refuse_stage_protocol_bypass(
                tx,
                AttemptId::from_slice(bytes).ok_or_else(|| bad_sql("stage plan attempt"))?,
            )
            .await?;
        }
        Ok(LockedStagePlan {
            dependencies,
            attempts,
            inserted,
            selectors,
        })
    }
    pub(crate) async fn recheck_recorded_stage_plan(
        tx: &Transaction<'_>,
        locked: &LockedStagePlan,
    ) -> Result<(), FinanceError> {
        // No earlier-rank lock is acquired after commit_plan's ledger/balance locks.
        let original = Selectors {
            attempts: locked.attempts.clone(),
            reservations: Vec::new(),
            usages: Vec::new(),
        };
        if original.resolve(tx, false).await? != locked.dependencies {
            return Err(FinanceError::Stale);
        }
        let current = locked.selectors.resolve(tx, false).await?;
        for bytes in target_attempts(&current)? {
            let attempt =
                AttemptId::from_slice(&bytes).ok_or_else(|| bad_sql("stage plan attempt"))?;
            if !locked.attempts.contains(&bytes) {
                if !locked.inserted.contains(&attempt) {
                    return Err(FinanceError::Stale);
                }
                let owned: bool = tx.query_one("SELECT xmin=pg_current_xact_id()::xid FROM finance.provider_attempt WHERE attempt_id=$1", &[&bytes]).await?.get(0);
                if !owned {
                    return Err(FinanceError::Stale);
                }
            }
            Self::refuse_stage_protocol_bypass(tx, attempt).await?;
        }
        Ok(())
    }
}
