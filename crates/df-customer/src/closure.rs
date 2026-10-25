//! Account closure (J14; `planning/campaigns-and-table-research.md` 8.7). Closing is blocked while
//! the account owns or hosts an `active` campaign. The store runs the whole decision in one
//! transaction: lock the account row and set `closing`, ask the campaign owner's participant
//! which campaigns block, roll back when any do, otherwise commit `closing` together with the
//! scheduled deletion of the owned archived campaigns and the cancelation of the account's
//! outgoing transfer proposals. The values below are what that exchange carries.

use df_types::{AccountId, CampaignId};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BlockReason {
    Owns,
    Hosts,
    OwnsAndHosts,
}

impl BlockReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Owns => "owns",
            Self::Hosts => "hosts",
            Self::OwnsAndHosts => "owns_and_hosts",
        }
    }
}

/// An active campaign the account owns or hosts: it must be transferred or archived first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockingCampaign {
    pub campaign: CampaignId,
    pub name: String,
    pub reason: BlockReason,
}

/// An archived campaign the account owns: the owner may transfer or delete it, or let the
/// closing window schedule its deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedArchivedCampaign {
    pub campaign: CampaignId,
    pub name: String,
}

/// What the campaign owner reports for an account about to close.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClosureCheck {
    pub blocking: Vec<BlockingCampaign>,
    pub owned_archived: Vec<OwnedArchivedCampaign>,
}

/// An owned archived campaign that now has a deletion date, and who must be told.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledDeletion {
    pub campaign: CampaignId,
    pub name: String,
    pub members: Vec<AccountId>,
}

/// The result of the close transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloseOutcome {
    /// Committed: the account is `closing`, its sessions are revoked, and the listed campaigns
    /// are scheduled for deletion at `delete_after_ms`.
    Closed {
        delete_after_ms: u64,
        scheduled: Vec<ScheduledDeletion>,
    },
    /// Rolled back: nothing changed. The person settles these first.
    Blocked(ClosureCheck),
    /// The account was already closing (a repeat of the request); nothing changed.
    AlreadyClosing { delete_after_ms: u64 },
}

/// Where one chunk of the campaign owner's `leave_all` stopped (8.7 step 7). A chunk is one
/// transaction that leaves a bounded number of campaigns in `campaign_id` order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaveChunk {
    /// More memberships may remain; the next chunk starts after this campaign.
    Continue(CampaignId),
    Done,
}

/// What one completion attempt of a closing account found. Only `Completed` lets the provider's
/// user be deleted and the personal data purged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionOutcome {
    /// Every campaign was left; nothing in the way remains.
    Completed,
    /// Blockers exist now (a transfer made the account a host after it closed). The account stays
    /// `closing` and nothing about it is deleted.
    Blocked(ClosureCheck),
    /// The account is not `closing`, or its window has not passed (it was reopened, or the clock
    /// moved): nothing changed.
    NotDue,
    /// The chunk limit for one pass was reached; the next pass continues.
    Continues,
}
