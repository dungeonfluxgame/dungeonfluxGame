//! What the page does first, in an order that keeps emailed and invited tokens out of every log.
//!
//! An invitation link (`/app/join#token=...`), a verification link and a reset link carry a secret
//! in the address bar. `boot` reads the location once, parses the route (the token now lives in a
//! local value), replaces the address with the token-free path, and only then installs the panic
//! hook and starts the router. Nothing that logs, reports an error or reads `location.href` exists
//! before the replacement (S1 8.2; B 7).

use crate::{route::Route, state::Msg};

/// The address as the browser reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Location {
    pub path: String,
    /// Without the leading `?`.
    pub query: String,
    /// Without the leading `#`.
    pub fragment: String,
}

/// The browser, as `boot` needs it. The wasm runtime implements it with `window`; the tests with a
/// recorder, so the order of the calls is checked natively.
pub trait Host {
    fn location(&self) -> Location;
    fn offset_east_minutes(&self) -> i32;
    /// `history.replaceState` with this path.
    fn replace_state(&mut self, path: &str);
    fn install_panic_hook(&mut self);
    /// Starts the router with the first message.
    fn start_router(&mut self, started: Msg);
}

/// The address to leave in the bar: the token-free route path, or the path as typed when the route
/// is unknown (so a mistyped address is not silently rewritten).
fn clean_address(location: &Location, route: &Route) -> String {
    match route {
        Route::NotFound => location.path.clone(),
        route => route.path(),
    }
}

pub fn boot(host: &mut impl Host) {
    let location = host.location();
    let route = Route::parse(&location.path, &location.query, &location.fragment);
    if !location.fragment.is_empty() || route.carries_token() || route == Route::GoogleWelcome {
        host.replace_state(&clean_address(&location, &route));
    }
    host.install_panic_hook();
    host.start_router(Msg::Started {
        route,
        offset_east_minutes: host.offset_east_minutes(),
    });
}

#[cfg(test)]
mod entry_tests;
