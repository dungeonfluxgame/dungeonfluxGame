//! The WorkOS User Management adapter for the account service (launch decision 28): hosted
//! production keeps passwords at WorkOS, called from our server; the browser never visits WorkOS
//! and no WorkOS token is kept. `WorkOsIdentity` implements `df_customer::IdentityProvider` over
//! an `HttpTransport`; `ReqwestTransport` is the real one and `FakeWorkOsServer` the in-memory
//! WorkOS the tests use. No test or build step here calls WorkOS.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

mod adapter;
#[cfg(any(test, feature = "test-fakes"))]
pub mod fake;
mod transport;

pub use adapter::{ApiKey, ConfigRefusal, WorkOsConfig, WorkOsIdentity};
pub use transport::{
    HttpRequest, HttpResponse, HttpTransport, MAXIMUM_RESPONSE_BYTES, Method, PRODUCTION_HOST,
    ReqwestTransport, TransportError, WorkOsEndpoint,
};

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod service_tests;
