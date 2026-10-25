//! The key-addressed [`ObjectStore`] over local files, for development and self-hosting. Each
//! object key is a directory `<root>/keyed/<domain>/<path>.obj/` of immutable version files, so a
//! write is a complete temporary file hard-linked into place: a reader never sees partial bytes and
//! two racing writers cannot both create the same version. File work runs on the blocking pool.
//!
//! `PrivateMedia` and `Backups` keep every version (as their hosted buckets are versioned); the
//! other domains keep only the newest, matching `versioning_status`.

use crate::AssetManifest;
use crate::NativeFileStore;
use crate::native_file_store::sync_directory;
use crate::object_store::{
    ConditionalWrites, Deadline, ListPage, ListedObject, ObjectKey, ObjectPrefix, ObjectStore,
    ObjectStoreError, ObjectVersion, StorageDomain, StoredObject, VersionListing, VersioningStatus,
};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const KEYED_DIRECTORY: &str = "keyed";
const TEMPORARY_DIRECTORY: &str = "keyed-tmp";
const OBJECT_SUFFIX: &str = ".obj";
const VERSION_SUFFIX: &str = ".bin";
/// A leftover temporary file older than this was abandoned by a crashed write.
const STALE_TEMPORARY_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const MAXIMUM_TEMPORARY_NAME_ATTEMPTS: usize = 8;
/// Pages are small; this only caps the up-front allocation for a huge requested limit.
const MAXIMUM_PAGE_PREALLOCATION: usize = 1024;

/// The owned, clonable view of a store that moves onto the blocking pool.
#[derive(Clone)]
struct KeyedFiles {
    root: PathBuf,
    max_object_bytes: u64,
}

fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

fn keeps_history(domain: StorageDomain) -> bool {
    matches!(domain, StorageDomain::PrivateMedia | StorageDomain::Backups)
}

fn io_error(error: &std::io::Error) -> ObjectStoreError {
    match error.kind() {
        ErrorKind::NotFound => ObjectStoreError::NotFound,
        ErrorKind::PermissionDenied => ObjectStoreError::Denied,
        _ => ObjectStoreError::Unavailable,
    }
}

fn version_file_name(generation: u64) -> String {
    format!("{generation:020}{VERSION_SUFFIX}")
}

fn version_of(generation: u64) -> Result<ObjectVersion, ObjectStoreError> {
    ObjectVersion::new(generation.to_string()).ok_or(ObjectStoreError::Unavailable)
}

fn generation_of(version: &ObjectVersion) -> Option<u64> {
    version.as_str().parse().ok()
}

