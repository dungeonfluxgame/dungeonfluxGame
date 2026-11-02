//! Running one `dev` command against the canvas that `serve` started.
//!
//! It owns the flow from a parsed command to the text to print: it sends the requests, reads the
//! answers back as the API's typed answers, and reports a failure as a typed `DevError`. It never
//! prints and never starts a server.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::client::{self, ClientError, Response};
use crate::command::{self, Command, CommandError, usage_text};
use crate::report::{describe_reply, format_canvas, format_events, format_kinds};
use crate::view::CanvasView;
use crate::wire::{EVENTS_AFTER_PARAMETER, EffectRequest, EventsReply, Reply, route};

/// Most characters of an unexpected answer echoed in an error.
const MAXIMUM_ECHOED_ANSWER: usize = 200;

/// Why a `dev` command failed.
#[derive(Debug, Clone, PartialEq)]
pub enum DevError {
    Command(CommandError),
    Client(ClientError),
    /// The canvas understood the request and refused it; the text says why.
    Rejected(String),
    /// The canvas answered with something this program does not understand.
    UnexpectedAnswer {
        status: u16,
        body: String,
    },
    UnreadableScript {
        path: PathBuf,
        message: String,
    },
    /// A line of a script that could not be parsed. Nothing in the script has run.
    ScriptLine {
        path: PathBuf,
        line: usize,
        error: CommandError,
    },
    /// A line of a script that failed when it ran. The lines before it have run.
    ScriptStopped {
        path: PathBuf,
        line: usize,
        error: Box<DevError>,
    },
}

impl fmt::Display for DevError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(error) => write!(formatter, "{error}"),
            Self::Client(error) => write!(formatter, "{error}"),
            Self::Rejected(text) => formatter.write_str(text),
            Self::UnexpectedAnswer { status, body } => {
                write!(formatter, "unexpected answer (status {status}): {body}")
            }
            Self::UnreadableScript { path, message } => {
                write!(formatter, "cannot read {}: {message}", path.display())
            }
            Self::ScriptLine { path, line, error } => {
                write!(formatter, "{}:{line}: {error}", path.display())
            }
            Self::ScriptStopped { path, line, error } => {
                write!(formatter, "{}:{line}: {error}", path.display())
            }
        }
    }
}

fn shortened(text: &str) -> String {
    text.chars().take(MAXIMUM_ECHOED_ANSWER).collect()
}

fn unexpected(response: &Response) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: shortened(&response.body),
    }
}

/// Reads an answer that must be a success carrying `T`.
fn read_json<T: serde::de::DeserializeOwned>(response: &Response) -> Result<T, DevError> {
    if response.status != 200 {
        return Err(unexpected(response));
    }
    serde_json::from_str(&response.body).map_err(|_| unexpected(response))
}

fn read_reply(response: &Response) -> Result<Result<String, String>, DevError> {
    let reply: Reply = serde_json::from_str(&response.body).map_err(|_| unexpected(response))?;
    Ok(describe_reply(&reply))
}

async fn send_effects(port: u16, requests: &[EffectRequest]) -> Result<String, DevError> {
    let mut lines = Vec::new();
    let mut any_rejected = false;
    for (index, request) in requests.iter().enumerate() {
        let body = serde_json::to_string(request)
            .map_err(|_| DevError::Rejected("could not write the request".to_owned()))?;
        let response = client::post_json(port, route::EFFECT, &body)
            .await
            .map_err(DevError::Client)?;
        match read_reply(&response)? {
            Ok(text) => lines.push((index, text)),
            Err(text) => {
                any_rejected = true;
                lines.push((index, text));
            }
        }
    }
    let text = if let [(_, only)] = lines.as_slice() {
        only.clone()
    } else {
        lines
            .iter()
            .map(|(index, text)| format!("effect {}: {text}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if any_rejected {
        Err(DevError::Rejected(text))
    } else {
        Ok(text)
    }
}

async fn fetch_canvas(port: u16) -> Result<CanvasView, DevError> {
    let response = client::get(port, route::CANVAS)
        .await
        .map_err(DevError::Client)?;
    read_json(&response)
}

async fn execute(port: u16, command: Command) -> Result<String, DevError> {
    match command {
        Command::Effects(requests) => send_effects(port, &requests).await,
        Command::Canvas => {
            let view = fetch_canvas(port).await?;
            format_canvas(&view)
                .map_err(|_| DevError::Rejected("could not write the canvas".to_owned()))
        }
        Command::Kinds => Ok(format_kinds(&fetch_canvas(port).await?)),
        Command::Events { after } => {
            let path = match after {
                Some(after) => format!("{}?{EVENTS_AFTER_PARAMETER}={after}", route::EVENTS),
                None => route::EVENTS.to_owned(),
            };
            let response = client::get(port, &path).await.map_err(DevError::Client)?;
            let reply: EventsReply = read_json(&response)?;
            Ok(format_events(&reply, after))
        }
        Command::Reload => {
            let response = client::post_json(port, route::RELOAD, "")
                .await
                .map_err(DevError::Client)?;
            read_reply(&response)?.map_err(DevError::Rejected)
        }
        Command::Help => Ok(usage_text()),
        Command::Script(path) => run_script(port, &path).await,
    }
}

/// Reads and checks every line of a script before anything runs, so a typo on line nine does not
/// leave the canvas half built.
async fn read_script(path: &Path) -> Result<Vec<(usize, Command)>, DevError> {
    let owned = path.to_path_buf();
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(&owned))
        .await
        .map_err(|error| DevError::UnreadableScript {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
        .map_err(|error| DevError::UnreadableScript {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let mut commands = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let script_error = |error| DevError::ScriptLine {
            path: path.to_path_buf(),
            line: line_number,
            error,
        };
        let words = command::split_words(line).map_err(script_error)?;
        let command = command::parse_script_line(&words).map_err(script_error)?;
        commands.push((line_number, command));
    }
    Ok(commands)
}

async fn run_script(port: u16, path: &Path) -> Result<String, DevError> {
    let commands = read_script(path).await?;
    let mut output = Vec::new();
    for (line, command) in &commands {
        let text = Box::pin(execute(port, command.clone()))
            .await
            .map_err(|error| DevError::ScriptStopped {
                path: path.to_path_buf(),
                line: *line,
                error: Box::new(error),
            })?;
        output.push(format!("{line}: {text}"));
    }
    output.push(format!("ran {} commands, all ok", commands.len()));
    Ok(output.join("\n"))
}

/// Runs one `dev` command line against the canvas on `port` and returns the text to print.
pub async fn run_dev(port: u16, words: &[String]) -> Result<String, DevError> {
    let command = command::parse(words).map_err(DevError::Command)?;
    execute(port, command).await
}
