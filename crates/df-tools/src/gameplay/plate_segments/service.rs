//! Plate staging consumes actual durable analysis operations; no provider, paid queue or cache.
//! Geometry, identity projection and audience-safe audit facts stay with gameplay.

use super::{
    facing::Facing,
    geometry::{SegmentedPerson, people_from_segments},
    identity::match_locator,
};
use crate::gameplay::imagery::{ImageFailure, ImageKey, Imagery, ReferenceImage, analysis};
use df_imagery::{
    AnalysisFacingDirection, AnalysisIdentityMatch, OperationHandle, SegmentationPreparation,
    SegmentationPrompt,
};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct AnalysedPerson {
    pub(in crate::gameplay) who: Option<String>,
    pub(in crate::gameplay) person: SegmentedPerson,
    pub(in crate::gameplay) facing: Option<Facing>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct Analysis {
    pub(in crate::gameplay) image_key: String,
    pub(in crate::gameplay) picture: (u32, u32),
    pub(in crate::gameplay) people: Vec<AnalysedPerson>,
    pub(in crate::gameplay) sam_micro_dollars: Option<u64>,
}

/// Inputs retain actual catalog references and the optional completed head-operation handle.
pub(in crate::gameplay) struct AnalysisInput {
    pub(in crate::gameplay) image_key: ImageKey,
    pub(in crate::gameplay) references: Vec<(String, ReferenceImage)>,
    pub(in crate::gameplay) locations: Option<OperationHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct AnalysisFailure(pub(in crate::gameplay) &'static str);

#[derive(Clone)]
pub(in crate::gameplay) struct Segmentation {
    imagery: Imagery,
}

impl Segmentation {
    pub(in crate::gameplay) fn new(imagery: Imagery) -> Self {
        Self { imagery }
    }

    /// The same service owns inference, replay, uncertainty, cancellation and actual usage.
    pub(in crate::gameplay) async fn analyse(
        &self,
        input: AnalysisInput,
    ) -> Result<Analysis, AnalysisFailure> {
        self.analyse_inner(input)
            .await
            .map_err(|error| AnalysisFailure(error.label()))
    }

    async fn analyse_inner(&self, input: AnalysisInput) -> Result<Analysis, ImageFailure> {
        let started = Instant::now();
        let segments = analysis::segments(
            &self.imagery,
            &input.image_key,
            SegmentationPreparation::ScenePlate,
            vec![SegmentationPrompt::Person, SegmentationPrompt::Head],
            16,
        )
        .await?;
        let person = unique_set(&segments.value, SegmentationPrompt::Person)?;
        let head = unique_set(&segments.value, SegmentationPrompt::Head)?;
        if segments.value.len() != 2
            || (person.width_px, person.height_px) != (head.width_px, head.height_px)
        {
            return Err(ImageFailure::BadOutput);
        }
        let picture = (person.width_px, person.height_px);
        let people =
            people_from_segments(&analysis::instances(person)?, &analysis::instances(head)?);
        let context = self.imagery.consumer_context().await?;
        let identity = if !people.is_empty() && context.config.analysis.staging_identity.is_some() {
            Some(
                analysis::identity(
                    &self.imagery,
                    &segments,
                    input.locations.as_ref(),
                    &input.references,
                )
                .await?,
            )
        } else {
            None
        };
        let who = match &identity {
            Some(identity) => assignments(&people, &identity.value)?,
            None => {
                let located = match input.locations.as_ref() {
                    Some(handle) => {
                        analysis::locations(&self.imagery, handle, &segments.source).await?
                    }
                    None => Vec::new(),
                };
                let matches = match_locator(&people, &located);
                people
                    .iter()
                    .map(|person| {
                        matches
                            .iter()
                            .find(|found| found.candidate == person.candidate)
                            .map(|found| found.id.clone())
                    })
                    .collect()
            }
        };
        let facings = match identity.as_ref() {
            Some(identity) if context.config.analysis.staging_facing.is_some() => {
                analysis::facing(&self.imagery, &segments, identity)
                    .await?
                    .value
            }
            _ => Vec::new(),
        };
        let mut analysed = Vec::with_capacity(people.len());
        for (person, who) in people.into_iter().zip(who) {
            let candidate = u32::try_from(person.candidate).map_err(|_| ImageFailure::BadOutput)?;
            let mut matches = facings.iter().filter(|found| found.candidate == candidate);
            let facing = matches
                .next()
                .and_then(|found| found.facing)
                .map(|facing| match facing {
                    AnalysisFacingDirection::Left => Facing::Left,
                    AnalysisFacingDirection::Right => Facing::Right,
                    AnalysisFacingDirection::TowardCamera => Facing::TowardCamera,
                    AnalysisFacingDirection::AwayFromCamera => Facing::AwayFromCamera,
                });
            if matches.next().is_some() {
                return Err(ImageFailure::BadOutput);
            }
            analysed.push(AnalysedPerson {
                who,
                person,
                facing,
            });
        }
        if facings.iter().any(|facing| {
            !analysed
                .iter()
                .any(|person| u32::try_from(person.person.candidate).ok() == Some(facing.candidate))
        }) {
            return Err(ImageFailure::BadOutput);
        }
        let result = Analysis {
            image_key: input.image_key.as_str().to_owned(),
            picture,
            people: analysed,
            sam_micro_dollars: segments.usage.as_ref().and_then(known_usd_cost),
        };
        crate::gameplay::actor::record_audit_line(
            "gameplay.plate_staging",
            format!(
                "PLATE-STAGING image={} people={} identified={} facing={} usd_micro={} ms={}",
                result.image_key,
                result.people.len(),
                result
                    .people
                    .iter()
                    .filter(|person| person.who.is_some())
                    .count(),
                result
                    .people
                    .iter()
                    .filter(|person| person.facing.is_some())
                    .count(),
                result
                    .sam_micro_dollars
                    .map_or_else(|| "unknown".into(), |value| value.to_string()),
                started.elapsed().as_millis()
            ),
        );
        Ok(result)
    }
}

fn unique_set(
    sets: &[df_imagery::AnalysisMaskSet],
    prompt: SegmentationPrompt,
) -> Result<&df_imagery::AnalysisMaskSet, ImageFailure> {
    let mut matches = sets.iter().filter(|set| set.prompt == prompt);
    let set = matches.next().ok_or(ImageFailure::BadOutput)?;
    if matches.next().is_some() {
        return Err(ImageFailure::BadOutput);
    }
    Ok(set)
}

fn assignments(
    people: &[SegmentedPerson],
    matches: &[AnalysisIdentityMatch],
) -> Result<Vec<Option<String>>, ImageFailure> {
    let mut who = vec![None; people.len()];
    for (index, found) in matches.iter().enumerate() {
        if matches[..index]
            .iter()
            .any(|other| other.reference_id == found.reference_id)
        {
            return Err(ImageFailure::BadOutput);
        }
        let Some(candidate) = found.candidate else {
            continue;
        };
        let slot = people
            .iter()
            .position(|person| u32::try_from(person.candidate).ok() == Some(candidate))
            .ok_or(ImageFailure::BadOutput)?;
        if who[slot].is_some() {
            return Err(ImageFailure::BadOutput);
        }
        who[slot] = Some(found.reference_id.text().to_owned());
    }
    Ok(who)
}

fn known_usd_cost(summary: &df_imagery::runtime::ImageUsageSummary) -> Option<u64> {
    use df_imagery::runtime::{ImageUsageAmount, StageUsageObservation};
    let mut stages = summary
        .stages
        .iter()
        .filter(|stage| stage.amount != ImageUsageAmount::NotApplicable)
        .peekable();
    stages.peek()?;
    let total = stages.try_fold(0u128, |total, stage| {
        if !matches!(stage.observation, StageUsageObservation::Recorded(_)) {
            return None;
        }
        match stage.amount {
            ImageUsageAmount::Known(amount) if amount.currency().as_str() == "USD" => {
                total.checked_add(amount.micros())
            }
            _ => None,
        }
    })?;
    // Larger authoritative values remain unknown in this legacy u64 USD view.
    u64::try_from(total).ok()
}

/// Digest utility retained for current pure projection callers; never an authority token.
pub(in crate::gameplay) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{people_from_segments, tests::instance};
    use super::*;
    use df_imagery::PrivateImageText;

    #[test]
    fn typed_assignments_preserve_candidate_numbers_and_reject_foreign_or_duplicate_claims() {
        let people = people_from_segments(
            &[
                instance(0.95, 20, 20, 80, 100),
                instance(0.9, 120, 20, 180, 100),
            ],
            &[],
        );
        let matched = |id: &str, candidate| AnalysisIdentityMatch {
            reference_id: PrivateImageText::new(id),
            candidate,
        };
        assert_eq!(
            assignments(
                &people,
                &[matched("hero-1", Some(2)), matched("npc-0", None)]
            )
            .unwrap(),
            vec![None, Some("hero-1".into())]
        );
        assert_eq!(
            assignments(&people, &[matched("hero-1", Some(3))]),
            Err(ImageFailure::BadOutput)
        );
        assert_eq!(
            assignments(
                &people,
                &[matched("hero-1", Some(1)), matched("npc-0", Some(1))]
            ),
            Err(ImageFailure::BadOutput)
        );
    }

    #[test]
    fn absent_segmentation_usage_stays_unknown_instead_of_becoming_zero() {
        let summary = df_imagery::runtime::ImageUsageSummary {
            job: df_imagery::ImageJobId::from_bytes(&[1; 16]).unwrap(),
            stages: vec![],
        };
        assert_eq!(known_usd_cost(&summary), None);
        assert_eq!(
            serde_json::json!({"micro_dollars": known_usd_cost(&summary)})["micro_dollars"],
            serde_json::Value::Null
        );
    }
}
