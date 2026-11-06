//! Scoped coherent projection of durable execution state for authenticated service reads.
use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;
use df_imagery::{CreatorAssessment, CreatorCategory, CreatorRoute};
use df_imagery::{
    ImageExecutionState, ImageJobId, ImageObservations, ImageOwner, ImageProgress, JournalError,
    owner_scope_bytes,
};
use std::sync::Arc;
use tokio_postgres::Config;

/// Opens the already-migrated journal read path; it performs no DDL or dispatch.
#[derive(Clone)]
pub struct PostgresImageObservations {
    pool: Arc<Pool>,
    usage: Option<UsageReader>,
    inspection: Arc<tokio::sync::Semaphore>,
}
#[derive(Clone)]
struct UsageReader {
    costs: Arc<dyn df_imagery::cost::ImageCostProjection>,
    limits: df_imagery::runtime::UsageProjectionLimits,
}
impl PostgresImageObservations {
    /// Uses the same live cost adapter as execution; actual amounts are never repriced here.
    pub fn with_usage_projection(
        mut self,
        costs: Arc<dyn df_imagery::cost::ImageCostProjection>,
        limits: df_imagery::runtime::UsageProjectionLimits,
    ) -> Result<Self, JournalError> {
        limits.validate().map_err(|_| JournalError::InvalidRecord)?;
        self.usage = Some(UsageReader { costs, limits });
        Ok(self)
    }

