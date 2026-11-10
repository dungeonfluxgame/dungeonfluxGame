//! The usage line of every command and the text printed by `help`.
//!
//! The lines are the ones the parsers answer with when a command is not understood, so the help and
//! the errors cannot disagree. A limit that appears in the text is built from its constant.

use df_playground::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_TRANSITION_MS, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT,
};

use super::media_generate::{MEDIA_BACKEND_USAGE, MEDIA_GENERATE_USAGE};
use crate::cast::cli::{CAMERA_ON_USAGE, CAST_USAGE, CUE_USAGE};
use crate::locate::cli::MEDIA_LOCATE_USAGE;
use crate::movement::cli::MOVEMENT_USAGE;
use crate::persona::cli::{
    GUIDANCE_OPTIONS, PERSONA_ADD_USAGE, PERSONA_ASK_USAGE, PERSONA_BACKFILL_USAGE,
    PERSONA_BACKFILLS_USAGE, PERSONA_BEHAVIOR_USAGE, PERSONA_CONVERSE_USAGE,
    PERSONA_COVERAGE_USAGE, PERSONA_DROP_USAGE, PERSONA_EDIT_USAGE, PERSONA_EMOTION_USAGE,
    PERSONA_EXPERTISE_USAGE, PERSONA_GENERATE_USAGE, PERSONA_REMOVE_USAGE, PERSONA_SAY_USAGE,
    PERSONA_SECRET_USAGE, PERSONA_SET_USAGE, PERSONA_STATE_USAGE, PERSONA_TEMPERAMENT_USAGE,
    PERSONA_TRIGGER_USAGE, PERSONA_USAGE, PERSONA_VOICE_USAGE,
};
use crate::services::SERVICES_USAGE;
use crate::speech::SPEECH_USAGE;
use crate::speech::cli::{
    SPEECH_CAST_USAGE, SPEECH_SPEAK_USAGE, SPEECH_TRANSCRIBE_USAGE, SPEECH_VOICE_USAGE,
};
use crate::tempo::cli::TEMPO_USAGE;
use crate::world::cli::{WORLD_PLACE_USAGE, WORLD_QUOTE_USAGE, WORLD_USAGE};

pub(super) const SPAWN_USAGE: &str = "spawn <kind> <id> [x=N] [y=N] [field=value ...]";
pub(super) const SET_USAGE: &str = "set <id> <field>=<value> [<field>=<value> ...]";
pub(super) const MOVE_USAGE: &str = "move <id> <x> <y>";
pub(super) const TRAVEL_USAGE: &str = "travel <id> <x> <y> [speed=N]";
pub(super) const STOP_USAGE: &str = "stop <id> [<id> ...]";
pub(super) const ROTATE_USAGE: &str = "rotate <id> <degrees> [pivot=X,Y]";
pub(super) const DESPAWN_USAGE: &str = "despawn <id> [<id> ...]";
pub(super) const EVENTS_USAGE: &str =
    "events [after] [area=AREA] [limit=N] | events level [always|verbose]";
pub(super) const HOST_USAGE: &str = "host status";
pub(super) const FRAME_USAGE: &str = "frame [after=N]";
pub(super) const SCRIPT_USAGE: &str = "script <file>";
pub(super) const PLAYGROUND_USAGE: &str = concat!(
    "playground [set <key>=<value> ... | go <fixture> [transition=cut|fade|wipe] [ms=N] | cycle ",
    "<a> <b> [times=N] [hold=MS] [transition=..] [ms=N]]",
);
pub(super) const WAIT_USAGE: &str = "wait <milliseconds>";
/// Every sub-verb `scene` accepts, each with its own usage line or help entry.
pub(super) const SCENE_USAGE: &str = concat!(
    "scene caption <text> [speaker=NAME] | caption-clear | party ... | choices ... | image ... | ",
    "image-clear <slot> | slide ... | camera ... | camera-reset ... | camera-on ... | cast | ",
    "cue ... | music ... | music-stop | sound ... | reset | narration ... | seat ... | seats ... ",
    "| logo ... | clues ... | lobby ... | loading ... | roleplay ... | pill ... | home ... | ",
    "start ... | ",
    "build ... | show | lookahead ... | timing ..."
);
pub(super) const CAPTION_USAGE: &str = "scene caption <text> [speaker=NAME]";
pub(super) const PARTY_USAGE: &str = concat!(
    "scene party add <name> hp=N [max=N] [color=#rrggbb] [conditions=a,b] | set <name> [hp=N] ",
    "[max=N] [conditions=a,b] | remove <name> | clear",
);
pub(super) const IMAGE_USAGE: &str = concat!(
    "scene image <tv-backdrop|tv-card|phone> <name> [caption=TEXT] [rotate=0-359] ",
    "[plate=show] | scene image-clear <slot>",
);
pub(super) const SLIDE_USAGE: &str =
    "scene slide tv-card <name> [from=15] [to=85] [over=5000] [caption=TEXT] [rotate=0-359]";
