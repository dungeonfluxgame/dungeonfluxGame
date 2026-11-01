//! Concrete native output writer. Raster execution supplies every required variant; this adapter
//! validates and stores their exact bytes, then calls the atomic publication transaction.

use super::{
    PostgresImagePublication, PublicationCommand, PublicationOutcome, PublicationSourceAuthority,
    rows, selection, sources,
};
use crate::media_catalog::PostgresMediaCatalog;
use df_assets::{
    AssetId, AssetManifest, AssetRecord, AssetState, AssetVariant, AttemptId, CatalogConfig,
    CatalogError, CatalogStore, ClaimOutcome, ClaimToken, Clock, Deadline, FinalizeRequest,
    HeldStep, ImageAlpha, ImageIntent, ImageProof, ImageProofError, ImageProofLimits, KeyLayout,
    LeaseOutcome, ObjectKey, ObjectStore, ObjectStoreError, ObjectVersion, StageOutcome,
    StagedObjectRecord, Step, StepClaim, StorageDomain, StorageOwner, StoredObject, VariantIntent,
    VariantRecord, VariantState, VariantWritten, Winner, WriteId, record_owner,
};
use df_imagery::execution::{CheckpointAlpha, OutputCheckpoint, OutputImage, OutputObject};
use df_imagery::publication::{
    FinalOutputSelection, FrozenOutput, PublicationError, PublishedImage,
};
use df_imagery::runtime::{
    ImagePublisher, PreparedImageOutput, PublicationContext, RuntimeError, RuntimeIds,
};
use sha2::{Digest, Sha256};
use std::sync::{Arc, RwLock};
use tokio::sync::Semaphore;
use tokio_postgres::Row;

mod admission;
mod context;
mod local_stage;
mod qualification;

#[derive(Clone, Copy)]
struct RetainedWrite<'a> {
    bytes: &'a [u8],
    manifest: AssetManifest,
    maximum_bytes: u64,
}

/// Composition supplies existing durable adapters and its clock; no credentials are retained here.
pub struct RuntimePublicationPorts<S, I, C> {
    pub catalog: Arc<PostgresMediaCatalog>,
    pub publication: PostgresImagePublication,
    pub objects: Arc<S>,
    pub ids: Arc<I>,
    pub clock: Arc<C>,
}

/// Storage settings and CPU admission are explicit runtime configuration, independent of uploads.
pub struct RuntimePublicationSettings {
    pub catalog: CatalogConfig,
    pub layout: KeyLayout,
    pub decode_concurrency: usize,
}

/// The native worker's catalog/object publication and local-only admission edge.
pub struct NativeImagePublisher<S, I, C> {
    ports: RuntimePublicationPorts<S, I, C>,
    settings: RuntimePublicationSettings,
    decode: RwLock<Arc<Semaphore>>,
    source_authority: Option<Arc<dyn PublicationSourceAuthority>>,
}

fn catalog_error(error: CatalogError) -> RuntimeError {
    RuntimeError::Publication(PublicationError::Catalog(error))
}
fn publication_error(error: PublicationError) -> RuntimeError {
    match error {
        PublicationError::SourceDenied(_) => RuntimeError::SourceDenied,
        error => RuntimeError::Publication(error),
    }
}
fn proof_error(error: ImageProofError) -> RuntimeError {
    publication_error(PublicationError::Proof(error))
}
fn conflict() -> RuntimeError {
    publication_error(PublicationError::BindingConflict)
}