impl KeyedFiles {
    fn object_dir(&self, key: &ObjectKey) -> PathBuf {
        let mut dir = self.domain_dir(key.domain());
        let mut segments = key.path().split('/').peekable();
        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                dir.push(segment);
            } else {
                dir.push(format!("{segment}{OBJECT_SUFFIX}"));
            }
        }
        dir
    }

    fn domain_dir(&self, domain: StorageDomain) -> PathBuf {
        self.root.join(KEYED_DIRECTORY).join(domain.label())
    }

    /// Ascending generations of the version files in an object directory.
    fn generations(dir: &Path) -> Result<Vec<u64>, ObjectStoreError> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&error)),
        };
        let mut generations = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&error))?;
            let name = entry.file_name();
            let generation = name
                .to_str()
                .and_then(|name| name.strip_suffix(VERSION_SUFFIX))
                .and_then(|stem| stem.parse::<u64>().ok());
            generations.extend(generation);
        }
        generations.sort_unstable();
        Ok(generations)
    }

    fn write_temporary(&self, body: &[u8]) -> Result<PathBuf, ObjectStoreError> {
        let directory = self.root.join(TEMPORARY_DIRECTORY);
        fs::create_dir_all(&directory).map_err(|error| io_error(&error))?;
        for _ in 0..MAXIMUM_TEMPORARY_NAME_ATTEMPTS {
            let path = directory.join(random_name());
            let opened = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path);
            let mut file = match opened {
                Ok(file) => file,
                // A name collision (a leftover or a rare repeat) only needs another name.
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(&error)),
            };
            let written = file.write_all(body).and_then(|()| file.sync_all());
            if let Err(error) = written {
                // The temporary is unreachable by readers; a leftover is reaped at start-up.
                let _removed = fs::remove_file(&path);
                return Err(io_error(&error));
            }
            return Ok(path);
        }
        Err(ObjectStoreError::Unavailable)
    }

    fn put(
        &self,
        key: &ObjectKey,
        current: Option<&ObjectVersion>,
        body: &[u8],
        expected: AssetManifest,
    ) -> Result<StoredObject, ObjectStoreError> {
        if body.len() as u64 > self.max_object_bytes {
            return Err(ObjectStoreError::TooLarge);
        }
        if manifest_of(body) != expected {
            return Err(ObjectStoreError::IntegrityMismatch);
        }
        let dir = self.object_dir(key);
        fs::create_dir_all(&dir).map_err(|error| io_error(&error))?;
        let existing = Self::generations(&dir)?;
        let generation = match (current, existing.last()) {
            (None, Some(_)) => return Err(ObjectStoreError::AlreadyExists),
            (None, None) => 1,
            (Some(required), Some(head)) if generation_of(required) == Some(*head) => head + 1,
            (Some(_), _) => return Err(ObjectStoreError::PreconditionFailed),
        };
        let temporary = self.write_temporary(body)?;
        let linked = fs::hard_link(&temporary, dir.join(version_file_name(generation)));
        // The version file now holds the bytes (or the link lost a race); the temporary is spent.
        let _removed = fs::remove_file(&temporary);
        match linked {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Err(if current.is_none() {
                    ObjectStoreError::AlreadyExists
                } else {
                    ObjectStoreError::PreconditionFailed
                });
            }
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                return Err(ObjectStoreError::Denied);
            }
            // Any other refusal means the filesystem cannot hard-link (FAT, some shares).
            Err(_) => return Err(ObjectStoreError::Unsupported),
        }
        // The link is durable only once its directory entry and the directory's own entry are.
        sync_directory(&dir).map_err(|error| io_error(&error))?;
        if let Some(parent) = dir.parent() {
            sync_directory(parent).map_err(|error| io_error(&error))?;
        }
        if !keeps_history(key.domain()) {
            for older in existing {
                // An older version already gone is the wanted state.
                let _removed = fs::remove_file(dir.join(version_file_name(older)));
            }
        }
        Ok(StoredObject {
            key: key.clone(),
            version: version_of(generation)?,
            manifest: expected,
        })
    }

    fn head(&self, key: &ObjectKey) -> Result<Option<StoredObject>, ObjectStoreError> {
        let dir = self.object_dir(key);
        let Some(head) = Self::generations(&dir)?.last().copied() else {
            return Ok(None);
        };
        let bytes = match fs::read(dir.join(version_file_name(head))) {
            Ok(bytes) => bytes,
            // Deleted between the listing and the read: the key has no current version.
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&error)),
        };
        Ok(Some(StoredObject {
            key: key.clone(),
            version: version_of(head)?,
            manifest: manifest_of(&bytes),
        }))
    }

    fn read(&self, object: &StoredObject) -> Result<Vec<u8>, ObjectStoreError> {
        let generation = generation_of(&object.version).ok_or(ObjectStoreError::NotFound)?;
        let file = fs::File::open(
            self.object_dir(&object.key)
                .join(version_file_name(generation)),
        )
        .map_err(|error| io_error(&error))?;
        let mut bytes = Vec::new();
        // One byte past the recorded length is enough to see that the file grew.
        file.take(object.manifest.byte_len.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| io_error(&error))?;
        if manifest_of(&bytes) != object.manifest {
            return Err(ObjectStoreError::IntegrityMismatch);
        }
        Ok(bytes)
    }

    fn list_versions(&self, key: &ObjectKey) -> Result<Vec<ObjectVersion>, ObjectStoreError> {
        Self::generations(&self.object_dir(key))?
            .into_iter()
            .map(version_of)
            .collect()
    }

    fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
    ) -> Result<(), ObjectStoreError> {
        let Some(generation) = generation_of(version) else {
            return Ok(());
        };
        let dir = self.object_dir(key);
        match fs::remove_file(dir.join(version_file_name(generation))) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&error)),
        }
        // Fails while versions remain or when a writer just added one; either outcome is correct.
        let _removed = fs::remove_dir(&dir);
        Ok(())
    }

    fn list_page(
        &self,
        prefix: &ObjectPrefix,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ListPage, ObjectStoreError> {
        let wanted = limit.saturating_add(1);
        let mut found = Vec::with_capacity(wanted.min(MAXIMUM_PAGE_PREALLOCATION));
        let bounds = ListBounds {
            prefix: prefix.path(),
            after,
            wanted,
        };
        Self::collect_objects(&self.domain_dir(prefix.domain()), "", &bounds, &mut found)?;
        let mut objects = Vec::with_capacity(found.len());
        for (path, dir) in found {
            let Some(version) = Self::generations(&dir)?.last().copied() else {
                continue;
            };
            let length = match fs::metadata(dir.join(version_file_name(version))) {
                Ok(metadata) => metadata.len(),
                // Deleted between the listing and the stat: no longer listed.
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(&error)),
            };
            objects.push(ListedObject {
                path,
                version: version_of(version)?,
                byte_len: length,
            });
        }
        let next_after = if objects.len() > limit {
            objects.truncate(limit);
            objects.last().map(|last| last.path.clone())
        } else {
            None
        };
        Ok(ListPage {
            objects,
            next_after,
        })
    }

    /// Walks the tree in key-path order and stops once `bounds.wanted` matching objects are found,
    /// skipping subtrees that cannot hold a match, so a page costs its own size, not the domain's.
    fn collect_objects(
        dir: &Path,
        relative: &str,
        bounds: &ListBounds<'_>,
        found: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), ObjectStoreError> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(&error)),
        };
        let mut children = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&error))?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !entry
                .file_type()
                .map_err(|error| io_error(&error))?
                .is_dir()
            {
                continue;
            }
            // A directory sorts as `name/` and an object as its stem, which is key-path order.
            let (sort_key, is_object) = match name.strip_suffix(OBJECT_SUFFIX) {
                Some(stem) => (stem.to_owned(), true),
                None => (format!("{name}/"), false),
            };
            children.push((sort_key, is_object, entry.path()));
        }
        children.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        for (sort_key, is_object, path) in children {
            if found.len() >= bounds.wanted {
                return Ok(());
            }
            let full = format!("{relative}{sort_key}");
            if is_object {
                if full.starts_with(bounds.prefix)
                    && bounds.after.is_none_or(|after| full.as_str() > after)
                {
                    found.push((full, path));
                }
            } else if bounds.can_hold_match(&full) {
                Self::collect_objects(&path, &full, bounds, found)?;
            }
        }
        Ok(())
    }
}

