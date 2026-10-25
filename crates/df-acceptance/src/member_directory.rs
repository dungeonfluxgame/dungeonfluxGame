use std::collections::BTreeMap;

use df_auth::membership::{MembershipAuthority, MembershipRecord, MembershipRequest};

/// Scope every `MemberDirectory` record is reported under.
pub const DIRECTORY_TENANT: u8 = 1;
pub const DIRECTORY_CAMPAIGN: u8 = 2;
pub const DIRECTORY_ROLE: u8 = 3;

#[derive(Debug)]
struct Member {
    active: bool,
    revision: u32,
    expires_at: Option<u64>,
}

/// A faithful fake membership authority for one campaign with a set of members.
///
/// It reports a member's current status, revision and expiry, reports nothing for a principal
/// that was never a member, and fails closed with a source error while `unavailable` is set.
/// Revoking or reinstating bumps the member's access revision, as the real adapter must.
#[derive(Debug, Default)]
pub struct MemberDirectory {
    members: BTreeMap<u8, Member>,
    unavailable: bool,
}

impl MemberDirectory {
    /// Adds an active member whose grant expires at `expires_at` seconds, if any.
    #[must_use]
    pub fn with_member(mut self, principal: u8, expires_at: Option<u64>) -> Self {
        self.members.insert(
            principal,
            Member {
                active: true,
                revision: 1,
                expires_at,
            },
        );
        self
    }

    pub fn revoke(&mut self, principal: u8) {
        if let Some(member) = self.members.get_mut(&principal) {
            member.active = false;
            member.revision += 1;
        }
    }

    pub fn reinstate(&mut self, principal: u8) {
        if let Some(member) = self.members.get_mut(&principal) {
            member.active = true;
            member.revision += 1;
        }
    }

    pub fn set_unavailable(&mut self, unavailable: bool) {
        self.unavailable = unavailable;
    }
}

impl MembershipAuthority for MemberDirectory {
    type Principal = u8;
    type Tenant = u8;
    type Campaign = u8;
    type Role = u8;
    type Revision = u32;
    type Error = ();

    fn read_current(
        &mut self,
        request: MembershipRequest<'_, Self>,
    ) -> Result<Option<MembershipRecord<Self>>, Self::Error> {
        if self.unavailable {
            return Err(());
        }
        Ok(self
            .members
            .get(request.principal)
            .map(|member| MembershipRecord {
                principal: *request.principal,
                tenant: *request.tenant,
                campaign: *request.campaign,
                role: *request.role,
                revision: member.revision,
                active: member.active,
                expires_at: member.expires_at,
            }))
    }
}

#[cfg(test)]
mod tests {
    use df_auth::membership::{MembershipError, authorize_membership};

    use super::*;

    const MEMBER: u8 = 10;
    const STRANGER: u8 = 11;

    fn request(principal: &u8) -> MembershipRequest<'_, MemberDirectory> {
        MembershipRequest {
            principal,
            tenant: &DIRECTORY_TENANT,
            campaign: &DIRECTORY_CAMPAIGN,
            role: &DIRECTORY_ROLE,
        }
    }

    #[test]
    fn a_principal_who_was_never_a_member_is_reported_missing() {
        let mut directory = MemberDirectory::default().with_member(MEMBER, None);

        let stranger = authorize_membership(&mut directory, request(&STRANGER), 0);
        let member = authorize_membership(&mut directory, request(&MEMBER), 0);

        assert_eq!(stranger.err(), Some(MembershipError::Missing));
        assert!(member.is_ok());
    }

    #[test]
    fn revoking_one_member_leaves_the_others_active() {
        let mut directory = MemberDirectory::default()
            .with_member(MEMBER, None)
            .with_member(STRANGER, None);
        directory.revoke(MEMBER);

        let revoked = authorize_membership(&mut directory, request(&MEMBER), 0);
        let other = authorize_membership(&mut directory, request(&STRANGER), 0);

        assert_eq!(revoked.err(), Some(MembershipError::Revoked));
        assert!(other.is_ok());
    }
}
