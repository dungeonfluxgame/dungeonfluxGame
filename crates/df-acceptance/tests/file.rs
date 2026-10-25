//! Plan section 17, FILE-01 to FILE-08: uploads, the durable asset catalog, quotas, deletion,
//! revoked access and exports (plan 13 and 14.1).
//!
//! Real tests drive `df-assets` publication and authorized range delivery, the latter with a
//! membership-backed authority built on `df-auth`. The upload validation pipeline, durable
//! catalog, quotas, deletion, delivery gateway and export need O2 and are blocked scenarios,
//! written against `planning/asset-storage-research.md` (no key-bound signed upload and no bearer
//! object-store URL exist in that design).
#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use df_acceptance::{
    ControlledClock, DIRECTORY_CAMPAIGN, DIRECTORY_ROLE, DIRECTORY_TENANT, MemberDirectory,
    blocked_scenario, operation, operation_context,
};
use df_assets::{
    AccessFailure, AssetManifest, AssetMetadataStore, AssetReadAuthority, AssetReadStore,
    AssetStore, AuthorizedBinding, ByteRange, ChunkOutcome, DurableObject, MetadataFailure,
    NativeFileStore, Publication, PublicationError, PublicationStatus, PublishedBinding,
    RangeError, StoreError, open_range, publish,
};
use df_auth::membership::{MembershipRequest, authorize_membership};
use sha2::{Digest, Sha256};

const OBJECT_LIMIT_BYTES: u64 = 1024;
const ALICE: u8 = 10;
const STRANGER: u8 = 11;
const PRIVATE_BYTES: &[u8] = b"0123456789abcdef";

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

/// A scratch directory under `artifacts/tmp/df-acceptance/`, created fresh: a leftover directory
/// of the same name from a crashed run is never reused, because `create_dir` fails on it.
struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new() -> Self {
        let parent =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/tmp/df-acceptance");
        fs::create_dir_all(&parent).expect("the scratch parent can be created");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        loop {
            let number = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{}-{number}-{nanos}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create the scratch directory {path:?}: {error}"),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn entries_in(&self, directory: &str) -> usize {
        fs::read_dir(self.0.join(directory)).map_or(0, Iterator::count)
    }
}

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        // Cleanup of a scratch directory; a leftover is ignored by git and swept with artifacts/tmp.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct RecordingMetadata {
    commits: Cell<usize>,
}

impl AssetMetadataStore for RecordingMetadata {
    type Version = u8;
    type Metadata = &'static str;

    fn lookup(&self, _: &u8) -> Result<Option<PublishedBinding<&'static str>>, MetadataFailure> {
        Ok(None)
    }

    fn publish_immutable(
        &self,
        _: &Publication<u8, &'static str>,
        _: DurableObject,
    ) -> Result<PublicationStatus, MetadataFailure> {
        self.commits.set(self.commits.get() + 1);
        Ok(PublicationStatus::Published)
    }
}

fn publication(bytes: AssetManifest) -> Publication<u8, &'static str> {
    Publication {
        operation: operation(1),
        version: 1,
        metadata: "upload",
        bytes,
    }
}

#[test]
fn file_01_an_oversize_upload_is_refused_while_staging_and_leaves_nothing_behind() {
    let root = TemporaryRoot::new();
    let store = NativeFileStore::new(root.path(), OBJECT_LIMIT_BYTES).expect("store opens");
    let oversize = vec![7_u8; usize::try_from(OBJECT_LIMIT_BYTES).expect("small") + 1];

    let staged = store.stage(operation(1), &mut &oversize[..]);

    assert!(matches!(staged, Err(StoreError::Capacity)));
    assert_eq!(root.entries_in("staging"), 0);
    assert_eq!(root.entries_in("objects"), 0);
}

