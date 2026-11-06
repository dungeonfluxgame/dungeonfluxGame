//! The Director feature's cadence and display bounds on the host.

/// How often the Director is woken while it has something to decide, in milliseconds.
pub const HOST_TICK_MS: u64 = 100;
/// Log entries `director show` prints, newest last. A display window, not a cap on the log: the
/// Director keeps its own log (`df-table`'s `maximum_log_entries`).
pub const SHOWN_LOG_ENTRIES: usize = 40;
