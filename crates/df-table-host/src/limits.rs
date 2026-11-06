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
/// hundreds of lines out in a second.
pub const MAXIMUM_LOGGED_LINES: usize = 100_000;
/// Most bytes of audit lines kept, whatever their number: the memory bound of the log. A line is
/// bounded too (a batch holds at most `MAXIMUM_BATCH_CHANGES` changes), so this is reached only by
/// a very long run of long lines.
pub const MAXIMUM_LOGGED_BYTES: usize = 256 * MEBIBYTE;
/// Most changes one batch of playground controls holds: far above any real batch, so a generated
/// batch is never refused for its length; it bounds the audit line of a batch.
pub const MAXIMUM_BATCH_CHANGES: usize = 100_000;
/// Most event lines one events answer and the viewer carry.
pub const MAXIMUM_EVENTS_PER_ANSWER: usize = 10_000;
/// Largest request body accepted on the developer API, in bytes. The media add route reads its
/// own body (see `media_ingest`).
pub const MAXIMUM_REQUEST_BYTES: usize = 256 * MEBIBYTE;
/// Most notices or problems one answer carries; more are summarized as "and N more".
pub const MAXIMUM_LISTED_PROBLEMS: usize = 10_000;
/// Largest answer the command line reads, in bytes.
pub const MAXIMUM_RESPONSE_BYTES: usize = 256 * MEBIBYTE;
/// Most characters of an unexpected answer echoed in an error: enough for any answer a dev route
/// gives, so the whole of it is seen.
pub const MAXIMUM_ECHOED_ANSWER_CHARACTERS: usize = MEBIBYTE;
/// Bytes of a local file the command line reads, encodes and sends at a time when it uploads one:
/// a multiple of three, so every piece but the last encodes to base64 without padding. A size
/// that bounds memory per piece, not the file.
pub const UPLOAD_PIECE_BYTES: usize = 3 * 64 * KIBIBYTE;
/// Bytes a media add request carries besides the file's base64: the JSON braces, the two keys,
/// the name (with escapes) and whitespace. It is added to the largest file's base64 to give the
/// largest add request (`media_ingest`).
pub const UPLOAD_ENVELOPE_BYTES: u64 = 4 * KIBIBYTE as u64;
/// Pieces of an upload waiting for the thread that decodes them, `UPLOAD_PIECE_BYTES` each at
/// most; a full queue holds the sender back, so an upload in flight holds a few pieces, not the
/// file. A size that bounds memory per upload, not the file.
pub const UPLOAD_QUEUE_PIECES: usize = 4;
/// Bytes of a media file read and sent at a time: the most of a file the host holds for one
/// answer. A size that bounds memory per answer, not the file.
pub const SERVE_CHUNK_BYTES: u64 = 256 * KIBIBYTE as u64;
/// Largest compiled package file served, in bytes. It is streamed, never held whole, so this
/// bounds a response and not memory.
pub const MAXIMUM_PACKAGE_FILE_BYTES: u64 = GIBIBYTE;
/// Largest page, theme or fixture file read, in bytes. These are read whole (a fixture is parsed
/// into a tree several times its size), so the bound is what such a file is for, not what a disk
/// holds.
pub const MAXIMUM_ASSET_FILE_BYTES: u64 = GIBIBYTE;
/// Longest name of a fixture file, in bytes: the longest a common file system allows (an external
/// limit, not a dev cap).
pub const MAXIMUM_FIXTURE_NAME_LENGTH: usize = 255;
/// Largest report a page may send about what it shows and plays, in bytes. With
/// `MAXIMUM_CLIENTS` this bounds the whole client table (about 16 GiB), whatever a report holds.
pub const MAXIMUM_REPORT_BYTES: usize = 16 * MEBIBYTE;
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
pub const MAXIMUM_KIND_FILE_BYTES: u64 = 64 * MEBIBYTE as u64;
/// Most characters of an element's label the debugging view draws (the text maximum of a field).
pub const MAXIMUM_LABEL_CHARACTERS: usize = 65_536;
/// Most bytes of audit lines waiting for the terminal, each counted with
/// `ECHO_LINE_OVERHEAD_BYTES`. A terminal that stops accepting output fills this and then lines
/// are dropped and counted, so a blocked console costs memory up to this and nothing else.
pub const MAXIMUM_PENDING_ECHO_BYTES: usize = 256 * MEBIBYTE;
/// What a queued audit line costs besides its text: the string header, allocator slack and the
/// queue node.
pub const ECHO_LINE_OVERHEAD_BYTES: usize = 128;
/// Longest name of a key of the media upload envelope (`name`, `data_base64`), in bytes: the
/// envelope's own keys are this short (a protocol bound, not a dev cap).
pub const MAXIMUM_UPLOAD_KEY_BYTES: usize = 16;
/// Longest media name in the upload envelope, in bytes: a media name is at most 255 plain
/// characters, and the envelope may spell it with escapes.
pub const MAXIMUM_UPLOAD_NAME_BYTES: usize = 512;

// Tuning values, not caps: how often and how long, collected here so they are found together.

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
/// Deadline for connecting, sending and reading one answer on the command line. It covers the
/// longest job a command waits for (`media generate wait=true`, whose deadline is
/// `imagegen::DEFAULT_DEADLINE`, 15 minutes, plus its grace) with a wide margin.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Deadline for each sending step of a streamed upload on the command line: connecting, and
/// sending one piece. Every piece gets a fresh deadline, so a long upload is never cut off while
/// it keeps moving; this only notices a host that has stopped reading.
pub const STREAM_STEP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Deadline for the answer to a streamed upload, once the whole body is sent. It is one fixed
/// ceiling: the host answers only after it has stored the whole file (a large video is probed and
/// written first), and nothing arrives meanwhile that could reset a deadline. It matches
/// `REQUEST_TIMEOUT`, the wait for the answer to any other command.
pub const STREAM_ANSWER_TIMEOUT: Duration = REQUEST_TIMEOUT;
/// How long the echo writer waits for a line before it reports what it dropped.
pub const ECHO_NOTICE_AFTER: Duration = Duration::from_millis(500);
/// The page glides a turn over this long (`.figure` in `theme.css`, `CAMERA_GLIDE_MS` in the
/// playground), and `transform-origin` does not glide, so a pivot changed during a glide jumps.
pub const FIGURE_GLIDE_MS: u64 = 90;
/// How long after a change of an element's turn its pivot may not change: the glide and a margin
/// for the frame that carries it to the page.
pub const TURN_SETTLE: Duration = Duration::from_millis(150);
