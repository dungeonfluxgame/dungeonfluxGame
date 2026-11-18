//! The addresses the page asks the host for, and the query of a frame request. The host's router
//! (`df_table_host::wire::route`, and its speech and persona routes) serves every one of these
//! constants and reads every query parameter here, so a rename breaks the build instead of a
//! running page.

/// The phone page (GET), served on the player listener: the playground page in its phone role
/// (`role::PageRole`).
pub const PLAY: &str = "/play";
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

/// The room a phone is joining: its adventure, who has joined and whether it gathers (GET, a
/// `seat::wire::RoomReply`).
pub const PLAY_ROOM: &str = "/play/room";
/// A phone says hello with its seat key (POST `{}`, a `seat::wire::HelloReply`).
pub const PLAY_HELLO: &str = "/play/hello";
/// A phone asks for a seat (POST a `seat::wire::JoinRequest`, a `seat::wire::JoinStatus`).
pub const PLAY_JOIN: &str = "/play/join";
/// A phone checks the room code typed at Next, before the join choices, under the same wait as a
/// join (POST a `seat::wire::CodeCheck`, a `seat::wire::CodeTaken`).
pub const PLAY_JOIN_CODE: &str = "/play/join/code";
/// Where a phone's request for a seat stands (GET, a `seat::wire::JoinStatus`).
pub const PLAY_JOIN_STATUS: &str = "/play/join/status";
/// The heroes the player at this phone may play (GET, a `seat::wire::HeroesReply`).
pub const PLAY_HEROES: &str = "/play/heroes";
/// The seat's view of the table (GET, a `seat::wire::SeatView`).
pub const PLAY_SEAT_VIEW: &str = "/play/seat/view";
/// Everything a seated phone does at the table, one typed action with its own id (POST a
/// `seat::wire::SeatAction`, a `seat::wire::SeatOutcome`; refusals are the
/// `seat::wire::SeatActionErrorKind` family). The one route for a seat's actions: only the
/// recording (`PLAY_SEAT_TRANSCRIBE`, a binary body) and the keyless entry routes stand apart.
pub const PLAY_SEAT_ACTION: &str = "/play/seat/action";
/// A recording of the seat's player written down (POST 16 kHz 16-bit mono PCM as
/// `SPEECH_CONTENT_TYPE`, a `voice_wire::Transcript`), while the seat's switch is on.
pub const PLAY_SEAT_TRANSCRIBE: &str = "/play/seat/transcribe";
/// What the seat's live pages show: its bag, the people here, its notebook, its map and the things
/// in sight (GET, a `pages::wire::SeatPages`).
pub const PLAY_SEAT_PAGES: &str = "/play/seat/pages";
/// A check of the seat's hero sheet rolled by the host (POST a `seat::wire::SeatRoll`, a
/// `seat::wire::SeatRolled`): the phone names the check, never a number.
pub const PLAY_SEAT_ROLL: &str = "/play/seat/roll";
/// What a new hero may be started as (GET, a `look::wire::LookStartOptions`), or a new hero for
/// the seat with its look studio afresh (POST a `look::wire::LookStartAsk`, a
/// `look::wire::LookStudioView`).
pub const PLAY_LOOK_START: &str = "/play/look/start";
/// A random look for the seat's hero, spending nothing (POST a `look::wire::LookRollAsk`, a
/// `look::wire::LookDraft`).
pub const PLAY_LOOK_ROLL: &str = "/play/look/roll";
/// The seat's look studio (GET, a `look::wire::LookStudioView`).
pub const PLAY_LOOK: &str = "/play/look";
/// The look controls of the seat's hero (GET, a `look::wire::LookControls`).
pub const PLAY_LOOK_CONTROLS: &str = "/play/look/controls";
/// The player's words read into details (POST a `look::wire::LookDetailsAsk`, a
/// `look::wire::LookDetails`).
pub const PLAY_LOOK_DETAILS: &str = "/play/look/details";
/// One new version (POST a `look::wire::LookRefineAsk`, a `look::wire::LookVersion`).
pub const PLAY_LOOK_REFINE: &str = "/play/look/refine";
/// Keep a version, or go back to one (POST a `look::wire::LookVersionAsk`).
pub const PLAY_LOOK_KEEP: &str = "/play/look/keep";
/// Keep the old version over a new one (POST a `look::wire::LookVersionAsk`).
pub const PLAY_LOOK_DISCARD: &str = "/play/look/discard";
/// Start again from the base pose (POST a `look::wire::LookAgainAsk`).
pub const PLAY_LOOK_AGAIN: &str = "/play/look/again";
/// Lock the look in (POST a `look::wire::LookLockAsk`, a `look::wire::LookStudioView`).
pub const PLAY_LOOK_LOCK: &str = "/play/look/lock";
/// The seat's hero build (GET, a `pages::creation::CreationView`).
pub const PLAY_CREATION: &str = "/play/creation";
/// Set and clear picks, then the rest is filled ("random unless chosen"); with nothing in it, it
/// starts the seat's build from its look-studio hero (POST a `pages::creation::BuildChoose`, a
/// `pages::creation::CreationView`).
pub const PLAY_CREATION_CHOOSE: &str = "/play/creation/choose";
/// "Surprise me": a new build from the hero's species and class (POST a
/// `pages::creation::BuildStep`, a `pages::creation::CreationView`).
pub const PLAY_CREATION_SURPRISE: &str = "/play/creation/surprise";
/// The reviewed build becomes the hero and the seat is ready in the lobby (POST a
/// `pages::creation::BuildStep`, a `pages::creation::CreationView`).
pub const PLAY_CREATION_SUBMIT: &str = "/play/creation/submit";
/// A ready seat goes back to reviewing and is not ready in the lobby (POST a
/// `pages::creation::BuildStep`, a `pages::creation::CreationView`).
pub const PLAY_CREATION_REOPEN: &str = "/play/creation/reopen";
/// The header that carries a phone's seat key on the `PLAY_*` routes, and never anywhere else:
/// not in an address, not in a log.
pub const SEAT_KEY_HEADER: &str = "x-df-seat-key";

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

