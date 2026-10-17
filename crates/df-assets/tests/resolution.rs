#![cfg(not(target_arch = "wasm32"))]

use df_assets::{
    AccessFailure, AssetManifest, AssetMetadataStore, AssetReadAuthority, AssetResolver,
    AssetStore, AuthorizedBinding, ByteRange, ChunkOutcome, DurableObject, MetadataFailure,
    NativeFileStore, Publication, PublicationStatus, PublishedBinding, RangeError, StoreError,
    publish,
};
use df_model::checkpoint::{AssetKind, AssetReference, ContentDigest};
use df_observe::OperationContext;
use df_types::{OperationId, RevisionLabel};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_ROOT: AtomicUsize = AtomicUsize::new(0);
const CONTENT: &[u8] = b"owned complete backing fixture";

fn root() -> PathBuf {
    let parent = PathBuf::from(std::env::var_os("TMPDIR").expect("owned TMPDIR is required"));
    let path = parent.join(format!(
        "df-assets-resolution-{}-{}",
        std::process::id(),
        TEST_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: "asset-resolution-fixture".to_owned(),
    }
}

fn manifest(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: u64::try_from(bytes.len()).unwrap(),
        sha256: Sha256::digest(bytes).into(),
    }
}

fn reference(bytes: &[u8]) -> AssetReference {
    let bytes = manifest(bytes);
    AssetReference {
        key: RevisionLabel::new(Some("fixture-v1")).unwrap(),
        digest: ContentDigest(bytes.sha256),
        byte_length: bytes.byte_len,
        kind: AssetKind::TacticalGeometry,
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Caller(u8);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Purpose {
    Display,
    Export,
}

const CALLER: Caller = Caller(1);

#[derive(Default)]
struct Metadata {
    published: RefCell<Option<PublishedBinding<AssetReference>>>,
    basis: Cell<u64>,
    gate: Cell<Option<AccessFailure>>,
    calls: Cell<usize>,
    stale_at_call: Cell<Option<usize>>,
}

// Test-only current-authority fixture; no production principal or RightsGrant is fabricated.
impl AssetReadAuthority for Metadata {
    type Caller = Caller;
    type Purpose = Purpose;
    type Basis = u64;

    fn current_authorized_binding(
        &self,
        caller: &Caller,
        version: &RevisionLabel,
        purpose: &Purpose,
        _range: ByteRange,
    ) -> Result<AuthorizedBinding<AssetReference, u64>, AccessFailure> {
        let calls = self.calls.get() + 1;
        self.calls.set(calls);
        if self.stale_at_call.get() == Some(calls) {
            self.basis.set(self.basis.get() + 1);
        }
        if let Some(failure) = self.gate.get() {
            return Err(failure);
        }
        if caller != &CALLER || purpose != &Purpose::Display {
            return Err(AccessFailure::Denied);
        }
        let published = self
            .lookup(version)
            .map_err(|_| AccessFailure::Unavailable)?
            .ok_or(AccessFailure::Absent)?;
        Ok(AuthorizedBinding {
            published,
            basis: self.basis.get(),
            max_chunk_bytes: 4,
        })
    }
}

impl AssetMetadataStore for Metadata {
    type Version = RevisionLabel;
    type Metadata = AssetReference;

    fn lookup(
        &self,
        version: &RevisionLabel,
    ) -> Result<Option<PublishedBinding<AssetReference>>, MetadataFailure> {
        Ok(self
            .published
            .borrow()
            .as_ref()
            .filter(|binding| &binding.metadata.key == version)
            .cloned())
    }

    fn publish_immutable(
        &self,
        candidate: &Publication<RevisionLabel, AssetReference>,
        object: DurableObject,
    ) -> Result<PublicationStatus, MetadataFailure> {
        let mut published = self.published.borrow_mut();
        if let Some(existing) = published.as_ref() {
            return Ok(
                if existing.metadata == candidate.metadata
                    && existing.bytes == candidate.bytes
                    && existing.object == object
                {
                    PublicationStatus::AlreadyPublished
                } else {
                    PublicationStatus::VersionConflict
                },
            );
        }
        *published = Some(PublishedBinding {
            metadata: candidate.metadata.clone(),
            bytes: candidate.bytes,
            object,
        });
        Ok(PublicationStatus::Published)
    }
}

fn fixture() -> (PathBuf, NativeFileStore, Metadata, AssetReference) {
    let path = root();
    let store = NativeFileStore::new(&path, 1024).unwrap();
    let metadata = Metadata::default();
    let requested = reference(CONTENT);
    let operation = OperationId::from_bytes(&[1; 16]).unwrap();
    let mut input = CONTENT;
    let staged = store.stage(operation, &mut input).unwrap();
    let candidate = Publication {
        operation,
        version: requested.key.clone(),
        metadata: requested.clone(),
        bytes: manifest(CONTENT),
    };
    assert_eq!(
        publish(&context(), &candidate, &staged, &store, &metadata)
            .unwrap()
            .status,
        PublicationStatus::Published,
    );
    (path, store, metadata, requested)
}

fn object_path(path: &std::path::Path) -> PathBuf {
    let digest = manifest(CONTENT).sha256;
    let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    path.join("objects").join(name)
}

fn full_range(requested: &AssetReference) -> ByteRange {
    ByteRange::new(0, requested.byte_length).unwrap()
}

#[test]
fn published_bytes_resolve_through_current_authority_and_bounded_native_chunks_after_restart() {
    let (path, _, metadata, requested) = fixture();
    let restarted = NativeFileStore::new(&path, 1024).unwrap();
    let context = context();
    let basis = metadata.basis.get();
    let resolver = AssetResolver::new(&metadata, &requested, &basis);
    let mut stream = resolver
        .open_native(
            &context,
            &restarted,
            &CALLER,
            &Purpose::Display,
            full_range(&requested),
        )
        .unwrap();
    let mut output = [0; 9];
    let mut received = Vec::with_capacity(CONTENT.len());
    while let ChunkOutcome::Bytes(length) = stream.read_chunk(&mut output).unwrap() {
        assert!((1..=4).contains(&length));
        received.extend_from_slice(&output[..length]);
    }
    assert_eq!(received, CONTENT);
    assert!(metadata.calls.get() > 2);
}

#[test]
fn complete_orphaned_backing_without_publication_remains_absent() {
    let path = root();
    let store = NativeFileStore::new(path, 1024).unwrap();
    let operation = OperationId::from_bytes(&[2; 16]).unwrap();
    let mut input = CONTENT;
    let staged = store.stage(operation, &mut input).unwrap();
    store
        .verify_and_promote(&staged, manifest(CONTENT))
        .unwrap();
    let metadata = Metadata::default();
    let requested = reference(CONTENT);
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Access(AccessFailure::Absent)),
    ));
}

