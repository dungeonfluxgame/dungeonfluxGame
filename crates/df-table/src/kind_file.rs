//! Kind files: the JSON an author writes, and its checking into definitions.
//!
//! # The kind-file format
//!
//! One JSON object per file, with these keys (a misspelt key is an error):
//!
//! - `name` (required): the kind's name, a lowercase identifier.
//! - `description`: one line for the `kinds` listing.
//! - `shape` (required): `circle` or `rect`.
//! - `width` and `height` (required): the size in canvas units, from 1 up to the canvas size. A
//!   circle is drawn with `width` as its diameter.
//! - `fields`: a list of fields, each with `name` and `type` (required) and optionally `default`
//!   and `required`. A type takes only its own options: `text` takes `maximum_length`, `number`
//!   takes `minimum` and `maximum`, `choice` takes `choices`, and `flag` and `color` take none.
//! - `label_field`: the field whose value is drawn beside the element (the element's id is drawn
//!   when it is missing or empty).
//! - `color_field`: a field of type `color` that fills the element.
//!
//! This module owns reading that schema and reporting every problem at once, because an author
//! fixes a file faster with the whole list. It never reads files (the host does) and never
//! creates elements.

use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;

use crate::element::{
    CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, FieldName, KindName, MAXIMUM_NAME_LENGTH,
    MAXIMUM_TEXT_LENGTH, NameError,
};
use crate::field_type::{FieldType, ValueError};
use crate::kind::{FieldDefinition, KindDefinition, Shape};

/// Most fields one kind may declare.
pub const MAXIMUM_FIELDS_PER_KIND: usize = 32;
/// Most options one choice field may list.
pub const MAXIMUM_CHOICES: usize = 32;
/// Smallest width or height of a kind, in canvas units.
pub const MINIMUM_SIZE_UNITS: f32 = 1.0;
const DEFAULT_NUMBER_LIMIT: f64 = 1_000_000.0;

/// The type names a kind file may use for a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FieldTypeName {
    Text,
    Number,
    Flag,
    Color,
    Choice,
}

/// A field as written in a kind file. Built only by reading JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldFile {
    name: String,
    #[serde(rename = "type")]
    field_type: FieldTypeName,
    default: Option<serde_json::Value>,
    #[serde(default)]
    required: bool,
    maximum_length: Option<usize>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    choices: Option<Vec<String>>,
}

/// A kind as written in a kind file. Built only by reading JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindFile {
    name: String,
    #[serde(default)]
    description: String,
    shape: Shape,
    width: f32,
    height: f32,
    #[serde(default)]
    fields: Vec<FieldFile>,
    label_field: Option<String>,
    color_field: Option<String>,
}

/// A kind-file option that does not belong to the field's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnusedOption {
    MaximumLength,
    NumberLimits,
    Choices,
}

impl fmt::Display for UnusedOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MaximumLength => "maximum_length",
            Self::NumberLimits => "minimum or maximum",
            Self::Choices => "choices",
        })
    }
}

/// What is wrong with the choices of a choice field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceProblem {
    NoneListed,
    TooMany { given: usize },
    EmptyOrTooLong { choice: String },
    Duplicate { choice: String },
}

/// What is wrong with one field of a kind file.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldProblem {
    InvalidName(NameError),
    Duplicate,
    TextLimit,
    NumberLimits { minimum: f64, maximum: f64 },
    UnusedOption(UnusedOption),
    Choices(ChoiceProblem),
    BadDefault(ValueError),
}

/// What is wrong with a kind file.
#[derive(Debug, Clone, PartialEq)]
pub enum KindProblem {
    InvalidName(NameError),
    Duplicate,
    BadSize,
    TooManyFields,
    Field {
        field: String,
        problem: FieldProblem,
    },
    LabelFieldMissing(String),
    ColorFieldMissing(String),
    ColorFieldNotAColor(String),
}

/// A problem with a kind, named so an author can find it. Names echoed from a file are shortened.
#[derive(Debug, Clone, PartialEq)]
pub struct KindError {
    pub kind: String,
    pub problem: KindProblem,
}

