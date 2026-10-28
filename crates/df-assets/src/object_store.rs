//! The object-store port (planning/asset-storage-research.md 5): five separated storage domains,
//! keys built only by the layout functions below from opaque ids, immutable writes, version-pinned
//! digest-checked reads. The store never authorizes anyone; the media catalog and delivery gateway
//! do. No presigned URLs exist (D23). Adapters: the in-memory fake (tests), the native file store
//! (local development), the Spaces adapter (hosted, `df-providers`).

use crate::AssetManifest;
use crate::media_ids::{AssetId, AttemptId, CharacterTemplateId, UploadId, WriteId, hex};
use df_types::{AccountId, CampaignId};
use std::future::Future;
use std::time::{Duration, Instant};

/// A separately credentialed permission domain (13.2). Prefixes are not authorization.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StorageDomain {
    PublicStatic,
    PrivateMedia,
    Quarantine,
    Exports,
    Backups,
}

impl StorageDomain {
    pub const ALL: [StorageDomain; 5] = [
        Self::PublicStatic,
        Self::PrivateMedia,
        Self::Quarantine,
        Self::Exports,
        Self::Backups,
    ];

    /// The domain a stored label names; `None` for any other text.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|domain| domain.label() == label)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PublicStatic => "public_static",
            Self::PrivateMedia => "private_media",
            Self::Quarantine => "quarantine",
            Self::Exports => "exports",
            Self::Backups => "backups",
        }
    }
}

/// The deployment prefix of every key (`prod`, `staging`, `dev`, ...): 1 to 16 of `[a-z0-9-]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectEnvironment(String);

/// The environment text is not a valid key prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidEnvironment;

/// The longest environment prefix accepted.
const MAXIMUM_ENVIRONMENT_CHARACTERS: usize = 16;

impl ObjectEnvironment {
    pub fn new(text: &str) -> Result<Self, InvalidEnvironment> {
        let valid = (1..=MAXIMUM_ENVIRONMENT_CHARACTERS).contains(&text.len())
            && text
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !is_windows_device_name(text);
        valid
            .then(|| Self(text.to_owned()))
            .ok_or(InvalidEnvironment)
    }
}

/// Windows cannot create a directory named after a device, so a local store under such an
/// environment prefix would fail only there.
fn is_windows_device_name(text: &str) -> bool {
    const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];
    let numbered = |prefix: &str| {
        text.strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest.as_bytes(), [b'1'..=b'9']))
    };
    DEVICES.contains(&text) || numbered("com") || numbered("lpt")
}

/// One stored representation of an asset.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AssetVariant {
    Master,
    Display,
}

impl AssetVariant {
    pub fn label(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Display => "display",
        }
    }
}

/// A complete object key. Constructed only by [`KeyLayout`], so it never contains user text.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectKey {
    domain: StorageDomain,
    path: String,
}

impl ObjectKey {
    /// A key read back from a catalog row or a store listing: nonempty slash-separated segments of
    /// lowercase letters, digits and hyphens, exactly the alphabet the layout writes. `None` for
    /// anything else, so stored text can never smuggle a path into a key.
    pub fn from_stored(domain: StorageDomain, path: &str) -> Option<Self> {
        let is_valid = !path.is_empty()
            && path.split('/').all(|segment| {
                !segment.is_empty()
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            });
        is_valid.then(|| Self {
            domain,
            path: path.to_owned(),
        })
    }

