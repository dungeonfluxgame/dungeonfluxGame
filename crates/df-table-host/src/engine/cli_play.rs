//! The verbs that drive and show the first playable path (O.16 H3): `lobby`, `scene start`, `wire`
//! and `play`.
//!
//! They add no rule and no state of their own. A `lobby` action is the tap a phone's button sends
//! (`input <seat> --tap <action>`), so the seat joins, readies, leaves or starts through the real
//! owner queue and answers with the typed outcome of that turn. `scene start` is the lobby's own
//! start made easy for a developer: it readies every seat at the table that is not ready and then
//! sends `start` as the first seat (or the one named), each a turn of its own with its own answer,
//! and so it declines in the lobby's words and not in this file's when the lobby will not start.
//! `wire view` and `wire send` speak the `MachineView` and `MachineIntent` contract a TV and a
//! phone are served (`machine_io`), and `play publish` writes the real shared-screen and seat
//! projections as a fixture the playground's TV and phone panes already show (the K0 contract),
//! so a person can look at the engine's own state on the pages the host serves.

use serde_json::{Value, json};

use crate::client;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::wire::{FixtureRemove, FixtureWrite, route};

use super::cli::{fetch, send};
use super::format::format_submit;
use super::machine_io::{
    WireAudienceReport, WireIntentBody, WireIntentKind, WireReceiptReport, WireViewReport,
};
use super::report::{EngineDump, SubmitReply};
use super::request::{AudienceSpec, InputBody, InputRequest};
use super::routes::{AUDIENCE_PARAMETER, path};

pub const LOBBY_USAGE: &str =
    "lobby show | join <seat> | leave <seat> | ready <seat> | unready <seat> | start <seat>";
pub const SCENE_START_USAGE: &str = "scene start [<seat>]";
pub const WIRE_USAGE: &str = "wire view <tv|seat-number> | wire send <seat> --tap <offer> | --text <text> | --say <text> | --pause  (any: --op N)";
pub const PLAY_USAGE: &str = "play publish [<seat>] | play remove";

/// The name of the fixture `play publish` writes.
pub const LIVE_FIXTURE: &str = "h3-live";

/// What a `lobby` verb does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyCommand {
    Show,
    /// One of the lobby's own actions, as the seat.
    Act {
        action: &'static str,
        seat: u64,
    },
}

/// A command of the first playable path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayCommand {
    Lobby(LobbyCommand),
    SceneStart { seat: Option<u64> },
    WireView(AudienceSpec),
    WireSend(WireIntentBody),
    Publish { seat: Option<u64> },
    Remove,
}

const LOBBY_ACTIONS: [&str; 5] = ["join", "leave", "ready", "unready", "start"];

fn usage(text: &'static str) -> CommandError {
    CommandError::Usage(text)
}

fn number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

fn parse_lobby(words: &[String]) -> Result<PlayCommand, CommandError> {
    match words {
        [verb] if verb == "show" => Ok(PlayCommand::Lobby(LobbyCommand::Show)),
        [verb, seat] => {
            let action = LOBBY_ACTIONS
                .iter()
                .copied()
                .find(|action| *action == verb)
                .ok_or_else(|| usage(LOBBY_USAGE))?;
            Ok(PlayCommand::Lobby(LobbyCommand::Act {
                action,
                seat: number("the seat", seat)?,
            }))
        }
        _ => Err(usage(LOBBY_USAGE)),
    }
}

/// `scene start`, with `words` the words after `start`.
pub fn parse_scene_start(words: &[String]) -> Result<PlayCommand, CommandError> {
    match words {
        [] => Ok(PlayCommand::SceneStart { seat: None }),
        [seat] => Ok(PlayCommand::SceneStart {
            seat: Some(number("the seat", seat)?),
        }),
        _ => Err(usage(SCENE_START_USAGE)),
    }
}

