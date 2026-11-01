//! Current host authority and immutable catalog reads for central image references.

use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use df_assets::{
    AssetKind, AssetRecord, AssetState, AssetVariant, Deadline, KeyLayout, ObjectStore,
    StoredObject, VariantState, record_owner,
};
use df_imagery::{
    ImageOwner, SourceRef,
    runtime::{AuthorizedImageSource, ImageSources, RuntimeError, SourceReceipt},
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio_postgres::{Client, Transaction};

use super::imagery_identity::{
    ExactAssetAllowlist, ImageryAuthorizer, ImageryPrincipal, LiveGrant, LocalRunGrant,
    ResolveError, resolve_hosted_principal, resolve_local_principal, resolve_local_run_principal,
};

/// Captured server-side proof, never accepted from an image request or diagnostic payload.
#[derive(Clone)]
pub(super) enum CurrentImageGrant {
    Hosted(LiveGrant),
    LocalRun(LocalRunGrant),
    LocalGuest {
        member: [u8; 16],
        session: [u8; 16],
        credential: [u8; 32],
    },
}

impl CurrentImageGrant {
    pub(super) async fn resolve(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<ImageryPrincipal, RuntimeError> {
        let result = match self {
            Self::Hosted(grant) => resolve_hosted_principal(transaction, grant).await,
            Self::LocalRun(grant) => resolve_local_run_principal(transaction, grant).await,
            Self::LocalGuest {
                member,
                session,
                credential,
            } => resolve_local_principal(transaction, member, session, credential).await,
        };
        result.map_err(|error| match error {
            ResolveError::Refused => RuntimeError::SourceDenied,
            ResolveError::Unavailable => RuntimeError::StorageUnavailable,
        })
    }

    async fn lock_publication_principal(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<ImageryPrincipal, df_imagery::publication::PublicationError> {
        use df_imagery::publication::{PublicationError, SourceDenial};
        use df_persistence::imagery_identity::{self as identity, AuthorityEvidence};
        let evidence = match self {
            Self::Hosted(grant) => {
                identity::lock_hosted_authority_evidence(
                    transaction,
                    &identity::HostedAuthorityCheck {
                        tenant: grant.tenant,
                        campaign: grant.campaign,
                        hosted_session: grant.hosted_session,
                        member: grant.member,
                        generation: grant.generation,
                        web_session: grant.web_session,
                        gameplay_credential: grant.gameplay_credential,
                    },
                )
                .await
            }
            Self::LocalRun(grant) => {
                identity::lock_local_run_authority_evidence(transaction, grant).await
            }
            Self::LocalGuest {
                member,
                session,
                credential,
            } => {
                identity::lock_local_guest_authority_evidence(
                    transaction,
                    member,
                    session,
                    credential,
                )
                .await
            }
        }
        .map_err(|_| PublicationError::Unavailable)?;
        match evidence {
            AuthorityEvidence::Refused => {
                Err(PublicationError::SourceDenied(SourceDenial::GrantRefused))
            }
            AuthorityEvidence::Unproven => Err(PublicationError::BindingConflict),
            AuthorityEvidence::Current => {
                self.resolve(transaction)
                    .await
                    .map_err(|error| match error {
                        RuntimeError::StorageUnavailable => PublicationError::Unavailable,
                        // An unexpected later refusal is not a locked negative proof.
                        _ => PublicationError::BindingConflict,
                    })
            }
        }
    }

    /// Caller holds catalog source locks before invoking this same-transaction host check.
    pub(super) async fn authorize_catalog_sources(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        sources: &[SourceRef],
    ) -> Result<ImageryPrincipal, RuntimeError> {
        let principal = self.resolve(transaction).await?;
        let policy = ImageryAuthorizer::new(ExactAssetAllowlist::default());
        for source in sources {
            policy
                .authorize_catalog_source(&principal, owner, source)
                .map_err(|_| RuntimeError::SourceDenied)?;
        }
        Ok(principal)
    }
}

/// Proof custody is local to one mounted database/session; every use resolves current authority.
#[derive(Clone)]
pub(super) struct CurrentImageGrants {
    entries: Arc<RwLock<HashMap<Vec<u8>, RegisteredImageGrant>>>,
    maximum: usize,
    capacity_refusals: Arc<AtomicU64>,
}
#[derive(Clone)]
struct RegisteredImageGrant {
    owner: ImageOwner,
    grant: CurrentImageGrant,
}
#[derive(Debug)]
pub(super) enum GrantRegistrationError {
    Capacity { maximum: usize },
    Authority(RuntimeError),
}
#[derive(Clone, Copy)]
pub(super) struct GrantDiagnostics {
    pub(super) registered: usize,
    pub(super) maximum: usize,
    pub(super) capacity_refusals: u64,
}
impl CurrentImageGrants {
    pub(super) fn new(maximum: usize) -> Result<Self, RuntimeError> {
        if maximum == 0 {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            maximum,
            capacity_refusals: Arc::new(AtomicU64::new(0)),
        })
    }
    pub(super) fn diagnostics(&self) -> Result<GrantDiagnostics, RuntimeError> {
        Ok(GrantDiagnostics {
            registered: self
                .entries
                .read()
                .map_err(|_| RuntimeError::StorageUnavailable)?
                .len(),
            maximum: self.maximum,
            capacity_refusals: self.capacity_refusals.load(Ordering::Relaxed),
        })
    }
    pub(super) async fn register(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        grant: CurrentImageGrant,
    ) -> Result<ImageryPrincipal, GrantRegistrationError> {
        use df_imagery::ImageAuthorizer;
        let principal = grant
            .resolve(transaction)
            .await
            .map_err(GrantRegistrationError::Authority)?;
        ImageryAuthorizer::new(ExactAssetAllowlist::default())
            .authorize_owner(&principal, owner, df_imagery::ServiceAction::Read)
            .map_err(|_| GrantRegistrationError::Authority(RuntimeError::SourceDenied))?;
        self.store_verified(owner, grant)?;
        Ok(principal)
    }
    // Only register, after canonical current-owner verification, enters this storage step.
    fn store_verified(
        &self,
        owner: &ImageOwner,
        grant: CurrentImageGrant,
    ) -> Result<(), GrantRegistrationError> {
        let key = df_imagery::owner_scope_bytes(owner);
        let mut grants = self
            .entries
            .write()
            .map_err(|_| GrantRegistrationError::Authority(RuntimeError::StorageUnavailable))?;
        if grants.len() >= self.maximum && !grants.contains_key(&key) {
            // A saturated diagnostic count stays visible instead of wrapping to zero.
            let _recorded = self.capacity_refusals.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |count| count.checked_add(1),
            );
            return Err(GrantRegistrationError::Capacity {
                maximum: self.maximum,
            });
        }
        // No proof is evicted: pending/Unknown work retains its exact current custody entry.
        grants.insert(
            key,
            RegisteredImageGrant {
                owner: owner.clone(),
                grant,
            },
        );
        Ok(())
    }

    fn grant_for(&self, owner: &ImageOwner) -> Result<CurrentImageGrant, RuntimeError> {
        self.entries
            .read()
            .map_err(|_| RuntimeError::StorageUnavailable)?
            .get(&df_imagery::owner_scope_bytes(owner))
            .filter(|registered| registered.owner == *owner)
            .map(|registered| registered.grant.clone())
            .ok_or(RuntimeError::SourceDenied)
    }

    pub(super) async fn resolve(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
    ) -> Result<ImageryPrincipal, RuntimeError> {
        let grant = self.grant_for(owner)?;
        let principal = grant.resolve(transaction).await?;
        df_imagery::ImageAuthorizer::authorize_owner(
            &ImageryAuthorizer::new(ExactAssetAllowlist::default()),
            &principal,
            owner,
            df_imagery::ServiceAction::Read,
        )
        .map_err(|_| RuntimeError::SourceDenied)?;
        Ok(principal)
    }

    pub(super) async fn authorize_catalog_sources(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        sources: &[SourceRef],
    ) -> Result<ImageryPrincipal, RuntimeError> {
        self.grant_for(owner)?
            .authorize_catalog_sources(transaction, owner, sources)
            .await
    }
}

/// A host-owned connection is driven and shut down by the mounted runtime owner.
/// Credential and catalog source locks remain held through the version-pinned object read.
pub(super) struct CatalogImageSources<S> {
    pub(super) objects: Arc<S>,
    pub(super) authority: Arc<Mutex<Client>>,
    pub(super) grants: CurrentImageGrants,
    pub(super) layout: KeyLayout,
    pub(super) maximum_bytes: u64,
    pub(super) deadline: Duration,
    pub(super) analysis: std::sync::OnceLock<Arc<dyn AnalysisRead>>,
}

impl<S: ObjectStore> ImageSources for CatalogImageSources<S> {
    async fn read_analysis(
        &self,
        owner: &ImageOwner,
        handle: &df_imagery::OperationHandle,
    ) -> Result<df_imagery::ImageAnalysisObservation, RuntimeError> {
        self.analysis
            .get()
            .ok_or(RuntimeError::UnsupportedCapability)?
            .read(owner, handle)
            .await
    }

    async fn read_source(
        &self,
        owner: &ImageOwner,
        source: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        if self.maximum_bytes == 0 || self.deadline.is_zero() {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let deadline = Deadline::after(self.deadline);
        let mut connection = self.authority.lock().await;
        let transaction = connection
            .transaction()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let principal = self.grants.resolve(&transaction, owner).await?;
        df_imagery::ImageAuthorizer::authorize_owner(
            &ImageryAuthorizer::new(ExactAssetAllowlist::default()),
            &principal,
            owner,
            df_imagery::ServiceAction::Read,
        )
        .map_err(|_| RuntimeError::SourceDenied)?;
        let facts = df_persistence::imagery_publication::lock_image_sources_in(
            &transaction,
            std::slice::from_ref(source),
            &source.audience,
        )
        .await
        .map_err(source_publication_error)?;
        let fact = facts.first().ok_or(RuntimeError::SourceDenied)?;
        check_source(&fact.asset, source)?;
        self.grants
            .authorize_catalog_sources(&transaction, owner, std::slice::from_ref(source))
            .await?;
        let stored = self.winning_master(fact)?;
        let maximum_bytes = self.source_read_bound(&transaction, fact).await?;
        if stored.manifest.byte_len == 0 || stored.manifest.byte_len > maximum_bytes {
            return Err(RuntimeError::SourceDenied);
        }
        let bytes = self
            .objects
            .get_version(&stored, deadline)
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        if bytes.len() as u64 != stored.manifest.byte_len
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != *source.digest.as_bytes()
        {
            return Err(RuntimeError::SourceDenied);
        }
        let receipt = SourceReceipt {
            pin: source.clone(),
            attempt: fact.asset.attempt,
            write: fact.master.write,
            version: stored.version,
            suppression_generation: fact.asset.suppression_generation,
            retention: fact.asset.retention,
        };
        // A failed acknowledgement never exposes bytes under uncertain authority.
        transaction
            .commit()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        Ok(AuthorizedImageSource { bytes, receipt })
    }
}

impl<S: ObjectStore> CatalogImageSources<S> {
    pub(super) fn winning_master(
        &self,
        fact: &df_persistence::imagery_publication::PublicationSource,
    ) -> Result<StoredObject, RuntimeError> {
        let row = &fact.asset;
        let source = &fact.source;
        let winner = &fact.master;
        let owner = record_owner(row).ok_or(RuntimeError::SourceDenied)?;
        if winner.asset != row.asset
            || winner.attempt != row.attempt
            || winner.variant != AssetVariant::Master
            || winner.state != VariantState::Written
            || !row.winning_writes.contains(&winner.write)
            || winner.key
                != self.layout.owned_asset(
                    &owner,
                    row.asset,
                    row.attempt,
                    winner.write,
                    AssetVariant::Master,
                )
        {
            return Err(RuntimeError::SourceDenied);
        }
        let manifest = winner.manifest.ok_or(RuntimeError::SourceDenied)?;
        if manifest.sha256 != *source.digest.as_bytes() {
            return Err(RuntimeError::SourceDenied);
        }
        Ok(StoredObject {
            key: winner.key.clone(),
            version: winner.version.clone().ok_or(RuntimeError::SourceDenied)?,
            manifest,
        })
    }
}

fn source_publication_error(error: df_imagery::publication::PublicationError) -> RuntimeError {
    match error {
        df_imagery::publication::PublicationError::Unavailable
        | df_imagery::publication::PublicationError::Unknown => RuntimeError::StorageUnavailable,
        _ => RuntimeError::SourceDenied,
    }
}

fn check_source(row: &AssetRecord, source: &SourceRef) -> Result<(), RuntimeError> {
    if row.asset != source.asset
        || row.kind != AssetKind::Image
        || row.state != AssetState::Ready
        || row.audience != source.audience
        || row.ready_at_ms.is_none()
        || row
            .published
            .as_ref()
            .is_none_or(|facts| facts.sha256 != *source.digest.as_bytes())
    {
        return Err(RuntimeError::SourceDenied);
    }
    Ok(())
}

impl CurrentImageGrant {
    // Only the registry invokes this after checking its retained, verified owner binding.
    fn lock_registered_authority<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        owner: &'a ImageOwner,
        _now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            use df_imagery::{ImageAuthorizer, ServiceAction, publication::PublicationError};
            let principal = self.lock_publication_principal(transaction).await?;
            ImageryAuthorizer::new(ExactAssetAllowlist::default())
                .authorize_owner(&principal, owner, ServiceAction::Read)
                .map_err(|_| PublicationError::BindingConflict)
        })
    }

    fn authorize_publication_sources<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        job: &'a df_imagery::ImageJob,
        sources: &'a [df_persistence::imagery_publication::PublicationSource],
        _now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            use df_imagery::publication::PublicationError;
            for source in sources {
                check_source(&source.asset, &source.source)
                    .map_err(|_| PublicationError::BindingConflict)?;
            }
            let pins = sources
                .iter()
                .map(|source| source.source.clone())
                .collect::<Vec<_>>();
            let principal = self
                .authorize_catalog_sources(transaction, &job.owner, &pins)
                .await
                .map_err(|error| match error {
                    RuntimeError::StorageUnavailable => PublicationError::Unavailable,
                    _ => PublicationError::BindingConflict,
                })?;
            let config = df_imagery::runtime::retained_config(job)
                .map_err(|_| PublicationError::BindingConflict)?;
            if config.accounting.mode == df_imagery::AccountingMode::Hosted
                && principal.mode() != super::imagery_identity::AuthorityMode::Hosted
            {
                return Err(PublicationError::BindingConflict);
            }
            Ok(())
        })
    }
}

