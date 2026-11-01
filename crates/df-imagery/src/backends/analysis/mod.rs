//! Private prepared analysis requests; service stages retain all execution authority.

pub(crate) mod photo;
pub(crate) mod standalone;
mod transport;

pub(crate) use transport::{InspectionInput, PreparedInspection};

#[cfg(test)]
pub(crate) mod testing;
