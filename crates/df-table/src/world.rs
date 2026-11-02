//! The world: every element on the canvas as an entity with components, and the rules that keep
//! the components consistent.
//!
//! Components are `Name`, `Kind`, `Position` and `Fields`, which every element has, `Motion`,
//! which an element has while it travels, and the appearance components `Rotation` (a `Degrees`)
//! and `Pivot`, which an element has once it has been turned. A name is unique and is how everything outside the
//! world refers to an element; the entity handle never leaves this crate's internals. The world
//! owns the structure (who exists, who moves) and never judges whether a value is valid for a
//! kind: `canvas.rs` checks that before it changes the world.
//!
//! Reads that leave the world return an `Element`, a snapshot assembled from the components, so a
//! viewer never holds a borrow into the stores.

use std::collections::BTreeMap;

use crate::ecs::{Entities, Entity, Store};
use crate::element::{
    Appearance, Degrees, Element, ElementId, FieldName, FieldValue, KindName, Pivot, Position,
};
use crate::motion::{self, Motion};

/// The fields of one element.
pub type Fields = BTreeMap<FieldName, FieldValue>;

/// What one tick of motion did, in names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Travelled {
    pub moved: usize,
    pub arrived: Vec<ElementId>,
}

#[derive(Debug, Clone, Default)]
pub struct World {
    entities: Entities,
    names: Store<ElementId>,
    kinds: Store<KindName>,
    positions: Store<Position>,
    fields: Store<Fields>,
    motions: Store<Motion>,
    rotations: Store<Degrees>,
    pivots: Store<Pivot>,
    /// Name to entity, in name order. The one place a name is turned into a handle.
    by_name: BTreeMap<ElementId, Entity>,
}

impl World {
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn contains(&self, id: &ElementId) -> bool {
        self.by_name.contains_key(id)
    }

    /// Makes an element, or returns `None` when the name is taken.
    pub fn spawn(
        &mut self,
        id: ElementId,
        kind: KindName,
        position: Position,
        fields: Fields,
    ) -> Option<()> {
        if self.by_name.contains_key(&id) {
            return None;
        }
        let entity = self.entities.spawn();
        self.names.insert(entity, id.clone());
        self.kinds.insert(entity, kind);
        self.positions.insert(entity, position);
        self.fields.insert(entity, fields);
        self.by_name.insert(id, entity);
        Some(())
    }

    /// Removes an element with every component it has. Returns whether it existed.
    pub fn despawn(&mut self, id: &ElementId) -> bool {
        let Some(entity) = self.by_name.remove(id) else {
            return false;
        };
        self.names.remove(entity);
        self.kinds.remove(entity);
        self.positions.remove(entity);
        self.fields.remove(entity);
        self.motions.remove(entity);
        self.rotations.remove(entity);
        self.pivots.remove(entity);
        self.entities.despawn(entity);
        true
    }

    /// Removes everything and returns how many elements there were.
    pub fn clear(&mut self) -> usize {
        let removed = self.len();
        *self = Self::default();
        removed
    }

    pub fn kind(&self, id: &ElementId) -> Option<&KindName> {
        self.kinds.get(*self.by_name.get(id)?)
    }

    /// The fields of an element, to change one. Validity is the caller's to check.
    pub fn fields_mut(&mut self, id: &ElementId) -> Option<&mut Fields> {
        self.fields.get_mut(*self.by_name.get(id)?)
    }

    /// Puts an element at a position at once, ending any travel it was on.
    pub fn place(&mut self, id: &ElementId, position: Position) -> bool {
        let Some(&entity) = self.by_name.get(id) else {
            return false;
        };
        self.motions.remove(entity);
        self.positions.insert(entity, position);
        true
    }

    /// Sets an element travelling, replacing any travel it was on.
    pub fn start_travel(&mut self, id: &ElementId, motion: Motion) -> bool {
        let Some(&entity) = self.by_name.get(id) else {
            return false;
        };
        self.motions.insert(entity, motion);
        true
    }

    /// Ends an element's travel where it stands. Returns whether it was travelling.
    pub fn stop_travel(&mut self, id: &ElementId) -> Option<bool> {
        let entity = *self.by_name.get(id)?;
        Some(self.motions.remove(entity).is_some())
    }

    /// Turns an element to `rotation`, about `pivot` when one is given and about the pivot it had
    /// otherwise. Travel goes on: turning never moves the element.
    pub fn rotate(&mut self, id: &ElementId, rotation: Degrees, pivot: Option<Pivot>) -> bool {
        let Some(&entity) = self.by_name.get(id) else {
            return false;
        };
        self.rotations.insert(entity, rotation);
        if let Some(pivot) = pivot {
            self.pivots.insert(entity, pivot);
        }
        true
    }

    /// How many elements have been turned, so a test can see that a removal took the component.
    #[cfg(test)]
    pub(crate) fn rotation_count(&self) -> usize {
        self.rotations.len()
    }

    /// Whether any element is travelling, so a host can skip ticking an idle canvas.
    pub fn is_moving(&self) -> bool {
        !self.motions.is_empty()
    }

    /// Runs the motion system for `seconds` and names the elements that arrived.
    pub fn advance_motion(&mut self, seconds: f32) -> Travelled {
        let report = motion::advance(&mut self.positions, &mut self.motions, seconds);
        let mut arrived: Vec<ElementId> = report
            .arrived
            .iter()
            .filter_map(|entity| self.names.get(*entity).cloned())
            .collect();
        arrived.sort();
        Travelled {
            moved: report.moved,
            arrived,
        }
    }

    fn snapshot(&self, entity: Entity) -> Option<Element> {
        Some(Element {
            id: self.names.get(entity)?.clone(),
            kind: self.kinds.get(entity)?.clone(),
            position: *self.positions.get(entity)?,
            fields: self.fields.get(entity)?.clone(),
            motion: self.motions.get(entity).copied(),
            appearance: Appearance {
                rotation: self.rotations.get(entity).copied().unwrap_or_default(),
                pivot: self.pivots.get(entity).copied().unwrap_or_default(),
            },
        })
    }

    pub fn element(&self, id: &ElementId) -> Option<Element> {
        self.snapshot(*self.by_name.get(id)?)
    }

    /// Every element as a snapshot, in name order.
    pub fn elements(&self) -> impl Iterator<Item = Element> + '_ {
        self.by_name
            .values()
            .filter_map(|entity| self.snapshot(*entity))
    }
}
