//! The dev command line of the scene builder: `scene build`, `scene show`, `plates` and
//! `buffer show` (coding style section 8, pattern 8).
//!
//! It owns the grammar and the usage text of these verbs, and running one against a host on a port.
//! Each verb has the shape of the other dev commands: words in, one typed `SceneBuilderCommand`
//! out, a usage line on a mistake (`CommandError::Usage`), and a numbered answer. They work from
//! the repl and from a script like any other command, because `command::parse` calls `parse` for
//! `plates` and `buffer` and `parse_scene` for the `scene` verbs it does not know.

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::format::{format_buffer, format_build, format_plates, format_run, format_show};
use super::request::{
    BufferShow, BuildReply, BuildRequest, DevMoment, DevStage, PlatesReport, RunReply, RunRequest,
    SceneReply, SceneShow, SlowStage,
};
use super::routes::path;
use crate::client::{self, Response};
use crate::command::CommandError;
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER;

pub const BUILD_USAGE: &str = "scene build <place> [fail=<stage>] [slow=<stage>:<seconds>]  (stage: describe, background, plate, locate)";
pub const SHOW_USAGE: &str = "scene show";
pub const PLATES_USAGE: &str = "plates | plates run <seconds> [quiet] [moment=line_boundary|mid_line|dice_or_result_card|reveal_beat]";
pub const BUFFER_USAGE: &str = "buffer show";

/// The top-level verbs this module parses, for the dispatch in `command::parse`. The `scene` verbs
/// `build` and `show` join the playground's `scene` verb, which `command::scene` hands over here.
pub const VERBS: [&str; 2] = ["plates", "buffer"];

/// A command for the scene builder's workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBuilderCommand {
    Build(BuildRequest),
    Show,
    Plates,
    PlatesRun(RunRequest),
    BufferShow,
}

/// The usage line of every verb, in the order `help` lists them.
pub fn verb_usages() -> [&'static str; 4] {
    [BUILD_USAGE, SHOW_USAGE, PLATES_USAGE, BUFFER_USAGE]
}

/// The help text of these verbs, to follow the other commands' in `help`.
pub fn help_text() -> String {
    format!(
        "\n  {BUILD_USAGE}\n      the dev party enters a new scene and it is built through the broker over fake providers\n      in a clock of its own, until its first plate is on screen; fail= makes a stage's provider refuse,\n      slow= makes a stage take that many seconds (past the budget, the establishing card holds the screen)\n\
         \n  {SHOW_USAGE}\n      what each open scene has stored, its jobs, the establishing card and where Locate found\n      each person and item (or the explicit fallback point of one it did not)\n\
         \n  {PLATES_USAGE}\n      the plate clock, its queue, the jobs in flight and the cost of the interval; run lets the clock\n      tick (a creature moves each beat, or nothing changes with quiet, and no plate is paid for)\n\
         \n  {BUFFER_USAGE}\n      the playout buffer: the plate on screen, what is ready and pending, and the lateness\n"
    )
}

fn usage(text: &'static str) -> CommandError {
    CommandError::Usage(text)
}

fn whole_number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

fn stage(key: &str, text: &str) -> Result<DevStage, CommandError> {
    DevStage::parse(text).ok_or_else(|| CommandError::BadChoice {
        key: key.to_owned(),
        text: text.to_owned(),
        allowed: "describe, background, plate or locate".to_owned(),
    })
}

fn build(words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    let [place, options @ ..] = words else {
        return Err(usage(BUILD_USAGE));
    };
    let mut request = BuildRequest {
        place: place.clone(),
        fail: None,
        slow: None,
    };
    for option in options {
        let (key, value) = option
            .split_once('=')
            .ok_or_else(|| CommandError::BadPair(option.clone()))?;
        match key {
            "fail" => {
                if request.fail.replace(stage("fail", value)?).is_some() {
                    return Err(CommandError::DuplicateKey("fail".to_owned()));
                }
            }
            "slow" => {
                let (name, duration) = value
                    .split_once(':')
                    .ok_or_else(|| CommandError::BadPair(option.clone()))?;
                let slow = SlowStage {
                    stage: stage("slow", name)?,
                    seconds: whole_number("the seconds", duration)?,
                };
                if request.slow.replace(slow).is_some() {
                    return Err(CommandError::DuplicateKey("slow".to_owned()));
                }
            }
            _ => return Err(usage(BUILD_USAGE)),
        }
    }
    Ok(SceneBuilderCommand::Build(request))
}

