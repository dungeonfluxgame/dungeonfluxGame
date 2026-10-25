//! The DungeonFlux account web app (coordinator decisions 28 and 29): the free-play landing page,
//! sign up, verify, sign in, password reset, the customer dashboard, campaign pages and the
//! operator console, as one Rust app compiled to wasm32 and served by B at `/app/`.
//!
//! Everything but the browser glue is native Rust with native tests: routes, the state machine,
//! the element tree each screen renders to, money formatting and the API contract. The wasm32
//! runtime only patches the DOM, owns the listeners, timers and requests, and feeds messages back.
//! The server decides every permission; the app shows what the server returns and never holds a
//! token in JS-reachable storage (decision 28).
#![forbid(unsafe_code)]

pub mod api;
pub mod copy;
pub mod diagnostics;
pub mod entry;
pub mod money;
pub mod node;
pub mod patch;
pub mod route;
pub mod state;
pub mod view;
pub mod wire;
pub mod wire_campaign;

#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(test)]
mod style_tests;
