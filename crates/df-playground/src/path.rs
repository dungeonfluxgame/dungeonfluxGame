//! The addresses the page asks the host for, and the query of a frame request. The host's router
//! (`df_table_host::wire::route`, and its speech and persona routes) serves every one of these
//! constants and reads every query parameter here, so a rename breaks the build instead of a
//! running page.

/// The live canvas (GET).
pub const CANVAS: &str = "/canvas.json";
/// Changes when any playground file does (GET).
pub const REVISION: &str = "/playground/revision";
/// The page's stylesheet (GET).
pub const THEME: &str = "/playground/theme.css";
/// The names of the fixture files (GET).
pub const FIXTURE_LIST: &str = "/playground/fixtures.json";
/// One fixture file, `NAME_PLACEHOLDER` standing for its file name (GET). `fixture_file` fills it.
pub const FIXTURE_FILE: &str = "/playground/fixtures/{name}";
/// One file of the media library, `NAME_PLACEHOLDER` standing for its name (GET). `media_file`
/// fills it.
pub const MEDIA_FILE: &str = "/playground/media/{name}";
/// One picture of the dice, `NAME_PLACEHOLDER` standing for its file name (GET). `dice_file`
/// fills it.
pub const DICE_FILE: &str = "/playground/dice/{name}";
/// The part of `FIXTURE_FILE`, `MEDIA_FILE` and `DICE_FILE` that names the file, as the host's
/// router writes a path parameter.
pub const NAME_PLACEHOLDER: &str = "{name}";
/// A long poll for the next frame (GET, with the parameters below).
pub const FRAME: &str = "/playground/frame";
/// What the playground shows (GET).
pub const CONTROLS: &str = "/playground/controls.json";
/// Changes what the playground shows (POST a `controls::ControlsChange`).
pub const CONTROLS_CHANGE: &str = "/dev/playground/controls";
/// A page tells the host what it is showing and playing (POST a `media::ClientReportPost`).
pub const CLIENT_REPORT: &str = "/dev/playground/report";
/// Whether speech to text is set up on the host (GET).
pub const SPEECH_STATUS: &str = "/dev/speech/status";
/// Turns a recording into text (POST raw 16 kHz mono signed 16-bit little-endian PCM).
pub const SPEECH_TRANSCRIBE: &str = "/dev/speech/transcribe";
/// The content type of the recording `SPEECH_TRANSCRIBE` takes.
pub const SPEECH_CONTENT_TYPE: &str = "audio/L16;rate=16000";
/// The stored personas, who each is (GET).
pub const PERSONA_LIST: &str = "/dev/persona";
/// Puts a question to a character of the shown scene (POST a `voice_wire::AskRequest`).
pub const PERSONA_ASK: &str = "/dev/persona/ask";

/// The query parameter of `FRAME` that names the last frame already seen.
pub const FRAME_AFTER_PARAMETER: &str = "after";
/// The query parameter of `FRAME` that names the run of the host the last frame came from.
pub const FRAME_BOOT_PARAMETER: &str = "boot";
/// The query parameter of `FRAME` with which a page says it accepts an empty answer when nothing
/// changed before the wait ran out.
pub const FRAME_IDLE_PARAMETER: &str = "idle";
/// The value of `FRAME_IDLE_PARAMETER` that asks for a bodiless 204 on a timeout.
pub const FRAME_IDLE_NO_CONTENT: u16 = 204;

/// The address of the fixture file `file_name`.
pub fn fixture_file(file_name: &str) -> String {
    FIXTURE_FILE.replace(NAME_PLACEHOLDER, file_name)
}

/// The address of the media file `name`.
pub fn media_file(name: &str) -> String {
    MEDIA_FILE.replace(NAME_PLACEHOLDER, name)
}

/// One page's timeline, `CLIENT_PLACEHOLDER` standing for the page's name (GET, with the
/// parameters of `timeline::TimelineQuery`). `client_timeline` fills it.
pub const CLIENT_TIMELINE: &str = "/playground/clients/{client}/timeline.json";
/// The part of `CLIENT_TIMELINE` that names the page, as the host's router writes a path
/// parameter.
pub const CLIENT_PLACEHOLDER: &str = "{client}";
/// The header every request of a page (`page-xxxxxxxx`) or of the dev CLI (`cli`) carries, so the
/// host can say who asked.
pub const CLIENT_HEADER: &str = "x-df-client";

/// The address of the timeline of the page `client`.
pub fn client_timeline(client: &str) -> String {
    CLIENT_TIMELINE.replace(CLIENT_PLACEHOLDER, client)
}

/// The address of the dice picture `name`.
pub fn dice_file(name: &str) -> String {
    DICE_FILE.replace(NAME_PLACEHOLDER, name)
}
