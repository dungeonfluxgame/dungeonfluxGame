//! Turning an `EffectRequest` into a typed `CanvasEffect`.
//!
//! It owns reading names and values: the kind decides the shape of every value, so a request
//! written on a command line (all text) and one written as JSON both end as typed values. It
//! never changes the canvas: it only builds the effect the canvas will check.

use std::collections::BTreeMap;
use std::fmt;

use df_table::canvas::{Canvas, CanvasEffect, CanvasError};
use df_table::element::{Color, ElementId, FieldName, FieldValue, KindName, NameError, Position};
use df_table::event::{EffectKind, RejectionCause};
use df_table::field_type::{FieldType, ValueError};
use df_table::motion::{Speed, SpeedError};

use crate::wire::{EffectRequest, ValueForm};

/// Which kind of name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    ElementId,
    Kind,
    Field,
}

impl fmt::Display for NameKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ElementId => "element id",
            Self::Kind => "kind",
            Self::Field => "field",
        })
    }
}

/// Why a request could not become an effect.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestError {
    Name {
        name_kind: NameKind,
        error: NameError,
    },
    Value {
        field: String,
        error: ValueError,
    },
    Speed(SpeedError),
    Canvas(CanvasError),
}

impl RequestError {
    pub fn cause(&self) -> RejectionCause {
        match self {
            Self::Name { .. } => RejectionCause::InvalidName,
            Self::Value { .. } => RejectionCause::InvalidValue,
            Self::Speed(_) => RejectionCause::InvalidSpeed,
            Self::Canvas(error) => error.cause(),
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name { name_kind, error } => write!(formatter, "{name_kind}: {error}"),
            Self::Value { field, error } => write!(formatter, "field '{field}': {error}"),
            Self::Speed(error) => write!(formatter, "{error}"),
            Self::Canvas(error) => write!(formatter, "{error}"),
        }
    }
}

/// Reads a value typed as text (on a command line) as the given field type, and checks it.
pub fn parse_text_value(field_type: &FieldType, text: &str) -> Result<FieldValue, ValueError> {
    let wrong = || ValueError::WrongShape {
        expected: field_type.expected(),
    };
    let value = match field_type {
        FieldType::Text { .. } | FieldType::Choice { .. } => FieldValue::Text(text.to_owned()),
        FieldType::Number { .. } => FieldValue::Number(text.parse::<f64>().map_err(|_| wrong())?),
        FieldType::Flag => match text {
            "true" => FieldValue::Flag(true),
            "false" => FieldValue::Flag(false),
            _ => return Err(wrong()),
        },
        FieldType::Color => FieldValue::Color(Color::parse(text).ok_or(ValueError::InvalidColor)?),
    };
    field_type.check(&value)?;
    Ok(value)
}

fn element_id(text: &str) -> Result<ElementId, RequestError> {
    ElementId::new(text).map_err(|error| RequestError::Name {
        name_kind: NameKind::ElementId,
        error,
    })
}

fn kind_name(text: &str) -> Result<KindName, RequestError> {
    KindName::new(text).map_err(|error| RequestError::Name {
        name_kind: NameKind::Kind,
        error,
    })
}

fn field_name(text: &str) -> Result<FieldName, RequestError> {
    FieldName::new(text).map_err(|error| RequestError::Name {
        name_kind: NameKind::Field,
        error,
    })
}

fn read_value(
    canvas: &Canvas,
    kind: &KindName,
    field: &FieldName,
    json: &serde_json::Value,
    form: ValueForm,
) -> Result<FieldValue, RequestError> {
    let field_type = canvas
        .field_type(kind, field)
        .map_err(RequestError::Canvas)?;
    let value_error = |error| RequestError::Value {
        field: field.to_string(),
        error,
    };
    match form {
        ValueForm::Json => field_type.from_json(json).map_err(value_error),
        ValueForm::Text => match json {
            serde_json::Value::String(text) => {
                parse_text_value(field_type, text).map_err(value_error)
            }
            _ => Err(value_error(ValueError::WrongShape {
                expected: "text on the command line",
            })),
        },
    }
}

impl EffectRequest {
    pub fn effect_kind(&self) -> EffectKind {
        match self {
            Self::Spawn { .. } => EffectKind::Spawn,
            Self::Despawn { .. } => EffectKind::Despawn,
            Self::Set { .. } => EffectKind::Set,
            Self::Move { .. } => EffectKind::Move,
            Self::Travel { .. } => EffectKind::Travel,
            Self::Stop { .. } => EffectKind::Stop,
            Self::Clear {} => EffectKind::Clear,
        }
    }

    /// The element the request is about, when it names one that is a valid id.
    pub fn element(&self) -> Option<ElementId> {
        match self {
            Self::Spawn { id, .. }
            | Self::Despawn { id }
            | Self::Set { id, .. }
            | Self::Move { id, .. }
            | Self::Travel { id, .. }
            | Self::Stop { id } => ElementId::new(id).ok(),
            Self::Clear {} => None,
        }
    }

    /// Builds the typed effect, reading each value by its field's type in the current kinds.
    pub fn into_effect(self, canvas: &Canvas) -> Result<CanvasEffect, RequestError> {
        match self {
            Self::Spawn {
                id,
                kind,
                x,
                y,
                fields,
                values,
            } => {
                let id = element_id(&id)?;
                let kind = kind_name(&kind)?;
                let mut typed = BTreeMap::new();
                for (name, json) in &fields {
                    let field = field_name(name)?;
                    let value = read_value(canvas, &kind, &field, json, values)?;
                    typed.insert(field, value);
                }
                let position = Position::new(
                    x.unwrap_or(Position::CENTER.x),
                    y.unwrap_or(Position::CENTER.y),
                );
                Ok(CanvasEffect::Spawn {
                    id,
                    kind,
                    position,
                    fields: typed,
                })
            }
            Self::Despawn { id } => Ok(CanvasEffect::Despawn {
                id: element_id(&id)?,
            }),
            Self::Set {
                id,
                field,
                value,
                values,
            } => {
                let id = element_id(&id)?;
                let field = field_name(&field)?;
                let kind = canvas
                    .element(&id)
                    .map(|element| element.kind.clone())
                    .ok_or_else(|| RequestError::Canvas(CanvasError::UnknownElement(id.clone())))?;
                let value = read_value(canvas, &kind, &field, &value, values)?;
                Ok(CanvasEffect::Set { id, field, value })
            }
            Self::Move { id, x, y } => Ok(CanvasEffect::Move {
                id: element_id(&id)?,
                position: Position::new(x, y),
            }),
            Self::Travel { id, x, y, speed } => Ok(CanvasEffect::Travel {
                id: element_id(&id)?,
                to: Position::new(x, y),
                speed: Speed::new(speed).map_err(RequestError::Speed)?,
            }),
            Self::Stop { id } => Ok(CanvasEffect::Stop {
                id: element_id(&id)?,
            }),
            Self::Clear {} => Ok(CanvasEffect::Clear),
        }
    }
}
