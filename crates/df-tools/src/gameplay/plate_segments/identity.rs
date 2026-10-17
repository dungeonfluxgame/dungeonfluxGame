//! Which SAM person is which cast member or hero.
//!
//! Two sources, in order. The head locator (a vision model that was already asked for each
//! person's head with their pinned reference portrait) has boxes: a SAM head that overlaps a
//! located box enough (intersection over union) takes that identity, with no new call. Where the
//! locator has nothing, a vision model is shown the pinned reference portraits and the numbered
//! candidate crops and picks, for each portrait, the candidate that shows the same person, or
//! `0` for none. A candidate claimed by two portraits is given to neither. Identity is never
//! decided by a name, a word or a position.

use df_providers::HeadBox;
use serde_json::{Value, json};

use super::{
    JudgeRequest, ReadError,
    geometry::{SegmentedPerson, intersection_over_union},
    reference_label,
};

/// A located head box counts as the same head as a SAM head from this overlap up. SAM's head
/// box runs to the chin, neck or hood and the locator's is hair to chin, so they agree loosely
/// (the research measured 0.73 to 0.87 against a hand-drawn box).
const MINIMUM_LOCATOR_OVERLAP: f32 = 0.3;
/// The vision judge sees at most this many portraits and candidates in one call.
pub(in crate::gameplay) const MAXIMUM_JUDGED: usize = 8;

/// A person the head locator found, by the plain id the locator used (`npc-0`, `hero-1`).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct LocatedPerson {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) head: HeadBox,
}

/// A located person matched to the SAM candidate with their head.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct LocatorMatch {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) candidate: usize,
    pub(in crate::gameplay) overlap: f32,
}

