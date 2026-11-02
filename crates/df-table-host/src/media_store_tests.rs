//! Adding files to the library: staged, checked and published one at a time.

use std::path::Path;
use std::sync::{Arc, Barrier};

use axum::body::Bytes;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tempfile::TempDir;

use crate::media_ingest::{IngestRefusal, MediaLimits};
use crate::media_library::media_entries;
use crate::media_store::{AddFailure, Added, ingest_upload};
use crate::video_fixtures::{mp4_of_seconds, webm_of_seconds};
use crate::wire::MediaAdd;

fn request_for(name: &str, bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&MediaAdd {
        name: name.to_owned(),
        data_base64: STANDARD.encode(bytes),
    })
    .expect("serialises")
}

/// Sends `request` in pieces to a thread running `ingest_upload`. The count is how many pieces
/// the thread took before it stopped reading.
fn ingest(
    directory: &Path,
    request: &[u8],
    piece: usize,
    limits: MediaLimits,
) -> (Result<Added, AddFailure>, usize) {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(2);
    let directory = directory.to_path_buf();
    let worker = std::thread::spawn(move || ingest_upload(receiver, &directory, &limits));
    let mut taken = 0;
    for part in request.chunks(piece) {
        if sender.blocking_send(Bytes::copy_from_slice(part)).is_err() {
            break;
        }
        taken += 1;
    }
    drop(sender);
    (worker.join().expect("the thread finishes"), taken)
}

