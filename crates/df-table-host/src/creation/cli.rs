//! The `creation` verbs of the dev command line (D19): list the offers, a seat's build steps
//! (begin, choose, clear, surprise, submit, reopen), show a seat's build or its sheet, show the
//! table, and start play.
//!
//! A seat's steps call the phone's own creation routes as that seat (`x-df-dev-seat`), one
//! surface for the phone and the command line. `creation sheet <seat> out=<file>` writes the
//! seat's derived sheet as the JSON file the check judge reads (`persona check run hero-<id>
//! sheet=<file>`). A refusal is the route's typed error.
use std::fmt;
use std::path::PathBuf;

use df_playground::pages::creation::{BuildChoose, BuildPick, BuildStep, CreationView};
use df_playground::path::{
    PLAY_CREATION, PLAY_CREATION_CHOOSE, PLAY_CREATION_REOPEN, PLAY_CREATION_SUBMIT,
    PLAY_CREATION_SURPRISE,
};
use df_rules::hero::HeroOffers;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::format::{format_offers, format_seat, format_start, format_table};
use super::request::{CreationReply, CreationRouteError, StartView, TableAsk, TableView};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_http::DevReply;
use crate::io_cause::IoCause;
use crate::seat::key::DEV_SEAT_HEADER;

/// Every `creation` verb, for the usage refusal and the help.
pub const CREATION_USAGE: &str = concat!(
    "creation offers | creation begin <seat> | creation choose <seat> <slot> <option> | ",
    "creation clear <seat> <slot> | creation surprise <seat> | creation submit <seat> | ",
    "creation reopen <seat> | creation show <seat> | creation sheet <seat> [out=<file>] | ",
    "creation table | creation start"
);

/// A seat's step: what it sends to which route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatStep {
    /// Set and clear picks (none: begin), then the rest is filled.
    Choose(BuildChoose),
    Surprise,
    Submit,
    Reopen,
}

/// A `creation` command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationCommand {
    Offers,
    Step {
        seat: u64,
        step: SeatStep,
    },
    Show(u64),
    /// The seat's sheet, printed, or written to `out`.
    Sheet {
        seat: u64,
        out: Option<PathBuf>,
    },
    Table,
    Start,
}

fn number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse()
        .map_err(|_| bad_choice(what, text, "a whole number"))
}

/// Reads the words after `creation`.
pub fn parse(words: &[String]) -> Result<CreationCommand, CommandError> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let step = |text: &str, step: SeatStep| {
        Ok(CreationCommand::Step {
            seat: number("seat", text)?,
            step,
        })
    };
    match words.as_slice() {
        ["offers"] => Ok(CreationCommand::Offers),
        ["begin", text] => step(text, SeatStep::Choose(BuildChoose::default())),
        ["choose", text, slot, option] => step(
            text,
            SeatStep::Choose(BuildChoose {
                picks: vec![BuildPick {
                    slot: (*slot).to_owned(),
                    option: (*option).to_owned(),
                }],
                clear: Vec::new(),
            }),
        ),
        ["clear", text, slot] => step(
            text,
            SeatStep::Choose(BuildChoose {
                picks: Vec::new(),
                clear: vec![(*slot).to_owned()],
            }),
        ),
        ["surprise", text] => step(text, SeatStep::Surprise),
        ["submit", text] => step(text, SeatStep::Submit),
        ["reopen", text] => step(text, SeatStep::Reopen),
        ["show", text] => number("seat", text).map(CreationCommand::Show),
        ["sheet", text] => Ok(CreationCommand::Sheet {
            seat: number("seat", text)?,
            out: None,
        }),
        ["sheet", text, out] => match out.split_once('=') {
            Some(("out", file)) if !file.is_empty() => Ok(CreationCommand::Sheet {
                seat: number("seat", text)?,
                out: Some(PathBuf::from(file)),
            }),
            _ => Err(bad_choice("sheet", out, "out=<file>")),
        },
        ["table"] => Ok(CreationCommand::Table),
        ["start"] => Ok(CreationCommand::Start),
        _ => Err(CommandError::Usage(CREATION_USAGE)),
    }
}

/// Why a `creation` verb did not answer, besides a request that did not reach the host or an
/// answer that did not read (`DevError`'s own); carried as `DevError::Creation`.
#[derive(Debug, Clone, PartialEq)]
pub enum CreationCliError {
    /// A creation route refused: its typed error and the host's sentence.
    Rejected {
        error: CreationRouteError,
        message: String,
    },
    /// The sheet file could not be written.
    Unwritable { path: PathBuf, cause: IoCause },
}

