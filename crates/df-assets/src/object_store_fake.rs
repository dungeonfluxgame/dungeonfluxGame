//! A faithful in-memory object store for tests: per-domain versioning, conditional writes as
//! configured, size and capacity bounds, deadlines, and injectable faults (a failed call, a write
//! whose acknowledgement is lost, a missed deadline) plus behind-the-catalog changes (an object
//! removed by a lifecycle rule, bytes altered in place).

use crate::AssetManifest;
use crate::object_store::{
    ConditionalWrites, Deadline, ListPage, ListedObject, ObjectKey, ObjectPrefix, ObjectStore,
    ObjectStoreError, ObjectVersion, StorageDomain, StoredObject, VersionListing, VersioningStatus,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// One injected failure, consumed by the next store call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    /// The call fails with this error and changes nothing.
    Fail(ObjectStoreError),
    /// A write takes effect but the caller receives `Unknown`; a read returns `Unknown`.
    LoseAcknowledgement,
    /// The call changes nothing and returns `DeadlineExceeded`.
    PastDeadline,
}

/// Bounds and behaviour of one fake store.
#[derive(Clone, Copy, Debug)]
pub struct FakeConfig {
    pub conditional_writes: ConditionalWrites,
    pub max_object_bytes: u64,
    pub capacity_bytes: u64,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            conditional_writes: ConditionalWrites {
                create_if_absent: true,
                replace_if_match: true,
            },
            max_object_bytes: 64 * 1024 * 1024,
            capacity_bytes: 1024 * 1024 * 1024,
        }
    }
}

struct FakeVersion {
    version: ObjectVersion,
    bytes: Vec<u8>,
    manifest: AssetManifest,
}

struct FakeState {
    objects: BTreeMap<(StorageDomain, String), Vec<FakeVersion>>,
    versioning: BTreeMap<StorageDomain, VersioningStatus>,
    faults: VecDeque<Fault>,
    next_version: u64,
    stored_bytes: u64,
}

pub struct FakeObjectStore {
    config: FakeConfig,
    state: Mutex<FakeState>,
}

/// The fake's data stay consistent when a test thread panics while holding the lock.
fn locked(state: &Mutex<FakeState>) -> MutexGuard<'_, FakeState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

enum FaultEffect {
    None,
    LoseAcknowledgement,
}

impl FakeObjectStore {
    /// Private media and backups are versioned; the other domains never were (D14 for exports).
    pub fn new(config: FakeConfig) -> Self {
        let versioning = StorageDomain::ALL
            .into_iter()
            .map(|domain| {
                let status = match domain {
                    StorageDomain::PrivateMedia | StorageDomain::Backups => {
                        VersioningStatus::Enabled
                    }
                    _ => VersioningStatus::NeverEnabled,
                };
                (domain, status)
            })
            .collect();
        Self {
            config,
            state: Mutex::new(FakeState {
                objects: BTreeMap::new(),
                versioning,
                faults: VecDeque::new(),
                next_version: 0,
                stored_bytes: 0,
            }),
        }
    }

    pub fn push_fault(&self, fault: Fault) {
        locked(&self.state).faults.push_back(fault);
    }

    pub fn set_versioning(&self, domain: StorageDomain, status: VersioningStatus) {
        locked(&self.state).versioning.insert(domain, status);
    }

    /// Removes every version of `key` without the catalog knowing (a lifecycle expiry).
    pub fn remove_behind(&self, key: &ObjectKey) {
        let mut state = locked(&self.state);
        if let Some(versions) = state.objects.remove(&(key.domain(), key.path().to_owned())) {
            let freed: u64 = versions
                .iter()
                .map(|version| version.manifest.byte_len)
                .sum();
            state.stored_bytes -= freed;
        }
    }

    /// Flips one byte of the current version in place, keeping its recorded manifest.
    pub fn corrupt_behind(&self, key: &ObjectKey) {
        let mut state = locked(&self.state);
        if let Some(current) = state
            .objects
            .get_mut(&(key.domain(), key.path().to_owned()))
            .and_then(|versions| versions.last_mut())
        {
            match current.bytes.first_mut() {
                Some(byte) => *byte ^= 0xff,
                None => current.bytes.push(0),
            }
        }
    }

    /// Number of keys (not versions) currently stored in `domain`.
    pub fn key_count(&self, domain: StorageDomain) -> usize {
        locked(&self.state)
            .objects
            .keys()
            .filter(|(stored_domain, _)| *stored_domain == domain)
            .count()
    }

    fn begin(&self, deadline: Deadline) -> Result<FaultEffect, ObjectStoreError> {
        if deadline.has_passed() {
            return Err(ObjectStoreError::DeadlineExceeded);
        }
        match locked(&self.state).faults.pop_front() {
            None => Ok(FaultEffect::None),
            Some(Fault::Fail(error)) => Err(error),
            Some(Fault::PastDeadline) => Err(ObjectStoreError::DeadlineExceeded),
            Some(Fault::LoseAcknowledgement) => Ok(FaultEffect::LoseAcknowledgement),
        }
    }

