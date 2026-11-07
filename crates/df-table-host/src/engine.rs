//! The owner actor of the table, over fake adapters, with its dev routes and command line (H0).
//!
//! `df-table` is pure; this is where its pieces run. One dedicated thread (`owner`) owns the real
//! `OwnerQueue`, the machine registry (the lobby and a minimal explore of C2) and the committed
//! table state. Inputs reach it through the queue's four lanes, run through the real machines with
//! the real ports backed by fake adapters (`adapters`: a scripted-style interpreter, rules owner,
//! narrator, seeded dice, the system clock and the broker of `crate::broker`), apply their effects,
//! and are saved by group commit (`commit`, the S1 decision). A view of the table comes from one
//! seam (`projection_adapter`) that the real projection of C5 replaces.
//!
//! The first playable path (H3) is the same machinery with a third machine: `setup::story_machines`
//! registers the lobby, the explore chained to the dialogue, and the dialogue, and
//! `EngineConfig::story` publishes each seat's standing buttons. `machine_io` serves the wire's
//! `MachineView` and takes its `MachineIntent` (`routes/play`, `cli/play`: the verbs `lobby`,
//! `scene start`, `wire` and `play`).
//!
//! The host owns every `.await`: `handle` is the bounded mailbox and the typed calls over it,
//! `routes` serves `/dev/engine/*` behind the host's guard, and `cli` parses and runs the verbs
//! `input`, `viable`, `dump`, `force`, `projection show`, `queue` and `describe` (plus the
//! broker's views) with the numbered answers of `format`. Every call is audited as one typed line
//! (`audit`), and every type that crosses the routes is in `report` and `request`.
//!
//! `turn` is the stand-in for the DM pipeline of C3: it runs one served input to its outcome with
//! the same shape the pipeline will have. Reading order: `setup`, `adapters`, `turn`, `commit`,
//! `owner` (with `submit`, `batch`, `reads`, `force`), `handle`, `routes`, `cli`.

pub mod adapters;
pub mod audit;
mod audit_fields;
mod broker_fields;
pub mod cli;
pub mod commit;
mod committer;
pub mod extras;
mod format;
mod fresh_id;
pub mod handle;
pub mod jev;
mod labels;
pub mod limits;
pub mod machine_io;
mod owner;
pub mod projection_adapter;
pub mod replay;
pub mod report;
pub mod request;
pub mod routes;
pub mod setup;
mod turn;
mod verb_set;

pub use self::broker_fields::SinkBrokerAudit;
pub use self::cli::play::{PlayCommand, parse_scene_start};
pub use self::cli::{EngineCommand, help_text, parse as parse_verb, run, verb_usages};
pub use self::extras::Extras;
pub use self::handle::EngineHandle;
pub use self::replay::{PendingReplay, ReplayError, Replayer};
pub use self::routes::router;

/// The verbs of the engine's command line, for the dispatch in `command::parse`.
pub use self::cli::VERBS;

#[cfg(test)]
mod adapters_tests;
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod machine_io_tests;
#[cfg(test)]
mod observe_tests;
#[cfg(test)]
mod owner_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod routes_tests;