impl CurrentImageGrants {
    async fn authorize_artifact_access(
        &self,
        transaction: &Transaction<'_>,
        owner: &ImageOwner,
        output: &AssetRecord,
        sources: &[df_persistence::imagery_publication::PublicationSource],
    ) -> Result<(), df_imagery::publication::PublicationError> {
        use df_imagery::{ImageAuthorizer, ServiceAction, publication::PublicationError};
        let ImageOwner::Run { campaign, .. } = owner else {
            return Err(PublicationError::BindingConflict);
        };
        if output.audience != df_assets::Audience::CampaignShared(*campaign) {
            return Err(PublicationError::BindingConflict);
        }
        for source in sources {
            check_source(&source.asset, &source.source)
                .map_err(|_| PublicationError::BindingConflict)?;
        }
        let pins = sources
            .iter()
            .map(|source| source.source.clone())
            .collect::<Vec<_>>();
        let principal = self
            .authorize_catalog_sources(transaction, owner, &pins)
            .await
            .map_err(|error| match error {
                RuntimeError::StorageUnavailable => PublicationError::Unavailable,
                _ => PublicationError::BindingConflict,
            })?;
        let policy = ImageryAuthorizer::new(ExactAssetAllowlist::default());
        policy
            .authorize_owner(&principal, owner, ServiceAction::Read)
            .and_then(|()| policy.authorize_audience(&principal, owner, &output.audience))
            .map_err(|_| PublicationError::BindingConflict)
    }
}

