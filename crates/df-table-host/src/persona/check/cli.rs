//! The `persona check` verbs of the dev command line: their grammar, the request each sends, and
//! the text of each answer.
//!
//! `persona check judge` prints the judge's typed answer for a character's action (or the
//! narrator's moment); `persona check run` does the whole flow on the TV: the call said, the roll
//! shown, the result read. The sheet and the station are read from their data files here, so the
//! host takes them as typed values. The text of an answer is the contract the gauntlet runner
//! reads: a header line, then indented `call:`, `designed:`, `called`, `result`, `roll:` and
//! `times:` lines.

use std::path::PathBuf;

use df_rules::roll::{Ability, Skill};

use super::request::{
    CheckJudgeRequest, CheckJudged, CheckKind, CheckRouteError, CheckRun, CheckRunRequest,
    CheckTest, Initiator, mode_key,
};
use super::routes::path;
use super::sheet::{read_sheet, read_station};
use crate::command::{CommandError, bad_choice};
use crate::dev::{DevError, read_text_file};
use crate::dev_client::{post_json, reply_value};
use crate::dev_http::DevReply;
use crate::persona::cli::{flag, id_word};
use crate::persona::options::take_options;

/// `persona check judge` and `persona check run`.
pub const PERSONA_CHECK_USAGE: &str = concat!(
    "persona check judge <character> [sheet=<file>] [stations=<file>] [station=<id>] ",
    "[action=<line>] [moment=player|narrator] [place=<words>] [sight=<words>]... ",
    "[checked=<kind>[:<ability>[:<skill>]]]... [intent=<kind>[:<ability>[:<skill>]]] ",
    "[model=<name>] | ",
    "persona check run <character> [the same options] [inspiration=<die place>] ",
    "[told=<what was just said aloud>] ",
    "[speak=true|false]"
);

/// Where the sheet of a character is looked for when no `sheet=` is given: its id and this
/// suffix, in the data folder of the market threads.
const SHEET_FOLDER: &str = "development/market-threads";
const SHEET_SUFFIX: &str = ".sheet.json";
const STATIONS_FILE: &str = "development/market-threads/gauntlet.stations.json";

/// What the two check verbs share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckArgs {
    pub(super) character: String,
    pub(super) sheet: PathBuf,
    pub(super) stations: PathBuf,
    pub(super) station: Option<String>,
    pub(super) action: Option<String>,
    pub(super) moment: Initiator,
    pub(super) place: Option<String>,
    pub(super) sight: Vec<String>,
    pub(super) checked: Vec<CheckTest>,
    pub(super) intent: Option<CheckTest>,
    pub(super) model: Option<String>,
}

/// A `persona check` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckCommand {
    Judge(CheckArgs),
    Run {
        args: CheckArgs,
        speak: bool,
        inspiration: Option<u8>,
        told: Option<String>,
    },
}

const SHARED_KEYS: [&str; 10] = [
    "sheet", "stations", "station", "action", "moment", "place", "sight", "checked", "model",
    "intent",
];

fn single<'a>(given: &[(&'a str, &'a str)], key: &str) -> Result<Option<&'a str>, CommandError> {
    let mut values = given.iter().filter(|(seen, _)| *seen == key);
    let first = values.next().map(|(_, value)| *value);
    if values.next().is_some() {
        return Err(CommandError::DuplicateKey(key.to_owned()));
    }
    Ok(first)
}

/// `<kind>[:<ability>[:<skill>]]`.
fn test_of(key: &'static str, text: &str) -> Result<CheckTest, CommandError> {
    const ALLOWED: &str = "<kind>[:<ability>[:<skill>]] with keys of the roll vocabulary";
    let mut parts = text.split(':');
    let kind = parts
        .next()
        .and_then(CheckKind::from_key)
        .ok_or_else(|| bad_choice(key, text, ALLOWED))?;
    let ability = parts
        .next()
        .map(|key| Ability::from_key(key).ok_or_else(|| bad_choice(key, text, ALLOWED)))
        .transpose()?;
    let skill = parts
        .next()
        .map(|key| Skill::from_key(key).ok_or_else(|| bad_choice(key, text, ALLOWED)))
        .transpose()?;
    if parts.next().is_some() {
        return Err(bad_choice(key, text, ALLOWED));
    }
    Ok(CheckTest {
        kind,
        ability,
        skill,
    })
}

