//! Scene transitions from player movement: "we walk to the town square", "we enter the hidden
//! door" and "we run down the hallway" become one typed path, [`types::MoveIntent`] to
//! [`types::SceneDesign`] to [`types::PlateBrief`] to a new plate (`planning/scene-transitions.md`).
//!
//! Reading order: `types` (the frozen shared contract), `interpret` (a line to a move),
//! `design` (a move to a scene design), `brief` (a design to the image prompt and character
//! references) and `transition` (the state of one transition, its deadlines and its records).
//!
//! Every stage is a pure function of its inputs so it can be run alone with a hand-written input,
//! and none of them calls a provider; the provider calls belong to the dev verbs and the host.

pub mod brief;
pub mod cli;
pub mod design;
pub mod interpret;
pub mod routes;
pub mod transition;
pub mod types;

#[cfg(test)]
mod brief_tests;
#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod design_tests;
#[cfg(test)]
mod interpret_tests;
#[cfg(test)]
mod transition_tests;
#[cfg(test)]
mod types_tests;
