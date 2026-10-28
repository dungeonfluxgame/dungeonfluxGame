//! The object-store contract suite (planning/asset-storage-research.md 5), run against every
//! adapter, plus the key-layout rules. Adapter-specific tamper hooks come from each adapter's test.

use crate::AssetManifest;
use crate::media_ids::{AssetId, AttemptId, CharacterTemplateId, UploadId, WriteId};
use crate::object_store::{
    AssetVariant, CharacterRevision, Deadline, InvalidEnvironment, KeyLayout, ObjectEnvironment,
    ObjectKey, ObjectStore, ObjectStoreError, StorageDomain, VersioningStatus,
};
use crate::object_store_fake::{FakeConfig, FakeObjectStore, Fault};
use df_types::{AccountId, CampaignId};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// Drives a future whose awaits complete without an external reactor (the fake's do).
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

pub(crate) fn manifest(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

fn soon() -> Deadline {
    Deadline::after(Duration::from_secs(30))
}

fn id(byte: u8) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    bytes
}

fn layout() -> KeyLayout {
    KeyLayout::new(ObjectEnvironment::new("test").unwrap())
}

fn campaign_key(asset: u8, attempt: u8, write: u8) -> ObjectKey {
    layout().campaign_asset(
        &CampaignId::from_bytes(&id(9)).unwrap(),
        AssetId::from_bytes(&id(asset)).unwrap(),
        AttemptId::from_bytes(&id(attempt)).unwrap(),
        WriteId::from_bytes(&id(write)).unwrap(),
        AssetVariant::Master,
    )
}

/// Every adapter passes these. `tamper` alters the stored bytes of a key behind the store's back.
fn contract<S: ObjectStore>(store: &S, tamper: &dyn Fn(&ObjectKey)) {
    let key = campaign_key(1, 1, 1);
    let bytes = b"a picture".to_vec();

    let stored = block_on(store.put_new(&key, &bytes, manifest(&bytes), soon())).unwrap();
    assert_eq!(stored.manifest, manifest(&bytes));
    let headed = block_on(store.head(&key, soon())).unwrap().unwrap();
    assert_eq!(headed, stored);
    assert_eq!(block_on(store.get_version(&stored, soon())).unwrap(), bytes);

    // put_new never overwrites when the backing enforces create-if-absent.
    if store.conditional_writes().create_if_absent {
        let other = b"another picture".to_vec();
        assert_eq!(
            block_on(store.put_new(&key, &other, manifest(&other), soon())),
            Err(ObjectStoreError::AlreadyExists)
        );
        assert_eq!(block_on(store.get_version(&stored, soon())).unwrap(), bytes);
    }

    // A body that does not match its manifest stores nothing.
    let unstored = campaign_key(2, 1, 1);
    assert_eq!(
        block_on(store.put_new(&unstored, b"abc", manifest(b"abd"), soon())),
        Err(ObjectStoreError::IntegrityMismatch)
    );
    assert_eq!(block_on(store.head(&unstored, soon())).unwrap(), None);

    // Compare-and-set replacement refuses a stale version.
    if store.conditional_writes().replace_if_match {
        let head_key = campaign_key(3, 1, 1);
        let first = block_on(store.put_new(&head_key, b"h1", manifest(b"h1"), soon())).unwrap();
        let second =
            block_on(store.put_if_match(&head_key, &first.version, b"h2", manifest(b"h2"), soon()))
                .unwrap();
        assert_ne!(first.version, second.version);
        assert_eq!(
            block_on(store.put_if_match(&head_key, &first.version, b"h3", manifest(b"h3"), soon())),
            Err(ObjectStoreError::PreconditionFailed)
        );
    }

    // Bytes altered behind the store fail the version-pinned digest check.
    tamper(&key);
    assert_eq!(
        block_on(store.get_version(&stored, soon())),
        Err(ObjectStoreError::IntegrityMismatch)
    );

    // Deleting a version removes it, and deleting it again succeeds.
    block_on(store.delete_version(&key, &stored.version, soon())).unwrap();
    block_on(store.delete_version(&key, &stored.version, soon())).unwrap();
    assert_eq!(block_on(store.head(&key, soon())).unwrap(), None);
    assert_eq!(
        block_on(store.get_version(&stored, soon())),
        Err(ObjectStoreError::NotFound)
    );

    // A passed deadline changes nothing.
    let late = campaign_key(4, 1, 1);
    assert_eq!(
        block_on(store.put_new(&late, b"x", manifest(b"x"), Deadline::after(Duration::ZERO))),
        Err(ObjectStoreError::DeadlineExceeded)
    );
    assert_eq!(block_on(store.head(&late, soon())).unwrap(), None);
}