impl df_persistence::imagery_publication::PublicationSourceAuthority for CurrentImageGrants {
    fn lock_authority<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        owner: &'a ImageOwner,
        now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            let grant = self.grant_for(owner).map_err(|error| match error {
                RuntimeError::StorageUnavailable => {
                    df_imagery::publication::PublicationError::Unavailable
                }
                _ => df_imagery::publication::PublicationError::BindingConflict,
            })?;
            grant
                .lock_registered_authority(transaction, owner, now_ms)
                .await
        })
    }
    fn authorize_sources<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        job: &'a df_imagery::ImageJob,
        sources: &'a [df_persistence::imagery_publication::PublicationSource],
        now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            let grant = self.grant_for(&job.owner).map_err(|error| match error {
                RuntimeError::StorageUnavailable => {
                    df_imagery::publication::PublicationError::Unavailable
                }
                _ => df_imagery::publication::PublicationError::BindingConflict,
            })?;
            grant
                .authorize_publication_sources(transaction, job, sources, now_ms)
                .await
        })
    }
    fn authorize_artifact<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        request: &'a df_imagery::ImageRequest,
        output: &'a AssetRecord,
        sources: &'a [df_persistence::imagery_publication::PublicationSource],
        _now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            use df_imagery::{
                ArtworkSubject, Freshness, Purpose, VariantRole, publication::PublicationError,
            };
            let ImageOwner::Run { campaign, run } = &request.identity.owner else {
                return Err(PublicationError::BindingConflict);
            };
            let artwork = request
                .operation
                .artwork()
                .ok_or(PublicationError::BindingConflict)?;
            let purpose = match &artwork.spec.subject {
                ArtworkSubject::Hero(_) | ArtworkSubject::Npc(_) | ArtworkSubject::Cast(_) => {
                    Purpose::CanonicalPerson
                }
                ArtworkSubject::Item(_) => Purpose::ItemCloseUp,
            };
            if artwork.spec.campaign != *campaign
                || artwork.selection.run != *run
                || request.audience != df_assets::Audience::CampaignShared(*campaign)
                || request.purpose.purpose != purpose
                || request.purpose.role != VariantRole::Primary
                || request.freshness != Freshness::ReuseCompatible
                || request
                    .operation
                    .pipeline()
                    .is_some_and(|pipeline| pipeline.creator.is_some())
            {
                return Err(PublicationError::BindingConflict);
            }
            self.authorize_artifact_access(transaction, &request.identity.owner, output, sources)
                .await
        })
    }

    fn authorize_artifact_read<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        current_owner: &'a ImageOwner,
        output: &'a AssetRecord,
        sources: &'a [df_persistence::imagery_publication::PublicationSource],
        _now_ms: i64,
    ) -> df_persistence::imagery_publication::PublicationAuthorityFuture<'a> {
        // Native publication verifies the retained eligible artwork recipe and exact consumed
        // source receipts first; this callback supplies only current run/campaign authority.
        Box::pin(self.authorize_artifact_access(transaction, current_owner, output, sources))
    }
}

