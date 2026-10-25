//! Support inbox contracts and ports; provider effects and persistence stay with their owners.
pub mod contracts;
#[cfg(feature = "fakes")]
pub mod fakes;
pub mod ports;

pub const RESERVED_SUPPORT_MIGRATION: u16 = 27;