fn listing_contract<S: ObjectStore>(store: &S) {
    for asset in 1..=5 {
        let key = campaign_key(asset, 1, 1);
        block_on(store.put_new(&key, &[asset], manifest(&[asset]), soon())).unwrap();
    }
    let prefix = layout().domain_prefix(StorageDomain::PrivateMedia);
    let first = block_on(store.list_page(&prefix, None, 2, soon())).unwrap();
    assert_eq!(first.objects.len(), 2);
    let after = first.next_after.clone().unwrap();
    let second = block_on(store.list_page(&prefix, Some(&after), 2, soon())).unwrap();
    let third =
        block_on(store.list_page(&prefix, second.next_after.as_deref(), 2, soon())).unwrap();
    assert_eq!(third.objects.len(), 1);
    assert_eq!(third.next_after, None);
    let mut paths: Vec<String> = first
        .objects
        .iter()
        .chain(&second.objects)
        .chain(&third.objects)
        .map(|object| object.path.clone())
        .collect();
    let listed = paths.len();
    paths.dedup();
    assert_eq!(listed, 5);
    assert_eq!(paths.len(), 5);
    let quarantine = layout().domain_prefix(StorageDomain::Quarantine);
    assert!(
        block_on(store.list_page(&quarantine, None, 10, soon()))
            .unwrap()
            .objects
            .is_empty()
    );
}

#[test]
fn the_fake_store_meets_the_object_store_contract() {
    let store = FakeObjectStore::new(FakeConfig::default());
    contract(&store, &|key| store.corrupt_behind(key));
}

/// A new native store in its own directory under the owned `TMPDIR`, with a runtime entered so the
/// store's blocking pool exists; the guard keeps the runtime current for the test.
/// A test directory under the owned `TMPDIR`, removed when the test ends.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // A directory that is already gone is the wanted state.
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

fn native_store() -> (tokio::runtime::Runtime, crate::NativeFileStore, TempRoot) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let temporary =
        std::path::PathBuf::from(std::env::var_os("TMPDIR").expect("owned TMPDIR is required"));
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = temporary.join(format!("df-assets-keyed-{}-{number}", std::process::id()));
    let store = crate::NativeFileStore::new(&root, 1024 * 1024).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    (runtime, store, TempRoot(root))
}

/// Flips the first byte of the newest version file of a key, behind the store.
fn corrupt_native(root: &std::path::Path, key: &ObjectKey) {
    let mut dir = root.join("keyed").join(key.domain().label());
    let segments: Vec<&str> = key.path().split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        if index + 1 == segments.len() {
            dir.push(format!("{segment}.obj"));
        } else {
            dir.push(segment);
        }
    }
    let mut versions: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    versions.sort();
    let head = versions.last().unwrap();
    let mut bytes = std::fs::read(head).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(head, bytes).unwrap();
}

#[test]
fn the_native_file_store_meets_the_object_store_contract() {
    let (runtime, store, root) = native_store();
    let _entered = runtime.enter();
    contract(&store, &|key| corrupt_native(root.path(), key));
}

#[test]
fn the_native_file_store_lists_in_bounded_pages_within_one_domain() {
    let (runtime, store, _root) = native_store();
    let _entered = runtime.enter();
    listing_contract(&store);
}

