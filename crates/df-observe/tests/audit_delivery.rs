#![cfg(not(target_arch = "wasm32"))]
//! Audit lines that find no sink are counted, never printed. Its own test binary, so no other
//! test has installed the process-wide sink first.
use std::sync::Mutex;

static DELIVERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn sink(event: &'static str, line: String) {
    if event == "test.reentrant" {
        // A sink that records again must not recurse; the nested line is counted instead.
        df_observe::record_audit_line("test.nested", "TEST nested".to_owned());
    }
    DELIVERED.lock().unwrap().push(line);
}

fn other_sink(_event: &'static str, _line: String) {}

#[test]
fn lines_without_a_sink_are_counted_and_the_first_sink_keeps_the_lines() {
    let before = df_observe::undelivered_audit_lines();
    df_observe::record_audit_line("test.early", "TEST early".to_owned());
    assert_eq!(df_observe::undelivered_audit_lines(), before + 1);

    assert!(df_observe::install_audit_sink(sink));
    assert!(
        !df_observe::install_audit_sink(other_sink),
        "a later install keeps the first sink"
    );
    df_observe::record_audit_line("test.delivered", "TEST delivered".to_owned());
    assert_eq!(df_observe::undelivered_audit_lines(), before + 1);
    df_observe::record_audit_line("test.reentrant", "TEST reentrant".to_owned());
    assert_eq!(df_observe::undelivered_audit_lines(), before + 2);

    let delivered = DELIVERED.lock().unwrap();
    assert_eq!(delivered.len(), 2, "{delivered:?}");
    assert!(delivered[0].starts_with("TEST delivered"), "{delivered:?}");
    assert!(delivered[1].starts_with("TEST reentrant"), "{delivered:?}");
}
