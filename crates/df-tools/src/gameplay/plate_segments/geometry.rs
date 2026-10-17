//! SAM instances to people: tight boxes, the head paired to each person, and the loader anchor.
//!
//! Boxes are fractions of the plate (`left < right`, `top < bottom`, each 0 to 1), the type the
//! head locator already returns. Heads and people are paired by how much of the head's mask lies
//! inside the person's mask: an exact overlap of pixels, never a position or a word.

use df_providers::{HeadBox, MaskRows, PixelBox, SegmentInstance};

/// SAM instances scoring under this are not used (fal's scores on real plates run 0.87 to 0.97
/// for people and 0.78 to 0.94 for heads).
pub(in crate::gameplay) const MINIMUM_SCORE: f32 = 0.5;
/// A head belongs to a person when at least this share of the head's pixels lie inside the
/// person's mask.
const HEAD_INSIDE_PERSON: f64 = 0.6;
/// The loader anchor is read from the top this share of the head's height.
const ANCHOR_BAND: f32 = 0.08;
const BODY_ANCHOR_BAND: f32 = 0.03;

pub(in crate::gameplay) fn box_width(found: &HeadBox) -> f32 {
    found.right - found.left
}

pub(in crate::gameplay) fn box_height(found: &HeadBox) -> f32 {
    found.bottom - found.top
}

pub(in crate::gameplay) fn centre(found: &HeadBox) -> (f32, f32) {
    (
        (found.left + found.right) / 2.0,
        (found.top + found.bottom) / 2.0,
    )
}

/// Intersection over union of two boxes; 0 when they do not touch.
pub(in crate::gameplay) fn intersection_over_union(first: &HeadBox, second: &HeadBox) -> f32 {
    let width = (first.right.min(second.right) - first.left.max(second.left)).max(0.0);
    let height = (first.bottom.min(second.bottom) - first.top.max(second.top)).max(0.0);
    let shared = width * height;
    let union =
        box_width(first) * box_height(first) + box_width(second) * box_height(second) - shared;
    if union <= 0.0 { 0.0 } else { shared / union }
}

/// Where the loader stands: just above the crown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::gameplay) struct Anchor {
    pub(in crate::gameplay) x: f32,
    pub(in crate::gameplay) y: f32,
    pub(in crate::gameplay) source: AnchorSource,
}

/// What the anchor was read from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum AnchorSource {
    /// The top of the person's own head mask.
    HeadMask,
    /// No head was paired; the top of the body mask.
    BodyTop,
}

impl AnchorSource {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::HeadMask => "head_mask",
            Self::BodyTop => "body_top",
        }
    }
}

/// One person SAM found on the plate.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct SegmentedPerson {
    /// The number the vision judge sees (counted from 1, left to right across the plate).
    pub(in crate::gameplay) candidate: usize,
    pub(in crate::gameplay) score: f32,
    /// The tight box of the person's mask. A person cut off by an occluder is a partial figure.
    pub(in crate::gameplay) body: HeadBox,
    /// The tight box of the person's own head mask, when a head was paired.
    pub(in crate::gameplay) head: Option<HeadBox>,
    pub(in crate::gameplay) head_score: Option<f32>,
    pub(in crate::gameplay) anchor: Anchor,
    /// The person's mask as a share of the plate (a depth cue).
    pub(in crate::gameplay) area_share: f32,
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

