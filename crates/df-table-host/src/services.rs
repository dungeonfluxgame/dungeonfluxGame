//! `services`: one read of how the host's media and text services are set up and how they stand.
//!
//! The host composes three services from its environment: the text surface (`text`), the picture
//! generator (`imagegen`) and the picture locator (`locate`), which asks identity questions of the
//! text surface. `routes::router` builds the text surface and the generator once and serves the
//! imagegen and locate routes over them, so `GET /dev/services` reports the same instances the
//! routes use. The report names backends, models, hosts, limits, counts and the variable to set
//! when a service is not configured; a secret is only ever reported as present or not.
//!
//! Configuration comes from the environment when the host starts and changes only with a restart,
//! so this is a read only. Reading order: `request` (the report and `ServicesError`), `routes`,
//! `cli` (the grammar and the client call) and `format` (what the verb prints).

pub mod cli;
pub mod format;
pub mod request;
pub mod routes;

pub use self::cli::{SERVICES_USAGE, Services, fetch, parse};
pub use self::format::format_services;
pub use self::request::{
    ImageMode, ImagegenReport, LocateReport, ServicesError, ServicesReply, ServicesReport,
    SpeechReport, TextReport,
};
pub use self::routes::{router, router_with};

#[cfg(test)]
mod services_tests;
