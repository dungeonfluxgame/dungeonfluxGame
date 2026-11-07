//! Whole-or-nothing file writes for the host: bytes are written to a temporary file in the target's
//! directory and renamed into place, so a reader never sees half a file. Every temporary name is
//! unique to one write of this process (process id plus a counter), so concurrent writes of one
//! name never share a temporary file and the last rename wins whole. Temporary names start with a
//! dot, so directory listings that skip hidden files never show them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// A path in `directory` for a temporary file standing in for `file_name`, shared by no other
/// write of this process.
pub(crate) fn temporary_path(directory: &Path, file_name: &str) -> PathBuf {
    // Relaxed is enough: the counter only has to hand out distinct numbers.
    let ordinal = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(".{file_name}.{}.{ordinal}.tmp", std::process::id()))
}

/// Writes `bytes` to `directory/file_name` whole or not at all, creating `directory` if missing.
/// On failure the temporary file is removed and the target is left as it was.
pub(crate) fn write_atomically(
    directory: &Path,
    file_name: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let temporary = temporary_path(directory, file_name);
    write_synced(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, directory.join(file_name)))
        .inspect_err(|_| {
            // The temporary file is ours; failure to remove it only leaves disk space used, and
            // the write's own error is what is reported.
            let _ = std::fs::remove_file(&temporary);
        })
}

/// Writes `bytes` to `path` and flushes them to disk, so a crash after the rename cannot leave an
/// empty or short file under the final name.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
