//! B's guard: who is calling.

use std::{future::Future, net::IpAddr};

use df_types::AccountId;
use http::request::Parts;

use super::table::AuthClass;

/// Who is calling. `account` is set only for session routes; `client_ip` comes from B's trusted
/// proxy chain, never from the socket peer.
#[derive(Clone, Copy, Debug)]
pub struct Caller {
    pub account: Option<AccountId>,
    pub client_ip: IpAddr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardRefusal {
    Unauthenticated,
    CsrfRefused,
    Unavailable,
}

/// B's session and CSRF layer, for the routes that need it.
///
/// TODO(S1-06): the real guard validates the web session cookie and the CSRF token (or the
/// pre-session token for `PublicWithPresessionCsrf`) and resolves the client address through the
/// trusted-proxy chain. The test guard reads a header.
pub trait Guard: Send + Sync + 'static {
    fn caller(
        &self,
        parts: &Parts,
        class: AuthClass,
    ) -> impl Future<Output = Result<Caller, GuardRefusal>> + Send;
}
