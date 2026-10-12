#![cfg(not(target_arch = "wasm32"))]

use df_assets::{
    AccessFailure, AssetManifest, AssetMetadataStore, AssetReadAuthority, AssetResolutionError,
    AssetStore, AssetUnavailable, AuthorizedBinding, ByteRange, ChunkOutcome, DurableObject,
    MetadataFailure, NativeFileStore, Publication, PublicationStatus, PublishedBinding, RangeError,
    SelectedAsset, StaleAsset, StoreError, publish,
};
use df_client::cache::{AssetCache, CacheError, CacheKey, CacheLimits, CacheScope};
use df_model::checkpoint::{
    AssetKind, AssetReference, AssetRequestKey, AudienceScope, ContentDigest, RecordId,
};
use df_observe::OperationContext;
use df_render::{
    FlatScene, ImageDecodeError, ImageDecodeLimits, Point, PresentationOutcome, ResourceError,
    ResourceLifecycle, ResourceLifecycleError, ResourceLimits, SceneRenderer, Viewport,
    WorkOutcome,
};
use df_types::{
    ClientBindingId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::fs;
// Scratch directory modes are a Unix-only property.
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

// Same complete 1x1 RGBA8 PNG as the existing renderer lifecycle/browser codec fixture.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 16, 73, 68, 65, 84, 120, 1, 1, 5, 0, 250, 255, 0, 16, 32, 48,
    255, 2, 4, 1, 96, 145, 5, 159, 157, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const PIXEL: [u8; 4] = [16, 32, 48, 255];
static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence)
}

fn scope() -> CacheScope {
    CacheScope {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        binding: ClientBindingId::from_bytes(&[3; 16]).unwrap(),
    }
}

fn limits() -> CacheLimits {
    CacheLimits {
        max_assets: 1,
        max_pending: 1,
        max_leases: 1,
        max_bytes: PNG.len(),
    }
}

fn decode_limits() -> ImageDecodeLimits {
    ImageDecodeLimits {
        max_encoded_bytes: PNG.len(),
        max_dimension: 1,
        max_decoded_bytes: 4,
        max_work_bytes: 512,
    }
}

fn request() -> AssetRequestKey {
    AssetRequestKey {
        schema: 1,
        source: ContentDigest([1; 32]),
        moment: RecordId::from_bytes(&[2; 16]).unwrap(),
        identity: label("image-identity-1"),
        style: label("prepared-style-1"),
        voice: None,
        provider: label("prepared"),
        model: label("prepared-1"),
        format: label("png-1"),
        references: Vec::new(),
        audience: AudienceScope::Shared,
        parameters: label("parameters-1"),
    }
}

#[derive(Clone, Eq, PartialEq)]
enum Purpose {
    Display,
    Export,
}

// This local fixture drives the existing authority port, never issues production grants.
struct Authority {
    published: RefCell<Option<PublishedBinding<AssetReference>>>,
    basis: Cell<u64>,
    refusal: Cell<Option<AccessFailure>>,
    calls: Cell<usize>,
    change_basis_at: Cell<Option<usize>>,
}

impl AssetMetadataStore for Authority {
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
        publication: &Publication<RevisionLabel, AssetReference>,
        object: DurableObject,
    ) -> Result<PublicationStatus, MetadataFailure> {
        let mut current = self.published.borrow_mut();
        if let Some(previous) = current.as_ref() {
            return Ok(
                if previous.metadata == publication.metadata
                    && previous.bytes == publication.bytes
                    && previous.object == object
                {
                    PublicationStatus::AlreadyPublished
                } else {
                    PublicationStatus::VersionConflict
                },
            );
        }
        *current = Some(PublishedBinding {
            metadata: publication.metadata.clone(),
            bytes: publication.bytes,
            object,
        });
        Ok(PublicationStatus::Published)
    }
}

impl AssetReadAuthority for Authority {
    type Caller = u8;
    type Purpose = Purpose;
    type Basis = u64;

