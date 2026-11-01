//! Canonical campaign finance ports for the mounted hosted image runtime.
use df_commerce::finance_ids::{CampaignKey, PayerId};
use df_finance::ports::{
    FencePort, FenceRequest, FenceVerdict, MediaPort, MediaRequest, MediaVerdict, MemberPort,
    PortUnavailable,
};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::Mutex;
use tokio_postgres::{Client, Transaction};

pub(super) struct CanonicalFence;
impl FencePort for CanonicalFence {
    async fn fence_current(
        &self,
        transaction: &Transaction<'_>,
        request: &FenceRequest,
    ) -> Result<FenceVerdict, PortUnavailable> {
        let row = transaction
            .query_one(
                "SELECT df_runtime.fence_current($1::bytea,$2::bytea,$3,$4,$5::bytea,'paid'), df_runtime.fence_current($1::bytea,$2::bytea,$3,$4,$5::bytea,'game')",
                &[
                    &request.tenant.as_bytes().as_slice(),
                    &request.campaign.as_bytes().as_slice(),
                    &request.recovery_epoch,
                    &request.claim_seq,
                    &request.placement_id.as_slice(),
                ],
            )
            .await
            .map_err(|_| PortUnavailable)?;
        Ok(if row.get::<_, bool>(0) {
            FenceVerdict::Current
        } else if row.get::<_, bool>(1) {
            FenceVerdict::PaidWorkFrozen
        } else {
            FenceVerdict::Stale
        })
    }
}

/// Provider stages use the actual CanonicalStagePort in the finance transaction.
/// Refuse the incompatible legacy media path rather than granting an extra allocation.
pub(super) struct CanonicalMedia;
impl MediaPort for CanonicalMedia {
    async fn admit_generation(
        &self,
        _transaction: &Transaction<'_>,
        _request: &MediaRequest,
    ) -> Result<MediaVerdict, PortUnavailable> {
        Err(PortUnavailable)
    }
}

pub(super) struct CanonicalMembers {
    client: Arc<Mutex<Client>>,
}
impl CanonicalMembers {
    pub(super) fn new(client: Client) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
        }
    }
}
impl MemberPort for CanonicalMembers {
    async fn authorized_payers(
        &self,
        campaign: CampaignKey,
        payers: &[PayerId],
    ) -> Result<BTreeSet<PayerId>, PortUnavailable> {
        let requested: Vec<Vec<u8>> = payers
            .iter()
            .map(|payer| payer.as_bytes().to_vec())
            .collect();
        // This is the campaign owner's one-statement member_snapshot authorization predicate.
        let rows = self.client.lock().await.query(
            "SELECT m.account_id FROM campaigns.campaign_member m JOIN campaigns.campaign c ON c.campaign_id=m.campaign_id JOIN accounts.account_status a ON a.account_id=m.account_id WHERE m.campaign_id=$1::bytea AND m.account_id=ANY($2::bytea[]) AND m.status='active' AND a.status='active'",
            &[&campaign.as_bytes().as_slice(), &requested],
        ).await.map_err(|_| PortUnavailable)?;
        rows.iter()
            .map(|row| {
                let bytes: Vec<u8> = row.try_get(0).map_err(|_| PortUnavailable)?;
                let bytes: [u8; 16] = bytes.try_into().map_err(|_| PortUnavailable)?;
                Ok(PayerId::from_array(bytes))
            })
            .collect()
    }
}

impl df_finance::ports::Clock for super::image_host::NativeClock {
    fn now_ms(&self) -> i64 {
        df_assets::Clock::now_ms(self)
    }
}
impl df_finance::ports::IdSource for super::image_host::NativeIds {
    fn next_id(&self) -> [u8; 16] {
        df_assets::IdSource::next_id_bytes(self)
    }
}
use super::{
    image_host::{NativeClock, NativeIds},
    image_sources::CurrentImageGrants,
};
use df_commerce::{admission::WorkClass, draft_image::StageCall};
use df_finance::service::{FinanceConfig, FinanceService};
use df_finance::{ImageAssetId, OwnerScope, ProviderStageAdmission, ProviderStageBinding};
use df_imagery::{
    ImageOwner, owner_scope_bytes,
    runtime::{DispatchAdmission, RuntimeError, SourceReceipt},
};
use df_persistence::{
    imagery_identity::HostRunBinding,
    imagery_runtime_admission::{
        FundedImageAdmission, RuntimeAdmissionPorts, RuntimeAdmissionSettings, RuntimeFunding,
        RuntimeImageAuthority,
    },
};

