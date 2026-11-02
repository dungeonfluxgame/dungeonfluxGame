//! The invented limits of the canvas, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant, memory stays bounded by design, and a value is scoped down later when a measurement
//! says so. These are sizes nobody has measured. The limits that are rules of the domain (a
//! rotation of 0 to 359 degrees, a share of 0 to 100 percent, the 100 by 56.25 canvas) are not
//! here: they live with the type that needs them.

/// Most elements the canvas holds at once. The playground sends every element in every frame, so
/// a frame is as large as this times an element; delta frames come before this is lowered or
/// reached.
pub const MAXIMUM_ELEMENTS: usize = 100_000;
/// Longest name (of an element, kind, field or choice), in characters. Names are ASCII.
pub const MAXIMUM_NAME_LENGTH: usize = 256;
/// Longest text value of a field, in characters, when its kind does not say less with its own
/// `maximum_length`.
pub const MAXIMUM_TEXT_LENGTH: usize = 65_536;
/// Most fields one kind has.
pub const MAXIMUM_FIELDS_PER_KIND: usize = 4_096;
/// Most choices one choice field has.
pub const MAXIMUM_CHOICES: usize = 4_096;
/// Fastest travel accepted, in canvas units per second. A speed need only be finite and above
/// zero (a rule of the domain); this only keeps one step finite and far from overflow.
pub const MAXIMUM_SPEED: f32 = 1_000_000.0;

/// Longest tick the travel system advances, in seconds (a safety rule, not an invented cap). A host
/// that stalled and wakes up must not carry every element across the canvas in one step, so a
/// longer gap is cut to this and the rest of the time is discarded: travel is a little late
/// after a stall, never early or teleported. The host's clock does not count the time it sleeps
/// while nothing moves as a stall.
pub const MAXIMUM_TICK_SECONDS: f32 = 0.25;

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
pub const MAXIMUM_OPEN_PENDINGS: usize = 1_000_000;
/// Most rulings a table remembers; the oldest is forgotten first.
pub const MAXIMUM_STORED_RULINGS: usize = 1_000_000;
/// Most committed operations whose result a replay may find again.
pub const MAXIMUM_REMEMBERED_RESULTS: usize = 1_000_000;
