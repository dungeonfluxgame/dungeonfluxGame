//! A copy of the canvas taken under the lock, for the viewer and the JSON answer.
//!
//! It owns the shape of what a reader of the canvas sees, in one typed form that the server
//! writes and the command line reads back. An element (`ElementView`) and the canvas of a frame
//! (`df_playground::frame::FrameCanvas`) are the page's own types, which the host writes, so the
//! two cannot drift. Taking a copy keeps the lock short: every element is
//! copied once, its problem is found in the same pass, and rendering or writing JSON happens after
//! the lock is released. A value that cannot be written (a number that is not finite) is an error,
//! never a quiet `null`.

use std::fmt;

use df_playground::fixture::ElementShape;
pub use df_playground::fixture::{ElementView, PivotView, TravelView};
use df_playground::frame::FrameCanvas;
use df_table::canvas::Canvas;
use df_table::element::{Color, Element, FieldValue, Pivot};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasView {
    pub revision: u64,
    pub elements: Vec<ElementView>,
    pub kinds: Vec<KindView>,
}

/// Why a copy of the canvas could not be made: a typed route error, answered as a `DevReply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ViewError {
    /// A number field holds a value JSON cannot write (not a finite number).
    NotWritable { element: String, field: String },
    /// A frame of the canvas could not be written or built; `detail` says why.
    NotBuilt { detail: String },
}

impl fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWritable { element, field } => write!(
                formatter,
                "field '{field}' of element '{element}' holds a number that is not finite"
            ),
            Self::NotBuilt { detail } => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for ViewError {}

impl crate::dev_http::RouteError for ViewError {
    fn status(&self) -> axum::http::StatusCode {
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// A field value as JSON, or `None` for a number that is not finite.
pub fn value_json(value: &FieldValue) -> Option<serde_json::Value> {
    Some(match value {
        FieldValue::Text(text) => serde_json::Value::String(text.clone()),
        FieldValue::Number(number) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*number)?)
        }
        FieldValue::Flag(flag) => serde_json::Value::Bool(*flag),
        FieldValue::Color(color) => serde_json::Value::String(color.to_hex()),
    })
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
                    default: field.default.as_ref().and_then(value_json),
                }
            })
            .collect(),
    }
}

/// One element as a reader sees it, copied from the canvas once. Its problem is found here, in
/// the same pass, not by searching a list of every element's problems.
fn element_view(canvas: &Canvas, element: Element) -> Result<ElementView, ViewError> {
    let definition = canvas.registry().get(&element.kind);
    let mut fields = serde_json::Map::new();
    for (name, value) in &element.fields {
        let json = value_json(value).ok_or_else(|| ViewError::NotWritable {
            element: element.id.to_string(),
            field: name.to_string(),
        })?;
        fields.insert(name.to_string(), json);
    }
    Ok(ElementView {
        id: element.id.to_string(),
        kind: element.kind.to_string(),
        x: element.position.x,
        y: element.position.y,
        label: definition.map_or_else(|| element.id.to_string(), |kind| kind.label(&element)),
        color: definition
            .and_then(|kind| kind.color(&element))
            .map(Color::to_hex),
        shape: element_shape(definition.map_or(FALLBACK_SHAPE, |kind| kind.shape)),
        width_units: definition.map_or(FALLBACK_SIZE_UNITS, |kind| kind.width_units),
        height_units: definition.map_or(FALLBACK_SIZE_UNITS, |kind| kind.height_units),
        fields,
        problem: canvas
            .problem_of(&element)
            .map(|problem| problem.to_string()),
        travelling: element.motion.map(|motion| TravelView {
            x: motion.target.x,
            y: motion.target.y,
            speed: motion.speed.units_per_second(),
        }),
        rotation_degrees: element.appearance.rotation.get(),
        pivot: pivot_view(element.appearance.pivot),
    })
}

fn element_views(canvas: &Canvas) -> Result<Vec<ElementView>, ViewError> {
    canvas
        .elements()
        .map(|element| element_view(canvas, element))
        .collect()
}

/// How the page draws an element of a kind of `shape`.
pub fn element_shape(shape: Shape) -> ElementShape {
    match shape {
        Shape::Circle => ElementShape::Circle,
        Shape::Rect => ElementShape::Rect,
    }
}

/// The point an element turns about, as the page reads it.
pub fn pivot_view(pivot: Pivot) -> PivotView {
    PivotView {
        x_percent: pivot.x.get(),
        y_percent: pivot.y.get(),
    }
}

/// The canvas as a frame carries it: its revision and its elements, without the kinds.
pub fn frame_canvas(canvas: &Canvas) -> Result<FrameCanvas, ViewError> {
    Ok(FrameCanvas {
        revision: canvas.revision(),
        elements: element_views(canvas)?,
    })
}

impl CanvasView {
    pub fn from_canvas(canvas: &Canvas) -> Result<Self, ViewError> {
        Ok(Self {
            revision: canvas.revision(),
            elements: element_views(canvas)?,
            kinds: canvas.registry().iter().map(kind_view).collect(),
        })
    }
}
