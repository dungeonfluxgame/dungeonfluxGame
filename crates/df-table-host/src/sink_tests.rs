use std::sync::{Arc, Mutex};

use df_table::element::ElementId;
use df_table::event::TableEvent;
use df_table::ports::EventSink;

use crate::event_log::EventLog;
use crate::sink::{HostSink, LineEcho};

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

#[test]
fn an_event_is_kept_as_its_rendered_audit_line() {
    let log = Arc::new(EventLog::default());
    let sink = HostSink::new(Arc::clone(&log), false);
    sink.record(&cleared());
    assert_eq!(texts(&log), vec!["CANVAS clear removed=3 revision=4"]);
}

#[test]
fn moves_are_dropped_unless_the_host_was_asked_to_record_them() {
    let quiet_log = Arc::new(EventLog::default());
    HostSink::new(Arc::clone(&quiet_log), false).record(&moved());
    assert!(texts(&quiet_log).is_empty());
    let loud_log = Arc::new(EventLog::default());
    HostSink::new(Arc::clone(&loud_log), true).record(&moved());
    assert_eq!(texts(&loud_log), vec!["CANVAS move element=pip revision=2"]);
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
    let sink = HostSink::new(Arc::clone(&log), true);
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
    let sink = HostSink::new(Arc::clone(&log), true);
    sink.record(&cleared());
    let (echo, printed) = collecting_echo(&log);
    echo.echo_new(&log);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(printed.lock().expect("not poisoned").is_empty());
}
