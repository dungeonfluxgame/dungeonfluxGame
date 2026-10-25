//! The finance views (D26, 18.9): `customer` is safe to show a table member by construction,
//! `operator` holds everything else. The two modules share no type and `customer` imports nothing
//! from `operator`.

pub mod customer;
pub mod operator;

#[cfg(test)]
mod customer_tests;
