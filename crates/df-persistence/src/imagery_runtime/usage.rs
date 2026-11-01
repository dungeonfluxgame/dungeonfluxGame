//! Append-only raw usage observations; acknowledgements never erase supplier evidence.

use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;
use df_imagery::cost::{CostRoute, CostUnits, UsageEvidence, UsageRevision};
use df_imagery::execution::{DispatchId, ExecutionStageId};
use df_imagery::runtime::{ImageUsageJournal, RuntimeError, RuntimeUsage};
use df_imagery::{ImageJobId, ImageOwner, owner_of_bytes, owner_scope_bytes};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_postgres::{Config, Row};

pub const IMAGERY_USAGE_MIGRATION_VERSION: i32 = 44;
pub const IMAGERY_USAGE_MIGRATION_SQL: &str =
    include_str!("../../migrations/0044_imagery_usage.sql");

/// Raw usage storage over a migrated PostgreSQL runtime database.
#[derive(Clone)]
pub struct PostgresImageUsage {
    pool: Arc<Pool>,
}
impl PostgresImageUsage {
    pub fn open_existing(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, RuntimeError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        Ok(Self {
            pool: Arc::new(Pool::new(config, transport, connections)),
        })
    }
}

pub(super) fn storage(_: tokio_postgres::Error) -> RuntimeError {
    RuntimeError::StorageUnavailable
}
pub(super) fn invalid<T>(result: Result<T, impl std::fmt::Debug>) -> Result<T, RuntimeError> {
    result.map_err(|_| RuntimeError::InvalidRecord)
}
fn encode_units(units: &[CostUnits]) -> Result<String, RuntimeError> {
    if units.len() > 32
        || units
            .iter()
            .any(|unit| unit.category.is_empty() || unit.category.len() > 32)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(Value::Array(
        units
            .iter()
            .map(|unit| json!({"category":unit.category,"quantity":unit.quantity}))
            .collect(),
    )
    .to_string())
}
pub(super) fn decode_units(text: &str) -> Result<Vec<CostUnits>, RuntimeError> {
    let value: Value = invalid(serde_json::from_str(text))?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= 32)
        .ok_or(RuntimeError::InvalidRecord)?;
    values
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .filter(|object| object.len() == 2)
                .ok_or(RuntimeError::InvalidRecord)?;
            let category = object
                .get("category")
                .and_then(Value::as_str)
                .filter(|category| !category.is_empty() && category.len() <= 32)
                .ok_or(RuntimeError::InvalidRecord)?;
            let quantity = object
                .get("quantity")
                .and_then(Value::as_u64)
                .ok_or(RuntimeError::InvalidRecord)?;
            Ok(CostUnits {
                category: category.to_owned(),
                quantity,
            })
        })
        .collect()
}
fn decode(row: &Row) -> Result<RuntimeUsage, RuntimeError> {
    let owner =
        owner_of_bytes(&row.get::<_, Vec<u8>>("owner_scope")).ok_or(RuntimeError::InvalidRecord)?;
    let evidence = match row.get::<_, String>("evidence").as_str() {
        "reported" => UsageEvidence::Reported,
        "estimated" => UsageEvidence::Estimated,
        "unknown" => UsageEvidence::Unknown,
        _ => return Err(RuntimeError::InvalidRecord),
    };
    Ok(RuntimeUsage {
        owner,
        job: invalid(ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id")))?,
        stage: invalid(ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("stage_id"),
        ))?,
        dispatch: invalid(DispatchId::from_bytes(
            &row.get::<_, Vec<u8>>("dispatch_id"),
        ))?,
        revision: UsageRevision(invalid(u64::try_from(row.get::<_, i64>("revision")))?),
        route: CostRoute {
            provider: row.get("provider"),
            model: row.get("model"),
            cause: row.get("cause"),
            detail: row.get("detail"),
            maxima: decode_units(&row.get::<_, String>("maxima_text"))?,
        },
        evidence,
        units: decode_units(&row.get::<_, String>("units_text"))?,
        usable: row.get("usable"),
        at_ms: invalid(u64::try_from(row.get::<_, i64>("observed_at_ms")))?,
        raw_output: row.get("raw_output"),
    })
}
const COLUMNS: &str = "u.*,u.maxima::text AS maxima_text,u.units::text AS units_text";

