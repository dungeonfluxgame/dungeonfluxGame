use std::io::{ErrorKind, Write};
use std::sync::{Arc, Mutex};

use df_table::element::ElementId;
use df_table::event::TableEvent;
use df_table::ports::EventSink;

use crate::event_log::EventLog;
use crate::observe::{EventLevel, HostEventSink, WallClock};
use crate::sink::{LineEcho, TerminalWriter};

fn moved() -> TableEvent {
    TableEvent::ElementMoved {
        element: ElementId::new("pip").expect("valid"),
        revision: 2,
    }
}

fn cleared() -> TableEvent {
    TableEvent::CanvasCleared {
        removed: 3,
        revision: 4,
    }
}

fn texts(log: &EventLog) -> Vec<String> {
    log.newest(100).into_iter().map(|line| line.text).collect()
}

/// The canvas's sink as `serve` builds it, at `level`.
fn canvas_sink(log: &Arc<EventLog>, level: EventLevel) -> HostEventSink {
    HostEventSink::new(Arc::clone(log), Arc::new(WallClock)).with_level(level)
}

#[test]
fn an_event_is_kept_as_its_rendered_audit_line() {
    let log = Arc::new(EventLog::default());
    let sink = canvas_sink(&log, EventLevel::Always);
    sink.record(&cleared());
    assert_eq!(texts(&log), vec!["CANVAS clear removed=3 revision=4"]);
}

#[test]
fn moves_are_left_out_and_counted_unless_the_host_was_asked_to_record_them() {
    let quiet_log = Arc::new(EventLog::default());
    let quiet = canvas_sink(&quiet_log, EventLevel::Always);
    quiet.record(&moved());
    assert!(texts(&quiet_log).is_empty());
    assert_eq!(quiet.health().filtered_verbose, 1);
    let loud_log = Arc::new(EventLog::default());
    let loud = canvas_sink(&loud_log, EventLevel::Verbose);
    loud.record(&moved());
    assert_eq!(texts(&loud_log), vec!["CANVAS move element=pip revision=2"]);
    assert_eq!(loud.health().filtered_verbose, 0);
}

fn collecting_echo(log: &EventLog) -> (LineEcho, Arc<Mutex<Vec<String>>>) {
    let printed = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&printed);
    let echo = LineEcho::starting_after(log, move |line| {
        writer.lock().expect("not poisoned").push(line.to_owned());
    });
    (echo, printed)
}

/// The lines written so far, once at least `count` are there (the writer is another thread).
fn written(printed: &Arc<Mutex<Vec<String>>>, count: usize) -> Vec<String> {
    for _ in 0..500 {
        let lines = printed.lock().expect("not poisoned").clone();
        if lines.len() >= count {
            return lines;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    printed.lock().expect("not poisoned").clone()
}

#[test]
fn the_echo_prints_each_new_line_once_and_in_order() {
    let log = Arc::new(EventLog::default());
    let sink = canvas_sink(&log, EventLevel::Verbose);
    let (echo, printed) = collecting_echo(&log);
    sink.record(&cleared());
    sink.record(&moved());
    echo.echo_new(&log);
    echo.echo_new(&log);
    assert_eq!(
        written(&printed, 2),
        vec![
            "CANVAS clear removed=3 revision=4",
            "CANVAS move element=pip revision=2"
        ]
    );
    sink.record(&cleared());
    echo.echo_new(&log);
    assert_eq!(written(&printed, 3).len(), 3);
}

#[test]
fn the_echo_skips_lines_logged_before_it_was_made() {
    let log = Arc::new(EventLog::default());
    let sink = canvas_sink(&log, EventLevel::Verbose);
    sink.record(&cleared());
    let (echo, printed) = collecting_echo(&log);
    echo.echo_new(&log);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(printed.lock().expect("not poisoned").is_empty());
}

/// A terminal that refuses every write, like a redirected log whose file was closed.
struct ClosedTerminal;

impl Write for ClosedTerminal {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(ErrorKind::BrokenPipe.into())
    }
}

fn lines_starting(log: &EventLog, prefix: &str) -> Vec<String> {
    texts(log)
        .into_iter()
        .filter(|text| text.starts_with(prefix))
        .collect()
}

#[test]
fn a_failed_terminal_write_is_recorded_once_and_every_line_is_kept_in_the_fallback_file() {
    let directory = tempfile::TempDir::new().expect("a scratch directory");
    let fallback = directory.path().join("terminal.log");
    let log = Arc::new(EventLog::default());
    let mut writer = TerminalWriter::new(Arc::clone(&log), fallback.clone());
    writer.write_line_to(&mut ClosedTerminal, "CANVAS first");
    writer.write_line_to(&mut ClosedTerminal, "CANVAS second");
    assert_eq!(
        lines_starting(&log, "HOST"),
        ["HOST log_sink_failed kind=BrokenPipe"]
    );
    assert_eq!(
        std::fs::read_to_string(fallback).expect("the fallback file"),
        "CANVAS first
CANVAS second
"
    );
}

#[test]
fn a_terminal_that_takes_its_lines_records_nothing_and_opens_no_file() {
    let directory = tempfile::TempDir::new().expect("a scratch directory");
    let fallback = directory.path().join("terminal.log");
    let log = Arc::new(EventLog::default());
    let mut writer = TerminalWriter::new(Arc::clone(&log), fallback.clone());
    let mut terminal = Vec::new();
    writer.write_line_to(&mut terminal, "CANVAS first");
    assert_eq!(
        terminal,
        b"CANVAS first
"
    );
    assert!(texts(&log).is_empty());
    assert!(!fallback.exists());
}

#[test]
fn a_fallback_file_that_cannot_be_opened_is_recorded_once_too() {
    let directory = tempfile::TempDir::new().expect("a scratch directory");
    let fallback = directory.path().join("missing").join("terminal.log");
    let log = Arc::new(EventLog::default());
    let mut writer = TerminalWriter::new(Arc::clone(&log), fallback);
    writer.write_line_to(&mut ClosedTerminal, "CANVAS first");
    writer.write_line_to(&mut ClosedTerminal, "CANVAS second");
    assert_eq!(
        lines_starting(&log, "HOST"),
        [
            "HOST log_sink_failed kind=BrokenPipe",
            "HOST log_fallback_failed kind=NotFound"
        ]
    );
}
