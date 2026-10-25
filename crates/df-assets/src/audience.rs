//! Who may see an asset (planning/asset-storage-research.md 6.4, plan 14.4, D25). The meet of two
//! audiences is a static table over the values alone (no membership lookup): an output made from
//! several inputs may be seen only by those who may see every input. `Withheld` (nobody) is refused
//! before admission, so nothing nobody may see is ever bought.

use df_types::{AccountId, CampaignId, MemberId};

/// A sorted, duplicate-free, nonempty set of campaign members.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MemberSet(Vec<MemberId>);

impl MemberSet {
    /// `None` for an empty list.
    pub fn new(mut members: Vec<MemberId>) -> Option<Self> {
        members.sort_unstable();
        members.dedup();
        (!members.is_empty()).then_some(Self(members))
    }

    pub fn members(&self) -> &[MemberId] {
        &self.0
    }

    fn intersection(&self, other: &Self) -> Option<Self> {
        let shared = self
            .0
            .iter()
            .filter(|member| other.0.binary_search(member).is_ok())
            .copied()
            .collect();
        Self::new(shared)
    }
}

/// The closed audience classes of the catalog.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Audience {
    /// Reviewed public examples only.
    PublicStatic,
    /// Every current member of the campaign (the table).
    CampaignShared(CampaignId),
    /// Exactly these current members of the campaign.
    Members(CampaignId, MemberSet),
    /// Whoever currently holds the campaign's host role (a game master's notes, D25).
    Host(CampaignId),
    /// One account's personal media; never follows any role.
    OwnerPrivate(AccountId),
    /// Nobody: server-only. Never granted and never admitted for generation.
    Withheld,
}