impl ImageUsageJournal for PostgresImageUsage {
    async fn record_sources(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
        sources: &[df_imagery::runtime::SourceReceipt],
    ) -> Result<(), RuntimeError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(storage)?;
        super::sources::record_sources_in(&tx, scope, sources).await?;
        tx.commit().await.map_err(storage)
    }
    async fn read_sources(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
    ) -> Result<Option<Vec<df_imagery::runtime::SourceReceipt>>, RuntimeError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(storage)?;
        let sources = super::sources::read_sources_in(&tx, scope).await?;
        tx.commit().await.map_err(storage)?;
        Ok(sources)
    }

    async fn record_usage(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        let maxima = encode_units(&usage.route.maxima)?;
        let units = encode_units(&usage.units)?;
        let revision = invalid(i64::try_from(usage.revision.0))?;
        let at_ms = invalid(i64::try_from(usage.at_ms))?;
        if revision <= 0
            || usage.route.maxima.is_empty()
            || usage.evidence == UsageEvidence::Unknown && usage.units != usage.route.maxima
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
            &[&usage.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(storage)?;
        let dispatch = tx.query_opt("SELECT j.owner_scope,d.job_id,d.stage_id FROM imagery.image_dispatch_attempt d JOIN imagery.image_job j ON j.job_id=d.job_id WHERE d.dispatch_id=$1",
            &[&usage.dispatch.as_bytes().as_slice()]).await.map_err(storage)?.ok_or(RuntimeError::InvalidRecord)?;
        if dispatch.get::<_, Vec<u8>>("owner_scope") != owner_scope_bytes(&usage.owner)
            || dispatch.get::<_, Vec<u8>>("job_id") != usage.job.as_bytes()
            || dispatch.get::<_, Vec<u8>>("stage_id") != usage.stage.as_bytes()
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let query = format!(
            "SELECT {COLUMNS} FROM imagery.image_dispatch_usage u WHERE dispatch_id=$1 ORDER BY revision DESC LIMIT 1"
        );
        if let Some(row) = tx
            .query_opt(&query, &[&usage.dispatch.as_bytes().as_slice()])
            .await
            .map_err(storage)?
        {
            let existing = decode(&row)?;
            if existing.revision == usage.revision && existing == *usage {
                return Ok(());
            }
            if existing.revision >= usage.revision || existing.route != usage.route {
                return Err(RuntimeError::InvalidRecord);
            }
        }
        tx.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms,raw_output) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::jsonb,$11,$12::text::jsonb,$13,$14,$15)",
            &[&usage.dispatch.as_bytes().as_slice(), &owner_scope_bytes(&usage.owner), &usage.job.as_bytes().as_slice(),
            &usage.stage.as_bytes().as_slice(), &revision, &usage.route.provider, &usage.route.model, &usage.route.cause,
            &usage.route.detail, &maxima, &usage.evidence.label(), &units, &usage.usable, &at_ms, &usage.raw_output]).await.map_err(storage)?;
        tx.commit()
            .await
            .map_err(|_| RuntimeError::Execution(df_imagery::execution::ExecutionError::Unknown))
    }
    async fn latest_usage(
        &self,
        owner: &ImageOwner,
        dispatch: DispatchId,
    ) -> Result<Option<RuntimeUsage>, RuntimeError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let query = format!(
            "SELECT {COLUMNS} FROM imagery.image_dispatch_usage u WHERE dispatch_id=$1 AND owner_scope=$2 ORDER BY revision DESC LIMIT 1"
        );
        client
            .query_opt(
                &query,
                &[&dispatch.as_bytes().as_slice(), &owner_scope_bytes(owner)],
            )
            .await
            .map_err(storage)?
            .as_ref()
            .map(decode)
            .transpose()
    }
    async fn read_usage_revision(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
        revision: UsageRevision,
    ) -> Result<Option<RuntimeUsage>, RuntimeError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(storage)?;
        let usage = read_usage_in(&tx, scope, revision).await?;
        tx.commit().await.map_err(storage)?;
        Ok(usage)
    }

    async fn pending_usage(
        &self,
        scope: &str,
        limit: u32,
    ) -> Result<Vec<RuntimeUsage>, RuntimeError> {
        if limit == 0 || scope.is_empty() || scope.len() > 128 {
            return Err(RuntimeError::InvalidRecord);
        }
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let query = format!(
            "SELECT {COLUMNS} FROM imagery.image_dispatch_usage u JOIN imagery.image_execution_stage s ON s.job_id=u.job_id AND s.stage_id=u.stage_id WHERE s.capacity_scope=$1 AND NOT u.projection_durable AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage newer WHERE newer.dispatch_id=u.dispatch_id AND newer.revision>u.revision) ORDER BY u.observed_at_ms,u.dispatch_id,u.revision LIMIT $2"
        );
        client
            .query(&query, &[&scope, &i64::from(limit)])
            .await
            .map_err(storage)?
            .iter()
            .map(decode)
            .collect()
    }
    async fn acknowledge_projection(
        &self,
        owner: &ImageOwner,
        dispatch: DispatchId,
        revision: UsageRevision,
    ) -> Result<(), RuntimeError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let changed = client.execute("UPDATE imagery.image_dispatch_usage SET projection_durable=true WHERE dispatch_id=$1 AND revision=$2 AND owner_scope=$3",
            &[&dispatch.as_bytes().as_slice(), &invalid(i64::try_from(revision.0))?, &owner_scope_bytes(owner)])
            .await.map_err(storage)?;
        if changed != 1 {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(())
    }
}