impl<S: ObjectStore> CatalogImageSources<S> {
    async fn source_read_bound(
        &self,
        transaction: &Transaction<'_>,
        source: &df_persistence::imagery_publication::PublicationSource,
    ) -> Result<u64, RuntimeError> {
        // Current grant and exact catalog winner remain locked. Imported inputs keep their
        // separate reviewed policy; generated output keeps its immutable producer limit.
        let retained = df_persistence::imagery_publication::read_generated_source_bound_in(
            transaction,
            &source.asset,
            &source.master,
        )
        .await?;
        source_byte_bound(source.asset.provenance, retained, self.maximum_bytes)
    }

    pub(super) async fn read_committed_portrait(
        &self,
        owner: &ImageOwner,
        approved: &df_model::checkpoint::AssetReference,
        proof: &dyn super::imagery::CommittedPortraitProof,
    ) -> Result<Vec<u8>, RuntimeError> {
        if !matches!(owner, ImageOwner::Run { .. })
            || approved.kind != df_model::checkpoint::AssetKind::Image
            || approved.byte_length == 0
        {
            return Err(RuntimeError::SourceDenied);
        }
        let key = super::imagery::ImageKey::parse(approved.key.as_str())
            .ok_or(RuntimeError::SourceDenied)?;
        let operation = super::imagery::central::operation_for_alias(&key)
            .map_err(|_| RuntimeError::SourceDenied)?;
        let mut connection = self.authority.lock().await;
        let transaction = connection
            .transaction()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let principal = self.grants.resolve(&transaction, owner).await?;
        df_imagery::ImageAuthorizer::authorize_owner(
            &ImageryAuthorizer::new(ExactAssetAllowlist::default()),
            &principal,
            owner,
            df_imagery::ServiceAction::Read,
        )
        .map_err(|_| RuntimeError::SourceDenied)?;
        proof
            .authorize(owner, approved)
            .map_err(|_| RuntimeError::SourceDenied)?;
        let length = i64::try_from(approved.byte_length).map_err(|_| RuntimeError::SourceDenied)?;
        let rows = transaction
            .query(
                "SELECT r.asset_id,o.owner_scope,a.audience_account FROM imagery.image_operation o \
             JOIN imagery.image_job_result r ON r.owner_scope=o.owner_scope AND r.job_id=o.job_id \
             JOIN media.asset a ON a.asset_id=r.asset_id \
             WHERE o.operation_id=$1 AND NOT r.private_reconciliation \
               AND a.kind='image' AND a.state='ready' AND a.audience_kind='owner_private' \
               AND a.sha256=$2 AND a.byte_length=$3 LIMIT 2",
                &[
                    &operation.as_bytes().as_slice(),
                    &approved.digest.0.as_slice(),
                    &length,
                ],
            )
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        if rows.len() != 1 {
            return Err(RuntimeError::SourceDenied);
        }
        let row = rows.first().ok_or(RuntimeError::SourceDenied)?;
        let asset: Vec<u8> = row.try_get(0).map_err(|_| RuntimeError::SourceDenied)?;
        let scope: Vec<u8> = row.try_get(1).map_err(|_| RuntimeError::SourceDenied)?;
        let account: Vec<u8> = row.try_get(2).map_err(|_| RuntimeError::SourceDenied)?;
        let account =
            df_types::AccountId::from_bytes(&account).map_err(|_| RuntimeError::SourceDenied)?;
        if df_imagery::owner_of_bytes(&scope) != Some(ImageOwner::Draft { account }) {
            return Err(RuntimeError::SourceDenied);
        }
        let pin = SourceRef {
            asset: df_assets::AssetId::from_bytes(&asset)
                .map_err(|_| RuntimeError::SourceDenied)?,
            digest: df_imagery::AssetDigest::from_bytes(approved.digest.0),
            audience: df_assets::Audience::OwnerPrivate(account),
        };
        // The committed Work is the additional share authority here. Ordinary Run source
        // reads still reject OwnerPrivate; this path never returns a private SourceRef.
        let facts = df_persistence::imagery_publication::lock_image_sources_in(
            &transaction,
            std::slice::from_ref(&pin),
            &pin.audience,
        )
        .await
        .map_err(source_publication_error)?;
        let fact = facts.first().ok_or(RuntimeError::SourceDenied)?;
        check_source(&fact.asset, &pin)?;
        let stored = self.winning_master(fact)?;
        let maximum_bytes = self.source_read_bound(&transaction, fact).await?;
        if stored.manifest.byte_len != approved.byte_length
            || stored.manifest.byte_len > maximum_bytes
        {
            return Err(RuntimeError::SourceDenied);
        }
        let bytes = self
            .objects
            .get_version(&stored, Deadline::after(self.deadline))
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        if bytes.len() as u64 != approved.byte_length
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != approved.digest.0
        {
            return Err(RuntimeError::SourceDenied);
        }
        proof
            .authorize(owner, approved)
            .map_err(|_| RuntimeError::SourceDenied)?;
        transaction
            .commit()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        Ok(bytes)
    }
}

