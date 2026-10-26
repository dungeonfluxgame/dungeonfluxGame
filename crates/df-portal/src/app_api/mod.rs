//! The JSON API of the account web app under `/app/api/`, and the bundle it is served with at
//! `/app/` (coordinator decisions 28 and 29).
//!
//! The browser app (`df-account-app`) is the only client. This module authenticates through
//! `CustomerService`, guards every mutation (`guard`), and reaches everything it does not own
//! through consumer-owned ports (`ports`): finance (F), the processor checkout (B), campaigns
//! (S1), operators (B) and the review queue (O2). The server decides every permission; a
//! customer never receives an operator type, and customer replies have no field for a table
//! fee, a multiplier or a provider price (decision 26).

mod admin;
mod auth;
mod campaign;
mod customer;
mod ids;
mod oauth;
mod router;
mod session;
mod state;
mod static_files;

pub mod guard;
pub mod ports;

#[cfg(any(test, feature = "test-fakes"))]
pub mod fakes;

pub use router::{api_router, router};
pub use state::AppApiState;
pub use static_files::router_for_bundle;

/// Composes the account API with its bundle for a server that supplies real service adapters.
pub fn compose<S, M, P>(
    service: df_customer::CustomerService<S, M>,
    origin: crate::PortalOrigin,
    ports: std::sync::Arc<P>,
    bundle_directory: impl Into<std::path::PathBuf>,
) -> axum::Router
where
    S: df_customer::AccountStore,
    M: df_customer::EmailSender,
    P: ports::Ports,
{
    router(AppApiState::new(service, origin, ports), bundle_directory)
}

#[cfg(test)]
mod tests;
