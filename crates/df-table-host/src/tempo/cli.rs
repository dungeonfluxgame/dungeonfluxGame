//! The `tempo` verbs of the dev command line: their grammar, the request each sends, and the text
//! of each answer. They are the routes of `tempo::routes`; the runner scripts read the first word
//! of each printed line (`tempo ...`, `music ...`).

use std::path::PathBuf;

use super::request::{
    ImpulseRequest, PrepareReport, PrepareStatus, TableLoaded, TempoReply, TempoView,
};
use super::routes::path;
use super::table::is_id;
use crate::command::{CommandError, bad_choice};
use crate::dev::{DevError, read_text_file};
use crate::dev_client::{self, get_json, post, read_answer};
use crate::music::MusicSource;

/// Every `tempo` verb, for the unknown-verb refusal and the help.
pub const TEMPO_USAGE: &str = concat!(
    "tempo [state] | tempo impulse <kind> [intensity] [note=<text>] | tempo tick | tempo reset | ",
    "tempo load <table.json> | tempo prepare"
);
/// `tempo impulse`: one event of the journey.
pub const TEMPO_IMPULSE_USAGE: &str = "tempo impulse <kind> [intensity 0-1000] [note=<text>]";
/// `tempo load`: the music table file.
pub const TEMPO_LOAD_USAGE: &str = "tempo load <table.json>";

/// A tempo command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempoCommand {
    /// Where the journey's tempo stands and what plays.
    State,
    /// Post one impulse.
    Impulse(ImpulseRequest),
    /// Move the engine to now and let the director decide.
    Tick,
    /// Start the journey again; the director plays the base.
    Reset,
    /// Load a music table from a file.
    Load(PathBuf),
    /// Make or find every profile's track.
    Prepare,
}

/// Reads the words after `tempo`.
pub fn parse(words: &[String]) -> Result<TempoCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(TempoCommand::State);
    };
    match (verb.as_str(), rest) {
        ("state", []) => Ok(TempoCommand::State),
        ("impulse", [kind, rest @ ..]) => impulse(kind, rest).map(TempoCommand::Impulse),
        ("impulse", []) => Err(CommandError::Usage(TEMPO_IMPULSE_USAGE)),
        ("tick", []) => Ok(TempoCommand::Tick),
        ("reset", []) => Ok(TempoCommand::Reset),
        ("load", [path]) => Ok(TempoCommand::Load(PathBuf::from(path))),
        ("load", _) => Err(CommandError::Usage(TEMPO_LOAD_USAGE)),
        ("prepare", []) => Ok(TempoCommand::Prepare),
        _ => Err(CommandError::Usage(TEMPO_USAGE)),
    }
}

fn impulse(kind: &str, rest: &[String]) -> Result<ImpulseRequest, CommandError> {
    if !is_id(kind) {
        return Err(bad_choice(
            "kind",
            kind,
            "1 to 24 letters, digits, '-' or '_'",
        ));
    }
    let mut request = ImpulseRequest {
        kind: kind.to_owned(),
        intensity: None,
        note: None,
    };
    for word in rest {
        if let Some(note) = word.strip_prefix("note=") {
            request.note = Some(note.to_owned());
        } else if request.intensity.is_none() {
            let intensity = word
                .parse::<u16>()
                .map_err(|_| bad_choice("intensity", word, "a whole number from 0 to 1000"))?;
            request.intensity = Some(intensity);
        } else {
            return Err(CommandError::Usage(TEMPO_IMPULSE_USAGE));
        }
    }
    Ok(request)
}

