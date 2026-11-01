//! Standalone inspection delegates the same photographic transport and closed policy.
use crate::{
    backends::analysis::{InspectionInput, PreparedInspection, photo},
    config::AnalysisPreparationLimits,
    request::{ImageInspection, PrivateImageText, SourceRef},
    runtime::{
        RuntimeError,
        inspection_policy::{
            self, FeatureBinding, InspectionAssessment, InspectionError, InspectionVerdict,
            PhotoCheckFailure, UnjudgedReason, VisibleFeature,
        },
    },
    service::analysis_types::*,
};
use df_providers::NativeHttpResponse;
use serde_json::Value;
use std::time::Duration;

/// Authorized saved source and ordered references for a single inspection, with no generation.
pub(crate) struct PhotoInspectionInput<'a> {
    pub(crate) spec: &'a PhotoInspectionSpec,
    pub(crate) source: &'a [u8],
    pub(crate) references: &'a [(SourceRef, Vec<u8>)],
    pub(crate) preparation: &'a AnalysisPreparationLimits,
    pub(crate) max_decoded_bytes: u64,
    pub(crate) timeout: Duration,
    pub(crate) maximum_response_bytes: usize,
}
/// The existing photographic exchange and its exact admission facts.
pub(crate) struct PreparedPhotoInspection {
    inner: PreparedInspection,
    pub(crate) text_bytes: u64,
    pub(crate) body_bytes: u64,
    pub(crate) picture_count: u64,
}
impl std::fmt::Debug for PreparedPhotoInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedPhotoInspection")
            .field("body_bytes", &self.body_bytes)
            .field("text_bytes", &self.text_bytes)
            .field("picture_count", &self.picture_count)
            .finish_non_exhaustive()
    }
}
impl PreparedPhotoInspection {
    pub(crate) fn prepare(
        input: PhotoInspectionInput<'_>,
        credential: &str,
    ) -> Result<Self, RuntimeError> {
        super::chat::verify(&input.spec.source, input.source)?;
        if input.spec.references.len() != input.references.len() {
            return Err(RuntimeError::SourceDenied);
        }
        for (expected, (actual, bytes)) in input.spec.references.iter().zip(input.references) {
            if expected != actual {
                return Err(RuntimeError::SourceDenied);
            }
            super::chat::verify(expected, bytes)?;
        }
        let inner = PreparedInspection::prepare(
            InspectionInput {
                spec: &input.spec.inspection,
                candidate: input.source,
                references: input.references,
                timeout: input.timeout,
                maximum_response_bytes: input.maximum_response_bytes,
                preparation: input.preparation,
                max_decoded_bytes: input.max_decoded_bytes,
            },
            credential,
        )?;
        Ok(Self {
            text_bytes: inner.text_bytes,
            body_bytes: inner.body_bytes,
            picture_count: inner.picture_count,
            inner,
        })
    }
    pub(crate) async fn execute(self) -> Result<NativeHttpResponse, RuntimeError> {
        self.inner.execute().await
    }
}
/// Projects validated closed labels and bounded features with their optional trusted binding.
pub(crate) fn project_photo_assessment(
    assessment: &InspectionAssessment,
) -> PhotoInspectionAssessment {
    let verdict = match assessment.verdict {
        InspectionVerdict::Pass => PhotoInspectionVerdict::Pass,
        InspectionVerdict::Failed(failure) => {
            PhotoInspectionVerdict::Failed(project_failure(failure))
        }
        InspectionVerdict::Unjudged(reason) => {
            PhotoInspectionVerdict::Unjudged(project_reason(reason))
        }
    };
    PhotoInspectionAssessment {
        contract_revision: assessment.contract_revision,
        verdict,
        visible_features: assessment
            .visible_features
            .iter()
            .map(|feature| PhotoVisibleFeature {
                name: PrivateImageText::new(&feature.name),
                look: PrivateImageText::new(&feature.look),
                region: feature.region,
                recorded_id: feature.recorded_id.as_deref().map(PrivateImageText::new),
            })
            .collect(),
        source_binding: assessment
            .source_binding
            .as_ref()
            .map(|binding| PhotoFeatureBinding {
                place: PrivateImageText::new(&binding.place),
                source_hash: PrivateImageText::new(&binding.source_hash),
            }),
    }
}
/// Revalidates a neutral retained assessment through the same strict canonical private schema.
pub(crate) fn restore_photo_assessment(
    assessment: &PhotoInspectionAssessment,
    spec: &ImageInspection,
) -> Result<InspectionAssessment, InspectionError> {
    let verdict = match assessment.verdict {
        PhotoInspectionVerdict::Pass => InspectionVerdict::Pass,
        PhotoInspectionVerdict::Failed(failure) => {
            InspectionVerdict::Failed(restore_failure(failure))
        }
        PhotoInspectionVerdict::Unjudged(reason) => {
            InspectionVerdict::Unjudged(restore_reason(reason))
        }
    };
    let restored = InspectionAssessment {
        contract_revision: assessment.contract_revision,
        verdict,
        visible_features: assessment
            .visible_features
            .iter()
            .map(|feature| VisibleFeature {
                name: feature.name.text().to_owned(),
                look: feature.look.text().to_owned(),
                region: feature.region,
                recorded_id: feature.recorded_id.as_ref().map(|id| id.text().to_owned()),
            })
            .collect(),
        source_binding: assessment
            .source_binding
            .as_ref()
            .map(|binding| FeatureBinding {
                place: binding.place.text().to_owned(),
                source_hash: binding.source_hash.text().to_owned(),
            }),
    };
    InspectionAssessment::from_value(&restored.to_value(), spec)
}
/// Keeps unavailable observations explicitly unjudged through the existing compatibility policy.
pub(crate) fn unjudged_photo(
    spec: &PhotoInspectionSpec,
    reason: PhotoInspectionUnjudgedReason,
) -> PhotoInspectionAssessment {
    project_photo_assessment(&InspectionAssessment::unjudged(
        &spec.inspection,
        restore_reason(reason),
    ))
}
/// Interprets retained model output with the existing schema and policy before neutral projection.
pub(crate) fn interpret_photo(
    value: &Value,
    spec: &PhotoInspectionSpec,
) -> Result<PhotoInspectionAssessment, RuntimeError> {
    let answer = photo::answer(value).map_err(|_| RuntimeError::InvalidRecord)?;
    let assessment = inspection_policy::interpret(answer, &spec.inspection)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(project_photo_assessment(&assessment))
}

