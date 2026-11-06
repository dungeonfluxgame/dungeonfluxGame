//! Answering a request from one open file: consistency under replacement, `HEAD` and streaming.

use std::fs::{File, OpenOptions};
use std::io::Write;

use tempfile::TempDir;

use super::{Conditions, Payload, Served, entity_tag, serve_file};
use crate::media_ingest::MediaLimits;
use crate::playground::FileError;

const CHUNK_BYTES: usize = 256 * 1024;

fn write_file(directory: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = directory.path().join(name);
    std::fs::write(&path, bytes).expect("writes");
    path
}

/// What a new add does: a new file takes the old one's name.
fn replace_file(directory: &TempDir, name: &str, bytes: &[u8]) {
    let staged = write_file(directory, ".staged.tmp", bytes);
    std::fs::rename(staged, directory.path().join(name)).expect("replaces");
}

fn range(text: &str) -> Conditions {
    Conditions {
        range: Some(text.to_owned()),
        ..Conditions::default()
    }
}

async fn body_of(payload: Payload) -> std::io::Result<Vec<u8>> {
    let response = payload.into_response(axum::http::StatusCode::OK);
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(std::io::Error::other)
}

fn expect_part(served: Result<Served, FileError>) -> (Payload, String, u64, u64, u64) {
    match served.expect("is served") {
        Served::Part {
            payload,
            tag,
            span,
            file_bytes,
        } => (payload, tag, span.start, span.end_inclusive, file_bytes),
        _ => panic!("expected a partial answer"),
    }
}

#[tokio::test]
async fn a_file_replaced_after_it_was_opened_is_answered_wholly_from_the_version_opened() {
    let directory = TempDir::new().expect("makes a directory");
    let path = write_file(&directory, "a.mp4", &[b'o'; 40]);
    let opened = File::open(&path).expect("opens");
    let opened_tag = entity_tag(&opened.metadata().expect("measures")).expect("a tag");
    replace_file(&directory, "a.mp4", &[b'n'; 5]);

    let served = serve_file(opened, &range("bytes=30-"), false, &MediaLimits::DEFAULT);
    let (payload, tag, start, end, file_bytes) = expect_part(served);
    assert_eq!((start, end, file_bytes), (30, 39, 40));
    assert_eq!(tag, opened_tag);
    assert_eq!(body_of(payload).await.expect("reads"), vec![b'o'; 10]);

    let fresh = serve_file(
        File::open(&path).expect("opens"),
        &range("bytes=0-"),
        false,
        &MediaLimits::DEFAULT,
    );
    let (payload, fresh_tag, start, end, file_bytes) = expect_part(fresh);
    assert_eq!((start, end, file_bytes), (0, 4, 5));
    assert_ne!(fresh_tag, opened_tag);
    assert_eq!(body_of(payload).await.expect("reads"), vec![b'n'; 5]);
}

#[test]
fn a_head_reads_no_bytes_it_needs_only_the_metadata() {
    let directory = TempDir::new().expect("makes a directory");
    let path = directory.path().join("big.mp4");
    let file = File::create(&path).expect("creates");
    file.set_len(40 * 1024 * 1024).expect("sizes");
    drop(file);
    // A handle that cannot be read from: any read would fail.
    let write_only = OpenOptions::new().write(true).open(&path).expect("opens");
    let served = serve_file(
        write_only,
        &Conditions::default(),
        true,
        &MediaLimits::DEFAULT,
    );
    match served.expect("is served") {
        Served::File {
            payload: Payload::Withheld { length },
            ..
        } => assert_eq!(length, 40 * 1024 * 1024),
        _ => panic!("expected the length of the whole file, withheld"),
    }
}

#[test]
fn a_file_over_the_size_limit_is_refused_from_its_length() {
    let directory = TempDir::new().expect("makes a directory");
    let path = write_file(&directory, "a.mp4", &[0; 100]);
    let limits = MediaLimits {
        maximum_file_bytes: 99,
        ..MediaLimits::DEFAULT
    };
    let served = serve_file(
        File::open(&path).expect("opens"),
        &Conditions::default(),
        false,
        &limits,
    );
    assert!(matches!(served, Err(FileError::TooLarge { .. })));
    let at_limit = MediaLimits {
        maximum_file_bytes: 100,
        ..MediaLimits::DEFAULT
    };
    let served = serve_file(
        File::open(&path).expect("opens"),
        &Conditions::default(),
        false,
        &at_limit,
    );
    assert!(served.is_ok());
}

#[tokio::test]
async fn a_file_larger_than_a_chunk_is_streamed_whole_and_in_order() {
    let directory = TempDir::new().expect("makes a directory");
    let bytes: Vec<u8> = (0..=250_u8).cycle().take(3 * CHUNK_BYTES + 17).collect();
    let path = write_file(&directory, "a.mp4", &bytes);
    let served = serve_file(
        File::open(&path).expect("opens"),
        &Conditions::default(),
        false,
        &MediaLimits::DEFAULT,
    )
    .expect("is served");
    let Served::File { payload, .. } = served else {
        panic!("expected the whole file");
    };
    assert_eq!(body_of(payload).await.expect("reads"), bytes);
}

#[tokio::test]
async fn a_range_inside_a_large_file_streams_only_its_bytes() {
    let directory = TempDir::new().expect("makes a directory");
    let bytes: Vec<u8> = (0..=250_u8).cycle().take(3 * CHUNK_BYTES).collect();
    let path = write_file(&directory, "a.mp4", &bytes);
    let start = CHUNK_BYTES - 5;
    let end = 2 * CHUNK_BYTES + 5;
    let served = serve_file(
        File::open(&path).expect("opens"),
        &range(&format!("bytes={start}-{end}")),
        false,
        &MediaLimits::DEFAULT,
    );
    let (payload, _, first, last, _) = expect_part(served);
    assert_eq!((first, last), (start as u64, end as u64));
    assert_eq!(body_of(payload).await.expect("reads"), bytes[start..=end]);
}

#[tokio::test]
async fn a_file_cut_short_in_place_after_it_was_measured_ends_the_answer_with_an_error() {
    let directory = TempDir::new().expect("makes a directory");
    let path = write_file(&directory, "a.mp4", &vec![1; 2 * CHUNK_BYTES]);
    let served = serve_file(
        File::open(&path).expect("opens"),
        &Conditions::default(),
        false,
        &MediaLimits::DEFAULT,
    )
    .expect("is served");
    let Served::File { payload, .. } = served else {
        panic!("expected the whole file");
    };
    let mut writer = OpenOptions::new().write(true).open(&path).expect("opens");
    writer.set_len(10).expect("cuts");
    writer.flush().expect("flushes");
    assert!(body_of(payload).await.is_err());
}
