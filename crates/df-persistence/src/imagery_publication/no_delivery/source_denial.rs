//! Current locked source refusals close only the actual output allocation, never supplier liability.
use super::{commit_in, evidence, rows};
use crate::imagery_publication::rows::database_error;
use crate::imagery_publication::{
    PublicationSourceAuthority, commit_publication, selection, sources,
};
use df_assets::AssetId;
use df_imagery::execution::{
    ExecutionScope, ExecutionStageId, TerminalActor, TerminalAllocation, TerminalCommand,
};
use df_imagery::publication::{
    MAXIMUM_RECIPE_BYTES, PublicationError, SourceClosure, SourceDenial,
};
use df_imagery::runtime::NoDeliveryReason;
use df_imagery::{ImageJob, owner_scope_bytes};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

const DOMAIN: &str = "df-imagery/source-denied-v1";

struct Input {
    job: ImageJob,
    source_scope: ExecutionScope,
    legacy: bool,
    asset: AssetId,
    allocation: TerminalAllocation,
    source_digest: [u8; 32],
    command: Value,
}

fn source_error(error: df_imagery::runtime::RuntimeError) -> PublicationError {
    match error {
        df_imagery::runtime::RuntimeError::InvalidRecord
        | df_imagery::runtime::RuntimeError::SourceDenied => PublicationError::BindingConflict,
        df_imagery::runtime::RuntimeError::Execution(error) => PublicationError::Execution(error),
        _ => PublicationError::Unavailable,
    }
}

