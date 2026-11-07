//! The resource broker: the rebuilt hub for text and image requests (H1, O.8, J.1).
//!
//! Every generated resource goes through here and nowhere else (coding style section 8, pattern 3):
//! one place keeps the queues, the in-flight map, the failure and retry rules, the spend record
//! and the fallback a purpose names. A request returns a handle at once and settles later, so the
//! owner thread never waits for a provider; readiness is polled with `status` or pushed with
//! `take_completions`. The hub does not spawn: the worker that runs provider calls claims work,
//! reports each call sent, and delivers answers (`providers` has the fakes and a pumping worker).
//!
//! Reading order: `purpose` (what a request is for), `request` (keys, epochs, statuses),
//! `queues` and `scheduler` (the scheduler), `ledger` (the spend record, which never blocks),
//! `events` (the typed record of every change), `hub` (the shared handle and the `Broker`
//! port), `providers` (the fakes) and `report` (the read-only views behind the dev verbs).
//!
//! Where the plan and the code differ: H.7 names four classes (`player_blocking`, `scene_build`,
//! `plate`, `prefetch`); this broker has the four of O.16 (live, plate, look-ahead, background),
//! where live is the first two and background is `prefetch`. Its events are host events
//! (`BrokerEvent`) because `TableEvent` is a closed enum of the pure crate.

pub mod events;
pub mod hub;
pub mod ledger;
pub mod limits;
pub mod providers;
pub mod purpose;
pub mod queues;
pub mod report;
pub mod request;
pub mod scheduler;

pub use self::events::{BrokerAudit, BrokerEvent, HubName};
pub use self::hub::BrokerHub;
pub use self::purpose::{Fallback, Price, PurposeTable, QueueClass, ResourceKind};
pub use self::queues::QueueBounds;
pub use self::request::{
    Epoch, NewRequest, ProviderCause, RequestKey, RequestStatus, ResourceResult,
};
pub use self::scheduler::{
    BrokerCore, BrokerLimits, BrokerLimitsError, CancelOutcome, ClaimedCall, CompleteOutcome,
    UnknownAttempt,
};

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod hub_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod purpose_tests;
#[cfg(test)]
mod queues_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod scheduler_tests;
