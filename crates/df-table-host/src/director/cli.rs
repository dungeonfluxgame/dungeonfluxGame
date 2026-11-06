//! The dev command line of the Director: `camera focus|wide` and `director ...` (O.14, G1).
//!
//! It owns the grammar and the usage text of these verbs, and running one against a host on a
//! port. Each verb has the shape of the other dev commands: words in, one typed
//! `DirectorCommand` out, a usage line on a mistake (`CommandError::Usage`), and a numbered answer
//! (the audit lines the verb wrote, with the move's serial). They work from the repl and from a
//! script like any other command, because `command::parse` calls `parse` for the verbs in
//! `VERBS`. A verb that drives the camera marks its lines `driven=true`. Element names and reasons
//! are checked here, so a mistake is named before anything is sent.
//!
//! The existing `scene camera` and `scene camera-reset` stay as they are: they are the workbench's
//! free camera, which ignores the Director's rules. `camera focus` is a Director direction with a
//! reason, so every rule of O.14 applies to it.

use std::str::FromStr;

use df_table::director::reason::Reason;
use df_table::element::ElementId;

use super::format::{format_act, format_show};
use super::request::{ActReply, DirectorAct, DirectorReply, ShowReply, StartKind, ViewEdit};
use super::routes::path;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

pub const CAMERA_USAGE: &str = concat!(
    "camera focus <element> [reason=<reason>] [zoom=N] [over=MS] [hold=MS]",
    " | camera wide [over=MS]",
);
pub const DIRECTOR_USAGE: &str = concat!(
    "director show | pause | resume | auto camera on|off | advance <ms>",
    " | end <reason> <element> | start scene|combat | view ...",
);
pub const VIEW_USAGE: &str = concat!(
    "director view hide|unhide|hero <element> | card | plate [width=PX] [stage=PX]",
    " | collapsed on|off | reduced on|off",
    " | beat result-card|reveal-beat <ms> [element=<id>] | clear",
);

/// The top-level verbs this module parses, for the dispatch in `command::parse`.
pub const VERBS: [&str; 2] = ["camera", "director"];

/// What an element name may hold, for the refusal of one that is not.
const ELEMENT_NAME_RULE: &str =
    "an element name (a to z, 0 to 9, '_' and '-', starting with a letter)";
const MILLISECONDS: &str = "a whole number of milliseconds";

/// A command for the Director.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectorCommand {
    Act(DirectorAct),
    Show,
}

/// The usage line of every verb, in the order `help` lists them.
pub fn verb_usages() -> [&'static str; 3] {
    [CAMERA_USAGE, DIRECTOR_USAGE, VIEW_USAGE]
}

/// The help text of these verbs, to follow the other commands' in `help`.
pub fn help_text() -> String {
    format!(
        concat!(
            "\n  {camera}\n",
            "      the Director moves the shared screen's camera: focus is a direction with a\n",
            "      reason (one of: {reasons}; speaker when none is given), so the rules of\n",
            "      the plan apply: a lower reason never interrupts a higher one, one new move\n",
            "      in 800 ms, never during a result card, a hidden element is refused, the\n",
            "      zoom is held to what the plate can bear, and it returns wide when its hold\n",
            "      ends. The lines it writes say driven=true\n",
            "\n  {director}\n",
            "      show lists the directions queued, applied and dropped, with reasons, and\n",
            "      what the Director sees; pause holds every automatic direction while you\n",
            "      drive by hand; auto turns the camera class on or off; advance moves the\n",
            "      Director's own clock so a hold runs out without waiting; end says the\n",
            "      reason of that element's line or turn ended; start says a scene or combat\n",
            "      began\n",
            "\n  {view}\n",
            "      what the Director is shown, until the engine reports it: hide an element\n",
            "      from the table (it cannot be focused, and a camera on it pulls back at\n",
            "      once), mark a hero (always focusable), show a card instead of a plate (no\n",
            "      focus is possible), the plate's pixel width and the stage's device pixels\n",
            "      (the zoom ceiling), a collapsed beat, reduced effects, or a result card or\n",
            "      reveal beat that holds the camera still\n",
        ),
        camera = CAMERA_USAGE,
        director = DIRECTOR_USAGE,
        view = VIEW_USAGE,
        reasons = Reason::keys(),
    )
}

