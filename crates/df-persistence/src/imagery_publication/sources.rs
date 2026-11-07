//! Current source catalog and host authority remain locked through atomic Ready publication.

use super::rows::database_error;
use df_assets::{
    AssetId, AssetKind, AssetManifest, AssetRecord, AssetState, AssetVariant, ClaimToken,
    ObjectKey, ObjectVersion, StorageDomain, VariantRecord, VariantState, WriteId,
};
use df_imagery::execution::ExecutionScope;
use df_imagery::publication::{PublicationError, SourceDenial};
use df_imagery::runtime::{DispatchAdmission, SourceReceipt};
use df_imagery::{ImageJob, ImageOwner, SourceRef};
use std::future::Future;
use std::pin::Pin;
use tokio_postgres::Transaction;

/// Current version-pinned catalog facts; a SourceRef alone never authorizes their use.
pub struct PublicationSource {
    pub source: SourceRef,
    pub asset: AssetRecord,
    pub master: VariantRecord,
}

pub type PublicationAuthorityFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), PublicationError>> + Send + 'a>>;

/// Locks and verifies current host grants in the same transaction as the catalog source rows.
pub trait PublicationSourceAuthority: Send + Sync {
    fn lock_authority<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        owner: &'a ImageOwner,
        now_ms: i64,
    ) -> PublicationAuthorityFuture<'a>;
    fn authorize_sources<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        job: &'a ImageJob,
        sources: &'a [PublicationSource],
        now_ms: i64,
    ) -> PublicationAuthorityFuture<'a>;
    /// Authorizes a current request's shared artifact access; foreign job ownership is not reused.
    fn authorize_artifact<'a>(
        &'a self,
        _tx: &'a Transaction<'_>,
        _request: &'a df_imagery::ImageRequest,
        _output: &'a AssetRecord,
        _sources: &'a [PublicationSource],
        _now_ms: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async { Err(PublicationError::BindingConflict) })
    }
    /// Rechecks current shared-artifact access without impersonating the producer's run.
    fn authorize_artifact_read<'a>(
        &'a self,
        _tx: &'a Transaction<'_>,
        _current_owner: &'a ImageOwner,
        _output: &'a AssetRecord,
        _sources: &'a [PublicationSource],
        _now_ms: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async { Err(PublicationError::BindingConflict) })
    }
}