#[test]
fn every_canonical_identity_field_must_match_even_when_bytes_exist() {
    let (_, store, metadata, requested) = fixture();
    let mut changed_key = requested.clone();
    changed_key.key = RevisionLabel::new(Some("fixture-v2")).unwrap();
    let mut changed_kind = requested.clone();
    changed_kind.kind = AssetKind::Audio;
    let mut changed_digest = requested.clone();
    changed_digest.digest.0[0] ^= 1;
    let mut changed_length = requested.clone();
    changed_length.byte_length += 1;
    let context = context();
    for (changed, expected) in [
        (changed_key, AccessFailure::Absent),
        (changed_kind, AccessFailure::Stale),
        (changed_digest, AccessFailure::Stale),
        (changed_length, AccessFailure::Stale),
    ] {
        // Test-only malformed records exercise the canonical resolver; immutable production
        // metadata must separately refuse mutation at its publication boundary.
        metadata.published.borrow_mut().as_mut().unwrap().metadata = changed;
        let resolver = AssetResolver::new(&metadata, &requested, &0);
        let result = resolver.open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested),
        );
        assert!(matches!(
            result,
            Err(RangeError::Access(actual)) if actual == expected,
        ));
    }
}

#[test]
fn canonical_manifest_and_object_disagreement_fail_with_typed_errors() {
    let (_, store, metadata, requested) = fixture();
    let original = metadata.published.borrow().as_ref().unwrap().clone();
    let mut malformed = original.clone();
    malformed.bytes.sha256[0] ^= 1;
    *metadata.published.borrow_mut() = Some(malformed);
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Access(AccessFailure::Unavailable)),
    ));
    let mut malformed = original;
    malformed.object = DurableObject::from_manifest(manifest(b"different"));
    *metadata.published.borrow_mut() = Some(malformed);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingIntegrity)),
    ));
}

