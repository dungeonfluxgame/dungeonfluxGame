//! Versioned semantic stage metadata; legacy version-1 bytes remain unchanged.

use super::{
    CodecError,
    primitives::{Reader, Writer, prompt_byte_bound},
};
use crate::config::{AccountHandle, ApiEndpoint, ModelName};
use crate::request::{
    AnalysisRoute, CreatorClassification, ImageContentRating, ImageInspection, ImagePipeline,
    InspectionKind, InspectionRequirement, PrivateImageText, ProviderModeration, RequestLimits,
    StagedIdentity,
};

fn text(out: &mut Writer, value: &PrivateImageText) -> Result<(), CodecError> {
    out.counted(value.text().as_bytes())
}

pub(super) fn route(out: &mut Writer, value: &AnalysisRoute) -> Result<(), CodecError> {
    out.counted(value.account.as_str().as_bytes())?;
    out.counted(value.model.as_str().as_bytes())?;
    out.counted(value.endpoint.as_str().as_bytes())?;
    out.u64_be(value.maximum_output_tokens)
}

pub(super) fn encode(out: &mut Writer, pipeline: &ImagePipeline) -> Result<(), CodecError> {
    out.u32_be(pipeline.revision)?;
    out.push(match pipeline.rating {
        ImageContentRating::Restrained => 0,
        ImageContentRating::Mature => 1,
    })?;
    out.boolean(pipeline.creator.is_some())?;
    if let Some(creator) = &pipeline.creator {
        route(out, &creator.route)?;
        out.u32_be(creator.contract_revision)?;
        text(out, &creator.public_appearance)?;
    }
    out.boolean(pipeline.inspection.is_some())?;
    if let Some(inspection) = &pipeline.inspection {
        encode_inspection(out, inspection)?;
    }
    out.boolean(pipeline.moderation.is_some())?;
    if let Some(moderation) = pipeline.moderation {
        out.push(match moderation {
            ProviderModeration::Auto => 0,
            ProviderModeration::Low => 1,
        })?;
    }
    let general = pipeline
        .creator
        .as_ref()
        .and_then(|creator| creator.general_prompt.as_ref());
    out.boolean(general.is_some())?;
    if let Some(general) = general {
        text(out, general)?;
    }
    Ok(())
}

pub(crate) fn encode_inspection(
    out: &mut Writer,
    inspection: &ImageInspection,
) -> Result<(), CodecError> {
    route(out, &inspection.route)?;
    out.u32_be(inspection.contract_revision)?;
    out.push(match inspection.kind {
        InspectionKind::ScenePlate => 0,
        InspectionKind::CanonicalRoom => 1,
        InspectionKind::CanonicalCharacter => 2,
    })?;
    match (inspection.expected_heroes, inspection.party_reference) {
        (Some(heroes), Some(reference)) => {
            out.push(1)?;
            out.push(heroes)?;
            out.u32_be(reference)?;
        }
        (None, None) => out.push(0)?,
        _ => return Err(CodecError::Malformed),
    }
    text(out, &inspection.hand_custody)?;
    out.u32_be(u32::try_from(inspection.staging.len()).map_err(|_| CodecError::TooLong)?)?;
    for identity in &inspection.staging {
        text(out, &identity.locator_id)?;
        text(out, &identity.reference_id)?;
        text(out, &identity.public_name)?;
        out.boolean(identity.hero)?;
    }
    out.u32_be(inspection.maximum_corrections)?;
    out.boolean(inspection.allow_visible_text)?;
    out.push(match inspection.requirement {
        InspectionRequirement::BestEffort => 0,
        InspectionRequirement::Required => 1,
    })
}

fn read_text<'a>(input: &mut Reader<'a>, maximum_bytes: usize) -> Result<&'a str, CodecError> {
    core::str::from_utf8(input.counted(maximum_bytes)?).map_err(|_| CodecError::InvalidText)
}

fn read_private(
    input: &mut Reader<'_>,
    maximum_bytes: usize,
) -> Result<PrivateImageText, CodecError> {
    Ok(PrivateImageText::new(read_text(input, maximum_bytes)?))
}

