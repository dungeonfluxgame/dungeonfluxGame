//! Versioned non-raster intent and decision snapshots. Caller ids and urgency never steer identity.
use super::analysis_types::*;
use super::{OperationHandle, PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES};
use crate::intent_codec::{
    CodecError, decode, encode,
    primitives::{Reader, Writer},
};
use crate::{
    Audience, ExecutionMode, Freshness, ImageOwner, PrivateImageText, Purpose, PurposedUse,
    RequestLimits, SourceRef,
};
use sha2::{Digest, Sha256};

pub(crate) const VERSION: u8 = 64;
const RESULT_VERSION: u8 = 1;
mod result;
pub(crate) use result::{decode_result, encode_result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodedAnalysis {
    pub owner: ImageOwner,
    pub purpose: PurposedUse,
    pub operation: ImageAnalysisOperation,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub audience: Audience,
    pub expected_source_revision: u64,
}
impl DecodedAnalysis {
    pub(crate) fn from_request(request: &ImageAnalysisRequest) -> Self {
        Self {
            owner: request.identity.owner.clone(),
            purpose: request.purpose,
            operation: request.operation.clone(),
            execution: request.execution,
            freshness: request.freshness,
            audience: request.audience.clone(),
            expected_source_revision: request.expected_source_revision,
        }
    }
}
pub(crate) fn is_analysis_intent(bytes: &[u8]) -> bool {
    bytes.first() == Some(&VERSION)
}

pub(crate) fn validate(value: &DecodedAnalysis, limits: &RequestLimits) -> Result<(), CodecError> {
    limits.validate().map_err(CodecError::Request)?;
    if value.audience == Audience::Withheld {
        return Err(CodecError::Malformed);
    }
    let mut text_chars = 0usize;
    let mut text = |value: &PrivateImageText| -> Result<(), CodecError> {
        if value.text().is_empty() {
            return Err(CodecError::Malformed);
        }
        text_chars = text_chars
            .checked_add(value.text().chars().count())
            .ok_or(CodecError::TooLong)?;
        if text_chars > limits.max_prompt_chars {
            return Err(CodecError::TooLong);
        }
        Ok(())
    };
    let revision = match &value.operation {
        ImageAnalysisOperation::PhotoInspection(spec) => {
            if spec.references.len() > limits.max_references {
                return Err(CodecError::TooLong);
            }
            crate::ImagePipeline {
                revision: 1,
                moderation: None,
                rating: crate::ImageContentRating::Restrained,
                creator: None,
                inspection: Some(spec.inspection.clone()),
            }
            .validate(limits)
            .map_err(CodecError::Request)?;
            spec.inspection.contract_revision
        }
        ImageAnalysisOperation::HeadLocations(spec) => {
            if spec.targets.is_empty() || spec.targets.len() > limits.max_references {
                return Err(CodecError::TooLong);
            }
            let mut seen = std::collections::BTreeSet::new();
            for target in &spec.targets {
                text(&target.id)?;
                text(&target.description)?;
                if !seen.insert(target.id.text()) {
                    return Err(CodecError::Malformed);
                }
            }
            spec.contract_revision
        }
        ImageAnalysisOperation::Segmentation(spec) => {
            if spec.prompts.is_empty()
                || spec.prompts.len() > 2
                || spec.maximum_masks == 0
                || spec.prompts.iter().enumerate().any(|(index, prompt)| {
                    spec.prompts
                        .iter()
                        .take(index)
                        .any(|earlier| earlier == prompt)
                })
            {
                return Err(CodecError::Malformed);
            }
            spec.contract_revision
        }
        ImageAnalysisOperation::StagingIdentity(spec) => {
            if spec.references.len() > limits.max_references {
                return Err(CodecError::TooLong);
            }
            let mut seen = std::collections::BTreeSet::new();
            for reference in &spec.references {
                text(&reference.id)?;
                if !seen.insert(reference.id.text()) {
                    return Err(CodecError::Malformed);
                }
            }
            validate_dependency(&value.owner, &spec.segmentation)?;
            if let Some(locations) = &spec.locations {
                validate_dependency(&value.owner, locations)?;
            }
            spec.contract_revision
        }
        ImageAnalysisOperation::StagingFacing(spec) => {
            validate_dependency(&value.owner, &spec.segmentation)?;
            validate_dependency(&value.owner, &spec.identity)?;
            spec.contract_revision
        }
    };
    if revision == 0
        || sources(&value.operation)
            .iter()
            .any(|source| source.audience == Audience::Withheld)
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
fn validate_dependency(owner: &ImageOwner, handle: &OperationHandle) -> Result<(), CodecError> {
    if &handle.owner != owner {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
pub(crate) fn sources(operation: &ImageAnalysisOperation) -> Vec<&SourceRef> {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => std::iter::once(&spec.source)
            .chain(spec.references.iter())
            .collect(),
        ImageAnalysisOperation::HeadLocations(spec) => std::iter::once(&spec.source)
            .chain(spec.face.iter())
            .collect(),
        ImageAnalysisOperation::Segmentation(spec) => vec![&spec.source],
        ImageAnalysisOperation::StagingIdentity(spec) => std::iter::once(&spec.source)
            .chain(spec.references.iter().map(|reference| &reference.source))
            .collect(),
        ImageAnalysisOperation::StagingFacing(spec) => vec![&spec.source],
    }
}
pub(crate) fn source(operation: &ImageAnalysisOperation) -> &SourceRef {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => &spec.source,
        ImageAnalysisOperation::HeadLocations(spec) => &spec.source,
        ImageAnalysisOperation::Segmentation(spec) => &spec.source,
        ImageAnalysisOperation::StagingIdentity(spec) => &spec.source,
        ImageAnalysisOperation::StagingFacing(spec) => &spec.source,
    }
}
pub(crate) fn revision(operation: &ImageAnalysisOperation) -> u32 {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => spec.inspection.contract_revision,
        ImageAnalysisOperation::HeadLocations(spec) => spec.contract_revision,
        ImageAnalysisOperation::Segmentation(spec) => spec.contract_revision,
        ImageAnalysisOperation::StagingIdentity(spec) => spec.contract_revision,
        ImageAnalysisOperation::StagingFacing(spec) => spec.contract_revision,
    }
}
pub(crate) fn fingerprint(value: &DecodedAnalysis) -> Result<[u8; 32], CodecError> {
    Ok(Sha256::digest(encode_analysis(value)?).into())
}
pub(crate) fn encode_analysis(value: &DecodedAnalysis) -> Result<Vec<u8>, CodecError> {
    validate(value, &PROTOCOL_LIMITS)?;
    let mut out = Writer::new(PROTOCOL_MAX_SNAPSHOT_BYTES);
    out.push(VERSION)?;
    encode::encode_owner(&mut out, &value.owner)?;
    out.push(u8::try_from(value.purpose.purpose.index()).map_err(|_| CodecError::TooLong)?)?;
    out.push(encode::role_tag(value.purpose.role))?;
    encode_operation(&mut out, &value.operation)?;
    encode::encode_execution(&mut out, value.execution)?;
    encode::encode_freshness(&mut out, &value.freshness)?;
    encode::encode_audience(&mut out, &value.audience)?;
    out.u64_be(value.expected_source_revision)?;
    Ok(out.finish())
}
pub(crate) fn decode_analysis(bytes: &[u8]) -> Result<DecodedAnalysis, CodecError> {
    if bytes.len() > PROTOCOL_MAX_SNAPSHOT_BYTES {
        return Err(CodecError::TooLong);
    }
    let mut input = Reader::new(bytes);
    if input.byte()? != VERSION {
        return Err(CodecError::Malformed);
    }
    let owner = decode::decode_owner(&mut input)?;
    let purpose = Purpose::from_index(usize::from(input.byte()?)).ok_or(CodecError::Malformed)?;
    let role = decode::decode_role(input.byte()?)?;
    let operation = decode_operation(&mut input)?;
    let execution = decode::decode_execution(input.byte()?)?;
    let freshness = decode::decode_freshness(&mut input)?;
    let audience = decode::decode_audience(&mut input)?;
    let expected_source_revision = input.u64_be()?;
    input.finish()?;
    let value = DecodedAnalysis {
        owner,
        purpose: PurposedUse { purpose, role },
        operation,
        execution,
        freshness,
        audience,
        expected_source_revision,
    };
    validate(&value, &PROTOCOL_LIMITS)?;
    // Reject alternate encodings (including duplicate audience members) at the persistence
    // boundary.
    if encode_analysis(&value)? != bytes {
        return Err(CodecError::Malformed);
    }
    Ok(value)
}
fn text(out: &mut Writer, value: &PrivateImageText) -> Result<(), CodecError> {
    out.counted(value.text().as_bytes())
}
fn read_text(input: &mut Reader<'_>) -> Result<PrivateImageText, CodecError> {
    let bytes = input.counted(PROTOCOL_MAX_SNAPSHOT_BYTES)?;
    Ok(PrivateImageText::new(
        std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidText)?,
    ))
}
fn count(input: &mut Reader<'_>, minimum: usize) -> Result<usize, CodecError> {
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    if count
        .checked_mul(minimum)
        .is_none_or(|bytes| bytes > input.remaining())
    {
        return Err(CodecError::TooLong);
    }
    Ok(count)
}
fn handle(out: &mut Writer, value: &OperationHandle) -> Result<(), CodecError> {
    encode::encode_owner(out, &value.owner)?;
    out.extend(value.operation.as_bytes())
}
fn read_handle(input: &mut Reader<'_>) -> Result<OperationHandle, CodecError> {
    Ok(OperationHandle {
        owner: decode::decode_owner(input)?,
        operation: df_types::OperationId::from_bytes(input.take(16)?)
            .map_err(|_| CodecError::InvalidId)?,
    })
}
fn prompt_tag(prompt: SegmentationPrompt) -> u8 {
    match prompt {
        SegmentationPrompt::Person => 0,
        SegmentationPrompt::Head => 1,
    }
}
fn read_prompt(input: &mut Reader<'_>) -> Result<SegmentationPrompt, CodecError> {
    match input.byte()? {
        0 => Ok(SegmentationPrompt::Person),
        1 => Ok(SegmentationPrompt::Head),
        _ => Err(CodecError::Malformed),
    }
}
fn encode_operation(
    out: &mut Writer,
    operation: &ImageAnalysisOperation,
) -> Result<(), CodecError> {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => {
            out.push(4)?;
            encode::encode_source_ref(out, &spec.source)?;
            out.u32_be(u32::try_from(spec.references.len()).map_err(|_| CodecError::TooLong)?)?;
            for reference in &spec.references {
                encode::encode_source_ref(out, reference)?;
            }
            crate::intent_codec::pipeline::encode_inspection(out, &spec.inspection)?;
        }
        ImageAnalysisOperation::HeadLocations(spec) => {
            out.push(0)?;
            encode::encode_source_ref(out, &spec.source)?;
            out.boolean(spec.face.is_some())?;
            if let Some(face) = &spec.face {
                encode::encode_source_ref(out, face)?;
            }
            out.u32_be(u32::try_from(spec.targets.len()).map_err(|_| CodecError::TooLong)?)?;
            for target in &spec.targets {
                text(out, &target.id)?;
                text(out, &target.description)?;
            }
        }
        ImageAnalysisOperation::Segmentation(spec) => {
            out.push(1)?;
            encode::encode_source_ref(out, &spec.source)?;
            out.push(match spec.preparation {
                SegmentationPreparation::FigureMask => 0,
                SegmentationPreparation::ScenePlate => 1,
            })?;
            out.u32_be(u32::try_from(spec.prompts.len()).map_err(|_| CodecError::TooLong)?)?;
            for prompt in &spec.prompts {
                out.push(prompt_tag(*prompt))?;
            }
            out.u32_be(spec.maximum_masks)?;
        }
        ImageAnalysisOperation::StagingIdentity(spec) => {
            out.push(2)?;
            encode::encode_source_ref(out, &spec.source)?;
            handle(out, &spec.segmentation)?;
            out.boolean(spec.locations.is_some())?;
            if let Some(locations) = &spec.locations {
                handle(out, locations)?;
            }
            out.u32_be(u32::try_from(spec.references.len()).map_err(|_| CodecError::TooLong)?)?;
            for reference in &spec.references {
                text(out, &reference.id)?;
                encode::encode_source_ref(out, &reference.source)?;
            }
        }
        ImageAnalysisOperation::StagingFacing(spec) => {
            out.push(3)?;
            encode::encode_source_ref(out, &spec.source)?;
            handle(out, &spec.segmentation)?;
            handle(out, &spec.identity)?;
        }
    }
    out.u32_be(revision(operation))
}
fn decode_operation(input: &mut Reader<'_>) -> Result<ImageAnalysisOperation, CodecError> {
    let tag = input.byte()?;
    let source = decode::decode_source_ref(input)?;
    Ok(match tag {
        4 => {
            let length = count(input, 49)?;
            let mut references = Vec::with_capacity(length);
            for _ in 0..length {
                references.push(decode::decode_source_ref(input)?);
            }
            let inspection = crate::intent_codec::pipeline::read_inspection(
                input,
                &PROTOCOL_LIMITS,
                crate::intent_codec::PIPELINE_INTENT_CODEC_VERSION,
            )?;
            if input.u32_be()? != inspection.contract_revision {
                return Err(CodecError::Malformed);
            }
            ImageAnalysisOperation::PhotoInspection(PhotoInspectionSpec {
                source,
                references,
                inspection,
            })
        }
        0 => {
            let face = if input.boolean()? {
                Some(decode::decode_source_ref(input)?)
            } else {
                None
            };
            let length = count(input, 8)?;
            let mut targets = Vec::with_capacity(length);
            for _ in 0..length {
                targets.push(AnalysisTarget {
                    id: read_text(input)?,
                    description: read_text(input)?,
                });
            }
            ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
                source,
                face,
                targets,
                contract_revision: input.u32_be()?,
            })
        }
        1 => {
            let preparation = match input.byte()? {
                0 => SegmentationPreparation::FigureMask,
                1 => SegmentationPreparation::ScenePlate,
                _ => return Err(CodecError::Malformed),
            };
            let length = count(input, 1)?;
            if length > 2 {
                return Err(CodecError::TooLong);
            }
            let mut prompts = Vec::with_capacity(length);
            for _ in 0..length {
                prompts.push(read_prompt(input)?);
            }
            ImageAnalysisOperation::Segmentation(SegmentationSpec {
                preparation,
                source,
                prompts,
                maximum_masks: input.u32_be()?,
                contract_revision: input.u32_be()?,
            })
        }
        2 => {
            let segmentation = read_handle(input)?;
            let locations = if input.boolean()? {
                Some(read_handle(input)?)
            } else {
                None
            };
            let length = count(input, 53)?;
            let mut references = Vec::with_capacity(length);
            for _ in 0..length {
                references.push(AnalysisReference {
                    id: read_text(input)?,
                    source: decode::decode_source_ref(input)?,
                });
            }
            ImageAnalysisOperation::StagingIdentity(StagingIdentitySpec {
                source,
                segmentation,
                locations,
                references,
                contract_revision: input.u32_be()?,
            })
        }
        3 => ImageAnalysisOperation::StagingFacing(StagingFacingSpec {
            source,
            segmentation: read_handle(input)?,
            identity: read_handle(input)?,
            contract_revision: input.u32_be()?,
        }),
        _ => return Err(CodecError::Malformed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetDigest, ImageAnalysisOperation, SegmentationSpec, VariantRole};
    fn intent() -> DecodedAnalysis {
        DecodedAnalysis {
            owner: ImageOwner::Draft {
                account: df_types::AccountId::from_bytes(&[1; 16]).unwrap(),
            },
            purpose: PurposedUse {
                purpose: Purpose::ReferencePrep,
                role: VariantRole::Mask,
            },
            operation: ImageAnalysisOperation::Segmentation(SegmentationSpec {
                source: SourceRef {
                    asset: df_assets::AssetId::from_bytes(&[2; 16]).unwrap(),
                    digest: AssetDigest::from_bytes([3; 32]),
                    audience: Audience::PublicStatic,
                },
                preparation: SegmentationPreparation::FigureMask,
                prompts: vec![SegmentationPrompt::Person, SegmentationPrompt::Head],
                maximum_masks: 8,
                contract_revision: 1,
            }),
            execution: ExecutionMode::Live,
            freshness: Freshness::ReuseCompatible,
            audience: Audience::OwnerPrivate(df_types::AccountId::from_bytes(&[1; 16]).unwrap()),
            expected_source_revision: 1,
        }
    }
    #[test]
    fn immutable_analysis_roundtrips_without_raster_output_or_caller_identity() {
        let value = intent();
        let bytes = encode_analysis(&value).unwrap();
        assert!(is_analysis_intent(&bytes));
        assert_eq!(decode_analysis(&bytes).unwrap(), value);
        let mut changed = value.clone();
        if let ImageAnalysisOperation::Segmentation(spec) = &mut changed.operation {
            spec.preparation = SegmentationPreparation::ScenePlate;
        }
        assert_ne!(fingerprint(&value).unwrap(), fingerprint(&changed).unwrap());
    }
    #[test]
    fn malformed_lengths_trailing_bytes_and_cross_owner_dependencies_are_rejected() {
        let value = intent();
        let bytes = encode_analysis(&value).unwrap();
        for length in 0..bytes.len() {
            assert!(decode_analysis(&bytes[..length]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(decode_analysis(&extra).is_err());
        let mut changed = value;
        changed.operation = ImageAnalysisOperation::StagingFacing(StagingFacingSpec {
            source: source(&changed.operation).clone(),
            segmentation: OperationHandle {
                owner: ImageOwner::Draft {
                    account: df_types::AccountId::from_bytes(&[4; 16]).unwrap(),
                },
                operation: df_types::OperationId::from_bytes(&[5; 16]).unwrap(),
            },
            identity: OperationHandle {
                owner: changed.owner.clone(),
                operation: df_types::OperationId::from_bytes(&[6; 16]).unwrap(),
            },
            contract_revision: 1,
        });
        assert!(encode_analysis(&changed).is_err());
    }
    #[test]
    fn segmentation_decision_roundtrips_and_invalid_runs_never_encode() {
        let value = intent();
        let mut result = ImageAnalysisResult::Segmentation {
            source: source(&value.operation).clone(),
            contract_revision: 1,
            masks: vec![AnalysisMaskSet {
                prompt: SegmentationPrompt::Person,
                width_px: 2,
                height_px: 2,
                instances: vec![AnalysisMask {
                    index: 0,
                    score: AnalysisUnit::new(0.9).unwrap(),
                    runs: vec![AnalysisMaskRun {
                        start: 1,
                        length: 2,
                    }],
                }],
            }],
        };
        let bytes = encode_result(&result, 1024).unwrap();
        assert_eq!(decode_result(&bytes, 1024).unwrap(), result);
        if let ImageAnalysisResult::Segmentation { masks, .. } = &mut result {
            masks[0].instances[0].runs[0].length = 4;
        }
        assert!(encode_result(&result, 1024).is_err());
    }
}
