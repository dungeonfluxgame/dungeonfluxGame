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
//! - `inspect` and `replay` (C7) are built: a report per stateful module for one audience, the verb
//!   set of the CLI, and recorded sessions with a replay runner. `pipeline` (C3) and `pending` (C4)
//!   are built. `envelope` and
//!   `owner_queue` are built (C1). `machine` (with `machine_spec` and `machine_slot`), `registry`, `effects`, the
//!   `lobby` and `explore` machines and the `conformance` suite are built (C2). `table_state`
//!   (C6) holds the applier and the table's facts, and `effects` re-exports `TableState` so the
//!   machines' imports did not change. `projection` (C5) turns an immutable snapshot and the
//!   owners' attachments into one view per audience.
//! - `pipeline` is a set of stages over a `PipelineTable` that the host owns, not a pipeline that
//!   keeps state: `Pipeline::run` changes the table in memory for one input and records nothing
//!   visible but the timing of its stages and its failures, so a batch of inputs is a way of
//!   grouping commits (O.16 S1). What would be visible is staged in the `TurnResult`: the lines
//!   that say what the input came to and asked of the ports (a die drawn, a decline, a
//!   clarification) and the lines that say what it changed, which the host releases with
//!   `TurnResult::release_events` after its durable commit. The host then calls
//!   `Pipeline::issue_requests` (broker requests under the correlation of the input that asked,
//!   and the reservations a cancelled pending gave up, told to the rules owner) and
//!   `Pipeline::attach_narration` (the narrator is a job the host runs, never a wait of the
//!   pipeline). A batch the host refuses is undone by `PipelineTable::checkpoint` and by giving
//!   its dice back (`TurnResult::return_draws`, newest turn first): a draw is provisional until
//!   the commit. A `TurnRecord` carries what the turn did and no commit identity; the host stamps
//!   that (`PipelineTable::stamp_commit_revision`), and the table's digest leaves the stamps out,
//!   so a batch of N inputs and N batches of one are the same table. `pending` holds the open
//!   pendings, windows and keyed rulings the table keeps; every pending holds the machine on
//!   `Waiting{pending_id}`. `golden` is the runner of the golden sessions (`World` is the
//!   scripted fakes and `SceneMachine` the machine they play in).
//! - `CanvasEffect` is the workbench's own effect and lives in `canvas.rs`. `effects::TableEffect`
//!   is the engine's, for the facts the whole table shares; the two do not share an applier yet.
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
//! what the engine asks), `effects`, `machine_spec`, `machine_slot`, `machine`, `registry`, `lobby`
//! and `explore` (the shared language of change and the machines), `table_state` and `projection`
//! (the table's facts and what each audience sees), `pipeline` and `pending` (an input's seven
//! stages and the questions that wait for answers), `testkit` (fakes and sample data), `golden`
//! (scripted worlds and golden sessions), `inspect` (reports and the verb set), `replay` (recorded
//! sessions) and `conformance` (the suite a machine must pass).

pub mod canvas;
pub mod config;
#[cfg(any(test, feature = "testkit"))]
pub mod conformance;
pub mod dialogue;
pub mod director;
pub mod director_error;
pub mod director_log;
pub mod director_reason;
pub mod director_view;
pub mod ecs;
pub mod effects;
pub mod element;
pub mod envelope;
pub mod event;
mod event_cause;
pub mod event_field;
pub mod explore;
pub mod field_type;
#[cfg(any(test, feature = "testkit"))]
pub mod golden;
pub mod identity;
pub mod inspect;
pub mod kind;
pub mod kind_file;
pub mod limits;
mod listed;
pub mod lobby;
pub mod machine;
pub mod machine_slot;
pub mod machine_spec;
pub mod motion;
pub mod outcome;
pub mod owner_queue;
pub mod pending;
pub mod pipeline;
pub mod plate_buffer;
pub mod plate_cost;
pub mod plate_digest;
pub mod plate_job;
pub mod plate_scheduler;
pub mod plate_timing;
pub mod ports;
pub mod private_data;
pub mod projection;
pub mod registry;
pub mod replay;
pub mod scene_plan;
pub mod scene_plan_brief;
pub mod scene_plan_card;
pub mod scene_plan_locate;
pub mod table_state;
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
mod conformance_tests;
#[cfg(test)]
mod dialogue_tests;
#[cfg(test)]
mod director_defect_tests;
#[cfg(test)]
mod director_pace_tests;
#[cfg(test)]
mod director_tests;
#[cfg(test)]
mod ecs_tests;
#[cfg(test)]
mod effects_tests;
#[cfg(test)]
mod element_tests;
#[cfg(test)]
mod envelope_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod explore_tests;
#[cfg(test)]
mod field_type_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod inspect_redaction_tests;
#[cfg(test)]
mod inspect_tests;
#[cfg(test)]
mod kind_tests;
#[cfg(test)]
mod lobby_tests;
#[cfg(test)]
mod machine_scan_tests;
#[cfg(test)]
mod machine_tests;
#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod outcome_tests;
#[cfg(test)]
mod owner_queue_churn_tests;
#[cfg(test)]
mod owner_queue_control_tests;
#[cfg(test)]
mod owner_queue_lease_tests;
#[cfg(test)]
mod owner_queue_property_tests;
#[cfg(test)]
mod owner_queue_tests;
#[cfg(test)]
mod pending_digest_tests;
#[cfg(test)]
mod pending_tests;
#[cfg(test)]
mod pending_window_tests;
#[cfg(test)]
mod pipeline_batch_tests;
#[cfg(test)]
mod pipeline_failure_tests;
#[cfg(test)]
mod pipeline_fixtures_tests;
#[cfg(test)]
mod pipeline_golden_tests;
#[cfg(test)]
mod pipeline_review_tests;
#[cfg(test)]
mod pipeline_sender_scope_tests;
#[cfg(test)]
mod pipeline_staging_tests;
#[cfg(test)]
mod pipeline_surface_tests;
#[cfg(test)]
mod pipeline_tests;
#[cfg(test)]
mod plate_buffer_tests;
#[cfg(test)]
mod plate_cost_tests;
#[cfg(test)]
mod plate_fixtures_tests;
#[cfg(test)]
mod plate_scheduler_tests;
#[cfg(test)]
mod ports_tests;
#[cfg(test)]
mod private_data_tests;
#[cfg(test)]
mod projection_attachments_tests;
#[cfg(test)]
mod projection_cost_tests;
#[cfg(test)]
mod projection_privacy_tests;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod projection_wire_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod replay_session_tests;
#[cfg(test)]
mod replay_tests;
#[cfg(test)]
mod scene_plan_card_tests;
#[cfg(test)]
mod scene_plan_locate_tests;
#[cfg(test)]
mod scene_plan_tests;
#[cfg(test)]
mod standing_offers_tests;
#[cfg(test)]
mod table_state_digest_tests;
#[cfg(test)]
mod table_state_lanes_tests;
#[cfg(test)]
mod table_state_rules_tests;
#[cfg(test)]
mod table_state_snapshot_tests;
#[cfg(test)]
mod table_state_tests;
#[cfg(test)]
mod testkit_tests;
#[cfg(test)]
mod tick_machine_tests;
#[cfg(test)]
mod world_tests;