#[test]
fn the_native_file_store_keeps_history_only_where_the_hosted_bucket_is_versioned() {
    let (runtime, store, _root) = native_store();
    let _entered = runtime.enter();
    let media = campaign_key(1, 1, 1);
    let first = block_on(store.put_new(&media, b"a", manifest(b"a"), soon())).unwrap();
    block_on(store.put_if_match(&media, &first.version, b"b", manifest(b"b"), soon())).unwrap();
    assert_eq!(
        block_on(store.list_versions(&media, soon())).unwrap().len(),
        2
    );
    assert_eq!(block_on(store.get_version(&first, soon())).unwrap(), b"a");

    let staged = layout().staged(
        AssetId::from_bytes(&id(1)).unwrap(),
        AttemptId::from_bytes(&id(1)).unwrap(),
        WriteId::from_bytes(&id(1)).unwrap(),
    );
    let staged_first = block_on(store.put_new(&staged, b"a", manifest(b"a"), soon())).unwrap();
    block_on(store.put_if_match(&staged, &staged_first.version, b"b", manifest(b"b"), soon()))
        .unwrap();
    assert_eq!(
        block_on(store.list_versions(&staged, soon()))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        block_on(store.get_version(&staged_first, soon())),
        Err(ObjectStoreError::NotFound)
    );
    assert_eq!(
        block_on(store.versioning_status(StorageDomain::Quarantine, soon())),
        Ok(VersioningStatus::NeverEnabled)
    );
}

