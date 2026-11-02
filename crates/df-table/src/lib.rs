//! The table engine library: pure, synchronous and free of I/O, clocks and dice.
//!
//! `planning/gameplay-engine-redesign.md` section O is the plan. This first slice is the blank
//! canvas workbench: elements whose kinds are data, the effects that change them, and the typed
//! events that explain every change. Machines, the pipeline and projection are built on the same
//! pattern in later steps.
//!
//! How this slice differs from section O, so nobody is surprised:
//!
//! - It is not step C0 of O.9 yet. There is no `outcome`, `envelope` or `owner_queue`; those come
//!   when the first machine needs them.
//! - `CanvasEffect` is the first `TableEffect` (O.3 `effects`); it lives in `canvas.rs` until a
//!   second kind of table state needs the same pattern.
//! - The revision is a bare counter. O names `SessionRevision` from `df-types`; this slice has no
//!   recovery epoch to put in it, so the type arrives with the session.
//! - `EventClass` has two values (always, verbose), not the plan's classes A and B, and an event
//!   has no `Correlation` or `OperationId` yet.
//! - Only one of the plan's eight ports exists: `EventSink`.
//! - The vocabulary says "canvas" where the plan says "table": a canvas is the dev workbench's
//!   table. The names converge when the first machine is built.
//!
//! Map of the crate, in reading order: `element` (names, positions, values), `field_type` (the
//! rule for a valid value), `kind_file` (the file format and its checking), `kind` (checked
//! definitions and the registry), `canvas` (the element set and its effects), `event` and
//! `ports` (what is recorded and where it goes), `testkit` (fakes and sample data).

pub mod canvas;
pub mod element;
pub mod event;
pub mod field_type;
pub mod kind;
pub mod kind_file;
pub mod ports;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

#[cfg(test)]
mod canvas_tests;
#[cfg(test)]
mod element_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod field_type_tests;
#[cfg(test)]
mod kind_tests;
