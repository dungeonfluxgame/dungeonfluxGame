//! Native integration composition. Account and campaign predicates stay in durable adapters.
mod reads;
pub(crate) use reads::Reads;
mod shares;
pub(crate) use shares::{Cleanup, Shares};

#[cfg(test)]
mod tests;

mod settings;
pub(crate) use settings::{Settings,share_tombstone_days,share_public_policy};
