//! Recording identity and storage consumer contract; no hashing or durability here.

use df_model::checkpoint::ExecutionMode;
use df_types::Usage;

use crate::RequestIdentity;

/// Complete caller-supplied identity for reusable media/output.
///
/// `Key` must include every semantic contract field supplied by its owner. Equality
/// is an identity fence, not proof of rights, current access, or durable publication.
pub struct RecordingManifest<Key, Contract> {
    pub key: Key,
    pub contract: Contract,
    pub identity: RequestIdentity,
    pub mode: ExecutionMode,
    pub usage: Usage,
}

impl<Key, Contract> std::fmt::Debug for RecordingManifest<Key, Contract> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordingManifest")
            .finish_non_exhaustive()
    }
}

/// Bounded caller paging request; zero or excessive limits are rejected by owners.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingPage {
    after: Option<u64>,
    limit: u16,
}

impl RecordingPage {
    pub const MAX_LIMIT: u16 = 256;

    pub fn new(after: Option<u64>, limit: u16) -> Result<Self, RecordingPageError> {
        if limit == 0 || limit > Self::MAX_LIMIT {
            return Err(RecordingPageError::InvalidLimit);
        }
        Ok(Self { after, limit })
    }

    pub fn after(self) -> Option<u64> {
        self.after
    }

    pub fn limit(self) -> u16 {
        self.limit
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingPageError {
    InvalidLimit,
}

/// Storage boundary failures retain denial, outage, and ambiguous commit distinctly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingError<Denied, Unavailable, Unknown> {
    Denied(Denied),
    Unavailable(Unavailable),
    Unknown(Unknown),
}

/// Result error shared by recording lookup, publication, and listing operations.
pub type RecordingStoreError<Denied, Unavailable, Unknown> =
    RecordingError<Denied, Unavailable, Unknown>;

/// Result shared by recording lookup, publication, and listing operations.
pub type RecordingStoreResult<Value, Denied, Unavailable, Unknown> =
    Result<Value, RecordingStoreError<Denied, Unavailable, Unknown>>;

/// A published record reference is opaque and is not itself a reusable authorization.
pub struct RecordingArtifact<Artifact> {
    pub artifact: Artifact,
    pub usage: Usage,
}

/// Consumer-owned recording port. The caller invokes publication only after its
/// existing complete-stream admission hook has verified identity, length, digest,
/// terminal completion and EOF. This trait performs no verification or storage.
pub trait RecordingStore<Key, Contract> {
    type Artifact;
    type Entry;
    type Denied;
    type Unavailable;
    type Unknown;

    fn lookup(
        &mut self,
        manifest: &RecordingManifest<Key, Contract>,
    ) -> RecordingStoreResult<Option<Self::Entry>, Self::Denied, Self::Unavailable, Self::Unknown>;

    fn publish_complete(
        &mut self,
        manifest: RecordingManifest<Key, Contract>,
        verified_bytes: Vec<u8>,
        verified_sha256: [u8; 32],
    ) -> RecordingStoreResult<
        RecordingArtifact<Self::Artifact>,
        Self::Denied,
        Self::Unavailable,
        Self::Unknown,
    >;

    fn list(
        &mut self,
        page: RecordingPage,
    ) -> RecordingStoreResult<Vec<Self::Entry>, Self::Denied, Self::Unavailable, Self::Unknown>;
}
