//! A pure campaign admission recipe keeps public artwork semantics separate from run authority.
//! Keys aid the existing catalog election; source grants and actual Ready proof remain mandatory.

use std::fmt::Write;

use df_assets::{MediaRecipeRequest, MediaRequestKey};

use crate::config::{BackendId, ImageConfig};
use crate::intent_codec::{decode_config, decode_intent, encode_artifact_semantics, encode_intent};
use crate::{
    Freshness, ImageContentRating, ImageOperation, ImageRequest, JournalError, VersionedSnapshot,
};

use super::{PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES, snapshot_version_matches};

/// Complete admission semantics are retained and compared, not replaced by their catalog key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignArtifactRecipe {
    pub key: MediaRequestKey,
    pub semantic: VersionedSnapshot,
}

pub(crate) struct ArtifactExecutorBasis {
    pub backend: BackendId,
    pub model: String,
    pub encoded: VersionedSnapshot,
}

/// Builds only eligible public campaign artwork identity; this performs no I/O or authorization.
pub fn campaign_artifact_recipe(
    request: &ImageRequest,
    config: &VersionedSnapshot,
) -> Result<Option<CampaignArtifactRecipe>, JournalError> {
    let encoded = encode_intent(request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| JournalError::InvalidRecord)?;
    let intent = VersionedSnapshot::new(
        u32::from(
            crate::intent_codec::intent_version(encoded.bytes())
                .map_err(|_| JournalError::InvalidRecord)?,
        ),
        encoded.bytes().to_vec(),
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    retained_campaign_artifact_recipe(&intent, config)
}

/// Uses the same complete recipe for a retained job without fabricating an operation or run.
pub fn retained_campaign_artifact_recipe(
    intent: &VersionedSnapshot,
    config: &VersionedSnapshot,
) -> Result<Option<CampaignArtifactRecipe>, JournalError> {
    if !snapshot_version_matches(intent) || !snapshot_version_matches(config) {
        return Err(JournalError::InvalidRecord);
    }
    let intent = decode_intent(
        intent.bytes(),
        &PROTOCOL_LIMITS,
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    let Some(artwork) = intent.operation.artwork() else {
        return Ok(None);
    };
    artwork
        .validate_for(&intent.owner, intent.purpose, &intent.audience)
        .map_err(|_| JournalError::InvalidRecord)?;
    if intent.freshness != Freshness::ReuseCompatible
        || intent
            .operation
            .pipeline()
            .is_some_and(|pipeline| pipeline.creator.is_some())
    {
        return Ok(None);
    }
    let config = super::cache::normalized_config(config)?;
    let decoded_config: ImageConfig = decode_config(config.bytes(), PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| JournalError::InvalidRecord)?;
    let basis = crate::runtime::artifact_executor_basis(&intent, &decoded_config)
        .map_err(|_| JournalError::InvalidRecord)?;
    let semantic = encode_artifact_semantics(
        &intent,
        &config,
        &basis.encoded,
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    let mut scope = String::from("campaign-artwork-v1/");
    for byte in artwork.spec.campaign.as_bytes() {
        write!(&mut scope, "{byte:02x}").map_err(|_| JournalError::InvalidRecord)?;
    }
    let (operation_kind, template_revision) = match &intent.operation {
        ImageOperation::Generate(spec) => ("generate", spec.prompt.template_version),
        ImageOperation::Edit(spec) => ("edit", spec.prompt.template_version),
        ImageOperation::Transform(_) => return Ok(None),
    };
    let delivered = intent.output.delivered();
    let dimensions = delivered.dimensions();
    let rating = match intent.operation.pipeline().map(|pipeline| pipeline.rating) {
        Some(ImageContentRating::Restrained) => "restrained",
        Some(ImageContentRating::Mature) => "mature",
        None => "unspecified",
    };
    let key = MediaRequestKey::derive_recipe(&MediaRecipeRequest {
        operation_kind,
        purpose: intent.purpose.purpose.label(),
        owner_scope: &scope,
        backend_profile: basis.backend.label(),
        model: &basis.model,
        quality: "frozen-config",
        rating,
        width_px: dimensions.width_px(),
        height_px: dimensions.height_px(),
        output_format: delivered.encoding().format().label(),
        alpha_policy: "frozen-output",
        template_revision,
        route_revision: 1,
        acceptance_revision: artwork.spec.version(),
        executor_revision: basis.encoded.revision(),
        source_binding: None,
        references: &[],
        prompt: "",
        pipeline_bytes: semantic.bytes(),
        regeneration_nonce: None,
        audience: &intent.audience,
    })
    .map_err(|_| JournalError::InvalidRecord)?;
    Ok(Some(CampaignArtifactRecipe { key, semantic }))
}

/// Projects canonical metadata onto native-authorized facts; this pure function grants no access.
pub fn project_retained_campaign_artifact(
    intent: &VersionedSnapshot,
    config: &VersionedSnapshot,
    mut artifact: crate::publication::ReadyArtifact,
) -> Result<crate::publication::ReadyArtifact, JournalError> {
    let recipe =
        retained_campaign_artifact_recipe(intent, config)?.ok_or(JournalError::InvalidRecord)?;
    let decoded = decode_intent(
        intent.bytes(),
        &PROTOCOL_LIMITS,
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    artifact.staging = decoded
        .operation
        .pipeline()
        .and_then(|pipeline| pipeline.inspection.as_ref())
        .map_or_else(Vec::new, |inspection| inspection.staging.clone());
    super::artifacts::verify_artifact_parts(
        &decoded.operation,
        &decoded.audience,
        decoded.output,
        &recipe,
        &artifact,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    Ok(artifact)
}
#[cfg(test)]
#[path = "artifact_recipe_tests.rs"]
mod tests;