    pub fn domain(&self) -> StorageDomain {
        self.domain
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A listing prefix inside one domain, also built only by [`KeyLayout`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPrefix {
    domain: StorageDomain,
    path: String,
}

impl ObjectPrefix {
    pub fn domain(&self) -> StorageDomain {
        self.domain
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

/// The template revision a character asset belongs to (the first segments of its key).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CharacterRevision {
    pub account: AccountId,
    pub template: CharacterTemplateId,
    pub revision: u64,
}

/// Who owns a `PrivateMedia` asset's bytes. The key prefix names the owner; the
/// catalog and delivery gateway authorize, never this layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOwner {
    Campaign(CampaignId),
    AccountDraft(AccountId),
}

/// The 13.2 key layout. Every `PrivateMedia` key carries the attempt and a random write id, so two
/// attempts, or two lease holders of one attempt, can never write the same key (5).
#[derive(Clone, Debug)]
pub struct KeyLayout {
    environment: ObjectEnvironment,
}

impl KeyLayout {
    pub fn new(environment: ObjectEnvironment) -> Self {
        Self { environment }
    }

    /// `<env>/users/<account>/characters/<template>/<revision>/<asset>/<attempt>/<write>/<variant>`
    pub fn character_asset(
        &self,
        owner: &CharacterRevision,
        asset: AssetId,
        attempt: AttemptId,
        write: WriteId,
        variant: AssetVariant,
    ) -> ObjectKey {
        self.key(
            StorageDomain::PrivateMedia,
            format!(
                "users/{}/characters/{}/{}/{}/{}/{}/{}",
                hex(owner.account.as_bytes()),
                owner.template.to_hex(),
                owner.revision,
                asset.to_hex(),
                attempt.to_hex(),
                write.to_hex(),
                variant.label()
            ),
        )
    }

    /// `<env>/users/<account>/draft-assets/<asset>/<attempt>/<write>/<variant>`
    pub fn account_draft_asset(
        &self,
        account: &AccountId,
        asset: AssetId,
        attempt: AttemptId,
        write: WriteId,
        variant: AssetVariant,
    ) -> ObjectKey {
        self.key(
            StorageDomain::PrivateMedia,
            format!(
                "users/{}/draft-assets/{}/{}/{}/{}",
                hex(account.as_bytes()),
                asset.to_hex(),
                attempt.to_hex(),
                write.to_hex(),
                variant.label()
            ),
        )
    }

    /// One published variant under whichever owner the catalog validated.
    pub fn owned_asset(
        &self,
        owner: &StorageOwner,
        asset: AssetId,
        attempt: AttemptId,
        write: WriteId,
        variant: AssetVariant,
    ) -> ObjectKey {
        match owner {
            StorageOwner::Campaign(campaign) => {
                self.campaign_asset(campaign, asset, attempt, write, variant)
            }
            StorageOwner::AccountDraft(account) => {
                self.account_draft_asset(account, asset, attempt, write, variant)
            }
        }
    }

    /// `<env>/campaigns/<campaign>/assets/<asset>/<attempt>/<write>/<variant>`
    pub fn campaign_asset(
        &self,
        campaign: &CampaignId,
        asset: AssetId,
        attempt: AttemptId,
        write: WriteId,
        variant: AssetVariant,
    ) -> ObjectKey {
        self.key(
            StorageDomain::PrivateMedia,
            format!(
                "campaigns/{}/assets/{}/{}/{}/{}",
                hex(campaign.as_bytes()),
                asset.to_hex(),
                attempt.to_hex(),
                write.to_hex(),
                variant.label()
            ),
        )
    }

    /// `<env>/staged/<asset>/<attempt>/<write>` in `Quarantine`: provider output before validation.
    pub fn staged(&self, asset: AssetId, attempt: AttemptId, write: WriteId) -> ObjectKey {
        self.key(
            StorageDomain::Quarantine,
            format!(
                "staged/{}/{}/{}",
                asset.to_hex(),
                attempt.to_hex(),
                write.to_hex()
            ),
        )
    }

    /// `<env>/uploads/<upload>/<write>` in `Quarantine`: uploaded bytes before validation.
    pub fn quarantine_source(&self, upload: UploadId, write: WriteId) -> ObjectKey {
        self.key(
            StorageDomain::Quarantine,
            format!("uploads/{}/{}", upload.to_hex(), write.to_hex()),
        )
    }

    /// Everything of this environment in one domain, for the operator's orphan reconciliation.
    pub fn domain_prefix(&self, domain: StorageDomain) -> ObjectPrefix {
        ObjectPrefix {
            domain,
            path: format!("{}/", self.environment.0),
        }
    }

    fn key(&self, domain: StorageDomain, rest: String) -> ObjectKey {
        ObjectKey {
            domain,
            path: format!("{}/{rest}", self.environment.0),
        }
    }
}

/// The store's identity of one version of one key (a Spaces version id or ETag, a local
/// generation). Opaque, never parsed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ObjectVersion(String);

/// The longest version text accepted from a store.
const MAXIMUM_VERSION_CHARACTERS: usize = 1024;

impl ObjectVersion {
    /// Accepts the store's nonempty version text up to 1024 characters.
    pub fn new(text: String) -> Option<Self> {
        (1..=MAXIMUM_VERSION_CHARACTERS)
            .contains(&text.len())
            .then_some(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One confirmed stored version and the manifest its bytes must match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredObject {
    pub key: ObjectKey,
    pub version: ObjectVersion,
    pub manifest: AssetManifest,
}

/// Which conditional writes the backing actually enforces (Spaces support is qualified live).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConditionalWrites {
    pub create_if_absent: bool,
    pub replace_if_match: bool,
}

/// A domain's versioning state. On S3-style stores turning versioning off only suspends it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersioningStatus {
    NeverEnabled,
    Enabled,
    Suspended,
}

/// One listed object (path relative to nothing: the full key path).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedObject {
    pub path: String,
    pub version: ObjectVersion,
    pub byte_len: u64,
}

/// One bounded page of a prefix listing; `next_after` continues it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListPage {
    pub objects: Vec<ListedObject>,
    pub next_after: Option<String>,
}

/// The latest instant a store call may finish. Taken from the monotonic clock at the edge only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deadline(Instant);

impl Deadline {
    pub fn after(budget: Duration) -> Self {
        Self(Instant::now() + budget)
    }

    pub fn instant(self) -> Instant {
        self.0
    }

    pub fn has_passed(self) -> bool {
        Instant::now() >= self.0
    }

    /// Whichever of the two deadlines comes first.
    pub fn earlier_of(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
}

/// Typed store outcomes; callers branch on them, never on text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectStoreError {
    NotFound,
    /// `put_new` met an existing key.
    AlreadyExists,
    /// `put_if_match` met a different current version.
    PreconditionFailed,
    /// The bytes do not match the expected or recorded manifest.
    IntegrityMismatch,
    /// One object is larger than the store's configured bound.
    TooLarge,
    /// The backing cannot do what the store needs (a filesystem without hard links).
    Unsupported,
    /// The store's configured capacity is used up.
    StorageFull,
    DeadlineExceeded,
    Unavailable,
    /// The request may have taken effect; head the key before any retry.
    Unknown,
    Denied,
}

impl ObjectStoreError {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::PreconditionFailed => "precondition_failed",
            Self::IntegrityMismatch => "integrity_mismatch",
            Self::TooLarge => "too_large",
            Self::Unsupported => "unsupported",
            Self::StorageFull => "storage_full",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
            Self::Denied => "denied",
        }
    }
}

