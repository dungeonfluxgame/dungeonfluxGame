//! Local admission uses the existing catalog and a guarded durable allocation, with no finance rows.
use super::{NativeImagePublisher, catalog_error, conflict, local_stage, publication_error};
use crate::imagery_publication::{rows, sources};
use df_assets::{
    Admission, AdmitOutcome, AssetId, AssetKind, AttemptId, Audience, Clock, DedupeScope,
    ObjectStore, Provenance, QuotaScope, RetentionClass,
};
use df_imagery::execution::{DispatchId, DispatchMode, StageKind};
use df_imagery::publication::FrozenOutput;
use df_imagery::runtime::{
    DispatchAdmission, DispatchDecisionSettlement, DispatchNoOutputSettlement,
    ImageDispatchAdmission, RuntimeError, RuntimeIds, SendAuthorization, retained_audience,
    retained_config,
};
use df_imagery::{AccountingMode, ImageJob, ImageOwner, owner_scope_bytes};
use tokio_postgres::IsolationLevel;

fn local_projection_error(error: tokio_postgres::Error) -> RuntimeError {
    if error.code().is_some_and(|code| code.code() == "23514") {
        conflict()
    } else {
        publication_error(rows::database_error(error))
    }
}

async fn project_candidate_in(
    tx: &tokio_postgres::Transaction<'_>,
    job: &ImageJob,
    stage: df_imagery::execution::ExecutionStageId,
    dispatch: DispatchId,
    usable: Option<bool>,
    now: i64,
) -> Result<(), RuntimeError> {
    use crate::imagery_publication::StageEconomicUse;
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
        &[&dispatch.as_bytes().as_slice()],
    )
    .await
    .map_err(local_projection_error)?;
    tx.query_one(
        "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
        &[
            &owner_scope_bytes(&job.owner),
            &job.job.as_bytes().as_slice(),
        ],
    )
    .await
    .map_err(local_projection_error)?;
    let economic = crate::imagery_publication::stage_economic_use_in(
        tx, &job.owner, &job.job, &stage, &dispatch,
    )
    .await
    .map_err(publication_error)?
    .ok_or_else(conflict)?;
    let economic_usable = match economic {
        StageEconomicUse::SelectedCandidate { .. } => true,
        StageEconomicUse::SupersededCandidate { .. }
        | StageEconomicUse::UndeliveredCandidate { .. } => false,
        StageEconomicUse::NoOutput { .. } | StageEconomicUse::Decision { .. } => {
            return Err(conflict());
        }
    };
    let usage =
        i32::try_from(economic.usage_revision()).map_err(|_| RuntimeError::InvalidRecord)?;
    let row = tx.query_opt(
        "SELECT evidence FROM imagery.image_dispatch_usage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 AND revision=$5",
        &[&owner_scope_bytes(&job.owner), &job.job.as_bytes().as_slice(), &stage.as_bytes().as_slice(), &dispatch.as_bytes().as_slice(), &i64::from(usage)],
    ).await.map_err(local_projection_error)?.ok_or_else(conflict)?;
    let expected = if row.get::<_, String>(0) == "unknown" {
        None
    } else {
        Some(economic_usable)
    };
    if usable != expected {
        return Err(conflict());
    }
    tx.query_one(
        "SELECT imagery.mirror_local_candidate_outcome($1,$2,$3,$4,$5,$6)",
        &[
            &owner_scope_bytes(&job.owner),
            &job.job.as_bytes().as_slice(),
            &stage.as_bytes().as_slice(),
            &dispatch.as_bytes().as_slice(),
            &usage,
            &now,
        ],
    )
    .await
    .map_err(local_projection_error)?;
    Ok(())
}

