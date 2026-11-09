//! The voice clips the host makes: files named `voice-<n>.mp3` (ElevenLabs) or `voice-<n>.wav`
//! (Forge) in the media library, so the page's existing sound path plays them.
//!
//! A clip is stored through the library's one staged add (`media_library::store_bytes`). The host
//! keeps at most `kept_clips()` and evicts the lowest-numbered first; only files named
//! `voice-<digits>` with a clip suffix are ever listed, counted or removed here, so no other media
//! is touched. Both formats share one numbering.
//! A clip is deliberately uncached by text: the same line voiced twice is two clips.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::limits::kept_clips;
use super::provider::ClipFormat;
use super::request::{ClipCause, ClipRow, SpeechError};
use crate::blocking::{WriteLock, run_blocking};
use crate::io_cause::IoCause;
use crate::media_ingest::IngestError;
use crate::media_library::{media_directory, store_bytes};
use crate::media_store::AddError;
use crate::state::AppState;

const CLIP_PREFIX: &str = "voice-";
/// Every format a clip may be in.
const CLIP_FORMATS: [ClipFormat; 2] = [ClipFormat::Mp3, ClipFormat::Wav];

/// Held while a clip's number is chosen and while clips are evicted or cleared.
static CLIP_WRITES: WriteLock = WriteLock::new();
/// The next clip number this run has not handed out; a number found on disk raises it.
static NEXT_CLIP: AtomicU64 = AtomicU64::new(1);

/// A clip that was kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Clip {
    pub(super) name: String,
    pub(super) bytes: u64,
}

/// The number in a clip's name, or `None` when the name is not one the host made.
pub(super) fn clip_number(name: &str) -> Option<u64> {
    let stem = name.strip_prefix(CLIP_PREFIX)?;
    let digits = CLIP_FORMATS
        .iter()
        .find_map(|format| stem.strip_suffix(format.suffix()))?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The host's clips in `directory`, oldest (lowest number) first.
pub(super) fn list_clips(directory: &Path) -> Result<Vec<(u64, ClipRow)>, SpeechError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(unreadable(&error)),
    };
    let mut clips = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| unreadable(&error))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(number) = clip_number(&name) else {
            continue;
        };
        let metadata = entry.metadata().map_err(|error| unreadable(&error))?;
        if metadata.is_file() {
            clips.push((
                number,
                ClipRow {
                    name,
                    bytes: metadata.len(),
                },
            ));
        }
    }
    clips.sort_by_key(|(number, _)| *number);
    Ok(clips)
}

fn unreadable(error: &std::io::Error) -> SpeechError {
    SpeechError::ClipsUnreadable {
        cause: IoCause::from_kind(error.kind()),
    }
}

/// The folder of the media library.
fn library(app: &AppState) -> PathBuf {
    media_directory(app.playground().files())
}

/// The name of the next clip in `format`: above every clip on disk and every number handed out
/// this run.
fn next_name(directory: &Path, format: ClipFormat) -> Result<String, SpeechError> {
    let _choosing = CLIP_WRITES.lock();
    let on_disk = list_clips(directory)?
        .last()
        .map_or(0, |(number, _)| *number);
    let number = NEXT_CLIP
        .load(Ordering::Relaxed)
        .max(on_disk.saturating_add(1));
    NEXT_CLIP.store(number.saturating_add(1), Ordering::Relaxed);
    Ok(format!("{CLIP_PREFIX}{number}{}", format.suffix()))
}

/// Removes the oldest clips of `directory` until `keep` are left; returns how many went.
pub(super) fn evict_clips(directory: &Path, keep: usize) -> Result<u64, SpeechError> {
    let _evicting = CLIP_WRITES.lock();
    let clips = list_clips(directory)?;
    let surplus = clips.len().saturating_sub(keep);
    let mut removed = 0;
    for (_, clip) in clips.into_iter().take(surplus) {
        match std::fs::remove_file(directory.join(&clip.name)) {
            Ok(()) => removed += 1,
            // Already gone: another remover got it first.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(unreadable(&error)),
        }
    }
    Ok(removed)
}

/// Keeps `audio` (a clip in `format`) in the media library as a new clip, then trims the oldest
/// clips past the bound. A trim that fails is recorded and never fails the clip that was made.
pub(super) async fn store_clip(
    app: &AppState,
    audio: &[u8],
    format: ClipFormat,
) -> Result<Clip, SpeechError> {
    let directory = library(app);
    let named = directory.clone();
    let name = run_blocking(move || next_name(&named, format), SpeechError::Stopped).await?;
    let added = store_bytes(directory.clone(), &name, audio)
        .await
        .map_err(|error| SpeechError::ClipUnstored {
            cause: match error {
                AddError::Ingest(IngestError::LibraryFull { .. }) => ClipCause::LibraryFull,
                AddError::Ingest(_) => ClipCause::Refused,
                AddError::File(_) => ClipCause::Unwritable,
            },
        })?;
    let trimmed = run_blocking(
        move || evict_clips(&directory, kept_clips()),
        SpeechError::Stopped,
    )
    .await;
    if let Err(error) = trimmed {
        app.record_line(format!("SPEECH trim_failed error={}", error.key()));
    }
    Ok(Clip {
        name: added.name,
        bytes: added.file_bytes,
    })
}

/// The host's clips in the media library, oldest first.
pub(super) async fn clips(app: &AppState) -> Result<Vec<ClipRow>, SpeechError> {
    let directory = library(app);
    let listed = run_blocking(move || list_clips(&directory), SpeechError::Stopped).await?;
    Ok(listed.into_iter().map(|(_, clip)| clip).collect())
}

/// Removes every clip the host made; returns how many.
pub(super) async fn clear_clips(app: &AppState) -> Result<u64, SpeechError> {
    let directory = library(app);
    run_blocking(move || evict_clips(&directory, 0), SpeechError::Stopped).await
}
