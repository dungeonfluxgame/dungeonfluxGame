//! The Director on the host (O.16 G1): the verbs `camera focus|wide` and `director ...`, the
//! Director's controlled clock and host tick, and the route that applies a direction.
//!
//! The decision is `df_table::director`, a pure stage that reads no clock and touches no file. This
//! module is where it meets the host: `workbench` owns the clock (real time plus what `director
//! advance` added) and builds the view of the shared screen the Director decides on, `routes` takes
//! a verb as JSON, lets the Director decide and applies each move through `apply_scene_change`, the
//! one path `POST /dev/playground/scene` also takes, so a dev verb and the engine make a camera
//! move with the same checked numbers and the same audit line. `request` is the wire, `format` the
//! text of `director show`, `cli` the grammar.
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
pub mod request;
pub mod routes;
mod workbench;

pub use self::cli::{DirectorCommand, execute, help_text, parse, verb_usages};
pub use self::request::{ActReply, DirectorAct, DirectorFault, DirectorReply, ShowReply};
pub use self::routes::{HOST_TICK_MS, router, router_for};
pub use self::workbench::{Decided, SHOWN_LOG_ENTRIES, Workbench, scene_change};

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod workbench_tests;