fn project_failure(failure: PhotoCheckFailure) -> PhotoInspectionFailure {
    match failure {
        PhotoCheckFailure::GridOrCollage => PhotoInspectionFailure::GridOrCollage,
        PhotoCheckFailure::CutoutComposite => PhotoInspectionFailure::CutoutComposite,
        PhotoCheckFailure::NotPhotograph => PhotoInspectionFailure::NotPhotograph,
        PhotoCheckFailure::ModernSetting => PhotoInspectionFailure::ModernSetting,
        PhotoCheckFailure::VisibleText => PhotoInspectionFailure::VisibleText,
        PhotoCheckFailure::ModernClothing => PhotoInspectionFailure::ModernClothing,
        PhotoCheckFailure::PeopleInEmptyRoom => PhotoInspectionFailure::PeopleInEmptyRoom,
        PhotoCheckFailure::CharacterCountNotOne => PhotoInspectionFailure::CharacterCountNotOne,
        PhotoCheckFailure::CharacterHasNoPeople => PhotoInspectionFailure::CharacterHasNoPeople,
        PhotoCheckFailure::UnexpectedDrawnWeapon => PhotoInspectionFailure::UnexpectedDrawnWeapon,
        PhotoCheckFailure::HeroCount { expected, seen } => {
            PhotoInspectionFailure::HeroCount { expected, seen }
        }
    }
}

fn restore_failure(failure: PhotoInspectionFailure) -> PhotoCheckFailure {
    match failure {
        PhotoInspectionFailure::GridOrCollage => PhotoCheckFailure::GridOrCollage,
        PhotoInspectionFailure::CutoutComposite => PhotoCheckFailure::CutoutComposite,
        PhotoInspectionFailure::NotPhotograph => PhotoCheckFailure::NotPhotograph,
        PhotoInspectionFailure::ModernSetting => PhotoCheckFailure::ModernSetting,
        PhotoInspectionFailure::VisibleText => PhotoCheckFailure::VisibleText,
        PhotoInspectionFailure::ModernClothing => PhotoCheckFailure::ModernClothing,
        PhotoInspectionFailure::PeopleInEmptyRoom => PhotoCheckFailure::PeopleInEmptyRoom,
        PhotoInspectionFailure::CharacterCountNotOne => PhotoCheckFailure::CharacterCountNotOne,
        PhotoInspectionFailure::CharacterHasNoPeople => PhotoCheckFailure::CharacterHasNoPeople,
        PhotoInspectionFailure::UnexpectedDrawnWeapon => PhotoCheckFailure::UnexpectedDrawnWeapon,
        PhotoInspectionFailure::HeroCount { expected, seen } => {
            PhotoCheckFailure::HeroCount { expected, seen }
        }
    }
}

