//! Bounded typed decision serialization; no publication identity is inferred.
use super::*;

pub(crate) fn encode_result(
    value: &ImageAnalysisResult,
    maximum: usize,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Writer::new(maximum);
    out.push(RESULT_VERSION)?;
    let (tag, source, revision) = match value {
        ImageAnalysisResult::PhotoInspection { source, assessment } => {
            (4, source, &assessment.contract_revision)
        }
        ImageAnalysisResult::HeadLocations {
            source,
            contract_revision,
            ..
        } => (0, source, contract_revision),
        ImageAnalysisResult::Segmentation {
            source,
            contract_revision,
            ..
        } => (1, source, contract_revision),
        ImageAnalysisResult::StagingIdentity {
            source,
            contract_revision,
            ..
        } => (2, source, contract_revision),
        ImageAnalysisResult::StagingFacing {
            source,
            contract_revision,
            ..
        } => (3, source, contract_revision),
    };
    if *revision == 0 {
        return Err(CodecError::Malformed);
    }
    out.push(tag)?;
    encode::encode_source_ref(&mut out, source)?;
    out.u32_be(*revision)?;
    match value {
        ImageAnalysisResult::PhotoInspection { assessment, .. } => {
            encode_photo(&mut out, assessment)?
        }
        ImageAnalysisResult::HeadLocations { heads, .. } => {
            out.u32_be(u32::try_from(heads.len()).map_err(|_| CodecError::TooLong)?)?;
            for head in heads {
                text(&mut out, &head.id)?;
                for coordinate in head.head.coordinates() {
                    out.float(coordinate, "head")?;
                }
            }
        }
        ImageAnalysisResult::Segmentation { masks, .. } => {
            out.u32_be(u32::try_from(masks.len()).map_err(|_| CodecError::TooLong)?)?;
            for set in masks {
                set.validate().map_err(|_| CodecError::Malformed)?;
                out.push(prompt_tag(set.prompt))?;
                out.u32_be(set.width_px)?;
                out.u32_be(set.height_px)?;
                out.u32_be(u32::try_from(set.instances.len()).map_err(|_| CodecError::TooLong)?)?;
                for mask in &set.instances {
                    out.u32_be(mask.index)?;
                    out.float(mask.score.get(), "score")?;
                    out.u32_be(u32::try_from(mask.runs.len()).map_err(|_| CodecError::TooLong)?)?;
                    for run in &mask.runs {
                        out.u32_be(run.start)?;
                        out.u32_be(run.length)?;
                    }
                }
            }
        }
        ImageAnalysisResult::StagingIdentity { matches, .. } => {
            out.u32_be(u32::try_from(matches.len()).map_err(|_| CodecError::TooLong)?)?;
            for matched in matches {
                text(&mut out, &matched.reference_id)?;
                out.boolean(matched.candidate.is_some())?;
                if let Some(candidate) = matched.candidate {
                    out.u32_be(candidate)?;
                }
            }
        }
        ImageAnalysisResult::StagingFacing { facings, .. } => {
            out.u32_be(u32::try_from(facings.len()).map_err(|_| CodecError::TooLong)?)?;
            for facing in facings {
                out.u32_be(facing.candidate)?;
                out.push(match facing.facing {
                    None => 0,
                    Some(AnalysisFacingDirection::Left) => 1,
                    Some(AnalysisFacingDirection::Right) => 2,
                    Some(AnalysisFacingDirection::TowardCamera) => 3,
                    Some(AnalysisFacingDirection::AwayFromCamera) => 4,
                })?;
            }
        }
    }
    Ok(out.finish())
}
pub(crate) fn decode_result(
    bytes: &[u8],
    maximum: usize,
) -> Result<ImageAnalysisResult, CodecError> {
    if bytes.len() > maximum {
        return Err(CodecError::TooLong);
    }
    let mut input = Reader::new(bytes);
    if input.byte()? != RESULT_VERSION {
        return Err(CodecError::Malformed);
    }
    let tag = input.byte()?;
    let source = decode::decode_source_ref(&mut input)?;
    let contract_revision = input.u32_be()?;
    let length = if tag == 4 {
        0
    } else {
        count(
            &mut input,
            match tag {
                0 => 20,
                1 => 13,
                2 | 3 => 5,
                _ => return Err(CodecError::Malformed),
            },
        )?
    };
    let value = match tag {
        4 => ImageAnalysisResult::PhotoInspection {
            source,
            assessment: decode_photo(&mut input, contract_revision)?,
        },
        0 => {
            let mut heads = Vec::with_capacity(length);
            for _ in 0..length {
                heads.push(AnalysisLocatedHead {
                    id: read_text(&mut input)?,
                    head: NormalizedImageBox::new(
                        input.float("head")?,
                        input.float("head")?,
                        input.float("head")?,
                        input.float("head")?,
                    )
                    .map_err(|_| CodecError::Malformed)?,
                });
            }
            ImageAnalysisResult::HeadLocations {
                source,
                contract_revision,
                heads,
            }
        }
        1 => {
            let mut masks = Vec::with_capacity(length);
            for _ in 0..length {
                let prompt = read_prompt(&mut input)?;
                let width_px = input.u32_be()?;
                let height_px = input.u32_be()?;
                let instances_length = count(&mut input, 12)?;
                let mut instances = Vec::with_capacity(instances_length);
                for _ in 0..instances_length {
                    let index = input.u32_be()?;
                    let score = AnalysisUnit::new(input.float("score")?)
                        .map_err(|_| CodecError::Malformed)?;
                    let run_length = count(&mut input, 8)?;
                    let mut runs = Vec::with_capacity(run_length);
                    for _ in 0..run_length {
                        runs.push(AnalysisMaskRun {
                            start: input.u32_be()?,
                            length: input.u32_be()?,
                        });
                    }
                    instances.push(AnalysisMask { index, score, runs });
                }
                let set = AnalysisMaskSet {
                    prompt,
                    width_px,
                    height_px,
                    instances,
                };
                set.validate().map_err(|_| CodecError::Malformed)?;
                masks.push(set);
            }
            ImageAnalysisResult::Segmentation {
                source,
                contract_revision,
                masks,
            }
        }
        2 => {
            let mut matches = Vec::with_capacity(length);
            for _ in 0..length {
                matches.push(AnalysisIdentityMatch {
                    reference_id: read_text(&mut input)?,
                    candidate: if input.boolean()? {
                        Some(input.u32_be()?)
                    } else {
                        None
                    },
                });
            }
            ImageAnalysisResult::StagingIdentity {
                source,
                contract_revision,
                matches,
            }
        }
        3 => {
            let mut facings = Vec::with_capacity(length);
            for _ in 0..length {
                let candidate = input.u32_be()?;
                let facing = match input.byte()? {
                    0 => None,
                    1 => Some(AnalysisFacingDirection::Left),
                    2 => Some(AnalysisFacingDirection::Right),
                    3 => Some(AnalysisFacingDirection::TowardCamera),
                    4 => Some(AnalysisFacingDirection::AwayFromCamera),
                    _ => return Err(CodecError::Malformed),
                };
                facings.push(AnalysisFacing { candidate, facing });
            }
            ImageAnalysisResult::StagingFacing {
                source,
                contract_revision,
                facings,
            }
        }
        _ => return Err(CodecError::Malformed),
    };
    input.finish()?;
    if encode_result(&value, maximum)? != bytes {
        return Err(CodecError::Malformed);
    }
    Ok(value)
}

