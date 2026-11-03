//! The closed schema of a Locate answer: the instances a segmentation request found.
//!
//! A broker result carries text or a picture, so the worker that runs a segmentation call reduces
//! the model's masks to a box and a score for each instance, normalised to the canvas in
//! thousandths of a unit, and writes them as this JSON. The builder reads it back here and nowhere
//! else: a field the schema does not name, a box off the canvas, an empty box or a score above 1 is
//! a `TextInvalid` failure of that request (C.7), never a guess. The fakes use `encode` to write
//! the same text, so a test and a real worker meet at one schema.
//!
//! Answers hold regions and scores only, no names and no player text.

use std::fmt;

use df_table::private_data::Narration;
use df_table::scene_plan_locate::{Instance, Region, RegionError};
use serde::{Deserialize, Serialize};

/// Why an answer was refused.
#[derive(Debug)]
pub enum AnswerError {
    /// The text is not the schema.
    Malformed(serde_json::Error),
    /// An instance is not a valid region or score.
    Region(RegionError),
}

impl fmt::Display for AnswerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(formatter, "a locate answer is not valid: {error}"),
            Self::Region(error) => write!(formatter, "a locate answer holds {error}"),
        }
    }
}

impl std::error::Error for AnswerError {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceRecord {
    x_min_milli_units: u32,
    y_min_milli_units: u32,
    x_max_milli_units: u32,
    y_max_milli_units: u32,
    score_permille: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerRecord {
    instances: Vec<InstanceRecord>,
}

/// The instances of a Locate answer.
pub fn parse_locate_answer(text: &Narration) -> Result<Vec<Instance>, AnswerError> {
    let record: AnswerRecord =
        serde_json::from_str(text.expose()).map_err(AnswerError::Malformed)?;
    record
        .instances
        .into_iter()
        .map(|instance| {
            let region = Region::new(
                instance.x_min_milli_units,
                instance.y_min_milli_units,
                instance.x_max_milli_units,
                instance.y_max_milli_units,
            )
            .map_err(AnswerError::Region)?;
            Instance::new(region, instance.score_permille).map_err(AnswerError::Region)
        })
        .collect()
}

/// A Locate answer for `instances`, for the worker that makes one.
pub fn encode_locate_answer(instances: &[Instance]) -> Result<Narration, serde_json::Error> {
    let record = AnswerRecord {
        instances: instances
            .iter()
            .map(|instance| {
                let (x_min, y_min, x_max, y_max) = instance.region().bounds_milli_units();
                InstanceRecord {
                    x_min_milli_units: x_min,
                    y_min_milli_units: y_min,
                    x_max_milli_units: x_max,
                    y_max_milli_units: y_max,
                    score_permille: instance.score_permille(),
                }
            })
            .collect(),
    };
    serde_json::to_string(&record).map(Narration::new)
}
