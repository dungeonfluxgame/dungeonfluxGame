//! The characters of the shown scene, for a chat or dialogue system: who is in the shot and where,
//! what each is doing (a cue), and the camera on one of them.
//!
//! Everything here works by character name against the shown scene's current layout (the
//! `df-scene-layout` record the `locate` feature stores, bound to the backdrop picture as it is
//! now). A name the layout does not have is refused with the names it does have; nothing is drawn
//! on the wrong character. A cue and a camera move go through the one scene path every scene change
//! takes (`playground::apply_scene_change`), so they are checked, recorded and drawn like the rest.
//!
//! - `GET /dev/scene/cast`: the `CastMember`s (name, region id, number, head, face, centre).
//! - `POST /dev/scene/cue`: a `CharacterCue` (`{"character":"elf","kind":"thinking"}`,
//!   `"speaking"` with `"line"`, or `"quiet"`).
//! - `POST /dev/scene/camera-on`: frame a character with a shot.
//!
//! Each answers a `CastReply` (the shared dev envelope with a typed `CastError`). The dev CLI's
//! `scene cast`, `scene cue` and `scene camera-on` are these routes; an engine sends the same JSON.

pub mod cli;
pub mod request;
pub mod routes;

pub use self::cli::{CastCommand, run};
pub use self::request::{CameraMoved, CameraRequest, CastList, Cued};
pub use self::routes::router;

#[cfg(test)]
mod cast_tests;
