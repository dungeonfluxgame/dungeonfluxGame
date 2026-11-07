//! Spans as the whole milliseconds reports, views and audit lines carry.

use std::time::Duration;

/// `span` in whole milliseconds, `u64::MAX` for a span too long for a `u64`.
pub fn saturating_millis(span: Duration) -> u64 {
    u64::try_from(span.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod duration_tests;