type HostPublisher = df_persistence::imagery_publication::NativeImagePublisher<
    df_assets::NativeFileStore,
    NativeIds,
    NativeClock,
>;
type CanonicalFinance = FinanceService<CanonicalFence, CanonicalMedia, CanonicalMembers>;
pub(super) type HostedAdmission = FundedImageAdmission<
    CanonicalFence,
    CanonicalMedia,
    CanonicalMembers,
    HostedRuntimeAuthority,
    HostPublisher,
>;

pub(super) struct HostedAdmissionInputs {
    pub(super) finance_client: Client,
    pub(super) member_client: Client,
    pub(super) authority_client: Client,
    pub(super) run: HostRunBinding,
    pub(super) fence: FenceRequest,
    pub(super) grants: CurrentImageGrants,
    pub(super) publisher: Arc<HostPublisher>,
    pub(super) usage: df_persistence::imagery_runtime::PostgresImageUsage,
    pub(super) settings: RuntimeAdmissionSettings,
    pub(super) ids: NativeIds,
}

pub(super) async fn open(
    mut inputs: HostedAdmissionInputs,
) -> Result<HostedAdmission, RuntimeError> {
    if inputs.run.tenant != *inputs.fence.tenant.as_bytes()
        || inputs.run.campaign != *inputs.fence.campaign.as_bytes()
    {
        return Err(RuntimeError::InvalidConfiguration);
    }
    let owner = ImageOwner::Run {
        campaign: df_types::CampaignId::from_bytes(&inputs.run.campaign)
            .map_err(|_| RuntimeError::InvalidConfiguration)?,
        run: df_types::RunId::from_bytes(&inputs.run.run)
            .map_err(|_| RuntimeError::InvalidConfiguration)?,
    };
    let transaction = inputs
        .authority_client
        .transaction()
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?;
    df_persistence::imagery_identity::require_hosted_identity_schema(&transaction)
        .await
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
    let principal = inputs.grants.resolve(&transaction, &owner).await?;
    if principal.mode() != df_persistence::imagery_identity::AuthorityMode::Hosted {
        return Err(RuntimeError::SourceDenied);
    }
    if CanonicalFence
        .fence_current(&transaction, &inputs.fence)
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?
        != FenceVerdict::Current
    {
        return Err(RuntimeError::SourceDenied);
    }
    transaction
        .commit()
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?;
    let finance = Arc::new(Mutex::new(FinanceService::new(
        inputs.finance_client,
        CanonicalFence,
        CanonicalMedia,
        CanonicalMembers::new(inputs.member_client),
        Box::new(NativeClock),
        Box::new(inputs.ids.clone()),
        FinanceConfig::default(),
    )));
    let authority = HostedRuntimeAuthority {
        finance: finance.clone(),
        run: inputs.run,
        fence: inputs.fence,
        grants: inputs.grants,
        ids: inputs.ids,
    };
    FundedImageAdmission::new(
        inputs.settings,
        RuntimeAdmissionPorts {
            finance,
            authority,
            local: inputs.publisher,
            usage: inputs.usage,
        },
    )
}

pub(super) struct HostedRuntimeAuthority {
    finance: Arc<Mutex<CanonicalFinance>>,
    run: HostRunBinding,
    fence: FenceRequest,
    grants: CurrentImageGrants,
    ids: NativeIds,
}