#[test]
fn a_matching_cache_cannot_replace_the_selected_durable_backing() {
    let (_, _, metadata, requested) = fixture();
    let selected_root = root();
    let selected = NativeFileStore::new(&selected_root, 1024).unwrap();
    fs::write(selected_root.join("disposable-cache"), CONTENT).unwrap();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &selected,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingMissing)),
    ));
}

#[cfg(unix)]
#[test]
fn a_matching_cache_symlink_cannot_replace_a_published_object() {
    use std::os::unix::fs::symlink;

    let (path, store, metadata, requested) = fixture();
    let cache = root().join("cached-copy");
    fs::write(&cache, CONTENT).unwrap();
    fs::remove_file(object_path(&path)).unwrap();
    symlink(&cache, object_path(&path)).unwrap();

    let restarted = NativeFileStore::new(&path, 1024).unwrap();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &restarted,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingIntegrity)),
    ));
    assert!(matches!(
        store.confirm(DurableObject::from_manifest(manifest(CONTENT))),
        Err(StoreError::BackingIntegrity),
    ));
    assert_eq!(fs::read(&cache).unwrap(), CONTENT);
}

#[cfg(unix)]
#[test]
fn fifo_backing_refuses_without_waiting_for_a_writer() {
    use std::process::Command;
    use std::time::{Duration, Instant};

    if std::env::var_os("DF_ASSETS_FIFO_CHILD").is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("fifo_backing_refuses_without_waiting_for_a_writer")
            .env("DF_ASSETS_FIFO_CHILD", "1")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("opening a FIFO backing blocked");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    let (path, store, metadata, requested) = fixture();
    let object = object_path(&path);
    fs::remove_file(&object).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&object)
            .status()
            .unwrap()
            .success()
    );
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingIntegrity)),
    ));
    assert!(matches!(
        store.confirm(DurableObject::from_manifest(manifest(CONTENT))),
        Err(StoreError::BackingIntegrity),
    ));
}

#[cfg(unix)]
#[test]
fn a_matching_cache_directory_cannot_replace_the_objects_directory() {
    use std::os::unix::fs::symlink;

    let (path, store, metadata, requested) = fixture();
    let object = object_path(&path);
    let cache = root();
    fs::copy(&object, cache.join(object.file_name().unwrap())).unwrap();
    fs::rename(path.join("objects"), path.join("original-objects")).unwrap();
    symlink(&cache, path.join("objects")).unwrap();

    assert!(matches!(
        NativeFileStore::new(&path, 1024),
        Err(StoreError::BackingIntegrity),
    ));
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingIntegrity)),
    ));
    assert!(matches!(
        store.confirm(DurableObject::from_manifest(manifest(CONTENT))),
        Err(StoreError::BackingIntegrity),
    ));
}

#[test]
fn missing_published_object_is_unavailable_and_never_returns_a_stream() {
    let (path, store, metadata, requested) = fixture();
    fs::rename(object_path(&path), path.join("retained-elsewhere")).unwrap();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingMissing)),
    ));
}

#[test]
fn truncated_corrupt_and_extended_backing_never_resolve() {
    let mut corrupt = CONTENT.to_vec();
    corrupt[0] ^= 1;
    let mut extended = CONTENT.to_vec();
    extended.push(0);
    for replacement in [CONTENT[..CONTENT.len() - 1].to_vec(), corrupt, extended] {
        let (path, store, metadata, requested) = fixture();
        let replacement_path = path.join("replacement-fixture");
        fs::write(&replacement_path, replacement).unwrap();
        let object = object_path(&path);
        fs::remove_file(&object).unwrap();
        fs::rename(replacement_path, object).unwrap();
        let resolver = AssetResolver::new(&metadata, &requested, &0);
        assert!(matches!(
            resolver.open_native(
                &context(),
                &store,
                &CALLER,
                &Purpose::Display,
                full_range(&requested)
            ),
            Err(RangeError::Store(StoreError::BackingIntegrity)),
        ));
    }
}