/// The page being listed: the prefix, the cursor and how many matches to find.
struct ListBounds<'a> {
    prefix: &'a str,
    after: Option<&'a str>,
    wanted: usize,
}

impl ListBounds<'_> {
    /// Whether any key path starting with `subtree` (ending in `/`) can match the prefix and lie
    /// after the cursor.
    fn can_hold_match(&self, subtree: &str) -> bool {
        let overlaps_prefix = subtree.starts_with(self.prefix) || self.prefix.starts_with(subtree);
        // Every path under `subtree` starts with it, so all are before the cursor exactly when the
        // cursor does not start with it and sorts after it.
        let is_wholly_before_cursor = self
            .after
            .is_some_and(|after| !after.starts_with(subtree) && after > subtree);
        overlaps_prefix && !is_wholly_before_cursor
    }
}

/// An unpredictable 16-hex-digit name: process-random hash keys over a counter, the process id and
/// the clock, so concurrent writers and crashed leftovers never share a name.
fn random_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    // Relaxed: only the uniqueness of the counter matters.
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    hasher.write_u32(std::process::id());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    hasher.write_u128(nanos);
    format!("{:016x}", hasher.finish())
}

/// Removes temporary files a crashed write left behind. Best effort at start-up: a file that
/// cannot be inspected or removed is left for the next start, and a younger one may belong to a
/// write in flight in another process.
pub(crate) fn reap_stale_temporaries(root: &Path) -> usize {
    let Ok(entries) = fs::read_dir(root.join(TEMPORARY_DIRECTORY)) else {
        return 0;
    };
    let mut reaped = 0;
    for entry in entries.flatten() {
        let is_stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_TEMPORARY_AGE);
        if is_stale && fs::remove_file(entry.path()).is_ok() {
            reaped += 1;
        }
    }
    if reaped > 0 {
        df_observe::record_audit_line("ASSET", format!("ASSET temp_reaped count={reaped}"));
    }
    reaped
}

