//! The `file_unreadable` line of each kind of playground file.

use std::io::ErrorKind;

use super::{FileError, ServedFileKind, record_unreadable};
use crate::test_support::{STARTER_FILES, start};

fn playground_lines(running: &crate::test_support::Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(1_000)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PLAYGROUND "))
        .collect()
}

#[tokio::test]
async fn a_file_that_cannot_be_read_writes_one_line_for_each_kind() {
    let running = start(&STARTER_FILES).await;
    for kind in ServedFileKind::ALL {
        record_unreadable(
            &running.state,
            kind,
            Some("index.html"),
            FileError::Io(ErrorKind::PermissionDenied),
        );
    }
    record_unreadable(
        &running.state,
        ServedFileKind::Fixture,
        Some("../escape.json"),
        FileError::Stopped,
    );
    let expected: Vec<String> = ServedFileKind::ALL
        .into_iter()
        .map(|kind| {
            format!(
                "PLAYGROUND file_unreadable kind={} name=index.html status=500",
                kind.key()
            )
        })
        .chain(["PLAYGROUND file_unreadable kind=fixture name=- status=500".to_owned()])
        .collect();
    assert_eq!(playground_lines(&running), expected);
}

#[tokio::test]
async fn a_missing_or_too_large_file_but_the_package_writes_no_line() {
    let running = start(&STARTER_FILES).await;
    for kind in ServedFileKind::ALL {
        if kind == ServedFileKind::Package {
            continue;
        }
        record_unreadable(&running.state, kind, Some("a.json"), FileError::NotFound);
        record_unreadable(
            &running.state,
            kind,
            Some("a.json"),
            FileError::TooLarge { maximum_bytes: 1 },
        );
    }
    assert_eq!(playground_lines(&running), Vec::<String>::new());
}
