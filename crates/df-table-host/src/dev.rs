//! Running one `dev` command against the canvas that `serve` started.
//!
//! It owns the flow from a parsed command to the text to print: it sends the requests, reads the
//! answers back as the API's typed answers, and reports a failure as a typed `DevError`. It never
//! prints and never starts a server.

use std::fmt;

use base64::Engine;
use std::path::{Path, PathBuf};

use crate::client::{self, ClientError, Response};
use crate::command::{self, Command, CommandError, usage_text};
use crate::playground::fixture_file_name;
use crate::report::{
    describe_reply, format_canvas, format_clients, format_events, format_kinds, format_media,
    format_playground,
};
use crate::view::CanvasView;
use crate::wire::{
    ClientView, ControlsReply, EVENTS_AFTER_PARAMETER, EventsReply, FixtureRemove, FixtureWrite,
    MediaAdd, MediaRemove, Reply, route,
};

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
    UnreadableFile {
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
            Self::UnreadableFile { path, message } => {
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

/// Reads an answer to a change of the playground: the new controls, or a refusal.
fn read_controls_reply(response: &Response) -> Result<Result<String, String>, DevError> {
    if response.status == 200
        && let Ok(reply) = serde_json::from_str::<ControlsReply>(&response.body)
    {
        return Ok(Ok(format!("ok (revision {})", reply.revision)));
    }
    read_reply(response)
}

/// Posts each body to `path` in order and reports every answer. A command with several bodies
/// numbers them; one failure does not stop the rest, and any failure makes the command fail.
async fn send_each(
    port: u16,
    path: &str,
    label: &str,
    bodies: &[String],
    read: fn(&Response) -> Result<Result<String, String>, DevError>,
) -> Result<String, DevError> {
    let mut lines = Vec::new();
    let mut any_rejected = false;
    for body in bodies {
        let response = client::post_json(port, path, body)
            .await
            .map_err(DevError::Client)?;
        match read(&response)? {
            Ok(text) => lines.push(text),
            Err(text) => {
                any_rejected = true;
                lines.push(text);
            }
        }
    }
    let text = if let [only] = lines.as_slice() {
        only.clone()
    } else {
        lines
            .iter()
            .enumerate()
            .map(|(index, text)| format!("{label} {}: {text}", index + 1))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    };
    if any_rejected {
        Err(DevError::Rejected(text))
    } else {
        Ok(text)
    }
}

fn to_body<T: serde::Serialize>(value: &T) -> Result<String, DevError> {
    serde_json::to_string(value)
        .map_err(|_| DevError::Rejected("could not write the request".to_owned()))
}

async fn fetch_fixture_names(port: u16) -> Result<Vec<String>, DevError> {
    let response = client::get(port, route::PLAYGROUND_FIXTURE_LIST)
        .await
        .map_err(DevError::Client)?;
    read_json(&response)
}

async fn show_playground(port: u16) -> Result<String, DevError> {
    let response = client::get(port, route::PLAYGROUND_CONTROLS)
        .await
        .map_err(DevError::Client)?;
    let reply: ControlsReply = read_json(&response)?;
    let mut text = format_playground(&reply, &fetch_fixture_names(port).await?);
    text.push('\n');
    text.push_str(&format_clients(&fetch_clients(port).await?));
    Ok(text)
}

async fn fetch_clients(port: u16) -> Result<Vec<ClientView>, DevError> {
    let response = client::get(port, route::PLAYGROUND_CLIENTS)
        .await
        .map_err(DevError::Client)?;
    read_json(&response)
}

async fn fetch_media(port: u16) -> Result<Vec<df_playground::media::MediaEntry>, DevError> {
    let response = client::get(port, route::PLAYGROUND_MEDIA_LIST)
        .await
        .map_err(DevError::Client)?;
    read_json(&response)
}

/// Reads a local file's bytes, refusing one over the library's limit without reading it.
async fn read_binary_file(path: &Path) -> Result<Vec<u8>, DevError> {
    let owned = path.to_path_buf();
    let unreadable = |message: String| DevError::UnreadableFile {
        path: path.to_path_buf(),
        message,
    };
    tokio::task::spawn_blocking(move || {
        let size = std::fs::metadata(&owned)?.len();
        if size > crate::media_library::MAXIMUM_MEDIA_BYTES as u64 {
            return Err(std::io::Error::other(format!(
                "the file is larger than {} bytes",
                crate::media_library::MAXIMUM_MEDIA_BYTES
            )));
        }
        std::fs::read(&owned)
    })
    .await
    .map_err(|error| unreadable(error.to_string()))?
    .map_err(|error| unreadable(error.to_string()))
}

async fn show_fixture(port: u16, name: &str) -> Result<String, DevError> {
    let file_name = fixture_file_name(name).map_err(DevError::Rejected)?;
    let path = route::PLAYGROUND_FIXTURE.replace("{name}", &file_name);
    let response = client::get(port, &path).await.map_err(DevError::Client)?;
    match response.status {
        200 => Ok(response.body),
        404 => Err(DevError::Rejected(format!(
            "there is no fixture '{file_name}'"
        ))),
        _ => Err(unexpected(&response)),
    }
}

async fn read_text_file(path: &Path) -> Result<String, DevError> {
    let owned = path.to_path_buf();
    let unreadable = |message: String| DevError::UnreadableFile {
        path: path.to_path_buf(),
        message,
    };
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&owned))
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .map_err(|error| unreadable(error.to_string()))
}