    fn write(
        &self,
        key: &ObjectKey,
        current: Option<&ObjectVersion>,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        let effect = self.begin(deadline)?;
        if body.len() as u64 > self.config.max_object_bytes {
            return Err(ObjectStoreError::TooLarge);
        }
        if manifest_of(body) != expected {
            return Err(ObjectStoreError::IntegrityMismatch);
        }
        let mut state = locked(&self.state);
        let slot = (key.domain(), key.path().to_owned());
        let existing = state
            .objects
            .get(&slot)
            .and_then(|versions| versions.last());
        match current {
            None if existing.is_some() && self.config.conditional_writes.create_if_absent => {
                return Err(ObjectStoreError::AlreadyExists);
            }
            Some(required)
                if self.config.conditional_writes.replace_if_match
                    && existing.map(|version| &version.version) != Some(required) =>
            {
                return Err(ObjectStoreError::PreconditionFailed);
            }
            _ => {}
        }
        let keeps_history = state.versioning.get(&key.domain()) == Some(&VersioningStatus::Enabled);
        let replaced_bytes = match (keeps_history, existing) {
            (false, Some(version)) => version.manifest.byte_len,
            _ => 0,
        };
        let stored_after = state.stored_bytes - replaced_bytes + expected.byte_len;
        if stored_after > self.config.capacity_bytes {
            return Err(ObjectStoreError::StorageFull);
        }
        state.next_version += 1;
        let version = ObjectVersion::new(format!("v{}", state.next_version))
            .ok_or(ObjectStoreError::Unavailable)?;
        state.stored_bytes = stored_after;
        let versions = state.objects.entry(slot).or_default();
        if !keeps_history {
            versions.clear();
        }
        versions.push(FakeVersion {
            version: version.clone(),
            bytes: body.to_vec(),
            manifest: expected,
        });
        match effect {
            FaultEffect::LoseAcknowledgement => Err(ObjectStoreError::Unknown),
            FaultEffect::None => Ok(StoredObject {
                key: key.clone(),
                version,
                manifest: expected,
            }),
        }
    }
}

impl ObjectStore for FakeObjectStore {
    fn conditional_writes(&self) -> ConditionalWrites {
        self.config.conditional_writes
    }

    async fn put_new(
        &self,
        key: &ObjectKey,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        self.write(key, None, body, expected, deadline)
    }

    async fn put_if_match(
        &self,
        key: &ObjectKey,
        current: &ObjectVersion,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        self.write(key, Some(current), body, expected, deadline)
    }

    async fn head(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Option<StoredObject>, ObjectStoreError> {
        if let FaultEffect::LoseAcknowledgement = self.begin(deadline)? {
            return Err(ObjectStoreError::Unknown);
        }
        let state = locked(&self.state);
        Ok(state
            .objects
            .get(&(key.domain(), key.path().to_owned()))
            .and_then(|versions| versions.last())
            .map(|version| StoredObject {
                key: key.clone(),
                version: version.version.clone(),
                manifest: version.manifest,
            }))
    }

    async fn get_version(
        &self,
        object: &StoredObject,
        deadline: Deadline,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        if let FaultEffect::LoseAcknowledgement = self.begin(deadline)? {
            return Err(ObjectStoreError::Unknown);
        }
        let state = locked(&self.state);
        let stored = state
            .objects
            .get(&(object.key.domain(), object.key.path().to_owned()))
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|version| version.version == object.version)
            })
            .ok_or(ObjectStoreError::NotFound)?;
        if manifest_of(&stored.bytes) != object.manifest {
            return Err(ObjectStoreError::IntegrityMismatch);
        }
        Ok(stored.bytes.clone())
    }

    async fn list_versions(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Vec<ObjectVersion>, VersionListing> {
        if let FaultEffect::LoseAcknowledgement =
            self.begin(deadline).map_err(VersionListing::Failed)?
        {
            return Err(VersionListing::Failed(ObjectStoreError::Unknown));
        }
        let state = locked(&self.state);
        Ok(state
            .objects
            .get(&(key.domain(), key.path().to_owned()))
            .map(|versions| {
                versions
                    .iter()
                    .map(|version| version.version.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
        deadline: Deadline,
    ) -> Result<(), ObjectStoreError> {
        let effect = self.begin(deadline)?;
        let mut state = locked(&self.state);
        let slot = (key.domain(), key.path().to_owned());
        let mut freed = 0;
        let mut now_empty = false;
        if let Some(versions) = state.objects.get_mut(&slot) {
            if let Some(index) = versions
                .iter()
                .position(|stored| &stored.version == version)
            {
                freed = versions.remove(index).manifest.byte_len;
            }
            now_empty = versions.is_empty();
        }
        if now_empty {
            state.objects.remove(&slot);
        }
        state.stored_bytes -= freed;
        match effect {
            FaultEffect::LoseAcknowledgement => Err(ObjectStoreError::Unknown),
            FaultEffect::None => Ok(()),
        }
    }

    async fn list_page(
        &self,
        prefix: &ObjectPrefix,
        after: Option<&str>,
        limit: usize,
        deadline: Deadline,
    ) -> Result<ListPage, ObjectStoreError> {
        if let FaultEffect::LoseAcknowledgement = self.begin(deadline)? {
            return Err(ObjectStoreError::Unknown);
        }
        let state = locked(&self.state);
        let mut objects: Vec<ListedObject> = state
            .objects
            .iter()
            .filter(|((domain, path), _)| {
                *domain == prefix.domain()
                    && path.starts_with(prefix.path())
                    && after.is_none_or(|after| path.as_str() > after)
            })
            .filter_map(|((_, path), versions)| {
                versions.last().map(|current| ListedObject {
                    path: path.clone(),
                    version: current.version.clone(),
                    byte_len: current.manifest.byte_len,
                })
            })
            .take(limit.saturating_add(1))
            .collect();
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

    async fn versioning_status(
        &self,
        domain: StorageDomain,
        deadline: Deadline,
    ) -> Result<VersioningStatus, ObjectStoreError> {
        if let FaultEffect::LoseAcknowledgement = self.begin(deadline)? {
            return Err(ObjectStoreError::Unknown);
        }
        Ok(locked(&self.state)
            .versioning
            .get(&domain)
            .copied()
            .unwrap_or(VersioningStatus::NeverEnabled))
    }
}