#[test]
fn file_01_a_manifest_that_declares_an_oversize_object_is_refused_before_any_byte_moves() {
    let root = TemporaryRoot::new();
    let store = NativeFileStore::new(root.path(), OBJECT_LIMIT_BYTES).expect("store opens");
    let metadata = RecordingMetadata::default();
    let staged = store
        .stage(operation(1), &mut &b"small"[..])
        .expect("a small object stages");
    let lying_manifest = AssetManifest {
        byte_len: OBJECT_LIMIT_BYTES + 1,
        sha256: [9; 32],
    };

    let result = publish(
        &operation_context(),
        &publication(lying_manifest),
        &staged,
        &store,
        &metadata,
    );

    assert!(matches!(
        result,
        Err(PublicationError::Store(StoreError::Capacity))
    ));
    assert_eq!(metadata.commits.get(), 0);
    assert_eq!(root.entries_in("objects"), 0);
}

#[test]
fn file_01_bytes_that_do_not_match_the_declared_digest_are_never_promoted() {
    let root = TemporaryRoot::new();
    let store = NativeFileStore::new(root.path(), OBJECT_LIMIT_BYTES).expect("store opens");
    let metadata = RecordingMetadata::default();
    let uploaded = b"pretends to be a png";
    let staged = store
        .stage(operation(1), &mut &uploaded[..])
        .expect("staging accepts any bytes within the limit");
    let declared = AssetManifest {
        byte_len: uploaded.len() as u64,
        ..manifest_of(b"the bytes the client promised")
    };

    let result = publish(
        &operation_context(),
        &publication(declared),
        &staged,
        &store,
        &metadata,
    );

    assert!(matches!(result, Err(PublicationError::HashMismatch)));
    assert_eq!(metadata.commits.get(), 0, "no metadata means no visibility");
    assert_eq!(root.entries_in("objects"), 0);
}

#[test]
fn file_01_a_truncated_or_padded_upload_is_never_promoted() {
    for (declared_len, uploaded) in [(100_u64, &[1_u8; 60][..]), (60, &[1_u8; 100][..])] {
        let root = TemporaryRoot::new();
        let store = NativeFileStore::new(root.path(), OBJECT_LIMIT_BYTES).expect("store opens");
        let metadata = RecordingMetadata::default();
        let staged = store
            .stage(operation(1), &mut &uploaded[..])
            .expect("staging accepts bytes within the limit");
        let declared = AssetManifest {
            byte_len: declared_len,
            sha256: Sha256::digest(uploaded).into(),
        };

        let result = publish(
            &operation_context(),
            &publication(declared),
            &staged,
            &store,
            &metadata,
        );

        assert!(matches!(result, Err(PublicationError::Incomplete)));
        assert_eq!(metadata.commits.get(), 0);
        assert_eq!(root.entries_in("objects"), 0);
    }
}

blocked_scenario! {
    fn file_01_oversize_decompression_bomb_and_wrong_type_uploads_are_rejected_before_promotion,
    blocked_on: "blocked on O2 API: df-assets::validation (ImagePolicy, Rejection) in the upload pipeline, with the screen behind a fake dispatcher (asset design 5, 18)",
    setup: [
        "quota-free player P with an empty personal library; fixtures: an oversize file, a 30000x30000 header with a tiny body, a PNG sent as JPEG, an animated WebP, an APNG, and a valid 200 KB PNG",
    ],
    steps: [
        "upload each fixture through the production upload endpoint with a valid ticket",
        "assert: each of the first five is Rejected before promotion with a typed reason, before any decode allocates beyond the configured pixel limit",
        "assert: nothing reaches PrivateMedia, none is deliverable, the reservation is released and the library quota is unchanged",
        "assert: the valid PNG is accepted, screened, and appears with the right owner and audience",
    ],
    outcome: "Rejected safely before promotion",
}