async fn read_input(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
) -> Result<Input, PublicationError> {
    if command.actor != command.tail {
        return Err(PublicationError::BindingConflict);
    }
    let scope = command.actor.scope();
    let legacy = command.candidate_dispatch.is_none();
    let dispatch = match (command.candidate_dispatch, &command.actor) {
        (Some(dispatch), _) => dispatch,
        (None, TerminalActor::Dispatch(actor)) => actor.dispatch,
        _ => return Err(PublicationError::BindingConflict),
    };
    let row=tx.query_opt("SELECT d.stage_id FROM imagery.image_dispatch_attempt d JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE s.owner_scope=$1 AND d.job_id=$2 AND d.dispatch_id=$3", &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    let candidate = ExecutionScope {
        owner: scope.owner.clone(),
        job: scope.job,
        stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>(0))
            .map_err(|_| PublicationError::InvalidRecord)?,
        dispatch,
    };
    crate::imagery_runtime::read_sources_in(tx, &candidate)
        .await
        .map_err(source_error)?
        .ok_or(PublicationError::BindingConflict)?;
    let source_digest = crate::imagery_runtime::sources_digest_in(tx, &candidate)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::BindingConflict)?;
    let row=tx.query_opt("SELECT stage_id,asset_id,publication_attempt FROM imagery.image_publication_allocation WHERE owner_scope=$1 AND job_id=$2", &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    let asset = AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    let allocation = TerminalAllocation::Existing {
        stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("stage_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        asset: *asset.as_bytes(),
        attempt: row
            .get::<_, Vec<u8>>("publication_attempt")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
    };
    let job = crate::read_in(tx, &scope.owner, scope.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let command_value = evidence::command_value(command, &job);
    Ok(Input {
        job,
        source_scope: candidate,
        legacy,
        asset,
        allocation,
        source_digest,
        command: command_value,
    })
}

async fn replay_in(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
    input: &Input,
) -> Result<Option<SourceClosure>, PublicationError> {
    let Some((receipt, binding)) = rows::read_in(tx, &input.job.owner, &input.job.job).await?
    else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_slice(&binding).map_err(|_| PublicationError::InvalidRecord)?;
    if receipt.reason != NoDeliveryReason::SourceDenied
        || command.now_ms < receipt.sealed_at_ms
        || value.get("domain") != Some(&json!(DOMAIN))
        || value.get("command") != Some(&input.command)
        || value.get("source_digest") != Some(&json!(input.source_digest))
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(Some(SourceClosure::Closed(receipt)))
}

fn label(reason: SourceDenial) -> &'static str {
    match reason {
        SourceDenial::GrantRefused => "grant_refused",
        SourceDenial::Missing => "missing",
        SourceDenial::Tombstoned => "tombstoned",
        SourceDenial::NotReady => "not_ready",
        SourceDenial::WinnerChanged => "winner_changed",
        SourceDenial::AudienceChanged => "audience_changed",
        SourceDenial::RetentionChanged => "retention_changed",
    }
}

async fn witness_in(
    tx: &Transaction<'_>,
    access: &sources::LockedSourceAccess,
) -> Result<Value, PublicationError> {
    let Some(asset) = access.denied_asset else {
        return Ok(Value::Null);
    };
    let record = crate::media_catalog_sql::asset_in(tx, asset)
        .await
        .map_err(PublicationError::Catalog)?
        .ok_or(PublicationError::InvalidRecord)?;
    let mut audience = Vec::new();
    record.audience.encode_into(&mut audience);
    let variant=access.locked.sources.iter().find(|fact|fact.asset.asset==asset).map(|fact|json!({"write":fact.master.write.as_bytes(),"version":fact.master.version.as_ref().map(|value|value.as_str()),"sha256":fact.master.manifest.map(|value|value.sha256)}));
    Ok(
        json!({"asset":asset.as_bytes(),"attempt":record.attempt.as_bytes(),"state":record.state.label(),"audience":audience,"suppression_generation":record.suppression_generation,"retention":record.retention.label(),"winning_writes":record.winning_writes.iter().map(|write|*write.as_bytes()).collect::<Vec<_>>(),"variant":variant}),
    )
}

/// Rechecks real source authority in the closure transaction; no caller supplies a denial flag.
pub async fn no_delivery_if_sources_denied_in(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
    authority: &dyn PublicationSourceAuthority,
) -> Result<SourceClosure, PublicationError> {
    crate::imagery_publication::rows::require_read_committed(tx).await?;
    let input = read_input(tx, command).await?;
    if let Some(existing) = replay_in(tx, command, &input).await? {
        return Ok(existing);
    }
    let mut access = sources::lock_for_closure_in(
        tx,
        &input.source_scope,
        input.asset,
        Some(authority),
        command.now_ms,
    )
    .await?;
    tx.query_one(
        "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
        &[
            &owner_scope_bytes(&input.job.owner),
            &input.job.job.as_bytes().as_slice(),
        ],
    )
    .await
    .map_err(database_error)?;
    let current = crate::read_in(tx, &input.job.owner, input.job.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    if current.intent != input.job.intent
        || current.config != input.job.config
        || current.recipe != input.job.recipe
        || current.resolved_key != input.job.resolved_key
        || access.receipt_digest != input.source_digest
    {
        return Err(PublicationError::BindingConflict);
    }
    let validation = crate::imagery_execution::validate_terminal_tail_in(tx, command)
        .await
        .map_err(PublicationError::Execution)?;
    if validation.allocation != input.allocation
        || (!input.legacy
            && validation
                .candidate
                .as_ref()
                .map(|candidate| &candidate.scope)
                != Some(&input.source_scope))
        || (input.legacy && validation.candidate.is_some())
    {
        return Err(PublicationError::BindingConflict);
    }
    crate::imagery_publication::rows::check_catalog_owner(&access.locked.output, &current.owner)?;
    super::check_allocation_in(tx, &current.owner, &current.job, validation.allocation).await?;
    let legacy_output = if input.legacy {
        Some(legacy_output_in(tx, &input, &current).await?)
    } else {
        None
    };
    if let Some(dispatch) = &validation.tail.dispatch {
        let scope = ExecutionScope {
            owner: current.owner.clone(),
            job: current.job,
            stage: dispatch.stage,
            dispatch: dispatch.dispatch,
        };
        let receipts = crate::imagery_runtime::sources_digest_in(tx, &scope)
            .await
            .map_err(|_| PublicationError::Unavailable)?
            .ok_or(PublicationError::BindingConflict)?;
        if receipts != input.source_digest {
            return Err(PublicationError::BindingConflict);
        }
    }
    if access.denial != Some(SourceDenial::GrantRefused) {
        match authority
            .authorize_sources(tx, &current, &access.locked.sources, command.now_ms)
            .await
        {
            Ok(()) => {}
            Err(PublicationError::SourceDenied(reason)) => {
                access.denial.get_or_insert(reason);
            }
            Err(error) => return Err(error),
        }
    }
    let Some(reason) = access.denial else {
        return Ok(SourceClosure::PublicationAllowed);
    };
    let (candidates, decisions) = evidence::retained_ancestors(tx, &current, &validation).await?;
    let candidate = validation
        .candidate
        .as_ref()
        .map(selection::candidate_binding)
        .transpose()?;
    let legacy_binding = legacy_output.as_ref().map(|output| json!({"write":output.write.as_bytes(),"version":output.version.as_str(),"sha256":output.sha256,"recipe_digest":output.recipe_digest}));
    let binding=json!({"domain":DOMAIN,"command":input.command,"source_digest":input.source_digest,"candidate":candidate,"legacy_output":legacy_binding,"denial":label(reason),"current":witness_in(tx,&access).await?}).to_string().into_bytes();
    if binding.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecord);
    }
    let retained = evidence::RetainedEvidence {
        reason: NoDeliveryReason::SourceDenied,
        decisions,
        candidates,
        locals: vec![],
        no_output_raw_sha256: None,
        legacy_output,
    };
    commit_in(tx, command, &validation, &retained, &binding)
        .await
        .map(SourceClosure::Closed)
}

