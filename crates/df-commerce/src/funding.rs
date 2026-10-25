//! Funding lots, the draw order and the trial grant (planning/finance-ledger-research.md 7).
//!
//! A lot is one source of credit for one payer. Credit is drawn promotional first (oldest first),
//! then purchased lots oldest first; only `Active` lots are drawn from.

use crate::finance_ids::LotId;

/// Where a lot's credit came from.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LotClass {
    Purchased,
    Promotional,
}

impl LotClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Purchased => "purchased",
            Self::Promotional => "promotional",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "purchased" => Some(Self::Purchased),
            "promotional" => Some(Self::Promotional),
            _ => None,
        }
    }
}

/// The promotional sources (closed).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromoSource {
    Trial,
    Goodwill,
}

impl PromoSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trial => "trial",
            Self::Goodwill => "goodwill",
        }
    }
}

/// A lot state (7.1).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LotState {
    Active,
    Frozen,
    ChargedBack,
    Closed,
    Forfeited,
}

impl LotState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::ChargedBack => "charged_back",
            Self::Closed => "closed",
            Self::Forfeited => "forfeited",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "active" => Some(Self::Active),
            "frozen" => Some(Self::Frozen),
            "charged_back" => Some(Self::ChargedBack),
            "closed" => Some(Self::Closed),
            "forfeited" => Some(Self::Forfeited),
            _ => None,
        }
    }
}

/// A lot as the draw sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawableLot {
    pub lot: LotId,
    pub class: LotClass,
    pub state: LotState,
    pub created_at_ms: i64,
    pub available_micro_dollars: u64,
}

/// Why a draw was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawRefusal {
    InsufficientCredit { available: u64, requested: u64 },
}

/// Promotional lots first, oldest first, then purchased lots oldest first (7.2). Ties by lot id.
pub fn draw_order(lots: &mut [DrawableLot]) {
    lots.sort_by_key(|lot| (lot.class == LotClass::Purchased, lot.created_at_ms, lot.lot));
}

/// The portions of `amount` taken from `lots` in draw order; only `Active` lots count.
pub fn plan_draw(
    amount: u64,
    lots: &[DrawableLot],
) -> Result<Vec<(LotId, LotClass, u64)>, DrawRefusal> {
    let mut ordered: Vec<DrawableLot> = lots
        .iter()
        .copied()
        .filter(|lot| lot.state == LotState::Active)
        .collect();
    draw_order(&mut ordered);
    let available: u64 = ordered.iter().map(|lot| lot.available_micro_dollars).sum();
    if available < amount {
        return Err(DrawRefusal::InsufficientCredit {
            available,
            requested: amount,
        });
    }
    let mut remaining = amount;
    let mut portions = Vec::new();
    for lot in ordered {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(lot.available_micro_dollars);
        if take > 0 {
            portions.push((lot.lot, lot.class, take));
            remaining -= take;
        }
    }
    Ok(portions)
}

/// The trial policy (D3, ruling (b)): one per account and one per verified email address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrialPolicy {
    pub enabled: bool,
    pub amount_micro_dollars: u64,
}

impl Default for TrialPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            amount_micro_dollars: 10_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrialDecision {
    Grant,
    AlreadyGrantedToAccount,
    AlreadyGrantedToEmail,
    Disabled,
}

pub fn decide_trial(
    policy: TrialPolicy,
    granted_to_account: bool,
    granted_to_email: bool,
) -> TrialDecision {
    if !policy.enabled || policy.amount_micro_dollars == 0 {
        TrialDecision::Disabled
    } else if granted_to_account {
        TrialDecision::AlreadyGrantedToAccount
    } else if granted_to_email {
        TrialDecision::AlreadyGrantedToEmail
    } else {
        TrialDecision::Grant
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(tag: u8, class: LotClass, created: i64, available: u64) -> DrawableLot {
        DrawableLot {
            lot: LotId::from_array([tag; 16]),
            class,
            state: LotState::Active,
            created_at_ms: created,
            available_micro_dollars: available,
        }
    }

    #[test]
    fn trial_credit_is_spent_before_purchased_credit() {
        let lots = [
            lot(1, LotClass::Purchased, 1, 20),
            lot(2, LotClass::Promotional, 5, 10),
        ];
        let portions = plan_draw(15, &lots).expect("draw");
        assert_eq!(portions[0].1, LotClass::Promotional);
        assert_eq!(portions[0].2, 10);
        assert_eq!(portions[1].2, 5);
    }

    #[test]
    fn purchased_lots_are_drawn_oldest_first() {
        let lots = [
            lot(1, LotClass::Purchased, 9, 20),
            lot(2, LotClass::Purchased, 3, 20),
        ];
        let portions = plan_draw(25, &lots).expect("draw");
        assert_eq!(portions[0].0, LotId::from_array([2; 16]));
        assert_eq!(portions[1].2, 5);
    }

    #[test]
    fn only_active_lots_are_drawn_from() {
        let mut frozen = lot(1, LotClass::Purchased, 1, 100);
        frozen.state = LotState::Frozen;
        assert_eq!(
            plan_draw(1, &[frozen]),
            Err(DrawRefusal::InsufficientCredit {
                available: 0,
                requested: 1
            })
        );
    }

    #[test]
    fn a_second_trial_for_the_same_email_key_or_account_is_refused() {
        let policy = TrialPolicy::default();
        assert_eq!(decide_trial(policy, false, false), TrialDecision::Grant);
        assert_eq!(
            decide_trial(policy, true, false),
            TrialDecision::AlreadyGrantedToAccount
        );
        assert_eq!(
            decide_trial(policy, false, true),
            TrialDecision::AlreadyGrantedToEmail
        );
    }
}
