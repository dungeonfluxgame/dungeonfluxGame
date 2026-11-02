//! A copy of the canvas taken under the lock, for the viewer and the JSON answer.
//!
//! It owns the shape of what a reader of the canvas sees, in one typed form that the server
//! writes and the command line reads back. Taking a copy keeps the lock short: rendering happens
//! after it is released.

use df_table::canvas::Canvas;
use df_table::element::{Color, FieldValue};
use df_table::field_type::FieldType;
use df_table::kind::{KindDefinition, Shape};
use serde::{Deserialize, Serialize};

/// Size drawn for an element whose kind has since been removed, in canvas units.
const FALLBACK_SIZE_UNITS: f32 = 4.0;
/// Shape drawn for an element whose kind has since been removed.
const FALLBACK_SHAPE: Shape = Shape::Rect;

/// The type of a field, without its limits (the limits are in `description`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Number,
    Flag,
    Color,
    Choice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldView {
    pub name: String,
    pub kind: FieldKind,
    /// The type and its limits in words, for a person.
    pub description: String,
    pub required: bool,
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KindView {
    pub name: String,
    pub description: String,
    pub shape: Shape,
    pub width_units: f32,
    pub height_units: f32,
    pub fields: Vec<FieldView>,
}

/// Where an element is heading and how fast, while it travels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TravelView {
    pub x: f32,
    pub y: f32,
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementView {
    pub id: String,
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub color: Option<String>,
    pub shape: Shape,
    pub width_units: f32,
    pub height_units: f32,
    pub fields: serde_json::Map<String, serde_json::Value>,
    /// Why the element no longer fits its kind, when it does not.
    pub problem: Option<String>,
    /// Where the element is heading, while it travels.
    #[serde(default)]
    pub travelling: Option<TravelView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasView {
    pub revision: u64,
    pub elements: Vec<ElementView>,
    pub kinds: Vec<KindView>,
}

/// A field value as JSON.
pub fn value_json(value: &FieldValue) -> serde_json::Value {
    match value {
        FieldValue::Text(text) => serde_json::Value::String(text.clone()),
        FieldValue::Number(number) => serde_json::Number::from_f64(*number)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        FieldValue::Flag(flag) => serde_json::Value::Bool(*flag),
        FieldValue::Color(color) => serde_json::Value::String(color.to_hex()),
    }
}

fn describe(field_type: &FieldType) -> (FieldKind, String) {
    match field_type {
        FieldType::Text { maximum_length } => {
            (FieldKind::Text, format!("text (at most {maximum_length})"))
        }
        FieldType::Number { minimum, maximum } => (
            FieldKind::Number,
            format!("number ({minimum} to {maximum})"),
        ),
        FieldType::Flag => (FieldKind::Flag, "flag (true or false)".to_owned()),
        FieldType::Color => (FieldKind::Color, "color (#rrggbb)".to_owned()),
        FieldType::Choice { choices } => {
            (FieldKind::Choice, format!("one of {}", choices.join(", ")))
        }
    }
}

fn kind_view(definition: &KindDefinition) -> KindView {
    KindView {
        name: definition.name.to_string(),
        description: definition.description.clone(),
        shape: definition.shape,
        width_units: definition.width_units,
        height_units: definition.height_units,
        fields: definition
            .fields
            .iter()
            .map(|field| {
                let (kind, description) = describe(&field.field_type);
                FieldView {
                    name: field.name.to_string(),
                    kind,
                    description,
                    required: field.required,
                    default: field.default.as_ref().map(value_json),
                }
            })
            .collect(),
    }
}

impl CanvasView {
    pub fn from_canvas(canvas: &Canvas) -> Self {
        let problems = canvas.problems();
        let elements = canvas
            .elements()
            .map(|element| {
                let definition = canvas.registry().get(&element.kind);
                ElementView {
                    id: element.id.to_string(),
                    kind: element.kind.to_string(),
                    x: element.position.x,
                    y: element.position.y,
                    label: definition
                        .map_or_else(|| element.id.to_string(), |kind| kind.label(&element)),
                    color: definition
                        .and_then(|kind| kind.color(&element))
                        .map(Color::to_hex),
                    shape: definition.map_or(FALLBACK_SHAPE, |kind| kind.shape),
                    width_units: definition.map_or(FALLBACK_SIZE_UNITS, |kind| kind.width_units),
                    height_units: definition.map_or(FALLBACK_SIZE_UNITS, |kind| kind.height_units),
                    fields: element
                        .fields
                        .iter()
                        .map(|(name, value)| (name.to_string(), value_json(value)))
                        .collect(),
                    problem: problems
                        .iter()
                        .find(|problem| problem.element == element.id)
                        .map(|problem| problem.problem.to_string()),
                    travelling: element.motion.map(|motion| TravelView {
                        x: motion.target.x,
                        y: motion.target.y,
                        speed: motion.speed.units_per_second(),
                    }),
                }
            })
            .collect();
        Self {
            revision: canvas.revision(),
            elements,
            kinds: canvas.registry().iter().map(kind_view).collect(),
        }
    }
}