impl Audience {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PublicStatic => "public_static",
            Self::CampaignShared(_) => "campaign_shared",
            Self::Members(..) => "members",
            Self::Host(_) => "host",
            Self::OwnerPrivate(_) => "owner_private",
            Self::Withheld => "withheld",
        }
    }

    fn campaign(&self) -> Option<&CampaignId> {
        match self {
            Self::CampaignShared(campaign) | Self::Members(campaign, _) | Self::Host(campaign) => {
                Some(campaign)
            }
            Self::PublicStatic | Self::OwnerPrivate(_) | Self::Withheld => None,
        }
    }

    /// The 6.4 meet table. Symmetric; `PublicStatic` is the identity and `Withheld` absorbs.
    pub fn meet(&self, other: &Self) -> Self {
        use Audience as A;
        if let (Some(left), Some(right)) = (self.campaign(), other.campaign())
            && left != right
        {
            return A::Withheld;
        }
        match (self, other) {
            (A::Withheld, _) | (_, A::Withheld) => A::Withheld,
            (A::PublicStatic, any) | (any, A::PublicStatic) => any.clone(),
            (A::CampaignShared(_), any) | (any, A::CampaignShared(_))
                if !matches!(any, A::OwnerPrivate(_)) =>
            {
                any.clone()
            }
            (A::Members(campaign, left), A::Members(_, right)) => left
                .intersection(right)
                .map_or(A::Withheld, |shared| A::Members(*campaign, shared)),
            (A::Host(campaign), A::Host(_)) => A::Host(*campaign),
            (A::OwnerPrivate(left), A::OwnerPrivate(right)) if left == right => {
                A::OwnerPrivate(*left)
            }
            _ => A::Withheld,
        }
    }

    /// The meet of a requested audience and every reference input's audience.
    pub fn meet_all<'a>(requested: &Self, inputs: impl IntoIterator<Item = &'a Self>) -> Self {
        inputs
            .into_iter()
            .fold(requested.clone(), |audience, input| audience.meet(input))
    }

    /// Canonical bytes for the request key: a tag byte, the campaign or account id where the
    /// variant has one, and for `Members` a big-endian `u32` count and the sorted member ids.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::PublicStatic => out.push(1),
            Self::CampaignShared(campaign) => {
                out.push(2);
                out.extend_from_slice(campaign.as_bytes());
            }
            Self::Members(campaign, members) => {
                out.push(3);
                out.extend_from_slice(campaign.as_bytes());
                // A member set is bounded by the seat limit (D9); saturation is unreachable and
                // only keeps the encoding total.
                let count = u32::try_from(members.0.len()).unwrap_or(u32::MAX);
                out.extend_from_slice(&count.to_be_bytes());
                for member in &members.0 {
                    out.extend_from_slice(member.as_bytes());
                }
            }
            Self::Host(campaign) => {
                out.push(4);
                out.extend_from_slice(campaign.as_bytes());
            }
            Self::OwnerPrivate(account) => {
                out.push(5);
                out.extend_from_slice(account.as_bytes());
            }
            Self::Withheld => out.push(6),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(byte: u8) -> [u8; 16] {
        let mut value = [0_u8; 16];
        value[15] = byte;
        value
    }

    fn campaign(byte: u8) -> CampaignId {
        CampaignId::from_bytes(&bytes(byte)).unwrap()
    }

    fn members(campaign_byte: u8, ids: &[u8]) -> Audience {
        Audience::Members(
            campaign(campaign_byte),
            MemberSet::new(
                ids.iter()
                    .map(|id| MemberId::from_bytes(&bytes(*id)).unwrap())
                    .collect(),
            )
            .unwrap(),
        )
    }

    fn owner(byte: u8) -> Audience {
        Audience::OwnerPrivate(AccountId::from_bytes(&bytes(byte)).unwrap())
    }

    fn check(left: &Audience, right: &Audience, expected: &Audience) {
        assert_eq!(&left.meet(right), expected, "{left:?} meet {right:?}");
        assert_eq!(&right.meet(left), expected, "{right:?} meet {left:?}");
    }

    #[test]
    fn public_static_is_the_identity_of_the_meet() {
        let shared = Audience::CampaignShared(campaign(1));
        let host = Audience::Host(campaign(1));
        for audience in [
            Audience::PublicStatic,
            shared,
            members(1, &[1, 2]),
            host,
            owner(7),
            Audience::Withheld,
        ] {
            check(&Audience::PublicStatic, &audience, &audience);
        }
    }

    #[test]
    fn campaign_shared_narrows_to_members_or_host_and_refuses_personal_media() {
        let shared = Audience::CampaignShared(campaign(1));
        check(&shared, &shared, &shared);
        check(&shared, &members(1, &[1, 2]), &members(1, &[1, 2]));
        check(
            &shared,
            &Audience::Host(campaign(1)),
            &Audience::Host(campaign(1)),
        );
        check(&shared, &owner(7), &Audience::Withheld);
    }

    #[test]
    fn members_meet_to_their_intersection_or_withheld() {
        check(
            &members(1, &[1, 2]),
            &members(1, &[2, 3]),
            &members(1, &[2]),
        );
        check(&members(1, &[1]), &members(1, &[2]), &Audience::Withheld);
        check(
            &members(1, &[1]),
            &Audience::Host(campaign(1)),
            &Audience::Withheld,
        );
        check(&members(1, &[1]), &owner(7), &Audience::Withheld);
    }

    #[test]
    fn host_meets_host_and_nothing_personal() {
        let host = Audience::Host(campaign(1));
        check(&host, &host, &host);
        check(&host, &owner(7), &Audience::Withheld);
    }

    #[test]
    fn owner_private_meets_only_the_same_account() {
        check(&owner(7), &owner(7), &owner(7));
        check(&owner(7), &owner(8), &Audience::Withheld);
    }

    #[test]
    fn withheld_absorbs_and_two_campaigns_never_meet() {
        for audience in [
            Audience::CampaignShared(campaign(1)),
            members(1, &[1]),
            Audience::Host(campaign(1)),
            owner(7),
        ] {
            check(&Audience::Withheld, &audience, &Audience::Withheld);
        }
        check(
            &Audience::CampaignShared(campaign(1)),
            &Audience::CampaignShared(campaign(2)),
            &Audience::Withheld,
        );
        check(&members(1, &[1]), &members(2, &[1]), &Audience::Withheld);
    }

    #[test]
    fn the_meet_of_all_inputs_folds_the_table() {
        let inputs = [Audience::CampaignShared(campaign(1)), members(1, &[1, 2])];
        assert_eq!(
            Audience::meet_all(&members(1, &[2, 3]), &inputs),
            members(1, &[2])
        );
    }

    #[test]
    fn a_member_set_is_sorted_deduplicated_and_never_empty() {
        let set = MemberSet::new(vec![
            MemberId::from_bytes(&bytes(2)).unwrap(),
            MemberId::from_bytes(&bytes(1)).unwrap(),
            MemberId::from_bytes(&bytes(2)).unwrap(),
        ])
        .unwrap();
        assert_eq!(set.members().len(), 2);
        assert!(set.members()[0] < set.members()[1]);
        assert_eq!(MemberSet::new(Vec::new()), None);
    }
}