    fn current_authorized_binding(
        &self,
        caller: &u8,
        version: &RevisionLabel,
        purpose: &Purpose,
        range: ByteRange,
    ) -> Result<AuthorizedBinding<AssetReference, u64>, AccessFailure> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        if let Some(refusal) = self.refusal.get() {
            return Err(refusal);
        }
        if *caller != 7 || *purpose != Purpose::Display {
            return Err(AccessFailure::Denied);
        }
        let published = self
            .lookup(version)
            .map_err(|_| AccessFailure::Unavailable)?
            .ok_or(AccessFailure::Absent)?;
        assert_eq!(range, ByteRange::new(0, PNG.len() as u64).unwrap());
        if self.change_basis_at.get() == Some(call) {
            self.basis.set(2);
        }
        Ok(AuthorizedBinding {
            published,
            basis: self.basis.get(),
            max_chunk_bytes: 13,
        })
    }
}

struct Fixture {
    root: PathBuf,
    store: NativeFileStore,
    authority: Authority,
    reference: AssetReference,
    request: AssetRequestKey,
    basis: u64,
    context: OperationContext,
}

impl Fixture {
    fn new() -> Self {
        let temporary = PathBuf::from(std::env::var_os("TMPDIR").expect("owned TMPDIR required"));
        assert!(temporary.is_absolute() && temporary.is_dir() && !temporary.is_symlink());
        let root = temporary.join(format!(
            "asset-outcomes-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        #[cfg(unix)]
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        #[cfg(not(unix))]
        fs::DirBuilder::new().create(&root).unwrap();
        let store = NativeFileStore::new(&root, PNG.len() as u64).unwrap();
        let authority = Authority {
            published: RefCell::new(None),
            basis: Cell::new(1),
            refusal: Cell::new(None),
            calls: Cell::new(0),
            change_basis_at: Cell::new(None),
        };
        let bytes = AssetManifest {
            byte_len: PNG.len() as u64,
            sha256: Sha256::digest(PNG).into(),
        };
        let reference = AssetReference {
            key: label("assets-i04-complete-png-1"),
            digest: ContentDigest(bytes.sha256),
            byte_length: bytes.byte_len,
            kind: AssetKind::Image,
        };
        let context = OperationContext {
            trace_parent: String::new(),
            build: "assets-i04-current-consumer".to_owned(),
        };
        let operation = OperationId::from_bytes(&[4; 16]).unwrap();
        let staged = store.stage(operation, &mut &PNG[..]).unwrap();
        let publication = Publication {
            operation,
            version: reference.key.clone(),
            metadata: reference.clone(),
            bytes,
        };
        assert_eq!(
            publish(&context, &publication, &staged, &store, &authority)
                .unwrap()
                .status,
            PublicationStatus::Published
        );
        Self {
            root,
            store,
            authority,
            reference,
            request: request(),
            basis: 1,
            context,
        }
    }

    fn key(&self) -> CacheKey {
        CacheKey {
            version: self.reference.key.clone(),
            bytes: AssetManifest {
                byte_len: self.reference.byte_length,
                sha256: self.reference.digest.0,
            },
        }
    }

    fn selected(&self) -> SelectedAsset<'_, Authority> {
        SelectedAsset::new(
            &self.request,
            &self.request,
            &self.reference,
            AssetKind::Image,
            &self.authority,
            &self.basis,
        )
        .unwrap()
    }

    fn read_current(&self, caller: u8, purpose: Purpose) -> Result<Vec<u8>, AssetResolutionError> {
        let selected = self.selected();
        let mut stream = selected.open_native(
            &self.context,
            &self.store,
            &caller,
            &purpose,
            ByteRange::new(0, self.reference.byte_length).unwrap(),
            &self.request,
        )?;
        let mut bytes = Vec::with_capacity(PNG.len());
        let mut scratch = [0; 13];
        while let ChunkOutcome::Bytes(length) = stream.read_chunk(&self.request, &mut scratch)? {
            bytes.extend_from_slice(&scratch[..length]);
        }
        Ok(bytes)
    }

    fn lifecycle(&self) -> ResourceLifecycle {
        let scope = scope();
        let mut lifecycle = ResourceLifecycle::from_scene(
            SceneRenderer::new((scope.session, scope.run, scope.binding)),
            scope,
            limits(),
            ResourceLimits {
                max_references: 1,
                max_resident: 1,
                max_pending: 1,
                max_decoded_bytes: 4,
                max_work_bytes: 512,
            },
            label("existing-png-preparation-1"),
        )
        .unwrap();
        let key = self.key();
        lifecycle
            .apply_current(scope, revision(1, 0), std::slice::from_ref(&key))
            .unwrap();
        lifecycle
            .update_scene(
                label("supplied-scene-1"),
                FlatScene {
                    revision: revision(1, 0),
                    viewport: Viewport {
                        origin: Point { x: 0.0, y: 0.0 },
                        width: 1.0,
                        height: 1.0,
                    },
                    label: "Prepared image fixture".to_owned(),
                    layers: Vec::new(),
                    tokens: Vec::new(),
                },
                std::slice::from_ref(&key),
            )
            .unwrap();
        lifecycle
    }

    fn fill(&self, lifecycle: &mut ResourceLifecycle) {
        let token = lifecycle.fetch(&self.key()).unwrap();
        let bytes = self.read_current(7, Purpose::Display).unwrap();
        assert_eq!(bytes, PNG);
        lifecycle.complete_fetch(&token, bytes).unwrap();
    }

    fn backing(&self) -> PathBuf {
        let name = self
            .reference
            .digest
            .0
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.root.join("objects").join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Streams borrow the fixture and have already dropped before its owned store cleanup.
        fs::remove_dir_all(&self.root).unwrap();
        assert!(!self.root.exists());
    }
}

fn assert_empty(lifecycle: &ResourceLifecycle) {
    assert_eq!(lifecycle.resident_bytes(), 0);
    assert_eq!(lifecycle.decoded_bytes(), 0);
    assert_eq!(lifecycle.work_bytes(), 0);
    assert_eq!(lifecycle.lease_count(), 0);
}

#[test]
fn missing_cache_requires_actual_complete_native_bytes_before_guarded_image_admission() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut lifecycle = fixture.lifecycle();
    assert_eq!(lifecycle.contains_bytes(&key), Ok(false));
    assert_eq!(
        lifecycle.prepare_png(&key, decode_limits()).err(),
        Some(ImageDecodeError::MissingBytes)
    );
    assert!(lifecycle.get(&key).unwrap().is_none());
    assert_empty(&lifecycle);