/// `persona check ...`.
pub(in crate::persona) fn parse_check(words: &[String]) -> Result<CheckCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_CHECK_USAGE);
    let [verb, character, rest @ ..] = words else {
        return Err(usage());
    };
    let run = match verb.as_str() {
        "judge" => false,
        "run" => true,
        _ => return Err(usage()),
    };
    let keys: Vec<&str> = if run {
        SHARED_KEYS
            .into_iter()
            .chain(["inspiration", "speak", "told"])
            .collect()
    } else {
        SHARED_KEYS.to_vec()
    };
    let (given, left) = take_options(rest, &keys);
    if !left.is_empty() {
        return Err(usage());
    }
    let character = id_word(character)?.to_string();
    let action = single(&given, "action")?.map(str::to_owned);
    let moment = match single(&given, "moment")? {
        Some(text) => Initiator::from_key(text)
            .ok_or_else(|| bad_choice("moment", text, "player or narrator"))?,
        None if action.is_some() => Initiator::Player,
        None => Initiator::Narrator,
    };
    let args = CheckArgs {
        sheet: single(&given, "sheet")?.map_or_else(
            || PathBuf::from(format!("{SHEET_FOLDER}/{character}{SHEET_SUFFIX}")),
            PathBuf::from,
        ),
        stations: single(&given, "stations")?
            .map_or_else(|| PathBuf::from(STATIONS_FILE), PathBuf::from),
        station: single(&given, "station")?.map(str::to_owned),
        action,
        moment,
        place: single(&given, "place")?.map(str::to_owned),
        sight: given
            .iter()
            .filter(|(key, _)| *key == "sight")
            .map(|(_, value)| (*value).to_owned())
            .collect(),
        checked: given
            .iter()
            .filter(|(key, _)| *key == "checked")
            .map(|(_, value)| test_of("checked", value))
            .collect::<Result<_, _>>()?,
        intent: single(&given, "intent")?
            .map(|text| test_of("intent", text))
            .transpose()?,
        model: single(&given, "model")?.map(str::to_owned),
        character,
    };
    if !run {
        return Ok(CheckCommand::Judge(args));
    }
    Ok(CheckCommand::Run {
        told: single(&given, "told")?.map(str::to_owned),
        args,
        speak: single(&given, "speak")?.map_or(Ok(true), |text| flag("speak", text))?,
        inspiration: single(&given, "inspiration")?
            .map(|text| {
                text.parse::<u8>().map_err(|_| CommandError::BadNumber {
                    what: "inspiration",
                    text: text.to_owned(),
                })
            })
            .transpose()?,
    })
}

async fn judge_request(args: &CheckArgs) -> Result<CheckJudgeRequest, DevError> {
    let unreadable = |path: &PathBuf, message: String| DevError::UnreadableFile {
        path: path.clone(),
        message,
    };
    let sheet_text = read_text_file(&args.sheet).await?;
    let sheet = read_sheet(&sheet_text).map_err(|message| unreadable(&args.sheet, message))?;
    let station = match &args.station {
        Some(id) => {
            let text = read_text_file(&args.stations).await?;
            Some(read_station(&text, id).map_err(|message| unreadable(&args.stations, message))?)
        }
        None => None,
    };
    Ok(CheckJudgeRequest {
        character: args.character.clone(),
        sheet,
        moment: args.moment,
        action: args.action.clone(),
        station,
        place: args.place.clone(),
        in_sight: args.sight.clone(),
        checked: args.checked.clone(),
        intent: args.intent.clone(),
        model: args.model.clone(),
    })
}

