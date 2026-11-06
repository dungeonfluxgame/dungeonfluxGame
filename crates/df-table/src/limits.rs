//! The crate's size, count and time bounds, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant, memory stays bounded by design, and a value is scoped down later when a measurement
//! says so. Most of these are sizes nobody has measured; a bound the plan or the wire gives (the
//! hand-off depth, the phone's lines, the wire frame) is here too and says where its number comes
//! from. The ranges of a value's own type (a rotation of 0 to 359 degrees, a share of 0 to 100
//! percent, the 100 by 56.25 canvas) and defaults that bound nothing are not here: they live with
//! the type or the code that needs them.

/// Most elements the canvas holds at once. The playground sends every element in every frame, so
/// a frame is as large as this times an element; delta frames come before this is lowered or
/// reached.
pub(crate) const MAXIMUM_ELEMENTS: usize = 100_000;
/// Longest name (of an element, kind, field or choice), in characters. Names are ASCII.
pub(crate) const MAXIMUM_NAME_LENGTH: usize = 256;
/// Longest text value of a field, in characters, when its kind does not say less with its own
/// `maximum_length`.
pub(crate) const MAXIMUM_TEXT_LENGTH: usize = 65_536;
/// Most fields one kind has.
pub(crate) const MAXIMUM_FIELDS_PER_KIND: usize = 4_096;
/// Most choices one choice field has.
pub(crate) const MAXIMUM_CHOICES: usize = 4_096;
/// Fastest travel accepted, in canvas units per second. A speed need only be finite and above
/// zero (a rule of the domain); this only keeps one step finite and far from overflow.
pub(crate) const MAXIMUM_SPEED: f32 = 1_000_000.0;

/// Longest tick the travel system advances, in seconds (a safety rule, not an invented cap). A host
/// that stalled and wakes up must not carry every element across the canvas in one step, so a
/// longer gap is cut to this and the rest of the time is discarded: travel is a little late
/// after a stall, never early or teleported. The host's clock does not count the time it sleeps
/// while nothing moves as a stall.
pub(crate) const MAXIMUM_TICK_SECONDS: f32 = 0.25;

/// Most nested waits one pending resolution may go through (the pipeline). Deliberately huge so it
/// never limits play; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_PENDING_NESTING: u32 = 1_000_000;
/// Most rounds of more-dice-please one resolution may take, besides the `decide_deadline_s` of the
/// clock (a count alone cannot bound a loop that calls out). Deliberately huge.
pub const MAXIMUM_DRAW_ROUNDS: u32 = 1_000_000;
/// Most resource requests a table remembers an answer for; the oldest is forgotten first.
pub const MAXIMUM_OUTSTANDING_REQUESTS: usize = 1_000_000;
/// Longest narration, in characters, that is qualified. Deliberately huge, like the rest: the
/// phone's own line limit (280 characters) is a presentation rule and not this bound.
pub const MAXIMUM_NARRATION_CHARS: usize = 1_000_000;
/// Most pendings a table holds open at once. A full book refuses a new pending.
pub(crate) const MAXIMUM_OPEN_PENDINGS: usize = 1_000_000;
/// Most rulings a table remembers; the oldest is forgotten first.
pub(crate) const MAXIMUM_STORED_RULINGS: usize = 1_000_000;
/// Most committed operations whose result a replay may find again.
pub(crate) const MAXIMUM_REMEMBERED_RESULTS: usize = 1_000_000;

// The owner queue.

/// The most seats one queue tracks. Deliberately huge so it never limits play; scope it down later
/// (owner, 2026-11-02).
pub(crate) const MAXIMUM_SEATS: usize = 1_000_000;

/// The most inputs the engine lane holds. The plan gives no number: J.4 leaves the lane caps to the
/// first run over real adapters. Deliberately huge; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_ENGINE_LANE_LENGTH: usize = 1_000_000;

/// The most inputs the timers and results lane holds. Same status as the engine lane cap.
pub(crate) const MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH: usize = 1_000_000;

/// The most seat controls (pause cards and pending answers) one seat may have waiting. The plan
/// gives no number. Deliberately huge; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_SEAT_CONTROLS_PENDING: usize = 1_000_000;

