//! Writes to the media library: an add is staged into a uniquely named temporary file as it
//! arrives, checked, and then published under the library's write lock, where the room check and
//! the rename are one step. A reader never sees half a file, two adds of one name never share a
//! temporary file, and the file limit holds however many adds run at once. It never reads the
//! library for serving.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use axum::body::Bytes;
use axum::http::StatusCode;
use tokio::sync::mpsc::Receiver;

use crate::media_ingest::{IngestError, MediaLimits, UploadKind, check_content, check_room};
use crate::media_library::media_entries;
use crate::media_upload::EnvelopeParser;
use crate::playground::FileError;

/// Held while the library's files are counted and a new one is renamed into place. One lock for
/// the process: the library is one directory, and the dev API is a loopback tool.
static LIBRARY_WRITE: Mutex<()> = Mutex::new(());

/// Why an add did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AddError {
    /// The request or the file is not one the library takes.
    Ingest(IngestError),
    /// The library's folder could not be read or written.
    File(FileError),
}

impl AddError {
    /// The closed word of the refusal, for the audit line: the ingest or file error's key.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::Ingest(error) => error.key(),
            Self::File(error) => error.key(),
        }
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Ingest(error) => error.status(),
            Self::File(error) => error.status(),
        }
    }
}

impl fmt::Display for AddError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ingest(error) => write!(formatter, "{error}"),
            Self::File(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AddError {}

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
            // Failure to remove is irrelevant: the file is ours and unlisted, and is only disk
            // space.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn stage(directory: &Path) -> Result<(File, StagedFile), FileError> {
    std::fs::create_dir_all(directory)?;
    let path = crate::atomic_file::temporary_path(directory, "upload");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
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
) -> Result<Added, AddError> {
    let (mut file, mut staged) = stage(directory).map_err(AddError::File)?;
    let mut parser = EnvelopeParser::new(limits);
    while let Some(chunk) = chunks.blocking_recv() {
        parser.feed(&chunk, &mut file).map_err(AddError::Ingest)?;
    }
    let envelope = parser.finish().map_err(AddError::Ingest)?;
    let upload = UploadKind::from_name(&envelope.name).ok_or(AddError::Ingest(
        IngestError::InvalidMediaName {
            name: envelope.name.clone(),
        },
    ))?;
    check_content(&mut file, upload, limits).map_err(AddError::Ingest)?;
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
) -> Result<(), AddError> {
    let _writing = LIBRARY_WRITE.lock().unwrap_or_else(PoisonError::into_inner);
    let existing = media_entries(directory).map_err(AddError::File)?;
    check_room(&existing, name, limits).map_err(AddError::Ingest)?;
    std::fs::rename(staged, directory.join(name)).map_err(|error| AddError::File(error.into()))
}
