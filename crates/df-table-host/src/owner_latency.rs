//! Spike S1 of O.16: does one owner thread hold six seats and a TV inside the H.5 budgets?
//!
//! The riskiest assumption of the design (L.7) is that per-audience projection and one
//! whole-checkpoint commit per input fit in one owner thread. This module measures that over fake
//! ports and made-up state, early, so a miss changes the design before more machines exist. It runs
//! the real `df_table::owner_queue::OwnerQueue` on one dedicated `std::thread`, feeds it from
//! producer threads, and for every served input pays what the design says an input costs: a stub
//! `step`, seven projections from an immutable snapshot, a checkpoint commit (serialise, hash,
//! store), and `finish`. Each phase is timed and summarised (`statistics`, `report`).
//!
//! It is a measurement tool, not product code: it reads `std::time::Instant`, which the pure crate
//! never does, and its workloads are stand-ins sized by `state::WorkloadProfile` and
//! `commit::Committer`, with the sizes stated in the report. Run it with
//! `cargo run --release -p df-table-host --example owner_latency -- help`. Rerun it after H0 and at
//! H3 against the real step and projection (O.16, S1): `real` is that rerun, over the real owner
//! actor and not over stand-ins.

pub mod commit;
pub mod error;
mod limits;
pub mod producers;
pub mod projection;
pub mod real;
pub mod report;
pub mod run;
pub mod sha256;
pub mod state;
pub mod statistics;