fn scope(context: &PublicationContext) -> df_imagery::execution::ExecutionScope {
    df_imagery::execution::ExecutionScope {
        owner: context.job.owner.clone(),
        job: context.job.job,
        stage: context.stage.stage,
        dispatch: context.dispatch,
    }
}

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> NativeImagePublisher<S, I, C> {
    pub fn new(
        settings: RuntimePublicationSettings,
        ports: RuntimePublicationPorts<S, I, C>,
    ) -> Result<Self, RuntimeError> {
        if settings.decode_concurrency == 0
            || settings.decode_concurrency > Semaphore::MAX_PERMITS
            || settings.catalog.step_lease_ms <= 0
            || settings.catalog.store_deadline.is_zero()
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let decode = Arc::new(Semaphore::new(settings.decode_concurrency));
        Ok(Self {
            ports,
            settings,
            decode: RwLock::new(decode),
            source_authority: None,
        })
    }

    /// Host grants are checked under the publication transaction's current source catalog locks.
    pub fn with_source_authority(mut self, authority: Arc<dyn PublicationSourceAuthority>) -> Self {
        self.source_authority = Some(authority);
        self
    }

    /// Replace the worker bound after the host drains its owned runtime, before applying its gate.
    pub fn set_decode_concurrency(&self, concurrency: usize) -> Result<(), RuntimeError> {
        if concurrency == 0 || concurrency > Semaphore::MAX_PERMITS {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let next = Arc::new(Semaphore::new(concurrency));
        *self
            .decode
            .write()
            .map_err(|_| RuntimeError::WorkerFailed)? = next;
        Ok(())
    }

    fn decode_semaphore(&self) -> Result<Arc<Semaphore>, RuntimeError> {
        self.decode
            .read()
            .map(|semaphore| Arc::clone(&semaphore))
            .map_err(|_| RuntimeError::WorkerFailed)
    }

    fn now(&self) -> Result<i64, RuntimeError> {
        let now = self.ports.clock.now_ms();
        if now < 0 {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(now)
    }

    fn until(&self, now: i64) -> Result<i64, RuntimeError> {
        now.checked_add(self.settings.catalog.step_lease_ms)
            .ok_or(RuntimeError::InvalidConfiguration)
    }

    fn ids(&self, context: &PublicationContext) -> Result<(AssetId, AttemptId), RuntimeError> {
        if context.job.job != context.stage.job || context.job.owner != context.stage.owner {
            return Err(conflict());
        }
        let planned = context
            .stage
            .planned_publication
            .ok_or(RuntimeError::InvalidRecord)?;
        Ok((
            AssetId::from_bytes(&planned.asset).map_err(|_| RuntimeError::InvalidRecord)?,
            AttemptId::from_bytes(&planned.attempt).map_err(|_| RuntimeError::InvalidRecord)?,
        ))
    }

    async fn inspect(
        &self,
        bytes: &[u8],
        intent: ImageIntent,
        limits: ImageProofLimits,
    ) -> Result<AssetManifest, RuntimeError> {
        limits
            .check_encoded_len(u64::try_from(bytes.len()).map_err(|_| RuntimeError::InvalidRecord)?)
            .map_err(proof_error)?;
        let permit = self
            .decode_semaphore()?
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::WorkerFailed)?;
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ImageProof::inspect_encoded(&bytes, intent, limits).map_err(proof_error)?;
            Ok(AssetManifest {
                byte_len: u64::try_from(bytes.len()).map_err(|_| RuntimeError::InvalidRecord)?,
                sha256: Sha256::digest(&bytes).into(),
            })
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
    }

    async fn put(
        &self,
        key: &ObjectKey,
        bytes: &[u8],
        manifest: AssetManifest,
        maximum_bytes: u64,
    ) -> Result<StoredObject, RuntimeError> {
        let deadline = Deadline::after(self.settings.catalog.store_deadline);
        match self
            .ports
            .objects
            .put_new_with_bound(key, bytes, manifest, maximum_bytes, deadline)
            .await
        {
            Ok(stored) => Ok(stored),
            Err(ObjectStoreError::Unknown | ObjectStoreError::AlreadyExists) => {
                let stored = self
                    .ports
                    .objects
                    .head(key, deadline)
                    .await
                    .map_err(|_| RuntimeError::StorageUnavailable)?
                    .ok_or(RuntimeError::StorageUnavailable)?;
                if stored.manifest != manifest {
                    return Err(conflict());
                }
                Ok(stored)
            }
            Err(_) => Err(RuntimeError::StorageUnavailable),
        }
    }

    async fn stage(
        &self,
        context: &PublicationContext,
        bytes: &[u8],
        manifest: AssetManifest,
        maximum_bytes: u64,
    ) -> Result<AssetRecord, RuntimeError> {
        let (asset, attempt) = self.ids(context)?;
        let mut record = self
            .ports
            .catalog
            .asset(asset)
            .await
            .map_err(catalog_error)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if record.attempt != attempt {
            return Err(conflict());
        }
        if record.state == AssetState::Uploading {
            let write = WriteId::from_bytes(&self.ports.ids.next_bytes())
                .map_err(|_| RuntimeError::InvalidRecord)?;
            let stored = self
                .put(
                    &self.settings.layout.staged(asset, attempt, write),
                    bytes,
                    manifest,
                    maximum_bytes,
                )
                .await?;
            let now = self.now()?;
            let staged = StagedObjectRecord {
                key: stored.key,
                version: stored.version,
                manifest,
                staged_at_ms: now,
            };
            match self
                .ports
                .catalog
                .record_staged(asset, attempt, &staged, now)
                .await
                .map_err(catalog_error)?
            {
                StageOutcome::Staged | StageOutcome::AlreadyStaged => {}
                _ => return Err(conflict()),
            }
            record = self
                .ports
                .catalog
                .asset(asset)
                .await
                .map_err(catalog_error)?
                .ok_or(RuntimeError::InvalidRecord)?;
        }
        if record
            .staged
            .as_ref()
            .is_none_or(|staged| staged.manifest != manifest)
        {
            return Err(conflict());
        }
        Ok(record)
    }

    async fn claim_catalog(&self, record: &AssetRecord) -> Result<HeldStep, RuntimeError> {
        let now = self.now()?;
        let held = if let Some(lease) = record.lease {
            let held = HeldStep {
                asset: record.asset,
                attempt: record.attempt,
                owner: lease.owner,
            };
            if self
                .ports
                .catalog
                .renew_step(held, self.until(now)?, now)
                .await
                .map_err(catalog_error)?
                != LeaseOutcome::Done
            {
                return Err(conflict());
            }
            held
        } else {
            let owner = ClaimToken::from_bytes(&self.ports.ids.next_bytes())
                .map_err(|_| RuntimeError::InvalidRecord)?;
            let step = if record.state == AssetState::Quarantined {
                Step::Validate
            } else if record.state == AssetState::Validated {
                Step::Publish
            } else {
                return Err(conflict());
            };
            let claim = StepClaim {
                asset: record.asset,
                attempt: record.attempt,
                step,
                expected_state: record.state,
                owner,
                lease_until_ms: self.until(now)?,
            };
            if self
                .ports
                .catalog
                .claim_step(claim, now)
                .await
                .map_err(catalog_error)?
                != ClaimOutcome::Claimed
            {
                return Err(conflict());
            }
            HeldStep {
                asset: record.asset,
                attempt: record.attempt,
                owner,
            }
        };
        if record.state == AssetState::Quarantined
            && self
                .ports
                .catalog
                .mark_validated(held, now)
                .await
                .map_err(catalog_error)?
                != LeaseOutcome::Done
        {
            return Err(conflict());
        }
        Ok(held)
    }

    async fn variant(
        &self,
        held: HeldStep,
        variant: AssetVariant,
    ) -> Result<Option<VariantRecord>, RuntimeError> {
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
        let row=client.query_opt("SELECT * FROM media.asset_variant WHERE asset_id=$1 AND attempt=$2 AND owner=$3 AND variant=$4 AND state='written' ORDER BY created_at_ms,write_id LIMIT 1", &[&held.asset.as_bytes().as_slice(),&held.attempt.as_bytes().as_slice(),&held.owner.as_bytes().as_slice(),&variant.label()]).await.map_err(|error|publication_error(rows::database_error(error)))?;
        row.as_ref().map(parse_variant).transpose()
    }

    async fn write_variant(
        &self,
        held: HeldStep,
        owner: &StorageOwner,
        variant: AssetVariant,
        write: RetainedWrite<'_>,
    ) -> Result<VariantRecord, RuntimeError> {
        let RetainedWrite {
            bytes,
            manifest,
            maximum_bytes,
        } = write;
        if let Some(record) = self.variant(held, variant).await? {
            if record.manifest != Some(manifest) {
                return Err(conflict());
            }
            return Ok(record);
        }
        let write = WriteId::from_bytes(&self.ports.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let key = self
            .settings
            .layout
            .owned_asset(owner, held.asset, held.attempt, write, variant);
        let now = self.now()?;
        if self
            .ports
            .catalog
            .intend_variant(
                held,
                &VariantIntent {
                    variant,
                    write,
                    key: key.clone(),
                },
                now,
            )
            .await
            .map_err(catalog_error)?
            != LeaseOutcome::Done
        {
            return Err(conflict());
        }
        let stored = self.put(&key, bytes, manifest, maximum_bytes).await?;
        if !self
            .ports
            .catalog
            .mark_variant_written(
                held.asset,
                held.attempt,
                &VariantWritten {
                    variant,
                    write,
                    version: stored.version,
                    manifest,
                },
            )
            .await
            .map_err(catalog_error)?
        {
            return Err(conflict());
        }
        self.variant(held, variant)
            .await?
            .ok_or(RuntimeError::InvalidRecord)
    }

    async fn read_checkpoint(
        &self,
        context: &PublicationContext,
    ) -> Result<OutputCheckpoint, RuntimeError> {
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
        let checkpoint = crate::imagery_execution::read_output_checkpoint_in(
            &tx,
            &context.job.owner,
            &context.job.job,
            &context.stage.stage,
            &context.dispatch,
        )
        .await
        .map_err(RuntimeError::Execution)?
        .ok_or(RuntimeError::InvalidRecord)?;
        tx.commit()
            .await
            .map_err(|_| publication_error(PublicationError::Unknown))?;
        Ok(checkpoint)
    }

    async fn prove(
        &self,
        record: VariantRecord,
        intent: ImageIntent,
        limits: ImageProofLimits,
        digest: [u8; 32],
    ) -> Result<ImageProof, RuntimeError> {
        let object = stored_object(&record)?;
        let permit = self
            .decode_semaphore()?
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::WorkerFailed)?;
        let bytes = self
            .ports
            .objects
            .get_version(
                &object,
                Deadline::after(self.settings.catalog.store_deadline),
            )
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ImageProof::decode(&record, &bytes, digest, intent, limits).map_err(proof_error)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
    }
}

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> ImagePublisher
    for NativeImagePublisher<S, I, C>
{
    async fn no_delivery_if_sources_denied(
        &self,
        command: &df_imagery::execution::TerminalCommand,
    ) -> Result<df_imagery::publication::SourceClosure, RuntimeError> {
        let authority = self
            .source_authority
            .as_deref()
            .ok_or(RuntimeError::UnsupportedCapability)?;
        self.ports
            .publication
            .no_delivery_if_sources_denied_with_authority(command, authority)
            .await
            .map_err(publication_error)
    }

    async fn no_delivery(
        &self,
        request: &df_imagery::publication::NoDeliveryRequest<'_>,
    ) -> Result<df_imagery::publication::NoDeliveryReceipt, RuntimeError> {
        self.ports
            .publication
            .no_delivery(request)
            .await
            .map(super::NoDeliveryOutcome::receipt)
            .map_err(publication_error)
    }

    async fn read_no_delivery(
        &self,
        owner: &df_imagery::ImageOwner,
        job: &df_imagery::ImageJobId,
    ) -> Result<Option<df_imagery::publication::NoDeliveryReceipt>, RuntimeError> {
        self.ports
            .publication
            .read_no_delivery(owner, job)
            .await
            .map_err(publication_error)
    }

    async fn image_economic_use(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
    ) -> Result<Option<df_imagery::runtime::ImageEconomicDisposition>, RuntimeError> {
        self.read_image_economic_use(scope).await
    }

    async fn select_output(
        &self,
        context: &PublicationContext,
        selected: &FinalOutputSelection,
    ) -> Result<(), RuntimeError> {
        self.check_context(context).await?;
        let qualified = self.qualify_selection(context, selected).await?;
        let (asset, _) = self.ids(context)?;
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
        let locked = sources::lock_in(
            &tx,
            &scope(context),
            asset,
            self.source_authority.as_deref(),
            now,
        )
        .await
        .map_err(publication_error)?;
        selection::select_in(
            &tx,
            context,
            selected,
            &locked.binding,
            qualified.as_ref(),
            now,
        )
        .await
        .map_err(publication_error)?;
        sources::authorize_in(&tx, &locked, self.source_authority.as_deref(), now)
            .await
            .map_err(publication_error)?;
        super::commit_publication(tx)
            .await
            .map_err(publication_error)
    }

    async fn store_output(
        &self,
        context: &PublicationContext,
        output: &PreparedImageOutput,
    ) -> Result<OutputCheckpoint, RuntimeError> {
        self.check_context(context).await?;
        let (frozen, digest, limits) = self.recipe(context).await?;
        if frozen.intent(AssetVariant::Display).is_some() != output.display.is_some() {
            return Err(RuntimeError::InvalidRecord);
        }
        let master_intent = frozen
            .intent(AssetVariant::Master)
            .ok_or(RuntimeError::InvalidRecord)?;
        let master_manifest = self.inspect(&output.master, master_intent, limits).await?;
        let display_manifest = match (&output.display, frozen.intent(AssetVariant::Display)) {
            (Some(bytes), Some(intent)) => Some(self.inspect(bytes, intent, limits).await?),
            (None, None) => None,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        let maximum_bytes = df_imagery::runtime::retained_config(&context.job)?
            .transforms
            .max_output_bytes;
        let record = self
            .stage(context, &output.master, master_manifest, maximum_bytes)
            .await?;
        let owner = record_owner(&record).ok_or(RuntimeError::InvalidRecord)?;
        let held = self.claim_catalog(&record).await?;
        let master = self
            .write_variant(
                held,
                &owner,
                AssetVariant::Master,
                RetainedWrite {
                    bytes: &output.master,
                    manifest: master_manifest,
                    maximum_bytes,
                },
            )
            .await?;
        if let (Some(bytes), Some(manifest)) = (&output.display, display_manifest) {
            self.write_variant(
                held,
                &owner,
                AssetVariant::Display,
                RetainedWrite {
                    bytes,
                    manifest,
                    maximum_bytes,
                },
            )
            .await?;
        }
        let proof = self.prove(master, master_intent, limits, digest).await?;
        let stored = stored_object(proof.record())?;
        OutputCheckpoint::new(
            &context.dispatch,
            OutputObject {
                storage_key: stored.key.path().to_owned(),
                object_version: stored.version.as_str().to_owned(),
                byte_len: stored.manifest.byte_len,
                sha256: stored.manifest.sha256,
            },
            OutputImage {
                mime: proof.intent().mime().to_owned(),
                width_px: Some(proof.intent().width_px()),
                height_px: Some(proof.intent().height_px()),
                alpha: match proof.alpha() {
                    ImageAlpha::Opaque => CheckpointAlpha::Opaque,
                    ImageAlpha::Transparent => CheckpointAlpha::Transparent,
                },
            },
            digest,
            context.usage_revision,
        )
        .map_err(|_| RuntimeError::InvalidRecord)
    }

    async fn publish(&self, context: &PublicationContext) -> Result<PublishedImage, RuntimeError> {
        use df_imagery::publication::ImageResults;
        self.check_context(context).await?;
        if let Some(result) = self
            .ports
            .publication
            .read_result(&context.job.owner, &context.job.job)
            .await
            .map_err(publication_error)?
        {
            let (asset, attempt) = self.ids(context)?;
            let (_, digest, _) = self.recipe(context).await?;
            if result.stage != context.stage.stage
                || result.dispatch != context.dispatch
                || result.asset != asset
                || result.attempt != attempt
                || result.recipe_digest != digest
            {
                return Err(conflict());
            }
            self.check_sources(context).await?;
            return Ok(result);
        }
        let (frozen, digest, limits) = self.recipe(context).await?;
        let (asset, attempt) = self.ids(context)?;
        let selected = self.final_selection(context).await?;
        let qualified = if let Some(selected) = &selected {
            self.qualify_stored(context, selected).await?
        } else {
            None
        };
        let record = self
            .ports
            .catalog
            .asset(asset)
            .await
            .map_err(catalog_error)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if record.attempt != attempt {
            return Err(conflict());
        }
        let held = self.claim_catalog(&record).await?;
        let checkpoint = self.read_checkpoint(context).await?;
        let master = self
            .variant(held, AssetVariant::Master)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        if master.key.path() != checkpoint.storage_key
            || master
                .version
                .as_ref()
                .is_none_or(|version| version.as_str() != checkpoint.object_version)
        {
            return Err(conflict());
        }
        let mut proofs = Vec::with_capacity(2);
        for variant in [AssetVariant::Master, AssetVariant::Display] {
            if let Some(intent) = frozen.intent(variant) {
                let record = self
                    .variant(held, variant)
                    .await?
                    .ok_or(RuntimeError::InvalidRecord)?;
                proofs.push(self.prove(record, intent, limits, digest).await?);
            }
        }
        let now = self.now()?;
        if self
            .ports
            .catalog
            .renew_step(held, self.until(now)?, now)
            .await
            .map_err(catalog_error)?
            != LeaseOutcome::Done
        {
            return Err(conflict());
        }
        let primary = master_intent(&frozen)?;
        let request = FinalizeRequest {
            winners: proofs
                .iter()
                .map(|proof| Winner {
                    variant: proof.variant(),
                    write: proof.record().write,
                })
                .collect(),
            mime: primary.mime().to_owned(),
            width_px: Some(primary.width_px()),
            height_px: Some(primary.height_px()),
            byte_limit: self.settings.catalog.quota.byte_limit(record.charge_scope),
        };
        let command = PublicationCommand {
            owner: &context.job.owner,
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
            claim: context.claim,
            claim_generation: context.stage.claim_generation,
            held,
            finalize: &request,
            proofs: &proofs,
            now_ms: now,
            recovery: context.recovery.as_ref(),
            source_authority: self.source_authority.as_deref(),
            qualified: qualified.as_ref(),
        };
        match self
            .ports
            .publication
            .publish(&command)
            .await
            .map_err(publication_error)?
        {
            PublicationOutcome::Recorded(result) | PublicationOutcome::Existing(result) => {
                Ok(result)
            }
            PublicationOutcome::NotRecorded(_) => Err(conflict()),
        }
    }
}

fn master_intent(recipe: &FrozenOutput) -> Result<ImageIntent, RuntimeError> {
    recipe
        .intent(AssetVariant::Master)
        .ok_or(RuntimeError::InvalidRecord)
}
fn stored_object(record: &VariantRecord) -> Result<StoredObject, RuntimeError> {
    Ok(StoredObject {
        key: record.key.clone(),
        version: record.version.clone().ok_or(RuntimeError::InvalidRecord)?,
        manifest: record.manifest.ok_or(RuntimeError::InvalidRecord)?,
    })
}
fn parse_variant(row: &Row) -> Result<VariantRecord, RuntimeError> {
    let variant = match row.get::<_, String>("variant").as_str() {
        "master" => AssetVariant::Master,
        "display" => AssetVariant::Display,
        _ => return Err(RuntimeError::InvalidRecord),
    };
    Ok(VariantRecord {
        asset: AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        attempt: AttemptId::from_bytes(&row.get::<_, Vec<u8>>("attempt"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        write: WriteId::from_bytes(&row.get::<_, Vec<u8>>("write_id"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        variant,
        owner: ClaimToken::from_bytes(&row.get::<_, Vec<u8>>("owner"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        state: VariantState::Written,
        key: ObjectKey::from_stored(
            StorageDomain::PrivateMedia,
            &row.get::<_, String>("object_key"),
        )
        .ok_or(RuntimeError::InvalidRecord)?,
        version: ObjectVersion::new(row.get("object_version")),
        manifest: Some(AssetManifest {
            byte_len: row
                .get::<_, i64>("byte_length")
                .try_into()
                .map_err(|_| RuntimeError::InvalidRecord)?,
            sha256: row
                .get::<_, Vec<u8>>("sha256")
                .try_into()
                .map_err(|_| RuntimeError::InvalidRecord)?,
        }),
        created_at_ms: row.get("created_at_ms"),
    })
}
