//! The command line: the only other place besides `environment.rs` that touches the process
//! environment, and only to read the argument list.

use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    /// `config check --config <path>`: load and validate, print the effective listing.
    ConfigCheck { config: Option<PathBuf> },
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
    let second = words.next();
    match (first.to_str(), second.and_then(|word| word.to_str())) {
        (Some("config"), Some("check")) => parse_config_check(words),
        (Some(other), _) => Err(UsageError::UnknownCommand(other.to_string())),
        (None, _) => Err(UsageError::UnknownCommand(
            first.to_string_lossy().into_owned(),
        )),
    }
}

fn parse_config_check<'a>(
    mut options: impl Iterator<Item = &'a OsString>,
) -> Result<Command, UsageError> {
    let mut config = None;
    while let Some(option) = options.next() {
        match option.to_str() {
            Some("--config") => {
                let value = options.next().ok_or(UsageError::MissingValue("--config"))?;
                config = Some(PathBuf::from(value));
            }
            _ => {
                return Err(UsageError::UnknownOption(
                    option.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(Command::ConfigCheck { config })
}
