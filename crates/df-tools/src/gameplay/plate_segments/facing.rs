//! Which way each person on the plate faces, as the picture shows it.
//!
//! One batched vision call per plate, over the crops of the people who were assigned an
//! identity. The answer is a closed label per person, read from the picture by the judge; the
//! server never infers facing from a name, a role or a position.

use serde_json::{Value, json};

use super::{JudgeRequest, ReadError};

/// The most people one facing call judges.
pub(in crate::gameplay) const MAXIMUM_JUDGED: usize = 16;

/// Which way the front of a person's body points on screen.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::gameplay) enum Facing {
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
    /// The label the judge answers with and the wire and audit lines carry.
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::TowardCamera => "camera",
            Self::AwayFromCamera => "away",
        }
    }

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
/// pictures labelled `cand-<n>`). `None` when there are none or more than [`MAXIMUM_JUDGED`].
pub(in crate::gameplay) fn facing_request(candidates: &[usize]) -> Option<JudgeRequest> {
    if candidates.is_empty() || candidates.len() > MAXIMUM_JUDGED {
        return None;
    }
    let instruction = "Each picture is a crop of one person from a film still, labelled cand-N. \
        For each, answer which way the front of the person's body points as seen in the picture: \
        left means toward the left edge of the picture, right toward the right edge, camera \
        means facing the viewer, away means the back is turned to the viewer. A person in \
        profile or three-quarter view faces the side their face or chest points to. Answer \
        unclear when the crop does not show enough to tell. Judge only what the picture shows; \
        ignore names and roles."
        .to_owned();
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
    Some(JudgeRequest {
        instruction,
        schema_name: "plate-facing",
        schema,
    })
}

/// The facing of each candidate in `candidates`, in that order (`None` for `unclear`). Every
/// candidate asked about must be answered exactly once, with a label the question offered.
pub(in crate::gameplay) fn read_facing(
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
            .ok_or(ReadError::Shape)? as usize;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_facing_question_offers_the_closed_labels_and_only_the_candidates_asked_about() {
        let request = facing_request(&[2, 5]).unwrap();
        let items = &request.schema["properties"]["facings"]["items"]["properties"];
        assert_eq!(items["candidate"]["enum"], json!([2, 5]));
        assert_eq!(
            items["facing"]["enum"],
            json!(["left", "right", "camera", "away", "unclear"])
        );
        assert!(facing_request(&[]).is_none());
        assert!(facing_request(&[1; 17]).is_none());
    }

    #[test]
    fn every_label_reads_back_to_its_facing_and_unclear_is_none() {
        let answer = json!({"facings": [
            {"candidate": 5, "facing": "away"},
            {"candidate": 2, "facing": "left"},
            {"candidate": 7, "facing": "unclear"},
            {"candidate": 1, "facing": "camera"},
            {"candidate": 3, "facing": "right"},
        ]});
        let read = read_facing(&answer, &[1, 2, 3, 5, 7]).unwrap();
        assert_eq!(
            read,
            vec![
                Some(Facing::TowardCamera),
                Some(Facing::Left),
                Some(Facing::Right),
                Some(Facing::AwayFromCamera),
                None,
            ]
        );
        for facing in [
            Facing::Left,
            Facing::Right,
            Facing::TowardCamera,
            Facing::AwayFromCamera,
        ] {
            assert!(LABELS.contains(&facing.label()));
        }
    }

    #[test]
    fn answers_outside_the_question_are_refused() {
        for (answer, expected) in [
            (json!({}), ReadError::Shape),
            (json!({"facings": [{"candidate": 1}]}), ReadError::Shape),
            (
                json!({"facings": [{"candidate": 1, "facing": "up"}]}),
                ReadError::OutsideChoices,
            ),
            (
                json!({"facings": [{"candidate": 9, "facing": "left"}]}),
                ReadError::OutsideChoices,
            ),
            (
                json!({"facings": [{"candidate": 1, "facing": "left"}]}),
                ReadError::Incomplete,
            ),
            (
                json!({"facings": [
                    {"candidate": 1, "facing": "left"},
                    {"candidate": 1, "facing": "right"},
                    {"candidate": 2, "facing": "right"},
                ]}),
                ReadError::Incomplete,
            ),
        ] {
            assert_eq!(read_facing(&answer, &[1, 2]), Err(expected), "{answer}");
        }
    }
}
