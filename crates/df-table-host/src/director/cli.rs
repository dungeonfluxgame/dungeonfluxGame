//! The dev command line of the Director: `camera focus|wide` and `director ...` (O.14, G1).
//!
//! It owns the grammar and the usage text of these verbs, and running one against a host on a
//! port. Each verb has the shape of the other dev commands: words in, one typed
//! `DirectorCommand` out, a usage line on a mistake (`CommandError::Usage`), and a numbered answer
//! (the audit lines the verb wrote, with the move's serial). They work from the repl and from a
//! script like any other command, because `command::parse` calls `parse` for the verbs in
//! `VERBS`. A verb that drives the camera marks its lines `driven=true`.
//!
//! The existing `scene camera` and `scene camera-reset` stay as they are: they are the workbench's
//! free camera, which ignores the Director's rules. `camera focus` is a Director direction with a
//! reason, so every rule of O.14 applies to it.

use df_table::director_reason::Reason;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::format::{format_act, format_show};
use super::request::{ActReply, DirectorAct, DirectorReply, ShowReply, StartKind, ViewEdit};
use super::routes::path;
use crate::client::{self, Response};
use crate::command::CommandError;
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER;

pub const CAMERA_USAGE: &str =
    "camera focus <element> [reason=<reason>] [zoom=N] [over=MS] [hold=MS] | camera wide [over=MS]";
pub const DIRECTOR_USAGE: &str = "director show | pause | resume | auto camera on|off | advance <ms> | end <reason> <element> | start scene|combat | view ...";
pub const VIEW_USAGE: &str = "director view hide|unhide|hero <element> | card | plate [width=PX] [stage=PX] | collapsed on|off | reduced on|off | beat result-card|reveal-beat <ms> [element=<id>] | clear";

/// The top-level verbs this module parses, for the dispatch in `command::parse`.
pub const VERBS: [&str; 2] = ["camera", "director"];

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
        "\n  {CAMERA_USAGE}\n      the Director moves the shared screen's camera: focus is a direction with a reason\n      (one of: {reasons}; speaker when none is given), so the rules of the plan apply: a\n      lower reason never interrupts a higher one, one new move in 800 ms, never during a result\n      card, a hidden element is refused, the zoom is held to what the plate can bear, and it\n      returns wide when its hold ends. The lines it writes say driven=true\n\
         \n  {DIRECTOR_USAGE}\n      show lists the directions queued, applied and dropped, with reasons, and what the\n      Director sees; pause holds every automatic direction while you drive by hand; auto turns\n      the camera class on or off; advance moves the Director's own clock so a hold runs out\n      without waiting; end says the reason of that element's line or turn ended; start says a scene or combat began\n\
         \n  {VIEW_USAGE}\n      what the Director is shown, until the engine reports it: hide an element from the table\n      (it cannot be focused, and a camera on it pulls back at once), mark a hero (always\n      focusable), show a card instead of a plate (no focus is possible), the plate's pixel width\n      and the stage's device pixels (the zoom ceiling), a collapsed beat, reduced effects, or a\n      result card or reveal beat that holds the camera still\n",
        reasons = Reason::keys(),
    )
}

fn usage(text: &'static str) -> CommandError {
    CommandError::Usage(text)
}

fn bad(key: &str, text: &str, allowed: &str) -> CommandError {
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
            return Err(bad("this command", key, allowed));
        }
        if found.iter().any(|(seen, _)| *seen == key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        found.push((key, value));
    }
    Ok(found)
}

fn whole<T: std::str::FromStr>(key: &str, text: &str, what: &str) -> Result<T, CommandError> {
    text.parse::<T>().map_err(|_| bad(key, text, what))
}

fn switch(word: &str, text: &str) -> Result<bool, CommandError> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(bad(word, text, "on or off")),
    }
}

fn focus(words: &[String]) -> Result<DirectorCommand, CommandError> {
    let [element, rest @ ..] = words else {
        return Err(usage(CAMERA_USAGE));
    };
    let allowed = "reason=, zoom=N, over=MS, hold=MS";
    let (mut reason, mut zoom, mut over, mut hold) = (None, None, None, None);
    for (key, value) in pairs(rest, &["reason", "zoom", "over", "hold"], allowed)? {
        match key {
            "reason" => {
                Reason::from_key(value).ok_or_else(|| bad(key, value, &Reason::keys()))?;
                reason = Some(value.to_owned());
            }
            "zoom" => {
                zoom = Some(whole(
                    key,
                    value,
                    "a whole number of percent (200 is twice as large)",
                )?)
            }
            "over" => over = Some(whole(key, value, "a whole number of milliseconds")?),
            _ => hold = Some(whole(key, value, "a whole number of milliseconds")?),
        }
    }
    Ok(DirectorCommand::Act(DirectorAct::Focus {
        element: element.clone(),
        reason,
        zoom,
        over,
        hold,
    }))
}

