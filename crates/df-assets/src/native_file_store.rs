use crate::{
    AssetManifest, AssetReadStore, AssetStore, DurableObject, PublicationError, PublishedBinding,
    StoreError,
};
use df_types::OperationId;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAX_PROMOTION_SLOTS: u32 = 1024;

/// Native file-backed byte port. The caller configures a durable root outside caches.
pub struct NativeFileStore {
    root: PathBuf,
    max_object_bytes: u64,
}

/// Private staging handle. Its existence does not imply metadata publication.
pub struct StagedUpload {
    root: PathBuf,
    path: PathBuf,
    operation: OperationId,
}

impl NativeFileStore {
    pub fn new(root: impl AsRef<Path>, max_object_bytes: u64) -> Result<Self, StoreError> {
        fs::create_dir_all(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        fs::create_dir_all(root.join("staging"))?;
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("promotion"))?;
        require_objects_dir(&root.join("objects"))?;
        sync_directory(&root)?;
        Ok(Self {
            root,
            max_object_bytes,
        })
    }

    fn object_path(&self, digest: &[u8; 32]) -> PathBuf {
        self.root.join("objects").join(hex(digest))
    }

    /// Reopen the same private staging object after a lost acknowledgement or process restart.
    /// Publication still rechecks its complete bytes and metadata authority.
    pub fn resume_staged(&self, operation: OperationId) -> Result<StagedUpload, StoreError> {
        let path = self.root.join("staging").join(hex(operation.as_bytes()));
        File::open(&path)?;
        Ok(StagedUpload {
            root: self.root.clone(),
            path,
            operation,
        })
    }

    fn check_staged(&self, staged: &StagedUpload) -> Result<(), StoreError> {
        if staged.root != self.root {
            return Err(StoreError::StagingConflict);
        }
        Ok(())
    }

    /// Confirm that a metadata-bound object remains complete and matches its exact manifest.
    pub fn confirm(&self, object: DurableObject) -> Result<(), StoreError> {
        let path = self.object_path(object.digest());
        verify_file(
            &path,
            AssetManifest {
                byte_len: object.byte_len(),
                sha256: *object.digest(),
            },
        )
    }
}

impl AssetStore for NativeFileStore {
    type Staged = StagedUpload;

    fn check_expected(&self, expected: AssetManifest) -> Result<(), StoreError> {
        if expected.byte_len > self.max_object_bytes {
            return Err(StoreError::Capacity);
        }
        Ok(())
    }

    fn stage(
        &self,
        operation: OperationId,
        reader: &mut dyn Read,
    ) -> Result<StagedUpload, StoreError> {
        let path = self.root.join("staging").join(hex(operation.as_bytes()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    StoreError::StagingConflict
                } else {
                    StoreError::Io(error)
                }
            })?;
        let staged_result = (|| {
            let mut length = 0_u64;
            let mut buffer = [0_u8; 65536];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                length = length
                    .checked_add(u64::try_from(read).map_err(|_| StoreError::Capacity)?)
                    .ok_or(StoreError::Capacity)?;
                if length > self.max_object_bytes {
                    return Err(StoreError::Capacity);
                }
                file.write_all(&buffer[..read])?;
            }
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = staged_result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        sync_directory(&self.root.join("staging"))?;
        Ok(StagedUpload {
            root: self.root.clone(),
            path,
            operation,
        })
    }

    fn staged_len(&self, staged: &StagedUpload) -> Result<u64, StoreError> {
        self.check_staged(staged)?;
        Ok(File::open(&staged.path)?.metadata()?.len())
    }

    fn staged_operation(&self, staged: &StagedUpload) -> Result<OperationId, StoreError> {
        self.check_staged(staged)?;
        Ok(staged.operation)
    }

    fn matches_published<M: Eq>(
        &self,
        staged: &StagedUpload,
        published: &PublishedBinding<M>,
    ) -> Result<bool, StoreError> {
        self.check_staged(staged)?;
        let mut published_file = self.open_verified(published.object, published.bytes)?;
        let mut staged_file = File::open(&staged.path)?;
        same_contents(&mut staged_file, &mut published_file, published.bytes)
    }

