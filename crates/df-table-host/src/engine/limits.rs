//! Every bound and timing value of the owner actor, in one place.
//!
//! The owner's rule is that there are no small invented caps: a bound is either stated by the plan
//! (and cited) or a very large named constant to be scoped down when a measurement says so (owner,
//! 2026-11-02). The lane caps of the queue itself are `df_table::owner_queue::QueueLimits`.

use std::time::Duration;

/// Commands that may wait for the owner thread before a caller is told it is busy. The mailbox is
/// the owner's inbox from the routes; the owner queue behind it holds the inputs. A caller that
/// finds it full is refused with a typed error and retries. Deliberately large.
pub const COMMAND_MAILBOX_CAPACITY: usize = 65_536;

/// Most turn reports kept for `queue show`; the oldest are forgotten first. Deliberately huge.
pub const MAXIMUM_RECENT_TURNS: usize = 100_000;
/// Most submissions (admitted or refused) kept for `queue show`; the oldest are forgotten first.
pub const MAXIMUM_RECENT_SUBMISSIONS: usize = 100_000;
/// Most turn or submission lines one `queue show` answer carries.
pub const MAXIMUM_LINES_PER_ANSWER: usize = 10_000;
/// Most broker ledger entries one `broker ledger` answer carries.
pub const MAXIMUM_LEDGER_ENTRIES_PER_ANSWER: usize = 10_000;
/// Most commit records a store remembers; the oldest are forgotten first. Deliberately huge;
/// scope it down later (owner, 2026-11-02).
pub const MAXIMUM_COMMIT_RECORDS: usize = 1_000_000;

/// Seats the dev engine joins to the owner queue at start (the authenticated session's members,
/// 1 to this number). The lobby then seats them at the table.
pub const DEV_SEAT_COUNT: u64 = 6;

/// The most inputs one commit may cover. Deliberately unbounded by count (S1 decision, 2026-11-02):
/// a batch is cut by the time slice, not by a number.
pub const MAXIMUM_BATCH_INPUTS: usize = usize::MAX;
/// The time slice that cuts a batch, in milliseconds: the owner stops collecting inputs into one
/// commit after this long. Deliberately large (five seconds), so a burst is one commit and not
/// several; to be tuned from measurement (S1 decision, owner 2026-11-02).
pub const BATCH_TIME_SLICE_MS: u64 = 5_000;

/// How long one commit may take before the batch is refused `table_busy`, in milliseconds
/// (`commit_deadline_ms`, S1 decision). A large default, to be tuned from measurement.
pub const DEFAULT_COMMIT_DEADLINE_MS: u64 = 30_000;
/// How long after a failed commit the owner refuses new input `table_busy` and starts no batch,
/// in milliseconds, so the queue does not pile up behind a stalled store (S1 decision).
pub const DEFAULT_COMMIT_BACKOFF_MS: u64 = 1_000;

/// How long the owner thread waits for a command before it ticks the broker and its watchdog.
pub const IDLE_TICK: Duration = Duration::from_millis(50);

/// The latency of a fake text call in the dev engine's broker, in milliseconds.
pub const DEV_TEXT_LATENCY_MS: u64 = 400;
/// The latency of a fake image call in the dev engine's broker, in milliseconds.
pub const DEV_IMAGE_LATENCY_MS: u64 = 2_000;

/// How long a call whose owner went away waits to learn why (a panic of the owner thread says so
/// within moments), before it reports only that the owner is gone.
pub const LOST_ANSWER_WAIT: Duration = Duration::from_secs(5);