#[test]
fn two_threads_creating_one_key_produce_exactly_one_winner() {
    let (runtime, store, root) = native_store();
    let _entered = runtime.enter();
    let key = campaign_key(1, 1, 1);
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4_u8)
            .map(|attempt| {
                let (store, key, runtime) = (&store, &key, runtime.handle());
                scope.spawn(move || {
                    let _entered = runtime.enter();
                    let body = [attempt; 8];
                    block_on(store.put_new(key, &body, manifest(&body), soon()))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let leftovers = std::fs::read_dir(root.path().join("keyed-tmp"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "no temporary file survives a race");
    assert!(
        outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .all(|error| *error == ObjectStoreError::AlreadyExists)
    );
}

#[test]
fn the_fake_store_lists_in_bounded_pages_within_one_domain() {
    listing_contract(&FakeObjectStore::new(FakeConfig::default()));
}

#[test]
fn a_lost_acknowledgement_leaves_the_write_in_place_and_answers_unknown() {
    let store = FakeObjectStore::new(FakeConfig::default());
    let key = campaign_key(1, 1, 1);
    store.push_fault(Fault::LoseAcknowledgement);
    assert_eq!(
        block_on(store.put_new(&key, b"x", manifest(b"x"), soon())),
        Err(ObjectStoreError::Unknown)
    );
    assert!(block_on(store.head(&key, soon())).unwrap().is_some());
}

#[test]
fn an_object_over_the_bound_is_refused_and_capacity_is_enforced() {
    let store = FakeObjectStore::new(FakeConfig {
        max_object_bytes: 4,
        capacity_bytes: 6,
        ..FakeConfig::default()
    });
    assert_eq!(
        block_on(store.put_new(&campaign_key(1, 1, 1), b"12345", manifest(b"12345"), soon())),
        Err(ObjectStoreError::TooLarge)
    );
    block_on(store.put_new(&campaign_key(2, 1, 1), b"1234", manifest(b"1234"), soon())).unwrap();
    assert_eq!(
        block_on(store.put_new(&campaign_key(3, 1, 1), b"123", manifest(b"123"), soon())),
        Err(ObjectStoreError::StorageFull)
    );
}

#[test]
fn exports_report_never_enabled_until_versioning_is_turned_on() {
    let store = FakeObjectStore::new(FakeConfig::default());
    assert_eq!(
        block_on(store.versioning_status(StorageDomain::Exports, soon())),
        Ok(VersioningStatus::NeverEnabled)
    );
    store.set_versioning(StorageDomain::Exports, VersioningStatus::Suspended);
    assert_eq!(
        block_on(store.versioning_status(StorageDomain::Exports, soon())),
        Ok(VersioningStatus::Suspended)
    );
}

#[test]
fn a_versioned_domain_keeps_every_version_until_each_is_deleted() {
    let store = FakeObjectStore::new(FakeConfig::default());
    let key = campaign_key(1, 1, 1);
    let first = block_on(store.put_new(&key, b"a", manifest(b"a"), soon())).unwrap();
    block_on(store.put_if_match(&key, &first.version, b"b", manifest(b"b"), soon())).unwrap();
    assert_eq!(
        block_on(store.list_versions(&key, soon())).unwrap().len(),
        2
    );
    assert_eq!(block_on(store.get_version(&first, soon())).unwrap(), b"a");
}

#[test]
fn keys_hold_only_the_environment_fixed_words_and_lowercase_hex() {
    let account = AccountId::from_bytes(&id(7)).unwrap();
    let revision = CharacterRevision {
        account,
        template: CharacterTemplateId::from_bytes(&id(8)).unwrap(),
        revision: 3,
    };
    let keys = [
        layout().character_asset(
            &revision,
            AssetId::from_bytes(&id(1)).unwrap(),
            AttemptId::from_bytes(&id(2)).unwrap(),
            WriteId::from_bytes(&id(3)).unwrap(),
            AssetVariant::Display,
        ),
        campaign_key(1, 2, 3),
        layout().staged(
            AssetId::from_bytes(&id(1)).unwrap(),
            AttemptId::from_bytes(&id(2)).unwrap(),
            WriteId::from_bytes(&id(3)).unwrap(),
        ),
        layout().quarantine_source(
            UploadId::from_bytes(&id(4)).unwrap(),
            WriteId::from_bytes(&id(5)).unwrap(),
        ),
    ];
    const WORDS: [&str; 11] = [
        "test",
        "users",
        "characters",
        "campaigns",
        "assets",
        "staged",
        "uploads",
        "master",
        "display",
        "3",
        "",
    ];
    for key in &keys {
        for segment in key.path().split('/') {
            let is_id = segment.len() == 32
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            assert!(
                is_id || WORDS.contains(&segment),
                "unexpected segment {segment}"
            );
        }
    }
    assert_eq!(keys[0].domain(), StorageDomain::PrivateMedia);
    assert_eq!(keys[2].domain(), StorageDomain::Quarantine);
    assert_eq!(keys[3].domain(), StorageDomain::Quarantine);
}

#[test]
fn two_writes_of_one_attempt_never_share_a_key() {
    assert_ne!(campaign_key(1, 1, 1), campaign_key(1, 1, 2));
    assert_ne!(campaign_key(1, 1, 1), campaign_key(1, 2, 1));
}

#[test]
fn an_environment_outside_the_closed_alphabet_is_refused() {
    assert_eq!(ObjectEnvironment::new(""), Err(InvalidEnvironment));
    assert_eq!(ObjectEnvironment::new("Prod"), Err(InvalidEnvironment));
    assert_eq!(ObjectEnvironment::new("a/b"), Err(InvalidEnvironment));
    assert_eq!(
        ObjectEnvironment::new(&"a".repeat(17)),
        Err(InvalidEnvironment)
    );
    assert!(ObjectEnvironment::new("staging-2").is_ok());
}

#[test]
fn stored_key_text_outside_the_layout_alphabet_is_refused() {
    let layout_key = campaign_key(1, 1, 1);
    let rebuilt = ObjectKey::from_stored(layout_key.domain(), layout_key.path()).unwrap();
    assert_eq!(rebuilt, layout_key);
    for bad in [
        "", "/a", "a//b", "a/../b", "A/b", "a b", r"a/b\c", "a/b.c", "a/b/",
    ] {
        assert_eq!(
            ObjectKey::from_stored(StorageDomain::PrivateMedia, bad),
            None,
            "{bad:?}"
        );
    }
    assert_eq!(
        StorageDomain::from_label("quarantine"),
        Some(StorageDomain::Quarantine)
    );
    assert_eq!(StorageDomain::from_label("Quarantine"), None);
}

#[test]
fn a_restart_reaps_only_stale_temporary_files() {
    let (runtime, store, root) = native_store();
    let _entered = runtime.enter();
    drop(store);
    let temporary = root.path().join("keyed-tmp");
    std::fs::create_dir_all(&temporary).unwrap();
    let stale = temporary.join("abandoned");
    let fresh = temporary.join("in-flight");
    for path in [&stale, &fresh] {
        std::fs::write(path, b"x").unwrap();
    }
    let two_hours = std::time::Duration::from_secs(2 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - two_hours)
        .unwrap();
    crate::NativeFileStore::new(root.path(), 1024).unwrap();
    assert!(!stale.exists(), "the abandoned temporary is reaped");
    assert!(
        fresh.exists(),
        "a recent one may belong to a write in flight"
    );
}

#[test]
fn an_environment_named_after_a_windows_device_is_refused() {
    for device in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
        assert_eq!(
            ObjectEnvironment::new(device),
            Err(InvalidEnvironment),
            "{device}"
        );
    }
    for fine in ["com", "com10", "console", "con-1", "lpt0", "prod"] {
        assert!(ObjectEnvironment::new(fine).is_ok(), "{fine}");
    }
}

#[test]
fn account_draft_keys_use_private_media_with_draft_segments() {
    let account = AccountId::from_bytes(&id(7)).unwrap();
    let asset = AssetId::from_bytes(&id(1)).unwrap();
    let attempt = AttemptId::from_bytes(&id(2)).unwrap();
    let write = WriteId::from_bytes(&id(3)).unwrap();
    let key = layout().account_draft_asset(&account, asset, attempt, write, AssetVariant::Master);
    assert_eq!(key.domain(), StorageDomain::PrivateMedia);
    let hex_id = |byte: u8| {
        let mut out = String::new();
        for byte in id(byte) {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    };
    assert_eq!(
        key.path(),
        format!(
            "test/users/{}/draft-assets/{}/{}/{}/master",
            hex_id(7),
            hex_id(1),
            hex_id(2),
            hex_id(3)
        )
    );
    let campaign = CampaignId::from_bytes(&id(7)).unwrap();
    let campaign_key = layout().campaign_asset(
        &campaign,
        asset,
        attempt,
        write,
        AssetVariant::Master,
    );
    assert_eq!(
        campaign_key.path(),
        format!(
            "test/campaigns/{}/assets/{}/{}/{}/master",
            hex_id(7),
            hex_id(1),
            hex_id(2),
            hex_id(3)
        )
    );
    assert_ne!(key.path(), campaign_key.path());
    let other_write = layout().account_draft_asset(
        &account,
        asset,
        attempt,
        WriteId::from_bytes(&id(4)).unwrap(),
        AssetVariant::Master,
    );
    assert_ne!(key, other_write);
    for segment in key.path().split('/') {
        let is_id = segment.len() == 32
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        assert!(
            is_id || ["test", "users", "draft-assets", "master"].contains(&segment),
            "unexpected segment {segment}"
        );
    }
}

#[test]
fn the_native_listing_is_in_key_order_and_pages_cover_it_exactly() {
    let (runtime, store, _root) = native_store();
    let _entered = runtime.enter();
    // `a-b` sorts before `a/c` as text, although directory `a` would be visited first.
    let paths = ["test/a-b", "test/a/c", "test/a/d", "test/b", "test/a0"];
    for path in paths {
        let key = ObjectKey::from_stored(StorageDomain::PrivateMedia, path).unwrap();
        block_on(store.put_new(&key, path.as_bytes(), manifest(path.as_bytes()), soon())).unwrap();
    }
    let prefix = layout().domain_prefix(StorageDomain::PrivateMedia);
    let mut listed = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = block_on(store.list_page(&prefix, after.as_deref(), 2, soon())).unwrap();
        assert!(page.objects.len() <= 2);
        listed.extend(page.objects.iter().map(|object| object.path.clone()));
        match page.next_after {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    let mut expected: Vec<String> = paths.iter().map(|path| (*path).to_owned()).collect();
    expected.sort();
    assert_eq!(listed, expected);
}