#[test]
fn capacity_refuses_before_missing_object_io() {
    let (path, _, metadata, requested) = fixture();
    fs::rename(object_path(&path), path.join("retained-elsewhere")).unwrap();
    let bounded = NativeFileStore::new(&path, requested.byte_length - 1).unwrap();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    assert!(matches!(
        resolver.open_native(
            &context(),
            &bounded,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::Capacity)),
    ));
}

#[test]
fn same_path_replacement_cannot_change_the_already_verified_stream() {
    let (path, store, metadata, requested) = fixture();
    let context = context();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    let mut stream = resolver
        .open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested),
        )
        .unwrap();
    let mut replacement = CONTENT.to_vec();
    replacement[0] ^= 1;
    let replacement_path = path.join("replacement-fixture");
    fs::write(&replacement_path, replacement).unwrap();
    let object = object_path(&path);
    fs::remove_file(&object).unwrap();
    fs::rename(replacement_path, object).unwrap();
    let mut output = [0; 4];
    let mut received = Vec::with_capacity(CONTENT.len());
    while let ChunkOutcome::Bytes(length) = stream.read_chunk(&mut output).unwrap() {
        received.extend_from_slice(&output[..length]);
    }
    assert_eq!(received, CONTENT);
    assert!(matches!(
        resolver.open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Store(StoreError::BackingIntegrity)),
    ));
}

#[test]
fn stale_basis_during_open_and_between_chunks_fails_before_disclosure() {
    let (_, store, metadata, requested) = fixture();
    let context = context();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    metadata.stale_at_call.set(Some(2));
    assert!(matches!(
        resolver.open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Access(AccessFailure::Stale)),
    ));
    metadata.stale_at_call.set(None);
    metadata.basis.set(0);
    let mut stream = resolver
        .open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested),
        )
        .unwrap();
    let mut output = [0xa5; 8];
    assert_eq!(
        stream.read_chunk(&mut output).unwrap(),
        ChunkOutcome::Bytes(4)
    );
    metadata.basis.set(1);
    output.fill(0xa5);
    assert!(matches!(
        stream.read_chunk(&mut output),
        Err(RangeError::Access(AccessFailure::Stale))
    ));
    assert_eq!(output, [0xa5; 8]);
    assert!(matches!(
        stream.read_chunk(&mut output),
        Err(RangeError::Terminated)
    ));
}

#[test]
fn current_caller_purpose_and_unavailable_authority_are_enforced() {
    let (_, store, metadata, requested) = fixture();
    let context = context();
    let resolver = AssetResolver::new(&metadata, &requested, &0);
    for (caller, purpose) in [(Caller(2), Purpose::Display), (CALLER, Purpose::Export)] {
        assert!(matches!(
            resolver.open_native(&context, &store, &caller, &purpose, full_range(&requested)),
            Err(RangeError::Access(AccessFailure::Denied)),
        ));
    }
    metadata.gate.set(Some(AccessFailure::Unavailable));
    assert!(matches!(
        resolver.open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested)
        ),
        Err(RangeError::Access(AccessFailure::Unavailable)),
    ));
    metadata.gate.set(None);
    let mut stream = resolver
        .open_native(
            &context,
            &store,
            &CALLER,
            &Purpose::Display,
            full_range(&requested),
        )
        .unwrap();
    metadata.gate.set(Some(AccessFailure::Unavailable));
    let mut output = [0xa5; 8];
    assert!(matches!(
        stream.read_chunk(&mut output),
        Err(RangeError::Access(AccessFailure::Unavailable))
    ));
    assert_eq!(output, [0xa5; 8]);
}