blocked_scenario! {
    fn file_02_a_ticket_or_staging_write_for_another_account_or_attempt_is_refused,
    blocked_on: "blocked on O2 API: UploadTicket bound to account, purpose and upload, with finalize and staging that re-check the binding and the current attempt (asset design 7)",
    setup: [
        "accounts X and Y; X holds an upload ticket T for upload U; bytes always pass through the server (there is no client PUT to an object key)",
        "a generation attempt A1 superseded by A2 for the same resource",
    ],
    steps: [
        "Y presents X's ticket T to finalize U, and to stage bytes for U",
        "a staging write is attempted for a different asset id, and for the superseded attempt A1",
        "assert: every one is refused; none becomes an asset owned by X or Y, X's and Y's catalogs and quotas are unchanged, and an audit event records the mismatch without the ticket value",
        "assert: X finalizing U with T still succeeds, and a second staging write after Quarantined is refused",
    ],
    outcome: "Denied or quarantined, never accepted as another user's asset",
}

blocked_scenario! {
    fn file_03_a_campaign_with_more_than_512_prior_assets_still_serves_every_asset_to_members_only,
    blocked_on: "blocked on O2 and A2 API: the hub job's by_request_key over the durable catalog instead of the 512-entry registry (asset design 4, audit F03)",
    setup: [
        "campaign C with host H, member M and non-member N; 600 generated assets registered through the production path",
        "restart the runner so no in-process registry survives; a fake provider that counts every call",
    ],
    steps: [
        "revisit the first 100 assets after eviction as M",
        "assert: every one is served from the durable catalog with its original owner and audience, and the provider saw zero calls",
        "request the same assets as N",
        "assert: every request is refused with the concealed 404, and nothing was regenerated or re-billed",
    ],
    outcome: "Durable catalog serves owned assets correctly",
}

blocked_scenario! {
    fn file_04_a_five_mebibyte_manifest_with_junk_imports_with_the_audience_its_grant_lines_prove,
    blocked_on: "blocked on O2 API: the offline importer over A2's record-wise manifest reader, re-keyed idempotently, with audience from grant lines (asset design 14)",
    setup: [
        "a 5 MiB provenance manifest of junk lines with a 1 MiB overlong line between real records for three owners",
        "grant lines: a Room grant for some images, Player or Withheld grants for others, none for the rest",
    ],
    steps: [
        "run the offline import, kill the importer mid-run, run it again, then restart the runner",
        "assert: every real record is registered exactly once, with its original owner and digest, and the overlong line costs only itself",
        "assert: an image is CampaignShared only where a Room grant line proves the table saw it; Player and Withheld lines import as Withheld; images with no grant line import as Host(c)",
        "assert: the session opens and no registration or ownership is lost",
    ],
    outcome: "No loss of registrations or asset ownership",
}

blocked_scenario! {
    fn file_05_when_the_campaign_media_quota_is_exhausted_essential_saves_still_succeed,
    blocked_on: "blocked on O2 API: media.quota_usage with typed QuotaRefusal and Deferred optional plates (asset design 10); a storage limit, not a spend cap (decision 10)",
    setup: [
        "campaign C at 100% of its media allowance, mid-session with six players",
    ],
    steps: [
        "play three turns that each request optional pictures and each commit a game decision",
        "assert: every decision commit and save point succeeds and the UI says Saved only after the authoritative commit",
        "assert: optional plates are Deferred with zero provider calls, an upload is refused, and the customer is not charged for them",
        "free space; assert the next need generates without restarting the session",
    ],
    outcome: "Essential save succeeds; optional media refused",
}

blocked_scenario! {
    fn file_06_deleting_an_asset_referenced_by_a_save_never_breaks_the_save,
    blocked_on: "blocked on O2 and O1 API: reference-counted deletion with tombstones, placeholders and GC, inside O1's save transaction (asset design 11)",
    setup: [
        "asset A referenced by save point 12 of campaign C and by branch B; A is owned by player P",
    ],
    steps: [
        "P requests deletion of A while the reference is live",
        "assert: the delete is refused with the reference named",
        "P consents to a placeholder: assert A is tombstoned, chrome is neutral, a new save point commits, and a branch from the old save commits",
        "resume from save point 12 and from branch B; advance the clock past gc_delay",
        "assert: both resume without error, the placeholder renders, no replay link is lost, and the bytes are purged only after the delay",
    ],
    outcome: "No broken state; explicit reference handling",
}

