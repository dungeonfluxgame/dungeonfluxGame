//! The live pages of a seated phone in play beyond Story and Act: the Bag, Talk, the Notebook, the
//! Map, Look closer and the paintings (`planning/phone-and-creation.md`, "The painter's day from
//! the phone", P4 "Moving by map", "The notebook" and "All panels").
//!
//! `wire` is what the host's seat pages route answers and what the phone sends it; `screens` turns
//! that answer and the seat's view into the phone kit's panels, so native tests cover every
//! decision and the browser only draws them. A page decides no rule: what a tap means is a typed
//! intent the host checks against ids (a suggested card's index, a line's index, a thing's id), and
//! what a sent line means is Jev's to read.
//!
//! Beside them: `creation` and `build`, the hero build after Lock in (P3: the creation routes, and
//! the quick hero, its review sheet and each part to go deeper), and `sheet`, the Hero and Roll
//! pages drawn from the hero's derived sheet.

pub mod build;
pub mod creation;
pub mod screens;
pub mod sheet;
pub mod wire;

#[cfg(test)]
mod build_tests;
#[cfg(test)]
mod pages_tests;
