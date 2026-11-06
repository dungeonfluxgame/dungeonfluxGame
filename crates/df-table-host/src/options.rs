//! The options of `serve` and the port of `dev`, read from the command line.
//!
//! It owns the option grammar and its defaults, and nothing else.

use std::fmt;
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 7878;
/// The kind directory when `--kinds` is not given, relative to where the host is started.
pub const DEFAULT_KIND_DIRECTORY: &str = "assets/canvas/kinds";
/// The playground's page, theme and fixtures when `--playground` is not given.
pub const DEFAULT_PLAYGROUND_DIRECTORY: &str = "assets/playground";
/// The playground's compiled package when `--package` is not given (see `development/playground.sh`).
pub const DEFAULT_PACKAGE_DIRECTORY: &str = "artifacts/playground/pkg";
/// The environment variable that names the port of `dev` when `--port` is not given.
pub const PORT_VARIABLE: &str = "DF_TABLE_PORT";

pub const SERVE_USAGE: &str = "\
df-table serve [--port N] [--kinds DIR] [--playground DIR] [--package DIR] [--telemetry DIR]
               [--moves] [--quiet]

  --port N     loopback port for the viewer and the developer API (default 7878)
  --kinds DIR  directory of kind files (default assets/canvas/kinds, relative to here)
  --playground DIR  the playground's page, theme and fixtures (default assets/playground)
  --package DIR     the playground's compiled package (default artifacts/playground/pkg)
  --telemetry DIR   the telemetry store root: the canvas's records also go down the shared
                    diagnostic path to it (default: none, the records stay in this process)
  --moves      also record moves, which are frequent (the log shows gaps without them)
  --quiet      do not echo audit lines to this terminal

The API has no password: any process on this machine can drive it.";

/// Why an option was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionError {
    Unknown(String),
    MissingValue(&'static str),
    BadPort(String),
}

impl fmt::Display for OptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(option) => write!(formatter, "unknown option '{option}'\n{SERVE_USAGE}"),
            Self::MissingValue(option) => write!(formatter, "{option} needs a value"),
            Self::BadPort(text) => {
                write!(
                    formatter,
                    "a port is a number from 0 to 65535, not '{text}'"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    pub port: u16,
    pub kind_directory: PathBuf,
    pub playground_directory: PathBuf,
    pub package_directory: PathBuf,
    /// The telemetry store root the shared diagnostic path writes to, when one is given.
    pub telemetry_root: Option<PathBuf>,
    pub records_moves: bool,
    pub echoes_lines: bool,
}

fn parse_port(text: &str) -> Result<u16, OptionError> {
    text.parse::<u16>()
        .map_err(|_| OptionError::BadPort(text.to_owned()))
}

/// Reads the options after `serve`.
pub fn parse_serve_options(arguments: &[String]) -> Result<ServeOptions, OptionError> {
    let mut options = ServeOptions {
        port: DEFAULT_PORT,
        kind_directory: PathBuf::from(DEFAULT_KIND_DIRECTORY),
        playground_directory: PathBuf::from(DEFAULT_PLAYGROUND_DIRECTORY),
        package_directory: PathBuf::from(DEFAULT_PACKAGE_DIRECTORY),
        telemetry_root: None,
        records_moves: false,
        echoes_lines: true,
    };
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                let value = arguments
                    .next()
                    .ok_or(OptionError::MissingValue("--port"))?;
                options.port = parse_port(value)?;
            }
            "--kinds" => {
                let value = arguments
                    .next()
                    .ok_or(OptionError::MissingValue("--kinds"))?;
                options.kind_directory = PathBuf::from(value);
            }
            "--playground" => {
                let value = arguments
                    .next()
                    .ok_or(OptionError::MissingValue("--playground"))?;
                options.playground_directory = PathBuf::from(value);
            }
            "--package" => {
                let value = arguments
                    .next()
                    .ok_or(OptionError::MissingValue("--package"))?;
                options.package_directory = PathBuf::from(value);
            }
            "--telemetry" => {
                let value = arguments
                    .next()
                    .ok_or(OptionError::MissingValue("--telemetry"))?;
                options.telemetry_root = Some(PathBuf::from(value));
            }
            "--moves" => options.records_moves = true,
            "--quiet" => options.echoes_lines = false,
            other => return Err(OptionError::Unknown(other.to_owned())),
        }
    }
    Ok(options)
}

/// Reads `--port N` from the front of a `dev` command line, or takes the environment's port, and
/// returns the port and the words of the command.
pub fn parse_dev_arguments(
    arguments: &[String],
    environment_port: Option<&str>,
) -> Result<(u16, Vec<String>), OptionError> {
    match arguments {
        [flag, value, rest @ ..] if flag == "--port" => Ok((parse_port(value)?, rest.to_vec())),
        [flag] if flag == "--port" => Err(OptionError::MissingValue("--port")),
        _ => {
            let port = environment_port.map_or(Ok(DEFAULT_PORT), parse_port)?;
            Ok((port, arguments.to_vec()))
        }
    }
}
