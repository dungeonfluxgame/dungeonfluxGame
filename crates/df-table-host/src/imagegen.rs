//! The picture generator: `media generate`, a dev verb that makes one picture from a prompt
//! through the image service's Codex CLI backend and stores it in the media library (O.8, O.16).
//!
//! The host owns every `.await`. The backend is blocking (a Codex turn takes minutes and its
//! future cannot move between runtime threads), so the service runs it in a blocking task under a
//! named deadline (`DEFAULT_DEADLINE`, 15 minutes) and the job is a task of its own: a caller
//! that stops waiting does not cancel it. `df-table` stays pure; the only crate that calls a
//! provider is `df-imagery`, through `df_imagery::oneshot`, and only from `backend`.
//!
//! Reading order: `limits` (every bound), `model` (the purpose, the shape and the checked request),
//! `backend` (the trait, the Codex backend, its preconditions and its typed errors), `request`
//! (what crosses the route: the body, the jobs and `GenerateError`), `events` (what is recorded),
//! `service` (the job, the deadline, the storage), `routes` (`/dev/media/generate`), `cli` (the
//! grammar of the verb and its sending) and `format` (what it prints).
//!
//! What it does not do: it makes one picture for one request and keeps no budget (the audit line
//! records and never blocks), it does not retry or fall back to another provider, it stores a PNG
//! as a PNG (no encoder is reachable from the host), and it does not choose or judge pictures.

pub mod backend;
pub mod cli;
pub mod events;
pub(crate) mod format;
pub mod limits;
pub mod model;
pub mod request;
pub mod routes;
pub mod service;

pub use self::backend::{
    BackendError, BackendLimits, CodexBackend, CodexSettings, ImageBackend, SwitchedOff,
    backend_for, backend_from_environment, backend_of,
};
pub use self::cli::{MediaGenerate, run};
pub use self::events::ImageGenEvent;
pub use self::limits::{DEFAULT_DEADLINE, MAXIMUM_NAME_CHARS};
pub use self::model::{
    Aspect, AspectError, BackendRequest, GenerateRequestError, GeneratedImage, ImageRequest,
    Priority, Purpose, ThingReference,
};
pub use self::request::{
    BackendSwitch, BackendView, GenerateError, GenerateReply, GenerateRequest, JobState, JobView,
    PictureCause, Provenance, Sidecar,
};
pub use self::routes::{router, router_with};
pub use self::service::{ImageGenerator, Started};

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod service_tests;
