//! A host log does not become the audit sink when another df-observe sink already is. Its own
//! test binary, so the foreign sink is installed before any host log.
use std::sync::{Arc, Mutex};

use df_table_host::event_log::{EventLog, install_as_audit_sink};

static FOREIGN_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn foreign_sink(_event: &'static str, line: String) {
    FOREIGN_LINES.lock().unwrap().push(line);
}

#[test]
fn a_host_log_is_not_the_sink_when_another_sink_was_installed_first() {
    assert!(df_observe::install_audit_sink(foreign_sink));
    let log = Arc::new(EventLog::default());
    assert!(
        !install_as_audit_sink(&log),
        "the foreign sink keeps the lines"
    );
    df_observe::record_audit_line("TEST", "TEST probe key=1".to_owned());
    assert!(log.newest(10).is_empty(), "{:?}", log.newest(10));
    let foreign = FOREIGN_LINES.lock().unwrap();
    assert_eq!(foreign.len(), 1, "{foreign:?}");
    assert!(foreign[0].starts_with("TEST probe key=1"), "{foreign:?}");
}