fn or_dash(text: Option<&str>) -> &str {
    text.unwrap_or("-")
}

/// The judge's answer as the lines the runner reads.
pub(in crate::persona) fn format_judged(character: &str, judged: &CheckJudged) -> String {
    let verdict = &judged.verdict;
    let mut lines = vec![format!(
        "{character} check: required={} initiator={} reason={} retried={} duration_ms={}",
        verdict.call.is_some(),
        verdict.initiator.key(),
        verdict.reason.key(),
        judged.retried,
        judged.duration_ms
    )];
    if let Some(call) = &verdict.call {
        lines.push(format!(
            "  call: kind={} ability={} skill={} dc={} mode={} grounds={} uses={}",
            call.kind.key(),
            or_dash(call.ability.map(Ability::key)),
            or_dash(call.skill.map(Skill::key)),
            call.dc_band
                .map_or_else(|| "-".to_owned(), |band| band.value().to_string()),
            mode_key(call.mode),
            call.grounds.key(),
            or_dash(call.uses.as_deref())
        ));
    }
    if let Some(designed) = &judged.designed {
        lines.push(format!(
            "  designed: kind={} match={}",
            designed.kind.key(),
            designed.matches
        ));
    }
    lines.join("\n")
}

/// A run's answer as the lines the runner reads.
pub(in crate::persona) fn format_run(character: &str, run: &CheckRun) -> String {
    let mut lines = vec![format_judged(character, &run.judged)];
    let Some(rolled) = &run.rolled else {
        lines.push("  no roll was called".to_owned());
        return lines.join("\n");
    };
    let view = &rolled.view;
    lines.push(format!(
        "  called ({}): {}",
        rolled.called_voice.key(),
        rolled.called_line
    ));
    lines.push(format!(
        "  result ({}): {}",
        rolled.result_voice.key(),
        rolled.result_line
    ));
    let faces: Vec<String> = view
        .dice
        .iter()
        .map(|die| {
            format!(
                "d{}={}{}",
                die.sides,
                die.face,
                if die.kept { "" } else { "x" }
            )
        })
        .collect();
    lines.push(format!(
        "  roll: {} kind={} total={} target={} outcome={} natural={} dice={}",
        view.title,
        view.kind.key(),
        view.total,
        view.target.as_ref().map_or_else(
            || "-".to_owned(),
            |target| format!("{} {}", target.label, target.value)
        ),
        view.outcome.map_or("-", |outcome| outcome.key()),
        view.natural.key(),
        faces.join(",")
    ));
    let times = &rolled.times;
    lines.push(format!(
        concat!(
            "  times: drawn_ms={} called_ms={} shown_ms={} result_start_ms={} result_end_ms={} ",
            "cleared_ms={} hold_ms={} min_hold_ms={}"
        ),
        times.drawn_ms,
        times.called_ms,
        times.shown_ms,
        times.result_start_ms,
        times.result_end_ms,
        times.cleared_ms,
        times.hold_ms,
        times.min_hold_ms
    ));
    lines.join("\n")
}

/// Runs a `persona check` command against the host on `port`.
pub(in crate::persona) async fn run(port: u16, command: CheckCommand) -> Result<String, DevError> {
    match command {
        CheckCommand::Judge(args) => {
            let request = judge_request(&args).await?;
            let reply: DevReply<CheckJudged, CheckRouteError> =
                post_json(port, path::PERSONA_CHECK_JUDGE, &request).await?;
            Ok(format_judged(&args.character, &reply_value(reply)?))
        }
        CheckCommand::Run {
            args,
            speak,
            inspiration,
            told,
        } => {
            let request = CheckRunRequest {
                judge: judge_request(&args).await?,
                speak,
                inspiration,
                told,
            };
            let reply: DevReply<CheckRun, CheckRouteError> =
                post_json(port, path::PERSONA_CHECK_RUN, &request).await?;
            Ok(format_run(&args.character, &reply_value(reply)?))
        }
    }
}
