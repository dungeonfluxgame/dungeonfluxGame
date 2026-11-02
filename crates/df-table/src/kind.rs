//! Kinds of canvas element, defined by data.
//!
//! A kind says what an element looks like and which fields it has, so a new kind is a file and a
//! reload, never a build. This module owns the checked definitions and the registry that holds
//! them. Reading and checking the files is `kind_file.rs`; spawning is `canvas.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::element::{Color, Element, FieldName, FieldValue, KindName};
use crate::field_type::FieldType;
use crate::kind_file::{KindError, KindFile, check_kind, duplicate_kind};

/// How an element is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Circle,
    Rect,
}

/// One declared field of a kind.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDefinition {
    pub name: FieldName,
    pub field_type: FieldType,
    pub default: Option<FieldValue>,
    pub required: bool,
}

/// A checked kind of element.
#[derive(Debug, Clone, PartialEq)]
pub struct KindDefinition {
    pub name: KindName,
    pub description: String,
    pub shape: Shape,
    /// Width in canvas units; a circle's diameter.
    pub width_units: f32,
    pub height_units: f32,
    pub fields: Vec<FieldDefinition>,
    pub label_field: Option<FieldName>,
    pub color_field: Option<FieldName>,
}

impl KindDefinition {
    pub fn field(&self, name: &FieldName) -> Option<&FieldDefinition> {
        self.fields.iter().find(|field| &field.name == name)
    }

    /// The text drawn beside an element: its label field, or its id when that is missing or empty.
    pub fn label(&self, element: &Element) -> String {
        self.label_field
            .as_ref()
            .and_then(|name| element.fields.get(name))
            .map(FieldValue::display_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| element.id.to_string())
    }

    /// The fill color of an element, when the kind has a color field.
    pub fn color(&self, element: &Element) -> Option<Color> {
        match self
            .color_field
            .as_ref()
            .and_then(|name| element.fields.get(name))
        {
            Some(FieldValue::Color(color)) => Some(*color),
            Some(_) | None => None,
        }
    }
}

/// The checked kinds a canvas can spawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KindRegistry {
    kinds: BTreeMap<KindName, KindDefinition>,
}

impl KindRegistry {
    /// Checks every file and either returns the registry or every problem found.
    pub fn from_files(files: &[KindFile]) -> Result<Self, Vec<KindError>> {
        let mut kinds = BTreeMap::new();
        let mut errors = Vec::new();
        for file in files {
            match check_kind(file) {
                Ok(definition) if kinds.contains_key(&definition.name) => {
                    errors.push(duplicate_kind(file));
                }
                Ok(definition) => {
                    kinds.insert(definition.name.clone(), definition);
                }
                Err(problems) => errors.extend(problems),
            }
        }
        if errors.is_empty() {
            Ok(Self { kinds })
        } else {
            Err(errors)
        }
    }

    pub fn get(&self, name: &KindName) -> Option<&KindDefinition> {
        self.kinds.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &KindDefinition> {
        self.kinds.values()
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}
