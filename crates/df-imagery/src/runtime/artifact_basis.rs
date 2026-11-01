//! Pure artifact execution basis reuses the runtime's actual route selectors; it performs no I/O.
use super::{RuntimeError, dispatch};
use crate::VersionedSnapshot;
use crate::config::ImageConfig;
use crate::intent_codec::DecodedIntent;
use crate::service::artifact_recipe::ArtifactExecutorBasis;

pub(crate) fn artifact_executor_basis(
    intent: &DecodedIntent,
    config: &ImageConfig,
) -> Result<ArtifactExecutorBasis, RuntimeError> {
    let backend = dispatch::backend(config, intent);
    let model = dispatch::model(config, intent)?;
    let account = dispatch::account(config, intent);
    let mut encoded = b"df-imagery/artifact-executor-v1".to_vec();
    for field in [
        backend.label().as_bytes(),
        model.as_bytes(),
        account.as_str().as_bytes(),
        crate::raster::webp_lossy::WEBP_ENCODER_SOURCE.as_bytes(),
    ] {
        let len = u32::try_from(field.len()).map_err(|_| RuntimeError::InvalidRecord)?;
        encoded.extend_from_slice(&len.to_be_bytes());
        encoded.extend_from_slice(field);
    }
    encoded.push(match dispatch::moderation(config, intent) {
        crate::backends::api::ImageModeration::Auto => 0,
        crate::backends::api::ImageModeration::Low => 1,
    });
    for revision in [
        crate::backends::codex_cli::ADAPTER_REVISION,
        crate::raster::color::RASTER_COLOR_REV,
        crate::raster::webp_lossy::WEBP_ENCODER_REVISION,
    ] {
        encoded.extend_from_slice(&revision.to_be_bytes());
    }
    let encoded = VersionedSnapshot::new(1, encoded, crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(ArtifactExecutorBasis {
        backend,
        model,
        encoded,
    })
}