    fixture.fill(&mut lifecycle);
    let input = lifecycle.prepare_png(&key, decode_limits()).unwrap();
    assert_eq!(input.key(), &key);
    let token = lifecycle.begin(&input, || {}).unwrap();
    assert!(lifecycle.work_bytes() > 0);
    // Controlled native codec completion; this is not browser decode or visual evidence.
    let image = input
        .finish_rgba(1, 1, PIXEL.to_vec().into_boxed_slice())
        .unwrap();
    lifecycle.complete(&token, image).unwrap();
    assert_eq!(
        lifecycle.get(&key).unwrap().unwrap().pixels().unwrap(),
        PIXEL
    );
    assert_eq!(lifecycle.resident_bytes(), PNG.len());
    assert_eq!(lifecycle.decoded_bytes(), 4);
    assert_eq!(lifecycle.work_bytes(), 0);
    lifecycle.release(&key).unwrap();
    assert!(lifecycle.get(&key).unwrap().is_none());
    assert_empty(&lifecycle);
    assert_eq!(lifecycle.dispose(), Ok(PresentationOutcome::Disposed));
    assert_eq!(
        lifecycle.dispose(),
        Ok(PresentationOutcome::AlreadyDisposed)
    );
}

#[test]
fn published_cache_only_copy_cannot_bypass_a_refused_fresh_native_resolve() {
    for missing_publication in [true, false] {
        let fixture = Fixture::new();
        let key = fixture.key();
        let mut cache = AssetCache::new(scope(), limits()).unwrap();
        cache
            .apply_current(scope(), revision(1, 0), std::slice::from_ref(&key))
            .unwrap();
        let complete = fixture.read_current(7, Purpose::Display).unwrap();
        let resident = cache.fetch(&key).unwrap();
        cache.complete(&resident, complete).unwrap();
        assert_eq!(cache.get(&key).unwrap().unwrap(), PNG);
        if missing_publication {
            *fixture.authority.published.borrow_mut() = None;
        } else {
            fs::remove_file(fixture.backing()).unwrap();
        }
        let pending = cache.fetch(&key).unwrap();
        let refusal = fixture.read_current(7, Purpose::Display).unwrap_err();
        if missing_publication {
            assert!(matches!(
                refusal,
                AssetResolutionError::Unavailable(AssetUnavailable::NotPublished)
            ));
        } else {
            assert!(matches!(
                refusal,
                AssetResolutionError::Unavailable(AssetUnavailable::BackingMissing)
            ));
        }
        cache.cancel(&pending).unwrap();
        assert_eq!(cache.pending_count(), 0);
        assert_eq!(cache.lease_count(), 0);
        // Previously delivered bytes are not a receipt for a new native disclosure.
        assert_eq!(cache.get(&key).unwrap().unwrap(), PNG);
        let lifecycle = fixture.lifecycle();
        assert_eq!(
            lifecycle.prepare_png(&key, decode_limits()).err(),
            Some(ImageDecodeError::MissingBytes)
        );
        assert_empty(&lifecycle);
        cache.dispose();
        assert_eq!(cache.resident_bytes(), 0);
    }
}

