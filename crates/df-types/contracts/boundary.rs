//! C-df-types-BOUNDARY candidate contract source, consumed only by contract fixtures.
//! The production df-types API and all wire/storage representations remain unchanged.
//! Construction preserves supplied values; it never issues identity or establishes access.
//!
//! One producer/consumer ledger (request -> result/error; resource owner):
//! - Session/member/binding/run/operation: canonical df-types byte/text constructors ->
//!   their distinct values / IdentityError or TextIdentityError. Actual consumers are
//!   df-api::request, df-auth/session scoped inputs, df-model::checkpoint::Basis and
//!   membership, and generated common DTO mappings in df-tools/tests/shared_contracts.
//! - Client: this candidate ClientId is one supplied client instance identity, distinct
//!   from stable ClientBindingId and the actual opaque df-client ConnectionGeneration.
//!   Candidate association is ClientId + ClientBindingId + current connection token;
//!   allocation/credential/wire ingestion still belongs to client/session/auth owners.
//! - Job: reuse df-model::checkpoint::JobId and its canonical byte validator. Actual
//!   RequestIdentity and SpeechIdentity bind Basis + JobId + OperationId + generation.
//! - Utterance: this candidate UtteranceId associates one utterance with an admitted
//!   JobId and its exact session/run/operation/generation; it is not an alias for JobId.
//!   SpeechScheduler remains the actual response owner; no speech wire tag is invented.
//! - Asset: this candidate AssetId is distinct from AssetReference.key and content digest.
//!   Candidate association retains the entire actual AssetReference. AssetResolver::new
//!   and open_native consume that reference, current caller/purpose/basis and ByteRange.
//!   AssetReadAuthority owns access; a hash, AssetId or verified backing is not permission.
//! - Revision: canonical RecoveryEpoch/SessionRevision -> RevisionError; checked sequence
//!   never advances an epoch. Persistence owns protected epoch issuance and recovery.
//! - Work generation: this candidate nonzero Generation -> GenerationError; advance is
//!   checked. Its run/job/operation/owner scope is mandatory at native completion. It
//!   does not replace pointer-identity ConnectionGeneration or manufacture owner authority.
//! - Money/Usage: canonical Currency/Money/Usage/LiabilityRate -> MoneyError; exact u128
//!   tagged micro-units and closed usage units. Commerce/provider owns quotes/settlement.
//! - Locale: canonical LocaleTag::parse -> LocaleTagError; locale owner settles/falls back.
//! - Ruleset: candidate RulesetId stores separate mechanics/catalog/source-manifest labels
//!   -> component-attributed RevisionLabelError. The actual RulesPins association retains
//!   mode, all digests and handler identity. Syntax is not catalog/source/rights approval.
//! - Build: canonical BuildIdentity::new -> BuildIdentityError; independent source/native/
//!   WASM/configuration/content labels. Build/content owners supply and verify artifacts.
//!
//! All above values own only ordinary Rust value memory; none owns a stream or task.
//! Actual consumer resource/terminal contracts and required fixture hooks:
//! - RpcConnection owns CallCancellation waits and RpcStream admission. Close/drop fences
//!   its ConnectionGeneration and releases only local waits; statuses/EOF/trailers stay
//!   distinct and cannot acknowledge a game mutation. Fixture: types_boundary client case;
//!   existing df-client contract tests cover returned streams, capacity and terminal status.
//! - AuthorizedRange privately owns the verified reader, rechecks current authorization
//!   before each bounded chunk, terminates on typed RangeError and drops its reader.
//!   Asset publication distinguishes MetadataFailure::Unknown from confirmed receipts.
//!   Fixture: types_boundary asset case plus existing df-assets publication/range/resolution.
//! - SpeechScheduler owns counted request/chunk buffers until finish/stop/owner teardown;
//!   dropping SpeechReceipt does not cancel admitted work. MediaLifecycle checks owner
//!   identity, operation, generation and scene before publication. Existing df-media
//!   speech/cancellation suites are the actual typed failure/owned-cleanup fixture hooks.
//! - CheckedRequest owns admitted bounded bytes; provider request/context identity is
//!   checked before use. Existing df-provider-api request suites are the actual hooks.
//!
//! Native consumer fixtures are excluded on WASM; candidate/value tests compile on both.
//! No new dependency edge, production export, codec, service, issuer or resource owner is
//! declared here. Later implementation must reuse these candidates through its own gate.

use df_types::{IdentityError, OperationId, RevisionLabel, RevisionLabelError, TextIdentityError};

// Each candidate wraps the canonical OperationId validator; the kinds stay distinct types.
macro_rules! candidate_identity {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
        pub struct $name(OperationId);

        impl $name {
            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                OperationId::from_bytes(bytes).map(Self)
            }

            /// Parses exactly 32 lowercase ASCII hexadecimal digits.
            pub fn from_hex(text: &str) -> Result<Self, TextIdentityError> {
                OperationId::from_hex(text).map(Self)
            }

            /// Returns the supplied canonical bytes without reordering.
            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }
    };
}

candidate_identity!(
    /// Candidate identity for one client instance, not its stable binding or connection token.
    ClientId
);
candidate_identity!(
    /// Candidate utterance identity, independent from the admitted native job identity.
    UtteranceId
);
candidate_identity!(
    /// Candidate semantic asset identity; byte backing and authorized publication are separate.
    AssetId
);

/// The ruleset component a rejected label belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RulesetComponent {
    Mechanics,
    Catalog,
    SourceManifest,
}

/// Safe attribution without retaining rejected supplied text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RulesetIdError {
    pub component: RulesetComponent,
    pub cause: RevisionLabelError,
}

/// Candidate exact three-part identity; the owning RulesPins retains digests and mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RulesetId {
    mechanics: RevisionLabel,
    catalog: RevisionLabel,
    source_manifest: RevisionLabel,
}

impl RulesetId {
    /// Validates mechanics, catalog and source manifest in that order; first rejection wins.
    pub fn new(
        mechanics: Option<&str>,
        catalog: Option<&str>,
        source_manifest: Option<&str>,
    ) -> Result<Self, RulesetIdError> {
        let mechanics = RevisionLabel::new(mechanics).map_err(|cause| RulesetIdError {
            component: RulesetComponent::Mechanics,
            cause,
        })?;
        let catalog = RevisionLabel::new(catalog).map_err(|cause| RulesetIdError {
            component: RulesetComponent::Catalog,
            cause,
        })?;
        let source_manifest =
            RevisionLabel::new(source_manifest).map_err(|cause| RulesetIdError {
                component: RulesetComponent::SourceManifest,
                cause,
            })?;
        Ok(Self {
            mechanics,
            catalog,
            source_manifest,
        })
    }

    pub fn mechanics(&self) -> &RevisionLabel {
        &self.mechanics
    }

    pub fn catalog(&self) -> &RevisionLabel {
        &self.catalog
    }

    pub fn source_manifest(&self) -> &RevisionLabel {
        &self.source_manifest
    }
}

/// Rejection of a supplied or advanced work generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationError {
    Zero,
    Exhausted,
}

/// Candidate work counter; equality alone is insufficient without its owner/run/job scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Generation(u64);

impl Generation {
    /// Accepts any nonzero supplied generation.
    pub fn new(value: u64) -> Result<Self, GenerationError> {
        if value == 0 {
            return Err(GenerationError::Zero);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }

    /// Advances by one, refusing to wrap past `u64::MAX`.
    pub fn next(self) -> Result<Self, GenerationError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(GenerationError::Exhausted)
    }
}
