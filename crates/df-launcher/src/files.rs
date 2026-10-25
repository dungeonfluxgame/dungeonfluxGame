//! Launcher data folder and small-file helpers.
use std::{
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Gives each staging file in this process its own name, so writers to one path never share it.
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How much of a file `tail` reads from its end.
const TAIL_WINDOW: u64 = 64 * 1024;

/// `%APPDATA%\DungeonFlux` on Windows (the platform config folder elsewhere), unless the
/// developer passed `--data-dir` to keep a test run apart from real settings and saves.
pub fn data_folder(explicit: Option<PathBuf>) -> Result<PathBuf, io::Error> {
    let folder = match explicit {
        Some(folder) => folder,
        None => dirs::config_dir()
            .ok_or_else(|| io::Error::other("this account has no application data folder"))?
            .join("DungeonFlux"),
    };
    std::fs::create_dir_all(&folder)?;
    Ok(folder)
}

/// Writes beside the target and renames over it, so a crash never leaves a half-written file.
/// The file is readable by its owner only on Unix (the configuration holds provider keys); on
/// Windows it inherits the folder's access list, which in a user profile is that user's.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), io::Error> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::other("file path has no name"))?;
    let mut staging = file_name.to_owned();
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    staging.push(format!(".{}.{sequence}.tmp", std::process::id()));
    let staging = path.with_file_name(staging);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&staging).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    written
        .and_then(|()| std::fs::rename(&staging, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&staging);
        })
}

/// The last `lines` lines of a text file, for showing why a start failed. Only the end of the
/// file is read, so a long game log costs the same to show as a short one.
pub fn tail(path: &Path, lines: usize) -> String {
    read_tail(path, lines).unwrap_or_default()
}

fn read_tail(path: &Path, lines: usize) -> Result<String, io::Error> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let window_start_byte = length.saturating_sub(TAIL_WINDOW);
    // The byte before the window says whether the window begins on a line boundary.
    let preceding_byte_offset = window_start_byte.saturating_sub(1);
    file.seek(SeekFrom::Start(preceding_byte_offset))?;
    let mut bytes = Vec::new();
    file.take(length - preceding_byte_offset)
        .read_to_end(&mut bytes)?;
    let (starts_mid_line, text) = if window_start_byte > 0 {
        let starts_at_line_boundary = bytes.first() == Some(&b'\n');
        let text = String::from_utf8_lossy(bytes.get(1..).unwrap_or_default());
        (!starts_at_line_boundary, text)
    } else {
        (false, String::from_utf8_lossy(&bytes))
    };
    let mut all: Vec<&str> = text.lines().collect();
    // A cut first line is a fragment, not a line the file holds. It is dropped only when a
    // non-blank line follows it. Otherwise it is the end of a line longer than the window, which
    // is still the useful part to show, even when blank lines come after it.
    let text_follows = all
        .get(1..)
        .is_some_and(|rest| rest.iter().any(|line| !line.trim().is_empty()));
    if starts_mid_line && text_follows {
        all.remove(0);
    }
    Ok(all[all.len().saturating_sub(lines)..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_tail_keeps_the_last_lines() {
        let folder = std::env::temp_dir().join(format!("df-launcher-files-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("x.txt");
        write_atomic(&path, b"old").unwrap();
        write_atomic(&path, b"one\ntwo\nthree\n").unwrap();
        assert_eq!(tail(&path, 2), "two\nthree");
        assert_eq!(tail(&folder.join("missing"), 2), "");
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// One folder per case, because the cases run in parallel and each removes its folder.
    fn tail_fixture(case: &str, text: &[u8]) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("df-launcher-tail-{case}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("file.log");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn tail_of_a_long_file_drops_the_line_the_window_cut_in_half() {
        // 11-byte lines in 220,000 bytes: the 64 KiB window starts at byte 154,464, inside
        // line 14042 (which spans 154,462 to 154,472). Whole lines 14043 to 19999 remain.
        let text: String = (0..20_000)
            .map(|line| format!("line-{line:05}\n"))
            .collect();
        assert_eq!(text.len(), 220_000);
        let path = tail_fixture("cut", text.as_bytes());
        assert_eq!(tail(&path, 2), "line-19998\nline-19999");
        let whole: Vec<String> = (14_043..20_000)
            .map(|line| format!("line-{line:05}"))
            .collect();
        assert_eq!(tail(&path, 100_000), whole.join("\n"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn tail_window_that_starts_on_a_line_boundary_keeps_its_first_line() {
        // 16-byte lines in 320,000 bytes: the window starts at byte 254,464, exactly where
        // line 15904 begins, so that complete line is shown.
        let text: String = (0..20_000)
            .map(|line| format!("line-{line:010}\n"))
            .collect();
        assert_eq!(text.len(), 320_000);
        let path = tail_fixture("boundary", text.as_bytes());
        let whole: Vec<String> = (15_904..20_000)
            .map(|line| format!("line-{line:010}"))
            .collect();
        assert_eq!(whole.len(), 4_096);
        assert_eq!(tail(&path, 100_000), whole.join("\n"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn tail_of_one_line_longer_than_the_window_returns_its_end() {
        // The only line is 70,000 bytes; the excerpt is its last 65,536 bytes, not nothing.
        let mut text = b"early\n".to_vec();
        text.extend(std::iter::repeat_n(b'z', 70_000));
        let path = tail_fixture("long-line", &text);
        assert_eq!(tail(&path, 3), "z".repeat(TAIL_WINDOW as usize));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn tail_of_one_long_line_followed_by_blank_lines_returns_its_end() {
        // The window's text is 65,534 z then two blank lines. The cut z-run is not dropped just
        // because blank lines follow it: the excerpt is the line's end and the blank line.
        let mut text = b"early\n".to_vec();
        text.extend(std::iter::repeat_n(b'z', 70_000));
        text.extend_from_slice(b"\n\n");
        let path = tail_fixture("long-line-blank", &text);
        assert_eq!(tail(&path, 3), format!("{}\n", "z".repeat(65_534)));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_writers_to_one_path_leave_one_complete_version_and_no_staging_files() {
        let folder = std::env::temp_dir().join(format!("df-launcher-race-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("saved-games.toml");
        let payloads: Vec<Vec<u8>> = (0u8..6).map(|writer| vec![b'a' + writer; 8192]).collect();
        let outcomes: Vec<bool> = std::thread::scope(|scope| {
            let workers: Vec<_> = payloads
                .iter()
                .map(|payload| {
                    let path = &path;
                    scope.spawn(move || (0..25).all(|_| write_atomic(path, payload).is_ok()))
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        // `any`, not `all`: while another replace of the same path is in flight, a write can
        // return an error (an unverified Windows MoveFileExW race), so the test requires only that
        // some write succeeds. Whatever succeeds must leave one complete payload.
        assert!(outcomes.iter().any(|written| *written));
        let written = std::fs::read(&path).unwrap();
        assert!(
            payloads.contains(&written),
            "the final file must be one complete payload"
        );
        let leftovers = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
