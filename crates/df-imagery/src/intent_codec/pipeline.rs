//! Versioned semantic stage metadata; legacy version-1 bytes remain unchanged.

use super::{
    CodecError,
    primitives::{
        MIN_STAGED_IDENTITY_BYTES, Reader, Writer, decode_moderation, decode_route,
        encode_moderation, encode_route, prompt_byte_bound,
    },
};
use crate::request::{
    CreatorClassification, ImageContentRating, ImageInspection, ImagePipeline, InspectionKind,
    InspectionRequirement, MAXIMUM_PUBLIC_APPEARANCE_BYTES, MAXIMUM_STAGED_IDENTITIES,
    PrivateImageText, RequestLimits, StagedIdentity,
};

fn encode_private_text(out: &mut Writer, value: &PrivateImageText) -> Result<(), CodecError> {
    out.counted(value.text().as_bytes())
}

pub(super) fn encode(out: &mut Writer, pipeline: &ImagePipeline) -> Result<(), CodecError> {
    out.u32_be(pipeline.revision)?;
    out.push(match pipeline.rating {
        ImageContentRating::Restrained => 0,
        ImageContentRating::Mature => 1,
    })?;
    out.boolean(pipeline.creator.is_some())?;
    if let Some(creator) = &pipeline.creator {
        encode_route(out, &creator.route)?;
        out.u32_be(creator.contract_revision)?;
        encode_private_text(out, &creator.public_appearance)?;
    }
    out.boolean(pipeline.inspection.is_some())?;
    if let Some(inspection) = &pipeline.inspection {
        encode_inspection(out, inspection)?;
    }
    out.boolean(pipeline.moderation.is_some())?;
    if let Some(moderation) = pipeline.moderation {
        encode_moderation(out, moderation)?;
    }
    let general = pipeline
        .creator
        .as_ref()
        .and_then(|creator| creator.general_prompt.as_ref());
    out.boolean(general.is_some())?;
    if let Some(general) = general {
        encode_private_text(out, general)?;
    }
    Ok(())
}

pub(crate) fn encode_inspection(
    out: &mut Writer,
    inspection: &ImageInspection,
) -> Result<(), CodecError> {
    encode_route(out, &inspection.route)?;
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
    encode_private_text(out, &inspection.hand_custody)?;
    out.u32_be(u32::try_from(inspection.staging.len()).map_err(|_| CodecError::TooLong)?)?;
    for identity in &inspection.staging {
        encode_private_text(out, &identity.locator_id)?;
        encode_private_text(out, &identity.reference_id)?;
        encode_private_text(out, &identity.public_name)?;
        out.boolean(identity.hero)?;
    }
    out.u32_be(inspection.maximum_corrections)?;
    out.boolean(inspection.allow_visible_text)?;
    out.push(match inspection.requirement {
        InspectionRequirement::BestEffort => 0,
        InspectionRequirement::Required => 1,
    })
}

fn read_private_text(
    input: &mut Reader<'_>,
    maximum_bytes: usize,
) -> Result<PrivateImageText, CodecError> {
    Ok(PrivateImageText::new(input.text(maximum_bytes)?))
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
            route: decode_route(input)?,
            contract_revision: input.u32_be()?,
            public_appearance: read_private_text(
                input,
                MAXIMUM_PUBLIC_APPEARANCE_BYTES.min(prompt_byte_bound(limits)),
            )?,
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
        Some(decode_moderation(input)?)
    } else {
        None
    };
    if version >= super::PIPELINE_INTENT_CODEC_VERSION && input.boolean()? {
        let general = read_private_text(input, prompt_byte_bound(limits))?;
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
    let route = decode_route(input)?;
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
    let hand_custody = read_private_text(input, prompt_byte_bound(limits))?;
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    // Every claimed identity must fit the unread input before the vector is allocated.
    if count > MAXIMUM_STAGED_IDENTITIES
        || count
            .checked_mul(MIN_STAGED_IDENTITY_BYTES)
            .is_none_or(|minimum| minimum > input.remaining())
    {
        return Err(CodecError::TooLong);
    }
    let mut staging = Vec::with_capacity(count);
    for _ in 0..count {
        staging.push(StagedIdentity {
            locator_id: read_private_text(input, prompt_byte_bound(limits))?,
            reference_id: read_private_text(input, prompt_byte_bound(limits))?,
            public_name: read_private_text(input, prompt_byte_bound(limits))?,
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
