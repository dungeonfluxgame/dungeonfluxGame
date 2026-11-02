//! A small entity-component core: entities are handles, components live in dense stores, and
//! systems (elsewhere) are plain functions over the stores.
//!
//! It owns only the generic machinery: handing out entities, noticing a stale handle, and storing
//! one kind of component per `Store`. It knows nothing about the canvas, its names or its kinds:
//! `world.rs` chooses the components and keeps the rules between them.
//!
//! A handle is an index plus a generation. When an entity is removed its index is reused with a
//! higher generation, so a handle kept from before can never read or change the new occupant.

use std::fmt;

/// A handle to one entity. Copyable and cheap; stale once the entity is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    fn slot(self) -> usize {
        self.index as usize
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}v{}", self.index, self.generation)
    }
}

/// Which entities exist. Indexes are reused, and each reuse bumps the generation.
#[derive(Debug, Clone, Default)]
pub struct Entities {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
    live: usize,
}

impl Entities {
    pub fn spawn(&mut self) -> Entity {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = index as usize;
            self.alive[slot] = true;
            return Entity {
                index,
                generation: self.generations[slot],
            };
        }
        // A slot is added only when none is free, so there are never more slots than the most
        // entities alive at once, and the canvas holds at most `MAXIMUM_ELEMENTS` (far below
        // 2^32). The caller checks that bound before it spawns.
        let index = u32::try_from(self.generations.len())
            .expect("the canvas bounds live entities far below 2^32");
        self.generations.push(0);
        self.alive.push(true);
        Entity {
            index,
            generation: 0,
        }
    }

    /// Removes the entity and returns whether it existed. A stale handle removes nothing.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let slot = entity.slot();
        self.alive[slot] = false;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        self.free.push(entity.index);
        self.live -= 1;
        true
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.alive.get(entity.slot()).copied().unwrap_or(false)
            && self.generations[entity.slot()] == entity.generation
    }

    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }
}

/// Every entity that has a component of type `T`, with the components stored densely so a system
/// that visits all of them walks one array.
#[derive(Debug, Clone)]
pub struct Store<T> {
    /// For each entity index, where its component is in `entities` and `items`.
    sparse: Vec<Option<u32>>,
    entities: Vec<Entity>,
    items: Vec<T>,
}

impl<T> Default for Store<T> {
    fn default() -> Self {
        Self {
            sparse: Vec::new(),
            entities: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl<T> Store<T> {
    fn position(&self, entity: Entity) -> Option<usize> {
        let dense = self.sparse.get(entity.slot()).copied().flatten()? as usize;
        (self.entities[dense] == entity).then_some(dense)
    }

    /// Gives the entity this component, returning the one it replaced.
    pub fn insert(&mut self, entity: Entity, item: T) -> Option<T> {
        if let Some(dense) = self.position(entity) {
            return Some(std::mem::replace(&mut self.items[dense], item));
        }
        if self.sparse.len() <= entity.slot() {
            self.sparse.resize(entity.slot() + 1, None);
        }
        // A store holds at most one component per entity, so it is as long as the live entities
        // that have it: below the bound of the line above.
        let dense = u32::try_from(self.items.len())
            .expect("a store holds at most one component per live entity, far below 2^32");
        self.sparse[entity.slot()] = Some(dense);
        self.entities.push(entity);
        self.items.push(item);
        None
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.position(entity).map(|dense| &self.items[dense])
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.position(entity).map(|dense| &mut self.items[dense])
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.position(entity).is_some()
    }

    /// Takes the component away from the entity. The last component moves into the gap, so the
    /// store stays dense.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let dense = self.position(entity)?;
        let last = self.items.len() - 1;
        self.items.swap(dense, last);
        self.entities.swap(dense, last);
        let item = self.items.pop();
        self.entities.pop();
        self.sparse[entity.slot()] = None;
        if dense != last {
            let moved = self.entities[dense];
            // `dense` is an index into the store, which is shorter than the bound above.
            self.sparse[moved.slot()] = Some(
                u32::try_from(dense).expect("a dense index is below the store length, below 2^32"),
            );
        }
        item
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.entities.iter().copied().zip(self.items.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.entities.iter().copied().zip(self.items.iter_mut())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.sparse.clear();
        self.entities.clear();
        self.items.clear();
    }
}
