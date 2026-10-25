//! Display pairing decisions (`planning/campaigns-and-table-research.md` 4.6): a device-
//! authorization-style challenge that ends in a read-only, device-bound ticket. Pure: the caller
//! reads the facts under the campaign lock and applies the decision.
//!
//! A ticket carries exactly one capability, viewing the public projection, and there is no way to
//! add another. Validity is exclusive at the expiry instant. Renewal extends the same ticket.

/// Compares two digests without an early exit.
pub fn digests_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingState {
    Pending,
    Approved,
    Redeemed,
    Cancelled,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingRefusal {
    /// The device secret does not match; the response is the neutral one.
    WrongDevice,
    Expired,
    Cancelled,
    NotApproved,
    NotPending,
    ApproverNotAllowed,
    /// A valid ticket cookie is already present or the first redemption succeeded.
    AlreadyRedeemed,
    ReissueUsed,
    ReissueWindowClosed,
    DisplayLimit,
}

/// What the store holds about one pairing.
#[derive(Clone, Copy, Debug)]
pub struct PairingFacts {
    pub state: PairingState,
    pub expires_at_ms: u64,
    pub device_digest: [u8; 32],
    pub reissue_count: u8,
    pub redeemed_at_ms: Option<u64>,
}

/// Whether the pairing may be approved now. `approver_may_pair` is the host or a co-host holding
/// `StartOrPauseSession` (4.1); the caller derives it from the member row.
pub fn approve(
    facts: &PairingFacts,
    approver_may_pair: bool,
    now_ms: u64,
) -> Result<(), PairingRefusal> {
    if !approver_may_pair {
        return Err(PairingRefusal::ApproverNotAllowed);
    }
    match facts.state {
        PairingState::Pending if now_ms < facts.expires_at_ms => Ok(()),
        PairingState::Pending | PairingState::Expired => Err(PairingRefusal::Expired),
        PairingState::Cancelled => Err(PairingRefusal::Cancelled),
        PairingState::Approved | PairingState::Redeemed => Err(PairingRefusal::NotPending),
    }
}

/// What a redemption does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedeemAction {
    /// Issue the first ticket (the cap is enforced against live tickets).
    IssueFirst,
    /// The response was lost: revoke the previous ticket and issue one replacement, once.
    Reissue,
}

