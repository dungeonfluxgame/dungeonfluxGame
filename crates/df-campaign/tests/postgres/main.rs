//! Contract tests of the campaign store against the coordinator's scratch PostgreSQL (17 or later),
//! one fresh database per test, under the production role's privileges. The adapter lives in
//! `df-persistence`; until B declares it there, these tests compile it by path.
#![cfg(not(target_arch = "wasm32"))]
#![allow(
    dead_code,
    reason = "the adapter is compiled here by path and exports more than these tests use"
)]

#[path = "../../../df-persistence/src/account_closure.rs"]
mod account_closure;
#[path = "../../../df-persistence/src/campaign_store/mod.rs"]
mod campaign_store;

mod closure;
mod display;
mod feed;
mod fence;
mod fixture;
mod integrity;
mod membership;
mod races;
mod route_limits;
mod routes;
mod schema;
mod smoke;
mod timeouts;
mod transfers;