fn bad_choice(key: &str, text: &str, allowed: &str) -> CommandError {
    CommandError::BadChoice {
        key: key.to_owned(),
        text: text.to_owned(),
        allowed: allowed.to_owned(),
    }
}

/// Reads `key=value` words, refusing a key given twice and any key not in `keys`.
fn pairs<'a>(
    words: &'a [String],
    keys: &[&str],
    allowed: &str,
) -> Result<Vec<(&'a str, &'a str)>, CommandError> {
    let mut found: Vec<(&str, &str)> = Vec::new();
    for word in words {
        let (key, value) = word
            .split_once('=')
            .filter(|(key, _)| !key.is_empty())
            .ok_or_else(|| CommandError::BadPair(word.clone()))?;
        if !keys.contains(&key) {
            return Err(bad_choice("this command", key, allowed));
        }
        if found.iter().any(|(seen, _)| *seen == key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        found.push((key, value));
    }
    Ok(found)
}

fn whole<T: FromStr>(key: &str, text: &str, what: &str) -> Result<T, CommandError> {
    text.parse::<T>().map_err(|_| bad_choice(key, text, what))
}

fn switch(word: &str, text: &str) -> Result<bool, CommandError> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(bad_choice(word, text, "on or off")),
    }
}

fn element(key: &str, text: &str) -> Result<ElementId, CommandError> {
    ElementId::new(text).map_err(|_| bad_choice(key, text, ELEMENT_NAME_RULE))
}

fn reason(key: &str, text: &str) -> Result<Reason, CommandError> {
    Reason::from_key(text).ok_or_else(|| bad_choice(key, text, &Reason::keys()))
}

fn focus(words: &[String]) -> Result<DirectorCommand, CommandError> {
    let [name, rest @ ..] = words else {
        return Err(CommandError::Usage(CAMERA_USAGE));
    };
    let allowed = "reason=, zoom=N, over=MS, hold=MS";
    let (mut chosen, mut zoom_percent, mut over_ms, mut hold_ms) = (None, None, None, None);
    for (key, value) in pairs(rest, &["reason", "zoom", "over", "hold"], allowed)? {
        match key {
            "reason" => chosen = Some(reason(key, value)?),
            "zoom" => {
                zoom_percent = Some(whole(
                    key,
                    value,
                    "a whole number of percent (200 is twice as large)",
                )?);
            }
            "over" => over_ms = Some(whole(key, value, MILLISECONDS)?),
            _ => hold_ms = Some(whole(key, value, MILLISECONDS)?),
        }
    }
    Ok(DirectorCommand::Act(DirectorAct::Focus {
        element: element("element", name)?,
        reason: chosen,
        zoom_percent,
        over_ms,
        hold_ms,
    }))
}

fn camera(words: &[String]) -> Result<DirectorCommand, CommandError> {
    match words.split_first() {
        Some((verb, rest)) if verb == "focus" => focus(rest),
        Some((verb, rest)) if verb == "wide" => {
            let mut over_ms = None;
            for (key, value) in pairs(rest, &["over"], "over=MS")? {
                over_ms = Some(whole(key, value, MILLISECONDS)?);
            }
            Ok(DirectorCommand::Act(DirectorAct::Wide { over_ms }))
        }
        _ => Err(CommandError::Usage(CAMERA_USAGE)),
    }
}

fn plate(words: &[String]) -> Result<ViewEdit, CommandError> {
    let (mut width_px, mut stage_px) = (None, None);
    for (key, value) in pairs(words, &["width", "stage"], "width=PX, stage=PX")? {
        let pixels = Some(whole(key, value, "a whole number of pixels")?);
        if key == "width" {
            width_px = pixels;
        } else {
            stage_px = pixels;
        }
    }
    Ok(ViewEdit::Plate { width_px, stage_px })
}