async fn fetch_canvas(port: u16) -> Result<CanvasView, DevError> {
    let response = client::get(port, route::CANVAS)
        .await
        .map_err(DevError::Client)?;
    read_json(&response)
}

async fn execute(port: u16, command: Command) -> Result<String, DevError> {
    match command {
        Command::Effects(requests) => {
            let bodies = requests
                .iter()
                .map(to_body)
                .collect::<Result<Vec<_>, _>>()?;
            send_each(port, route::EFFECT, "effect", &bodies, read_reply).await
        }
        Command::Playground => show_playground(port).await,
        Command::PlaygroundSet(changes) => {
            let bodies = changes.iter().map(to_body).collect::<Result<Vec<_>, _>>()?;
            send_each(
                port,
                route::DEV_PLAYGROUND_CONTROLS,
                "change",
                &bodies,
                read_controls_reply,
            )
            .await
        }
        Command::Scene(change) => {
            let body = to_body(&change)?;
            send_each(
                port,
                route::DEV_PLAYGROUND_SCENE,
                "change",
                &[body],
                read_controls_reply,
            )
            .await
        }
        Command::Media => Ok(format_media(&fetch_media(port).await?)),
        Command::MediaAdd { path, name } => {
            let name = match name {
                Some(name) => name,
                None => path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .ok_or_else(|| {
                        DevError::Rejected(format!("{} has no file name", path.display()))
                    })?,
            };
            let bytes = read_binary_file(&path).await?;
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let body = to_body(&MediaAdd { name, data_base64 })?;
            send_each(port, route::DEV_MEDIA_ADD, "media", &[body], read_reply).await
        }
        Command::MediaRemove(name) => {
            let body = to_body(&MediaRemove { name })?;
            send_each(port, route::DEV_MEDIA_REMOVE, "media", &[body], read_reply).await
        }
        Command::Clients => Ok(format_clients(&fetch_clients(port).await?)),
        Command::Fixtures => Ok(fetch_fixture_names(port).await?.join(
            "
",
        )),
        Command::FixtureShow(name) => show_fixture(port, &name).await,
        Command::FixtureWrite { name, path } => {
            let text = read_text_file(&path).await?;
            let body = to_body(&FixtureWrite { name, text })?;
            send_each(
                port,
                route::DEV_FIXTURE_WRITE,
                "fixture",
                &[body],
                read_reply,
            )
            .await
        }
        Command::FixtureRemove(name) => {
            let body = to_body(&FixtureRemove { name })?;
            send_each(
                port,
                route::DEV_FIXTURE_REMOVE,
                "fixture",
                &[body],
                read_reply,
            )
            .await
        }
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
        Command::Repl => Err(DevError::Rejected(
            "repl is started from the command line: df-table dev repl".to_owned(),
        )),
        Command::Script(path) => run_script(port, &path).await,
    }
}

/// Reads and checks every line of a script before anything runs, so a typo on line nine does not
/// leave the canvas half built.
async fn read_script(path: &Path) -> Result<Vec<(usize, Command)>, DevError> {
    let text = read_text_file(path).await?;
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

/// Runs one parsed command against the host on `port` and returns the text to print.
pub async fn run_dev_command(port: u16, command: Command) -> Result<String, DevError> {
    execute(port, command).await
}

/// Runs one `dev` command line against the canvas on `port` and returns the text to print.
pub async fn run_dev(port: u16, words: &[String]) -> Result<String, DevError> {
    let command = command::parse(words).map_err(DevError::Command)?;
    execute(port, command).await
}
