//! The rows of the media catalog as typed values (planning/asset-storage-research.md 6.6, 16): the
//! closed vocabularies stored as labels, the asset record, step leases, staged objects, variants
//! and the settlement outbox. Adapters map these to their storage; nothing here does I/O.

use crate::asset_lifecycle::{AssetState, FinalizeOutcome, RejectReason};
use crate::audience::Audience;
use crate::media_ids::{AssetId, AttemptId, ClaimToken, WriteId};
use crate::media_request_key::MediaRequestKey;
use crate::object_store::{AssetVariant, ObjectKey, ObjectVersion};
use crate::{AssetManifest, QuotaScope};
use df_types::{AccountId, CampaignId};

macro_rules! label_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }

            /// The value a stored label names; `None` for any other text.
            pub fn from_label(label: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|value| value.label() == label)
            }
        }
    };
}

label_enum!(
    /// What the bytes are.
    AssetKind {
        Image => "image",
        Audio => "audio",
        Video => "video",
        CharacterJson => "character_json",
        CheckpointBundle => "checkpoint_bundle",
        ExportArchive => "export_archive",
    }
);

label_enum!(
    /// How the asset came to exist.
    Provenance {
        Generated => "generated",
        Uploaded => "uploaded",
        Reviewed => "reviewed",
        ClaimedLibrary => "claimed_library",
    }
);

label_enum!(
    /// Which retention period governs the asset (configuration decides the periods).
    RetentionClass {
        Quarantine => "quarantine",
        Intermediate => "intermediate",
        Personal => "personal",
        CampaignMedia => "campaign_media",
        Export => "export",
        Backup => "backup",
    }
);

label_enum!(
    /// The unit of work a step lease protects (6.1).
    Step {
        Stage => "stage",
        Validate => "validate",
        Screen => "screen",
        Publish => "publish",
        Purge => "purge",
    }
);

label_enum!(
    /// Why an asset was tombstoned (11).
    TombstoneReason {
        UserDelete => "user_delete",
        AccountDeletion => "account_deletion",
        Retention => "retention",
        Erasure => "erasure",
        RestoreReplay => "restore_replay",
    }
);

label_enum!(
    /// Where a variant write is in its life (6.2).
    VariantState {
        Intended => "intended",
        Written => "written",
        Deleting => "deleting",
    }
);

/// The sixteen bytes a request key is unique within: the campaign, or the account for personal
/// media, or zeros for reviewed public photographs (12).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DedupeScope([u8; 16]);

impl DedupeScope {
    pub fn campaign(campaign: &CampaignId) -> Self {
        Self(*campaign.as_bytes())
    }

    pub fn account(account: &AccountId) -> Self {
        Self(*account.as_bytes())
    }

    pub fn public_library() -> Self {
        Self([0; 16])
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// A claim on one step of one attempt, valid until `until_ms` unless a newer claim replaces it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepLease {
    pub step: Step,
    pub owner: ClaimToken,
    pub until_ms: i64,
}

/// The proof a worker presents for every write: which asset and attempt it claimed, and with
/// which fresh token. A write carrying an older token is a lost lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeldStep {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub owner: ClaimToken,
}

/// The one staged object of an attempt: provider output or an upload awaiting validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedObjectRecord {
    pub key: ObjectKey,
    pub version: ObjectVersion,
    pub manifest: AssetManifest,
    /// The time of the first complete staging write; the quarantine age counts from it (6.2).
    pub staged_at_ms: i64,
}

/// What a published asset is, filled in when it becomes `Ready`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedFacts {
    pub mime: String,
    /// Total stored bytes of the winning writes: what the quota commits.
    pub byte_len: u64,
    /// The master variant's digest.
    pub sha256: [u8; 32],
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
}

/// One asset row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetRecord {
    pub asset: AssetId,
    pub kind: AssetKind,
    pub provenance: Provenance,
    pub charge_scope: QuotaScope,
    pub origin_account: Option<AccountId>,
    pub campaign: Option<CampaignId>,
    pub audience: Audience,
    pub dedupe_scope: DedupeScope,
    pub request_key: Option<MediaRequestKey>,
    pub retention: RetentionClass,
    pub state: AssetState,
    pub reject_reason: Option<RejectReason>,
    pub attempt: AttemptId,
    pub lease: Option<StepLease>,
    pub staged: Option<StagedObjectRecord>,
    pub winning_writes: Vec<WriteId>,
    pub published: Option<PublishedFacts>,
    pub suppression_generation: u64,
    pub ready_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// One variant write of an attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantRecord {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub write: WriteId,
    pub variant: AssetVariant,
    pub owner: ClaimToken,
    pub state: VariantState,
    pub key: ObjectKey,
    pub version: Option<ObjectVersion>,
    pub manifest: Option<AssetManifest>,
    pub created_at_ms: i64,
}

/// Usage of one quota scope (10).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QuotaUsage {
    pub committed_bytes: u64,
    pub reserved_bytes: u64,
    pub in_flight_uploads: u32,
    pub template_count: u32,
}

/// One terminal report for F's settlement (D22, D27): exactly one per attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutboxEntry {
    pub sequence: i64,
    pub attempt: AttemptId,
    pub asset: AssetId,
    pub outcome: FinalizeOutcome,
    pub at_ms: i64,
    pub is_consumed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_closed_vocabulary_round_trips_through_its_label() {
        for kind in AssetKind::ALL {
            assert_eq!(AssetKind::from_label(kind.label()), Some(*kind));
        }
        for value in Provenance::ALL {
            assert_eq!(Provenance::from_label(value.label()), Some(*value));
        }
        for value in RetentionClass::ALL {
            assert_eq!(RetentionClass::from_label(value.label()), Some(*value));
        }
        for value in Step::ALL {
            assert_eq!(Step::from_label(value.label()), Some(*value));
        }
        for value in TombstoneReason::ALL {
            assert_eq!(TombstoneReason::from_label(value.label()), Some(*value));
        }
        for value in VariantState::ALL {
            assert_eq!(VariantState::from_label(value.label()), Some(*value));
        }
        assert_eq!(AssetKind::from_label("Image"), None);
    }
}
