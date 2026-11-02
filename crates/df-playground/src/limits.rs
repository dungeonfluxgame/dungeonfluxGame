//! Every limit and every timing value of the playground model and page, in one place.
//!
//! The owner's rule is that no limit is invented small: a cap is either a rule of the domain, or a
//! very large number whose only job is to keep memory bounded by design, and it is scoped down
//! later from a measurement. Each constant says which of the two it is. The other modules re-export
//! the ones their callers already import from them.

// Rules of the domain. These are not tuned; they are what the value means.

/// Largest rotation of a picture or a figure, in degrees. A full turn is 360, which is 0.
pub const MAXIMUM_ROTATION_DEGREES: u16 = 359;
/// Least zoom of the camera, in percent. Below 100 the world no longer covers the screen (O.14
/// camera rule 3), and 100 is the whole stage.
pub const MINIMUM_ZOOM_PERCENT: u16 = 100;
/// Loudest volume, in percent.
pub const MAXIMUM_VOLUME: u8 = 100;
/// A slide goes from and to a share of the screen's width, in percent.
pub const MAXIMUM_SLIDE_PERCENT: u8 = 100;
/// The point a figure turns about is a share of the figure's own size, in percent.
pub const MAXIMUM_PIVOT_PERCENT: u8 = 100;

// Very large caps. Each only keeps memory bounded by design.

/// Most characters in a caption line.
pub const MAXIMUM_CAPTION_CHARACTERS: usize = 100_000;
/// Most characters in a name, a speaker, a label or a camera target.
pub const MAXIMUM_NAME_CHARACTERS: usize = 1_000;
/// Most members in the party strip.
pub const MAXIMUM_PARTY_MEMBERS: usize = 1_000;
/// Most choices in one list (suggested, a group or unavailable).
pub const MAXIMUM_CHOICES_PER_LIST: usize = 1_000;
/// Most sound cues the host keeps for pages to collect; a page that was away longer misses the
/// older ones.
pub const MAXIMUM_CUES_KEPT: usize = 4_096;
/// Longest a slide may take, in milliseconds (a day).
pub const MAXIMUM_SLIDE_MS: u32 = 86_400_000;
/// Longest a camera move may take, in milliseconds (a day).
pub const MAXIMUM_CAMERA_MS: u32 = 86_400_000;
/// Longest a transition between scenes may take, in milliseconds (a day). The 1.5 second minimum
/// for a dissolve in K.3 is a different rule, about what the table sees.
pub const MAXIMUM_TRANSITION_MS: u32 = 86_400_000;
/// Most the camera may zoom in, in percent (100 times the whole stage).
pub const MAXIMUM_ZOOM_PERCENT: u16 = 10_000;
/// Most slides a page remembers the start of, so a redraw carries on instead of starting again.
pub const MAXIMUM_REMEMBERED_SLIDES: usize = 1_024;
/// Most errors a client report carries; older ones are dropped.
pub const MAXIMUM_REPORTED_ERRORS: usize = 1_000;
/// Longest media file name accepted (the longest a common file system allows).
pub const MAXIMUM_MEDIA_NAME_LENGTH: usize = 255;
/// Most sound effects a page holds at once, played or still waiting; the oldest are let go first.
pub const MAXIMUM_LIVE_EFFECTS: usize = 4_096;

// Tuning values. These are not caps; they are how often and how long.

/// Milliseconds between looks at the server for changed files, a rebuilt package and a changed
/// fixture list. The canvas and the controls do not wait for this.
pub const POLL_MS: u32 = 1_000;
/// Polls between reports sent even when nothing changed, so the host can tell the page is alive.
pub const HEARTBEAT_POLLS: u32 = 10;
/// The first wait after a failure to reach the host, in milliseconds.
pub const FIRST_RETRY_MS: u32 = 1_000;
/// The longest wait after failures to reach the host, in milliseconds.
pub const LONGEST_RETRY_MS: u32 = 10_000;
/// How long a transition takes until one is chosen, in milliseconds.
pub const DEFAULT_TRANSITION_MS: u32 = 700;
/// How long the screen glides to a camera framing that is not a move (a retarget or a pull back),
/// in milliseconds. It matches the glide of a figure.
pub const CAMERA_GLIDE_MS: u32 = 90;