fn encode_photo(out: &mut Writer, value: &PhotoInspectionAssessment) -> Result<(), CodecError> {
    match value.verdict {
        PhotoInspectionVerdict::Pass => out.push(0)?,
        PhotoInspectionVerdict::Failed(failure) => {
            out.push(1)?;
            out.push(match failure {
                PhotoInspectionCause::GridOrCollage => 0,
                PhotoInspectionCause::CutoutComposite => 1,
                PhotoInspectionCause::NotPhotograph => 2,
                PhotoInspectionCause::ModernSetting => 3,
                PhotoInspectionCause::VisibleText => 4,
                PhotoInspectionCause::ModernClothing => 5,
                PhotoInspectionCause::PeopleInEmptyRoom => 6,
                PhotoInspectionCause::CharacterCountNotOne => 7,
                PhotoInspectionCause::CharacterHasNoPeople => 8,
                PhotoInspectionCause::HeroCount { .. } => 9,
                PhotoInspectionCause::UnexpectedDrawnWeapon => 10,
            })?;
            if let PhotoInspectionCause::HeroCount { expected, seen } = failure {
                out.push(expected)?;
                out.u64_be(seen)?;
            }
        }
        PhotoInspectionVerdict::Unjudged(reason) => {
            out.push(2)?;
            out.push(match reason {
                PhotoInspectionUnjudgedReason::Preparation => 0,
                PhotoInspectionUnjudgedReason::Transport => 1,
                PhotoInspectionUnjudgedReason::Provider => 2,
                PhotoInspectionUnjudgedReason::InvalidResponse => 3,
                PhotoInspectionUnjudgedReason::Deadline => 4,
                PhotoInspectionUnjudgedReason::Disabled => 5,
                PhotoInspectionUnjudgedReason::Cancelled => 6,
            })?;
        }
    }
    out.u32_be(u32::try_from(value.visible_features.len()).map_err(|_| CodecError::TooLong)?)?;
    for feature in &value.visible_features {
        text(out, &feature.name)?;
        text(out, &feature.look)?;
        let [left, top, right, bottom] = feature.region;
        if left >= right || top >= bottom || right > 1000 || bottom > 1000 {
            return Err(CodecError::Malformed);
        }
        for coordinate in feature.region {
            out.u16_be(coordinate)?;
        }
        out.boolean(feature.recorded_id.is_some())?;
        if let Some(id) = &feature.recorded_id {
            text(out, id)?;
        }
    }
    out.boolean(value.source_binding.is_some())?;
    if let Some(binding) = &value.source_binding {
        text(out, &binding.place)?;
        text(out, &binding.source_hash)?;
    }
    Ok(())
}
fn decode_photo(
    input: &mut Reader<'_>,
    contract_revision: u32,
) -> Result<PhotoInspectionAssessment, CodecError> {
    let verdict = match input.byte()? {
        0 => PhotoInspectionVerdict::Pass,
        1 => PhotoInspectionVerdict::Failed(match input.byte()? {
            0 => PhotoInspectionCause::GridOrCollage,
            1 => PhotoInspectionCause::CutoutComposite,
            2 => PhotoInspectionCause::NotPhotograph,
            3 => PhotoInspectionCause::ModernSetting,
            4 => PhotoInspectionCause::VisibleText,
            5 => PhotoInspectionCause::ModernClothing,
            6 => PhotoInspectionCause::PeopleInEmptyRoom,
            7 => PhotoInspectionCause::CharacterCountNotOne,
            8 => PhotoInspectionCause::CharacterHasNoPeople,
            9 => PhotoInspectionCause::HeroCount {
                expected: input.byte()?,
                seen: input.u64_be()?,
            },
            10 => PhotoInspectionCause::UnexpectedDrawnWeapon,
            _ => return Err(CodecError::Malformed),
        }),
        2 => PhotoInspectionVerdict::Unjudged(match input.byte()? {
            0 => PhotoInspectionUnjudgedReason::Preparation,
            1 => PhotoInspectionUnjudgedReason::Transport,
            2 => PhotoInspectionUnjudgedReason::Provider,
            3 => PhotoInspectionUnjudgedReason::InvalidResponse,
            4 => PhotoInspectionUnjudgedReason::Deadline,
            5 => PhotoInspectionUnjudgedReason::Disabled,
            6 => PhotoInspectionUnjudgedReason::Cancelled,
            _ => return Err(CodecError::Malformed),
        }),
        _ => return Err(CodecError::Malformed),
    };
    let length = count(input, 17)?;
    let mut visible_features = Vec::with_capacity(length);
    for _ in 0..length {
        visible_features.push(PhotoVisibleFeature {
            name: read_text(input)?,
            look: read_text(input)?,
            region: [
                input.u16_be()?,
                input.u16_be()?,
                input.u16_be()?,
                input.u16_be()?,
            ],
            recorded_id: if input.boolean()? {
                Some(read_text(input)?)
            } else {
                None
            },
        });
    }
    let source_binding = if input.boolean()? {
        Some(PhotoFeatureBinding {
            place: read_text(input)?,
            source_hash: read_text(input)?,
        })
    } else {
        None
    };
    Ok(PhotoInspectionAssessment {
        contract_revision,
        verdict,
        visible_features,
        source_binding,
    })
}