async fn admit_catalog_in(
    tx: &tokio_postgres::Transaction<'_>,
    input: &DispatchAdmission<'_>,
    admission: &Admission,
    now: i64,
) -> Result<(), RuntimeError> {
    let job = input.job;
    let stage = input.stage;
    let asset = admission.asset;
    let attempt = admission.attempt;
    let key = admission.request_key.ok_or(RuntimeError::InvalidRecord)?;
    let is_anchor = local_stage::check_claim_in(tx, input, now).await?;
    match crate::media_catalog_sql::lock_quota_then_asset(tx, asset)
        .await
        .map_err(catalog_error)?
    {
        Some((_, record)) => {
            if record.attempt != attempt || record.request_key != Some(key) {
                return Err(conflict());
            }
            rows::check_catalog_owner(&record, &job.owner).map_err(publication_error)?;
            if !is_anchor
                && !matches!(
                    record.state,
                    df_assets::AssetState::Uploading
                        | df_assets::AssetState::Quarantined
                        | df_assets::AssetState::Validated
                )
            {
                return Err(conflict());
            }
        }
        None => {
            if !is_anchor {
                return Err(conflict());
            }
            if crate::media_catalog::admit_generation(tx, admission, now)
                .await
                .map_err(catalog_error)?
                != AdmitOutcome::Admitted
            {
                return Err(conflict());
            }
        }
    }
    if !is_anchor {
        return Ok(());
    }
    tx.query_one(
        "SELECT imagery.allocate_local_output($1,$2,$3,$4,$5,$6)",
        &[
            &owner_scope_bytes(&job.owner),
            &job.job.as_bytes().as_slice(),
            &stage.stage.as_bytes().as_slice(),
            &asset.as_bytes().as_slice(),
            &attempt.as_bytes().as_slice(),
            &now,
        ],
    )
    .await
    .map_err(|error| {
        if error.code().is_some_and(|code| code.code() == "23514") {
            conflict()
        } else {
            publication_error(rows::database_error(error))
        }
    })?;
    Ok(())
}

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> NativeImagePublisher<S, I, C> {
    fn build_admission(&self, input: &DispatchAdmission<'_>) -> Result<Admission, RuntimeError> {
        let job = input.job;
        let stage = input.stage;
        let config = input.config;
        let planned = stage
            .planned_publication
            .ok_or(RuntimeError::InvalidRecord)?;
        let asset = AssetId::from_bytes(&planned.asset).map_err(|_| RuntimeError::InvalidRecord)?;
        let attempt =
            AttemptId::from_bytes(&planned.attempt).map_err(|_| RuntimeError::InvalidRecord)?;
        let key = job.resolved_key.ok_or(RuntimeError::InvalidRecord)?;
        let recipe = job.recipe.as_ref().ok_or(RuntimeError::InvalidRecord)?;
        FrozenOutput::decode(recipe).map_err(publication_error)?;
        let audience = retained_audience(job)?;
        let (charge_scope, origin_account, campaign, dedupe_scope, retention) = match job.owner {
            ImageOwner::Run { campaign, .. } => (
                QuotaScope::Campaign(campaign),
                match audience {
                    Audience::OwnerPrivate(account) => Some(account),
                    _ => None,
                },
                Some(campaign),
                DedupeScope::campaign(&campaign),
                RetentionClass::CampaignMedia,
            ),
            ImageOwner::Draft { account } => (
                QuotaScope::Account(account),
                Some(account),
                None,
                DedupeScope::account(&account),
                RetentionClass::Personal,
            ),
        };
        Ok(Admission {
            asset,
            attempt,
            kind: AssetKind::Image,
            provenance: Provenance::Generated,
            charge_scope,
            origin_account,
            campaign,
            audience,
            dedupe_scope,
            request_key: Some(key),
            retention,
            reserve_bytes: config.transforms.max_output_bytes,
            byte_limit: self.settings.catalog.quota.byte_limit(charge_scope),
            in_flight_upload_limit: None,
        })
    }

    async fn admit_decision(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> Result<DispatchMode, RuntimeError> {
        let now = self.now()?;
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        sources::lock_admission_in(&tx, input, None, self.source_authority.as_deref(), now)
            .await
            .map_err(publication_error)?;
        local_stage::check_claim_in(&tx, input, now).await?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)?;
        Ok(DispatchMode::Local)
    }
}

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> ImageDispatchAdmission
    for NativeImagePublisher<S, I, C>
{
    async fn outcome_no_output(
        &self,
        input: &DispatchNoOutputSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        let proof = input.qualified;
        let scope = proof.scope();
        if input.mode != DispatchMode::Local {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if input.job.owner != scope.owner
            || input.job.job != scope.job
            || input.usage.owner != scope.owner
            || input.usage.job != scope.job
            || input.usage.stage != scope.stage
            || input.usage.dispatch != scope.dispatch
            || input.usage.revision.0 != u64::from(proof.usage_revision())
            || !proof.basis().kind.is_standalone()
        {
            return Err(conflict());
        }
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
            &[&scope.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(|error| publication_error(rows::database_error(error)))?;
        let metadata=tx.query_one("SELECT intent_digest,config_digest FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE", &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice()]).await.map_err(|error|publication_error(rows::database_error(error)))?;
        let intent: [u8; 32] = metadata
            .get::<_, Vec<u8>>("intent_digest")
            .try_into()
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let config: [u8; 32] = metadata
            .get::<_, Vec<u8>>("config_digest")
            .try_into()
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let basis = crate::imagery_execution::read_stage_basis_in(
            &tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
        )
        .await
        .map_err(RuntimeError::Execution)?;
        if basis != *proof.basis()
            || !proof.matches_financial_metadata(&scope.owner, &scope.job, &intent, &config)
        {
            return Err(conflict());
        }
        let now = self.now()?;
        let raw = proof.raw_sha256().map(|value| value.to_vec());
        tx.query_one(
            "SELECT imagery.mirror_local_analysis_failure($1,$2,$3,$4,$5,$6,$7)",
            &[
                &owner_scope_bytes(&scope.owner),
                &scope.job.as_bytes().as_slice(),
                &scope.stage.as_bytes().as_slice(),
                &scope.dispatch.as_bytes().as_slice(),
                &i32::try_from(proof.usage_revision()).map_err(|_| RuntimeError::InvalidRecord)?,
                &raw,
                &now,
            ],
        )
        .await
        .map_err(local_projection_error)?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)
    }

    async fn outcome_decision(
        &self,
        input: &DispatchDecisionSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        let usage = input.usage;
        let scope = input.qualified.scope();
        if input.mode != DispatchMode::Local {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if usage.owner != input.job.owner
            || usage.job != input.job.job
            || scope.owner != usage.owner
            || scope.job != usage.job
            || scope.stage != usage.stage
            || scope.dispatch != usage.dispatch
            || u64::from(input.qualified.usage_revision()) != usage.revision.0
        {
            return Err(conflict());
        }
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
            &[&scope.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(|error| publication_error(rows::database_error(error)))?;
        tx.query_one(
            "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
            &[
                &owner_scope_bytes(&scope.owner),
                &scope.job.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|error| publication_error(rows::database_error(error)))?;
        crate::imagery_publication::decision_economic_use_in(
            &tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
            &scope.dispatch,
            input.qualified,
        )
        .await
        .map_err(publication_error)?
        .ok_or_else(conflict)?;
        let reference = crate::imagery_execution::read_decision_reference_unlocked_in(
            &tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
            &scope.dispatch,
        )
        .await
        .map_err(RuntimeError::Execution)?
        .ok_or_else(conflict)?;
        let now = self.now()?;
        tx.query_one(
            "SELECT imagery.mirror_local_decision_outcome($1,$2,$3,$4,$5,$6,$7,$8)",
            &[
                &owner_scope_bytes(&scope.owner),
                &scope.job.as_bytes().as_slice(),
                &scope.stage.as_bytes().as_slice(),
                &scope.dispatch.as_bytes().as_slice(),
                &i32::try_from(reference.receipt.usage_revision)
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                &reference.payload_sha256.as_slice(),
                &input.qualified.raw_sha256().as_slice(),
                &now,
            ],
        )
        .await
        .map_err(|error| {
            if error.code().is_some_and(|code| code.code() == "23514") {
                conflict()
            } else {
                publication_error(rows::database_error(error))
            }
        })?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)
    }

    async fn admit(&self, input: &DispatchAdmission<'_>) -> Result<DispatchMode, RuntimeError> {
        let job = input.job;
        let stage = input.stage;
        let config = input.config;
        if !stage.local_accounting
            || (config.accounting.mode != AccountingMode::RecordOnly
                && stage.kind != StageKind::LocalTransform)
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if retained_config(job)? != *config || stage.owner != job.owner || stage.job != job.job {
            return Err(conflict());
        }
        if stage.kind == StageKind::Classification || stage.kind.is_standalone() {
            return self.admit_decision(input).await;
        }
        let admission = self.build_admission(input)?;
        let now = self.now()?;
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        sources::lock_admission_in(
            &tx,
            input,
            Some(admission.charge_scope),
            self.source_authority.as_deref(),
            now,
        )
        .await
        .map_err(publication_error)?;
        admit_catalog_in(&tx, input, &admission, now).await?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)?;
        Ok(DispatchMode::Local)
    }

    async fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        if mode != DispatchMode::Local {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if retained_config(input.job)? != *input.config
            || !input.stage.local_accounting
            || (input.config.accounting.mode == AccountingMode::Hosted
                && input.stage.kind != StageKind::LocalTransform)
        {
            return Err(conflict());
        }
        let now = self.now()?;
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        let scope = df_imagery::execution::ExecutionScope {
            owner: input.job.owner.clone(),
            job: input.job.job,
            stage: input.stage.stage,
            dispatch: input.dispatch,
        };
        let stored = crate::imagery_runtime::read_sources_in(&tx, &scope)
            .await?
            .ok_or(RuntimeError::SourceDenied)?;
        if stored != input.sources {
            return Err(conflict());
        }
        sources::lock_admission_in(&tx, input, None, self.source_authority.as_deref(), now)
            .await
            .map_err(publication_error)?;
        let command = df_imagery::execution::MarkSentCommand::new(
            input.job.owner.clone(),
            input.job.job,
            input.stage.stage,
            input.dispatch,
            input.claim,
            input.stage.claim_generation,
            now,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let sent = crate::imagery_execution::mark_local_sent_in(&tx, &command)
            .await
            .map_err(RuntimeError::Execution)?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)?;
        Ok(match sent {
            df_imagery::execution::LocalSentOutcome::New => SendAuthorization::New,
            df_imagery::execution::LocalSentOutcome::AlreadySent => SendAuthorization::AlreadySent,
        })
    }

    async fn outcome(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        if mode != DispatchMode::Local {
            return Err(RuntimeError::UnsupportedCapability);
        }
        let now = self.now()?;
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        let stored=tx.query_opt("SELECT s.stage_kind,s.stage_id,d.dispatch_state,EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint c WHERE c.dispatch_id=d.dispatch_id) AS has_candidate,EXISTS(SELECT 1 FROM imagery.image_job_result r WHERE r.owner_scope=s.owner_scope AND r.job_id=d.job_id AND r.stage_id=d.stage_id AND r.dispatch_id=d.dispatch_id AND NOT EXISTS(SELECT 1 FROM imagery.image_final_selection f WHERE f.job_id=r.job_id)) AS has_legacy_ready,EXISTS(SELECT 1 FROM imagery.image_no_delivery n WHERE n.owner_scope=s.owner_scope AND n.job_id=d.job_id AND n.actor_stage_id=d.stage_id AND n.actor_dispatch_id=d.dispatch_id AND n.reason='source_denied' AND n.legacy_output_write_id IS NOT NULL) AS has_legacy_undelivered FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id) WHERE s.owner_scope=$1 AND s.job_id=$2 AND d.dispatch_id=$3", &[&owner_scope_bytes(&job.owner),&job.job.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()]).await.map_err(|error|publication_error(rows::database_error(error)))?.ok_or(RuntimeError::InvalidRecord)?;
        let kind = StageKind::parse_label(&stored.get::<_, String>("stage_kind"))
            .ok_or(RuntimeError::InvalidRecord)?;
        if stored.get::<_, String>("dispatch_state") == "declined" {
            if usable.is_some() {
                return Err(conflict());
            }
            let scope = df_imagery::execution::ExecutionScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: df_imagery::execution::ExecutionStageId::from_bytes(
                    &stored.get::<_, Vec<u8>>("stage_id"),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
                dispatch,
            };
            crate::imagery_execution::read_decline_reference_in(&tx, &scope)
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or_else(conflict)?;
        }
        if stored.get::<_, bool>("has_candidate")
            || ((stored.get::<_, bool>("has_legacy_ready")
                || stored.get::<_, bool>("has_legacy_undelivered"))
                && matches!(
                    kind,
                    StageKind::Generation
                        | StageKind::Edit
                        | StageKind::Correction
                        | StageKind::Extraction
                ))
        {
            let stage = df_imagery::execution::ExecutionStageId::from_bytes(
                &stored.get::<_, Vec<u8>>("stage_id"),
            )
            .map_err(|_| RuntimeError::InvalidRecord)?;
            project_candidate_in(&tx, job, stage, dispatch, usable, now).await?;
            return crate::imagery_publication::commit_publication(tx)
                .await
                .map_err(publication_error);
        }
        if kind != StageKind::LocalTransform && usable != Some(true) {
            tx.query_one(
                "SELECT imagery.mirror_local_transport_outcome($1,$2,$3,$4,$5)",
                &[
                    &owner_scope_bytes(&job.owner),
                    &job.job.as_bytes().as_slice(),
                    &stored.get::<_, Vec<u8>>("stage_id"),
                    &dispatch.as_bytes().as_slice(),
                    &now,
                ],
            )
            .await
            .map_err(local_projection_error)?;
            return crate::imagery_publication::commit_publication(tx)
                .await
                .map_err(publication_error);
        }
        tx.query_one(
            "SELECT imagery.mirror_local_outcome($1,$2,$3,$4,$5)",
            &[
                &owner_scope_bytes(&job.owner),
                &job.job.as_bytes().as_slice(),
                &dispatch.as_bytes().as_slice(),
                &usable,
                &now,
            ],
        )
        .await
        .map_err(|error| {
            if error.code().is_some_and(|code| code.code() == "23514") {
                conflict()
            } else {
                publication_error(rows::database_error(error))
            }
        })?;
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)
    }
}
