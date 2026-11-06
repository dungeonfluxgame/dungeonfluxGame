//! The Director on the host (O.16 G1): the verbs `camera focus|wide` and `director ...`, the
//! Director's controlled clock and host tick, and the route that applies a direction.
//!
//! The decision is `df_table::director`, a pure stage that reads no clock and touches no file. This
//! module is where it meets the host: `workbench` reads the injected clock (plus what `director
//! advance` added) and builds the view of the shared screen the Director decides on, `routes` takes
//! a verb as JSON, lets the Director decide and applies each move through `apply_scene_change`, the
//! one path `POST /dev/playground/scene` also takes, so a dev verb and the engine make a camera
//! move with the same checked numbers and the same audit line. `request` is the wire (the shared
//! dev envelope with `DirectorError`), `format` the text of `director show`, `cli` the grammar and
//! `limits` the tick and the show window.
//!
//! Where the plan and the code differ: the Director is in `df-table` and not in `df-presentation`
//! (the pure crate is where C6 and H0 are, and `df-presentation` is not a dependency of either); the
//! stand-in view is what `director view` edits, because no machine reports positions, speakers or
//! visibility yet (G2); a seat's `requested` reason and the engine's reasons are the same
//! `Request`, driven today only by `camera focus reason=`. `scene state`, `camera frame` and
//! `camera follow`, and the slide and cue classes of `director auto`, are not built (see the plan's
//! as-built note for G1). The serial in a `CAMERA` line is the Director's own count of moves; it
//! equals the playground's `camera_count` while the Director is the only thing driving the camera.

#[cfg(test)]
mod apply_hook;
pub mod cli;
mod format;
pub mod limits;
pub mod request;
pub mod routes;
mod workbench;

/// The old name of `run`, kept only because `dev.rs` (another owner's file) still calls it.
pub use self::cli::run as execute;
pub use self::cli::{DirectorCommand, help_text, parse, run, verb_usages};
pub use self::request::{ActReply, DirectorAct, DirectorError, DirectorReply, ShowReply};
pub use self::routes::{router, router_for};

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod workbench_tests;
