//! Exact legacy identity and facing questions and closed answer policies.
use super::{Question, ReadError, reference_label};
use serde_json::{Value, json};
const IDENTITY_INSTRUCTION: &str = concat!(
    "The pictures ref-1 to ref-{references} are reference portraits of the people we expect in a film still. ",
    "The pictures cand-1 to cand-{candidates} are crops of the people found in that still, one person each. ",
    "For each reference portrait, answer the number of the candidate that shows the same person, matching face, hair and costume only, never a name, a role or where the candidate stands. ",
    "Answer 0 when no candidate is that person or you cannot tell. ",
    "A candidate can be the answer for at most one portrait.",
);
const FACING_INSTRUCTION: &str = concat!(
    "Each picture is a crop of one person from a film still, labelled cand-N. ",
    "For each, answer which way the front of the person's body points as seen in the picture: left means toward the left edge of the picture, right toward the right edge, camera means facing the viewer, away means the back is turned to the viewer. ",
    "A person in profile or three-quarter view faces the side their face or chest points to. ",
    "Answer unclear when the crop does not show enough to tell. ",
    "Judge only what the picture shows; ignore names and roles.",
);
const MAXIMUM_IDENTITY_JUDGED: usize = 8;
const MAXIMUM_FACING_JUDGED: usize = 16;
pub(super) fn identity_request(references: usize, candidates: usize) -> Option<Question> {
    if references == 0
        || candidates == 0
        || references > MAXIMUM_IDENTITY_JUDGED
        || candidates > MAXIMUM_IDENTITY_JUDGED
    {
        return None;
    }
    let reference_labels: Vec<String> = (1..=references).map(reference_label).collect();
    let instruction = IDENTITY_INSTRUCTION
        .replace("{references}", &references.to_string())
        .replace("{candidates}", &candidates.to_string());
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
    Some(Question {
        instruction,
        schema_name: "plate-identity",
        schema,
    })
}

/// The judge's answer for each reference portrait, in portrait order: `Some(candidate)` or
/// `None`. Every portrait must be answered exactly once with a number the question offered; a
/// candidate chosen for two portraits is given to neither.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IdentityAnswer {
    pub(super) assignments: Vec<Option<usize>>,
}

pub(super) fn read_identity(
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
            .and_then(|number| usize::try_from(number).ok())
            .ok_or(ReadError::Shape)?;
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
    for number in 1..=candidates {
        let claimants = chosen.iter().filter(|pick| **pick == Some(number)).count();
        if claimants > 1 {
            for pick in chosen.iter_mut().filter(|pick| **pick == Some(number)) {
                *pick = None;
            }
        }
    }
    Ok(IdentityAnswer {
        assignments: chosen,
    })
}

/// Which way the front of a person's body points on screen.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Facing {
    /// Toward the left edge of the picture.
    Left,
    /// Toward the right edge of the picture.
    Right,
    /// Toward the viewer.
    TowardCamera,
    /// Back to the viewer.
    AwayFromCamera,
}

impl Facing {
    fn from_label(label: &str) -> Result<Option<Self>, ReadError> {
        match label {
            "left" => Ok(Some(Self::Left)),
            "right" => Ok(Some(Self::Right)),
            "camera" => Ok(Some(Self::TowardCamera)),
            "away" => Ok(Some(Self::AwayFromCamera)),
            UNCLEAR => Ok(None),
            _ => Err(ReadError::OutsideChoices),
        }
    }
}

/// The judge's way out when the crop does not show enough; it is not a [`Facing`].
const UNCLEAR: &str = "unclear";
const LABELS: [&str; 5] = ["left", "right", "camera", "away", UNCLEAR];

/// The question for the judge over the crops of `candidates` (candidate numbers counted from 1,
/// pictures labelled `cand-<n>`). `None` when there are none or more than [`MAXIMUM_FACING_JUDGED`].
pub(super) fn facing_request(candidates: &[usize]) -> Option<Question> {
    if candidates.is_empty() || candidates.len() > MAXIMUM_FACING_JUDGED {
        return None;
    }
    let instruction = FACING_INSTRUCTION.to_owned();
    let schema = json!({
        "type": "object",
        "properties": {
            "facings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "candidate": {"type": "integer", "enum": candidates},
                        "facing": {"type": "string", "enum": LABELS},
                    },
                    "required": ["candidate", "facing"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["facings"],
        "additionalProperties": false,
    });
    Some(Question {
        instruction,
        schema_name: "plate-facing",
        schema,
    })
}

/// The facing of each candidate in `candidates`, in that order (`None` for `unclear`). Every
/// candidate asked about must be answered exactly once, with a label the question offered.
pub(super) fn read_facing(
    answer: &Value,
    candidates: &[usize],
) -> Result<Vec<Option<Facing>>, ReadError> {
    let entries = answer
        .get("facings")
        .and_then(Value::as_array)
        .ok_or(ReadError::Shape)?;
    let mut found: Vec<Option<Option<Facing>>> = vec![None; candidates.len()];
    for entry in entries {
        let number = entry
            .get("candidate")
            .and_then(Value::as_u64)
            .and_then(|number| usize::try_from(number).ok())
            .ok_or(ReadError::Shape)?;
        let facing = Facing::from_label(
            entry
                .get("facing")
                .and_then(Value::as_str)
                .ok_or(ReadError::Shape)?,
        )?;
        let slot = candidates
            .iter()
            .position(|asked| *asked == number)
            .ok_or(ReadError::OutsideChoices)?;
        if found[slot].is_some() {
            return Err(ReadError::Incomplete);
        }
        found[slot] = Some(facing);
    }
    found
        .into_iter()
        .map(|facing| facing.ok_or(ReadError::Incomplete))
        .collect()
}
