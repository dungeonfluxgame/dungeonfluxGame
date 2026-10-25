//! Independent acceptance scenarios for the hosted launch (master launch plan, sections 15.4 and 17).
//!
//! This crate holds no production code. Its `tests/` run the plan's release gates against the
//! public contracts of the owning crates, with faithful fakes and controlled clocks. A scenario
//! whose owning API does not exist yet is written as a `blocked_scenario!`: it is `#[ignore]`d
//! with the missing API as the reason, and it fails loudly if someone runs it before it is
//! implemented. A scenario is never weakened to pass; it is replaced by a real test when its
//! owner lands the API.

mod allocation;
mod credit_owner;
mod fixtures;
mod member_directory;
mod provider_fixtures;
mod scenario;
mod spend_owner;

pub use allocation::assert_allocation_contract;
pub use credit_owner::{CreditApplied, CreditOwner};
pub use fixtures::{
    ControlledClock, dollars, empty_credit_state, operation, spend_consent,
    spend_snapshot_limited_by_job, usd_micros,
};
pub use member_directory::{DIRECTORY_CAMPAIGN, DIRECTORY_ROLE, DIRECTORY_TENANT, MemberDirectory};
pub use provider_fixtures::{
    checked_request, operation_context, request_binding, request_owner_state,
};
pub use spend_owner::SpendOwner;
