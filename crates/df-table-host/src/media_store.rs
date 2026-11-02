//! Writes to the media library: an add is staged into a uniquely named temporary file as it
//! arrives, checked, and then published under the library's write lock, where the room check and
//! the rename are one step. A reader never sees half a file, two adds of one name never share a
//! temporary file, and the file limit holds however many adds run at once. It never reads the
//! library for serving.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::body::Bytes;
use tokio::sync::mpsc::Receiver;

use crate::media_ingest::{IngestRefusal, MediaLimits, UploadKind, check_content, check_room};
use crate::media_library::media_entries;
use crate::media_upload::EnvelopeParser;
use crate::playground::{FileProblem, problem_from};

/// Held while the library's files are counted and a new one is renamed into place. One lock for
/// the process: the library is one directory, and the dev API is a loopback tool.
static LIBRARY_WRITE: Mutex<()> = Mutex::new(());
static NEXT_STAGED_FILE: AtomicU64 = AtomicU64::new(0);

/// Why an add did not finish.
#[derive(Debug)]
pub(crate) enum AddFailure {
    Refused(IngestRefusal),
    File(FileProblem),
}

/// A file that was added.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Added {
    pub(crate) name: String,
    pub(crate) file_bytes: u64,
}

/// A temporary file in the library directory, removed unless it was published. Its name starts
/// with a dot, so the library never lists it.
struct StagedFile {
    path: PathBuf,
    published: bool,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.published {
            // Failure to remove is irrelevant: the file is ours and unlisted, and is only disk space.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn stage(directory: &Path) -> Result<(File, StagedFile), FileProblem> {
    std::fs::create_dir_all(directory).map_err(|error| problem_from(&error))?;
    let number = NEXT_STAGED_FILE.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(".upload-{}-{number}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| problem_from(&error))?;
    Ok((
        file,
        StagedFile {
            path,
            published: false,
        },
    ))
}

/// Reads one add request from `chunks` (blocking), stages the file and publishes it. It stops
/// reading at the first refusal: the sender sees the channel close.
pub(crate) fn ingest_upload(
    mut chunks: Receiver<Bytes>,
    directory: &Path,
    limits: &MediaLimits,
) -> Result<Added, AddFailure> {
    let (mut file, mut staged) = stage(directory).map_err(AddFailure::File)?;
    let mut parser = EnvelopeParser::new(limits);
    while let Some(chunk) = chunks.blocking_recv() {
        parser
            .feed(&chunk, &mut file)
            .map_err(AddFailure::Refused)?;
    }
    let envelope = parser.finish().map_err(AddFailure::Refused)?;
    let upload = UploadKind::from_name(&envelope.name).ok_or(AddFailure::Refused(
        IngestRefusal::NotAMediaName {
            name: envelope.name.clone(),
        },
    ))?;
    check_content(&mut file, upload, limits).map_err(AddFailure::Refused)?;
    drop(file);
    publish(directory, &staged.path, &envelope.name, limits)?;
    staged.published = true;
    Ok(Added {
        name: envelope.name,
        file_bytes: envelope.file_bytes,
    })
}

/// Renames the staged file to `name` if the library has room, under the write lock.
fn publish(
    directory: &Path,
    staged: &Path,
    name: &str,
    limits: &MediaLimits,
) -> Result<(), AddFailure> {
    let _writing = LIBRARY_WRITE.lock().unwrap_or_else(PoisonError::into_inner);
    let existing = media_entries(directory).map_err(AddFailure::File)?;
    check_room(&existing, name, limits).map_err(AddFailure::Refused)?;
    std::fs::rename(staged, directory.join(name))
        .map_err(|error| AddFailure::File(problem_from(&error)))
}