#[test]
fn stale_source_identity_request_and_kind_never_produce_native_or_cache_media() {
    let fixture = Fixture::new();
    for stale in [
        StaleAsset::Source,
        StaleAsset::Identity,
        StaleAsset::Request,
        StaleAsset::Kind,
    ] {
        let mut current = fixture.request.clone();
        let mut reference = fixture.reference.clone();
        match stale {
            StaleAsset::Source => current.source = ContentDigest([9; 32]),
            StaleAsset::Identity => current.identity = label("image-identity-2"),
            StaleAsset::Request => current.style = label("prepared-style-2"),
            StaleAsset::Kind => reference.kind = AssetKind::Audio,
            StaleAsset::AccessBasis => unreachable!(),
        }
        let error = SelectedAsset::new(
            &fixture.request,
            &current,
            &reference,
            AssetKind::Image,
            &fixture.authority,
            &fixture.basis,
        )
        .err()
        .unwrap();
        assert!(matches!(error, AssetResolutionError::Stale(value) if value == stale));
    }
    assert_eq!(fixture.authority.calls.get(), 0);
    let mut lifecycle = fixture.lifecycle();
    let fetch = lifecycle.fetch(&fixture.key()).unwrap();
    let selected = fixture.selected();
    let caller = 7;
    let mut stream = selected
        .open_native(
            &fixture.context,
            &fixture.store,
            &caller,
            &Purpose::Display,
            ByteRange::new(0, PNG.len() as u64).unwrap(),
            &fixture.request,
        )
        .unwrap();
    let mut changed = fixture.request.clone();
    changed.source = ContentDigest([9; 32]);
    let mut output = [0xa5; 13];
    assert!(matches!(
        stream.read_chunk(&changed, &mut output),
        Err(AssetResolutionError::Stale(StaleAsset::Source))
    ));
    assert_eq!(output, [0xa5; 13]);
    assert!(matches!(
        stream.read_chunk(&fixture.request, &mut output),
        Err(AssetResolutionError::Range(RangeError::Terminated))
    ));
    assert_eq!(output, [0xa5; 13]);
    lifecycle.cancel_fetch(&fetch).unwrap();
    assert_empty(&lifecycle);
}

#[test]
fn changed_access_basis_after_private_read_is_terminal_and_never_completes_the_cache() {
    let fixture = Fixture::new();
    let mut lifecycle = fixture.lifecycle();
    let fetch = lifecycle.fetch(&fixture.key()).unwrap();
    let selected = fixture.selected();
    let caller = 7;
    let mut stream = selected
        .open_native(
            &fixture.context,
            &fixture.store,
            &caller,
            &Purpose::Display,
            ByteRange::new(0, PNG.len() as u64).unwrap(),
            &fixture.request,
        )
        .unwrap();
    // I02 checks both before and after its private scratch read; change at the latter.
    fixture
        .authority
        .change_basis_at
        .set(Some(fixture.authority.calls.get() + 2));
    let mut output = [0xa5; 13];
    assert!(matches!(
        stream.read_chunk(&fixture.request, &mut output),
        Err(AssetResolutionError::Stale(StaleAsset::AccessBasis))
    ));
    assert_eq!(output, [0xa5; 13]);
    assert!(matches!(
        stream.read_chunk(&fixture.request, &mut output),
        Err(AssetResolutionError::Range(RangeError::Terminated))
    ));
    assert_eq!(output, [0xa5; 13]);
    lifecycle.cancel_fetch(&fetch).unwrap();
    assert_eq!(
        lifecycle.prepare_png(&fixture.key(), decode_limits()).err(),
        Some(ImageDecodeError::MissingBytes)
    );
    assert_empty(&lifecycle);
}

