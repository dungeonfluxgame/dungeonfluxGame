//! The held file is the capability. Dropping it never retires an ambiguous record.
use crate::file_identity::{self, FileIdentity, same};
use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

const RECORD_LIMIT: usize = 16 * 1024;
const RECORD_NAME: &str = "preview-owner.txt";

#[derive(Debug)]
pub(super) enum PreviewError {
    IncompleteConfiguration,
    InvalidLabel(&'static str),
    InvalidRoot,
    InvalidPath,
    InvalidListener,
    RecordTooLarge,
    RecordChanged,
    RecordMissing,
    Io(io::Error),
    Worker(tokio::task::JoinError),
}

impl fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompleteConfiguration => {
                formatter.write_str("incomplete managed preview configuration")
            }
            Self::InvalidLabel(field) => write!(formatter, "invalid preview {field}"),
            Self::InvalidRoot => formatter.write_str(
                "preview state root must be canonical and attempt-owned beneath artifacts/tmp",
            ),
            Self::InvalidPath => formatter.write_str(
                "preview path must be a canonical UTF-8 directory of at most 4096 bytes",
            ),
            Self::InvalidListener => {
                formatter.write_str("preview listener must bind a nonzero 127.0.0.1 port")
            }
            Self::RecordTooLarge => formatter.write_str("preview record exceeds 16 KiB"),
            Self::RecordChanged => {
                formatter.write_str("preview ownership changed; record retained")
            }
            Self::RecordMissing => {
                formatter.write_str("preview ownership record missing; release refused")
            }
            Self::Io(error) => write!(formatter, "preview filesystem failure: {error}"),
            Self::Worker(error) => write!(formatter, "preview filesystem worker failed: {error}"),
        }
    }
}

impl Error for PreviewError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Worker(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PreviewError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) struct PreviewConfiguration {
    root: PathBuf,
    attempt: String,
    web_source: String,
}

fn label(value: OsString, field: &'static str) -> Result<String, PreviewError> {
    let value = value
        .into_string()
        .map_err(|_| PreviewError::InvalidLabel(field))?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(PreviewError::InvalidLabel(field));
    }
    Ok(value)
}

impl PreviewConfiguration {
    pub(super) fn from_environment() -> Result<Option<Self>, PreviewError> {
        Self::from_values([
            env::var_os("DF_PREVIEW_STATE_ROOT"),
            env::var_os("DF_PREVIEW_ATTEMPT_ID"),
            env::var_os("DF_PREVIEW_WEB_SOURCE_ID"),
        ])
    }

    fn from_values(values: [Option<OsString>; 3]) -> Result<Option<Self>, PreviewError> {
        match values {
            [None, None, None] => Ok(None),
            [Some(root), Some(attempt), Some(web_source)] => Ok(Some(Self {
                root: root.into(),
                attempt: label(attempt, "attempt ID")?,
                web_source: label(web_source, "web source ID")?,
            })),
            _ => Err(PreviewError::IncompleteConfiguration),
        }
    }

    pub(super) async fn register(
        self,
        web_root: PathBuf,
        address: SocketAddr,
    ) -> Result<PreviewRegistration, PreviewError> {
        tokio::task::spawn_blocking(move || self.register_file(web_root, address, crate::BUILD_ID))
            .await
            .map_err(PreviewError::Worker)?
    }

    fn register_file(
        self,
        web_root: PathBuf,
        address: SocketAddr,
        native_build: &str,
    ) -> Result<PreviewRegistration, PreviewError> {
        if address.ip() != std::net::Ipv4Addr::LOCALHOST || address.port() == 0 {
            return Err(PreviewError::InvalidListener);
        }
        label(native_build.into(), "native build ID")?;
        if native_build == "unregistered-cargo-build" {
            return Err(PreviewError::InvalidLabel("native build ID"));
        }
        let root = canonical_directory(&self.root)?;
        if !file_identity::canonical_equals(&root, &self.root)
            || !attempt_root(&root, &self.attempt)?
        {
            return Err(PreviewError::InvalidRoot);
        }
        let web_root = canonical_directory(&web_root)?;
        if web_root.starts_with(&root) || root.starts_with(&web_root) {
            return Err(PreviewError::InvalidRoot);
        }
        let root_identity = file_identity::snapshot_path(&root)?;
        let record = self.record(address, &web_root, native_build)?;
        let path = root.join(RECORD_NAME);
        let mut file = file_identity::create_record_exclusive(&path)?;
        // Any partial write remains ambiguous; startup never announces readiness.
        file.write_all(&record)?;
        file.sync_all()?;
        let file_identity = file_identity::snapshot_file(&file)?;
        if !same(&root_identity, &file_identity::snapshot_path(&root)?)
            || !same(&file_identity, &file_identity::snapshot_path(&path)?)
        {
            return Err(PreviewError::RecordChanged);
        }
        Ok(PreviewRegistration {
            file,
            path,
            root_identity,
            file_identity,
            record,
        })
    }

