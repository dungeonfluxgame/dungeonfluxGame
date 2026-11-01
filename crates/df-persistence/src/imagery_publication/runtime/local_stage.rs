//! Local stage admission verifies durable claims while preserving the one publication anchor.
use super::{conflict, publication_error};
use crate::imagery_publication::rows;
use df_imagery::owner_scope_bytes;
use df_imagery::runtime::{DispatchAdmission, RuntimeError};
use sha2::{Digest, Sha256};
use tokio_postgres::{Row, Transaction};

pub(super) async fn check_claim_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    now_ms: i64,
) -> Result<bool, RuntimeError> {
    let job = tx
        .query_opt(
            "SELECT * FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
            &[
                &input.job.job.as_bytes().as_slice(),
                &owner_scope_bytes(&input.job.owner),
            ],
        )
        .await
        .map_err(|error| publication_error(rows::database_error(error)))?
        .ok_or(RuntimeError::InvalidRecord)?;
    let stage=tx.query_opt("SELECT * FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2 AND owner_scope=$3 FOR UPDATE", &[&input.job.job.as_bytes().as_slice(),&input.stage.stage.as_bytes().as_slice(),&owner_scope_bytes(&input.job.owner)]).await.map_err(|error|publication_error(rows::database_error(error)))?.ok_or(RuntimeError::InvalidRecord)?;
    if job.get::<_, Vec<u8>>("intent_bytes") != input.job.intent.bytes()
        || job.get::<_, i64>("intent_revision") != i64::from(input.job.intent.revision())
        || job.get::<_, Vec<u8>>("config_bytes") != input.job.config.bytes()
        || job.get::<_, i64>("config_revision") != i64::from(input.job.config.revision())
        || job.get::<_, String>("state") == "retired"
        || job.get::<_, i64>("absolute_deadline_ms") <= now_ms
        || stage.get::<_, String>("stage_kind") != input.stage.kind.label()
        || stage.get::<_, Vec<u8>>("config_digest")
            != <[u8; 32]>::from(Sha256::digest(input.job.config.bytes()))
        || !stage.get::<_, bool>("local_accounting")
        || stage.get::<_, Vec<u8>>("claim_token") != input.claim.as_bytes()
        || stage.get::<_, i64>("claim_generation")
            != i64::try_from(input.stage.claim_generation)
                .map_err(|_| RuntimeError::InvalidRecord)?
        || stage.get::<_, i64>("lease_until_ms") < now_ms
    {
        return Err(conflict());
    }
    let demand:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_interest WHERE job_id=$1 AND state='attached')", &[&input.job.job.as_bytes().as_slice()]).await.map_err(|error|publication_error(rows::database_error(error)))?.get(0);
    if !demand {
        return Err(conflict());
    }
    if input.stage.kind.is_standalone() {
        if input.stage.planned_publication.is_some()
            || job.get::<_, String>("state") != "planning"
            || job
                .get::<_, Option<Vec<u8>>>("planned_publication_asset")
                .is_some()
            || job
                .get::<_, Option<Vec<u8>>>("planned_publication_attempt")
                .is_some()
        {
            return Err(conflict());
        }
        return Ok(false);
    }
    let planned = input
        .stage
        .planned_publication
        .ok_or(RuntimeError::InvalidRecord)?;
    if job
        .get::<_, Option<Vec<u8>>>("planned_publication_asset")
        .as_deref()
        != Some(planned.asset.as_slice())
        || job
            .get::<_, Option<Vec<u8>>>("planned_publication_attempt")
            .as_deref()
            != Some(planned.attempt.as_slice())
    {
        return Err(conflict());
    }
    if input.stage.kind == df_imagery::execution::StageKind::Classification {
        if job.get::<_, String>("state") != "planning" {
            return Err(conflict());
        }
        return Ok(false);
    }
    let is_anchor = job
        .get::<_, Option<Vec<u8>>>("planned_output_stage")
        .as_deref()
        == Some(input.stage.stage.as_bytes().as_slice());
    if !is_anchor {
        check_existing_allocation(tx, input, &job).await?;
    }
    Ok(is_anchor)
}

async fn check_existing_allocation(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    job: &Row,
) -> Result<(), RuntimeError> {
    let anchor = job
        .get::<_, Option<Vec<u8>>>("planned_output_stage")
        .ok_or(RuntimeError::InvalidRecord)?;
    let planned = input
        .stage
        .planned_publication
        .ok_or(RuntimeError::InvalidRecord)?;
    let allocation=tx.query_opt("SELECT * FROM imagery.image_publication_allocation WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND asset_id=$4 AND publication_attempt=$5", &[&owner_scope_bytes(&input.job.owner),&input.job.job.as_bytes().as_slice(),&anchor,&planned.asset.as_slice(),&planned.attempt.as_slice()]).await.map_err(|error|publication_error(rows::database_error(error)))?.ok_or(RuntimeError::InvalidRecord)?;
    if allocation.get::<_, Vec<u8>>("original_recipe_key")
        != job
            .get::<_, Option<Vec<u8>>>("resolved_recipe_key")
            .ok_or(RuntimeError::InvalidRecord)?
        || allocation.get::<_, i64>("original_recipe_revision")
            != job
                .get::<_, Option<i64>>("recipe_revision")
                .ok_or(RuntimeError::InvalidRecord)?
        || allocation.get::<_, Vec<u8>>("original_recipe_bytes")
            != job
                .get::<_, Option<Vec<u8>>>("recipe_bytes")
                .ok_or(RuntimeError::InvalidRecord)?
        || allocation.get::<_, i64>("original_config_revision")
            != job.get::<_, i64>("config_revision")
        || allocation.get::<_, Vec<u8>>("original_config_bytes")
            != job.get::<_, Vec<u8>>("config_bytes")
    {
        return Err(conflict());
    }
    Ok(())
}
