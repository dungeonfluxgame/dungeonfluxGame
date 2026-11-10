//! The picture generator's size, count and time bounds, in one place. A bound that only protects
//! against a mistake is generous; one tied to an outside contract says which.

use std::time::Duration;

/// The limit for one generation unless the owner sets another: the owner wants large limits, and
/// a Codex picture takes minutes.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Extra time the host waits after the backend's own deadline for its blocking task to hand back
/// an answer, before it calls the attempt timed out itself.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(60);

/// The long side of the size asked of the backend, in pixels. The Codex image tool picks its own
/// size (about 1672 by 941 for a wide picture), so this is a request, not a promise.
pub const REQUESTED_LONG_SIDE_PX: u32 = 1672;

/// Most characters of text one request sends the backend: the prompt, a blank line and the
/// purpose's direction together. An outside contract: `df_imagery::oneshot` refuses longer text.
pub const MAXIMUM_SENT_TEXT_CHARS: usize = df_imagery::oneshot::MAXIMUM_PROMPT_CHARS;

/// Most pictures one request attaches: the picture to edit and the references together. An
/// outside contract: `df_imagery::oneshot` refuses more (Forge takes up to 10).
pub const MAXIMUM_PICTURES: usize = df_imagery::oneshot::MAXIMUM_PICTURES;

/// Most pictures one Codex request attaches, the picture to edit and the references together. An
/// outside contract: the Codex image tool takes five, and given six it used five and the job ended
/// `InputMismatch` (dungeon run 18: every room plate of the vault, where six people are, was
/// refused).
pub const CODEX_MAXIMUM_PICTURES: usize = 5;

/// The most pictures a plate request the host builds itself carries unless the owner sets another
/// (`media backend refs=<N>`, `persona watch refs_max=<N>`): below what a backend takes, because
/// more references dilute the picture and slow it. The people's sheets come first, then the things
/// in use, then what is only seen; the picture to edit counts as one. Never above the backend's
/// own maximum.
pub const DEFAULT_REFERENCE_CAP: usize = 3;

/// What the provenance sidecar adds to a picture name.
pub const PROVENANCE_SUFFIX: &str = ".provenance.json";

/// Longest picture name, in characters (names are ASCII, so also bytes). An outside contract: a
/// media name is at most `df_playground::limits::MAXIMUM_MEDIA_NAME_BYTES`, and the sidecar's
/// name, the longest file named after a picture, adds `PROVENANCE_SUFFIX`.
pub const MAXIMUM_NAME_CHARS: usize =
    df_playground::limits::MAXIMUM_MEDIA_NAME_BYTES - PROVENANCE_SUFFIX.len();

/// The most either number of a picture shape may be, against a mistyped shape.
pub const MAXIMUM_ASPECT_PART: u32 = 64;

/// The widest or tallest picture shape: this many to one, against a mistyped shape.
pub const MAXIMUM_ASPECT_RATIO: u32 = 4;

/// Most jobs remembered for `media generate status`. When there are more, the oldest finished
/// jobs are forgotten first; a running job is never forgotten.
pub const MAXIMUM_REMEMBERED_JOBS: usize = 100_000;