/// Locks source assets in stable id order, then verifies exact current winning image versions.
pub async fn lock_image_sources_in(
    tx: &Transaction<'_>,
    pins: &[SourceRef],
    requested: &df_assets::Audience,
) -> Result<Vec<PublicationSource>, PublicationError> {
    let mut assets = pins.iter().map(|pin| pin.asset).collect::<Vec<_>>();
    assets.sort_unstable_by_key(|asset| *asset.as_bytes());
    assets.dedup();
    for asset in assets {
        tx.query_opt(
            "SELECT asset_id FROM media.asset WHERE asset_id=$1 FOR UPDATE",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .ok_or(PublicationError::NotFound)?;
    }
    let mut sources = Vec::with_capacity(pins.len());
    for pin in pins {
        sources.push(read_source(tx, pin, requested).await?);
    }
    Ok(sources)
}

pub(super) struct LockedSources {
    pub job: ImageJob,
    pub output: AssetRecord,
    pub sources: Vec<PublicationSource>,
    pub binding: Vec<u8>,
}

pub(super) struct LockedSourceAccess {
    pub locked: LockedSources,
    pub denial: Option<SourceDenial>,
    pub denied_asset: Option<AssetId>,
    pub receipt_digest: [u8; 32],
}

/// Compares exact consumed catalog facts; current grant authorization remains a separate gate.
pub fn matches_source_receipt(fact: &PublicationSource, receipt: &SourceReceipt) -> bool {
    fact.source == receipt.pin
        && fact.asset.attempt == receipt.attempt
        && fact.master.write == receipt.write
        && fact.master.version.as_ref() == Some(&receipt.version)
        && fact.asset.suppression_generation == receipt.suppression_generation
        && fact.asset.retention == receipt.retention
}

fn receipt_denial(fact: &PublicationSource, receipt: &SourceReceipt) -> Option<SourceDenial> {
    if fact.asset.retention != receipt.retention {
        Some(SourceDenial::RetentionChanged)
    } else if !matches_source_receipt(fact, receipt) {
        Some(SourceDenial::WinnerChanged)
    } else {
        None
    }
}

pub(super) async fn lock_admission_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    output_scope: Option<df_assets::QuotaScope>,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<(), PublicationError> {
    let pins = df_imagery::runtime::retained_sources(input.job)
        .map_err(|_| PublicationError::InvalidRecord)?;
    if pins.len() != input.sources.len()
        || pins
            .iter()
            .zip(input.sources)
            .any(|(pin, receipt)| pin != &receipt.pin)
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Some(authority) = authority {
        authority
            .lock_authority(tx, &input.job.owner, now_ms)
            .await?;
    } else if !pins.is_empty() || input.config.accounting.mode == df_imagery::AccountingMode::Hosted
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Some(scope) = output_scope {
        crate::media_catalog_sql::lock_quota(tx, scope)
            .await
            .map_err(PublicationError::Catalog)?;
    }
    let output = output_scope
        .map(|_| {
            AssetId::from_bytes(
                &input
                    .stage
                    .planned_publication
                    .ok_or(PublicationError::InvalidRecord)?
                    .asset,
            )
            .map_err(|_| PublicationError::InvalidRecord)
        })
        .transpose()?;
    let mut assets = pins
        .iter()
        .map(|pin| pin.asset)
        .chain(output)
        .collect::<Vec<_>>();
    assets.sort_unstable_by_key(|asset| *asset.as_bytes());
    assets.dedup();
    for asset in assets {
        let row = tx
            .query_opt(
                "SELECT asset_id FROM media.asset WHERE asset_id=$1 FOR UPDATE",
                &[&asset.as_bytes().as_slice()],
            )
            .await
            .map_err(database_error)?;
        if row.is_none() && Some(asset) != output {
            return Err(PublicationError::NotFound);
        }
    }
    let requested = df_imagery::runtime::retained_audience(input.job)
        .map_err(|_| PublicationError::InvalidRecord)?;
    let mut facts = Vec::with_capacity(pins.len());
    for (pin, receipt) in pins.iter().zip(input.sources) {
        let fact = read_source(tx, pin, &requested).await?;
        if !matches_source_receipt(&fact, receipt) {
            return Err(PublicationError::BindingConflict);
        }
        facts.push(fact);
    }
    if let Some(authority) = authority {
        authority
            .authorize_sources(tx, input.job, &facts, now_ms)
            .await?;
    }
    Ok(())
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), PublicationError> {
    let length = u64::try_from(value.len()).map_err(|_| PublicationError::InvalidRecord)?;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

fn bind_source(bytes: &mut Vec<u8>, fact: &PublicationSource) -> Result<(), PublicationError> {
    append(bytes, fact.source.asset.as_bytes())?;
    append(bytes, fact.asset.attempt.as_bytes())?;
    append(bytes, fact.master.write.as_bytes())?;
    append(bytes, fact.master.key.path().as_bytes())?;
    append(
        bytes,
        fact.master
            .version
            .as_ref()
            .ok_or(PublicationError::InvalidRecord)?
            .as_str()
            .as_bytes(),
    )?;
    let manifest = fact
        .master
        .manifest
        .ok_or(PublicationError::InvalidRecord)?;
    bytes.extend_from_slice(&manifest.byte_len.to_be_bytes());
    bytes.extend_from_slice(&manifest.sha256);
    bytes.extend_from_slice(&fact.asset.suppression_generation.to_be_bytes());
    append(bytes, fact.asset.retention.label().as_bytes())?;
    fact.asset.audience.encode_into(bytes);
    Ok(())
}

async fn read_source(
    tx: &Transaction<'_>,
    pin: &SourceRef,
    requested: &df_assets::Audience,
) -> Result<PublicationSource, PublicationError> {
    let asset = crate::media_catalog_sql::asset_in(tx, pin.asset)
        .await
        .map_err(PublicationError::Catalog)?
        .ok_or(PublicationError::BindingConflict)?;
    if asset.kind != AssetKind::Image {
        return Err(PublicationError::BindingConflict);
    }
    if tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM media.tombstone WHERE asset_id=$1)",
            &[&pin.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .get::<_, bool>(0)
    {
        return Err(PublicationError::SourceDenied(SourceDenial::Tombstoned));
    }
    if asset.state != AssetState::Ready {
        return Err(PublicationError::SourceDenied(SourceDenial::NotReady));
    }
    if asset.audience != pin.audience {
        return Err(PublicationError::SourceDenied(
            SourceDenial::AudienceChanged,
        ));
    }
    if requested.meet(&asset.audience) != *requested {
        return Err(PublicationError::BindingConflict);
    }
    let published = asset
        .published
        .as_ref()
        .ok_or(PublicationError::InvalidRecord)?;
    if published.sha256 != *pin.digest.as_bytes() {
        return Err(PublicationError::SourceDenied(SourceDenial::WinnerChanged));
    }
    let winners = asset
        .winning_writes
        .iter()
        .map(|write| write.as_bytes().to_vec())
        .collect::<Vec<_>>();
    let rows=tx.query("SELECT * FROM media.asset_variant WHERE asset_id=$1 AND attempt=$2 AND variant='master' AND state='written' AND write_id=ANY($3) LIMIT 2", &[&pin.asset.as_bytes().as_slice(),&asset.attempt.as_bytes().as_slice(),&winners]).await.map_err(database_error)?;
    let [row] = rows.as_slice() else {
        return Err(PublicationError::BindingConflict);
    };
    let domain = StorageDomain::from_label(&row.get::<_, String>("object_domain"))
        .filter(|domain| {
            matches!(
                domain,
                StorageDomain::PublicStatic | StorageDomain::PrivateMedia
            )
        })
        .ok_or(PublicationError::InvalidRecord)?;
    let master = VariantRecord {
        asset: pin.asset,
        attempt: asset.attempt,
        write: WriteId::from_bytes(&row.get::<_, Vec<u8>>("write_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        variant: AssetVariant::Master,
        owner: ClaimToken::from_bytes(&row.get::<_, Vec<u8>>("owner"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        state: VariantState::Written,
        key: ObjectKey::from_stored(domain, &row.get::<_, String>("object_key"))
            .ok_or(PublicationError::InvalidRecord)?,
        version: row
            .get::<_, Option<String>>("object_version")
            .and_then(ObjectVersion::new),
        manifest: Some(AssetManifest {
            byte_len: u64::try_from(
                row.get::<_, Option<i64>>("byte_length")
                    .ok_or(PublicationError::InvalidRecord)?,
            )
            .map_err(|_| PublicationError::InvalidRecord)?,
            sha256: row
                .get::<_, Option<Vec<u8>>>("sha256")
                .ok_or(PublicationError::InvalidRecord)?
                .try_into()
                .map_err(|_| PublicationError::InvalidRecord)?,
        }),
        created_at_ms: row.get("created_at_ms"),
    };
    if master.version.is_none() {
        return Err(PublicationError::InvalidRecord);
    }
    if master
        .manifest
        .is_none_or(|manifest| manifest.sha256 != *pin.digest.as_bytes())
    {
        return Err(PublicationError::SourceDenied(SourceDenial::WinnerChanged));
    }
    Ok(PublicationSource {
        source: pin.clone(),
        asset,
        master,
    })
}

pub(super) async fn lock_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    output_id: AssetId,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<LockedSources, PublicationError> {
    let access = lock_for_closure_in(tx, scope, output_id, authority, now_ms).await?;
    if let Some(reason) = access.denial {
        return Err(PublicationError::SourceDenied(reason));
    }
    Ok(access.locked)
}

pub(super) async fn lock_for_closure_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    output_id: AssetId,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<LockedSourceAccess, PublicationError> {
    lock_with_owner_in(tx, scope, output_id, authority, &scope.owner, now_ms).await
}

pub(super) async fn lock_artifact_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    output_id: AssetId,
    authority: &dyn PublicationSourceAuthority,
    current_owner: &ImageOwner,
    now_ms: i64,
) -> Result<LockedSources, PublicationError> {
    let access =
        lock_with_owner_in(tx, scope, output_id, Some(authority), current_owner, now_ms).await?;
    if let Some(reason) = access.denial {
        return Err(PublicationError::SourceDenied(reason));
    }
    Ok(access.locked)
}

async fn lock_with_owner_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    output_id: AssetId,
    authority: Option<&dyn PublicationSourceAuthority>,
    authority_owner: &ImageOwner,
    now_ms: i64,
) -> Result<LockedSourceAccess, PublicationError> {
    let job = crate::read_in(tx, &scope.owner, scope.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let pins =
        df_imagery::runtime::retained_sources(&job).map_err(|_| PublicationError::InvalidRecord)?;
    let requested = df_imagery::runtime::retained_audience(&job)
        .map_err(|_| PublicationError::InvalidRecord)?;
    let receipts = crate::imagery_runtime::read_sources_in(tx, scope)
        .await
        .map_err(|error| match error {
            df_imagery::runtime::RuntimeError::InvalidRecord
            | df_imagery::runtime::RuntimeError::SourceDenied => PublicationError::BindingConflict,
            df_imagery::runtime::RuntimeError::Execution(error) => {
                PublicationError::Execution(error)
            }
            _ => PublicationError::Unavailable,
        })?
        .ok_or(PublicationError::BindingConflict)?;
    if receipts.len() != pins.len()
        || pins
            .iter()
            .zip(&receipts)
            .any(|(pin, receipt)| pin != &receipt.pin)
    {
        return Err(PublicationError::BindingConflict);
    }
    let receipt_digest = crate::imagery_runtime::sources_digest_in(tx, scope)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::BindingConflict)?;
    let mut denial = None;
    let mut denied_asset = None;
    if let Some(authority) = authority {
        match authority.lock_authority(tx, authority_owner, now_ms).await {
            Ok(()) => {}
            Err(PublicationError::SourceDenied(reason)) => denial = Some(reason),
            Err(error) => return Err(error),
        }
    } else if !pins.is_empty()
        || df_imagery::runtime::retained_config(&job)
            .map_err(|_| PublicationError::InvalidRecord)?
            .accounting
            .mode
            == df_imagery::AccountingMode::Hosted
    {
        return Err(PublicationError::BindingConflict);
    }
    let output = crate::media_catalog_sql::asset_in(tx, output_id)
        .await
        .map_err(PublicationError::Catalog)?
        .ok_or(PublicationError::NotFound)?;
    crate::media_catalog_sql::lock_quota(tx, output.charge_scope)
        .await
        .map_err(PublicationError::Catalog)?;
    let mut assets = pins
        .iter()
        .map(|pin| pin.asset)
        .chain([output_id])
        .collect::<Vec<_>>();
    assets.sort_unstable_by_key(|asset| *asset.as_bytes());
    assets.dedup();
    for asset in assets {
        tx.query_opt(
            "SELECT asset_id FROM media.asset WHERE asset_id=$1 FOR UPDATE",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .ok_or(PublicationError::NotFound)?;
    }
    let locked = crate::media_catalog_sql::asset_in(tx, output_id)
        .await
        .map_err(PublicationError::Catalog)?
        .ok_or(PublicationError::NotFound)?;
    if output.charge_scope != locked.charge_scope {
        return Err(PublicationError::BindingConflict);
    }
    if tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM media.tombstone WHERE asset_id=$1)",
            &[&output_id.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .get::<_, bool>(0)
    {
        return Err(PublicationError::SourceDenied(SourceDenial::Tombstoned));
    }
    let mut sources = Vec::with_capacity(pins.len());
    let mut binding = b"df-imagery/publication-sources-v1".to_vec();
    for (pin, receipt) in pins.into_iter().zip(receipts) {
        let source = match read_source(tx, &pin, &requested).await {
            Ok(source) => source,
            Err(PublicationError::SourceDenied(reason)) => {
                if denial.is_none() {
                    denial = Some(reason);
                    denied_asset = Some(pin.asset);
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(reason) = receipt_denial(&source, &receipt)
            && denial.is_none()
        {
            denial = Some(reason);
            denied_asset = Some(pin.asset);
        }
        bind_source(&mut binding, &source)?;
        sources.push(source);
    }
    Ok(LockedSourceAccess {
        locked: LockedSources {
            job,
            output: locked,
            sources,
            binding,
        },
        denial,
        denied_asset,
        receipt_digest,
    })
}

pub(super) async fn authorize_in(
    tx: &Transaction<'_>,
    locked: &LockedSources,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<(), PublicationError> {
    let current = crate::read_in(tx, &locked.job.owner, locked.job.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    if current.intent != locked.job.intent
        || current.config != locked.job.config
        || current.recipe != locked.job.recipe
        || current.resolved_key != locked.job.resolved_key
        || current.state != locked.job.state
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Some(authority) = authority {
        authority
            .authorize_sources(tx, &current, &locked.sources, now_ms)
            .await
    } else if locked.sources.is_empty()
        && df_imagery::runtime::retained_config(&current)
            .map_err(|_| PublicationError::InvalidRecord)?
            .accounting
            .mode
            == df_imagery::AccountingMode::RecordOnly
    {
        Ok(())
    } else {
        Err(PublicationError::BindingConflict)
    }
}