    fn record(
        &self,
        address: SocketAddr,
        web_root: &Path,
        native_build: &str,
    ) -> Result<Vec<u8>, PreviewError> {
        let record = format!(
            "DF-PREVIEW-OWNER-V1\nattempt={}\npid={}\nport={}\nweb_root={:?}\ndata_root={:?}\nnative_build={}\nweb_source={}\n",
            self.attempt,
            std::process::id(),
            address.port(),
            path_text(web_root)?,
            path_text(&self.root)?,
            native_build,
            self.web_source,
        ).into_bytes();
        if record.len() > RECORD_LIMIT {
            return Err(PreviewError::RecordTooLarge);
        }
        Ok(record)
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, PreviewError> {
    let path = fs::canonicalize(path)?;
    path_text(&path)?;
    if !path.is_dir() {
        return Err(PreviewError::InvalidPath);
    }
    Ok(path)
}

fn path_text(path: &Path) -> Result<&str, PreviewError> {
    let value = path.to_str().ok_or(PreviewError::InvalidPath)?;
    if !path.is_absolute() || value.len() > 4096 {
        return Err(PreviewError::InvalidPath);
    }
    Ok(value)
}

fn attempt_root(root: &Path, attempt: &str) -> Result<bool, PreviewError> {
    if matches!(attempt, "." | "..") {
        return Ok(false);
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or(PreviewError::InvalidRoot)?;
    // Managed worktrees live under the main repository's artifacts/worktrees.
    let artifacts = workspace
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "artifacts"))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace.join("artifacts"));
    let temporary_root = artifacts.join("tmp");
    let canonical_temporary = fs::canonicalize(&temporary_root)?;
    if !file_identity::canonical_equals(&canonical_temporary, &temporary_root) {
        return Ok(false);
    }
    // Compare in canonical form on every platform: `root` arrives canonicalized,
    // which is the verbatim form on Windows.
    let namespace = canonical_temporary.join(attempt);
    if root == namespace || !root.starts_with(&namespace) {
        return Ok(false);
    }
    for directory in [namespace.as_path(), root] {
        let identity = file_identity::snapshot_path(directory)?;
        if !identity.is_dir() || !identity.mode_equals(0o700) {
            return Ok(false);
        }
    }
    Ok(true)
}

// No Clone and no Drop deletion: only an orderly serving scope may consume this.
pub(super) struct PreviewRegistration {
    file: File,
    path: PathBuf,
    root_identity: FileIdentity,
    file_identity: FileIdentity,
    record: Vec<u8>,
}

impl PreviewRegistration {
    /// Caller must have dropped the serving futures (and their listener) first.
    pub(super) async fn release_after_listener_stopped(self) -> Result<(), PreviewError> {
        tokio::task::spawn_blocking(move || self.release_file())
            .await
            .map_err(PreviewError::Worker)?
    }