pub(super) const SCENE_CAMERA_USAGE: &str =
    "scene camera <element> [zoom=200] [over=1200] | scene camera-reset [over=1200]";
pub(super) const MUSIC_USAGE: &str = concat!(
    "scene music <name> [on=both|tv|phone] [volume=0-100] [loop=true|false] | scene music ",
    "generate as=NAME [seconds=3-600] [on=both|tv|phone] [volume=0-100] <prompt...> | ",
    "scene music-stop"
);
pub(super) const SOUND_USAGE: &str = "scene sound <name> [on=both|tv|phone] [volume=0-100]";
pub(super) const NARRATION_USAGE: &str = concat!(
    "scene narration set <text> [speaker=NAME|none] [badge=GLYPH|none] [continue=TEXT|none] ",
    "[voice=true|false] [placement=center|right] [scene=I/N[:TITLE]|none] | edit <key>=<value> ",
    "... (the same keys and text=) | clear",
);
pub(super) const SEAT_USAGE: &str = concat!(
    "scene seat set <name> [class=TEXT|none] [level=N|none] [portrait=FILE|none] ",
    "[glyph=NAME|none] [accent=#rrggbb|none] [hp=N] [max=N] [conditions=a,b] | remove <name>",
);
pub(super) const SEATS_USAGE: &str = "scene seats open <N> | clear";
pub(super) const CLUES_USAGE: &str = concat!(
    "scene clues set \"id|name|detail|seen\" \"id|name|detail|worked|picture.png\" ... (oldest ",
    "first, the newest last) | scene clues clear",
);
pub(super) const LOGO_USAGE: &str = "scene logo on|off";
pub(super) const MEDIA_USAGE: &str =
    "media | media head <name> | media add <file> [as=NAME] | media remove <name>";
pub(super) const CHOICES_USAGE: &str = concat!(
    "scene choices suggested [<label>[=<explanation>] ...] | group <name> [<label> ...] | ",
    "unavailable [<label>=<reason> ...] | clear",
);
pub(super) const FIXTURE_USAGE: &str =
    "fixture show <name> | fixture write <name> <file> | fixture remove <name>";
pub(super) const REPL_USAGE: &str = "repl";
pub(super) const CLIENTS_USAGE: &str = concat!(
    "clients [<client>] | clients timeline <client> ",
    "[kind=line|plate|figure|clip|scene|sound|music|draw|camera|capture|transcribe|ask|outline|",
    "card|fade|third|clues|error] ",
    "[after=SEQ]",
);
pub(super) const FIXTURES_USAGE: &str = "fixtures";

/// The keys `playground set` accepts and what each takes.
pub(super) fn playground_keys() -> String {
    format!(
        "fixture=<name>, device=both|tv|phone, pace=slow|normal|fast, status=fixture|<status>, \
         unavailable=true|false, grid=true|false, sam=true|false, tts=true|false, \
         stt=true|false, transition=cut|fade|wipe, \
         ms=0-{MAXIMUM_TRANSITION_MS}, zoom_limit=model|pixels"
    )
}

