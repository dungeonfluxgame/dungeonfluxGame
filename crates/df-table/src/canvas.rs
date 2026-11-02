//! The blank canvas: elements spawned at run time from data-defined kinds.
//!
//! It owns the element set, the checks that keep every element valid for its kind, and the
//! revision that moves on every change. Every effect either changes the canvas and records the
//! change, or is rejected and records the rejection, so nothing happens silently inside the
//! canvas. It never reads files, draws anything or waits: the host loads kinds, renders and
//! serves, and records the rejections that happen before a request becomes an effect.

use std::collections::BTreeMap;
use std::fmt;

use crate::element::{
    CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, Element, ElementId, FieldName, FieldValue, KindName,
    Position,
};
use crate::event::{EffectKind, RejectionCause, TableEvent};
use crate::field_type::{FieldType, ValueError};
use crate::kind::KindRegistry;
use crate::ports::EventSink;

/// Most elements the canvas holds at once.
pub const MAXIMUM_ELEMENTS: usize = 512;

/// One change to the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEffect {
    Spawn {
        id: ElementId,
        kind: KindName,
        position: Position,
        fields: BTreeMap<FieldName, FieldValue>,
    },
    Despawn {
        id: ElementId,
    },
    Set {
        id: ElementId,
        field: FieldName,
        value: FieldValue,
    },
    Move {
        id: ElementId,
        position: Position,
    },
    Clear,
}

impl CanvasEffect {
    pub fn kind(&self) -> EffectKind {
        match self {
            Self::Spawn { .. } => EffectKind::Spawn,
            Self::Despawn { .. } => EffectKind::Despawn,
            Self::Set { .. } => EffectKind::Set,
            Self::Move { .. } => EffectKind::Move,
            Self::Clear => EffectKind::Clear,
        }
    }

    /// The element the effect is about, when it is about one.
    pub fn element(&self) -> Option<&ElementId> {
        match self {
            Self::Spawn { id, .. }
            | Self::Despawn { id }
            | Self::Set { id, .. }
            | Self::Move { id, .. } => Some(id),
            Self::Clear => None,
        }
    }
}

/// Why an effect was rejected, or why an element does not fit its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasError {
    UnknownKind(KindName),
    DuplicateElement(ElementId),
    UnknownElement(ElementId),
    UnknownField {
        kind: KindName,
        field: FieldName,
    },
    InvalidValue {
        field: FieldName,
        error: ValueError,
    },
    /// A spawn left out a required field that has no default.
    MissingRequiredField(FieldName),
    /// An existing element has no value for a required field. Defaults apply only when an
    /// element is made, so a field added to a kind later is absent from the elements made before.
    RequiredFieldAbsent(FieldName),
    TooManyElements,
    OutOfBounds,
}

impl CanvasError {
    pub fn cause(&self) -> RejectionCause {
        match self {
            Self::UnknownKind(_) => RejectionCause::UnknownKind,
            Self::DuplicateElement(_) => RejectionCause::DuplicateElement,
            Self::UnknownElement(_) => RejectionCause::UnknownElement,
            Self::UnknownField { .. } => RejectionCause::UnknownField,
            Self::InvalidValue { .. } => RejectionCause::InvalidValue,
            Self::MissingRequiredField(_) | Self::RequiredFieldAbsent(_) => {
                RejectionCause::MissingRequiredField
            }
            Self::TooManyElements => RejectionCause::TooManyElements,
            Self::OutOfBounds => RejectionCause::OutOfBounds,
        }
    }
}

impl fmt::Display for CanvasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(kind) => write!(formatter, "there is no kind '{kind}'"),
            Self::DuplicateElement(id) => write!(formatter, "'{id}' already exists"),
            Self::UnknownElement(id) => write!(formatter, "there is no element '{id}'"),
            Self::UnknownField { kind, field } => {
                write!(formatter, "kind '{kind}' has no field '{field}'")
            }
            Self::InvalidValue { field, error } => write!(formatter, "field '{field}': {error}"),
            Self::MissingRequiredField(field) => {
                write!(formatter, "field '{field}' is required and has no default")
            }
            Self::RequiredFieldAbsent(field) => write!(
                formatter,
                "field '{field}' is required but this element has no value for it"
            ),
            Self::TooManyElements => write!(
                formatter,
                "the canvas holds at most {MAXIMUM_ELEMENTS} elements"
            ),
            Self::OutOfBounds => write!(
                formatter,
                "x is from 0 to {CANVAS_WIDTH_UNITS} and y is from 0 to {CANVAS_HEIGHT_UNITS}"
            ),
        }
    }
}