fn parse_wire(words: &[String]) -> Result<PlayCommand, CommandError> {
    match words {
        [verb, who] if verb == "view" => {
            let audience = match who.as_str() {
                "tv" => AudienceSpec::Tv,
                seat => AudienceSpec::Seat {
                    seat: seat.parse::<u64>().map_err(|_| CommandError::BadChoice {
                        key: "the audience".to_owned(),
                        text: who.clone(),
                        allowed: "tv or a seat number".to_owned(),
                    })?,
                },
            };
            Ok(PlayCommand::WireView(audience))
        }
        [verb, seat, rest @ ..] if verb == "send" => {
            let seat = number("the seat", seat)?;
            let (operation, rest) = take_operation(rest)?;
            let what = match rest.as_slice() {
                [flag, offer] if flag == "--tap" => WireIntentKind::Tap {
                    offer: offer.clone(),
                },
                [flag, line @ ..] if flag == "--text" && !line.is_empty() => WireIntentKind::Text {
                    text: line.join(" "),
                },
                [flag, line @ ..] if flag == "--say" && !line.is_empty() => WireIntentKind::Say {
                    text: line.join(" "),
                },
                [flag] if flag == "--pause" => WireIntentKind::Pause,
                _ => return Err(usage(WIRE_USAGE)),
            };
            Ok(PlayCommand::WireSend(WireIntentBody {
                seat,
                what,
                operation,
                machine: None,
                observed: None,
            }))
        }
        _ => Err(usage(WIRE_USAGE)),
    }
}

/// Takes `--op N` out of the words, wherever it is.
fn take_operation(words: &[String]) -> Result<(Option<u64>, Vec<String>), CommandError> {
    let mut operation = None;
    let mut rest = Vec::new();
    let mut remaining = words.iter();
    while let Some(word) = remaining.next() {
        if word == "--op" {
            let value = remaining.next().ok_or_else(|| usage(WIRE_USAGE))?;
            if operation
                .replace(number("the operation id", value)?)
                .is_some()
            {
                return Err(CommandError::DuplicateKey("--op".to_owned()));
            }
        } else {
            rest.push(word.clone());
        }
    }
    Ok((operation, rest))
}

fn parse_play(words: &[String]) -> Result<PlayCommand, CommandError> {
    match words {
        [verb] if verb == "publish" => Ok(PlayCommand::Publish { seat: None }),
        [verb, seat] if verb == "publish" => Ok(PlayCommand::Publish {
            seat: Some(number("the seat", seat)?),
        }),
        [verb] if verb == "remove" => Ok(PlayCommand::Remove),
        _ => Err(usage(PLAY_USAGE)),
    }
}