fn source_byte_bound(
    provenance: df_assets::Provenance,
    generated_maximum_bytes: Option<u64>,
    reviewed_maximum_bytes: u64,
) -> Result<u64, RuntimeError> {
    match (provenance, generated_maximum_bytes) {
        (df_assets::Provenance::Generated, Some(maximum)) if maximum > 0 => Ok(maximum),
        (df_assets::Provenance::Generated, None) => Err(RuntimeError::StorageUnavailable),
        (_, None) if reviewed_maximum_bytes > 0 => Ok(reviewed_maximum_bytes),
        _ => Err(RuntimeError::InvalidRecord),
    }
}

pub(super) type AnalysisReadFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<df_imagery::ImageAnalysisObservation, RuntimeError>>
            + Send
            + 'a,
    >,
>;
pub(super) trait AnalysisRead: Send + Sync {
    fn read<'a>(
        &'a self,
        owner: &'a ImageOwner,
        handle: &'a df_imagery::OperationHandle,
    ) -> AnalysisReadFuture<'a>;
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    fn owner(index: u64) -> ImageOwner {
        let mut bytes = [1; 16];
        bytes[..8].copy_from_slice(&index.to_be_bytes());
        ImageOwner::Run {
            campaign: df_types::CampaignId::from_bytes(&[1; 16]).unwrap(),
            run: df_types::RunId::from_bytes(&bytes).unwrap(),
        }
    }
    // Storage fixtures never resolve or expose this proof as authority. Production enters
    // store_verified only after register completes actual canonical current-owner checks.
    fn storage_proof(owner: &ImageOwner, fence: u8) -> CurrentImageGrant {
        let ImageOwner::Run { run, .. } = owner else {
            unreachable!()
        };
        CurrentImageGrant::LocalRun(LocalRunGrant {
            session: [1; 16],
            run: *run.as_bytes(),
            fence: [fence; 16],
            bootstrap_credential: [3; 32],
        })
    }
    #[test]
    fn configured_proof_capacity_above_512_accepts_more_than_the_old_lifetime_limit() {
        let grants = CurrentImageGrants::new(600).unwrap();
        for index in 0..513 {
            let owner = owner(index);
            grants
                .store_verified(&owner, storage_proof(&owner, 2))
                .unwrap();
        }
        let diagnostics = grants.diagnostics().unwrap();
        assert_eq!(diagnostics.registered, 513);
        assert_eq!(diagnostics.maximum, 600);
        assert_eq!(diagnostics.capacity_refusals, 0);
        assert!(matches!(
            CurrentImageGrants::new(0),
            Err(RuntimeError::InvalidConfiguration)
        ));
    }
    #[test]
    fn retained_owner_mismatch_cannot_supply_a_publication_grant() {
        let grants = CurrentImageGrants::new(2).unwrap();
        let requested = owner(0);
        let other = owner(1);
        grants.entries.write().unwrap().insert(
            df_imagery::owner_scope_bytes(&requested),
            RegisteredImageGrant {
                owner: other.clone(),
                grant: storage_proof(&other, 2),
            },
        );
        assert!(matches!(
            grants.grant_for(&requested),
            Err(RuntimeError::SourceDenied)
        ));
    }

    #[test]
    fn full_capacity_preserves_all_existing_proofs_and_allows_verified_replacement() {
        let grants = CurrentImageGrants::new(2).unwrap();
        for index in 0..2 {
            let owner = owner(index);
            grants
                .store_verified(&owner, storage_proof(&owner, 2))
                .unwrap();
        }
        let extra = owner(2);
        assert!(matches!(
            grants.store_verified(&extra, storage_proof(&extra, 2)),
            Err(GrantRegistrationError::Capacity { maximum: 2 })
        ));
        for index in 0..2 {
            assert!(
                matches!(grants.grant_for(&owner(index)), Ok(CurrentImageGrant::LocalRun(grant)) if grant.fence == [2;16])
            );
        }
        let existing = owner(0);
        grants
            .store_verified(&existing, storage_proof(&existing, 4))
            .unwrap();
        assert!(
            matches!(grants.grant_for(&existing), Ok(CurrentImageGrant::LocalRun(grant)) if grant.fence == [4;16])
        );
        let diagnostics = grants.diagnostics().unwrap();
        assert_eq!(diagnostics.registered, 2);
        assert_eq!(diagnostics.capacity_refusals, 1);
    }
}

#[cfg(test)]
mod read_bound_tests {
    use super::*;
    #[test]
    fn missing_or_corrupt_generated_lineage_cannot_borrow_the_reviewed_input_limit() {
        assert!(matches!(
            source_byte_bound(df_assets::Provenance::Generated, None, 16 * 1024 * 1024),
            Err(RuntimeError::StorageUnavailable)
        ));
        assert!(matches!(
            source_byte_bound(df_assets::Provenance::Generated, Some(0), 16 * 1024 * 1024),
            Err(RuntimeError::InvalidRecord)
        ));
        assert_eq!(
            source_byte_bound(df_assets::Provenance::Generated, Some(32), 16).unwrap(),
            32
        );
        for provenance in [
            df_assets::Provenance::Reviewed,
            df_assets::Provenance::Uploaded,
            df_assets::Provenance::ClaimedLibrary,
        ] {
            assert_eq!(source_byte_bound(provenance, None, 16).unwrap(), 16);
        }
    }
}