/// An element that no longer fits its kind, found after the kinds changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementProblem {
    pub element: ElementId,
    pub problem: CanvasError,
}

/// What a reload of the kinds did.
#[derive(Debug, Clone, PartialEq)]
pub struct ReloadReport {
    pub kinds: usize,
    pub problems: Vec<ElementProblem>,
}

/// The type of a field of a kind, or why there is none.
fn field_type_of<'a>(
    registry: &'a KindRegistry,
    kind: &KindName,
    field: &FieldName,
) -> Result<&'a FieldType, CanvasError> {
    registry
        .get(kind)
        .ok_or_else(|| CanvasError::UnknownKind(kind.clone()))?
        .field(field)
        .map(|definition| &definition.field_type)
        .ok_or_else(|| CanvasError::UnknownField {
            kind: kind.clone(),
            field: field.clone(),
        })
}

fn check_element(registry: &KindRegistry, element: &Element) -> Result<(), CanvasError> {
    let definition = registry
        .get(&element.kind)
        .ok_or_else(|| CanvasError::UnknownKind(element.kind.clone()))?;
    for (name, value) in &element.fields {
        let field = definition
            .field(name)
            .ok_or_else(|| CanvasError::UnknownField {
                kind: element.kind.clone(),
                field: name.clone(),
            })?;
        field
            .field_type
            .check(value)
            .map_err(|error| CanvasError::InvalidValue {
                field: name.clone(),
                error,
            })?;
    }
    for field in &definition.fields {
        if field.required && !element.fields.contains_key(&field.name) {
            return Err(CanvasError::RequiredFieldAbsent(field.name.clone()));
        }
    }
    Ok(())
}

/// The set of elements on the blank canvas.
#[derive(Debug, Clone)]
pub struct Canvas {
    registry: KindRegistry,
    elements: BTreeMap<ElementId, Element>,
    revision: u64,
}

impl Canvas {
    pub fn new(registry: KindRegistry) -> Self {
        Self {
            registry,
            elements: BTreeMap::new(),
            revision: 0,
        }
    }