    fn release_file(mut self) -> Result<(), PreviewError> {
        let root = self.path.parent().ok_or(PreviewError::InvalidRoot)?;
        if !file_identity::canonical_equals(&fs::canonicalize(root)?, root)
            || !same(&self.root_identity, &file_identity::snapshot_path(root)?)
        {
            return Err(PreviewError::RecordChanged);
        }
        let observed = match file_identity::snapshot_path(&self.path) {
            Ok(identity) => identity,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(PreviewError::RecordMissing);
            }
            Err(error) => return Err(error.into()),
        };
        if !same(&self.file_identity, &observed) || observed.len() > RECORD_LIMIT as u64 {
            return Err(PreviewError::RecordChanged);
        }
        self.file.seek(SeekFrom::Start(0))?;
        let mut record = Vec::new();
        (&mut self.file)
            .take(RECORD_LIMIT as u64 + 1)
            .read_to_end(&mut record)?;
        // Exact equality recognizes only the finite, validated serialization that
        // this capability wrote. Trailing, malformed, or changed fields all refuse.
        if record != self.record
            || !same(
                &self.file_identity,
                &file_identity::snapshot_path(&self.path)?,
            )
        {
            return Err(PreviewError::RecordChanged);
        }
        fs::remove_file(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Symlink and mode assertions are Unix-only cases.
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt, symlink};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn private_directory(path: &Path) {
        // Directory modes are a Unix-only property.
        #[cfg(unix)]
        {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        #[cfg(not(unix))]
        {
            fs::DirBuilder::new().create(path).unwrap();
        }
    }

    fn root() -> PathBuf {
        let attempt = format!(
            "preview-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let namespace = workspace.join("artifacts").join("tmp").join(attempt);
        private_directory(&namespace);
        let root = namespace.join("state");
        private_directory(&root);
        root
    }

    fn configuration(root: PathBuf) -> PreviewConfiguration {
        PreviewConfiguration {
            attempt: root
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
            root,
            web_source: "web-test".into(),
        }
    }

    fn registration(root: &Path) -> PreviewRegistration {
        configuration(root.into())
            .register_file(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                "127.0.0.1:43195".parse().unwrap(),
                "native-test",
            )
            .unwrap()
    }

    #[test]
    fn configuration_is_all_absent_or_all_valid() {
        assert!(
            PreviewConfiguration::from_values([None, None, None])
                .unwrap()
                .is_none()
        );
        for mask in 1..7 {
            let values = std::array::from_fn(|index| {
                (mask & (1 << index) != 0).then(|| OsString::from("valid"))
            });
            assert!(matches!(
                PreviewConfiguration::from_values(values),
                Err(PreviewError::IncompleteConfiguration)
            ));
        }
        for value in ["", "bad label", "../bad", "λ"] {
            assert!(label(value.into(), "test").is_err());
        }
        assert!(label("x".repeat(128).into(), "test").is_ok());
        assert!(label("x".repeat(129).into(), "test").is_err());
    }

    #[test]
    fn held_record_releases_exclusively_and_preserves_other_contents() {
        let root = root();
        fs::write(root.join("sentinel"), b"keep").unwrap();
        let owner = registration(&root);
        let record = fs::read_to_string(root.join(RECORD_NAME)).unwrap();
        assert!(record.contains(&format!("pid={}\n", std::process::id())));
        assert!(record.contains("port=43195\n"));
        assert!(record.contains("native_build=native-test\nweb_source=web-test\n"));
        assert!(record.contains(&format!("data_root={:?}\n", root.to_str().unwrap())));
        assert!(
            configuration(root.clone())
                .register_file(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                    "127.0.0.1:43195".parse().unwrap(),
                    "native-test"
                )
                .is_err()
        );
        owner.release_file().unwrap();
        assert!(!root.join(RECORD_NAME).exists());
        assert_eq!(fs::read(root.join("sentinel")).unwrap(), b"keep");
    }

    #[test]
    fn every_identity_change_and_extra_bytes_refuse_release() {
        for field in [
            "attempt",
            "pid",
            "port",
            "web_root",
            "data_root",
            "native_build",
            "web_source",
            "trailing",
            "oversized",
        ] {
            let root = root();
            let owner = registration(&root);
            let path = root.join(RECORD_NAME);
            let original = fs::read_to_string(&path).unwrap();
            let changed = if field == "oversized" {
                "x".repeat(RECORD_LIMIT + 1)
            } else if field == "trailing" {
                format!("{original}extra\n")
            } else {
                original
                    .lines()
                    .map(|line| {
                        if line.starts_with(&format!("{field}=")) {
                            format!("{field}=changed")
                        } else {
                            line.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            };
            fs::write(&path, &changed).unwrap();
            assert!(
                matches!(owner.release_file(), Err(PreviewError::RecordChanged)),
                "{field}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), changed);
        }
    }

    #[test]
    fn identical_replacement_missing_symlink_and_drop_stay_ambiguous() {
        // The symlink swap and the root swap (a directory rename while the
        // record file is open) are Unix-only cases; the other swaps keep
        // running on Windows.
        #[cfg(unix)]
        let kinds = ["replacement", "missing", "symlink", "drop", "root"];
        #[cfg(not(unix))]
        let kinds = ["replacement", "missing", "drop"];
        for kind in kinds {
            let root = root();
            let owner = registration(&root);
            let path = root.join(RECORD_NAME);
            let record = fs::read(&path).unwrap();
            if kind == "drop" {
                drop(owner);
                assert_eq!(fs::read(&path).unwrap(), record);
                continue;
            }
            if kind == "root" {
                fs::rename(&root, root.with_extension("old")).unwrap();
                // Directory modes are a Unix-only property.
                #[cfg(unix)]
                fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
                #[cfg(not(unix))]
                fs::DirBuilder::new().create(&root).unwrap();
                fs::write(&path, &record).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
                if kind == "replacement" {
                    fs::write(&path, &record).unwrap();
                }
                #[cfg(unix)]
                if kind == "symlink" {
                    let target = root.join("foreign");
                    fs::write(&target, &record).unwrap();
                    symlink(&target, &path).unwrap();
                }
            }
            let result = owner.release_file();
            assert!(
                matches!(
                    result,
                    Err(PreviewError::RecordChanged | PreviewError::RecordMissing)
                ),
                "{kind}: {result:?}"
            );
            if kind != "missing" {
                assert_eq!(fs::read(&path).unwrap(), record);
            }
        }
    }

    #[test]
    fn root_build_and_listener_validation_precede_record_creation() {
        let root = root();
        for (address, build) in [
            ("127.0.0.1:0", "native"),
            ("127.0.0.2:43195", "native"),
            ("127.0.0.1:43195", "unregistered-cargo-build"),
            ("127.0.0.1:43195", "bad build"),
        ] {
            assert!(
                configuration(root.clone())
                    .register_file(
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                        address.parse().unwrap(),
                        build
                    )
                    .is_err()
            );
            assert!(!root.join(RECORD_NAME).exists());
        }
        // Registering through a symlinked root is a Unix-only case.
        #[cfg(unix)]
        {
            let alias = root.with_extension("alias");
            symlink(&root, &alias).unwrap();
            assert!(
                configuration(alias)
                    .register_file(
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                        "127.0.0.1:43195".parse().unwrap(),
                        "native"
                    )
                    .is_err()
            );
        }
        let mut wrong_owner = configuration(root.clone());
        wrong_owner.attempt = "foreign".into();
        assert!(
            wrong_owner
                .register_file(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                    "127.0.0.1:43195".parse().unwrap(),
                    "native"
                )
                .is_err()
        );
        assert!(!root.join(RECORD_NAME).exists());
    }

    // POSIX `/`-rooted paths are not absolute on Windows.
    #[cfg(unix)]
    #[test]
    fn utf8_path_limit_is_measured_before_debug_escaping() {
        let at_limit = format!("/{}", "x".repeat(4095));
        assert!(path_text(Path::new(&at_limit)).is_ok());
        assert!(path_text(Path::new(&(at_limit + "x"))).is_err());
        assert!(path_text(Path::new("relative")).is_err());
    }

    // The 16 KiB boundary math assumes POSIX `/`-rooted paths and escaping.
    #[cfg(unix)]
    #[test]
    fn serialized_record_accepts_exactly_16_kib_and_refuses_one_more_byte() {
        let configuration = configuration(PathBuf::from(format!("/{}", "\\".repeat(4095))));
        let plain_web = format!("/{}", "x".repeat(4095));
        let address = "127.0.0.1:43195".parse().unwrap();
        let base = configuration
            .record(address, Path::new(&plain_web), "native")
            .unwrap()
            .len();
        let escaped = RECORD_LIMIT - base;
        assert!(escaped < 4095);
        let web = format!("/{}{}", "\\".repeat(escaped), "x".repeat(4095 - escaped));
        assert_eq!(
            configuration
                .record(address, Path::new(&web), "native")
                .unwrap()
                .len(),
            RECORD_LIMIT
        );
        let larger_web = format!(
            "/{}{}",
            "\\".repeat(escaped + 1),
            "x".repeat(4094 - escaped)
        );
        assert!(matches!(
            configuration.record(address, Path::new(&larger_web), "native"),
            Err(PreviewError::RecordTooLarge)
        ));
    }
}
