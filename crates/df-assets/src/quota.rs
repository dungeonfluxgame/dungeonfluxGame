//! Storage quotas (plan 13.8; planning/asset-storage-research.md 10, 3.13). One usage row per scope;
//! every quota-changing transition locks the affected rows in [`QuotaScope`] order (accounts before
//! campaigns, then by id) before any asset row (D25), so two transactions can never wait on each
//! other's quota rows. A quota is a storage limit, not a spend cap (D10).

use df_types::{AccountId, CampaignId};

/// The owner of one usage row. `Ord` is the lock order: `Account` sorts before `Campaign`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum QuotaScope {
    Account(AccountId),
    Campaign(CampaignId),
}

impl QuotaScope {
    pub fn kind_label(self) -> &'static str {
        match self {
            Self::Account(_) => "account",
            Self::Campaign(_) => "campaign",
        }
    }

    pub fn id_bytes(&self) -> &[u8; 16] {
        match self {
            Self::Account(account) => account.as_bytes(),
            Self::Campaign(campaign) => campaign.as_bytes(),
        }
    }
}

/// The scopes a transition touches, in the order their rows must be locked.
pub fn lock_order(scopes: &[QuotaScope]) -> Vec<QuotaScope> {
    let mut ordered = scopes.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    ordered
}

/// Plan 13.8 defaults; the values are configuration (`content/storage.json`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaPolicy {
    pub account_bytes: u64,
    pub account_templates: u32,
    pub account_in_flight_uploads: u32,
    pub campaign_bytes: u64,
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        Self {
            account_bytes: 100 * 1024 * 1024,
            account_templates: 10,
            account_in_flight_uploads: 3,
            campaign_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

impl QuotaPolicy {
    pub fn byte_limit(&self, scope: QuotaScope) -> u64 {
        match scope {
            QuotaScope::Account(_) => self.account_bytes,
            QuotaScope::Campaign(_) => self.campaign_bytes,
        }
    }
}

/// Space a request needed that the scope does not have. Clears when space frees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaRefusal {
    pub scope: QuotaScope,
    pub needed_bytes: u64,
    pub available_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        bytes
    }

    #[test]
    fn accounts_lock_before_campaigns_whatever_their_ids() {
        let campaign = QuotaScope::Campaign(CampaignId::from_bytes(&id(1)).unwrap());
        let account = QuotaScope::Account(AccountId::from_bytes(&id(9)).unwrap());
        let other_account = QuotaScope::Account(AccountId::from_bytes(&id(2)).unwrap());
        assert_eq!(
            lock_order(&[campaign, account, other_account, account]),
            vec![other_account, account, campaign]
        );
    }
}