fn stored_id(row: &tokio_postgres::Row, column: &str) -> Result<[u8; 16], RuntimeError> {
    row.try_get::<_, Vec<u8>>(column)
        .map_err(|_| RuntimeError::InvalidRecord)?
        .try_into()
        .map_err(|_| RuntimeError::InvalidRecord)
}

impl HostedRuntimeAuthority {
    fn owns(&self, owner: &ImageOwner) -> bool {
        matches!(owner, ImageOwner::Run { campaign, run } if campaign.as_bytes() == &self.run.campaign && run.as_bytes() == &self.run.run)
    }

    fn command(&self, binding: ProviderStageBinding) -> ProviderStageAdmission {
        let route = df_finance::admit::priced_route(
            &binding.route.provider,
            &binding.route.model,
            &binding.route.route,
            &binding
                .route
                .maximum_units
                .iter()
                .map(|(category, _)| category.as_str())
                .collect::<Vec<_>>(),
        );
        ProviderStageAdmission {
            admit: df_finance::admit::AdmitCommand {
                operation: binding.operation,
                attempt: binding.attempt,
                effect: binding.effect,
                fence: self.fence,
                session: binding.session,
                route,
                work_class: WorkClass::Optional,
                maximum_units: binding.route.maximum_units.clone(),
                estimate_units: binding.route.maximum_units.clone(),
                quote: None,
                request_digest: binding.call.payload_digest,
                media: None,
            },
            binding,
        }
    }
}