/// The value of a tempo reply, or its refusal (the typed `TempoError`) as the command's failure.
fn reply_value<T>(reply: TempoReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

/// The two lines every moving verb prints: the tempo, then the music.
fn format_view(view: &TempoView) -> String {
    let tempo = format!(
        concat!(
            "tempo phase={} intensity={} inertia={} momentum={} target={} fatigue={} ",
            "at_ms={} policy={}"
        ),
        view.phase.key(),
        view.intensity,
        view.inertia,
        view.momentum.key(),
        view.target,
        view.fatigue,
        view.now_ms,
        view.policy
    );
    let cue = match &view.playing {
        Some(playing) => format!(
            "cue={} track={} {} volume={}",
            playing.profile,
            playing.track,
            source_word(playing.source),
            playing.volume_percent
        ),
        None => "cue=none".to_owned(),
    };
    let decision = view.decision.as_ref().map_or_else(String::new, |decision| {
        let until = decision
            .until_ms
            .map_or_else(String::new, |until| format!(" until_ms={until}"));
        format!(
            " | decision={} reason={} wanted={}{until}",
            decision.action.key(),
            decision.reason.key(),
            decision.wanted
        )
    });
    format!("{tempo}\nmusic {cue}{decision}")
}

/// What the library did for a track, in the words a person reads.
fn source_word(source: MusicSource) -> &'static str {
    match source {
        MusicSource::Library => "reused",
        MusicSource::Composed => "composed",
    }
}

fn format_state(view: &TempoView) -> String {
    let mut lines = vec![format_view(view)];
    lines.push(format!("recent: {}", view.recent.len()));
    lines.extend(view.recent.iter().map(|impulse| {
        let note = impulse
            .note
            .as_ref()
            .map_or_else(String::new, |note| format!("  {note}"));
        format!(
            "  {} x{} at_ms={}{note}",
            impulse.kind, impulse.intensity, impulse.at_ms
        )
    }));
    lines.join("\n")
}

fn format_loaded(loaded: &TableLoaded) -> String {
    format!(
        "ok table version={} base={} profiles={} impulses={}",
        loaded.version,
        loaded.base,
        loaded.profiles.join(","),
        loaded.impulses.join(",")
    )
}

/// The prepare report: a summary line, then one line per profile.
fn format_prepared(report: &PrepareReport) -> String {
    let mut lines = vec![format!(
        "prepared {} profiles: {} reused, {} composed ({} s of music asked), {} failed",
        report.profiles.len(),
        report.reused,
        report.composed,
        report.composed_seconds,
        report.failed
    )];
    lines.extend(report.profiles.iter().map(|prepared| {
        let status = match &prepared.status {
            PrepareStatus::Reused { bytes } => format!("reused ({bytes} bytes)"),
            PrepareStatus::Composed { bytes, compose_ms } => {
                format!("composed ({bytes} bytes, took {compose_ms} ms)")
            }
            PrepareStatus::Failed { error } => format!("failed ({})", error.key()),
        };
        format!(
            "  {}  {}  {} s  {status}",
            prepared.profile, prepared.track, prepared.length_seconds
        )
    }));
    lines.join("\n")
}

/// Runs one tempo command against the host on `port`.
pub async fn run(port: u16, command: TempoCommand) -> Result<String, DevError> {
    match command {
        TempoCommand::State => {
            let view: TempoView = reply_value(get_json(port, path::TEMPO).await?)?;
            Ok(format_state(&view))
        }
        TempoCommand::Impulse(request) => {
            let view = send(port, path::IMPULSE, &request).await?;
            Ok(format_view(&view))
        }
        TempoCommand::Tick => Ok(format_view(&send(port, path::TICK, &()).await?)),
        TempoCommand::Reset => Ok(format_view(&send(port, path::RESET, &()).await?)),
        TempoCommand::Load(file) => {
            let text = read_text_file(&file).await?;
            let response = post(port, path::TABLE, &text).await?;
            let loaded: TableLoaded = reply_value(read_answer(&response)?)?;
            Ok(format_loaded(&loaded))
        }
        TempoCommand::Prepare => {
            let report: PrepareReport =
                reply_value(dev_client::post_json(port, path::PREPARE, &()).await?)?;
            Ok(format_prepared(&report))
        }
    }
}

/// Posts `request` and reads the view the route answers with.
async fn send<Request: serde::Serialize>(
    port: u16,
    route: &str,
    request: &Request,
) -> Result<TempoView, DevError> {
    reply_value(dev_client::post_json(port, route, request).await?)
}