fn domain_versioning(domain: StorageDomain) -> VersioningStatus {
    if keeps_history(domain) {
        VersioningStatus::Enabled
    } else {
        VersioningStatus::NeverEnabled
    }
}

/// Runs file work on the blocking pool; a pool failure is the store being unavailable.
async fn blocking<T: Send + 'static>(
    deadline: Deadline,
    work: impl FnOnce() -> Result<T, ObjectStoreError> + Send + 'static,
) -> Result<T, ObjectStoreError> {
    if deadline.has_passed() {
        return Err(ObjectStoreError::DeadlineExceeded);
    }
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ObjectStoreError::Unavailable)?
}

impl NativeFileStore {
    fn keyed(&self) -> KeyedFiles {
        KeyedFiles {
            root: self.root().to_path_buf(),
            max_object_bytes: self.max_object_bytes(),
        }
    }
}

impl ObjectStore for NativeFileStore {
    fn conditional_writes(&self) -> ConditionalWrites {
        ConditionalWrites {
            create_if_absent: true,
            replace_if_match: true,
        }
    }

    async fn put_new(
        &self,
        key: &ObjectKey,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        let (files, key, body) = (self.keyed(), key.clone(), body.to_vec());
        blocking(deadline, move || files.put(&key, None, &body, expected)).await
    }

    async fn put_if_match(
        &self,
        key: &ObjectKey,
        current: &ObjectVersion,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        let (files, key, current, body) =
            (self.keyed(), key.clone(), current.clone(), body.to_vec());
        blocking(deadline, move || {
            files.put(&key, Some(&current), &body, expected)
        })
        .await
    }

    async fn head(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Option<StoredObject>, ObjectStoreError> {
        let (files, key) = (self.keyed(), key.clone());
        blocking(deadline, move || files.head(&key)).await
    }

    async fn get_version(
        &self,
        object: &StoredObject,
        deadline: Deadline,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let (files, object) = (self.keyed(), object.clone());
        blocking(deadline, move || files.read(&object)).await
    }

    async fn list_versions(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Vec<ObjectVersion>, VersionListing> {
        let (files, key) = (self.keyed(), key.clone());
        blocking(deadline, move || files.list_versions(&key))
            .await
            .map_err(VersionListing::Failed)
    }

    async fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
        deadline: Deadline,
    ) -> Result<(), ObjectStoreError> {
        let (files, key, version) = (self.keyed(), key.clone(), version.clone());
        blocking(deadline, move || files.delete_version(&key, &version)).await
    }

    async fn list_page(
        &self,
        prefix: &ObjectPrefix,
        after: Option<&str>,
        limit: usize,
        deadline: Deadline,
    ) -> Result<ListPage, ObjectStoreError> {
        let (files, prefix, after) = (self.keyed(), prefix.clone(), after.map(str::to_owned));
        blocking(deadline, move || {
            files.list_page(&prefix, after.as_deref(), limit)
        })
        .await
    }

    async fn versioning_status(
        &self,
        domain: StorageDomain,
        deadline: Deadline,
    ) -> Result<VersioningStatus, ObjectStoreError> {
        if deadline.has_passed() {
            return Err(ObjectStoreError::DeadlineExceeded);
        }
        Ok(domain_versioning(domain))
    }
}