fn project_reason(reason: UnjudgedReason) -> PhotoInspectionUnjudgedReason {
    match reason {
        UnjudgedReason::Preparation => PhotoInspectionUnjudgedReason::Preparation,
        UnjudgedReason::Transport => PhotoInspectionUnjudgedReason::Transport,
        UnjudgedReason::Provider => PhotoInspectionUnjudgedReason::Provider,
        UnjudgedReason::InvalidResponse => PhotoInspectionUnjudgedReason::InvalidResponse,
        UnjudgedReason::Deadline => PhotoInspectionUnjudgedReason::Deadline,
        UnjudgedReason::Disabled => PhotoInspectionUnjudgedReason::Disabled,
        UnjudgedReason::Cancelled => PhotoInspectionUnjudgedReason::Cancelled,
    }
}

fn restore_reason(reason: PhotoInspectionUnjudgedReason) -> UnjudgedReason {
    match reason {
        PhotoInspectionUnjudgedReason::Preparation => UnjudgedReason::Preparation,
        PhotoInspectionUnjudgedReason::Transport => UnjudgedReason::Transport,
        PhotoInspectionUnjudgedReason::Provider => UnjudgedReason::Provider,
        PhotoInspectionUnjudgedReason::InvalidResponse => UnjudgedReason::InvalidResponse,
        PhotoInspectionUnjudgedReason::Deadline => UnjudgedReason::Deadline,
        PhotoInspectionUnjudgedReason::Disabled => UnjudgedReason::Disabled,
        PhotoInspectionUnjudgedReason::Cancelled => UnjudgedReason::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InspectionKind, backends::analysis::testing};
    #[test]
    fn every_closed_photo_failure_and_unjudged_reason_roundtrips_neutral_schema() {
        let failures = [
            PhotoCheckFailure::GridOrCollage,
            PhotoCheckFailure::CutoutComposite,
            PhotoCheckFailure::NotPhotograph,
            PhotoCheckFailure::ModernSetting,
            PhotoCheckFailure::VisibleText,
            PhotoCheckFailure::ModernClothing,
            PhotoCheckFailure::PeopleInEmptyRoom,
            PhotoCheckFailure::CharacterCountNotOne,
            PhotoCheckFailure::CharacterHasNoPeople,
            PhotoCheckFailure::HeroCount {
                expected: 2,
                seen: 3,
            },
            PhotoCheckFailure::UnexpectedDrawnWeapon,
        ];
        for failure in failures {
            let kind = match failure {
                PhotoCheckFailure::PeopleInEmptyRoom => InspectionKind::CanonicalRoom,
                PhotoCheckFailure::CharacterCountNotOne
                | PhotoCheckFailure::CharacterHasNoPeople => InspectionKind::CanonicalCharacter,
                _ => InspectionKind::ScenePlate,
            };
            let spec = testing::spec(
                kind,
                matches!(
                    failure,
                    PhotoCheckFailure::HeroCount { .. } | PhotoCheckFailure::UnexpectedDrawnWeapon
                ),
            );
            let assessment = InspectionAssessment {
                contract_revision: 1,
                verdict: InspectionVerdict::Failed(failure),
                visible_features: Vec::new(),
                source_binding: None,
            };
            assert_eq!(
                restore_photo_assessment(&project_photo_assessment(&assessment), &spec).unwrap(),
                assessment
            );
        }
        let reasons = [
            UnjudgedReason::Preparation,
            UnjudgedReason::Transport,
            UnjudgedReason::Provider,
            UnjudgedReason::InvalidResponse,
            UnjudgedReason::Deadline,
            UnjudgedReason::Disabled,
            UnjudgedReason::Cancelled,
        ];
        let spec = testing::spec(InspectionKind::ScenePlate, false);
        for reason in reasons {
            let assessment = InspectionAssessment::unjudged(&spec, reason);
            assert_eq!(
                restore_photo_assessment(&project_photo_assessment(&assessment), &spec).unwrap(),
                assessment
            );
        }
    }
    #[test]
    fn neutral_photo_replay_refuses_inconsistent_features_and_revision() {
        let spec = testing::spec(InspectionKind::ScenePlate, false);
        let raw = InspectionAssessment::unjudged(&spec, UnjudgedReason::InvalidResponse);
        let mut neutral = project_photo_assessment(&raw);
        neutral.contract_revision = 2;
        assert!(restore_photo_assessment(&neutral, &spec).is_err());
        neutral.contract_revision = 1;
        neutral.visible_features.push(PhotoVisibleFeature {
            name: PrivateImageText::new("private feature"),
            look: PrivateImageText::new("private look"),
            region: [0, 0, 100, 100],
            recorded_id: None,
        });
        assert!(restore_photo_assessment(&neutral, &spec).is_err());
        assert!(!format!("{neutral:?}").contains("private feature"));
    }
}
