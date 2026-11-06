use std::fmt;

/// Borrowed identity, approved basis, and artifact from one immutable stored entry.
///
/// This is a lookup view, not a recording-admission or artifact-publication contract.
pub type ReadEntry<'a, Key, Basis, Artifact> = (&'a Key, &'a Basis, &'a Artifact);

/// Stored-entry absence and storage failure remain separate at the read boundary.
pub type ReadResult<'a, Key, Basis, Artifact, Failure> =
    Result<Option<ReadEntry<'a, Key, Basis, Artifact>>, Failure>;

/// Read-only access to complete, previously admitted prepared and replay artifacts.
///
/// The owner supplies canonical key, basis, and artifact types. Implementations must
/// read stored entries without generating content, invoking a live provider, changing
/// execution mode, or publishing a partial/unapproved recording. Each call performs
/// one bounded lookup and preserves storage failure rather than reporting a miss.
/// The borrowed key and basis identify the same immutable entry as the artifact.
pub trait PreparedRead {
    type Key: Eq;
    type Basis: Eq;
    type Artifact;
    type Failure;

    fn read_prepared(
        &self,
        key: &Self::Key,
    ) -> ReadResult<'_, Self::Key, Self::Basis, Self::Artifact, Self::Failure>;

    fn read_replay(
        &self,
        key: &Self::Key,
    ) -> ReadResult<'_, Self::Key, Self::Basis, Self::Artifact, Self::Failure>;
}

/// Honest outcomes when a stored artifact cannot satisfy this admitted lookup.
#[derive(Debug, Eq, PartialEq)]
pub enum LookupError<Failure> {
    Missing,
    Stale,
    Unavailable(Failure),
}

impl<Failure: fmt::Display> fmt::Display for LookupError<Failure> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("no stored artifact exists for this key"),
            Self::Stale => formatter.write_str("the stored artifact has another key or basis"),
            Self::Unavailable(error) => write!(formatter, "the artifact store failed: {error}"),
        }
    }
}

impl<Failure: std::error::Error + 'static> std::error::Error for LookupError<Failure> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            Self::Missing | Self::Stale => None,
        }
    }
}

/// Reads a prepared artifact only when its entire canonical key and basis match.
///
/// No fallback or live-provider callable is accepted. The caller supplies the exact
/// current approved basis; freshness is never inferred from elapsed time or a prefix.
pub fn lookup_prepared<'a, Store: PreparedRead>(
    store: &'a Store,
    key: &Store::Key,
    current_basis: &Store::Basis,
) -> Result<&'a Store::Artifact, LookupError<Store::Failure>> {
    resolve(store.read_prepared(key), key, current_basis)
}

/// Reads a complete replay artifact without starting live work on any outcome.
///
/// Replay and prepared storage are deliberately queried through separate read ports;
/// a replay miss does not search another mode or retry a storage failure.
pub fn lookup_replay<'a, Store: PreparedRead>(
    store: &'a Store,
    key: &Store::Key,
    current_basis: &Store::Basis,
) -> Result<&'a Store::Artifact, LookupError<Store::Failure>> {
    resolve(store.read_replay(key), key, current_basis)
}

fn resolve<'a, Key: Eq, Basis: Eq, Artifact, Failure>(
    entry: ReadResult<'a, Key, Basis, Artifact, Failure>,
    key: &Key,
    current_basis: &Basis,
) -> Result<&'a Artifact, LookupError<Failure>> {
    let (stored_key, stored_basis, artifact) = entry
        .map_err(LookupError::Unavailable)?
        .ok_or(LookupError::Missing)?;
    if stored_key != key || stored_basis != current_basis {
        return Err(LookupError::Stale);
    }
    Ok(artifact)
}

/// Current owner authorization around a complete cache/recording read.
///
/// The canonical key/basis must bind the exact payload, source/context, schema,
/// policy/model, run/job/generations, locale/template, audience/projection and
/// supported output parameters. Unknown rights or unsupported parameters refuse.
/// Equality is identity, never a grant: reauthorize current source/access before
/// storage and again before exposing any result, including a miss or failure.
/// This port neither retains personal payloads nor grants publication authority.
pub trait ReadAuthority: PreparedRead {
    type AuthorizationFailure;

    fn authorize_prepared(
        &self,
        key: &Self::Key,
        basis: &Self::Basis,
    ) -> Result<(), Self::AuthorizationFailure>;

    fn authorize_replay(
        &self,
        key: &Self::Key,
        basis: &Self::Basis,
    ) -> Result<(), Self::AuthorizationFailure>;
}

/// A current authorization refusal does not disclose a stale/missing entry.
#[derive(Debug, Eq, PartialEq)]
pub enum AuthorizedLookupError<Failure, AuthorizationFailure> {
    Authority(AuthorizationFailure),
    Lookup(LookupError<Failure>),
}

impl<Failure, AuthorizationFailure> fmt::Display
    for AuthorizedLookupError<Failure, AuthorizationFailure>
where
    Failure: fmt::Display,
    AuthorizationFailure: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority(error) => write!(formatter, "the read is not authorized: {error}"),
            Self::Lookup(error) => write!(formatter, "the authorized lookup failed: {error}"),
        }
    }
}

impl<Failure, AuthorizationFailure> std::error::Error
    for AuthorizedLookupError<Failure, AuthorizationFailure>
where
    Failure: std::error::Error + 'static,
    AuthorizationFailure: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Authority(error) => Some(error),
            Self::Lookup(error) => Some(error),
        }
    }
}

/// The artifact, or the authorization or lookup refusal, for one authorized read of `Store`.
pub type AuthorizedLookupResult<'a, Store> = Result<
    &'a <Store as PreparedRead>::Artifact,
    AuthorizedLookupError<
        <Store as PreparedRead>::Failure,
        <Store as ReadAuthority>::AuthorizationFailure,
    >,
>;

/// Reauthorizes a prepared read before storage and before returning any outcome.
pub fn lookup_authorized_prepared<'a, Store: ReadAuthority>(
    store: &'a Store,
    key: &Store::Key,
    basis: &Store::Basis,
) -> AuthorizedLookupResult<'a, Store> {
    store
        .authorize_prepared(key, basis)
        .map_err(AuthorizedLookupError::Authority)?;
    let result = lookup_prepared(store, key, basis);
    store
        .authorize_prepared(key, basis)
        .map_err(AuthorizedLookupError::Authority)?;
    result.map_err(AuthorizedLookupError::Lookup)
}

/// Reauthorizes a complete recording read; never queries prepared/live storage.
pub fn lookup_authorized_replay<'a, Store: ReadAuthority>(
    store: &'a Store,
    key: &Store::Key,
    basis: &Store::Basis,
) -> AuthorizedLookupResult<'a, Store> {
    store
        .authorize_replay(key, basis)
        .map_err(AuthorizedLookupError::Authority)?;
    let result = lookup_replay(store, key, basis);
    store
        .authorize_replay(key, basis)
        .map_err(AuthorizedLookupError::Authority)?;
    result.map_err(AuthorizedLookupError::Lookup)
}
