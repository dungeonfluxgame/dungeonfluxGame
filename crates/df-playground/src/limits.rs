//! Every size, count and time bound and every timing value of the playground model and page, in
//! one place. Each is imported from here; no other module defines or re-exports one.
//!
//! The owner's rule is that no limit is invented small: a bound is a rule of the domain, an
//! external contract (a file system, a browser), or a very large number whose only job is to keep
//! memory bounded by design, and it is scoped down later from a measurement. Each constant says
//! which it is.

// Rules of the domain. These are not tuned; they are what the value means.

/// Largest rotation of a picture or a figure, in degrees. A full turn is 360, which is 0.
pub const MAXIMUM_ROTATION_DEGREES: u16 = 359;
/// Least zoom of the camera, in percent. Below 100 the world no longer covers the screen (O.14
/// camera rule 3), and 100 is the whole stage.
pub const MINIMUM_ZOOM_PERCENT: u16 = 100;
/// The whole of a share: a slider's value, a bar's fill, a place on a screen.
pub const MAXIMUM_PERCENT: u8 = 100;
/// Loudest volume, in percent.
pub const MAXIMUM_VOLUME_PERCENT: u8 = MAXIMUM_PERCENT;
/// A slide goes from and to a share of the screen's width, in percent.
pub const MAXIMUM_SLIDE_PERCENT: u8 = MAXIMUM_PERCENT;
/// The point a figure turns about is a share of the figure's own size, in percent.
pub const MAXIMUM_PIVOT_PERCENT: u8 = MAXIMUM_PERCENT;
/// Most suggested choices a phone shows above the groups (the choice-list rule, plan K.14).
pub const MAXIMUM_SUGGESTED_CHOICES: usize = 4;
/// Most words in a choice label (the choice-list rule, plan K.14).
pub const MAXIMUM_LABEL_WORDS: usize = 4;
/// Shortest time a caption stays up, in milliseconds (plan K.5).
pub const MINIMUM_CAPTION_DWELL_MS: u64 = 1_800;
/// Time per word that a person needs to read a caption across a room, in milliseconds (plan K.5).
pub const DWELL_MS_PER_WORD: u64 = 330;
/// Shortest time a result card stays up, in milliseconds (plan K.12).
pub const MINIMUM_RESULT_CARD_DWELL_MS: u64 = 3_000;

// External contracts.

/// Longest media file name accepted, in bytes: the longest a common file system allows.
pub const MAXIMUM_MEDIA_NAME_BYTES: usize = 255;
/// Longest wait a browser timer takes, in milliseconds (`setTimeout` holds a signed 32-bit delay
/// and fires at once for a longer one).
pub const LONGEST_BROWSER_TIMER_MS: u64 = 2_147_483_647;

