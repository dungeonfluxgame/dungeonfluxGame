//! The usage line of every command and the text printed by `help`.
//!
//! The lines are the ones the parsers answer with when a command is not understood, so the help and
//! the errors cannot disagree. A limit that appears in the text is built from its constant.

use df_playground::controls::MAXIMUM_TRANSITION_MS;
use df_playground::scene::{MAXIMUM_CAMERA_MS, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT};

use super::media_generate::MEDIA_GENERATE_USAGE;

pub(super) const SPAWN_USAGE: &str = "spawn <kind> <id> [x=N] [y=N] [field=value ...]";
pub(super) const SET_USAGE: &str = "set <id> <field>=<value> [<field>=<value> ...]";
pub(super) const MOVE_USAGE: &str = "move <id> <x> <y>";
pub(super) const TRAVEL_USAGE: &str = "travel <id> <x> <y> [speed=N]";
pub(super) const STOP_USAGE: &str = "stop <id> [<id> ...]";
pub(super) const ROTATE_USAGE: &str = "rotate <id> <degrees> [pivot=X,Y]";
pub(super) const DESPAWN_USAGE: &str = "despawn <id> [<id> ...]";
pub(super) const EVENTS_USAGE: &str = "events [after]";
pub(super) const SCRIPT_USAGE: &str = "script <file>";
pub(super) const PLAYGROUND_USAGE: &str = "playground [set <key>=<value> ... | go <fixture> [transition=cut|fade|wipe] [ms=N] | cycle <a> <b> [times=N] [hold=MS] [transition=..] [ms=N]]";
pub(super) const WAIT_USAGE: &str = "wait <milliseconds>";
pub(super) const SCENE_USAGE: &str = "scene caption <text> [speaker=NAME] | caption-clear | party ... | choices ... | image ... | camera ... | music ... | sound ... | reset";
pub(super) const CAPTION_USAGE: &str = "scene caption <text> [speaker=NAME]";
pub(super) const PARTY_USAGE: &str = "scene party add <name> hp=N [max=N] [color=#rrggbb] [conditions=a,b] | set <name> [hp=N] [max=N] [conditions=a,b] | remove <name> | clear";
pub(super) const IMAGE_USAGE: &str = "scene image <tv-backdrop|tv-card|phone> <name> [caption=TEXT] [rotate=0-359] | scene image-clear <slot>";
pub(super) const SLIDE_USAGE: &str =
    "scene slide tv-card <name> [from=15] [to=85] [over=5000] [caption=TEXT] [rotate=0-359]";
pub(super) const CAMERA_USAGE: &str =
    "scene camera <element> [zoom=200] [over=1200] | scene camera-reset [over=1200]";
pub(super) const MUSIC_USAGE: &str =
    "scene music <name> [on=both|tv|phone] [volume=0-100] [loop=true|false] | scene music-stop";
pub(super) const SOUND_USAGE: &str = "scene sound <name> [on=both|tv|phone] [volume=0-100]";
pub(super) const NARRATION_USAGE: &str = "scene narration set <text> [speaker=NAME|none] [badge=GLYPH|none] [continue=TEXT|none] [voice=true|false] [placement=center|right] [scene=I/N[:TITLE]|none] | edit <key>=<value> ... (the same keys and text=) | clear";
pub(super) const SEAT_USAGE: &str = "scene seat set <name> [class=TEXT|none] [level=N|none] [portrait=FILE|none] [glyph=NAME|none] [accent=#rrggbb|none] [hp=N] [max=N] [conditions=a,b] | remove <name>";
pub(super) const SEATS_USAGE: &str = "scene seats open <N> | clear";
pub(super) const LOGO_USAGE: &str = "scene logo on|off";
pub(super) const MEDIA_USAGE: &str = "media | media add <file> [as=NAME] | media remove <name>";
pub(super) const CHOICES_USAGE: &str = "scene choices suggested [<label>[=<explanation>] ...] | group <name> [<label> ...] | unavailable [<label>=<reason> ...] | clear";
pub(super) const FIXTURE_USAGE: &str =
    "fixture show <name> | fixture write <name> <file> | fixture remove <name>";
pub(super) const REPL_USAGE: &str = "repl";
pub(super) const CLIENTS_USAGE: &str = "clients";
pub(super) const FIXTURES_USAGE: &str = "fixtures";

