//! The picture generator: `media generate`, a dev verb that makes one picture from a prompt
//! through the image service's Codex CLI backend and stores it in the media library (O.8, O.16).
//!
//! The host owns every `.await`. The backend is blocking (a Codex turn takes minutes and its
//! future cannot move between runtime threads), so the service runs it in a blocking task under a
//! named deadline (`DEFAULT_DEADLINE`, 15 minutes) and the job is a task of its own: a caller
//! that stops waiting does not cancel it. `df-table` stays pure; the only crate that calls a
//! provider is `df-imagery`, through `df_imagery::oneshot`, and only from `backend`.
//!
//! Reading order: `model` (the request, the typed refusals, the answers), `backend` (the trait,
//! the Codex backend and its preconditions), `events` (what is recorded), `service` (the job, the
//! deadline, the storage), `routes` (`/dev/media/generate`) and `cli` (the command line's side).
//! The grammar of the verb is in `command::media_generate`.
//!
//! What it does not do: it makes one picture for one request and keeps no budget (the audit line
//! records and never blocks), it does not retry or fall back to another provider, it stores a PNG
//! as a PNG (no encoder is reachable from the host), and it does not choose or judge pictures.

pub mod backend;
pub mod cli;
pub mod events;
pub mod model;
pub mod routes;
pub mod service;

pub use self::backend::{
    CodexBackend, CodexSettings, ImageBackend, NotConfigured, backend_for, backend_from_environment,
};
pub use self::cli::run;
pub use self::events::ImageGenEvent;
pub use self::model::{
    Aspect, BackendError, BackendRequest, DEFAULT_DEADLINE, GenerateBody, GenerateReply,
    GeneratedImage, ImageRequest, JobState, JobView, Provenance, Purpose, RefusalCause,
    RequestProblem,
};
pub use self::routes::{router, router_with};
pub use self::service::{Context, ImageGenerator, Refusal, Started};

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod service_tests;
