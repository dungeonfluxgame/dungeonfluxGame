//! Every limit and timing value of the host, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant, memory stays bounded by design, and a value is scoped down later when a measurement
//! says so. A limit here is either a safety rule (it is not in this file, it lives with the domain
//! type that needs it, such as a rotation of 0 to 359) or a size nobody has measured, and those
//! are the ones in this file. The help text and the error messages of the host are built from
//! these constants, so they follow a change.
//!
//! The limits of the media library (file size, file count, video duration) are in
//! `media_ingest`, which owns the checks that use them.

use std::time::Duration;

const KIBIBYTE: usize = 1024;
const MEBIBYTE: usize = 1024 * KIBIBYTE;
const GIBIBYTE: u64 = 1024 * 1024 * 1024;

/// Most audit lines kept. The log is the evidence of a playtest, and 30 arrivals a tick can push
/// hundreds of lines out in a second, so the bound is a memory bound.
pub const MAXIMUM_LOGGED_LINES: usize = 100_000;
/// Most event lines one events answer and the viewer carry.
pub const MAXIMUM_EVENTS_PER_ANSWER: usize = 10_000;
/// Largest request body accepted on the developer API, in bytes. The media add route reads its
/// own body (see `media_ingest`).
pub const MAXIMUM_REQUEST_BYTES: usize = 16 * MEBIBYTE;
/// Most notices or problems one answer carries; more are summarized as "and N more".
pub const MAXIMUM_LISTED_PROBLEMS: usize = 10_000;
/// Largest answer the command line reads, in bytes.
pub const MAXIMUM_RESPONSE_BYTES: usize = 256 * MEBIBYTE;
/// Most characters of an unexpected answer echoed in an error.
pub const MAXIMUM_ECHOED_ANSWER: usize = 8_192;
/// Largest playground file served (the compiled package is the biggest), in bytes.
pub const MAXIMUM_FILE_BYTES: u64 = GIBIBYTE;
/// Most open pages remembered at once; a page silent for `FORGET_CLIENT_AFTER` is forgotten
/// first, so this bounds a flood of page names, not real pages.
pub const MAXIMUM_CLIENTS: usize = 1_024;
/// Longest name a page may give itself.
pub const MAXIMUM_CLIENT_NAME: usize = 256;
/// Longest `wait` and longest hold in a `cycle`, in milliseconds (a day).
pub const MAXIMUM_WAIT_MS: u64 = 86_400_000;
/// Most rounds a `cycle` runs.
pub const MAXIMUM_CYCLES: u32 = u32::MAX;
/// Most kind files read from the kind directory.
pub const MAXIMUM_KIND_FILES: usize = 4_096;
/// Largest kind file, in bytes.
pub const MAXIMUM_KIND_FILE_BYTES: u64 = 4 * MEBIBYTE as u64;
/// Most characters of an element's label the debugging view draws (the text maximum of a field).
pub const MAXIMUM_LABEL_CHARACTERS: usize = 65_536;
/// Most bytes of audit lines waiting for the terminal. A terminal that stops accepting output
/// fills this and then lines are dropped and counted, so a blocked console costs memory up to
/// this and nothing else.
pub const MAXIMUM_PENDING_ECHO_BYTES: usize = 256 * MEBIBYTE;

/// Tuning values, not caps: how often and how long, collected here so they are found together.
/// How often the host's clock moves travelling elements (about 30 times a second), while
/// something moves.
pub const TICK_INTERVAL: Duration = Duration::from_millis(33);
/// Longest a frame request is held waiting for a change before it is answered anyway.
pub const FRAME_WAIT: Duration = Duration::from_secs(25);
/// A page not heard from for this long is forgotten: it sends a sign of life every ten seconds,
/// so silence means it was closed.
pub const FORGET_CLIENT_AFTER: Duration = Duration::from_secs(45);
/// How long open connections get to finish after an interrupt.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// Deadline for connecting, sending and reading one answer on the command line.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
