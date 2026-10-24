//! Launcher data folder and small-file helpers.
use std::{io, path::Path, path::PathBuf};

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
    staging.push(format!(".{}.tmp", std::process::id()));
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

/// The last `lines` lines of a text file, for showing why a start failed.
pub fn tail(path: &Path, lines: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(64 * 1024);
    let text = String::from_utf8_lossy(bytes.get(start..).unwrap_or_default());
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
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
}
