use df_observe::TelemetryError;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
// O_NOFOLLOW differs by target. macOS: 0x100. Linux x86 and x86_64: 0x20000. Linux aarch64 and
// arm: 0x8000 (on those, 0x20000 is not O_NOFOLLOW, so an open would follow the link silently).
// Values from the kernel's per-arch fcntl headers; the crate has no libc dependency to check against.
#[cfg(target_os = "macos")]
const NOFOLLOW: i32 = 0x100;
#[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
const NOFOLLOW: i32 = 0x20000;
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
const NOFOLLOW: i32 = 0x8000;
#[cfg(all(
    target_os = "linux",
    not(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "arm"
    ))
))]
compile_error!(
    "df-telemetry: O_NOFOLLOW is not verified for this Linux architecture; add its value to ownership.rs"
);
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
compile_error!(
    "df-telemetry: O_NOFOLLOW is not verified for this unix target; add its value to ownership.rs"
);
/// Stable Win32 flag values for no-follow opens that can also open directories.
/// The identity fields themselves come from the `winapi-util` safe wrapper.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[cfg(unix)]
fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_link(metadata: &Metadata) -> bool {
    // Any reparse point (symlink, junction) is treated as a symlink.
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn is_hardlinked(_path: &Path, metadata: &Metadata) -> bool {
    metadata.is_file() && metadata.nlink() != 1
}

#[cfg(windows)]
fn is_hardlinked(path: &Path, metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    // The stable standard library exposes no link count here, so it is read
    // from a no-follow handle; an unreadable count refuses fail-closed.
    let handle = OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path);
    let links = handle
        .as_ref()
        .ok()
        .and_then(|file| winapi_util::file::information(file).ok())
        .map(|information| information.number_of_links());
    links != Some(1)
}

#[cfg(windows)]
fn is_open_reparse_point(file: &File) -> bool {
    winapi_util::file::information(file)
        .map(|information| information.file_attributes() as u32 & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}
const MARKER: &[u8] = b"df-telemetry native synthetic owned root v1\n";
pub(crate) struct OwnedRoot {
    pub path: PathBuf,
    _lock: File,
}
pub(crate) fn safe_path(path: &Path) -> Result<(), TelemetryError> {
    if !path.is_absolute() {
        return Err(TelemetryError::ForeignRoot);
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(TelemetryError::ForeignRoot);
        }
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if is_link(&metadata) || is_hardlinked(&prefix, &metadata) => {
                return Err(TelemetryError::ForeignRoot);
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(TelemetryError::Io),
        }
    }
    Ok(())
}
pub(crate) fn open_file(
    path: &Path,
    write: bool,
    create_new: bool,
) -> Result<File, TelemetryError> {
    safe_path(path)?;
    #[cfg(unix)]
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .create_new(create_new)
        .custom_flags(NOFOLLOW)
        .mode(0o600)
        .open(path)
        .map_err(|_| TelemetryError::Io)?;
    #[cfg(windows)]
    let file = {
        // Created with inherited ACLs and never followed through a link: refuse
        // what was opened when it is a reparse point.
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .create_new(create_new)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| TelemetryError::Io)?;
        if is_open_reparse_point(&file) {
            return Err(TelemetryError::ForeignRoot);
        }
        file
    };
    Ok(file)
}
pub(crate) fn sync_directory(path: &Path) -> Result<(), TelemetryError> {
    #[cfg(unix)]
    {
        open_file(path, false, false)?
            .sync_all()
            .map_err(|_| TelemetryError::Io)
    }
    // A directory handle cannot be flushed this way on Windows: documented
    // no-op. Durability there rests on file syncs and the checks that remain.
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}
impl OwnedRoot {
    pub fn open(path: &Path) -> Result<Self, TelemetryError> {
        safe_path(path)?;
        let parent = path.parent().ok_or(TelemetryError::ForeignRoot)?;
        // This foundation deliberately has no deployment path/configuration authority.
        // Only coordinator-provisioned synthetic roots beneath this local MAIN are writable.
        if parent
            != Path::new("/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06")
        {
            return Err(TelemetryError::ForeignRoot);
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(TelemetryError::ForeignRoot)?;
        if !name.starts_with("ownedfixture-")
            || name.len() > 100
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(TelemetryError::ForeignRoot);
        }
        let new = match fs::create_dir(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(_) => return Err(TelemetryError::Io),
        };
        if new {
            // Created with inherited ACLs on Windows, where modes have no
            // counterpart and the read-only attribute must stay off.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|_| TelemetryError::Io)?;
            }
            let mut marker = open_file(&path.join("OWNER"), true, true)?;
            marker.write_all(MARKER).map_err(|_| TelemetryError::Io)?;
            marker.sync_all().map_err(|_| TelemetryError::Io)?;
            fs::create_dir(path.join("spool")).map_err(|_| TelemetryError::Io)?;
            sync_directory(path)?;
            sync_directory(parent)?;
        } else {
            let mut marker = open_file(&path.join("OWNER"), false, false)
                .map_err(|_| TelemetryError::ForeignRoot)?;
            if marker.metadata().map_err(|_| TelemetryError::Io)?.len() != MARKER.len() as u64 {
                return Err(TelemetryError::ForeignRoot);
            }
            let mut content = [0u8; MARKER.len()];
            marker
                .read_exact(&mut content)
                .map_err(|_| TelemetryError::ForeignRoot)?;
            if content != MARKER {
                return Err(TelemetryError::ForeignRoot);
            }
        }
        for entry in fs::read_dir(path).map_err(|_| TelemetryError::Io)? {
            let entry = entry.map_err(|_| TelemetryError::Io)?;
            safe_path(&entry.path())?;
            let name = entry.file_name();
            if !matches!(
                name.to_str(),
                Some(
                    "OWNER"
                        | "LOCK"
                        | "spool"
                        | "telemetry.sqlite3"
                        | "telemetry.sqlite3-wal"
                        | "telemetry.sqlite3-shm"
                        | "checkpoint"
                        | "checkpoint-next"
                        | "emergency"
                )
            ) {
                return Err(TelemetryError::ForeignRoot);
            }
        }
        let lock_path = path.join("LOCK");
        let lock = if lock_path.exists() {
            open_file(&lock_path, true, false)?
        } else {
            open_file(&lock_path, true, true)?
        };
        lock.try_lock().map_err(|_| TelemetryError::Ownership)?;
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::NOFOLLOW;
    use std::{
        fs::{self, OpenOptions},
        os::unix::fs::{OpenOptionsExt, symlink},
    };

    // A wrong NOFOLLOW value opens the target instead of refusing the link, so this fails on the
    // wrong target. ELOOP is errno 40 on the Linux architectures this crate builds for.
    #[test]
    fn nofollow_refuses_a_symlink_on_linux() {
        let dir =
            std::env::temp_dir().join(format!("df-telemetry-nofollow-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target");
        let link = dir.join("link");
        fs::write(&target, b"x").unwrap();
        symlink(&target, &link).unwrap();
        let result = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW)
            .open(&link);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result.unwrap_err().raw_os_error(), Some(40));
    }
}
