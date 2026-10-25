use crate::{AssetManifest, AssetMetadataStore, DurableObject, PublishedBinding, StoreError};
use df_observe::OperationContext;
use std::io::{Read, Seek, SeekFrom};

/// Hard upper bound per disclosure; the current authority may impose a smaller limit.
pub const MAX_CHUNK_BYTES: usize = 65536;

/// Nonempty half-open byte interval. Construction never wraps start plus length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Result<Self, RangeError> {
        if start >= end {
            return Err(RangeError::InvalidRange);
        }
        Ok(Self { start, end })
    }

    pub fn from_start_len(start: u64, length: u64) -> Result<Self, RangeError> {
        let end = start.checked_add(length).ok_or(RangeError::InvalidRange)?;
        Self::new(start, end)
    }

    pub fn start(self) -> u64 {
        self.start
    }

    pub fn end(self) -> u64 {
        self.end
    }
}

/// Current metadata/access answer, issued by the caller's authoritative adapter.
pub struct AuthorizedBinding<M, B> {
    pub published: PublishedBinding<M>,
    pub basis: B,
    pub max_chunk_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessFailure {
    Denied,
    Absent,
    Stale,
    Unavailable,
}

/// The metadata/access owner validates trusted identity, audience, purpose, rights,
/// suppression and the exact interval on every call. `Caller` is its trusted boundary
/// type, never a trace ID or self-declared grant. `Basis` must bind all current tenant,
/// principal/audience, access, rights and suppression revisions required by that owner.
/// A cached decision is insufficient; outages and missing facts fail closed.
/// The delivery owner must serialize each final check and bounded disclosure with its
/// revocation fence. This synchronous byte boundary does not create that authority or
/// transaction. Production callers must not treat backing verification as permission.
pub trait AssetReadAuthority: AssetMetadataStore {
    type Caller: ?Sized;
    type Purpose: Clone + Eq;
    type Basis: Eq;

    fn current_authorized_binding(
        &self,
        caller: &Self::Caller,
        version: &Self::Version,
        purpose: &Self::Purpose,
        range: ByteRange,
    ) -> Result<AuthorizedBinding<Self::Metadata, Self::Basis>, AccessFailure>;
}

/// Opens the exact complete immutable object, verifies its full manifest on the same
/// descriptor returned for streaming, and enforces the store's configured object bound.
/// The caller must keep this descriptor private; possession does not authorize delivery.
pub trait AssetReadStore {
    type Reader: Read + Seek;

    fn open_verified(
        &self,
        object: DurableObject,
        manifest: AssetManifest,
    ) -> Result<Self::Reader, StoreError>;
}

/// Typed internal failures. Transport owners must conceal denied versus absent assets.
#[derive(Debug)]
pub enum RangeError {
    InvalidRange,
    InvalidChunkLimit,
    EmptyOutput,
    StaleBinding,
    Access(AccessFailure),
    Store(StoreError),
    Incomplete,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkOutcome {
    Bytes(usize),
    Complete,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StreamState {
    Active,
    Complete,
    Failed,
}

/// Borrowed, opaque stream. Drop cancels further reads; no background/read-ahead work.
/// A failure is terminal, and the caller's output buffer is untouched on failure.
pub struct AuthorizedRange<'a, A: AssetReadAuthority, R> {
    context: &'a OperationContext,
    authority: &'a A,
    caller: &'a A::Caller,
    version: A::Version,
    purpose: A::Purpose,
    captured: AuthorizedBinding<A::Metadata, A::Basis>,
    range: ByteRange,
    position_bytes: u64,
    reader: R,
    state: StreamState,
}

/// Authorize before opening bytes, then fence any changes during verification/seek.
pub fn open_range<'a, S, A>(
    context: &'a OperationContext,
    byte_store: &S,
    authority: &'a A,
    caller: &'a A::Caller,
    version: &A::Version,
    purpose: &A::Purpose,
    range: ByteRange,
) -> Result<AuthorizedRange<'a, A, S::Reader>, RangeError>
where
    S: AssetReadStore,
    A: AssetReadAuthority,
{
    let mut span = df_observe::begin(context, "asset.range.open");
    let result = (|| {
        let captured = authority
            .current_authorized_binding(caller, version, purpose, range)
            .map_err(RangeError::Access)?;
        if range.end > captured.published.bytes.byte_len {
            return Err(RangeError::InvalidRange);
        }
        validate_binding(&captured)?;
        let mut reader = byte_store
            .open_verified(captured.published.object, captured.published.bytes)
            .map_err(RangeError::Store)?;
        let position_bytes = reader
            .seek(SeekFrom::Start(range.start))
            .map_err(StoreError::from)
            .map_err(RangeError::Store)?;
        if position_bytes != range.start {
            return Err(RangeError::Store(StoreError::BackingIntegrity));
        }
        let stream = AuthorizedRange {
            context,
            authority,
            caller,
            version: version.clone(),
            purpose: purpose.clone(),
            captured,
            range,
            position_bytes: range.start,
            reader,
            state: StreamState::Active,
        };
        stream.current_chunk_limit()?;
        Ok(stream)
    })();
    span.finish_unmeasured(result_status(&result));
    result
}

fn validate_binding<M, B>(binding: &AuthorizedBinding<M, B>) -> Result<(), RangeError> {
    if binding.max_chunk_bytes == 0 {
        return Err(RangeError::InvalidChunkLimit);
    }
    if binding.published.object.byte_len() != binding.published.bytes.byte_len
        || binding.published.object.digest() != &binding.published.bytes.sha256
    {
        return Err(RangeError::Store(StoreError::BackingIntegrity));
    }
    Ok(())
}

impl<A: AssetReadAuthority, R: Read + Seek> AuthorizedRange<'_, A, R> {
    fn current_chunk_limit(&self) -> Result<usize, RangeError> {
        let current = self
            .authority
            .current_authorized_binding(self.caller, &self.version, &self.purpose, self.range)
            .map_err(RangeError::Access)?;
        validate_binding(&current)?;
        if current.basis != self.captured.basis
            || current.published.metadata != self.captured.published.metadata
            || current.published.bytes != self.captured.published.bytes
            || current.published.object != self.captured.published.object
        {
            return Err(RangeError::StaleBinding);
        }
        Ok(current.max_chunk_bytes.min(MAX_CHUNK_BYTES))
    }