fn camera(words: &[String]) -> Result<DirectorCommand, CommandError> {
    match words.split_first() {
        Some((verb, rest)) if verb == "focus" => focus(rest),
        Some((verb, rest)) if verb == "wide" => {
            let mut over = None;
            for (key, value) in pairs(rest, &["over"], "over=MS")? {
                over = Some(whole(key, value, "a whole number of milliseconds")?);
            }
            Ok(DirectorCommand::Act(DirectorAct::Wide { over }))
        }
        _ => Err(usage(CAMERA_USAGE)),
    }
}

fn view(words: &[String]) -> Result<DirectorCommand, CommandError> {
    let edit = match words {
        [verb, element] if verb == "hide" => ViewEdit::Hide {
            element: element.clone(),
        },
        [verb, element] if verb == "unhide" => ViewEdit::Unhide {
            element: element.clone(),
        },
        [verb, element] if verb == "hero" => ViewEdit::Hero {
            element: element.clone(),
        },
        [verb] if verb == "card" => ViewEdit::Card,
        [verb] if verb == "clear" => ViewEdit::Clear,
        [verb, rest @ ..] if verb == "plate" => {
            let (mut width_px, mut stage_px) = (None, None);
            for (key, value) in pairs(rest, &["width", "stage"], "width=PX, stage=PX")? {
                let pixels = Some(whole(key, value, "a whole number of pixels")?);
                if key == "width" {
                    width_px = pixels;
                } else {
                    stage_px = pixels;
                }
            }
            ViewEdit::Plate { width_px, stage_px }
        }
        [verb, on] if verb == "collapsed" => ViewEdit::Collapsed {
            on: switch("collapsed", on)?,
        },
        [verb, on] if verb == "reduced" => ViewEdit::Reduced {
            on: switch("reduced", on)?,
        },
        [verb, kind, ms, rest @ ..] if verb == "beat" => {
            let reveal = match kind.as_str() {
                "result-card" => false,
                "reveal-beat" => true,
                other => return Err(bad("beat", other, "result-card or reveal-beat")),
            };
            let mut element = None;
            for (_, value) in pairs(rest, &["element"], "element=<id>")? {
                element = Some(value.to_owned());
            }
            ViewEdit::Beat {
                reveal,
                ms: whole("beat", ms, "a whole number of milliseconds")?,
                element,
            }
        }
        _ => return Err(usage(VIEW_USAGE)),
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
            Err(bad(
                "auto",
                class,
                "camera (the slide and cue classes arrive with their stages)",
            ))
        }
        [verb, ms] if verb == "advance" => act(DirectorAct::Advance {
            ms: whole("advance", ms, "a whole number of milliseconds")?,
        }),
        [verb, reason, element] if verb == "end" => {
            Reason::from_key(reason).ok_or_else(|| bad("end", reason, &Reason::keys()))?;
            act(DirectorAct::End {
                reason: reason.clone(),
                element: element.clone(),
            })
        }
        [verb, what] if verb == "start" => act(DirectorAct::Start {
            what: match what.as_str() {
                "scene" => StartKind::Scene,
                "combat" => StartKind::Combat,
                other => return Err(bad("start", other, "scene or combat")),
            },
        }),
        [verb, rest @ ..] if verb == "view" => view(rest),
        _ => Err(usage(DIRECTOR_USAGE)),
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

fn unreadable(response: &Response, reason: String) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: response.body.chars().take(MAXIMUM_ECHOED_ANSWER).collect(),
        reason: Some(reason),
    }
}

/// Reads an answer as the Director's reply: the value, or the refusal as a `DevError::Rejected`.
fn read<T: DeserializeOwned>(response: &Response) -> Result<T, DevError> {
    let reply: DirectorReply<T> = serde_json::from_str(&response.body)
        .map_err(|error| unreadable(response, error.to_string()))?;
    match reply {
        DirectorReply::Done { value } => Ok(value),
        DirectorReply::Rejected { message, .. } => Err(DevError::Rejected(message)),
    }
}

async fn send<T: DeserializeOwned>(
    port: u16,
    address: &str,
    body: &impl Serialize,
) -> Result<T, DevError> {
    let text = serde_json::to_string(body).map_err(|error| DevError::Unwritable {
        what: "request",
        reason: error.to_string(),
    })?;
    let response = client::post_json(port, address, &text)
        .await
        .map_err(DevError::Client)?;
    read(&response)
}

/// Runs one Director command against the host on `port` and returns the text to print.
pub async fn execute(port: u16, command: DirectorCommand) -> Result<String, DevError> {
    match command {
        DirectorCommand::Act(act) => {
            let reply: ActReply = send(port, path::ACT, &act).await?;
            Ok(format_act(&reply))
        }
        DirectorCommand::Show => {
            let response = client::get(port, path::SHOW)
                .await
                .map_err(DevError::Client)?;
            let show: ShowReply = read(&response)?;
            Ok(format_show(&show))
        }
    }
}
