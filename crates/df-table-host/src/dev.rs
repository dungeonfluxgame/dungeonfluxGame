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
use crate::limits::MAXIMUM_ECHOED_ANSWER;
use crate::playground::fixture_file_name;
use crate::report::{
    describe_reply, format_canvas, format_clients, format_events, format_kinds, format_media,
    format_playground,
};
use crate::view::CanvasView;
use crate::wire::{
    ClientView, ControlsBatch, ControlsReply, EVENTS_AFTER_PARAMETER, EventsReply, FixtureRemove,
    FixtureWrite, MediaRemove, Reply, route,
};

/// Why a `dev` command failed.
#[derive(Debug, Clone, PartialEq)]
pub enum DevError {
    Command(CommandError),
    Client(ClientError),
    /// The canvas understood the request and refused it; the text says why.
    Rejected(String),
    /// The canvas answered with something this program does not understand. `reason` is why the
    /// answer did not read, when it was text that should have been JSON of a known shape.
    UnexpectedAnswer {
        status: u16,
        body: String,
        reason: Option<String>,
    },
    /// A request or an answer this program built could not be written as JSON.
    Unwritable {
        what: &'static str,
        reason: String,
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
            Self::UnexpectedAnswer {
                status,
                body,
                reason: None,
            } => write!(formatter, "unexpected answer (status {status}): {body}"),
            Self::UnexpectedAnswer {
                status,
                body,
                reason: Some(reason),
            } => write!(
                formatter,
                "unexpected answer (status {status}, {reason}): {body}"
            ),
            Self::Unwritable { what, reason } => {
                write!(formatter, "could not write the {what}: {reason}")
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
        reason: None,
    }
}

/// An answer that should have read as JSON of a known shape and did not, with the reason.
fn unreadable_answer(response: &Response, error: &serde_json::Error) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: shortened(&response.body),
        reason: Some(error.to_string()),
    }
}

/// Reads an answer that must be a success carrying `T`.
fn read_json<T: serde::de::DeserializeOwned>(response: &Response) -> Result<T, DevError> {
    if response.status != 200 {
        return Err(unexpected(response));
    }
    serde_json::from_str(&response.body).map_err(|error| unreadable_answer(response, &error))
}

fn read_reply(response: &Response) -> Result<Result<String, String>, DevError> {
    let reply: Reply = serde_json::from_str(&response.body)
        .map_err(|error| unreadable_answer(response, &error))?;
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
    serde_json::to_string(value).map_err(|error| DevError::Unwritable {
        what: "request",
        reason: error.to_string(),
    })
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

/// Bytes of a local file read, encoded and sent at a time: a multiple of three, so every piece but
/// the last encodes to base64 without padding.
const UPLOAD_PIECE_BYTES: usize = 3 * 64 * 1024;

/// The size of a local file, refusing one over `maximum_bytes` before any of it is read.
pub(crate) fn checked_file_size(size: u64, maximum_bytes: u64) -> Result<u64, String> {
    if size > maximum_bytes {
        return Err(format!("the file is larger than {maximum_bytes} bytes"));
    }
    Ok(size)
}

/// The length of the add request for a file of `file_bytes`: the envelope around the file's
/// base64, or `None` when it does not fit a length.
fn upload_body_length(envelope_bytes: usize, file_bytes: u64) -> Option<u64> {
    file_bytes
        .div_ceil(3)
        .checked_mul(4)?
        .checked_add(u64::try_from(envelope_bytes).ok()?)
}

/// The next piece of `file`, up to `UPLOAD_PIECE_BYTES`, read on a blocking thread.
async fn read_piece(file: std::fs::File) -> std::io::Result<(std::fs::File, Vec<u8>)> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = file;
        let mut piece = Vec::new();
        (&mut file)
            .take(u64::try_from(UPLOAD_PIECE_BYTES).unwrap_or(u64::MAX))
            .read_to_end(&mut piece)?;
        Ok((file, piece))
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Sends a local file to the library without holding it: the file is read, encoded and written to
/// the socket a piece at a time, and the host decodes it as it arrives.
async fn upload_media(port: u16, name: String, path: &Path) -> Result<String, DevError> {
    let unreadable = |message: String| DevError::UnreadableFile {
        path: path.to_path_buf(),
        message,
    };
    let owned = path.to_path_buf();
    let (file, size) = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(owned)?;
        let size = file.metadata()?.len();
        Ok::<_, std::io::Error>((file, size))
    })
    .await
    .map_err(|error| unreadable(error.to_string()))?
    .map_err(|error| unreadable(error.to_string()))?;
    let size =
        checked_file_size(size, crate::media_library::MAXIMUM_MEDIA_BYTES).map_err(&unreadable)?;
    let quoted_name = to_body(&name)?;
    let prefix = format!("{{\"name\":{quoted_name},\"data_base64\":\"");
    let suffix = "\"}";
    let length = upload_body_length(prefix.len() + suffix.len(), size)
        .ok_or_else(|| unreadable("the file is too large to send".to_owned()))?;
    let mut post = client::StreamedPost::begin(port, route::DEV_MEDIA_ADD, length)
        .await
        .map_err(DevError::Client)?;
    let sent = send_upload(&mut post, &prefix, suffix, file, size).await;
    // The host may have answered before the end (a refusal); its answer says why, so it is read
    // even when sending stopped.
    let response = match post.finish().await {
        Ok(response) => response,
        Err(error) => {
            return Err(match sent {
                Err(UploadStop::File(message)) => unreadable(message),
                _ => DevError::Client(error),
            });
        }
    };
    match read_reply(&response)? {
        Ok(text) => Ok(text),
        Err(text) => Err(DevError::Rejected(text)),
    }
}