blocked_scenario! {
    fn file_07_after_a_revoking_campaign_event_the_next_request_or_chunk_is_refused_and_no_grant_is_issued,
    blocked_on: "blocked on O2 and S1 API: the delivery gateway's RevocationFence over S1's campaign_event feed, with fresh-load renewal (asset design 8.1 to 8.3)",
    setup: [
        "member Q holds a grant G for a campaign asset and is partway through a ranged download; the controlled clock and grant_lifetime from configuration",
    ],
    steps: [
        "the host revokes Q (a campaign_event with revoking = true)",
        "assert: Q's next request and the next chunk of the open download are refused; no new grant is issued to Q",
        "assert: renewal after revocation is refused by the fresh load, and reinstating Q later does not revive G or the old download",
        "assert: no bearer object-store URL exists; only bytes already in Q's browser cache persist, until max-age, as the help page states",
    ],
    outcome: "Documented expiry behavior; no new URL issued",
}

blocked_scenario! {
    fn file_08_an_account_export_never_includes_other_players_secrets,
    blocked_on: "blocked on O2 and O1 API: audience-filtered ExportSource sections against O1's real campaign projection (asset design 12, part b); this test must fail until filtering is correct",
    setup: [
        "campaign C with host H and players P and Q; H has hidden game-master state; Q has private notes and a draft portrait; P has private notes",
        "plant a unique marker string in each of those secrets, using O1's real projection and not a fake source",
    ],
    steps: [
        "export P's account data and, separately, the host's campaign export",
        "scan every file in both archives, including nested JSON and media metadata, for the markers",
        "assert: P's export contains P's profile, templates, permitted excerpts, own usage statements and owned uploads",
        "assert: no marker belonging to Q, to the host's hidden state, to credentials or to operator audit appears in any archive",
        "assert: the archive link is short-lived, requires recent authentication, its access is recorded, and the archive and its wrapped key are deleted at expiry",
    ],
    outcome: "Test must fail until filtering is correct",
}

/// Private campaign asset whose every read is authorized against current membership.
struct PrivateAsset<'a> {
    clock: &'a ControlledClock,
    directory: RefCell<MemberDirectory>,
}

impl PrivateAsset<'_> {
    fn manifest() -> AssetManifest {
        manifest_of(PRIVATE_BYTES)
    }

    fn revoke(&self, principal: u8) {
        self.directory.borrow_mut().revoke(principal);
    }

    fn reinstate(&self, principal: u8) {
        self.directory.borrow_mut().reinstate(principal);
    }
}

impl AssetMetadataStore for PrivateAsset<'_> {
    type Version = u8;
    type Metadata = &'static str;

    fn lookup(&self, _: &u8) -> Result<Option<PublishedBinding<&'static str>>, MetadataFailure> {
        Ok(Some(PublishedBinding {
            metadata: "draft-portrait",
            bytes: Self::manifest(),
            object: DurableObject::from_manifest(Self::manifest()),
        }))
    }

    fn publish_immutable(
        &self,
        _: &Publication<u8, &'static str>,
        _: DurableObject,
    ) -> Result<PublicationStatus, MetadataFailure> {
        Err(MetadataFailure::Unavailable)
    }
}

impl AssetReadAuthority for PrivateAsset<'_> {
    type Caller = u8;
    type Purpose = ();
    type Basis = u32;

    fn current_authorized_binding(
        &self,
        caller: &u8,
        version: &u8,
        _: &(),
        _: ByteRange,
    ) -> Result<AuthorizedBinding<&'static str, u32>, AccessFailure> {
        let mut directory = self.directory.borrow_mut();
        let capability = authorize_membership(
            &mut *directory,
            MembershipRequest {
                principal: caller,
                tenant: &DIRECTORY_TENANT,
                campaign: &DIRECTORY_CAMPAIGN,
                role: &DIRECTORY_ROLE,
            },
            self.clock.now_seconds(),
        )
        .map_err(|_| AccessFailure::Denied)?;
        let published = self
            .lookup(version)
            .map_err(|_| AccessFailure::Unavailable)?
            .ok_or(AccessFailure::Absent)?;
        Ok(AuthorizedBinding {
            published,
            basis: *capability.revision(),
            max_chunk_bytes: 4,
        })
    }
}

