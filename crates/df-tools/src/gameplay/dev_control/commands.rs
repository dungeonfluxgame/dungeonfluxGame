//! The one new message to the session owner: development jumps.
//!
//! A dev order crosses the owner inbox like any other call and is reduced on
//! the single owner thread: the candidate is built from the owner's own
//! current checkpoint there, so no state races exist. The owner commits it
//! through [`commit_owner_candidate`](df_session::submission::DurableOwner::commit_owner_candidate),
//! the same CAS + receipt + publication path as player commits.

use df_model::checkpoint::{
    AssetReference, CheckpointLimits, ContentReference, ResourceConstraint, RuleReference,
};

/// The admitted references a candidate rebuild needs, owned so the order can
/// cross the inbox. Cloned once from the run's recovery source at startup.
#[derive(Clone)]
pub(crate) struct OwnedInventory {
    pub rules: Vec<RuleReference>,
    pub content: Vec<ContentReference>,
    pub resources: Vec<ResourceConstraint>,
    pub assets: Vec<AssetReference>,
    pub limits: CheckpointLimits,
}

impl OwnedInventory {
    pub(crate) fn retained_heap_bytes(&self) -> Option<usize> {
        self.rules
            .capacity()
            .checked_mul(size_of::<RuleReference>())?
            .checked_add(
                self.content
                    .capacity()
                    .checked_mul(size_of::<ContentReference>())?,
            )?
            .checked_add(
                self.resources
                    .capacity()
                    .checked_mul(size_of::<ResourceConstraint>())?,
            )?
            .checked_add(
                self.assets
                    .capacity()
                    .checked_mul(size_of::<AssetReference>())?,
            )
    }
}

/// One development jump for the owner thread to build and commit.
pub(crate) enum DevJob {
    /// Bring back a saved moment (named checkpoint, back/forward target): the
    /// checkpoint plus the actor-side state the checkpoint does not carry.
    Restore {
        moment: Box<super::history::SavedMoment>,
        /// Human provenance, e.g. `checkpoint "before-round-2"`, `back 2`.
        what: String,
    },
    /// Set a hero's hit points (member bytes, validated against bounds).
    HitPoints { member: [u8; 16], value: i64 },
    /// Set the narrative (provider) budget (validated against the pack cap).
    Budget { value: u64, cap: u64 },
}

impl DevJob {
    pub(crate) fn retained_heap_bytes(&self) -> Option<usize> {
        match self {
            Self::Restore { moment, what } => {
                moment.retained_heap_bytes()?.checked_add(what.capacity())?
            }
            Self::HitPoints { .. } | Self::Budget { .. } => 0,
        }
        .checked_add(size_of::<Self>())
    }

    /// Provenance recorded in the new decision's `semantic_output`.
    pub(crate) fn note(&self) -> String {
        match self {
            Self::Restore { what, .. } => format!("dev:restore:{what}"),
            Self::HitPoints { value, .. } => format!("dev:override:hp={value}"),
            Self::Budget { value, .. } => format!("dev:override:budget={value}"),
        }
    }
}

/// The inbox message payload: what to build. The commit itself is scoped as
/// table operation through the call's seat credential.
pub(crate) struct DevOrder {
    pub job: DevJob,
    pub inventory: OwnedInventory,
}

impl DevOrder {
    pub(crate) fn retained_heap_bytes(&self) -> Option<usize> {
        self.job
            .retained_heap_bytes()?
            .checked_add(self.inventory.retained_heap_bytes()?)
    }
}

/// What the owner thread answers: the new revision plus the human
/// before/after for the table log, the override log and the JSON answer.
pub(crate) struct DevResult {
    pub epoch: u64,
    pub sequence: u64,
    pub before: String,
    pub after: String,
}
