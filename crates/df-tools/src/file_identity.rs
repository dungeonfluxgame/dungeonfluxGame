//! Native file-identity snapshots for the build-set boundary and preview ownership.
//!
//! The `#[cfg(unix)]` side keeps the existing comparisons unchanged: device,
//! inode, owner, mode, link count and timestamps, with the same creation modes
//! and flags the callers used before.
//!
//! Windows has no uid/mode counterpart in this port: creating a file or
//! directory "with mode 0o600/0o700" creates it with inherited ACLs, setting a
//! mode is a no-op, a check that a mode equals some value is treated as
//! satisfied, and an owner comparison compares the volume only. The read-only
//! attribute is never set: it would block replacing `current` by rename and
//! block cleanup. Integrity on Windows therefore rests on the checks that
//! remain: SHA-256 digests of every part, identity snapshots, link counts,
//! reparse-point rejection and the closed set of allowed names. Anything that
//! is a reparse point (symlink, junction) is treated as a symlink is on Unix:
//! rejected.
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;

/// Stable Win32 attribute and flag values used for identity and no-follow opens.
/// They are fixed ABI constants documented with the file APIs; only the
/// identity fields themselves come from the `winapi-util` safe wrapper, so no
/// `windows-sys` dependency or FFI boundary is needed here.
#[cfg(windows)]
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// Identity snapshot of one filesystem object, captured without following links.
#[derive(Clone, Debug)]
pub(crate) struct FileIdentity {
    #[cfg(unix)]
    inner: fs::Metadata,
    #[cfg(windows)]
    inner: WindowsIdentity,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct WindowsIdentity {
    volume: u64,
    index: u64,
    attributes: u32,
    links: u64,
    len: u64,
    write_time: Option<u64>,
    creation_time: Option<u64>,
}

#[cfg(windows)]
impl WindowsIdentity {
    fn from_information(information: &winapi_util::file::Information) -> Self {
        Self {
            volume: information.volume_serial_number(),
            index: information.file_index(),
            attributes: information.file_attributes() as u32,
            links: information.number_of_links(),
            len: information.file_size(),
            write_time: information.last_write_time(),
            creation_time: information.creation_time(),
        }
    }

    fn is_reparse_point(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    fn is_directory(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }
}

impl FileIdentity {
    pub(crate) fn is_file(&self) -> bool {
        #[cfg(unix)]
        {
            self.inner.is_file()
        }
        #[cfg(windows)]
        {
            !self.inner.is_reparse_point() && !self.inner.is_directory()
        }
    }

    pub(crate) fn is_dir(&self) -> bool {
        #[cfg(unix)]
        {
            self.inner.is_dir()
        }
        #[cfg(windows)]
        {
            !self.inner.is_reparse_point() && self.inner.is_directory()
        }
    }

    pub(crate) fn is_symlink(&self) -> bool {
        #[cfg(unix)]
        {
            self.inner.file_type().is_symlink()
        }
        #[cfg(windows)]
        {
            self.inner.is_reparse_point()
        }
    }

    pub(crate) fn len(&self) -> u64 {
        #[cfg(unix)]
        {
            self.inner.len()
        }
        #[cfg(windows)]
        {
            self.inner.len
        }
    }

    pub(crate) fn links(&self) -> u64 {
        #[cfg(unix)]
        {
            self.inner.nlink()
        }
        #[cfg(windows)]
        {
            self.inner.links
        }
    }

    pub(crate) fn volume(&self) -> u64 {
        #[cfg(unix)]
        {
            self.inner.dev()
        }
        #[cfg(windows)]
        {
            self.inner.volume
        }
    }

    pub(crate) fn owner(&self) -> u64 {
        #[cfg(unix)]
        {
            u64::from(self.inner.uid())
        }
        #[cfg(windows)]
        {
            // No uid counterpart: the volume comparison above is the owner check.
            self.inner.volume
        }
    }

    pub(crate) fn mode_equals(&self, expected: u32) -> bool {
        #[cfg(unix)]
        {
            self.inner.mode() & 0o777 == expected
        }
        #[cfg(windows)]
        {
            // No mode counterpart: a mode check is treated as satisfied.
            let _ = expected;
            true
        }
    }

