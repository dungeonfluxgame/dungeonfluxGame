//! The command line: the only other place besides `environment.rs` that touches the process
//! environment, and only to read the argument list.

use std::{ffi::OsString, net::SocketAddr, path::PathBuf};

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    /// `config check --config <path>`: load and validate, print the effective listing.
    ConfigCheck { config: Option<PathBuf> },
    /// `control-plane --config <path>`
    ControlPlane { config: Option<PathBuf> },
    /// `campaign-runtime --descriptor <path>`
    CampaignRuntime { descriptor: Option<PathBuf> },
    /// `egress --config <path>`
    Egress { config: Option<PathBuf> },
    /// `healthcheck --address <addr> --signal <name>`
    Healthcheck {
        address: Option<SocketAddr>,
        signal: Option<String>,
        ca_file: Option<PathBuf>,
        server_name: Option<String>,
    },
    /// `migrate <status|plan|apply> --config <path>`
    Migrate {
        action: String,
        config: Option<PathBuf>,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum UsageError {
    MissingCommand,
    UnknownCommand(String),
    MissingValue(&'static str),
    UnknownOption(String),
}

/// The arguments after the program name.
pub fn arguments() -> Vec<OsString> {
    std::env::args_os().skip(1).collect()
}

pub fn parse(arguments: &[OsString]) -> Result<Command, UsageError> {
    let mut words = arguments.iter();
    let first = words.next().ok_or(UsageError::MissingCommand)?;
    match first.to_str() {
        Some("config") => match words.next().and_then(|word| word.to_str()) {
            Some("check") => {
                parse_path_option(words, "--config").map(|config| Command::ConfigCheck { config })
            }
            other => Err(UsageError::UnknownCommand(
                other.unwrap_or_default().to_string(),
            )),
        },
        Some("control-plane") => {
            parse_path_option(words, "--config").map(|config| Command::ControlPlane { config })
        }
        Some("campaign-runtime") => parse_path_option(words, "--descriptor")
            .map(|descriptor| Command::CampaignRuntime { descriptor }),
        Some("egress") => {
            parse_path_option(words, "--config").map(|config| Command::Egress { config })
        }
        Some("healthcheck") => parse_healthcheck(words),
        Some("migrate") => {
            let action = words
                .next()
                .and_then(|word| word.to_str())
                .ok_or(UsageError::MissingValue("migrate action"))?
                .to_string();
            if !matches!(action.as_str(), "status" | "plan" | "apply") {
                return Err(UsageError::UnknownCommand(action));
            }
            parse_path_option(words, "--config").map(|config| Command::Migrate { action, config })
        }
        Some(other) => Err(UsageError::UnknownCommand(other.to_string())),
        None => Err(UsageError::UnknownCommand(
            first.to_string_lossy().into_owned(),
        )),
    }
}

fn parse_path_option<'a>(
    mut options: impl Iterator<Item = &'a OsString>,
    flag: &'static str,
) -> Result<Option<PathBuf>, UsageError> {
    let mut value = None;
    while let Some(option) = options.next() {
        match option.to_str() {
            Some(found) if found == flag => {
                let next = options.next().ok_or(UsageError::MissingValue(flag))?;
                value = Some(PathBuf::from(next));
            }
            _ => {
                return Err(UsageError::UnknownOption(
                    option.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(value)
}

fn parse_healthcheck<'a>(
    mut options: impl Iterator<Item = &'a OsString>,
) -> Result<Command, UsageError> {
    let mut address = None;
    let mut signal = None;
    let mut ca_file = None;
    let mut server_name = None;
    while let Some(option) = options.next() {
        match option.to_str() {
            Some("--address") => {
                let value = options
                    .next()
                    .ok_or(UsageError::MissingValue("--address"))?;
                address = value
                    .to_str()
                    .and_then(|text| text.parse::<SocketAddr>().ok());
                if address.is_none() {
                    return Err(UsageError::UnknownOption("--address".to_string()));
                }
            }
            Some("--signal") => {
                let value = options.next().ok_or(UsageError::MissingValue("--signal"))?;
                signal = Some(value.to_string_lossy().into_owned());
            }
            Some("--ca-file") => {
                let value = options
                    .next()
                    .ok_or(UsageError::MissingValue("--ca-file"))?;
                ca_file = Some(PathBuf::from(value));
            }
            Some("--server-name") => {
                let value = options
                    .next()
                    .ok_or(UsageError::MissingValue("--server-name"))?;
                server_name = Some(value.to_string_lossy().into_owned());
            }
            _ => {
                return Err(UsageError::UnknownOption(
                    option.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(Command::Healthcheck {
        address,
        signal,
        ca_file,
        server_name,
    })
}
