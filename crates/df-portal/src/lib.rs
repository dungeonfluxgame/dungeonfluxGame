//! The customer portal: server-rendered HTML pages for the business site, sign up, log in,
//! email verification, password reset and the account pages (planning/account-service-research.md
//! sections 6 and 11). The account web app (`app_api`, the bundle at `/app/`) is mounted by
//! `router_with_app`.
//!
//! Owns HTTP, markup, cookies and CSRF only. Account rules live in `df-customer`; this crate has
//! no database, gateway or game types and never touches game state. No script is served: every
//! control is a link or a form (coordinator decision 8).
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

mod account_routes;
pub mod app_api;
mod auth_routes;
mod client_ip;
#[cfg(feature = "dev-routes")]
mod dev_routes;
pub mod html;
pub mod layout;
mod respond;
mod security;
mod site;
mod time_text;

use std::sync::Arc;

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    middleware,
    routing::{get, post},
};
#[cfg(feature = "dev-routes")]
use df_customer::DevOutbox;
use df_customer::{AccountStore, CustomerService, EmailSender};

pub use client_ip::{ClientIpRefusal, NetworkRefusal, TrustedProxies, client_ip};
pub use security::{ClientAddressPolicy, PRESESSION_COOKIE, SESSION_COOKIE};

/// Largest accepted form body (plan 6.1).
pub const MAXIMUM_FORM_BYTES: usize = 16 * 1024;

pub use df_customer::OriginRefusal;

/// The one canonical external origin links and form checks use (`bootstrap-origin-policy.md`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortalOrigin(df_customer::PublicOrigin);

impl PortalOrigin {
    /// Accepts `https://host[:port]`, or `http://` on `127.0.0.1`, `[::1]` or `localhost` for
    /// loopback development. Never derived from a request.
    pub fn parse(text: &str) -> Result<Self, OriginRefusal> {
        df_customer::PublicOrigin::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// What every handler shares.
pub struct PortalState<S, M> {
    service: CustomerService<S, M>,
    origin: Arc<PortalOrigin>,
    proxies: Arc<TrustedProxies>,
    /// Where a person lands after signing in.
    home: &'static str,
}

impl<S, M> Clone for PortalState<S, M> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            origin: Arc::clone(&self.origin),
            proxies: Arc::clone(&self.proxies),
            home: self.home,
        }
    }
}

impl<S, M> PortalState<S, M> {
    pub fn new(service: CustomerService<S, M>, origin: PortalOrigin) -> Self {
        Self {
            service,
            origin: Arc::new(origin),
            proxies: Arc::new(TrustedProxies::none()),
            home: "/account",
        }
    }

    /// The proxy networks whose `X-Forwarded-For` is believed. The default is none: the peer
    /// address of the connection is the client.
    pub fn with_trusted_proxies(mut self, proxies: TrustedProxies) -> Self {
        self.proxies = Arc::new(proxies);
        self
    }

    /// The address policy every router built from this state applies.
    pub fn client_address_policy(&self) -> ClientAddressPolicy {
        ClientAddressPolicy {
            proxies: (*self.proxies).clone(),
        }
    }

    /// Lands people on `path` after signing in (the campaign dashboard, `/app`, once mounted).
    pub fn with_home(mut self, path: &'static str) -> Self {
        self.home = path;
        self
    }
}

/// Public pages and the signed-in account pages.
fn shared_routes<S, M, T>() -> Router<T>
where
    S: AccountStore,
    M: EmailSender,
    T: Clone + Send + Sync + 'static,
    PortalState<S, M>: FromRef<T>,
{
    Router::new()
        .route("/", get(site::home))
        .route("/how-it-plays", get(site::how_it_plays))
        .route("/pricing", get(site::pricing))
        .route("/faq", get(site::faq))
        .route("/terms", get(site::terms))
        .route("/privacy", get(site::privacy))
        .route("/portal/{asset}", get(site::asset))
        .route(
            "/verify-email/resend",
            post(account_routes::resend_verification::<S, M>),
        )
        .route("/logout", post(account_routes::logout::<S, M>))
        .route("/account", get(account_routes::overview::<S, M>))
        .route(
            "/account/profile",
            get(account_routes::profile_form::<S, M>).post(account_routes::profile::<S, M>),
        )
        .route("/account/security", get(account_routes::security::<S, M>))
        .route("/account/sessions", get(account_routes::sessions::<S, M>))
        .route(
            "/account/sessions/revoke",
            post(account_routes::revoke_one::<S, M>),
        )
        .route(
            "/account/sessions/revoke-others",
            post(account_routes::revoke_others::<S, M>),
        )
        .route(
            "/account/sessions/revoke-all",
            post(account_routes::revoke_all::<S, M>),
        )
        .route("/account/data", get(account_routes::data::<S, M>))
        .route("/account/data/export", post(account_routes::export::<S, M>))
        .route("/account/close", post(account_routes::close::<S, M>))
}