/// Parses `lobby`, `wire` or `play`.
pub fn parse(verb: &str, words: &[String]) -> Result<PlayCommand, CommandError> {
    match verb {
        "lobby" => parse_lobby(words),
        "wire" => parse_wire(words),
        "play" => parse_play(words),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// The seats of the lobby that said they are ready, from the lobby's saved state.
fn ready_seats(dump: &EngineDump) -> Vec<u64> {
    dump.slot
        .get("ready")
        .and_then(Value::as_array)
        .map(|seats| seats.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn seats_text(seats: &[u64]) -> String {
    if seats.is_empty() {
        "none".to_owned()
    } else {
        seats
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn format_lobby(dump: &EngineDump, readout: &str) -> String {
    format!(
        "1: the table is in {} ({}) at revision {}\n2: seats at the table: {}; ready: {}\n3: the shared screen reads: {readout}",
        dump.machine,
        dump.state,
        dump.revision,
        seats_text(&dump.table.seats),
        seats_text(&ready_seats(dump)),
    )
}

async fn tap(port: u16, seat: u64, action: &str) -> Result<SubmitReply, DevError> {
    let request = InputRequest {
        input: InputBody::Tap {
            seat,
            action: action.to_owned(),
        },
        operation: None,
        operation_id: None,
        view: None,
    };
    send(port, path::INPUT, &request).await
}

async fn audience_view(port: u16, audience: &str) -> Result<Value, DevError> {
    let address = format!("{}?{AUDIENCE_PARAMETER}={audience}", path::PROJECTION);
    fetch(port, &address).await
}

async fn lobby(port: u16, command: LobbyCommand) -> Result<String, DevError> {
    match command {
        LobbyCommand::Act { action, seat } => Ok(format_submit(&tap(port, seat, action).await?)),
        LobbyCommand::Show => {
            let dump: EngineDump = fetch(port, path::DUMP).await?;
            let tv = audience_view(port, "tv").await?;
            let readout = tv.get("readout").and_then(Value::as_str).unwrap_or("");
            Ok(format_lobby(&dump, readout))
        }
    }
}

fn rejected(message: String) -> DevError {
    DevError::Rejected(message)
}

async fn scene_start(port: u16, starter: Option<u64>) -> Result<String, DevError> {
    let dump: EngineDump = fetch(port, path::DUMP).await?;
    if dump.machine != "lobby" || dump.state != "gathering" {
        return Err(rejected(format!(
            "the lobby is not gathering (the table is in {}, {}); there is no scene to start",
            dump.machine, dump.state
        )));
    }
    let Some(first) = dump.table.seats.first().copied() else {
        return Err(rejected(
            "nobody is at the table; use `lobby join <seat>` first".to_owned(),
        ));
    };
    let ready = ready_seats(&dump);
    let mut lines = Vec::new();
    for seat in dump.table.seats.iter().filter(|seat| !ready.contains(seat)) {
        let reply = tap(port, *seat, "ready").await?;
        lines.push(format!("seat {seat} ready:\n{}", format_submit(&reply)));
    }
    let starter = starter.unwrap_or(first);
    let reply = tap(port, starter, "start").await?;
    lines.push(format!("seat {starter} start:\n{}", format_submit(&reply)));
    Ok(lines.join("\n"))
}

fn offer_line(offer: &super::machine_io::WireOfferReport) -> String {
    match &offer.unavailable_reason {
        Some(reason) => format!(
            "{} ({}) not available: {reason}",
            offer.label, offer.offer_id
        ),
        None => format!("{} ({})", offer.label, offer.offer_id),
    }
}

fn format_wire_view(report: &WireViewReport) -> String {
    let mut lines = vec![
        format!(
            "machine {} state '{}' revision {} status {}",
            report.machine, report.state, report.revision, report.status
        ),
        format!("readout: {}", report.readout),
    ];
    if let Some(reason) = &report.reason {
        lines.push(format!("reason: {reason}"));
    }
    match &report.audience {
        WireAudienceReport::SharedScreen {
            place,
            speaker,
            caption,
            party,
        } => {
            lines.push(format!("shared screen at {place}"));
            lines.push(format!(
                "caption: {}{caption}",
                speaker
                    .as_ref()
                    .map_or_else(String::new, |speaker| format!("{speaker}: "))
            ));
            lines.push(format!("party: {}", seats_or_none(party)));
        }
        WireAudienceReport::Seat {
            hero,
            narration,
            note,
            offers,
        } => {
            lines.push(format!("phone of {hero}"));
            lines.extend(narration.iter().map(|line| format!("narration: {line}")));
            if let Some(note) = note {
                lines.push(format!("note: {note}"));
            }
            lines.extend(
                offers
                    .iter()
                    .map(|offer| format!("offer: {}", offer_line(offer))),
            );
        }
    }
    lines.push(format!(
        "{} protobuf bytes on the wire",
        report.protobuf_bytes
    ));
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn seats_or_none(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

fn format_receipt(report: &WireReceiptReport) -> String {
    let mut line = format!("1: {} at revision {}", report.result, report.revision);
    if let Some(code) = &report.code {
        line.push_str(&format!(" code={code}"));
    }
    if let Some(reason) = &report.reason {
        line.push_str(&format!(": {reason}"));
    }
    if let Some(pending) = report.pending {
        line.push_str(&format!(" pending={pending}"));
    }
    if report.replayed {
        line.push_str(" (replayed)");
    }
    format!("{line}\n2: operation {}", report.operation)
}

/// The fixture text of the real projections of `tv` and one seat, as the K0 contract defines it.
fn fixture_text(tv: &Value, seat: &Value, seat_number: u64) -> String {
    json!({
        "name": LIVE_FIXTURE,
        "description": format!("The engine's own shared screen and the phone of seat {seat_number}, as projected now."),
        "tv": tv.get("view").cloned().unwrap_or(Value::Null),
        "phone": seat.get("view").cloned().unwrap_or(Value::Null),
    })
    .to_string()
}

async fn publish(port: u16, seat: Option<u64>) -> Result<String, DevError> {
    let seat = match seat {
        Some(seat) => seat,
        None => {
            let dump: EngineDump = fetch(port, path::DUMP).await?;
            dump.table.seats.first().copied().ok_or_else(|| {
                rejected("nobody is at the table; use `lobby join <seat>` first".to_owned())
            })?
        }
    };
    let tv = audience_view(port, "tv").await?;
    let phone = audience_view(port, &seat.to_string()).await?;
    let body = FixtureWrite {
        name: format!("{LIVE_FIXTURE}.json"),
        text: fixture_text(&tv, &phone, seat),
    };
    let text = serde_json::to_string(&body).map_err(|error| DevError::Unwritable {
        what: "fixture",
        reason: error.to_string(),
    })?;
    let response = client::post_json(port, route::DEV_FIXTURE_WRITE, &text)
        .await
        .map_err(DevError::Client)?;
    if response.status != 200 {
        return Err(rejected(format!(
            "the playground refused the fixture ({}): {}",
            response.status, response.body
        )));
    }
    Ok(format!(
        "1: wrote the fixture {LIVE_FIXTURE} (the shared screen and the phone of seat {seat}); show it with `playground go {LIVE_FIXTURE}`"
    ))
}

async fn remove(port: u16) -> Result<String, DevError> {
    let text = serde_json::to_string(&FixtureRemove {
        name: format!("{LIVE_FIXTURE}.json"),
    })
    .map_err(|error| DevError::Unwritable {
        what: "fixture",
        reason: error.to_string(),
    })?;
    let response = client::post_json(port, route::DEV_FIXTURE_REMOVE, &text)
        .await
        .map_err(DevError::Client)?;
    if response.status != 200 {
        return Err(rejected(format!(
            "the playground refused to remove the fixture ({}): {}",
            response.status, response.body
        )));
    }
    Ok(format!("1: removed the fixture {LIVE_FIXTURE}"))
}

/// Runs one command of the first playable path against the host on `port`.
pub async fn run(port: u16, command: PlayCommand) -> Result<String, DevError> {
    match command {
        PlayCommand::Lobby(command) => lobby(port, command).await,
        PlayCommand::SceneStart { seat } => scene_start(port, seat).await,
        PlayCommand::WireView(audience) => {
            let who = match audience {
                AudienceSpec::Tv => "tv".to_owned(),
                AudienceSpec::Seat { seat } => seat.to_string(),
            };
            let address = format!("{}?{AUDIENCE_PARAMETER}={who}", path::WIRE_VIEW);
            let report: WireViewReport = fetch(port, &address).await?;
            Ok(format_wire_view(&report))
        }
        PlayCommand::WireSend(body) => {
            let report: WireReceiptReport = send(port, path::WIRE_INTENT, &body).await?;
            Ok(format_receipt(&report))
        }
        PlayCommand::Publish { seat } => publish(port, seat).await,
        PlayCommand::Remove => remove(port).await,
    }
}
