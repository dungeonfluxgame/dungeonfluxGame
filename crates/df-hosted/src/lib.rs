//! Hosted account composition. Domain services retain authority; unavailable owner ports refuse.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

mod campaign_accounts;
mod campaign_funding;
mod campaign_sessions;
mod campaign_views;
mod campaigns;
mod checkout;
pub mod config;
mod database;
mod finance;
mod mail;
mod ports;

mod account_closure;
#[allow(
    dead_code,
    reason = "unchanged staged S1 adapter includes two unused private queries; remove when its owning crate exports it"
)]
#[path = "../../df-persistence/src/campaign_store/mod.rs"]
pub mod campaign_store;
mod runtime;

pub use runtime::{Hosted, OpenError};

#[cfg(test)]
mod journey_tests;