fn beat(kind: &str, hold: &str, words: &[String]) -> Result<ViewEdit, CommandError> {
    let reveal = match kind {
        "result-card" => false,
        "reveal-beat" => true,
        other => return Err(bad_choice("beat", other, "result-card or reveal-beat")),
    };
    let mut subject = None;
    for (key, value) in pairs(words, &["element"], "element=<id>")? {
        subject = Some(element(key, value)?);
    }
    Ok(ViewEdit::Beat {
        reveal,
        hold_ms: whole("beat", hold, MILLISECONDS)?,
        element: subject,
    })
}

fn view(words: &[String]) -> Result<DirectorCommand, CommandError> {
    let edit = match words {
        [verb, name] if verb == "hide" => ViewEdit::Hide {
            element: element("hide", name)?,
        },
        [verb, name] if verb == "unhide" => ViewEdit::Unhide {
            element: element("unhide", name)?,
        },
        [verb, name] if verb == "hero" => ViewEdit::Hero {
            element: element("hero", name)?,
        },
        [verb] if verb == "card" => ViewEdit::Card,
        [verb] if verb == "clear" => ViewEdit::Clear,
        [verb, rest @ ..] if verb == "plate" => plate(rest)?,
        [verb, on] if verb == "collapsed" => ViewEdit::Collapsed {
            on: switch("collapsed", on)?,
        },
        [verb, on] if verb == "reduced" => ViewEdit::Reduced {
            on: switch("reduced", on)?,
        },
        [verb, kind, hold, rest @ ..] if verb == "beat" => beat(kind, hold, rest)?,
        _ => return Err(CommandError::Usage(VIEW_USAGE)),
    };
    Ok(DirectorCommand::Act(DirectorAct::View { edit }))
}

fn director(words: &[String]) -> Result<DirectorCommand, CommandError> {
    let act = |act| Ok(DirectorCommand::Act(act));
    match words {
        [verb] if verb == "show" => Ok(DirectorCommand::Show),
        [verb] if verb == "pause" => act(DirectorAct::Pause),
        [verb] if verb == "resume" => act(DirectorAct::Resume),
        [verb, class, on] if verb == "auto" && class == "camera" => act(DirectorAct::Auto {
            on: switch("auto camera", on)?,
        }),
        [verb, class, _] if verb == "auto" && matches!(class.as_str(), "slide" | "cue") => {
            Err(bad_choice(
                "auto",
                class,
                "camera (the slide and cue classes arrive with their stages)",
            ))
        }
        [verb, by] if verb == "advance" => act(DirectorAct::Advance {
            by_ms: whole("advance", by, MILLISECONDS)?,
        }),
        [verb, key, name] if verb == "end" => act(DirectorAct::End {
            reason: reason("end", key)?,
            element: element("end", name)?,
        }),
        [verb, what] if verb == "start" => act(DirectorAct::Start {
            what: StartKind::from_key(what)
                .ok_or_else(|| bad_choice("start", what, "scene or combat"))?,
        }),
        [verb, rest @ ..] if verb == "view" => view(rest),
        _ => Err(CommandError::Usage(DIRECTOR_USAGE)),
    }
}

/// Parses one top-level verb of this module. `verb` is one of `VERBS`.
pub fn parse(verb: &str, words: &[String]) -> Result<DirectorCommand, CommandError> {
    match verb {
        "camera" => camera(words),
        "director" => director(words),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// The value of a Director reply, or its refusal (the typed `DirectorError`) as the command's
/// failure.
fn reply_value<T>(reply: DirectorReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

/// Runs one Director command against the host on `port` and returns the text to print.
pub async fn run(port: u16, command: DirectorCommand) -> Result<String, DevError> {
    match command {
        DirectorCommand::Act(act) => {
            let reply: ActReply = reply_value(post_json(port, path::ACT, &act).await?)?;
            Ok(format_act(&reply))
        }
        DirectorCommand::Show => {
            let show: ShowReply = reply_value(get_json(port, path::SHOW).await?)?;
            Ok(format_show(&show))
        }
    }
}