    fn verify_and_promote(
        &self,
        staged: &StagedUpload,
        expected: AssetManifest,
    ) -> Result<DurableObject, PublicationError> {
        self.check_staged(staged).map_err(PublicationError::Store)?;
        if expected.byte_len > self.max_object_bytes {
            return Err(PublicationError::Store(StoreError::Capacity));
        }
        let mut temporary = None;
        let result = (|| {
            let mut input = File::open(&staged.path).map_err(StoreError::from)?;
            let operation_name = hex(staged.operation.as_bytes());
            let mut output = None;
            for slot in 0..MAX_PROMOTION_SLOTS {
                let name = if slot == 0 {
                    operation_name.clone()
                } else {
                    format!("{operation_name}-{slot}")
                };
                let path = self.root.join("promotion").join(name);
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(file) => {
                        temporary = Some(path);
                        output = Some(file);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(PublicationError::Store(StoreError::Io(error))),
                }
            }
            let mut output = output.ok_or(StoreError::StagingConflict)?;
            let mut hasher = Sha256::new();
            let mut length = 0_u64;
            let mut buffer = [0_u8; 65536];
            loop {
                let read = input.read(&mut buffer).map_err(StoreError::from)?;
                if read == 0 {
                    break;
                }
                length = length
                    .checked_add(u64::try_from(read).map_err(|_| PublicationError::Incomplete)?)
                    .ok_or(PublicationError::Incomplete)?;
                if length > self.max_object_bytes {
                    return Err(PublicationError::Store(StoreError::Capacity));
                }
                if length > expected.byte_len {
                    return Err(PublicationError::Incomplete);
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(StoreError::from)?;
                hasher.update(&buffer[..read]);
            }
            if length != expected.byte_len {
                return Err(PublicationError::Incomplete);
            }
            let digest: [u8; 32] = hasher.finalize().into();
            if digest != expected.sha256 {
                return Err(PublicationError::HashMismatch);
            }
            let mut permissions = output.metadata().map_err(StoreError::from)?.permissions();
            permissions.set_readonly(true);
            output
                .set_permissions(permissions)
                .map_err(StoreError::from)?;
            output.flush().map_err(StoreError::from)?;
            output.sync_all().map_err(StoreError::from)?;
            let destination = self.object_path(&expected.sha256);
            require_objects_dir(&self.root.join("objects"))?;
            let path = temporary.as_ref().ok_or(StoreError::StagingConflict)?;
            match fs::hard_link(path, &destination) {
                Ok(()) => {
                    sync_directory(&self.root.join("objects")).map_err(StoreError::from)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut destination_file = self
                        .open_verified(DurableObject::from_manifest(expected), expected)
                        .map_err(PublicationError::Store)?;
                    let mut promoted_file = File::open(path).map_err(StoreError::from)?;
                    if !same_contents(&mut promoted_file, &mut destination_file, expected)
                        .map_err(PublicationError::Store)?
                    {
                        return Err(PublicationError::Store(StoreError::BackingIntegrity));
                    }
                    #[cfg(not(windows))]
                    destination_file.sync_all().map_err(StoreError::from)?;
                    sync_directory(&self.root.join("objects")).map_err(StoreError::from)?;
                }
                Err(error) => return Err(PublicationError::Store(StoreError::Io(error))),
            }
            Ok(DurableObject::from_manifest(expected))
        })();
        if let Some(temporary) = temporary {
            let removed = fs::remove_file(&temporary);
            if result.is_ok() {
                removed.map_err(StoreError::from)?;
                sync_directory(&self.root.join("promotion")).map_err(StoreError::from)?;
            }
        }
        result
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    // Windows does not support flushing directory handles like Unix fsync.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn verify_file(path: &Path, expected: AssetManifest) -> Result<(), StoreError> {
    let mut file = open_backing(path)?;
    if file.metadata()?.len() != expected.byte_len {
        return Err(StoreError::BackingIntegrity);
    }
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(read).map_err(|_| StoreError::BackingIntegrity)?)
            .ok_or(StoreError::BackingIntegrity)?;
        if length > expected.byte_len {
            return Err(StoreError::BackingIntegrity);
        }
        hasher.update(&buffer[..read]);
    }
    if length != expected.byte_len {
        return Err(StoreError::BackingIntegrity);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if digest != expected.sha256 {
        return Err(StoreError::BackingIntegrity);
    }
    Ok(())
}

fn open_backing(path: &Path) -> Result<File, StoreError> {
    require_regular_backing_path(path)?;
    let file = File::open(path).map_err(backing_open_error)?;
    // A matching cache target must not stand in for an object in the durable root.
    require_regular_backing_path(path)?;
    if !file.metadata()?.is_file() {
        return Err(StoreError::BackingIntegrity);
    }
    Ok(file)
}

fn require_regular_backing_path(path: &Path) -> Result<(), StoreError> {
    require_objects_dir(path.parent().ok_or(StoreError::BackingIntegrity)?)?;
    if !fs::symlink_metadata(path)
        .map_err(backing_open_error)?
        .file_type()
        .is_file()
    {
        return Err(StoreError::BackingIntegrity);
    }
    Ok(())
}

fn require_objects_dir(path: &Path) -> Result<(), StoreError> {
    if !fs::symlink_metadata(path)
        .map_err(backing_open_error)?
        .file_type()
        .is_dir()
    {
        return Err(StoreError::BackingIntegrity);
    }
    Ok(())
}

fn backing_open_error(error: std::io::Error) -> StoreError {
    if error.kind() == std::io::ErrorKind::NotFound {
        StoreError::BackingMissing
    } else {
        StoreError::Io(error)
    }
}

fn same_contents(
    left: &mut File,
    right: &mut File,
    expected: AssetManifest,
) -> Result<bool, StoreError> {
    if right.metadata()?.len() != expected.byte_len {
        return Err(StoreError::BackingIntegrity);
    }
    if left.metadata()?.len() != expected.byte_len {
        return Ok(false);
    }
    same_contents_readers(left, right, expected)
}

// The private reader boundary permits deterministic mutation at a real file read in tests.
// Both callers pass open files; successful prefix equality alone cannot prove completeness.
fn same_contents_readers(
    left: &mut dyn Read,
    right: &mut dyn Read,
    expected: AssetManifest,
) -> Result<bool, StoreError> {
    let mut remaining = expected.byte_len;
    let mut hasher = Sha256::new();
    let mut left_bytes = [0_u8; 65536];
    let mut right_bytes = [0_u8; 65536];
    while remaining != 0 {
        let length =
            usize::try_from(remaining.min(65536)).map_err(|_| StoreError::BackingIntegrity)?;
        left.read_exact(&mut left_bytes[..length])?;
        right.read_exact(&mut right_bytes[..length])?;
        if left_bytes[..length] != right_bytes[..length] {
            return Ok(false);
        }
        hasher.update(&right_bytes[..length]);
        remaining -= u64::try_from(length).map_err(|_| StoreError::BackingIntegrity)?;
    }
    let left_end = left.read(&mut left_bytes[..1])?;
    let right_end = right.read(&mut right_bytes[..1])?;
    let digest: [u8; 32] = hasher.finalize().into();
    if right_end != 0 || digest != expected.sha256 {
        return Err(StoreError::BackingIntegrity);
    }
    Ok(left_end == 0)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

impl AssetReadStore for NativeFileStore {
    type Reader = File;

    fn open_verified(
        &self,
        object: DurableObject,
        manifest: AssetManifest,
    ) -> Result<File, StoreError> {
        self.check_expected(manifest)?;
        if object.byte_len() != manifest.byte_len || object.digest() != &manifest.sha256 {
            return Err(StoreError::BackingIntegrity);
        }
        let mut file = open_backing(&self.object_path(object.digest()))?;
        if file.metadata()?.len() != manifest.byte_len {
            return Err(StoreError::BackingIntegrity);
        }
        let mut hasher = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; 65536];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(read).map_err(|_| StoreError::BackingIntegrity)?)
                .ok_or(StoreError::BackingIntegrity)?;
            if length > manifest.byte_len {
                return Err(StoreError::BackingIntegrity);
            }
            hasher.update(&buffer[..read]);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        if length != manifest.byte_len || digest != manifest.sha256 {
            return Err(StoreError::BackingIntegrity);
        }
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

    fn files(left: &[u8], right: &[u8]) -> (File, File, PathBuf, PathBuf) {
        let temporary =
            PathBuf::from(std::env::var_os("TMPDIR").expect("owned TMPDIR is required"));
        let number = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = temporary.join(format!(
            "df-assets-comparison-{}-{number}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let left_path = root.join("left");
        let right_path = root.join("right");
        fs::write(&left_path, left).unwrap();
        fs::write(&right_path, right).unwrap();
        (
            File::open(&left_path).unwrap(),
            File::open(&right_path).unwrap(),
            left_path,
            right_path,
        )
    }

    fn manifest(bytes: &[u8]) -> AssetManifest {
        AssetManifest {
            byte_len: u64::try_from(bytes.len()).unwrap(),
            sha256: Sha256::digest(bytes).into(),
        }
    }

    #[test]
    fn native_store_initializes_directory_tree() {
        let temporary =
            PathBuf::from(std::env::var_os("TMPDIR").expect("owned TMPDIR is required"));
        let number = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = temporary.join(format!(
            "df-assets-native-store-{}-{number}",
            std::process::id()
        ));

        let store = NativeFileStore::new(&root, 1024).unwrap();

        assert!(store.root.join("staging").is_dir());
        assert!(store.root.join("objects").is_dir());
        assert!(store.root.join("promotion").is_dir());
    }

    fn verified_files(left: &[u8], right: &[u8]) -> (File, File, PathBuf, PathBuf) {
        let (left_file, _, left_path, right_path) = files(left, right);
        let store = NativeFileStore::new(right_path.parent().unwrap(), 1024).unwrap();
        let expected = manifest(right);
        let object_path = store.object_path(&expected.sha256);
        fs::rename(&right_path, &object_path).unwrap();
        let right_file = store
            .open_verified(DurableObject::from_manifest(expected), expected)
            .unwrap();
        (left_file, right_file, left_path, object_path)
    }

    enum Mutation {
        Append(PathBuf),
        Truncate(PathBuf),
    }

    struct MutatingReader {
        file: File,
        remaining: u64,
        mutation: Option<Mutation>,
    }

    impl Read for MutatingReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let read = self.file.read(output)?;
            self.remaining = self.remaining.saturating_sub(u64::try_from(read).unwrap());
            if self.remaining == 0 {
                match self.mutation.take() {
                    Some(Mutation::Append(path)) => {
                        let mut writer = OpenOptions::new().append(true).open(path)?;
                        writer.write_all(b"-tail")?;
                        writer.sync_all()?;
                    }
                    Some(Mutation::Truncate(path)) => {
                        OpenOptions::new().write(true).open(path)?.set_len(0)?;
                    }
                    None => {}
                }
            }
            Ok(read)
        }
    }

    struct EofFailure {
        file: File,
    }

    impl Read for EofFailure {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let read = self.file.read(output)?;
            if read == 0 {
                return Err(io::Error::other("controlled EOF read failure"));
            }
            Ok(read)
        }
    }