/// The sample rate the speech-to-text adapter reads, in hertz (raw signed 16-bit little-endian
/// mono PCM, `planning/chat-pipeline.md` 8.6).
pub const CAPTURE_SAMPLE_RATE_HZ: u32 = 16_000;
/// Shortest recording the speech-to-text adapter takes, in milliseconds (3,200 bytes of PCM).
pub const MINIMUM_CAPTURE_MS: u64 = 100;
/// Longest recording the speech-to-text adapter takes, in milliseconds (1,920,000 bytes of PCM).
pub const MAXIMUM_CAPTURE_MS: u64 = 60_000;
/// Most characters the question box takes (`planning/chat-pipeline.md` 8.6: what the text input
/// holds).
pub const MAXIMUM_QUESTION_CHARACTERS: usize = 800;

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
pub const MAXIMUM_SLIDE_MS: u64 = 86_400_000;
/// Longest a camera move may take, in milliseconds (a day).
pub const MAXIMUM_CAMERA_MS: u64 = 86_400_000;
/// Longest a transition between scenes may take, in milliseconds (a day). The 1.5 second minimum
/// for a dissolve in K.3 is a different rule, about what the table sees.
pub const MAXIMUM_TRANSITION_MS: u64 = 86_400_000;
/// Most the camera may ask to zoom in, in percent (100 times the whole stage). This is a large
/// constant that only bounds the number; the zoom that is drawn is held to what the picture can
/// bear (`camera::zoom_ceiling_percent`: the plate's pixel width over the stage's device pixels).
pub const MAXIMUM_ZOOM_PERCENT: u16 = 10_000;
/// Most slides a page remembers the start of, so a redraw carries on instead of starting again.
pub const MAXIMUM_REMEMBERED_SLIDES: usize = 1_024;
/// Most errors a client report carries; older ones are dropped.
pub const MAXIMUM_REPORTED_ERRORS: usize = 1_000;
/// Longest one error in a client report, in characters.
pub const MAXIMUM_REPORTED_ERROR_CHARACTERS: usize = 8_192;
/// Most pictures a client report lists.
pub const MAXIMUM_REPORTED_IMAGES: usize = 10_000;
/// Most a whole client report holds, in bytes of text: the bound that keeps a table of pages
/// bounded by design, whatever the fields add up to.
pub const MAXIMUM_REPORT_BYTES: usize = 1_048_576;
/// Most timeline records a page holds before it has reported them, and so most a report carries;
/// past it the oldest unreported go first.
pub const MAXIMUM_TIMELINE_RECORDS: usize = 4_096;
/// Most timeline records the host keeps of each page; past it the oldest go first.
pub const MAXIMUM_KEPT_TIMELINE_RECORDS: usize = 16_384;
/// Longest id of a timeline record, and of each piece a draw record names, in characters. Ids are
/// media names, figure ids, scene files and hashes, so this is the longest media name.
pub const MAXIMUM_TIMELINE_ID_CHARACTERS: usize = MAXIMUM_MEDIA_NAME_BYTES;
/// Most pieces a draw record names.
pub const MAXIMUM_TIMELINE_PARTS: usize = 1_000;
/// Most sound effects a page holds at once, played or still waiting; the oldest are let go first.
pub const MAXIMUM_LIVE_EFFECTS: usize = 4_096;
/// Most fact rows on the adventure card.
pub const MAXIMUM_FACT_ROWS: usize = 1_000;
/// Most campaign cards in the recent-campaigns strip.
pub const MAXIMUM_CAMPAIGN_CARDS: usize = 1_000;
/// Most cards on the settings panel.
pub const MAXIMUM_SETTING_CARDS: usize = 1_000;
/// Most rows in one card of the settings panel.
pub const MAXIMUM_ROWS_PER_SETTING_CARD: usize = 1_000;
/// Most options in one dropdown.
pub const MAXIMUM_DROPDOWN_OPTIONS: usize = 1_000;
/// Most named steps on one slider.
pub const MAXIMUM_SLIDER_STEPS: usize = 1_000;
/// Most an amount may be, in hundredths of the currency (a billion).
pub const MAXIMUM_AMOUNT_MINOR_UNITS: u64 = 100_000_000_000;
/// Most items of the navigation rail.
pub const MAXIMUM_NAV_ITEMS: usize = 1_000;
/// Most buttons of the action bar.
pub const MAXIMUM_ACTION_BUTTONS: usize = 1_000;
/// Most characters in the badge on a navigation item.
pub const MAXIMUM_BADGE_CHARACTERS: usize = 1_000;
/// Longest focus id, in characters.
pub const MAXIMUM_FOCUS_CHARACTERS: usize = 1_000;
/// Most seats a lobby, or chips a loading screen, holds.
pub const MAXIMUM_LOBBY_SEATS: usize = 1_000;
/// Most speech bubbles on a roleplay card.
pub const MAXIMUM_SPEECH_BUBBLES: usize = 1_000;
/// Most character markers on a roleplay card.
pub const MAXIMUM_CHARACTER_MARKERS: usize = 1_000;

// Tuning values. These are not caps; they are how often and how long.

/// Milliseconds between looks at the server for changed files, a rebuilt package and a changed
/// fixture list. The canvas and the controls do not wait for this.
pub const POLL_MS: u64 = 1_000;
/// Polls between reports sent even when nothing changed, so the host can tell the page is alive.
pub const HEARTBEAT_POLLS: u32 = 10;
/// The first wait after a failure to reach the host, in milliseconds.
pub const FIRST_RETRY_MS: u64 = 1_000;
/// The longest wait after failures to reach the host, in milliseconds.
pub const LONGEST_RETRY_MS: u64 = 10_000;
/// How long a transition takes until one is chosen, in milliseconds.
pub const DEFAULT_TRANSITION_MS: u64 = 700;
/// How long the screen glides to a camera framing that is not a move (a retarget or a pull back),
/// in milliseconds. It matches the glide of a figure.
pub const CAMERA_GLIDE_MS: u64 = 90;
/// How many answers in a row from one older run of the host are taken to mean that run is the real
/// one. A host that restarts with a clock set back has an older boot id than the run the page was
/// following, and the page must not refuse it for ever.
pub const OLDER_RUN_STRIKES: u8 = 3;

/// Milliseconds between looks at a recording that is running, to update its time and level and to
/// notice a lost microphone or the longest recording.
pub const CAPTURE_REFRESH_MS: u64 = 200;
/// How long the composer keeps a message about a refusal or a failure, in milliseconds. A message
/// stays long enough to read at arm's length.
pub const COMPOSER_NOTE_MS: u64 = 7_000;

// Layout tuning of the overlay components. They are how the screen is composed, not caps.

/// The share of a hero's health, in percent, below which the bar turns amber.
pub const HEALTH_AMBER_PERCENT: u16 = 50;
/// The share of a hero's health, in percent, below which the bar turns red.
pub const HEALTH_RED_PERCENT: u16 = 25;