    /// Moves on with every change, so a viewer can tell whether it is current.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn registry(&self) -> &KindRegistry {
        &self.registry
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.elements.values()
    }

    pub fn element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.get(id)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The type of a field of a kind, for reading a value that arrives as text or JSON.
    pub fn field_type(
        &self,
        kind: &KindName,
        field: &FieldName,
    ) -> Result<&FieldType, CanvasError> {
        field_type_of(&self.registry, kind, field)
    }

    /// Elements that no longer fit their kind, for a viewer to flag.
    pub fn problems(&self) -> Vec<ElementProblem> {
        self.elements
            .values()
            .filter_map(|element| {
                check_element(&self.registry, element)
                    .err()
                    .map(|problem| ElementProblem {
                        element: element.id.clone(),
                        problem,
                    })
            })
            .collect()
    }

    /// Applies one effect and records it, or records why it was rejected.
    ///
    /// The sink is called while the caller may hold a lock on this canvas, so it must not block.
    pub fn apply(&mut self, effect: CanvasEffect, sink: &dyn EventSink) -> Result<(), CanvasError> {
        let kind = effect.kind();
        let element = effect.element().cloned();
        match self.change(effect) {
            Ok(event) => {
                sink.record(&event);
                Ok(())
            }
            Err(error) => {
                sink.record(&TableEvent::EffectRejected {
                    effect: Some(kind),
                    element,
                    cause: error.cause(),
                });
                Err(error)
            }
        }
    }

    fn change(&mut self, effect: CanvasEffect) -> Result<TableEvent, CanvasError> {
        match effect {
            CanvasEffect::Spawn {
                id,
                kind,
                position,
                fields,
            } => self.spawn(id, kind, position, fields),
            CanvasEffect::Despawn { id } => self.despawn(id),
            CanvasEffect::Set { id, field, value } => self.set(id, field, value),
            CanvasEffect::Move { id, position } => self.move_element(id, position),
            CanvasEffect::Clear => {
                let removed = self.elements.len();
                self.elements.clear();
                let revision = self.advance();
                Ok(TableEvent::CanvasCleared { removed, revision })
            }
        }
    }

    fn spawn(
        &mut self,
        id: ElementId,
        kind: KindName,
        position: Position,
        mut fields: BTreeMap<FieldName, FieldValue>,
    ) -> Result<TableEvent, CanvasError> {
        let definition = self
            .registry
            .get(&kind)
            .ok_or_else(|| CanvasError::UnknownKind(kind.clone()))?;
        if self.elements.len() >= MAXIMUM_ELEMENTS {
            return Err(CanvasError::TooManyElements);
        }
        if self.elements.contains_key(&id) {
            return Err(CanvasError::DuplicateElement(id));
        }
        if !position.is_on_canvas() {
            return Err(CanvasError::OutOfBounds);
        }
        for field in &definition.fields {
            if !fields.contains_key(&field.name)
                && let Some(default) = &field.default
            {
                fields.insert(field.name.clone(), default.clone());
            }
        }
        for field in &definition.fields {
            if field.required && !fields.contains_key(&field.name) {
                return Err(CanvasError::MissingRequiredField(field.name.clone()));
            }
        }
        let element = Element {
            id: id.clone(),
            kind: kind.clone(),
            position,
            fields,
        };
        check_element(&self.registry, &element)?;
        self.elements.insert(id.clone(), element);
        let revision = self.advance();
        Ok(TableEvent::ElementSpawned {
            element: id,
            kind,
            revision,
        })
    }

    fn despawn(&mut self, id: ElementId) -> Result<TableEvent, CanvasError> {
        self.elements
            .remove(&id)
            .ok_or_else(|| CanvasError::UnknownElement(id.clone()))?;
        let revision = self.advance();
        Ok(TableEvent::ElementDespawned {
            element: id,
            revision,
        })
    }

    fn set(
        &mut self,
        id: ElementId,
        field: FieldName,
        value: FieldValue,
    ) -> Result<TableEvent, CanvasError> {
        // Borrow the registry and the elements separately so the element is looked up once.
        let Self {
            registry, elements, ..
        } = self;
        let element = elements
            .get_mut(&id)
            .ok_or_else(|| CanvasError::UnknownElement(id.clone()))?;
        field_type_of(registry, &element.kind, &field)?
            .check(&value)
            .map_err(|error| CanvasError::InvalidValue {
                field: field.clone(),
                error,
            })?;
        element.fields.insert(field.clone(), value);
        let revision = self.advance();
        Ok(TableEvent::ElementChanged {
            element: id,
            field,
            revision,
        })
    }

    fn move_element(
        &mut self,
        id: ElementId,
        position: Position,
    ) -> Result<TableEvent, CanvasError> {
        let element = self
            .elements
            .get_mut(&id)
            .ok_or_else(|| CanvasError::UnknownElement(id.clone()))?;
        if !position.is_on_canvas() {
            return Err(CanvasError::OutOfBounds);
        }
        element.position = position;
        let revision = self.advance();
        Ok(TableEvent::ElementMoved {
            element: id,
            revision,
        })
    }

    fn advance(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// Swaps in reloaded kinds. Elements that no longer fit stay on the canvas and are reported,
    /// so fixing a kind file never loses the scene built from it.
    pub fn replace_registry(
        &mut self,
        registry: KindRegistry,
        sink: &dyn EventSink,
    ) -> ReloadReport {
        self.registry = registry;
        let problems = self.problems();
        let revision = self.advance();
        sink.record(&TableEvent::KindsReloaded {
            kinds: self.registry.len(),
            invalid_elements: problems.len(),
            revision,
        });
        ReloadReport {
            kinds: self.registry.len(),
            problems,
        }
    }
}