/// The most seat controls waiting at the table in all, across every seat. Without it a seat could
/// queue a million per seat and keep the engine and player lanes waiting behind them. The plan
/// gives no number. Deliberately huge; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_CONTROLS_LANE_LENGTH: usize = 1_000_000;

/// The most finished operations whose result a duplicate can still find (O.7); the oldest finished
/// one is forgotten first. Deliberately huge; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_REMEMBERED_OPERATIONS: usize = 1_000_000;

/// The longest an input may stay in progress before `reap_expired` may abandon it, in milliseconds
/// (one hour). J.4's 60 s job lease and 20 s stall watchdog are the broker's; this is the owner's
/// backstop for a lost `finish`. Deliberately huge; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_IN_PROGRESS_MS: u64 = 3_600_000;

// The persistent map behind the pending book: its buckets and chunks bound what one change after
// a checkpoint copies, not what the map holds.

/// How many entries one bucket holds before the map doubles its buckets.
pub(crate) const ENTRIES_PER_BUCKET: usize = 8;
/// The fewest buckets a map has.
pub(crate) const MINIMUM_BUCKETS: usize = 16;
/// The most buckets a map has: a clone copies at most this many pointers.
pub(crate) const MAXIMUM_BUCKETS: usize = 4_096;
/// How many keys one chunk of the order queue holds: a push copies at most this many.
pub(crate) const CHUNK_LENGTH: usize = 1_024;

// Projection. The numbers that follow the plan or the wire are stated as such; the rest are huge on
// purpose (owner, 2026-11-02).

/// How many lines a phone shows: "reconnect resyncs the projection and the last 20 narration lines"
/// (H.4). The plan's number, and a window over the story, not a limit on it. A decline's reason
/// counts as one of the lines.
pub(crate) const PHONE_NARRATION_LINES: usize = 20;

/// The most one encoded view may take on the wire, in bytes: the bridge's 65,536 byte message
/// limit with the headroom `df-api` keeps (`MAXIMUM_MESSAGE_BYTES` there, 60 KiB). The text of a
/// view is held well below it so its structure always fits (#1686).
pub(crate) const WIRE_FRAME_BYTES: usize = 60 * 1024;

/// All the text one view may carry, in bytes: three quarters of the wire frame, which leaves the
/// rest for ids, numbers, the party strip and the framing.
pub(crate) const VIEW_TEXT_BUDGET_BYTES: usize = WIRE_FRAME_BYTES / 4 * 3;

/// The longest single text that is published (a line, a readout, a place, a fact, a decline), in
/// bytes: an eighth of the view's text budget, so the mandatory texts of a view and the newest line
/// and fact always fit the wire frame together. A longer text is refused when it is published, with
/// a typed error.
pub(crate) const MAXIMUM_TEXT_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 8;

/// The share of the view's text budget that the phone's story may take, in bytes. The newest lines
/// come first and older ones are left off, and the count left off is typed data. The newest entry
/// is always shown.
pub(crate) const NARRATION_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// The share of the view's text budget that the private note may take, in bytes. The newest facts
/// come first and older ones are left off, and the count left off is typed data. The newest fact is
/// always shown.
pub(crate) const NOTE_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// The most text the offers of one seat may hold together, in bytes, checked when they are
/// published: an offer is never dropped from a view, so the owner is told when they do not fit.
pub(crate) const OFFERS_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// Most story entries one queue keeps (the public one, and one per seat); the oldest is dropped
/// first. Deliberately huge so it never limits play.
pub(crate) const MAXIMUM_STORY_ENTRIES: usize = 1_000_000;

/// Most facts one seat knows at once. Deliberately huge so it never limits play.
pub(crate) const MAXIMUM_FACTS_PER_SEAT: usize = 1_000_000;

/// Most offers one seat holds at once. Deliberately huge so it never limits play.
pub(crate) const MAXIMUM_OFFERS_PER_SEAT: usize = 1_000_000;

/// Most seats that have a story, facts, a readout, offers or a notice: a seat has them only while
/// it is at the table, so the table's own bound is the bound here.
pub(crate) const MAXIMUM_SEATS_WITH_ENTRIES: usize = MAXIMUM_SEATS;

/// Most scenes that have a place name: a scene needs a seat, so the table's bound again.
pub(crate) const MAXIMUM_PLACES: usize = MAXIMUM_SEATS;

