//! Resolver-created imagery principals, exact audience checks and immutable source pins.
//! Canonical authority remains in the account, campaign and gameplay owners.

use df_assets::Audience;
use df_imagery::{
    AuthDenied, ImageAuthorizer as ImageAuthorizerPort, ImageOperation, ImageOwner, ImageRequest,
    ImageTransform, MaskSource, ServiceAction, SourceRef, StoredImageAuthorization,
    TransformSource,
};
pub use df_persistence::imagery_identity::{AuthorityMode, LocalRunGrant};

/// Current authenticated hosted account/session and gameplay credential proof.
#[derive(Clone, Eq, PartialEq)]
pub struct LiveGrant {
    pub tenant: [u8; 16],
    pub campaign: [u8; 16],
    pub hosted_session: [u8; 16],
    pub member: [u8; 16],
    pub generation: [u8; 16],
    pub web_session: [u8; 16],
    pub gameplay_credential: [u8; 32],
}

/// Resolution failure without owner bytes or credential details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolveError {
    Refused,
    Unavailable,
}

/// Authority snapshots are resolved afresh for every service operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageryPrincipal {
    Hosted(HostedPrincipal),
    LocalGuest(LocalGuestPrincipal),
    LocalRun(LocalRunPrincipal),
}

/// A hosted principal can only be constructed from current canonical authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostedPrincipal {
    account: [u8; 16],
    campaign: [u8; 16],
    run: [u8; 16],
    member: [u8; 16],
    generation: [u8; 16],
    host_authority: bool,
}

/// A local member account and retained bootstrap generation, without hosted authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalGuestPrincipal {
    account: [u8; 16],
    generation: [u8; 16],
}

/// Server-owned local run authority; its current fence and bootstrap proof are checked at resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRunPrincipal {
    authority: df_persistence::imagery_identity::LocalRunAuthority,
    owner: ImageOwner,
}

impl ImageryPrincipal {
    pub fn mode(&self) -> AuthorityMode {
        match self {
            Self::Hosted(_) => AuthorityMode::Hosted,
            Self::LocalGuest(_) => AuthorityMode::LocalGuest,
            Self::LocalRun(_) => AuthorityMode::LocalRun,
        }
    }

    pub fn account(&self) -> Option<&[u8; 16]> {
        match self {
            Self::Hosted(principal) => Some(&principal.account),
            Self::LocalGuest(principal) => Some(&principal.account),
            Self::LocalRun(_) => None,
        }
    }

    pub fn local_run_owner(&self) -> Option<&ImageOwner> {
        match self {
            Self::LocalRun(principal) => Some(&principal.owner),
            Self::Hosted(_) | Self::LocalGuest(_) => None,
        }
    }

    pub fn generation(&self) -> &[u8; 16] {
        match self {
            Self::Hosted(principal) => &principal.generation,
            Self::LocalGuest(principal) => &principal.generation,
            Self::LocalRun(principal) => &principal.authority.fence,
        }
    }

    pub fn campaign_run(&self) -> Option<([u8; 16], [u8; 16])> {
        match self {
            Self::Hosted(principal) => Some((principal.campaign, principal.run)),
            Self::LocalGuest(_) => None,
            Self::LocalRun(principal) => {
                Some((principal.authority.campaign, principal.authority.run))
            }
        }
    }

