//! The dev command line of the scene builder: `scene build`, `scene show`, `plates` and
//! `buffer show` (coding style section 8, pattern 8).
//!
//! It owns the grammar and the usage text of these verbs, and running one against a host on a port.
//! Each verb has the shape of the other dev commands: words in, one typed `SceneBuilderCommand`
//! out, a usage line on a mistake (`CommandError::Usage`), and a numbered answer. They work from
//! the repl and from a script like any other command, because `command::parse` calls `parse` for
//! `plates` and `buffer` and `parse_scene` for the `scene` verbs it does not know.

use df_table::table_state::DisplayName;

use super::format::{format_buffer, format_build, format_plates, format_run, format_show};
use super::lookahead::{self, LookaheadCommand};
use super::request::{
    BufferShow, BuildReply, BuildRequest, DevMoment, DevStage, PlatesReport, RunReply, RunRequest,
    SceneReply, SceneShow, SlowStage,
};
use super::routes::path;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;

pub const BUILD_USAGE: &str = concat!(
    "scene build <place> [fail=<stage>] [slow=<stage>:<seconds>]",
    "  (stage: describe, background, plate, locate)",
);
pub const SHOW_USAGE: &str = "scene show";
pub const PLATES_USAGE: &str = concat!(
    "plates | plates run <seconds> [quiet]",
    " [moment=line_boundary|mid_line|dice_or_result_card|reveal_beat]",
);
pub const BUFFER_USAGE: &str = "buffer show";

/// The top-level verbs this module parses, for the dispatch in `command::parse`. The `scene` verbs
/// `build` and `show` join the playground's `scene` verb, which `command::scene` hands over here.
pub const VERBS: [&str; 2] = ["plates", "buffer"];

/// A command for the scene builder's workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBuilderCommand {
    Build(BuildRequest),
    Lookahead(LookaheadCommand),
    Show,
    Plates,
    PlatesRun(RunRequest),
    BufferShow,
}

/// The usage line of every verb, in the order `help` lists them.
pub fn verb_usages() -> [&'static str; 5] {
    [
        BUILD_USAGE,
        SHOW_USAGE,
        PLATES_USAGE,
        BUFFER_USAGE,
        lookahead::cli::LOOKAHEAD_USAGE,
    ]
}

/// The help text of these verbs, to follow the other commands' in `help`.
pub fn help_text() -> String {
    format!(
        concat!(
            "\n  {build}\n",
            "      the dev party enters a new scene and it is built through the broker over\n",
            "      fake providers in a clock of its own, until its first plate is on screen;\n",
            "      fail= makes a stage's provider refuse, slow= makes a stage take that many\n",
            "      seconds (past the budget, the establishing card holds the screen)\n",
            "\n  {show}\n",
            "      what each open scene has stored, its jobs, the establishing card and where\n",
            "      Locate found each person and item (or the explicit fallback point of one\n",
            "      it did not)\n",
            "\n  {plates}\n",
            "      the plate clock, its queue, the jobs in flight and the cost of the\n",
            "      interval; run lets the clock tick (a creature moves each beat, or nothing\n",
            "      changes with quiet, and no plate is paid for)\n",
            "\n  {buffer}\n",
            "      the playout buffer: the plate on screen, what is ready and pending, and\n",
            "      the lateness\n",
            "{lookahead}",
        ),
        build = BUILD_USAGE,
        show = SHOW_USAGE,
        plates = PLATES_USAGE,
        buffer = BUFFER_USAGE,
        lookahead = lookahead::cli::help_text(),
    )
}

/// A whole number of `what` (seconds, a value, thousandths) from 0.
pub(super) fn whole_number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

/// The stage named `text` for `key`, one of the stages `allowed` lists, or the refusal naming them.
pub(super) fn dev_stage(
    key: &str,
    text: &str,
    is_allowed: impl Fn(DevStage) -> bool,
    allowed: &str,
) -> Result<DevStage, CommandError> {
    DevStage::from_key(text)
        .filter(|stage| is_allowed(*stage))
        .ok_or_else(|| CommandError::BadChoice {
            key: key.to_owned(),
            text: text.to_owned(),
            allowed: allowed.to_owned(),
        })
}

/// `slow=<stage>:<seconds>` of `option`, its stage one of the stages `allowed` lists.
pub(super) fn slow_stage(
    option: &str,
    value: &str,
    is_allowed: impl Fn(DevStage) -> bool,
    allowed: &str,
) -> Result<SlowStage, CommandError> {
    let (name, duration) = value
        .split_once(':')
        .ok_or_else(|| CommandError::BadPair(option.to_owned()))?;
    Ok(SlowStage {
        stage: dev_stage("slow", name, is_allowed, allowed)?,
        seconds: whole_number("the seconds", duration)?,
    })
}