/// Shortens text from a file before it is echoed, so a hostile or accidental long name cannot
/// flood a terminal.
fn shown(text: &str) -> String {
    let mut characters = text.chars();
    let head: String = characters.by_ref().take(MAXIMUM_NAME_LENGTH).collect();
    if characters.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

impl fmt::Display for FieldProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => write!(formatter, "{error}"),
            Self::Duplicate => formatter.write_str("is declared twice"),
            Self::TextLimit => write!(
                formatter,
                "maximum_length is from 1 to {MAXIMUM_TEXT_LENGTH}"
            ),
            Self::NumberLimits { minimum, maximum } => write!(
                formatter,
                "minimum {minimum} must not exceed maximum {maximum} (a limit left out is {DEFAULT_NUMBER_LIMIT})"
            ),
            Self::UnusedOption(option) => {
                write!(formatter, "'{option}' does not apply to this type")
            }
            Self::Choices(ChoiceProblem::NoneListed) => {
                formatter.write_str("a choice field lists its choices")
            }
            Self::Choices(ChoiceProblem::TooMany { given }) => {
                write!(formatter, "{given} choices; at most {MAXIMUM_CHOICES}")
            }
            Self::Choices(ChoiceProblem::EmptyOrTooLong { choice }) => write!(
                formatter,
                "choice '{choice}' must be from 1 to {MAXIMUM_NAME_LENGTH} characters"
            ),
            Self::Choices(ChoiceProblem::Duplicate { choice }) => {
                write!(formatter, "choice '{choice}' is listed twice")
            }
            Self::BadDefault(error) => write!(formatter, "default: {error}"),
        }
    }
}

impl fmt::Display for KindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "kind '{}': ", self.kind)?;
        match &self.problem {
            KindProblem::InvalidName(error) => write!(formatter, "{error}"),
            KindProblem::Duplicate => formatter.write_str("defined twice"),
            KindProblem::BadSize => write!(
                formatter,
                "width is from {MINIMUM_SIZE_UNITS} to {CANVAS_WIDTH_UNITS} and height from \
                 {MINIMUM_SIZE_UNITS} to {CANVAS_HEIGHT_UNITS}"
            ),
            KindProblem::TooManyFields => {
                write!(formatter, "at most {MAXIMUM_FIELDS_PER_KIND} fields")
            }
            KindProblem::Field { field, problem } => write!(formatter, "field '{field}' {problem}"),
            KindProblem::LabelFieldMissing(field) => {
                write!(
                    formatter,
                    "label_field '{field}' is not a field of this kind"
                )
            }
            KindProblem::ColorFieldMissing(field) => {
                write!(
                    formatter,
                    "color_field '{field}' is not a field of this kind"
                )
            }
            KindProblem::ColorFieldNotAColor(field) => {
                write!(formatter, "color_field '{field}' must have type color")
            }
        }
    }
}

fn check_choices(choices: &[String]) -> Result<(), ChoiceProblem> {
    if choices.is_empty() {
        return Err(ChoiceProblem::NoneListed);
    }
    if choices.len() > MAXIMUM_CHOICES {
        return Err(ChoiceProblem::TooMany {
            given: choices.len(),
        });
    }
    let mut seen = BTreeSet::new();
    for choice in choices {
        let length = choice.chars().count();
        if length == 0 || length > MAXIMUM_NAME_LENGTH {
            return Err(ChoiceProblem::EmptyOrTooLong {
                choice: shown(choice),
            });
        }
        if !seen.insert(choice) {
            return Err(ChoiceProblem::Duplicate {
                choice: shown(choice),
            });
        }
    }
    Ok(())
}

fn field_type_from_file(file: &FieldFile) -> Result<FieldType, FieldProblem> {
    let reject = |present: bool, option: UnusedOption| {
        if present {
            Err(FieldProblem::UnusedOption(option))
        } else {
            Ok(())
        }
    };
    let has_number_limit = file.minimum.is_some() || file.maximum.is_some();
    match file.field_type {
        FieldTypeName::Text => {
            reject(has_number_limit, UnusedOption::NumberLimits)?;
            reject(file.choices.is_some(), UnusedOption::Choices)?;
            let maximum_length = file.maximum_length.unwrap_or(MAXIMUM_TEXT_LENGTH);
            if maximum_length == 0 || maximum_length > MAXIMUM_TEXT_LENGTH {
                return Err(FieldProblem::TextLimit);
            }
            Ok(FieldType::Text { maximum_length })
        }
        FieldTypeName::Number => {
            reject(file.maximum_length.is_some(), UnusedOption::MaximumLength)?;
            reject(file.choices.is_some(), UnusedOption::Choices)?;
            let minimum = file.minimum.unwrap_or(-DEFAULT_NUMBER_LIMIT);
            let maximum = file.maximum.unwrap_or(DEFAULT_NUMBER_LIMIT);
            if !(minimum.is_finite() && maximum.is_finite() && minimum <= maximum) {
                return Err(FieldProblem::NumberLimits { minimum, maximum });
            }
            Ok(FieldType::Number { minimum, maximum })
        }
        FieldTypeName::Flag | FieldTypeName::Color => {
            reject(file.maximum_length.is_some(), UnusedOption::MaximumLength)?;
            reject(has_number_limit, UnusedOption::NumberLimits)?;
            reject(file.choices.is_some(), UnusedOption::Choices)?;
            Ok(if file.field_type == FieldTypeName::Flag {
                FieldType::Flag
            } else {
                FieldType::Color
            })
        }
        FieldTypeName::Choice => {
            reject(file.maximum_length.is_some(), UnusedOption::MaximumLength)?;
            reject(has_number_limit, UnusedOption::NumberLimits)?;
            let choices = file
                .choices
                .clone()
                .ok_or(FieldProblem::Choices(ChoiceProblem::NoneListed))?;
            check_choices(&choices).map_err(FieldProblem::Choices)?;
            Ok(FieldType::Choice { choices })
        }
    }
}