struct MemoryBytes;

impl AssetReadStore for MemoryBytes {
    type Reader = Cursor<Vec<u8>>;

    fn open_verified(
        &self,
        _: DurableObject,
        manifest: AssetManifest,
    ) -> Result<Self::Reader, StoreError> {
        if manifest != manifest_of(PRIVATE_BYTES) {
            return Err(StoreError::BackingIntegrity);
        }
        Ok(Cursor::new(PRIVATE_BYTES.to_vec()))
    }
}

fn private_asset(clock: &ControlledClock, alice_expires_at: Option<u64>) -> PrivateAsset<'_> {
    PrivateAsset {
        clock,
        directory: RefCell::new(MemberDirectory::default().with_member(ALICE, alice_expires_at)),
    }
}

fn whole_object() -> ByteRange {
    ByteRange::new(0, PRIVATE_BYTES.len() as u64).expect("a nonempty range")
}

#[test]
fn file_07_a_revoked_member_gets_no_new_access_and_an_open_stream_stops_at_its_next_chunk() {
    let clock = ControlledClock::starting_at(1_000);
    let asset = private_asset(&clock, None);
    let context = operation_context();
    let mut stream = open_range(
        &context,
        &MemoryBytes,
        &asset,
        &ALICE,
        &1,
        &(),
        whole_object(),
    )
    .expect("an active member opens the asset");
    let mut chunk = [0_u8; 4];
    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Ok(ChunkOutcome::Bytes(4))
    ));

    asset.revoke(ALICE);

    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Err(RangeError::Access(AccessFailure::Denied))
    ));
    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Err(RangeError::Terminated)
    ));
    let reopened = open_range(
        &context,
        &MemoryBytes,
        &asset,
        &ALICE,
        &1,
        &(),
        whole_object(),
    );
    assert!(matches!(
        reopened.err(),
        Some(RangeError::Access(AccessFailure::Denied))
    ));
}

#[test]
fn file_07_a_principal_who_was_never_a_member_is_denied() {
    let clock = ControlledClock::starting_at(1_000);
    let asset = private_asset(&clock, None);
    let context = operation_context();

    let opened = open_range(
        &context,
        &MemoryBytes,
        &asset,
        &STRANGER,
        &1,
        &(),
        whole_object(),
    );

    assert!(matches!(
        opened.err(),
        Some(RangeError::Access(AccessFailure::Denied))
    ));
}

#[test]
fn file_07_a_reinstated_member_does_not_revive_a_stream_opened_under_the_old_membership() {
    let clock = ControlledClock::starting_at(1_000);
    let asset = private_asset(&clock, None);
    let context = operation_context();
    let mut stream = open_range(
        &context,
        &MemoryBytes,
        &asset,
        &ALICE,
        &1,
        &(),
        whole_object(),
    )
    .expect("an active member opens the asset");

    asset.revoke(ALICE);
    asset.reinstate(ALICE);

    let mut chunk = [0_u8; 4];
    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Err(RangeError::StaleBinding)
    ));
}

#[test]
fn file_07_access_ends_exactly_when_the_membership_expires() {
    let clock = ControlledClock::starting_at(1_000);
    let asset = private_asset(&clock, Some(1_300));
    let context = operation_context();
    let mut stream = open_range(
        &context,
        &MemoryBytes,
        &asset,
        &ALICE,
        &1,
        &(),
        whole_object(),
    )
    .expect("a member opens the asset before expiry");
    let mut chunk = [0_u8; 4];
    clock.advance(299);
    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Ok(ChunkOutcome::Bytes(4))
    ));

    clock.advance(1);

    assert!(matches!(
        stream.read_chunk(&mut chunk),
        Err(RangeError::Access(AccessFailure::Denied))
    ));
}