    /// Return at most one currently authorized chunk. Completion means the exact requested
    /// interval was delivered; EOF, denial, stale basis or I/O failure cannot report completion.
    pub fn read_chunk(&mut self, output: &mut [u8]) -> Result<ChunkOutcome, RangeError> {
        let mut span = df_observe::begin(self.context, "asset.range.read");
        let result = self.read_chunk_inner(output);
        if result.is_err() {
            self.state = StreamState::Failed;
        }
        match &result {
            Ok(ChunkOutcome::Bytes(chunk_bytes)) => span.finish("bytes", *chunk_bytes),
            _ => span.finish_unmeasured(result_status(&result)),
        }
        result
    }

    fn read_chunk_inner(&mut self, output: &mut [u8]) -> Result<ChunkOutcome, RangeError> {
        match self.state {
            StreamState::Failed => return Err(RangeError::Terminated),
            StreamState::Complete => return Ok(ChunkOutcome::Complete),
            StreamState::Active => {}
        }
        if output.is_empty() {
            return Err(RangeError::EmptyOutput);
        }
        let maximum_chunk_bytes = self.current_chunk_limit()?;
        let remaining_bytes = self.range.end - self.position_bytes;
        let chunk_bytes = usize::try_from(remaining_bytes.min(MAX_CHUNK_BYTES as u64))
            .map_err(|_| RangeError::InvalidRange)?
            .min(maximum_chunk_bytes)
            .min(output.len());
        let mut scratch = [0_u8; MAX_CHUNK_BYTES];
        if let Err(error) = self.reader.read_exact(&mut scratch[..chunk_bytes]) {
            return if error.kind() == std::io::ErrorKind::UnexpectedEof {
                Err(RangeError::Incomplete)
            } else {
                Err(RangeError::Store(StoreError::Io(error)))
            };
        }
        // Private scratch keeps in-flight bytes hidden if access changed during I/O.
        if chunk_bytes > self.current_chunk_limit()? {
            return Err(RangeError::StaleBinding);
        }
        let position_bytes = self
            .position_bytes
            .checked_add(chunk_bytes as u64)
            .ok_or(RangeError::InvalidRange)?;
        output[..chunk_bytes].copy_from_slice(&scratch[..chunk_bytes]);
        self.position_bytes = position_bytes;
        if position_bytes == self.range.end {
            self.state = StreamState::Complete;
        }
        Ok(ChunkOutcome::Bytes(chunk_bytes))
    }
}

fn result_status<T>(result: &Result<T, RangeError>) -> &'static str {
    match result {
        Ok(_) => "complete",
        Err(RangeError::InvalidRange) => "invalid_range",
        Err(RangeError::InvalidChunkLimit) => "invalid_chunk_limit",
        Err(RangeError::EmptyOutput) => "empty_output",
        Err(RangeError::StaleBinding | RangeError::Access(AccessFailure::Stale)) => "stale",
        Err(RangeError::Access(AccessFailure::Denied | AccessFailure::Absent)) => "denied",
        Err(RangeError::Access(AccessFailure::Unavailable)) => "access_unavailable",
        Err(RangeError::Store(_)) => "byte_store_failure",
        Err(RangeError::Incomplete) => "incomplete",
        Err(RangeError::Terminated) => "terminated",
    }
}