fn field_from_file(file: &FieldFile) -> Result<FieldDefinition, FieldProblem> {
    let name = FieldName::new(&file.name).map_err(FieldProblem::InvalidName)?;
    let field_type = field_type_from_file(file)?;
    let default = file
        .default
        .as_ref()
        .map(|json| field_type.from_json(json))
        .transpose()
        .map_err(FieldProblem::BadDefault)?;
    Ok(FieldDefinition {
        name,
        field_type,
        default,
        required: file.required,
    })
}

fn size_is_valid(size: f32, maximum: f32) -> bool {
    size.is_finite() && (MINIMUM_SIZE_UNITS..=maximum).contains(&size)
}

/// Checks one kind file, returning the definition or every problem in it.
pub(crate) fn check_kind(file: &KindFile) -> Result<KindDefinition, Vec<KindError>> {
    let kind = shown(&file.name);
    let error = |problem| KindError {
        kind: kind.clone(),
        problem,
    };
    let mut errors = Vec::new();
    let name = match KindName::new(&file.name) {
        Ok(name) => Some(name),
        Err(problem) => {
            errors.push(error(KindProblem::InvalidName(problem)));
            None
        }
    };
    if !(size_is_valid(file.width, CANVAS_WIDTH_UNITS)
        && size_is_valid(file.height, CANVAS_HEIGHT_UNITS))
    {
        errors.push(error(KindProblem::BadSize));
    }
    if file.fields.len() > MAXIMUM_FIELDS_PER_KIND {
        errors.push(error(KindProblem::TooManyFields));
    }

    let mut fields = Vec::new();
    // Every declared name, including a field that failed, so a label or color field that points
    // at a failed field is not blamed a second time.
    let mut declared = BTreeSet::new();
    for field_file in file.fields.iter().take(MAXIMUM_FIELDS_PER_KIND) {
        let field_error = |problem| {
            error(KindProblem::Field {
                field: shown(&field_file.name),
                problem,
            })
        };
        let first_time = declared.insert(field_file.name.clone());
        match field_from_file(field_file) {
            Ok(_) if !first_time => errors.push(field_error(FieldProblem::Duplicate)),
            Ok(field) => fields.push(field),
            Err(problem) => errors.push(field_error(problem)),
        }
    }

    let find = |name: &str| fields.iter().find(|field| field.name.as_str() == name);
    let label_field = file.label_field.as_deref().and_then(|label| {
        if let Some(field) = find(label) {
            Some(field.name.clone())
        } else {
            if !declared.contains(label) {
                errors.push(error(KindProblem::LabelFieldMissing(shown(label))));
            }
            None
        }
    });
    let color_field = file
        .color_field
        .as_deref()
        .and_then(|color| match find(color) {
            Some(field) if field.field_type == FieldType::Color => Some(field.name.clone()),
            Some(_) => {
                errors.push(error(KindProblem::ColorFieldNotAColor(shown(color))));
                None
            }
            None => {
                if !declared.contains(color) {
                    errors.push(error(KindProblem::ColorFieldMissing(shown(color))));
                }
                None
            }
        });

    match (name, errors.is_empty()) {
        (Some(name), true) => Ok(KindDefinition {
            name,
            description: file.description.clone(),
            shape: file.shape,
            width_units: file.width,
            height_units: file.height,
            fields,
            label_field,
            color_field,
        }),
        _ => Err(errors),
    }
}

/// The error for a kind defined by two files.
pub(crate) fn duplicate_kind(file: &KindFile) -> KindError {
    KindError {
        kind: shown(&file.name),
        problem: KindProblem::Duplicate,
    }
}
