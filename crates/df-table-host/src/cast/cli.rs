//! `scene cast`, `scene cue` and `scene camera-on` on the dev command line: their grammar, the
//! request each sends, and the text of each answer.

use df_scene_layout::{CharacterCue, CueKind, Shot, percent_text};

use super::request::{CameraMoved, CameraRequest, CastList, CastReply, Cued};
use super::routes::path;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;

pub const CAST_USAGE: &str = "scene cast";
pub const CUE_USAGE: &str = "scene cue <character> think | speak <line...> | quiet";
pub const CAMERA_ON_USAGE: &str = "scene camera-on <character> [shot=face|upper|full] [over=MS]";

/// A cast command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastCommand {
    /// List the characters of the shot.
    List,
    /// Send a cue about a character.
    Cue(CharacterCue),
    /// Frame a character.
    Camera {
        character: String,
        shot: Option<Shot>,
        over_ms: Option<u64>,
    },
}

/// Reads the words after `scene cue`.
pub fn parse_cue(words: &[String]) -> Result<CastCommand, CommandError> {
    let (character, kind) = match words {
        [character, verb] if verb == "think" => (character, CueKind::Thinking),
        [character, verb] if verb == "quiet" => (character, CueKind::Quiet),
        [character, verb, line @ ..] if verb == "speak" && !line.is_empty() => (
            character,
            CueKind::Speaking {
                line: line.join(" "),
            },
        ),
        _ => return Err(CommandError::Usage(CUE_USAGE)),
    };
    Ok(CastCommand::Cue(CharacterCue {
        character: character.clone(),
        kind,
    }))
}

/// Reads the words after `scene camera-on`.
pub fn parse_camera_on(words: &[String]) -> Result<CastCommand, CommandError> {
    let [character, options @ ..] = words else {
        return Err(CommandError::Usage(CAMERA_ON_USAGE));
    };
    let (mut shot, mut over_ms) = (None, None);
    for option in options {
        match option.split_once('=') {
            Some(("shot", value)) if shot.is_none() => {
                shot = Some(Shot::from_key(value).ok_or(CommandError::Usage(CAMERA_ON_USAGE))?);
            }
            Some(("over", value)) if over_ms.is_none() => {
                over_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| CommandError::Usage(CAMERA_ON_USAGE))?,
                );
            }
            Some((key @ ("shot" | "over"), _)) => {
                return Err(CommandError::DuplicateKey(key.to_owned()));
            }
            _ => return Err(CommandError::Usage(CAMERA_ON_USAGE)),
        }
    }
    Ok(CastCommand::Camera {
        character: character.clone(),
        shot,
        over_ms,
    })
}

/// The value of a cast reply, or its refusal as the command's failure.
///
/// Transitional: the refusal becomes text until `DevError` carries a route's typed error
/// (`dev_client::get_value` / `post_value` then replace this); the route tests match the typed
/// `CastError` on the reply itself.
fn reply_value<T>(reply: CastReply<T>) -> Result<T, DevError> {
    match reply {
        DevReply::Done { value } => Ok(value),
        DevReply::Rejected { error, message } => Err(DevError::Rejected(format!(
            "refused ({}): {message}",
            error.key()
        ))),
    }
}

/// Runs one cast command against the host on `port`.
pub async fn run(port: u16, command: CastCommand) -> Result<String, DevError> {
    match command {
        CastCommand::List => {
            let list: CastList = reply_value(get_json(port, path::CAST).await?)?;
            Ok(format_cast(&list))
        }
        CastCommand::Cue(cue) => {
            let cued: Cued = reply_value(post_json(port, path::CUE, &cue).await?)?;
            Ok(format_cued(&cued))
        }
        CastCommand::Camera {
            character,
            shot,
            over_ms,
        } => {
            let request = CameraRequest {
                character,
                shot,
                over_ms,
            };
            let moved: CameraMoved =
                reply_value(post_json(port, path::CAMERA_ON, &request).await?)?;
            Ok(format_camera(&moved))
        }
    }
}

fn format_cast(list: &CastList) -> String {
    if list.members.is_empty() {
        return format!(
            "no named characters in {} ({})",
            list.picture, "name them with media locate label or identify"
        );
    }
    let point = |(x, y): (u16, u16)| format!("{},{}", percent_text(x), percent_text(y));
    let lines: Vec<String> = list
        .members
        .iter()
        .map(|member| {
            format!(
                "  {} [{} #{}] head {} face {} centre {} %",
                member.name,
                member.region_id,
                member.number,
                point(member.head),
                point(member.face),
                point(member.centre)
            )
        })
        .collect();
    format!("cast of {}:\n{}", list.picture, lines.join("\n"))
}

fn format_cued(cued: &Cued) -> String {
    let doing = match &cued.cue.kind {
        CueKind::Thinking => "thinking".to_owned(),
        CueKind::Speaking { line } => format!("saying: {line}"),
        CueKind::Quiet => "quiet".to_owned(),
    };
    format!(
        "ok (revision {}) {} is {doing}",
        cued.revision, cued.cue.character
    )
}

fn format_camera(moved: &CameraMoved) -> String {
    format!(
        concat!(
            "ok (revision {}) camera on {} at {}% over {} ms ",
            "(the page holds the zoom to what the plate's pixels bear)"
        ),
        moved.revision, moved.target, moved.zoom_percent, moved.duration_ms
    )
}