    #[test]
    fn comparison_accepts_complete_equal_files_including_empty_and_multiple_chunks() {
        for content in [Vec::new(), b"asset".to_vec(), vec![7; 131073]] {
            let (mut left, mut right, _, _) = files(&content, &content);
            assert!(same_contents(&mut left, &mut right, manifest(&content)).unwrap());
        }
    }

    #[test]
    fn comparison_refuses_different_content_and_non_manifest_lengths() {
        let expected = manifest(b"asset");
        for left_content in [
            b"other".as_slice(),
            b"asset-tail".as_slice(),
            b"as".as_slice(),
        ] {
            let (mut left, mut right, _, _) = files(left_content, b"asset");
            assert!(!same_contents(&mut left, &mut right, expected).unwrap());
        }
        for right_content in [b"asset-tail".as_slice(), b"as".as_slice()] {
            let (mut left, mut right, _, _) = files(b"asset", right_content);
            assert!(matches!(
                same_contents(&mut left, &mut right, expected),
                Err(StoreError::BackingIntegrity)
            ));
        }
        let (mut left, mut right, _, _) = files(b"asset-tail", b"asset-tail");
        assert!(matches!(
            same_contents(&mut left, &mut right, expected),
            Err(StoreError::BackingIntegrity)
        ));
    }

