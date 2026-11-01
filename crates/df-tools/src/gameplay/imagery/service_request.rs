//! Domain intent conversion; execution, prices, queues and retries remain in df-imagery.

use df_imagery::{
    AnalysisRoute, Audience, CreatorClassification, Dimensions, EditSpec, ExecutionMode, Freshness,
    GenerateSpec, ImageContentRating, ImageInspection, ImageOperation, ImageOwner, ImagePipeline,
    InspectionKind, OperationIdentity, OutputProfile, PrivateImageText, Purpose, PurposedUse,
    SourceRef, StagedIdentity, TextPrompt, Urgency, VariantRole,
};
use df_types::OperationId;
use sha2::{Digest, Sha256};

use super::{ImageFailure, ImageKind, ImagePriority, ImageRequest};

/// Authority and freshness supplied from the current domain operation, never inferred from text.
pub(in crate::gameplay) struct RequestScope {
    pub owner: ImageOwner,
    pub operation: OperationId,
    pub source_revision: u64,
    pub audience: Audience,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub priority: ImagePriority,
}

/// Frozen configured routes; no credentials or endpoint selection by a consumer.
pub(in crate::gameplay) struct PipelinePolicy {
    pub rating: ImageContentRating,
    pub moderation: Option<df_imagery::ProviderModeration>,
    pub inspection: Option<AnalysisRoute>,
    pub creator: Option<CreatorClassification>,
}

/// Pins must describe the exact ordered bytes admitted by the reference catalog bridge.
/// The service computes and sets the canonical fingerprint after this conversion.
pub(in crate::gameplay) fn prepare(
    request: &ImageRequest,
    scope: RequestScope,
    pins: Vec<SourceRef>,
    output: OutputProfile,
    policy: PipelinePolicy,
) -> Result<df_imagery::ImageRequest, ImageFailure> {
    if pins.len() != request.references.len()
        || pins
            .iter()
            .zip(&request.references)
            .any(|(pin, reference)| {
                *pin.digest.as_bytes() != <[u8; 32]>::from(Sha256::digest(&reference.bytes))
            })
    {
        return Err(ImageFailure::InvalidRequest);
    }
    let delivered = output.delivered();
    let dimensions = Dimensions::new(request.output.width, request.output.height)
        .map_err(|_| ImageFailure::InvalidRequest)?;
    if delivered.dimensions() != dimensions
        || (request.output.transparent
            && delivered.alpha() != df_imagery::AlphaHandling::PreserveAlpha)
    {
        return Err(ImageFailure::InvalidRequest);
    }
    let is_creator = policy.creator.is_some();
    let inspection = inspection(request, policy.inspection)?;
    let template_revision = if request.prompt.artwork().is_some() {
        match request.kind {
            ImageKind::CanonicalCharacter => 0xDF02_0001,
            ImageKind::ArtifactCloseUp => 0xDF02_0005,
            _ => return Err(ImageFailure::InvalidRequest),
        }
    } else {
        request.kind.template_revision()
    };
    let mut prompt = TextPrompt::new(request.prompt.as_str(), template_revision)
        .and_then(|prompt| {
            prompt.with_pipeline(ImagePipeline {
                revision: 1,
                moderation: policy.moderation,
                rating: policy.rating,
                creator: policy.creator,
                inspection,
            })
        })
        .map_err(|_| ImageFailure::InvalidRequest)?;
    if let Some(artwork) = request.prompt.artwork() {
        prompt = prompt.with_artwork(artwork.clone());
    }
    let purpose = match request.kind {
        ImageKind::CanonicalCharacter if is_creator => Purpose::CreatorPortrait,
        ImageKind::CanonicalCharacter => Purpose::CanonicalPerson,
        ImageKind::CanonicalRoom => Purpose::CanonicalRoom,
        ImageKind::SceneRefresh | ImageKind::ScenePlate => Purpose::ScenePlate,
        ImageKind::ScenePlateEdit => Purpose::SceneEdit,
        ImageKind::CastExpression => Purpose::Expression,
        ImageKind::ArtifactCloseUp => Purpose::ItemCloseUp,
    };
    let operation = if matches!(
        request.kind,
        ImageKind::ScenePlateEdit | ImageKind::SceneRefresh
    ) {
        let mut pins = pins.into_iter();
        ImageOperation::Edit(EditSpec {
            prompt,
            base: pins.next().ok_or(ImageFailure::InvalidRequest)?,
            references: pins.collect(),
        })
    } else {
        ImageOperation::Generate(GenerateSpec {
            prompt,
            references: pins,
        })
    };
    Ok(df_imagery::ImageRequest {
        identity: OperationIdentity {
            owner: scope.owner,
            operation: scope.operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose,
            role: VariantRole::Primary,
        },
        operation,
        execution: scope.execution,
        freshness: scope.freshness,
        urgency: match scope.priority {
            ImagePriority::Foreground => Urgency::Visible,
            ImagePriority::Presence => Urgency::Present,
            ImagePriority::Background => Urgency::Speculative,
        },
        audience: scope.audience,
        expected_source_revision: scope.source_revision,
        output,
    })
}

fn inspection(
    request: &ImageRequest,
    route: Option<AnalysisRoute>,
) -> Result<Option<ImageInspection>, ImageFailure> {
    let kind = match request.kind {
        ImageKind::ScenePlate | ImageKind::ScenePlateEdit => InspectionKind::ScenePlate,
        ImageKind::CanonicalRoom => InspectionKind::CanonicalRoom,
        _ => return Ok(None),
    };
    let Some(route) = route else {
        return Ok(None);
    };
    let party = request.prompt.scene_party();
    let party_reference = party
        .map(|party| u32::try_from(party.sheet_image()).map_err(|_| ImageFailure::InvalidRequest))
        .transpose()?;
    if party_reference.is_some_and(|index| index as usize > request.references.len()) {
        return Err(ImageFailure::InvalidRequest);
    }
    Ok(Some(ImageInspection {
        route,
        requirement: df_imagery::InspectionRequirement::BestEffort,
        contract_revision: 1,
        kind,
        expected_heroes: party.map(|party| party.heroes()),
        party_reference,
        hand_custody: PrivateImageText::new(request.prompt.scene_hand_custody()),
        staging: request
            .prompt
            .staging_order()
            .iter()
            .map(|person| StagedIdentity {
                locator_id: PrivateImageText::new(&person.locator_id),
                reference_id: PrivateImageText::new(&person.reference_id),
                public_name: PrivateImageText::new(&person.name),
                hero: person.hero,
            })
            .collect(),
        maximum_corrections: 1,
        // Existing post-correction gate accepts visible text alone and drops visual evidence.
        allow_visible_text: true,
    }))
}
