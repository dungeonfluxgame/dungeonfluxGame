//! Plates: the pictures of a scene, from the tick that asks for one to the swap that shows it
//! (D16, D17, D18, K.2, K.3).
//!
//! `job` is one snapshot that asks for one picture, `scheduler` the clock that takes those
//! snapshots, `buffer` the playout that decides when a finished plate reaches the screen, `timing`
//! the J.4 numbers they share, `cost` what an hour of plates costs, and `digest` the deterministic
//! digest the scheduler and the host's request keys use.

pub mod buffer;
pub mod cost;
pub mod digest;
pub mod job;
pub mod scheduler;
pub mod timing;