    pub fn member(&self) -> Option<[u8; 16]> {
        match self {
            Self::Hosted(principal) => Some(principal.member),
            Self::LocalGuest(_) | Self::LocalRun(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn hosted_for_tests(
        account: [u8; 16],
        campaign: [u8; 16],
        run: [u8; 16],
        member: [u8; 16],
        generation: [u8; 16],
    ) -> Self {
        Self::Hosted(HostedPrincipal {
            account,
            campaign,
            run,
            member,
            generation,
            host_authority: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn local_run_for_tests(campaign: [u8; 16], run: [u8; 16], fence: [u8; 16]) -> Self {
        let authority = df_persistence::imagery_identity::LocalRunAuthority {
            tenant: df_persistence::local_demo_scope::TENANT,
            session: [0x41; 16],
            campaign,
            run,
            fence,
        };
        Self::LocalRun(LocalRunPrincipal {
            owner: ImageOwner::Run {
                campaign: df_types::CampaignId::from_bytes(&campaign).unwrap(),
                run: df_types::RunId::from_bytes(&run).unwrap(),
            },
            authority,
        })
    }

    #[cfg(test)]
    pub(crate) fn local_for_tests(account: [u8; 16], generation: [u8; 16]) -> Self {
        Self::LocalGuest(LocalGuestPrincipal {
            account,
            generation,
        })
    }
}

fn resolve_error(error: df_persistence::imagery_identity::IdentityError) -> ResolveError {
    use df_persistence::imagery_identity::IdentityError;
    match error {
        IdentityError::Refused | IdentityError::Conflict => ResolveError::Refused,
        IdentityError::Unavailable { .. } | IdentityError::Unknown { .. } => {
            ResolveError::Unavailable
        }
    }
}

/// Resolves the stored mapping and locks current canonical and gameplay authority through commit.
pub async fn resolve_hosted_principal(
    transaction: &tokio_postgres::Transaction<'_>,
    grant: &LiveGrant,
) -> Result<ImageryPrincipal, ResolveError> {
    use df_persistence::imagery_identity::{
        read_host_run, read_host_seat_by_member, verify_host_principal,
    };
    let seat = read_host_seat_by_member(
        transaction,
        &grant.tenant,
        &grant.campaign,
        &grant.hosted_session,
        &grant.member,
    )
    .await
    .map_err(resolve_error)?
    .ok_or(ResolveError::Refused)?;
    if !seat.active || seat.generation != grant.generation {
        return Err(ResolveError::Refused);
    }
    let run = read_host_run(
        transaction,
        &grant.tenant,
        &grant.campaign,
        &grant.hosted_session,
    )
    .await
    .map_err(resolve_error)?
    .ok_or(ResolveError::Refused)?;
    if run.run != seat.run || run.gameplay_session != seat.gameplay_session {
        return Err(ResolveError::Refused);
    }
    let host_authority = verify_host_principal(
        transaction,
        &run,
        &seat,
        &grant.web_session,
        &grant.gameplay_credential,
    )
    .await
    .map_err(resolve_error)?;
    Ok(ImageryPrincipal::Hosted(HostedPrincipal {
        account: seat.account,
        campaign: seat.campaign,
        run: seat.run,
        member: seat.member,
        generation: seat.generation,
        host_authority,
    }))
}

/// Resolves a member-scoped local account only after checking its current credential.
pub async fn resolve_local_principal(
    transaction: &tokio_postgres::Transaction<'_>,
    member: &[u8; 16],
    session: &[u8; 16],
    credential: &[u8; 32],
) -> Result<ImageryPrincipal, ResolveError> {
    use df_persistence::imagery_identity::{read_local_guest, verify_local_credential};
    verify_local_credential(transaction, member, session, credential)
        .await
        .map_err(resolve_error)?;
    let stored = read_local_guest(transaction, member, session)
        .await
        .map_err(resolve_error)?
        .ok_or(ResolveError::Refused)?;
    Ok(ImageryPrincipal::LocalGuest(LocalGuestPrincipal {
        account: stored.account,
        generation: stored.generation,
    }))
}

/// Resolves only the local server owner from its captured startup fence and private bootstrap grant.
pub async fn resolve_local_run_principal(
    transaction: &tokio_postgres::Transaction<'_>,
    grant: &LocalRunGrant,
) -> Result<ImageryPrincipal, ResolveError> {
    let authority = df_persistence::imagery_identity::resolve_local_run(transaction, grant)
        .await
        .map_err(resolve_error)?;
    let owner = ImageOwner::Run {
        campaign: df_types::CampaignId::from_bytes(&authority.campaign)
            .map_err(|_| ResolveError::Refused)?,
        run: df_types::RunId::from_bytes(&authority.run).map_err(|_| ResolveError::Refused)?,
    };
    Ok(ImageryPrincipal::LocalRun(LocalRunPrincipal {
        authority,
        owner,
    }))
}

/// Complete reference facts supplied by asset authority for this authenticated context.
pub trait AssetReferenceFacts: Send + Sync {
    fn reference_authorized(&self, source: &SourceRef) -> bool;
    /// Host facts require a current authorized analytical read matching the exact base image.
    fn analysis_authorized(
        &self,
        _handle: &df_imagery::OperationHandle,
        _analyzed: &SourceRef,
    ) -> bool {
        false
    }
}

/// Exact immutable source pins authorized by the asset owner for this operation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExactAssetAllowlist {
    assets: Vec<SourceRef>,
}

impl ExactAssetAllowlist {
    pub fn new(assets: Vec<SourceRef>) -> Self {
        Self { assets }
    }
}

impl AssetReferenceFacts for ExactAssetAllowlist {
    fn reference_authorized(&self, source: &SourceRef) -> bool {
        self.assets.contains(source)
    }
}

/// Synchronous policy over principals created by the current-authority resolvers.
pub struct ImageryAuthorizer<R> {
    facts: R,
}

impl<R> ImageryAuthorizer<R> {
    pub fn new(facts: R) -> Self {
        Self { facts }
    }
}

impl<R: AssetReferenceFacts> ImageryAuthorizer<R> {
    /// Scope and audience policy for analytical work without a fabricated raster request.
    pub fn authorize_audience(
        &self,
        principal: &ImageryPrincipal,
        owner: &ImageOwner,
        audience: &Audience,
    ) -> Result<(), AuthDenied> {
        if self.owner_in_scope(principal, owner) && self.audience_in_scope(principal, audience) {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    /// Policy for work-owned source access after exact catalog winner, digest and audience validation.
    /// This checks no bytes and never substitutes for the service's exact source-reference facts.
    pub fn authorize_catalog_source(
        &self,
        principal: &ImageryPrincipal,
        work_owner: &ImageOwner,
        source: &SourceRef,
    ) -> Result<(), AuthDenied> {
        if self.owner_in_scope(principal, work_owner)
            && self.audience_in_scope(principal, &source.audience)
        {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn owner_in_scope(&self, principal: &ImageryPrincipal, owner: &ImageOwner) -> bool {
        match (principal, owner) {
            (ImageryPrincipal::Hosted(hosted), ImageOwner::Run { campaign, run }) => {
                hosted.campaign == *campaign.as_bytes() && hosted.run == *run.as_bytes()
            }
            (ImageryPrincipal::Hosted(hosted), ImageOwner::Draft { account }) => {
                hosted.account == *account.as_bytes()
            }
            (ImageryPrincipal::LocalGuest(guest), ImageOwner::Draft { account }) => {
                guest.account == *account.as_bytes()
            }
            (ImageryPrincipal::LocalGuest(_), ImageOwner::Run { .. }) => false,
            (ImageryPrincipal::LocalRun(local), owner) => local.owner == *owner,
        }
    }

    fn audience_in_scope(&self, principal: &ImageryPrincipal, audience: &Audience) -> bool {
        match principal {
            ImageryPrincipal::Hosted(hosted) => match audience {
                Audience::Withheld => false,
                Audience::CampaignShared(campaign) => hosted.campaign == *campaign.as_bytes(),
                Audience::Host(campaign) => {
                    hosted.host_authority && hosted.campaign == *campaign.as_bytes()
                }
                Audience::Members(campaign, members) => {
                    let member = df_types::MemberId::from_bytes(&hosted.member);
                    hosted.campaign == *campaign.as_bytes()
                        && member.is_ok_and(|member| members.members().contains(&member))
                }
                Audience::PublicStatic => true,
                Audience::OwnerPrivate(account) => hosted.account == *account.as_bytes(),
            },
            ImageryPrincipal::LocalRun(local) => match audience {
                Audience::PublicStatic => true,
                Audience::CampaignShared(campaign) | Audience::Host(campaign) => {
                    local.authority.campaign == *campaign.as_bytes()
                }
                Audience::OwnerPrivate(_) | Audience::Members(..) | Audience::Withheld => false,
            },
            ImageryPrincipal::LocalGuest(guest) => match audience {
                Audience::PublicStatic => true,
                Audience::OwnerPrivate(account) => guest.account == *account.as_bytes(),
                _ => false,
            },
        }
    }

    fn pins_authorized(
        &self,
        principal: &ImageryPrincipal,
        owner: &ImageOwner,
        operation: &ImageOperation,
    ) -> bool {
        if let ImageOperation::Transform(df_imagery::TransformSpec {
            source:
                TransformSource::SegmentationMask {
                    segmentation,
                    analyzed,
                    ..
                },
            ..
        }) = operation
        {
            if segmentation.owner != *owner
                || !self.owner_in_scope(principal, &segmentation.owner)
                || !self.facts.analysis_authorized(segmentation, analyzed)
            {
                return false;
            }
        }
        pinned_assets(operation).iter().all(|source| {
            self.audience_in_scope(principal, &source.audience)
                && self.facts.reference_authorized(source)
        })
    }
}

impl<R: AssetReferenceFacts> ImageAuthorizerPort for ImageryAuthorizer<R> {
    type Principal = ImageryPrincipal;

    fn authorize_request(
        &self,
        principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        if !self.owner_in_scope(principal, &request.identity.owner)
            || !self.audience_in_scope(principal, &request.audience)
            || !self.pins_authorized(principal, &request.identity.owner, &request.operation)
        {
            return Err(AuthDenied);
        }
        Ok(())
    }

    fn authorize_owner(
        &self,
        principal: &Self::Principal,
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if self.owner_in_scope(principal, owner) {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_stored(
        &self,
        principal: &Self::Principal,
        _action: ServiceAction,
        _view: &df_imagery::OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if stored.handle.owner != *stored.owner
            || !self.owner_in_scope(principal, stored.owner)
            || !self.audience_in_scope(principal, stored.audience)
            || !self.pins_authorized(principal, stored.owner, stored.operation)
        {
            return Err(AuthDenied);
        }
        Ok(())
    }
}

fn pinned_assets(operation: &ImageOperation) -> Vec<SourceRef> {
    let mut assets = Vec::new();
    match operation {
        ImageOperation::Generate(spec) => assets.extend_from_slice(&spec.references),
        ImageOperation::Edit(spec) => {
            assets.push(spec.base.clone());
            assets.extend_from_slice(&spec.references);
        }
        ImageOperation::Transform(spec) => transform_pins(spec, &mut assets),
    }
    assets
}

fn transform_pins(spec: &df_imagery::TransformSpec, assets: &mut Vec<SourceRef>) {
    source_pins(&spec.source, assets);
    for step in &spec.steps {
        match step {
            ImageTransform::Mask(mask) => match mask.source() {
                MaskSource::Asset(reference) => assets.push(reference.clone()),
                MaskSource::Coverage(coverage) => {
                    assets.push(coverage.coverage.clone());
                    assets.push(coverage.analyzed.clone());
                }
            },
            ImageTransform::Composite(layers) => {
                assets.extend(layers.iter().map(|layer| layer.source().clone()))
            }
            _ => {}
        }
    }
}

fn source_pins(source: &TransformSource, assets: &mut Vec<SourceRef>) {
    match source {
        TransformSource::Image(reference) => assets.push(reference.clone()),
        TransformSource::SegmentationMask { analyzed, .. } => assets.push(analyzed.clone()),
        TransformSource::Canvas(_) => {}
        TransformSource::Coverage(coverage) => {
            assets.push(coverage.coverage.clone());
            assets.push(coverage.analyzed.clone());
        }
    }
}