impl fmt::Display for CreationCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected { message, .. } => write!(formatter, "refused: {message}"),
            Self::Unwritable { path, cause } => {
                write!(
                    formatter,
                    "{} could not be written: {cause}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for CreationCliError {}

impl From<CreationCliError> for DevError {
    fn from(error: CreationCliError) -> Self {
        Self::Creation(error)
    }
}

fn value_of<T>(reply: CreationReply<T>) -> Result<T, DevError> {
    match reply {
        DevReply::Done { value } => Ok(value),
        DevReply::Rejected { error, message } => {
            Err(CreationCliError::Rejected { error, message }.into())
        }
    }
}

async fn post<Request: Serialize, Reply: DeserializeOwned>(
    port: u16,
    route: &str,
    request: &Request,
) -> Result<Reply, DevError> {
    value_of(crate::dev_client::post_json(port, route, request).await?)
}

async fn get<Reply: DeserializeOwned>(port: u16, route: &str) -> Result<Reply, DevError> {
    value_of(crate::dev_client::get_json(port, route).await?)
}

/// Posts `request` to the seat's own route `route`, as the seat.
async fn post_as<Request: Serialize>(
    port: u16,
    seat: u64,
    route: &str,
    request: &Request,
) -> Result<CreationView, DevError> {
    let seat = seat.to_string();
    let headers = [(DEV_SEAT_HEADER, seat.as_str())];
    value_of(crate::dev_client::post_json_with(port, route, &headers, request).await?)
}

async fn seat_view(port: u16, seat: u64) -> Result<CreationView, DevError> {
    let seat = seat.to_string();
    let headers = [(DEV_SEAT_HEADER, seat.as_str())];
    value_of(crate::dev_client::get_json_with(port, PLAY_CREATION, &headers).await?)
}

/// Writes `text` to `path`, off the async workers.
async fn write_file(path: PathBuf, text: String) -> Result<(), CreationCliError> {
    let target = path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&target, text))
        .await
        .map_err(|_| CreationCliError::Unwritable {
            path: path.clone(),
            cause: IoCause::Other,
        })?
        .map_err(|error| CreationCliError::Unwritable {
            path,
            cause: IoCause::from_kind(error.kind()),
        })
}

/// The seat's sheet file as pretty JSON, printed or written to `out`.
async fn sheet(port: u16, seat: u64, out: Option<PathBuf>) -> Result<String, DevError> {
    let sheet: serde_json::Value = get(port, &format!("{}?seat={seat}", path::SHEET)).await?;
    let text = serde_json::to_string_pretty(&sheet).map_err(|error| DevError::Unwritable {
        what: "sheet",
        reason: error.to_string(),
    })?;
    let Some(path) = out else {
        return Ok(text);
    };
    write_file(path.clone(), format!("{text}\n")).await?;
    let id = sheet["id"].as_str().unwrap_or_default().to_owned();
    Ok(format!(
        "wrote seat {seat}'s sheet to {}; roll from it with: persona check run {id} sheet={}",
        path.display(),
        path.display()
    ))
}

/// Runs one `creation` command against the host on `port`.
pub async fn run(port: u16, command: CreationCommand) -> Result<String, DevError> {
    match command {
        CreationCommand::Offers => {
            let offers: HeroOffers = get(port, path::OFFERS).await?;
            Ok(format_offers(&offers))
        }
        CreationCommand::Step { seat, step } => {
            let view = match step {
                SeatStep::Choose(choose) => {
                    post_as(port, seat, PLAY_CREATION_CHOOSE, &choose).await?
                }
                SeatStep::Surprise => {
                    post_as(port, seat, PLAY_CREATION_SURPRISE, &BuildStep {}).await?
                }
                SeatStep::Submit => {
                    post_as(port, seat, PLAY_CREATION_SUBMIT, &BuildStep {}).await?
                }
                SeatStep::Reopen => {
                    post_as(port, seat, PLAY_CREATION_REOPEN, &BuildStep {}).await?
                }
            };
            Ok(format_seat(seat, &view))
        }
        CreationCommand::Show(seat) => Ok(format_seat(seat, &seat_view(port, seat).await?)),
        CreationCommand::Sheet { seat, out } => sheet(port, seat, out).await,
        CreationCommand::Table => {
            let view: TableView = get(port, path::TABLE).await?;
            Ok(format_table(&view))
        }
        CreationCommand::Start => {
            let view: StartView = post(port, path::START, &TableAsk {}).await?;
            Ok(format_start(&view))
        }
    }
}