fn finish<T: Clone + Send + Sync + 'static>(
    routes: Router<T>,
    state: T,
    policy: ClientAddressPolicy,
) -> Router {
    routes
        .layer(DefaultBodyLimit::max(MAXIMUM_FORM_BYTES))
        .layer(middleware::from_fn(security::headers_and_span))
        .layer(axum::Extension(policy))
        .with_state(state)
}

/// The server-rendered portal. It serves either credential source: the service decides
/// whether passwords are local or at the identity provider.
pub fn router<S: AccountStore, M: EmailSender>(state: PortalState<S, M>) -> Router {
    let routes = shared_routes::<S, M, PortalState<S, M>>()
        .route(
            "/signup",
            get(auth_routes::signup_form::<S, M>).post(auth_routes::signup::<S, M>),
        )
        .route(
            "/verify-email",
            get(auth_routes::verify_form::<S, M>).post(auth_routes::verify::<S, M>),
        )
        .route(
            "/login",
            get(auth_routes::login_form::<S, M>).post(auth_routes::login::<S, M>),
        )
        .route(
            "/password-reset",
            get(auth_routes::reset_request_form::<S, M>).post(auth_routes::reset_request::<S, M>),
        )
        .route(
            "/password-reset/confirm",
            get(auth_routes::reset_confirm_form::<S, M>).post(auth_routes::reset_confirm::<S, M>),
        )
        .route(
            "/email-change/confirm",
            get(auth_routes::email_change_form::<S, M>).post(auth_routes::email_change::<S, M>),
        )
        .route(
            "/account/security/password",
            post(account_routes::change_password::<S, M>),
        )
        .route(
            "/account/security/email",
            post(account_routes::change_email::<S, M>),
        );
    let policy = state.client_address_policy();
    finish(routes, state, policy)
}

/// The portal pages and the account web app: the JSON API under `/app/api/` and the bundle at
/// `/app/` (launch decision 28). The app is merged after the portal's layers, so the portal's
/// no-script policy covers only the portal's own pages. `/app/join` is the bundle's shell, with
/// no redirect of any kind, so an invitation link's fragment cannot be lost or leaked.
pub fn router_with_app<S, M, P>(
    state: PortalState<S, M>,
    ports: Arc<P>,
    bundle_directory: impl Into<std::path::PathBuf>,
) -> Router
where
    S: AccountStore,
    M: EmailSender,
    P: app_api::ports::Ports,
{
    let api = app_api::AppApiState::new(state.service.clone(), (*state.origin).clone(), ports);
    let policy = state.client_address_policy();
    router(state)
        .merge(app_api::router(api, bundle_directory))
        .layer(axum::Extension(policy))
}

/// The read-only development views (`dev accounts`, `dev outbox`), JSON. Composition merges them
/// into the existing loopback, token-guarded `/dev/*` surface; they carry no guard of their own.
#[cfg(feature = "dev-routes")]
pub fn dev_routes<S: AccountStore, M: EmailSender>(
    state: PortalState<S, M>,
    outbox: Option<Arc<DevOutbox>>,
) -> Router {
    dev_routes::router(state, outbox)
}

#[cfg(test)]
mod router_tests;

#[cfg(test)]
mod origin_tests {
    use super::*;

    #[test]
    fn https_and_loopback_http_origins_are_accepted() {
        for origin in [
            "https://play.example.test",
            "https://play.example.test:8443",
            "http://127.0.0.1:8099",
            "http://localhost:8099",
            "http://[::1]:8099",
        ] {
            assert!(PortalOrigin::parse(origin).is_ok(), "{origin}");
        }
    }

    #[test]
    fn plain_http_elsewhere_and_non_origins_are_refused() {
        assert_eq!(
            PortalOrigin::parse("http://192.168.1.20:8099"),
            Err(OriginRefusal::NotSecure)
        );
        for origin in [
            "https://play.example.test/",
            "https://play.example.test/account",
            "https://user@play.example.test",
            "play.example.test",
        ] {
            assert_eq!(
                PortalOrigin::parse(origin),
                Err(OriginRefusal::NotAnOrigin),
                "{origin}"
            );
        }
    }
}