    #[test]
    fn comparison_refuses_staged_tail_appended_after_last_prefix_read() {
        let content = vec![9; 65537];
        let expected = manifest(&content);
        let (left, mut right, left_path, _) = files(&content, &content);
        assert_eq!(left.metadata().unwrap().len(), expected.byte_len);
        assert_eq!(right.metadata().unwrap().len(), expected.byte_len);
        let mut left = MutatingReader {
            file: left,
            remaining: expected.byte_len,
            mutation: Some(Mutation::Append(left_path.clone())),
        };
        assert!(!same_contents_readers(&mut left, &mut right, expected).unwrap());
        assert_eq!(
            fs::metadata(left_path).unwrap().len(),
            expected.byte_len + 5
        );
    }

    #[test]
    fn comparison_refuses_backing_tail_appended_after_last_prefix_read() {
        let content = b"asset";
        let expected = manifest(content);
        let (mut left, right, _, right_path) = verified_files(content, content);
        assert_eq!(left.metadata().unwrap().len(), expected.byte_len);
        assert_eq!(right.metadata().unwrap().len(), expected.byte_len);
        let mut right = MutatingReader {
            file: right,
            remaining: expected.byte_len,
            mutation: Some(Mutation::Append(right_path.clone())),
        };
        assert!(matches!(
            same_contents_readers(&mut left, &mut right, expected),
            Err(StoreError::BackingIntegrity)
        ));
        assert_eq!(fs::read(right_path).unwrap(), b"asset-tail");
    }