pub(super) fn read_route(input: &mut Reader<'_>) -> Result<AnalysisRoute, CodecError> {
    Ok(AnalysisRoute {
        account: AccountHandle::new(read_text(input, 64)?).map_err(CodecError::Config)?,
        model: ModelName::new(read_text(input, 128)?).map_err(CodecError::Config)?,
        endpoint: ApiEndpoint::new(read_text(input, 256)?).map_err(CodecError::Config)?,
        maximum_output_tokens: input.u64_be()?,
    })
}

pub(super) fn decode(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
    version: u8,
) -> Result<ImagePipeline, CodecError> {
    let revision = input.u32_be()?;
    let rating = match input.byte()? {
        0 => ImageContentRating::Restrained,
        1 => ImageContentRating::Mature,
        _ => return Err(CodecError::Malformed),
    };
    let mut creator = if input.boolean()? {
        Some(CreatorClassification {
            route: read_route(input)?,
            contract_revision: input.u32_be()?,
            public_appearance: read_private(input, (8 * 1024).min(prompt_byte_bound(limits)))?,
            general_prompt: None,
        })
    } else {
        None
    };
    let inspection = if input.boolean()? {
        Some(read_inspection(input, limits, version)?)
    } else {
        None
    };
    let moderation = if version >= super::MODERATION_INTENT_CODEC_VERSION && input.boolean()? {
        Some(match input.byte()? {
            0 => ProviderModeration::Auto,
            1 => ProviderModeration::Low,
            _ => return Err(CodecError::Malformed),
        })
    } else {
        None
    };
    if version >= super::PIPELINE_INTENT_CODEC_VERSION && input.boolean()? {
        let general = read_private(input, prompt_byte_bound(limits))?;
        creator
            .as_mut()
            .ok_or(CodecError::Malformed)?
            .general_prompt = Some(general);
    }
    let pipeline = ImagePipeline {
        revision,
        moderation,
        rating,
        creator,
        inspection,
    };
    pipeline.validate(limits).map_err(CodecError::Request)?;
    Ok(pipeline)
}

pub(crate) fn read_inspection(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
    version: u8,
) -> Result<ImageInspection, CodecError> {
    let route = read_route(input)?;
    let contract_revision = input.u32_be()?;
    let kind = match input.byte()? {
        0 => InspectionKind::ScenePlate,
        1 => InspectionKind::CanonicalRoom,
        2 => InspectionKind::CanonicalCharacter,
        _ => return Err(CodecError::Malformed),
    };
    let (expected_heroes, party_reference) = match input.byte()? {
        0 => (None, None),
        1 => (Some(input.byte()?), Some(input.u32_be()?)),
        _ => return Err(CodecError::Malformed),
    };
    let hand_custody = read_private(input, prompt_byte_bound(limits))?;
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    // Three counted texts plus a boolean must fit before allocating the vector.
    if count > 24
        || count
            .checked_mul(13)
            .is_none_or(|minimum| minimum > input.remaining())
    {
        return Err(CodecError::TooLong);
    }
    let mut staging = Vec::with_capacity(count);
    for _ in 0..count {
        staging.push(StagedIdentity {
            locator_id: read_private(input, prompt_byte_bound(limits))?,
            reference_id: read_private(input, prompt_byte_bound(limits))?,
            public_name: read_private(input, prompt_byte_bound(limits))?,
            hero: input.boolean()?,
        });
    }
    Ok(ImageInspection {
        route,
        contract_revision,
        kind,
        expected_heroes,
        party_reference,
        hand_custody,
        staging,
        maximum_corrections: input.u32_be()?,
        allow_visible_text: input.boolean()?,
        requirement: if version >= super::INSPECTION_REQUIREMENT_CODEC_VERSION {
            match input.byte()? {
                0 => InspectionRequirement::BestEffort,
                1 => InspectionRequirement::Required,
                _ => return Err(CodecError::Malformed),
            }
        } else {
            InspectionRequirement::BestEffort
        },
    })
}