impl RuntimeImageAuthority for HostedRuntimeAuthority {
    async fn funding(&self, input: &DispatchAdmission<'_>) -> Result<RuntimeFunding, RuntimeError> {
        if !self.owns(&input.job.owner) {
            return Err(RuntimeError::SourceDenied);
        }
        let finance = self.finance.lock().await;
        let exists = finance.client().query_opt("SELECT 1 FROM imagery.image_finance_admission_binding WHERE owner_scope=$1 AND job_id=$2 AND dispatch_id=$3", &[&owner_scope_bytes(&input.job.owner), &input.job.job.as_bytes().as_slice(), &input.dispatch.as_bytes().as_slice()]).await.map_err(|_| RuntimeError::StorageUnavailable)?.is_some();
        if exists {
            let df_persistence::imagery_runtime_admission::AdmittedFunding::Campaign(binding) =
                df_persistence::imagery_runtime_admission::read_admitted_funding(
                    &finance,
                    input.job,
                    input.dispatch,
                )
                .await?
            else {
                return Err(RuntimeError::InvalidRecord);
            };
            return Ok(RuntimeFunding::Campaign(Box::new(self.command(*binding))));
        }
        let cost = input.quote.ok_or(RuntimeError::InvalidRecord)?;
        let call =
            df_persistence::imagery_runtime_admission::runtime_stage_call(finance.client(), input)
                .await?;
        let row = finance.client().query_opt("SELECT price_version_id FROM finance.session_funding WHERE tenant_id=$1::bytea AND campaign_id=$2::bytea AND session_id=$3::bytea AND status='open'", &[&self.run.tenant.as_slice(), &self.run.campaign.as_slice(), &self.run.hosted_session.as_slice()]).await.map_err(|_| RuntimeError::StorageUnavailable)?.ok_or(RuntimeError::SourceDenied)?;
        let mut maximum_units: Vec<_> = cost
            .maxima
            .iter()
            .map(|unit| (unit.category.clone(), unit.quantity))
            .collect();
        maximum_units.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut route = df_commerce::draft_image::StageRoute {
            key: [0; 32],
            kind: call.kind,
            provider: cost.provider.clone(),
            model: cost.model.clone(),
            route: cost.detail.clone(),
            cause: cost.cause.clone(),
            detail: cost.detail.clone(),
            price_version: stored_id(&row, "price_version_id")?,
            maximum_units,
        };
        route.key = route.digest();
        route.validate().map_err(|_| RuntimeError::InvalidRecord)?;
        use df_finance::ports::IdSource;
        let operation = self.ids.next_id();
        let attempt = self.ids.next_id();
        let effect = self.ids.next_id();
        if [operation, attempt, effect].contains(&[0; 16]) {
            return Err(RuntimeError::StorageUnavailable);
        }
        let media_asset = if call.kind == df_commerce::draft_image::PaidStageKind::Classification
            || call.kind.is_standalone()
        {
            None
        } else {
            Some(
                ImageAssetId::new(
                    call.planned_publication
                        .ok_or(RuntimeError::InvalidRecord)?
                        .asset
                        .0,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
        };
        let binding = ProviderStageBinding {
            owner_scope: OwnerScope::new(owner_scope_bytes(&input.job.owner))
                .map_err(|_| RuntimeError::InvalidRecord)?,
            call,
            route_key: route.key,
            route,
            tenant: self.fence.tenant,
            campaign: self.fence.campaign,
            session: df_commerce::finance_ids::SessionKey::from_array(self.run.hosted_session),
            operation: df_commerce::finance_ids::OperationKey::from_array(operation),
            attempt: df_commerce::finance_ids::AttemptId::from_array(attempt),
            effect: df_commerce::finance_ids::EffectId::from_array(effect),
            media_asset,
        };
        Ok(RuntimeFunding::Campaign(Box::new(self.command(binding))))
    }

    async fn verify(
        &self,
        transaction: &Transaction<'_>,
        payer: df_persistence::imagery_draft_admission::StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        sources: &[SourceReceipt],
        _replay: bool,
    ) -> Result<(), df_finance::draft_image::Error> {
        use df_finance::draft_image::Error;
        let df_persistence::imagery_draft_admission::StagePayer::Campaign(binding) = payer else {
            return Err(Error::LinkConflict);
        };
        if !self.owns(owner)
            || binding.call != *call
            || binding.tenant != self.fence.tenant
            || binding.campaign != self.fence.campaign
            || binding.session.as_bytes() != &self.run.hosted_session
        {
            return Err(Error::LinkConflict);
        }
        if CanonicalFence
            .fence_current(transaction, &self.fence)
            .await
            .map_err(|_| Error::PortUnavailable)?
            != FenceVerdict::Current
        {
            return Err(Error::LinkConflict);
        }
        let principal = self
            .grants
            .authorize_catalog_sources(
                transaction,
                owner,
                &sources
                    .iter()
                    .map(|source| source.pin.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .map_err(|error| match error {
                RuntimeError::StorageUnavailable => Error::PortUnavailable,
                _ => Error::LinkConflict,
            })?;
        if principal.mode() != df_persistence::imagery_identity::AuthorityMode::Hosted {
            return Err(Error::LinkConflict);
        }
        let pins = sources
            .iter()
            .map(|source| source.pin.clone())
            .collect::<Vec<_>>();
        let ImageOwner::Run { campaign, .. } = owner else {
            return Err(Error::LinkConflict);
        };
        let facts = df_persistence::imagery_publication::lock_image_sources_in(
            transaction,
            &pins,
            &df_assets::Audience::CampaignShared(*campaign),
        )
        .await
        .map_err(|error| match error {
            df_imagery::publication::PublicationError::Unavailable => Error::PortUnavailable,
            _ => Error::LinkConflict,
        })?;
        if facts.len() != sources.len()
            || !facts.iter().zip(sources).all(|(fact, receipt)| {
                df_persistence::imagery_publication::matches_source_receipt(fact, receipt)
            })
        {
            return Err(Error::LinkConflict);
        }
        let price = transaction.query_opt("SELECT 1 FROM finance.session_funding WHERE tenant_id=$1::bytea AND campaign_id=$2::bytea AND session_id=$3::bytea AND price_version_id=$4::bytea AND status='open' FOR SHARE", &[&self.run.tenant.as_slice(), &self.run.campaign.as_slice(), &self.run.hosted_session.as_slice(), &binding.route.price_version.as_slice()]).await.map_err(|error| Error::Finance(error.into()))?;
        price.map(|_| ()).ok_or(Error::LinkConflict)
    }
}