/// The header in which the player listener gives a phone short-lived signed links to the media
/// its answer names, as `name=link;name=link`: the player listener serves a media file only with
/// its link (`MEDIA_LINK_PARAM`). The loopback dev listener gives none and needs none.
pub const MEDIA_LINKS_HEADER: &str = "x-df-media-links";
/// The query parameter that carries a media file's signed link.
pub const MEDIA_LINK_PARAM: &str = "ml";

thread_local! {
    /// The signed links the page was given, by media name (`keeps_link` says which one is kept).
    static MEDIA_LINKS: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// The expiry of a signed link (`<expiry, unix seconds>.<signature>`), read on the host's clock.
fn link_expiry(link: &str) -> Option<u64> {
    link.split_once('.')?.0.parse().ok()
}

/// Whether the page keeps the link `kept` it has for a file over the link `offered` in a newer
/// answer. Every answer signs anew, so taking each new link would give the file a new address on
/// every poll and the browser would fetch the whole picture again; the kept link is changed only
/// once the offered one lasts at least half a lifetime (`MEDIA_LINK_LIFETIME_S`) longer, so a kept
/// link always has half a lifetime left. Both expiries are the host's, so the phone's own clock
/// does not matter. A link that cannot be read is replaced.
pub fn keeps_link(kept: &str, offered: &str) -> bool {
    match (link_expiry(kept), link_expiry(offered)) {
        (Some(kept), Some(offered)) => {
            offered < kept.saturating_add(crate::limits::MEDIA_LINK_LIFETIME_S / 2)
        }
        _ => false,
    }
}

/// Keeps the signed links of a `MEDIA_LINKS_HEADER` value: a file's link only changes when
/// `keeps_link` lets it, so its address stays the same and the browser's copy is used.
pub fn remember_media_links(header: &str) {
    MEDIA_LINKS.with(|links| {
        let mut links = links.borrow_mut();
        for (name, link) in header.split(';').filter_map(|pair| pair.split_once('=')) {
            if links.get(name).is_some_and(|kept| keeps_link(kept, link)) {
                continue;
            }
            links.insert(name.to_owned(), link.to_owned());
        }
    });
}

/// Forgets every signed link: the host restarted, and its links died with it.
pub fn forget_media_links() {
    MEDIA_LINKS.with(|links| links.borrow_mut().clear());
}

/// The address of the media file `name`, with its signed link when the page was given one.
pub fn media_file(name: &str) -> String {
    let address = MEDIA_FILE.replace(NAME_PLACEHOLDER, name);
    match MEDIA_LINKS.with(|links| links.borrow().get(name).cloned()) {
        Some(link) => format!("{address}?{MEDIA_LINK_PARAM}={link}"),
        None => address,
    }
}

/// One page's timeline, `CLIENT_PLACEHOLDER` standing for the page's name (GET, with the
/// parameters of `timeline::TimelineQuery`). `client_timeline` fills it.
pub const CLIENT_TIMELINE: &str = "/playground/clients/{client}/timeline.json";
/// The part of `CLIENT_TIMELINE` that names the page, as the host's router writes a path
/// parameter.
pub const CLIENT_PLACEHOLDER: &str = "{client}";
/// The header every request of a page (`page-xxxxxxxx`, a phone's `phone-xxxxxxxx`) or of the dev
/// CLI (`cli`) carries, so the host can say who asked.
pub const CLIENT_HEADER: &str = "x-df-client";

/// The address of the timeline of the page `client`.
pub fn client_timeline(client: &str) -> String {
    CLIENT_TIMELINE.replace(CLIENT_PLACEHOLDER, client)
}

/// The address of the dice picture `name`.
pub fn dice_file(name: &str) -> String {
    DICE_FILE.replace(NAME_PLACEHOLDER, name)
}

/// The value `key` has in a page address's query (`?phone_kit=3&x=1`), if the query names it.
pub fn query_value<'query>(query: &'query str, key: &str) -> Option<&'query str> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}