    /// A directory that may hold owned state: a real directory, not a link,
    /// with the private mode where modes exist.
    pub(crate) fn is_private_dir(&self) -> bool {
        self.is_dir() && !self.is_symlink() && self.mode_equals(0o700)
    }
}

/// Capture the identity of `path` without following a trailing link, so that
/// directories can be snapshotted on every platform.
pub(crate) fn snapshot_path(path: &Path) -> io::Result<FileIdentity> {
    #[cfg(unix)]
    {
        fs::symlink_metadata(path).map(|inner| FileIdentity { inner })
    }
    #[cfg(windows)]
    {
        let handle = OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        snapshot_file(&handle)
    }
}

/// Capture the identity of an already open handle.
pub(crate) fn snapshot_file(file: &File) -> io::Result<FileIdentity> {
    #[cfg(unix)]
    {
        file.metadata().map(|inner| FileIdentity { inner })
    }
    #[cfg(windows)]
    {
        winapi_util::file::information(file).map(|information| FileIdentity {
            inner: WindowsIdentity::from_information(&information),
        })
    }
}

/// True when both snapshots describe the same object with unchanged bytes.
pub(crate) fn same(expected: &FileIdentity, observed: &FileIdentity) -> bool {
    #[cfg(unix)]
    {
        expected.inner.dev() == observed.inner.dev()
            && expected.inner.ino() == observed.inner.ino()
            && expected.inner.uid() == observed.inner.uid()
            && expected.inner.mode() == observed.inner.mode()
            && expected.inner.is_file() == observed.inner.is_file()
            && expected.inner.is_dir() == observed.inner.is_dir()
            && !observed.inner.file_type().is_symlink()
            && (!expected.inner.is_file()
                || (expected.inner.len() == observed.inner.len()
                    && observed.inner.nlink() == 1
                    && expected.inner.mtime() == observed.inner.mtime()
                    && expected.inner.mtime_nsec() == observed.inner.mtime_nsec()
                    && expected.inner.ctime() == observed.inner.ctime()
                    && expected.inner.ctime_nsec() == observed.inner.ctime_nsec()))
    }
    #[cfg(windows)]
    {
        expected.inner.volume == observed.inner.volume
            && expected.inner.index == observed.inner.index
            && expected.inner.is_directory() == observed.inner.is_directory()
            && !observed.inner.is_reparse_point()
            && (!expected.is_file()
                || (expected.inner.len == observed.inner.len
                    && observed.inner.links == 1
                    && expected.inner.write_time == observed.inner.write_time
                    && expected.inner.creation_time == observed.inner.creation_time))
    }
}

/// Canonical-path equality. `canonicalize` returns the verbatim form
/// (`\\?\C:\...`) on Windows, which can never equal the caller's absolute
/// native path, so only the `\\?\X:\` form is accepted after removing its
/// prefix; UNC and device paths keep failing. Callers must still pass an
/// absolute native path that is already normalised; the one caller that keeps
/// a verbatim path it canonicalized itself (preview release) compares equal
/// after the same prefix removal on both sides.
pub(crate) fn canonical_equals(canonical: &Path, expected: &Path) -> bool {
    #[cfg(unix)]
    {
        canonical == expected
    }
    #[cfg(windows)]
    {
        fn stripped(path: &Path) -> Option<&str> {
            let text = path.as_os_str().to_str()?;
            let bytes = text.as_bytes();
            if bytes.len() > 7
                && bytes.starts_with(b"\\\\?\\")
                && bytes[4].is_ascii_alphabetic()
                && bytes[5] == b':'
                && bytes[6] == b'\\'
            {
                Some(&text[4..])
            } else {
                Some(text)
            }
        }
        match (stripped(canonical), stripped(expected)) {
            (Some(left), Some(right)) => Path::new(left) == Path::new(right),
            _ => false,
        }
    }
}

/// The `/`-separated relative name of `path` under `root`. Manifests stay
/// byte-for-byte identical on every platform; native separators never leak in.
pub(crate) fn relative_name(root: &Path, path: &Path) -> Option<String> {
    let suffix = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in suffix.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Create one directory; on Unix with the private mode, elsewhere with
/// inherited ACLs.
pub(crate) fn create_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(windows)]
    {
        fs::create_dir(path)
    }
}

/// Create a directory and its missing parents; on Unix with the private mode,
/// elsewhere with inherited ACLs.
pub(crate) fn create_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path)
    }
}

/// Create a new file for exclusive owned writing; on Unix with mode 0o600,
/// elsewhere with inherited ACLs.
pub(crate) fn create_file_exclusive(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
}

/// Create a new file for exclusive owned reading and writing; on Unix with
/// mode 0o600, elsewhere with inherited ACLs.
pub(crate) fn create_record_exclusive(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
    }
}

/// Restrict an owned file or directory to `mode`; on Unix the exact permission
/// bits, elsewhere a no-op that never sets the read-only attribute.
pub(crate) fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        // No mode counterpart, and the read-only attribute would block
        // replacing `current` by rename and block cleanup.
        let _ = (path, mode);
        Ok(())
    }
}

/// Flush a directory entry. Directories cannot be flushed this way on Windows,
/// where this is a documented no-op; on Unix it is unchanged.
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}

/// Replace the launcher with the selected executable. On Unix this keeps the
/// existing process replacement; on Windows the launcher holds no locks,
/// unpublished state or serving futures at this point, so it spawns the
/// selected executable with the same arguments, waits for it, and ends with
/// the child's exit code (a missing code maps to failure).
pub(crate) fn launch_selected(executable: &Path, argv: &[OsString]) -> io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(executable).args(argv).exec()
    }
    #[cfg(windows)]
    {
        match std::process::Command::new(executable).args(argv).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => error,
        }
    }
}
