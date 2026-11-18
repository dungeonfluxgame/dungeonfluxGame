//! The look studio (`planning/phone-and-creation.md` P3, "The look studio"; REF-032 to REF-045,
//! REF-058, REF-059): a hero's look is refined step by step on one fixed pose, then locked in as
//! the hero's portrait and the three-view reference sheet `ref-<hero id>.png`.
//!
//! The owner makes and picks each body frame's base pose and its three-view sheet once
//! (`studio::pose_candidates`, `studio::choose_pose`). The phone draws the controls the studio file
//! defines (`catalog`), keeps the draft and sends it with each refine; Jev reads the player's own
//! words into details and judges each look before a picture is made (`jev`). A refine edits the
//! kept version with the base pose as a reference and asks only for what changed; the first renders
//! the whole description on the base pose. Versions and the lock-in are kept per seat (`store`).
//!
//! Reading order: `request` (the error and the dev bodies), `catalog`, `jev`, `store`, `studio`
//! (the work), `routes` and `cli`. Every step is one `LOOK` line: `pose_candidates`,
//! `pose_chosen`, `hero`, `details`, `held`, `refine`, `stale`, `failed`, `kept`, `discarded`,
//! `again`, `locked`, `refused`. A name or the player's words are never in one, and never in an
//! image prompt.

pub(crate) mod catalog;
pub mod cli;
pub(crate) mod jev;
pub mod request;
pub mod routes;
pub(crate) mod store;
pub(crate) mod studio;

pub use self::cli::{LOOK_USAGE, LookCommand, parse};
pub use self::studio::LookStudio;

#[cfg(test)]
mod look_tests;