#[test]
fn denied_caller_or_purpose_and_unavailable_rights_never_admit_a_decoder_input() {
    let fixture = Fixture::new();
    let mut lifecycle = fixture.lifecycle();
    for (caller, purpose) in [(8, Purpose::Display), (7, Purpose::Export)] {
        let fetch = lifecycle.fetch(&fixture.key()).unwrap();
        assert!(matches!(
            fixture.read_current(caller, purpose),
            Err(AssetResolutionError::Denied)
        ));
        lifecycle.cancel_fetch(&fetch).unwrap();
        assert_empty(&lifecycle);
    }
    fixture
        .authority
        .refusal
        .set(Some(AccessFailure::Unavailable));
    let fetch = lifecycle.fetch(&fixture.key()).unwrap();
    assert!(matches!(
        fixture.read_current(7, Purpose::Display),
        Err(AssetResolutionError::Unavailable(
            AssetUnavailable::CurrentAuthority
        ))
    ));
    lifecycle.cancel_fetch(&fetch).unwrap();
    assert_eq!(
        lifecycle.prepare_png(&fixture.key(), decode_limits()).err(),
        Some(ImageDecodeError::MissingBytes)
    );
    assert_empty(&lifecycle);
}

#[test]
fn corrupt_immutable_backing_cannot_create_a_verified_cache_entry() {
    let fixture = Fixture::new();
    let mut corrupt = PNG.to_vec();
    corrupt[45] ^= 1;
    fs::remove_file(fixture.backing()).unwrap();
    fs::write(fixture.backing(), corrupt).unwrap();
    let mut lifecycle = fixture.lifecycle();
    let fetch = lifecycle.fetch(&fixture.key()).unwrap();
    assert!(matches!(
        fixture.read_current(7, Purpose::Display),
        Err(AssetResolutionError::Store(StoreError::BackingIntegrity))
    ));
    lifecycle.cancel_fetch(&fetch).unwrap();
    assert_eq!(
        lifecycle.prepare_png(&fixture.key(), decode_limits()).err(),
        Some(ImageDecodeError::MissingBytes)
    );
    assert_empty(&lifecycle);
}

#[test]
fn incomplete_corrupt_and_old_epoch_completion_cannot_fabricate_a_current_image() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let bytes = fixture.read_current(7, Purpose::Display).unwrap();
    let mut lifecycle = fixture.lifecycle();
    let short = lifecycle.fetch(&key).unwrap();
    assert_eq!(
        lifecycle.complete_fetch(&short, bytes[..bytes.len() - 1].to_vec()),
        Err(ResourceLifecycleError::Cache(CacheError::Incomplete))
    );
    let bad = lifecycle.fetch(&key).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[45] ^= 1;
    assert_eq!(
        lifecycle.complete_fetch(&bad, corrupt),
        Err(ResourceLifecycleError::Cache(CacheError::HashMismatch))
    );
    assert_empty(&lifecycle);
    let old = lifecycle.fetch(&key).unwrap();
    lifecycle
        .apply_current(scope(), revision(2, 0), std::slice::from_ref(&key))
        .unwrap();
    let current = lifecycle.fetch(&key).unwrap();
    assert_eq!(
        lifecycle.complete_fetch(&old, bytes.clone()),
        Err(ResourceLifecycleError::Cache(CacheError::StaleFetch))
    );
    assert_empty(&lifecycle);
    assert_eq!(
        lifecycle.prepare_png(&key, decode_limits()).err(),
        Some(ImageDecodeError::MissingBytes)
    );
    lifecycle.complete_fetch(&current, bytes).unwrap();
    assert!(lifecycle.prepare_png(&key, decode_limits()).is_ok());
    lifecycle.dispose().unwrap();
    assert_empty(&lifecycle);
}

