//! `dev repl`: commands read from standard input, one per line, run against the host without
//! starting a process for each.
//!
//! A new process costs tens of milliseconds, which is what limits how fast a caller, a person or a
//! model, can move things on the canvas; a command here costs about a millisecond. It owns the
//! line loop and what it prints. Each line is an ordinary `dev` command, so everything a command
//! can do, this can do, except `script` and `repl`, which would hide where a failure came from.

use std::io::BufRead;

use tokio::sync::mpsc;

use crate::command::{self, Command, CommandError};
use crate::dev::run_dev_command;
use crate::serve::write_terminal_line;

/// A line that ends the session.
pub const QUIT_WORDS: [&str; 2] = ["quit", "exit"];

/// What one line did.
#[derive(Debug, Clone, PartialEq)]
pub enum LineResult {
    /// Nothing to run: a blank line or a comment.
    Skipped,
    Quit,
    Ok(String),
    Failed(String),
}

/// Runs one line of the session.
pub async fn run_line(port: u16, line: &str) -> LineResult {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return LineResult::Skipped;
    }
    if QUIT_WORDS.contains(&line) {
        return LineResult::Quit;
    }
    let words = match command::split_words(line) {
        Ok(words) => words,
        Err(error) => return LineResult::Failed(error.to_string()),
    };
    let command = match command::parse(&words) {
        Ok(Command::Script(_)) => {
            return LineResult::Failed(CommandError::NotAllowedInScript("script").to_string());
        }
        Ok(Command::Repl) => {
            return LineResult::Failed(CommandError::NotAllowedInScript("repl").to_string());
        }
        Ok(command) => command,
        Err(error) => return LineResult::Failed(error.to_string()),
    };
    match run_dev_command(port, command).await {
        Ok(text) => LineResult::Ok(text),
        Err(error) => LineResult::Failed(error.to_string()),
    }
}

/// Runs lines from `lines` until they end or one says `quit`, writing each answer with `out` and
/// each failure with `err`. A failure does not end the session. Returns how many lines failed.
pub async fn run_lines(
    port: u16,
    mut lines: mpsc::UnboundedReceiver<String>,
    out: impl Fn(&str),
    err: impl Fn(&str),
) -> usize {
    let mut failures = 0;
    while let Some(line) = lines.recv().await {
        match run_line(port, &line).await {
            LineResult::Skipped => {}
            LineResult::Quit => break,
            LineResult::Ok(text) => out(&text),
            LineResult::Failed(text) => {
                failures += 1;
                err(&text);
            }
        }
    }
    failures
}

/// Reads `input` on a separate thread, because reading blocks, and runs its lines.
pub async fn run_repl(port: u16, input: impl BufRead + Send + 'static) -> usize {
    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    run_lines(port, receiver, write_terminal_line, write_error_line).await
}

fn write_error_line(text: &str) {
    use std::io::Write;
    // A closed stderr leaves nothing else to report to.
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}
