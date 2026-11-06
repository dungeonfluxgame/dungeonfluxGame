//! Versioned public artwork identity is separate from the run's selection fence.
//! These are canonical claims, never permission to read a campaign, source, or published asset.

use std::fmt;

use df_assets::Audience;
use df_types::{CampaignId, RunId};
use sha2::{Digest, Sha256};

use crate::{ImageOwner, Purpose, PurposedUse, VariantRole};

/// Version of the explicit campaign artwork identity and prompt-binding encoding.
pub const CAMPAIGN_ARTWORK_SPEC_VERSION: u32 = 1;
/// Maximum UTF-8 bytes in one stable, server-owned subject key.
pub const MAXIMUM_ARTWORK_SUBJECT_BYTES: usize = 256;

/// Invalid artwork metadata reports no subject text or public appearance facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtworkError {
    EmptySubject,
    UnsupportedVersion { actual: u32 },
    SubjectTooLong { actual_bytes: usize },
    OwnerScopeMismatch,
    AudienceMismatch,
    PurposeMismatch,
}

/// A stable public definition key or content address, never a run-local entity identity.
#[derive(Clone, Eq, PartialEq)]
pub struct ArtworkSubjectKey {
    text: Box<str>,
    len_bytes: u32,
}

impl ArtworkSubjectKey {
    /// Refuses blank keys and keys over [`MAXIMUM_ARTWORK_SUBJECT_BYTES`].
    pub fn new(text: &str) -> Result<Self, ArtworkError> {
        if text.trim().is_empty() {
            return Err(ArtworkError::EmptySubject);
        }
        if text.len() > MAXIMUM_ARTWORK_SUBJECT_BYTES {
            return Err(ArtworkError::SubjectTooLong {
                actual_bytes: text.len(),
            });
        }
        let len_bytes = u32::try_from(text.len()).map_err(|_| ArtworkError::SubjectTooLong {
            actual_bytes: text.len(),
        })?;
        Ok(Self {
            text: text.into(),
            len_bytes,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for ArtworkSubjectKey {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("ArtworkSubjectKey")
            .field("len_bytes", &self.len_bytes)
            .finish()
    }
}

/// Explicit namespaces keep identical hero, NPC, cast, and item keys distinct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtworkSubject {
    Hero(ArtworkSubjectKey),
    Npc(ArtworkSubjectKey),
    Cast(ArtworkSubjectKey),
    Item(ArtworkSubjectKey),
}

impl ArtworkSubject {
    pub fn key(&self) -> &ArtworkSubjectKey {
        match self {
            Self::Hero(key) | Self::Npc(key) | Self::Cast(key) | Self::Item(key) => key,
        }
    }

    pub(crate) fn tag(&self) -> u8 {
        match self {
            Self::Hero(_) => 0,
            Self::Npc(_) => 1,
            Self::Cast(_) => 2,
            Self::Item(_) => 3,
        }
    }

    pub(crate) fn purpose(&self) -> Purpose {
        match self {
            Self::Hero(_) | Self::Npc(_) | Self::Cast(_) => Purpose::CanonicalPerson,
            Self::Item(_) => Purpose::ItemCloseUp,
        }
    }
}

/// Public authored identity; complete provider text, sources and output policy remain separate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignArtworkSpec {
    pub campaign: CampaignId,
    pub subject: ArtworkSubject,
    pub public_revision: [u8; 32],
}

impl CampaignArtworkSpec {
    /// The identity encoding this build writes: always [`CAMPAIGN_ARTWORK_SPEC_VERSION`].
    pub fn version(&self) -> u32 {
        CAMPAIGN_ARTWORK_SPEC_VERSION
    }

    /// A stable versioned prompt binding; equal bindings alone never authorize artifact reuse.
    pub fn binding_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"df-imagery/campaign-artwork-v1");
        digest.update(self.version().to_be_bytes());
        digest.update(self.campaign.as_bytes());
        digest.update([self.subject.tag()]);
        digest.update(self.subject.key().len_bytes.to_be_bytes());
        digest.update(self.subject.key().as_str().as_bytes());
        digest.update(self.public_revision);
        digest.finalize().into()
    }
}

/// The actual run selection is retained in intent and never placed in shared provider text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSelectionFence {
    pub run: RunId,
    pub binding: [u8; 32],
}

/// Full immutable demand metadata keeps public identity and current run selection distinct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtworkBinding {
    pub spec: CampaignArtworkSpec,
    pub selection: RunSelectionFence,
}

impl ArtworkBinding {
    pub(crate) fn validate_for(
        &self,
        owner: &ImageOwner,
        purpose: PurposedUse,
        audience: &Audience,
    ) -> Result<(), ArtworkError> {
        let ImageOwner::Run { campaign, run } = owner else {
            return Err(ArtworkError::OwnerScopeMismatch);
        };
        if self.spec.campaign != *campaign || self.selection.run != *run {
            return Err(ArtworkError::OwnerScopeMismatch);
        }
        if audience != &Audience::CampaignShared(*campaign) {
            return Err(ArtworkError::AudienceMismatch);
        }
        if purpose.role != VariantRole::Primary || purpose.purpose != self.spec.subject.purpose() {
            return Err(ArtworkError::PurposeMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod artwork_tests;