fn names_on_disk(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("lists the directory")
        .map(|entry| {
            entry
                .expect("reads an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn refusal(result: Result<Added, AddFailure>) -> IngestRefusal {
    match result {
        Err(AddFailure::Refused(refusal)) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_picture_a_sound_and_both_videos_are_stored_under_their_names_and_nothing_else_is_left() {
    let directory = TempDir::new().expect("makes a directory");
    for (name, bytes) in [
        ("still.png", b"a picture".to_vec()),
        ("bell.wav", b"RIFF a sound".to_vec()),
        ("hearth.mp4", mp4_of_seconds(3)),
        ("rain.webm", webm_of_seconds(4)),
    ] {
        let (result, _) = ingest(
            directory.path(),
            &request_for(name, &bytes),
            1000,
            MediaLimits::DEFAULT,
        );
        let added = result.expect("is added");
        assert_eq!(added.name, name);
        assert_eq!(added.file_bytes, bytes.len() as u64);
        assert_eq!(
            std::fs::read(directory.path().join(name)).expect("reads"),
            bytes
        );
    }
    assert_eq!(
        names_on_disk(directory.path()),
        ["bell.wav", "hearth.mp4", "rain.webm", "still.png"]
    );
}

#[test]
fn a_refused_file_leaves_no_staged_file_behind() {
    let directory = TempDir::new().expect("makes a directory");
    let cases = [
        (request_for("note.mp4", b"words"), "a video check"),
        (request_for("a.png", b""), "an empty file"),
        (
            br#"{"name":"a.png","data_base64":"aGk"#.to_vec(),
            "a cut request",
        ),
        (
            br#"{"name":"a.png","data_base64":"!!!!"}"#.to_vec(),
            "bad base64",
        ),
    ];
    for (request, what) in cases {
        let (result, _) = ingest(directory.path(), &request, 7, MediaLimits::DEFAULT);
        assert!(result.is_err(), "{what}");
    }
    assert_eq!(names_on_disk(directory.path()), Vec::<String>::new());
}

#[test]
fn an_empty_file_and_a_cut_request_are_refused_with_their_reasons() {
    let directory = TempDir::new().expect("makes a directory");
    let (empty, _) = ingest(
        directory.path(),
        &request_for("a.png", b""),
        1000,
        MediaLimits::DEFAULT,
    );
    assert_eq!(refusal(empty), IngestRefusal::Empty);
    let (cut, _) = ingest(
        directory.path(),
        br#"{"name":"a.png","data_base64":"aGk"#,
        1000,
        MediaLimits::DEFAULT,
    );
    assert!(matches!(refusal(cut), IngestRefusal::MalformedRequest(_)));
}

#[test]
fn a_request_over_the_limit_stops_being_read_when_the_limit_is_crossed() {
    let directory = TempDir::new().expect("makes a directory");
    let limits = MediaLimits {
        maximum_file_bytes: 100,
        ..MediaLimits::DEFAULT
    };
    let request = request_for("a.png", &[9_u8; 100_000]);
    let pieces = request.len().div_ceil(64);
    let (result, taken) = ingest(directory.path(), &request, 64, limits);
    assert_eq!(
        refusal(result),
        IngestRefusal::TooLarge { maximum_bytes: 100 }
    );
    assert!(taken < 20, "took {taken} of {pieces} pieces");
    assert_eq!(names_on_disk(directory.path()), Vec::<String>::new());
}

#[test]
fn a_file_of_exactly_the_size_limit_is_stored_and_one_byte_more_is_refused() {
    let directory = TempDir::new().expect("makes a directory");
    let limits = MediaLimits {
        maximum_file_bytes: 1000,
        ..MediaLimits::DEFAULT
    };
    let (at_limit, _) = ingest(
        directory.path(),
        &request_for("a.png", &[1; 1000]),
        333,
        limits,
    );
    assert!(at_limit.is_ok());
    let (over, _) = ingest(
        directory.path(),
        &request_for("b.png", &[1; 1001]),
        333,
        limits,
    );
    assert_eq!(
        refusal(over),
        IngestRefusal::TooLarge {
            maximum_bytes: 1000
        }
    );
}

#[test]
fn the_library_refuses_a_new_name_at_the_file_limit_but_replaces_an_existing_one() {
    let directory = TempDir::new().expect("makes a directory");
    let limits = MediaLimits {
        maximum_files: 2,
        ..MediaLimits::DEFAULT
    };
    for name in ["a.png", "b.png"] {
        let (result, _) = ingest(directory.path(), &request_for(name, b"first"), 100, limits);
        assert!(result.is_ok(), "{name}");
    }
    let (third, _) = ingest(
        directory.path(),
        &request_for("c.png", b"third"),
        100,
        limits,
    );
    assert_eq!(
        refusal(third),
        IngestRefusal::LibraryFull { maximum_files: 2 }
    );
    let (replaced, _) = ingest(
        directory.path(),
        &request_for("a.png", b"second"),
        100,
        limits,
    );
    assert!(replaced.is_ok());
    assert_eq!(
        std::fs::read(directory.path().join("a.png")).expect("reads"),
        b"second"
    );
    assert_eq!(names_on_disk(directory.path()), ["a.png", "b.png"]);
}

#[test]
fn many_adds_at_once_leave_the_library_exactly_at_its_file_limit() {
    for round in 0..5 {
        let directory = TempDir::new().expect("makes a directory");
        let limits = MediaLimits {
            maximum_files: 3,
            ..MediaLimits::DEFAULT
        };
        for name in ["one.png", "two.png"] {
            let (result, _) = ingest(directory.path(), &request_for(name, b"x"), 100, limits);
            assert!(result.is_ok());
        }
        let adders = 16;
        let start = Arc::new(Barrier::new(adders));
        let threads: Vec<_> = (0..adders)
            .map(|index| {
                let start = Arc::clone(&start);
                let directory = directory.path().to_path_buf();
                std::thread::spawn(move || {
                    let request = request_for(&format!("new{index}.png"), b"payload");
                    start.wait();
                    ingest(&directory, &request, 100, limits).0
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().expect("finishes"))
            .collect();
        let stored = results.iter().filter(|result| result.is_ok()).count();
        assert_eq!(stored, 1, "round {round}");
        for result in results.into_iter().filter(Result::is_err) {
            assert_eq!(
                refusal(result),
                IngestRefusal::LibraryFull { maximum_files: 3 }
            );
        }
        assert_eq!(
            media_entries(directory.path()).expect("lists").len(),
            3,
            "round {round}"
        );
        assert_eq!(names_on_disk(directory.path()).len(), 3, "round {round}");
    }
}

#[test]
fn two_adds_of_one_name_at_once_never_share_a_staged_file_and_leave_one_whole_upload() {
    for round in 0..5 {
        let directory = TempDir::new().expect("makes a directory");
        let adders = 8_u8;
        let start = Arc::new(Barrier::new(usize::from(adders)));
        let threads: Vec<_> = (0..adders)
            .map(|index| {
                let start = Arc::clone(&start);
                let directory = directory.path().to_path_buf();
                std::thread::spawn(move || {
                    let request = request_for("clip.png", &vec![index; 30_000]);
                    start.wait();
                    ingest(&directory, &request, 997, MediaLimits::DEFAULT).0
                })
            })
            .collect();
        for thread in threads {
            thread
                .join()
                .expect("finishes")
                .expect("every add of the name succeeds");
        }
        let stored = std::fs::read(directory.path().join("clip.png")).expect("reads");
        assert_eq!(stored.len(), 30_000, "round {round}");
        assert!(
            stored.iter().all(|&byte| byte == stored[0]),
            "round {round}: the file mixes two uploads"
        );
        assert_eq!(
            names_on_disk(directory.path()),
            ["clip.png"],
            "round {round}"
        );
    }
}