/// Decides a redemption. `live_tickets` counts the session's live tickets; a reissue replaces one
/// ticket with one, so the cap applies only to the first issue.
pub fn redeem(
    facts: &PairingFacts,
    presented_device: &[u8; 32],
    ticket_cookie_valid: bool,
    live_tickets: u32,
    cap: u32,
    reissue_window_ms: u64,
    now_ms: u64,
) -> Result<RedeemAction, PairingRefusal> {
    if !digests_equal(&facts.device_digest, presented_device) {
        return Err(PairingRefusal::WrongDevice);
    }
    match facts.state {
        PairingState::Cancelled => Err(PairingRefusal::Cancelled),
        PairingState::Expired => Err(PairingRefusal::Expired),
        PairingState::Pending => Err(PairingRefusal::NotApproved),
        PairingState::Approved => {
            if now_ms >= facts.expires_at_ms {
                Err(PairingRefusal::Expired)
            } else if live_tickets >= cap {
                Err(PairingRefusal::DisplayLimit)
            } else {
                Ok(RedeemAction::IssueFirst)
            }
        }
        PairingState::Redeemed => {
            if ticket_cookie_valid {
                return Err(PairingRefusal::AlreadyRedeemed);
            }
            if facts.reissue_count >= 1 {
                return Err(PairingRefusal::ReissueUsed);
            }
            let redeemed_at = facts.redeemed_at_ms.unwrap_or(0);
            if now_ms.saturating_sub(redeemed_at) >= reissue_window_ms {
                return Err(PairingRefusal::ReissueWindowClosed);
            }
            Ok(RedeemAction::Reissue)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketRefusal {
    Revoked,
    WrongDevice,
    Expired,
    ApproverLostAuthority,
    SessionEnded,
    /// Renewal is only allowed in the last window of the ticket's life.
    OutsideRenewalWindow,
    /// The pairing's renewable period or the session end has been reached.
    RenewalCapReached,
}

#[derive(Clone, Copy, Debug)]
pub struct TicketFacts {
    pub device_digest: [u8; 32],
    pub expires_at_ms: u64,
    pub revoked: bool,
}

/// Whether a presented ticket authorizes viewing the public projection now. The ticket and the
/// device secret must both match; the approver must still be able to pair; the session must be
/// usable (an ended or unreadable session counts as revoked).
pub fn ticket_valid(
    ticket: &TicketFacts,
    presented_device: &[u8; 32],
    approver_authorized: bool,
    session_usable: bool,
    now_ms: u64,
) -> Result<(), TicketRefusal> {
    if ticket.revoked {
        return Err(TicketRefusal::Revoked);
    }
    if !digests_equal(&ticket.device_digest, presented_device) {
        return Err(TicketRefusal::WrongDevice);
    }
    if now_ms >= ticket.expires_at_ms {
        return Err(TicketRefusal::Expired);
    }
    if !approver_authorized {
        return Err(TicketRefusal::ApproverLostAuthority);
    }
    if !session_usable {
        return Err(TicketRefusal::SessionEnded);
    }
    Ok(())
}

/// The renewal policy: how long a ticket lives and when it may be extended.
#[derive(Clone, Copy, Debug)]
pub struct RenewalPolicy {
    pub ticket_lifetime_ms: u64,
    pub window_ms: u64,
    /// The pairing's `renewable_until`: approval time plus the maximum, never past session end.
    pub renewable_until_ms: u64,
}

/// The new expiry of the same ticket, or why it cannot be extended. Renewal at or after the old
/// expiry, outside the last window, or past the cap is refused; after that the display re-pairs.
pub fn extend(
    ticket: &TicketFacts,
    presented_device: &[u8; 32],
    approver_authorized: bool,
    session_usable: bool,
    policy: &RenewalPolicy,
    now_ms: u64,
) -> Result<u64, TicketRefusal> {
    ticket_valid(
        ticket,
        presented_device,
        approver_authorized,
        session_usable,
        now_ms,
    )?;
    if now_ms < ticket.expires_at_ms.saturating_sub(policy.window_ms) {
        return Err(TicketRefusal::OutsideRenewalWindow);
    }
    let candidate = now_ms.saturating_add(policy.ticket_lifetime_ms);
    let extended = candidate.min(policy.renewable_until_ms);
    if extended <= ticket.expires_at_ms {
        return Err(TicketRefusal::RenewalCapReached);
    }
    Ok(extended)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [8; 32];

    fn facts(state: PairingState) -> PairingFacts {
        PairingFacts {
            state,
            expires_at_ms: 600_000,
            device_digest: DEVICE,
            reissue_count: 0,
            redeemed_at_ms: None,
        }
    }

    #[test]
    fn only_a_delegated_approver_may_approve_a_pending_unexpired_pairing() {
        assert_eq!(approve(&facts(PairingState::Pending), true, 10), Ok(()));
        assert_eq!(
            approve(&facts(PairingState::Pending), false, 10),
            Err(PairingRefusal::ApproverNotAllowed)
        );
        assert_eq!(
            approve(&facts(PairingState::Pending), true, 600_000),
            Err(PairingRefusal::Expired)
        );
        assert_eq!(
            approve(&facts(PairingState::Approved), true, 10),
            Err(PairingRefusal::NotPending)
        );
        assert_eq!(
            approve(&facts(PairingState::Cancelled), true, 10),
            Err(PairingRefusal::Cancelled)
        );
    }

    #[test]
    fn redemption_needs_the_device_secret_and_an_approval() {
        let approved = facts(PairingState::Approved);
        assert_eq!(
            redeem(&approved, &OTHER, false, 0, 4, 60_000, 10),
            Err(PairingRefusal::WrongDevice)
        );
        assert_eq!(
            redeem(
                &facts(PairingState::Pending),
                &DEVICE,
                false,
                0,
                4,
                60_000,
                10
            ),
            Err(PairingRefusal::NotApproved)
        );
        assert_eq!(
            redeem(&approved, &DEVICE, false, 0, 4, 60_000, 10),
            Ok(RedeemAction::IssueFirst)
        );
        assert_eq!(
            redeem(&approved, &DEVICE, false, 0, 4, 60_000, 600_000),
            Err(PairingRefusal::Expired)
        );
    }

    #[test]
    fn the_display_cap_applies_to_the_first_issue_only() {
        let approved = facts(PairingState::Approved);
        assert_eq!(
            redeem(&approved, &DEVICE, false, 4, 4, 60_000, 10),
            Err(PairingRefusal::DisplayLimit)
        );
        assert_eq!(
            redeem(&approved, &DEVICE, false, 3, 4, 60_000, 10),
            Ok(RedeemAction::IssueFirst)
        );
        let redeemed = PairingFacts {
            redeemed_at_ms: Some(100),
            ..facts(PairingState::Redeemed)
        };
        assert_eq!(
            redeem(&redeemed, &DEVICE, false, 4, 4, 60_000, 200),
            Ok(RedeemAction::Reissue)
        );
    }

    #[test]
    fn a_lost_response_allows_exactly_one_reissue_inside_the_window() {
        let redeemed = PairingFacts {
            redeemed_at_ms: Some(1_000),
            ..facts(PairingState::Redeemed)
        };
        assert_eq!(
            redeem(&redeemed, &DEVICE, true, 1, 4, 60_000, 2_000),
            Err(PairingRefusal::AlreadyRedeemed)
        );
        assert_eq!(
            redeem(&redeemed, &DEVICE, false, 1, 4, 60_000, 2_000),
            Ok(RedeemAction::Reissue)
        );
        assert_eq!(
            redeem(&redeemed, &DEVICE, false, 1, 4, 60_000, 61_000),
            Err(PairingRefusal::ReissueWindowClosed)
        );
        let reissued = PairingFacts {
            reissue_count: 1,
            ..redeemed
        };
        assert_eq!(
            redeem(&reissued, &DEVICE, false, 1, 4, 60_000, 2_000),
            Err(PairingRefusal::ReissueUsed)
        );
    }

    fn ticket() -> TicketFacts {
        TicketFacts {
            device_digest: DEVICE,
            expires_at_ms: 100_000,
            revoked: false,
        }
    }

    #[test]
    fn a_ticket_is_valid_one_millisecond_before_expiry_and_refused_at_expiry() {
        assert_eq!(ticket_valid(&ticket(), &DEVICE, true, true, 99_999), Ok(()));
        assert_eq!(
            ticket_valid(&ticket(), &DEVICE, true, true, 100_000),
            Err(TicketRefusal::Expired)
        );
    }

    #[test]
    fn a_ticket_without_its_device_secret_or_with_a_lost_approver_or_ended_session_is_refused() {
        assert_eq!(
            ticket_valid(&ticket(), &OTHER, true, true, 10),
            Err(TicketRefusal::WrongDevice)
        );
        assert_eq!(
            ticket_valid(&ticket(), &DEVICE, false, true, 10),
            Err(TicketRefusal::ApproverLostAuthority)
        );
        assert_eq!(
            ticket_valid(&ticket(), &DEVICE, true, false, 10),
            Err(TicketRefusal::SessionEnded)
        );
        let revoked = TicketFacts {
            revoked: true,
            ..ticket()
        };
        assert_eq!(
            ticket_valid(&revoked, &DEVICE, true, true, 10),
            Err(TicketRefusal::Revoked)
        );
    }

    fn policy() -> RenewalPolicy {
        RenewalPolicy {
            ticket_lifetime_ms: 100_000,
            window_ms: 10_000,
            renewable_until_ms: 1_000_000,
        }
    }

    #[test]
    fn renewal_inside_the_window_extends_the_same_ticket() {
        assert_eq!(
            extend(&ticket(), &DEVICE, true, true, &policy(), 95_000),
            Ok(195_000)
        );
    }

    #[test]
    fn renewal_outside_the_window_or_at_the_expiry_instant_is_refused() {
        assert_eq!(
            extend(&ticket(), &DEVICE, true, true, &policy(), 80_000),
            Err(TicketRefusal::OutsideRenewalWindow)
        );
        assert_eq!(
            extend(&ticket(), &DEVICE, true, true, &policy(), 100_000),
            Err(TicketRefusal::Expired)
        );
        assert_eq!(
            extend(&ticket(), &DEVICE, true, true, &policy(), 90_000),
            Ok(190_000)
        );
    }

    #[test]
    fn renewal_is_capped_by_the_pairings_renewable_period() {
        let near = RenewalPolicy {
            renewable_until_ms: 150_000,
            ..policy()
        };
        assert_eq!(
            extend(&ticket(), &DEVICE, true, true, &near, 95_000),
            Ok(150_000)
        );
        let spent = TicketFacts {
            expires_at_ms: 150_000,
            ..ticket()
        };
        assert_eq!(
            extend(&spent, &DEVICE, true, true, &near, 149_000),
            Err(TicketRefusal::RenewalCapReached)
        );
    }

    #[test]
    fn digest_comparison_is_exact() {
        assert!(digests_equal(&DEVICE, &DEVICE));
        assert!(!digests_equal(&DEVICE, &OTHER));
        let mut almost = DEVICE;
        almost[31] ^= 1;
        assert!(!digests_equal(&DEVICE, &almost));
    }
}