/// A version listing the backing cannot answer, or a failed one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionListing {
    Unsupported,
    Failed(ObjectStoreError),
}

/// The consumer-owned object-store port. Every call carries a deadline; nothing here retries.
pub trait ObjectStore: Send + Sync {
    fn conditional_writes(&self) -> ConditionalWrites;

    /// Stores `body` under a key that must not exist yet. The bytes are checked against
    /// `expected` before anything is stored; a mismatch stores nothing.
    fn put_new(
        &self,
        key: &ObjectKey,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> impl Future<Output = Result<StoredObject, ObjectStoreError>> + Send;

    /// Replaces the current version only if it is still `current` (the journal head).
    fn put_if_match(
        &self,
        key: &ObjectKey,
        current: &ObjectVersion,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> impl Future<Output = Result<StoredObject, ObjectStoreError>> + Send;

    /// The current version, or `None` when the key has none.
    fn head(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> impl Future<Output = Result<Option<StoredObject>, ObjectStoreError>> + Send;

    /// The complete bytes of exactly that version, checked against its manifest
    /// (`IntegrityMismatch` otherwise). Never returns unverified bytes.
    fn get_version(
        &self,
        object: &StoredObject,
        deadline: Deadline,
    ) -> impl Future<Output = Result<Vec<u8>, ObjectStoreError>> + Send;

    fn list_versions(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> impl Future<Output = Result<Vec<ObjectVersion>, VersionListing>> + Send;

    /// Deletes one version; deleting an absent version succeeds (retries are safe).
    fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
        deadline: Deadline,
    ) -> impl Future<Output = Result<(), ObjectStoreError>> + Send;

    /// At most `limit` objects under `prefix` with paths after `after`, in path order.
    fn list_page(
        &self,
        prefix: &ObjectPrefix,
        after: Option<&str>,
        limit: usize,
        deadline: Deadline,
    ) -> impl Future<Output = Result<ListPage, ObjectStoreError>> + Send;

    fn versioning_status(
        &self,
        domain: StorageDomain,
        deadline: Deadline,
    ) -> impl Future<Output = Result<VersioningStatus, ObjectStoreError>> + Send;
}
