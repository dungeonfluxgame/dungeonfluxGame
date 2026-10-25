//! The hosted server's production composition: typed configuration, health and metrics, and the
//! ownership contracts the control plane, campaign runtime and egress dispatcher share.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

pub mod command_line;
#[allow(
    dead_code,
    reason = "ownership contract waiting for the production composition"
)]
pub(crate) mod composition_registry;
pub mod config;
#[allow(
    dead_code,
    reason = "ownership contract waiting for the production composition"
)]
pub(crate) mod deployment;
#[cfg(feature = "local-dev")]
pub mod environment;
pub mod exporter;
pub mod health;
pub mod metrics;
pub mod runtime_recovery;
pub mod serving;
#[allow(
    dead_code,
    reason = "ownership contract waiting for the production composition"
)]
pub(crate) mod shutdown_policy;

#[cfg(test)]
mod composition_registry_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod deployment_tests;
#[cfg(test)]
mod exporter_tests;
#[cfg(test)]
mod metrics_tests;
#[cfg(test)]
mod runtime_recovery_tests;
#[cfg(test)]
mod serving_tests;
#[cfg(test)]
mod shutdown_policy_tests;