fn run(words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    let [seconds, options @ ..] = words else {
        return Err(usage(PLATES_USAGE));
    };
    let mut request = RunRequest {
        seconds: whole_number("the seconds", seconds)?,
        quiet: false,
        moment: DevMoment::default(),
    };
    for option in options {
        if option == "quiet" {
            request.quiet = true;
            continue;
        }
        let Some(("moment", value)) = option.split_once('=') else {
            return Err(usage(PLATES_USAGE));
        };
        request.moment = DevMoment::parse(value).ok_or_else(|| CommandError::BadChoice {
            key: "moment".to_owned(),
            text: value.to_owned(),
            allowed: "line_boundary, mid_line, dice_or_result_card or reveal_beat".to_owned(),
        })?;
    }
    Ok(SceneBuilderCommand::PlatesRun(request))
}

/// Parses the `scene` verbs `build` and `show`; `verb` is one of them.
pub fn parse_scene(verb: &str, words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    match verb {
        "build" => build(words),
        "show" if words.is_empty() => Ok(SceneBuilderCommand::Show),
        "show" => Err(usage(SHOW_USAGE)),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Parses one top-level verb of this module. `verb` is one of `VERBS`.
pub fn parse(verb: &str, words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    match (verb, words) {
        ("plates", []) => Ok(SceneBuilderCommand::Plates),
        ("plates", [run_word, rest @ ..]) if run_word == "run" => run(rest),
        ("plates", _) => Err(usage(PLATES_USAGE)),
        ("buffer", [show]) if show == "show" => Ok(SceneBuilderCommand::BufferShow),
        ("buffer", _) => Err(usage(BUFFER_USAGE)),
        (other, _) => Err(CommandError::Unknown(other.to_owned())),
    }
}

fn unreadable(response: &Response, reason: String) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: response.body.chars().take(MAXIMUM_ECHOED_ANSWER).collect(),
        reason: Some(reason),
    }
}

/// Reads an answer as the workbench's reply: the value, or the refusal as a `DevError::Rejected`.
fn read<T: DeserializeOwned>(response: &Response) -> Result<T, DevError> {
    let reply: SceneReply<T> = serde_json::from_str(&response.body)
        .map_err(|error| unreadable(response, error.to_string()))?;
    match reply {
        SceneReply::Done { value } => Ok(value),
        SceneReply::Rejected { message, .. } => Err(DevError::Rejected(message)),
    }
}

async fn fetch<T: DeserializeOwned>(port: u16, address: &str) -> Result<T, DevError> {
    let response = client::get(port, address).await.map_err(DevError::Client)?;
    read(&response)
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

/// Runs one scene builder command against the host on `port` and returns the text to print.
pub async fn execute(port: u16, command: SceneBuilderCommand) -> Result<String, DevError> {
    match command {
        SceneBuilderCommand::Build(request) => {
            let reply: BuildReply = send(port, path::BUILD, &request).await?;
            Ok(format_build(&reply))
        }
        SceneBuilderCommand::Show => {
            let show: SceneShow = fetch(port, path::SHOW).await?;
            Ok(format_show(&show))
        }
        SceneBuilderCommand::Plates => {
            let report: PlatesReport = fetch(port, path::PLATES).await?;
            Ok(format_plates(&report))
        }
        SceneBuilderCommand::PlatesRun(request) => {
            let reply: RunReply = send(port, path::PLATES, &request).await?;
            Ok(format_run(&reply))
        }
        SceneBuilderCommand::BufferShow => {
            let show: BufferShow = fetch(port, path::BUFFER).await?;
            Ok(format_buffer(&show))
        }
    }
}