async fn legacy_output_in(
    tx: &Transaction<'_>,
    input: &Input,
    job: &ImageJob,
) -> Result<evidence::LegacyOutput, PublicationError> {
    if df_imagery::runtime::requires_final_selection(job)
        .map_err(|_| PublicationError::InvalidRecord)?
    {
        return Err(PublicationError::BindingConflict);
    }
    let recipe = job
        .recipe
        .as_ref()
        .ok_or(PublicationError::BindingConflict)?;
    let digest = df_imagery::publication::recipe_digest(recipe);
    let TerminalAllocation::Existing { attempt, .. } = input.allocation else {
        return Err(PublicationError::BindingConflict);
    };
    // A legacy supplier result has no Candidate receipt. Its exact immutable output checkpoint
    // must instead identify one actual written Master in this allocation; ambiguity is structural.
    let rows=tx.query("SELECT v.write_id,p.object_version,p.sha256,p.recipe_digest FROM imagery.image_dispatch_attempt d JOIN imagery.image_execution_stage s USING(job_id,stage_id) JOIN imagery.image_stage_cursor c ON c.job_id=d.job_id AND c.stage_id=d.stage_id AND c.current_dispatch=d.dispatch_id JOIN imagery.image_dispatch_checkpoint p ON p.dispatch_id=d.dispatch_id AND p.usage_revision=d.usage_revision AND p.book_call_id=d.book_call_id JOIN imagery.image_dispatch_usage u ON u.owner_scope=s.owner_scope AND u.job_id=d.job_id AND u.stage_id=d.stage_id AND u.dispatch_id=d.dispatch_id AND u.revision=d.usage_revision JOIN media.asset_variant v ON v.asset_id=$5 AND v.attempt=$6 AND v.variant='master' AND v.state='written' AND v.object_domain='private_media' AND v.object_key=p.storage_key AND v.object_version=p.object_version AND v.byte_length=p.byte_len AND v.sha256=p.sha256 WHERE s.owner_scope=$1 AND d.job_id=$2 AND d.stage_id=$3 AND d.dispatch_id=$4 AND d.dispatch_state='succeeded' AND s.stage_kind IN ('generation','edit','correction','extraction') AND p.recipe_digest=$7 AND NOT EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint x WHERE x.dispatch_id=d.dispatch_id) AND NOT EXISTS(SELECT 1 FROM imagery.image_final_selection x WHERE x.job_id=d.job_id) AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result x WHERE x.job_id=d.job_id)", &[&owner_scope_bytes(&job.owner),&job.job.as_bytes().as_slice(),&input.source_scope.stage.as_bytes().as_slice(),&input.source_scope.dispatch.as_bytes().as_slice(),&input.asset.as_bytes().as_slice(),&attempt.as_slice(),&digest.as_slice()]).await.map_err(database_error)?;
    if rows.len() != 1 {
        return Err(PublicationError::BindingConflict);
    }
    let row = &rows[0];
    Ok(evidence::LegacyOutput {
        write: df_assets::WriteId::from_bytes(&row.get::<_, Vec<u8>>("write_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        version: df_assets::ObjectVersion::new(row.get::<_, String>("object_version"))
            .ok_or(PublicationError::InvalidRecord)?,
        sha256: row
            .get::<_, Vec<u8>>("sha256")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
        recipe_digest: digest,
    })
}

impl crate::imagery_publication::PostgresImagePublication {
    pub async fn no_delivery_if_sources_denied_with_authority(
        &self,
        command: &TerminalCommand,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<SourceClosure, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(database_error)?;
        let result = no_delivery_if_sources_denied_in(&tx, command, authority).await?;
        commit_publication(tx).await?;
        Ok(result)
    }
}
