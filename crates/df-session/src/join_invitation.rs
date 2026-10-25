//! Admission policy for a reusable join invitation and for an approved room-code request
//! (`planning/campaigns-and-table-research.md` 4.4). Pure: the caller reads the facts under the
//! campaign lock and supplies them; this module decides and the caller applies.
//!
//! `OneShotInvitation` (`invitation.rs`) stays as it is; `max_uses = 1` here is the one-shot case.
//! The unknown-token case is a store miss, not a case of this type.

/// The role an admission creates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinRole {
    Player,
    Contributor,
}

/// What the campaign knows about the joining account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberState {
    NotAMember,
    Active,
    /// Left on their own: any currently valid invitation may bring them back.
    Left,
    /// Removed, and someone of sufficient rank has allowed them back.
    RemovedMayRejoin,
    /// Removed and not allowed back: the response does not reveal the removal.
    RemovedNotAllowed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinFailure {
    Expired,
    Revoked,
    Exhausted,
    Full,
    ContributorsFull,
    AlreadyJoined,
    NoLongerValid,
    CampaignUnavailable,
    AccountClosing,
}

/// A successful admission: the role to create and whether it reactivates the same member row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinUse {
    pub joins_as: JoinRole,
    pub rejoin: bool,
}

/// The facts of one admission, read under the campaign lock.
#[derive(Clone, Copy, Debug)]
pub struct JoinContext {
    pub member: MemberState,
    pub campaign_active: bool,
    pub account_active: bool,
    pub seats_free: u32,
    pub contributors_free: u32,
    pub now_ms: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct JoinInvitation {
    pub expires_at_ms: u64,
    pub max_uses: u32,
    pub use_count: u32,
    pub revoked: bool,
    pub joins_as: JoinRole,
}

/// The checks every admission shares (invitation or approved request), in their fixed order:
/// an active member is `AlreadyJoined` (also after expiry, so an exact retry stays resolvable),
/// then the campaign, the account, and a removal that was not undone.
fn precheck(context: &JoinContext) -> Result<(), JoinFailure> {
    if context.member == MemberState::Active {
        return Err(JoinFailure::AlreadyJoined);
    }
    if !context.campaign_active {
        return Err(JoinFailure::CampaignUnavailable);
    }
    if !context.account_active {
        return Err(JoinFailure::AccountClosing);
    }
    if context.member == MemberState::RemovedNotAllowed {
        return Err(JoinFailure::NoLongerValid);
    }
    Ok(())
}

fn capacity(context: &JoinContext, role: JoinRole) -> Result<JoinUse, JoinFailure> {
    match role {
        JoinRole::Player if context.seats_free == 0 => Err(JoinFailure::Full),
        JoinRole::Contributor if context.contributors_free == 0 => {
            Err(JoinFailure::ContributorsFull)
        }
        _ => Ok(JoinUse {
            joins_as: role,
            rejoin: matches!(
                context.member,
                MemberState::Left | MemberState::RemovedMayRejoin
            ),
        }),
    }
}

/// An approver's admission of a code request into `role`. A removed account without permission
/// to return is `NoLongerValid` here; the caller maps it to "allow this person back first".
pub fn admit_approved(context: &JoinContext, role: JoinRole) -> Result<JoinUse, JoinFailure> {
    precheck(context)?;
    capacity(context, role)
}

impl JoinInvitation {
    /// Decides one use of this invitation. The caller increments `use_count` when it applies the
    /// result.
    pub fn consume(&self, context: &JoinContext) -> Result<JoinUse, JoinFailure> {
        precheck(context)?;
        if self.revoked {
            return Err(JoinFailure::Revoked);
        }
        if context.now_ms >= self.expires_at_ms {
            return Err(JoinFailure::Expired);
        }
        if self.use_count >= self.max_uses {
            return Err(JoinFailure::Exhausted);
        }
        capacity(context, self.joins_as)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invitation() -> JoinInvitation {
        JoinInvitation {
            expires_at_ms: 1_000,
            max_uses: 3,
            use_count: 0,
            revoked: false,
            joins_as: JoinRole::Player,
        }
    }

    fn context() -> JoinContext {
        JoinContext {
            member: MemberState::NotAMember,
            campaign_active: true,
            account_active: true,
            seats_free: 2,
            contributors_free: 2,
            now_ms: 10,
        }
    }

    #[test]
    fn a_usable_invitation_admits_a_new_member_in_its_role() {
        assert_eq!(
            invitation().consume(&context()),
            Ok(JoinUse {
                joins_as: JoinRole::Player,
                rejoin: false
            })
        );
        let contributor = JoinInvitation {
            joins_as: JoinRole::Contributor,
            ..invitation()
        };
        assert_eq!(
            contributor.consume(&context()),
            Ok(JoinUse {
                joins_as: JoinRole::Contributor,
                rejoin: false
            })
        );
    }

    #[test]
    fn an_active_member_is_already_joined_even_after_expiry_exhaustion_or_revocation() {
        let spent = JoinInvitation {
            use_count: 3,
            revoked: true,
            ..invitation()
        };
        let late = JoinContext {
            member: MemberState::Active,
            now_ms: 5_000,
            ..context()
        };
        assert_eq!(spent.consume(&late), Err(JoinFailure::AlreadyJoined));
    }

    #[test]
    fn campaign_state_and_account_state_come_before_the_invitation_checks() {
        let revoked = JoinInvitation {
            revoked: true,
            ..invitation()
        };
        let archived = JoinContext {
            campaign_active: false,
            ..context()
        };
        assert_eq!(
            revoked.consume(&archived),
            Err(JoinFailure::CampaignUnavailable)
        );
        let closing = JoinContext {
            account_active: false,
            ..context()
        };
        assert_eq!(revoked.consume(&closing), Err(JoinFailure::AccountClosing));
    }

    #[test]
    fn a_removed_member_without_permission_gets_no_longer_valid_before_any_invitation_reason() {
        let expired = JoinInvitation {
            expires_at_ms: 5,
            ..invitation()
        };
        let removed = JoinContext {
            member: MemberState::RemovedNotAllowed,
            ..context()
        };
        assert_eq!(expired.consume(&removed), Err(JoinFailure::NoLongerValid));
    }

    #[test]
    fn revoked_expired_and_exhausted_have_their_own_reasons_in_that_order() {
        let spent = JoinInvitation {
            use_count: 3,
            expires_at_ms: 5,
            revoked: true,
            ..invitation()
        };
        assert_eq!(spent.consume(&context()), Err(JoinFailure::Revoked));
        let spent = JoinInvitation {
            revoked: false,
            ..spent
        };
        assert_eq!(spent.consume(&context()), Err(JoinFailure::Expired));
        let spent = JoinInvitation {
            expires_at_ms: 1_000,
            ..spent
        };
        assert_eq!(spent.consume(&context()), Err(JoinFailure::Exhausted));
    }

    #[test]
    fn expiry_is_exclusive_at_the_instant() {
        let at = JoinContext {
            now_ms: 1_000,
            ..context()
        };
        assert_eq!(invitation().consume(&at), Err(JoinFailure::Expired));
        let before = JoinContext {
            now_ms: 999,
            ..context()
        };
        assert!(invitation().consume(&before).is_ok());
    }

    #[test]
    fn a_player_invitation_is_full_without_seats_and_a_contributor_invitation_is_never_seat_full() {
        let no_seats = JoinContext {
            seats_free: 0,
            ..context()
        };
        assert_eq!(invitation().consume(&no_seats), Err(JoinFailure::Full));
        let contributor = JoinInvitation {
            joins_as: JoinRole::Contributor,
            ..invitation()
        };
        assert!(contributor.consume(&no_seats).is_ok());
        let no_contributors = JoinContext {
            contributors_free: 0,
            ..context()
        };
        assert_eq!(
            contributor.consume(&no_contributors),
            Err(JoinFailure::ContributorsFull)
        );
        assert!(invitation().consume(&no_contributors).is_ok());
    }

    #[test]
    fn a_member_who_left_or_was_allowed_back_rejoins_in_the_invitations_role() {
        let contributor = JoinInvitation {
            joins_as: JoinRole::Contributor,
            ..invitation()
        };
        for state in [MemberState::Left, MemberState::RemovedMayRejoin] {
            let rejoining = JoinContext {
                member: state,
                ..context()
            };
            assert_eq!(
                contributor.consume(&rejoining),
                Ok(JoinUse {
                    joins_as: JoinRole::Contributor,
                    rejoin: true
                })
            );
        }
    }

    #[test]
    fn a_single_use_invitation_is_the_one_shot_case() {
        let once = JoinInvitation {
            max_uses: 1,
            use_count: 1,
            ..invitation()
        };
        assert_eq!(once.consume(&context()), Err(JoinFailure::Exhausted));
    }

    #[test]
    fn an_approved_request_follows_the_same_state_and_capacity_rules() {
        assert_eq!(
            admit_approved(&context(), JoinRole::Contributor),
            Ok(JoinUse {
                joins_as: JoinRole::Contributor,
                rejoin: false
            })
        );
        let removed = JoinContext {
            member: MemberState::RemovedNotAllowed,
            ..context()
        };
        assert_eq!(
            admit_approved(&removed, JoinRole::Player),
            Err(JoinFailure::NoLongerValid)
        );
        let no_seats = JoinContext {
            seats_free: 0,
            ..context()
        };
        assert_eq!(
            admit_approved(&no_seats, JoinRole::Player),
            Err(JoinFailure::Full)
        );
    }
}