/// Confirms the immutable raw usage revision before execution selects a decision checkpoint.
pub async fn usage_recorded_in(
    tx: &tokio_postgres::Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    revision: UsageRevision,
) -> Result<bool, RuntimeError> {
    let revision = invalid(i64::try_from(revision.0))?;
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_dispatch_usage WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4 AND revision=$5 AND raw_output IS NOT NULL AND octet_length(raw_output)>0",
        &[&dispatch.as_bytes().as_slice(),&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&revision])
        .await.map_err(storage)?.is_some())
}

/// Matches the exact scoped encoded output without copying the retained supplier bytes.
pub async fn usage_output_matches_in(
    tx: &tokio_postgres::Transaction<'_>,
    scope: &df_imagery::execution::DecisionScope,
    revision: UsageRevision,
    byte_len: u64,
    sha256: &[u8; 32],
) -> Result<bool, RuntimeError> {
    let revision = invalid(i64::try_from(revision.0))?;
    let byte_len = invalid(i64::try_from(byte_len))?;
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_dispatch_usage WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4 AND revision=$5 AND octet_length(raw_output)::bigint=$6 AND sha256(raw_output)=$7",
        &[&scope.dispatch.as_bytes().as_slice(), &owner_scope_bytes(&scope.owner), &scope.job.as_bytes().as_slice(), &scope.stage.as_bytes().as_slice(), &revision, &byte_len, &sha256.as_slice()])
        .await.map_err(storage)?.is_some())
}

/// Requires an actual non-unknown unusable observation before selecting a failed usage revision.
pub async fn usage_definitive_failure_in(
    tx: &tokio_postgres::Transaction<'_>,
    scope: &df_imagery::execution::ExecutionScope,
    revision: UsageRevision,
) -> Result<bool, RuntimeError> {
    let revision = invalid(i64::try_from(revision.0))?;
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_dispatch_usage WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4 AND revision=$5 AND evidence IN ('reported','estimated') AND NOT usable",
        &[&scope.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&revision])
        .await.map_err(storage)?.is_some())
}

/// Reads one exact retained raw observation for bounded semantic qualification outside catalog locks.
pub async fn read_usage_in(
    tx: &tokio_postgres::Transaction<'_>,
    scope: &df_imagery::execution::ExecutionScope,
    revision: UsageRevision,
) -> Result<Option<RuntimeUsage>, RuntimeError> {
    let revision = invalid(i64::try_from(revision.0))?;
    let query = format!(
        "SELECT {COLUMNS} FROM imagery.image_dispatch_usage u WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4 AND revision=$5"
    );
    tx.query_opt(
        &query,
        &[
            &scope.dispatch.as_bytes().as_slice(),
            &owner_scope_bytes(&scope.owner),
            &scope.job.as_bytes().as_slice(),
            &scope.stage.as_bytes().as_slice(),
            &revision,
        ],
    )
    .await
    .map_err(storage)?
    .as_ref()
    .map(decode)
    .transpose()
}

/// Confirms uncertainty at the exact selected raw revision without changing retained liability.
pub async fn usage_unknown_in(
    tx: &tokio_postgres::Transaction<'_>,
    scope: &df_imagery::execution::ExecutionScope,
    revision: UsageRevision,
) -> Result<bool, RuntimeError> {
    let revision = invalid(i64::try_from(revision.0))?;
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_dispatch_usage WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4 AND revision=$5 AND evidence='unknown'",
        &[&scope.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&revision])
        .await.map_err(storage)?.is_some())
}
