//! The scene builder, the plate scheduler's host side and the playout buffer (O.16 SB).
//!
//! A scene is built through the resource broker in five stages (D12 to D15): Describe stores the
//! description, Background makes the place with nobody in it, Plate makes the picture of the scene,
//! Locate finds each person and item in it, and Ready hands the plate and its anchors to the
//! playout buffer. Each stage is one kind of broker request with its own `PurposeSpec`, its output
//! is stored and audited, and the next stage starts only from a stored output. A result for a scene
//! that moved on is rejected (D.2). Waiting for art is never a gate: after `scene_build_budget_s`
//! play continues on the establishing card (K.2). The pure parts are in `df-table`: the stage plan,
//! Locate's policy, the card, the scheduler, the buffer and the cost view (`scene_plan*`,
//! `plate_*`); this module is where they meet the broker and the audit.
//!
//! Reading order: `purposes` (what each stage asks for), `inputs` and `answers` (what a request is
//! for and the schema of a Locate answer), `events` (what is recorded), `job_run` and `builder` (the
//! pipeline), `playout` and `pacer` (the clock and the screen), `report` (the read-only views),
//! `fakes` (the scripted provider worker), and for the developer surface `workbench`, `request`,
//! `format`, `cli` and `routes`.
//!
//! What is only a seam: the look-ahead worker of SB-LA is not built. `BuildPriority::Lookahead` and
//! the `lookahead_*` purposes are the seam, so a worker that opens the next scene's record at that
//! class can reuse everything here. No real provider is called: the only worker is the fake one.
//!
//! Where the plan and the code differ: Ready is the last step and asks the broker for nothing, so
//! "each stage is one request" holds for the four stages before it; Locate is one request for each
//! prompt (J.1), not one request. The J.4 settings of the scene builder are in `PlateTiming`
//! (`df-table`), not yet in the settings list.

mod answers;
mod builder;
pub mod cli;
mod dev_world;
mod events;
mod fakes;
mod format;
mod inputs;
mod job_run;
mod pacer;
mod playout;
mod purposes;
mod report;
pub mod request;
pub mod routes;
mod workbench;

pub use self::answers::{AnswerError, encode_locate_answer, parse_locate_answer};
pub use self::builder::{
    BuilderError, BuilderOutput, CardView, JobOutcome, SCENE_LEVEL_JOB, SceneBuilder,
};
pub use self::cli::{SceneBuilderCommand, execute, help_text, parse, parse_scene, verb_usages};
pub use self::events::{LocateCounts, SceneBuilderAudit, SceneBuilderEvent, StageSource, end_key};
pub use self::fakes::{
    DEFAULT_IMAGE_LATENCY_MS, DEFAULT_LOCATE_LATENCY_MS, DEFAULT_TEXT_LATENCY_MS, FakeStageWorker,
    PumpReport, StageScript, StartedCall,
};
pub use self::inputs::{MAXIMUM_REMEMBERED_INPUTS, StageInput, StageInputs};
pub use self::pacer::{PacerError, Pumped, ScenePacer};
pub use self::playout::Playout;
pub use self::purposes::{
    BuildPriority, LOCATE_DEADLINE_MS, LOCATE_PRICE_MICRO_DOLLARS, StagePurposes, cost_view,
    scene_purpose_table, scene_purposes, stage_uses,
};
pub use self::report::{
    AnchorReport, CardReport, FailureReport, JobReport, LocateReport, PointReport, SceneReport,
};
pub use self::routes::router;
pub use self::workbench::Workbench;

#[cfg(test)]
mod tests_support;

#[cfg(test)]
mod builder_tests;
#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod pacer_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod workbench_tests;