/// The text printed by `help`.
pub fn usage_text() -> String {
    let video_limits = crate::media_library::video_limits_text();
    let playground_keys = playground_keys();
    let engine_help = crate::engine::help_text();
    let scene_builder_help = crate::scene_builder::help_text();
    let director_help = crate::director::help_text();
    let screens_help = format!(
        "{}{}{}{}{}{}",
        super::scene::lobby::HELP,
        super::scene::pills::HELP,
        super::scene::home::HELP,
        super::scene::home::configure::HELP,
        super::scene::home::adventure::HELP,
        super::scene::home::campaigns::HELP,
    );
    format!(
        "df-table dev [--port N] <command>\n\
         \n  {SPAWN_USAGE}\n      put an element on the canvas (x and y are its position; a \
         field\n      called x or y can be changed with set)\n\
         \n  {SET_USAGE}\n      change fields of an element\n\
         \n  {MOVE_USAGE}\n      move an element at once (x 0 to 100, y 0 to 56.25)\n\
         \n  {MOVEMENT_USAGE}\n      run a movement line through the scene transition: say runs every \
         stage,\n      interpret, design and brief stop after that stage (no provider is called); \
         plate=NAME\n      makes the new backdrop with the image service and shows the characters \
         in it; act has\n      the character say a line (through persona say) and runs that line \
         as a move, adding the\n      character to the movers when the line says \"I\" or \"we\"\n\
         \n  {TRAVEL_USAGE}\n      send an element toward a point; the host moves it a little \
         every tick, so it\n      glides there on its own (speed in canvas units per second, \
         default 20)\n\
         \n  {STOP_USAGE}\n      end travel where the element stands\n\
         \n  {ROTATE_USAGE}\n      turn an element clockwise to a whole number of degrees, 0 to \
         359 (180 is upside\n      down), about its centre or about pivot=X,Y, each a whole \
         percent of its own width and\n      height from its top left corner (0,50 is a hinge on \
         the left edge); without pivot= it\n      keeps the pivot it has; changing the pivot of \
         an element that is already turned moves\n      its position so the figure does not jump\n\
         \n  {REPL_USAGE}\n      read commands from standard input, one per line, until quit; a \
         command costs\n      about a millisecond instead of a process start\n\
         \n  {DESPAWN_USAGE}\n      remove elements\n\
         \n  clear\n      remove every element\n\
         \n  canvas\n      print the canvas as JSON\n\
         \n  kinds\n      list the kinds and their fields\n\
         \n  {EVENTS_USAGE}\n      print the newest events, or those after a sequence, each \
         with the time it was recorded (at_ms);\n      area= keeps one area (the first word of a \
         line, such as LOCATE) and limit= the newest N;\n      a forgotten=N header says how \
         many lines the log dropped to keep its bounds; `events level`\n      reads the event \
         level of the host's sinks and `events level verbose` also records the class B\n      \
         detail (STEP, PROJECTION), `events level always` goes back to class A only\n\
         \n  {HOST_USAGE}\n      the audit log (lines kept, forgotten, echo), the event sinks \
         (level, counts, lag and loss,\n      the shared path to the telemetry store) and the \
         clock\n\
         \n  {FRAME_USAGE}\n      the playground's frame: its count, the run of the host, its size \
         and top-level keys; with\n      after=N it waits for the frame after N (or says nothing \
         changed)\n\
         \n  reload\n      re-read the kind files without a restart\n\
         \n  {PLAYGROUND_USAGE}\n      show or change what the playground page shows; keys: \
         {playground_keys}\n      go and cycle change the scene and the transition together or \
         not at all\n\
         \n  {SCENE_USAGE}\n      change the playground's caption, party strip or phone choices \
         live;\n      {PARTY_USAGE}\n      {CHOICES_USAGE}\n      a fixture is not changed; scene \
         reset or another fixture goes back to the file\n\
         \n  {IMAGE_USAGE}\n      put a picture from the media library on the shared screen \
         (tv-backdrop behind the\n      figures, tv-card in the middle) or at the top of a \
         phone's story; plate=show marks the backdrop as the day's show plate, which the page \
         shows without the inventory panel\n\
         \n  {SLIDE_USAGE}\n      slide the card across the shared screen (defaults: from 15 to \
         85 percent of its width, over\n      5 seconds); a redraw part-way carries on from where \
         it was\n\
         \n  {SCENE_CAMERA_USAGE}\n      \
         move the shared screen's camera in on an element and keep it in the middle \
         (zoom is\n      \
         {MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT} percent, over is 0 to \
         {MAXIMUM_CAMERA_MS} ms; defaults 200 and 1200), or back out to the whole\n      \
         stage; the caption, card and party strip stay put\n\
         \n  {CAMERA_ON_USAGE}\n      \
         frame a character named in the backdrop's SAM layout (media locate label): \
         the shot's focus and zoom\n      \
         come from its mask (face, upper body or the whole figure); \
         `scene camera region:<name>[:<shot>] zoom=N`\n      \
         aims the plain camera at the same targets\n\
         \n  {CAST_USAGE}\n      \
         the named characters of the shown scene's backdrop (from its SAM \
         layout): name, region id and number,\n      \
         head, face and centre; what a chat system reads to know who is in the shot\n\
         \n  {CUE_USAGE}\n      \
         a chat system's cue about one character by name: a wordless thinking indicator, \
         a bubble with a\n      \
         spoken line, or nothing; each character has one bubble of its own, anchored to \
         its mask (an engine\n      \
         posts the same JSON to /dev/scene/cue)\n\
         \n  {MUSIC_USAGE}\n      play music until it is stopped or replaced; generate plays \
         the library's sound NAME\n      looping, or first composes it from the prompt \
         (instrumental, 120 seconds unless told)\n      and stores it as NAME.mp3, so one \
         track is made per narrative scene\n\
         \n  {SOUND_USAGE}\n      play a sound once\n\
         \n  {NARRATION_USAGE}\n      the narrator's box in the caption's place: set it from its \
         text, change any of its\n      fields (speaker, badge, continue label, voice mark, \
         placement, scene progress; none takes a\n      field away) or clear it\n\
         \n  {SEAT_USAGE}\n      a hero of the party strip as a seat card: look (class, level, \
         portrait, glyph, accent) and\n      hit points; a hero not yet in the party needs hp=\n\
         \n  {SEATS_USAGE}\n      open seats after the heroes, or take every look and open seat \
         away\n\
         \n  {LOGO_USAGE}\n      the DungeonFlux logo at the top left of the shared screen\n\
         \n  {CLUES_USAGE}\n      the clue board down the left of the shared screen: every clue \
         found so far, one quoted word each: id, name, detail, seen or worked (resolved)\n      \
         and an optional picture from the library; or take it away\n\
         \n  {MEDIA_USAGE}\n      list, add (a local file, copied into the library) or remove \
         pictures, sounds and videos;\n      {video_limits}; a video's size is in the listing; \
         videos are not placed on a screen yet;\n      the listing ends with the library's \
         limits; head asks for a file without fetching it and\n      prints its status, type, \
         length and ETag\n\
         \n  {MEDIA_GENERATE_USAGE}\n      make one picture from a prompt through the image \
         service (the Codex CLI backend) and store it in\n      the library as NAME.png; the host \
         needs DF_IMAGERY_MODE=live, DF_IMAGERY_BACKEND=codex-cli \
         and\n      CODEX_HOME=<signed-in folder> in its environment; a picture takes minutes, so \
         without wait=true\n      it runs on and `media generate status` shows it\n\
         \n  {MEDIA_BACKEND_USAGE}\n      show the image backend new pictures are made with, \
         or switch it while the host runs\n      (Forge takes the url and project the host was \
         started with unless given); jobs already\n      running finish on the backend they \
         started on\n\
         \n  {MEDIA_LOCATE_USAGE}\n      \
         find the people (or heads) in a library picture with a SAM 3.1 segmentation through the \
         imagery\n      \
         API (fal) and store its masks and head points as layouts/<picture>.layout.json; a scene \
         whose backdrop is that\n      \
         picture gets the layout, its bubbles can anchor to a named region and the SAM layer draws \
         it\n      \
         (playground set sam=true|false); show prints it, points prints each named character's \
         points and shots, label names region N (- clears);\n      \
         identify asks the text service (Responses with GPT-6 Luna, or Codex; \
         DF_TEXT_IDENTITY_MODEL picks the model) which region is which\n      \
         character from words and library portraits, and names only the matches it is sure of \
         (min=, default 600);\n      \
         the host needs DF_FAL_KEY or FAL_KEY (scans), and for identify either OPENAI_API_KEY or \
         DF_TEXT_BACKEND=codex\n      \
         with DF_TEXT_CODEX_EXECUTABLE (the signed-in Codex CLI)\n\
         \n  {CLIENTS_USAGE}\n      what each open page says it is showing and playing, and \
         whether its sound is\n      locked (a browser plays sound only after someone clicks the \
         page); each page also shows its frame\n      against the host's, its link, the revision \
         and fixture it loaded; `clients <client>` is one\n      page; `clients timeline` lists \
         when the page started and ended each line, picture, figure,\n      clip, scene, sound, \
         camera framing and draw, why, and how long it was meant to last\n\
         \n  {SERVICES_USAGE}\n      how the text, image and locate services are set up and stand \
         (backend, model, whether a key is\n      present, never the key)\n\
         \n  {PERSONA_USAGE}\
         \n      the personas of characters (who each is, their job, how they talk, what they \
         keep back), one file each in the playground's personas folder, linked to a \
         character of the shown scene by equal text: list them; show one (public is \
         what any listener may know, owner also shows the private side, for the \
         operator only); write one from a JSON file (replaces the same id); or change \
         one with typed edits, applied all together or not at all:\
         \n      {PERSONA_SET_USAGE}   (display_name, job, speech_style, backstory, \
         private_backstory)\
         \n      {PERSONA_ADD_USAGE}   {PERSONA_DROP_USAGE}   (personality, motivations, goals, \
         needs, fears, boundaries)\
         \n      {PERSONA_SECRET_USAGE}\
         \n      {PERSONA_BEHAVIOR_USAGE}\
         \n      {PERSONA_TRIGGER_USAGE}\
         \n      {PERSONA_VOICE_USAGE}   (rhythm, address, deflection are texts; favored, never, \
         tics, idioms, samples are lists; samples are style only; generate asks the model for \
         only the voice, expertise and voice packs, packs chosen from the world bible's list)\
         \n      {PERSONA_EXPERTISE_USAGE}   (what the character knows from their trade or life)\
         \n      {PERSONA_TEMPERAMENT_USAGE}   (where feelings rest and how strongly they move, \
         0 to 100; generate asks the model for the temperament and its emotional triggers)\
         \n      {PERSONA_EMOTION_USAGE}   (a trigger on a feeling: state=<emotion>:<at_least> is \
         a number checked by code, the event is a statement a model judges; set= moves \
         emotions, behavior= switches stance)\
         \n      {PERSONA_STATE_USAGE}   (the live feelings, mood, attention and attitudes, kept \
         in persona-state/<id>.json; the emotion list is persona/emotions.json, read fresh)\
         \n      {PERSONA_EDIT_USAGE}   applies a file {{\"edits\": [...]}} of any edits. Quote \
         a value with spaces. A trigger or a reveal condition is a plain statement a \
         model judges, never a keyword.\
         \n      {PERSONA_GENERATE_USAGE}   has the text service write the persona of one named \
         character of the shown scene, or of all of them at once, keeping a stored \
         persona unless overwrite=true.\
         \n      {PERSONA_SAY_USAGE}   has it word one line from a character's persona and cues \
         it on the scene (a dev tool: it reports the behavior, triggers and secrets the \
         model says the line used; the production engine, not the model, will own \
         secret reveals).\
         \n      {PERSONA_CONVERSE_USAGE}   has the characters of the shown scene talk to each \
         other, one `persona say` per turn: the next speaker is whoever the last line \
         named (unless they spoke in the last three turns, then whoever has been quiet \
         longest), each line waits for its reading time or its voice (pace=false does \
         not), and a topic is the first thing on the table; plate=NAME then has the image          service paint a new tv backdrop from the talk (it takes minutes, the scene shows a          spinner meanwhile), scans it for the people and names them, and puts it behind the          scene (needs the imagery and locate settings of media generate and media locate).\
         \n      {PERSONA_ASK_USAGE}   puts a question to a character: the text service judges \
         whether the persona holds enough to answer, fills what it lacks by adding \
         detail only (recorded in personas/<id>.backfills.json; a failed backfill still \
         gets an answer), and the character answers in character, cued on the scene.\
         \n      {PERSONA_COVERAGE_USAGE}   only judges. {PERSONA_BACKFILL_USAGE}   only fills \
         the named gaps (fields: backstory, private_backstory, personality, \
         motivations, goals, needs, fears, boundaries, secrets, behaviors, triggers). \
         {PERSONA_BACKFILLS_USAGE}   lists what was added.\
         \n      The guidance options, for generate, ask, coverage and backfill: \
         {GUIDANCE_OPTIONS}. Keys given here replace the file's; hook, thread, \
         establish and avoid repeat; without guidance a backfill is a generic story \
         detail.\
         \n      recent=<text> (repeat) gives say and ask the last lines of the talk so the \
         character never reuses a phrase; from=<id> says who speaks to them, so triggers \
         are judged by what the asker does and not by an accepted offer.\
         \n      model=<name> asks that model instead of the default (gpt-6-luna, or \
         DF_TEXT_PERSONA_MODEL); see services for the profile each uses.\
         \n      {PERSONA_REMOVE_USAGE}   deletes one (its backfill record stays, as history). \
         scene cast shows each cast member's job.\n\
         \n  {WORLD_USAGE}\
         \n      the world bible, canonical facts a designer edits as data in the playground's \
         world/bible.json (read fresh on every call, no rebuild): items and their values in \
         copper, places with stalls (keeper, items sold, bulk tiers, haggle floor) and voice \
         packs. The persona prompts are given the facts of the active place and the packs a \
         character belongs to. Prices are computed here; a persona voices them and never \
         invents one. {WORLD_PLACE_USAGE} sets the active place (- clears it); \
         {WORLD_QUOTE_USAGE} prices a lot at the active place (list, bulk, lowest the keeper \
         takes); validate reads the bible now and reports what is wrong with it.\n\
         \n  {TEMPO_USAGE}\
         \n      the tempo engine (df-tempo) and the music director: tempo load reads a music \
         table (the policy, the impulse kinds and the profiles, all data), tempo prepare finds \
         or composes every profile's track once, tempo impulse posts an event of the journey, \
         and the director plays the profile the tempo calls for, with a minimum dwell and a dip \
         fade between tracks; tempo reset starts the journey again on the base profile.\n\
         \n  {SPEECH_USAGE}\
         \n      voice chat through the speech service (see services): the status and the \
         voices; {SPEECH_SPEAK_USAGE} voices operator text in a character's voice (- for the \
         default voice), keeps it as a voice-<n>.mp3 clip in the media library and plays it \
         as a scene sound; {SPEECH_TRANSCRIBE_USAGE} prints what a 16 kHz mono 16-bit PCM or \
         WAV recording says; {SPEECH_VOICE_USAGE} sets or clears a character's voice in \
         voices.json; {SPEECH_CAST_USAGE} has a judge choose the character's voice from the \
         supplier's list by their persona (a character with a persona and no voice is cast so \
         before their first line); clips lists the clips the host made and clear deletes them. \
         persona \
         ask and say take speak=true to voice the line and start it with its bubble, and \
         ask takes coverage=false to skip the coverage question.\n\
         \n  {WAIT_USAGE}\n      pause here; stage scenes over time in a script or a session\n\
         \n  {FIXTURES_USAGE}\n      list the playground's fixture files\n\
         \n  {FIXTURE_USAGE}\n      read, write (checked first) or delete a fixture file\n\
         \n  {SCRIPT_USAGE}\n      run a file of commands, one per line; a line may start with #\n\
         {engine_help}{scene_builder_help}{director_help}{screens_help}\n  help\n      this text\n\
         \nQuote a value with spaces: name=\"Pip the Brave\".\n\
         Several effects in one command are applied in order and every failure is reported;\n\
         a script stops at its first failing line.\n\
         The port is --port N, or the DF_TABLE_PORT environment variable, or 7878."
    )
}
