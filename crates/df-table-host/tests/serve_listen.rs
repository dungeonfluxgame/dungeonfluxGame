//! `serve` drains and stops the telemetry store it composed even when it cannot listen, so the
//! records written before the failure reach the store. Its own test binary, because `run_serve`
//! makes its log the process's audit sink.

use std::sync::Mutex;

use df_table_host::options::ServeOptions;
use df_table_host::serve::{ServeError, run_serve};
use df_telemetry::{DiagnosticReader, QueryFilter};

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_in_use_still_drains_the_records_written_before_it_to_the_store() {
    let directory = tempfile::TempDir::new().expect("a directory");
    let root = directory.path().join("telemetry");
    let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port to take");
    let port = taken.local_addr().expect("its address").port();
    let options = ServeOptions {
        port,
        // No kind directory, so serving records one `reload_rejected` before it listens.
        kind_directory: directory.path().join("no-kinds"),
        playground_directory: directory.path().join("playground"),
        package_directory: directory.path().join("pkg"),
        telemetry_root: Some(root.clone()),
        records_moves: false,
        echoes_lines: false,
    };
    let announced = Mutex::new(Vec::new());

    let served = run_serve(options, |line| {
        announced.lock().expect("the lock").push(line.to_owned());
    })
    .await;

    assert!(
        matches!(served, Err(ServeError::Listen { port: refused, .. }) if refused == port),
        "{served:?}"
    );
    let announced = announced.into_inner().expect("the lock");
    assert!(
        !announced
            .iter()
            .any(|line| line.contains("telemetry store")),
        "the store confirmed every record: {announced:?}"
    );
    let mut reader = DiagnosticReader::open(&root).expect("a read-only reader");
    let page = reader
        .query(&QueryFilter::default(), 0, 100)
        .expect("a page");
    assert_eq!(page.records.len(), 1);
    assert!(contains(&page.records[0].otlp, "reload_rejected errors=1"));
    drop(taken);
}
