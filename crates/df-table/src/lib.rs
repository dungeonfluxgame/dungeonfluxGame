//! The table engine library: pure, synchronous and free of I/O, clocks and dice.
//!
//! `planning/gameplay-engine-redesign.md` section O is the plan. The crate holds two layers. The
//! engine foundations of step C0 (O.9): `outcome` (the four outcomes and the declines), `event`
//! (typed events and their audit lines), `identity` (ids and the `Correlation` every port call
//! takes), `private_data` (text and secrets that cannot reach a record), `ports` (the eight seams),
//! `config` (typed settings) and `testkit` (a fake per port). And the blank canvas workbench that
//! the first host slice drives: elements whose kinds are data, the effects that change them, and
//! the typed events that explain every change. Machines, the pipeline and projection are built on
//! the same pattern in later steps.
//!
//! How the built crate differs from section O, so nobody is surprised:
//!
//! - There is no `envelope`, `owner_queue`, `machine`, `registry`, `pipeline`, `pending`,
//!   `effects`, `table_state`, `projection` or `inspect` yet (C1 to C7).
//! - `CanvasEffect` is the first `TableEffect` (O.3 `effects`); it lives in `canvas.rs` until a
//!   second kind of table state needs the same pattern.
//! - The canvas events keep their bare `revision` counter. `SessionRevision`, `OperationId` and the
//!   other ids are local types in `identity`, shaped like their `df-types` namesakes; the
//!   `df-types` edge is added at C6 (question 87).
//! - `EventClass::Always` is class A of O.6 and `EventClass::Verbose` is class B. The canvas
//!   events carry no `Correlation`; the engine events do.
//! - `private_data` and `identity` are split from `event`, so no file mixes
//!   two jobs; O.3 lists both under `event`.
//! - Elements are entities in an entity-component world (`ecs`, `world`, `motion`), which section O
//!   does not describe; later machines are meant to add components and systems to it.
//! - The vocabulary says "canvas" where the plan says "table": a canvas is the dev workbench's
//!   table. The names converge when the first machine is built.
//!
//! Elements are entities in a small entity-component world: `ecs` is the generic machinery
//! (generational handles, dense component stores), `world` chooses the components (name, kind,
//! position, fields, and a motion while travelling) and keeps them consistent, and `motion` is
//! the system that moves travelling entities each tick. `canvas` turns an effect into a change of
//! the world and a typed event; a read leaves the world as an `Element` snapshot.
//!
//! Map of the crate, in reading order: `element` (names, positions, values), `field_type` (the
//! rule for a valid value), `kind_file` (the file format and its checking), `kind` (checked
//! definitions and the registry), `ecs`, `world` and `motion` (the entities and their travel),
//! `canvas` (effects, ticks and the revision), `identity`, `private_data`, `outcome`, `config`,
//! `event` and `event_field` (what is recorded and how it is rendered), `ports` (where it goes and
//! what the engine asks), `testkit` (fakes and sample data).

pub mod canvas;
pub mod config;
pub mod ecs;
pub mod element;
pub mod event;
pub mod event_field;
pub mod field_type;
pub mod identity;
pub mod kind;
pub mod kind_file;
mod listed;
pub mod motion;
pub mod outcome;
pub mod ports;
pub mod private_data;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod world;

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod canvas_tests;
#[cfg(test)]
mod canvas_travel_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod ecs_tests;
#[cfg(test)]
mod element_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod field_type_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod kind_tests;
#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod outcome_tests;
#[cfg(test)]
mod ports_tests;
#[cfg(test)]
mod private_data_tests;
#[cfg(test)]
mod testkit_tests;
#[cfg(test)]
mod world_tests;