/// Why sending a file stopped before its end.
enum UploadStop {
    File(String),
    Host,
}

async fn send_upload(
    post: &mut client::StreamedPost,
    prefix: &str,
    suffix: &str,
    mut file: std::fs::File,
    size: u64,
) -> Result<(), UploadStop> {
    let to_host = |_: ClientError| UploadStop::Host;
    post.send(prefix.as_bytes()).await.map_err(to_host)?;
    let mut remaining = size;
    while remaining > 0 {
        let (next, piece) = read_piece(file)
            .await
            .map_err(|error| UploadStop::File(error.to_string()))?;
        file = next;
        let read = u64::try_from(piece.len()).unwrap_or(u64::MAX);
        if read == 0 || read > remaining {
            return Err(UploadStop::File(
                "the file changed while it was read".to_owned(),
            ));
        }
        remaining -= read;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&piece);
        post.send(encoded.as_bytes()).await.map_err(to_host)?;
    }
    post.send(suffix.as_bytes()).await.map_err(to_host)
}

/// Changes the playground to `changes` as one change: the host applies all of them or none.
async fn go(
    port: u16,
    changes: Vec<df_playground::controls::ControlsChange>,
) -> Result<String, DevError> {
    let body = to_body(&ControlsBatch { changes })?;
    send_each(
        port,
        route::DEV_PLAYGROUND_CONTROLS_BATCH,
        "change",
        &[body],
        read_controls_reply,
    )
    .await
}

/// Goes to each of two scenes in turn, `times` rounds, pausing `hold_ms` between scenes (so
/// `2 * times - 1` pauses; the last scene is not held, the command returns). Each step changes the
/// transition and the scene as one change. A scene that cannot be reached stops the cycle there
/// and says which step it was.
async fn cycle(
    port: u16,
    scenes: [String; 2],
    times: u32,
    hold_ms: u64,
    transition: &[df_playground::controls::ControlsChange],
) -> Result<String, DevError> {
    let mut lines = Vec::new();
    let total_steps = u64::from(times).saturating_mul(2);
    let mut step = 0_u64;
    for _ in 0..times {
        for scene in &scenes {
            step += 1;
            let mut changes = transition.to_vec();
            changes.push(df_playground::controls::ControlsChange::Fixture(
                scene.clone(),
            ));
            match go(port, changes).await {
                Ok(text) => lines.push(format!("{step}: {scene} {}", last_line(&text))),
                Err(error) => {
                    lines.push(format!("{step}: {scene} failed: {error}"));
                    return Err(DevError::Rejected(lines.join("\n")));
                }
            }
            if step < total_steps {
                tokio::time::sleep(std::time::Duration::from_millis(hold_ms)).await;
            }
        }
    }
    lines.push(format!(
        "cycled {times} rounds between {} and {}",
        scenes[0], scenes[1]
    ));
    Ok(lines.join("\n"))
}

fn last_line(text: &str) -> &str {
    text.lines().last().unwrap_or(text)
}

async fn show_fixture(port: u16, name: &str) -> Result<String, DevError> {
    let file_name =
        fixture_file_name(name).map_err(|error| DevError::Rejected(error.to_string()))?;
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

/// Runs one parsed command: the groups of commands that talk about the playground or the library
/// are run by their own functions.
async fn execute(port: u16, command: Command) -> Result<String, DevError> {
    match command {
        Command::Playground
        | Command::PlaygroundSet(_)
        | Command::PlaygroundGo(_)
        | Command::PlaygroundCycle { .. }
        | Command::Scene(_)
        | Command::Wait(_) => execute_playground(port, command).await,
        Command::Media
        | Command::MediaAdd { .. }
        | Command::MediaRemove(_)
        | Command::Clients
        | Command::Fixtures
        | Command::FixtureShow(_)
        | Command::FixtureWrite { .. }
        | Command::FixtureRemove(_) => execute_library(port, command).await,
        Command::MediaGenerate(command) => crate::imagegen::run(port, command).await,
        Command::Effects(requests) => {
            let bodies = requests
                .iter()
                .map(to_body)
                .collect::<Result<Vec<_>, _>>()?;
            send_each(port, route::EFFECT, "effect", &bodies, read_reply).await
        }
        Command::Canvas => {
            let view = fetch_canvas(port).await?;
            format_canvas(&view).map_err(|error| DevError::Unwritable {
                what: "canvas",
                reason: error.to_string(),
            })
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
        Command::Engine(command) => crate::engine::run(port, command).await,
        Command::SceneBuilder(command) => crate::scene_builder::execute(port, command).await,
        Command::Repl => Err(DevError::Rejected(
            "repl is started from the command line: df-table dev repl".to_owned(),
        )),
        Command::Script(path) => run_script(port, &path).await,
    }
}

/// The commands that change or show what the playground shows, and `wait`.
async fn execute_playground(port: u16, command: Command) -> Result<String, DevError> {
    match command {
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
        Command::PlaygroundGo(changes) => go(port, changes).await,
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
        Command::Wait(ms) => {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            Ok(format!("waited {ms} ms"))
        }
        Command::PlaygroundCycle {
            first,
            second,
            times,
            hold_ms,
            transition,
        } => cycle(port, [first, second], times, hold_ms, &transition).await,
        other => Err(DevError::Rejected(format!(
            "{other:?} is not a playground command"
        ))),
    }
}

/// The commands about the media library, the open pages and the fixture files.
async fn execute_library(port: u16, command: Command) -> Result<String, DevError> {
    match command {
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
            upload_media(port, name, &path).await
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
        other => Err(DevError::Rejected(format!(
            "{other:?} is not a library command"
        ))),
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
