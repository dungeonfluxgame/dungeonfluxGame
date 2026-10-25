//! df-docker-proxy: a body-checking Docker API proxy for the trusted runner (LAUNCH.md decision 18). The policy and the
//! HTTP framing are pure and unit-tested; the unix-socket server is in main.rs.
pub mod http;
pub mod policy;
pub mod strict;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_strict;
