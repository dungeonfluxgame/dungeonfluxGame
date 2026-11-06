//! Every bound and timing value of the broker, in one place.
//!
//! The owner's rule is that there are no small invented caps: a bound is either stated by the plan
//! (and cited here) or a very large named constant to be scoped down when a measurement says so
//! (owner, 2026-11-02). Spend is never bounded: the ledger records and never blocks.

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

/// The deadline of an `interpret` request, in milliseconds: the lower end of the observed Codex
/// text latency (H.5), so a slower lane times out and the seat gets buttons.
pub const INTERPRET_DEADLINE_MS: u64 = 15_000;
/// The deadline of narration, scene description, dress and staging text, in milliseconds: the
/// upper end of the observed text latency (H.5).
pub const TEXT_DEADLINE_MS: u64 = 45_000;
/// The deadline of the current scene's background, in milliseconds: more than
/// `scene_build_budget_s` of J.4 so a slow build still lands late, and the table plays on the
/// establishing card meanwhile (K.2).
pub const BACKGROUND_DEADLINE_MS: u64 = 120_000;
/// The deadline of a plate or a portrait, in milliseconds: the upper end of the observed Codex
/// image latency, 90 to 240 s (H.4).
pub const IMAGE_DEADLINE_MS: u64 = 240_000;

/// How long a claimed attempt is leased without a heartbeat, in milliseconds (J.4: job lease 60 s,
/// H.9).
pub const LEASE_MS: u64 = 60_000;
/// How many times a request whose lease expired before its provider call was sent is put back in
/// its queue before it fails (H.9: three attempts).
pub const MAXIMUM_LEASE_ATTEMPTS: u8 = 3;

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
