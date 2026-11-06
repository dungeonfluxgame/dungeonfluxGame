//! The grammar and the running of the event and settings verbs: `history`, `trace`, `replay` and
//! `config`.
//!
//! Each has the shape of the other engine verbs: words in, one typed request out, a usage line on
//! a mistake, a numbered answer. They are parsed by the same `command::parse`, so they work from
//! the repl and from a script. `cli` dispatches to the functions here; nothing in this file knows
//! what an event or a setting is beyond the shape of its request.

use crate::command::CommandError;
use crate::dev::DevError;
use crate::engine::format::observe::{format_config, format_history, format_replay, format_trace};
use crate::engine::request::{
    ClassFilter, ConfigControl, HistoryRequest, ReplayRequest, TraceOperation,
};
use crate::engine::routes::{
    AREA_PARAMETER, CLASS_PARAMETER, LIMIT_PARAMETER, OPERATION_PARAMETER, path,
};

use super::{fetch, send, whole_number, with_query};

pub const HISTORY_USAGE: &str = "history [--limit N] [--area AREA] [--class a|b]";
pub const TRACE_USAGE: &str = "trace <operation>";
pub const REPLAY_USAGE: &str = "replay <log> [--until N] [--diff]";
pub const CONFIG_USAGE: &str = "config show | config set <name> <value>";

/// `--limit N`, `--area AREA` and `--class a|b`, in any order, each once.
pub fn parse_history(words: &[String]) -> Result<HistoryRequest, CommandError> {
    let mut request = HistoryRequest::default();
    let mut remaining = words.iter();
    while let Some(flag) = remaining.next() {
        let value = remaining.next().ok_or(CommandError::Usage(HISTORY_USAGE))?;
        let taken = match flag.as_str() {
            "--limit" => request
                .limit
                .replace(usize::try_from(whole_number("the limit", value)?).unwrap_or(usize::MAX))
                .is_some(),
            "--area" => {
                if value.is_empty()
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return Err(CommandError::BadChoice {
                        key: "the area".to_owned(),
                        text: value.clone(),
                        allowed: "an audit area such as INPUT, TURN or ENGINE".to_owned(),
                    });
                }
                request.area.replace(value.clone()).is_some()
            }
            "--class" => {
                let class = match value.as_str() {
                    "a" | "A" | "always" => ClassFilter::Always,
                    "b" | "B" | "verbose" => ClassFilter::Verbose,
                    _ => {
                        return Err(CommandError::BadChoice {
                            key: "the class".to_owned(),
                            text: value.clone(),
                            allowed: "a or b".to_owned(),
                        });
                    }
                };
                request.class.replace(class).is_some()
            }
            _ => return Err(CommandError::Usage(HISTORY_USAGE)),
        };
        if taken {
            return Err(CommandError::DuplicateKey(flag.clone()));
        }
    }
    Ok(request)
}

/// One operation: the number `input --op` took, or the 32 hexadecimal digits the reports print.
pub fn parse_trace(words: &[String]) -> Result<TraceOperation, CommandError> {
    match words {
        [operation] => TraceOperation::new(operation).map_err(|_| CommandError::BadChoice {
            key: "the operation".to_owned(),
            text: operation.clone(),
            allowed: "a number from 1 or 32 hexadecimal digits".to_owned(),
        }),
        _ => Err(CommandError::Usage(TRACE_USAGE)),
    }
}

/// A log name, then `--until N` and `--diff` in any order.
pub fn parse_replay(words: &[String]) -> Result<ReplayRequest, CommandError> {
    let [log, flags @ ..] = words else {
        return Err(CommandError::Usage(REPLAY_USAGE));
    };
    if log.starts_with("--") {
        return Err(CommandError::Usage(REPLAY_USAGE));
    }
    let mut request = ReplayRequest {
        log: log.clone(),
        until: None,
        diff: false,
    };
    let mut remaining = flags.iter();
    while let Some(flag) = remaining.next() {
        match flag.as_str() {
            "--diff" => {
                if std::mem::replace(&mut request.diff, true) {
                    return Err(CommandError::DuplicateKey(flag.clone()));
                }
            }
            "--until" => {
                let value = remaining.next().ok_or(CommandError::Usage(REPLAY_USAGE))?;
                let until = whole_number("the turn to stop at", value)?;
                if request.until.replace(until).is_some() {
                    return Err(CommandError::DuplicateKey(flag.clone()));
                }
            }
            _ => return Err(CommandError::Usage(REPLAY_USAGE)),
        }
    }
    Ok(request)
}

pub fn parse_config(words: &[String]) -> Result<ConfigControl, CommandError> {
    match words {
        [verb] if verb == "show" => Ok(ConfigControl::Show),
        [verb, name, value] if verb == "set" => Ok(ConfigControl::Set {
            name: name.clone(),
            value: whole_number("the value", value)?,
        }),
        _ => Err(CommandError::Usage(CONFIG_USAGE)),
    }
}

/// The address of `history` with its query.
fn history_address(request: &HistoryRequest) -> String {
    let mut query = Vec::new();
    if let Some(limit) = request.limit {
        query.push(format!("{LIMIT_PARAMETER}={limit}"));
    }
    if let Some(area) = &request.area {
        query.push(format!("{AREA_PARAMETER}={area}"));
    }
    if let Some(class) = request.class {
        let letter = match class {
            ClassFilter::Always => "a",
            ClassFilter::Verbose => "b",
        };
        query.push(format!("{CLASS_PARAMETER}={letter}"));
    }
    if query.is_empty() {
        path::HISTORY.to_owned()
    } else {
        format!("{}?{}", path::HISTORY, query.join("&"))
    }
}

pub async fn run_history(port: u16, request: HistoryRequest) -> Result<String, DevError> {
    Ok(format_history(
        &fetch(port, &history_address(&request)).await?,
    ))
}

pub async fn run_trace(port: u16, operation: TraceOperation) -> Result<String, DevError> {
    let address = with_query(path::TRACE, OPERATION_PARAMETER, &String::from(operation));
    Ok(format_trace(&fetch(port, &address).await?))
}

pub async fn run_replay(port: u16, request: ReplayRequest) -> Result<String, DevError> {
    Ok(format_replay(&send(port, path::REPLAY, &request).await?))
}

pub async fn run_config(port: u16, control: ConfigControl) -> Result<String, DevError> {
    let report = match control {
        ConfigControl::Show => fetch(port, path::CONFIG).await?,
        set @ ConfigControl::Set { .. } => send(port, path::CONFIG, &set).await?,
    };
    Ok(format_config(&report))
}