/// The place of a scene: a display name.
pub(super) fn place(text: &str) -> Result<DisplayName, CommandError> {
    DisplayName::new(text).map_err(|error| CommandError::BadChoice {
        key: "the place".to_owned(),
        text: text.to_owned(),
        allowed: format!("a place name ({error})"),
    })
}

const EVERY_STAGE: &str = "describe, background, plate or locate";

fn build(words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    let [name, options @ ..] = words else {
        return Err(CommandError::Usage(BUILD_USAGE));
    };
    let mut request = BuildRequest {
        place: place(name)?,
        fail: None,
        slow: None,
    };
    let any_stage = |_| true;
    for option in options {
        let (key, value) = option
            .split_once('=')
            .ok_or_else(|| CommandError::BadPair(option.clone()))?;
        match key {
            "fail" => {
                let stage = dev_stage("fail", value, any_stage, EVERY_STAGE)?;
                if request.fail.replace(stage).is_some() {
                    return Err(CommandError::DuplicateKey("fail".to_owned()));
                }
            }
            "slow" => {
                let slow = slow_stage(option, value, any_stage, EVERY_STAGE)?;
                if request.slow.replace(slow).is_some() {
                    return Err(CommandError::DuplicateKey("slow".to_owned()));
                }
            }
            _ => return Err(CommandError::Usage(BUILD_USAGE)),
        }
    }
    Ok(SceneBuilderCommand::Build(request))
}

fn plates_run(words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    let [seconds, options @ ..] = words else {
        return Err(CommandError::Usage(PLATES_USAGE));
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
            return Err(CommandError::Usage(PLATES_USAGE));
        };
        request.moment = DevMoment::from_key(value).ok_or_else(|| CommandError::BadChoice {
            key: "moment".to_owned(),
            text: value.to_owned(),
            allowed: "line_boundary, mid_line, dice_or_result_card or reveal_beat".to_owned(),
        })?;
    }
    Ok(SceneBuilderCommand::PlatesRun(request))
}

/// Parses the `scene` verbs `build`, `show` and `lookahead`; `verb` is one of them.
pub fn parse_scene(verb: &str, words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    match verb {
        "build" => build(words),
        "show" if words.is_empty() => Ok(SceneBuilderCommand::Show),
        "show" => Err(CommandError::Usage(SHOW_USAGE)),
        "lookahead" => lookahead::parse(words).map(SceneBuilderCommand::Lookahead),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Parses one top-level verb of this module. `verb` is one of `VERBS`.
pub fn parse(verb: &str, words: &[String]) -> Result<SceneBuilderCommand, CommandError> {
    match (verb, words) {
        ("plates", []) => Ok(SceneBuilderCommand::Plates),
        ("plates", [run_word, rest @ ..]) if run_word == "run" => plates_run(rest),
        ("plates", _) => Err(CommandError::Usage(PLATES_USAGE)),
        ("buffer", [show]) if show == "show" => Ok(SceneBuilderCommand::BufferShow),
        ("buffer", _) => Err(CommandError::Usage(BUFFER_USAGE)),
        (other, _) => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// The value of a scene builder reply, or its refusal as the command's failure.
///
/// Transitional: the refusal becomes text until `DevError` carries a route's typed error
/// (`dev_client::get_value` / `post_value` then replace this); the route tests match the typed
/// `SceneBuilderError` on the reply itself.
pub(super) fn reply_value<T>(reply: SceneReply<T>) -> Result<T, DevError> {
    match reply {
        DevReply::Done { value } => Ok(value),
        DevReply::Rejected { error, message } => Err(DevError::Rejected(format!(
            "refused ({}): {message}",
            error.key()
        ))),
    }
}

/// Runs one scene builder command against the host on `port` and returns the text to print.
pub async fn run(port: u16, command: SceneBuilderCommand) -> Result<String, DevError> {
    match command {
        SceneBuilderCommand::Build(request) => {
            let reply: BuildReply = reply_value(post_json(port, path::BUILD, &request).await?)?;
            Ok(format_build(&reply))
        }
        SceneBuilderCommand::Lookahead(command) => lookahead::run(port, command).await,
        SceneBuilderCommand::Show => {
            let show: SceneShow = reply_value(get_json(port, path::SHOW).await?)?;
            Ok(format_show(&show))
        }
        SceneBuilderCommand::Plates => {
            let report: PlatesReport = reply_value(get_json(port, path::PLATES).await?)?;
            Ok(format_plates(&report))
        }
        SceneBuilderCommand::PlatesRun(request) => {
            let reply: RunReply = reply_value(post_json(port, path::PLATES, &request).await?)?;
            Ok(format_run(&reply))
        }
        SceneBuilderCommand::BufferShow => {
            let show: BufferShow = reply_value(get_json(port, path::BUFFER).await?)?;
            Ok(format_buffer(&show))
        }
    }
}
