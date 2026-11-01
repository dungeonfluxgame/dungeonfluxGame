//! Legacy score filtering, mask overlap pairing, left-to-right numbering and locator matching.
use super::ReadError;
use crate::runtime::RuntimeError;
use crate::service::analysis_types::{AnalysisMaskSet, ImageAnalysisResult, SegmentationPrompt};
use df_providers::{HeadBox, MaskRows, PixelBox, SegmentInstance};
const MINIMUM_SCORE: f32 = 0.5;
const HEAD_INSIDE_PERSON: f64 = 0.6;
const MINIMUM_LOCATOR_OVERLAP: f32 = 0.3;
#[derive(Clone, Debug)]
pub(super) struct SegmentedPerson {
    pub(super) candidate: usize,
    pub(super) body: HeadBox,
    pub(super) head: Option<HeadBox>,
}
fn fractions(found: PixelBox, mask: &MaskRows) -> HeadBox {
    let (width, height) = (mask.width() as f32, mask.height() as f32);
    HeadBox {
        left: found.left as f32 / width,
        top: found.top as f32 / height,
        right: found.right as f32 / width,
        bottom: found.bottom as f32 / height,
    }
}
fn centre(found: &HeadBox) -> (f32, f32) {
    (
        (found.left + found.right) / 2.0,
        (found.top + found.bottom) / 2.0,
    )
}
fn intersection_over_union(a: &HeadBox, b: &HeadBox) -> f32 {
    let shared = (a.right.min(b.right) - a.left.max(b.left)).max(0.0)
        * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.0);
    let union =
        (a.right - a.left) * (a.bottom - a.top) + (b.right - b.left) * (b.bottom - b.top) - shared;
    if union <= 0.0 { 0.0 } else { shared / union }
}
struct Pairing {
    person: usize,
    head: usize,
    inside: f64,
}

/// The people on the plate from the `person` and `head` answers. Instances under
/// [`MINIMUM_SCORE`] and empty masks are dropped; each head is given to at most one person, the
/// person whose mask holds most of it, and each person takes at most one head. People are
/// numbered from 1, left to right.
pub(super) fn people_from_segments(
    people: &[SegmentInstance],
    heads: &[SegmentInstance],
) -> Vec<SegmentedPerson> {
    let people: Vec<&SegmentInstance> = people
        .iter()
        .filter(|found| found.score >= MINIMUM_SCORE && !found.mask.is_empty())
        .collect();
    let heads: Vec<&SegmentInstance> = heads
        .iter()
        .filter(|found| found.score >= MINIMUM_SCORE && !found.mask.is_empty())
        .collect();

    let mut pairings: Vec<Pairing> = Vec::new();
    for (person_index, person) in people.iter().enumerate() {
        for (head_index, head) in heads.iter().enumerate() {
            let inside = person.mask.overlap(&head.mask) as f64 / head.mask.pixel_count() as f64;
            if inside >= HEAD_INSIDE_PERSON {
                pairings.push(Pairing {
                    person: person_index,
                    head: head_index,
                    inside,
                });
            }
        }
    }
    pairings.sort_by(|a, b| {
        b.inside
            .total_cmp(&a.inside)
            .then(heads[b.head].score.total_cmp(&heads[a.head].score))
    });
    let mut head_of: Vec<Option<usize>> = vec![None; people.len()];
    let mut head_taken = vec![false; heads.len()];
    for pairing in pairings {
        if head_of[pairing.person].is_none() && !head_taken[pairing.head] {
            head_of[pairing.person] = Some(pairing.head);
            head_taken[pairing.head] = true;
        }
    }

    let mut found: Vec<SegmentedPerson> = people
        .iter()
        .zip(head_of)
        .filter_map(|(person, head_index)| {
            let body = fractions(person.mask.bounds()?, &person.mask);
            let head = head_index.map(|index| heads[index]);
            let head_box = head.and_then(|head| {
                head.mask
                    .bounds()
                    .map(|bounds| fractions(bounds, &head.mask))
            });
            Some(SegmentedPerson {
                candidate: 0,
                body,
                head: head_box,
            })
        })
        .collect();
    found.sort_by(|a, b| centre(&a.body).0.total_cmp(&centre(&b.body).0));
    for (index, person) in found.iter_mut().enumerate() {
        person.candidate = index + 1;
    }
    found
}

pub(super) fn people(result: &ImageAnalysisResult) -> Result<Vec<SegmentedPerson>, RuntimeError> {
    let ImageAnalysisResult::Segmentation { masks, .. } = result else {
        return Err(RuntimeError::InvalidRecord);
    };
    fn instances(set: &AnalysisMaskSet) -> Result<Vec<SegmentInstance>, RuntimeError> {
        set.validate().map_err(|_| RuntimeError::InvalidRecord)?;
        set.instances
            .iter()
            .map(|mask| {
                Ok(SegmentInstance {
                    index: mask.index,
                    score: mask.score.get(),
                    mask: MaskRows::from_runs(
                        set.width_px,
                        set.height_px,
                        mask.runs
                            .iter()
                            .map(|run| df_providers::MaskRun {
                                start: run.start,
                                length: run.length,
                            })
                            .collect(),
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                })
            })
            .collect()
    }
    let persons = masks
        .iter()
        .find(|set| set.prompt == SegmentationPrompt::Person)
        .ok_or(RuntimeError::InvalidRecord)?;
    let heads = masks
        .iter()
        .find(|set| set.prompt == SegmentationPrompt::Head)
        .ok_or(RuntimeError::InvalidRecord)?;
    if masks.len() != 2
        || (persons.width_px, persons.height_px) != (heads.width_px, heads.height_px)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(people_from_segments(
        &instances(persons)?,
        &instances(heads)?,
    ))
}
pub(super) fn locator_matches(
    people: &[SegmentedPerson],
    located: Option<&ImageAnalysisResult>,
) -> Result<Vec<(String, usize)>, ReadError> {
    let Some(located) = located else {
        return Ok(Vec::new());
    };
    let ImageAnalysisResult::HeadLocations { heads, .. } = located else {
        return Err(ReadError::Shape);
    };
    let mut candidates = Vec::new();
    for (index, known) in heads.iter().enumerate() {
        let [left, top, right, bottom] = known.head.coordinates();
        let known_box = HeadBox {
            left,
            top,
            right,
            bottom,
        };
        for person in people {
            if let Some(head) = &person.head {
                let overlap = intersection_over_union(&known_box, head);
                if overlap >= MINIMUM_LOCATOR_OVERLAP {
                    candidates.push((overlap, index, person.candidate));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
    let mut matches: Vec<(String, usize)> = Vec::new();
    for (_, index, candidate) in candidates {
        let id = heads[index].id.text();
        if matches
            .iter()
            .all(|(taken, number)| *number != candidate && taken != id)
        {
            matches.push((id.to_owned(), candidate));
        }
    }
    matches.sort_by_key(|(_, number)| *number);
    Ok(matches)
}