/// The keys `playground set` accepts and what each takes.
pub(super) fn playground_keys() -> String {
    format!(
        "fixture=<name>, device=both|tv|phone, pace=slow|normal|fast, status=fixture|<status>, unavailable=true|false, grid=true|false, transition=cut|fade|wipe, ms=0-{MAXIMUM_TRANSITION_MS}"
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
        super::scene_lobby::HELP,
        super::scene_pills::HELP,
        super::scene_home::HELP,
        super::scene_home_configure::HELP,
        super::scene_home_adventure::HELP,
        super::scene_home_campaigns::HELP,
    );
    format!(
        "df-table dev [--port N] <command>\n\
         \n  {SPAWN_USAGE}\n      put an element on the canvas (x and y are its position; a field\n      called x or y can be changed with set)\n\
         \n  {SET_USAGE}\n      change fields of an element\n\
         \n  {MOVE_USAGE}\n      move an element at once (x 0 to 100, y 0 to 56.25)\n\
         \n  {TRAVEL_USAGE}\n      send an element toward a point; the host moves it a little every tick, so it\n      glides there on its own (speed in canvas units per second, default 20)\n\
         \n  {STOP_USAGE}\n      end travel where the element stands\n\
         \n  {ROTATE_USAGE}\n      turn an element clockwise to a whole number of degrees, 0 to 359 (180 is upside\n      down), about its centre or about pivot=X,Y, each a whole percent of its own width and\n      height from its top left corner (0,50 is a hinge on the left edge); without pivot= it\n      keeps the pivot it has; changing the pivot of an element that is already turned moves\n      its position so the figure does not jump\n\
         \n  {REPL_USAGE}\n      read commands from standard input, one per line, until quit; a command costs\n      about a millisecond instead of a process start\n\
         \n  {DESPAWN_USAGE}\n      remove elements\n\
         \n  clear\n      remove every element\n\
         \n  canvas\n      print the canvas as JSON\n\
         \n  kinds\n      list the kinds and their fields\n\
         \n  {EVENTS_USAGE}\n      print the newest events, or those after a sequence\n\
         \n  reload\n      re-read the kind files without a restart\n\
         \n  {PLAYGROUND_USAGE}\n      show or change what the playground page shows; keys: {playground_keys}\n      go and cycle change the scene and the transition together or not at all\n\
         \n  {SCENE_USAGE}\n      change the playground's caption, party strip or phone choices live;\n      {PARTY_USAGE}\n      {CHOICES_USAGE}\n      a fixture is not changed; scene reset or another fixture goes back to the file\n\
         \n  {IMAGE_USAGE}\n      put a picture from the media library on the shared screen (tv-backdrop behind the\n      figures, tv-card in the middle) or at the top of a phone's story\n\
         \n  {SLIDE_USAGE}\n      slide the card across the shared screen (defaults: from 15 to 85 percent of its width, over\n      5 seconds); a redraw part-way carries on from where it was\n\
         \n  {CAMERA_USAGE}\n      move the shared screen's camera in on an element and keep it in the middle (zoom is\n      {MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT} percent, over is 0 to {MAXIMUM_CAMERA_MS} ms; defaults 200 and 1200), or back out to the whole\n      stage; the caption, card and party strip stay put\n\
         \n  {MUSIC_USAGE}\n      play music until it is stopped or replaced\n\
         \n  {SOUND_USAGE}\n      play a sound once\n\
         \n  {NARRATION_USAGE}\n      the narrator's box in the caption's place: set it from its text, change any of its\n      fields (speaker, badge, continue label, voice mark, placement, scene progress; none takes a\n      field away) or clear it\n\
         \n  {SEAT_USAGE}\n      a hero of the party strip as a seat card: look (class, level, portrait, glyph, accent) and\n      hit points; a hero not yet in the party needs hp=\n\
         \n  {SEATS_USAGE}\n      open seats after the heroes, or take every look and open seat away\n\
         \n  {LOGO_USAGE}\n      the DungeonFlux logo at the top left of the shared screen\n\
         \n  {MEDIA_USAGE}\n      list, add (a local file, copied into the library) or remove pictures, sounds and videos;\n      {video_limits}; a video's size is in the listing; videos are not placed on a screen yet\n\
         \n  {MEDIA_GENERATE_USAGE}\n      make one picture from a prompt through the image service (the Codex CLI backend) and store it in\n      the library as NAME.png; the host needs DF_IMAGERY_MODE=live, DF_IMAGERY_BACKEND=codex-cli and\n      CODEX_HOME=<signed-in folder> in its environment; a picture takes minutes, so without wait=true\n      it runs on and `media generate status` shows it\n\
         \n  {CLIENTS_USAGE}\n      what each open page says it is showing and playing, and whether its sound is\n      locked (a browser plays sound only after someone clicks the page)\n\
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
