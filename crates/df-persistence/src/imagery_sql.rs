//! Parameterized statements for the imagery operation journal: one place per query, taking
//! the caller's transaction so later catalog and finance composition can share it. Every method
//! of the public adapter owns one transaction and delegates here. Every statement asserts the
//! explicit `READ COMMITTED` builder; row locks carry the guarantees. No statement sleeps or
//! retries on its own.

mod rows;
mod statements;

pub(crate) use rows::record_failure;
pub(crate) use statements::{accept_in, freeze_in, lookup_in, read_in, withdraw_in};