/// Most machines that have a state label: the registry is small, and this is far above it.
pub(crate) const MAXIMUM_STATE_LABELS: usize = 1_000_000;

// The table state.

/// Most cast entries a table holds at once, over all its scenes. Deliberately huge so it never
/// limits play; scope it down later.
pub(crate) const MAXIMUM_CAST_ENTRIES: usize = 1_000_000;

/// Most conditions one cast entry carries. Deliberately huge so it never limits play.
pub(crate) const MAXIMUM_CONDITIONS_PER_ENTRY: usize = 1_000_000;

/// The depth of a scene's hand-off stack: the plan's own number (RES27, O.7), not an invented cap.
/// A hand-off that would push a fourth frame is refused and the machine keeps asking.
pub(crate) const MAXIMUM_HAND_OFF_DEPTH: usize = 3;

// Machines.

/// The most items a hand-off carries. Seats are bounded by `MAXIMUM_SEATS`, so this never limits
/// play; scope it down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_CARRY_ITEMS: usize = 1_000_000;

// Settings: the upper end of every setting range.

/// The upper end of every range for now. Deliberately huge so it never limits play; scope each
/// setting down on its own later (owner, 2026-11-02). The plan gives defaults and no ranges.
pub(crate) const LARGE_SETTING_MAXIMUM: u32 = 1_000_000;
/// The upper end of every look-ahead setting for now. Deliberately huge (owner, 2026-11-02).
pub(crate) const LARGE_LOOKAHEAD_MAXIMUM: u32 = 1_000_000;
/// The upper end of every timing for now. Deliberately huge, so it never limits play (owner,
/// 2026-11-02).
pub(crate) const LARGE_TIMING_MAXIMUM_S: u32 = 1_000_000;

// The Director: the defaults of its size settings.

/// The default of most elements one request may name (an area). Deliberately large.
pub(crate) const DEFAULT_MAXIMUM_TARGETS: usize = 100_000;
/// The default of most requests that may wait for the next host tick. Deliberately large.
pub(crate) const DEFAULT_MAXIMUM_WAITING: usize = 100_000;
/// The default of most element names that may wait in all, over every waiting request: the bound
/// on memory, which the two caps above do not give together. Deliberately large.
pub(crate) const DEFAULT_MAXIMUM_WAITING_TARGETS: usize = 1_000_000;
/// The default of most entries the log keeps. Deliberately large; an entry is a few words.
pub(crate) const DEFAULT_MAXIMUM_LOG_ENTRIES: usize = 100_000;

// Look-ahead.

/// Most scenes the story may name as expected at once. Deliberately huge (owner, 2026-11-02).
pub(crate) const MAXIMUM_EXPECTED_SCENES: usize = 1_000_000;
/// Most scenes the book holds, built or building. Deliberately huge (owner, 2026-11-02).
pub(crate) const MAXIMUM_BOOKED_SCENES: usize = 1_000_000;

// Declines and replay.

/// The most alternatives a decline holds. Deliberately huge so it never limits play; scope it down
/// later (owner, 2026-11-02). The phone's own choice-list rule of O.12 is applied when it draws.
pub(crate) const MAXIMUM_ALTERNATIVES: usize = 10_000;
/// The most turns one recording holds. Deliberately huge so it never limits a session; scope it
/// down later (owner, 2026-11-02).
pub(crate) const MAXIMUM_RECORDED_TURNS: usize = 1_000_000;

// The testkit fakes.

/// The most events a `RecordingSink` keeps unless a test asks for fewer. Deliberately huge so it
/// never limits a test; scope it down later (owner, 2026-11-02). The oldest are dropped first.
#[cfg(any(test, feature = "testkit"))]
pub(crate) const MAXIMUM_RECORDED_EVENTS: usize = 1_000_000;

/// The most requests a `ScriptedBroker` remembers unless a test asks for fewer. Deliberately huge;
/// scope it down later (owner, 2026-11-02). The oldest are forgotten first, and asking about a
/// forgotten handle is `stale`.
#[cfg(any(test, feature = "testkit"))]
pub(crate) const MAXIMUM_REMEMBERED_REQUESTS: usize = 1_000_000;
