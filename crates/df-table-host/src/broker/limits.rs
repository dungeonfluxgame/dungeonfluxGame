//! Every bound and timing value of the broker, in one place.
//!
//! The owner's rule is that there are no small invented caps: a bound is either stated by the plan
//! (and cited here) or a very large named constant to be scoped down when a measurement says so
//! (owner, 2026-11-02). Spend is never bounded: the ledger records and never blocks.

use super::purpose::RetryPolicy;

/// Providers the text lane may run at once. Not measured and not in the plan: deliberately large,
/// to be scoped down when the lane's real concurrency is measured (owner, 2026-11-02).
/// `BrokerLimits` overrides it per hub.
pub const DEFAULT_TEXT_SLOTS: usize = 1_024;
/// Providers the image lane may run at once. Same status as `DEFAULT_TEXT_SLOTS`.
pub const DEFAULT_IMAGE_SLOTS: usize = 1_024;

/// The longest text answer a purpose accepts, in characters. The plan gives a size rule per
/// purpose and no number: deliberately huge, to be scoped down per purpose when measured (owner,
/// 2026-11-02).
pub const MAXIMUM_TEXT_ANSWER_CHARACTERS: u32 = 16_777_216;
/// The longest side of a picture answer, in pixels. Same status as the text maximum.
pub const MAXIMUM_IMAGE_ANSWER_EDGE_PIXELS: u32 = 65_536;
/// Text slots only the live class may use (H.7: at least one text slot is reserved for the
/// player-blocking class).
pub const RESERVED_LIVE_TEXT_SLOTS: usize = 1;

/// Most live requests waiting. The live class cannot drop (H.7), so a full queue refuses the new
/// request with a typed error. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_QUEUED_LIVE: usize = 1_000_000;
/// Most plate requests waiting; the oldest is dropped past it (J.4 and H.7: plate 4). It is the
/// engine's own plate queue bound, so the broker and the scheduler cannot disagree.
pub const MAXIMUM_QUEUED_PLATES: usize = df_table::plate::scheduler::MAXIMUM_QUEUED_JOBS;
/// Most look-ahead requests waiting; the oldest is dropped past it. The plan names no bound for
/// the class SB-LA adds (its `lookahead_depth` is a setting). Deliberately huge; scope it down
/// later (owner, 2026-11-02).
pub const MAXIMUM_QUEUED_LOOKAHEAD: usize = 1_000_000;
/// Most background (prefetch) requests waiting; the oldest is dropped past it (J.4: prefetch 8).
pub const MAXIMUM_QUEUED_BACKGROUND: usize = 8;

/// The default deadline of every provider call, in milliseconds. The plan cites observed
/// latencies (H.5 text 15 to 45 s, H.4 pictures 90 to 240 s) and scene-build budgets (J.4, K.2);
/// these defaults are deliberately far above them so a deadline never gets in the way of dev work,
/// and the two variables below scope them down (owner, 2026-11-06).
pub const DEFAULT_DEADLINE_MS: u64 = 3_600_000;
/// The variable that sets the deadline of the text purposes (`interpret`, narration, scene
/// description, dress and staging text), in whole seconds above zero.
pub const TEXT_DEADLINE_SECONDS_VARIABLE: &str = "DF_BROKER_TEXT_DEADLINE_SECS";
/// The variable that sets the deadline of the picture purposes (backgrounds, plates and
/// portraits), in whole seconds above zero.
pub const IMAGE_DEADLINE_SECONDS_VARIABLE: &str = "DF_BROKER_IMAGE_DEADLINE_SECS";

/// The deadline, in milliseconds, of the text purposes: `DF_BROKER_TEXT_DEADLINE_SECS` when a
/// whole number of seconds above zero, else `DEFAULT_DEADLINE_MS`.
pub fn text_deadline_ms() -> u64 {
    deadline_ms(crate::environment::text(TEXT_DEADLINE_SECONDS_VARIABLE).as_deref())
}

/// The deadline, in milliseconds, of the picture purposes: `DF_BROKER_IMAGE_DEADLINE_SECS` when a
/// whole number of seconds above zero, else `DEFAULT_DEADLINE_MS`.
pub fn image_deadline_ms() -> u64 {
    deadline_ms(crate::environment::text(IMAGE_DEADLINE_SECONDS_VARIABLE).as_deref())
}

/// The deadline a setting's text asks for, in milliseconds; no text, or text that is not a whole
/// number of seconds above zero, is `DEFAULT_DEADLINE_MS`.
pub(super) fn deadline_ms(text: Option<&str>) -> u64 {
    text.and_then(|text| text.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(DEFAULT_DEADLINE_MS, |seconds| seconds.saturating_mul(1_000))
}

/// How long a claimed attempt is leased without a heartbeat, in milliseconds (J.4 and H.9 give
/// 60 s; the default is far above it so a slow dev call is not requeued, owner, 2026-11-06).
pub const LEASE_MS: u64 = 3_600_000;
/// How many times a request whose lease expired before its provider call was sent is put back in
/// its queue before it fails (H.9 gives three; the default is far above it, owner, 2026-11-06).
pub const MAXIMUM_LEASE_ATTEMPTS: u32 = 1_000;

/// The retries of H.8: two, with a backoff that starts at half a second and stops at eight.
pub const DEFAULT_RETRY: RetryPolicy = RetryPolicy {
    maximum_retries: 2,
    initial_backoff_ms: 500,
    maximum_backoff_ms: 8_000,
};
/// A purpose whose result a person waits for at once gets no retry past the first (C.7: one
/// retry, then buttons).
pub const SINGLE_RETRY: RetryPolicy = RetryPolicy {
    maximum_retries: 1,
    initial_backoff_ms: 250,
    maximum_backoff_ms: 2_000,
};
/// J.4: SAM retries are one call retry. The backoff is the broker's short one for a text-lane call.
pub const LOCATE_RETRY: RetryPolicy = RetryPolicy {
    maximum_retries: 1,
    initial_backoff_ms: 500,
    maximum_backoff_ms: 2_000,
};

/// Most ledger entries kept; the oldest are forgotten first and the per-purpose totals survive.
/// Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_LEDGER_ENTRIES: usize = 1_000_000;
/// Most settled requests kept for their owner to read; the oldest are forgotten first, and a
/// forgotten handle reads as stale. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_KEPT_SETTLED: usize = 1_000_000;
/// Most completions waiting to be pushed to the owner; the oldest are dropped first and counted.
/// Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_PENDING_COMPLETIONS: usize = 1_000_000;
/// Most abandoned attempts remembered, so a late result is recognised and rejected. Deliberately
/// huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_REMEMBERED_ABANDONED: usize = 1_000_000;
/// Most scheduled provider calls a fake provider holds. Deliberately huge.
pub const MAXIMUM_SCHEDULED_CALLS: usize = 1_000_000;