    pub fn open_existing(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, JournalError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(|_| JournalError::Unavailable)?;
        if connections == 0 {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            pool: Arc::new(Pool::new(config, transport, connections)),
            usage: None,
            inspection: Arc::new(tokio::sync::Semaphore::new(connections)),
        })
    }
}
impl ImageObservations for PostgresImageObservations {
    async fn read_analysis(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageAnalysisObservation>, JournalError> {
        let permit = self
            .inspection
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let retained = {
            let mut slot = self
                .pool
                .checkout()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
            let tx = client
                .build_transaction()
                .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
                .start()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            tx.query_opt("SELECT revision FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR SHARE",
                &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(|_|JournalError::Unavailable)?;
            let record = crate::imagery_store::read_in(&tx, owner, job).await?;
            let checkpoint =
                crate::imagery_execution::read_analysis_checkpoint_in(&tx, owner, &job)
                    .await
                    .map_err(|_| JournalError::Unavailable)?;
            let checkpoint = if let Some(checkpoint) = checkpoint {
                let selected_at_ms: i64 = tx.query_one(
                    "SELECT selected_at_ms FROM imagery.image_analysis_checkpoint WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND checkpoint_version=$4 AND result_sha256=$5",
                    &[&owner_scope_bytes(owner), &job.as_bytes().as_slice(), &checkpoint.scope.stage.as_bytes().as_slice(), &i64::from(checkpoint.checkpoint_version), &checkpoint.result_sha256.as_slice()],
                ).await.map_err(|_|JournalError::Unavailable)?.get(0);
                if selected_at_ms <= 0 {
                    return Err(JournalError::InvalidRecord);
                }
                Some((checkpoint, selected_at_ms))
            } else {
                None
            };
            tx.commit().await.map_err(|_| JournalError::Unavailable)?;
            match (record, checkpoint) {
                (Some(record), Some(checkpoint)) => Some((record, checkpoint)),
                (None, None) | (Some(_), None) => None,
                (None, Some(_)) => return Err(JournalError::InvalidRecord),
            }
        };
        let Some((record, (checkpoint, selected_at_ms))) = retained else {
            return Ok(None);
        };
        let revision = record.revision;
        if checkpoint.scope.owner != record.owner || checkpoint.scope.job != record.job {
            return Err(JournalError::InvalidRecord);
        }
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            df_imagery::runtime::analysis_result(&checkpoint)
        })
        .await
        .map_err(|_| JournalError::Unavailable)?
        .map_err(|_| JournalError::InvalidRecord)?;
        Ok(Some(df_imagery::ImageAnalysisObservation {
            job_revision: revision,
            selected_at_ms,
            result,
        }))
    }
    async fn read_inspection(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageInspectionObservation>, JournalError> {
        let permit = self
            .inspection
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let retained = {
            let mut slot = self
                .pool
                .checkout()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
            let tx = client
                .build_transaction()
                .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
                .start()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            let retained =
                crate::imagery_publication::read_published_inspection_in(&tx, owner, &job)
                    .await
                    .map_err(|_| JournalError::Unavailable)?;
            tx.commit().await.map_err(|_| JournalError::Unavailable)?;
            retained
        };
        let Some(retained) = retained else {
            return Ok(None);
        };
        let revision = retained.job.revision;
        let assessment = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match retained.evidence {
                crate::imagery_publication::PublishedInspectionEvidence::Decision(decision) => {
                    df_imagery::runtime::project_inspection_decision(&retained.job, &decision)
                }
                crate::imagery_publication::PublishedInspectionEvidence::Local(local) => {
                    df_imagery::runtime::project_local_inspection(&retained.job, &local)
                }
            }
        })
        .await
        .map_err(|_| JournalError::Unavailable)?
        .map_err(|_| JournalError::InvalidRecord)?;
        Ok(Some(df_imagery::ImageInspectionObservation {
            job_revision: revision,
            assessment,
        }))
    }
    async fn read_usage(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageUsageObservation>, JournalError> {
        let Some(reader) = &self.usage else {
            return Ok(None);
        };
        let rows = {
            let mut slot = self
                .pool
                .checkout()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
            let tx = client
                .build_transaction()
                .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
                .start()
                .await
                .map_err(|_| JournalError::Unavailable)?;
            let rows =
                super::usage_projection::read_usage_in(&tx, owner, job, reader.limits).await?;
            tx.commit().await.map_err(|_| JournalError::Unavailable)?;
            rows
        };
        let Some(rows) = rows else { return Ok(None) };
        let job_revision = rows.job_revision;
        let summary = super::usage_projection::project_rows(
            rows,
            owner,
            job,
            reader.costs.as_ref(),
            reader.limits,
        )
        .await?;
        Ok(Some(df_imagery::ImageUsageObservation {
            job_revision,
            summary,
        }))
    }

    async fn read_progress(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
        now_ms: i64,
    ) -> Result<Option<ImageProgress>, JournalError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
        let row = client.query_opt(concat!(
            "WITH RECURSIVE lineage AS (SELECT s.*,0 AS depth FROM imagery.image_execution_stage s ",
            "WHERE s.job_id=$1 AND s.parent_stage_id IS NULL UNION ALL SELECT s.*,p.depth+1 ",
            "FROM imagery.image_execution_stage s JOIN lineage p ON s.parent_stage_id=p.stage_id ",
            "AND s.job_id=p.job_id) SELECT j.revision,j.state,j.absolute_deadline_ms, ",
            "s.stage_state,s.stage_kind,s.queued_notice,s.queued_not_before_ms,s.retry_dispatch_id,d.dispatch_state,d.failure_cause,classification.payload AS classification_payload,n.reason AS no_delivery_reason, ",
            "EXISTS(SELECT 1 FROM imagery.image_job_result r JOIN media.asset a ON a.asset_id=r.asset_id ",
            "AND a.attempt=r.publication_attempt WHERE r.job_id=j.job_id AND NOT r.private_reconciliation ",
            "AND a.state='ready') AS ready FROM imagery.image_job j LEFT JOIN imagery.image_no_delivery n ",
            "ON n.job_id=j.job_id AND n.owner_scope=j.owner_scope LEFT JOIN LATERAL ",
            "(SELECT * FROM lineage ORDER BY depth DESC,(stage_kind='local_transform') DESC, ",
            "created_at_ms DESC,stage_id DESC LIMIT 1) s ON true LEFT JOIN imagery.image_stage_cursor c ",
            "ON c.job_id=s.job_id AND c.stage_id=s.stage_id LEFT JOIN imagery.image_dispatch_attempt d ",
            "ON d.dispatch_id=c.current_dispatch LEFT JOIN LATERAL (SELECT o.payload FROM ",
            "imagery.image_decision_observation o JOIN imagery.image_decision_checkpoint p ",
            "ON p.dispatch_id=o.dispatch_id AND p.observation_version=o.observation_version ",
            "WHERE o.job_id=j.job_id AND o.owner_scope=j.owner_scope AND o.decision_kind='classification' ",
            "ORDER BY o.observed_at_ms DESC,o.dispatch_id DESC LIMIT 1) classification ON true ",
            "WHERE j.job_id=$1 AND j.owner_scope=$2"),
            &[&job.as_bytes().as_slice(), &owner_scope_bytes(owner)]).await
            .map_err(|_| JournalError::Unavailable)?;
        let Some(row) = row else { return Ok(None) };
        let revision = u64::try_from(row.get::<_, i64>("revision"))
            .map_err(|_| JournalError::InvalidRecord)?;
        let dispatch: Option<String> = row.get("dispatch_state");
        let retry_pending = row.get::<_, Option<Vec<u8>>>("retry_dispatch_id").is_some();
        let failure = row
            .get::<_, Option<String>>("failure_cause")
            .map(|label| {
                df_imagery::execution::FailureCause::from_key(&label)
                    .ok_or(JournalError::InvalidRecord)
            })
            .transpose()?
            .filter(|_| !retry_pending);
        let assessment = row
            .get::<_, Option<Vec<u8>>>("classification_payload")
            .as_deref()
            .map(parse_assessment)
            .transpose()?;
        let state = if let Some(reason) = row.get::<_, Option<String>>("no_delivery_reason") {
            match reason.as_str() {
                "creator_needs_revision" => ImageExecutionState::NeedsRevision,
                "creator_uncertain" => ImageExecutionState::Uncertain,
                "supplier_declined" => ImageExecutionState::Unknown,
                "expired" => ImageExecutionState::Expired,
                "retired" => ImageExecutionState::Withdrawn,
                "supplier_no_output" => ImageExecutionState::Failed {
                    cause: failure.ok_or(JournalError::InvalidRecord)?,
                },
                "missing_credential" | "unsupported_profile" | "source_denied" => {
                    ImageExecutionState::Failed {
                        cause: df_imagery::execution::FailureCause::Refused,
                    }
                }
                "inspection_rejected"
                | "required_inspection_unjudged"
                | "required_inspection_preparation"
                | "required_inspection_disabled" => ImageExecutionState::Failed {
                    cause: df_imagery::execution::FailureCause::InvalidOutput,
                },
                _ => return Err(JournalError::InvalidRecord),
            }
        } else if row.get::<_, bool>("ready") {
            ImageExecutionState::Ready
        } else if row.get::<_, String>("state") == "retired" {
            ImageExecutionState::Withdrawn
        } else {
            match dispatch.as_deref() {
                Some("declined") if retry_pending => ImageExecutionState::Queued,
                Some("declined" | "unknown") => ImageExecutionState::Unknown,
                Some("failed") => ImageExecutionState::Failed {
                    cause: crate::imagery_runtime::observations::failure(
                        row.get::<_, Option<String>>("failure_cause").as_deref(),
                    )?,
                },
                Some("sent") => ImageExecutionState::Running,
                Some("prepared") => ImageExecutionState::Queued,
                Some("succeeded")
                    if row.get::<_, Option<String>>("stage_kind").as_deref()
                        == Some("classification") =>
                {
                    match assessment.map(|assessment| assessment.route) {
                        Some(CreatorRoute::Revise) => ImageExecutionState::NeedsRevision,
                        Some(CreatorRoute::Uncertain) => ImageExecutionState::Uncertain,
                        _ => ImageExecutionState::Queued,
                    }
                }
                Some("succeeded") => ImageExecutionState::Publishing,
                Some("released") => ImageExecutionState::Withdrawn,
                Some(_) => return Err(JournalError::InvalidRecord),
                None if now_ms >= row.get::<_, i64>("absolute_deadline_ms") => {
                    ImageExecutionState::Expired
                }
                None => {
                    if row.get::<_, Option<String>>("stage_state").is_some() {
                        ImageExecutionState::Queued
                    } else {
                        ImageExecutionState::Planning
                    }
                }
            }
        };
        let operational_notice = if state == ImageExecutionState::Queued && dispatch.is_none() {
            row.get::<_, Option<String>>("queued_notice")
                .map(|label| {
                    let reason = df_imagery::execution::QuotaDeferralReason::from_key(&label)
                        .ok_or(JournalError::InvalidRecord)?;
                    let retry_at_ms = row
                        .get::<_, Option<i64>>("queued_not_before_ms")
                        .ok_or(JournalError::InvalidRecord)?;
                    if retry_at_ms <= 0 {
                        return Err(JournalError::InvalidRecord);
                    }
                    Ok(df_imagery::execution::QueueDeferralNotice {
                        reason,
                        retry_at_ms,
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(Some(ImageProgress {
            revision,
            state,
            failure,
            operational_notice,
            assessment,
        }))
    }
}
fn failure(label: Option<&str>) -> Result<df_imagery::execution::FailureCause, JournalError> {
    label
        .and_then(df_imagery::execution::FailureCause::from_key)
        .ok_or(JournalError::InvalidRecord)
}

fn parse_assessment(bytes: &[u8]) -> Result<CreatorAssessment, JournalError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| JournalError::InvalidRecord)?;
    let object = value
        .as_object()
        .filter(|object| object.len() == 4)
        .ok_or(JournalError::InvalidRecord)?;
    if object
        .get("schema_revision")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err(JournalError::InvalidRecord);
    }
    let category = object
        .get("category")
        .and_then(serde_json::Value::as_str)
        .and_then(CreatorCategory::from_key)
        .ok_or(JournalError::InvalidRecord)?;
    let route = object
        .get("route")
        .and_then(serde_json::Value::as_str)
        .and_then(CreatorRoute::from_key)
        .ok_or(JournalError::InvalidRecord)?;
    let contract_revision = object
        .get("contract_revision")
        .and_then(serde_json::Value::as_u64)
        .and_then(|revision| u32::try_from(revision).ok())
        .filter(|revision| *revision > 0)
        .ok_or(JournalError::InvalidRecord)?;
    Ok(CreatorAssessment {
        category,
        route,
        contract_revision,
    })
}
