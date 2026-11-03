//! The look-ahead worker of SB-LA: while a scene plays, the next scene's description and background
//! are built through the broker on the lowest queue class, so the transition finds it Ready (idea 1
//! of "Ideas to come", O.16).
//!
//! Reading order: `worker` (the build, the broker requests and the lease and epoch checks, over
//! the pure planner in `df_table::lookahead`), `request` (what the dev routes carry), `bench` (the
//! workbench side: the expected scene, the scripted providers and the verbs' answers), `format` and
//! `cli` (the verbs `scene lookahead show|pause|resume|target|clear`).
//!
//! What the first version builds: the certain next scene's dress step (Describe) and Background. A
//! scene built ahead is adopted by the scene builder at the transition (`SceneBuilder::adopt`), so
//! its first plate starts at the Plate stage. What is not built: the plate and Locate stages of a
//! look-ahead build (they need S2, and a plate shows the cast at the transition, which is not known
//! earlier), weighted candidates for exits and routes (the planner ranks them and the worker builds
//! them, but no machine states them yet), and `lookahead_*` as entries of the settings list.

mod bench;
pub mod cli;
mod format;
pub mod request;
mod worker;

pub use self::bench::{BenchContext, LookaheadBench};
pub use self::cli::{LookaheadCommand, execute, parse};
pub use self::request::{LookaheadReply, LookaheadRequest};
pub use self::worker::LookaheadWorker;

#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod worker_tests;