#[test]
fn release_revocation_recovery_and_disposal_reject_late_terminal_surface_and_release_work() {
    for invalidation in 0..4 {
        let fixture = Fixture::new();
        let key = fixture.key();
        let mut lifecycle = fixture.lifecycle();
        fixture.fill(&mut lifecycle);
        let input = lifecycle.prepare_png(&key, decode_limits()).unwrap();
        let work = input.plan().budget.work_bytes;
        let aborts = Rc::new(Cell::new(0));
        let observed = aborts.clone();
        let token = lifecycle
            .begin(&input, move || observed.set(observed.get() + 1))
            .unwrap();
        let late = input
            .finish_rgba(1, 1, PIXEL.to_vec().into_boxed_slice())
            .unwrap();
        match invalidation {
            0 => lifecycle.release(&key).unwrap(),
            1 => lifecycle
                .apply_current(scope(), revision(1, 1), &[])
                .unwrap(),
            2 => lifecycle
                .apply_current(scope(), revision(2, 0), std::slice::from_ref(&key))
                .unwrap(),
            3 => assert_eq!(lifecycle.dispose(), Ok(PresentationOutcome::Disposed)),
            _ => unreachable!(),
        }
        assert_eq!(aborts.get(), 1);
        assert_eq!(lifecycle.work_bytes(), work);
        assert_eq!(lifecycle.decoded_bytes(), 0);
        let error = lifecycle.complete(&token, late).unwrap_err();
        assert_eq!(
            error,
            if invalidation == 3 {
                ResourceError::Closed
            } else {
                ResourceError::Cancelled
            }
        );
        assert_eq!(lifecycle.work_bytes(), 0);
        assert_eq!(lifecycle.decoded_bytes(), 0);
        assert_eq!(lifecycle.lease_count(), 0);
        assert_eq!(
            lifecycle.finish_failed(&token),
            Err(ResourceError::StaleDecode)
        );
        // Cache-current references revoke leases; the still-current scene key has no resource.
        match invalidation {
            0..=2 => assert!(lifecycle.get(&key).unwrap().is_none()),
            3 => assert!(matches!(lifecycle.get(&key), Err(ResourceError::Closed))),
            _ => unreachable!(),
        }
        if invalidation == 0 {
            assert_eq!(
                lifecycle.prepare_png(&key, decode_limits()).err(),
                Some(ImageDecodeError::MissingBytes)
            );
        }
        lifecycle.dispose().unwrap();
        assert_empty(&lifecycle);
    }
}

#[test]
fn explicit_decode_cancellation_retains_work_until_terminal_and_never_recreates_a_resource() {
    let fixture = Fixture::new();
    let key = fixture.key();
    let mut lifecycle = fixture.lifecycle();
    fixture.fill(&mut lifecycle);
    let input = lifecycle.prepare_png(&key, decode_limits()).unwrap();
    let work = input.plan().budget.work_bytes;
    let aborts = Rc::new(Cell::new(0));
    let observed = aborts.clone();
    let token = lifecycle
        .begin(&input, move || observed.set(observed.get() + 1))
        .unwrap();
    let late = input
        .finish_rgba(1, 1, PIXEL.to_vec().into_boxed_slice())
        .unwrap();
    lifecycle.cancel(&token).unwrap();
    assert_eq!(aborts.get(), 1);
    assert_eq!(lifecycle.work_bytes(), work);
    assert_eq!(lifecycle.finish_failed(&token), Ok(WorkOutcome::Cancelled));
    assert_eq!(lifecycle.work_bytes(), 0);
    assert_eq!(
        lifecycle.complete(&token, late),
        Err(ResourceError::StaleDecode)
    );
    assert!(lifecycle.get(&key).unwrap().is_none());
    lifecycle.release(&key).unwrap();
    lifecycle.dispose().unwrap();
    assert_empty(&lifecycle);
}