    #[test]
    fn comparison_propagates_truncation_after_initial_length_checks() {
        let expected = manifest(b"asset");
        let (left, mut right, _, right_path) = files(b"asset", b"asset");
        assert_eq!(left.metadata().unwrap().len(), expected.byte_len);
        assert_eq!(right.metadata().unwrap().len(), expected.byte_len);
        let mut left = MutatingReader {
            file: left,
            remaining: expected.byte_len,
            mutation: Some(Mutation::Truncate(right_path.clone())),
        };
        assert!(matches!(
            same_contents_readers(&mut left, &mut right, expected),
            Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert_eq!(fs::metadata(right_path).unwrap().len(), 0);
    }

    #[test]
    fn comparison_propagates_both_eof_read_failures_including_empty_files() {
        for content in [b"".as_slice(), b"asset".as_slice()] {
            let expected = manifest(content);
            let (left, mut right, _, _) = files(content, content);
            assert!(matches!(
                same_contents_readers(&mut EofFailure { file: left }, &mut right, expected),
                Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::Other
            ));
            let (mut left, right, _, _) = files(content, content);
            assert!(matches!(
                same_contents_readers(&mut left, &mut EofFailure { file: right }, expected),
                Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::Other
            ));
        }
    }

    #[test]
    fn comparison_rechecks_digest_if_verified_backing_changes_to_matching_same_length_bytes() {
        let expected = manifest(b"asset");
        let (mut left, mut right, _, right_path) = verified_files(b"other", b"asset");
        fs::write(&right_path, b"other").unwrap();
        assert!(matches!(
            same_contents(&mut left, &mut right, expected),
            Err(StoreError::BackingIntegrity)
        ));
        assert_eq!(fs::read(right_path).unwrap(), b"other");
    }

    #[test]
    fn comparison_keeps_verified_descriptor_when_digest_path_is_replaced() {
        let content = b"asset";
        let expected = manifest(content);
        let (mut left, mut right, _, right_path) = verified_files(content, content);
        fs::remove_file(&right_path).unwrap();
        fs::write(&right_path, b"other").unwrap();
        assert!(same_contents(&mut left, &mut right, expected).unwrap());
        assert_eq!(fs::read(right_path).unwrap(), b"other");
    }
}