fn anchor_of(mask: &MaskRows, band: f32, source: AnchorSource) -> Option<Anchor> {
    let bounds = mask.bounds()?;
    let rows = ((bounds.bottom - bounds.top) as f32 * band)
        .round()
        .max(1.0) as u32;
    let (x, top) = mask.top_centre(rows)?;
    Some(Anchor {
        x: x / mask.width() as f32,
        y: top as f32 / mask.height() as f32,
        source,
    })
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
pub(in crate::gameplay) fn people_from_segments(
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
            let anchor = match head {
                Some(head) => anchor_of(&head.mask, ANCHOR_BAND, AnchorSource::HeadMask),
                None => None,
            }
            .or_else(|| anchor_of(&person.mask, BODY_ANCHOR_BAND, AnchorSource::BodyTop))?;
            let plate = f64::from(person.mask.width()) * f64::from(person.mask.height());
            Some(SegmentedPerson {
                candidate: 0,
                score: person.score,
                body,
                head: head_box,
                head_score: head.map(|head| head.score),
                anchor,
                area_share: (person.mask.pixel_count() as f64 / plate) as f32,
            })
        })
        .collect();
    found.sort_by(|a, b| centre(&a.body).0.total_cmp(&centre(&b.body).0));
    for (index, person) in found.iter_mut().enumerate() {
        person.candidate = index + 1;
    }
    found
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use df_providers::MaskRun;

    /// A filled rectangle mask over a 200 by 100 picture, as a SAM instance.
    pub(in crate::gameplay) fn instance(
        score: f32,
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
    ) -> SegmentInstance {
        let runs = (top..bottom)
            .map(|y| MaskRun {
                start: y * 200 + left,
                length: right - left,
            })
            .collect();
        SegmentInstance {
            index: 0,
            score,
            mask: MaskRows::from_runs(200, 100, runs).unwrap(),
        }
    }

    #[test]
    fn heads_are_paired_to_the_people_that_hold_them_and_candidates_run_left_to_right() {
        // The right-hand person is listed first; people come back in left-to-right order.
        let people = [
            instance(0.95, 120, 20, 180, 100),
            instance(0.9, 20, 30, 70, 100),
        ];
        let heads = [
            instance(0.9, 135, 20, 160, 40),
            instance(0.85, 30, 30, 55, 50),
        ];
        let found = people_from_segments(&people, &heads);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].candidate, 1);
        assert_eq!(found[1].candidate, 2);
        let left_head = found[0].head.unwrap();
        assert_eq!((left_head.left, left_head.top), (0.15, 0.3));
        assert_eq!(found[0].head_score, Some(0.85));
        let right_head = found[1].head.unwrap();
        assert_eq!((right_head.left, right_head.right), (0.675, 0.8));
        assert_eq!(found[1].body.left, 0.6);
    }

    #[test]
    fn the_loader_anchor_is_the_top_of_the_head_mask() {
        let found = people_from_segments(
            &[instance(0.95, 20, 20, 80, 100)],
            &[instance(0.9, 40, 20, 60, 40)],
        );
        let anchor = found[0].anchor;
        assert_eq!(anchor.source, AnchorSource::HeadMask);
        assert!((anchor.x - 0.25).abs() < 0.001, "{}", anchor.x);
        assert_eq!(anchor.y, 0.2);
    }

    #[test]
    fn a_person_without_a_paired_head_anchors_at_the_top_of_the_body_and_has_no_head_box() {
        // The head lies outside the person's mask.
        let found = people_from_segments(
            &[instance(0.95, 20, 20, 80, 100)],
            &[instance(0.9, 150, 20, 170, 40)],
        );
        assert_eq!(found[0].head, None);
        assert_eq!(found[0].anchor.source, AnchorSource::BodyTop);
        assert_eq!(found[0].anchor.y, 0.2);
    }

    #[test]
    fn a_head_goes_to_the_person_that_holds_most_of_it_and_only_one_person_takes_it() {
        // One head lies inside both people (85% of it inside the first, all of it inside the
        // second, the nearer person): the second takes it and the first goes without.
        let people = [
            instance(0.95, 20, 20, 62, 100),
            instance(0.9, 40, 25, 110, 100),
        ];
        let heads = [instance(0.9, 45, 25, 65, 45)];
        let found = people_from_segments(&people, &heads);
        assert_eq!(
            found.iter().filter(|person| person.head.is_some()).count(),
            1
        );
        assert!(found[0].head.is_none());
        assert!(found[1].head.is_some());
    }

    #[test]
    fn low_scores_and_empty_masks_are_not_people() {
        let empty = SegmentInstance {
            index: 0,
            score: 0.99,
            mask: MaskRows::parse("", 200, 100).unwrap(),
        };
        let found = people_from_segments(
            &[
                instance(0.4, 20, 20, 80, 100),
                empty,
                instance(0.9, 100, 20, 150, 100),
            ],
            &[],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].candidate, 1);
        assert_eq!(found[0].body.left, 0.5);
    }

    #[test]
    fn intersection_over_union_is_zero_apart_and_one_for_the_same_box() {
        let one = HeadBox {
            left: 0.1,
            top: 0.1,
            right: 0.3,
            bottom: 0.3,
        };
        let other = HeadBox {
            left: 0.2,
            top: 0.1,
            right: 0.4,
            bottom: 0.3,
        };
        let far = HeadBox {
            left: 0.6,
            top: 0.6,
            right: 0.8,
            bottom: 0.8,
        };
        assert!((intersection_over_union(&one, &one) - 1.0).abs() < 1e-6);
        assert!((intersection_over_union(&one, &other) - 1.0 / 3.0).abs() < 1e-5);
        assert_eq!(intersection_over_union(&one, &far), 0.0);
    }
}