/// Matches each located head to the SAM person whose head overlaps it most, best overlaps first,
/// each located person and each candidate used once. People with no paired head cannot match.
pub(in crate::gameplay) fn match_locator(
    people: &[SegmentedPerson],
    located: &[LocatedPerson],
) -> Vec<LocatorMatch> {
    let mut candidates: Vec<(f32, usize, usize)> = Vec::new();
    for (located_index, known) in located.iter().enumerate() {
        for person in people {
            let Some(head) = person.head.as_ref() else {
                continue;
            };
            let overlap = intersection_over_union(&known.head, head);
            if overlap >= MINIMUM_LOCATOR_OVERLAP {
                candidates.push((overlap, located_index, person.candidate));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
    let mut matches: Vec<LocatorMatch> = Vec::new();
    for (overlap, located_index, candidate) in candidates {
        let id = &located[located_index].id;
        if matches
            .iter()
            .all(|taken| taken.candidate != candidate && &taken.id != id)
        {
            matches.push(LocatorMatch {
                id: id.clone(),
                candidate,
                overlap,
            });
        }
    }
    matches.sort_by_key(|found| found.candidate);
    matches
}

/// The question for the vision judge: `references` portraits (labelled `ref-1` ...) and
/// `candidates` numbered crops (`cand-1` ...). `None` when either count is zero or over
/// [`MAXIMUM_JUDGED`].
pub(in crate::gameplay) fn identity_request(
    references: usize,
    candidates: usize,
) -> Option<JudgeRequest> {
    if references == 0
        || candidates == 0
        || references > MAXIMUM_JUDGED
        || candidates > MAXIMUM_JUDGED
    {
        return None;
    }
    let reference_labels: Vec<String> = (1..=references).map(reference_label).collect();
    let instruction = format!(
        "The pictures ref-1 to ref-{references} are reference portraits of the people we expect \
         in a film still. The pictures cand-1 to cand-{candidates} are crops of the people found \
         in that still, one person each. For each reference portrait, answer the number of the \
         candidate that shows the same person, matching face, hair and costume only, never a \
         name, a role or where the candidate stands. Answer 0 when no candidate is that person \
         or you cannot tell. A candidate can be the answer for at most one portrait."
    );
    let numbers: Vec<usize> = (0..=candidates).collect();
    let schema = json!({
        "type": "object",
        "properties": {
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "reference": {"type": "string", "enum": reference_labels},
                        "candidate": {"type": "integer", "enum": numbers},
                    },
                    "required": ["reference", "candidate"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["matches"],
        "additionalProperties": false,
    });
    Some(JudgeRequest {
        instruction,
        schema_name: "plate-identity",
        schema,
    })
}

/// The judge's answer for each reference portrait, in portrait order: `Some(candidate)` or
/// `None`. Every portrait must be answered exactly once with a number the question offered; a
/// candidate chosen for two portraits is given to neither (`conflicts` counts those candidates).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct IdentityAnswer {
    pub(in crate::gameplay) assignments: Vec<Option<usize>>,
    pub(in crate::gameplay) conflicts: usize,
}

pub(in crate::gameplay) fn read_identity(
    answer: &Value,
    references: usize,
    candidates: usize,
) -> Result<IdentityAnswer, ReadError> {
    let entries = answer
        .get("matches")
        .and_then(Value::as_array)
        .ok_or(ReadError::Shape)?;
    let mut chosen: Vec<Option<usize>> = vec![None; references];
    let mut answered = vec![false; references];
    for entry in entries {
        let label = entry
            .get("reference")
            .and_then(Value::as_str)
            .ok_or(ReadError::Shape)?;
        let number = entry
            .get("candidate")
            .and_then(Value::as_u64)
            .ok_or(ReadError::Shape)? as usize;
        let slot = (1..=references)
            .find(|index| reference_label(*index) == label)
            .ok_or(ReadError::OutsideChoices)?
            - 1;
        if number > candidates {
            return Err(ReadError::OutsideChoices);
        }
        if answered[slot] {
            return Err(ReadError::Incomplete);
        }
        answered[slot] = true;
        chosen[slot] = (number > 0).then_some(number);
    }
    if answered.iter().any(|done| !done) {
        return Err(ReadError::Incomplete);
    }
    let mut conflicts = 0;
    for number in 1..=candidates {
        let claimants = chosen.iter().filter(|pick| **pick == Some(number)).count();
        if claimants > 1 {
            conflicts += 1;
            for pick in chosen.iter_mut().filter(|pick| **pick == Some(number)) {
                *pick = None;
            }
        }
    }
    Ok(IdentityAnswer {
        assignments: chosen,
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{people_from_segments, tests::instance};
    use super::*;

    fn people() -> Vec<SegmentedPerson> {
        people_from_segments(
            &[
                instance(0.95, 20, 20, 80, 100),
                instance(0.9, 120, 20, 180, 100),
            ],
            &[
                instance(0.9, 35, 20, 60, 40),
                instance(0.9, 135, 20, 160, 40),
            ],
        )
    }

    fn located(id: &str, left: f32, top: f32, right: f32, bottom: f32) -> LocatedPerson {
        LocatedPerson {
            id: id.to_owned(),
            head: HeadBox {
                left,
                top,
                right,
                bottom,
            },
        }
    }

    #[test]
    fn located_heads_take_the_sam_head_they_overlap() {
        // The SAM heads are (0.175..0.3, 0.2..0.4) and (0.675..0.8, 0.2..0.4).
        let found = match_locator(
            &people(),
            &[
                located("npc-0", 0.68, 0.18, 0.81, 0.40),
                located("hero-1", 0.17, 0.2, 0.31, 0.42),
            ],
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].id.as_str(), found[0].candidate), ("hero-1", 1));
        assert_eq!((found[1].id.as_str(), found[1].candidate), ("npc-0", 2));
        assert!(found.iter().all(|found| found.overlap > 0.6));
    }

    #[test]
    fn a_located_head_over_nobody_matches_nothing_and_a_candidate_is_used_once() {
        let found = match_locator(
            &people(),
            &[
                located("npc-0", 0.4, 0.5, 0.5, 0.7),
                located("npc-1", 0.17, 0.2, 0.31, 0.42),
                located("npc-2", 0.18, 0.21, 0.30, 0.40),
            ],
        );
        // npc-2 overlaps the first head better than npc-1 and takes it; npc-1 is left out.
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].candidate), ("npc-2", 1));
    }

    #[test]
    fn a_person_without_a_head_cannot_be_matched_by_the_locator() {
        let headless = people_from_segments(&[instance(0.95, 20, 20, 80, 100)], &[]);
        assert!(match_locator(&headless, &[located("npc-0", 0.1, 0.2, 0.4, 0.4)]).is_empty());
    }

    #[test]
    fn the_identity_question_offers_only_the_numbers_and_portraits_it_was_given() {
        let request = identity_request(2, 3).unwrap();
        let items = &request.schema["properties"]["matches"]["items"]["properties"];
        assert_eq!(items["reference"]["enum"], json!(["ref-1", "ref-2"]));
        assert_eq!(items["candidate"]["enum"], json!([0, 1, 2, 3]));
        assert!(request.instruction.contains("never a name"));
        assert!(identity_request(0, 3).is_none());
        assert!(identity_request(2, 0).is_none());
        assert!(identity_request(9, 3).is_none());
        assert!(identity_request(2, 9).is_none());
    }

    #[test]
    fn an_answer_gives_each_portrait_a_candidate_or_none() {
        let answer = read_identity(
            &json!({"matches": [
                {"reference": "ref-2", "candidate": 0},
                {"reference": "ref-1", "candidate": 3},
            ]}),
            2,
            3,
        )
        .unwrap();
        assert_eq!(answer.assignments, vec![Some(3), None]);
        assert_eq!(answer.conflicts, 0);
    }

    #[test]
    fn a_candidate_claimed_twice_goes_to_neither_portrait() {
        let answer = read_identity(
            &json!({"matches": [
                {"reference": "ref-1", "candidate": 2},
                {"reference": "ref-2", "candidate": 2},
                {"reference": "ref-3", "candidate": 1},
            ]}),
            3,
            3,
        )
        .unwrap();
        assert_eq!(answer.assignments, vec![None, None, Some(1)]);
        assert_eq!(answer.conflicts, 1);
    }

    #[test]
    fn an_answer_outside_what_was_offered_is_refused() {
        for (answer, expected) in [
            (json!({}), ReadError::Shape),
            (
                json!({"matches": [{"reference": "ref-1"}]}),
                ReadError::Shape,
            ),
            (
                json!({"matches": [{"reference": "ref-9", "candidate": 1}]}),
                ReadError::OutsideChoices,
            ),
            (
                json!({"matches": [{"reference": "Marek", "candidate": 1}]}),
                ReadError::OutsideChoices,
            ),
            (
                json!({"matches": [{"reference": "ref-1", "candidate": 4}]}),
                ReadError::OutsideChoices,
            ),
            (
                json!({"matches": [{"reference": "ref-1", "candidate": 1}]}),
                ReadError::Incomplete,
            ),
            (
                json!({"matches": [
                    {"reference": "ref-1", "candidate": 1},
                    {"reference": "ref-1", "candidate": 2},
                ]}),
                ReadError::Incomplete,
            ),
        ] {
            assert_eq!(read_identity(&answer, 2, 3), Err(expected), "{answer}");
        }
    }
}
