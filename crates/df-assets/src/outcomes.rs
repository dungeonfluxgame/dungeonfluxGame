use crate::{
    AccessFailure, AssetReadAuthority, AssetResolver, AuthorizedRange, ByteRange, ChunkOutcome,
    NativeFileStore, RangeError, StoreError,
};
use df_model::checkpoint::{AssetKind, AssetReference, AssetRequestKey};
use df_observe::OperationContext;
use df_types::RevisionLabel;
use std::fs::File;

/// Missing publication, backing or current rights facts never produce substitute media.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetUnavailable {
    NotPublished,
    BackingMissing,
    CurrentAuthority,
}

/// A retained request, reference or access basis cannot silently acquire newer meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleAsset {
    Source,
    Identity,
    Request,
    Kind,
    AccessBasis,
}

/// Internal outcome classification; public callers conceal denied versus absent references.
#[derive(Debug)]
pub enum AssetResolutionError {
    Unavailable(AssetUnavailable),
    Stale(StaleAsset),
    Denied,
    Store(StoreError),
    Range(RangeError),
}

impl From<RangeError> for AssetResolutionError {
    fn from(error: RangeError) -> Self {
        match error {
            RangeError::Access(AccessFailure::Absent) => {
                Self::Unavailable(AssetUnavailable::NotPublished)
            }
            RangeError::Access(AccessFailure::Unavailable) => {
                Self::Unavailable(AssetUnavailable::CurrentAuthority)
            }
            RangeError::Access(AccessFailure::Denied) => Self::Denied,
            RangeError::Access(AccessFailure::Stale) | RangeError::StaleBinding => {
                Self::Stale(StaleAsset::AccessBasis)
            }
            RangeError::Store(StoreError::BackingMissing) => {
                Self::Unavailable(AssetUnavailable::BackingMissing)
            }
            RangeError::Store(error) => Self::Store(error),
            error => Self::Range(error),
        }
    }
}

/// One source-selected native asset. The current metadata authority still owns every disclosure.
/// This wrapper adds no grant, schema, durable registry, fallback or alternative byte path.
pub struct SelectedAsset<'a, A>
where
    A: AssetReadAuthority<Version = RevisionLabel, Metadata = AssetReference>,
{
    captured: &'a AssetRequestKey,
    reference: &'a AssetReference,
    expected_kind: AssetKind,
    resolver: AssetResolver<'a, A>,
}

impl<'a, A> SelectedAsset<'a, A>
where
    A: AssetReadAuthority<Version = RevisionLabel, Metadata = AssetReference>,
{
    /// The session supplies the current key and the reference associated with that demand.
    /// `expected_basis` is the captured authority token; it is rechecked, never treated as a grant.
    /// The owner serializes current source selection and final disclosure against source changes,
    /// just as the I02 authority owner serializes its revocation fence. A copied key is not authority.
    pub fn new(
        captured: &'a AssetRequestKey,
        current: &AssetRequestKey,
        reference: &'a AssetReference,
        expected_kind: AssetKind,
        authority: &'a A,
        expected_basis: &'a A::Basis,
    ) -> Result<Self, AssetResolutionError> {
        let selected = Self {
            captured,
            reference,
            expected_kind,
            resolver: AssetResolver::new(authority, reference, expected_basis),
        };
        selected.check_current(current)?;
        Ok(selected)
    }

    fn check_current(&self, current: &AssetRequestKey) -> Result<(), AssetResolutionError> {
        if self.captured.source != current.source {
            return Err(AssetResolutionError::Stale(StaleAsset::Source));
        }
        if self.captured.identity != current.identity {
            return Err(AssetResolutionError::Stale(StaleAsset::Identity));
        }
        if self.captured != current {
            return Err(AssetResolutionError::Stale(StaleAsset::Request));
        }
        if self.reference.kind != self.expected_kind {
            return Err(AssetResolutionError::Stale(StaleAsset::Kind));
        }
        Ok(())
    }

    /// Recheck source before I03/I02 authorize and open the complete immutable native object.
    pub fn open_native<'s>(
        &'s self,
        context: &'s OperationContext,
        store: &NativeFileStore,
        caller: &'s A::Caller,
        purpose: &A::Purpose,
        range: ByteRange,
        current: &AssetRequestKey,
    ) -> Result<AssetRange<'s, 'a, A>, AssetResolutionError> {
        self.check_current(current)?;
        let inner = self
            .resolver
            .open_native(context, store, caller, purpose, range)?;
        Ok(AssetRange {
            selected: self,
            inner,
            has_failed: false,
        })
    }
}

/// Borrowed stream whose failures are terminal and leave the caller's output untouched.
pub struct AssetRange<'s, 'a, A>
where
    A: AssetReadAuthority<Version = RevisionLabel, Metadata = AssetReference>,
{
    selected: &'s SelectedAsset<'a, A>,
    inner: AuthorizedRange<'s, AssetResolver<'a, A>, File>,
    has_failed: bool,
}

impl<A> AssetRange<'_, '_, A>
where
    A: AssetReadAuthority<Version = RevisionLabel, Metadata = AssetReference>,
{
    /// The session owner supplies current source state for each read. I02 independently
    /// rechecks rights, canonical binding and basis before and after its private scratch read.
    /// If either check refuses, no output bytes are changed and no future read can resume it.
    pub fn read_chunk(
        &mut self,
        current: &AssetRequestKey,
        output: &mut [u8],
    ) -> Result<ChunkOutcome, AssetResolutionError> {
        if self.has_failed {
            return Err(AssetResolutionError::Range(RangeError::Terminated));
        }
        let result = self.selected.check_current(current).and_then(|()| {
            self.inner
                .read_chunk(output)
                .map_err(AssetResolutionError::from)
        });
        if result.is_err() {
            self.has_failed = true;
        }
        result
    }
}
